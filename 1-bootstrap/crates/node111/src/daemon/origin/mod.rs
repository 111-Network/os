//! The hub as update origin, the serving side (Layer 7 §9.1; S4, S8, I7; §19 C6, D7).
//!
//! An origin is three names: `channel-111-stable.toml`, `channel-111-stable.toml.minisig` and
//! `blob/<sha256>`. A hub keeps them in `state/origin/` and serves them at
//! `GET /origin/v1/{channel-111-stable.toml, channel-111-stable.toml.minisig, blob/<sha256>}` on
//! the existing dashboard listener ([`routes`], mounted beside `/health` outside the session gate
//! by `dashboard::routes::app`): read-only, unauthenticated, GET only (405 otherwise), no CSRF,
//! the dashboard's CSP and Host guard. No third port.
//!
//! - **When (§9.1, §19 C6).** Served only while the fabric holds the hub role, LAN sharing is on
//!   for this lifetime (`ApiState::lan_enabled`: the dashboard is LAN-bound only then) and
//!   `origin.serve` is on with its `origin_serve` consent recorded ([`serving`]); otherwise every
//!   name answers 404. The switch is read from the fabric's relay roles
//!   (`FabricHandle::relay_roles`, set from `state/relay.json` at start and by the relay settings
//!   route), the same fact behind role bit 5 (`ROLE_ORIGIN`), so the route and the announce
//!   never disagree; nothing is read from disk to decide.
//! - **What (§9.1).** `state/origin/` holds only bytes that passed
//!   [`ChannelManifest::parse_verified`] (the compiled key, the keychain walk, the channel, the
//!   node's own floor) and blobs whose sha256 and size match a payload pin of that manifest, for
//!   the hub's own target and the others listed in `origin.targets` ([`Origin::admit`]). The
//!   daemon is the only writer; a blob is named by the sha256 of the bytes written, hashed as
//!   they are copied. A blob request is answered only when the held manifest pins that sha256
//!   with the file's exact size, so a stray file under `blob/` is never served.
//! - **How (§9.1).** Bodies stream from disk in 64 KiB chunks (`tokio::fs` and a body stream),
//!   never read whole; files are opened `O_NOFOLLOW | O_NONBLOCK` and must be regular.
//! - **Size (§13.2 `RelayOrigin.bytes`).** [`Origin::bytes`] walks `state/origin/` on demand and
//!   keeps the sum until the directory changes (an in-process generation, bumped by every write
//!   here, and the directories' and files' metadata for a write by another process).
//!
//! Never adds: a listener, a task, a timer or a resident buffer (§1 rule 2, §16). Disk: one
//! payload per served target plus the manifest and its signature (§16).

