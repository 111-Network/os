//! Layer 5 F2 acceptance (`os/50-messaging/README.md` §2, §4, §9, §10.2, §15,
//! §17; `os/50-messaging/TASKS.md` row F2): the fabric wired into `111d` with
//! the real identity handle (a fresh `id111` identity on a temp root, its
//! counter store, SQLCipher Mode S), driven through the real dashboard app
//! on loopback sockets.
//!
//! - every §15 route's happy path, each 2xx body and every problem checked
//!   against the contract digest (`40-interface/ui/test/contract.gen.ts`,
//!   generated from `openapi.yaml`) by a port of the UI's JSON-Schema-subset
//!   validator — including the SSE `message` / `contact` / `fabric` frames;
//! - hub offer → member enrollment over a real `tcp-lan` link (and the same
//!   through `111 fabric offer|enroll|status`), a message delivered, the
//!   member's `fabric/settings` persisted (0600);
//! - the auth matrix: session (paired LAN cookie, trusted loopback) / the
//!   agent's bearer (reads only) / nothing (401); settings, offer and dismiss
//!   loopback-only;
//! - SSE coalescing: a 16-fragment transfer over a 160 B link ⇒ one frame
//!   per message change, never one per fragment;
//! - `kind: message` proposals: bad body, bad stamp, non-contact, pending
//!   request refused (the proposal stays open); the happy path is marked
//!   applied only after the store committed the message, exactly once;
//! - `restore_pending`: `409` on send and on proposal acceptance;
//! - the listener is bound only while the settings ask for it;
//! - `reason_code` `not_enrolled` / `cancelled` and `Problem.code`
//!   `restore_pending` pass the contract validator.
//!
//! Wave 2 bounce (F2 review): restore-pending with a stored announce runs no
//! link and leaves no `counters` file (M1, end to end); the production
//! `refuse_loopback` rule on dials, accepts and enrollment; hand-configured
//! `peers` with `listen_lan`; `cancelMessage`; the home hub persisted before
//! the enroll answer and rebuilt from the store after a corrupt settings
//! file; a bare enrollment code read through the CLI's terminal reader; the
//! applied-proposal guard on disk (a pending record survives a restart); the
//! announce re-minted on a settings change.

use axum::extract::connect_info::MockConnectInfo;
use mesh111::link::lossy::{self, EndSpec, LossyConfig, LossyEnd};
use mesh111::link::{LinkId, LinkKind, PathProfile};
use node111::daemon::dashboard::api_v1::auth;
use node111::daemon::dashboard::api_v1::events::Frame;
use node111::daemon::dashboard::{
    routes, serve_listener, ApiConfig, ApiState, DashboardState, SharedState,
};
use node111::daemon::fabric::cli::{self as fcli, FabricCmd};
use node111::daemon::fabric::{self, json as fjson, FabricConfig, FabricHandle};
use node111::daemon::DaemonLayout;
use serde_json::{json, Map, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;
use zeroize::Zeroizing;

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

/// `^…$` patterns of the contract: literals, `[…]` classes, `+`, `{n}`,
/// `{n,m}` (greedy; no alternation — the contract has none).
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

fn date_time_ok(s: &str) -> bool {
    // ^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$
    let b = s.as_bytes();
    let d = |r: std::ops::Range<usize>| b.get(r).is_some_and(|x| x.iter().all(u8::is_ascii_digit));
    if b.len() < 20
        || !(d(0..4) && b[4] == b'-' && d(5..7) && b[7] == b'-' && d(8..10) && b[10] == b'T')
        || !(d(11..13) && b[13] == b':' && d(14..16) && b[16] == b':' && d(17..19))
    {
        return false;
    }
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        let n = r.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return false;
        }
        rest = &r[n..];
    }
    rest == "Z"
        || (rest.len() == 6
            && matches!(rest.as_bytes()[0], b'+' | b'-')
            && rest.as_bytes()[3] == b':')
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

