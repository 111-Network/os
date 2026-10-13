//! Layer 4 R1 acceptance (os/interface/README.md §1, §11; api/README.md):
//! pairing auth (401 → pair → 200, wrong code, rate limit, rotation, logout),
//! loopback bypass, Host guard matrix, CSRF (content-type + Origin/Host),
//! static UI serving (fixture dist + embedded/notice), the `/v1` legacy page,
//! and `/api/v1/events` (reset, Last-Event-ID replay, live, keep-alive).
//!
//! Peer simulation: a server is either served with real `ConnectInfo`
//! (loopback socket ⇒ loopback peer) or with axum's `MockConnectInfo(addr)`
//! layer, which makes every request look like it came from `addr` (how a LAN
//! peer is simulated over a loopback socket — no production test hook).

use axum::extract::connect_info::MockConnectInfo;
use node111::daemon::dashboard::api_v1::auth;
use node111::daemon::dashboard::{
    routes, serve_listener, ApiConfig, ApiState, DashboardState, SharedState,
};
use node111::daemon::DaemonLayout;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
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
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", self.body))
    }
    /// Assert an RFC 9457 problem with this status + code.
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

/// One HTTP/1.1 request. `host: None` omits the Host header (sent as HTTP/1.0).
fn request(
    port: u16,
    method: &str,
    path: &str,
    host: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Resp {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut req = match host {
        Some(h) => format!("{method} {path} HTTP/1.1\r\nHost: {h}\r\nConnection: close\r\n"),
        None => format!("{method} {path} HTTP/1.0\r\n"),
    };
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
    request(port, "GET", path, Some("127.0.0.1"), &[], None)
}

fn get_with(port: u16, path: &str, headers: &[(&str, &str)]) -> Resp {
    request(port, "GET", path, Some("127.0.0.1"), headers, None)
}

fn post_json(port: u16, path: &str, body: &str, headers: &[(&str, &str)]) -> Resp {
    let mut h = vec![("Content-Type", "application/json")];
    h.extend_from_slice(headers);
    request(port, "POST", path, Some("127.0.0.1"), &h, Some(body))
}

/// Read an SSE response until `done(text)` or the deadline; returns the text.
fn sse_read(
    port: u16,
    headers: &[(&str, &str)],
    deadline: Duration,
    done: impl Fn(&str) -> bool,
) -> String {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let mut req = "GET /api/v1/events HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/event-stream\r\n"
        .to_string();
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    conn.write_all(req.as_bytes()).unwrap();
    let start = Instant::now();
    let mut acc = Vec::new();
    let mut buf = [0u8; 4096];
    while start.elapsed() < deadline {
        match conn.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => acc.extend_from_slice(&buf[..n]),
            Err(_) => {} // timeout tick
        }
        if done(&String::from_utf8_lossy(&acc)) {
            break;
        }
    }
    String::from_utf8_lossy(&acc).into_owned()
}

/// Parsed SSE frames `(id, event, data)` from a raw (chunked) response.
fn frames(raw: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for block in raw.split("\n\n") {
        let (mut id, mut ev, mut data) = (String::new(), String::new(), String::new());
        for line in block.lines() {
            if let Some(v) = line.strip_prefix("id: ") {
                id = v.to_string();
            } else if let Some(v) = line.strip_prefix("event: ") {
                ev = v.to_string();
            } else if let Some(v) = line.strip_prefix("data: ") {
                data = v.to_string();
            }
        }
        if !ev.is_empty() {
            out.push((id, ev, data));
        }
    }
    out
}

struct Node {
    api: Arc<ApiState>,
    _dir: tempfile::TempDir,
}

fn node_with(lan: bool, tweak: impl FnOnce(&mut ApiConfig)) -> Node {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let mut cfg = ApiConfig::new(layout, lan);
    cfg.keepalive = Duration::from_millis(150);
    tweak(&mut cfg);
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.13")));
    Node {
        api: ApiState::new(dash, cfg),
        _dir: dir,
    }
}

