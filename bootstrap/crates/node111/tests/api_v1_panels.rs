//! Layer 5 P1 acceptance (os/messaging/README.md §13 "Hosted panels", §15
//! `listPanels` / `openPanelSession`): the ticket → cookie flow (spent ticket
//! `403` + re-mint, a ticket presented with a valid cookie is spent), guards on
//! every request (cross-site refused, no CORS headers ever, the Host/Origin
//! matrix, `OPTIONS` refused), the main listener's `localhost` → `127.0.0.1`
//! `308`, the response policy (frame-ancestors / XFO / nosniff / referrer),
//! SPA vs API paths (shim dispatch, `502 upstream_error`, never `index.html`
//! under `/api`), the per-request dashboard CSP `frame-src`, loopback-only
//! binding, untrusted loopback (Termux) still needing the ticket, and
//! revocation on `111 dashboard rotate`, panel disable and LAN-off.
//!
//! Harness: a temp node root with the shipped `meshtastic.toml` (optionally
//! edited), a temp bundle under `<root>/current/panels/meshtastic-web`, a stub
//! shim registered as `meshtastic-http`, the dashboard app on an ephemeral
//! loopback port and the panel's own listener on a free port (its slot block
//! narrowed to that one port). LAN peers are simulated on the dashboard with
//! axum's `MockConnectInfo`; LAN sharing is simulated with `force_loopback`
//! so no test opens a LAN socket.

use axum::extract::connect_info::MockConnectInfo;
use futures_util::future::BoxFuture;
use node111::daemon::dashboard::api_v1::auth;
use node111::daemon::dashboard::{
    routes, serve_listener, ApiConfig, ApiState, DashboardState, SharedState,
};
use node111::daemon::panels::{
    self, manifest, ticket, PanelHost, PanelsConfig, Shim, ShimError, ShimRegistry, ShimRequest,
    ShimResponse, ShimSession,
};
use node111::daemon::services::ServiceId;
use node111::daemon::DaemonLayout;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

// ---------------------------------------------------------------- http client

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

    fn all(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", self.body))
    }

    /// An RFC 9457 problem with this status + code.
    fn problem(&self, status: u16, code: &str) {
        assert_eq!(self.status, status, "body: {}", self.body);
        assert_eq!(
            self.header("content-type"),
            Some("application/problem+json"),
            "{}",
            self.body
        );
        assert_eq!(self.json()["code"], code, "{}", self.body);
    }

    /// No `Access-Control-*` header, ever (§13).
    fn no_cors(&self) {
        for (k, _) in &self.headers {
            assert!(
                !k.to_ascii_lowercase().starts_with("access-control-"),
                "{k} leaked"
            );
        }
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

/// One HTTP/1.1 request to `127.0.0.1:port` with `Host: host`.
fn send(
    port: u16,
    method: &str,
    path: &str,
    host: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Resp {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
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

/// Wait (bounded) until nothing accepts on `127.0.0.1:port` (an aborted
/// listener task drops its socket on its next poll).
async fn wait_closed(port: u16) {
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("listener on {port} still accepting");
}

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

// ---------------------------------------------------------------- stub shim

/// `meshtastic-http` stand-in: `GET /api/v1/fromradio` answers the session
/// key it was handed, `PUT /api/v1/toradio` echoes the body, `/api/v1/down`
/// is an unavailable upstream; anything else is not a shim path.
#[derive(Default)]
struct Stub {
    seen: Mutex<Vec<(ShimSession, String)>>,
}

impl Shim for Stub {
    fn name(&self) -> &'static str {
        "meshtastic-http"
    }

    fn handle<'a>(
        &'a self,
        session: &'a ShimSession,
        req: ShimRequest,
    ) -> BoxFuture<'a, Result<ShimResponse, ShimError>> {
        Box::pin(async move {
            self.seen
                .lock()
                .unwrap()
                .push((session.clone(), req.path.clone()));
            match (req.method.as_str(), req.path.as_str()) {
                ("PUT", "/api/v1/toradio") => {
                    Ok(ShimResponse::ok("application/x-protobuf", req.body))
                }
                ("GET", "/api/v1/fromradio") => {
                    Ok(ShimResponse::ok("text/plain", session.session.clone()))
                }
                (_, "/api/v1/down") => Err(ShimError::Upstream("downlink refused".into())),
                _ => Err(ShimError::NotFound),
            }
        })
    }
}

// ---------------------------------------------------------------- fixture

struct Opts {
    lan: bool,
    own: Vec<IpAddr>,
    trust_loopback: bool,
    ttl: Duration,
    force_loopback: bool,
    edit: fn(String) -> String,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            lan: false,
            own: Vec::new(),
            trust_loopback: true,
            ttl: ticket::TICKET_TTL,
            force_loopback: true,
            edit: |t| t,
        }
    }
}

struct Fx {
    api: Arc<ApiState>,
    host: Arc<PanelHost>,
    stub: Arc<Stub>,
    /// Dashboard port (real ConnectInfo: loopback peer).
    dash: u16,
    /// The panel's slot.
    port: u16,
    layout: DaemonLayout,
    _dir: tempfile::TempDir,
}

const INDEX: &str = "<!doctype html><title>panel</title>";

fn write_bundle(root: &std::path::Path) {
    let b = root.join("current/panels/meshtastic-web");
    for d in ["devices", "fonts", "i18n", "assets"] {
        std::fs::create_dir_all(b.join(d)).unwrap();
    }
    std::fs::write(b.join("index.html"), INDEX).unwrap();
    std::fs::write(b.join("devices/list.json"), "{\"devices\":[]}").unwrap();
    std::fs::write(b.join("i18n/en.json"), "{}").unwrap();
    std::fs::write(b.join("assets/app.js"), "console.log(1)").unwrap();
}

