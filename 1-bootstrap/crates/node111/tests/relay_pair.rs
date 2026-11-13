//! Layer 7 row D6 acceptance (`os/7-relay/README.md` §4.6 steps 1–6, §4.1 rule 1, §13.1; contract
//! `offerRelayPair`, `pairRelayHub`, `unpairRelayHub`): hubs with the real identity handle and
//! fabric on loopback sockets, driven through the real dashboard app and the CLI.
//!
//! - the bundle drawn by the offer round trips (an IPv6 literal included) and is dismissed with
//!   the Layer 5 offer route;
//! - the complement matrix: every pick that is not the complement of the bundle's role with the
//!   same link, or that §4.6 forbids, is `409 pair_role_mismatch` and records nothing; the
//!   offer's own `via` rules;
//! - `409 needs_hub` on both routes without the hub role, and nothing minted, pinned or dialled;
//! - loopback only, and the agent's bearer refused, on all three routes;
//! - the `pair_plain` warning round trip (verbatim text, hash, a stale hash refused, the hash kept
//!   with the pair), a `lan = true` pair on a TEST-NET address running plain with no consent;
//! - two hubs paired end to end inside TLS 1.3 (loopback is not private): the hub that scans first
//!   is refused until the other has pinned it, both `ENROLL`s land, both records are whole, the
//!   row reaches `fabric/settings`; then unpaired, idempotently;
//! - `111 fabric pair --show` and `111 fabric pair <bundle> [--lan]` with the warning asked;
//! - a pin added or removed is seen by the next handshake while the TLS state stays cached.

use axum::extract::connect_info::MockConnectInfo;
use id111::keyset::NodeId;
use mesh111::fabric::{PairRole, PairVia, PairedHub};
use mesh111::link::tcp_lan::StreamWrap;
use node111::daemon::dashboard::api_v1::auth;
use node111::daemon::dashboard::{
    routes, serve_listener, ApiConfig, ApiState, DashboardState, SharedState,
};
use node111::daemon::fabric::cli::{self as fcli, FabricCmd};
use node111::daemon::fabric::settings::{LinkSpki, PairLink, PairTable, ED25519_SPKI_PREFIX};
use node111::daemon::fabric::{self, FabricConfig, FabricHandle};
use node111::daemon::relay::linkkey::{self, LinkKey};
use node111::daemon::relay::pair::{
    pair_plain_hash, pair_plain_hash_hex, PairBundle, PAIR_PLAIN_WARNING,
};
use node111::daemon::relay::tls::{self, Outbound, RpkWrap};
use node111::daemon::DaemonLayout;
use serde_json::{json, Map, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

// ---------------------------------------------------------------- contract

/// The contract digest (schemas + operations) the UI tests validate with.
struct Contract {
    schemas: Map<String, Value>,
    ops: Map<String, Value>,
}

fn digest_object(text: &str, marker: &str) -> Map<String, Value> {
    let start = text.find(marker).expect("marker") + marker.len();
    let mut it = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
    match it.next() {
        Some(Ok(Value::Object(m))) => m,
        other => panic!("contract digest {marker}: {other:?}"),
    }
}

fn contract() -> &'static Contract {
    static C: OnceLock<Contract> = OnceLock::new();
    C.get_or_init(|| {
        let path = format!(
            "{}/../../../4-interface/ui/test/contract.gen.ts",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(path).expect("contract.gen.ts");
        Contract {
            schemas: digest_object(&text, "export const schemas: Record<string, JsonSchema> = "),
            ops: digest_object(
                &text,
                "export const operations: Record<string, ContractOperation> = ",
            ),
        }
    })
}

/// `^…$` patterns of the contract: literals, `[…]` classes, `+`, `{n}`, `{n,m}`.
fn re_match(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat
        .strip_prefix('^')
        .and_then(|p| p.strip_suffix('$'))
        .unwrap_or_else(|| panic!("unanchored pattern {pat}"))
        .chars()
        .collect();
    enum Atom {
        Lit(char),
        Class(Vec<(char, char)>),
    }
    let mut atoms: Vec<(Atom, usize, usize)> = Vec::new();
    let mut i = 0;
    while i < p.len() {
        let atom = if p[i] == '[' {
            let mut ranges = Vec::new();
            i += 1;
            while p[i] != ']' {
                if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
                    ranges.push((p[i], p[i + 2]));
                    i += 3;
                } else {
                    ranges.push((p[i], p[i]));
                    i += 1;
                }
            }
            i += 1;
            Atom::Class(ranges)
        } else {
            i += 1;
            Atom::Lit(p[i - 1])
        };
        let (min, max) = match p.get(i) {
            Some('+') => {
                i += 1;
                (1, usize::MAX)
            }
            Some('{') => {
                let end = p[i..].iter().position(|c| *c == '}').unwrap() + i;
                let inner: String = p[i + 1..end].iter().collect();
                i = end + 1;
                match inner.split_once(',') {
                    Some((a, b)) => (a.parse().unwrap(), b.parse().unwrap()),
                    None => {
                        let n = inner.parse().unwrap();
                        (n, n)
                    }
                }
            }
            _ => (1, 1),
        };
        atoms.push((atom, min, max));
    }
    let s: Vec<char> = s.chars().collect();
    let mut at = 0;
    for (atom, min, max) in atoms {
        let mut n = 0;
        while n < max
            && at < s.len()
            && match &atom {
                Atom::Lit(c) => s[at] == *c,
                Atom::Class(r) => r.iter().any(|(a, b)| s[at] >= *a && s[at] <= *b),
            }
        {
            n += 1;
            at += 1;
        }
        if n < min {
            return false;
        }
    }
    at == s.len()
}

