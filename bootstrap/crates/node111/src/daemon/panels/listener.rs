//! One listener per enabled panel (Layer 5 §13 "Listeners", "Auth", "Guards",
//! "Response policy").
//!
//! Request pipeline ([`app`]), every method, every path:
//! 1. guards ([`guard::check`]): exact Host (minus `localhost`, the panel's own
//!    port), `Sec-Fetch-Site` (same-origin only on API paths), Origin
//!    authority, `OPTIONS` refused;
//! 2. a ticket parameter ⇒ the swap: a request carrying a valid `p111_<id>`
//!    ignores (and spends) the ticket and is redirected to `entry.path`;
//!    otherwise a live ticket of this panel, this peer and a still-alive
//!    minting session becomes the cookie (`303` to `entry.path`); a spent,
//!    expired or unknown ticket is `403 panel_ticket_expired`;
//! 3. no valid `p111_<id>` ⇒ `401 unauthenticated` (no loopback skip);
//! 4. `/api/*` on a shim panel ⇒ the shim (body capped at `[proxy]
//!    max_body` and read under an absolute deadline, [`Timeouts::body`] —
//!    `408` when it is not in by then; `502 upstream_error` when its upstream
//!    is down); everything else ⇒ the static bundle with the SPA fallback for
//!    non-API paths;
//! 5. the response policy ([`headers::apply`]) on every answer.
//!
//! The generic reverse proxy (`[source] kind = child|detected`, WebSocket/SSE)
//! is reserved: such manifests are never registered, so step 4 has exactly two
//! arms. A proxy arm slots in beside the shim arm, applying
//! [`headers::strip_request_cookies`] / [`headers::filter_upstream_set_cookie`].
//!
//! Serving ([`serve`], §13 "Header-read and idle timeouts on every
//! listener"): HTTP/1.1 over hyper with a header-read timeout (re-armed for
//! every request head, so it also bounds keep-alive idle) and an I/O idle
//! timeout per connection, a cap on concurrent connections **and a smaller
//! one per peer address** (one LAN host cannot take every slot), and the peer
//! address attached as `ConnectInfo<SocketAddr>` (what the auth helpers
//! read). The main dashboard listener is served by the same loop with
//! [`Timeouts::dashboard`].
//!
//! Binding ([`bind`]): exactly where the dashboard binds — `127.0.0.1:<port>`,
//! or `0.0.0.0:<port>` when LAN sharing is on, plus `[::1]:<port>` best effort;
//! `[auth] mode = "loopback-only"` binds loopback regardless of LAN sharing.

use super::guard::{self, AcceptedHost, HostPolicy};
use super::manifest::AuthMode;
use super::shim::{Shim, ShimError, ShimRequest, ShimSession};
use super::static_files;
use super::ticket::{self, PanelSessionRec, PeerClass};
use super::{headers, Panel, PanelHost};
use crate::daemon::dashboard::api_v1::{auth, Problem};
use crate::daemon::dashboard::ApiState;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};

/// Per-listener limits (§13: "Header-read and idle timeouts on every
/// listener").
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// A request head must arrive within this (also closes idle keep-alive
    /// connections).
    pub header_read: Duration,
    /// No byte read or written for this long closes the connection.
    pub idle: Duration,
    /// Concurrent connections per listener; more are closed on accept.
    pub max_conns: usize,
    /// Concurrent connections per peer address (canonical IP) on one
    /// listener; more are closed on accept.
    pub max_conns_per_peer: usize,
    /// A shim request body must be complete this long after its head was
    /// accepted (absolute: a trickled body cannot hold a slot by touching the
    /// idle clock). Only the panel shim arm reads it.
    pub body: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            header_read: Duration::from_secs(10),
            idle: Duration::from_secs(60),
            max_conns: 128,
            max_conns_per_peer: 16,
            body: Duration::from_secs(30),
        }
    }
}

/// The main dashboard listener's I/O idle timeout: longer than its longest
/// legitimately silent exchange (a non-streamed chat turn, whose relay budget
/// is 10 min; SSE streams carry a keep-alive every 15 s).
pub const DASHBOARD_IDLE: Duration = Duration::from_secs(11 * 60);