fn shipped_manifest() -> String {
    let (_, bytes) = manifest::embedded()
        .into_iter()
        .find(|(n, _)| n == "meshtastic.toml")
        .expect("meshtastic.toml embedded");
    String::from_utf8(bytes).unwrap()
}

fn panel_config(layout: &DaemonLayout, port: u16, o: &Opts) -> PanelsConfig {
    let mut c = PanelsConfig::new(layout);
    c.ports = port..=port;
    c.ticket_ttl = o.ttl;
    c.proc_net = layout.root.join("no-proc-net");
    c.force_loopback = o.force_loopback;
    c
}

async fn fixture(o: Opts) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let mut cfg = ApiConfig::new(layout.clone(), o.lan);
    cfg.trust_loopback = o.trust_loopback;
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.14")));
    let api = ApiState::new(dash, cfg);
    api.set_own_addrs(o.own.clone());
    write_bundle(dir.path());

    let stub = Arc::new(Stub::default());
    let mut shims = ShimRegistry::new();
    shims.register(Arc::clone(&stub) as Arc<dyn Shim>).unwrap();
    let port = free_port();
    let text = (o.edit)(shipped_manifest());
    let host = Arc::new(PanelHost::new(
        panel_config(&layout, port, &o),
        vec![("meshtastic.toml".into(), text.into_bytes())],
        &shims,
        o.lan,
    ));
    assert_eq!(host.registered().len(), 1, "the declaration registers");
    api.insert_ext(Arc::clone(&host));
    set_enabled(&host, &api, true).await;
    let dash = serve_dash(&api, None).await;
    Fx {
        api,
        host,
        stub,
        dash,
        port,
        layout,
        _dir: dir,
    }
}

/// Reconcile with the meshtastic service set up (`on`) or not.
async fn set_enabled(host: &Arc<PanelHost>, api: &Arc<ApiState>, on: bool) {
    let setup = move |id: ServiceId| on && id == ServiceId::Meshtastic;
    host.reconcile(api, Some(&setup)).await;
}

