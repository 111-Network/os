//! Dashboard pairing auth (spec §11, D15 "token auth now").
//!
//! - **Loopback = trusted, by policy** ([`loopback_trusted`], read once at
//!   start): a request whose peer address is loopback (incl.
//!   `::ffff:127.0.0.1`) bypasses auth — except under Termux, where Android
//!   gives apps no per-app loopback isolation (any app on the phone could
//!   drive the node through `127.0.0.1:11180`), so loopback peers pair like
//!   LAN peers. `DASHBOARD_TRUST_LOOPBACK=true|false` overrides the detection.
//!   Either way a loopback peer still *is* loopback for the `loopback_only`
//!   field checks, and `/health` and `POST /auth/pair` stay open.
//! - **Everyone else needs a session**: cookie `s111=<64 hex>` whose sha256 is in
//!   `<root>/state/dashboard-sessions.json` (`{schema, sessions:[{hash, created,
//!   last_seen, peer, user_agent?}]}`), obtained by `POST /api/v1/auth/pair {code}` with the
//!   8-char pair code from `<root>/state/dashboard-pair.json` (`{schema, code,
//!   created}`, generated at first daemon start, printed by `111 dashboard`).
//! - **Rate limit:** 5 failed pairs per peer IP per minute → `429 rate_limited`;
//!   20 failures in total → the code rotates (logged). See [`FAILS_BEFORE_ROTATE`]
//!   for the trade-off.
//! - **CSRF** ([`csrf_guard`]): mutating requests must be `application/json` and,
//!   when `Origin` is present, its authority must equal `Host`.
//! - **Daemon bearer token** (Layer 5 §17, D20 e): `111d` mints one
//!   [`DaemonToken`] per agent spawn (`intelligence::start`, exported to
//!   `111-agent` as `AGENT111_DAEMON_TOKEN`) and keeps it in [`ApiState`] ext.
//!   [`require_session`] accepts `Authorization: Bearer <token>` **only from a
//!   loopback peer and only when it equals that token** (constant time). A
//!   bearer from any other peer, a wrong or ambiguous bearer, or a bearer while
//!   no token was minted is `401` — a presented credential is never ignored.
//!   Loopback trust is not reopened by this (D16 a stands under Termux): the
//!   token is what lets the agent in where loopback is untrusted.
//! - **The global write gate** (Layer 6 `6-transport/README.md` §13 "One
//!   global rule", D26 #2): the agent's bearer **reads** — it is refused
//!   (`401`) on **every** method but `GET`, with no allowlist
//!   ([`bearer_write_refused`], checked first in [`require_session`] and on
//!   the open `/auth/*` writes by [`bearer_write_gate`]). Identify, restarts,
//!   proposal decisions, bridge edges, sends and the join descriptor are all
//!   writes; the agent files proposals and the human decides. A presented
//!   bearer is never ignored on a write, even beside a valid session cookie.
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
use crate::util::fs::atomic_write_owned;
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
use std::fmt;
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
///
/// Trade-off (chosen design, kept on purpose): the per-peer limit alone does
/// not bound guessing — a LAN attacker can spread guesses over many source
/// addresses (an IPv6 /64 is effectively unlimited). Rotating after 20 wrong
/// codes from *anyone* caps the guesses any one code ever receives at 20
/// (≈ 20 / 30⁸ success odds). The cost: anyone on the LAN can force a rotation
/// by sending 20 wrong codes, so the owner has to read the new code from
/// `111 dashboard`. Existing sessions survive such a rotation (only
/// `111 dashboard rotate` revokes them), so paired browsers are unaffected.
pub const FAILS_BEFORE_ROTATE: u32 = 20;
/// Peers whose recent failures are remembered at most (memory bound: entries
/// older than [`FAIL_WINDOW`] are pruned on every attempt, and beyond this cap
/// the peer whose last failure is oldest is forgotten first).
pub const MAX_FAIL_PEERS: usize = 1024;
/// Sessions kept (least recently seen dropped beyond this).
pub const MAX_SESSIONS: usize = 64;
/// `last_seen` is persisted at most this often (in memory it is always fresh).
const LAST_SEEN_PERSIST: Duration = Duration::from_secs(600);

/// Overrides the loopback-trust detection: `true` | `false` (anything else is
/// ignored). Read from the daemon environment (`/etc/111/env` is loaded into
/// it at start).
pub const TRUST_LOOPBACK_ENV: &str = "DASHBOARD_TRUST_LOOPBACK";
/// Set by Termux in every shell and service it starts.
pub const TERMUX_VERSION_ENV: &str = "TERMUX_VERSION";
/// Termux's install prefix (exists on a phone node running under Termux).
pub const TERMUX_PREFIX: &str = "/data/data/com.termux/files/usr";