fn type_ok(v: &Value, t: &str) -> bool {
    match t {
        "string" => v.is_string(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        _ => false,
    }
}

/// A port of `4-interface/ui/test/helpers/validate.ts` (the subset these shapes use).
fn validate(v: &Value, schema: &Value, path: &str, errs: &mut Vec<String>) {
    let c = contract();
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        let name = r.strip_prefix("#/components/schemas/").expect("local ref");
        let s = c
            .schemas
            .get(name)
            .unwrap_or_else(|| panic!("no schema {name}"));
        return validate(v, s, path, errs);
    }
    if let Some(alts) = schema.get("oneOf").and_then(Value::as_array) {
        let ok = alts.iter().filter(|a| {
            let mut e = Vec::new();
            validate(v, a, path, &mut e);
            e.is_empty()
        });
        if ok.count() != 1 {
            errs.push(format!("{path}: not exactly one of oneOf"));
        }
        return;
    }
    if let Some(t) = schema.get("type") {
        let types: Vec<&str> = match t {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !types.iter().any(|t| type_ok(v, t)) {
            errs.push(format!("{path}: expected {types:?}, got {v}"));
            return;
        }
    }
    if let Some(e) = schema.get("enum").and_then(Value::as_array) {
        if !e.contains(v) {
            errs.push(format!("{path}: {v} not in enum {e:?}"));
        }
    }
    if let Some(n) = v.as_f64() {
        if schema
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|m| n < m)
        {
            errs.push(format!("{path}: below minimum"));
        }
    }
    if let Some(s) = v.as_str() {
        let len = s.encode_utf16().count() as u64;
        if schema
            .get("minLength")
            .and_then(Value::as_u64)
            .is_some_and(|m| len < m)
        {
            errs.push(format!("{path}: shorter than minLength"));
        }
        if schema
            .get("maxLength")
            .and_then(Value::as_u64)
            .is_some_and(|m| len > m)
        {
            errs.push(format!("{path}: longer than maxLength"));
        }
        if let Some(p) = schema.get("pattern").and_then(Value::as_str) {
            if !re_match(p, s) {
                errs.push(format!("{path}: {s:?} does not match {p}"));
            }
        }
    }
    if let (Some(a), Some(items)) = (v.as_array(), schema.get("items")) {
        for (i, x) in a.iter().enumerate() {
            validate(x, items, &format!("{path}[{i}]"), errs);
        }
    }
    if let Some(o) = v.as_object() {
        for k in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !o.contains_key(k) {
                errs.push(format!("{path}: missing required {k:?}"));
            }
        }
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (k, sub) in props {
                if let Some(x) = o.get(k) {
                    validate(x, sub, &format!("{path}.{k}"), errs);
                }
            }
        }
    }
}

fn assert_schema(name: &str, v: &Value) {
    let mut errs = Vec::new();
    validate(
        v,
        &json!({ "$ref": format!("#/components/schemas/{name}") }),
        "$",
        &mut errs,
    );
    assert!(errs.is_empty(), "{name}: {errs:#?}\n{v:#}");
}

/// A response against operation `op`: its success status and schema, or a valid `Problem` with
/// one of the operation's error statuses.
fn check_op(op: &str, r: &Resp) {
    let o = contract()
        .ops
        .get(op)
        .unwrap_or_else(|| panic!("no operation {op}"));
    let success = o["success"].as_u64().unwrap() as u16;
    if (200..300).contains(&r.status) {
        assert_eq!(r.status, success, "{op}: {}", r.body);
        if let Some(schema) = o.get("schema").filter(|s| !s.is_null()) {
            let mut errs = Vec::new();
            validate(&r.json(), schema, "$", &mut errs);
            assert!(errs.is_empty(), "{op}: {errs:#?}\n{}", r.body);
        }
    } else {
        assert_eq!(
            r.header("content-type"),
            Some("application/problem+json"),
            "{op}: {}",
            r.body
        );
        assert_schema("Problem", &r.json());
        let errors: Vec<u64> = o["errors"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_u64)
            .collect();
        assert!(
            errors.contains(&u64::from(r.status)) || r.status == 401,
            "{op}: {} is not one of {errors:?}: {}",
            r.status,
            r.body
        );
    }
}

// ---------------------------------------------------------------- http

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", self.body))
    }

    fn problem(&self, status: u16, code: &str) {
        assert_eq!(self.status, status, "body: {}", self.body);
        assert_eq!(self.json()["code"], code, "{}", self.body);
        assert_schema("Problem", &self.json());
    }
}

