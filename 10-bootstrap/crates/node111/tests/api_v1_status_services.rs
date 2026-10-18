//! Layer 4 R2 acceptance (`os/interface/README.md` §7, §10;
//! `os/interface/api/openapi.yaml`): `/api/v1/status` schema pins + superset of
//! `/api/status`, the `update` block from state files and the live updater,
//! the services registry (§7 rows from injected facts, 2-tick debounce,
//! `starting` grace, `since`, `services` frames), log tails, settings (merge,
//! alias rules, loopback-only fields, offer dual write), KB traversal guards,
//! restart / update-check, and the `status` heartbeat.
//!
//! Peers: real `ConnectInfo` over a loopback socket, or axum's
//! `MockConnectInfo(addr)` to simulate a LAN peer (same harness as
//! `dashboard_auth.rs`).

use axum::extract::connect_info::MockConnectInfo;
use node111::daemon::dashboard::api_v1::{actions, auth, status};
use node111::daemon::dashboard::{
    routes, serve_listener, ApiConfig, ApiState, DashboardState, OfferState, SharedState,
    TransportState,
};
use node111::daemon::intelligence::ChildDisplay;
use node111::daemon::kb::KbStatus;
use node111::daemon::services::{
    self, compute, AgentStatusProbe, ChildFacts, Facts, GatewayFacts, Probe, Radio, RadioFacts,
    Registry, RnodeBound, ServiceId, ServiceState,
};
use node111::daemon::supervise;
use node111::daemon::updater::{self, LastResult, UpdaterStatus};
use node111::daemon::DaemonLayout;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- harness

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
        assert_eq!(
            self.header("content-type"),
            Some("application/problem+json"),
            "{}",
            self.body
        );
        let p = self.json();
        assert_eq!(p["code"], code, "{p}");
        assert_eq!(p["status"], status);
        assert!(p["title"].is_string() && p["type"].is_string(), "{p}");
    }
}

fn dechunk(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some((size, after)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 || after.len() < n {
            break;
        }
        out.push_str(&after[..n]);
        rest = after[n..].trim_start_matches("\r\n");
    }
    out
}

