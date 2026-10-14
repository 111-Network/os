//! Panel auth: ticket → cookie, everywhere — no loopback skip (Layer 5 §13
//! "Auth").
//!
//! - **Ticket** ([`Tickets`]): minted by `POST /api/v1/panels/{id}/session`
//!   behind the dashboard session (on trusted loopback the session is
//!   implicit; the ticket is still minted). 128 bits from the OS random
//!   source, single use, [`TICKET_TTL`] (30 s, monotonic clock), held in memory
//!   only (by its sha256), bound to the minting session ([`Binding`]) and the
//!   minting peer ([`PeerClass`]). Any presentation burns it.
//! - **Cookie** `p111_<id>` ([`SessionStore`]): host-only (no `Domain`),
//!   `HttpOnly`, `SameSite=Strict`, `Path=/`, a session cookie (no
//!   `Max-Age`). 256 bits; only its sha256 is recorded, in
//!   `state/panel-sessions.json` (0600). Read by exact name; every copy is
//!   tried (D16 f).
//! - **Binding** ([`Binding`], [`binding_alive`]): a session minted under an
//!   `s111` dashboard session lives while that session does (so `111 dashboard
//!   rotate`, logout and forgetting the companion revoke it); one minted on
//!   trusted loopback lives while loopback stays trusted and the pair code is
//!   unchanged (a rotation changes the code). The peer is matched by class:
//!   any loopback address is one class (`111.localhost` may resolve to either
//!   `127.0.0.1` or `::1`), any other peer by exact address.
//! - **Revocation:** binding loss (rotate), panel disable, LAN-off (sessions
//!   minted while LAN sharing was on, or from a non-loopback peer, are dropped
//!   when the daemon starts with LAN sharing off). Every revocation is
//!   broadcast ([`SessionStore::subscribe`]) so a shim can drop its per-session
//!   upstream state.
//!
//! No wall clock decides anything here: tickets expire on `Instant`, eviction
//! order is a persisted counter; `created` is display-only.

use crate::daemon::dashboard::api_v1::auth::{self, ct_eq};
use crate::daemon::dashboard::ApiState;
use crate::daemon::services::ServiceId;
use axum::http::{header, HeaderMap};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// The ticket query parameter on the panel origin.
pub const TICKET_PARAM: &str = "p111_ticket";
/// Ticket entropy (128 bits, §13).
pub const TICKET_BYTES: usize = 16;
/// Ticket lifetime (§13: 30 s).
pub const TICKET_TTL: Duration = Duration::from_secs(30);
/// Outstanding tickets kept at most (expired ones are purged first).
pub const MAX_TICKETS: usize = 256;
/// Cookie name prefix (`p111_<id>`).
pub const COOKIE_PREFIX: &str = "p111_";
/// Cookie entropy (256 bits).
pub const COOKIE_BYTES: usize = 32;
/// Panel sessions kept at most (the lowest `seq` is dropped beyond this).
pub const MAX_SESSIONS: usize = 128;
/// `state/panel-sessions.json`.
pub const SESSIONS_FILE: &str = "panel-sessions.json";
/// Largest sessions file read (checked before parsing).
const MAX_SESSIONS_FILE: u64 = 1024 * 1024;

const HEX: &[u8; 16] = b"0123456789abcdef";

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(HEX[usize::from(b >> 4)]));
        s.push(char::from(HEX[usize::from(b & 0x0f)]));
    }
    s
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The OS random source failed.
#[derive(Debug)]
pub struct RandomError(getrandom::Error);

impl fmt::Display for RandomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OS randomness unavailable: {}", self.0)
    }
}

impl std::error::Error for RandomError {}

fn random_hex<const N: usize>() -> Result<String, RandomError> {
    let mut raw = zeroize::Zeroizing::new([0u8; N]);
    getrandom::getrandom(raw.as_mut_slice()).map_err(RandomError)?;
    Ok(hex(raw.as_slice()))
}