use crate::calver::Calver;
use crate::daemon::dashboard::ApiState;
use crate::daemon::fabric::FabricHandle;
use crate::daemon::updater::fetch::PAYLOAD_MAX_BYTES;
use crate::floor::Floor;
use crate::manifest::{ChannelManifest, MANIFEST_MAX_BYTES, SIGNATURE_MAX_BYTES};
use axum::body::{Body, Bytes};
use axum::extract::{Path as UrlPath, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use sha2::Digest;
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub use crate::daemon::updater::fetch::{MANIFEST_NAME, SIGNATURE_NAME};

/// `state/origin/` (§9.1).
pub const ORIGIN_DIR: &str = "origin";
/// `state/origin/blob/`: one file per held payload, named by its sha256 (§9.1).
pub const BLOB_DIR: &str = "blob";
/// The route prefix (§9.1).
pub const ROUTE_PREFIX: &str = "/origin/v1/";

/// Read and stream chunk size.
const CHUNK: usize = 64 * 1024;
/// Prefix of a blob being written (never a sha256, so never served; removed by the next prune).
const TMP_PREFIX: &str = ".tmp-";
const FILE_MODE: u32 = 0o600;
const DIR_MODE: u32 = 0o700;

/// `state/origin/` under the daemon's `state/` directory.
pub fn dir(state_dir: &Path) -> PathBuf {
    state_dir.join(ORIGIN_DIR)
}

/// 64 lowercase hex digits (the pin format, and the only blob name ever opened).
pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

// ---------------------------------------------------------------- the gate

/// §9.1: the three names are served only while the fabric holds the hub role, LAN sharing is on
/// and `origin.serve` is on with its consent recorded. Fail closed: no fabric handle ⇒ off.
pub fn serving(api: &ApiState) -> bool {
    api.lan_enabled
        && api
            .ext::<Arc<FabricHandle>>()
            .is_some_and(|f| f.relay_roles().origin_serve && f.persisted().fabric.hub_role)
}

// ---------------------------------------------------------------- admission

/// Why [`Origin::admit`] stored nothing (log-safe: no foreign bytes beyond a calver or a path).
#[derive(Debug)]
pub enum AdmitError {
    /// The manifest and signature did not pass `parse_verified` (or the floor is unreadable).
    Unverified(String),
    /// The offered release is older than the one held (an origin never steps back, §9.5 T4).
    Older {
        /// The release held now.
        held: Calver,
        /// The release offered.
        offered: Calver,
    },
    /// The hub's own target has no payload pin in the manifest, or no blob given (or held)
    /// matches it: a hub holds its own target's payload (§9.1).
    OwnPayloadMissing(String),
    /// A filesystem step failed; nothing new is served (written blobs are pruned later).
    Io(String),
}

impl std::fmt::Display for AdmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdmitError::Unverified(why) => write!(f, "the manifest does not verify: {why}"),
            AdmitError::Older { held, offered } => write!(
                f,
                "release {offered} is older than the release {held} this origin holds"
            ),
            AdmitError::OwnPayloadMissing(target) => {
                write!(f, "no verified payload for this hub's own target {target}")
            }
            AdmitError::Io(why) => write!(f, "origin store: {why}"),
        }
    }
}

impl std::error::Error for AdmitError {}

fn io_err(what: &str, path: &Path, e: impl std::fmt::Display) -> AdmitError {
    AdmitError::Io(format!("{what} {}: {e}", path.display()))
}

/// What [`Origin::admit`] stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admitted {
    /// The release now served.
    pub release: Calver,
    /// The served targets whose payload is held now (the own target first).
    pub held: Vec<String>,
    /// The served targets the manifest pins but whose payload is not held (no blob given).
    pub missing: Vec<String>,
    /// Blob paths given that match no payload pin of a served target (not stored).
    pub unpinned: Vec<PathBuf>,
}

/// The trust anchors of an admission: the compiled key and the node's floor.
struct Trust<'a> {
    pubkey: &'a str,
    floor: Floor,
}

/// One payload pin of a served target.
#[derive(Debug, Clone)]
struct Pin {
    target: String,
    sha256: String,
    size: u64,
}

/// The pins of `targets` in `m` (a target the manifest ships nothing for has none).
fn pins_for(m: &ChannelManifest, targets: &[String]) -> Vec<Pin> {
    targets
        .iter()
        .filter_map(|t| {
            m.payload_for(t).map(|p| Pin {
                target: t.clone(),
                sha256: p.sha256.clone(),
                size: p.size,
            })
        })
        .filter(|p| is_sha256_hex(&p.sha256) && p.size <= PAYLOAD_MAX_BYTES)
        .collect()
}

/// A blob copied into `blob/` under a temporary name, hashed as written.
struct Copied {
    tmp: PathBuf,
    sha256: String,
}

/// `state/origin/` as one store: [`Origin::admit`] (L4), [`Origin::retain_targets`] (the relay
/// settings route), [`Origin::bytes`] (`RelayStatus.origin.bytes`). One per daemon
/// ([`Origin::shared`]); the routes read the directory directly.
pub struct Origin {
    dir: PathBuf,
    state_dir: PathBuf,
    /// Serializes this process's writes. Another process (`111 update --from`, row L4) admits
    /// under `state/staging.lock`, which the daemon's own hand-over holds too.
    write: Mutex<()>,
    /// Bumped by every write here (the size cache's in-process key).
    generation: AtomicU64,
    cache: Mutex<Option<(Stamp, u64)>>,
}

impl Origin {
    /// The store under `state_dir` (nothing is created until the first admission).
    pub fn new(state_dir: &Path) -> Origin {
        Origin {
            dir: dir(state_dir),
            state_dir: state_dir.to_path_buf(),
            write: Mutex::new(()),
            generation: AtomicU64::new(0),
            cache: Mutex::new(None),
        }
    }