/// One request to `127.0.0.1:port` (`Connection: close`, no chunked answers here).
fn http(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&Value>,
) -> Resp {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut req =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    let payload = body.map(Value::to_string).unwrap_or_default();
    if method != "GET" {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            payload.len()
        ));
    }
    req.push_str("\r\n");
    req.push_str(&payload);
    conn.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    conn.read_to_end(&mut raw).unwrap();
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(raw.len());
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let body = raw.get(split + 4..).unwrap_or(&[]);
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    Resp {
        status,
        headers,
        body: String::from_utf8_lossy(body).into_owned(),
    }
}

// ---------------------------------------------------------------- nodes

const LAN_PEER: &str = "192.168.1.50:40000";

struct Node {
    api: Arc<ApiState>,
    fabric: Arc<FabricHandle>,
    /// Dashboard on a real loopback socket (trusted loopback).
    port: u16,
    /// The same app with every request from a LAN peer.
    lan: u16,
    layout: DaemonLayout,
    id: NodeId,
    _dir: tempfile::TempDir,
}

async fn serve(api: &Arc<ApiState>, peer: Option<&str>) -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = routes::app(Arc::clone(api));
    match peer {
        None => {
            tokio::spawn(serve_listener(listener, app));
        }
        Some(p) => {
            let addr: SocketAddr = p.parse().unwrap();
            let app = app.layer(MockConnectInfo(addr));
            tokio::spawn(async move { axum::serve(listener, app).await });
        }
    }
    port
}

fn test_config(layout: &DaemonLayout, hub: bool) -> FabricConfig {
    let mut c = FabricConfig::production(layout, None);
    c.listen = "127.0.0.1:0".parse().unwrap();
    // every hub of this test is on loopback
    c.tcp.refuse_loopback = false;
    c.tcp.backoff_min = Duration::from_millis(30);
    c.tcp.backoff_max = Duration::from_millis(200);
    c.tcp.handshake_timeout = Duration::from_secs(3);
    c.hub_default = hub;
    // the scan route waits min(3 s, this) for the attempt's first answer
    c.enroll_timeout = Duration::from_secs(1);
    c.housekeeping = Duration::from_millis(200);
    c
}

async fn node(hub: bool) -> Node {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let identity = node111::identity::generate(&layout.identity_dir()).unwrap();
    let hex = identity.node_id_hex();
    let id = NodeId::from(id111::encoding::hex_decode(&hex).unwrap());
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.11.11")));
    dash.write().unwrap().node_id = Some(hex);
    let api = ApiState::new(dash, ApiConfig::new(layout.clone(), false));
    let fabric = fabric::start(test_config(&layout, hub), Some(identity), &api).await;
    assert!(fabric.running(), "{:?}", fabric.down_reason());
    api.insert_ext(Arc::clone(&fabric));
    let port = serve(&api, None).await;
    let lan = serve(&api, Some(LAN_PEER)).await;
    Node {
        api,
        fabric,
        port,
        lan,
        layout,
        id,
        _dir: dir,
    }
}

