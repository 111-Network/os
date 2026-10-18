//! Hardened ingest of the presence reader's output (Layer 5 §14 "Presence
//! reader"): `fabric/presence/meshtastic.json` → the **Nearby** list
//! (`GET /api/v1/nearby`, SSE `nearby`).
//!
//! The file is written by a separate GPL process from RF-origin data, so
//! `111d` trusts nothing in it:
//!
//! - opened `O_NOFOLLOW | O_NONBLOCK` (a symlink is refused, a FIFO cannot
//!   block the daemon), then `fstat`: a **regular file of at most 1 MiB**; the
//!   size is checked before anything is parsed, and the read is capped too;
//! - parsed into typed fields: `schema` must be `1`, at most **4096 nodes**, no
//!   duplicate node numbers, `id` equal to `!` + the node number as 8 hex
//!   digits, every optional number finite and inside the reader's documented
//!   range (`snr` −100..100, `rssi` −200..0, `hops_away` 0..7,
//!   `channel_utilization` 0..100, lat/lon in degrees), `pubkey` standard
//!   base64 of 32 bytes;
//! - names are **untrusted RF-origin strings**: control, bidi and invisible
//!   characters stripped, cut to **40 UTF-8 bytes** on a character boundary
//!   (defence in depth: the reader already does both);
//! - `last_heard_hint` is the node's own clock (unix seconds) — rendered as an
//!   RFC 3339 *hint*, never used to order or to judge freshness.
//!
//! An invalid file (symlink, not regular, oversize, bad JSON, bad schema)
//! **keeps the previous snapshot** and bumps a counter (logged once per
//! distinct failure). A missing file clears the snapshot: the reader deletes
//! its stale file at start, so "no file" means "no node DB yet".

use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// The reader's output file name inside `fabric/presence/`.
pub const FILE: &str = "meshtastic.json";
/// Largest file accepted (§14: ≤ 1 MiB), checked before parsing.
pub const MAX_FILE_BYTES: u64 = 1 << 20;
/// Most nodes accepted in one snapshot (§14).
pub const MAX_NODES: usize = 4096;
/// Name cap in UTF-8 bytes (§14).
pub const MAX_NAME_BYTES: usize = 40;
/// Only schema accepted.
pub const SCHEMA: u32 = 1;

/// `<root>/fabric/presence` (0700; the reader writes `meshtastic.json` 0644).
pub fn presence_dir(root: &Path) -> PathBuf {
    root.join("fabric/presence")
}

/// Why a presence file was not accepted (the previous snapshot is kept).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestError {
    /// No file (not an error for the snapshot: it clears).
    Missing,
    /// The path is a symlink (`O_NOFOLLOW` → `ELOOP`).
    Symlink,
    /// Not a regular file (FIFO, socket, directory, device).
    NotRegular,
    /// Larger than [`MAX_FILE_BYTES`].
    TooLarge(u64),
    /// Any other I/O failure.
    Io(String),
    /// Not JSON of the expected shape (wrong type, missing field).
    Parse(String),
    /// Parsed, but a rule failed (schema, counts, ranges, ids).
    Invalid(String),
}

impl fmt::Display for IngestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IngestError::Missing => f.write_str("no presence file"),
            IngestError::Symlink => f.write_str("the presence file is a symlink"),
            IngestError::NotRegular => f.write_str("the presence file is not a regular file"),
            IngestError::TooLarge(n) => write!(f, "the presence file is {n} bytes (max 1 MiB)"),
            IngestError::Io(e) => write!(f, "cannot read the presence file: {e}"),
            IngestError::Parse(e) => write!(f, "the presence file is not valid: {e}"),
            IngestError::Invalid(e) => write!(f, "the presence file breaks a rule: {e}"),
        }
    }
}

impl std::error::Error for IngestError {}

/// A node's position (`NearbyNode.position`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Position {
    pub lat: f64,
    pub lon: f64,
    pub alt_m: Option<f64>,
}

