//! Dashboard pairing auth (spec §11, D15 "token auth now").
//!
//! - **Loopback = trusted.** A request whose peer address is loopback (incl.
//!   `::ffff:127.0.0.1`) bypasses auth.
//! - **Everyone else needs a session**: cookie `s111=<64 hex>` whose sha256 is in
//!   `<root>/state/dashboard-sessions.json` (`{schema, sessions:[{hash, created,
//!   last_seen, peer}]}`), obtained by `POST /api/v1/auth/pair {code}` with the
//!   8-char pair code from `<root>/state/dashboard-pair.json` (`{schema, code,
//!   created}`, generated at first daemon start, printed by `111 dashboard`).
//! - **Rate limit:** 5 failed pairs per peer IP per minute → `429 rate_limited`;
//!   20 failures in total → the code rotates (logged).
//! - **CSRF** ([`csrf_guard`]): mutating requests must be `application/json` and,
//!   when `Origin` is present, its authority must equal `Host`.
//!
//! Both files are re-read when they change on disk (inode/mtime/len stamp), so
//! `111 dashboard rotate` takes effect on a running daemon without a restart.
//!
//! Test hook: peers are read from `ConnectInfo<SocketAddr>` (production, set by
//! `into_make_service_with_connect_info`) or, when absent, axum's
//! `MockConnectInfo<SocketAddr>` — tests wrap the router in
//! `.layer(MockConnectInfo(addr))` to simulate a LAN peer over a loopback socket.
//! Nothing in production inserts `MockConnectInfo`, and `ConnectInfo` wins.

use super::{ApiRouter, ApiState, Problem};
use crate::util::fs::atomic_write;
use axum::extract::connect_info::{ConnectInfo, MockConnectInfo};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{header, Extensions, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const PAIR_FILE: &str = "dashboard-pair.json";
pub const SESSIONS_FILE: &str = "dashboard-sessions.json";
pub const COOKIE: &str = "s111";
/// Cookie + server-side session lifetime (180 days).
pub const SESSION_MAX_AGE_SECS: u64 = 15_552_000;
/// Failed pairs per peer IP inside [`FAIL_WINDOW`] before `429`.
pub const FAILS_PER_PEER: usize = 5;
pub const FAIL_WINDOW: Duration = Duration::from_secs(60);
/// Total failures (since the last rotation) that rotate the code.
pub const FAILS_BEFORE_ROTATE: u32 = 20;
/// Sessions kept (least recently seen dropped beyond this).
pub const MAX_SESSIONS: usize = 64;
/// `last_seen` is persisted at most this often (in memory it is always fresh).
const LAST_SEEN_PERSIST: Duration = Duration::from_secs(600);

/// Crockford base32 minus every vowel (Crockford already drops I L O U; A and E
/// go too so no code spells a word): 30 symbols, 8 chars ≈ 39 bits.
pub const CODE_ALPHABET: &[u8; 30] = b"0123456789BCDFGHJKMNPQRSTVWXYZ";

// ---------------------------------------------------------------- peer helpers

/// The request's peer address: `ConnectInfo` (production) else `MockConnectInfo`
/// (tests). `None` only if neither is present — callers treat that as remote.
pub fn peer_addr(ext: &Extensions) -> Option<SocketAddr> {
    ext.get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0)
        .or_else(|| ext.get::<MockConnectInfo<SocketAddr>>().map(|m| m.0))
}

/// Loopback peer (fails closed: unknown peer = not loopback).
pub fn peer_is_loopback(ext: &Extensions) -> bool {
    peer_addr(ext).is_some_and(|a| a.ip().to_canonical().is_loopback())
}

/// Is this request from a loopback peer (R2 uses it for loopback-only fields).
pub fn is_loopback(req: &Request) -> bool {
    peer_is_loopback(req.extensions())
}

/// Extractor for loopback-only routes: rejects with `403 loopback_only`.
pub struct LoopbackOnly;

impl<S: Send + Sync> FromRequestParts<S> for LoopbackOnly {
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if peer_is_loopback(&parts.extensions) {
            Ok(LoopbackOnly)
        } else {
            Err(loopback_only())
        }
    }
}

/// The `403 loopback_only` problem (for per-field checks in R2's settings PATCH).
pub fn loopback_only() -> Problem {
    Problem::new(
        StatusCode::FORBIDDEN,
        "loopback_only",
        "available on the node itself",
    )
    .detail("This setting can only be changed from a browser on the node (loopback).")
}