/// Serve the dashboard; `peer: Some(addr)` makes every request come from it.
async fn serve_dash(api: &Arc<ApiState>, peer: Option<&str>) -> u16 {
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

impl Fx {
    fn ph(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// `POST /api/v1/panels/meshtastic/session` on the dashboard.
    fn mint_on(&self, dash: u16, host: &str, extra: &[(&str, &str)]) -> Resp {
        let mut h = vec![("Content-Type", "application/json")];
        h.extend_from_slice(extra);
        send(
            dash,
            "POST",
            "/api/v1/panels/meshtastic/session",
            host,
            &h,
            Some("{}"),
        )
    }

    /// Mint on the loopback dashboard; returns the ticket URL's path+query.
    fn mint(&self, extra: &[(&str, &str)]) -> String {
        let r = self.mint_on(self.dash, &format!("127.0.0.1:{}", self.dash), extra);
        assert_eq!(r.status, 200, "{}", r.body);
        ticket_path(&r.json()["url"], &self.ph())
    }

    /// A request on the panel origin as the panel's own JS would send it.
    fn panel(&self, method: &str, path: &str, headers: &[(&str, &str)]) -> Resp {
        let r = send(self.port, method, path, &self.ph(), headers, None);
        r.no_cors();
        r
    }

    /// Swap `ticket_path` for the cookie value.
    fn swap(&self, ticket_path: &str) -> String {
        let r = self.panel("GET", ticket_path, &[]);
        assert_eq!(r.status, 303, "{}", r.body);
        assert_eq!(r.header("location"), Some("/"));
        cookie_value(&r)
    }

    /// Mint + swap.
    fn open(&self) -> String {
        let t = self.mint(&[]);
        self.swap(&t)
    }
}

/// The ticket URL must be on `authority`; returns its path+query.
fn ticket_path(url: &serde_json::Value, authority: &str) -> String {
    let url = url.as_str().expect("url");
    let rest = url
        .strip_prefix(&format!("http://{authority}"))
        .unwrap_or_else(|| panic!("{url} is not on {authority}"));
    let ticket = rest
        .strip_prefix("/?p111_ticket=")
        .unwrap_or_else(|| panic!("{url}"));
    assert_eq!(ticket.len(), 32, "128-bit hex ticket");
    assert!(ticket
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    rest.to_string()
}

/// The `p111_meshtastic` cookie set by a swap (and its attributes checked).
fn cookie_value(r: &Resp) -> String {
    let set = r.all("set-cookie");
    assert_eq!(set.len(), 1, "{set:?}");
    let sc = set[0];
    let attrs: Vec<&str> = sc.split(';').map(str::trim).collect();
    for a in ["HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(attrs.contains(&a), "{sc} lacks {a}");
    }
    for a in &attrs[1..] {
        let k = a.split('=').next().unwrap().to_ascii_lowercase();
        assert!(
            !matches!(k.as_str(), "domain" | "max-age" | "expires"),
            "{sc}: host-only session cookie"
        );
    }
    attrs[0]
        .strip_prefix("p111_meshtastic=")
        .expect("p111_meshtastic")
        .to_string()
}

fn cookie_header(v: &str) -> String {
    format!("p111_meshtastic={v}")
}

fn s111_of(r: &Resp) -> String {
    let sc = r.header("set-cookie").expect("Set-Cookie");
    sc.split(';')
        .next()
        .unwrap()
        .strip_prefix("s111=")
        .expect("s111")
        .to_string()
}

fn pair(port: u16, api: &ApiState) -> String {
    let code = auth::read_pair_code(&api.layout.state_dir())
        .unwrap()
        .expect("pair code");
    let r = send(
        port,
        "POST",
        "/api/v1/auth/pair",
        "127.0.0.1",
        &[("Content-Type", "application/json")],
        Some(&format!(r#"{{"code":"{code}"}}"#)),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    s111_of(&r)
}

// ---------------------------------------------------------------- tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ticket_flow_spent_ticket_and_remint() {
    let fx = fixture(Opts::default()).await;
    let ck = |v: &str| cookie_header(v);

    // no loopback skip: the panel origin needs the cookie even on loopback
    fx.panel("GET", "/", &[]).problem(401, "unauthenticated");

    // mint: URL only in the reply, on the host the browser typed
    let r = fx.mint_on(fx.dash, &format!("127.0.0.1:{}", fx.dash), &[]);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.header("cache-control"), Some("no-store"));
    let j = r.json();
    assert_eq!(j["schema"], 1);
    assert_eq!(j["expires_in"], 30);
    let t1 = ticket_path(&j["url"], &fx.ph());
    let r = fx.mint_on(fx.dash, &format!("[::1]:{}", fx.dash), &[]);
    ticket_path(&r.json()["url"], &format!("[::1]:{}", fx.port));
    let r = fx.mint_on(fx.dash, &format!("111.localhost:{}", fx.dash), &[]);
    ticket_path(&r.json()["url"], &format!("111.localhost:{}", fx.port));

    // swap → host-only session cookie + redirect to entry.path
    let c1 = fx.swap(&t1);
    assert_eq!(c1.len(), 64);
    // a spent ticket without a cookie: 403 panel_ticket_expired
    fx.panel("GET", &t1, &[])
        .problem(403, "panel_ticket_expired");
    // the dashboard re-mints
    let t2 = fx.mint(&[]);
    let c2 = fx.swap(&t2);
    assert_ne!(c1, c2);

    // the cookie serves the bundle
    let r = fx.panel("GET", "/", &[("Cookie", &ck(&c1))]);
    assert_eq!((r.status, r.body.as_str()), (200, INDEX));

    // a valid cookie ignores a ticket, redirects to entry.path — and spends it
    let t3 = fx.mint(&[]);
    let r = fx.panel("GET", &t3, &[("Cookie", &ck(&c1))]);
    assert_eq!(r.status, 303);
    assert_eq!(r.header("location"), Some("/"));
    assert!(r.header("set-cookie").is_none(), "no second cookie");
    fx.panel("GET", &t3, &[])
        .problem(403, "panel_ticket_expired");

    // every copy of the cookie is tried (exact name)
    let r = fx.panel(
        "GET",
        "/",
        &[
            ("Cookie", "p111_meshtastic=junk; P111_meshtastic=x"),
            ("Cookie", &ck(&c2)),
        ],
    );
    assert_eq!(r.status, 200);
    fx.panel(
        "GET",
        "/",
        &[("Cookie", &format!("p111_meshtasticx={c2}; s111={c2}"))],
    )
    .problem(401, "unauthenticated");

    // malformed / unknown tickets are refused the same way
    for bad in [
        "/?p111_ticket=",
        "/?p111_ticket=zz",
        "/?p111_ticket=00112233445566778899aabbccddeeff",
    ] {
        fx.panel("GET", bad, &[])
            .problem(403, "panel_ticket_expired");
    }
    // a ticket presented with a non-GET is burned and refused
    let t4 = fx.mint(&[]);
    fx.panel("POST", &t4, &[("Origin", &fx.origin())])
        .problem(403, "panel_ticket_expired");
    fx.panel("GET", &t4, &[])
        .problem(403, "panel_ticket_expired");

    // sessions are recorded 0600 without the cookie values
    let path = fx.layout.state_dir().join(ticket::SESSIONS_FILE);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(&c1) && !text.contains(&c2));
    assert_eq!(fx.host.sessions.snapshot().len(), 2);

    // unknown or undeclared panels: 404
    for id in ["nope", "kb"] {
        send(
            fx.dash,
            "POST",
            &format!("/api/v1/panels/{id}/session"),
            &format!("127.0.0.1:{}", fx.dash),
            &[("Content-Type", "application/json")],
            Some("{}"),
        )
        .problem(404, "not_found");
    }
    // the mint is a mutating API call: CSRF rules apply
    let r = send(
        fx.dash,
        "POST",
        "/api/v1/panels/meshtastic/session",
        &format!("127.0.0.1:{}", fx.dash),
        &[("Content-Type", "text/plain")],
        Some("{}"),
    );
    assert_eq!(r.status, 415);
    fx.mint_on(
        fx.dash,
        &format!("127.0.0.1:{}", fx.dash),
        &[("Origin", &fx.origin())],
    )
    .problem(403, "bad_request");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_ticket_and_peer_binding() {
    let fx = fixture(Opts {
        ttl: Duration::from_millis(300),
        ..Opts::default()
    })
    .await;
    let r = fx.mint_on(fx.dash, &format!("127.0.0.1:{}", fx.dash), &[]);
    assert_eq!(r.json()["expires_in"], 1, "never 0");
    let t = ticket_path(&r.json()["url"], &fx.ph());
    tokio::time::sleep(Duration::from_millis(450)).await;
    fx.panel("GET", &t, &[])
        .problem(403, "panel_ticket_expired");

    // a ticket minted by a LAN peer (paired) is refused from another peer
    let lan_dash = serve_dash(&fx.api, Some("192.168.1.50:40000")).await;
    fx.mint_on(lan_dash, &format!("127.0.0.1:{lan_dash}"), &[])
        .problem(401, "unauthenticated");
    let s111 = pair(lan_dash, &fx.api);
    let r = fx.mint_on(
        lan_dash,
        &format!("127.0.0.1:{lan_dash}"),
        &[("Cookie", &format!("s111={s111}"))],
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let t = ticket_path(&r.json()["url"], &fx.ph());
    // presented from loopback (not the minting peer)
    fx.panel("GET", &t, &[])
        .problem(403, "panel_ticket_expired");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_site_refused_and_no_cors_headers() {
    let fx = fixture(Opts::default()).await;
    let c = cookie_header(&fx.open());
    let ok = fx.panel(
        "GET",
        "/",
        &[("Cookie", &c), ("Sec-Fetch-Site", "same-site")],
    );
    assert_eq!(ok.status, 200);
    let ok = fx.panel(
        "GET",
        "/",
        &[("Cookie", &c), ("Sec-Fetch-Site", "same-origin")],
    );
    assert_eq!(ok.status, 200);

    // cross-site: refused for any method and mode, cookie or not
    for (m, extra) in [
        ("GET", vec![("Sec-Fetch-Mode", "navigate")]),
        (
            "GET",
            vec![("Sec-Fetch-Mode", "cors"), ("Origin", "http://evil.com")],
        ),
        ("GET", vec![("Sec-Fetch-Mode", "no-cors")]),
        ("PUT", vec![("Origin", "http://127.0.0.1:1")]),
    ] {
        let mut h = vec![("Cookie", c.as_str()), ("Sec-Fetch-Site", "cross-site")];
        h.extend(extra);
        let r = fx.panel(m, "/api/v1/fromradio", &h);
        r.problem(403, "bad_request");
        assert!(
            fx.stub.seen.lock().unwrap().is_empty(),
            "never reached the shim"
        );
    }
    // a cross-site ticket presentation is refused before the swap
    let t = fx.mint(&[]);
    fx.panel("GET", &t, &[("Sec-Fetch-Site", "cross-site")])
        .problem(403, "bad_request");

    // OPTIONS (a preflight) is refused and answers no CORS header
    let r = fx.panel(
        "OPTIONS",
        "/api/v1/toradio",
        &[
            ("Origin", "http://evil.com"),
            ("Access-Control-Request-Method", "PUT"),
        ],
    );
    r.problem(403, "bad_request");

    // shim answers carry no CORS headers either (checked by `panel`)
    let r = fx.panel("GET", "/api/v1/fromradio", &[("Cookie", &c)]);
    assert_eq!(r.status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn host_and_origin_matrix() {
    let fx = fixture(Opts::default()).await;
    let c = cookie_header(&fx.open());
    let p = fx.port;

    // Host: the dashboard's accepted list minus localhost, the panel's own port
    for h in [
        format!("127.0.0.1:{p}"),
        format!("[::1]:{p}"),
        format!("111.localhost:{p}"),
        format!("111.local:{p}"),
        format!("111-2.local:{p}"),
    ] {
        let r = send(p, "GET", "/", &h, &[("Cookie", &c)], None);
        assert_eq!(r.status, 200, "{h}: {}", r.body);
    }
    for h in [
        format!("localhost:{p}"),
        format!("LOCALHOST:{p}"),
        format!("evil.com:{p}"),
        format!("192.168.1.5:{p}"),
        "127.0.0.1:11180".to_string(),
        "127.0.0.1".to_string(),
        format!("::1:{p}"),
    ] {
        let r = send(p, "GET", "/", &h, &[("Cookie", &c)], None);
        r.problem(400, "bad_request");
        r.no_cors();
        // refusals carry the response policy too (no accepted host ⇒ 'none')
        assert_eq!(
            r.header("content-security-policy"),
            Some("frame-ancestors 'none'"),
            "{h}"
        );
    }

    // Origin authority (host and port) on non-GET; Referer when Origin is absent
    let o = fx.origin();
    let body = Some("\u{1}\u{2}");
    let put = |hs: &[(&str, &str)]| {
        let mut h = vec![("Cookie", c.as_str())];
        h.extend_from_slice(hs);
        let r = send(p, "PUT", "/api/v1/toradio", &fx.ph(), &h, body);
        r.no_cors();
        r
    };
    assert_eq!(put(&[("Origin", &o)]).status, 200);
    assert_eq!(put(&[("Referer", &format!("{o}/x?y=1"))]).status, 200);
    for bad in [
        format!("http://127.0.0.1:{}", fx.dash),
        "http://127.0.0.1".to_string(),
        format!("https://127.0.0.1:{p}"),
        format!("http://111.localhost:{p}"),
        "null".to_string(),
    ] {
        put(&[("Origin", &bad)]).problem(403, "bad_request");
    }
    put(&[]).problem(403, "bad_request");
    // an SSE open and a WebSocket upgrade are guarded like non-GET
    let r = fx.panel(
        "GET",
        "/api/v1/fromradio",
        &[("Cookie", &c), ("Accept", "text/event-stream")],
    );
    r.problem(403, "bad_request");
    let r = fx.panel("GET", "/", &[("Cookie", &c), ("Upgrade", "websocket")]);
    r.problem(403, "bad_request");

    // LAN sharing on: the node's own address is accepted, others are not
    let lan = fixture(Opts {
        lan: true,
        own: vec!["192.168.1.5".parse().unwrap()],
        ..Opts::default()
    })
    .await;
    let lc = cookie_header(&lan.open());
    let lp = lan.port;
    let r = send(
        lp,
        "GET",
        "/",
        &format!("192.168.1.5:{lp}"),
        &[("Cookie", &lc)],
        None,
    );
    assert_eq!(r.status, 200);
    send(
        lp,
        "GET",
        "/",
        &format!("192.168.1.9:{lp}"),
        &[("Cookie", &lc)],
        None,
    )
    .problem(400, "bad_request");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn main_listener_redirects_localhost() {
    let fx = fixture(Opts::default()).await;
    let d = fx.dash;
    let r = send(d, "GET", "/", &format!("localhost:{d}"), &[], None);
    assert_eq!(r.status, 308);
    assert_eq!(
        r.header("location"),
        Some(format!("http://127.0.0.1:{d}/").as_str())
    );
    let r = send(d, "GET", "/messages?x=1&y=2", "LOCALHOST", &[], None);
    assert_eq!(r.status, 308);
    assert_eq!(
        r.header("location"),
        Some("http://127.0.0.1/messages?x=1&y=2")
    );
    // the method and body survive a 308; API calls are redirected too
    let r = send(
        d,
        "POST",
        "/api/v1/panels/meshtastic/session",
        &format!("localhost.:{d}"),
        &[("Content-Type", "application/json")],
        Some("{}"),
    );
    assert_eq!(r.status, 308);
    assert_eq!(
        r.header("location"),
        Some(format!("http://127.0.0.1:{d}/api/v1/panels/meshtastic/session").as_str())
    );
    // [::1] is matched bracketed and served; other hosts are untouched
    for h in [
        format!("[::1]:{d}"),
        format!("127.0.0.1:{d}"),
        "111.localhost".into(),
    ] {
        let r = send(d, "GET", "/api/v1/panels", &h, &[], None);
        assert_eq!(r.status, 200, "{h}");
    }
    // the Layer 1 machine contracts keep answering on any accepted Host
    let r = send(d, "GET", "/health", &format!("localhost:{d}"), &[], None);
    assert!(matches!(r.status, 200 | 503), "{}", r.status);
    // a refused Host is still refused (the Host guard runs first)
    send(d, "GET", "/", "evil.com", &[], None).problem(400, "bad_request");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn response_policy_rewrites_headers() {
    let fx = fixture(Opts::default()).await;
    let c = cookie_header(&fx.open());
    let check = |r: &Resp, host: &str| {
        assert!(r.header("x-frame-options").is_none());
        assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(r.header("referrer-policy"), Some("no-referrer"));
        let csp = r.all("content-security-policy");
        assert_eq!(
            csp,
            [format!("frame-ancestors http://{host}:11180 http://{host}:80").as_str()],
            "only ours, never the dashboard's CSP"
        );
        // hop-by-hop: only the client's own `Connection: close` comes back
        for k in ["connection", "keep-alive", "upgrade", "proxy-connection"] {
            assert!(
                r.header(k)
                    .is_none_or(|v| k == "connection" && v == "close"),
                "{k}: {:?}",
                r.header(k)
            );
        }
        r.no_cors();
    };
    // static file, SPA fallback, shim answer, redirect, refusal — all policed
    for path in ["/", "/assets/app.js", "/messages/7", "/api/v1/fromradio"] {
        let r = fx.panel("GET", path, &[("Cookie", &c)]);
        assert_eq!(r.status, 200, "{path}");
        check(&r, "127.0.0.1");
    }
    let t = fx.mint(&[]);
    check(&fx.panel("GET", &t, &[("Cookie", &c)]), "127.0.0.1");
    check(&fx.panel("GET", "/", &[]), "127.0.0.1");
    // frame-ancestors follows the request's accepted host
    for (h, u) in [
        (format!("[::1]:{}", fx.port), "[::1]"),
        (format!("111.localhost:{}", fx.port), "111.localhost"),
    ] {
        let r = send(fx.port, "GET", "/", &h, &[("Cookie", &c)], None);
        assert_eq!(r.status, 200);
        check(&r, u);
    }
    // the dashboard keeps its own policy (frame-ancestors 'none')
    let r = send(
        fx.dash,
        "GET",
        "/api/v1/panels",
        &format!("127.0.0.1:{}", fx.dash),
        &[],
        None,
    );
    let csp = r.header("content-security-policy").unwrap();
    assert!(csp.contains("frame-ancestors 'none'") && csp.contains("default-src 'self'"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spa_fallback_vs_api_paths() {
    let fx = fixture(Opts {
        edit: |t| t.replace("max_body = \"8MiB\"", "max_body = \"16B\""),
        ..Opts::default()
    })
    .await;
    let c1 = fx.open();
    let c = cookie_header(&c1);
    let get = |path: &str| fx.panel("GET", path, &[("Cookie", &c)]);

    // plain files by extension, directories, client routes
    let r = get("/devices/list.json");
    assert_eq!(
        (r.status, r.header("content-type")),
        (200, Some("application/json"))
    );
    let r = get("/assets/app.js");
    assert_eq!(r.status, 200);
    assert_eq!(
        r.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    for route in ["/messages/7", "/map", "/config/radio/"] {
        let r = get(route);
        assert_eq!((r.status, r.body.as_str()), (200, INDEX), "{route}");
    }
    // a missing asset is a 404, not HTML
    let r = get("/assets/missing.js");
    assert_eq!(r.status, 404);
    assert_ne!(r.body, INDEX);
    // traversal never escapes the bundle
    for p in ["/../state/panel-sessions.json", "/%2e%2e/x", "/a%2Fb"] {
        let r = get(p);
        assert!(!r.body.contains("sessions"), "{p}");
    }

    // API paths: the shim, keyed by the p111 session; never index.html
    let r = get("/api/v1/fromradio");
    assert_eq!(r.status, 200);
    let session_a = r.body.clone();
    assert_eq!(session_a.len(), 32, "opaque session key, never the cookie");
    assert_ne!(session_a, c1);
    assert_eq!(
        get("/api/v1/fromradio").body,
        session_a,
        "stable per cookie"
    );
    let other = cookie_header(&fx.open());
    let r = fx.panel("GET", "/api/v1/fromradio", &[("Cookie", &other)]);
    assert_ne!(r.body, session_a, "one shim session per panel session");
    {
        let seen = fx.stub.seen.lock().unwrap();
        assert!(seen.iter().all(|(s, _)| s.panel == "meshtastic"));
        assert!(seen.iter().all(|(s, _)| s.peer.is_loopback()));
    }
    // upstream down → 502 upstream_error (problem+json)
    get("/api/v1/down").problem(502, "upstream_error");
    // unknown API paths → problem 404, never the SPA
    for p in ["/api/v1/nope", "/api", "/api/", "/api/v1/nope.html"] {
        let r = fx.panel(
            "GET",
            p,
            &[
                ("Cookie", &c),
                ("Accept", "text/html,application/xhtml+xml"),
            ],
        );
        r.problem(404, "not_found");
    }
    // body cap ([proxy] max_body) before the shim sees a body
    let o = fx.origin();
    let r = send(
        fx.port,
        "PUT",
        "/api/v1/toradio",
        &fx.ph(),
        &[("Cookie", &c), ("Origin", &o)],
        Some("0123456789abcdef"),
    );
    assert_eq!((r.status, r.body.as_str()), (200, "0123456789abcdef"));
    send(
        fx.port,
        "PUT",
        "/api/v1/toradio",
        &fx.ph(),
        &[("Cookie", &c), ("Origin", &o)],
        Some("0123456789abcdefX"),
    )
    .problem(413, "bad_request");
    // static paths are GET/HEAD only
    let r = send(
        fx.port,
        "POST",
        "/",
        &fx.ph(),
        &[("Cookie", &c), ("Origin", &o)],
        Some("{}"),
    );
    r.problem(405, "bad_request");

    // a panel without a shim: API paths are a 404 problem, never the SPA
    let plain = fixture(Opts {
        edit: |t| {
            t.lines()
                .filter(|l| !l.trim_start().starts_with("shim ="))
                .collect::<Vec<_>>()
                .join("\n")
        },
        ..Opts::default()
    })
    .await;
    let pc = cookie_header(&plain.open());
    plain
        .panel("GET", "/api/v1/fromradio", &[("Cookie", &pc)])
        .problem(404, "not_found");
    assert!(plain.stub.seen.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dashboard_csp_is_per_host() {
    let fx = fixture(Opts {
        lan: true,
        own: vec!["192.168.1.5".parse().unwrap()],
        ..Opts::default()
    })
    .await;
    let d = fx.dash;
    let p = fx.port;
    let csp = |host: &str| {
        let r = send(d, "GET", "/api/v1/panels", host, &[], None);
        assert_eq!(r.status, 200, "{host}");
        r.header("content-security-policy").unwrap().to_string()
    };
    for (h, origin) in [
        (format!("127.0.0.1:{d}"), format!("http://127.0.0.1:{p}")),
        (format!("[::1]:{d}"), format!("http://[::1]:{p}")),
        (
            "111.localhost".to_string(),
            format!("http://111.localhost:{p}"),
        ),
        ("111.local".to_string(), format!("http://111.local:{p}")),
        (
            format!("192.168.1.5:{d}"),
            format!("http://192.168.1.5:{p}"),
        ),
    ] {
        let v = csp(&h);
        assert_eq!(v, format!("{}; frame-src {origin}", routes::CSP), "{h}");
        assert!(v.contains("frame-ancestors 'none'"));
    }
    // the UI shell gets it too
    let r = send(d, "GET", "/", &format!("127.0.0.1:{d}"), &[], None);
    assert!(r
        .header("content-security-policy")
        .unwrap()
        .ends_with(&format!("frame-src http://127.0.0.1:{p}")));
    // panel disabled ⇒ nothing to frame ⇒ exactly the closed policy
    set_enabled(&fx.host, &fx.api, false).await;
    assert_eq!(csp(&format!("127.0.0.1:{d}")), routes::CSP);
    wait_closed(p).await;
    set_enabled(&fx.host, &fx.api, true).await;
    assert!(csp(&format!("127.0.0.1:{d}")).contains("frame-src"));

    // a loopback-only panel is never framed on a LAN host
    let lo = fixture(Opts {
        lan: true,
        own: vec!["192.168.1.5".parse().unwrap()],
        edit: |t| t.replace("mode = \"111-session\"", "mode = \"loopback-only\""),
        ..Opts::default()
    })
    .await;
    let ld = lo.dash;
    let r = send(
        ld,
        "GET",
        "/api/v1/panels",
        &format!("192.168.1.5:{ld}"),
        &[],
        None,
    );
    assert_eq!(r.header("content-security-policy"), Some(routes::CSP));
    let r = send(
        ld,
        "GET",
        "/api/v1/panels",
        &format!("127.0.0.1:{ld}"),
        &[],
        None,
    );
    assert!(r
        .header("content-security-policy")
        .unwrap()
        .contains("frame-src"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn loopback_only_panels_bind_loopback() {
    // LAN sharing on and no test override: the listener still binds loopback
    let fx = fixture(Opts {
        lan: true,
        own: vec!["192.168.1.5".parse().unwrap()],
        force_loopback: false,
        edit: |t| t.replace("mode = \"111-session\"", "mode = \"loopback-only\""),
        ..Opts::default()
    })
    .await;
    let addrs = fx.host.bound_addrs(ServiceId::Meshtastic);
    assert!(!addrs.is_empty());
    assert!(addrs.iter().all(|a| a.ip().is_loopback()), "{addrs:?}");
    assert!(addrs.iter().all(|a| a.port() == fx.port));
    // the bind plan of a 111-session panel follows the dashboard
    use node111::daemon::panels::listener::bind_ip;
    use node111::daemon::panels::manifest::AuthMode;
    assert!(bind_ip(true, AuthMode::Session111).is_unspecified());
    assert!(bind_ip(true, AuthMode::LoopbackOnly).is_loopback());

    // the ticket flow is unchanged on loopback
    let c = cookie_header(&fx.open());
    assert_eq!(fx.panel("GET", "/", &[("Cookie", &c)]).status, 200);
    // loopback names only, even with LAN sharing on
    let p = fx.port;
    for h in [format!("192.168.1.5:{p}"), format!("111.local:{p}")] {
        send(p, "GET", "/", &h, &[("Cookie", &c)], None).problem(400, "bad_request");
    }
    // minting from a LAN dashboard host is refused (the URL could not work)
    fx.mint_on(fx.dash, &format!("192.168.1.5:{}", fx.dash), &[])
        .problem(403, "loopback_only");
    // and the panel list says where it is
    let r = send(
        fx.dash,
        "GET",
        "/api/v1/panels",
        &format!("127.0.0.1:{}", fx.dash),
        &[],
        None,
    );
    assert_eq!(r.json()["panels"][0]["state"]["listening"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn untrusted_loopback_still_needs_the_ticket() {
    // Termux: loopback is not trusted — the browser pairs like a LAN peer
    let fx = fixture(Opts {
        trust_loopback: false,
        ..Opts::default()
    })
    .await;
    fx.panel("GET", "/", &[]).problem(401, "unauthenticated");
    fx.mint_on(fx.dash, &format!("127.0.0.1:{}", fx.dash), &[])
        .problem(401, "unauthenticated");
    let s111 = pair(fx.dash, &fx.api);
    let s = format!("s111={s111}");
    // the dashboard cookie is never a panel credential
    fx.panel("GET", "/", &[("Cookie", &s)])
        .problem(401, "unauthenticated");
    let t = fx.mint(&[("Cookie", &s)]);
    let c = cookie_header(&fx.swap(&t));
    assert_eq!(fx.panel("GET", "/", &[("Cookie", &c)]).status, 200);
    // bound to the minting session: logging out revokes the panel session
    let r = send(
        fx.dash,
        "POST",
        "/api/v1/auth/logout",
        &format!("127.0.0.1:{}", fx.dash),
        &[("Content-Type", "application/json"), ("Cookie", &s)],
        Some("{}"),
    );
    assert!(r.status < 300, "{} {}", r.status, r.body);
    fx.panel("GET", "/", &[("Cookie", &c)])
        .problem(401, "unauthenticated");
    assert!(fx.host.sessions.snapshot().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revocation_on_rotate_disable_and_lan_off() {
    // trusted loopback (implicit session) — bound to the pair-code generation
    let fx = fixture(Opts::default()).await;
    let mut revoked = fx.host.revocations();
    let c = cookie_header(&fx.open());
    let pending = fx.mint(&[]);
    assert_eq!(fx.panel("GET", "/", &[("Cookie", &c)]).status, 200);
    let session = fx.host.sessions.snapshot()[0].id.clone();
    auth::rotate_files(&fx.layout.state_dir()).unwrap();
    // the 2 s sweep (or the next request) revokes it
    assert_eq!(fx.host.sweep(&fx.api), 1);
    let gone = revoked.try_recv().unwrap();
    assert_eq!(
        (gone.panel.as_str(), gone.session.as_str()),
        ("meshtastic", session.as_str())
    );
    fx.panel("GET", "/", &[("Cookie", &c)])
        .problem(401, "unauthenticated");
    // an outstanding ticket of the old generation is dropped too
    fx.panel("GET", &pending, &[])
        .problem(403, "panel_ticket_expired");
    let text = std::fs::read_to_string(fx.layout.state_dir().join(ticket::SESSIONS_FILE)).unwrap();
    assert!(!text.contains(&session));

    // paired session (Termux / LAN browser): rotate empties the session file
    let tx = fixture(Opts {
        trust_loopback: false,
        ..Opts::default()
    })
    .await;
    let s = format!("s111={}", pair(tx.dash, &tx.api));
    let t = tx.mint(&[("Cookie", &s)]);
    let c = cookie_header(&tx.swap(&t));
    assert_eq!(tx.panel("GET", "/", &[("Cookie", &c)]).status, 200);
    auth::rotate_files(&tx.layout.state_dir()).unwrap();
    tx.panel("GET", "/", &[("Cookie", &c)])
        .problem(401, "unauthenticated");
    assert!(tx.host.sessions.snapshot().is_empty());

    // panel disable: listener gone, sessions and tickets dropped, mint 404
    let fx = fixture(Opts::default()).await;
    let _c = fx.open();
    let t = fx.mint(&[]);
    set_enabled(&fx.host, &fx.api, false).await;
    assert!(fx.host.sessions.snapshot().is_empty());
    assert!(fx.host.tickets.is_empty(), "{t} dropped");
    assert!(fx.host.bound_addrs(ServiceId::Meshtastic).is_empty());
    wait_closed(fx.port).await;
    fx.mint_on(fx.dash, &format!("127.0.0.1:{}", fx.dash), &[])
        .problem(404, "not_found");
    let r = send(
        fx.dash,
        "GET",
        "/api/v1/panels",
        &format!("127.0.0.1:{}", fx.dash),
        &[],
        None,
    );
    let st = &r.json()["panels"][0]["state"];
    assert_eq!(
        (st["enabled"].as_bool(), st["listening"].as_bool()),
        (Some(false), Some(false))
    );
    // re-enabled: same slot, a fresh ticket works
    set_enabled(&fx.host, &fx.api, true).await;
    let c = cookie_header(&fx.open());
    assert_eq!(fx.panel("GET", "/", &[("Cookie", &c)]).status, 200);

    // LAN-off: sessions minted while LAN sharing was on do not survive a start
    // with it off; loopback sessions minted with it off do
    let on = fixture(Opts {
        lan: true,
        own: vec!["192.168.1.5".parse().unwrap()],
        ..Opts::default()
    })
    .await;
    let _lan_cookie = on.open();
    assert_eq!(on.host.sessions.snapshot().len(), 1);
    assert!(on.host.sessions.snapshot()[0].lan);
    let cfg = panel_config(&on.layout, on.port, &Opts::default());
    let off = PanelHost::new(cfg, Vec::new(), &ShimRegistry::new(), false);
    assert!(off.sessions.snapshot().is_empty(), "LAN-off revokes");
    let off_fx = fixture(Opts::default()).await;
    let _c = off_fx.open();
    let cfg = panel_config(&off_fx.layout, off_fx.port, &Opts::default());
    let again = PanelHost::new(cfg, Vec::new(), &ShimRegistry::new(), false);
    assert_eq!(again.sessions.snapshot().len(), 1, "loopback session kept");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn panel_list_slots_and_registry_facts() {
    let fx = fixture(Opts::default()).await;
    let r = send(
        fx.dash,
        "GET",
        "/api/v1/panels",
        &format!("127.0.0.1:{}", fx.dash),
        &[],
        None,
    );
    assert_eq!(r.status, 200);
    let j = r.json();
    assert_eq!(j["schema"], 1);
    let p = &j["panels"][0];
    assert_eq!(p["id"], "meshtastic");
    assert_eq!(p["port"], fx.port);
    assert_eq!(p["entry"], "/");
    assert_eq!(p["needs_secure_context"], false);
    assert_eq!(p["allow"], serde_json::json!(["serial", "bluetooth"]));
    assert_eq!(
        p["optional_hosts"],
        serde_json::json!(["tiles.openfreemap.org", "api.meshtastic.org"])
    );
    assert_eq!(p["title"], "Meshtastic");
    assert!(p["secure_note"].is_string());
    assert_eq!(p["state"]["enabled"], true);
    assert_eq!(p["state"]["listening"], true);
    assert_eq!(p["state"]["health"], "unknown");

    // the services-registry provider hands the same facts
    let provided = (panels::provider(&fx.host))().expect("facts");
    assert_eq!(serde_json::to_value(&provided).unwrap(), j["panels"]);
    // a shim drop count is surfaced
    fx.host.report_dropped(ServiceId::Meshtastic, 3);
    assert_eq!(fx.host.panels()[0].state.dropped, Some(3));

    // slots are stable in state/panels.json
    let slots: serde_json::Value = serde_json::from_slice(
        &std::fs::read(fx.layout.state_dir().join(panels::SLOTS_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(slots["slots"]["meshtastic"], fx.port);
    // a wider block with lower free ports: the recorded slot is kept
    let mut cfg = panel_config(&fx.layout, fx.port, &Opts::default());
    cfg.ports = fx.port - 3..=fx.port + 3;
    let mut shims = ShimRegistry::new();
    shims
        .register(Arc::new(Stub::default()) as Arc<dyn Shim>)
        .unwrap();
    let again = PanelHost::new(
        cfg,
        vec![("meshtastic.toml".into(), shipped_manifest().into_bytes())],
        &shims,
        false,
    );
    assert_eq!(again.panel(ServiceId::Meshtastic).unwrap().port, fx.port);

    // unregistered declarations: unknown key, reserved websocket, unknown shim
    for text in [
        format!("extra = 1\n{}", shipped_manifest()),
        shipped_manifest().replace("websocket = false", "websocket = true"),
        shipped_manifest().replace("sse = false", "sse = true"),
    ] {
        let cfg = panel_config(&fx.layout, fx.port, &Opts::default());
        let h = PanelHost::new(
            cfg,
            vec![("meshtastic.toml".into(), text.into_bytes())],
            &shims,
            false,
        );
        assert!(h.registered().is_empty());
    }
    let cfg = panel_config(&fx.layout, fx.port, &Opts::default());
    let h = PanelHost::new(
        cfg,
        vec![("meshtastic.toml".into(), shipped_manifest().into_bytes())],
        &ShimRegistry::new(),
        false,
    );
    assert!(h.registered().is_empty(), "the named shim is not built in");

    // no panel host at all: an empty list
    let bare = {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("state")).unwrap();
        let layout = DaemonLayout {
            root: dir.path().to_path_buf(),
            etc: dir.path().join("etc"),
        };
        let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.14")));
        (ApiState::new(dash, ApiConfig::new(layout, false)), dir)
    };
    let bp = serve_dash(&bare.0, None).await;
    let r = send(
        bp,
        "GET",
        "/api/v1/panels",
        &format!("127.0.0.1:{bp}"),
        &[],
        None,
    );
    assert_eq!(r.json()["panels"], serde_json::json!([]));
    assert_eq!(r.header("content-security-policy"), Some(routes::CSP));
}