    /// The daemon's one store (`ApiState` ext `Arc<Origin>`), registered on first use.
    pub fn shared(api: &ApiState) -> Arc<Origin> {
        if let Some(o) = api.ext::<Arc<Origin>>() {
            return o;
        }
        let o = Arc::new(Origin::new(&api.layout.state_dir()));
        api.insert_ext(Arc::clone(&o));
        o
    }

    /// `state/origin/`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Admit a release for serving (§9.1; row L4 calls it with the bytes the updater or
    /// `111 update --from` fetched, and the paths of payload files on disk): the manifest and
    /// its signature must pass `parse_verified` under the compiled key and this node's floor
    /// (`state/floor.json`); each blob is copied into `blob/` while it is hashed and kept only
    /// when its sha256 and size match a payload pin of a served target (`origin.targets`, the
    /// own target first); the own target's payload must be held afterwards. Then the manifest
    /// and its signature replace the held ones and every blob the new manifest does not pin for
    /// a served target is removed. On any refusal nothing new is served.
    ///
    /// Blocking (file I/O and hashing): call it from `spawn_blocking`.
    pub fn admit(
        &self,
        manifest_bytes: &[u8],
        signature: &str,
        blobs: &[&Path],
    ) -> Result<Admitted, AdmitError> {
        let floor = Floor::read(&self.state_dir.join("floor.json"))
            .map_err(|e| AdmitError::Unverified(format!("{e:#}")))?
            .unwrap_or_else(Floor::initial);
        let trust = Trust {
            pubkey: crate::daemon::updater::MANIFEST_PUBKEY.trim(),
            floor,
        };
        let settings = crate::daemon::relay::settings::load(&self.state_dir).settings;
        let targets = settings.origin_targets().to_vec();
        self.admit_with(&trust, &targets, manifest_bytes, signature, blobs)
    }

    /// [`Origin::admit`] with the anchors and the served targets given (`targets[0]` is the own
    /// target).
    fn admit_with(
        &self,
        trust: &Trust<'_>,
        targets: &[String],
        manifest_bytes: &[u8],
        signature: &str,
        blobs: &[&Path],
    ) -> Result<Admitted, AdmitError> {
        // Sizes before anything is parsed (parse_verified checks them too).
        if manifest_bytes.len() > MANIFEST_MAX_BYTES || signature.len() > SIGNATURE_MAX_BYTES {
            return Err(AdmitError::Unverified(
                "the manifest or its signature is over its size cap".into(),
            ));
        }
        let m =
            ChannelManifest::parse_verified(manifest_bytes, signature, trust.pubkey, &trust.floor)
                .map_err(|e| AdmitError::Unverified(format!("{e:#}")))?;
        let _w = self.write.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(held) = self.held_manifest() {
            if held.release() > m.release() {
                return Err(AdmitError::Older {
                    held: held.release(),
                    offered: m.release(),
                });
            }
        }
        let own = targets.first().cloned().unwrap_or_default();
        let pins = pins_for(&m, targets);
        let blob_dir = self.dir.join(BLOB_DIR);
        make_dir(&self.dir)?;
        make_dir(&blob_dir)?;

        // Copy and hash every candidate; keep the ones a pin names.
        let mut unpinned = Vec::new();
        let mut kept: Vec<Copied> = Vec::new();
        let mut cleanup = TmpGuard(Vec::new());
        for path in blobs {
            match copy_hashed(path, &blob_dir, &pins)? {
                Some(c) => {
                    cleanup.0.push(c.tmp.clone());
                    kept.push(c);
                }
                None => unpinned.push(path.to_path_buf()),
            }
        }
        let held_after =
            |p: &Pin| kept.iter().any(|c| c.sha256 == p.sha256) || held_blob(&blob_dir, p);
        if !pins.iter().any(|p| p.target == own && held_after(p)) {
            return Err(AdmitError::OwnPayloadMissing(own));
        }
        let mut held = Vec::new();
        let mut missing = Vec::new();
        for p in &pins {
            if held_after(p) {
                held.push(p.target.clone());
            } else {
                missing.push(p.target.clone());
            }
        }

        // Publish: the blobs, then the manifest and its signature, then the prune.
        for c in &kept {
            let to = blob_dir.join(&c.sha256);
            std::fs::rename(&c.tmp, &to).map_err(|e| io_err("rename", &to, e))?;
        }
        cleanup.0.clear();
        crate::util::fs::fsync_dir(&blob_dir).map_err(|e| AdmitError::Io(format!("{e:#}")))?;
        for (name, bytes) in [
            (MANIFEST_NAME, manifest_bytes),
            (SIGNATURE_NAME, signature.as_bytes()),
        ] {
            crate::util::fs::atomic_write(&self.dir.join(name), bytes, FILE_MODE)
                .map_err(|e| AdmitError::Io(format!("{e:#}")))?;
        }
        let keep: BTreeSet<String> = pins.iter().map(|p| p.sha256.clone()).collect();
        prune(&blob_dir, &keep);
        self.generation.fetch_add(1, Ordering::Relaxed);
        Ok(Admitted {
            release: m.release(),
            held,
            missing,
            unpinned,
        })
    }