/// A port of `40-interface/ui/test/helpers/validate.ts` (same subset).
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
        if schema.get("format").and_then(Value::as_str) == Some("date-time") && !date_time_ok(s) {
            errs.push(format!("{path}: {s:?} not RFC 3339"));
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

/// A response against operation `op`: its success status and schema, or a
/// valid `Problem`.
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

/// Every Layer 5 SSE frame against its `x-sse-events` schema.
fn check_frames(frames: &[Frame]) {
    for f in frames {
        let name = match f.event.as_str() {
            "message" => "SseMessage",
            "contact" => "SseContact",
            "fabric" => "FabricStatus",
            _ => continue,
        };
        let v: Value = serde_json::from_str(&f.data).unwrap();
        assert_schema(name, &v);
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

// ---------------------------------------------------------------- nodes

const LAN_PEER: &str = "192.168.1.50:40000";

#[derive(Clone, Default)]
struct Opts {
    hub: bool,
    drop_counters: bool,
    /// The production rule (§2): loopback is never a peer.
    refuse_loopback: bool,
    /// `fabric/settings` written before the first start.
    settings: Option<String>,
}

struct Node {
    api: Arc<ApiState>,
    fabric: Arc<FabricHandle>,
    /// Dashboard on a real loopback socket (trusted loopback).
    port: u16,
    /// The same app with every request from a LAN peer.
    lan: u16,
    layout: DaemonLayout,
    id: String,
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

fn test_config(layout: &DaemonLayout, o: Opts) -> FabricConfig {
    let mut c = FabricConfig::production(layout, None);
    c.listen = "127.0.0.1:0".parse().unwrap();
    // every node of this test is on loopback; one test turns the
    // production rule back on
    c.tcp.refuse_loopback = o.refuse_loopback;
    c.tcp.backoff_min = Duration::from_millis(30);
    c.tcp.backoff_max = Duration::from_millis(200);
    c.tcp.handshake_timeout = Duration::from_secs(3);
    c.hub_default = o.hub;
    c.enroll_timeout = Duration::from_secs(10);
    c.housekeeping = Duration::from_millis(200);
    c
}

async fn node(o: Opts) -> Node {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let identity = node111::identity::generate(&layout.identity_dir()).unwrap();
    let id = identity.node_id_hex();
    if o.drop_counters {
        std::fs::remove_file(layout.identity_dir().join("counters")).unwrap();
    }
    if let Some(s) = &o.settings {
        std::fs::create_dir_all(layout.root.join("fabric")).unwrap();
        std::fs::write(layout.root.join("fabric/settings"), s).unwrap();
    }
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.14")));
    dash.write().unwrap().node_id = Some(id.clone());
    let api = ApiState::new(dash, ApiConfig::new(layout.clone(), false));
    let fabric = fabric::start(test_config(&layout, o), Some(identity), &api).await;
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

impl Node {
    fn get(&self, path: &str) -> Resp {
        http(self.port, "GET", path, &[], None)
    }

    fn post(&self, path: &str, body: Value) -> Resp {
        http(self.port, "POST", path, &[], Some(&body))
    }

    fn patch(&self, path: &str, body: Value) -> Resp {
        http(self.port, "PATCH", path, &[], Some(&body))
    }

    fn delete(&self, path: &str) -> Resp {
        http(self.port, "DELETE", path, &[], None)
    }

    fn card(&self) -> String {
        let r = self.get("/api/v1/contacts/me/card");
        check_op("getOwnCard", &r);
        r.json()["text"].as_str().unwrap().to_string()
    }

    /// Add `other` as a contact by card (`oob`).
    fn add(&self, other: &Node) -> Value {
        let r = self.post("/api/v1/contacts", json!({ "card": other.card() }));
        check_op("addContact", &r);
        r.json()
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

    fn sub(&self) -> tokio::sync::broadcast::Receiver<Arc<Frame>> {
        self.api.events.subscribe(Some(self.api.events.head())).rx
    }

    /// Stop this node's fabric, run `between` on the stopped root (it may
    /// hand back the identity to start with; otherwise it is re-read from
    /// disk), and start the fabric again, serving the same dashboard app.
    async fn restart(
        &mut self,
        o: Opts,
        between: impl FnOnce(&DaemonLayout) -> Option<node111::identity::Identity>,
    ) -> Arc<FabricHandle> {
        self.fabric.shutdown().await;
        let identity = between(&self.layout)
            .unwrap_or_else(|| node111::identity::load(&self.layout.identity_dir()).unwrap());
        let again = fabric::start(test_config(&self.layout, o), Some(identity), &self.api).await;
        assert!(again.running(), "{:?}", again.down_reason());
        self.api.insert_ext(Arc::clone(&again));
        self.fabric = Arc::clone(&again);
        again
    }
}

/// The hello and the announce a `tcp-lan` listener opens a connection with.
async fn opening_announce(at: SocketAddr) -> Vec<u8> {
    use mesh111::link::tcp_lan::{read_frame, read_hello, T_ANNOUNCE};
    let mut s = tokio::net::TcpStream::connect(at).await.unwrap();
    read_hello(&mut s).await.unwrap();
    let (ty, body) = read_frame(&mut s).await.unwrap();
    assert_eq!(ty, T_ANNOUNCE);
    body
}

/// A hub offer from `hub` (route) — `(code, card text, address)`.
fn offer_of(hub: &Node) -> (String, String, String) {
    let r = hub.post("/api/v1/fabric/hub/offer", json!({}));
    check_op("offerHub", &r);
    let o = r.json();
    (
        o["code"].as_str().unwrap().to_string(),
        o["card"]["text"].as_str().unwrap().to_string(),
        o["address"].as_str().unwrap().to_string(),
    )
}

/// `member` enrolls with `hub` by route.
fn enroll_by_route(member: &Node, hub: &Node) {
    let (code, card, address) = offer_of(hub);
    let r = member.post(
        "/api/v1/fabric/hub/enroll",
        json!({ "address": address, "code": code, "card": card }),
    );
    check_op("enrollHub", &r);
    assert_eq!(r.status, 200, "{}", r.body);
}

/// Poll `f` (blocking HTTP inside) until it yields, up to `secs`.
async fn until<T>(secs: u64, what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Frames received on `rx` within `wait` after the last one.
async fn drain(
    rx: &mut tokio::sync::broadcast::Receiver<Arc<Frame>>,
    wait: Duration,
) -> Vec<Frame> {
    let mut out = Vec::new();
    while let Ok(Ok(f)) = tokio::time::timeout(wait, rx.recv()).await {
        out.push((*f).clone());
    }
    out
}

fn message_frames(frames: &[Frame], id: &str) -> Vec<Value> {
    frames
        .iter()
        .filter(|f| f.event == "message")
        .map(|f| serde_json::from_str::<Value>(&f.data).unwrap())
        .filter(|v| v["message"]["id"] == id)
        .collect()
}

fn thread_of(n: &Node, peer: &str) -> Value {
    let r = n.get(&format!("/api/v1/messages/threads/{peer}"));
    check_op("getThread", &r);
    r.json()
}

/// Pair two nodes over an in-process lossy link (no loss).
async fn lossy_link(
    a: &Node,
    b: &Node,
    profile: PathProfile,
    kind: LinkKind,
) -> (Arc<LossyEnd>, Arc<LossyEnd>) {
    let fa = a.fabric.fabric().unwrap();
    let fb = b.fabric.fabric().unwrap();
    let cfg = LossyConfig {
        profile,
        kind,
        ..LossyConfig::default()
    };
    let (ea, eb) = lossy::pair(
        EndSpec {
            id: LinkId::new("lossy-a"),
            announce: a.fabric.own_announce(),
            events: fa.link_events(),
            cfg: cfg.clone(),
        },
        EndSpec {
            id: LinkId::new("lossy-b"),
            announce: b.fabric.own_announce(),
            events: fb.link_events(),
            cfg,
        },
    );
    fa.add_link(ea.clone(), None).await.unwrap();
    fb.add_link(eb.clone(), None).await.unwrap();
    (ea, eb)
}

fn write_proposal(n: &Node, id: &str, stamp: Option<&str>, body: &str) {
    let dir = n.layout.root.join("agent/proposals");
    std::fs::create_dir_all(&dir).unwrap();
    let p = json!({
        "schema": 1, "id": id, "node_id": stamp, "task_id": null, "kind": "message",
        "title": "Send a message", "body": body, "created_at": "2026-10-14T10:00:00Z",
    });
    std::fs::write(dir.join(format!("{id}.json")), p.to_string()).unwrap();
}

fn decide(n: &Node, id: &str, decision: &str) -> Resp {
    let r = n.post(
        &format!("/api/v1/proposals/{id}/decision"),
        json!({ "decision": decision }),
    );
    check_op("decideProposal", &r);
    r
}

// ---------------------------------------------------------------- tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hub_offer_enroll_send_and_every_route() {
    let a = node(Opts {
        hub: true,
        ..Opts::default()
    })
    .await;
    let b = node(Opts::default()).await;
    let d = node(Opts::default()).await;
    let e = node(Opts::default()).await;
    let (mut sa, mut sb) = (a.sub(), b.sub());

    // listener bound only when the settings say so (A: Cat 4 hub default)
    let a_addr = a.fabric.listen_addr().await.expect("hub listens");
    assert!(
        b.fabric.listen_addr().await.is_none(),
        "member does not listen"
    );

    // GET /fabric (both shapes valid; A is a hub, B is not)
    let r = a.get("/api/v1/fabric");
    check_op("getFabric", &r);
    let st = r.json();
    assert_eq!(st["settings"]["hub_role"], true);
    assert_eq!(st["store"]["open"], true);
    assert_eq!(st["links"][0]["kind"], "tcp-lan");
    let r = b.get("/api/v1/fabric");
    check_op("getFabric", &r);
    assert_eq!(r.json()["hub"]["enrolled"], false);

    // own cards (public announce only)
    let card = a.card();
    assert!(card.len() > 200, "{card}");

    // `111 fabric offer|enroll|status`: D enrolls with A through the CLI
    let (port_a, port_d) = (a.port, d.port);
    let out = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        fcli::run_on(port_a, &FabricCmd::Offer, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    })
    .await
    .unwrap();
    assert!(out.contains("Enrollment code:"), "{out}");
    assert!(out.contains(&a_addr.to_string()), "{out}");
    let bundle = out
        .lines()
        .find_map(|l| l.strip_prefix("On the member: 111 fabric enroll '"))
        .and_then(|l| l.strip_suffix('\''))
        .expect("bundle line")
        .to_string();
    let enrolled = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        let cmd = fcli::parse(&["enroll".into(), bundle]).unwrap();
        fcli::run_on(port_d, &cmd, &mut out).unwrap();
        let mut st = Vec::new();
        fcli::run_on(port_d, &FabricCmd::Status, &mut st).unwrap();
        (
            String::from_utf8(out).unwrap(),
            String::from_utf8(st).unwrap(),
        )
    })
    .await
    .unwrap();
    assert!(
        enrolled.0.starts_with("Enrolled with hub"),
        "{}",
        enrolled.0
    );
    assert!(enrolled.1.contains("Messaging: running"), "{}", enrolled.1);
    assert!(enrolled.1.contains(&a.id), "{}", enrolled.1);

    // route offer (loopback, no-store) → B enrolls by route
    let r = a.post("/api/v1/fabric/hub/offer", json!({}));
    check_op("offerHub", &r);
    assert_eq!(r.header("cache-control"), Some("no-store"));
    let offer = r.json();
    assert_eq!(offer["address"], a_addr.to_string());
    assert!(offer["bundle"]["text"]
        .as_str()
        .unwrap()
        .starts_with("111HUB:"));
    // by host name (U2: the Settings page sends `host:port`; the route
    // resolves it, preferring IPv4 — the hub listens on IPv4)
    let by_name = format!("localhost:{}", a_addr.port());
    let r = b.post(
        "/api/v1/fabric/hub/enroll",
        json!({ "address": by_name, "code": offer["code"], "card": offer["card"]["text"] }),
    );
    check_op("enrollHub", &r);
    let hub = r.json();
    assert_eq!(hub["enrolled"], true);
    assert_eq!(hub["node_id"], a.id);
    assert_eq!(hub["address"], a_addr.to_string(), "persisted by address");
    // a second enrollment while enrolled: 409
    let r = b.post(
        "/api/v1/fabric/hub/enroll",
        json!({ "address": offer["address"], "code": offer["code"] }),
    );
    check_op("enrollHub", &r);
    r.problem(409, "bad_request");

    // the member's settings are persisted (0600) with the home hub — before
    // the enroll answer (F2 review m2: no wait for the SSE pump)
    let saved = std::fs::read_to_string(b.layout.root.join("fabric/settings")).unwrap();
    assert!(saved.contains(&a.id), "{saved}");
    assert!(saved.contains("\"home_hub\""), "{saved}");
    assert!(saved.contains(&a_addr.to_string()), "{saved}");
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(b.layout.root.join("fabric/settings"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    // `111 fabric enroll --address … --card …` with the code read through
    // the CLI's terminal reader, never argv (F2 review m4)
    let (code, card_a, _) = offer_of(&a);
    let (port_e, addr) = (e.port, a_addr.to_string());
    let out = tokio::task::spawn_blocking(move || {
        let cmd = fcli::parse(&[
            "enroll".into(),
            "--address".into(),
            addr,
            "--card".into(),
            card_a,
        ])
        .unwrap();
        let mut asked = 0;
        let mut out = Vec::new();
        fcli::run_with(port_e, &cmd, &mut out, &mut || {
            asked += 1;
            Ok(Zeroizing::new(code.clone()))
        })
        .unwrap();
        assert_eq!(asked, 1, "the code is asked for once");
        String::from_utf8(out).unwrap()
    })
    .await
    .unwrap();
    assert!(out.starts_with("Enrolled with hub"), "{out}");
    assert!(e.fabric.persisted().fabric.home_hub.is_some());

    let r = a.delete("/api/v1/fabric/hub/offer");
    check_op("dismissHubOffer", &r);
    assert_eq!(r.status, 204);

    // contacts: enrolled both ways; list, get, patch
    let r = b.get("/api/v1/contacts");
    check_op("listContacts", &r);
    let r = b.get(&format!("/api/v1/contacts/{}", a.id));
    check_op("getContact", &r);
    assert_eq!(r.json()["trust"], "enrolled");
    let r = until(5, "A lists B", || {
        let r = a.get(&format!("/api/v1/contacts/{}", b.id));
        (r.status == 200).then_some(r)
    })
    .await;
    check_op("getContact", &r);
    let r = b.patch(
        &format!("/api/v1/contacts/{}", a.id),
        json!({ "alias": "Home hub", "pinned": true }),
    );
    check_op("patchContact", &r);
    assert_eq!(r.json()["alias"], "Home hub");
    assert_eq!(r.json()["pinned"], true);

    // send B → A over the enrolled tcp-lan link
    let r = b.post(
        "/api/v1/messages",
        json!({ "to": a.id, "text": "hello hub" }),
    );
    check_op("sendMessage", &r);
    assert_eq!(r.status, 202);
    let mid = r.json()["id"].as_str().unwrap().to_string();
    until(10, "delivered", || {
        let t = thread_of(&b, &a.id);
        let m = t["messages"]
            .as_array()?
            .iter()
            .find(|m| m["id"] == mid)?
            .clone();
        (m["state"] == "delivered").then_some(())
    })
    .await;
    let t = thread_of(&a, &b.id);
    let got = &t["messages"][0];
    assert_eq!(got["dir"], "in");
    assert_eq!(got["text"], "hello hub");
    assert!(got["sent_at_claim"].is_string(), "{got}");
    let r = a.get("/api/v1/messages/threads");
    check_op("listThreads", &r);
    let threads = r.json();
    assert_eq!(threads["threads"][0]["peer"], b.id);
    assert_eq!(threads["threads"][0]["unread"], 1);

    // pages: limit and since_seq
    for i in 0..3 {
        let r = b.post(
            "/api/v1/messages",
            json!({ "to": a.id, "text": format!("n{i}") }),
        );
        assert_eq!(r.status, 202, "{}", r.body);
    }
    let all = until(10, "four in A's thread", || {
        let t = thread_of(&a, &b.id);
        (t["messages"].as_array()?.len() == 4).then_some(t)
    })
    .await;
    let seqs: Vec<u64> = all["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["local_seq"].as_u64().unwrap())
        .collect();
    let r = a.get(&format!("/api/v1/messages/threads/{}?limit=2", b.id));
    check_op("getThread", &r);
    let p = r.json();
    assert_eq!(p["more"], true);
    assert_eq!(
        p["messages"][0]["local_seq"], seqs[2],
        "newest two, ascending"
    );
    let r = a.get(&format!(
        "/api/v1/messages/threads/{}?since_seq={}&limit=2",
        b.id, seqs[0]
    ));
    check_op("getThread", &r);
    let p = r.json();
    assert_eq!(p["messages"][0]["local_seq"], seqs[1], "first two above");
    assert_eq!(p["messages"][1]["local_seq"], seqs[2]);
    assert_eq!(p["more"], true);
    let r = a.get(&format!(
        "/api/v1/messages/threads/{}?since_seq={}",
        b.id, seqs[3]
    ));
    assert_eq!(r.json()["messages"], json!([]));
    assert_eq!(r.json()["more"], false);
    // read mark
    let r = a.patch(
        &format!("/api/v1/contacts/{}", b.id),
        json!({ "read_seq": seqs[3] }),
    );
    check_op("patchContact", &r);
    let r = a.get("/api/v1/messages/threads");
    assert_eq!(r.json()["threads"][0]["unread"], 0);
    // bad pages, unknown peers
    a.get(&format!("/api/v1/messages/threads/{}?limit=0", b.id))
        .problem(400, "bad_request");
    a.get(&format!("/api/v1/messages/threads/{}", "0".repeat(64)))
        .problem(404, "not_found");
    a.get("/api/v1/messages/threads/NOTHEX")
        .problem(404, "not_found");

    // retry: delivered ⇒ 409; unknown ⇒ 404
    let r = b.post(&format!("/api/v1/messages/{mid}/retry"), json!({}));
    check_op("retryMessage", &r);
    r.problem(409, "bad_request");
    b.post("/api/v1/messages/msg-999999/retry", json!({}))
        .problem(404, "not_found");
    // a queued message to a node1… contact can be retried (200 Message)
    let stranger = "f".repeat(64);
    let r = b.post("/api/v1/contacts", json!({ "node_id": stranger }));
    check_op("addContact", &r);
    assert_eq!(r.json()["has_key"], false);
    let r = b.post(
        "/api/v1/messages",
        json!({ "to": stranger, "text": "later" }),
    );
    assert_eq!(r.status, 202, "{}", r.body);
    let qid = r.json()["id"].as_str().unwrap().to_string();
    let r = b.post(&format!("/api/v1/messages/{qid}/retry"), json!({}));
    check_op("retryMessage", &r);
    assert_eq!(r.json()["state"], "queued");
    assert_eq!(r.json()["reason_code"], "waiting_for_key");
    // cancelMessage (lead's additive route): an own message that has not
    // left ⇒ 202, still `queued`, `reason_code: cancelled` (§4 tombstone)
    let r = b.post(&format!("/api/v1/messages/{qid}/cancel"), json!({}));
    check_op("cancelMessage", &r);
    assert_eq!(r.status, 202, "{}", r.body);
    assert_eq!(r.json()["id"], qid);
    assert_eq!(r.json()["state"], "queued");
    assert_eq!(r.json()["reason_code"], "cancelled");
    // delivered, incoming, unknown
    let r = b.post(&format!("/api/v1/messages/{mid}/cancel"), json!({}));
    check_op("cancelMessage", &r);
    r.problem(409, "bad_request");
    let incoming = got["id"].as_str().unwrap().to_string();
    a.post(&format!("/api/v1/messages/{incoming}/cancel"), json!({}))
        .problem(409, "bad_request");
    b.post("/api/v1/messages/msg-999999/cancel", json!({}))
        .problem(404, "not_found");
    b.post("/api/v1/messages/nope/cancel", json!({}))
        .problem(404, "not_found");
    // send refusals
    b.post(
        "/api/v1/messages",
        json!({ "to": "1".repeat(64), "text": "x" }),
    )
    .problem(404, "not_found");
    b.post("/api/v1/messages", json!({ "to": a.id, "text": "" }))
        .problem(400, "bad_request");
    b.post(
        "/api/v1/messages",
        json!({ "to": a.id.to_uppercase(), "text": "x" }),
    )
    .problem(400, "bad_request");
    // forget the node1… contact
    let r = b.delete(&format!("/api/v1/contacts/{stranger}"));
    check_op("forgetContact", &r);
    assert_eq!(r.status, 204);
    b.delete(&format!("/api/v1/contacts/{stranger}"))
        .problem(404, "not_found");
    // add-by-card conflicts and junk
    b.post("/api/v1/contacts", json!({ "card": "not base32 !!" }))
        .problem(400, "bad_request");
    b.post("/api/v1/contacts", json!({ "card": card, "node_id": a.id }))
        .problem(400, "bad_request");
    let r = b.post("/api/v1/contacts", json!({ "card": card }));
    check_op("addContact", &r); // already known: unchanged
    assert_eq!(r.json()["trust"], "enrolled");

    // settings: listener follows listen_lan immediately
    let r = b.patch(
        "/api/v1/fabric/settings",
        json!({ "listen_lan": true, "quotas": { "kb": 1000000 } }),
    );
    check_op("patchFabricSettings", &r);
    assert_eq!(r.json()["listen_lan"], true);
    assert_eq!(r.json()["quotas"]["kb"], 1_000_000);
    assert!(b.fabric.listen_addr().await.is_some(), "listen_lan binds");
    let r = b.patch(
        "/api/v1/fabric/settings",
        json!({ "listen_lan": false, "quotas": { "kb": null } }),
    );
    check_op("patchFabricSettings", &r);
    assert!(r.json()["quotas"]["kb"].is_null());
    assert!(b.fabric.listen_addr().await.is_none(), "off unbinds");
    b.patch("/api/v1/fabric/settings", json!({ "hub_role": "yes" }))
        .problem(400, "bad_request");
    // a node that holds no custody cannot offer
    b.post("/api/v1/fabric/hub/offer", json!({}))
        .problem(409, "bad_request");

    // the fabric service row reads LIVE on both ends
    until(5, "facts", || {
        let fa = (a.fabric.provider())()?;
        let fb = (b.fabric.provider())()?;
        (fa.store_open && fa.members == Some(3) && fb.hub_reachable == Some(true)).then_some(())
    })
    .await;

    // every SSE frame either node published is contract-valid
    let fa = drain(&mut sa, Duration::from_millis(400)).await;
    let fb = drain(&mut sb, Duration::from_millis(400)).await;
    check_frames(&fa);
    check_frames(&fb);
    assert!(fa.iter().any(|f| f.event == "contact"), "A saw B arrive");
    assert!(fb.iter().any(|f| f.event == "fabric"));
    let ours = message_frames(&fb, &mid);
    assert_eq!(ours[0]["change"], "new");
    assert!(
        ours.iter().any(|m| m["message"]["state"] == "delivered"),
        "{ours:?}"
    );

    for n in [&a, &b, &d, &e] {
        n.fabric.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn auth_matrix_session_bearer_nothing() {
    let a = node(Opts {
        hub: true,
        ..Opts::default()
    })
    .await;
    let b = node(Opts::default()).await;
    a.add(&b);
    let token = auth::DaemonToken::mint().unwrap();
    a.api.insert_ext(token.clone());
    let bearer = format!("Bearer {}", token.as_str());
    let cookie = a.session();
    let bid = b.id.clone();
    let other = "e".repeat(64);

    let reads = [
        "/api/v1/messages/threads".to_string(),
        format!("/api/v1/messages/threads/{bid}"),
        "/api/v1/contacts".to_string(),
        format!("/api/v1/contacts/{bid}"),
        "/api/v1/contacts/me/card".to_string(),
        "/api/v1/fabric".to_string(),
    ];
    for path in &reads {
        // session (paired LAN cookie)
        let r = http(a.lan, "GET", path, &[("Cookie", &cookie)], None);
        assert_eq!(r.status, 200, "session {path}: {}", r.body);
        // the agent's bearer, from loopback: reads are allowed
        let r = http(a.port, "GET", path, &[("Authorization", &bearer)], None);
        assert_eq!(r.status, 200, "bearer {path}: {}", r.body);
        // nothing
        http(a.lan, "GET", path, &[], None).problem(401, "unauthenticated");
    }

    // writes: a session reaches the handler; the bearer and nothing never do
    let writes: Vec<(&str, String, Value)> = vec![
        (
            "POST",
            "/api/v1/messages".into(),
            json!({ "to": bid, "text": "x" }),
        ),
        (
            "POST",
            "/api/v1/messages/msg-424242/retry".into(),
            json!({}),
        ),
        (
            "POST",
            "/api/v1/messages/msg-424242/cancel".into(),
            json!({}),
        ),
        (
            "POST",
            "/api/v1/contacts".into(),
            json!({ "node_id": other }),
        ),
        (
            "PATCH",
            format!("/api/v1/contacts/{bid}"),
            json!({ "pinned": true }),
        ),
        (
            "DELETE",
            format!("/api/v1/contacts/{}", "d".repeat(64)),
            Value::Null,
        ),
        (
            "POST",
            "/api/v1/fabric/hub/enroll".into(),
            json!({ "address": "nope", "code": "x" }),
        ),
    ];
    for (m, path, body) in &writes {
        let body = (!body.is_null()).then_some(body);
        let r = http(a.lan, m, path, &[("Cookie", &cookie)], body);
        assert!(
            ![401, 403].contains(&r.status),
            "session {m} {path}: {} {}",
            r.status,
            r.body
        );
        http(a.port, m, path, &[("Authorization", &bearer)], body).problem(401, "unauthenticated");
        http(a.lan, m, path, &[], body).problem(401, "unauthenticated");
    }

    // loopback-only: settings, offer, dismiss
    let lo: Vec<(&str, &str, Value)> = vec![
        (
            "PATCH",
            "/api/v1/fabric/settings",
            json!({ "custody_opt_in": true }),
        ),
        ("POST", "/api/v1/fabric/hub/offer", json!({})),
        ("DELETE", "/api/v1/fabric/hub/offer", Value::Null),
    ];
    for (m, path, body) in &lo {
        let body = (!body.is_null()).then_some(body);
        http(a.lan, m, path, &[("Cookie", &cookie)], body).problem(403, "loopback_only");
        http(a.port, m, path, &[("Authorization", &bearer)], body).problem(401, "unauthenticated");
        http(a.lan, m, path, &[], body).problem(401, "unauthenticated");
        let r = http(a.port, m, path, &[], body);
        assert!(
            r.status == 200 || r.status == 204,
            "loopback {m} {path}: {}",
            r.body
        );
    }
    // a bearer from a LAN peer is refused outright (C4 rule)
    http(
        a.lan,
        "GET",
        "/api/v1/fabric",
        &[("Authorization", &bearer)],
        None,
    )
    .problem(401, "unauthenticated");

    // the bearer cannot accept a message proposal either (proposals are
    // decided by the human; the decision route is a write behind the same
    // gate — the agent token passes require_session, so the fabric path
    // must not be reachable through it)
    write_proposal(&a, "prop-bearer", Some(&a.id), &format!("to: {bid}\n\nhi"));
    let before = thread_of(&a, &bid)["messages"].as_array().unwrap().len();
    let r = http(
        a.port,
        "POST",
        "/api/v1/proposals/prop-bearer/decision",
        &[("Authorization", &bearer)],
        Some(&json!({ "decision": "accepted" })),
    );
    r.problem(401, "unauthenticated");
    assert_eq!(
        thread_of(&a, &bid)["messages"].as_array().unwrap().len(),
        before
    );
    assert!(a.get("/api/v1/proposals/prop-bearer").json()["decision"].is_null());

    a.fabric.shutdown().await;
    b.fabric.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sse_coalesces_a_sixteen_fragment_transfer() {
    let a = node(Opts::default()).await;
    let b = node(Opts::default()).await;
    a.add(&b);
    b.add(&a);
    let (ea, _eb) = lossy_link(&a, &b, PathProfile::unknown(), LinkKind::KissModem).await;
    // the longest text the 160 B path still carries in Tier 1 (≤ 16 fragments)
    let (mut sa, mut sb) = (a.sub(), b.sub());
    let mut len = 1600;
    let (text, mid) = loop {
        let text: String = "abcdefghij".chars().cycle().take(len).collect();
        let r = a.post("/api/v1/messages", json!({ "to": b.id, "text": text }));
        if r.status == 202 {
            break (text, r.json()["id"].as_str().unwrap().to_string());
        }
        r.problem(400, "bad_request");
        len -= 25;
        assert!(len > 200, "no Tier 1 size found");
    };
    until(15, "delivered over 160 B", || {
        let t = thread_of(&a, &b.id);
        let m = t["messages"]
            .as_array()?
            .iter()
            .find(|m| m["id"] == mid)?
            .clone();
        (m["state"] == "delivered").then_some(())
    })
    .await;
    let frames_a = drain(&mut sa, Duration::from_millis(400)).await;
    let frames_b = drain(&mut sb, Duration::from_millis(400)).await;
    check_frames(&frames_a);
    check_frames(&frames_b);
    let frags = ea.stats().sent;
    assert!(
        frags >= 14,
        "a multi-fragment transfer ({frags} frames A→B)"
    );
    // sender: one frame per change (new, sent, delivered), never per fragment
    let own = message_frames(&frames_a, &mid);
    assert!(
        !own.is_empty() && own.len() <= 4,
        "{} frames for {frags} fragments: {own:?}",
        own.len()
    );
    assert_eq!(own[0]["change"], "new");
    assert_eq!(own.last().unwrap()["message"]["state"], "delivered");
    // receiver: exactly one `new` frame carrying the whole text
    let inc: Vec<Value> = frames_b
        .iter()
        .filter(|f| f.event == "message")
        .map(|f| serde_json::from_str::<Value>(&f.data).unwrap())
        .collect();
    assert_eq!(inc.len(), 1, "{inc:?}");
    assert_eq!(inc[0]["change"], "new");
    assert_eq!(inc[0]["message"]["text"], text);
    a.fabric.shutdown().await;
    b.fabric.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn message_proposals_are_checked_then_applied_once() {
    let a = node(Opts::default()).await;
    let b = node(Opts::default()).await;
    let c = node(Opts::default()).await;
    a.add(&b);
    // C writes first over a tcp-lan-kind link: A files a message request
    c.add(&a);
    lossy_link(&a, &c, PathProfile::tcp_lan(), LinkKind::TcpLan).await;
    let r = c.post("/api/v1/messages", json!({ "to": a.id, "text": "may I?" }));
    assert_eq!(r.status, 202, "{}", r.body);
    until(10, "request filed", || {
        let r = a.get(&format!("/api/v1/contacts/{}", c.id));
        (r.status == 200 && r.json()["request"] == true).then_some(())
    })
    .await;

    let (aid, bid, cid) = (a.id.clone(), b.id.clone(), c.id.clone());
    let cases = [
        (
            "prop-body",
            Some(aid.clone()),
            format!("to:{bid}\n\nhi"),
            502,
            "upstream_error",
        ),
        (
            "prop-stamp",
            Some(bid.clone()),
            format!("to: {bid}\n\nhi"),
            502,
            "upstream_error",
        ),
        (
            "prop-nostamp",
            None,
            format!("to: {bid}\n\nhi"),
            502,
            "upstream_error",
        ),
        (
            "prop-stranger",
            Some(aid.clone()),
            format!("to: {}\n\nhi", "9".repeat(64)),
            409,
            "bad_request",
        ),
        (
            "prop-request",
            Some(aid.clone()),
            format!("to: {cid}\n\nhi"),
            409,
            "bad_request",
        ),
    ];
    for (id, stamp, body, status, code) in &cases {
        write_proposal(&a, id, stamp.as_deref(), body);
        decide(&a, id, "accepted").problem(*status, code);
        // refused acceptance leaves it open
        let r = a.get(&format!("/api/v1/proposals/{id}"));
        check_op("getProposal", &r);
        assert!(r.json()["decision"].is_null(), "{id} stays open");
    }
    assert!(thread_of(&a, &bid)["messages"]
        .as_array()
        .unwrap()
        .is_empty());

    // a message proposal lists with kind `message` (KINDS)
    let r = a.get("/api/v1/proposals?status=open");
    check_op("listProposals", &r);
    assert!(r.json()["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["kind"] == "message"));

    // dismissing sends nothing
    write_proposal(&a, "prop-no", Some(&aid), &format!("to: {bid}\n\nnever"));
    let r = decide(&a, "prop-no", "dismissed");
    assert_eq!(r.json()["decision"]["decision"], "dismissed");

    // happy path: applied after the store commit, exactly once
    write_proposal(
        &a,
        "prop-ok",
        Some(&aid),
        &format!("to: {bid}\n\nline one\n  verbatim "),
    );
    let r = decide(&a, "prop-ok", "accepted");
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["decision"]["decision"], "accepted");
    let file: Value = serde_json::from_str(
        &std::fs::read_to_string(a.layout.root.join("agent/decisions/prop-ok.json")).unwrap(),
    )
    .unwrap();
    let mid = file["applied"]["message_id"].as_str().unwrap().to_string();
    let t = thread_of(&a, &bid);
    let msgs = t["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1, "exactly one message: {msgs:?}");
    assert_eq!(msgs[0]["id"], mid);
    assert_eq!(msgs[0]["text"], "line one\n  verbatim ");
    assert_eq!(msgs[0]["dir"], "own");
    decide(&a, "prop-ok", "accepted").problem(409, "already_decided");
    assert_eq!(thread_of(&a, &bid)["messages"].as_array().unwrap().len(), 1);

    // F2 review m6: the guard is the record on disk, not process memory. A
    // crash after the pending record (before or after the send) leaves the
    // proposal decided, `applied.pending` — a retried accept, in this or any
    // later daemon, never sends it.
    write_proposal(&a, "prop-crash", Some(&aid), &format!("to: {bid}\n\nonce"));
    std::fs::write(
        a.layout.root.join("agent/decisions/prop-crash.json"),
        json!({"schema": 1, "decision": "accepted", "at": "2026-10-15T08:00:00Z",
               "applied": {"pending": true}})
        .to_string(),
    )
    .unwrap();
    decide(&a, "prop-crash", "accepted").problem(409, "already_decided");
    assert_eq!(thread_of(&a, &bid)["messages"].as_array().unwrap().len(), 1);
    // the record is written before the send: no record, no send
    write_proposal(&a, "prop-ro", Some(&aid), &format!("to: {bid}\n\nnever"));
    let decisions = a.layout.root.join("agent/decisions");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&decisions, std::fs::Permissions::from_mode(0o500)).unwrap();
    let r = decide(&a, "prop-ro", "accepted");
    std::fs::set_permissions(&decisions, std::fs::Permissions::from_mode(0o700)).unwrap();
    r.problem(500, "internal");
    assert_eq!(thread_of(&a, &bid)["messages"].as_array().unwrap().len(), 1);
    assert!(a.get("/api/v1/proposals/prop-ro").json()["decision"].is_null());

    // the refused request proposal stayed open: once the human accepts C's
    // message request (PATCH accept), accepting it sends
    let t = thread_of(&a, &cid);
    assert_eq!(t["request"], true);
    assert_eq!(t["messages"][0]["text"], "may I?", "held, shown read-only");
    let r = a.patch(
        &format!("/api/v1/contacts/{cid}"),
        json!({ "accept": true }),
    );
    check_op("patchContact", &r);
    assert_eq!(r.json()["request"], false);
    a.patch(
        &format!("/api/v1/contacts/{cid}"),
        json!({ "accept": false }),
    )
    .problem(400, "bad_request");
    let r = decide(&a, "prop-request", "accepted");
    assert_eq!(r.status, 200, "{}", r.body);
    let t = thread_of(&a, &cid);
    assert_eq!(t["request"], false);
    let own: Vec<&Value> = t["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["dir"] == "own")
        .collect();
    assert_eq!(own.len(), 1);
    assert_eq!(own[0]["text"], "hi");

    for n in [&a, &b, &c] {
        n.fabric.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restore_pending_answers_409_and_seals_nothing() {
    let r = node(Opts {
        drop_counters: true,
        ..Opts::default()
    })
    .await;
    let b = node(Opts::default()).await;
    assert!(r.fabric.sealer().unwrap().restore_pending());
    assert!(r.fabric.own_announce().is_empty(), "no announce minted");
    assert!(r.fabric.tcp().is_none(), "no tcp-lan without an announce");
    let st = r.get("/api/v1/fabric");
    check_op("getFabric", &st);
    // no card to show yet, so no hub offer either (its bundle carries it)
    r.get("/api/v1/contacts/me/card")
        .problem(409, "restore_pending");
    let o = r.post("/api/v1/fabric/hub/offer", json!({}));
    check_op("offerHub", &o);
    o.problem(409, "restore_pending");
    // contacts still work (nothing is sealed to add one)
    r.add(&b);
    let resp = r.post("/api/v1/messages", json!({ "to": b.id, "text": "hi" }));
    check_op("sendMessage", &resp);
    resp.problem(409, "restore_pending");
    write_proposal(&r, "prop-rp", Some(&r.id), &format!("to: {}\n\nhi", b.id));
    decide(&r, "prop-rp", "accepted").problem(409, "restore_pending");
    let p = r.get("/api/v1/proposals/prop-rp");
    assert!(p.json()["decision"].is_null());
    assert!(thread_of(&r, &b.id)["messages"]
        .as_array()
        .unwrap()
        .is_empty());
    let st = r.get("/api/v1/fabric").json();
    assert_eq!(st["store"]["restore_pending"], true);
    assert_eq!(b.get("/api/v1/fabric").json()["store"]["restore_pending"], false);
    r.fabric.shutdown().await;
    b.fabric.shutdown().await;
    // F2 review M1: the clean stop wrote no counters file (no reset)
    let dir = r.layout.identity_dir();
    assert!(!dir.join("counters").exists());
    assert!(id111::counters::CounterStore::restore_pending(&dir).is_ok());
}

/// F2 review M1, end to end: a node whose `counters` file went missing
/// while a valid announce is stored (and whose settings ask for a listener
/// and a direct peer) runs no link at all, presents no card, reports
/// `restore_pending` in `GET /fabric` and the service facts, and its clean
/// stop leaves the identity dir state-absent — the floor stays settable and
/// nothing re-issues announce counter 1.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restore_pending_with_a_stored_announce_runs_no_link_and_resets_nothing() {
    let other = node(Opts::default()).await;
    let peers = json!({
        "schema": 1, "hub_role": true, "listen_lan": true,
        "peers": [{ "node_id": other.id, "address": "127.0.0.1:9" }],
    })
    .to_string();
    let o = Opts {
        hub: true,
        settings: Some(peers),
        ..Opts::default()
    };
    let mut r = node(o.clone()).await;
    // a normal first run: an announce is minted and stored, tcp-lan runs
    let stored = r.fabric.own_announce();
    assert!(!stored.is_empty());
    assert!(r.fabric.tcp().is_some() && r.fabric.listen_addr().await.is_some());
    let dir = r.layout.identity_dir();
    // after the clean stop the file goes missing; `111d`'s own identity
    // check would refuse such a dir at start, so the identity is read first
    let again = r
        .restart(o, |l| {
            let dir = l.identity_dir();
            let identity = node111::identity::load(&dir).unwrap();
            std::fs::remove_file(dir.join("counters")).unwrap();
            Some(identity)
        })
        .await;
    assert_eq!(
        std::fs::read(r.layout.root.join("fabric/announce")).unwrap(),
        stored,
        "the valid announce is still on disk"
    );
    assert!(again.restore_pending());
    assert!(again.own_announce().is_empty(), "never presented while pending");
    assert!(again.tcp().is_none(), "no tcp-lan: no dial, no peers");
    assert!(again.listen_addr().await.is_none(), "binds nothing");
    r.get("/api/v1/contacts/me/card")
        .problem(409, "restore_pending");
    r.post("/api/v1/fabric/hub/offer", json!({}))
        .problem(409, "restore_pending");
    let st = r.get("/api/v1/fabric");
    check_op("getFabric", &st);
    assert_eq!(st.json()["store"]["restore_pending"], true);
    assert_eq!(st.json()["links"], json!([]));
    let facts = (again.provider())().unwrap();
    assert!(facts.store_open && facts.restore_pending, "{facts:?}");
    again.shutdown().await;
    assert!(!dir.join("counters").exists(), "no counter reset on disk");
    let mut floor = id111::counters::CounterStore::restore_pending(&dir).unwrap();
    assert!(matches!(
        floor.next_announce_counter(),
        Err(id111::counters::CounterError::RestoreFloorRequired)
    ));
    other.fabric.shutdown().await;
}

/// F2 review m9 (test gap): the production rule — loopback is never a peer
/// (§2) — on enrollment (literal and resolved name), on dials (`peers`) and
/// on accepts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refuse_loopback_refuses_loopback_dials_accepts_and_enrollment() {
    let hub = node(Opts {
        hub: true,
        ..Opts::default()
    })
    .await;
    let hub_addr = hub.fabric.listen_addr().await.unwrap();
    let card = hub.card();
    let p = node(Opts {
        refuse_loopback: true,
        ..Opts::default()
    })
    .await;
    assert!(p.fabric.refuse_loopback());
    for address in [
        hub_addr.to_string(),
        format!("localhost:{}", hub_addr.port()),
        format!("[::ffff:127.0.0.1]:{}", hub_addr.port()),
    ] {
        let r = p.post(
            "/api/v1/fabric/hub/enroll",
            json!({ "address": address, "code": "ABCDEFGH", "card": card }),
        );
        check_op("enrollHub", &r);
        r.problem(400, "bad_request");
        assert!(r.json()["detail"].as_str().unwrap().contains("loopback"), "{}", r.body);
    }
    for address in ["0.0.0.0:11183", "[::]:11183", "224.0.0.1:11183"] {
        p.post(
            "/api/v1/fabric/hub/enroll",
            json!({ "address": address, "code": "ABCDEFGH", "card": card }),
        )
        .problem(400, "bad_request");
    }
    assert!(p.fabric.persisted().fabric.home_hub.is_none());
    // accepts: P listens (its rule on); the hub dials it from loopback and is
    // refused at accept — no connection ever comes up on either side
    let r = p.patch("/api/v1/fabric/settings", json!({ "listen_lan": true }));
    assert_eq!(r.status, 200, "{}", r.body);
    let p_addr = p.fabric.listen_addr().await.expect("P listens");
    use mesh111::link::Dialer;
    let p_id = p.fabric.fabric().unwrap().node_id();
    hub.fabric.tcp().unwrap().maintain(p_addr, Some(p_id));
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(p.fabric.tcp().unwrap().connection_count(), 0, "accept refused");
    assert_eq!(hub.fabric.tcp().unwrap().connection_count(), 0);
    // dials: a loopback `peers` entry makes the file invalid under the rule
    // (moved aside, everything off) — nothing is dialed
    let hub_id = hub.id.clone();
    let settings = json!({
        "schema": 1, "listen_lan": true,
        "peers": [{ "node_id": hub_id, "address": hub_addr.to_string() }],
    })
    .to_string();
    let q = node(Opts {
        refuse_loopback: true,
        settings: Some(settings),
        ..Opts::default()
    })
    .await;
    assert!(q.fabric.persisted().peers.is_empty());
    assert!(!q.fabric.persisted().fabric.listen_lan, "fail closed");
    assert!(q.layout.root.join("fabric/settings.invalid-1").exists());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(q.fabric.tcp().unwrap().connection_count(), 0);
    for n in [&hub, &p, &q] {
        n.fabric.shutdown().await;
    }
}

/// F2 review m9 (test gap): hand-configured direct `peers` — two members on
/// one LAN without a hub; one listens (`listen_lan`), the other keeps a dial
/// to it from its settings file, and a message is delivered over that link.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn peers_from_settings_dial_a_listen_lan_node() {
    let a = node(Opts::default()).await;
    let r = a.patch("/api/v1/fabric/settings", json!({ "listen_lan": true }));
    assert_eq!(r.status, 200, "{}", r.body);
    let a_addr = a.fabric.listen_addr().await.expect("listen_lan binds");
    let settings = json!({
        "schema": 1,
        "peers": [{ "node_id": a.id, "address": a_addr.to_string() }],
    })
    .to_string();
    let b = node(Opts {
        settings: Some(settings),
        ..Opts::default()
    })
    .await;
    assert_eq!(b.fabric.persisted().peers.len(), 1);
    assert!(b.fabric.listen_addr().await.is_none(), "B only dials");
    until(10, "B's dial to A is up", || {
        (b.fabric.tcp()?.connection_count() == 1).then_some(())
    })
    .await;
    a.add(&b);
    b.add(&a);
    let r = b.post("/api/v1/messages", json!({ "to": a.id, "text": "direct" }));
    assert_eq!(r.status, 202, "{}", r.body);
    let mid = r.json()["id"].as_str().unwrap().to_string();
    until(10, "delivered over the peers link", || {
        let t = thread_of(&b, &a.id);
        let m = t["messages"]
            .as_array()?
            .iter()
            .find(|m| m["id"] == mid)?
            .clone();
        (m["state"] == "delivered").then_some(())
    })
    .await;
    assert_eq!(thread_of(&a, &b.id)["messages"][0]["text"], "direct");
    a.fabric.shutdown().await;
    b.fabric.shutdown().await;
}

/// F2 review m3: a corrupt `fabric/settings` is moved aside and the member's
/// home hub comes back from its `enrolled` contact (with the address when
/// it can be salvaged from the bad file).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn corrupt_settings_are_moved_aside_and_the_home_hub_rebuilt() {
    let hub = node(Opts {
        hub: true,
        ..Opts::default()
    })
    .await;
    let hub_addr = hub.fabric.listen_addr().await.unwrap();
    let mut m = node(Opts::default()).await;
    enroll_by_route(&m, &hub);
    let settings = m.layout.root.join("fabric/settings");
    // one byte of damage: unparsable — the hub is rebuilt, address unknown
    let mut bytes = std::fs::read(&settings).unwrap();
    bytes[0] = b'#';
    let damaged = bytes.clone();
    let again = m
        .restart(Opts::default(), |l| {
            std::fs::write(l.root.join("fabric/settings"), &damaged).unwrap();
            None
        })
        .await;
    let h = again.persisted().fabric.home_hub.expect("rebuilt");
    assert_eq!(fjson::hex(&h.node_id), hub.id);
    assert_eq!(h.address, None);
    assert_eq!(
        std::fs::read(m.layout.root.join("fabric/settings.invalid-1")).unwrap(),
        bytes,
        "the bad file is kept"
    );
    let st = m.get("/api/v1/fabric").json();
    assert_eq!(st["hub"]["enrolled"], true);
    assert_eq!(st["hub"]["node_id"], hub.id);
    let saved = std::fs::read_to_string(&settings).unwrap();
    assert!(saved.contains(&hub.id), "rewritten: {saved}");
    // a type error elsewhere: the address is salvaged too
    let typed = json!({ "schema": 1, "hub_role": "yes",
                        "home_hub": { "node_id": hub.id, "address": hub_addr.to_string() } })
    .to_string();
    let again = m
        .restart(Opts::default(), |l| {
            std::fs::write(l.root.join("fabric/settings"), &typed).unwrap();
            None
        })
        .await;
    let h = again.persisted().fabric.home_hub.expect("rebuilt");
    assert_eq!(h.address, Some(hub_addr));
    assert!(!again.persisted().fabric.hub_role, "fail closed");
    assert!(m.layout.root.join("fabric/settings.invalid-2").exists());
    until(10, "the hub is reachable again", || {
        let st = m.get("/api/v1/fabric").json();
        (st["hub"]["reachable"] == true).then_some(())
    })
    .await;
    // the hub itself: its enrolled contacts are members — no home hub
    let mut hub = hub;
    let h2 = hub
        .restart(
            Opts {
                hub: true,
                ..Opts::default()
            },
            |l| {
                std::fs::write(l.root.join("fabric/settings"), "{broken").unwrap();
                None
            },
        )
        .await;
    assert!(h2.persisted().fabric.home_hub.is_none());
    m.fabric.shutdown().await;
    hub.fabric.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restart_keeps_the_announce_store_and_settings() {
    // a clean stop writes the store's clean record and the counters' clean
    // flag; the next start keeps the stored announce (never re-minted)
    let a = node(Opts::default()).await;
    let first = a.fabric.own_announce();
    let r = a.patch(
        "/api/v1/fabric/settings",
        json!({ "custody_opt_in": true, "listen_lan": true }),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    // F2 review m7: the roles changed, so the announce is re-minted now —
    // the own card and every new tcp-lan connection carry it
    let now = a.fabric.own_announce();
    assert_ne!(now, first, "re-minted on the settings change");
    assert_eq!(a.card(), mesh111::contacts::card_text(&now));
    let at = a.fabric.listen_addr().await.unwrap();
    assert_eq!(opening_announce(at).await, now);
    a.fabric.shutdown().await;
    // custody_opt_in changes the announce roles: the next start re-mints
    let id = node111::identity::load(&a.layout.identity_dir()).unwrap();
    let again = fabric::start(test_config(&a.layout, Opts::default()), Some(id), &a.api).await;
    assert!(again.running(), "{:?}", again.down_reason());
    let second = again.own_announce();
    assert_ne!(first, second, "roles changed ⇒ a new announce");
    assert_eq!(second, now, "the re-minted announce was stored");
    assert!(again.persisted().fabric.custody_opt_in);
    again.shutdown().await;
    let id = node111::identity::load(&a.layout.identity_dir()).unwrap();
    let third = fabric::start(test_config(&a.layout, Opts::default()), Some(id), &a.api).await;
    assert_eq!(
        third.own_announce(),
        second,
        "unchanged roles ⇒ the stored announce"
    );
    third.shutdown().await;
}

#[test]
fn contract_codes_pass_the_validator() {
    use mesh111::fabric::MessageView;
    use mesh111::outbox::Reason;
    use mesh111::store::{Dir, MsgState};
    for (reason, state) in [
        (Reason::NotEnrolled, MsgState::Sent),
        (Reason::Cancelled, MsgState::Queued),
        (Reason::RestorePending, MsgState::Queued),
    ] {
        let v = MessageView {
            id: 7,
            peer: id111::keyset::NodeId::from([0x2a; 32]),
            dir: Dir::Own,
            kind: 1,
            text: Some(String::new()),
            tombstone: true,
            state,
            reason: Some(reason),
            local_seq: 3,
            received_seq: None,
            sent_at_claim: None,
            stream: 0,
            seq: mesh111::ids::MsgSeq::new(5, 1),
        };
        let m = fjson::message(&v, 1_760_000_000);
        assert_schema("Message", &m);
        assert_schema(
            "SseMessage",
            &json!({ "change": "state", "peer": m["peer"], "message": m }),
        );
        assert_eq!(m["reason_code"], reason.code());
    }
    // the emitted set names both codes, and Problem.code has restore_pending
    let yaml = std::fs::read_to_string(format!(
        "{}/../../../40-interface/api/openapi.yaml",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let line = yaml
        .lines()
        .find(|l| l.contains("reason_code: { type: [string, \"null\"]"))
        .expect("Message.reason_code");
    assert!(
        line.contains("not_enrolled (") && line.contains("cancelled ("),
        "{line}"
    );
    let codes = &contract().schemas["Problem"]["properties"]["code"]["enum"];
    assert!(codes
        .as_array()
        .unwrap()
        .contains(&json!("restore_pending")));
    let kinds = &contract().schemas["Proposal"]["properties"]["kind"]["enum"];
    assert!(kinds.as_array().unwrap().contains(&json!("message")));
    // patterns the validator relies on
    assert!(re_match("^[0-9a-f]{64}$", &"a".repeat(64)));
    assert!(!re_match("^[0-9a-f]{64}$", &"a".repeat(63)));
    assert!(re_match("^node1[a-z0-9]+$", "node1qq"));
    assert!(!re_match("^node1[a-z0-9]+$", "node1"));
    assert!(re_match("^[A-Za-z0-9_-]{1,128}$", "msg-7"));
    assert!(!re_match("^[01]+$", "012"));
    assert!(date_time_ok("2026-10-14T10:00:00Z") && !date_time_ok("2026-10-14"));
}