/// One node heard (`NearbyNode`, openapi). Every string is untrusted
/// RF-origin text: render as text only.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NearbyNode {
    pub id: String,
    pub num: u32,
    pub long_name: String,
    pub short_name: String,
    pub pubkey: Option<String>,
    pub snr: Option<f64>,
    pub rssi: Option<i32>,
    pub hops_away: Option<u8>,
    /// The node's own clock as RFC 3339 — a display hint only.
    pub last_heard_hint: Option<String>,
    pub position: Option<Position>,
    pub channel_utilization: Option<f64>,
}

// ---- the raw, typed file shape (unknown keys are ignored, never published)

#[derive(Deserialize)]
struct RawFile {
    schema: u32,
    nodes: Vec<RawNode>,
}

#[derive(Deserialize)]
struct RawPosition {
    lat: f64,
    lon: f64,
    #[serde(default)]
    alt: Option<f64>,
}

#[derive(Deserialize)]
struct RawNode {
    id: String,
    num: u64,
    long_name: String,
    short_name: String,
    #[serde(default)]
    pubkey: Option<String>,
    #[serde(default)]
    snr: Option<f64>,
    #[serde(default)]
    rssi: Option<i64>,
    #[serde(default)]
    hops_away: Option<u64>,
    #[serde(default)]
    last_heard_hint: Option<u64>,
    #[serde(default)]
    position: Option<RawPosition>,
    #[serde(default)]
    channel_utilization: Option<f64>,
}

/// Bidi controls and invisible formatting characters (Trojan-Source class).
fn is_bidi_or_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{061c}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{2069}'
            | '\u{feff}'
    )
}

/// An RF-origin name → display-safe text: control/bidi/invisible characters
/// dropped, trimmed, cut to `max_bytes` UTF-8 bytes on a char boundary.
pub fn sanitize_name(s: &str, max_bytes: usize) -> String {
    let filtered: String = s
        .chars()
        .filter(|c| !c.is_control() && !is_bidi_or_invisible(*c))
        .collect();
    let t = filtered.trim();
    let mut out = String::new();
    for c in t.chars() {
        if out.len() + c.len_utf8() > max_bytes {
            break;
        }
        out.push(c);
    }
    out.trim_end().to_string()
}