/// The loopback-trust policy from its inputs: the override wins when it is
/// `true` or `false`; otherwise loopback is trusted unless this is Termux
/// (`TERMUX_VERSION` non-empty, or [`TERMUX_PREFIX`] exists).
pub fn loopback_trust_policy(
    override_value: Option<&str>,
    termux_version: Option<&str>,
    termux_prefix_exists: bool,
) -> bool {
    match override_value
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("true") => true,
        Some("false") => false,
        _ => !(termux_version.is_some_and(|v| !v.trim().is_empty()) || termux_prefix_exists),
    }
}

/// This process's loopback-trust policy ([`loopback_trust_policy`] over the
/// real environment). Computed once at daemon start (`ApiConfig::new`).
pub fn loopback_trusted() -> bool {
    loopback_trust_policy(
        std::env::var(TRUST_LOOPBACK_ENV).ok().as_deref(),
        std::env::var(TERMUX_VERSION_ENV).ok().as_deref(),
        Path::new(TERMUX_PREFIX).is_dir(),
    )
}

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

/// Drop failure stamps older than [`FAIL_WINDOW`] and peers left with none.
fn prune_failures(failures: &mut HashMap<IpAddr, VecDeque<Instant>>, now: Instant) {
    failures.retain(|_, q| {
        while q
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= FAIL_WINDOW)
        {
            q.pop_front();
        }
        !q.is_empty()
    });
}

/// Record one failure from `peer`, keeping at most [`MAX_FAIL_PEERS`] peers.
fn record_failure(failures: &mut HashMap<IpAddr, VecDeque<Instant>>, peer: IpAddr, now: Instant) {
    prune_failures(failures, now);
    failures.entry(peer).or_default().push_back(now);
    while failures.len() > MAX_FAIL_PEERS {
        let Some(stalest) = failures
            .iter()
            .filter(|(ip, _)| **ip != peer)
            .min_by_key(|(_, q)| q.back().copied())
            .map(|(ip, _)| *ip)
        else {
            break;
        };
        failures.remove(&stalest);
    }
}

// ---------------------------------------------------------------- on-disk files

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairFile {
    pub schema: u32,
    /// Display form `XXXX-XXXX`.
    pub code: String,
    pub created: String,
    /// Random nonce of this pair-code generation (32 bytes, lowercase hex),
    /// new with every code. What "minted during this pair-code generation"
    /// binds to (Layer 5 §13: trusted-loopback panel sessions die with a
    /// rotation) without deriving anything from the ~39-bit code. Absent in
    /// files written before it existed ([`AuthStore::pair_generation`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRec {
    /// sha256 (hex) of the cookie value (the 64-hex token string).
    pub hash: String,
    pub created: String,
    pub last_seen: String,
    pub peer: String,
    /// Browser `User-Agent` at pair time (sanitized, ≤ 256 chars); `None` for
    /// sessions paired before R4 recorded it. Read by the device registry
    /// (companions); never used for auth decisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
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
        generation: Some(hex(&random_bytes::<32>())),
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
    write_json_owned(path, value, None)
}