// ---------------------------------------------------------------- pair codes

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).expect("OS randomness unavailable");
    b
}

/// A fresh code in display form `XXXX-XXXX` (rejection-sampled, unbiased).
pub fn generate_code() -> String {
    let mut out = Vec::with_capacity(9);
    while out.len() < 9 {
        for b in random_bytes::<16>() {
            if out.len() == 4 {
                out.push(b'-');
            }
            if out.len() == 9 {
                break;
            }
            // 240 = 8 × 30: bytes ≥ 240 would bias the low symbols.
            if b < 240 {
                out.push(CODE_ALPHABET[(b % 30) as usize]);
            }
        }
    }
    String::from_utf8(out).expect("ascii")
}

/// Normalize user input: drop dashes/spaces, uppercase, Crockford read-alikes
/// (O→0, I/L→1). `None` unless exactly 8 alphabet symbols remain.
pub fn normalize_code(input: &str) -> Option<[u8; 8]> {
    let mut out = [0u8; 8];
    let mut n = 0;
    for c in input.chars() {
        let c = match c.to_ascii_uppercase() {
            '-' | ' ' | '\t' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !c.is_ascii() || !CODE_ALPHABET.contains(&(c as u8)) || n == 8 {
            return None;
        }
        out[n] = c as u8;
        n += 1;
    }
    (n == 8).then_some(out)
}

/// Constant-time byte comparison (length is not secret).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

fn parse_rfc3339(s: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
}

// ---------------------------------------------------------------- on-disk files

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairFile {
    pub schema: u32,
    /// Display form `XXXX-XXXX`.
    pub code: String,
    pub created: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRec {
    /// sha256 (hex) of the cookie value (the 64-hex token string).
    pub hash: String,
    pub created: String,
    pub last_seen: String,
    pub peer: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionsFile {
    pub schema: u32,
    #[serde(default)]
    pub sessions: Vec<SessionRec>,
}

/// Identity of a file version (atomic_write replaces the inode on every write).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    ino: u64,
    mtime_ns: i128,
    len: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some(Stamp {
        ino: m.ino(),
        mtime_ns: m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128,
        len: m.len(),
    })
}

fn new_pair_file() -> PairFile {
    PairFile {
        schema: 1,
        code: generate_code(),
        created: now_rfc3339(),
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> std::io::Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let body = serde_json::to_vec_pretty(value)?;
    atomic_write(path, &body, 0o600)
}

/// `111 dashboard`: the current pair code (`Ok(None)` = not generated yet; an
/// `Err` is usually permission denied — the file is node111-owned 0600).
pub fn read_pair_code(state_dir: &Path) -> std::io::Result<Option<String>> {
    let pf: Option<PairFile> = read_json(&state_dir.join(PAIR_FILE))?;
    Ok(pf
        .filter(|p| normalize_code(&p.code).is_some())
        .map(|p| p.code))
}

/// `111 dashboard rotate`: new code + revoke every session. Run as root, the
/// files are handed to `node111` so the daemon can keep using them.
pub fn rotate_files(state_dir: &Path) -> anyhow::Result<String> {
    let pf = new_pair_file();
    let pair = state_dir.join(PAIR_FILE);
    let sessions = state_dir.join(SESSIONS_FILE);
    write_json(&pair, &pf)?;
    write_json(
        &sessions,
        &SessionsFile {
            schema: 1,
            sessions: Vec::new(),
        },
    )?;
    if unsafe { libc::geteuid() } == 0 {
        for p in [&pair, &sessions] {
            if let Err(e) = chown_to_node_user(p) {
                eprintln!("111 dashboard: chown node111 {}: {e:#}", p.display());
            }
        }
    }
    Ok(pf.code)
}

/// chown `node111:node111` (uid/gid looked up via getpwnam).
fn chown_to_node_user(path: &Path) -> anyhow::Result<()> {
    let name = std::ffi::CString::new("node111")?;
    let pw = unsafe { libc::getpwnam(name.as_ptr()) };
    anyhow::ensure!(!pw.is_null(), "user node111 not found");
    let (uid, gid) = unsafe { ((*pw).pw_uid, (*pw).pw_gid) };
    std::os::unix::fs::chown(path, Some(uid), Some(gid))?;
    Ok(())
}

// ---------------------------------------------------------------- the store

/// Result of a successful pairing.
#[derive(Debug, Clone)]
pub struct NewSession {
    /// Cookie value (64 lowercase hex). Only its sha256 is stored.
    pub token: String,
    pub created: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairError {
    /// Too many failures from this peer; retry after this many seconds.
    RateLimited {
        retry_after: u64,
    },
    Wrong,
}

struct Inner {
    pair: PairFile,
    pair_stamp: Option<Stamp>,
    sessions: Vec<SessionRec>,
    sessions_stamp: Option<Stamp>,
    failures: HashMap<IpAddr, VecDeque<Instant>>,
    total_failures: u32,
    last_persist: Option<Instant>,
    warned_write: bool,
}

/// Pair code + sessions + rate limiter. File-backed under `<root>/state/` in the
/// daemon; memory-only when `dir` is `None`.
pub struct AuthStore {
    dir: Option<PathBuf>,
    inner: Mutex<Inner>,
}

impl AuthStore {
    /// Load (or create) the pair code and sessions. Never fails: an unwritable
    /// state dir is logged and the store keeps working from memory.
    pub fn open(dir: Option<PathBuf>) -> Self {
        let store = AuthStore {
            dir,
            inner: Mutex::new(Inner {
                pair: new_pair_file(),
                pair_stamp: None,
                sessions: Vec::new(),
                sessions_stamp: None,
                failures: HashMap::new(),
                total_failures: 0,
                last_persist: None,
                warned_write: false,
            }),
        };
        {
            let mut inner = store.inner.lock().unwrap();
            store.refresh(&mut inner);
            if let Some(dir) = &store.dir {
                // First start (or an unreadable/corrupt file): persist a code.
                let valid = read_json::<PairFile>(&dir.join(PAIR_FILE))
                    .ok()
                    .flatten()
                    .is_some_and(|p| normalize_code(&p.code).is_some());
                if !valid {
                    let pf = new_pair_file();
                    inner.pair = pf.clone();
                    store.persist_pair(&mut inner);
                }
            }
        }
        store
    }

    fn pair_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(PAIR_FILE))
    }

    fn sessions_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(SESSIONS_FILE))
    }

    /// Re-read whichever file changed on disk since we last saw it.
    fn refresh(&self, inner: &mut Inner) {
        if let Some(p) = self.pair_path() {
            let st = stamp(&p);
            if st.is_some() && st != inner.pair_stamp {
                if let Ok(Some(pf)) = read_json::<PairFile>(&p) {
                    if normalize_code(&pf.code).is_some() {
                        inner.pair = pf;
                        inner.total_failures = 0;
                    }
                }
                inner.pair_stamp = st;
            }
        }
        if let Some(p) = self.sessions_path() {
            let st = stamp(&p);
            if st != inner.sessions_stamp {
                inner.sessions = match read_json::<SessionsFile>(&p) {
                    Ok(Some(f)) => f.sessions,
                    _ => Vec::new(),
                };
                inner.sessions_stamp = st;
            }
        }
    }

    fn warn_write(inner: &mut Inner, what: &str, e: &anyhow::Error) {
        if !inner.warned_write {
            eprintln!(
                "111 daemon: dashboard auth: cannot persist {what}: {e:#} (continuing in memory)"
            );
            inner.warned_write = true;
        }
    }

    fn persist_pair(&self, inner: &mut Inner) {
        if let Some(p) = self.pair_path() {
            match write_json(&p, &inner.pair) {
                Ok(()) => inner.pair_stamp = stamp(&p),
                Err(e) => Self::warn_write(inner, "pair code", &e),
            }
        }
    }

    fn persist_sessions(&self, inner: &mut Inner) {
        inner.last_persist = Some(Instant::now());
        if let Some(p) = self.sessions_path() {
            let file = SessionsFile {
                schema: 1,
                sessions: inner.sessions.clone(),
            };
            match write_json(&p, &file) {
                Ok(()) => inner.sessions_stamp = stamp(&p),
                Err(e) => Self::warn_write(inner, "sessions", &e),
            }
        }
    }

    /// Current code (display form). For tests and in-process callers; the CLI
    /// reads the file.
    pub fn pair_code(&self) -> String {
        let mut inner = self.inner.lock().unwrap();
        self.refresh(&mut inner);
        inner.pair.code.clone()
    }

    /// Validate a cookie token. Returns the session's `created` timestamp.
    pub fn session_for(&self, token: &str) -> Option<String> {
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let want = sha256_hex(token.to_ascii_lowercase().as_bytes());
        let mut inner = self.inner.lock().unwrap();
        self.refresh(&mut inner);
        let now = time::OffsetDateTime::now_utc();
        let idx = inner
            .sessions
            .iter()
            .position(|s| ct_eq(s.hash.as_bytes(), want.as_bytes()))?;
        let created = inner.sessions[idx].created.clone();
        let expired = parse_rfc3339(&created)
            .is_none_or(|c| (now - c).whole_seconds() > SESSION_MAX_AGE_SECS as i64);
        if expired {
            inner.sessions.remove(idx);
            self.persist_sessions(&mut inner);
            return None;
        }
        inner.sessions[idx].last_seen = now_rfc3339();
        if inner
            .last_persist
            .is_none_or(|t| t.elapsed() >= LAST_SEEN_PERSIST)
        {
            self.persist_sessions(&mut inner);
        }
        Some(created)
    }

    /// Try a pair code from `peer`.
    pub fn pair(&self, peer: IpAddr, code: &str) -> Result<NewSession, PairError> {
        let mut inner = self.inner.lock().unwrap();
        self.refresh(&mut inner);
        let now = Instant::now();
        let fails = inner.failures.entry(peer).or_default();
        while fails
            .front()
            .is_some_and(|t| now.duration_since(*t) >= FAIL_WINDOW)
        {
            fails.pop_front();
        }
        if fails.len() >= FAILS_PER_PEER {
            let oldest = *fails.front().expect("non-empty");
            let wait = FAIL_WINDOW.saturating_sub(now.duration_since(oldest));
            return Err(PairError::RateLimited {
                retry_after: wait.as_secs().max(1),
            });
        }
        let ok = match (normalize_code(code), normalize_code(&inner.pair.code)) {
            (Some(given), Some(want)) => ct_eq(&given, &want),
            _ => false,
        };
        if !ok {
            inner.failures.entry(peer).or_default().push_back(now);
            inner.total_failures += 1;
            if inner.total_failures >= FAILS_BEFORE_ROTATE {
                inner.pair = new_pair_file();
                inner.total_failures = 0;
                self.persist_pair(&mut inner);
                eprintln!(
                    "111 daemon: dashboard pair code rotated after {FAILS_BEFORE_ROTATE} failed attempts (last from {peer}); `111 dashboard` shows the new code"
                );
            }
            return Err(PairError::Wrong);
        }
        inner.failures.remove(&peer);
        let token = hex(&random_bytes::<32>());
        let created = now_rfc3339();
        inner.sessions.push(SessionRec {
            hash: sha256_hex(token.as_bytes()),
            created: created.clone(),
            last_seen: created.clone(),
            peer: peer.to_string(),
        });
        if inner.sessions.len() > MAX_SESSIONS {
            // drop the least recently seen (RFC 3339 UTC strings sort by time)
            let (idx, _) = inner
                .sessions
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.last_seen.cmp(&b.1.last_seen))
                .expect("non-empty");
            inner.sessions.remove(idx);
        }
        self.persist_sessions(&mut inner);
        Ok(NewSession { token, created })
    }

    /// Revoke the session behind `token` (no-op if unknown).
    pub fn logout(&self, token: &str) -> bool {
        let want = sha256_hex(token.to_ascii_lowercase().as_bytes());
        let mut inner = self.inner.lock().unwrap();
        self.refresh(&mut inner);
        let before = inner.sessions.len();
        inner
            .sessions
            .retain(|s| !ct_eq(s.hash.as_bytes(), want.as_bytes()));
        let removed = inner.sessions.len() != before;
        if removed {
            self.persist_sessions(&mut inner);
        }
        removed
    }

    /// New code + revoke all sessions (in-process equivalent of `rotate_files`).
    pub fn rotate_all(&self) -> String {
        let mut inner = self.inner.lock().unwrap();
        inner.pair = new_pair_file();
        inner.total_failures = 0;
        inner.sessions.clear();
        self.persist_pair(&mut inner);
        self.persist_sessions(&mut inner);
        inner.pair.code.clone()
    }
}