    /// Drop every held blob the held manifest does not pin for one of `targets` (the relay
    /// settings route calls it after `origin.targets` changed; the own target is always listed
    /// there, §6.3). Returns the bytes removed. Blocking.
    pub fn retain_targets(&self, targets: &[String]) -> u64 {
        let _w = self.write.lock().unwrap_or_else(|p| p.into_inner());
        let blob_dir = self.dir.join(BLOB_DIR);
        let keep: BTreeSet<String> = match self.held_manifest() {
            Some(m) => pins_for(&m, targets)
                .into_iter()
                .map(|p| p.sha256)
                .collect(),
            None => BTreeSet::new(),
        };
        let removed = prune(&blob_dir, &keep);
        if removed > 0 {
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
        removed
    }

    /// The held manifest, parsed (it passed `parse_verified` when it was admitted).
    fn held_manifest(&self) -> Option<ChannelManifest> {
        let bytes = read_capped(&self.dir.join(MANIFEST_NAME), MANIFEST_MAX_BYTES as u64)?;
        ChannelManifest::parse(std::str::from_utf8(&bytes).ok()?).ok()
    }

    /// The size of `state/origin/` in bytes (`RelayStatus.origin.bytes`): the regular files in
    /// it and in `blob/`, walked on demand and cached until the directory changes. Blocking (a
    /// few `stat`s; a walk only after a change).
    pub fn bytes(&self) -> u64 {
        let stamp = self.stamp();
        if let Ok(c) = self.cache.lock() {
            if let Some((s, n)) = c.as_ref() {
                if *s == stamp {
                    return *n;
                }
            }
        }
        let n = walk_bytes(&self.dir) + walk_bytes(&self.dir.join(BLOB_DIR));
        if let Ok(mut c) = self.cache.lock() {
            *c = Some((stamp, n));
        }
        n
    }

    fn stamp(&self) -> Stamp {
        let meta = |p: PathBuf| {
            std::fs::symlink_metadata(p)
                .ok()
                .map(|m| (m.mtime(), m.mtime_nsec(), m.ino(), m.len()))
        };
        Stamp {
            generation: self.generation.load(Ordering::Relaxed),
            files: [
                meta(self.dir.clone()),
                meta(self.dir.join(BLOB_DIR)),
                meta(self.dir.join(MANIFEST_NAME)),
                meta(self.dir.join(SIGNATURE_NAME)),
            ],
        }
    }
}

/// The size cache's key: equality only (no ordering decision rides a file time).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
    generation: u64,
    files: [Option<(i64, i64, u64, u64)>; 4],
}

/// Temporary blob files removed on a refusal.
struct TmpGuard(Vec<PathBuf>);

impl Drop for TmpGuard {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn make_dir(dir: &Path) -> Result<(), AdmitError> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(dir)
        .map_err(|e| io_err("create", dir, e))
}

/// Is the pin's blob already held (a regular file of the pinned size)?
fn held_blob(blob_dir: &Path, p: &Pin) -> bool {
    std::fs::symlink_metadata(blob_dir.join(&p.sha256))
        .is_ok_and(|m| m.file_type().is_file() && m.len() == p.size)
}

