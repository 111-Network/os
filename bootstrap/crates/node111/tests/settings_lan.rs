//! LAN sharing has one source of truth (`state/settings.json` `lan_enabled`);
//! `DASHBOARD_LAN` in the env file is an explicit override that locks it
//! (contract: `Settings.lan_locked`, PATCH → 409 `settings_locked`).
//! `111 config dashboard.lan` writes the same file and removes the override.

use axum::extract::connect_info::MockConnectInfo;
use node111::config;
use node111::daemon::dashboard::api_v1::auth;
use node111::daemon::dashboard::bind::{self, LanConfig};
use node111::daemon::dashboard::{
    routes, serve_listener, ApiConfig, ApiState, DashboardState, SharedState,
};
use node111::daemon::DaemonLayout;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, RwLock};
use std::time::Duration;

// ---------------------------------------------------------------- harness

struct Resp {
    status: u16,
    content_type: Option<String>,
    set_cookie: Option<String>,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", self.body))
    }
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
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let header = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    Resp {
        status,
        content_type: header("content-type"),
        set_cookie: header("set-cookie"),
        body: body.to_string(),
    }
}

fn get(port: u16, path: &str) -> Value {
    let r = request(port, "GET", path, &[], None);
    assert_eq!(r.status, 200, "{}", r.body);
    r.json()
}

fn patch(port: u16, body: &str, cookie: Option<&str>) -> Resp {
    let mut h = vec![("Content-Type", "application/json")];
    if let Some(c) = cookie {
        h.push(("Cookie", c));
    }
    request(port, "PATCH", "/api/v1/settings", &h, Some(body))
}

struct Node {
    api: Arc<ApiState>,
    _dir: tempfile::TempDir,
}

impl Node {
    fn state_dir(&self) -> std::path::PathBuf {
        self.api.layout.state_dir()
    }
    fn env_file(&self) -> std::path::PathBuf {
        self.api.layout.etc.join("env")
    }
    fn write_env(&self, body: &str) {
        std::fs::create_dir_all(&self.api.layout.etc).unwrap();
        std::fs::write(self.env_file(), body).unwrap();
    }
    fn write_settings(&self, lan: bool) {
        std::fs::write(
            self.state_dir().join(bind::SETTINGS_FILE),
            json!({"schema": 1, "alias": "kept", "lan_enabled": lan}).to_string(),
        )
        .unwrap();
    }
    fn settings_on_disk(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.state_dir().join(bind::SETTINGS_FILE)).unwrap())
            .unwrap()
    }
    /// What the daemon will read at its next start.
    fn daemon_view(&self) -> LanConfig {
        let env = node111::daemon::load_env(&self.api.layout.etc).unwrap();
        bind::lan_config(&env, &self.state_dir())
    }
}

fn node() -> Node {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.14")));
    Node {
        api: ApiState::new(dash, ApiConfig::new(layout, false)),
        _dir: dir,
    }
}

/// `peer: None` = real loopback ConnectInfo, else every request comes from `peer`.
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
    let r = request(
        port,
        "POST",
        "/api/v1/auth/pair",
        &[("Content-Type", "application/json")],
        Some(&json!({ "code": code }).to_string()),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let sc = r.set_cookie.expect("Set-Cookie");
    sc.split(';').next().unwrap().to_string()
}

