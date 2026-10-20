//! Layer 6 C1 acceptance for the transport routes (`os/60-transport/README.md` §11, §12,
//! §13; contract `getTransport`, `patchTransportPosture`, `joinTransport`) on a real
//! `Spine` over a temp root, with `rnstatus` a fake `Runner` and rnsd a fake child (no
//! Python, no Reticulum):
//!
//! - `GET /transport` reads the facts cache (never `rnstatus` on the request path), the
//!   posture, region, callsign and order of `interfaces.toml`; no credential in it;
//! - `PATCH /transport/posture`: a region is the human's confirmation (unknown ⇒ 400), a
//!   `[reticulum]` change restarts rnsd through the spine, the order is the spine's, the
//!   agent's bearer is refused;
//! - `POST /transport/join`: `409` while LAN sharing or the Backbone listener is off; the
//!   descriptor with `Cache-Control: no-store` on a loopback or paired session, the IFAC
//!   passphrase only here; `401` for the bearer and for an unpaired LAN peer;
//! - `PATCH /bridge/settings` hands `order` to the spine (validated first).

use axum::extract::connect_info::MockConnectInfo;
use node111::daemon::bridge::{Bridge, BridgeConfig};
use node111::daemon::dashboard::api_v1::auth::DaemonToken;
use node111::daemon::dashboard::api_v1::transport::TransportApi;
use node111::daemon::dashboard::{
    routes, serve_listener, ApiConfig, ApiState, DashboardState, SharedState,
};
use node111::daemon::devices::store;
use node111::daemon::fabric::{self, FabricConfig};
use node111::daemon::transport::config;
use node111::daemon::transport::interfaces;
use node111::daemon::transport::posture::{BoxFut, RnsdChild, Spine};
use node111::daemon::transport::rnstatus::{FactsCache, InterfaceFacts, Rnstatus, TickConfig};
use node111::daemon::DaemonLayout;
use node111::install::categorize::Category;
use node111::util::run::{Output, Runner};
use serde_json::{json, Map, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

// ---------------------------------------------------------------- contract

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
            "{}/../../../40-interface/ui/test/contract.gen.ts",
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

/// `^…$` patterns: literals, `[…]` classes, `+`, `{n}`, `{n,m}` (the contract's subset).
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

/// A port of `40-interface/ui/test/helpers/validate.ts` (same subset, `oneOf` = any).
fn validate(v: &Value, schema: &Value, path: &str, errs: &mut Vec<String>) {
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        let name = r.strip_prefix("#/components/schemas/").expect("local ref");
        let s = contract()
            .schemas
            .get(name)
            .unwrap_or_else(|| panic!("no schema {name}"));
        return validate(v, s, path, errs);
    }
    if let Some(alts) = schema.get("oneOf").and_then(Value::as_array) {
        let ok = alts.iter().any(|a| {
            let mut e = Vec::new();
            validate(v, a, path, &mut e);
            e.is_empty()
        });
        if !ok {
            errs.push(format!("{path}: no oneOf branch matches {v}"));
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
    if let Some(k) = schema.get("const") {
        if k != v {
            errs.push(format!("{path}: expected const {k}"));
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
        if schema
            .get("maximum")
            .and_then(Value::as_f64)
            .is_some_and(|m| n > m)
        {
            errs.push(format!("{path}: above maximum"));
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
    if let Some(a) = v.as_array() {
        if let Some(items) = schema.get("items") {
            for (i, x) in a.iter().enumerate() {
                validate(x, items, &format!("{path}[{i}]"), errs);
            }
        }
        if schema
            .get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|m| (a.len() as u64) < m)
        {
            errs.push(format!("{path}: fewer than minItems"));
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
        if let Some(add) = schema.get("additionalProperties").filter(|a| a.is_object()) {
            let props = schema.get("properties").and_then(Value::as_object);
            for (k, x) in o {
                if props.is_none_or(|p| !p.contains_key(k)) {
                    validate(x, add, &format!("{path}.{k}"), errs);
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

/// A response against operation `op`: its success status and schema, or a `Problem`.
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

    fn problem(&self, status: u16, code: &str) -> Value {
        assert_eq!(self.status, status, "body: {}", self.body);
        let v = self.json();
        assert_eq!(v["code"], code, "{}", self.body);
        assert_schema("Problem", &v);
        v
    }
}

fn dechunk(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = raw;
    while let Some(eol) = rest.windows(2).position(|w| w == b"\r\n") {
        let n = std::str::from_utf8(&rest[..eol])
            .ok()
            .and_then(|s| usize::from_str_radix(s.trim(), 16).ok())
            .unwrap_or(0);
        let after = &rest[eol + 2..];
        let Some(chunk) = after.get(..n).filter(|_| n > 0) else {
            break;
        };
        out.extend_from_slice(chunk);
        let tail = &after[n..];
        rest = tail.strip_prefix(b"\r\n").unwrap_or(tail);
    }
    out
}

/// One request to `127.0.0.1:port`; writes carry `Content-Type: application/json`.
fn http(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&Value>,
) -> Resp {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(60)))
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
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
    });
    let body = if chunked {
        dechunk(body)
    } else {
        body.to_vec()
    };
    Resp {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
        headers,
    }
}

// ---------------------------------------------------------------- fakes

/// rnstatus: every call logged; reads answer nothing.
#[derive(Clone, Default)]
struct Rec {
    log: Arc<Mutex<Vec<String>>>,
}

impl Runner for Rec {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        let mut line = program.rsplit('/').next().unwrap_or(program).to_string();
        for a in args.iter().skip(2) {
            line.push(' ');
            line.push_str(a);
        }
        self.log.lock().unwrap().push(line);
        Ok(Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        })
    }

    fn run_as(&self, _: &str, _: &str, _: &[&str]) -> anyhow::Result<Output> {
        anyhow::bail!("not used")
    }
}

struct FakeRnsd {
    log: Arc<Mutex<Vec<String>>>,
    running: AtomicBool,
}

impl RnsdChild for FakeRnsd {
    fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn restart(&self, _grace: Duration) -> BoxFut<'_, bool> {
        self.log.lock().unwrap().push("restart rnsd".into());
        Box::pin(async { true })
    }
}

const LAN_PEER: &str = "192.168.1.50:40000";
const OWN_LAN: &str = "192.168.1.10";

struct Node {
    api: Arc<ApiState>,
    spine: Arc<Spine>,
    facts: Arc<FactsCache>,
    log: Arc<Mutex<Vec<String>>>,
    token: DaemonToken,
    port: u16,
    lan: u16,
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

async fn node(lan_sharing: bool) -> Node {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let st = layout.state_dir();
    let startup = interfaces::startup(&st, &st.join(store::FILE), &config::config_path(&layout));
    let rec = Rec::default();
    let log = Arc::clone(&rec.log);
    let rn = Arc::new(Rnstatus::new(Arc::new(rec), &layout.root));
    let spine = Spine::new(
        layout.clone(),
        startup,
        rn,
        lan_sharing,
        Some(Category::Hub4A),
    );
    spine.set_rnsd(Arc::new(FakeRnsd {
        log: Arc::clone(&log),
        running: AtomicBool::new(true),
    }));
    spine.write_config().unwrap();
    let facts = FactsCache::new(TickConfig::default());
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.19")));
    let mut acfg = ApiConfig::new(layout.clone(), lan_sharing);
    acfg.persist_auth = false;
    acfg.trust_loopback = true;
    let api = ApiState::new(dash, acfg);
    api.set_own_addrs(vec!["127.0.0.1".parse().unwrap(), OWN_LAN.parse().unwrap()]);
    api.insert_ext(Arc::new(TransportApi {
        spine: Arc::clone(&spine),
        facts: Arc::clone(&facts),
    }));
    let token = DaemonToken::mint().unwrap();
    api.insert_ext(token.clone());
    let port = serve(&api, None).await;
    let lan = serve(&api, Some(LAN_PEER)).await;
    log.lock().unwrap().clear();
    Node {
        api,
        spine,
        facts,
        log,
        token,
        port,
        lan,
        _dir: dir,
    }
}

impl Node {
    fn get(&self, path: &str) -> Resp {
        http(self.port, "GET", path, &[], None)
    }
    fn patch(&self, path: &str, body: Value) -> Resp {
        http(self.port, "PATCH", path, &[], Some(&body))
    }
    fn post(&self, path: &str, body: Value) -> Resp {
        http(self.port, "POST", path, &[], Some(&body))
    }
    fn bearer(&self) -> String {
        format!("Bearer {}", self.token.as_str())
    }
    fn session(&self) -> String {
        let code = self.api.auth.pair_code();
        let s = self
            .api
            .auth
            .pair(LAN_PEER.parse::<SocketAddr>().unwrap().ip(), &code)
            .unwrap();
        format!("s111={}", s.token)
    }
    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.lock().unwrap())
    }
}

// ---------------------------------------------------------------- tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_status_reads_the_cache_and_the_file_never_rnstatus() {
    let n = node(false).await;
    // nothing read yet: unavailable, no interfaces; the request ran nothing
    let r = n.get("/api/v1/transport");
    check_op("getTransport", &r);
    let v = r.json();
    assert_eq!(v["interfaces_state"], "unavailable");
    assert_eq!(v["interfaces"], json!([]));
    assert_eq!(v["posture"]["share_instance"], true);
    assert_eq!(v["posture"]["enable_transport"], true, "Cat 4 default");
    assert!(v["region"].is_null());
    assert_eq!(v["order"][0], "111");
    assert!(n.take_log().is_empty(), "the route never runs rnstatus");
    // the tick recorded facts: ok, the interfaces from the cache
    n.facts.record(Ok(vec![InterfaceFacts {
        name: "Auto Interface".into(),
        kind: "AutoInterface".into(),
        up: true,
        rx: 1200,
        tx: 800,
        peers: Some(2),
    }]));
    let r = n.get("/api/v1/transport");
    check_op("getTransport", &r);
    let v = r.json();
    assert_eq!(v["interfaces_state"], "ok");
    assert_eq!(v["interfaces"][0]["name"], "Auto Interface");
    assert_eq!(v["interfaces"][0]["peers"], 2);
    assert!(!r.body.contains("rpc_key") && !r.body.contains("passphrase"));
    // the bearer reads
    let r = http(
        n.port,
        "GET",
        "/api/v1/transport",
        &[("Authorization", &n.bearer())],
        None,
    );
    assert_eq!(r.status, 200, "{}", r.body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn posture_patches_go_through_the_spine_and_refuse_the_bearer() {
    let n = node(false).await;
    // a region: the request is the human's confirmation
    let r = n.patch("/api/v1/transport/posture", json!({"region": "EU_868"}));
    check_op("patchTransportPosture", &r);
    let v = r.json();
    assert_eq!(
        (v["applied"].as_bool(), v["reverted"].as_bool()),
        (Some(true), Some(false))
    );
    assert_eq!(v["status"]["region"], "EU_868");
    assert_eq!(n.spine.file().radio.region.as_deref(), Some("EU_868"));
    // an unknown region and out-of-contract values: 400, nothing written
    n.patch("/api/v1/transport/posture", json!({"region": "MARS_1"}))
        .problem(400, "bad_request");
    n.patch(
        "/api/v1/transport/posture",
        json!({"posture": {"lan": {"port": 80}}}),
    )
    .problem(400, "bad_request");
    n.patch(
        "/api/v1/transport/posture",
        json!({"posture": {"surprise": true}}),
    )
    .problem(400, "bad_request");
    assert_eq!(n.spine.file().radio.region.as_deref(), Some("EU_868"));
    // a [reticulum] key: rnsd restarts through the spine (never pkill)
    n.take_log();
    let r = n.patch(
        "/api/v1/transport/posture",
        json!({"posture": {"enable_transport": false}}),
    );
    check_op("patchTransportPosture", &r);
    assert_eq!(r.json()["status"]["posture"]["enable_transport"], false);
    assert!(n.take_log().contains(&"restart rnsd".to_string()));
    // callsign and order
    let r = n.patch(
        "/api/v1/transport/posture",
        json!({"callsign": "W1AW", "order": ["111", "meshtastic", "lxmf"]}),
    );
    check_op("patchTransportPosture", &r);
    assert_eq!(r.json()["status"]["callsign"], "W1AW");
    assert_eq!(
        r.json()["status"]["order"],
        json!(["111", "meshtastic", "lxmf"])
    );
    let r = n.patch("/api/v1/transport/posture", json!({"callsign": null}));
    assert!(r.json()["status"]["callsign"].is_null());
    n.patch(
        "/api/v1/transport/posture",
        json!({"order": ["111", "111"]}),
    )
    .problem(400, "bad_request");
    // the agent's bearer: 401, nothing changes
    http(
        n.port,
        "PATCH",
        "/api/v1/transport/posture",
        &[("Authorization", &n.bearer())],
        Some(&json!({"region": "US"})),
    )
    .problem(401, "unauthenticated");
    assert_eq!(n.spine.file().radio.region.as_deref(), Some("EU_868"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_join_descriptor_is_a_post_for_trusted_sessions_only() {
    // LAN sharing off: no relay to join
    let off = node(false).await;
    off.post("/api/v1/transport/join", json!({}))
        .problem(409, "bad_request");
    let n = node(true).await;
    // loopback: the descriptor, no-store, the LAN address (not loopback)
    let r = n.post("/api/v1/transport/join", json!({}));
    check_op("joinTransport", &r);
    assert_eq!(r.header("cache-control"), Some("no-store"));
    let v = r.json();
    assert_eq!(v["address"], OWN_LAN);
    assert_eq!(v["port"], 4242);
    assert!(
        v.get("ifac_passphrase").is_none(),
        "IFAC off: no credential"
    );
    // IFAC on: the passphrase rides only here
    let r = n.patch(
        "/api/v1/transport/posture",
        json!({"posture": {"lan": {"ifac": true}}}),
    );
    check_op("patchTransportPosture", &r);
    let r = n.post("/api/v1/transport/join", json!({}));
    check_op("joinTransport", &r);
    let pass = r.json()["ifac_passphrase"].as_str().unwrap().to_string();
    assert_eq!(r.json()["ifac_name"], "111-lan");
    assert!(
        !n.get("/api/v1/transport").body.contains(&pass),
        "never on a GET"
    );
    // a paired LAN session: the descriptor; asked on the node's own LAN address
    let cookie = n.session();
    let mut conn = TcpStream::connect(("127.0.0.1", n.lan)).unwrap();
    let req = format!(
        "POST /api/v1/transport/join HTTP/1.1\r\nHost: {OWN_LAN}\r\nConnection: close\r\nCookie: {cookie}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
    );
    conn.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    conn.read_to_string(&mut raw).unwrap();
    assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
    assert!(raw.contains(&format!("\"address\":\"{OWN_LAN}\"")), "{raw}");
    // the bearer and an unpaired LAN peer: 401
    http(
        n.port,
        "POST",
        "/api/v1/transport/join",
        &[("Authorization", &n.bearer())],
        Some(&json!({})),
    )
    .problem(401, "unauthenticated");
    http(
        n.lan,
        "POST",
        "/api/v1/transport/join",
        &[],
        Some(&json!({})),
    )
    .problem(401, "unauthenticated");
    // the Backbone listener off: 409
    let r = n.patch(
        "/api/v1/transport/posture",
        json!({"posture": {"lan": {"backbone_listen": false}}}),
    );
    check_op("patchTransportPosture", &r);
    n.post("/api/v1/transport/join", json!({}))
        .problem(409, "bad_request");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_bridge_settings_order_is_the_spines() {
    let n = node(false).await;
    // a bridge on a real fabric beside the spine
    let identity = node111::identity::generate(&n.spine.layout().identity_dir()).unwrap();
    let mut c = FabricConfig::production(n.spine.layout(), None);
    c.listen = "127.0.0.1:0".parse().unwrap();
    c.housekeeping = Duration::from_secs(3600);
    let fabric = fabric::start(c, Some(identity), &n.api).await;
    assert!(fabric.running());
    let bridge = Bridge::new(
        BridgeConfig::new(&n.spine.layout().root),
        Arc::clone(&fabric),
        &n.api,
    );
    n.api.insert_ext(bridge);
    let r = n.get("/api/v1/bridge/settings");
    check_op("getBridgeSettings", &r);
    assert_eq!(r.json()["order"], json!(n.spine.file().order.networks));
    let r = n.patch(
        "/api/v1/bridge/settings",
        json!({"order": ["app:simplex", "111", "lxmf"]}),
    );
    check_op("patchBridgeSettings", &r);
    assert_eq!(r.json()["order"], json!(["app:simplex", "111", "lxmf"]));
    assert_eq!(
        n.spine.file().order.networks,
        ["app:simplex", "111", "lxmf"]
    );
    // an invalid order: 400 and nothing changes
    n.patch("/api/v1/bridge/settings", json!({"order": ["Bad Id"]}))
        .problem(400, "bad_request");
    assert_eq!(
        n.spine.file().order.networks,
        ["app:simplex", "111", "lxmf"]
    );
    // GET /networks follows the order (networks it does not name keep release order after)
    let nets = n.get("/api/v1/networks").json()["networks"].clone();
    let ids: Vec<&str> = nets
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids[..2], ["app:simplex", "lxmf"], "{ids:?}");
    assert_eq!(ids[2..], ["meshtastic", "app:deltachat"], "{ids:?}");
    fabric.shutdown().await;
}