/// Copy `src` into `blob_dir` under a temporary name, hashing the bytes as they are written.
/// `Some` when the copy's sha256 and size match one of `pins`; `None` (the copy removed) when
/// the file is not regular, its size matches no pin, it changed while read or its hash matches
/// none. Only an I/O failure in `blob_dir` is an error.
fn copy_hashed(src: &Path, blob_dir: &Path, pins: &[Pin]) -> Result<Option<Copied>, AdmitError> {
    let Ok(mut from) = std::fs::File::open(src) else {
        return Ok(None);
    };
    let Ok(meta) = from.metadata() else {
        return Ok(None);
    };
    let size = meta.len();
    if !meta.is_file() || !pins.iter().any(|p| p.size == size) {
        return Ok(None);
    }
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let (mut to, tmp) = loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = blob_dir.join(format!("{TMP_PREFIX}{}-{n}", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(FILE_MODE)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)
        {
            Ok(f) => break (f, tmp),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_err("create", &tmp, e)),
        }
    };
    let mut guard = TmpGuard(vec![tmp.clone()]);
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut total: u64 = 0;
    loop {
        let n = match from.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return Ok(None),
        };
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > size {
            return Ok(None); // grew while read
        }
        hasher.update(&buf[..n]);
        to.write_all(&buf[..n])
            .map_err(|e| io_err("write", &tmp, e))?;
    }
    if total != size {
        return Ok(None);
    }
    let sha256: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if !pins.iter().any(|p| p.sha256 == sha256 && p.size == size) {
        return Ok(None);
    }
    to.sync_all().map_err(|e| io_err("fsync", &tmp, e))?;
    guard.0.clear(); // the caller's guard owns it now
    Ok(Some(Copied { tmp, sha256 }))
}

/// Remove every entry of `blob_dir` not named in `keep` (temporary files included). Returns the
/// bytes removed. Best effort: a file that cannot be removed is left (and never served unless
/// a held pin names it).
fn prune(blob_dir: &Path, keep: &BTreeSet<String>) -> u64 {
    let Ok(rd) = std::fs::read_dir(blob_dir) else {
        return 0;
    };
    let mut removed = 0;
    for e in rd.flatten() {
        let name = e.file_name();
        if name.to_str().is_some_and(|n| keep.contains(n)) {
            continue;
        }
        let len = e.metadata().map(|m| m.len()).unwrap_or(0);
        if std::fs::remove_file(e.path()).is_ok() {
            removed += len;
        }
    }
    removed
}

/// The regular files directly in `dir`, summed (symlinks are not followed).
fn walk_bytes(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.file_type().is_file())
        .map(|m| m.len())
        .sum()
}

/// A regular file of at most `max` bytes, read whole (only the manifest, ≤ 1 MiB, transiently).
fn read_capped(path: &Path, max: u64) -> Option<Vec<u8>> {
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let m = f.metadata().ok()?;
    if !m.is_file() || m.len() > max {
        return None;
    }
    let mut out = Vec::with_capacity(m.len() as usize);
    Read::take(&mut f, max).read_to_end(&mut out).ok()?;
    Some(out)
}

// ---------------------------------------------------------------- the routes

/// `GET /origin/v1/{channel-111-stable.toml, channel-111-stable.toml.minisig, blob/{sha256}}`
/// (§9.1): GET only (axum answers 405 to any other method), outside the session gate. Merged
/// into the dashboard's open router (`dashboard::routes::app`).
pub fn routes() -> axum::Router<Arc<ApiState>> {
    axum::Router::new()
        .route(&format!("{ROUTE_PREFIX}{MANIFEST_NAME}"), get(manifest))
        .route(&format!("{ROUTE_PREFIX}{SIGNATURE_NAME}"), get(signature))
        .route(&format!("{ROUTE_PREFIX}{BLOB_DIR}/{{sha256}}"), get(blob))
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CACHE_CONTROL, "no-store")],
        "not found",
    )
        .into_response()
}

