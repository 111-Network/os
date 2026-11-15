//! Layer 7 row D10 acceptance (`os/7-relay/README.md` §6.3, §12, §13.1, §13.2, §2.4 item 2; contract
//! `getRelay`, `patchRelaySettings`, `checkRelayCarrier`, `flushRelayCarrier`, `decideProposal` for
//! `kind: relay`): nodes with the real identity handle and fabric on loopback sockets, driven
//! through the real dashboard app, every answer held to the contract digest.
//!
//! - `GET /relay` is `RelayStatus` with everything off, the `rns` heartbeat row from Layer 6
//!   (`owner: layer6`), the agent's bearer reads it, and a GET writes no file and starts no frame;
//! - the `carrier_enable` round trip (the §6.2 copy, its hash recorded with the write, a stale
//!   hash refused), the secrets write-only: never in a GET, a frame or a problem, only in
//!   `state/relay.json` (0600); off forgets the consent;
//! - every heartbeat write answers `409` (`no_such_transport`; `settings_locked` for `rns`, its
//!   detail naming `PATCH /api/v1/bridge/settings`) and writes nothing;
//! - `needs_hub`, `lan_off`, the `gateway_bit` and `origin_serve` copies verbatim, a switch turned
//!   off asks again; the role bits follow the write (row D9's hook);
//! - the lead's rulings: (a) the owed carrier consent accepted through `PATCH` with no other key,
//!   a stale hash refused with the current text, check and flush then `202`; (b) the own target
//!   stays; (c) a pair met at a private address reads `{tls: none, lan: false}`;
//! - a metered cap set or raised re-asks `carrier_enable` with the cap in the text and `metered`
//!   in `addons`, a lower cap does not;
//! - check and flush: `409 bad_request` with no carrier, `consent_required`, `metered_cap`,
//!   `licensed_board`, then the registered hook (no-op until row D11: `started: false`);
//! - the agent's bearer `401` on the settings, check and flush routes;
//! - `kind: relay` proposals: `presence.interval` refused `unsupported_action`, a body not of the
//!   shape `502`, `gateway.enable` through the settings route's consent round trip with
//!   `accept_warning` on the decision, `carrier.flush`, `hub.pair` and `needs_hub`;
//! - the `relay` frame: a full `RelayStatus` after a write, after `relay::changed()` and on a
//!   fabric event that moved the status, never for a GET or a write that changed nothing.
//!
//! The `gateway` row's facts and the frame's binding are process statics (one daemon per
//! process), so the tests here run one at a time (`SERIAL`).

use axum::extract::connect_info::MockConnectInfo;
use id111::keyset::NodeId;
use mesh111::fabric::{FabricEvent, PairedHub};
use node111::daemon::dashboard::api_v1::{auth, relay7};
use node111::daemon::dashboard::{
    routes, serve_listener, ApiConfig, ApiState, DashboardState, SharedState,
};
use node111::daemon::fabric::{self, FabricConfig, FabricHandle};
use node111::daemon::relay::pair::PairBundle;
use node111::daemon::relay::settings::{
    self as rs, CarrierKind, MeteredLink, MeteredMatch, RelaySettings,
};
use node111::daemon::relay::{self, consent, CarrierHook, NoopCarrier};
use node111::daemon::updater::fetch::TARGET_TRIPLE;
use node111::daemon::DaemonLayout;
use serde_json::{json, Map, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

/// One test at a time: the gateway row and the frame's binding are process statics.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const SECRET_TOKEN: &str = "tok-SECRET-7f3a91";
/// 32 bytes of 0x42 in RFC 4648 base32 (lowercase, unpadded).
const TOPIC_SECRET: &str = "ijbeeqscijbeeqscijbeeqscijbeeqscijbeeqscijbeeqscijba";
const SERVER: &str = "https://ntfy.example.org";

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
/// one of the operation's error statuses (401 is the global gate's, on every operation).
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

    /// The `consent` member of a `409 consent_required`.
    fn consent(&self) -> Value {
        self.problem(409, "consent_required");
        let c = self.json()["consent"].clone();
        assert_schema("ConsentRequired", &c);
        c
    }
}

/// One request to `127.0.0.1:port` (`Connection: close`, no chunked answers here). Blocking:
/// called on a blocking thread so the server's tasks keep running.
fn http_blocking(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(String, String)],
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