fn sha256_hex(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

/// sha256 (hex) of a dashboard `s111` token, exactly as the auth store keys
/// its sessions.
pub fn session_hash(token: &str) -> String {
    sha256_hex(&token.to_ascii_lowercase())
}

/// The pair-code generation a trusted-loopback mint is bound to (a rotation
/// changes the code). Domain-separated hash; as sensitive as the pair file
/// itself and kept in the same 0600 class.
pub fn pair_epoch(code: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"111 panel epoch v1\0");
    h.update(code.as_bytes());
    hex(&h.finalize())
}

/// `p111_<id>`.
pub fn cookie_name(panel: ServiceId) -> String {
    format!("{COOKIE_PREFIX}{}", panel.as_str())
}

/// Every `p111_<id>` cookie value in `headers`, in order (exact name, every
/// `Cookie` header, every copy — D16 f).
pub fn panel_cookies(headers: &HeaderMap, panel: ServiceId) -> Vec<String> {
    let name = cookie_name(panel);
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .filter(|(k, _)| *k == name)
        .map(|(_, v)| v.trim().to_string())
        .collect()
}

/// The `Set-Cookie` value for a fresh panel session (session cookie: no
/// `Max-Age`/`Expires`; host-only: no `Domain`).
pub fn set_cookie(panel: ServiceId, token: &str) -> String {
    format!(
        "{}={token}; HttpOnly; SameSite=Strict; Path=/",
        cookie_name(panel)
    )
}

/// The ticket parameter of a raw query: `None` when absent, `Some(value)`
/// (possibly empty / malformed) when present (the first occurrence).
pub fn ticket_param(query: Option<&str>) -> Option<String> {
    query?.split('&').find_map(|kv| match kv.split_once('=') {
        Some((k, v)) if k == TICKET_PARAM => Some(v.to_string()),
        None if kv == TICKET_PARAM => Some(String::new()),
        _ => None,
    })
}

/// The peer a ticket/cookie is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerClass {
    /// Any loopback address (incl. `::ffff:127.0.0.1`).
    Loopback,
    /// Exactly this address.
    Remote(IpAddr),
}

impl PeerClass {
    pub fn of(ip: IpAddr) -> Self {
        let ip = ip.to_canonical();
        if ip.is_loopback() {
            PeerClass::Loopback
        } else {
            PeerClass::Remote(ip)
        }
    }

    fn label(&self) -> String {
        match self {
            PeerClass::Loopback => "loopback".into(),
            PeerClass::Remote(ip) => ip.to_string(),
        }
    }

    fn parse(s: &str) -> Option<Self> {
        if s == "loopback" {
            return Some(PeerClass::Loopback);
        }
        let ip: IpAddr = s.parse().ok()?;
        (!ip.to_canonical().is_loopback()).then_some(PeerClass::Remote(ip.to_canonical()))
    }
}

/// What a ticket and its cookie are bound to (§13: the minting session).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Binding {
    /// Minted under a dashboard `s111` session ([`session_hash`] of its token).
    Session { hash: String },
    /// Minted on trusted loopback (implicit session) during this pair-code
    /// generation ([`pair_epoch`]).
    Loopback { epoch: String },
}

impl Binding {
    fn valid_shape(&self) -> bool {
        match self {
            Binding::Session { hash } => is_hex(hash, 64),
            Binding::Loopback { epoch } => is_hex(epoch, 64),
        }
    }
}

/// Is the minting session still alive? (`111 dashboard rotate` empties the
/// session file and changes the pair code; logout / forgetting a companion
/// removes one session.)
pub fn binding_alive(api: &ApiState, b: &Binding) -> bool {
    match b {
        Binding::Session { hash } => api
            .auth
            .sessions()
            .iter()
            .any(|s| ct_eq(s.hash.as_bytes(), hash.as_bytes())),
        Binding::Loopback { epoch } => {
            api.auth.loopback_trusted()
                && ct_eq(
                    epoch.as_bytes(),
                    pair_epoch(&api.auth.pair_code()).as_bytes(),
                )
        }
    }
}