// ---------------------------------------------------------------- middleware

/// The `s111` cookie value, if any (all `Cookie` headers are searched).
pub fn session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE)
        .map(|(_, v)| v.trim().to_string())
}

fn unauthenticated() -> Problem {
    Problem::new(
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
        "pairing required",
    )
    .detail("Run `111 dashboard` on the node and enter its pair code.")
}

/// Gate for every `/api/v1/*` route except `/api/v1/auth/*`.
pub async fn require_session(
    State(api): State<Arc<ApiState>>,
    req: Request,
    next: Next,
) -> Response {
    if is_loopback(&req) {
        return next.run(req).await;
    }
    match session_cookie(req.headers()) {
        Some(tok) if api.auth.session_for(&tok).is_some() => next.run(req).await,
        _ => unauthenticated().into_response(),
    }
}

/// `host[:port]` lowercased with the default `:80` dropped.
fn norm_authority(a: &str) -> String {
    let a = a.trim().to_ascii_lowercase();
    match a.strip_suffix(":80") {
        Some(rest) if !rest.is_empty() => rest.to_string(),
        _ => a,
    }
}

/// Does `Origin` name the same authority as `Host`? (`null`, other schemes,
/// paths, or a missing Host all fail.)
pub fn origin_matches_host(origin: &str, host: Option<&str>) -> bool {
    let Some(host) = host else { return false };
    let Some(rest) = origin.trim().strip_prefix("http://") else {
        return false;
    };
    if rest.contains('/') {
        return false;
    }
    norm_authority(rest) == norm_authority(host)
}