async fn http(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Resp {
    let (method, path) = (method.to_string(), path.to_string());
    let headers: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    tokio::task::spawn_blocking(move || {
        http_blocking(port, &method, &path, &headers, body.as_ref())
    })
    .await
    .unwrap()
}

// ---------------------------------------------------------------- nodes

const LAN_PEER: &str = "192.168.1.50:40000";

struct Node {
    api: Arc<ApiState>,
    fabric: Arc<FabricHandle>,
    /// Dashboard on a real loopback socket (trusted loopback).
    port: u16,
    /// The same app with every request from a LAN peer (no session).
    lan: u16,
    layout: DaemonLayout,
    id: NodeId,
    token: auth::DaemonToken,
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
    c.enroll_timeout = Duration::from_secs(1);
    c.housekeeping = Duration::from_millis(200);
    c
}

/// A node: `hub` (the hub role), `lan` (LAN sharing), `seed` writes files under `state/` before
/// the daemon starts. `relay::start` runs as the daemon runs it.
async fn node_with(hub: bool, lan: bool, seed: impl FnOnce(&Path)) -> Node {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    seed(&layout.state_dir());
    let identity = node111::identity::generate(&layout.identity_dir()).unwrap();
    let hex = identity.node_id_hex();
    let id = NodeId::from(id111::encoding::hex_decode(&hex).unwrap());
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.11.15")));
    dash.write().unwrap().node_id = Some(hex);
    let api = ApiState::new(dash, ApiConfig::new(layout.clone(), lan));
    let token = auth::DaemonToken::mint().unwrap();
    api.insert_ext(token.clone());
    let fabric = fabric::start(test_config(&layout, hub), Some(identity), &api).await;
    assert!(fabric.running(), "{:?}", fabric.down_reason());
    api.insert_ext(Arc::clone(&fabric));
    relay::start(&api, &fabric);
    let port = serve(&api, None).await;
    let lan_port = serve(&api, Some(LAN_PEER)).await;
    Node {
        api,
        fabric,
        port,
        lan: lan_port,
        layout,
        id,
        token,
        _dir: dir,
    }
}

async fn node(hub: bool) -> Node {
    node_with(hub, false, |_| {}).await
}

fn hex(id: &NodeId) -> String {
    id.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

impl Node {
    async fn get(&self, path: &str) -> Resp {
        http(self.port, "GET", path, &[], None).await
    }

    async fn status(&self) -> Value {
        let r = self.get("/api/v1/relay").await;
        check_op("getRelay", &r);
        assert_eq!(r.status, 200, "{}", r.body);
        let v = r.json();
        assert!(!r.body.contains(SECRET_TOKEN) && !r.body.contains(TOPIC_SECRET));
        v
    }

    async fn patch(&self, body: Value) -> Resp {
        let r = http(
            self.port,
            "PATCH",
            "/api/v1/relay/settings",
            &[],
            Some(body),
        )
        .await;
        check_op("patchRelaySettings", &r);
        assert!(
            !r.body.contains(SECRET_TOKEN) && !r.body.contains(TOPIC_SECRET),
            "a secret echoed: {}",
            r.body
        );
        r
    }

    async fn post(&self, path: &str, body: Value) -> Resp {
        http(self.port, "POST", path, &[], Some(body)).await
    }

    async fn carrier(&self, what: &str) -> Resp {
        let op = if what == "check" {
            "checkRelayCarrier"
        } else {
            "flushRelayCarrier"
        };
        let r = self
            .post(&format!("/api/v1/relay/carrier/{what}"), json!({}))
            .await;
        check_op(op, &r);
        r
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.token.as_str())
    }

    fn relay_file(&self) -> std::path::PathBuf {
        rs::path(&self.layout.state_dir())
    }

    /// A proposal file as `111-agent` writes it, stamped for this node.
    fn propose(&self, id: &str, body: &Value) {
        let dir = self.layout.root.join("agent/proposals");
        std::fs::create_dir_all(&dir).unwrap();
        let p = json!({
            "schema": 1, "id": id, "node_id": hex(&self.id), "task_id": null, "kind": "relay",
            "title": "Relay change", "body": body.to_string(),
            "created_at": "2026-11-15T09:00:00Z",
        });
        std::fs::write(dir.join(format!("{id}.json")), p.to_string()).unwrap();
    }

    async fn decide(&self, id: &str, accept: Option<&str>) -> Resp {
        let mut body = json!({"decision": "accepted"});
        if let Some(h) = accept {
            body["accept_warning"] = json!(h);
        }
        let r = self
            .post(&format!("/api/v1/proposals/{id}/decision"), body)
            .await;
        check_op("decideProposal", &r);
        r
    }

    async fn decision_of(&self, id: &str) -> Value {
        let r = self.get(&format!("/api/v1/proposals/{id}")).await;
        assert_eq!(r.status, 200, "{}", r.body);
        r.json()["decision"].clone()
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

    /// A hook of the test's own in place of the registered one.
    fn hook(&self, h: Arc<dyn CarrierHook>) {
        self.api.insert_ext(h);
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

/// The `relay` frames published on `api`'s bus from now on.
struct Frames(
    tokio::sync::broadcast::Receiver<Arc<node111::daemon::dashboard::api_v1::events::Frame>>,
);

impl Frames {
    fn on(api: &ApiState) -> Frames {
        Frames(api.events.subscribe(Some(api.events.head())).rx)
    }

    /// The next `relay` frame within `ms`, validated as `RelayStatus`.
    async fn next(&mut self, ms: u64) -> Option<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
        loop {
            let f = tokio::time::timeout_at(deadline, self.0.recv())
                .await
                .ok()?
                .ok()?;
            if f.event == "relay" {
                assert!(
                    !f.data.contains(SECRET_TOKEN) && !f.data.contains(TOPIC_SECRET),
                    "a secret in a frame"
                );
                let v: Value = serde_json::from_str(&f.data).unwrap();
                assert_schema("RelayStatus", &v);
                return Some(v);
            }
        }
    }
}

/// The hook a test registers to drive the budget and the board.
struct Budget {
    cap: bool,
    licensed: bool,
}

impl CarrierHook for Budget {
    fn reconfigure(&self, _: &RelaySettings) {}
    fn check(&self) -> bool {
        true
    }
    fn flush(&self) -> bool {
        true
    }
    fn metered_cap_reached(&self) -> bool {
        self.cap
    }
    fn licensed_board(&self) -> bool {
        self.licensed
    }
}

/// A metered Wi-Fi link `ssid` with `cap`.
fn link(ssid: &str, cap: u64) -> MeteredLink {
    MeteredLink {
        matcher: MeteredMatch::Ssid(ssid.to_string()),
        daily_cap_bytes: cap,
    }
}

/// Turn the ntfy carrier on through its consent round trip; the hash recorded.
async fn carrier_on(n: &Node) -> String {
    let body = json!({"carrier": {"kind": "ntfy", "server": SERVER, "mode": "5A"}});
    let c = n.patch(body.clone()).await.consent();
    let mut with = body;
    with["accept_warning"] = c["hash"].clone();
    let r = n.patch(with).await;
    assert_eq!(r.status, 200, "{}", r.body);
    c["hash"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------- tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_status_is_every_switch_off_and_the_agent_reads_it() {
    let _one = SERIAL.lock().await;
    let n = node(false).await;
    let mut frames = Frames::on(&n.api);
    let v = n.status().await;
    assert_eq!(v["schema"], 1);
    let g = &v["gateway"];
    assert_eq!(g["enabled"], false);
    assert_eq!(g["state"], "gateway_none");
    assert_eq!(g["carrier"], Value::Null);
    assert_eq!(g["mode"], Value::Null);
    assert_eq!(g["pending"], 0);
    assert_eq!(g["metered_cap_reached"], false);
    assert_eq!(g["termux_poll_min"], 15);
    assert_eq!(
        v["carrier"],
        json!({"kind": "none", "server": null, "mode": "5A", "token": {"set": false},
               "topic_secret": {"set": false}, "consent_hash": null})
    );
    assert_eq!(v["paired_hubs"], json!([]));
    assert_eq!(v["forward"], json!({"refused": 0, "too_large": 0}));
    // §2.4 item 2: the three Layer 7 transports at 0, the rns row Layer 6's
    assert_eq!(
        v["presence"]["heartbeat"],
        json!([
            {"transport": "tcp-lan", "interval_min": 0, "owner": "layer7"},
            {"transport": "kiss-modem", "interval_min": 0, "owner": "layer7"},
            {"transport": "meshtastic-client", "interval_min": 0, "owner": "layer7"},
            {"transport": "rns", "interval_min": 60, "owner": "layer6"},
        ])
    );
    assert_eq!(
        v["discovery"],
        json!({"lan_record": false, "public_gateway_bit": false, "offload_bit": false})
    );
    assert_eq!(
        v["origin"],
        json!({"serve": false, "targets": [TARGET_TRIPLE], "bytes": 0})
    );
    assert_eq!(v["updates"], json!({"check_internet": true}));
    assert_eq!(v["metered"], json!([]));
    // the agent's bearer reads it
    let auth_h = n.bearer();
    let r = http(
        n.port,
        "GET",
        "/api/v1/relay",
        &[("Authorization", &auth_h)],
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(!r.body.contains(n.token.as_str()));
    // a LAN peer without a session is refused by the gate
    http(n.lan, "GET", "/api/v1/relay", &[], None)
        .await
        .problem(401, "unauthenticated");
    // never adds: a file write or a frame on a GET
    assert!(!n.relay_file().exists(), "a GET writes no file");
    assert!(frames.next(400).await.is_none(), "a GET publishes nothing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_carrier_consent_round_trip_keeps_the_secrets_write_only() {
    let _one = SERIAL.lock().await;
    let n = node(true).await;
    let mut frames = Frames::on(&n.api);
    let body = json!({"carrier": {"kind": "ntfy", "server": SERVER, "mode": "5A",
                                  "token": SECRET_TOKEN, "topic_secret": TOPIC_SECRET}});
    let r = n.patch(body.clone()).await;
    let c = r.consent();
    assert_eq!(c["action"], "carrier_enable");
    assert_eq!(c["subject"], format!("ntfy:{SERVER}").as_str());
    let text = consent::carrier_enable_text(SERVER, &[]);
    assert_eq!(c["warning_text"], text.as_str(), "the §6.2 copy");
    assert_eq!(c["hash"], consent::hash(&text).as_str());
    assert_eq!(c["addons"], json!([]));
    assert!(
        !n.relay_file().exists(),
        "nothing written before the consent"
    );
    // a stale hash is refused with the current text
    let mut stale = body.clone();
    stale["accept_warning"] = json!("ab".repeat(32));
    assert_eq!(n.patch(stale).await.consent()["hash"], c["hash"]);
    // the repeat with the hash: written, the hash recorded
    let mut with = body.clone();
    with["accept_warning"] = c["hash"].clone();
    let r = n.patch(with).await;
    assert_eq!(r.status, 200, "{}", r.body);
    let v = r.json();
    assert_eq!(
        v["carrier"],
        json!({"kind": "ntfy", "server": SERVER, "mode": "5A", "token": {"set": true},
               "topic_secret": {"set": true}, "consent_hash": c["hash"]})
    );
    // the frame: a full RelayStatus, no secret (Frames::next checks)
    let f = frames
        .next(3000)
        .await
        .expect("a relay frame after the write");
    assert_eq!(f["carrier"]["consent_hash"], c["hash"]);
    // the file holds them, 0600; nothing else does
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(n.relay_file()).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let file = std::fs::read_to_string(n.relay_file()).unwrap();
    assert!(file.contains(SECRET_TOKEN));
    let held = rs::load(&n.layout.state_dir()).settings;
    assert_eq!(held.ntfy_token().unwrap().expose(), SECRET_TOKEN);
    assert_eq!(held.carrier_kind(), CarrierKind::Ntfy);
    assert_eq!(n.status().await["carrier"]["token"]["set"], true);
    // the same carrier again asks nothing; a mode change asks nothing
    let r = n
        .patch(json!({"carrier": {"kind": "ntfy", "server": SERVER, "mode": "5B"}}))
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["carrier"]["mode"], "5B");
    // null clears a secret
    let r = n.patch(json!({"carrier": {"token": null}})).await;
    assert_eq!(r.json()["carrier"]["token"]["set"], false);
    assert_eq!(r.json()["carrier"]["topic_secret"]["set"], true);
    // off forgets the consent: on again shows the copy again
    let r = n.patch(json!({"carrier": {"kind": "none"}})).await;
    assert_eq!(r.json()["carrier"]["consent_hash"], Value::Null);
    let c2 = n
        .patch(json!({"carrier": {"kind": "ntfy"}}))
        .await
        .consent();
    assert_eq!(c2["hash"], c["hash"]);
    // a body that is not the patch's shape: 400, no value echoed
    let r = n
        .patch(json!({"carrier": {"token": SECRET_TOKEN, "ntfy": {"token": SECRET_TOKEN}}}))
        .await;
    r.problem(400, "bad_request");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_heartbeat_write_is_refused_and_nothing_is_written() {
    let _one = SERIAL.lock().await;
    let n = node(true).await;
    for t in ["tcp-lan", "kiss-modem", "meshtastic-client"] {
        for m in [0, 9, 60] {
            n.patch(json!({"presence": {"heartbeat": [{"transport": t, "interval_min": m}]}}))
                .await
                .problem(409, "no_such_transport");
        }
    }
    let r = n
        .patch(json!({"presence": {"heartbeat": [{"transport": "rns", "interval_min": 30}]}}))
        .await;
    r.problem(409, "settings_locked");
    let detail = r.json()["detail"].as_str().unwrap().to_string();
    assert!(detail.contains("PATCH /api/v1/bridge/settings"), "{detail}");
    // a heartbeat beside other keys: refused, and the other keys are not written either
    n.patch(json!({"discovery": {"offload_bit": true},
                   "presence": {"heartbeat": [{"transport": "tcp-lan", "interval_min": 5}]}}))
        .await
        .problem(409, "no_such_transport");
    assert!(!n.relay_file().exists(), "nothing written");
    assert_eq!(n.status().await["discovery"]["offload_bit"], false);
    // an empty list writes nothing and is not a heartbeat write
    let r = n.patch(json!({"presence": {"heartbeat": []}})).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(!n.relay_file().exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hub_only_switches_lan_sharing_and_their_consents() {
    let _one = SERIAL.lock().await;
    // a member: needs_hub, nothing written
    let m = node_with(false, true, |_| {}).await;
    m.patch(json!({"gateway": {"enabled": true}}))
        .await
        .problem(409, "needs_hub");
    m.patch(json!({"origin": {"serve": true}}))
        .await
        .problem(409, "needs_hub");
    assert!(!m.relay_file().exists());
    // a hub without LAN sharing: lan_off
    let h = node_with(true, false, |_| {}).await;
    h.patch(json!({"origin": {"serve": true}}))
        .await
        .problem(409, "lan_off");
    // a hub with LAN sharing: the §12 copies, verbatim
    let n = node_with(true, true, |_| {}).await;
    let c = n.patch(json!({"origin": {"serve": true}})).await.consent();
    assert_eq!(c["action"], "origin_serve");
    assert_eq!(
        c["warning_text"],
        "Any device on this network can download this hub's verified update files without \
         signing in. They are public files signed by the 111 release key; nothing about you is \
         in them."
    );
    assert_eq!(
        c["hash"],
        consent::hash(consent::ORIGIN_SERVE_TEXT).as_str()
    );
    let r = n
        .patch(json!({"origin": {"serve": true}, "accept_warning": c["hash"]}))
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["origin"]["serve"], true);
    // row D9's hook ran: the switch reaches the fabric's role bits and the origin's gate
    assert!(n.fabric.relay_roles().origin_serve);
    let c = n
        .patch(json!({"discovery": {"public_gateway_bit": true}}))
        .await
        .consent();
    assert_eq!(c["action"], "gateway_bit");
    assert_eq!(
        c["warning_text"],
        "This adds 'willing to act as a gateway' to this node's signed card. Everyone who \
         receives the card, on any network, learns that this node has internet. Your hub's \
         members learn it without this."
    );
    let r = n
        .patch(json!({"discovery": {"public_gateway_bit": true}, "accept_warning": c["hash"]}))
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["discovery"]["public_gateway_bit"], true);
    // the gateway switch itself takes no consent on a hub
    let r = n.patch(json!({"gateway": {"enabled": true}})).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["gateway"]["enabled"], true);
    // off forgets: on again asks again
    let r = n.patch(json!({"origin": {"serve": false}})).await;
    assert_eq!(r.json()["origin"]["serve"], false);
    assert!(!n.fabric.relay_roles().origin_serve);
    n.patch(json!({"origin": {"serve": true}})).await.consent();
    // two gates in one write converge, one per round
    let both = json!({"discovery": {"public_gateway_bit": false}});
    assert_eq!(n.patch(both).await.status, 200);
    let body = json!({"discovery": {"public_gateway_bit": true}, "origin": {"serve": true}});
    let c1 = n.patch(body.clone()).await.consent();
    assert_eq!(c1["action"], "gateway_bit");
    let mut r1 = body.clone();
    r1["accept_warning"] = c1["hash"].clone();
    let c2 = n.patch(r1).await.consent();
    assert_eq!(c2["action"], "origin_serve");
    let mut r2 = body;
    r2["accept_warning"] = c2["hash"].clone();
    let r = n.patch(r2).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["discovery"]["public_gateway_bit"], true);
    assert_eq!(r.json()["origin"]["serve"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_own_target_stays_and_the_list_is_checked() {
    let _one = SERIAL.lock().await;
    let n = node(true).await;
    let other = if TARGET_TRIPLE == "aarch64-unknown-linux-musl" {
        "x86_64-unknown-linux-musl"
    } else {
        "aarch64-unknown-linux-musl"
    };
    let r = n.patch(json!({"origin": {"targets": [other]}})).await;
    r.problem(409, "bad_request");
    let d = r.json()["detail"].as_str().unwrap().to_string();
    assert!(d.contains(TARGET_TRIPLE), "{d}");
    n.patch(json!({"origin": {"targets": [TARGET_TRIPLE, "Not A Triple"]}}))
        .await
        .problem(400, "bad_request");
    let r = n
        .patch(json!({"origin": {"targets": [other, TARGET_TRIPLE]}}))
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["origin"]["targets"], json!([TARGET_TRIPLE, other]));
    assert_eq!(
        rs::load(&n.layout.state_dir()).settings.origin_targets(),
        [TARGET_TRIPLE.to_string(), other.to_string()]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_metered_cap_set_or_raised_asks_the_carrier_consent_again() {
    let _one = SERIAL.lock().await;
    let n = node(true).await;
    let first = carrier_on(&n).await;
    let cap = |bytes: u64| json!({"metered": [{"match": {"ssid": "Phone hotspot"}, "daily_cap_bytes": bytes}]});
    let c = n.patch(cap(5_000_000)).await.consent();
    assert_eq!(c["action"], "carrier_enable");
    assert_eq!(c["addons"], json!(["metered"]));
    let text = c["warning_text"].as_str().unwrap();
    assert_eq!(
        text,
        consent::carrier_enable_text(SERVER, &[link("Phone hotspot", 5_000_000)])
    );
    assert!(text.contains("5000000 bytes"), "{text}");
    assert_eq!(n.status().await["metered"], json!([]), "nothing written");
    let mut with = cap(5_000_000);
    with["accept_warning"] = c["hash"].clone();
    let r = n.patch(with).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["carrier"]["consent_hash"], c["hash"]);
    assert_eq!(r.json()["metered"], cap(5_000_000)["metered"]);
    // raised: asked again
    let c3 = n.patch(cap(6_000_000)).await.consent();
    assert_ne!(c3["hash"], c["hash"]);
    // lowered: no question; the recorded hash follows the new text
    let r = n.patch(cap(1_000)).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(
        r.json()["carrier"]["consent_hash"],
        consent::hash(&consent::carrier_enable_text(
            SERVER,
            &[link("Phone hotspot", 1_000)]
        ))
        .as_str()
    );
    // removed: the text without the cap is the first one again
    let r = n.patch(json!({"metered": []})).await;
    assert_eq!(r.json()["carrier"]["consent_hash"], first.as_str());
    // a carrier still consented: check and flush pass the gate
    assert_eq!(n.carrier("check").await.status, 202);
}

/// Review finding 2: with two metered links, adding the second (its cap under the first's),
/// or raising the cap that is not the largest, asks `carrier_enable` again; lowering one does
/// not, and the recorded hash follows the new text.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_metered_link_is_in_the_carrier_text() {
    let _one = SERIAL.lock().await;
    let n = node(true).await;
    carrier_on(&n).await;
    let caps = |links: &[(&str, u64)]| {
        let v: Vec<Value> = links
            .iter()
            .map(|(m, b)| json!({"match": {"ssid": m}, "daily_cap_bytes": b}))
            .collect();
        json!({ "metered": v })
    };
    let accept = |body: Value, c: &Value| {
        let mut with = body;
        with["accept_warning"] = c["hash"].clone();
        with
    };
    let one = caps(&[("Home", 5_000_000)]);
    let c = n.patch(one.clone()).await.consent();
    assert_eq!(n.patch(accept(one, &c)).await.status, 200);
    // a second capped link, its cap under the largest: asked
    let two = caps(&[("Home", 5_000_000), ("Phone", 1_000)]);
    let r = n.patch(two.clone()).await;
    assert_eq!(r.status, 409, "{}", r.body);
    let c2 = r.consent();
    assert_eq!(
        c2["warning_text"].as_str().unwrap(),
        consent::carrier_enable_text(SERVER, &[link("Home", 5_000_000), link("Phone", 1_000)])
    );
    assert_eq!(
        n.status().await["metered"],
        c_metered(&[("Home", 5_000_000)])
    );
    let r = n.patch(accept(two, &c2)).await;
    assert_eq!(r.status, 200, "{}", r.body);
    // the cap that is not the largest, raised: asked
    let raised = caps(&[("Home", 5_000_000), ("Phone", 2_000)]);
    let r = n.patch(raised).await;
    assert_eq!(r.status, 409, "{}", r.body);
    assert_eq!(r.json()["code"], "consent_required");
    // one lowered: no question, the hash follows
    let r = n
        .patch(caps(&[("Home", 4_000_000), ("Phone", 1_000)]))
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(
        r.json()["carrier"]["consent_hash"],
        consent::hash(&consent::carrier_enable_text(
            SERVER,
            &[link("Home", 4_000_000), link("Phone", 1_000)]
        ))
        .as_str()
    );
    assert_eq!(n.carrier("check").await.status, 202);
}

/// The `metered` list the status shows for `links` (Wi-Fi matches).
fn c_metered(links: &[(&str, u64)]) -> Value {
    json!(links
        .iter()
        .map(|(m, b)| json!({"match": {"ssid": m}, "daily_cap_bytes": b}))
        .collect::<Vec<_>>())
}

/// Make the next announce counter commit fail (a directory where its temp file goes, EISDIR,
/// also for root; row D4b's own test does the same); the returned guard's path is removed to
/// fix the disk.
fn block_counters(n: &Node) -> std::path::PathBuf {
    let blocker = n
        .layout
        .identity_dir()
        .join(format!("{}.tmp", id111::counters::COUNTERS_FILE));
    std::fs::create_dir(&blocker).unwrap();
    std::fs::write(blocker.join("keep"), b"x").unwrap();
    blocker
}

/// Review finding 1: a role-bit re-mint that fails answers 500 "saved … Save again to retry"
/// and the fabric's switches go back (row D4b); the retry with the same body (the file already
/// holds it) announces them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_announce_is_retried_by_the_same_write() {
    let _one = SERIAL.lock().await;
    let n = node(true).await;
    let body = json!({"discovery": {"offload_bit": true}});
    let blocker = block_counters(&n);
    let r = n.patch(body.clone()).await;
    assert_eq!(r.status, 500, "{}", r.body);
    assert!(r.body.contains("Save again to retry"), "{}", r.body);
    assert_eq!(n.status().await["discovery"]["offload_bit"], true, "saved");
    assert!(!n.fabric.relay_roles().offload_bit, "the switch put back");
    // still failing: the same write answers 500 again (not a silent 200)
    let r = n.patch(body.clone()).await;
    assert_eq!(r.status, 500, "{}", r.body);
    // the disk fixed: the same body announces
    std::fs::remove_dir_all(&blocker).unwrap();
    let mut frames = Frames::on(&n.api);
    let r = n.patch(body.clone()).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(n.fabric.relay_roles().offload_bit, "announced on the retry");
    assert!(
        frames.next(3000).await.is_some(),
        "the retry publishes a frame"
    );
    // and once only
    let r = n.patch(body).await;
    assert_eq!(r.status, 200, "{}", r.body);
}

/// Review finding 1, the relay-proposal path: `gateway.enable` whose re-mint fails reopens the
/// proposal; deciding it again (the file already holds the gateway on) announces bit 3 and the
/// offer's gateway fact.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_announce_is_retried_by_deciding_the_proposal_again() {
    let _one = SERIAL.lock().await;
    let n = node(true).await;
    carrier_on(&n).await;
    // the public gateway bit consented while the gateway is off: no bit yet
    let gb = json!({"discovery": {"public_gateway_bit": true}});
    let c = n.patch(gb.clone()).await.consent();
    let mut with = gb;
    with["accept_warning"] = c["hash"].clone();
    assert_eq!(n.patch(with).await.status, 200);
    assert!(n.fabric.relay_roles().gateway_bit);
    assert!(n.fabric.gateway_offer().is_none());
    let body = json!({"why": "the agent's reason",
                      "actions": [{"action": "gateway.enable", "carrier": "ntfy", "mode": "5A"}]});
    n.propose("p-retry", &body);
    let blocker = block_counters(&n);
    let r = n.decide("p-retry", None).await;
    assert_eq!(r.status, 500, "{}", r.body);
    assert_eq!(n.decision_of("p-retry").await, Value::Null, "reopened");
    assert_eq!(n.status().await["gateway"]["enabled"], true, "saved");
    assert!(n.fabric.gateway_offer().is_none(), "the switches put back");
    std::fs::remove_dir_all(&blocker).unwrap();
    let r = n.decide("p-retry", None).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["decision"]["decision"], "accepted");
    assert!(n.fabric.gateway_offer().is_some(), "announced on the retry");
}

/// Review finding 3(b): a `gateway_bit` consent recorded under another text (a copy an update
/// changed) keeps bit 3 off (D9 reads only a consent that holds) and is asked on the next write
/// to `discovery`; accepting it there turns the bit on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stale_switch_consent_is_off_and_asked_again() {
    let _one = SERIAL.lock().await;
    let n = node_with(true, true, |state| {
        rs::update(state, |s: &mut RelaySettings| {
            s.set_public_gateway_bit(true);
            s.set_origin_serve(true);
            s.set_consent_hash(rs::ConsentKind::GatewayBit, Some(&"ab".repeat(32)))?;
            s.set_consent_hash(rs::ConsentKind::OriginServe, Some(&"ab".repeat(32)))
        })
        .unwrap();
    })
    .await;
    let roles = n.fabric.relay_roles();
    assert!(!roles.gateway_bit && !roles.origin_serve, "stale: off");
    // a write elsewhere passes and asks nothing
    let r = n.patch(json!({"updates": {"check_internet": false}})).await;
    assert_eq!(r.status, 200, "{}", r.body);
    // the discovery section: gateway_bit asked
    let c = n
        .patch(json!({"discovery": {"lan_record": false}}))
        .await
        .consent();
    assert_eq!(c["action"], "gateway_bit");
    // the origin section: origin_serve asked, accepted: bit 5 follows
    let body = json!({"origin": {"targets": [TARGET_TRIPLE]}});
    let c = n.patch(body.clone()).await.consent();
    assert_eq!(c["action"], "origin_serve");
    let mut with = body;
    with["accept_warning"] = c["hash"].clone();
    let r = n.patch(with).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(n.fabric.relay_roles().origin_serve);
}

/// Re-check finding 1: a refused round that records the `gateway_bit` consent for a switch
/// already on (its old consent stale) turns the fabric's bit 3 switch on at once, though the
/// round answers the next gate's `409`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_consent_recorded_by_a_refused_round_turns_its_bit_on() {
    let _one = SERIAL.lock().await;
    let n = node_with(true, true, |state| {
        rs::update(state, |s: &mut RelaySettings| {
            s.set_public_gateway_bit(true);
            s.set_consent_hash(rs::ConsentKind::GatewayBit, Some(&"ab".repeat(32)))
        })
        .unwrap();
    })
    .await;
    assert!(!n.fabric.relay_roles().gateway_bit, "stale: off");
    let gb = consent::hash(consent::GATEWAY_BIT_TEXT);
    let c = n
        .patch(
            json!({"discovery": {"lan_record": false}, "origin": {"serve": true},
                      "accept_warning": gb}),
        )
        .await
        .consent();
    assert_eq!(c["action"], "origin_serve", "the next gate answers");
    let held = rs::load(&n.layout.state_dir()).settings;
    assert!(
        consent::holds(&held, rs::ConsentKind::GatewayBit),
        "recorded"
    );
    assert!(!held.origin_serve(), "nothing else written");
    assert!(
        n.fabric.relay_roles().gateway_bit,
        "the fabric follows the recorded consent"
    );
}

/// Re-check finding 3: a write cut short after `state/relay.json` (the file and the cell hold
/// the gateway on, the gateway row was never configured) is reconciled by the same write again:
/// on an unchanged file every follower runs, here the gateway row (`gateway::apply`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_write_after_one_cut_short_runs_every_follower() {
    let _one = SERIAL.lock().await;
    let n = node(true).await;
    carrier_on(&n).await;
    assert_eq!(n.status().await["gateway"]["state"], "gateway_off");
    // the write's file and cell, and nothing after them
    let (after, changed) = rs::update(&n.layout.state_dir(), |s: &mut RelaySettings| {
        s.set_gateway_enabled(true);
        Ok::<(), rs::SettingsError>(())
    })
    .unwrap();
    assert!(changed);
    relay7::cell(&n.api).replace(after);
    let st = n.status().await;
    assert_eq!(st["gateway"]["enabled"], true);
    assert_eq!(st["gateway"]["state"], "gateway_off", "cut short");
    // the same write again: the file is unchanged, the followers run
    let r = n.patch(json!({"gateway": {"enabled": true}})).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_ne!(
        r.json()["gateway"]["state"],
        "gateway_off",
        "the gateway row follows the file"
    );
    assert!(n.fabric.gateway_offer().is_some(), "and the offer");
}

/// Re-check finding 2: a frame build begun before a write never publishes after that write's
/// frame, even forced (the write bumps the generation the build read: `relay7::build`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_build_begun_before_a_write_never_publishes_after_its_frame() {
    let _one = SERIAL.lock().await;
    let n = node(true).await;
    let mut frames = Frames::on(&n.api);
    let stale = relay7::build(&n.api).await;
    let r = n.patch(json!({"updates": {"check_internet": false}})).await;
    assert_eq!(r.status, 200, "{}", r.body);
    let f = frames.next(3000).await.expect("the write's frame");
    assert_eq!(f["updates"]["check_internet"], false);
    relay7::emit(&n.api, stale, true);
    assert!(
        frames.next(600).await.is_none(),
        "the stale build is not published"
    );
    // a build begun after the write still publishes
    let fresh = relay7::build(&n.api).await;
    relay7::emit(&n.api, fresh, true);
    let f = frames.next(3000).await.expect("a fresh build");
    assert_eq!(f["updates"]["check_internet"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_and_flush_refuse_in_order_and_ruling_a_accepts_through_patch() {
    let _one = SERIAL.lock().await;
    // no carrier: 409 bad_request
    let bare = node(true).await;
    bare.carrier("check").await.problem(409, "bad_request");
    bare.carrier("flush").await.problem(409, "bad_request");
    // a carrier held without its consent (a file written by hand, say)
    let n = node_with(true, false, |state| {
        let mut s = RelaySettings::default();
        s.set_carrier_kind(CarrierKind::Ntfy);
        s.set_ntfy_server(Some(SERVER)).unwrap();
        rs::save(state, &s).unwrap();
    })
    .await;
    let noop = Arc::new(NoopCarrier::default());
    n.hook(noop.clone());
    let owed = consent::carrier_enable("ntfy", SERVER, &[]);
    for what in ["check", "flush"] {
        let c = n.carrier(what).await.consent();
        assert_eq!(c["action"], "carrier_enable");
        assert_eq!(c["hash"], owed.hash.as_str());
        assert_eq!(c["warning_text"], owed.warning_text.as_str());
    }
    assert_eq!((noop.checks(), noop.flushes()), (0, 0));
    // the agent's bearer: 401 on both, the token never echoed
    let auth_h = n.bearer();
    for what in ["check", "flush"] {
        let r = http(
            n.port,
            "POST",
            &format!("/api/v1/relay/carrier/{what}"),
            &[("Authorization", &auth_h)],
            Some(json!({})),
        )
        .await;
        r.problem(401, "unauthenticated");
        assert!(!r.body.contains(n.token.as_str()));
    }
    // ruling (a): a stale hash alone is refused with the current text ...
    let stale = consent::carrier_enable("ntfy", "https://old.example", &[]).hash;
    let c = n.patch(json!({"accept_warning": stale})).await.consent();
    assert_eq!(c["hash"], owed.hash.as_str());
    assert_eq!(n.status().await["carrier"]["consent_hash"], Value::Null);
    // ... and the owed hash alone records it, though no stored value changes
    let r = n.patch(json!({"accept_warning": owed.hash})).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["carrier"]["consent_hash"], owed.hash.as_str());
    // then both answer 202 and reach the hook (no-op until D11: started false)
    let r = n.carrier("check").await;
    assert_eq!(r.status, 202, "{}", r.body);
    assert_eq!(r.json(), json!({"schema": 1, "started": false}));
    let r = n.carrier("flush").await;
    assert_eq!(r.status, 202, "{}", r.body);
    assert_eq!((noop.checks(), noop.flushes()), (1, 1));
    // the budget and the board, after the consent
    n.hook(Arc::new(Budget {
        cap: true,
        licensed: true,
    }));
    n.carrier("flush").await.problem(409, "metered_cap");
    assert_eq!(n.status().await["gateway"]["metered_cap_reached"], true);
    n.hook(Arc::new(Budget {
        cap: false,
        licensed: true,
    }));
    n.carrier("check").await.problem(409, "licensed_board");
    n.hook(Arc::new(Budget {
        cap: false,
        licensed: false,
    }));
    let r = n.carrier("check").await;
    assert_eq!(r.json(), json!({"schema": 1, "started": true}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_agent_bearer_is_refused_on_the_settings_check_and_flush_routes() {
    let _one = SERIAL.lock().await;
    let n = node(true).await;
    let auth_h = n.bearer();
    let session = {
        let code = n.api.auth.pair_code();
        let s = n
            .api
            .auth
            .pair(LAN_PEER.parse::<SocketAddr>().unwrap().ip(), &code)
            .unwrap();
        format!("s111={}", s.token)
    };
    for (method, path, body) in [
        (
            "PATCH",
            "/api/v1/relay/settings",
            json!({"discovery": {"offload_bit": true}}),
        ),
        ("POST", "/api/v1/relay/carrier/check", json!({})),
        ("POST", "/api/v1/relay/carrier/flush", json!({})),
    ] {
        let r = http(
            n.port,
            method,
            path,
            &[("Authorization", &auth_h)],
            Some(body.clone()),
        )
        .await;
        r.problem(401, "unauthenticated");
        assert!(!r.body.contains(n.token.as_str()));
        // beside a valid session cookie too
        http(
            n.lan,
            method,
            path,
            &[("Authorization", &auth_h), ("Cookie", &session)],
            Some(body.clone()),
        )
        .await
        .problem(401, "unauthenticated");
        // a person on a paired session passes the gate
        let r = http(n.lan, method, path, &[("Cookie", &session)], Some(body)).await;
        assert_ne!(r.status, 401, "{path}: {}", r.body);
    }
    assert_eq!(
        n.status().await["discovery"]["offload_bit"],
        true,
        "the paired write"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_proposals_run_the_routes_paths() {
    let _one = SERIAL.lock().await;
    let n = node(true).await;
    let noop = Arc::new(NoopCarrier::default());
    n.hook(noop.clone());
    let body = |actions: Value| json!({"why": "the agent's reason", "actions": actions});
    // presence.interval: unsupported_action before any transport check; the proposal stays open
    n.propose(
        "p-interval",
        &body(json!([{"action": "presence.interval", "transport": "tcp-lan", "minutes": 30}])),
    );
    n.decide("p-interval", None)
        .await
        .problem(409, "unsupported_action");
    assert_eq!(n.decision_of("p-interval").await, Value::Null);
    // an action outside the vocabulary
    n.propose("p-unknown", &body(json!([{"action": "rns.restart"}])));
    n.decide("p-unknown", None)
        .await
        .problem(409, "unsupported_action");
    // a body that is not the shape: 502
    n.propose("p-shape", &json!({"actions": [{"action": "hub.pair"}]}));
    n.decide("p-shape", None)
        .await
        .problem(502, "upstream_error");
    n.propose(
        "p-args",
        &body(
            json!([{"action": "gateway.enable", "carrier": "ntfy", "mode": "5A", "server": "x"}]),
        ),
    );
    n.decide("p-args", None)
        .await
        .problem(502, "upstream_error");
    // gateway.enable with no server configured: the route's own refusal
    let enable = body(json!([{"action": "gateway.enable", "carrier": "ntfy", "mode": "5B"}]));
    n.propose("p-enable", &enable);
    n.decide("p-enable", None).await.problem(409, "bad_request");
    // the server set while the carrier is off asks nothing
    let r = n.patch(json!({"carrier": {"server": SERVER}})).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["carrier"]["kind"], "none");
    // now the carrier's consent, answered on the decision route
    let c = n.decide("p-enable", None).await.consent();
    assert_eq!(c["action"], "carrier_enable");
    assert_eq!(c["subject"], format!("ntfy:{SERVER}").as_str());
    assert_eq!(n.decision_of("p-enable").await, Value::Null, "still open");
    assert_eq!(
        n.status().await["gateway"]["enabled"],
        false,
        "nothing written"
    );
    let mut frames = Frames::on(&n.api);
    let r = n.decide("p-enable", c["hash"].as_str()).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["decision"]["decision"], "accepted");
    let v = n.status().await;
    assert_eq!(v["gateway"]["enabled"], true);
    assert_eq!(v["carrier"]["kind"], "ntfy");
    assert_eq!(v["carrier"]["mode"], "5B");
    assert_eq!(v["carrier"]["consent_hash"], c["hash"]);
    let f = frames.next(3000).await.expect("a relay frame");
    assert_eq!(f["gateway"]["enabled"], true);
    let rec: Value = serde_json::from_str(
        &std::fs::read_to_string(n.layout.root.join("agent/decisions/p-enable.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(rec["applied"], json!({"actions": ["gateway.enable"]}));
    // decided once
    n.decide("p-enable", None)
        .await
        .problem(409, "already_decided");
    // carrier.flush and hub.pair
    n.propose(
        "p-flush",
        &body(json!([{"action": "carrier.flush"}, {"action": "hub.pair"}])),
    );
    let r = n.decide("p-flush", None).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(noop.flushes(), 1);
    let rec: Value = serde_json::from_str(
        &std::fs::read_to_string(n.layout.root.join("agent/decisions/p-flush.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        rec["applied"],
        json!({"actions": ["carrier.flush", "hub.pair"], "flush_started": false})
    );
    // dismissing applies nothing
    n.propose("p-dismiss", &body(json!([{"action": "carrier.flush"}])));
    let r = n
        .post(
            "/api/v1/proposals/p-dismiss/decision",
            json!({"decision": "dismissed"}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(noop.flushes(), 1);
    // the agent's bearer never decides
    n.propose("p-bearer", &body(json!([{"action": "carrier.flush"}])));
    let auth_h = n.bearer();
    http(
        n.port,
        "POST",
        "/api/v1/proposals/p-bearer/decision",
        &[("Authorization", &auth_h)],
        Some(json!({"decision": "accepted"})),
    )
    .await
    .problem(401, "unauthenticated");
    assert_eq!(noop.flushes(), 1);
    // a member: needs_hub for hub.pair and for gateway.enable
    let m = node_with(false, false, |state| {
        let mut s = RelaySettings::default();
        s.set_ntfy_server(Some(SERVER)).unwrap();
        rs::save(state, &s).unwrap();
    })
    .await;
    m.propose("p-pair", &body(json!([{"action": "hub.pair"}])));
    m.decide("p-pair", None).await.problem(409, "needs_hub");
    m.propose("p-gw", &enable);
    m.decide("p-gw", None).await.problem(409, "needs_hub");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_relay_frame_follows_changes_and_never_a_get() {
    let _one = SERIAL.lock().await;
    let n = node(true).await;
    let mut frames = Frames::on(&n.api);
    // a fabric event before any frame: the first build publishes
    let far = NodeId::from([5u8; 32]);
    relay::tap(&FabricEvent::Forward {
        target: far,
        pending: 1,
    });
    let f = frames.next(3000).await.expect("the first frame");
    assert_eq!(f["paired_hubs"], json!([]));
    // the same status again: no frame for a fabric event, nor for a Fabric event that moved
    // nothing on a node with no pair
    relay::tap(&FabricEvent::Forward {
        target: far,
        pending: 1,
    });
    relay::tap(&FabricEvent::Fabric);
    assert!(frames.next(600).await.is_none(), "an unchanged status");
    // a GET, and a write that changes nothing, publish nothing
    n.status().await;
    assert_eq!(n.patch(json!({})).await.status, 200);
    assert_eq!(
        n.patch(json!({"updates": {"check_internet": true}}))
            .await
            .status,
        200
    );
    assert!(frames.next(600).await.is_none());
    // relay::changed() (the carrier's outcome, D11): a frame even with nothing else moved
    relay::changed();
    frames.next(3000).await.expect("a forced frame");
    // ... also from a thread of its own (the carrier's 5B stream, D11)
    std::thread::spawn(relay::changed).join().unwrap();
    frames
        .next(3000)
        .await
        .expect("a forced frame from another thread");
    // a write: a frame with the change
    n.patch(json!({"updates": {"check_internet": false}})).await;
    let f = frames.next(3000).await.expect("a frame after the write");
    assert_eq!(f["updates"]["check_internet"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paired_hubs_say_what_each_link_runs() {
    let _one = SERIAL.lock().await;
    let a = node(true).await;
    let b = node(true).await; // bound last: b's bus carries the fabric-driven frames
    let mut frames = Frames::on(&b.api);
    // a scans b's bundle, marked lan (plain on loopback); its ENROLL lands at b
    let r = b
        .post(
            "/api/v1/relay/pair/offer",
            json!({"role": "peer", "via": "tcp"}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let bundle_b = r.json()["bundle"]["text"].as_str().unwrap().to_string();
    let r = a
        .post(
            "/api/v1/relay/pair",
            json!({"bundle": bundle_b, "role": "peer", "via": "tcp", "lan": true}),
        )
        .await;
    assert_eq!(r.status, 202, "{}", r.body);
    let theirs =
        |recs: Vec<PairedHub>, far: NodeId| recs.iter().any(|p| p.node_id == far && p.theirs);
    until(30, "a's enrolment recorded at b", || async {
        theirs(b.records().await, a.id).then_some(())
    })
    .await;
    // b holds a's record and no half of its own yet: b's address is loopback (not private), so
    // a dials it inside TLS once b pins a: rpk
    let v = b.status().await;
    let row = v["paired_hubs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["node_id"] == hex(&a.id).as_str())
        .cloned()
        .expect("a in b's paired hubs");
    assert_eq!(row["role"], "peer");
    assert_eq!(row["via"], "tcp");
    assert_eq!(row["lan"], false);
    assert_eq!(row["tls"], "rpk");
    // the fabric-driven frame on b names a
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out: a relay frame on b naming a"
        );
        let Some(f) = frames.next(200).await else {
            continue;
        };
        if f["paired_hubs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["node_id"] == hex(&a.id).as_str())
        {
            break;
        }
    }
    // b scans a's bundle shown at a private address, not marked lan: ruling (c)
    let r = a
        .post(
            "/api/v1/relay/pair/offer",
            json!({"role": "peer", "via": "tcp"}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let shown = PairBundle::parse(r.json()["bundle"]["text"].as_str().unwrap()).unwrap();
    let private = PairBundle {
        address: Some("192.168.77.7:11183".parse().unwrap()),
        ..shown
    };
    let r = b
        .post(
            "/api/v1/relay/pair",
            json!({"bundle": private.text(), "role": "peer", "via": "tcp"}),
        )
        .await;
    assert_eq!(r.status, 202, "{}", r.body);
    let whole =
        |recs: Vec<PairedHub>, far: NodeId| recs.iter().any(|p| p.node_id == far && p.whole());
    until(30, "both pairs whole", || async {
        (whole(a.records().await, b.id) && whole(b.records().await, a.id)).then_some(())
    })
    .await;
    let row = |v: &Value, far: &NodeId| {
        v["paired_hubs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["node_id"] == hex(far).as_str())
            .cloned()
            .unwrap()
    };
    let at_b = row(&b.status().await, &a.id);
    assert_eq!(
        (&at_b["via"], &at_b["lan"], &at_b["tls"]),
        (&json!("tcp"), &json!(false), &json!("none")),
        "a private meeting address, no mark: plain by §4.1 rule 1: {at_b}"
    );
    assert_eq!(at_b["label"].as_str().unwrap().len(), 12);
    let at_a = row(&a.status().await, &b.id);
    assert_eq!((&at_a["lan"], &at_a["tls"]), (&json!(true), &json!("none")));
    // unpair both (idempotent routes, row D6)
    for (n, far) in [(&a, &b.id), (&b, &a.id)] {
        let r = http(
            n.port,
            "DELETE",
            &format!("/api/v1/relay/paired/{}", hex(far)),
            &[],
            None,
        )
        .await;
        assert_eq!(r.status, 204, "{}", r.body);
    }
}

/// Review finding 6: `quotas.transit` (Layer 7 §5.8) through `PATCH /fabric/settings`: a value
/// set reads back in the answer and in `GET /fabric`; `null` restores the derived default, one
/// quarter of the custody quota; a class outside the list names `transit` among the classes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_transit_quota_is_set_and_null_restores_the_derived_quarter() {
    let _one = SERIAL.lock().await;
    let n = node(true).await;
    let set = |body: Value| async move {
        let r = http(n.port, "PATCH", "/api/v1/fabric/settings", &[], Some(body)).await;
        check_op("patchFabricSettings", &r);
        r
    };
    let transit = || async {
        let r = n.get("/api/v1/fabric").await;
        check_op("getFabric", &r);
        r.json()["settings"]["quotas"]["transit"].clone()
    };
    let r = set(json!({"quotas": {"custody": 4_000_000, "transit": 123_456}})).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["quotas"]["transit"], 123_456);
    assert_eq!(transit().await, 123_456);
    let r = set(json!({"quotas": {"transit": null}})).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(
        r.json()["quotas"]["transit"],
        1_000_000,
        "a quarter of custody"
    );
    assert_eq!(transit().await, 1_000_000);
    let r = set(json!({"quotas": {"msg": 5}})).await;
    assert_eq!(r.status, 400, "{}", r.body);
    assert!(
        r.json()["detail"].as_str().unwrap().contains("transit"),
        "{}",
        r.body
    );
}