impl Timeouts {
    /// The main dashboard listener (§13 "every listener"): the same
    /// header-read timeout (slow heads and idle keep-alive), an I/O idle
    /// timeout of [`DASHBOARD_IDLE`], and no connection caps — the dashboard
    /// never had any, and a cap there would let one peer lock the owner out
    /// of the whole node UI. Its bodies are bounded by axum's body limit.
    pub fn dashboard() -> Self {
        Timeouts {
            idle: DASHBOARD_IDLE,
            max_conns: usize::MAX,
            max_conns_per_peer: usize::MAX,
            ..Timeouts::default()
        }
    }
}

/// What one panel listener's handlers share.
pub struct PanelCtx {
    pub api: Arc<ApiState>,
    pub host: Arc<PanelHost>,
    pub panel: Arc<Panel>,
}

/// The panel origin's app (guards, auth, dispatch, response policy). Serve it
/// with [`serve`] (production) or any server that attaches the peer address.
pub fn app(ctx: PanelCtx) -> axum::Router {
    axum::Router::new()
        .fallback(handle)
        .with_state(Arc::new(ctx))
}

async fn handle(State(ctx): State<Arc<PanelCtx>>, req: Request) -> Response {
    let (accepted, mut resp) = respond(&ctx, req).await;
    headers::apply(resp.headers_mut(), accepted.as_ref());
    resp
}

fn no_store(mut r: Response) -> Response {
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn no_session() -> Response {
    no_store(
        Problem::new(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "panel session required",
        )
        .detail("Open this panel from the 111 dashboard.")
        .into_response(),
    )
}

fn ticket_expired() -> Response {
    no_store(
        Problem::new(
            StatusCode::FORBIDDEN,
            "panel_ticket_expired",
            "panel ticket expired",
        )
        .detail("Reopen the panel from the 111 dashboard.")
        .into_response(),
    )
}

fn to_entry(entry: &str) -> Response {
    let mut r = StatusCode::SEE_OTHER.into_response();
    if let Ok(v) = HeaderValue::from_str(entry) {
        r.headers_mut().insert(header::LOCATION, v);
    }
    no_store(r)
}

async fn respond(ctx: &PanelCtx, req: Request) -> (Option<AcceptedHost>, Response) {
    let panel = &ctx.panel;
    // fail closed: a request without a known peer is refused
    let Some(peer) = auth::peer_addr(req.extensions()).map(|a| a.ip().to_canonical()) else {
        let p = Problem::new(StatusCode::FORBIDDEN, "bad_request", "unknown peer");
        return (None, p.into_response());
    };
    let own = ctx.api.own_addrs();
    let policy = HostPolicy {
        lan: ctx.api.lan_enabled,
        own: &own,
        loopback_only: panel.manifest.auth == AuthMode::LoopbackOnly,
    };
    let accepted = match guard::check(
        req.method(),
        req.uri().path(),
        req.headers(),
        panel.port,
        &policy,
    ) {
        Ok(a) => a,
        Err(r) => return (None, r.problem().into_response()),
    };
    let class = PeerClass::of(peer);
    let cookies = ticket::panel_cookies(req.headers(), panel.id());
    let session = ctx.host.session_for(&ctx.api, panel.id(), &cookies, class);

    if let Some(presented) = ticket::ticket_param(req.uri().query()) {
        let resp = swap(ctx, req.method(), &presented, session.is_some(), class);
        return (Some(accepted), resp);
    }
    let Some(session) = session else {
        return (Some(accepted), no_session());
    };
    if req.headers().contains_key(header::UPGRADE) {
        let p = Problem::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "upgrades are not available on this panel",
        );
        return (Some(accepted), p.into_response());
    }
    let path = req.uri().path().to_string();
    if static_files::is_api_path(&path) {
        if let Some(shim) = &panel.shim {
            let resp = dispatch(ctx, shim.as_ref(), &session, peer, req).await;
            return (Some(accepted), resp);
        }
    }
    let resp = static_files::serve(&panel.bundle_dir, req.method(), &path, req.headers()).await;
    (Some(accepted), resp)
}

