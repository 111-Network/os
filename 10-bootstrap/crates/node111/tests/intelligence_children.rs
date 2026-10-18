//! Layer 3 wiring: 111d supervises `llama-server` and `111-agent` as children.
//! Fake child binaries (sh scripts) on a temp root; every test finishes well
//! under 10s. Absent pieces are honest `absent`, never errors.

use node111::daemon::dashboard::{serve, DashboardState, SharedState};
use node111::daemon::intelligence::{start, ChildDisplay, IntelligenceState, AGENT_URL};
use node111::daemon::DaemonLayout;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// A §7 machine-string NodeId (lowercase hex of a 32B Ed25519 public key).
const NODE_ID_HEX: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

fn layout_at(root: &Path) -> DaemonLayout {
    DaemonLayout {
        root: root.to_path_buf(),
        etc: root.join("etc"),
    }
}

fn write_exec(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn install_agent(root: &Path, body: &str) {
    write_exec(&root.join("current/bin/111-agent"), body);
}

fn install_llama(root: &Path, body: &str) {
    write_exec(&root.join("current/bin/llama-server"), body);
}

fn install_model(root: &Path, name: &str) -> PathBuf {
    let dir = root.join("models/current");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), b"GGUF").unwrap();
    dir.join(name)
}

async fn wait_for(what: &str, timeout: Duration, mut cond: impl FnMut() -> bool) {
    let t0 = Instant::now();
    while !cond() {
        assert!(t0.elapsed() < timeout, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

fn read_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_absent_is_honest_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(&layout_at(dir.path()), Some("Node2A"), Some(NODE_ID_HEX));
    assert_eq!(h.state(), IntelligenceState::default());
    assert_eq!(h.state().agent, ChildDisplay::Absent);
    assert_eq!(h.state().model_server, ChildDisplay::Absent);
    assert_eq!(h.state().agent_url, None);
    assert!(h.daemon_token().is_none(), "no agent, no bearer token");
    assert!(
        !dir.path().join("agent").exists(),
        "nothing spawned, nothing created"
    );
    h.stop(); // no-op, no panic
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_server_needs_both_binary_and_model() {
    // binary, no model
    let dir = tempfile::tempdir().unwrap();
    install_llama(dir.path(), "#!/bin/sh\nexec sleep 300\n");
    let h = start(&layout_at(dir.path()), None, None);
    assert_eq!(h.state().model_server, ChildDisplay::Absent);
    h.stop();

    // model, no binary
    let dir = tempfile::tempdir().unwrap();
    install_model(dir.path(), "x.gguf");
    let h = start(&layout_at(dir.path()), None, None);
    assert_eq!(h.state().model_server, ChildDisplay::Absent);
    h.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_spawns_with_expected_env() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let state_dir = root.join("agent");
    // The fake reads its exec-time environment block directly, so the
    // assertion is on what 111d passed, independent of shell import rules.
    install_agent(
        root,
        &format!(
            "#!/bin/sh\n\
             tr '\\0' '\\n' < /proc/$$/environ > '{sd}/env.tmp'\n\
             mv '{sd}/env.tmp' '{sd}/env.txt'\n\
             exec sleep 300\n",
            sd = state_dir.display()
        ),
    );
    let h = start(&layout_at(root), Some("Node2A"), Some(NODE_ID_HEX));
    wait_for("agent running", Duration::from_secs(5), || {
        h.state().agent == ChildDisplay::Running
    })
    .await;
    let env_txt = state_dir.join("env.txt");
    wait_for("env dump", Duration::from_secs(5), || env_txt.exists()).await;

    let env = std::fs::read_to_string(&env_txt).unwrap();
    let lines: Vec<&str> = env.lines().collect();
    for want in [
        "AGENT111_CATEGORY=Node2A".to_string(),
        format!("AGENT111_KB_DIR={}/current/share/kb", root.display()),
        format!("AGENT111_STATE_DIR={}/agent", root.display()),
        format!("AGENT111_ENV_FILE={}/etc/agent.env", root.display()),
        format!("AGENT111_NODE_ID={NODE_ID_HEX}"),
        // Layer 5 §17: loopback API base + the per-spawn bearer token
        "AGENT111_DAEMON_API_URL=http://127.0.0.1:11180/api/v1".to_string(),
        format!(
            "AGENT111_DAEMON_TOKEN={}",
            h.daemon_token()
                .expect("minted for a spawned agent")
                .as_str()
        ),
    ] {
        assert!(lines.contains(&want.as_str()), "missing {want:?} in {env}");
    }

    let s = h.state();
    assert_eq!(s.agent, ChildDisplay::Running);
    assert_eq!(s.agent_url.as_deref(), Some(AGENT_URL));
    assert_eq!(s.model_server, ChildDisplay::Absent);

    // state dir is private; the supervisor opened the child's log there
    let mode = state_dir.metadata().unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
    assert!(state_dir.join("111-agent.log").exists());
    h.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_defaults_when_category_and_node_id_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let state_dir = root.join("agent");
    install_agent(
        root,
        &format!(
            "#!/bin/sh\n\
             tr '\\0' '\\n' < /proc/$$/environ > '{sd}/env.tmp'\n\
             mv '{sd}/env.tmp' '{sd}/env.txt'\n\
             exec sleep 300\n",
            sd = state_dir.display()
        ),
    );
    let h = start(&layout_at(root), None, None);
    let env_txt = state_dir.join("env.txt");
    wait_for("env dump", Duration::from_secs(5), || env_txt.exists()).await;
    let env = std::fs::read_to_string(&env_txt).unwrap();
    assert!(env.lines().any(|l| l == "AGENT111_CATEGORY=unknown"));
    assert!(
        !env.lines().any(|l| l.starts_with("AGENT111_NODE_ID=")),
        "no node id known ⇒ none passed"
    );
    h.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_server_spawns_with_model_argv() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let argv = root.join("agent/argv.txt");
    install_llama(
        root,
        &format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$@\" > '{a}.tmp'\n\
             mv '{a}.tmp' '{a}'\n\
             exec sleep 300\n",
            a = argv.display()
        ),
    );
    // Production layout: models/current is a symlink to a model release.
    let release = root.join("models/releases/m1");
    std::fs::create_dir_all(&release).unwrap();
    std::fs::write(release.join("b.gguf"), b"GGUF").unwrap();
    std::fs::write(release.join("a.gguf"), b"GGUF").unwrap();
    std::fs::write(release.join("notes.txt"), b"not a model").unwrap();
    std::os::unix::fs::symlink("releases/m1", root.join("models/current")).unwrap();

    let h = start(&layout_at(root), Some("Node2A"), Some(NODE_ID_HEX));
    wait_for("model server running", Duration::from_secs(5), || {
        h.state().model_server == ChildDisplay::Running
    })
    .await;
    wait_for("argv dump", Duration::from_secs(5), || argv.exists()).await;

    let got = std::fs::read_to_string(&argv).unwrap();
    let want = format!(
        "--model\n{}/models/current/a.gguf\n--host\n127.0.0.1\n--port\n11181\n",
        root.display()
    );
    assert_eq!(
        got, want,
        "first .gguf alphabetically, loopback, port 11181"
    );

    let s = h.state();
    assert_eq!(s.model_server, ChildDisplay::Running);
    assert_eq!(s.agent, ChildDisplay::Absent, "no agent binary installed");
    assert_eq!(
        s.agent_url, None,
        "agent_url follows the agent, not the model"
    );
    assert!(root.join("agent/llama-server.log").exists());
    h.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_looping_agent_degrades_without_panic() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let runs = root.join("agent/runs.txt");
    install_agent(
        root,
        &format!("#!/bin/sh\necho run >> '{}'\nexit 1\n", runs.display()),
    );
    let h = start(&layout_at(root), Some("Node2A"), None);

    // Default policy: five exits inside 60s ⇒ Degraded after roughly 3-5s of
    // backoff (250ms doubling, ±20% jitter). Until then it is running or
    // backing off, never absent or stopped.
    let t0 = Instant::now();
    loop {
        let s = h.state();
        assert!(
            matches!(
                s.agent,
                ChildDisplay::Running | ChildDisplay::Backoff | ChildDisplay::Degraded
            ),
            "unexpected {s:?}"
        );
        assert_eq!(s.model_server, ChildDisplay::Absent);
        if s.agent == ChildDisplay::Degraded {
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(9),
            "never degraded: {s:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let s = h.state();
    assert_eq!(s.agent, ChildDisplay::Degraded);
    assert_eq!(s.agent_url, None, "a degraded agent is not advertised");
    let n = std::fs::read_to_string(&runs).unwrap().lines().count();
    assert!((1..=5).contains(&n), "restarts stop at the threshold: {n}");
    h.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_terminates_both_children() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let agent_pid = root.join("agent/agent.pid");
    let model_pid = root.join("agent/model.pid");
    // `exec sleep` keeps the pid, so the recorded pid is the child itself.
    install_agent(
        root,
        &format!(
            "#!/bin/sh\necho $$ > '{p}.tmp'\nmv '{p}.tmp' '{p}'\nexec sleep 300\n",
            p = agent_pid.display()
        ),
    );
    install_llama(
        root,
        &format!(
            "#!/bin/sh\necho $$ > '{p}.tmp'\nmv '{p}.tmp' '{p}'\nexec sleep 300\n",
            p = model_pid.display()
        ),
    );
    install_model(root, "m.gguf");

    let h = start(&layout_at(root), Some("Node2A"), Some(NODE_ID_HEX));
    wait_for("both pids", Duration::from_secs(5), || {
        read_pid(&agent_pid).is_some() && read_pid(&model_pid).is_some()
    })
    .await;
    let s = h.state();
    assert_eq!(s.agent, ChildDisplay::Running);
    assert_eq!(s.model_server, ChildDisplay::Running);
    let (a, m) = (read_pid(&agent_pid).unwrap(), read_pid(&model_pid).unwrap());
    assert!(alive(a) && alive(m));

    h.stop();
    wait_for("children gone", Duration::from_secs(5), || {
        !alive(a) && !alive(m)
    })
    .await;
    wait_for("stopped states", Duration::from_secs(5), || {
        let s = h.state();
        s.agent == ChildDisplay::Stopped && s.model_server == ChildDisplay::Stopped
    })
    .await;
    assert_eq!(h.state().agent_url, None);
}

// ---- /api/status ----

fn http_get(port: u16, path: &str) -> (u16, String) {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    conn.write_all(req.as_bytes()).unwrap();
    let mut buf = String::new();
    conn.read_to_string(&mut buf).unwrap();
    let status: u16 = buf
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

async fn serve_state(state: SharedState) -> u16 {
    let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let bind = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    tokio::spawn(async move {
        let _ = serve(state, bind, false).await;
    });
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return port;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("server never came up");
}

fn status_json(port: u16) -> serde_json::Value {
    let (code, body) = http_get(port, "/api/status");
    assert_eq!(code, 200);
    serde_json::from_str(&body).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn api_status_gains_additive_intelligence_object() {
    let state: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.13")));
    let port = serve_state(Arc::clone(&state)).await;

    // Fresh daemon, nothing mirrored yet: honest absent.
    let v = status_json(port);
    assert_eq!(
        v["intelligence"],
        serde_json::json!({"agent": "absent", "model_server": "absent", "agent_url": null})
    );
    // Additive only: every pre-existing top-level key is still present.
    for key in [
        "schema",
        "node",
        "kb",
        "transport_state",
        "health",
        "updates",
        "clock_fault",
        "offer",
        "error_state",
    ] {
        assert!(v.get(key).is_some(), "pinned key {key} missing: {v}");
    }

    // A live fake agent, mirrored exactly as the daemon's 2s task does.
    let dir = tempfile::tempdir().unwrap();
    install_agent(dir.path(), "#!/bin/sh\nexec sleep 300\n");
    let h = start(&layout_at(dir.path()), Some("Node2A"), Some(NODE_ID_HEX));
    wait_for("agent running", Duration::from_secs(5), || {
        h.state().agent == ChildDisplay::Running
    })
    .await;
    state.write().unwrap().intelligence = h.state();
    let v = status_json(port);
    assert_eq!(
        v["intelligence"],
        serde_json::json!({
            "agent": "running",
            "model_server": "absent",
            "agent_url": "http://127.0.0.1:11182"
        })
    );
    // No identity material rides along.
    let raw = v["intelligence"].to_string();
    for banned in ["secret", "mnemonic", "keyset", NODE_ID_HEX] {
        assert!(!raw.contains(banned), "leaks {banned}: {raw}");
    }
    h.stop();
}