/// CSRF defense for mutating `/api/v1` requests (all peers, incl. loopback).
pub async fn csrf_guard(req: Request, next: Next) -> Response {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    let headers = req.headers();
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"));
    if !json {
        return Problem::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "bad_request",
            "mutating requests must be application/json",
        )
        .into_response();
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
        let ok = origin.to_str().is_ok_and(|o| origin_matches_host(o, host));
        if !ok {
            return Problem::new(
                StatusCode::FORBIDDEN,
                "bad_request",
                "cross-origin request refused",
            )
            .into_response();
        }
    }
    next.run(req).await
}

// ---------------------------------------------------------------- routes

#[derive(Debug, Serialize)]
pub struct AuthSession {
    pub schema: u32,
    pub loopback: bool,
    pub paired: bool,
    pub lan_enabled: bool,
    pub created: Option<String>,
}

#[derive(Deserialize)]
struct PairReq {
    code: String,
}

fn lan_effective(api: &ApiState) -> bool {
    api.bind.read().unwrap().lan
}

async fn session(
    State(api): State<Arc<ApiState>>,
    ext: Extensions,
    headers: HeaderMap,
) -> Json<AuthSession> {
    let created = session_cookie(&headers).and_then(|t| api.auth.session_for(&t));
    Json(AuthSession {
        schema: 1,
        loopback: peer_is_loopback(&ext),
        paired: created.is_some(),
        lan_enabled: lan_effective(&api),
        created,
    })
}