/// The ticket swap (§13 lifecycle). Any presentation burns the ticket.
fn swap(
    ctx: &PanelCtx,
    method: &Method,
    presented: &str,
    has_session: bool,
    class: PeerClass,
) -> Response {
    let panel = &ctx.panel;
    let rec = ctx.host.tickets.take(presented);
    if has_session {
        return to_entry(&panel.manifest.entry);
    }
    if *method != Method::GET && *method != Method::HEAD {
        return ticket_expired();
    }
    let Some(t) = rec else {
        return ticket_expired();
    };
    if t.expired()
        || t.panel != panel.id()
        || t.peer != class
        || !ctx.host.is_enabled(panel.id())
        || !ticket::binding_alive(&ctx.api, &t.binding)
    {
        return ticket_expired();
    }
    match ctx
        .host
        .sessions
        .create(panel.id(), class, t.binding, ctx.api.lan_enabled)
    {
        Ok(new) => {
            let mut r = to_entry(&panel.manifest.entry);
            if let Ok(v) = HeaderValue::from_str(&ticket::set_cookie(panel.id(), &new.token)) {
                r.headers_mut().insert(header::SET_COOKIE, v);
            }
            r
        }
        Err(e) => {
            eprintln!(
                "111 daemon: panel {}: cannot create a session: {e}",
                panel.id().as_str()
            );
            Problem::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "cannot open a panel session",
            )
            .into_response()
        }
    }
}

/// Is this body error a length-limit overflow?
fn is_length_limit(e: &axum::Error) -> bool {
    let mut src: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(s) = src {
        if s.is::<http_body_util::LengthLimitError>() {
            return true;
        }
        src = s.source();
    }
    false
}

/// `408`: the shim request body was not complete within [`Timeouts::body`].
/// The unread rest of the body makes hyper close the connection.
fn body_timeout(deadline: Duration) -> Response {
    Problem::new(
        StatusCode::REQUEST_TIMEOUT,
        "bad_request",
        "request body too slow",
    )
    .detail(format!(
        "The whole body must arrive within {} s.",
        deadline.as_secs().max(1)
    ))
    .into_response()
}

/// Hand an `/api/*` request to the panel's shim.
async fn dispatch(
    ctx: &PanelCtx,
    shim: &dyn Shim,
    session: &PanelSessionRec,
    peer: IpAddr,
    req: Request,
) -> Response {
    let limit = ctx.panel.manifest.max_body;
    let (parts, body) = req.into_parts();
    let declared = parts
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    if declared.is_some_and(|n| n > limit as u64) {
        return ShimError::TooLarge { limit }.problem().into_response();
    }
    let deadline = ctx.host.cfg.timeouts.body;
    let body = match tokio::time::timeout(deadline, axum::body::to_bytes(body, limit)).await {
        Ok(Ok(b)) => b,
        Ok(Err(e)) if is_length_limit(&e) => {
            return ShimError::TooLarge { limit }.problem().into_response()
        }
        Ok(Err(_)) => {
            return ShimError::BadRequest("the request body could not be read".into())
                .problem()
                .into_response()
        }
        Err(_) => return body_timeout(deadline),
    };
    let s = ShimSession {
        panel: ctx.panel.id().as_str().to_string(),
        session: session.id.clone(),
        peer,
    };
    let r = ShimRequest {
        method: parts.method,
        path: parts.uri.path().to_string(),
        query: parts.uri.query().map(str::to_string),
        content_type: parts
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        body,
    };
    match shim.handle(&s, r).await {
        Ok(out) => {
            let mut resp = Response::new(Body::from(out.body));
            *resp.status_mut() = out.status;
            if let Some(ct) = out
                .content_type
                .and_then(|c| HeaderValue::from_str(&c).ok())
            {
                resp.headers_mut().insert(header::CONTENT_TYPE, ct);
            }
            no_store(resp)
        }
        Err(e) => no_store(e.problem().into_response()),
    }
}

// ---------------------------------------------------------------- serving

/// Per-connection activity clock (monotonic).
struct Activity {
    start: Instant,
    last_ms: AtomicU64,
}

impl Activity {
    fn new() -> Self {
        Activity {
            start: Instant::now(),
            last_ms: AtomicU64::new(0),
        }
    }

    fn touch(&self) {
        let ms = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.last_ms.store(ms, Ordering::Relaxed);
    }