/// The binding for a mint from this request: trusted loopback ⇒ the implicit
/// session; else the first valid `s111` cookie; else `None` (refuse).
pub fn mint_binding(
    api: &ApiState,
    ext: &axum::http::Extensions,
    headers: &HeaderMap,
) -> Option<Binding> {
    if auth::trusted_loopback(api, ext) {
        return Some(Binding::Loopback {
            epoch: pair_epoch(&api.auth.pair_code()),
        });
    }
    api.auth
        .session_in(headers)
        .map(|(token, _)| Binding::Session {
            hash: session_hash(&token),
        })
}

// ---------------------------------------------------------------- tickets

/// One outstanding ticket.
#[derive(Debug, Clone)]
pub struct TicketRec {
    pub panel: ServiceId,
    pub peer: PeerClass,
    pub binding: Binding,
    expires: Instant,
}

impl TicketRec {
    pub fn expired(&self) -> bool {
        Instant::now() >= self.expires
    }
}

/// Outstanding tickets (memory only; a daemon restart drops them).
pub struct Tickets {
    ttl: Duration,
    map: Mutex<HashMap<String, TicketRec>>,
}

impl Tickets {
    pub fn new(ttl: Duration) -> Self {
        Tickets {
            ttl,
            map: Mutex::new(HashMap::new()),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, TicketRec>> {
        self.map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Mint a ticket (hex, 32 chars). Only its sha256 is kept.
    pub fn mint(
        &self,
        panel: ServiceId,
        peer: PeerClass,
        binding: Binding,
    ) -> Result<String, RandomError> {
        let ticket = random_hex::<TICKET_BYTES>()?;
        let now = Instant::now();
        let mut map = self.lock();
        map.retain(|_, t| t.expires > now);
        while map.len() >= MAX_TICKETS {
            let Some(oldest) = map
                .iter()
                .min_by_key(|(_, t)| t.expires)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            map.remove(&oldest);
        }
        map.insert(
            sha256_hex(&ticket),
            TicketRec {
                panel,
                peer,
                binding,
                expires: now + self.ttl,
            },
        );
        Ok(ticket)
    }

    /// Burn `ticket` (any presentation spends it) and return what it was,
    /// expired or not. Malformed values never match.
    pub fn take(&self, ticket: &str) -> Option<TicketRec> {
        if !is_hex(ticket, TICKET_BYTES * 2) {
            return None;
        }
        self.lock().remove(&sha256_hex(ticket))
    }

    /// Drop every ticket of `panel` (panel disable).
    pub fn drop_panel(&self, panel: ServiceId) {
        self.lock().retain(|_, t| t.panel != panel);
    }

    /// Drop tickets whose minting session is gone.
    pub fn retain_bound(&self, alive: impl Fn(&Binding) -> bool) {
        self.lock().retain(|_, t| alive(&t.binding));
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------- sessions

/// One panel session as recorded in `state/panel-sessions.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelSessionRec {
    /// Opaque per-session key handed to shims (`ShimSession.session`).
    pub id: String,
    /// sha256 (hex) of the cookie value.
    pub hash: String,
    /// Panel id.
    pub panel: String,
    /// `loopback` or the peer address.
    pub peer: String,
    pub binding: Binding,
    /// Minted while LAN sharing was on.
    pub lan: bool,
    /// Eviction order (a counter, never a clock).
    pub seq: u64,
    /// RFC 3339 UTC, display only.
    pub created: String,
}

impl PanelSessionRec {
    fn valid_shape(&self) -> bool {
        is_hex(&self.id, 32)
            && is_hex(&self.hash, 64)
            && ServiceId::parse(&self.panel).is_some()
            && PeerClass::parse(&self.peer).is_some()
            && self.binding.valid_shape()
    }

    pub fn peer_class(&self) -> Option<PeerClass> {
        PeerClass::parse(&self.peer)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionsFile {
    schema: u32,
    #[serde(default)]
    next_seq: u64,
    #[serde(default)]
    sessions: Vec<PanelSessionRec>,
}

/// A session revoked (broadcast to shims: drop the upstream state keyed by
/// `session`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revoked {
    pub panel: String,
    pub session: String,
}

/// A freshly created panel session.
pub struct NewSession {
    /// The cookie value (only its hash is stored).
    pub token: zeroize::Zeroizing<String>,
    pub rec: PanelSessionRec,
}

struct Inner {
    recs: Vec<PanelSessionRec>,
    next_seq: u64,
    warned: bool,
}

/// The panel-session table (file-backed in the daemon, memory-only when the
/// path is `None`).
pub struct SessionStore {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
    revoked: broadcast::Sender<Revoked>,
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

impl SessionStore {
    /// Load the table. `lan_now` is this lifetime's LAN sharing: when it is
    /// off, every session minted while it was on (or from a non-loopback
    /// peer) is revoked (§13 "LAN-off"). An unreadable or garbled file starts
    /// empty (fail closed: every cookie is then unknown).
    pub fn open(path: Option<PathBuf>, lan_now: bool) -> Self {
        let mut recs = Vec::new();
        let mut next_seq = 0;
        if let Some(p) = &path {
            match read_file(p) {
                Ok(Some(f)) => {
                    next_seq = f.next_seq;
                    recs = f.sessions.into_iter().filter(|r| r.valid_shape()).collect();
                }
                Ok(None) => {}
                Err(e) => eprintln!(
                    "111 daemon: panels: cannot read {} ({e}); starting with no panel sessions",
                    p.display()
                ),
            }
        }
        let (revoked, _) = broadcast::channel(64);
        let store = SessionStore {
            path,
            inner: Mutex::new(Inner {
                next_seq: next_seq.max(recs.iter().map(|r| r.seq + 1).max().unwrap_or(0)),
                recs,
                warned: false,
            }),
            revoked,
        };
        if !lan_now {
            store.revoke(|r| r.lan || r.peer != "loopback");
        }
        store
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn persist(&self, inner: &mut Inner) {
        let Some(p) = &self.path else { return };
        let file = SessionsFile {
            schema: 1,
            next_seq: inner.next_seq,
            sessions: inner.recs.clone(),
        };
        let res = serde_json::to_vec_pretty(&file)
            .map_err(anyhow::Error::from)
            .and_then(|body| crate::util::fs::atomic_write(p, &body, 0o600));
        match res {
            Ok(()) => inner.warned = false,
            Err(e) if !inner.warned => {
                eprintln!(
                    "111 daemon: panels: cannot persist {}: {e:#} (continuing in memory)",
                    p.display()
                );
                inner.warned = true;
            }
            Err(_) => {}
        }
    }

    /// Create a session for `panel` and persist it.
    pub fn create(
        &self,
        panel: ServiceId,
        peer: PeerClass,
        binding: Binding,
        lan: bool,
    ) -> Result<NewSession, RandomError> {
        let token = zeroize::Zeroizing::new(random_hex::<COOKIE_BYTES>()?);
        let id = random_hex::<16>()?;
        let mut inner = self.lock();
        let rec = PanelSessionRec {
            id,
            hash: sha256_hex(&token),
            panel: panel.as_str().to_string(),
            peer: peer.label(),
            binding,
            lan,
            seq: inner.next_seq,
            created: now_rfc3339(),
        };
        inner.next_seq += 1;
        inner.recs.push(rec.clone());
        let mut evicted = Vec::new();
        while inner.recs.len() > MAX_SESSIONS {
            let (idx, _) = inner
                .recs
                .iter()
                .enumerate()
                .min_by_key(|(_, r)| r.seq)
                .expect("non-empty");
            evicted.push(inner.recs.remove(idx));
        }
        self.persist(&mut inner);
        drop(inner);
        for r in evicted {
            let _ = self.revoked.send(Revoked {
                panel: r.panel,
                session: r.id,
            });
        }
        Ok(NewSession { token, rec })
    }

    /// The session of `panel` behind any of `cookies` (every copy tried),
    /// for `peer`. The binding is the caller's to check ([`binding_alive`]).
    pub fn find(
        &self,
        panel: ServiceId,
        cookies: &[String],
        peer: PeerClass,
    ) -> Option<PanelSessionRec> {
        let inner = self.lock();
        cookies.iter().find_map(|c| {
            if !is_hex(&c.to_ascii_lowercase(), COOKIE_BYTES * 2) {
                return None;
            }
            let want = sha256_hex(&c.to_ascii_lowercase());
            inner
                .recs
                .iter()
                .find(|r| {
                    ct_eq(r.hash.as_bytes(), want.as_bytes())
                        && r.panel == panel.as_str()
                        && r.peer_class() == Some(peer)
                })
                .cloned()
        })
    }

    /// Revoke every session matching `pred`; persists and broadcasts.
    /// Returns how many were revoked.
    pub fn revoke(&self, pred: impl Fn(&PanelSessionRec) -> bool) -> usize {
        let mut inner = self.lock();
        let (gone, kept): (Vec<_>, Vec<_>) = inner.recs.drain(..).partition(|r| pred(r));
        inner.recs = kept;
        if !gone.is_empty() {
            self.persist(&mut inner);
        }
        drop(inner);
        for r in &gone {
            let _ = self.revoked.send(Revoked {
                panel: r.panel.clone(),
                session: r.id.clone(),
            });
        }
        gone.len()
    }

    /// Revocations as they happen (shims drop per-session upstream state).
    pub fn subscribe(&self) -> broadcast::Receiver<Revoked> {
        self.revoked.subscribe()
    }

    /// Is the session `id` of `panel` still recorded?
    pub fn is_live(&self, panel: &str, id: &str) -> bool {
        self.lock()
            .recs
            .iter()
            .any(|r| r.panel == panel && r.id == id)
    }

    /// Every recorded session (tests, diagnostics).
    pub fn snapshot(&self) -> Vec<PanelSessionRec> {
        self.lock().recs.clone()
    }
}

fn read_file(path: &std::path::Path) -> std::io::Result<Option<SessionsFile>> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !meta.is_file() || meta.len() > MAX_SESSIONS_FILE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not a regular file of at most 1 MiB",
        ));
    }
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn loopback() -> Binding {
        Binding::Loopback {
            epoch: pair_epoch("ABCD-EFGH"),
        }
    }

    #[test]
    fn tickets_are_single_use_and_expire() {
        let t = Tickets::new(Duration::from_millis(50));
        let a = t
            .mint(ServiceId::Meshtastic, PeerClass::Loopback, loopback())
            .unwrap();
        assert!(is_hex(&a, 32), "{a}");
        let rec = t.take(&a).expect("first presentation");
        assert_eq!(rec.panel, ServiceId::Meshtastic);
        assert!(!rec.expired());
        assert!(t.take(&a).is_none(), "single use");
        let b = t
            .mint(ServiceId::Meshtastic, PeerClass::Loopback, loopback())
            .unwrap();
        std::thread::sleep(Duration::from_millis(80));
        assert!(t.take(&b).expect("still burnable").expired());
        assert!(t.take("zz").is_none() && t.take(&"A".repeat(32)).is_none());
        // capacity bound
        for _ in 0..(MAX_TICKETS + 10) {
            t.mint(ServiceId::Meshtastic, PeerClass::Loopback, loopback())
                .unwrap();
        }
        assert!(t.len() <= MAX_TICKETS);
        t.drop_panel(ServiceId::Meshtastic);
        assert!(t.is_empty());
    }

    #[test]
    fn cookies_every_copy_exact_name() {
        let mut h = HeaderMap::new();
        h.append(
            header::COOKIE,
            HeaderValue::from_static("p111_meshtastic=a; p111_meshtasticx=b; P111_meshtastic=c"),
        );
        h.append(
            header::COOKIE,
            HeaderValue::from_static("p111_meshtastic=d"),
        );
        assert_eq!(panel_cookies(&h, ServiceId::Meshtastic), ["a", "d"]);
        assert_eq!(
            set_cookie(ServiceId::Meshtastic, "t"),
            "p111_meshtastic=t; HttpOnly; SameSite=Strict; Path=/"
        );
        assert_eq!(
            ticket_param(Some("a=1&p111_ticket=xy")).as_deref(),
            Some("xy")
        );
        assert_eq!(ticket_param(Some("p111_ticket")).as_deref(), Some(""));
        assert_eq!(ticket_param(Some("p111_tickets=1")), None);
        assert_eq!(ticket_param(None), None);
    }

    #[test]
    fn peer_classes() {
        for ip in ["127.0.0.1", "::1", "::ffff:127.0.0.1", "127.0.0.9"] {
            assert_eq!(
                PeerClass::of(ip.parse().unwrap()),
                PeerClass::Loopback,
                "{ip}"
            );
        }
        let lan = PeerClass::of("::ffff:192.168.1.9".parse().unwrap());
        assert_eq!(lan, PeerClass::Remote("192.168.1.9".parse().unwrap()));
        assert_eq!(PeerClass::parse(&lan.label()), Some(lan));
        assert_eq!(PeerClass::parse("127.0.0.1"), None, "loopback is a class");
    }

    #[test]
    fn store_persists_finds_and_revokes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SESSIONS_FILE);
        let store = SessionStore::open(Some(path.clone()), true);
        let lan_peer = PeerClass::of("192.168.1.9".parse().unwrap());
        let a = store
            .create(
                ServiceId::Meshtastic,
                PeerClass::Loopback,
                loopback(),
                false,
            )
            .unwrap();
        let b = store
            .create(
                ServiceId::Meshtastic,
                lan_peer,
                Binding::Session {
                    hash: session_hash(&"ab".repeat(32)),
                },
                true,
            )
            .unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains(a.token.as_str()),
            "cookie values never stored"
        );
        // exact panel + peer; every copy tried
        let cookies = vec!["junk".to_string(), a.token.to_string()];
        let got = store
            .find(ServiceId::Meshtastic, &cookies, PeerClass::Loopback)
            .unwrap();
        assert_eq!(got.id, a.rec.id);
        assert!(store
            .find(ServiceId::Fabric, &cookies, PeerClass::Loopback)
            .is_none());
        assert!(store
            .find(ServiceId::Meshtastic, &cookies, lan_peer)
            .is_none());
        // reload keeps both; LAN-off drops the LAN one
        drop(store);
        let again = SessionStore::open(Some(path.clone()), true);
        assert_eq!(again.snapshot().len(), 2);
        let mut rx = again.subscribe();
        drop(again);
        let off = SessionStore::open(Some(path.clone()), false);
        let left = off.snapshot();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, a.rec.id);
        assert!(!off.is_live("meshtastic", &b.rec.id));
        // revocation is broadcast
        let mut rx2 = off.subscribe();
        assert_eq!(off.revoke(|r| r.panel == "meshtastic"), 1);
        assert_eq!(
            rx2.try_recv().unwrap(),
            Revoked {
                panel: "meshtastic".into(),
                session: a.rec.id.clone()
            }
        );
        assert!(rx.try_recv().is_err());
        // a garbled file starts empty (fail closed)
        std::fs::write(&path, b"{nope").unwrap();
        assert!(SessionStore::open(Some(path), true).snapshot().is_empty());
    }

    #[test]
    fn store_is_bounded() {
        let store = SessionStore::open(None, true);
        let mut rx = store.subscribe();
        let first = store
            .create(
                ServiceId::Meshtastic,
                PeerClass::Loopback,
                loopback(),
                false,
            )
            .unwrap();
        for _ in 0..MAX_SESSIONS {
            store
                .create(
                    ServiceId::Meshtastic,
                    PeerClass::Loopback,
                    loopback(),
                    false,
                )
                .unwrap();
        }
        assert_eq!(store.snapshot().len(), MAX_SESSIONS);
        assert!(!store.is_live("meshtastic", &first.rec.id));
        assert_eq!(rx.try_recv().unwrap().session, first.rec.id);
    }
}
