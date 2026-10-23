//! Layer 6 C1 acceptance (`os/6-transport/README.md` §8, §9, §10, §12, §13; D26 #2;
//! `6-transport/TASKS.md` row C1): the bridge routes, the proposal kinds and the daemon
//! wiring, driven through the real dashboard app on loopback sockets over a real fabric (a
//! fresh `id111` identity, SQLCipher store V3) with scripted `StubAdapter`s.
//!
//! - every route's 2xx body and every problem against the contract digest
//!   (`4-interface/ui/test/contract.gen.ts`) with the UI's JSON-Schema-subset validator
//!   (`oneOf` = any branch);
//! - networks (a disabled optional component listed with its cached size), conversations,
//!   thread paging, a foreign DM opened, a person's read clearing `unread`;
//! - direct sends (202 with the plan and the adapter-state reason, 409 oversize, split rows
//!   one per part), the fan-out preview that writes and logs nothing (stderr captured), the
//!   atomic fan-out refusal;
//! - relay edges through the 409 consent round trip, DM / `src = dst` refused, pause,
//!   resume, the two options, delete;
//! - settings: HAM and uplink confirmations through the 409 round trip with the text hash,
//!   the adapter switch;
//! - components: enable refused without the licence acknowledgement, enabled with it,
//!   disabled, files removed;
//! - proposals: `bridge_message` accept queues and records, a consent gate keeps it open
//!   until the decision echoes the hash, `bridge_edge` accept creates the edge, `device`
//!   identify runs the consented entry point, `service.restart` restarts a supervised child
//!   and is `not_supervised` otherwise, unknown and seam actions are `unsupported_action`,
//!   a malformed body `upstream_error`, a stamp for another node refused;
//! - the agent's bearer reads (`GET /networks`) and is refused (`401`) on every Layer 6
//!   write;
//! - SSE `network_message` and `bridge` frames observed on `/events`;
//! - a fresh Layer 6 wiring spawns nothing: no recv task, no runtime task, nothing enabled
//!   (the LXMF seam and the Nearby sink registered all the same);
//! - W1: `POST /messages/fm-…/retry|cancel` (`Bridge::resend` / `Bridge::cancel`; a `msg-…`
//!   id keeps the Layer 5 path), and the LXMF seam end to end against a scripted helper —
//!   the client config written with rnsd's `rpc_key` only once the notice passes and a
//!   spine exists (fail closed otherwise), the run arguments, the adapter attached and
//!   live in `GET /networks`, its `heard` merged into `GET /nearby` as a `reticulum`
//!   source with the SSE `nearby` frame, and disable removing the network, the source and
//!   the child.