fn node() -> Node {
    node_with(false, |_| {})
}

/// Serve `api`; `peer: None` = real ConnectInfo (loopback socket), else every
/// request appears to come from `peer`.
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

fn pair_code(api: &ApiState) -> String {
    auth::read_pair_code(&api.layout.state_dir())
        .unwrap()
        .expect("pair file created at start")
}

fn cookie_of(r: &Resp) -> String {
    let sc = r.header("set-cookie").expect("Set-Cookie");
    let v = sc.split(';').next().unwrap();
    v.strip_prefix("s111=").expect("s111 cookie").to_string()
}

// ---------------------------------------------------------------- auth

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lan_peer_pairs_401_then_200_then_logout() {
    let n = node();
    let lan = serve(&n.api, Some("192.168.1.50:40000")).await;

    // pair file exists, 0600, valid code
    let pf = n.api.layout.state_dir().join(auth::PAIR_FILE);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(pf.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    let code = pair_code(&n.api);
    assert!(auth::normalize_code(&code).is_some(), "{code}");

    // unpaired LAN peer: every gated /api/v1 route is 401 problem+json
    for path in [
        "/api/v1/events",
        "/api/v1/status",
        "/api/v1/services",
        "/api/v1/settings",
    ] {
        get(lan, path).problem(401, "unauthenticated");
    }
    // ... and so are the Layer 1 seam and the minimal page (node facts)
    get(lan, "/api/status").problem(401, "unauthenticated");
    get(lan, "/v1").problem(401, "unauthenticated");
    // the UI (to reach /pair) and the bare health probe stay open
    assert_eq!(get(lan, "/").status, 200);
    assert!(matches!(get(lan, "/health").status, 200 | 503));
    let s = get(lan, "/api/v1/auth/session");
    assert_eq!(s.status, 200);
    let v = s.json();
    assert_eq!(v["loopback"], false);
    assert_eq!(v["paired"], false);
    assert_eq!(v["lan_enabled"], false);
    assert!(v["created"].is_null());

    // wrong code → 401
    post_json(lan, "/api/v1/auth/pair", r#"{"code":"0000-0000"}"#, &[])
        .problem(401, "unauthenticated");
    // malformed body → 400
    post_json(lan, "/api/v1/auth/pair", r#"{"nope":1}"#, &[]).problem(400, "bad_request");

    // right code, typed sloppily (lowercase, no dash, spaces)
    let sloppy = format!(" {} ", code.replace('-', "").to_lowercase());
    let r = post_json(
        lan,
        "/api/v1/auth/pair",
        &format!(r#"{{"code":"{sloppy}"}}"#),
        &[],
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let sc = r.header("set-cookie").unwrap().to_string();
    for attr in ["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=15552000"] {
        assert!(sc.contains(attr), "{sc}");
    }
    let token = cookie_of(&r);
    assert_eq!(token.len(), 64);
    assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
    let v = r.json();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["paired"], true);
    assert_eq!(v["loopback"], false);
    assert!(v["created"].is_string());

    // sessions file: hash only, never the token
    let raw = std::fs::read_to_string(n.api.layout.state_dir().join(auth::SESSIONS_FILE)).unwrap();
    assert!(!raw.contains(&token), "token persisted in clear");
    let sf: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(sf["sessions"][0]["peer"], "192.168.1.50");
    assert_eq!(sf["sessions"][0]["hash"].as_str().unwrap().len(), 64);

    // paired: gated routes pass auth (R2's status answers 200, events streams 200)
    let cookie = format!("s111={token}");
    let r = get_with(lan, "/api/v1/status", &[("Cookie", &cookie)]);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["auth"]["paired"], true);
    assert_eq!(r.json()["auth"]["loopback"], false);
    let sse = sse_read(lan, &[("Cookie", &cookie)], Duration::from_secs(3), |t| {
        t.contains("event: reset")
    });
    assert!(sse.starts_with("HTTP/1.1 200"), "{sse}");
    assert!(
        sse.to_ascii_lowercase()
            .contains("content-type: text/event-stream"),
        "{sse}"
    );
    let v = get_with(lan, "/api/v1/auth/session", &[("Cookie", &cookie)]).json();
    assert_eq!(v["paired"], true);
    assert!(v["created"].is_string());

    // the session opens the Layer 1 seam and the minimal page too
    assert_eq!(
        get_with(lan, "/api/status", &[("Cookie", &cookie)]).status,
        200
    );
    assert_eq!(get_with(lan, "/v1", &[("Cookie", &cookie)]).status, 200);

    // a forged/unknown token is still 401 ...
    let bogus = format!("s111={}", "a".repeat(64));
    get_with(lan, "/api/v1/status", &[("Cookie", &bogus)]).problem(401, "unauthenticated");
    // ... but cannot shadow a valid one sent alongside it
    let both = format!("{bogus}; s111={token}");
    assert_eq!(
        get_with(lan, "/api/v1/status", &[("Cookie", &both)]).status,
        200
    );

    // logout revokes (204 + cookie cleared) → 401 again
    let r = post_json(lan, "/api/v1/auth/logout", "{}", &[("Cookie", &cookie)]);
    assert_eq!(r.status, 204);
    assert!(r.header("set-cookie").unwrap().contains("Max-Age=0"));
    get_with(lan, "/api/v1/status", &[("Cookie", &cookie)]).problem(401, "unauthenticated");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rate_limit_per_peer_then_rotation_after_twenty() {
    let n = node();
    let a = serve(&n.api, Some("192.168.1.10:1000")).await;
    let b = serve(&n.api, Some("192.168.1.11:1000")).await;
    let code = pair_code(&n.api);

    for _ in 0..auth::FAILS_PER_PEER {
        post_json(a, "/api/v1/auth/pair", r#"{"code":"0000-0000"}"#, &[])
            .problem(401, "unauthenticated");
    }
    // 6th attempt from the same peer: 429 even with the right code
    let r = post_json(
        a,
        "/api/v1/auth/pair",
        &format!(r#"{{"code":"{code}"}}"#),
        &[],
    );
    r.problem(429, "rate_limited");
    let retry: u64 = r.header("retry-after").unwrap().parse().unwrap();
    assert!((1..=60).contains(&retry));
    // another peer is not limited by A's failures
    assert_eq!(
        post_json(
            b,
            "/api/v1/auth/pair",
            &format!(r#"{{"code":"{code}"}}"#),
            &[]
        )
        .status,
        200
    );

    // 20 failures in total (4 peers × 5, each under its own limit) rotate the code
    let n = node();
    let mut ports = Vec::new();
    for i in 0..4 {
        ports.push(serve(&n.api, Some(&format!("10.0.0.{}:9", 20 + i))).await);
    }
    let old = pair_code(&n.api);
    for p in &ports {
        for _ in 0..auth::FAILS_PER_PEER {
            post_json(*p, "/api/v1/auth/pair", r#"{"code":"0000-0000"}"#, &[])
                .problem(401, "unauthenticated");
        }
    }
    let new = pair_code(&n.api);
    assert_ne!(
        old,
        new,
        "code must rotate after {} failures",
        auth::FAILS_BEFORE_ROTATE
    );
    let fresh = serve(&n.api, Some("10.0.0.99:9")).await;
    post_json(
        fresh,
        "/api/v1/auth/pair",
        &format!(r#"{{"code":"{old}"}}"#),
        &[],
    )
    .problem(401, "unauthenticated");
    assert_eq!(
        post_json(
            fresh,
            "/api/v1/auth/pair",
            &format!(r#"{{"code":"{new}"}}"#),
            &[]
        )
        .status,
        200
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cli_rotation_revokes_running_sessions() {
    let n = node();
    let lan = serve(&n.api, Some("192.168.1.60:1")).await;
    let code = pair_code(&n.api);
    let r = post_json(
        lan,
        "/api/v1/auth/pair",
        &format!(r#"{{"code":"{code}"}}"#),
        &[],
    );
    let cookie = format!("s111={}", cookie_of(&r));
    assert_eq!(
        get_with(lan, "/api/v1/auth/session", &[("Cookie", &cookie)]).json()["paired"],
        true
    );
    // an open event stream of that session ...
    let api = Arc::clone(&n.api);
    let state_dir = n.api.layout.state_dir();
    let rotator = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        // `111 dashboard rotate` writes the files; the running daemon sees it
        let new = auth::rotate_files(&state_dir).unwrap();
        api.events
            .publish("status", &serde_json::json!({"after": "rotate"}));
        new
    });
    let c2 = cookie.clone();
    let raw = tokio::task::spawn_blocking(move || {
        sse_read(lan, &[("Cookie", &c2)], Duration::from_secs(3), |_| false)
    })
    .await
    .unwrap();
    let new = rotator.await.unwrap();
    // ... got its reset frame, then was cut before the post-rotation frame
    assert!(raw.contains("event: reset"), "{raw}");
    assert!(
        !raw.contains("after"),
        "revoked stream still delivering: {raw}"
    );
    assert_ne!(new, code);
    get_with(lan, "/api/v1/status", &[("Cookie", &cookie)]).problem(401, "unauthenticated");
    post_json(
        lan,
        "/api/v1/auth/pair",
        &format!(r#"{{"code":"{code}"}}"#),
        &[],
    )
    .problem(401, "unauthenticated");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn loopback_peers_bypass_auth() {
    let n = node();
    // real ConnectInfo over the loopback socket
    let real = serve(&n.api, None).await;
    let sse = sse_read(real, &[], Duration::from_secs(3), |t| {
        t.contains("event: reset")
    });
    assert!(sse.starts_with("HTTP/1.1 200"), "{sse}");
    assert_eq!(get(real, "/api/v1/status").json()["auth"]["loopback"], true);
    let v = get(real, "/api/v1/auth/session").json();
    assert_eq!(v["loopback"], true);
    assert_eq!(v["paired"], false);
    // IPv6 loopback and v4-mapped loopback count too
    for peer in ["[::1]:5000", "[::ffff:127.0.0.1]:5000", "127.0.0.2:1"] {
        let p = serve(&n.api, Some(peer)).await;
        assert_eq!(get(p, "/api/v1/status").json()["auth"]["loopback"], true);
    }
    // a LAN peer does not
    let lan = serve(&n.api, Some("[fe80::1]:5000")).await;
    get(lan, "/api/v1/status").problem(401, "unauthenticated");
}

// ---------------------------------------------------------------- host guard

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn host_guard_matrix() {
    let off = node();
    let port = serve(&off.api, None).await;
    let allowed = [
        "127.0.0.1",
        "127.0.0.1:11180",
        "localhost",
        "localhost:80",
        "LOCALHOST",
        "[::1]",
        "[::1]:11180",
        "111.localhost",
        "111.localhost:11180",
        "111.local",
        "111.local.",
        "111-2.local",
        "111-12.local:80",
    ];
    for h in allowed {
        let r = request(port, "GET", "/health", Some(h), &[], None);
        assert!(matches!(r.status, 200 | 503), "{h} refused: {}", r.status);
    }
    let refused = [
        "evil.com",
        "127.0.0.1.evil.com",
        "localhost.evil.com",
        "111.local.evil.com",
        "evil.111.localhost",
        "111-.local",
        "111-x.local",
        "[::2]",
        "::1",
        "[::1",
        "127.0.0.1:",
        "127.0.0.1:99999",
        "127.0.0.1:80x",
        "192.168.1.5", // own address, but LAN is off
    ];
    for h in refused {
        let r = request(port, "GET", "/health", Some(h), &[], None);
        r.problem(400, "bad_request");
        assert_eq!(r.json()["title"], "refused Host header");
    }
    // no Host at all (HTTP/1.0 probe) passes
    let r = request(port, "GET", "/health", None, &[], None);
    assert!(matches!(r.status, 200 | 503), "{}", r.status);

    // LAN on: the node's own addresses are allowed, other IPs still refused
    let on = node_with(true, |_| {});
    on.api.set_own_addrs(vec![
        "192.168.1.5".parse::<IpAddr>().unwrap(),
        "fe80::1".parse().unwrap(),
    ]);
    let port = serve(&on.api, None).await;
    for h in ["192.168.1.5", "192.168.1.5:11180", "[fe80::1]", "111.local"] {
        let r = request(port, "GET", "/health", Some(h), &[], None);
        assert!(matches!(r.status, 200 | 503), "{h} refused with LAN on");
    }
    for h in ["192.168.1.6", "10.0.0.1:80", "[fe80::2]", "evil.com"] {
        request(port, "GET", "/health", Some(h), &[], None).problem(400, "bad_request");
    }
    // the guard covers /api/v1 and static too
    request(
        port,
        "GET",
        "/api/v1/auth/session",
        Some("evil.com"),
        &[],
        None,
    )
    .problem(400, "bad_request");
    request(port, "GET", "/", Some("evil.com"), &[], None).problem(400, "bad_request");
}

// ---------------------------------------------------------------- CSRF

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn csrf_content_type_and_origin() {
    let n = node();
    let port = serve(&n.api, None).await; // loopback: CSRF still applies

    // no / wrong content type → 415 problem bad_request (before auth or handler)
    request(
        port,
        "POST",
        "/api/v1/auth/logout",
        Some("127.0.0.1"),
        &[],
        None,
    )
    .problem(415, "bad_request");
    request(
        port,
        "POST",
        "/api/v1/node/restart",
        Some("127.0.0.1"),
        &[("Content-Type", "text/plain")],
        Some("{}"),
    )
    .problem(415, "bad_request");
    request(
        port,
        "POST",
        "/api/v1/auth/pair",
        Some("127.0.0.1"),
        &[("Content-Type", "application/x-www-form-urlencoded")],
        Some("code=x"),
    )
    .problem(415, "bad_request");
    request(
        port,
        "PATCH",
        "/api/v1/settings",
        Some("127.0.0.1"),
        &[],
        Some("{}"),
    )
    .problem(415, "bad_request");

    // Origin present and ≠ Host → 403 bad_request
    let host = format!("127.0.0.1:{port}");
    for origin in [
        "http://evil.com",
        "null",
        "https://127.0.0.1",
        "http://127.0.0.1",
    ] {
        request(
            port,
            "POST",
            "/api/v1/auth/logout",
            Some(&host),
            &[("Content-Type", "application/json"), ("Origin", origin)],
            Some("{}"),
        )
        .problem(403, "bad_request");
    }
    // same-origin (and charset param) passes
    let origin = format!("http://127.0.0.1:{port}");
    let r = request(
        port,
        "POST",
        "/api/v1/auth/logout",
        Some(&host),
        &[
            ("Content-Type", "application/json; charset=utf-8"),
            ("Origin", &origin),
        ],
        Some("{}"),
    );
    assert_eq!(r.status, 204, "{}", r.body);
    // no Origin at all (non-browser client) passes
    assert_eq!(
        post_json(port, "/api/v1/auth/logout", "{}", &[]).status,
        204
    );
    // GETs are never CSRF-checked
    let r = request(
        port,
        "GET",
        "/api/v1/auth/session",
        Some(&host),
        &[("Origin", "http://evil.com")],
        None,
    );
    assert_eq!(r.status, 200);
}

// ---------------------------------------------------------------- static UI

fn fixture_dist() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let w = |rel: &str, body: &[u8]| {
        let p = d.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    };
    w(
        "index.html",
        b"<!doctype html><title>111 node</title><div id=root></div>",
    );
    w("assets/index-Ab12Cd.js", b"console.log(1)");
    w("assets/index-Ab12Cd.css", b"body{}");
    w("fonts/GeistMono-Variable.woff2", b"wOF2");
    w("fonts/LICENSE-geist.txt", b"license");
    w("favicon.svg", b"<svg/>");
    w(".secret", b"no");
    d
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn static_ui_from_fixture_dist() {
    let dist = fixture_dist();
    let ui = dist.path().to_path_buf();
    let n = node_with(false, move |c| c.ui_dir = Some(ui));
    let port = serve(&n.api, Some("192.168.1.77:1")).await; // unpaired LAN peer: static is open

    let r = get(port, "/");
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("text/html; charset=utf-8"));
    assert_eq!(r.header("cache-control"), Some("no-cache"));
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.header("x-frame-options"), Some("DENY"));
    assert!(r.body.contains("111 node"));
    let etag = r.header("etag").unwrap().to_string();
    // revalidation
    let r = get_with(port, "/", &[("If-None-Match", &etag)]);
    assert_eq!(r.status, 304);
    assert!(r.body.is_empty());

    let cases = [
        (
            "/assets/index-Ab12Cd.js",
            "text/javascript; charset=utf-8",
            "public, max-age=31536000, immutable",
        ),
        (
            "/assets/index-Ab12Cd.css",
            "text/css; charset=utf-8",
            "public, max-age=31536000, immutable",
        ),
        ("/fonts/GeistMono-Variable.woff2", "font/woff2", "no-cache"),
        (
            "/fonts/LICENSE-geist.txt",
            "text/plain; charset=utf-8",
            "no-cache",
        ),
        ("/favicon.svg", "image/svg+xml", "no-cache"),
        ("/index.html", "text/html; charset=utf-8", "no-cache"),
    ];
    for (path, ctype, cache) in cases {
        let r = get(port, path);
        assert_eq!(r.status, 200, "{path}");
        assert_eq!(r.header("content-type"), Some(ctype), "{path}");
        assert_eq!(r.header("cache-control"), Some(cache), "{path}");
    }
    let head = request(
        port,
        "HEAD",
        "/assets/index-Ab12Cd.js",
        Some("127.0.0.1"),
        &[],
        None,
    );
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());

    for path in [
        "/missing.js",
        "/.secret",
        "/assets/../.secret",
        "/assets/%2e%2e/.secret",
        "/assets/",
    ] {
        assert_eq!(get(port, path).status, 404, "{path}");
    }
    // unknown API paths are problem+json, not the SPA
    get(port, "/api/v1/nope").problem(404, "not_found");
    // static is GET-only
    assert_eq!(
        request(port, "DELETE", "/", Some("127.0.0.1"), &[], None).status,
        405
    );
    assert_eq!(
        request(
            port,
            "POST",
            "/assets/index-Ab12Cd.js",
            Some("127.0.0.1"),
            &[],
            None
        )
        .status,
        405
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn embedded_ui_or_notice_and_legacy_page() {
    let n = node();
    let port = serve(&n.api, None).await;
    let r = get(port, "/");
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("text/html; charset=utf-8"));
    assert_eq!(r.header("cache-control"), Some("no-cache"));
    if routes::ui_embedded() {
        // the UI build's own markup is the UI tasks' business; only "not the notice"
        assert!(!r.body.contains("dashboard UI not built"));
    } else {
        assert!(r.body.contains("111 node"), "{}", r.body);
        assert!(r.body.contains("dashboard UI not built"), "{}", r.body);
    }
    // the Layer 1 minimal page lives at /v1 now
    let r = get(port, "/v1");
    assert_eq!(r.status, 200);
    assert!(r.body.contains("111 node"));
    for banned in ["mnemonic", "keyset", "BIP-39"] {
        assert!(!r.body.contains(banned), "/v1 leaks {banned}");
    }
    // Layer 1 assets that are not the page are never served
    for p in [
        "/lora-table.toml",
        "/assets/lora-table.toml",
        "/rns/config.toml.tmpl",
    ] {
        assert_eq!(get(port, p).status, 404, "{p}");
    }
}

// ---------------------------------------------------------------- events

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn events_reset_replay_live_and_keepalive() {
    let n = node();
    let port = serve(&n.api, None).await;
    let head0 = n.api.events.head();

    // fresh client: first frame is reset{expired} carrying the head id
    let raw = sse_read(port, &[], Duration::from_secs(3), |t| {
        t.contains("event: reset")
    });
    let f = frames(&raw);
    assert_eq!(f[0].1, "reset", "{raw}");
    assert_eq!(f[0].2, r#"{"reason":"expired"}"#);
    assert_eq!(f[0].0, head0.to_string());

    // keep-alive comment while idle (interval shortened to 150 ms here)
    let raw = sse_read(port, &[], Duration::from_secs(3), |t| {
        t.contains(": keep-alive")
    });
    assert!(
        raw.contains("\n: keep-alive\n") || raw.contains(": keep-alive\n"),
        "{raw}"
    );

    // replay from the ring
    let id1 = n.api.events.publish("status", &serde_json::json!({"n": 1}));
    let id2 = n
        .api
        .events
        .publish("services", &serde_json::json!({"services": []}));
    let id3 = n
        .api
        .events
        .publish("proposal", &serde_json::json!({"id": "prop-1"}));
    assert!(id1 > head0 && id2 == id1 + 1 && id3 == id2 + 1);
    let last = id1.to_string();
    let raw = sse_read(
        port,
        &[("Last-Event-ID", &last)],
        Duration::from_secs(3),
        |t| t.contains(&format!("id: {id3}")),
    );
    let f = frames(&raw);
    assert_eq!(
        f.iter()
            .map(|x| (x.0.clone(), x.1.clone()))
            .collect::<Vec<_>>(),
        vec![
            (id2.to_string(), "services".to_string()),
            (id3.to_string(), "proposal".to_string())
        ],
        "{raw}"
    );
    assert_eq!(f[1].2, r#"{"id":"prop-1"}"#);

    // an id from a previous daemon lifetime → reset{restart}
    let raw = sse_read(
        port,
        &[("Last-Event-ID", "5")],
        Duration::from_secs(3),
        |t| t.contains("event: reset"),
    );
    assert_eq!(frames(&raw)[0].2, r#"{"reason":"restart"}"#);

    // live: resume at head, then publish
    let api = Arc::clone(&n.api);
    let head = id3.to_string();
    let publisher = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        api.events.publish("agent", &serde_json::json!({"seq": 7}))
    });
    let raw = tokio::task::spawn_blocking(move || {
        sse_read(
            port,
            &[("Last-Event-ID", &head)],
            Duration::from_secs(5),
            |t| t.contains("event: agent"),
        )
    })
    .await
    .unwrap();
    let id4 = publisher.await.unwrap();
    let f = frames(&raw);
    assert_eq!(
        f.len(),
        1,
        "no reset/replay expected when resuming at head: {raw}"
    );
    assert_eq!(
        f[0],
        (
            id4.to_string(),
            "agent".to_string(),
            r#"{"seq":7}"#.to_string()
        )
    );
}

// ---------------------------------------------------------------- headers + problems

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn security_headers_on_every_response() {
    let dist = fixture_dist();
    let ui = dist.path().to_path_buf();
    let n = node_with(false, move |c| c.ui_dir = Some(ui));
    let port = serve(&n.api, None).await;
    for path in [
        "/",
        "/assets/index-Ab12Cd.js",
        "/missing.js",
        "/health",
        "/api/status",
        "/api/v1/status",
        "/api/v1/nope",
    ] {
        let r = get(port, path);
        assert_eq!(
            r.header("content-security-policy"),
            Some(routes::CSP),
            "{path}"
        );
        assert_eq!(
            r.header("x-content-type-options"),
            Some("nosniff"),
            "{path}"
        );
        assert_eq!(r.header("referrer-policy"), Some("no-referrer"), "{path}");
    }
    let csp = routes::CSP;
    for directive in [
        "default-src 'self'",
        "script-src 'self';",
        "connect-src 'self'",
        "frame-ancestors 'none'",
        "base-uri 'self'",
        "form-action 'self'",
    ] {
        assert!(csp.contains(directive), "{directive}");
    }
    // refused Host answers carry them too
    let r = request(port, "GET", "/", Some("evil.com"), &[], None);
    assert_eq!(r.header("content-security-policy"), Some(routes::CSP));

    // /v1: same policy, plus the hash of its one inline script (and only that)
    let r = get(port, "/v1");
    assert_eq!(r.status, 200);
    let v1 = r.header("content-security-policy").unwrap();
    let body = &r.body;
    let open = body.find("<script>").unwrap() + "<script>".len();
    let close = open + body[open..].find("</script>").unwrap();
    use sha2::Digest;
    let want = routes::base64(&sha2::Sha256::digest(&body.as_bytes()[open..close]));
    assert!(
        v1.contains(&format!("script-src 'self' 'sha256-{want}';")),
        "{v1}"
    );
    assert_eq!(v1.replace(&format!(" 'sha256-{want}'"), ""), routes::CSP);
}

#[test]
fn base64_vectors() {
    for (raw, enc) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(routes::base64(raw.as_bytes()), enc, "{raw}");
    }
    // a script with no inline <script> keeps the strict policy verbatim
    assert_eq!(routes::csp_with_inline_scripts("<p>hi</p>"), routes::CSP);
}

/// POST with a body far above axum's 2 MiB default limit; the writer runs on
/// its own thread (the server answers 413 before reading it all).
fn post_oversized(port: u16, path: &str) -> Resp {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let len = 3 << 20;
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {len}\r\n\r\n"
    );
    let mut w = conn.try_clone().unwrap();
    let writer = std::thread::spawn(move || {
        let _ = w.write_all(head.as_bytes());
        let chunk = vec![b' '; 64 * 1024];
        for _ in 0..(len / chunk.len()) {
            if w.write_all(&chunk).is_err() {
                break;
            }
        }
    });
    let mut raw = Vec::new();
    let _ = conn.read_to_end(&mut raw);
    let _ = writer.join();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn framework_errors_are_problem_json() {
    let n = node();
    let port = serve(&n.api, None).await;
    // unknown API route, any method: 404 (not the static catch-all's 405)
    post_json(port, "/api/v1/proposals/not-a-ulid/decide", "{}", &[]).problem(404, "not_found");
    request(port, "DELETE", "/api/v1/nope", Some("127.0.0.1"), &[], None).problem(404, "not_found");
    // wrong method on a real route: 405 problem, Allow kept
    let r = get(port, "/api/v1/node/restart");
    r.problem(405, "bad_request");
    assert!(
        r.header("allow").is_some_and(|a| a.contains("POST")),
        "{:?}",
        r.headers
    );
    // Path<String> rejection (not UTF-8): 400 problem with the reason as detail
    let r = get(port, "/api/v1/services/%FF");
    r.problem(400, "bad_request");
    assert!(r.json()["detail"].as_str().is_some_and(|d| !d.is_empty()));
    // oversized body (axum's 2 MiB default limit): 413 problem
    let r = tokio::task::spawn_blocking(move || post_oversized(port, "/api/v1/auth/pair"))
        .await
        .unwrap();
    r.problem(413, "bad_request");
    // the Layer 1 seam is GET-only: 405 there is problem+json as well
    request(port, "POST", "/api/status", Some("127.0.0.1"), &[], None).problem(405, "bad_request");
    // non-API paths keep their plain answers
    let r = get(port, "/missing.js");
    assert_eq!(r.status, 404);
    assert_ne!(r.header("content-type"), Some("application/problem+json"));
}