    fn idle_for(&self) -> Duration {
        self.start
            .elapsed()
            .saturating_sub(Duration::from_millis(self.last_ms.load(Ordering::Relaxed)))
    }
}

/// A TCP stream that records every byte of progress in [`Activity`].
struct IdleIo {
    inner: TcpStream,
    activity: Arc<Activity>,
}

impl AsyncRead for IdleIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let r = Pin::new(&mut this.inner).poll_read(cx, buf);
        if matches!(r, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            this.activity.touch();
        }
        r
    }
}

impl AsyncWrite for IdleIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let r = Pin::new(&mut this.inner).poll_write(cx, buf);
        if matches!(r, Poll::Ready(Ok(n)) if n > 0) {
            this.activity.touch();
        }
        r
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let r = Pin::new(&mut this.inner).poll_write_vectored(cx, bufs);
        if matches!(r, Poll::Ready(Ok(n)) if n > 0) {
            this.activity.touch();
        }
        r
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

async fn serve_conn(stream: TcpStream, peer: SocketAddr, app: axum::Router, t: Timeouts) {
    let _ = stream.set_nodelay(true);
    let activity = Arc::new(Activity::new());
    let io = TokioIo::new(IdleIo {
        inner: stream,
        activity: Arc::clone(&activity),
    });
    let tower = TowerToHyperService::new(app);
    let svc = hyper::service::service_fn(move |mut req: hyper::Request<hyper::body::Incoming>| {
        req.extensions_mut().insert(ConnectInfo(peer));
        hyper::service::Service::call(&tower, req)
    });
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(t.header_read);
    // upgrades stay possible at the protocol level (axum's own `serve` allowed
    // them); panels refuse every upgrade in [`respond`]
    let conn = builder.serve_connection(io, svc).with_upgrades();
    tokio::pin!(conn);
    loop {
        let idle = activity.idle_for();
        if idle >= t.idle {
            return; // dropping the connection closes it
        }
        tokio::select! {
            _ = conn.as_mut() => return,
            _ = tokio::time::sleep(t.idle - idle) => {}
        }
    }
}

/// Open connections per peer address of one listener.
#[derive(Default)]
struct PeerSlots {
    open: std::sync::Mutex<std::collections::HashMap<IpAddr, usize>>,
}

/// One held per-peer slot; released when dropped (the connection task ends,
/// is aborted, or unwinds).
struct PeerSlot {
    slots: Arc<PeerSlots>,
    ip: IpAddr,
}

impl PeerSlots {
    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<IpAddr, usize>> {
        self.open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Take a slot for `ip` unless it already holds `cap`.
    fn take(self: &Arc<Self>, ip: IpAddr, cap: usize) -> Option<PeerSlot> {
        let ip = ip.to_canonical();
        let mut open = self.lock();
        if open.get(&ip).copied().unwrap_or(0) >= cap {
            return None; // never leaves an empty entry behind
        }
        *open.entry(ip).or_insert(0) += 1;
        Some(PeerSlot {
            slots: Arc::clone(self),
            ip,
        })
    }

    #[cfg(test)]
    fn held(&self, ip: IpAddr) -> usize {
        self.lock().get(&ip.to_canonical()).copied().unwrap_or(0)
    }
}

impl Drop for PeerSlot {
    fn drop(&mut self) {
        let mut open = self.slots.lock();
        if let Some(n) = open.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                open.remove(&self.ip);
            }
        }
    }
}

/// Serve `app` on `listener` until the task is aborted (aborting drops every
/// open connection with it). A connection beyond [`Timeouts::max_conns`], or
/// beyond [`Timeouts::max_conns_per_peer`] for its peer address, is closed on
/// accept.
pub async fn serve(listener: TcpListener, app: axum::Router, t: Timeouts) {
    let mut conns = tokio::task::JoinSet::new();
    let slots = Arc::new(PeerSlots::default());
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    if conns.len() >= t.max_conns {
                        drop(stream);
                        continue;
                    }
                    let Some(slot) = slots.take(peer.ip(), t.max_conns_per_peer) else {
                        drop(stream);
                        continue;
                    };
                    let app = app.clone();
                    conns.spawn(async move {
                        let _slot = slot;
                        serve_conn(stream, peer, app, t).await;
                    });
                }
                // EMFILE and friends: back off instead of spinning
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            },
            Some(_) = conns.join_next(), if !conns.is_empty() => {}
        }
    }
}