// ---------------------------------------------------------------- tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn settings_file_decides_when_the_env_is_silent() {
    let n = node();
    let lo = serve(&n.api, None).await;

    // nothing anywhere: off, unlocked (contract: lan_locked is required)
    let s = get(lo, "/api/v1/settings");
    assert_eq!(
        s,
        json!({"schema": 1, "alias": null, "lan_enabled": false, "lan_locked": false, "offer": null})
    );

    for lan in [true, false] {
        n.write_settings(lan);
        let s = get(lo, "/api/v1/settings");
        assert_eq!(s["lan_enabled"], lan);
        assert_eq!(s["lan_locked"], false);
        assert_eq!(get(lo, "/api/v1/status")["lan_enabled"], lan);
        assert_eq!(
            n.daemon_view(),
            LanConfig {
                enabled: lan,
                locked: false
            }
        );
    }

    // an env file without DASHBOARD_LAN changes nothing; the toggle round-trips
    n.write_env("# 111 node config\nOTHER=1\n");
    let r = patch(lo, r#"{"lan_enabled":true}"#, None);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["lan_enabled"], true);
    assert_eq!(r.json()["lan_locked"], false);
    let r = patch(lo, r#"{"lan_enabled":false}"#, None);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["lan_enabled"], false);
    assert_eq!(n.settings_on_disk()["lan_enabled"], false);
    assert!(!n.daemon_view().enabled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn env_override_wins_both_ways_and_locks_the_toggle() {
    let n = node();
    let lo = serve(&n.api, None).await;
    let lan = serve(&n.api, Some("192.168.1.70:5000")).await;
    let cookie = pair(lan, &n.api);

    // env false over settings true: OFF (the old OR-logic said on)
    n.write_settings(true);
    n.write_env("DASHBOARD_LAN=false\n");
    let s = get(lo, "/api/v1/settings");
    assert_eq!(s["lan_enabled"], false);
    assert_eq!(s["lan_locked"], true);
    assert_eq!(get(lo, "/api/v1/status")["lan_enabled"], false);
    assert_eq!(
        n.daemon_view(),
        LanConfig {
            enabled: false,
            locked: true
        }
    );

    // env true over settings false: ON
    n.write_settings(false);
    n.write_env("DASHBOARD_LAN=true\n");
    let s = get(lo, "/api/v1/settings");
    assert_eq!(s["lan_enabled"], true);
    assert_eq!(s["lan_locked"], true);
    assert!(n.daemon_view().enabled);

    // PATCH lan_enabled while pinned: 409 settings_locked naming the env file,
    // nothing written — for either value, and mixed with an alias
    let before = n.settings_on_disk();
    for body in [
        r#"{"lan_enabled":false}"#,
        r#"{"lan_enabled":true}"#,
        r#"{"alias":"x","lan_enabled":false}"#,
    ] {
        let r = patch(lo, body, None);
        assert_eq!(r.status, 409, "{body}: {}", r.body);
        assert_eq!(r.content_type.as_deref(), Some("application/problem+json"));
        let p = r.json();
        assert_eq!(p["code"], "settings_locked", "{p}");
        assert_eq!(p["status"], 409);
        let detail = p["detail"].as_str().unwrap();
        assert!(detail.contains("DASHBOARD_LAN"), "{detail}");
        assert!(
            detail.contains(&n.env_file().display().to_string()),
            "{detail}"
        );
    }
    assert_eq!(
        n.settings_on_disk(),
        before,
        "a locked PATCH writes nothing"
    );

    // other fields still save while LAN is pinned
    let r = patch(lo, r#"{"alias":"Kitchen node"}"#, None);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["alias"], "Kitchen node");
    assert_eq!(r.json()["lan_locked"], true);

    // a LAN peer is refused as loopback_only first (it may not touch the field at all)
    let r = patch(lan, r#"{"lan_enabled":false}"#, Some(&cookie));
    assert_eq!(r.status, 403, "{}", r.body);
    assert_eq!(r.json()["code"], "loopback_only");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_cli_writes_settings_and_removes_the_override() {
    let n = node();
    let lo = serve(&n.api, None).await;
    n.write_settings(false);
    n.write_env("# 111 node config\nNODE111_X=keep\nDASHBOARD_LAN=true\nDASHBOARD_LAN=false\n");
    assert!(get(lo, "/api/v1/settings")["lan_locked"] == true);

    // `111 config dashboard.lan true`
    let change = config::set_dashboard_lan(&n.state_dir(), &n.env_file(), true).unwrap();
    assert!(change.env_line_removed);
    let env = std::fs::read_to_string(n.env_file()).unwrap();
    assert!(!env.contains("DASHBOARD_LAN"), "{env}");
    assert!(env.contains("NODE111_X=keep"), "{env}");
    let disk = n.settings_on_disk();
    assert_eq!(disk["lan_enabled"], true);
    assert_eq!(disk["alias"], "kept", "the merge keeps other settings");
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(n.state_dir().join(bind::SETTINGS_FILE))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o644, "same mode the daemon's PATCH writes");

    // dashboard and daemon now agree: on, unlocked, and the toggle works again
    let s = get(lo, "/api/v1/settings");
    assert_eq!(s["lan_enabled"], true);
    assert_eq!(s["lan_locked"], false);
    assert_eq!(
        n.daemon_view(),
        LanConfig {
            enabled: true,
            locked: false
        }
    );
    let r = patch(lo, r#"{"lan_enabled":false}"#, None);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["lan_enabled"], false);

    // `111 config dashboard.lan false` with no env line: env file untouched
    let env_before = std::fs::read(n.env_file()).unwrap();
    let change = config::set_dashboard_lan(&n.state_dir(), &n.env_file(), false).unwrap();
    assert!(!change.env_line_removed);
    assert_eq!(std::fs::read(n.env_file()).unwrap(), env_before);
    assert_eq!(n.settings_on_disk()["lan_enabled"], false);

    // no env file at all: settings still written, nothing created in etc/
    std::fs::remove_file(n.env_file()).unwrap();
    let change = config::set_dashboard_lan(&n.state_dir(), &n.env_file(), true).unwrap();
    assert!(!change.env_line_removed);
    assert!(!n.env_file().exists());
    assert!(n.daemon_view().enabled);
}