fn valid_pubkey(s: &str) -> bool {
    // standard base64 of exactly 32 bytes: 43 symbols + one '='
    s.len() == 44
        && s.ends_with('=')
        && !s[..43].contains('=')
        && s[..43]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

fn in_range(v: Option<f64>, lo: f64, hi: f64, what: &str) -> Result<Option<f64>, IngestError> {
    match v {
        Some(x) if !(x.is_finite() && (lo..=hi).contains(&x)) => {
            Err(IngestError::Invalid(format!("{what} out of range")))
        }
        other => Ok(other),
    }
}

fn rfc3339_secs(secs: u64) -> Option<String> {
    let t = time::OffsetDateTime::from_unix_timestamp(i64::try_from(secs).ok()?).ok()?;
    t.format(&time::format_description::well_known::Rfc3339)
        .ok()
}

/// Validate one raw node (see module docs).
fn node(raw: RawNode) -> Result<NearbyNode, IngestError> {
    let invalid = |m: &str| IngestError::Invalid(m.to_string());
    let num = u32::try_from(raw.num)
        .ok()
        .filter(|n| *n > 0 && *n < u32::MAX)
        .ok_or_else(|| invalid("node number out of range"))?;
    let id = format!("!{num:08x}");
    if raw.id != id {
        return Err(invalid("node id does not match its number"));
    }
    if let Some(p) = &raw.pubkey {
        if !valid_pubkey(p) {
            return Err(invalid("pubkey is not base64 of 32 bytes"));
        }
    }
    let snr = in_range(raw.snr, -100.0, 100.0, "snr")?;
    let rssi = match raw.rssi {
        Some(r) if !(-200..=0).contains(&r) => return Err(invalid("rssi out of range")),
        Some(r) => Some(r as i32),
        None => None,
    };
    let hops_away = match raw.hops_away {
        Some(h) if h > 7 => return Err(invalid("hops_away out of range")),
        Some(h) => Some(h as u8),
        None => None,
    };
    let last_heard_hint = match raw.last_heard_hint {
        Some(s) if s == 0 || s > u64::from(u32::MAX) => {
            return Err(invalid("last_heard_hint out of range"))
        }
        Some(s) => rfc3339_secs(s),
        None => None,
    };
    let position = match raw.position {
        None => None,
        Some(p) => Some(Position {
            lat: in_range(Some(p.lat), -90.0, 90.0, "lat")?.unwrap_or_default(),
            lon: in_range(Some(p.lon), -180.0, 180.0, "lon")?.unwrap_or_default(),
            alt_m: in_range(p.alt, -1000.0, 100_000.0, "alt")?,
        }),
    };
    let channel_utilization = in_range(raw.channel_utilization, 0.0, 100.0, "channel_utilization")?;
    Ok(NearbyNode {
        id,
        num,
        long_name: sanitize_name(&raw.long_name, MAX_NAME_BYTES),
        short_name: sanitize_name(&raw.short_name, MAX_NAME_BYTES),
        pubkey: raw.pubkey,
        snr,
        rssi,
        hops_away,
        last_heard_hint,
        position,
        channel_utilization,
    })
}

/// Validate a whole document (bytes already size-checked).
pub fn parse(bytes: &[u8]) -> Result<Vec<NearbyNode>, IngestError> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(IngestError::TooLarge(bytes.len() as u64));
    }
    let raw: RawFile =
        serde_json::from_slice(bytes).map_err(|e| IngestError::Parse(e.to_string()))?;
    if raw.schema != SCHEMA {
        return Err(IngestError::Invalid(format!(
            "schema {} (want 1)",
            raw.schema
        )));
    }
    if raw.nodes.len() > MAX_NODES {
        return Err(IngestError::Invalid(format!(
            "{} nodes (max {MAX_NODES})",
            raw.nodes.len()
        )));
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(raw.nodes.len());
    for r in raw.nodes {
        let n = node(r)?;
        if !seen.insert(n.num) {
            return Err(IngestError::Invalid("duplicate node number".into()));
        }
        out.push(n);
    }
    out.sort_by_key(|n| n.num);
    Ok(out)
}

/// Identity of the file version read (skip re-parsing an unchanged file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
}

/// Open `path` safely and read it whole: `O_NOFOLLOW`, regular file,
/// ≤ [`MAX_FILE_BYTES`] (checked on the descriptor before the read and again
/// on the bytes read). Returns the bytes and the version stamp.
pub fn read_file(path: &Path) -> Result<(Vec<u8>, Stamp), IngestError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => IngestError::Missing,
            _ if e.raw_os_error() == Some(libc::ELOOP) => IngestError::Symlink,
            _ => IngestError::Io(e.to_string()),
        })?;
    let meta = f.metadata().map_err(|e| IngestError::Io(e.to_string()))?;
    if !meta.file_type().is_file() {
        return Err(IngestError::NotRegular);
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(IngestError::TooLarge(meta.len()));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    f.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| IngestError::Io(e.to_string()))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(IngestError::TooLarge(bytes.len() as u64));
    }
    let stamp = Stamp {
        dev: meta.dev(),
        ino: meta.ino(),
        len: meta.len(),
        mtime: meta.mtime(),
        mtime_nsec: meta.mtime_nsec(),
    };
    Ok((bytes, stamp))
}

/// What one [`Presence::refresh`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refresh {
    /// Nothing changed.
    Unchanged,
    /// A new valid snapshot was accepted (or the snapshot cleared).
    Changed,
    /// The file was refused; the previous snapshot is kept.
    Rejected(IngestError),
}

#[derive(Default)]
struct State {
    nodes: Option<Vec<NearbyNode>>,
    stamp: Option<Stamp>,
    last_error: Option<String>,
}

/// The current snapshot of one presence file.
pub struct Presence {
    path: PathBuf,
    state: Mutex<State>,
    rejected: AtomicU64,
}