fn hex(id: &NodeId) -> String {
    id.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

impl Node {
    fn post(&self, path: &str, body: Value) -> Resp {
        http(self.port, "POST", path, &[], Some(&body))
    }

    fn delete(&self, path: &str) -> Resp {
        http(self.port, "DELETE", path, &[], None)
    }

    fn get(&self, path: &str) -> Resp {
        http(self.port, "GET", path, &[], None)
    }

    /// `POST /relay/pair/offer {role, via}`: the bundle text.
    fn offer(&self, role: &str, via: &str) -> String {
        let r = self.post(
            "/api/v1/relay/pair/offer",
            json!({"role": role, "via": via}),
        );
        check_op("offerRelayPair", &r);
        assert_eq!(r.status, 200, "{}", r.body);
        r.json()["bundle"]["text"].as_str().unwrap().to_string()
    }

    fn pair(&self, body: Value) -> Resp {
        let r = self.post("/api/v1/relay/pair", body);
        check_op("pairRelayHub", &r);
        r
    }

    fn unpair(&self, far: &NodeId) -> Resp {
        let r = self.delete(&format!("/api/v1/relay/paired/{}", hex(far)));
        check_op("unpairRelayHub", &r);
        r
    }

    fn table(&self) -> Vec<PairLink> {
        self.fabric.pair_table().read().unwrap().clone()
    }

    fn link(&self, far: &NodeId) -> Option<PairLink> {
        self.table().into_iter().find(|l| l.node_id == *far)
    }

    async fn records(&self) -> Vec<PairedHub> {
        self.fabric
            .fabric()
            .unwrap()
            .settings()
            .await
            .unwrap()
            .paired_hubs
    }

    fn has_link_key(&self) -> bool {
        linkkey::path(&self.layout.root).exists()
    }

    /// A paired LAN session cookie.
    fn session(&self) -> String {
        let code = self.api.auth.pair_code();
        let s = self
            .api
            .auth
            .pair(LAN_PEER.parse::<SocketAddr>().unwrap().ip(), &code)
            .unwrap();
        format!("s111={}", s.token)
    }
}

/// Poll `f` until it yields, up to `secs`.
async fn until<T, F: std::future::Future<Output = Option<T>>>(
    secs: u64,
    what: &str,
    mut f: impl FnMut() -> F,
) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `bundle` with another address and, when `keep_key` is false, no link key (a build without
/// TLS-RPK: the warning case).
fn rewritten(bundle: &str, address: &str, keep_key: bool) -> String {
    let b = PairBundle::parse(bundle).unwrap();
    PairBundle {
        address: Some(address.parse().unwrap()),
        link_spki: b.link_spki.filter(|_| keep_key),
        ..b
    }
    .text()
}

// ---------------------------------------------------------------- tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_offer_draws_a_bundle_that_round_trips() {
    let a = node(true).await;
    let r = a.post(
        "/api/v1/relay/pair/offer",
        json!({"role": "leaf", "via": "tcp"}),
    );
    check_op("offerRelayPair", &r);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.header("cache-control"), Some("no-store"));
    let v = r.json();
    assert_eq!(v["schema"], 1);
    let text = v["bundle"]["text"].as_str().unwrap();
    assert!(v["bundle"]["qr"]["size"].as_u64().unwrap() > 0);
    let b = PairBundle::parse(text).unwrap();
    // the current card, the code shown once, the link key minted now, the listener's address
    let card = a.get("/api/v1/contacts/me/card").json()["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(b.card, card);
    assert_eq!(b.code, v["code"].as_str().unwrap().replace('-', ""));
    assert!(a.has_link_key());
    let key = *LinkKey::load(&a.layout.root).unwrap().unwrap().spki();
    assert_eq!(b.link_spki, Some(key));
    assert_eq!(v["link_spki"], key.text().as_str());
    assert_eq!(b.address, a.fabric.offer_address().await);
    assert!(b.address.is_some());
    assert_eq!((b.role, b.via), (PairRole::Leaf, PairVia::Tcp));
    assert_eq!(PairBundle::parse(&b.text()), Ok(b.clone()));
    // a bracketed IPv6 literal with its port survives the round trip
    for v6 in [
        "[2001:db8::7]:11183",
        "[fd00:1:2:3:4:5:6:7]:65535",
        "[::ffff:192.0.2.1]:11183",
    ] {
        let with = PairBundle {
            address: Some(v6.parse().unwrap()),
            ..b.clone()
        };
        let t = with.text();
        assert!(
            t.contains(&format!(":{}:leaf:tcp", with.address.unwrap())),
            "{t}"
        );
        assert_eq!(PairBundle::parse(&t), Ok(with));
    }
    // the key is minted once: a second offer shows the same key and a fresh code
    let again = PairBundle::parse(&a.offer("peer", "carrier")).unwrap();
    assert_eq!(again.link_spki, Some(key));
    assert_ne!(again.code, b.code);
    assert_eq!((again.role, again.via), (PairRole::Peer, PairVia::Carrier));
    // the offer is open until the Layer 5 route dismisses it
    assert_eq!(a.get("/api/v1/fabric").json()["hub"]["offer_active"], true);
    let d = a.delete("/api/v1/fabric/hub/offer");
    assert_eq!(d.status, 204, "{}", d.body);
    assert_eq!(a.get("/api/v1/fabric").json()["hub"]["offer_active"], false);
    // the offer's own via rules (§4.6 step 2): a carrier pair is a peer
    for (role, via) in [("leaf", "carrier"), ("gateway", "carrier")] {
        let r = a.post(
            "/api/v1/relay/pair/offer",
            json!({"role": role, "via": via}),
        );
        check_op("offerRelayPair", &r);
        r.problem(409, "pair_role_mismatch");
    }
    for bad in [
        json!({}),
        json!({"role": "peer"}),
        json!({"role": "x", "via": "tcp"}),
    ] {
        let r = a.post("/api/v1/relay/pair/offer", bad);
        check_op("offerRelayPair", &r);
        r.problem(400, "bad_request");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_pick_but_the_complement_with_the_same_link_is_refused() {
    let a = node(true).await;
    let b = node(true).await;
    let allowed = |role: PairRole, via: PairVia| via == PairVia::Tcp || role == PairRole::Peer;
    let mut refused = 0;
    for shown in PairRole::ALL {
        for shown_via in PairVia::ALL {
            if !allowed(shown, shown_via) {
                continue;
            }
            let bundle = a.offer(shown.code(), shown_via.code());
            for role in PairRole::ALL {
                for via in PairVia::ALL {
                    if shown.complement() == role && via == shown_via && allowed(role, via) {
                        continue; // the fitting pick: paired end to end below
                    }
                    let r =
                        b.pair(json!({"bundle": bundle, "role": role.code(), "via": via.code()}));
                    r.problem(409, "pair_role_mismatch");
                    refused += 1;
                }
            }
        }
    }
    assert_eq!(refused, 4 * 6 - 4);
    // nothing recorded, pinned, minted or dialled on the scanning hub
    assert!(b.table().is_empty());
    assert!(b.records().await.is_empty());
    assert!(!b.has_link_key());
    assert_eq!(b.fabric.tcp().unwrap().connection_count(), 0);
    // a malformed bundle is 400
    for bad in [
        "111HUB:CARD:CODE:10.0.0.1:11183",
        "111PAIR:CARD:CODE",
        "111PAIR:CARD:CODE:::peer:udp",
    ] {
        let r = b.pair(json!({"bundle": bad, "role": "peer", "via": "tcp"}));
        r.problem(400, "bad_request");
    }
    // a bundle whose card is no card is 400, still with nothing recorded
    let forged = rewritten(&a.offer("peer", "tcp"), "203.0.113.9:11183", true).replacen(
        "111PAIR:",
        "111PAIR:AAAA",
        1,
    );
    b.pair(json!({"bundle": forged, "role": "peer", "via": "tcp"}))
        .problem(400, "bad_request");
    assert!(b.table().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn both_routes_need_the_hub_role_and_a_member_adds_nothing() {
    let m = node(false).await;
    let a = node(true).await;
    let r = m.post(
        "/api/v1/relay/pair/offer",
        json!({"role": "peer", "via": "tcp"}),
    );
    check_op("offerRelayPair", &r);
    r.problem(409, "needs_hub");
    let bundle = a.offer("peer", "tcp");
    for lan in [false, true] {
        m.pair(json!({"bundle": bundle, "role": "peer", "via": "tcp", "lan": lan}))
            .problem(409, "needs_hub");
    }
    // the warning case too: the hub role is asked before the warning
    m.pair(json!({"bundle": rewritten(&bundle, "203.0.113.7:11183", false), "role": "peer", "via": "tcp"}))
        .problem(409, "needs_hub");
    // never adds: no key minted, no half, no record, no dial
    assert!(!m.has_link_key());
    assert!(m.table().is_empty());
    assert!(m.records().await.is_empty());
    assert_eq!(m.fabric.tcp().unwrap().connection_count(), 0);
    // unpairing what was never paired is a no-op
    assert_eq!(m.unpair(&a.id).status, 204);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_pairing_routes_are_loopback_only_and_refuse_the_agent() {
    let a = node(true).await;
    let b = node(true).await;
    let token = auth::DaemonToken::mint().unwrap();
    a.api.insert_ext(token.clone());
    let bearer = format!("Bearer {}", token.as_str());
    let cookie = a.session();
    let bundle = b.offer("peer", "tcp");
    let far = format!("/api/v1/relay/paired/{}", hex(&b.id));
    let calls: [(&str, &str, Option<Value>); 3] = [
        (
            "POST",
            "/api/v1/relay/pair/offer",
            Some(json!({"role": "peer", "via": "tcp"})),
        ),
        (
            "POST",
            "/api/v1/relay/pair",
            Some(json!({"bundle": bundle, "role": "peer", "via": "tcp", "lan": true})),
        ),
        ("DELETE", &far, None),
    ];
    for (method, path, body) in &calls {
        // the agent's bearer from loopback: refused, nothing taken from it
        let r = http(
            a.port,
            method,
            path,
            &[("Authorization", &bearer)],
            body.as_ref(),
        );
        r.problem(401, "unauthenticated");
        // a paired LAN session: these are loopback only
        let r = http(a.lan, method, path, &[("Cookie", &cookie)], body.as_ref());
        r.problem(403, "loopback_only");
    }
    assert!(a.table().is_empty());
    assert!(!a.has_link_key());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_warning_round_trip_keeps_its_hash_with_the_pair() {
    let a = node(true).await;
    let b = node(true).await;
    // b's bundle as a build without TLS-RPK would show it, at a public (TEST-NET-3) address
    let addr: SocketAddr = "203.0.113.7:11183".parse().unwrap();
    let bundle = rewritten(&b.offer("peer", "tcp"), "203.0.113.7:11183", false);
    let body = json!({"bundle": bundle, "role": "peer", "via": "tcp"});
    let r = a.pair(body.clone());
    r.problem(409, "consent_required");
    let c = &r.json()["consent"];
    assert_schema("ConsentRequired", c);
    assert_eq!(c["action"], "pair_plain");
    assert_eq!(c["subject"], hex(&b.id).as_str());
    assert_eq!(c["warning_text"], PAIR_PLAIN_WARNING);
    assert_eq!(c["hash"], pair_plain_hash_hex().as_str());
    assert_eq!(c["addons"], json!([]));
    // refusing pairs nothing: no half, no record, no key
    assert!(a.table().is_empty());
    assert!(a.records().await.is_empty());
    assert!(!a.has_link_key());
    // a stale or wrong echo is asked again
    let mut wrong = body.clone();
    wrong["accept_warning"] = json!("00".repeat(32));
    a.pair(wrong).problem(409, "consent_required");
    assert!(a.table().is_empty());
    // the accepted text's hash goes with the pair
    let mut yes = body.clone();
    yes["accept_warning"] = json!(pair_plain_hash_hex());
    let r = a.pair(yes);
    assert_eq!(r.status, 202, "{}", r.body);
    let v = r.json();
    assert_eq!(v["node_id"], hex(&b.id).as_str());
    assert_eq!(v["tls"], "plain");
    assert_eq!(v["lan"], false);
    assert_eq!(
        (v["role"].as_str(), v["via"].as_str()),
        (Some("peer"), Some("tcp"))
    );
    let l = a.link(&b.id).expect("the half is recorded");
    assert_eq!(l.warning_hash, Some(pair_plain_hash()));
    assert_eq!(l.link_spki, None);
    assert_eq!(l.address, Some(addr));
    assert!(!l.lan);
    // §4.1 rule 4: plain to this non-private pair is allowed only because of the warning
    assert_eq!(
        tls::outbound_rule(&a.table(), addr, Some(b.id)),
        Outbound::Plain
    );
    let mut no_warning = l.clone();
    no_warning.warning_hash = None;
    assert_eq!(
        tls::outbound_rule(&[no_warning], addr, Some(b.id)),
        Outbound::Refused
    );
    // this hub minted its own link key for the pairing
    assert!(a.has_link_key());
    // unpair: the half and the attempt go
    assert_eq!(a.unpair(&b.id).status, 204);
    assert!(a.link(&b.id).is_none());
    assert!(a.records().await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lan_pair_on_a_test_net_address_runs_plain_with_no_consent() {
    let a = node(true).await;
    let b = node(true).await;
    let shown = b.offer("peer", "tcp");
    // the link key pinned, the owner's mark set, a non-private address: plain, no warning
    for (address, keep_key) in [
        ("198.51.100.7:11183", true),
        ("198.51.100.7:11183", false),
        ("[2001:db8::7]:11183", true),
    ] {
        let addr: SocketAddr = address.parse().unwrap();
        let r = a.pair(json!({
            "bundle": rewritten(&shown, address, keep_key),
            "role": "peer",
            "via": "tcp",
            "lan": true,
        }));
        assert_eq!(r.status, 202, "{address}: {}", r.body);
        let v = r.json();
        assert_eq!(v["tls"], "none");
        assert_eq!(v["lan"], true);
        let l = a.link(&b.id).unwrap();
        assert!(l.lan);
        assert_eq!(l.warning_hash, None, "no pair_plain consent");
        assert_eq!(l.address, Some(addr));
        assert_eq!(l.link_spki.is_some(), keep_key);
        assert_eq!(
            tls::outbound_rule(&a.table(), addr, Some(b.id)),
            Outbound::Plain
        );
        assert_eq!(a.unpair(&b.id).status, 204);
        assert!(a.link(&b.id).is_none());
    }
    // without the mark the same pinned bundle runs TLS (no warning either)
    let addr: SocketAddr = "198.51.100.7:11183".parse().unwrap();
    let r = a.pair(json!({"bundle": rewritten(&shown, "198.51.100.7:11183", true), "role": "peer", "via": "tcp"}));
    assert_eq!(r.status, 202, "{}", r.body);
    assert_eq!(r.json()["tls"], "rpk");
    let pin = PairBundle::parse(&shown).unwrap().link_spki.unwrap();
    assert_eq!(
        tls::outbound_rule(&a.table(), addr, Some(b.id)),
        Outbound::Tls(pin)
    );
    assert_eq!(a.unpair(&b.id).status, 204);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_hubs_pair_inside_tls_and_unpair() {
    let a = node(true).await;
    let b = node(true).await;
    let bundle_a = a.offer("leaf", "tcp");
    let bundle_b = b.offer("gateway", "tcp");
    let shown_a = PairBundle::parse(&bundle_a).unwrap();
    let shown_b = PairBundle::parse(&bundle_b).unwrap();
    // b scans first: its TLS dial is refused until a has pinned b (the dial retries)
    let r = b.pair(json!({"bundle": bundle_a, "role": "gateway", "via": "tcp"}));
    assert_eq!(r.status, 202, "{}", r.body);
    assert_eq!(
        r.json()["tls"],
        "rpk",
        "loopback is not private: TLS 1.3 RPK"
    );
    assert_eq!(r.json()["role"], "gateway");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        b.records().await.iter().all(|p| !p.ours),
        "b's ENROLL cannot land before a pins b"
    );
    let r = a.pair(json!({"bundle": bundle_b, "role": "leaf", "via": "tcp"}));
    assert_eq!(r.status, 202, "{}", r.body);
    assert_eq!(r.json()["tls"], "rpk");
    // both enrolments land; each side records the other whole, with its own role
    let whole = |recs: Vec<PairedHub>, far: NodeId, role: PairRole| {
        recs.iter()
            .any(|p| p.node_id == far && p.whole() && p.role == role && p.via == PairVia::Tcp)
    };
    until(30, "both pairs whole", || async {
        (whole(a.records().await, b.id, PairRole::Leaf)
            && whole(b.records().await, a.id, PairRole::Gateway))
        .then_some(())
    })
    .await;
    // each pins the other's link key and dials it inside TLS
    let la = a.link(&b.id).unwrap();
    let lb = b.link(&a.id).unwrap();
    assert_eq!(la.link_spki, shown_b.link_spki);
    assert_eq!(lb.link_spki, shown_a.link_spki);
    assert_eq!(
        tls::outbound_rule(&a.table(), shown_b.address.unwrap(), Some(b.id)),
        Outbound::Tls(shown_b.link_spki.unwrap())
    );
    assert_eq!(
        tls::outbound_rule(&b.table(), shown_a.address.unwrap(), Some(a.id)),
        Outbound::Tls(shown_a.link_spki.unwrap())
    );
    // the row reaches fabric/settings with the fabric's record (§4.6 JSON)
    let file = until(10, "the pair row on disk", || async {
        let text = std::fs::read_to_string(a.layout.root.join("fabric/settings")).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        let row = v["paired_hubs"]
            .as_array()?
            .iter()
            .find(|r| r["node_id"] == hex(&b.id).as_str())?
            .clone();
        (row["ours"] == true && row["theirs"] == true).then_some(row)
    })
    .await;
    assert_eq!(file["role"], "leaf");
    assert_eq!(file["via"], "tcp");
    assert_eq!(
        file["link_spki"],
        shown_b.link_spki.unwrap().text().as_str()
    );
    assert_eq!(file["lan"], false);
    assert_eq!(file["warning_hash"], Value::Null);
    // a whole pair is not paired again
    let fresh = b.offer("gateway", "tcp");
    a.pair(json!({"bundle": fresh, "role": "leaf", "via": "tcp"}))
        .problem(409, "bad_request");
    // unpair: the record, the half and its pin go; idempotent
    assert_eq!(a.unpair(&b.id).status, 204);
    assert!(a.link(&b.id).is_none());
    until(10, "a's record of b dropped", || async {
        a.records()
            .await
            .iter()
            .all(|p| p.node_id != b.id)
            .then_some(())
    })
    .await;
    assert_eq!(a.unpair(&b.id).status, 204);
    assert_eq!(b.unpair(&a.id).status, 204);
    assert!(b.link(&a.id).is_none());
}

/// §4.6 step 3, S22: one dialled address suffices. The hub that scans a bundle with no address
/// waits; the other hub's dial carries its `ENROLL`, and the first hub's `ENROLL` rides that
/// connection back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_second_enroll_rides_the_first_hubs_connection() {
    let a = node(true).await;
    let b = node(true).await;
    let bundle_a = a.offer("peer", "tcp");
    let mut shown_b = PairBundle::parse(&b.offer("peer", "tcp")).unwrap();
    shown_b.address = None;
    // a scans first: nothing to dial, so its attempt waits for b
    let r = a.pair(json!({"bundle": shown_b.text(), "role": "peer", "via": "tcp"}));
    assert_eq!(r.status, 202, "{}", r.body);
    let l = a.link(&b.id).unwrap();
    assert_eq!(l.address, None);
    assert_eq!(l.link_spki, shown_b.link_spki);
    assert!(!l.far_holds_address);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(a.records().await.is_empty());
    assert_eq!(a.fabric.tcp().unwrap().connection_count(), 0);
    // b scans a and dials it (inside TLS: a pinned b); a's ENROLL rides b's connection
    let r = b.pair(json!({"bundle": bundle_a, "role": "peer", "via": "tcp"}));
    assert_eq!(r.status, 202, "{}", r.body);
    let whole =
        |recs: Vec<PairedHub>, far: NodeId| recs.iter().any(|p| p.node_id == far && p.whole());
    until(30, "both pairs whole over one dialled address", || async {
        (whole(a.records().await, b.id) && whole(b.records().await, a.id)).then_some(())
    })
    .await;
    assert_eq!(a.unpair(&b.id).status, 204);
    assert_eq!(b.unpair(&a.id).status, 204);
}

/// An attempt the far hub never answers ends with its window: the scan route reports it, and
/// this hub's half (with its pin) is forgotten, since the fabric records nothing for that hub.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attempt_nobody_answers_ends_with_its_window_and_leaves_nothing() {
    use node111::daemon::relay::pair::{pair_within, PairAttempts, PairPick, PairRefusal};
    let a = node(true).await;
    let b = node(true).await;
    let bundle = rewritten(&b.offer("peer", "tcp"), "192.0.2.7:11183", true);
    let attempts = Arc::new(PairAttempts::default());
    let pick = PairPick {
        bundle,
        role: PairRole::Peer,
        via: PairVia::Tcp,
        lan: true,
        accept_warning: None,
    };
    let r = pair_within(
        &a.fabric,
        &attempts,
        &a.layout.root,
        pick,
        Duration::from_millis(300),
    )
    .await;
    match r {
        Err(PairRefusal::Problem(p)) => assert_eq!((p.status, p.code), (502, "upstream_error")),
        other => panic!("{other:?}"),
    }
    assert!(a.link(&b.id).is_none());
    assert!(a.records().await.is_empty());
    assert!(!attempts.in_flight(&b.id).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_cli_shows_and_pairs_and_asks_the_warning() {
    let a = node(true).await;
    let b = node(true).await;
    let (pa, pb) = (a.port, b.port);
    // `111 fabric pair --show` on b
    let shown = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        let cmd = fcli::parse(&["pair".into(), "--show".into()]).unwrap();
        fcli::run_pair_with(pb, &cmd, &mut out, &mut |_| panic!("no question here")).unwrap();
        String::from_utf8(out).unwrap()
    })
    .await
    .unwrap();
    assert!(shown.contains("Pairing code: "), "{shown}");
    let line = shown
        .lines()
        .find(|l| l.starts_with("On the other hub: 111 fabric pair '"))
        .expect("the line to run on the other hub");
    let bundle = line
        .trim_start_matches("On the other hub: 111 fabric pair '")
        .trim_end_matches('\'')
        .to_string();
    assert_eq!(PairBundle::parse(&bundle).unwrap().role, PairRole::Peer);
    // `111 fabric pair <bundle>` on a, for a link that cannot be encrypted: asked, "no"
    let plain = rewritten(&bundle, "203.0.113.7:11183", false);
    let p2 = plain.clone();
    let (out, asked) = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        let mut asked = Vec::new();
        let cmd = fcli::parse(&["pair".into(), p2]).unwrap();
        fcli::run_pair_with(pa, &cmd, &mut out, &mut |t| {
            asked.push(t.to_string());
            Ok(false)
        })
        .unwrap();
        (String::from_utf8(out).unwrap(), asked)
    })
    .await
    .unwrap();
    assert_eq!(asked, [PAIR_PLAIN_WARNING]);
    assert!(out.contains("Not paired."), "{out}");
    assert!(a.table().is_empty());
    // "yes": paired, the hash kept
    let p3 = plain.clone();
    let out = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        let cmd = fcli::parse(&["pair".into(), p3]).unwrap();
        fcli::run_pair_with(pa, &cmd, &mut out, &mut |_| Ok(true)).unwrap();
        String::from_utf8(out).unwrap()
    })
    .await
    .unwrap();
    assert!(out.contains("Pairing with hub "), "{out}");
    assert!(out.contains("link plain"), "{out}");
    assert_eq!(a.link(&b.id).unwrap().warning_hash, Some(pair_plain_hash()));
    assert_eq!(a.unpair(&b.id).status, 204);
    // `--lan`: no question, marked
    let out = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        let cmd = fcli::parse(&["pair".into(), plain, "--lan".into()]).unwrap();
        fcli::run_pair_with(pa, &cmd, &mut out, &mut |_| {
            panic!("no question for a lan pair")
        })
        .unwrap();
        String::from_utf8(out).unwrap()
    })
    .await
    .unwrap();
    assert!(out.contains("marked LAN or tunnel"), "{out}");
    let l = a.link(&b.id).unwrap();
    assert!(l.lan);
    assert_eq!(l.warning_hash, None);
    assert_eq!(a.unpair(&b.id).status, 204);
    // a refusal is reported with the daemon's words
    let e = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        let cmd = FabricCmd::PairShow {
            role: PairRole::Leaf,
            via: PairVia::Carrier,
        };
        fcli::run_pair_with(pa, &cmd, &mut out, &mut |_| Ok(false))
            .unwrap_err()
            .to_string()
    })
    .await
    .unwrap();
    assert!(e.contains("do not fit"), "{e}");
}