fn request(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Resp {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    conn.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    conn.read_to_end(&mut raw).unwrap();
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
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
    Resp {
        status,
        body: if chunked {
            dechunk(body)
        } else {
            body.to_string()
        },
        headers,
    }
}

fn get(port: u16, path: &str) -> Resp {
    request(port, "GET", path, &[], None)
}

fn get_cookie(port: u16, path: &str, cookie: &str) -> Resp {
    request(port, "GET", path, &[("Cookie", cookie)], None)
}

fn send_json(port: u16, method: &str, path: &str, body: &str, cookie: Option<&str>) -> Resp {
    let mut h = vec![("Content-Type", "application/json")];
    if let Some(c) = cookie {
        h.push(("Cookie", c));
    }
    request(port, method, path, &h, Some(body))
}

struct Node {
    api: Arc<ApiState>,
    _dir: tempfile::TempDir,
}

impl Node {
    fn root(&self) -> std::path::PathBuf {
        self.api.layout.root.clone()
    }
    fn state_dir(&self) -> std::path::PathBuf {
        self.api.layout.state_dir()
    }
    fn write(&self, rel: &str, body: &str) {
        let p = self.root().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
}

const NODE_ID: &str = "3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29";

fn node() -> Node {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let mut d = DashboardState::new("2026.10.13");
    d.node_id = Some(NODE_ID.into());
    d.category = Some("Node2A".into());
    d.kb = Some(KbStatus {
        sha256: "ab".repeat(32),
        entries: 42,
    });
    d.components.identity = true;
    d.components.kb = true;
    let dash: SharedState = Arc::new(RwLock::new(d));
    Node {
        api: ApiState::new(dash, ApiConfig::new(layout, false)),
        _dir: dir,
    }
}

/// Serve `api`; `peer: None` = real loopback ConnectInfo, else every request
/// appears to come from `peer`.
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

/// Pair a LAN peer; returns the `Cookie` header value.
fn pair(port: u16, api: &ApiState) -> String {
    let code = auth::read_pair_code(&api.layout.state_dir())
        .unwrap()
        .expect("pair file");
    let r = send_json(
        port,
        "POST",
        "/api/v1/auth/pair",
        &json!({ "code": code }).to_string(),
        None,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let sc = r.header("set-cookie").expect("Set-Cookie");
    sc.split(';').next().unwrap().to_string()
}

/// Every frame named `name` currently in the events ring.
fn frames(api: &ApiState, name: &str) -> Vec<Value> {
    api.events
        .subscribe(Some(api.events.base()))
        .replay
        .iter()
        .filter(|f| f.event == name)
        .map(|f| serde_json::from_str(&f.data).unwrap())
        .collect()
}

async fn wait_until(deadline: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    f()
}

fn keys(v: &Value) -> Vec<String> {
    v.as_object()
        .unwrap_or_else(|| panic!("not an object: {v}"))
        .keys()
        .cloned()
        .collect()
}

fn has_all(v: &Value, required: &[&str]) {
    for k in required {
        assert!(v.get(k).is_some(), "missing required key {k:?} in {v}");
    }
}

fn one_of(v: &Value, allowed: &[Value]) {
    assert!(allowed.contains(v), "{v} not in {allowed:?}");
}

fn is_rfc3339(s: &Value) -> bool {
    s.as_str().is_some_and(|s| {
        time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).is_ok()
    })
}

// ---------------------------------------------------------------- status

/// Every leaf of `legacy` exists in `v1` with the same value (`skip` paths
/// excepted).
fn assert_superset(legacy: &Value, v1: &Value, path: &str, skip: &[&str]) {
    if skip.contains(&path) {
        return;
    }
    match legacy {
        Value::Object(m) => {
            for (k, lv) in m {
                let p = format!("{path}.{k}");
                let Some(vv) = v1.get(k) else {
                    panic!("/api/v1/status lacks {p}");
                };
                assert_superset(lv, vv, &p, skip);
            }
        }
        other => assert_eq!(other, v1, "value differs at {path}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn status_schema_pins_and_superset() {
    let n = node();
    n.write("current/bin/111-agent", "#!/bin/sh\n");
    n.write("current/share/kb/llms.txt", "# 111 KB\n");
    let port = serve(&n.api, None).await;

    let r = get(port, "/api/v1/status");
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r
        .header("content-type")
        .unwrap()
        .starts_with("application/json"));
    let s = r.json();

    // Status.required
    has_all(
        &s,
        &[
            "schema",
            "node",
            "transport_state",
            "health",
            "update",
            "intelligence",
            "bind",
            "lan_enabled",
            "auth",
            "layers",
        ],
    );
    assert_eq!(s["schema"], 1);
    // node.required + values
    let node = &s["node"];
    has_all(
        node,
        &[
            "node_id",
            "label",
            "bech32",
            "alias",
            "category",
            "version",
            "uptime_sec",
        ],
    );
    assert_eq!(node["node_id"], NODE_ID);
    let bytes: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&NODE_ID[i * 2..i * 2 + 2], 16).unwrap())
        .collect();
    let disp = id111::encoding::NodeIdDisplay::new(&bytes.try_into().unwrap());
    assert_eq!(node["label"], disp.label());
    assert_eq!(node["bech32"], disp.bech32());
    assert!(node["bech32"].as_str().unwrap().starts_with("node1"));
    assert!(node["alias"].is_null());
    one_of(
        &node["category"],
        &[
            json!("Relay1"),
            json!("Node2A"),
            json!("Node2B"),
            json!("Bridge3"),
            json!("Hub4A"),
            json!("Hub4B"),
            Value::Null,
        ],
    );
    assert!(node["uptime_sec"].is_u64());
    // enums
    one_of(
        &s["transport_state"],
        &[
            json!("no transport"),
            json!("isolated"),
            json!("joined"),
            json!("transport degraded"),
        ],
    );
    one_of(&s["health"]["status"], &[json!("ok"), json!("error")]);
    one_of(
        &s["health"]["components"]["transport"],
        &[
            json!("alive"),
            json!("absent"),
            json!("degraded"),
            json!("down"),
        ],
    );
    let up = &s["update"];
    has_all(up, &["state", "pending", "last_check", "clock_fault"]);
    one_of(
        &up["state"],
        &[
            json!("idle"),
            json!("checking"),
            json!("staged"),
            json!("probation"),
            json!("reverting"),
            json!("failed"),
        ],
    );
    assert_eq!(up["state"], "idle");
    assert_eq!(up["pending"], false);
    assert!(up["pending_version"].is_null() && up["last_check"].is_null());
    assert!(up["last_result"].is_null());
    assert_eq!(up["clock_fault"], false);
    let intel = &s["intelligence"];
    let child = [
        json!("absent"),
        json!("running"),
        json!("backoff"),
        json!("degraded"),
        json!("stopped"),
    ];
    one_of(&intel["agent"], &child);
    one_of(&intel["model_server"], &child);
    assert!(intel["agent_state"].is_null() && intel["model_id"].is_null());
    assert!(intel.get("agent_url").is_some());
    has_all(&s["bind"], &["urls", "lan"]);
    assert!(s["bind"]["urls"].is_array());
    assert_eq!(s["lan_enabled"], false);
    assert_eq!(s["auth"], json!({"loopback": true, "paired": false}));
    assert_eq!(
        s["layers"],
        json!([0, 1, 2, 3, 4]),
        "agent binary present ⇒ 3"
    );
    assert_eq!(s["kb"], json!({"sha256": "ab".repeat(32), "entries": 42}));

    // strict superset of /api/status
    let legacy = get(port, "/api/status").json();
    assert_superset(&legacy, &s, "", &[".node.uptime_sec"]);

    // without the agent binary, no layer 3
    std::fs::remove_file(n.root().join("current/bin/111-agent")).unwrap();
    assert_eq!(
        get(port, "/api/v1/status").json()["layers"],
        json!([0, 1, 2, 4])
    );
    // "working", not "installed": a failed identity drops 2 ...
    n.api.dash.write().unwrap().components.identity = false;
    assert_eq!(
        get(port, "/api/v1/status").json()["layers"],
        json!([0, 1, 4])
    );
    // ... a KB directory that went away drops 0 ...
    std::fs::rename(
        n.root().join("current/share/kb"),
        n.root().join("current/share/kb.gone"),
    )
    .unwrap();
    assert_eq!(get(port, "/api/v1/status").json()["layers"], json!([1, 4]));
    // ... and so does a KB that failed verification, even with the files back
    std::fs::rename(
        n.root().join("current/share/kb.gone"),
        n.root().join("current/share/kb"),
    )
    .unwrap();
    assert_eq!(
        get(port, "/api/v1/status").json()["layers"],
        json!([0, 1, 4])
    );
    n.api.dash.write().unwrap().kb = None;
    assert_eq!(get(port, "/api/v1/status").json()["layers"], json!([1, 4]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn status_auth_reflects_the_request() {
    let n = node();
    let lan = serve(&n.api, Some("192.168.1.60:5000")).await;
    get(lan, "/api/v1/status").problem(401, "unauthenticated");
    let cookie = pair(lan, &n.api);
    let s = get_cookie(lan, "/api/v1/status", &cookie).json();
    assert_eq!(s["auth"], json!({"loopback": false, "paired": true}));
    // loopback with a valid cookie: both true
    let lo = serve(&n.api, None).await;
    let s = get_cookie(lo, "/api/v1/status", &cookie).json();
    assert_eq!(s["auth"], json!({"loopback": true, "paired": true}));
}

#[test]
fn update_block_derivation_from_state_files() {
    let dir = tempfile::tempdir().unwrap();
    let sd = dir.path();
    let mut dash = DashboardState::new("2026.10.13");
    dash.floor_release = Some("2026.09.01".into());
    let up = UpdaterStatus::default();

    let v = status::update_view(sd, &dash, Some(&up));
    assert_eq!(serde_json::to_value(&v).unwrap()["state"], "idle");
    assert_eq!(v.floor.as_deref(), Some("2026.09.01"));
    assert!(v.last_check.is_none() && v.last_result.is_none() && !v.clock_fault);

    // last check + clock fault come from their files
    std::fs::write(
        sd.join("last-update-check.json"),
        r#"{"schema":1,"at":"2026-10-13T10:00:00Z"}"#,
    )
    .unwrap();
    std::fs::write(sd.join("clock-fault.json"), r#"{"schema":1}"#).unwrap();
    let v = status::update_view(sd, &dash, Some(&up));
    assert_eq!(v.last_check.as_deref(), Some("2026-10-13T10:00:00Z"));
    assert!(v.clock_fault);

    // failed: last outcome refused/degraded
    up.finish(LastResult::Degraded);
    let v = status::update_view(sd, &dash, Some(&up));
    assert_eq!(serde_json::to_value(&v).unwrap()["state"], "failed");
    assert_eq!(serde_json::to_value(&v).unwrap()["last_result"], "degraded");
    up.finish(LastResult::UpToDate);
    assert_eq!(
        serde_json::to_value(status::update_view(sd, &dash, Some(&up))).unwrap()["state"],
        "idle"
    );
    up.finish(LastResult::Refused);

    // checking beats failed
    assert!(!up.begin_check());
    let v = serde_json::to_value(status::update_view(sd, &dash, Some(&up))).unwrap();
    assert_eq!(v["state"], "checking");
    assert_eq!(v["last_result"], "refused");

    // staged beats checking; pending + version from the request file
    std::fs::write(
        sd.join("update-request.json"),
        r#"{"schema":1,"version":"2026.10.20","manifest_sha256":"x","staged_path":"/s"}"#,
    )
    .unwrap();
    let v = serde_json::to_value(status::update_view(sd, &dash, Some(&up))).unwrap();
    assert_eq!(v["state"], "staged");
    assert_eq!(v["pending"], true);
    assert_eq!(v["pending_version"], "2026.10.20");

    // probation beats staged
    std::fs::remove_file(sd.join("update-request.json")).unwrap();
    node111::probation::Journal {
        schema: 1,
        pending: true,
        version: "2026.10.21".into(),
        previous: "2026.10.13".into(),
        deadline_epoch: 0,
        health_confirmed: false,
    }
    .write(&sd.join("update.json"))
    .unwrap();
    let v = serde_json::to_value(status::update_view(sd, &dash, Some(&up))).unwrap();
    assert_eq!(v["state"], "probation");
    assert_eq!(v["pending_version"], "2026.10.21");
    assert_eq!(v["pending"], false);

    // reverting beats everything
    std::fs::write(sd.join("revert-request.json"), "{}").unwrap();
    let v = serde_json::to_value(status::update_view(sd, &dash, Some(&up))).unwrap();
    assert_eq!(v["state"], "reverting");

    // a non-pending journal is ignored; no updater ⇒ never checking/failed
    std::fs::remove_file(sd.join("revert-request.json")).unwrap();
    node111::probation::Journal::clear(&sd.join("update.json")).unwrap();
    let v = serde_json::to_value(status::update_view(sd, &dash, None)).unwrap();
    assert_eq!(v["state"], "idle");
    assert!(v["last_result"].is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn update_check_wakes_the_updater_loop() {
    let n = node();
    let closed = {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        l.local_addr().unwrap().port()
    };
    let cfg = updater::fetch::UpdaterConfig {
        manifest_base: format!("http://127.0.0.1:{closed}"),
        fallback_base: None,
        current_version: "2026.10.13".parse().unwrap(),
        timeout: Duration::from_secs(2),
    };
    let hooks = updater::Hooks {
        wake: Arc::clone(&n.api.update_check),
        status: Arc::new(UpdaterStatus::default()),
    };
    let st = Arc::clone(&hooks.status);
    n.api.insert_ext(Arc::clone(&st));
    tokio::spawn(updater::run_with_hooks(
        n.api.layout.clone(),
        Arc::clone(&n.api.dash),
        cfg,
        hooks,
    ));
    assert!(wait_until(Duration::from_secs(10), || st.checks() >= 1).await);
    assert_eq!(st.last_result(), Some(LastResult::Degraded));
    assert!(!st.checking());

    let port = serve(&n.api, None).await;
    let u = get(port, "/api/v1/status").json()["update"].clone();
    assert_eq!(u["state"], "failed", "{u}");
    assert_eq!(u["last_result"], "degraded");

    // the loop now sleeps ≥ 2 min of backoff; only the wake can run check #2
    let r = send_json(port, "POST", "/api/v1/update/check", "{}", None);
    assert_eq!(r.status, 202, "{}", r.body);
    assert_eq!(r.json(), json!({"schema": 1, "started": true}));
    assert!(
        wait_until(Duration::from_secs(10), || st.checks() >= 2).await,
        "updater did not wake"
    );
}

// ---------------------------------------------------------------- registry rules

fn base_facts() -> Facts {
    Facts {
        category: Some("Node2B".into()),
        boot_elapsed: Duration::from_secs(600),
        ..Facts::default()
    }
}

fn running(health: Probe, since_spawn_s: u64) -> ChildFacts {
    ChildFacts {
        display: ChildDisplay::Running,
        pid: Some(4242),
        since_spawn: Some(Duration::from_secs(since_spawn_s)),
        health,
        log: Some("/x/111-agent.log".into()),
    }
}

fn row(f: &Facts, id: ServiceId) -> services::Computed {
    compute(f).into_iter().find(|c| c.id == id).unwrap()
}

fn st(f: &Facts, id: ServiceId) -> (ServiceState, &'static str, bool) {
    let c = row(f, id);
    (c.state, c.reason_code, c.can_run_here)
}

use ServiceState::{Error as E, Live as L, Starting as S, Unset as U};

#[test]
fn rows_agent() {
    let mut f = base_facts();
    let a = row(&f, ServiceId::Agent);
    assert_eq!((a.state, a.reason_code), (U, "binary_absent"));
    assert!(a.actions.is_empty(), "absent child: no log file");

    f.agent = running(Probe::Status(200), 600);
    f.agent_status = Some(AgentStatusProbe {
        state: Some("local".into()),
        model_id: Some("qwen3-4b".into()),
    });
    let a = row(&f, ServiceId::Agent);
    assert_eq!((a.state, a.reason_code), (L, "healthy"));
    assert_eq!(
        serde_json::to_value(&a.actions).unwrap(),
        json!(["restart", "log"])
    );
    assert_eq!(a.detail["port"], 11182);
    assert_eq!(a.detail["child"], "running");
    assert_eq!(a.detail["model_id"], "qwen3-4b");
    assert_eq!(a.detail["health"], json!({"status": 200}));

    f.agent.health = Probe::Status(503);
    assert_eq!(st(&f, ServiceId::Agent), (E, "health_503", true));
    f.agent.health = Probe::Status(500);
    assert_eq!(st(&f, ServiceId::Agent).1, "health_error");
    f.agent.health = Probe::Unreachable("refused".into());
    assert_eq!(st(&f, ServiceId::Agent), (E, "health_unreachable", true));
    // inside the 90 s grace after a spawn: starting
    f.agent.since_spawn = Some(Duration::from_secs(89));
    assert_eq!(st(&f, ServiceId::Agent), (S, "health_pending", true));
    f.agent.since_spawn = Some(Duration::from_secs(90));
    assert_eq!(st(&f, ServiceId::Agent).0, E);

    f.agent.display = ChildDisplay::Degraded;
    assert_eq!(st(&f, ServiceId::Agent), (E, "child_degraded", true));
    // a crash-looped child is exactly what restart is for
    assert_eq!(
        serde_json::to_value(row(&f, ServiceId::Agent).actions).unwrap(),
        json!(["restart", "log"])
    );
    // stopped for shutdown: nothing to restart any more
    f.agent.display = ChildDisplay::Stopped;
    assert_eq!(
        serde_json::to_value(row(&f, ServiceId::Agent).actions).unwrap(),
        json!(["log"])
    );
    f.agent.display = ChildDisplay::Backoff;
    assert_eq!(st(&f, ServiceId::Agent), (E, "child_backoff", true));
    // never spawned yet at boot ⇒ starting, until the grace ends
    f.agent.since_spawn = None;
    f.boot_elapsed = Duration::from_secs(3);
    assert_eq!(st(&f, ServiceId::Agent), (S, "spawn_pending", true));
    f.boot_elapsed = Duration::from_secs(120);
    assert_eq!(st(&f, ServiceId::Agent), (E, "child_backoff", true));
}

#[test]
fn rows_model() {
    let mut f = base_facts();
    assert_eq!(st(&f, ServiceId::Model), (U, "binary_absent", true));
    f.model_binary = true;
    f.offer = Some(OfferState {
        tier: "standard".into(),
        state: "offered".into(),
    });
    let m = row(&f, ServiceId::Model);
    assert_eq!((m.state, m.reason_code), (U, "no_model"));
    assert_eq!(
        m.detail["offer"],
        json!({"tier": "standard", "state": "offered"})
    );

    f.model_file = Some("qwen3-4b-q4_k_m.gguf".into());
    f.model = running(Probe::Status(503), 100);
    // llama-server gets 120 s (loading the model)
    assert_eq!(st(&f, ServiceId::Model), (S, "health_pending", true));
    f.model.since_spawn = Some(Duration::from_secs(121));
    assert_eq!(st(&f, ServiceId::Model), (E, "health_503", true));
    f.model.health = Probe::Status(200);
    let m = row(&f, ServiceId::Model);
    assert_eq!((m.state, m.reason_code), (L, "healthy"));
    assert_eq!(m.detail["port"], 11181);
    assert_eq!(m.detail["model_file"], "qwen3-4b-q4_k_m.gguf");
    assert_eq!(
        serde_json::to_value(&m.actions).unwrap(),
        json!(["restart", "log"])
    );
    // restart is about the child, log about the file
    f.model.log = None;
    assert_eq!(
        serde_json::to_value(row(&f, ServiceId::Model).actions).unwrap(),
        json!(["restart"])
    );
}

#[test]
fn rows_transport() {
    let mut f = base_facts();
    let t = row(&f, ServiceId::Transport);
    assert_eq!((t.state, t.reason_code), (U, "no_python"));
    assert!(t.actions.is_empty());
    // python3 present but the venv could not be built: still the stub
    f.python3 = true;
    assert_eq!(st(&f, ServiceId::Transport), (U, "venv_unavailable", true));
    // a leftover rnsd.log on the stub: log only, no rnsd child to restart
    f.transport_log = Some("/x/rns/rnsd.log".into());
    assert_eq!(
        serde_json::to_value(row(&f, ServiceId::Transport).actions).unwrap(),
        json!(["log"])
    );

    f.transport_state = TransportState::Isolated;
    f.transport_child = ChildDisplay::Running;
    f.transport_log = Some("/x/rns/rnsd.log".into());
    let t = row(&f, ServiceId::Transport);
    assert_eq!((t.state, t.reason_code), (L, "isolated"));
    assert_eq!(t.detail["peers"], "isolated");
    assert_eq!(t.detail["transport_state"], "isolated");
    assert_eq!(
        serde_json::to_value(&t.actions).unwrap(),
        json!(["restart", "log"])
    );

    f.transport_state = TransportState::Joined;
    assert_eq!(st(&f, ServiceId::Transport), (L, "joined", true));
    f.transport_state = TransportState::Degraded;
    f.transport_child = ChildDisplay::Degraded;
    assert_eq!(
        st(&f, ServiceId::Transport),
        (E, "transport_degraded", true)
    );
    // rnsd backing off: starting at boot, error later
    f.transport_state = TransportState::Isolated;
    f.transport_child = ChildDisplay::Backoff;
    assert_eq!(st(&f, ServiceId::Transport), (E, "child_backoff", true));
    f.boot_elapsed = Duration::from_secs(5);
    assert_eq!(st(&f, ServiceId::Transport), (S, "spawn_pending", true));
}

#[test]
fn rows_radio_bound_stack() {
    let heltec = Radio {
        vid: "10c4".into(),
        pid: "ea60".into(),
        product: "CP2102".into(),
        name: "Heltec WiFi LoRa 32 V3 (CP2102)".into(),
        tty: Some("ttyUSB0".into()),
        by_id: Some("usb-Silicon_Labs_CP2102-if00-port0".into()),
    };
    let rnode = RnodeBound {
        interface: "RNode LoRa".into(),
        port: "/dev/ttyUSB0".into(),
        tty: "ttyUSB0".into(),
    };
    let stack_requires =
        "a mesh stack bound to the board — Reticulum RNode interface or Meshtastic";
    let rec = |f: &Facts| {
        let reg = Registry::new();
        reg.tick(compute(f), "t0");
        reg.get(ServiceId::Radio).unwrap()
    };
    let mut f = base_facts();

    // no board on a non-phone: unset, can still run here; sysfs unreadable is
    // the same answer
    f.radio.boards = Some(vec![]);
    assert_eq!(st(&f, ServiceId::Radio), (U, "no_radio", true));
    f.radio.boards = None;
    assert_eq!(st(&f, ServiceId::Radio), (U, "no_radio", true));
    let r = rec(&f);
    assert_eq!(r.plain_name, "LoRa radio");
    assert_eq!(
        r.description,
        "Long-range radio for the mesh. A LoRa board must be bound to a stack before it carries traffic."
    );
    assert_ne!(r.requires, Some(stack_requires));
    // phone-class category: cannot run here
    f.phone_like = true;
    assert_eq!(st(&f, ServiceId::Radio), (U, "no_radio", false));
    f.phone_like = false;

    // board present, nothing bound
    f.radio.boards = Some(vec![heltec.clone()]);
    assert_eq!(st(&f, ServiceId::Radio), (U, "radio_unbound", true));
    let r = rec(&f);
    assert_eq!(r.requires, Some(stack_requires));
    assert_eq!(r.kb_entry, Some("projects/reticulum.md"));
    assert_eq!(r.detail["boards"][0]["name"], heltec.name);
    assert_eq!(r.detail["boards"][0]["tty"], "ttyUSB0");
    // a stanza without rnsd running is not live; rnsd without a stanza neither
    f.radio.rnsd_running = true;
    assert_eq!(st(&f, ServiceId::Radio).1, "radio_unbound");

    // (a) RNode: rnsd running + stanza on a detected port
    f.radio.rnode = vec![rnode.clone()];
    let c = row(&f, ServiceId::Radio);
    assert_eq!((c.state, c.reason_code), (L, "radio_bound_rnode"));
    assert_eq!(c.detail["rnode_bindings"][0]["interface"], "RNode LoRa");
    assert!(c.actions.is_empty());
    // rnsd stops: starting while < 60 s down, then error
    f.radio.rnsd_running = false;
    f.radio.down_for = Some(Duration::from_secs(10));
    assert_eq!(st(&f, ServiceId::Radio), (S, "stack_waiting", true));
    f.radio.down_for = Some(Duration::from_secs(61));
    assert_eq!(st(&f, ServiceId::Radio), (E, "stack_down", true));

    // (b) meshtasticd answering 4403 (SPI HAT: no USB board at all)
    f.radio = RadioFacts {
        boards: Some(vec![]),
        meshtastic_open: true,
        meshtastic_seen: true,
        ..RadioFacts::default()
    };
    let r = rec(&f);
    assert_eq!((r.state, r.reason_code), (L, "radio_bound_meshtastic"));
    assert_eq!(r.kb_entry, Some("projects/meshtastic.md"));
    assert_eq!(r.detail["meshtastic"]["port"], 4403);
    // it stops answering: waiting, then stack_down after 60 s
    f.radio.meshtastic_open = false;
    f.radio.down_for = Some(Duration::from_secs(59));
    assert_eq!(st(&f, ServiceId::Radio), (S, "stack_waiting", true));
    f.radio.down_for = Some(Duration::from_secs(60));
    assert_eq!(st(&f, ServiceId::Radio), (E, "stack_down", true));
}

#[test]
fn rows_kb_gateway_codeagent() {
    let mut f = base_facts();
    assert_eq!(st(&f, ServiceId::Kb), (U, "kb_absent", true));
    f.kb_payload_present = true;
    assert_eq!(st(&f, ServiceId::Kb), (E, "verify_failed", true));
    f.kb = Some(KbStatus {
        sha256: "cd".repeat(32),
        entries: 7,
    });
    let k = row(&f, ServiceId::Kb);
    assert_eq!((k.state, k.reason_code), (L, "verified"));
    assert_eq!(k.detail["entries"], 7);
    assert_eq!(k.detail["sha256"], "cd".repeat(32));

    assert_eq!(st(&f, ServiceId::Gateway), (U, "wan_unknown", true));
    f.gateway = GatewayFacts {
        online: Some(false),
        url: Some("https://install.111.network/204".into()),
        checked_at: Some("2026-10-13T10:00:00Z".into()),
        error: Some("timeout".into()),
    };
    let g = row(&f, ServiceId::Gateway);
    assert_eq!((g.state, g.reason_code), (U, "wan_offline"));
    assert!(g.actions.is_empty(), "gateway never offers actions");
    f.gateway.online = Some(true);
    let g = row(&f, ServiceId::Gateway);
    assert_eq!((g.state, g.reason_code), (L, "wan_ok"));
    assert!(g.actions.is_empty());

    // codeagent: hub ∧ opencode
    assert_eq!(st(&f, ServiceId::Codeagent), (U, "not_hub", false));
    f.opencode = Some("/usr/local/bin/opencode".into());
    assert_eq!(st(&f, ServiceId::Codeagent), (U, "not_hub", false));
    for hub in ["Hub4A", "Hub4B"] {
        f.category = Some(hub.into());
        assert_eq!(st(&f, ServiceId::Codeagent), (L, "opencode_found", true));
    }
    f.opencode = None;
    assert_eq!(st(&f, ServiceId::Codeagent), (U, "opencode_absent", true));
}

#[test]
fn registry_debounce_two_ticks_and_since() {
    let reg = Registry::new();
    assert!(reg.snapshot().is_empty());
    let mut f = base_facts();
    f.agent = running(Probe::Status(200), 600);

    // first tick publishes as-is
    assert!(reg.tick(compute(&f), "2026-10-13T10:00:00Z"));
    let a = reg.get(ServiceId::Agent).unwrap();
    assert_eq!((a.state, a.since.as_str()), (L, "2026-10-13T10:00:00Z"));

    // one bad tick does not flip (detail is fresh, state is not)
    f.agent.health = Probe::Status(503);
    assert!(!reg.tick(compute(&f), "2026-10-13T10:00:02Z"));
    let a = reg.get(ServiceId::Agent).unwrap();
    assert_eq!(a.state, L);
    assert_eq!(a.reason_code, "healthy");
    assert_eq!(a.detail["health"], json!({"status": 503}));
    // ... and flipping back cancels it
    f.agent.health = Probe::Status(200);
    assert!(!reg.tick(compute(&f), "2026-10-13T10:00:04Z"));
    f.agent.health = Probe::Status(503);
    assert!(!reg.tick(compute(&f), "2026-10-13T10:00:06Z"));
    assert_eq!(reg.get(ServiceId::Agent).unwrap().state, L);
    // the second identical tick publishes, `since` = that tick
    assert!(reg.tick(compute(&f), "2026-10-13T10:00:08Z"));
    let a = reg.get(ServiceId::Agent).unwrap();
    assert_eq!((a.state, a.reason_code), (E, "health_503"));
    assert_eq!(a.since, "2026-10-13T10:00:08Z");
    // steady state: no change, since stays
    assert!(!reg.tick(compute(&f), "2026-10-13T10:00:10Z"));
    assert_eq!(
        reg.get(ServiceId::Agent).unwrap().since,
        "2026-10-13T10:00:08Z"
    );
    // other services kept their first-tick since
    assert_eq!(
        reg.get(ServiceId::Kb).unwrap().since,
        "2026-10-13T10:00:00Z"
    );

    // a can_run_here change is a state change too (debounced the same way)
    f.phone_like = true;
    assert!(!reg.tick(compute(&f), "2026-10-13T10:00:12Z"));
    assert!(reg.get(ServiceId::Radio).unwrap().can_run_here);
    assert!(reg.tick(compute(&f), "2026-10-13T10:00:14Z"));
    assert!(!reg.get(ServiceId::Radio).unwrap().can_run_here);
}

#[test]
fn registry_starting_grace_through_debounce() {
    // a freshly spawned agent whose health is not up yet reads `starting`,
    // then `live` once healthy — never `error` inside the grace
    let reg = Registry::new();
    let mut f = base_facts();
    f.agent = running(Probe::Unreachable("refused".into()), 0);
    reg.tick(compute(&f), "t0");
    assert_eq!(reg.get(ServiceId::Agent).unwrap().state, S);
    for s in [2, 4, 6, 60, 88] {
        f.agent.since_spawn = Some(Duration::from_secs(s));
        reg.tick(compute(&f), "t");
        assert_eq!(reg.get(ServiceId::Agent).unwrap().state, S, "at {s}s");
    }
    f.agent.since_spawn = Some(Duration::from_secs(92));
    reg.tick(compute(&f), "t92");
    assert_eq!(
        reg.get(ServiceId::Agent).unwrap().state,
        S,
        "1 tick: debounced"
    );
    reg.tick(compute(&f), "t94");
    assert_eq!(reg.get(ServiceId::Agent).unwrap().state, E);
}

// ---------------------------------------------------------------- services routes

const RECORD_KEYS: [&str; 12] = [
    "id",
    "name",
    "plain_name",
    "kind",
    "state",
    "since",
    "reason_code",
    "reason",
    "can_run_here",
    "description",
    "actions",
    "detail",
];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn services_routes_and_frames() {
    let n = node();
    let port = serve(&n.api, None).await;
    // no registry in this process yet ⇒ 503
    get(port, "/api/v1/services").problem(503, "internal");
    get(port, "/api/v1/services/agent").problem(503, "internal");

    let reg = Arc::new(Registry::new());
    n.api.insert_ext(Arc::clone(&reg));
    let mut f = base_facts();
    f.agent = running(Probe::Status(200), 600);
    assert!(services::tick_and_publish(
        &n.api,
        &reg,
        &f,
        "2026-10-13T10:00:00Z"
    ));
    assert_eq!(frames(&n.api, "services").len(), 1);

    let list = get(port, "/api/v1/services").json();
    assert_eq!(list["schema"], 1);
    let recs = list["services"].as_array().unwrap();
    let ids: Vec<_> = recs.iter().map(|r| r["id"].as_str().unwrap()).collect();
    // Layer 5 §15/§19: nine ids, `fabric` and `meshtastic` appended
    assert_eq!(
        ids,
        [
            "agent",
            "model",
            "transport",
            "radio",
            "kb",
            "gateway",
            "codeagent",
            "fabric",
            "meshtastic"
        ]
    );
    for r in recs {
        has_all(r, &RECORD_KEYS);
        one_of(
            &r["kind"],
            &[json!("child"), json!("detected"), json!("builtin")],
        );
        one_of(
            &r["state"],
            &[
                json!("live"),
                json!("unset"),
                json!("error"),
                json!("starting"),
            ],
        );
        assert!(is_rfc3339(&r["since"]), "{}", r["since"]);
        assert!(r["reason"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(r["description"].as_str().is_some_and(|s| s.len() > 40));
        assert!(r["detail"].is_object());
        for a in r["actions"].as_array().unwrap() {
            one_of(a, &[json!("restart"), json!("log")]);
            if a == "restart" {
                assert_eq!(r["kind"], "child", "restart only for children: {r}");
            }
        }
        let rc = r["reason_code"].as_str().unwrap();
        assert!(
            rc.bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'_' || b.is_ascii_digit()),
            "reason_code must be snake_case: {rc}"
        );
        if let Some(k) = r.get("kb_entry") {
            assert!(k.as_str().unwrap().ends_with(".md"));
        }
        // Layer 5 §13: `panel` is always present, `null` without a panel
        assert_eq!(r["panel"], Value::Null, "{r}");
    }
    let agent = get(port, "/api/v1/services/agent").json();
    assert_eq!(agent["state"], "live");
    assert_eq!(agent["plain_name"], "Local AI agent");
    assert_eq!(agent["name"], "111-agent");
    assert_eq!(agent["kind"], "child");
    let kb = get(port, "/api/v1/services/kb").json();
    assert_eq!(kb["kind"], "builtin");
    assert!(
        kb.get("kb_entry").is_none(),
        "optional keys omitted when unset"
    );
    let radio = get(port, "/api/v1/services/radio").json();
    assert_eq!(radio["kb_entry"], "projects/reticulum.md");
    assert_eq!(radio["reason_code"], "no_radio");
    // Layer 5 rows with no provider installed read NOT SET honestly
    let fabric = get(port, "/api/v1/services/fabric").json();
    assert_eq!(
        (
            &fabric["plain_name"],
            &fabric["state"],
            &fabric["reason_code"]
        ),
        (&json!("Messaging"), &json!("unset"), &json!("fabric_off"))
    );
    assert_eq!(fabric["kind"], "builtin");
    let mesh = get(port, "/api/v1/services/meshtastic").json();
    assert_eq!(
        (&mesh["plain_name"], &mesh["state"], &mesh["reason_code"]),
        (
            &json!("Meshtastic"),
            &json!("unset"),
            &json!("no_meshtastic")
        )
    );
    assert_eq!(mesh["kind"], "child");
    assert_eq!(mesh["kb_entry"], "projects/meshtastic-web.md");
    let r = get(port, "/api/v1/services/nope");
    r.problem(404, "not_found");
    let detail = r.json()["detail"].as_str().unwrap().to_string();
    for id in ServiceId::ALL {
        assert!(detail.contains(id.as_str()), "{detail}");
    }
    get(port, "/api/v1/services/Agent").problem(404, "not_found");

    // frames: only on a published change
    assert!(!services::tick_and_publish(&n.api, &reg, &f, "t1"));
    f.agent.display = ChildDisplay::Degraded;
    assert!(!services::tick_and_publish(&n.api, &reg, &f, "t2"));
    assert_eq!(frames(&n.api, "services").len(), 1);
    assert!(services::tick_and_publish(&n.api, &reg, &f, "t3"));
    let fr = frames(&n.api, "services");
    assert_eq!(fr.len(), 2);
    assert_eq!(keys(&fr[1]), ["services"]);
    assert_eq!(fr[1]["services"][0]["state"], "error");
    assert_eq!(fr[1]["services"][0]["reason_code"], "child_degraded");
    assert_eq!(get(port, "/api/v1/services/agent").json()["state"], "error");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn service_log_tail() {
    let n = node();
    let port = serve(&n.api, None).await;
    let mut log = String::new();
    for i in 0..600 {
        log.push_str(&format!("line {i}\n"));
    }
    log.push_str("\u{1b}[31mred\u{1b}[0m\tbell\u{7}cr\r\n");
    n.write("agent/111-agent.log", &log);

    let r = get(port, "/api/v1/services/agent/log").json();
    assert_eq!(r["schema"], 1);
    assert_eq!(r["service"], "agent");
    assert!(r["source"]
        .as_str()
        .unwrap()
        .ends_with("agent/111-agent.log"));
    let lines = r["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 100, "default n");
    assert_eq!(lines[99], "red\tbellcr", "control chars stripped, tab kept");
    assert_eq!(lines[98], "line 599");

    for (q, want) in [("1000", 500), ("500", 500), ("0", 1), ("-3", 1), ("7", 7)] {
        let r = get(port, &format!("/api/v1/services/agent/log?n={q}")).json();
        assert_eq!(r["lines"].as_array().unwrap().len(), want, "n={q}");
    }
    get(port, "/api/v1/services/agent/log?n=abc").problem(400, "bad_request");
    get(port, "/api/v1/services/nope/log").problem(404, "not_found");

    // services without a log file, and a child whose file does not exist yet
    for id in [
        "radio",
        "kb",
        "gateway",
        "codeagent",
        "model",
        "transport",
        "fabric",
        "meshtastic",
    ] {
        let r = get(port, &format!("/api/v1/services/{id}/log")).json();
        assert_eq!(r["source"], "none", "{id}");
        assert_eq!(r["lines"], json!([]), "{id}");
        assert_eq!(r["service"], id);
    }
    n.write("rns/rnsd.log", "[2026-10-13] rnsd started\n");
    let r = get(port, "/api/v1/services/transport/log").json();
    assert!(r["source"].as_str().unwrap().ends_with("rns/rnsd.log"));
    assert_eq!(r["lines"], json!(["[2026-10-13] rnsd started"]));
    n.write("agent/llama-server.log", "loading model\n");
    let r = get(port, "/api/v1/services/model/log?n=5").json();
    assert_eq!(r["lines"], json!(["loading model"]));
    // Layer 5 §14: meshmud's log (the path its child spec appends to)
    n.write("meshtastic/meshmud.log", "uplink /dev/ttyACM0 open\n");
    let r = get(port, "/api/v1/services/meshtastic/log").json();
    assert!(r["source"]
        .as_str()
        .unwrap()
        .ends_with("meshtastic/meshmud.log"));
    assert_eq!(r["lines"], json!(["uplink /dev/ttyACM0 open"]));
}

// ---------------------------------------------------------------- settings

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn settings_merge_alias_and_loopback_only() {
    let n = node();
    let lo = serve(&n.api, None).await;
    let lan = serve(&n.api, Some("192.168.1.70:5000")).await;
    let cookie = pair(lan, &n.api);
    let settings_file = n.state_dir().join("settings.json");

    let s = get(lo, "/api/v1/settings").json();
    assert_eq!(
        s,
        json!({"schema": 1, "alias": null, "lan_enabled": false, "lan_locked": false, "offer": null})
    );

    // alias from a paired LAN peer: allowed, trimmed; shows in /status
    let r = send_json(
        lan,
        "PATCH",
        "/api/v1/settings",
        r#"{"alias":"  Kitchen node  "}"#,
        Some(&cookie),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["alias"], "Kitchen node");
    assert_eq!(
        get(lo, "/api/v1/status").json()["node"]["alias"],
        "Kitchen node"
    );
    let status_frames = frames(&n.api, "status");
    assert_eq!(status_frames.len(), 1, "a settings change publishes status");
    assert_eq!(status_frames[0]["node"]["alias"], "Kitchen node");
    assert_eq!(
        status_frames[0]["auth"],
        json!({"loopback": false, "paired": false}),
        "broadcast frames carry the placeholder auth block"
    );

    // loopback-only fields from LAN: 403, nothing written (even mixed with alias)
    for body in [
        r#"{"lan_enabled":true}"#,
        r#"{"offer":"accepted"}"#,
        r#"{"alias":"other","lan_enabled":false}"#,
    ] {
        send_json(lan, "PATCH", "/api/v1/settings", body, Some(&cookie))
            .problem(403, "loopback_only");
    }
    let s = get_cookie(lan, "/api/v1/settings", &cookie).json();
    assert_eq!(s["alias"], "Kitchen node");
    assert_eq!(s["lan_enabled"], false);
    assert!(!n.state_dir().join("offer.json").exists());

    // loopback: lan_enabled merges, alias survives
    let r = send_json(
        lo,
        "PATCH",
        "/api/v1/settings",
        r#"{"lan_enabled":true}"#,
        None,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(
        r.json(),
        json!({"schema": 1, "alias": "Kitchen node", "lan_enabled": true, "lan_locked": false, "offer": null})
    );
    let on_disk: Value = serde_json::from_slice(&std::fs::read(&settings_file).unwrap()).unwrap();
    assert_eq!(on_disk["schema"], 1);
    assert_eq!(on_disk["lan_enabled"], true);
    assert_eq!(on_disk["alias"], "Kitchen node");
    // the daemon's start-time reader agrees
    assert!(node111::daemon::dashboard::bind::lan_configured(
        &[],
        &n.state_dir()
    ));
    assert_eq!(get(lo, "/api/v1/status").json()["lan_enabled"], true);

    // validation
    let long = json!({ "alias": "x".repeat(65) }).to_string();
    send_json(lo, "PATCH", "/api/v1/settings", &long, None).problem(400, "bad_request");
    for body in [
        r#"{"alias":3}"#,
        r#"{"lan_enabled":"yes"}"#,
        r#"{"offer":"offered"}"#,
        r#"{"nickname":"x"}"#,
        r#"[]"#,
        "nope",
    ] {
        send_json(lo, "PATCH", "/api/v1/settings", body, None).problem(400, "bad_request");
    }
    // empty alias clears; null clears
    let r = send_json(lo, "PATCH", "/api/v1/settings", r#"{"alias":"   "}"#, None);
    assert!(r.json()["alias"].is_null());
    send_json(lo, "PATCH", "/api/v1/settings", r#"{"alias":"x"}"#, None);
    let r = send_json(lo, "PATCH", "/api/v1/settings", r#"{"alias":null}"#, None);
    assert!(r.json()["alias"].is_null());
    assert!(get(lo, "/api/v1/status").json()["node"]["alias"].is_null());

    // configured lan_enabled: settings, unless the env file's DASHBOARD_LAN
    // pins it (then lan_locked, and a lan_enabled PATCH is 409; the full matrix
    // lives in tests/settings_lan.rs)
    send_json(
        lo,
        "PATCH",
        "/api/v1/settings",
        r#"{"lan_enabled":false}"#,
        None,
    );
    assert_eq!(get(lo, "/api/v1/settings").json()["lan_enabled"], false);
    n.write("etc/env", "DASHBOARD_LAN=true\n");
    let s = get(lo, "/api/v1/settings").json();
    assert_eq!(s["lan_enabled"], true);
    assert_eq!(s["lan_locked"], true);
    assert_eq!(get(lo, "/api/v1/status").json()["lan_enabled"], true);
    send_json(
        lo,
        "PATCH",
        "/api/v1/settings",
        r#"{"lan_enabled":false}"#,
        None,
    )
    .problem(409, "settings_locked");

    // PATCH needs JSON (CSRF guard from R1 still applies)
    request(
        lo,
        "PATCH",
        "/api/v1/settings",
        &[("Content-Type", "text/plain")],
        Some("{}"),
    )
    .problem(415, "bad_request");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn settings_offer_dual_write() {
    let n = node();
    let lo = serve(&n.api, None).await;
    // Layer 1 recorded an offer at install
    n.write(
        "state/offer.json",
        r#"{"schema":1,"tier":"standard","state":"offered","recorded_at":"2026-10-01T00:00:00Z"}"#,
    );
    assert_eq!(get(lo, "/api/v1/settings").json()["offer"], "offered");

    let r = send_json(
        lo,
        "PATCH",
        "/api/v1/settings",
        r#"{"offer":"accepted"}"#,
        None,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["offer"], "accepted");
    // state/offer.json in exactly the `111 config offer` format
    let offer: Value =
        serde_json::from_slice(&std::fs::read(n.state_dir().join("offer.json")).unwrap()).unwrap();
    let mut k = keys(&offer);
    k.sort();
    assert_eq!(k, ["recorded_at", "schema", "state", "tier"]);
    assert_eq!(offer["schema"], 1);
    assert_eq!(offer["tier"], "standard");
    assert_eq!(offer["state"], "accepted");
    assert!(is_rfc3339(&offer["recorded_at"]));
    // settings.json mirrors it; /status.offer follows immediately
    let settings: Value =
        serde_json::from_slice(&std::fs::read(n.state_dir().join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(settings["offer"], "accepted");
    assert_eq!(
        get(lo, "/api/v1/status").json()["offer"],
        json!({"tier": "standard", "state": "accepted"})
    );

    // `111 config offer dismiss` (Layer 1) wins on read: offer.json is the record
    n.write(
        "state/offer.json",
        r#"{"schema":1,"tier":"standard","state":"dismissed","recorded_at":"2026-10-02T00:00:00Z"}"#,
    );
    assert_eq!(get(lo, "/api/v1/settings").json()["offer"], "dismissed");
}

// ---------------------------------------------------------------- kb

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kb_index_entry_and_traversal_refusals() {
    let n = node();
    let port = serve(&n.api, None).await;
    // no KB payload yet
    get(port, "/api/v1/kb").problem(404, "not_found");
    get(port, "/api/v1/kb/entry?path=projects/meshtastic.md").problem(404, "not_found");

    // `current` is a symlink to a release dir, as on a real node
    let rel = n.root().join("releases/2026.10.13");
    std::fs::create_dir_all(rel.join("share/kb/projects")).unwrap();
    std::os::unix::fs::symlink(&rel, n.root().join("current")).unwrap();
    std::fs::write(rel.join("share/kb/llms.txt"), "# 111 KB\n- projects/\n").unwrap();
    std::fs::write(
        rel.join("share/kb/projects/meshtastic.md"),
        "# Meshtastic\nLoRa mesh.\n",
    )
    .unwrap();
    std::fs::write(rel.join("share/kb/projects/notes.txt"), "not md").unwrap();
    std::fs::write(n.root().join("secret.md"), "# secret").unwrap();
    std::os::unix::fs::symlink(
        n.root().join("secret.md"),
        rel.join("share/kb/projects/evil.md"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        rel.join("share/kb/projects/notes.txt"),
        rel.join("share/kb/projects/sneaky.md"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        rel.join("share/kb/projects/meshtastic.md"),
        rel.join("share/kb/projects/alias.md"),
    )
    .unwrap();

    let r = get(port, "/api/v1/kb");
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("text/plain; charset=utf-8"));
    assert!(r.body.starts_with("# 111 KB"));

    let r = get(port, "/api/v1/kb/entry?path=projects/meshtastic.md");
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(
        r.header("content-type"),
        Some("text/markdown; charset=utf-8")
    );
    assert_eq!(r.body, "# Meshtastic\nLoRa mesh.\n");
    // url-encoded slash, and an in-KB symlink, are fine
    assert_eq!(
        get(port, "/api/v1/kb/entry?path=projects%2Fmeshtastic.md").status,
        200
    );
    assert_eq!(
        get(port, "/api/v1/kb/entry?path=projects/alias.md").status,
        200
    );

    for q in [
        "",
        "?path=",
        "?other=1",
        "?path=../secret.md",
        "?path=..%2Fsecret.md",
        "?path=%2e%2e/%2e%2e/secret.md",
        "?path=projects/../../../secret.md",
        "?path=/etc/passwd",
        "?path=%2Fetc%2Fhostname.md",
        "?path=projects/notes.txt",
        "?path=llms.txt",
        "?path=projects/.hidden.md",
        "?path=projects//meshtastic.md",
        "?path=projects%5Cmeshtastic.md",
        "?path=projects/evil.md",
        "?path=projects/sneaky.md",
    ] {
        get(port, &format!("/api/v1/kb/entry{q}")).problem(400, "bad_request");
    }
    get(port, "/api/v1/kb/entry?path=projects/missing.md").problem(404, "not_found");
    get(port, "/api/v1/kb/entry?path=nodir/missing.md").problem(404, "not_found");
}

// ---------------------------------------------------------------- actions

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restart_replies_then_signals() {
    let n = node();
    let port = serve(&n.api, None).await;
    // not wired in this process ⇒ 503
    send_json(port, "POST", "/api/v1/node/restart", "{}", None).problem(503, "internal");

    let signal = Arc::new(tokio::sync::Notify::new());
    n.api
        .insert_ext(actions::RestartSignal(Arc::clone(&signal)));
    let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let waiter = {
        let (signal, fired) = (Arc::clone(&signal), Arc::clone(&fired));
        tokio::spawn(async move {
            signal.notified().await;
            fired.store(true, std::sync::atomic::Ordering::SeqCst);
        })
    };
    let t0 = Instant::now();
    let r = send_json(port, "POST", "/api/v1/node/restart", "{}", None);
    assert_eq!(r.status, 202, "{}", r.body);
    assert_eq!(r.json(), json!({"schema": 1, "restarting": true}));
    assert!(
        !fired.load(std::sync::atomic::Ordering::SeqCst),
        "signal must wait for the reply"
    );
    tokio::time::timeout(Duration::from_secs(3), waiter)
        .await
        .expect("restart signal fired")
        .unwrap();
    assert!(
        t0.elapsed() >= Duration::from_millis(250),
        "{:?}",
        t0.elapsed()
    );

    // LAN peers need a session (gate), and CSRF still applies
    let lan = serve(&n.api, Some("10.0.0.9:4000")).await;
    send_json(lan, "POST", "/api/v1/node/restart", "{}", None).problem(401, "unauthenticated");
    request(
        port,
        "POST",
        "/api/v1/node/restart",
        &[
            ("Content-Type", "application/json"),
            ("Origin", "http://evil.example"),
        ],
        Some("{}"),
    )
    .problem(403, "bad_request");
}

fn sleeper(name: &str) -> supervise::ChildSpec {
    supervise::ChildSpec {
        name: name.into(),
        program: "/bin/sleep".into(),
        args: vec!["30".into()],
        env: vec![],
        backoff: supervise::BackoffPolicy::default(),
        stdout_log: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn service_restart_route() {
    let n = node();
    let port = serve(&n.api, None).await;
    // nothing supervised in this process: every known id is 409, unknown 404
    for id in ServiceId::ALL {
        send_json(
            port,
            "POST",
            &format!("/api/v1/services/{}/restart", id.as_str()),
            "{}",
            None,
        )
        .problem(409, "not_supervised");
    }
    send_json(port, "POST", "/api/v1/services/nope/restart", "{}", None).problem(404, "not_found");

    // the model server is a supervised child; transport is the stub (absent)
    let model = supervise::spawn_child(sleeper("llama-server"));
    assert!(wait_until(Duration::from_secs(3), || model.pid().is_some()).await);
    let old = model.pid().unwrap();
    n.api.insert_ext(Arc::new(
        services::Supervised::default()
            .with(ServiceId::Model, Some(model.control()))
            .with(ServiceId::Transport, None),
    ));
    let r = send_json(port, "POST", "/api/v1/services/model/restart", "{}", None);
    assert_eq!(r.status, 202, "{}", r.body);
    assert_eq!(
        r.json(),
        json!({"schema": 1, "id": "model", "restarting": true})
    );
    assert!(
        wait_until(Duration::from_secs(5), || model
            .pid()
            .is_some_and(|p| p != old))
        .await,
        "respawned with a new pid"
    );
    assert_eq!(model.state(), supervise::ChildState::Running);
    for id in ["transport", "agent", "radio", "kb", "gateway", "codeagent"] {
        let r = send_json(
            port,
            "POST",
            &format!("/api/v1/services/{id}/restart"),
            "{}",
            None,
        );
        r.problem(409, "not_supervised");
        assert!(r.json()["detail"].as_str().is_some_and(|d| !d.is_empty()));
    }

    // same gate as every write: LAN needs a session, CSRF applies, POST only
    let lan = serve(&n.api, Some("10.0.0.9:4000")).await;
    send_json(lan, "POST", "/api/v1/services/model/restart", "{}", None)
        .problem(401, "unauthenticated");
    request(
        port,
        "POST",
        "/api/v1/services/model/restart",
        &[
            ("Content-Type", "application/json"),
            ("Origin", "http://evil.example"),
        ],
        Some("{}"),
    )
    .problem(403, "bad_request");
    request(
        port,
        "POST",
        "/api/v1/services/model/restart",
        &[("Content-Type", "text/plain")],
        Some("{}"),
    )
    .problem(415, "bad_request");
    get(port, "/api/v1/services/model/restart").problem(405, "bad_request");

    // after shutdown the child is gone for good: 409
    model.stop_and_wait(Duration::from_secs(5)).await;
    send_json(port, "POST", "/api/v1/services/model/restart", "{}", None)
        .problem(409, "not_supervised");

    // Layer 5 §14: `meshtastic` owns two children — both restart, meshmud
    // first, then the presence reader
    let meshmud = supervise::spawn_child(sleeper("meshmud"));
    let reader = supervise::spawn_child(sleeper("meshtastic-presence"));
    assert!(
        wait_until(Duration::from_secs(3), || meshmud.pid().is_some()
            && reader.pid().is_some())
        .await
    );
    let (old_m, old_r) = (meshmud.pid().unwrap(), reader.pid().unwrap());
    n.api
        .insert_ext(Arc::new(services::Supervised::default().with_children(
            ServiceId::Meshtastic,
            vec![meshmud.control(), reader.control()],
        )));
    let r = send_json(
        port,
        "POST",
        "/api/v1/services/meshtastic/restart",
        "{}",
        None,
    );
    assert_eq!(r.status, 202, "{}", r.body);
    assert!(
        wait_until(Duration::from_secs(10), || meshmud
            .pid()
            .is_some_and(|p| p != old_m)
            && reader.pid().is_some_and(|p| p != old_r))
        .await,
        "both children respawned"
    );
    // the primary stopped for good ⇒ the service is no longer restartable
    meshmud.stop_and_wait(Duration::from_secs(5)).await;
    send_json(
        port,
        "POST",
        "/api/v1/services/meshtastic/restart",
        "{}",
        None,
    )
    .problem(409, "not_supervised");
    reader.stop_and_wait(Duration::from_secs(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn update_check_notifies_and_reports_checking() {
    let n = node();
    let port = serve(&n.api, None).await;

    // without an updater in-process: still wakes, says started:false
    let woke = {
        let wake = Arc::clone(&n.api.update_check);
        tokio::spawn(async move { wake.notified().await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    let r = send_json(port, "POST", "/api/v1/update/check", "{}", None);
    assert_eq!(r.status, 202);
    assert_eq!(r.json(), json!({"schema": 1, "started": false}));
    tokio::time::timeout(Duration::from_secs(2), woke)
        .await
        .expect("notify fired")
        .unwrap();

    // with the updater status: started + `checking` until the loop finishes
    let st = Arc::new(UpdaterStatus::default());
    n.api.insert_ext(Arc::clone(&st));
    let woke = {
        let wake = Arc::clone(&n.api.update_check);
        tokio::spawn(async move { wake.notified().await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    let r = send_json(port, "POST", "/api/v1/update/check", "{}", None);
    assert_eq!(r.json(), json!({"schema": 1, "started": true}));
    tokio::time::timeout(Duration::from_secs(2), woke)
        .await
        .expect("notify fired")
        .unwrap();
    assert!(st.checking());
    assert_eq!(
        get(port, "/api/v1/status").json()["update"]["state"],
        "checking"
    );
    let last = frames(&n.api, "status");
    assert_eq!(last.last().unwrap()["update"]["state"], "checking");

    // a second request while checking joins the running check: no new permit
    let r = send_json(port, "POST", "/api/v1/update/check", "{}", None);
    assert_eq!(r.status, 202);
    let wake = Arc::clone(&n.api.update_check);
    let extra = tokio::time::timeout(Duration::from_millis(300), wake.notified()).await;
    assert!(extra.is_err(), "coalesced: no second wake queued");
}

// ---------------------------------------------------------------- heartbeat

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn status_heartbeat_publishes_on_change_only() {
    let n = node();
    let _p = status::spawn_heartbeat_every(Arc::clone(&n.api), Duration::from_millis(40));
    assert!(
        wait_until(Duration::from_secs(3), || !frames(&n.api, "status")
            .is_empty())
        .await
    );
    let first = frames(&n.api, "status");
    assert_eq!(first.len(), 1);
    has_all(
        &first[0],
        &[
            "schema",
            "node",
            "update",
            "bind",
            "auth",
            "layers",
            "lan_enabled",
        ],
    );
    // uptime ticking alone is not a change
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert_eq!(frames(&n.api, "status").len(), 1);
    // a real change publishes within a tick
    n.api.dash.write().unwrap().transport_state = TransportState::Joined;
    assert!(
        wait_until(Duration::from_secs(3), || frames(&n.api, "status").len()
            == 2)
        .await
    );
    assert_eq!(frames(&n.api, "status")[1]["transport_state"], "joined");
}