async fn manifest(State(api): State<Arc<ApiState>>) -> Response {
    let path = dir(&api.layout.state_dir()).join(MANIFEST_NAME);
    serve_file(
        &api,
        &path,
        None,
        MANIFEST_MAX_BYTES as u64,
        "application/toml",
    )
    .await
}

async fn signature(State(api): State<Arc<ApiState>>) -> Response {
    let path = dir(&api.layout.state_dir()).join(SIGNATURE_NAME);
    let ctype = "text/plain; charset=utf-8";
    serve_file(&api, &path, None, SIGNATURE_MAX_BYTES as u64, ctype).await
}

async fn blob(State(api): State<Arc<ApiState>>, UrlPath(sha256): UrlPath<String>) -> Response {
    if !serving(&api) || !is_sha256_hex(&sha256) {
        return not_found();
    }
    let origin = dir(&api.layout.state_dir());
    // Only a blob the held manifest pins, at the pinned size.
    let pinned = {
        let path = origin.join(MANIFEST_NAME);
        tokio::task::spawn_blocking(move || {
            let bytes = read_capped(&path, MANIFEST_MAX_BYTES as u64)?;
            let m = ChannelManifest::parse(std::str::from_utf8(&bytes).ok()?).ok()?;
            m.payloads()
                .iter()
                .find(|p| p.sha256 == sha256)
                .map(|p| (p.sha256.clone(), p.size))
        })
        .await
        .ok()
        .flatten()
    };
    let Some((sha256, size)) = pinned else {
        return not_found();
    };
    let path = origin.join(BLOB_DIR).join(&sha256);
    let ctype = "application/octet-stream";
    serve_file(&api, &path, Some(size), PAYLOAD_MAX_BYTES, ctype).await
}