/// The IPv4 address a panel listener binds (§13 "bound exactly where the
/// dashboard is bound"; loopback-only panels bind loopback regardless).
pub fn bind_ip(lan: bool, mode: AuthMode) -> IpAddr {
    if lan && mode == AuthMode::Session111 {
        Ipv4Addr::UNSPECIFIED.into()
    } else {
        Ipv4Addr::LOCALHOST.into()
    }
}

/// Bind a panel's listeners: the IPv4 address of [`bind_ip`] (required) and
/// `[::1]` (best effort, silent when IPv6 is off). Empty when the IPv4 bind
/// fails (logged).
pub async fn bind(port: u16, lan: bool, mode: AuthMode) -> Vec<TcpListener> {
    let ip = bind_ip(lan, mode);
    let addr = SocketAddr::new(ip, port);
    let main = match crate::daemon::dashboard::bind::bind_listener(addr, !ip.is_loopback()).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("111 daemon: panel listener {addr} failed: {e:#}");
            return Vec::new();
        }
    };
    let mut out = vec![main];
    let v6 = SocketAddr::new(Ipv6Addr::LOCALHOST.into(), port);
    match TcpListener::bind(v6).await {
        Ok(l) => out.push(l),
        Err(e) => {
            let ipv6_off = matches!(
                e.raw_os_error(),
                Some(libc::EADDRNOTAVAIL) | Some(libc::EAFNOSUPPORT) | Some(libc::EPROTONOSUPPORT)
            );
            if !ipv6_off {
                eprintln!("111 daemon: panel listener {v6} failed ({e}); IPv4 only");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_plan_follows_the_dashboard() {
        let lo: IpAddr = Ipv4Addr::LOCALHOST.into();
        let any: IpAddr = Ipv4Addr::UNSPECIFIED.into();
        assert_eq!(bind_ip(false, AuthMode::Session111), lo);
        assert_eq!(bind_ip(true, AuthMode::Session111), any);
        assert_eq!(bind_ip(false, AuthMode::LoopbackOnly), lo);
        assert_eq!(bind_ip(true, AuthMode::LoopbackOnly), lo);
    }

    #[test]
    fn peer_slots_cap_and_release() {
        let slots = Arc::new(PeerSlots::default());
        let a: IpAddr = "192.168.1.9".parse().unwrap();
        let mapped: IpAddr = "::ffff:192.168.1.9".parse().unwrap();
        let b: IpAddr = "192.168.1.10".parse().unwrap();
        let held: Vec<PeerSlot> = (0..3).map(|_| slots.take(a, 3).unwrap()).collect();
        assert!(slots.take(a, 3).is_none(), "cap per peer");
        assert!(slots.take(mapped, 3).is_none(), "one peer, either spelling");
        let other = slots.take(b, 3).expect("another peer is unaffected");
        assert_eq!((slots.held(a), slots.held(b)), (3, 1));
        drop(held);
        assert_eq!(slots.held(a), 0);
        assert!(slots.take(a, 3).is_some(), "released on drop");
        drop(other);
        assert!(slots.lock().get(&b).is_none(), "no empty entries kept");
        assert!(slots.take(b, 0).is_none());
        assert!(slots.lock().is_empty(), "a refusal records nothing");
    }

    #[test]
    fn dashboard_limits() {
        let d = Timeouts::dashboard();
        let p = Timeouts::default();
        assert_eq!(d.header_read, p.header_read);
        assert!(d.idle > Duration::from_secs(600), "a 10 min chat turn fits");
        assert_eq!(
            (d.max_conns, d.max_conns_per_peer),
            (usize::MAX, usize::MAX)
        );
        assert!(p.max_conns_per_peer < p.max_conns);
    }

    #[test]
    fn activity_clock() {
        let a = Activity::new();
        std::thread::sleep(Duration::from_millis(20));
        assert!(a.idle_for() >= Duration::from_millis(20));
        a.touch();
        assert!(a.idle_for() < Duration::from_millis(20));
    }
}