impl Presence {
    pub fn new(path: PathBuf) -> Self {
        Presence {
            path,
            state: Mutex::new(State::default()),
            rejected: AtomicU64::new(0),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Re-read the file when it changed (blocking I/O: call from a blocking
    /// context). A refused file keeps the previous snapshot and bumps
    /// [`Self::rejected`]; a missing file clears it.
    pub fn refresh(&self) -> Refresh {
        match read_file(&self.path) {
            Err(IngestError::Missing) => {
                let mut st = self.lock();
                st.stamp = None;
                st.last_error = None;
                if st.nodes.take().is_some() {
                    Refresh::Changed
                } else {
                    Refresh::Unchanged
                }
            }
            Err(e) => self.reject(e, None),
            Ok((bytes, stamp)) => {
                if self.lock().stamp == Some(stamp) {
                    return Refresh::Unchanged;
                }
                match parse(&bytes) {
                    Ok(nodes) => {
                        let mut st = self.lock();
                        st.stamp = Some(stamp);
                        st.last_error = None;
                        if st.nodes.as_ref() == Some(&nodes) {
                            Refresh::Unchanged
                        } else {
                            st.nodes = Some(nodes);
                            Refresh::Changed
                        }
                    }
                    Err(e) => self.reject(e, Some(stamp)),
                }
            }
        }
    }

    fn reject(&self, e: IngestError, stamp: Option<Stamp>) -> Refresh {
        let mut st = self.lock();
        // a bad version already seen is not counted again
        if stamp.is_some() && st.stamp == stamp {
            return Refresh::Unchanged;
        }
        if stamp.is_some() {
            st.stamp = stamp;
        }
        self.rejected.fetch_add(1, Ordering::Relaxed);
        let msg = e.to_string();
        // logged once per distinct failure (the counter keeps the rest)
        if st.last_error.as_deref() != Some(msg.as_str()) {
            eprintln!(
                "111 meshtastic: {} refused ({msg}); keeping the previous node list",
                self.path.display()
            );
        }
        st.last_error = Some(msg);
        Refresh::Rejected(e)
    }

    /// The accepted node DB (`None` = the reader has produced none yet).
    pub fn nodes(&self) -> Option<Vec<NearbyNode>> {
        self.lock().nodes.clone()
    }

    /// Number of nodes in the accepted snapshot (0 when none).
    pub fn count(&self) -> u32 {
        self.lock()
            .nodes
            .as_ref()
            .map_or(0, |n| n.len().min(u32::MAX as usize) as u32)
    }

    /// A node DB is present.
    pub fn present(&self) -> bool {
        self.lock().nodes.is_some()
    }

    /// Refused files since start (the "log counter" of §14).
    pub fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }

    /// Forget the snapshot (the binding went away).
    pub fn clear(&self) -> bool {
        let mut st = self.lock();
        st.stamp = None;
        st.last_error = None;
        st.nodes.take().is_some()
    }
}

/// `NearbySource` (openapi): one per identified Meshtastic board.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NearbySource {
    pub id: String,
    pub kind: &'static str,
    /// "heard via Meshtastic on <board alias>".
    pub label: String,
    pub device_id: Option<String>,
    pub nodes: Vec<NearbyNode>,
}

impl NearbySource {
    pub fn meshtastic(device_id: &str, board: &str, nodes: Vec<NearbyNode>) -> Self {
        NearbySource {
            id: format!("meshtastic:{device_id}"),
            kind: "meshtastic",
            label: format!("heard via Meshtastic on {board}"),
            device_id: Some(device_id.to_string()),
            nodes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(nodes: serde_json::Value) -> Vec<u8> {
        json!({"schema": 1, "nodes": nodes})
            .to_string()
            .into_bytes()
    }

    fn good() -> serde_json::Value {
        json!([{
            "id": "!1b2c3d4e", "num": 0x1b2c3d4e_u32, "long_name": "Hilltop", "short_name": "HT",
            "pubkey": format!("{}=", "A".repeat(43)), "snr": 5.5, "rssi": -97, "hops_away": 1,
            "last_heard_hint": 1760000000, "position": {"lat": 51.5074, "lon": -0.1278, "alt": 35},
            "channel_utilization": 7.25, "future_field": "ignored"
        }])
    }

    #[test]
    fn a_valid_document_is_typed() {
        let nodes = parse(&doc(good())).unwrap();
        assert_eq!(nodes.len(), 1);
        let n = &nodes[0];
        assert_eq!((n.id.as_str(), n.num), ("!1b2c3d4e", 0x1b2c3d4e));
        assert_eq!(n.long_name, "Hilltop");
        assert_eq!(n.rssi, Some(-97));
        assert_eq!(n.hops_away, Some(1));
        assert_eq!(n.last_heard_hint.as_deref(), Some("2025-10-09T08:53:20Z"));
        let p = n.position.as_ref().unwrap();
        assert_eq!((p.lat, p.lon, p.alt_m), (51.5074, -0.1278, Some(35.0)));
        let v = serde_json::to_value(n).unwrap();
        assert!(v.get("future_field").is_none());
        assert_eq!(v["position"]["alt_m"], 35.0);
        // optional fields absent → null
        let bare = parse(&doc(
            json!([{"id": "!00000001", "num": 1, "long_name": "", "short_name": ""}]),
        ))
        .unwrap();
        assert!(bare[0].snr.is_none() && bare[0].position.is_none());
        // empty node DB is a valid snapshot
        assert!(parse(&doc(json!([]))).unwrap().is_empty());
    }

    #[test]
    fn names_are_stripped_and_cut_to_40_bytes() {
        assert_eq!(
            sanitize_name("a\u{7}b\u{1b}[2Jc\u{202e}d\u{2066}e\u{200b}f", 40),
            "ab[2Jcdef"
        );
        let long = "é".repeat(30); // 60 bytes
        let cut = sanitize_name(&long, 40);
        assert_eq!(cut.len(), 40);
        assert_eq!(cut.chars().count(), 20);
        // a multi-byte char straddling the cut is dropped, never split
        let s = format!("{}€", "x".repeat(39));
        assert_eq!(sanitize_name(&s, 40), "x".repeat(39));
        let nodes = parse(&doc(json!([{
            "id": "!00000002", "num": 2, "long_name": "y".repeat(200), "short_name": "\u{1b}]0;pwn\u{7}"
        }])))
        .unwrap();
        assert_eq!(nodes[0].long_name.len(), 40);
        assert_eq!(nodes[0].short_name, "]0;pwn");
    }

    #[test]
    fn rule_violations_are_invalid() {
        let base = |extra: serde_json::Value| {
            let mut n = json!({"id": "!00000003", "num": 3, "long_name": "n", "short_name": "s"});
            for (k, v) in extra.as_object().unwrap() {
                n[k] = v.clone();
            }
            doc(json!([n]))
        };
        for (what, bytes) in [
            (
                "schema 2",
                json!({"schema": 2, "nodes": []}).to_string().into_bytes(),
            ),
            ("no schema", json!({"nodes": []}).to_string().into_bytes()),
            (
                "nodes not a list",
                json!({"schema": 1, "nodes": {}}).to_string().into_bytes(),
            ),
            ("not json", b"{nope".to_vec()),
            ("num 0", base(json!({"num": 0, "id": "!00000000"}))),
            (
                "num broadcast",
                base(json!({"num": 0xffffffff_u32, "id": "!ffffffff"})),
            ),
            ("num too big", base(json!({"num": 0x1_0000_0000_u64}))),
            ("id mismatch", base(json!({"id": "!deadbeef"}))),
            ("name not a string", base(json!({"long_name": 5}))),
            ("snr string", base(json!({"snr": "5"}))),
            ("snr range", base(json!({"snr": 1000.0}))),
            ("rssi range", base(json!({"rssi": 12}))),
            ("hops range", base(json!({"hops_away": 8}))),
            ("util range", base(json!({"channel_utilization": 101.0}))),
            (
                "lat range",
                base(json!({"position": {"lat": 91.0, "lon": 0.0}})),
            ),
            ("heard zero", base(json!({"last_heard_hint": 0}))),
            ("pubkey", base(json!({"pubkey": "not base64"}))),
            (
                "pubkey 33",
                base(json!({"pubkey": format!("{}==", "A".repeat(42))})),
            ),
        ] {
            assert!(parse(&bytes).is_err(), "{what} must be refused");
        }
        let dup = doc(json!([
            {"id": "!00000004", "num": 4, "long_name": "a", "short_name": "a"},
            {"id": "!00000004", "num": 4, "long_name": "b", "short_name": "b"}
        ]));
        assert!(matches!(parse(&dup), Err(IngestError::Invalid(_))));
        let many: Vec<_> = (1..=(MAX_NODES as u32 + 1))
            .map(
                |n| json!({"id": format!("!{n:08x}"), "num": n, "long_name": "", "short_name": ""}),
            )
            .collect();
        assert!(matches!(
            parse(&doc(json!(many))),
            Err(IngestError::Invalid(_))
        ));
        let max: Vec<_> = (1..=(MAX_NODES as u32))
            .map(
                |n| json!({"id": format!("!{n:08x}"), "num": n, "long_name": "", "short_name": ""}),
            )
            .collect();
        assert_eq!(parse(&doc(json!(max))).unwrap().len(), MAX_NODES);
    }

    #[test]
    fn file_hardening_keeps_the_last_good_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let p = Presence::new(path.clone());
        // missing → nothing, unchanged
        assert_eq!(p.refresh(), Refresh::Unchanged);
        assert!(!p.present());

        std::fs::write(&path, doc(good())).unwrap();
        assert_eq!(p.refresh(), Refresh::Changed);
        assert_eq!(p.count(), 1);
        assert_eq!(
            p.refresh(),
            Refresh::Unchanged,
            "same version is not re-parsed"
        );

        // invalid JSON → rejected, previous kept, counted once
        std::fs::write(&path, b"{\"schema\":1,\"nodes\":[").unwrap();
        assert!(matches!(
            p.refresh(),
            Refresh::Rejected(IngestError::Parse(_))
        ));
        assert_eq!(p.count(), 1);
        assert_eq!(p.rejected(), 1);
        assert_eq!(
            p.refresh(),
            Refresh::Unchanged,
            "same bad version counted once"
        );
        assert_eq!(p.rejected(), 1);

        // oversize → rejected before parsing
        let big = vec![b' '; MAX_FILE_BYTES as usize + 1];
        std::fs::write(&path, &big).unwrap();
        assert!(matches!(
            p.refresh(),
            Refresh::Rejected(IngestError::TooLarge(_))
        ));
        assert_eq!(p.count(), 1);

        // a symlink is refused even when its target is valid
        let target = dir.path().join("elsewhere.json");
        std::fs::write(&target, doc(json!([]))).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert_eq!(p.refresh(), Refresh::Rejected(IngestError::Symlink));
        assert_eq!(p.count(), 1, "still the last good snapshot");

        // a FIFO never blocks and is refused
        std::fs::remove_file(&path).unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap();
        assert!(made.success());
        assert_eq!(p.refresh(), Refresh::Rejected(IngestError::NotRegular));
        // a directory too
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(p.refresh(), Refresh::Rejected(IngestError::NotRegular));
        assert_eq!(p.count(), 1);
        assert!(p.rejected() >= 4);

        // valid again → accepted; then the reader deletes it → cleared
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, doc(json!([]))).unwrap();
        assert_eq!(p.refresh(), Refresh::Changed);
        assert_eq!((p.present(), p.count()), (true, 0));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(p.refresh(), Refresh::Changed);
        assert!(!p.present());
    }

    #[test]
    fn sources_are_labelled() {
        let s = NearbySource::meshtastic("abcd1234abcd1234", "Bench radio", vec![]);
        assert_eq!(s.label, "heard via Meshtastic on Bench radio");
        assert_eq!(s.id, "meshtastic:abcd1234abcd1234");
        assert_eq!(serde_json::to_value(&s).unwrap()["kind"], "meshtastic");
    }
}