/// Stream a regular file of at most `max` bytes (exactly `exact` when given) while serving;
/// else 404.
async fn serve_file(
    api: &ApiState,
    path: &Path,
    exact: Option<u64>,
    max: u64,
    ctype: &'static str,
) -> Response {
    if !serving(api) {
        return not_found();
    }
    let Ok(file) = tokio::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .await
    else {
        return not_found();
    };
    let Ok(meta) = file.metadata().await else {
        return not_found();
    };
    let len = meta.len();
    if !meta.is_file() || len > max || exact.is_some_and(|e| e != len) {
        return not_found();
    }
    let cache = if exact.is_some() {
        // content-addressed
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut resp = Response::new(stream_body(file, len));
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    resp
}

/// The first `len` bytes of `file`, in [`CHUNK`]-sized frames as the client reads them.
fn stream_body(file: tokio::fs::File, len: u64) -> Body {
    use tokio::io::AsyncReadExt;
    let reader = Some(file.take(len));
    let stream = futures_util::stream::unfold(reader, |reader| async move {
        let mut r = reader?;
        let mut buf = vec![0u8; CHUNK];
        match r.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<Bytes, std::io::Error>(Bytes::from(buf)), Some(r)))
            }
            Err(e) => Some((Err(e), None)),
        }
    });
    Body::from_stream(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    const R1: &str = "2026.10.12";
    const R2: &str = "2026.11.02";
    const X86: &str = "x86_64-unknown-linux-musl";
    const ARM: &str = "aarch64-unknown-linux-musl";

    fn fixture(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/l7-D7/{rel}"))
    }

    fn release(rel: &str) -> (Vec<u8>, String) {
        (
            std::fs::read(fixture(&format!("{rel}/{MANIFEST_NAME}"))).unwrap(),
            std::fs::read_to_string(fixture(&format!("{rel}/{SIGNATURE_NAME}"))).unwrap(),
        )
    }

    fn payload(rel: &str, arch: &str) -> PathBuf {
        fixture(&format!("payload/{rel}-{arch}.bin"))
    }

    fn trust() -> Trust<'static> {
        Trust {
            pubkey: crate::daemon::updater::MANIFEST_PUBKEY.trim(),
            floor: Floor::initial(),
        }
    }

    fn own() -> Vec<String> {
        vec![X86.to_string()]
    }

    fn sha(p: &Path) -> String {
        crate::util::hash::sha256_file(p).unwrap()
    }

    fn blobs(o: &Origin) -> BTreeSet<String> {
        std::fs::read_dir(o.dir().join(BLOB_DIR))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn admits_verified_bytes_and_the_own_payload() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        let (m, s) = release(R1);
        let p = payload(R1, "x86_64");
        let a = o.admit_with(&trust(), &own(), &m, &s, &[&p]).unwrap();
        assert_eq!(a.release.to_string(), R1);
        assert_eq!(a.held, own());
        assert!(a.missing.is_empty() && a.unpinned.is_empty());
        assert_eq!(std::fs::read(o.dir().join(MANIFEST_NAME)).unwrap(), m);
        assert_eq!(
            std::fs::read_to_string(o.dir().join(SIGNATURE_NAME)).unwrap(),
            s
        );
        let name = sha(&p);
        assert_eq!(blobs(&o), BTreeSet::from([name.clone()]));
        assert_eq!(
            std::fs::read(o.dir().join(BLOB_DIR).join(&name)).unwrap(),
            std::fs::read(&p).unwrap()
        );
    }

    #[test]
    fn unverified_bytes_are_refused_and_nothing_is_written() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        let (mut m, s) = release(R1);
        let p = payload(R1, "x86_64");
        // one byte changed after signing
        let at = m.iter().position(|b| *b == b'1').unwrap();
        m[at] = b'2';
        let e = o.admit_with(&trust(), &own(), &m, &s, &[&p]).unwrap_err();
        assert!(matches!(e, AdmitError::Unverified(_)), "{e}");
        // a signature from another release
        let (m1, _) = release(R1);
        let (_, s2) = release(R2);
        let e = o.admit_with(&trust(), &own(), &m1, &s2, &[&p]).unwrap_err();
        assert!(matches!(e, AdmitError::Unverified(_)), "{e}");
        // a key that is not the compiled one
        let other = std::fs::read_to_string(fixture("../minisign/other.pub")).unwrap();
        let t = Trust {
            pubkey: other.lines().last().unwrap().trim(),
            floor: Floor::initial(),
        };
        let e = o
            .admit_with(&t, &own(), &m1, &release(R1).1, &[&p])
            .unwrap_err();
        assert!(matches!(e, AdmitError::Unverified(_)), "{e}");
        assert!(!o.dir().exists(), "nothing written");
        assert_eq!(o.bytes(), 0);
    }

    #[test]
    fn the_floor_refuses_a_release_below_it() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        let mut floor = Floor::initial();
        floor.highest_release = R2.into();
        let t = Trust {
            pubkey: crate::daemon::updater::MANIFEST_PUBKEY.trim(),
            floor,
        };
        let (m, s) = release(R1);
        let e = o
            .admit_with(&t, &own(), &m, &s, &[&payload(R1, "x86_64")])
            .unwrap_err();
        assert!(matches!(e, AdmitError::Unverified(_)), "{e}");
        assert!(!o.dir().exists());
    }

    #[test]
    fn a_blob_no_pin_names_is_not_stored() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        let (m, s) = release(R1);
        // the right size, the wrong bytes
        let p = payload(R1, "x86_64");
        let mut junk = std::fs::read(&p).unwrap();
        junk[0] ^= 1;
        let jp = d.path().join("junk.bin");
        std::fs::write(&jp, &junk).unwrap();
        // the next release's payload: pinned by another manifest only
        let other = payload(R2, "x86_64");
        let a = o
            .admit_with(&trust(), &own(), &m, &s, &[&jp, &other, &p])
            .unwrap();
        assert_eq!(a.unpinned, vec![jp.clone(), other.clone()]);
        assert_eq!(
            blobs(&o),
            BTreeSet::from([sha(&p)]),
            "no temporary file left"
        );
    }

    #[test]
    fn the_own_payload_is_required() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        let (m, s) = release(R1);
        let e = o.admit_with(&trust(), &own(), &m, &s, &[]).unwrap_err();
        assert!(
            matches!(e, AdmitError::OwnPayloadMissing(ref t) if t == X86),
            "{e}"
        );
        // another target's payload alone does not stand in for it
        let both = vec![X86.to_string(), ARM.to_string()];
        let e = o
            .admit_with(&trust(), &both, &m, &s, &[&payload(R1, "aarch64")])
            .unwrap_err();
        assert!(matches!(e, AdmitError::OwnPayloadMissing(_)), "{e}");
        assert!(!o.dir().join(MANIFEST_NAME).exists());
        assert!(blobs(&o).is_empty(), "the refused copies are removed");
    }

    #[test]
    fn other_targets_only_when_listed() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        let (m, s) = release(R1);
        let (x, a) = (payload(R1, "x86_64"), payload(R1, "aarch64"));
        let r = o.admit_with(&trust(), &own(), &m, &s, &[&x, &a]).unwrap();
        assert_eq!(r.unpinned, vec![a.clone()]);
        assert_eq!(blobs(&o), BTreeSet::from([sha(&x)]));
        let both = vec![X86.to_string(), ARM.to_string()];
        let r = o.admit_with(&trust(), &both, &m, &s, &[&a]).unwrap();
        assert_eq!(r.held, both, "the own payload already held counts");
        assert_eq!(blobs(&o), BTreeSet::from([sha(&x), sha(&a)]));
        // the target is dropped from origin.targets: its payload goes
        assert_eq!(
            o.retain_targets(&own()),
            std::fs::metadata(&a).unwrap().len()
        );
        assert_eq!(blobs(&o), BTreeSet::from([sha(&x)]));
        // a served target with no blob given is reported missing
        let d2 = tempfile::tempdir().unwrap();
        let o2 = Origin::new(d2.path());
        let r = o2.admit_with(&trust(), &both, &m, &s, &[&x]).unwrap();
        assert_eq!((r.held, r.missing), (own(), vec![ARM.to_string()]));
    }

    #[test]
    fn a_newer_release_replaces_and_prunes_an_older_one_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        let (m1, s1) = release(R1);
        let (m2, s2) = release(R2);
        o.admit_with(&trust(), &own(), &m1, &s1, &[&payload(R1, "x86_64")])
            .unwrap();
        let p2 = payload(R2, "x86_64");
        let a = o.admit_with(&trust(), &own(), &m2, &s2, &[&p2]).unwrap();
        assert_eq!(a.release.to_string(), R2);
        assert_eq!(
            blobs(&o),
            BTreeSet::from([sha(&p2)]),
            "the old payload pruned"
        );
        assert_eq!(std::fs::read(o.dir().join(MANIFEST_NAME)).unwrap(), m2);
        let e = o
            .admit_with(&trust(), &own(), &m1, &s1, &[&payload(R1, "x86_64")])
            .unwrap_err();
        assert!(matches!(e, AdmitError::Older { .. }), "{e}");
        assert_eq!(std::fs::read(o.dir().join(MANIFEST_NAME)).unwrap(), m2);
        assert_eq!(blobs(&o), BTreeSet::from([sha(&p2)]));
        // the same release again is idempotent
        o.admit_with(&trust(), &own(), &m2, &s2, &[&p2]).unwrap();
        assert_eq!(blobs(&o), BTreeSet::from([sha(&p2)]));
    }

    #[test]
    fn bytes_walks_on_demand_and_follows_changes() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        assert_eq!(o.bytes(), 0);
        let (m, s) = release(R1);
        let p = payload(R1, "x86_64");
        o.admit_with(&trust(), &own(), &m, &s, &[&p]).unwrap();
        let want = m.len() as u64 + s.len() as u64 + std::fs::metadata(&p).unwrap().len();
        assert_eq!(o.bytes(), want);
        assert_eq!(o.bytes(), want, "cached");
        // a write by another process (no generation bump here) is seen through the metadata
        let other = Origin::new(d.path());
        let (m2, s2) = release(R2);
        let p2 = payload(R2, "x86_64");
        other
            .admit_with(&trust(), &own(), &m2, &s2, &[&p2])
            .unwrap();
        let want2 = m2.len() as u64 + s2.len() as u64 + std::fs::metadata(&p2).unwrap().len();
        assert_eq!(o.bytes(), want2);
    }

    #[test]
    fn sha_names() {
        assert!(is_sha256_hex(&"a".repeat(64)));
        assert!(!is_sha256_hex(&"A".repeat(64)));
        assert!(!is_sha256_hex(&"a".repeat(63)));
        assert!(!is_sha256_hex(&format!("../{}", "a".repeat(61))));
    }
}