async fn pair(
    State(api): State<Arc<ApiState>>,
    ext: Extensions,
    body: axum::body::Bytes,
) -> Response {
    let Ok(req) = serde_json::from_slice::<PairReq>(&body) else {
        return Problem::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "body must be {\"code\": \"xxxx-xxxx\"}",
        )
        .into_response();
    };
    let peer = peer_addr(&ext)
        .map(|a| a.ip().to_canonical())
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    match api.auth.pair(peer, &req.code) {
        Ok(s) => {
            let cookie = format!(
                "{COOKIE}={}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_MAX_AGE_SECS}",
                s.token
            );
            let mut resp = Json(AuthSession {
                schema: 1,
                loopback: peer_is_loopback(&ext),
                paired: true,
                lan_enabled: lan_effective(&api),
                created: Some(s.created),
            })
            .into_response();
            if let Ok(v) = HeaderValue::from_str(&cookie) {
                resp.headers_mut().insert(header::SET_COOKIE, v);
            }
            resp
        }
        Err(PairError::RateLimited { retry_after }) => {
            let mut resp = Problem::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "too many wrong pair codes",
            )
            .detail(format!("Wait {retry_after} s before trying again."))
            .into_response();
            resp.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry_after));
            resp
        }
        Err(PairError::Wrong) => Problem::new(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "wrong pair code",
        )
        .detail("Check the code printed by `111 dashboard` on the node.")
        .into_response(),
    }
}