/// The lead's D6 note on row D5b: the TLS state is cached behind a stamp of the key file; a pin
/// added or removed must reach the next handshake. The server verifier reads the live pair table
/// on every handshake, so it does with no reload: the key is read once across all three.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pin_added_or_removed_reaches_the_next_handshake_with_the_tls_state_cached() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ka = *LinkKey::load_or_mint(da.path()).unwrap().spki();
    let kb = *LinkKey::load_or_mint(db.path()).unwrap().spki();
    let (id_a, id_b) = (NodeId::from([0xa1; 32]), NodeId::from([0xb2; 32]));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    // another pinned pair keeps the peek on while b's pin comes and goes
    let other = LinkSpki::from_der(&[&ED25519_SPKI_PREFIX[..], &[9u8; 32][..]].concat()).unwrap();
    let pin_b = PairLink {
        link_spki: Some(kb),
        ..PairLink::new(id_b)
    };
    let table_a: PairTable = Arc::new(RwLock::new(vec![
        PairLink {
            link_spki: Some(other),
            ..PairLink::new(NodeId::from([0xc3; 32]))
        },
        pin_b.clone(),
    ]));
    let table_b: PairTable = Arc::new(RwLock::new(vec![PairLink {
        address: Some(at),
        link_spki: Some(ka),
        ..PairLink::new(id_a)
    }]));
    let wa = RpkWrap::new(da.path().to_path_buf(), Arc::clone(&table_a));
    let wb = RpkWrap::new(db.path().to_path_buf(), table_b);
    let handshake = || async {
        let server = async {
            let (s, remote) = listener.accept().await?;
            wa.inbound(remote, s).await.map(|(_, who)| who)
        };
        let client = async {
            let s = tokio::net::TcpStream::connect(at).await?;
            wb.outbound(at, Some(id_a), s).await
        };
        let (sv, _cl) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(server, client)
        })
        .await
        .expect("the handshake ends");
        sv
    };
    assert_eq!(handshake().await.unwrap(), Some(id_b));
    assert_eq!(wa.key_loads(), 1);
    // b's pin removed: its next handshake is refused, the TLS state is not rebuilt
    table_a.write().unwrap().retain(|l| l.node_id != id_b);
    assert!(handshake().await.is_err());
    assert_eq!(wa.key_loads(), 1);
    // pinned again: admitted at once
    table_a.write().unwrap().push(pin_b);
    assert_eq!(handshake().await.unwrap(), Some(id_b));
    assert_eq!(wa.key_loads(), 1);
    assert!(wa.tls_built());
}