use axum::extract::connect_info::MockConnectInfo;
use node111::daemon::bridge::{
    AccountFacts, AdapterFacts, Auth, Bridge, BridgeConfig, ChannelFacts, ConvKind, ConvRef,
    Inbound, InboundText, StubAdapter,
};
use node111::daemon::components::manifest::{self, Component};
use node111::daemon::components::registry::ComponentState;
use node111::daemon::components::runner::RunnerConfig;
use node111::daemon::components::{Components, ComponentsConfig, Dependencies, Wiring};
use node111::daemon::dashboard::api_v1::auth::DaemonToken;
use node111::daemon::dashboard::{
    routes, serve_listener, ApiConfig, ApiState, DashboardState, SharedState,
};
use node111::daemon::fabric::{self, json as fjson, FabricConfig, FabricHandle};
use node111::daemon::ports::PortTable;
use node111::daemon::services::{ServiceId, Supervised};
use node111::daemon::supervise::{spawn_child, BackoffPolicy, ChildControl, ChildSpec};
use node111::daemon::DaemonLayout;
use node111::util::hash::sha256_bytes;
use serde_json::{json, Map, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

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

/// A port of `4-interface/ui/test/helpers/validate.ts` (same subset, `oneOf` = any).
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

/// Read `/api/v1/events` (replaying after `last`) until `done` sees the frames or the
/// deadline passes; returns the raw stream text.
fn sse_after(port: u16, last: u64, done: impl Fn(&str) -> bool) -> String {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let req = format!(
        "GET /api/v1/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nLast-Event-ID: {last}\r\n\r\n"
    );
    conn.write_all(req.as_bytes()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    while Instant::now() < deadline {
        match conn.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(_) => {}
        }
        if done(&String::from_utf8_lossy(&raw)) {
            break;
        }
    }
    String::from_utf8_lossy(&raw).into_owned()
}

/// `(event, data)` of every SSE frame in a raw stream (chunk sizes are skipped: they
/// never start with `event:` or `data:`).
fn frames(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let (mut ev, mut data) = (String::new(), String::new());
    for line in raw.lines() {
        if let Some(e) = line.strip_prefix("event: ") {
            ev = e.to_string();
        } else if let Some(d) = line.strip_prefix("data: ") {
            data = d.to_string();
        } else if line.is_empty() && !ev.is_empty() {
            out.push((std::mem::take(&mut ev), std::mem::take(&mut data)));
        }
    }
    out
}

// ---------------------------------------------------------------- nodes

const LAN_PEER: &str = "192.168.1.50:40000";

/// The demo optional component (a binary payload that says ready, then echoes), on the
/// `app:deltachat` network (a declared adapter manifest).
const BIN: &[u8] = b"#!/bin/sh\necho '{\"type\":\"ready\",\"dest\":\"00\"}'\nexec cat\n";

fn demo_component() -> Component {
    let toml = format!(
        r#"id = "demo"
adapter = "app:deltachat"
title = "Demo peer"
[payload]
kind = "binary"
url = "https://example.invalid/dl/demo-bin"
sha256 = "{sha}"
size = {size}
[run]
program = "demo-bin"
ports = []
health = "ready"
health_timeout_secs = 5
[license]
spdx = "MPL-2.0"
read_at = "https://example.invalid/LICENSE"
notice = "This installs the demo peer (MPL-2.0); read its licence at https://example.invalid/LICENSE. You install it."
"#,
        sha = sha256_bytes(BIN),
        size = BIN.len(),
    );
    let mut all = manifest::load(&[("demo.toml".to_string(), toml.into_bytes())]);
    assert_eq!(all.len(), 1);
    all.pop().unwrap()
}

struct NoDeps;

impl Dependencies for NoDeps {
    fn child(&self, _: ServiceId) -> Option<ChildControl> {
        None
    }
    fn serving(&self, _: ServiceId) -> bool {
        false
    }
}

struct Node {
    api: Arc<ApiState>,
    fabric: Arc<FabricHandle>,
    bridge: Arc<Bridge>,
    components: Components,
    token: DaemonToken,
    /// Trusted loopback.
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

async fn node() -> Node {
    node_with(vec![demo_component()]).await
}

/// A node whose optional components are `declared`.
async fn node_with(declared: Vec<Component>) -> Node {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    std::fs::create_dir_all(dir.path().join("current")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let identity = node111::identity::generate(&layout.identity_dir()).unwrap();
    let id = identity.node_id_hex();
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.19")));
    dash.write().unwrap().node_id = Some(id.clone());
    let mut acfg = ApiConfig::new(layout.clone(), false);
    acfg.persist_auth = false;
    acfg.trust_loopback = true;
    let api = ApiState::new(dash, acfg);
    let mut c = FabricConfig::production(&layout, None);
    c.listen = "127.0.0.1:0".parse().unwrap();
    c.housekeeping = Duration::from_secs(3600);
    let fabric = fabric::start(c, Some(identity), &api).await;
    assert!(fabric.running(), "{:?}", fabric.down_reason());
    api.insert_ext(Arc::clone(&fabric));
    let mut ccfg = ComponentsConfig::new(&layout.root);
    ccfg.runner = RunnerConfig {
        retry: Duration::from_millis(50),
        retry_max: Duration::from_millis(200),
        settle: Duration::ZERO,
        grace: Duration::from_secs(2),
    };
    let components = Components::new(
        ccfg,
        declared,
        Wiring {
            cmd: Arc::new(node111::testkit::MockRunner::new(Vec::<(
                String,
                node111::util::run::Output,
            )>::new())),
            ports: Arc::new(PortTable::new()),
            deps: Arc::new(NoDeps),
            tick: Arc::new(|| 1),
        },
    );
    let bridge = Bridge::new(BridgeConfig::new(&layout.root), Arc::clone(&fabric), &api);
    bridge.set_components(Arc::new(components.clone()));
    api.insert_ext(Arc::clone(&bridge));
    api.insert_ext(components.clone());
    let token = DaemonToken::mint().unwrap();
    api.insert_ext(token.clone());
    let port = serve(&api, None).await;
    let lan = serve(&api, Some(LAN_PEER)).await;
    Node {
        api,
        fabric,
        bridge,
        components,
        token,
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
    fn bearer(&self) -> String {
        format!("Bearer {}", self.token.as_str())
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

    fn fab(&self) -> mesh111::fabric::Fabric {
        self.fabric.fabric().unwrap().clone()
    }

    async fn conv(&self, r: &ConvRef) -> i64 {
        let kind = match r.kind {
            ConvKind::Channel => mesh111::store::ForeignKind::Channel,
            ConvKind::Dm => mesh111::store::ForeignKind::Dm,
            ConvKind::Group => mesh111::store::ForeignKind::Group,
        };
        self.fab()
            .foreign_conversation_upsert(&r.network, &r.account, kind, &r.native_id, None)
            .await
            .unwrap()
    }

    async fn rows(&self, conv: i64) -> Vec<mesh111::store::ForeignMessageRow> {
        self.fab()
            .foreign_thread(conv, None, 10_000)
            .await
            .unwrap()
            .messages
    }

    async fn wait_rows(&self, conv: i64, n: usize) -> Vec<mesh111::store::ForeignMessageRow> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let got = self.rows(conv).await;
            if got.len() >= n {
                return got;
            }
            assert!(Instant::now() < deadline, "{} of {n} rows", got.len());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Write an agent proposal file (`kind`, `body`, stamped for `stamp`).
    fn propose(&self, id: &str, kind: &str, body: &str, stamp: Option<&str>) {
        let dir = self.layout.root.join("agent/proposals");
        std::fs::create_dir_all(&dir).unwrap();
        let v = json!({
            "schema": 1, "id": id, "node_id": stamp, "task_id": null, "kind": kind,
            "title": format!("t-{id}"), "body": body, "created_at": "2026-10-19T10:00:00Z",
        });
        std::fs::write(dir.join(format!("{id}.json")), v.to_string()).unwrap();
    }

    fn decide(&self, id: &str, accept: Option<&str>) -> Resp {
        let mut b = json!({ "decision": "accepted" });
        if let Some(h) = accept {
            b["accept_warning"] = json!(h);
        }
        self.post(&format!("/api/v1/proposals/{id}/decision"), b)
    }

    fn decision_file(&self, id: &str) -> Option<Value> {
        let p = self.layout.root.join(format!("agent/decisions/{id}.json"));
        std::fs::read_to_string(p)
            .ok()
            .map(|t| serde_json::from_str(&t).unwrap())
    }
}

fn stub(id: &'static str, label: &str, tag: &str, account: &str) -> Arc<StubAdapter> {
    Arc::new(
        StubAdapter::new(id, StubAdapter::default_caps(label, tag)).with_facts(AdapterFacts {
            accounts: vec![AccountFacts {
                id: account.into(),
                label: None,
                licensed: false,
            }],
            region_set: Some(true),
            ..AdapterFacts::default()
        }),
    )
}

fn cref(net: &str, account: &str, kind: ConvKind, native: &str) -> ConvRef {
    ConvRef {
        network: net.into(),
        account: account.into(),
        kind,
        native_id: native.into(),
    }
}

fn text(conv: &ConvRef, sender: &str, name: &str, id: &str, body: &str) -> Inbound {
    Inbound::Text(InboundText {
        conv: conv.clone(),
        sender_native: sender.into(),
        sender_name: name.into(),
        auth: Auth::ChannelKey,
        native_msg_id: id.into(),
        body: body.into(),
        claim_ts: None,
        meta: json!({}),
    })
}

/// Two stub networks with one channel each.
struct Pair {
    n: Node,
    a: Arc<StubAdapter>,
    b: Arc<StubAdapter>,
    ca: ConvRef,
    src: i64,
    dst: i64,
}

async fn pair() -> Pair {
    let n = node().await;
    let a = stub("stub-a", "Stub A", "A", "na");
    let b = stub("stub-b", "Stub B", "B", "nb");
    n.bridge.enable(a.clone()).await.unwrap();
    n.bridge.enable(b.clone()).await.unwrap();
    let ca = cref("stub-a", "na", ConvKind::Channel, "ch:alpha");
    let cb = cref("stub-b", "nb", ConvKind::Channel, "ch:beta");
    let src = n.conv(&ca).await;
    let dst = n.conv(&cb).await;
    Pair {
        n,
        a,
        b,
        ca,
        src,
        dst,
    }
}

// ---------------------------------------------------------------- networks

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn networks_conversations_threads_and_a_foreign_dm() {
    let p = pair().await;
    let n = &p.n;
    // the demo component is cached, not installed: listed, disabled, with its size
    let cache = n.layout.root.join("cache/packages/demo");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("demo-bin"), BIN).unwrap();

    let r = n.get("/api/v1/networks");
    check_op("listNetworks", &r);
    let v = r.json();
    let nets = v["networks"].as_array().unwrap();
    let dc = nets.iter().find(|x| x["id"] == "app:deltachat").unwrap();
    assert_eq!(dc["state"], "disabled");
    assert_eq!(dc["component"]["id"], "demo");
    assert_eq!(dc["component"]["state"], "cached");
    assert_eq!(dc["component"]["cached_bytes"], BIN.len());
    assert!(dc["component"]["installed_bytes"].is_null());
    assert!(dc["component"].get("title").is_none(), "no extra fields");
    let sa = nets.iter().find(|x| x["id"] == "stub-a").unwrap();
    assert_eq!(sa["state"], "live");
    // the agent's bearer reads
    let r = http(
        n.port,
        "GET",
        "/api/v1/networks",
        &[("Authorization", &n.bearer())],
        None,
    );
    assert_eq!(r.status, 200, "{}", r.body);

    // inbound traffic → conversations → a paged thread
    for i in 0..3 {
        p.a.feed(text(
            &p.ca,
            "!a1",
            "Alice",
            &format!("{i}"),
            &format!("hello {i}"),
        ));
    }
    n.wait_rows(p.src, 3).await;
    let r = n.get("/api/v1/networks/stub-a/conversations");
    check_op("listForeignConversations", &r);
    let convs = r.json()["conversations"].as_array().unwrap().clone();
    assert!(convs.iter().any(|c| c["id"] == fjson::fc_id(p.src)));
    n.get("/api/v1/networks/nowhere/conversations")
        .problem(404, "not_found");
    n.get("/api/v1/networks/NOPE/conversations")
        .problem(404, "not_found");

    let path = format!(
        "/api/v1/networks/stub-a/conversations/{}",
        fjson::fc_id(p.src)
    );
    let r = n.get(&format!("{path}?limit=2"));
    check_op("getForeignConversation", &r);
    let t = r.json();
    let msgs = t["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(t["more"], true);
    assert_eq!(msgs[1]["text"], "hello 2");
    assert_eq!(msgs[0]["uncertain"], false);
    // a person read the newest window: unread is cleared, the agent's read never does it
    let first_seq = msgs[0]["seq"].as_u64().unwrap();
    let r = n.get(&format!("{path}?since_seq=0&limit=1"));
    check_op("getForeignConversation", &r);
    assert_eq!(r.json()["messages"][0]["text"], "hello 0");
    assert!(r.json()["messages"][0]["seq"].as_u64().unwrap() < first_seq);
    let unread = |n: &Node| {
        n.get("/api/v1/networks/stub-a/conversations").json()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == fjson::fc_id(p.src))
            .unwrap()["unread"]
            .clone()
    };
    // channel traffic does not count unread here (only DMs do), so check a DM below
    let _ = unread(n);
    n.get(&format!("{path}?limit=0"))
        .problem(400, "bad_request");
    n.get(&format!("{path}?limit=501"))
        .problem(400, "bad_request");
    n.get("/api/v1/networks/stub-b/conversations/fc-1")
        .problem(404, "not_found");
    n.get("/api/v1/networks/stub-a/conversations/ch:alpha")
        .problem(404, "not_found");

    // open a foreign DM: idempotent, validated, never a contact
    let r = n.post(
        "/api/v1/networks/stub-a/conversations",
        json!({"kind": "dm", "native_id": "!0badc0de"}),
    );
    check_op("openForeignConversation", &r);
    let dm = r.json();
    assert_eq!(
        (dm["kind"].as_str(), dm["native_id"].as_str()),
        (Some("dm"), Some("dm:!0badc0de"))
    );
    let again = n.post(
        "/api/v1/networks/stub-a/conversations",
        json!({"kind": "dm", "native_id": "!0badc0de"}),
    );
    assert_eq!(again.json()["id"], dm["id"]);
    n.post(
        "/api/v1/networks/stub-a/conversations",
        json!({"kind": "channel", "native_id": "x"}),
    )
    .problem(400, "bad_request");
    n.post(
        "/api/v1/networks/stub-a/conversations",
        json!({"kind": "dm", "native_id": "a b"}),
    )
    .problem(400, "bad_request");
    n.post(
        "/api/v1/networks/stub-a/conversations",
        json!({"kind": "dm", "native_id": "!x", "extra": 1}),
    )
    .problem(400, "bad_request");
    n.post(
        "/api/v1/networks/app%3Adeltachat/conversations",
        json!({"kind": "dm", "native_id": "a@b"}),
    )
    .problem(409, "component_unavailable");
    let r = http(
        n.port,
        "POST",
        "/api/v1/networks/stub-a/conversations",
        &[("Authorization", &n.bearer())],
        Some(&json!({"kind": "dm", "native_id": "!0badc0de"})),
    );
    r.problem(401, "unauthenticated");
    // a DM to us raises unread; a person's read of the newest window clears it
    let dm_id = fjson::parse_fc_id(dm["id"].as_str().unwrap()).unwrap();
    let dm_ref = cref("stub-a", "na", ConvKind::Dm, "dm:!0badc0de");
    p.a.feed(text(&dm_ref, "!0badc0de", "Bob", "d1", "psst"));
    n.wait_rows(dm_id, 1).await;
    let dm_path = format!(
        "/api/v1/networks/stub-a/conversations/{}",
        dm["id"].as_str().unwrap()
    );
    let r = http(
        n.port,
        "GET",
        &dm_path,
        &[("Authorization", &n.bearer())],
        None,
    );
    assert_eq!(
        r.json()["conversation"]["unread"],
        1,
        "the agent's read leaves it"
    );
    let r = n.get(&dm_path);
    assert_eq!(r.json()["conversation"]["unread"], 0);
    let r = n.get(&dm_path);
    assert_eq!(r.json()["conversation"]["unread"], 0, "stays read");
}

// ---------------------------------------------------------------- sends

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn direct_sends_queue_with_the_reason_refuse_oversize_and_split_per_part() {
    let p = pair().await;
    let n = &p.n;
    let path = format!(
        "/api/v1/networks/stub-a/conversations/{}/messages",
        fjson::fc_id(p.src)
    );
    // region unset: not a call error — 202 and the row carries the reason
    p.a.update_facts(|f| f.region_set = Some(false));
    let r = n.post(&path, json!({"text": "hello mesh"}));
    check_op("sendForeignMessage", &r);
    let v = r.json();
    assert_eq!(v["plan"]["parts"], 1);
    let id = fjson::parse_fm_id(v["id"].as_str().unwrap()).unwrap();
    let row = n.fab().foreign_message(id).await.unwrap();
    assert_eq!(row.reason_code.as_deref(), Some("region_unset"));
    p.a.update_facts(|f| f.region_set = Some(true));
    // oversize in refuse mode: 409 and nothing queued
    let before = n.rows(p.src).await.len();
    let long = "x".repeat(300);
    let r = n.post(&path, json!({"text": long}));
    let pb = r.problem(409, "oversize");
    assert!(
        pb["detail"].as_str().unwrap().contains("300 B > 232 B"),
        "{pb}"
    );
    assert_eq!(n.rows(p.src).await.len(), before);
    // split: one row per part, numbered
    let r = n.post(&path, json!({"text": long, "size_mode": "split"}));
    check_op("sendForeignMessage", &r);
    let ids = r.json()["ids"].as_array().unwrap().clone();
    assert!(ids.len() >= 2);
    let t = n
        .get(&format!(
            "/api/v1/networks/stub-a/conversations/{}",
            fjson::fc_id(p.src)
        ))
        .json();
    let parts: Vec<&Value> = t["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| ids.contains(&m["id"]))
        .collect();
    assert_eq!(parts.len(), ids.len());
    assert_eq!(parts[0]["part"]["k"], 1);
    assert_eq!(parts[0]["part"]["n"], ids.len());
    // the wrong network, a bad body, an empty text
    n.post(
        &format!(
            "/api/v1/networks/stub-b/conversations/{}/messages",
            fjson::fc_id(p.src)
        ),
        json!({"text": "x"}),
    )
    .problem(404, "not_found");
    n.post(&path, json!({"text": ""}))
        .problem(400, "bad_request");
    n.post(&path, json!({"txt": "x"}))
        .problem(400, "bad_request");
    // the agent's bearer never sends
    http(
        n.port,
        "POST",
        &path,
        &[("Authorization", &n.bearer())],
        Some(&json!({"text": "from the agent"})),
    )
    .problem(401, "unauthenticated");
    // the adapter off: the network's component is unavailable
    n.bridge.disable("stub-a").await;
    n.post(&path, json!({"text": "x"}))
        .problem(409, "component_unavailable");
}

/// Redirect this process's stderr into a file while `f` runs (the preview's "no log of
/// the text" check).
fn capture_stderr<T>(f: impl FnOnce() -> T) -> (T, String) {
    let file = tempfile::NamedTempFile::new().unwrap();
    use std::os::unix::io::AsRawFd;
    // SAFETY: plain fd duplication of this process's stderr and back.
    let saved = unsafe { libc::dup(2) };
    assert!(saved >= 0);
    unsafe {
        libc::dup2(file.as_file().as_raw_fd(), 2);
    }
    let out = f();
    unsafe {
        libc::dup2(saved, 2);
        libc::close(saved);
    }
    (
        out,
        std::fs::read_to_string(file.path()).unwrap_or_default(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_fanout_preview_writes_and_logs_nothing_and_a_refusal_queues_nothing() {
    let p = pair().await;
    let n = &p.n;
    let secret = "preview-text-7f3a9c-never-logged";
    let req = json!({
        "text": secret,
        "targets": [
            {"network": "stub-a", "conv": fjson::fc_id(p.src)},
            {"network": "stub-b", "conv": fjson::fc_id(p.dst)},
        ],
    });
    let port = n.port;
    let (r, logged) = capture_stderr(|| {
        http(
            port,
            "POST",
            "/api/v1/messages/fanout/preview",
            &[],
            Some(&req),
        )
    });
    check_op("previewFanout", &r);
    let v = r.json();
    assert_eq!(v["ok"], true);
    assert_eq!(v["items"].as_array().unwrap().len(), 2);
    assert!(
        !logged.contains(secret),
        "the text reached stderr: {logged}"
    );
    assert!(n.rows(p.src).await.is_empty() && n.rows(p.dst).await.is_empty());
    assert!(
        p.a.sent().is_empty() && p.b.sent().is_empty(),
        "nothing transmitted"
    );

    // a refusal anywhere: 409 with the per-target report, nothing queued anywhere
    p.b.update_caps(|c| c.max_text_bytes.channel = Some(10));
    let r = n.post("/api/v1/messages/fanout", req.clone());
    let pb = r.problem(409, "oversize");
    let targets = pb["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0]["target"]["conv"], fjson::fc_id(p.dst));
    assert_schema("TargetRefusal", &targets[0]);
    assert!(n.rows(p.src).await.is_empty() && n.rows(p.dst).await.is_empty());
    // the preview says the same without writing
    let r = n.post("/api/v1/messages/fanout/preview", req.clone());
    check_op("previewFanout", &r);
    assert_eq!(r.json()["ok"], false);
    // fixed: the group is queued, one row per target
    p.b.update_caps(|c| c.max_text_bytes.channel = Some(232));
    let r = n.post("/api/v1/messages/fanout", req);
    check_op("sendFanout", &r);
    let v = r.json();
    assert!(v["group_id"].as_str().unwrap().starts_with("sg-"));
    assert_eq!(v["items"].as_array().unwrap().len(), 2);
    assert_eq!(n.rows(p.src).await.len(), 1);
    assert_eq!(n.rows(p.dst).await.len(), 1);
    // bad bodies
    n.post(
        "/api/v1/messages/fanout",
        json!({"text": "x", "targets": []}),
    )
    .problem(400, "bad_request");
    n.post(
        "/api/v1/messages/fanout",
        json!({"text": "x", "targets": [{"network": "stub-a", "conv": "fc-999999"}]}),
    )
    .problem(404, "not_found");
}

// ---------------------------------------------------------------- edges

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn edges_need_the_warning_hash_refuse_dm_and_self_and_pause_resume_delete() {
    let p = pair().await;
    let n = &p.n;
    let body = json!({"src": fjson::fc_id(p.src), "dst": fjson::fc_id(p.dst)});
    let r = n.post("/api/v1/bridge/edges", body.clone());
    let pb = r.problem(409, "consent_required");
    assert_schema("ConsentRequired", &pb["consent"]);
    let text = pb["consent"]["warning_text"].as_str().unwrap().to_string();
    let hash = pb["consent"]["hash"].as_str().unwrap().to_string();
    assert_eq!(hash, sha256_bytes(text.as_bytes()));
    assert!(text.contains("will have their messages copied"), "{text}");
    // a stale hash is refused again; nothing exists
    let mut stale = body.clone();
    stale["accept_warning"] = json!("0".repeat(64));
    n.post("/api/v1/bridge/edges", stale)
        .problem(409, "consent_required");
    let mut bad = body.clone();
    bad["accept_warning"] = json!("not-a-hash");
    n.post("/api/v1/bridge/edges", bad)
        .problem(400, "bad_request");
    assert_eq!(n.get("/api/v1/bridge/edges").json()["edges"], json!([]));
    // with the hash: 201, recorded by this session
    let mut ok = body.clone();
    ok["accept_warning"] = json!(hash);
    let r = n.post("/api/v1/bridge/edges", ok);
    check_op("createBridgeEdge", &r);
    let e = r.json();
    assert_eq!(e["enabled_by"], "loopback");
    assert_eq!(e["consent_hash"], hash.as_str());
    let eid = e["id"].as_str().unwrap().to_string();
    // a paired session's label is its companion id, never the token
    let cookie = n.session();
    let mut rev = json!({"src": fjson::fc_id(p.dst), "dst": fjson::fc_id(p.src)});
    let c = http(
        n.lan,
        "POST",
        "/api/v1/bridge/edges",
        &[("Cookie", &cookie)],
        Some(&rev),
    )
    .problem(409, "consent_required");
    rev["accept_warning"] = c["consent"]["hash"].clone();
    let r = http(
        n.lan,
        "POST",
        "/api/v1/bridge/edges",
        &[("Cookie", &cookie)],
        Some(&rev),
    );
    check_op("createBridgeEdge", &r);
    let by = r.json()["enabled_by"].as_str().unwrap().to_string();
    assert!(by.starts_with("paired:"), "{by}");
    assert!(
        !by.contains(cookie.trim_start_matches("s111=")),
        "never the token"
    );
    // DM endpoints and src = dst: 400
    let dm = n.conv(&cref("stub-a", "na", ConvKind::Dm, "dm:!1")).await;
    n.post(
        "/api/v1/bridge/edges",
        json!({"src": fjson::fc_id(dm), "dst": fjson::fc_id(p.dst)}),
    )
    .problem(400, "bad_request");
    n.post(
        "/api/v1/bridge/edges",
        json!({"src": fjson::fc_id(p.src), "dst": fjson::fc_id(p.src)}),
    )
    .problem(400, "bad_request");
    n.post(
        "/api/v1/bridge/edges",
        json!({"src": "fc-999999", "dst": fjson::fc_id(p.src)}),
    )
    .problem(404, "not_found");
    // list, get, pause, resume (a user pause resumes on the recorded text), options
    let r = n.get("/api/v1/bridge/edges");
    check_op("listBridgeEdges", &r);
    assert_eq!(r.json()["edges"].as_array().unwrap().len(), 2);
    let ep = format!("/api/v1/bridge/edges/{eid}");
    check_op("getBridgeEdge", &n.get(&ep));
    let r = n.patch(&ep, json!({"paused": true}));
    check_op("patchBridgeEdge", &r);
    assert_eq!(
        (
            r.json()["paused"].as_bool(),
            r.json()["pause_reason"].as_str()
        ),
        (Some(true), Some("user"))
    );
    let r = n.patch(&ep, json!({"paused": false}));
    check_op("patchBridgeEdge", &r);
    assert_eq!(r.json()["paused"], false);
    let r = n.patch(&ep, json!({"honour_ok_to_mqtt": true, "oversize": "split"}));
    check_op("patchBridgeEdge", &r);
    assert_eq!(
        (
            r.json()["honour_ok_to_mqtt"].as_bool(),
            r.json()["oversize"].as_str()
        ),
        (Some(true), Some("split"))
    );
    assert_eq!(r.json()["consent_hash"], hash.as_str(), "the consent stays");
    n.patch(&ep, json!({"surprise": 1}))
        .problem(400, "bad_request");
    n.get("/api/v1/bridge/edges/be-999999")
        .problem(404, "not_found");
    n.get("/api/v1/bridge/edges/7").problem(404, "not_found");
    // the agent's bearer: 401 on every edge write
    for (m, path, b) in [
        (
            "POST",
            "/api/v1/bridge/edges".to_string(),
            Some(body.clone()),
        ),
        ("PATCH", ep.clone(), Some(json!({"paused": true}))),
        ("DELETE", ep.clone(), None),
    ] {
        http(
            n.port,
            m,
            &path,
            &[("Authorization", &n.bearer())],
            b.as_ref(),
        )
        .problem(401, "unauthenticated");
    }
    // delete
    let r = n.delete(&ep);
    assert_eq!(r.status, 204, "{}", r.body);
    n.get(&ep).problem(404, "not_found");
    n.delete(&ep).problem(404, "not_found");
}

// ---------------------------------------------------------------- settings

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn settings_confirmations_use_the_409_round_trip_never_a_bare_patch() {
    let p = pair().await;
    let n = &p.n;
    // a licensed account, an uplinked channel
    p.b.update_facts(|f| {
        f.accounts[0].licensed = true;
        f.channels.push(ChannelFacts {
            account: "nb".into(),
            native_id: "ch:beta".into(),
            uplink: true,
            default_key: false,
        });
    });
    let r = n.get("/api/v1/bridge/settings");
    check_op("getBridgeSettings", &r);
    // HAM: 409 with the text and hash, then confirmed with it
    let r = n.patch(
        "/api/v1/bridge/settings",
        json!({"ham": [{"account": "nb", "confirmed": true}]}),
    );
    let pb = r.problem(409, "consent_required");
    assert_eq!(pb["consent"]["action"], "ham_confirm");
    let h = pb["consent"]["hash"].clone();
    let r = n.patch(
        "/api/v1/bridge/settings",
        json!({"ham": [{"account": "nb", "confirmed": true, "accept_warning": h}]}),
    );
    check_op("patchBridgeSettings", &r);
    assert!(r.json()["ham"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["account"] == "nb" && x["confirmed"] == true));
    // the uplinked channel's send switch: the same round trip
    let ch = json!({"conv": fjson::fc_id(p.dst), "send": true});
    let r = n.patch("/api/v1/bridge/settings", json!({"channels": [ch]}));
    let pb = r.problem(409, "consent_required");
    assert_eq!(pb["consent"]["action"], "uplink_send");
    let mut ch2 = ch.clone();
    ch2["accept_warning"] = pb["consent"]["hash"].clone();
    let r = n.patch("/api/v1/bridge/settings", json!({"channels": [ch2]}));
    check_op("patchBridgeSettings", &r);
    assert!(r.json()["channels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["conv"] == fjson::fc_id(p.dst) && c["send"] == true));
    // the compiled-in adapter switch; an optional component is not switched here
    let r = n.patch(
        "/api/v1/bridge/settings",
        json!({"adapters": {"meshtastic": false}}),
    );
    check_op("patchBridgeSettings", &r);
    assert_eq!(r.json()["adapters"]["meshtastic"], false);
    assert_eq!(n.bridge.adapter_switch("meshtastic"), Some(false));
    n.patch(
        "/api/v1/bridge/settings",
        json!({"adapters": {"lxmf": true}}),
    )
    .problem(400, "bad_request");
    // the order belongs to the spine: none here, refused without writing
    n.patch("/api/v1/bridge/settings", json!({"order": ["111"]}))
        .problem(400, "bad_request");
    n.patch("/api/v1/bridge/settings", json!({"surprise": 1}))
        .problem(400, "bad_request");
    http(
        n.port,
        "PATCH",
        "/api/v1/bridge/settings",
        &[("Authorization", &n.bearer())],
        Some(&json!({"adapters": {"meshtastic": true}})),
    )
    .problem(401, "unauthenticated");
    assert_eq!(n.bridge.adapter_switch("meshtastic"), Some(false));
}

// ---------------------------------------------------------------- components

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn components_enable_after_the_licence_notice_then_disable_and_remove_files() {
    let n = node().await;
    let cache = n.layout.root.join("cache/packages/demo");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("demo-bin"), BIN).unwrap();
    let r = n.get("/api/v1/bridge/components");
    check_op("listBridgeComponents", &r);
    assert_eq!(r.json()["components"][0]["state"], "cached");
    // without the acknowledgement: 409 with the notice and its hash, nothing installed
    let r = n.post("/api/v1/bridge/components/demo/enable", json!({}));
    let pb = r.problem(409, "consent_required");
    assert_eq!(pb["consent"]["action"], "component_enable");
    let hash = pb["consent"]["hash"].as_str().unwrap().to_string();
    assert_eq!(hash, demo_component().notice_hash());
    assert!(!n.layout.root.join("components/demo").exists());
    n.post(
        "/api/v1/bridge/components/demo/enable",
        json!({"accept_notice_hash": "0".repeat(64)}),
    )
    .problem(409, "consent_required");
    // the bearer is refused before anything
    http(
        n.port,
        "POST",
        "/api/v1/bridge/components/demo/enable",
        &[("Authorization", &n.bearer())],
        Some(&json!({"accept_notice_hash": hash})),
    )
    .problem(401, "unauthenticated");
    // with it: installed, started, healthy (200 Component)
    let r = n.post(
        "/api/v1/bridge/components/demo/enable",
        json!({"accept_notice_hash": hash}),
    );
    check_op("enableBridgeComponent", &r);
    assert_eq!(r.json()["state"], "running");
    assert_eq!(r.json()["acknowledged"], true);
    assert!(n.layout.root.join("components/demo/demo-bin").is_file());
    // remove-files while running: refused
    n.post("/api/v1/bridge/components/demo/remove-files", json!({}))
        .problem(409, "bad_request");
    // disable (idempotent), then remove the files and the cache
    let r = n.post("/api/v1/bridge/components/demo/disable", json!({}));
    check_op("disableBridgeComponent", &r);
    assert_ne!(r.json()["state"], "running");
    let r = n.post("/api/v1/bridge/components/demo/disable", json!({}));
    check_op("disableBridgeComponent", &r);
    let r = n.post(
        "/api/v1/bridge/components/demo/remove-files",
        json!({"cache": true}),
    );
    check_op("removeBridgeComponentFiles", &r);
    assert!(!n.layout.root.join("components/demo").exists());
    assert!(!cache.exists());
    assert_eq!(r.json()["state"], "declared");
    let st = n.components.status("demo").await.unwrap();
    assert_eq!(st.state, ComponentState::Declared);
    // unknown ids
    n.post("/api/v1/bridge/components/nope/enable", json!({}))
        .problem(404, "not_found");
    n.post("/api/v1/bridge/components/Bad.Id/disable", json!({}))
        .problem(404, "not_found");
}

// ---------------------------------------------------------------- proposals

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bridge_message_proposals_queue_on_accept_and_a_gate_keeps_them_open() {
    let p = pair().await;
    let n = &p.n;
    let body = format!(
        "to: {}\nto: {}\n\nhello from the agent\n",
        fjson::fc_id(p.src),
        fjson::fc_id(p.dst)
    );
    n.propose("prop-bm1", "bridge_message", &body, Some(&n.id));
    let r = n.get("/api/v1/proposals/prop-bm1");
    check_op("getProposal", &r);
    // the agent's bearer never decides
    http(
        n.port,
        "POST",
        "/api/v1/proposals/prop-bm1/decision",
        &[("Authorization", &n.bearer())],
        Some(&json!({"decision": "accepted"})),
    )
    .problem(401, "unauthenticated");
    let r = n.decide("prop-bm1", None);
    check_op("decideProposal", &r);
    assert_eq!(r.json()["decision"]["decision"], "accepted");
    let rec = n.decision_file("prop-bm1").unwrap();
    assert!(
        rec["applied"]["group_id"]
            .as_str()
            .unwrap()
            .starts_with("sg-"),
        "{rec}"
    );
    assert_eq!(rec["applied"]["items"].as_array().unwrap().len(), 2);
    let rows = n.rows(p.src).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].body, "hello from the agent\n", "verbatim");
    n.decide("prop-bm1", None).problem(409, "already_decided");

    // a gate (an uplinked channel, send off): 409 with the text; the proposal stays open
    p.b.update_facts(|f| {
        f.channels.push(ChannelFacts {
            account: "nb".into(),
            native_id: "ch:beta".into(),
            uplink: true,
            default_key: false,
        })
    });
    let body = format!("to: {}\n\nsecond\n", fjson::fc_id(p.dst));
    n.propose("prop-bm2", "bridge_message", &body, Some(&n.id));
    let r = n.decide("prop-bm2", None);
    let pb = r.problem(409, "consent_required");
    assert_schema("ConsentRequired", &pb["consent"]);
    assert_eq!(pb["consent"]["action"], "uplink_send");
    assert!(n.decision_file("prop-bm2").is_none(), "still open");
    assert!(n.get("/api/v1/proposals/prop-bm2").json()["decision"].is_null());
    let before = n.rows(p.dst).await.len();
    // a stale hash: asked again
    n.decide("prop-bm2", Some(&"0".repeat(64)))
        .problem(409, "consent_required");
    // the hash shown: the confirmation is recorded and the text is queued
    let r = n.decide("prop-bm2", pb["consent"]["hash"].as_str());
    check_op("decideProposal", &r);
    assert_eq!(n.rows(p.dst).await.len(), before + 1);
    let s = n.get("/api/v1/bridge/settings").json();
    assert!(s["channels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["conv"] == fjson::fc_id(p.dst) && c["send"] == true));

    // bodies outside the contract: 502, open; a stamp for another node: 502
    n.propose(
        "prop-bm3",
        "bridge_message",
        "to: ch:LongFast\n\nx",
        Some(&n.id),
    );
    n.decide("prop-bm3", None).problem(502, "upstream_error");
    assert!(n.decision_file("prop-bm3").is_none());
    let other = "ab".repeat(32);
    n.propose(
        "prop-bm4",
        "bridge_message",
        &format!("to: {}\n\nx", fjson::fc_id(p.src)),
        Some(&other),
    );
    n.decide("prop-bm4", None).problem(502, "upstream_error");
    // a target that names nothing: 404, open
    n.propose(
        "prop-bm5",
        "bridge_message",
        "to: fc-999999\n\nx",
        Some(&n.id),
    );
    n.decide("prop-bm5", None).problem(404, "not_found");
    assert!(n.decision_file("prop-bm5").is_none());
    // dismissing applies nothing
    n.propose(
        "prop-bm6",
        "bridge_message",
        &format!("to: {}\n\nnever", fjson::fc_id(p.src)),
        Some(&n.id),
    );
    let before = n.rows(p.src).await.len();
    let r = n.post(
        "/api/v1/proposals/prop-bm6/decision",
        json!({"decision": "dismissed"}),
    );
    check_op("decideProposal", &r);
    assert_eq!(n.rows(p.src).await.len(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bridge_edge_proposals_run_the_edge_consent_round_trip() {
    let p = pair().await;
    let n = &p.n;
    let body = format!(
        "from: {}\nto: {}\n\nthe two channels talk about the same thing",
        fjson::fc_id(p.src),
        fjson::fc_id(p.dst)
    );
    n.propose("prop-be1", "bridge_edge", &body, Some(&n.id));
    let pb = n.decide("prop-be1", None).problem(409, "consent_required");
    assert_eq!(pb["consent"]["action"], "edge_enable");
    assert!(n.decision_file("prop-be1").is_none());
    assert_eq!(n.get("/api/v1/bridge/edges").json()["edges"], json!([]));
    let r = n.decide("prop-be1", pb["consent"]["hash"].as_str());
    check_op("decideProposal", &r);
    let rec = n.decision_file("prop-be1").unwrap();
    assert!(rec["applied"]["edge_id"]
        .as_str()
        .unwrap()
        .starts_with("be-"));
    let edges = n.get("/api/v1/bridge/edges").json()["edges"].clone();
    assert_eq!(edges.as_array().unwrap().len(), 1);
    assert_eq!(edges[0]["consent_hash"], pb["consent"]["hash"]);
    // a DM endpoint: 400, open
    let dm = n.conv(&cref("stub-a", "na", ConvKind::Dm, "dm:!2")).await;
    n.propose(
        "prop-be2",
        "bridge_edge",
        &format!(
            "from: {}\nto: {}\n\nx",
            fjson::fc_id(dm),
            fjson::fc_id(p.dst)
        ),
        Some(&n.id),
    );
    n.decide("prop-be2", None).problem(400, "bad_request");
    assert!(n.decision_file("prop-be2").is_none());
}

fn sleeper(name: &str) -> node111::daemon::supervise::ChildHandle {
    spawn_child(ChildSpec {
        name: name.into(),
        program: PathBuf::from("/bin/sleep"),
        args: vec!["30".into()],
        env: vec![],
        backoff: BackoffPolicy::default(),
        stdout_log: None,
    })
}

/// rnstatus for the device core's spine: logged, answers nothing.
#[derive(Clone, Default)]
struct NullRnstatus;

impl node111::util::run::Runner for NullRnstatus {
    fn run(&self, _: &str, _: &[&str]) -> anyhow::Result<node111::util::run::Output> {
        Ok(node111::util::run::Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        })
    }

    fn run_as(&self, _: &str, _: &str, _: &[&str]) -> anyhow::Result<node111::util::run::Output> {
        anyhow::bail!("not used")
    }
}

/// The device core on `n` (C4): the registry, the identify sequence, a spine and the
/// applier, as the daemon wires them.
fn device_core(n: &Node) -> Arc<node111::daemon::transport::posture::Spine> {
    use node111::daemon::devices::{self, identify, registry::RegistryConfig};
    use node111::daemon::transport::{config, interfaces, posture::Spine, rnstatus::Rnstatus};
    let st = n.layout.state_dir();
    let startup = interfaces::startup(
        &st,
        &st.join(devices::store::FILE),
        &config::config_path(&n.layout),
    );
    let spine = Spine::new(
        n.layout.clone(),
        startup,
        Arc::new(Rnstatus::new(Arc::new(NullRnstatus), &n.layout.root)),
        false,
        None,
    );
    spine.write_config().unwrap();
    let reg = devices::install(&n.api, RegistryConfig::production(&n.layout));
    identify::install(
        &n.api,
        &reg,
        identify::IdentifyConfig::production(&n.layout.root, false),
    );
    reg.attach_spine(Arc::clone(&spine));
    let applier =
        devices::applier::Applier::production(&n.api, Arc::clone(&spine), None, None).unwrap();
    n.api.insert_ext(applier);
    spine
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn device_proposals_go_to_the_device_core_and_refuse_what_is_not_applied() {
    let n = node().await;
    // without the device core: unsupported here, the proposal stays open
    n.propose(
        "prop-dv0",
        "device",
        r#"{"why":"x","actions":[{"action":"radio.region.set","region":"EU_868"}]}"#,
        Some(&n.id),
    );
    n.decide("prop-dv0", None)
        .problem(409, "unsupported_action");
    assert!(n.decision_file("prop-dv0").is_none());
    let spine = device_core(&n);
    // an applied action through the applier: the region is written (acceptance is the
    // user's confirmation, §12), the decision recorded by the applier
    let r = n.decide("prop-dv0", None);
    check_op("decideProposal", &r);
    assert_eq!(r.json()["decision"]["decision"], "accepted");
    assert_eq!(spine.file().radio.region.as_deref(), Some("EU_868"));
    let rec = n.decision_file("prop-dv0").unwrap();
    assert_eq!(rec["applied"]["applied"], true, "{rec}");
    n.decide("prop-dv0", None).problem(409, "already_decided");
    // service.restart of a supervised child: accepted, it restarts
    let child = sleeper("agent-test");
    let control = child.control();
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.pid().is_none() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let pid = control.pid().unwrap();
    n.api.insert_ext(Arc::new(
        Supervised::default().with(ServiceId::Agent, Some(control.clone())),
    ));
    let body = r#"{"why":"the agent hangs","actions":[{"action":"service.restart","service_id":"agent"}]}"#;
    n.propose("prop-dv1", "device", body, Some(&n.id));
    let r = n.decide("prop-dv1", None);
    check_op("decideProposal", &r);
    let rec = n.decision_file("prop-dv1").unwrap();
    assert_eq!(
        rec["applied"]["actions"][0]["action"], "service.restart",
        "{rec}"
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if control.pid().is_some_and(|p| p != pid) {
            break;
        }
        assert!(Instant::now() < deadline, "the child was restarted");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // a child 111d does not supervise: 409 not_supervised, open
    n.propose(
        "prop-dv2",
        "device",
        r#"{"why":"x","actions":[{"action":"service.restart","service_id":"model"}]}"#,
        Some(&n.id),
    );
    n.decide("prop-dv2", None).problem(409, "not_supervised");
    assert!(n.decision_file("prop-dv2").is_none());
    // the vocabulary 111d never applies: 409 unsupported_action, nothing runs
    for (id, action) in [
        ("prop-dv3", r#"{"action":"firmware.flash"}"#),
        ("prop-dv4", r#"{"action":"meshtastic.set"}"#),
        ("prop-dv5", r#"{"action":"self.destruct"}"#),
    ] {
        n.propose(
            id,
            "device",
            &format!(r#"{{"why":"x","actions":[{action}]}}"#),
            Some(&n.id),
        );
        n.decide(id, None).problem(409, "unsupported_action");
        assert!(n.decision_file(id).is_none(), "{id}");
    }
    // a body that is not the one JSON object: 502; a stamp for another node: 502
    n.propose(
        "prop-dv7",
        "device",
        "identify the radio please",
        Some(&n.id),
    );
    n.decide("prop-dv7", None).problem(502, "upstream_error");
    n.propose("prop-dv9", "device", body, Some(&"cd".repeat(32)));
    n.decide("prop-dv9", None).problem(502, "upstream_error");
    // identify: the accept runs the same consented sequence as the route (here no such
    // device: both answer the same problem, nothing ran, the proposal stays open)
    let route = n.post(
        "/api/v1/devices/abcd1234/identify",
        json!({"consent": true}),
    );
    n.propose(
        "prop-dv8",
        "device",
        r#"{"why":"identify this radio once (a consented probe)","actions":[{"action":"identify","device_id":"abcd1234"}]}"#,
        Some(&n.id),
    );
    let r = n.decide("prop-dv8", None);
    assert_eq!(r.status, route.status, "{} vs {}", r.body, route.body);
    assert_eq!(r.json()["code"], route.json()["code"]);
    assert!(
        n.decision_file("prop-dv8").is_none(),
        "refused before it ran: open"
    );
    // the identify route itself refuses the agent's bearer now (it carries its own flag)
    http(
        n.port,
        "POST",
        "/api/v1/devices/abcd1234/identify",
        &[("Authorization", &n.bearer())],
        Some(&json!({"consent": true})),
    )
    .problem(401, "unauthenticated");
    // ... and so does the restart route
    http(
        n.port,
        "POST",
        "/api/v1/services/agent/restart",
        &[("Authorization", &n.bearer())],
        Some(&json!({})),
    )
    .problem(401, "unauthenticated");
    control.stop_and_wait(Duration::from_secs(2)).await;
}

// ---------------------------------------------------------------- SSE

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn network_message_and_bridge_frames_reach_the_event_stream() {
    let p = pair().await;
    let n = &p.n;
    let head = n.api.events.head();
    p.a.feed(text(&p.ca, "!a1", "Alice", "s1", "over the air"));
    n.wait_rows(p.src, 1).await;
    // an adapter change: the core publishes the `bridge` snapshot
    n.bridge.disable("stub-b").await;
    let want = format!("\"conv\":\"{}\"", fjson::fc_id(p.src));
    let raw = sse_after(n.port, head, |s| {
        s.contains(&want) && s.contains("\"change\":\"new\"") && s.contains("event: bridge")
    });
    let got = frames(&raw);
    let nm: Vec<Value> = got
        .iter()
        .filter(|(e, _)| e == "network_message")
        .map(|(_, d)| serde_json::from_str(d).unwrap())
        .collect();
    assert!(!nm.is_empty(), "{raw}");
    for v in &nm {
        assert_schema("SseNetworkMessage", v);
    }
    assert!(
        nm.iter()
            .any(|v| v["conv"] == fjson::fc_id(p.src) && v["change"] == "new"),
        "{nm:?}"
    );
    assert!(!raw.contains("over the air"), "a frame carries no text");
    let br: Vec<&(String, String)> = got.iter().filter(|(e, _)| e == "bridge").collect();
    assert!(!br.is_empty(), "{raw}");
    let v: Value = serde_json::from_str(&br[0].1).unwrap();
    assert_schema("SseBridge", &v);
}

// ---------------------------------------------------------------- daemon wiring

#[test]
fn a_fresh_layer6_spawns_nothing_and_enables_nothing() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("state")).unwrap();
        let layout = DaemonLayout {
            root: dir.path().to_path_buf(),
            etc: dir.path().join("etc"),
        };
        let identity = node111::identity::generate(&layout.identity_dir()).unwrap();
        let key = node111::daemon::bridge_conv_key(&identity.keyset().to_bytes()).unwrap();
        let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.19")));
        let mut acfg = ApiConfig::new(layout.clone(), false);
        acfg.persist_auth = false;
        let api = ApiState::new(dash, acfg);
        let mut c = FabricConfig::production(&layout, None);
        c.listen = "127.0.0.1:0".parse().unwrap();
        c.housekeeping = Duration::from_secs(3600);
        let fabric = fabric::start(c, Some(identity), &api).await;
        assert!(fabric.running());
        // the Meshtastic client is registered but not started (no board: service off)
        let client = node111::daemon::meshtastic::client::MeshtasticAdapter::new(
            node111::daemon::meshtastic::client::ClientConfig::production(
                node111::daemon::meshtastic::meshmud::DOWNLINK,
            ),
            *key,
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        let metrics = tokio::runtime::Handle::current().metrics();
        let before = metrics.num_alive_tasks();
        let l6 = node111::daemon::start_layer6(node111::daemon::Layer6Parts {
            api: Arc::clone(&api),
            fabric,
            ports: Arc::new(PortTable::new()),
            tick: Arc::new(|| 0),
            transport: None,
            mesh_client: Some(Arc::clone(&client)),
            cmd: Arc::new(node111::testkit::MockRunner::new(Vec::<(
                String,
                node111::util::run::Output,
            )>::new())),
        });
        for _ in 0..10 {
            l6.tick_once().await;
        }
        assert_eq!(
            l6.bridge.recv_tasks(),
            0,
            "no recv task with no adapter enabled"
        );
        assert!(l6.bridge.enabled().is_empty());
        assert_eq!(l6.bridge.tick(), 0, "the core's pass returned at once");
        assert!(
            !client.running(),
            "the client starts only while meshtastic is LIVE"
        );
        let after = metrics.num_alive_tasks();
        assert!(
            after <= before,
            "Layer 6 spawned {} task(s)",
            after - before
        );
        assert!(api.ext::<Arc<Bridge>>().is_some());
        assert!(api.ext::<Components>().is_some());
        // W1: the LXMF seam and the Nearby sink are registered, and still nothing runs
        let adapters = api
            .ext::<Arc<node111::daemon::dashboard::api_v1::bridge::ComponentAdapters>>()
            .unwrap();
        assert!(adapters.registered("lxmf"));
        let heard = api
            .ext::<Arc<node111::daemon::dashboard::api_v1::nearby::BridgeNearby>>()
            .unwrap();
        assert!(heard.sources().is_empty());
        assert!(
            l6.components.stdio("lxmf").await.is_none(),
            "no helper child"
        );
        assert!(
            !dir.path().join("bridge/lxmf").exists(),
            "no file of the helper while it is disabled"
        );
        // W3: the Delta Chat network's hooks and factory are registered, and still nothing
        // of either component runs or exists on disk
        assert!(
            adapters.registered("deltachat"),
            "the core's adapter factory"
        );
        assert!(adapters.prepares("deltachat"), "the core's prepare hook");
        assert!(adapters.prepares("madmail"), "the relay's prepare hook");
        assert!(
            !adapters.registered("madmail"),
            "the relay carries no adapter"
        );
        for id in ["deltachat", "madmail"] {
            assert!(l6.components.stdio(id).await.is_none(), "no {id} child");
        }
        assert!(
            !dir.path().join("bridge/deltachat").exists(),
            "no file of Delta Chat or Madmail while they are disabled"
        );
        l6.shutdown().await;
    });
}

// ---------------------------------------------------------------- W1: foreign retry / cancel

/// The `ForeignMessage` JSON of row `id` in `conv`'s thread (`None`: not listed).
fn listed(n: &Node, net: &str, conv: i64, id: i64) -> Option<Value> {
    let t = n
        .get(&format!(
            "/api/v1/networks/{net}/conversations/{}",
            fjson::fc_id(conv)
        ))
        .json();
    t["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == fjson::fm_id(id))
        .cloned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn foreign_rows_retry_and_cancel_by_fm_id_and_msg_ids_keep_the_layer5_path() {
    use mesh111::store::ForeignState;
    let p = pair().await;
    let n = &p.n;
    let path = format!(
        "/api/v1/networks/stub-a/conversations/{}/messages",
        fjson::fc_id(p.src)
    );
    let send = |text: &str| {
        let r = n.post(&path, json!({ "text": text }));
        check_op("sendForeignMessage", &r);
        fjson::parse_fm_id(r.json()["id"].as_str().unwrap()).unwrap()
    };
    let retry = |id: &str| n.post(&format!("/api/v1/messages/{id}/retry"), json!({}));
    let cancel = |id: &str| n.post(&format!("/api/v1/messages/{id}/cancel"), json!({}));

    // queued, nothing handed: cancel deletes the row (202), GET shows it gone
    let q = send("cancel me");
    assert!(listed(n, "stub-a", p.src, q).is_some());
    let r = cancel(&fjson::fm_id(q));
    assert_eq!(r.status, 202, "{}", r.body);
    assert_eq!(r.json()["id"], fjson::fm_id(q));
    assert!(n.fab().foreign_message(q).await.is_err(), "deleted");
    assert!(
        listed(n, "stub-a", p.src, q).is_none(),
        "gone from the thread"
    );
    cancel(&fjson::fm_id(q)).problem(404, "not_found");

    // a sent row: not cancellable (409, Layer 5 copy) and not waiting for a decision
    let s = send("on air");
    n.fab().foreign_handoff_begin(s).await.unwrap();
    n.fab().foreign_accepted(s, "native-1").await.unwrap();
    n.fab()
        .foreign_set_state(s, ForeignState::Sent, Some("accepted by the radio"), None)
        .await
        .unwrap();
    let pb = cancel(&fjson::fm_id(s)).problem(409, "bad_request");
    assert!(pb["title"].as_str().unwrap().contains("cancelled"), "{pb}");
    assert_eq!(
        n.fab().foreign_message(s).await.unwrap().state,
        ForeignState::Sent,
        "kept"
    );
    retry(&fjson::fm_id(s)).problem(409, "bad_request");

    // an uncertain row (handed, never answered, not in flight): may have been sent, so it
    // is not cancelled; retry sends it again as a new row and the old one records it
    let u = send("maybe sent");
    n.fab().foreign_handoff_begin(u).await.unwrap();
    assert!(n.bridge.uncertain(u).await.unwrap());
    let row = listed(n, "stub-a", p.src, u).unwrap();
    assert_eq!(row["uncertain"], true, "{row}");
    assert_eq!(row["reason_code"], "handoff_pending", "{row}");
    cancel(&fjson::fm_id(u)).problem(409, "bad_request");
    let r = retry(&fjson::fm_id(u));
    assert_eq!(r.status, 200, "{}", r.body);
    assert_schema("ForeignSendAccepted", &r.json());
    let new = fjson::parse_fm_id(r.json()["id"].as_str().unwrap()).unwrap();
    assert_ne!(new, u, "a new row");
    let old = n.fab().foreign_message(u).await.unwrap();
    assert!(
        old.meta.contains(&format!("\"resent_as\":{new}")),
        "{}",
        old.meta
    );
    assert_eq!(
        old.state,
        ForeignState::Queued,
        "the old row keeps its state"
    );
    let fresh = n.fab().foreign_message(new).await.unwrap();
    assert_eq!(fresh.state, ForeignState::Queued);
    assert_eq!(fresh.body, "maybe sent");
    assert!(listed(n, "stub-a", p.src, new).is_some());
    // an inbound row is neither
    p.a.feed(text(&p.ca, "!a1", "Alice", "in-1", "hello"));
    let rows = n.wait_rows(p.src, 4).await;
    let inbound = rows
        .iter()
        .find(|r| r.dir == mesh111::store::ForeignDir::In)
        .unwrap()
        .id;
    cancel(&fjson::fm_id(inbound)).problem(409, "bad_request");
    retry(&fjson::fm_id(inbound)).problem(409, "bad_request");

    // unknown and malformed fm- ids
    for id in ["fm-999999", "fm-0", "fm-x", "fm-"] {
        retry(id).problem(404, "not_found");
        cancel(id).problem(404, "not_found");
    }
    // a Layer 5 id takes the Layer 5 path: msg-<n> never names foreign row n
    let k = send("a foreign row");
    cancel(&format!("msg-{k}")).problem(404, "not_found");
    retry(&format!("msg-{k}")).problem(404, "not_found");
    assert!(n.fab().foreign_message(k).await.is_ok(), "untouched");
    // the agent's bearer is refused by the global gate before any handler
    for op in ["retry", "cancel"] {
        http(
            n.port,
            "POST",
            &format!("/api/v1/messages/{}/{op}", fjson::fm_id(k)),
            &[("Authorization", &n.bearer())],
            Some(&json!({})),
        )
        .problem(401, "unauthenticated");
    }
    assert!(n.fab().foreign_message(k).await.is_ok());
}

// ---------------------------------------------------------------- W1: the LXMF seam

/// Our delivery destination and a peer the scripted helper hears.
const LX_DEST: &str = "0123456789abcdef0123456789abcdef";
const LX_PEER: &str = "fedcba9876543210fedcba9876543210";

/// A scripted LXMF helper (a binary payload): records its arguments under `$HOME`, says
/// `ready`, reports one `heard`, then reads stdin until it is stopped (its stdout stays
/// open: a closed stdout ends the run for the adapter).
fn lxmf_bin() -> Vec<u8> {
    format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"$HOME/lxmf-args\"\n\
         printf '%s\\n' '{{\"type\":\"ready\",\"dest\":\"{LX_DEST}\",\"pubkey\":\"{pk}\",\"instance\":\"111\"}}'\n\
         printf '%s\\n' '{{\"type\":\"heard\",\"dest\":\"{LX_PEER}\",\"name\":\"Bob\"}}'\n\
         while IFS= read -r line; do :; done\n",
        pk = "ab".repeat(64)
    )
    .into_bytes()
}

/// The `lxmf` component as a binary payload running [`lxmf_bin`] (the real declaration is
/// a pip closure on Python 3.11 after rnsd; the seam under test does not depend on it).
fn lxmf_component() -> Component {
    let bin = lxmf_bin();
    let toml = format!(
        r#"id = "lxmf"
adapter = "lxmf"
title = "Reticulum / LXMF helper"
[payload]
kind = "binary"
url = "https://example.invalid/dl/lxmf-fake"
sha256 = "{sha}"
size = {size}
[run]
program = "lxmf-fake"
args = ["--config-dir", "{{bridge}}/rns"]
ports = []
health = "ready"
health_timeout_secs = 10
[license]
spdx = "LicenseRef-Reticulum"
read_at = "https://example.invalid/LICENSE"
notice = "This installs a scripted LXMF helper for a test; read its licence at https://example.invalid/LICENSE. You install it."
"#,
        sha = sha256_bytes(&bin),
        size = bin.len(),
    );
    let mut all = manifest::load(&[("lxmf.toml".to_string(), toml.into_bytes())]);
    assert_eq!(all.len(), 1, "the test declaration registers");
    all.pop().unwrap()
}

fn network<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["networks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == id)
        .unwrap_or_else(|| panic!("no network {id}: {v}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_lxmf_seam_writes_the_client_config_attaches_the_adapter_and_feeds_nearby() {
    use node111::daemon::dashboard::api_v1::bridge::ComponentAdapters;
    use node111::daemon::dashboard::api_v1::transport::TransportApi;
    use node111::daemon::transport::config;
    use node111::daemon::transport::rnstatus::{FactsCache, TickConfig};
    use std::os::unix::fs::PermissionsExt;
    let n = node_with(vec![lxmf_component()]).await;
    let root = n.layout.root.clone();
    let cache = root.join("cache/packages/lxmf");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("lxmf-fake"), lxmf_bin()).unwrap();
    let adapters = Arc::new(ComponentAdapters::default());
    node111::daemon::register_lxmf(&adapters, &n.api, &n.bridge);
    n.api.insert_ext(Arc::clone(&adapters));
    node111::daemon::wire_bridge_nearby(&n.api, &n.bridge);
    assert!(adapters.registered("lxmf"));
    let enable = "/api/v1/bridge/components/lxmf/enable";
    let hash = lxmf_component().notice_hash();
    let client = config::client_config_path(&root);

    // without the acknowledgement: 409 consent_required, and the hook wrote nothing
    n.post(enable, json!({})).problem(409, "consent_required");
    assert!(!root.join("bridge/lxmf").exists());
    // without a transport spine: fail closed — the contract's problem, nothing started
    let pb = n
        .post(enable, json!({ "accept_notice_hash": hash }))
        .problem(409, "component_unavailable");
    assert_eq!(pb["reason_code"], "requires_unmet", "{pb}");
    assert!(!client.exists());
    assert!(!root.join("components/lxmf").exists(), "nothing installed");
    assert!(n.components.stdio("lxmf").await.is_none(), "no child");
    assert!(n.bridge.enabled().is_empty());

    // the spine, with rnsd's config (and its rpc_key) rendered
    let spine = lxmf_spine(&n);
    n.api.insert_ext(Arc::new(TransportApi {
        spine,
        facts: FactsCache::new(TickConfig::default()),
    }));
    let head = n.api.events.head();
    let r = n.post(enable, json!({ "accept_notice_hash": hash }));
    check_op("enableBridgeComponent", &r);
    assert_eq!(r.json()["state"], "running", "{}", r.body);
    // the client config: 0600 in a 0700 dir, rnsd's key and instance, no interfaces
    let text = std::fs::read_to_string(&client).unwrap();
    let key = std::fs::read_to_string(root.join("secrets/rns-rpc-key")).unwrap();
    assert!(
        text.contains(&format!("rpc_key = {}", key.trim())),
        "{text}"
    );
    assert!(text.contains("share_instance = Yes") && text.contains("instance_name = 111"));
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&client), 0o600);
    assert_eq!(mode(&config::client_dir(&root)), 0o700);
    // the run arguments the adapter appends (§6 (3))
    let args = std::fs::read_to_string(root.join("lxmf-args")).unwrap();
    assert!(args.contains("--config-dir"), "{args}");
    assert!(args.contains("--announce-interval 60"), "{args}");
    // the adapter is enabled in the core and goes live on the helper's ready line
    assert_eq!(n.bridge.enabled(), vec!["lxmf".to_string()]);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let r = n.get("/api/v1/networks");
        check_op("listNetworks", &r);
        if network(&r.json(), "lxmf")["state"] == "live" {
            break;
        }
        assert!(Instant::now() < deadline, "never live: {}", r.body);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // its heard reaches GET /nearby as a `reticulum` source (the core's pass delivers it)
    let deadline = Instant::now() + Duration::from_secs(20);
    let src = loop {
        n.bridge.tick_once().await;
        let r = n.get("/api/v1/nearby");
        check_op("listNearby", &r);
        let v = r.json();
        if let Some(s) = v["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["kind"] == "reticulum")
        {
            break s.clone();
        }
        assert!(Instant::now() < deadline, "never heard: {v}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_schema("NearbySource", &src);
    assert_eq!(src["id"], format!("reticulum:{LX_DEST}"));
    assert_eq!(src["label"], "heard via Reticulum");
    assert_eq!(src["device_id"], Value::Null);
    assert_eq!(src["nodes"].as_array().unwrap().len(), 1);
    let node = &src["nodes"][0];
    assert_eq!(node["id"], LX_PEER);
    assert_eq!(node["num"], 0);
    assert_eq!(node["long_name"], "Bob");
    assert_eq!(node["short_name"], &LX_PEER[..8]);
    // and the SSE `nearby` frame carries the merged list
    let raw = sse_after(n.port, head, |s| s.contains("event: nearby"));
    let frame = frames(&raw)
        .into_iter()
        .find(|(e, _)| e == "nearby")
        .unwrap_or_else(|| panic!("no nearby frame: {raw}"));
    let v: Value = serde_json::from_str(&frame.1).unwrap();
    assert_schema("SseNearby", &v);
    assert!(frame.1.contains(LX_PEER), "{}", frame.1);

    // disable: the adapter goes (before the child), its source with it, the child stops
    let r = n.post("/api/v1/bridge/components/lxmf/disable", json!({}));
    check_op("disableBridgeComponent", &r);
    assert_ne!(r.json()["state"], "running");
    assert!(n.bridge.enabled().is_empty());
    let v = n.get("/api/v1/networks").json();
    assert_eq!(network(&v, "lxmf")["state"], "disabled", "{v}");
    let v = n.get("/api/v1/nearby").json();
    assert!(v["sources"].as_array().unwrap().is_empty(), "{v}");
    let st = n.components.status("lxmf").await.unwrap();
    assert_ne!(st.state, ComponentState::Running);
    assert!(
        n.components.stdio("lxmf").await.is_none(),
        "no child, no stdio"
    );
}

/// A transport spine over `n`'s root with rnsd's config rendered (its `rpc_key` minted).
fn lxmf_spine(n: &Node) -> Arc<node111::daemon::transport::posture::Spine> {
    use node111::daemon::devices;
    use node111::daemon::transport::{config, interfaces, posture::Spine, rnstatus::Rnstatus};
    let st = n.layout.state_dir();
    let startup = interfaces::startup(
        &st,
        &st.join(devices::store::FILE),
        &config::config_path(&n.layout),
    );
    let spine = Spine::new(
        n.layout.clone(),
        startup,
        Arc::new(Rnstatus::new(Arc::new(NullRnstatus), &n.layout.root)),
        false,
        None,
    );
    spine.write_config().unwrap();
    spine
}

// ---------------------------------------------------------------- W3: the resume loop

/// W3: `Layer6::resume` runs the one loop (`Components::resume_enabled_with`) through
/// `enable_component`: a component whose prepare hook fails stays off (its switch as it
/// was, nothing started or written), and resumes — hook, child, adapter — once the hook
/// can pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_runs_the_prepare_hook_and_a_failing_hook_leaves_the_component_off() {
    use node111::daemon::components::consent;
    use node111::daemon::dashboard::api_v1::bridge::ComponentAdapters;
    use node111::daemon::dashboard::api_v1::transport::TransportApi;
    use node111::daemon::transport::config;
    use node111::daemon::transport::rnstatus::{FactsCache, TickConfig};
    let n = node_with(vec![lxmf_component()]).await;
    let root = n.layout.root.clone();
    let cache = root.join("cache/packages/lxmf");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("lxmf-fake"), lxmf_bin()).unwrap();
    let adapters = Arc::new(ComponentAdapters::default());
    node111::daemon::register_lxmf(&adapters, &n.api, &n.bridge);
    n.api.insert_ext(Arc::clone(&adapters));
    // the switch is on and the notice acknowledged (an earlier lifetime enabled it)
    consent::require(
        &root,
        &lxmf_component(),
        Some(&lxmf_component().notice_hash()),
    )
    .unwrap();
    consent::update(&root, "lxmf", |r| r.enabled = true).unwrap();
    let l6 = node111::daemon::Layer6 {
        bridge: Arc::clone(&n.bridge),
        components: n.components.clone(),
        mesh_client: None,
    };

    // no transport spine: the hook refuses, so nothing starts and nothing is written
    l6.resume(&n.api).await;
    assert!(n.components.stdio("lxmf").await.is_none(), "no child");
    assert!(!config::client_config_path(&root).exists());
    assert!(!root.join("components/lxmf").exists(), "nothing installed");
    assert!(n.bridge.enabled().is_empty());
    assert!(
        consent::load(&root).record("lxmf").enabled,
        "the switch stays as it was"
    );

    // with the spine the same resume passes the hook, starts the child, attaches
    n.api.insert_ext(Arc::new(TransportApi {
        spine: lxmf_spine(&n),
        facts: FactsCache::new(TickConfig::default()),
    }));
    l6.resume(&n.api).await;
    assert!(
        n.components.stdio("lxmf").await.is_some(),
        "the helper runs"
    );
    assert!(config::client_config_path(&root).is_file());
    assert_eq!(n.bridge.enabled(), vec!["lxmf".to_string()]);
    l6.shutdown().await;
    assert!(n.bridge.enabled().is_empty());
    assert!(n.components.stdio("lxmf").await.is_none());
}

// ---------------------------------------------------------------- W3: Delta Chat + Madmail

/// The fake relay (as `tests/bridge_deltachat.rs`'s): records its arguments beside its
/// config, answers the one-shot CLI and `create-user`; for `run` it listens on the IMAP
/// address of the configuration 111 wrote — unless `NOLISTEN` sits beside the config,
/// then it only keeps its stdio open.
const W3_MADMAIL: &str = r#"#!/bin/sh
echo "$5 $6 $7" >> "$(dirname "$2")/madmail-calls.log"
case "$5" in
  run)
    if [ -e "$(dirname "$2")/NOLISTEN" ]; then
      while IFS= read -r line; do :; done
      exit 0
    fi
    exec python3 -I -c '
import socket, sys, tomllib
cfg = tomllib.load(open(sys.argv[1], "rb"))
host, port = cfg["imap_tls_listen"].rsplit(":", 1)
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind((host, int(port)))
s.listen(8)
while True:
    c, _ = s.accept()
    c.close()
' "$2" ;;
  --json) printf '{"dclogin":"dclogin:hub@[127.0.0.1]/?p=pw&v=1&ih=127.0.0.1&ip=11242&is=ssl&sh=127.0.0.1&sp=11241&ss=ssl&ic=3","email":"hub@[127.0.0.1]","password":"pw","username":"hub"}\n' ;;
  *) exit 0 ;;
esac
"#;

fn w3_python3() -> PathBuf {
    std::env::var_os("PATH")
        .and_then(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join("python3"))
                .find(|c| c.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("/usr/bin/python3"))
}

/// The core: `tests/fake_deltachat_rpc.py` behind a shell stub.
fn w3_core() -> String {
    format!(
        "#!/bin/sh\nexec \"{}\" -I \"{}\" --log \"$FAKE_DC_LOG\"\n",
        w3_python3().display(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fake_deltachat_rpc.py")
            .display()
    )
}

fn w3_decl(id: &str, file: &str, payload: &[u8], run: &str, requires: &str) -> Component {
    let toml = format!(
        r#"id = "{id}"
adapter = "app:deltachat"
title = "{title}"
[payload]
kind = "binary"
url = "https://example.invalid/dl/{file}"
sha256 = "{sha}"
size = {size}
[requires]
{requires}
[run]
program = "{file}"
{run}
health = "none"
[license]
spdx = "MPL-2.0"
read_at = "https://example.invalid/LICENSE"
notice = "Test notice for {id}. You install it."
"#,
        title = if id == "madmail" {
            "Madmail"
        } else {
            "Delta Chat core"
        },
        sha = sha256_bytes(payload),
        size = payload.len(),
    );
    let mut all = manifest::load(&[(format!("{id}.toml"), toml.into_bytes())]);
    assert_eq!(all.len(), 1, "the test declaration of {id} registers");
    all.pop().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_deltachat_seam_renders_the_relay_requires_its_listener_and_attaches_the_core() {
    use node111::daemon::bridge::deltachat::{DcConfig, RelayPaths, RelayPorts};
    use node111::daemon::dashboard::api_v1::bridge::ComponentAdapters;
    let ports = {
        let l: Vec<std::net::TcpListener> = (0..4)
            .map(|_| std::net::TcpListener::bind("127.0.0.1:0").unwrap())
            .collect();
        let p: Vec<u16> = l.iter().map(|l| l.local_addr().unwrap().port()).collect();
        RelayPorts {
            smtp: p[0],
            submission: p[1],
            imap: p[2],
            https: p[3],
        }
    };
    let core = w3_core();
    let relay = w3_decl(
        "madmail",
        "madmail-test",
        W3_MADMAIL.as_bytes(),
        &format!(
            "args = [\"--config\", \"{{bridge}}/madmail.toml\", \"--state-dir\", \"{{bridge}}/madmail\", \"run\"]\nports = {:?}",
            ports.all()
        ),
        "",
    );
    let rpc = w3_decl(
        "deltachat",
        "deltachat-test",
        core.as_bytes(),
        "env = { DC_ACCOUNTS_PATH = \"{bridge}/accounts\", FAKE_DC_LOG = \"{bridge}/fake-log.jsonl\" }",
        "component = \"madmail\"",
    );
    let (relay_hash, rpc_hash) = (relay.notice_hash(), rpc.notice_hash());
    let n = node_with(vec![relay, rpc]).await;
    let root = n.layout.root.clone();
    for (id, file, bytes) in [
        ("madmail", "madmail-test", W3_MADMAIL.as_bytes()),
        ("deltachat", "deltachat-test", core.as_bytes()),
    ] {
        let d = root.join("cache/packages").join(id);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(file), bytes).unwrap();
    }
    let adapters = Arc::new(ComponentAdapters::default());
    node111::daemon::register_deltachat_with(
        &adapters,
        &n.api,
        &n.bridge,
        node111::daemon::DeltaChatWiring {
            cli: Arc::new(node111::util::run::SystemRunner),
            ports,
            cfg: DcConfig {
                call_timeout: Duration::from_secs(5),
                configure_timeout: Duration::from_secs(5),
                settle_timeout: Duration::from_secs(2),
                ..DcConfig::default()
            },
            listen_timeout: Duration::from_secs(3),
        },
    );
    n.api.insert_ext(Arc::clone(&adapters));
    assert_eq!(
        node111::daemon::deltachat_relay_bind(&n.api),
        node111::daemon::bridge::deltachat::RelayBind::Loopback,
        "LAN sharing is off: loopback"
    );
    let paths = RelayPaths::of(&root);
    let calls = || std::fs::read_to_string(paths.dir.join("madmail-calls.log")).unwrap_or_default();
    let (relay_on, core_on) = (
        "/api/v1/bridge/components/madmail/enable",
        "/api/v1/bridge/components/deltachat/enable",
    );

    // without the relay's acknowledgement: 409, and its hook wrote nothing
    n.post(relay_on, json!({})).problem(409, "consent_required");
    assert!(
        !paths.dir.exists(),
        "nothing written before the notice passes"
    );
    // the core before the relay: requires_unmet, nothing started
    let pb = n
        .post(core_on, json!({ "accept_notice_hash": rpc_hash }))
        .problem(409, "component_unavailable");
    assert_eq!(pb["reason_code"], "requires_unmet", "{pb}");
    assert!(n.components.stdio("deltachat").await.is_none());

    // the relay: its first configuration is loopback-only, written before its child starts
    let r = n.post(relay_on, json!({ "accept_notice_hash": relay_hash }));
    check_op("enableBridgeComponent", &r);
    assert_eq!(r.json()["state"], "running", "{}", r.body);
    let conf = std::fs::read_to_string(&paths.config).unwrap();
    assert!(conf.contains("loopback only"), "{conf}");
    assert!(conf.contains(&format!("imap_tls_listen = \"127.0.0.1:{}\"", ports.imap)));
    assert!(
        n.bridge.enabled().is_empty(),
        "the relay alone carries no network"
    );

    // a relay whose IMAP listener never opens: the core's hook refuses, the core never starts
    std::fs::write(paths.dir.join("NOLISTEN"), "").unwrap();
    n.post(relay_on, json!({})); // idempotent: already running, its config kept
    let r = n.post(core_on, json!({ "accept_notice_hash": rpc_hash }));
    let pb = r.problem(409, "component_unavailable");
    assert_eq!(pb["reason_code"], "health_failed", "{pb}");
    assert!(
        pb["detail"].as_str().unwrap().contains("IMAP listener"),
        "{pb}"
    );
    assert!(n.components.stdio("deltachat").await.is_none(), "no core");
    assert!(!paths.accounts.join("fake-state.json").exists());
    assert!(n.bridge.enabled().is_empty());

    // the listener opens: settled (push off, federation rejected, registration closed on
    // loopback), re-rendered, restarted, then the core starts and its adapter attaches
    std::fs::remove_file(paths.dir.join("NOLISTEN")).unwrap();
    let r = n.post(core_on, json!({ "accept_notice_hash": rpc_hash }));
    check_op("enableBridgeComponent", &r);
    assert_eq!(r.json()["state"], "running", "{}", r.body);
    let log = calls();
    for want in ["push off", "federation policy reject", "registration close"] {
        assert!(log.lines().any(|l| l.trim() == want), "{want}: {log}");
    }
    assert_eq!(n.bridge.enabled(), vec!["app:deltachat".to_string()]);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let r = n.get("/api/v1/networks");
        check_op("listNetworks", &r);
        if network(&r.json(), "app:deltachat")["state"] == "live" {
            break;
        }
        assert!(Instant::now() < deadline, "never live: {}", r.body);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let log = calls();
    assert!(
        log.lines().any(|l| l.trim() == "--json create-user"),
        "the bridge account was created on the relay: {log}"
    );
    assert!(
        paths.accounts.join("fake-state.json").is_file(),
        "DC_ACCOUNTS_PATH"
    );
    // enabling the core again changes nothing (no relay restart under a live core)
    let runs = |t: &str| t.lines().filter(|l| l.trim() == "run").count();
    let before = runs(&calls());
    let r = n.post(core_on, json!({}));
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(runs(&calls()), before, "the relay was not restarted");

    // disable: the network leaves the core first, then nothing of either runs
    let r = n.post("/api/v1/bridge/components/deltachat/disable", json!({}));
    check_op("disableBridgeComponent", &r);
    assert!(n.bridge.enabled().is_empty());
    let v = n.get("/api/v1/networks").json();
    assert_ne!(network(&v, "app:deltachat")["state"], "live", "{v}");
    let r = n.post("/api/v1/bridge/components/madmail/disable", json!({}));
    check_op("disableBridgeComponent", &r);
    for id in ["deltachat", "madmail"] {
        let st = n.components.status(id).await.unwrap();
        assert_ne!(st.state, ComponentState::Running, "{id}");
        assert!(n.components.stdio(id).await.is_none(), "{id}: no child");
    }
    n.components.shutdown().await;
}