async fn logout(State(api): State<Arc<ApiState>>, headers: HeaderMap) -> Response {
    if let Some(tok) = session_cookie(&headers) {
        api.auth.logout(&tok);
    }
    (
        StatusCode::NO_CONTENT,
        [(
            header::SET_COOKIE,
            format!("{COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"),
        )],
    )
        .into_response()
}

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/auth/session", get(session))
        .route("/api/v1/auth/pair", post(pair))
        .route("/api/v1/auth/logout", post(logout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_well_formed_and_vowel_free() {
        for _ in 0..200 {
            let c = generate_code();
            assert_eq!(c.len(), 9, "{c}");
            assert_eq!(&c[4..5], "-");
            assert!(normalize_code(&c).is_some(), "{c}");
            assert!(!c.chars().any(|ch| "AEIOUL".contains(ch)), "{c}");
        }
    }

    #[test]
    fn normalization_accepts_human_input() {
        let want = normalize_code("7KQ2-XM9D").unwrap();
        for input in ["7kq2-xm9d", "7KQ2XM9D", " 7kq2 xm9d ", "7-K-Q-2-X-M-9-D"] {
            assert_eq!(normalize_code(input), Some(want), "{input}");
        }
        // Crockford read-alikes
        assert_eq!(normalize_code("o1l0-bcdf"), normalize_code("0110-BCDF"));
        for bad in ["", "7KQ2-XM9", "7KQ2-XM9DD", "7KQ2-XM9A", "7KQ2-XM9É"] {
            assert_eq!(normalize_code(bad), None, "{bad}");
        }
    }

    #[test]
    fn origin_host_matching() {
        assert!(origin_matches_host(
            "http://127.0.0.1:11180",
            Some("127.0.0.1:11180")
        ));
        assert!(origin_matches_host(
            "http://111.localhost",
            Some("111.localhost")
        ));
        assert!(origin_matches_host(
            "http://111.localhost",
            Some("111.localhost:80")
        ));
        assert!(origin_matches_host("http://111.LOCAL", Some("111.local")));
        assert!(!origin_matches_host(
            "http://evil.com",
            Some("127.0.0.1:11180")
        ));
        assert!(!origin_matches_host("null", Some("127.0.0.1")));
        assert!(!origin_matches_host("https://127.0.0.1", Some("127.0.0.1")));
        assert!(!origin_matches_host(
            "http://127.0.0.1:11180",
            Some("127.0.0.1")
        ));
        assert!(!origin_matches_host("http://127.0.0.1", None));
    }

    #[test]
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        h.append(
            header::COOKIE,
            HeaderValue::from_static("a=1; s111=abc ; b=2"),
        );
        assert_eq!(session_cookie(&h).as_deref(), Some("abc"));
        let mut h = HeaderMap::new();
        h.append(header::COOKIE, HeaderValue::from_static("x111=no"));
        assert_eq!(session_cookie(&h), None);
    }

    #[test]
    fn memory_store_pair_logout_and_rate_limit() {
        let store = AuthStore::open(None);
        let peer: IpAddr = "192.168.1.9".parse().unwrap();
        let code = store.pair_code();
        let s = store.pair(peer, &code.to_lowercase()).unwrap();
        assert_eq!(s.token.len(), 64);
        assert!(store.session_for(&s.token).is_some());
        assert!(store.session_for(&"0".repeat(64)).is_none());
        assert!(store.logout(&s.token));
        assert!(store.session_for(&s.token).is_none());
        for _ in 0..FAILS_PER_PEER {
            assert!(matches!(
                store.pair(peer, "0000-0000"),
                Err(PairError::Wrong)
            ));
        }
        assert!(matches!(
            store.pair(peer, &code),
            Err(PairError::RateLimited { .. })
        ));
    }

    #[test]
    fn file_store_survives_reopen_and_sees_external_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let store = AuthStore::open(Some(dir.path().to_path_buf()));
        let code = read_pair_code(dir.path()).unwrap().unwrap();
        assert_eq!(code, store.pair_code());
        let s = store.pair("10.0.0.2".parse().unwrap(), &code).unwrap();
        // the token itself is never written, only its hash
        let raw = std::fs::read_to_string(dir.path().join(SESSIONS_FILE)).unwrap();
        assert!(!raw.contains(&s.token));
        assert!(raw.contains(&sha256_hex(s.token.as_bytes())));
        // reopen: same code, session still valid
        let again = AuthStore::open(Some(dir.path().to_path_buf()));
        assert_eq!(again.pair_code(), code);
        assert!(again.session_for(&s.token).is_some());
        // CLI rotation while "running": the store notices
        let new_code = rotate_files(dir.path()).unwrap();
        assert_ne!(new_code, code);
        assert_eq!(store.pair_code(), new_code);
        assert!(store.session_for(&s.token).is_none());
    }
}