/// [`write_json`] with the tmp file `fchown`ed to `owner` before the rename.
fn write_json_owned<T: Serialize>(
    path: &Path,
    value: &T,
    owner: Option<(u32, u32)>,
) -> anyhow::Result<()> {
    let body = serde_json::to_vec_pretty(value)?;
    atomic_write_owned(path, &body, 0o600, owner)
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
/// files are handed to `node111` so the daemon can keep using them: the tmp
/// file is `fchown`ed through its descriptor before the rename (state/ is
/// node111-writable, so a chown by path afterwards could follow a symlink the
/// daemon user planted there and hand root a chown of any file).
pub fn rotate_files(state_dir: &Path) -> anyhow::Result<String> {
    let pf = new_pair_file();
    let pair = state_dir.join(PAIR_FILE);
    let sessions = state_dir.join(SESSIONS_FILE);
    let owner = if unsafe { libc::geteuid() } == 0 {
        match node_user_ids() {
            Ok(ids) => Some(ids),
            Err(e) => {
                eprintln!("111 dashboard: files stay root-owned: {e:#}");
                None
            }
        }
    } else {
        None
    };
    write_json_owned(&pair, &pf, owner)?;
    write_json_owned(
        &sessions,
        &SessionsFile {
            schema: 1,
            sessions: Vec::new(),
        },
        owner,
    )?;
    Ok(pf.code)
}

/// `node111`'s uid/gid (getpwnam).
fn node_user_ids() -> anyhow::Result<(u32, u32)> {
    let name = std::ffi::CString::new("node111")?;
    let pw = unsafe { libc::getpwnam(name.as_ptr()) };
    anyhow::ensure!(!pw.is_null(), "user node111 not found");
    Ok(unsafe { ((*pw).pw_uid, (*pw).pw_gid) })
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
    warned_read: bool,
}

/// Pair code + sessions + rate limiter. File-backed under `<root>/state/` in the
/// daemon; memory-only when `dir` is `None`.
pub struct AuthStore {
    dir: Option<PathBuf>,
    inner: Mutex<Inner>,
    /// Loopback peers bypass the session check (see module docs).
    trust_loopback: bool,
}

impl AuthStore {
    /// Load (or create) the pair code and sessions. Never fails: an unwritable
    /// state dir is logged and the store keeps working from memory.
    pub fn open(dir: Option<PathBuf>) -> Self {
        let store = AuthStore {
            dir,
            trust_loopback: true,
            inner: Mutex::new(Inner {
                pair: new_pair_file(),
                pair_stamp: None,
                sessions: Vec::new(),
                sessions_stamp: None,
                failures: HashMap::new(),
                total_failures: 0,
                last_persist: None,
                warned_write: false,
                warned_read: false,
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

    /// Set the loopback-trust policy (`ApiState::new` passes the value
    /// computed once at start; tests pass `false` to simulate Termux).
    pub fn with_loopback_trust(mut self, trusted: bool) -> Self {
        self.trust_loopback = trusted;
        self
    }

    /// Do loopback peers bypass the session check?
    pub fn loopback_trusted(&self) -> bool {
        self.trust_loopback
    }

    fn pair_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(PAIR_FILE))
    }

    fn sessions_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(SESSIONS_FILE))
    }

    /// Re-read whichever file changed on disk since we last saw it. The stamp
    /// is recorded only when the read itself succeeded: a transient I/O error
    /// must not mark a version as seen (the next call retries it). An absent
    /// or garbled file is a successful read of nothing usable.
    fn refresh(&self, inner: &mut Inner) {
        if let Some(p) = self.pair_path() {
            let st = stamp(&p);
            if st.is_some() && st != inner.pair_stamp {
                match read_json::<PairFile>(&p) {
                    Ok(pf) => {
                        if let Some(mut pf) = pf.filter(|pf| normalize_code(&pf.code).is_some()) {
                            // a file from before the nonce: one for this
                            // lifetime (never written back — that could undo
                            // a concurrent `111 dashboard rotate`)
                            pf.generation
                                .get_or_insert_with(|| hex(&random_bytes::<32>()));
                            inner.pair = pf;
                            inner.total_failures = 0;
                        }
                        inner.pair_stamp = st;
                        inner.warned_read = false;
                    }
                    Err(e) => Self::warn_read(inner, &p, &e),
                }
            }
        }
        if let Some(p) = self.sessions_path() {
            let st = stamp(&p);
            if st != inner.sessions_stamp {
                match read_json::<SessionsFile>(&p) {
                    Ok(f) => {
                        inner.sessions = f.map(|f| f.sessions).unwrap_or_default();
                        inner.sessions_stamp = st;
                        inner.warned_read = false;
                    }
                    Err(e) => {
                        // fail closed for this call (a rotation may have revoked
                        // everything), unstamped so the next call re-reads
                        inner.sessions.clear();
                        Self::warn_read(inner, &p, &e);
                    }
                }
            }
        }
    }

    /// One stderr line per failing stretch (a persistently unreadable file
    /// would otherwise log on every request).
    fn warn_read(inner: &mut Inner, path: &Path, e: &std::io::Error) {
        if !inner.warned_read {
            eprintln!(
                "111 daemon: dashboard auth: cannot read {}: {e} (retrying on every request)",
                path.display()
            );
            inner.warned_read = true;
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

    /// The current pair-code generation's nonce ([`PairFile::generation`]).
    /// Changes with every new code (`111 dashboard rotate`, the failure
    /// rotation); a pair file written before the nonce existed gets one for
    /// this daemon lifetime only (so a restart counts as a new generation
    /// until the next rotation writes one).
    pub fn pair_generation(&self) -> String {
        let mut inner = self.inner.lock().unwrap();
        self.refresh(&mut inner);
        inner
            .pair
            .generation
            .get_or_insert_with(|| hex(&random_bytes::<32>()))
            .clone()
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
        self.pair_with_agent(peer, code, None)
    }

    /// Valid (unexpired) sessions as stored — read-only view for the device
    /// registry (paired browsers are `companion` devices). Does not touch
    /// `last_seen`.
    pub fn sessions(&self) -> Vec<SessionRec> {
        let mut inner = self.inner.lock().unwrap();
        self.refresh(&mut inner);
        let now = time::OffsetDateTime::now_utc();
        inner
            .sessions
            .iter()
            .filter(|s| {
                parse_rfc3339(&s.created)
                    .is_some_and(|c| (now - c).whole_seconds() <= SESSION_MAX_AGE_SECS as i64)
            })
            .cloned()
            .collect()
    }

    /// Revoke the session whose stored hash is `hash` (forgetting a companion
    /// device). Returns whether one was removed.
    pub fn revoke_hash(&self, hash: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        self.refresh(&mut inner);
        let before = inner.sessions.len();
        inner
            .sessions
            .retain(|s| !ct_eq(s.hash.as_bytes(), hash.as_bytes()));
        let removed = inner.sessions.len() != before;
        if removed {
            self.persist_sessions(&mut inner);
        }
        removed
    }

    /// [`Self::pair`], recording the browser's `User-Agent` on the session
    /// (control/bidi chars stripped, capped at 256 chars).
    pub fn pair_with_agent(
        &self,
        peer: IpAddr,
        code: &str,
        user_agent: Option<&str>,
    ) -> Result<NewSession, PairError> {
        let user_agent = user_agent
            .map(|ua| {
                ua.chars()
                    .filter(|c| {
                        !c.is_control()
                            && !matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                    })
                    .take(256)
                    .collect::<String>()
                    .trim()
                    .to_string()
            })
            .filter(|ua| !ua.is_empty());
        let mut inner = self.inner.lock().unwrap();
        self.refresh(&mut inner);
        let now = Instant::now();
        prune_failures(&mut inner.failures, now);
        if let Some(fails) = inner
            .failures
            .get(&peer)
            .filter(|f| f.len() >= FAILS_PER_PEER)
        {
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
            record_failure(&mut inner.failures, peer, now);
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
            user_agent,
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

// ---------------------------------------------------------------- daemon token (L5 §17)

/// Child-environment name of the per-spawn daemon token (Layer 5 §17).
pub const DAEMON_TOKEN_ENV: &str = "AGENT111_DAEMON_TOKEN";

/// Bytes of OS randomness in a [`DaemonToken`] (hex-encoded: 64 chars).
pub const DAEMON_TOKEN_BYTES: usize = 32;

/// The OS random source failed while minting a [`DaemonToken`].
#[derive(Debug)]
pub struct TokenError(getrandom::Error);

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot mint the daemon token: OS randomness: {}", self.0)
    }
}

impl std::error::Error for TokenError {}

/// The per-spawn bearer secret `111d` hands `111-agent` (Layer 5 §17):
/// [`DAEMON_TOKEN_BYTES`] bytes from the OS random source, lowercase hex.
///
/// Lives in [`ApiState`] ext (inserted by the daemon start hook) and in the
/// agent child's environment only — never on disk, never in a log (`Debug` is
/// redacted). This copy is wiped when the last clone drops; the agent's child
/// spec keeps its own copy for respawns while the supervisor lives.
#[derive(Clone)]
pub struct DaemonToken(Arc<zeroize::Zeroizing<String>>);

impl DaemonToken {
    /// Mint a fresh token (once per daemon start, before the agent spawns).
    pub fn mint() -> Result<Self, TokenError> {
        let mut raw = zeroize::Zeroizing::new([0u8; DAEMON_TOKEN_BYTES]);
        getrandom::getrandom(raw.as_mut_slice()).map_err(TokenError)?;
        let mut text = zeroize::Zeroizing::new(String::with_capacity(2 * DAEMON_TOKEN_BYTES));
        for b in raw.iter() {
            text.push(char::from(HEX_DIGITS[usize::from(b >> 4)]));
            text.push(char::from(HEX_DIGITS[usize::from(b & 0x0f)]));
        }
        Ok(DaemonToken(Arc::new(text)))
    }

    /// The token text (64 lowercase hex) — for the agent's child environment.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Does `presented` equal this token? Constant time in the token bytes
    /// (the length, 64, is public).
    pub fn matches(&self, presented: &str) -> bool {
        ct_eq(presented.as_bytes(), self.0.as_bytes())
    }
}

impl fmt::Debug for DaemonToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DaemonToken(<redacted>)")
    }
}

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Every `Authorization: Bearer` credential, in header order (scheme matched
/// case-insensitively, RFC 9110 §11.1; a bare `Bearer` yields `""`). Other
/// schemes and non-ASCII header values are not bearer credentials.
pub fn bearer_tokens(headers: &HeaderMap) -> Vec<&str> {
    headers
        .get_all(header::AUTHORIZATION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| {
            let v = v.trim();
            match v.split_once([' ', '\t']) {
                Some((scheme, rest)) if scheme.eq_ignore_ascii_case("bearer") => Some(rest.trim()),
                None if v.eq_ignore_ascii_case("bearer") => Some(""),
                _ => None,
            }
        })
        .collect()
}

/// What a request's bearer credentials decide (Layer 5 §17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BearerVerdict {
    /// No bearer presented: the loopback-trust / cookie rules decide.
    Absent,
    /// Exactly one bearer, from a loopback peer, equal to the daemon token.
    Accepted,
    /// A bearer was presented and fails: non-loopback peer, no token minted
    /// this lifetime, wrong token, or more than one bearer header.
    Refused,
}

/// The bearer rule as a pure function (see [`BearerVerdict`]). The token is
/// compared only after the peer check, so a LAN peer learns nothing from
/// timing.
pub fn bearer_verdict(
    headers: &HeaderMap,
    peer_loopback: bool,
    token: Option<&DaemonToken>,
) -> BearerVerdict {
    let presented = bearer_tokens(headers);
    match (presented.as_slice(), peer_loopback, token) {
        ([], _, _) => BearerVerdict::Absent,
        ([one], true, Some(t)) if t.matches(one) => BearerVerdict::Accepted,
        _ => BearerVerdict::Refused,
    }
}

fn bearer_refused() -> Problem {
    Problem::new(
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
        "bearer token refused",
    )
    .detail("Bearer tokens are accepted only from the node itself, and only the daemon's own.")
}

/// The global write gate (§13, D26 #2; module doc): a request that presents
/// any bearer credential on a method other than `GET` is refused, whatever
/// the token, the peer or the route. `HEAD` and `OPTIONS` count as writes
/// here (fail closed: the agent only ever needs `GET`).
pub fn bearer_write_refused(method: &Method, headers: &HeaderMap) -> bool {
    *method != Method::GET && !bearer_tokens(headers).is_empty()
}

/// The `401` of the write gate.
fn bearer_write_problem() -> Problem {
    Problem::new(
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
        "a dashboard session is required for this change",
    )
    .detail(
        "The agent's token reads only; every change is made by a person on this node or a paired \
         browser. The agent files a proposal instead.",
    )
}

/// The write gate as a middleware for the routes outside [`require_session`]
/// (the open `/auth/*` writes): the same rule, nothing else.
pub async fn bearer_write_gate(req: Request, next: Next) -> Response {
    if bearer_write_refused(req.method(), req.headers()) {
        return bearer_write_problem().into_response();
    }
    next.run(req).await
}

// ---------------------------------------------------------------- middleware

/// Every `s111` cookie value, in header order (all `Cookie` headers are
/// searched). A browser sends several when cookies with the same name exist
/// for different paths/domains — one planted by another site on the LAN must
/// not shadow the real one, so callers try them all ([`AuthStore::session_in`]).
pub fn session_cookies(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .filter(|(k, _)| *k == COOKIE)
        .map(|(_, v)| v.trim().to_string())
        .collect()
}

impl AuthStore {
    /// The first `s111` cookie in `headers` that names a valid session:
    /// `(token, created)`.
    pub fn session_in(&self, headers: &HeaderMap) -> Option<(String, String)> {
        session_cookies(headers)
            .into_iter()
            .find_map(|t| self.session_for(&t).map(|created| (t, created)))
    }
}

fn unauthenticated() -> Problem {
    Problem::new(
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
        "pairing required",
    )
    .detail("Run `111 dashboard` on the node and enter its pair code.")
}

/// A loopback peer *and* loopback is trusted by policy: no session needed.
pub fn trusted_loopback(api: &ApiState, ext: &Extensions) -> bool {
    api.auth.loopback_trusted() && peer_is_loopback(ext)
}

/// The companion `DeviceRecord.id` of the session behind `token` (the device
/// registry derives it from the stored session hash).
pub fn companion_device_id(token: &str) -> String {
    crate::daemon::devices::device_id(
        crate::daemon::devices::DeviceKind::Companion,
        &sha256_hex(token.to_ascii_lowercase().as_bytes()),
    )
}

/// What a write records as its author (`relay_edges.enabled_by`, the HAM and
/// uplink confirmations, Layer 6 §3, §8): `loopback` for a trusted loopback
/// peer without a session, `paired:<companion device id>` for a paired
/// session (the id the device registry shows; derived from the stored hash,
/// never the token), `unknown` otherwise. Never a secret.
pub fn session_label(api: &ApiState, headers: &HeaderMap, ext: &Extensions) -> String {
    if let Some((token, _)) = api.auth.session_in(headers) {
        return format!("paired:{}", companion_device_id(&token));
    }
    if trusted_loopback(api, ext) {
        return "loopback".into();
    }
    "unknown".into()
}

/// Gate for every `/api/v1/*` route except `/api/v1/auth/*` (and the Layer 1
/// `/api/status` + `/v1`). Order: the global write gate first (any bearer on
/// a non-`GET` is `401`, [`bearer_write_refused`]); then a presented bearer
/// decides ([`bearer_verdict`]: loopback peer + the daemon token, else
/// `401`); then a trusted loopback peer passes; then a valid `s111` session
/// cookie.
pub async fn require_session(
    State(api): State<Arc<ApiState>>,
    req: Request,
    next: Next,
) -> Response {
    if bearer_write_refused(req.method(), req.headers()) {
        return bearer_write_problem().into_response();
    }
    let token = api.ext::<DaemonToken>();
    match bearer_verdict(
        req.headers(),
        peer_is_loopback(req.extensions()),
        token.as_ref(),
    ) {
        BearerVerdict::Accepted => return next.run(req).await,
        BearerVerdict::Refused => return bearer_refused().into_response(),
        BearerVerdict::Absent => {}
    }
    if trusted_loopback(&api, req.extensions()) {
        return next.run(req).await;
    }
    match api.auth.session_in(req.headers()) {
        Some(_) => next.run(req).await,
        None => unauthenticated().into_response(),
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
    /// The node's loopback-trust policy (false under Termux: the phone's own
    /// browser pairs too).
    pub loopback_trusted: bool,
    /// The caller's own companion `DeviceRecord.id` (`null` when unpaired or
    /// trusted loopback).
    pub device_id: Option<String>,
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
    let session = api.auth.session_in(&headers);
    let device_id = session
        .as_ref()
        .filter(|_| !trusted_loopback(&api, &ext))
        .map(|(token, _)| companion_device_id(token));
    Json(AuthSession {
        schema: 1,
        loopback: peer_is_loopback(&ext),
        paired: session.is_some(),
        lan_enabled: lan_effective(&api),
        created: session.map(|(_, created)| created),
        loopback_trusted: api.auth.loopback_trusted(),
        device_id,
    })
}

async fn pair(
    State(api): State<Arc<ApiState>>,
    ext: Extensions,
    headers: HeaderMap,
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
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok());
    match api.auth.pair_with_agent(peer, &req.code, user_agent) {
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
                loopback_trusted: api.auth.loopback_trusted(),
                device_id: (!trusted_loopback(&api, &ext)).then(|| companion_device_id(&s.token)),
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
    for tok in session_cookies(&headers) {
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
        .route_layer(axum::middleware::from_fn(bearer_write_gate))
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
    fn loopback_trust_policy_matrix() {
        // default (plain Linux): trusted
        assert!(loopback_trust_policy(None, None, false));
        // Termux by its env var or by its prefix: not trusted
        assert!(!loopback_trust_policy(None, Some("0.118.1"), false));
        assert!(!loopback_trust_policy(None, None, true));
        assert!(
            loopback_trust_policy(None, Some(""), false),
            "empty is unset"
        );
        // the override wins both ways
        assert!(loopback_trust_policy(Some("true"), Some("0.118.1"), true));
        assert!(loopback_trust_policy(Some(" TRUE "), None, true));
        assert!(!loopback_trust_policy(Some("false"), None, false));
        // anything else is no override
        assert!(loopback_trust_policy(Some("yes"), None, false));
        assert!(!loopback_trust_policy(Some("0"), None, true));
        // this dev/CI box is plain Linux unless told otherwise
        if std::env::var_os(TRUST_LOOPBACK_ENV).is_none()
            && std::env::var_os(TERMUX_VERSION_ENV).is_none()
            && !Path::new(TERMUX_PREFIX).exists()
        {
            assert!(loopback_trusted(), "default on Linux unchanged");
        }
        assert!(AuthStore::open(None).loopback_trusted(), "store default");
        assert!(!AuthStore::open(None)
            .with_loopback_trust(false)
            .loopback_trusted());
    }

    #[test]
    fn companion_id_matches_the_registry_derivation() {
        let token = "ab".repeat(32);
        assert_eq!(
            companion_device_id(&token),
            crate::daemon::devices::device_id(
                crate::daemon::devices::DeviceKind::Companion,
                &sha256_hex(token.as_bytes())
            )
        );
        assert_eq!(
            companion_device_id(&token.to_uppercase()),
            companion_device_id(&token)
        );
    }

    #[test]
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        h.append(
            header::COOKIE,
            HeaderValue::from_static("a=1; s111=abc ; b=2"),
        );
        assert_eq!(session_cookies(&h), ["abc"]);
        let mut h = HeaderMap::new();
        h.append(header::COOKIE, HeaderValue::from_static("x111=no"));
        assert!(session_cookies(&h).is_empty());
        let mut h = HeaderMap::new();
        h.append(
            header::COOKIE,
            HeaderValue::from_static("s111=one; s111=two"),
        );
        h.append(header::COOKIE, HeaderValue::from_static("s111=three"));
        assert_eq!(session_cookies(&h), ["one", "two", "three"]);
    }

    #[test]
    fn a_shadowing_cookie_does_not_hide_the_valid_one() {
        let store = AuthStore::open(None);
        let s = store
            .pair("192.168.1.9".parse().unwrap(), &store.pair_code())
            .unwrap();
        let planted = "f".repeat(64);
        let mut h = HeaderMap::new();
        let v = format!("s111={planted}; s111={}", s.token);
        h.append(header::COOKIE, HeaderValue::from_str(&v).unwrap());
        let (tok, created) = store.session_in(&h).expect("second value validates");
        assert_eq!((tok, created), (s.token.clone(), s.created));
        let mut h = HeaderMap::new();
        h.append(
            header::COOKIE,
            HeaderValue::from_str(&format!("s111={planted}")).unwrap(),
        );
        assert!(store.session_in(&h).is_none());
    }

    #[test]
    fn failure_map_prunes_and_is_capped() {
        let t0 = Instant::now();
        let mut f: HashMap<IpAddr, VecDeque<Instant>> = HashMap::new();
        let ip = |i: u32| IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + i));
        record_failure(&mut f, ip(1), t0);
        record_failure(&mut f, ip(2), t0 + Duration::from_secs(30));
        assert_eq!(f.len(), 2);
        // a minute after ip(1)'s failure it is gone; ip(2) is still in window
        record_failure(&mut f, ip(3), t0 + FAIL_WINDOW);
        assert!(!f.contains_key(&ip(1)));
        assert!(f.contains_key(&ip(2)) && f.contains_key(&ip(3)));
        prune_failures(&mut f, t0 + Duration::from_secs(200));
        assert!(f.is_empty());
        // many peers inside one window: bounded, the stalest forgotten first
        for i in 0..(MAX_FAIL_PEERS as u32 + 50) {
            record_failure(&mut f, ip(100 + i), t0 + Duration::from_millis(i as u64));
        }
        assert_eq!(f.len(), MAX_FAIL_PEERS);
        assert!(!f.contains_key(&ip(100)), "oldest evicted");
        assert!(f.contains_key(&ip(100 + MAX_FAIL_PEERS as u32 + 49)));
    }

    #[test]
    fn many_peers_never_grow_the_store_unbounded() {
        let store = AuthStore::open(None);
        for i in 0..(MAX_FAIL_PEERS as u32 + 200) {
            let peer = IpAddr::V4(Ipv4Addr::from(0xc0a8_0000 + i));
            assert!(matches!(
                store.pair(peer, "0000-0000"),
                Err(PairError::Wrong)
            ));
        }
        assert!(store.inner.lock().unwrap().failures.len() <= MAX_FAIL_PEERS);
    }

    #[test]
    fn transient_read_failure_does_not_mark_a_version_seen() {
        if unsafe { libc::geteuid() } == 0 {
            return; // root reads 0000 files: the failure cannot be simulated
        }
        use std::os::unix::fs::PermissionsExt;
        let set_mode = |p: &Path, m: u32| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap()
        };
        let dir = tempfile::tempdir().unwrap();
        let store = AuthStore::open(Some(dir.path().to_path_buf()));
        let other = AuthStore::open(Some(dir.path().to_path_buf()));
        let old = store.pair_code();
        assert_eq!(other.pair_code(), old);

        // pair file: a rotation the daemon fails to read once is still adopted later
        let new = rotate_files(dir.path()).unwrap();
        let pair = dir.path().join(PAIR_FILE);
        set_mode(&pair, 0o000);
        assert_eq!(store.pair_code(), old, "unreadable: keeps what it had");
        set_mode(&pair, 0o600);
        assert_eq!(store.pair_code(), new, "re-read once readable");

        // sessions file: unreadable fails closed, then the session is back
        let s = store.pair("10.0.0.2".parse().unwrap(), &new).unwrap();
        let sessions = dir.path().join(SESSIONS_FILE);
        set_mode(&sessions, 0o000);
        assert!(other.session_for(&s.token).is_none(), "fail closed");
        set_mode(&sessions, 0o600);
        assert!(
            other.session_for(&s.token).is_some(),
            "re-read once readable"
        );
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
    fn daemon_token_is_fresh_hex_and_redacted() {
        let a = DaemonToken::mint().unwrap();
        let b = DaemonToken::mint().unwrap();
        assert_eq!(a.as_str().len(), 2 * DAEMON_TOKEN_BYTES);
        assert!(a
            .as_str()
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
        assert_ne!(a.as_str(), b.as_str(), "minted per spawn");
        assert!(a.matches(a.as_str()));
        assert!(a.clone().matches(a.as_str()), "clones share the secret");
        assert!(!a.matches(b.as_str()));
        assert!(!a.matches(""));
        assert!(!a.matches(&a.as_str()[..63]));
        assert!(!a.matches(&a.as_str().to_uppercase()), "exact bytes only");
        let dbg = format!("{a:?}");
        assert!(!dbg.contains(a.as_str()), "{dbg}");
        assert_eq!(DAEMON_TOKEN_ENV, "AGENT111_DAEMON_TOKEN");
    }

    fn auth_headers(values: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in values {
            h.append(header::AUTHORIZATION, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn bearer_parsing() {
        assert!(bearer_tokens(&HeaderMap::new()).is_empty());
        assert_eq!(bearer_tokens(&auth_headers(&["Bearer abc"])), ["abc"]);
        assert_eq!(bearer_tokens(&auth_headers(&["bearer  abc "])), ["abc"]);
        assert_eq!(bearer_tokens(&auth_headers(&["BEARER\tabc"])), ["abc"]);
        assert_eq!(bearer_tokens(&auth_headers(&["Bearer"])), [""]);
        assert!(bearer_tokens(&auth_headers(&["Basic YTpi"])).is_empty());
        assert!(bearer_tokens(&auth_headers(&["Bearerabc"])).is_empty());
        assert_eq!(
            bearer_tokens(&auth_headers(&["Basic x", "Bearer one", "Bearer two"])),
            ["one", "two"]
        );
    }

    #[test]
    fn the_write_gate_refuses_a_bearer_on_every_method_but_get() {
        let methods = [
            Method::GET,
            Method::HEAD,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
            Method::from_bytes(b"PROPFIND").unwrap(),
        ];
        let token = DaemonToken::mint().unwrap();
        let good = format!("Bearer {}", token.as_str());
        for m in &methods {
            // no bearer: the gate does not decide (cookies / loopback do)
            assert!(!bearer_write_refused(m, &HeaderMap::new()), "{m}");
            // other schemes are not bearer credentials
            assert!(
                !bearer_write_refused(m, &auth_headers(&["Basic YTpi"])),
                "{m}"
            );
            for h in [good.as_str(), "Bearer nope", "bearer ", "Bearer"] {
                let hs = auth_headers(&[h]);
                assert_eq!(bearer_write_refused(m, &hs), *m != Method::GET, "{m} {h}");
                // the right token from loopback reads, and still never writes
                if h == good {
                    assert_eq!(
                        bearer_verdict(&hs, true, Some(&token)),
                        BearerVerdict::Accepted
                    );
                }
            }
            let two = auth_headers(&[&good, &good]);
            assert_eq!(bearer_write_refused(m, &two), *m != Method::GET, "{m}");
        }
    }

    #[test]
    fn bearer_rule_is_loopback_only_and_exact() {
        use BearerVerdict::{Absent, Accepted, Refused};
        let t = DaemonToken::mint().unwrap();
        let good = format!("Bearer {}", t.as_str());
        let other = DaemonToken::mint().unwrap();
        let wrong = format!("Bearer {}", other.as_str());
        // nothing presented: the other rules decide, wherever the peer is
        for lo in [true, false] {
            assert_eq!(bearer_verdict(&HeaderMap::new(), lo, Some(&t)), Absent);
            assert_eq!(
                bearer_verdict(&auth_headers(&["Basic YTpi"]), lo, Some(&t)),
                Absent
            );
        }
        // the right token from loopback: in
        assert_eq!(
            bearer_verdict(&auth_headers(&[&good]), true, Some(&t)),
            Accepted
        );
        // the right token from anywhere else: refused, never ignored
        assert_eq!(
            bearer_verdict(&auth_headers(&[&good]), false, Some(&t)),
            Refused
        );
        // wrong / empty / duplicated / no token minted: refused
        assert_eq!(
            bearer_verdict(&auth_headers(&[&wrong]), true, Some(&t)),
            Refused
        );
        assert_eq!(
            bearer_verdict(&auth_headers(&["Bearer"]), true, Some(&t)),
            Refused
        );
        assert_eq!(
            bearer_verdict(&auth_headers(&[&good, &good]), true, Some(&t)),
            Refused
        );
        assert_eq!(
            bearer_verdict(&auth_headers(&[&wrong, &good]), true, Some(&t)),
            Refused
        );
        assert_eq!(bearer_verdict(&auth_headers(&[&good]), true, None), Refused);
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
