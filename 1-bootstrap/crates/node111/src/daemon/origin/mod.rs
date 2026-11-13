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
//!   they are copied. A blob already held counts for a new manifest only after it is hashed
//!   again (a mismatch removes it and reports its target missing). A blob request is answered
//!   only when the held manifest pins that sha256 with the file's exact size, so a stray file
//!   under `blob/` is never served. Source blobs are opened as served ones are
//!   (`O_NOFOLLOW | O_NONBLOCK`, regular files only), so a FIFO or a symlink handed to
//!   [`Origin::admit`] never blocks or redirects the store.
//! - **The pair (§9.1).** The manifest and its signature are written to temporary names first,
//!   then renamed signature first, manifest second; a failure before the manifest's rename puts
//!   the previous signature back (kept under a hard link, or a synced copy where the state
//!   directory refuses `link()`), so the previous pair stays served. A pair that does not verify
//!   together (a crash between the two renames) is never served: the routes answer as when no
//!   release is held until the next admission replaces it ([`Origin::pair_served`], checked
//!   where the routes first open the origin and again only when either file changed).
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
/// The open flags of every file this module reads (served or admitted): never a symlink, never
/// a blocking open (a FIFO), never inherited.
const READ_FLAGS: i32 = libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
/// The longest parser detail logged (debug builds only; [`AdmitError`] carries none).
const DETAIL_MAX: usize = 160;
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
    /// The manifest and signature did not pass `parse_verified` (or the floor is unreadable):
    /// one fixed reason per failure class ([`UNVERIFIED_SIZE`], [`UNVERIFIED_FLOOR`],
    /// [`UNVERIFIED_SIGNATURE`]), never the parser's text (it can quote the offered bytes; a
    /// debug build logs it, escaped and capped).
    Unverified(&'static str),
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

/// [`AdmitError::Unverified`]: the manifest or its signature is over its size cap.
pub const UNVERIFIED_SIZE: &str = "the manifest or its signature is over its size cap";
/// [`AdmitError::Unverified`]: this node's floor (`state/floor.json`) could not be read.
pub const UNVERIFIED_FLOOR: &str = "this node's update floor is unreadable";
/// [`AdmitError::Unverified`]: `parse_verified` refused the pair (signature, keychain, channel,
/// floor or format).
pub const UNVERIFIED_SIGNATURE: &str =
    "the manifest and its signature do not verify under the compiled key and this node's floor";

/// The detail behind a fixed reason: logged only in a debug build, escaped (no control byte or
/// newline reaches the log) and capped at [`DETAIL_MAX`] characters.
fn debug_detail(reason: &'static str, detail: &anyhow::Error) {
    if cfg!(debug_assertions) {
        eprintln!("111 origin: {reason}: {}", log_safe(&format!("{detail:#}")));
    }
}

/// `s` escaped (`char::escape_debug`) and capped at [`DETAIL_MAX`] characters.
fn log_safe(s: &str) -> String {
    let mut out: String = s
        .chars()
        .flat_map(char::escape_debug)
        .take(DETAIL_MAX)
        .collect();
    if s.chars()
        .flat_map(char::escape_debug)
        .nth(DETAIL_MAX)
        .is_some()
    {
        out.push('…');
    }
    out
}

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
    /// The compiled key the held pair is checked under ([`Origin::pair_served`]).
    pubkey: &'static str,
    /// Whether the held pair verifies, keyed on the two files' identity: one verify per pair
    /// seen (row D5c item 4).
    pair: Mutex<Option<(PairStamp, bool)>>,
    /// Tests: pair verifies run (the "once per pair" proof, row D5c item 4).
    #[cfg(test)]
    pair_checks: AtomicU64,
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
            pubkey: crate::daemon::updater::MANIFEST_PUBKEY.trim(),
            pair: Mutex::new(None),
            #[cfg(test)]
            pair_checks: AtomicU64::new(0),
        }
    }

    /// The daemon's one store (`ApiState` ext `Arc<Origin>`), registered on first use under the
    /// ext's write lock: two first callers share one `Origin` and so one write mutex.
    pub fn shared(api: &ApiState) -> Arc<Origin> {
        api.ext_or_insert_with(|| Arc::new(Origin::new(&api.layout.state_dir())))
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
            .map_err(|e| {
                debug_detail(UNVERIFIED_FLOOR, &e);
                AdmitError::Unverified(UNVERIFIED_FLOOR)
            })?
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
        self.admit_faulty(
            trust,
            targets,
            manifest_bytes,
            signature,
            blobs,
            &|_| Ok(()),
        )
    }

    /// [`Origin::admit_with`], with `fault` called at each step of the pair's publication (a
    /// test fails it there; production passes one that never fails).
    fn admit_faulty(
        &self,
        trust: &Trust<'_>,
        targets: &[String],
        manifest_bytes: &[u8],
        signature: &str,
        blobs: &[&Path],
        fault: &dyn Fn(PairStep) -> std::io::Result<()>,
    ) -> Result<Admitted, AdmitError> {
        // Sizes before anything is parsed (parse_verified checks them too).
        if manifest_bytes.len() > MANIFEST_MAX_BYTES || signature.len() > SIGNATURE_MAX_BYTES {
            return Err(AdmitError::Unverified(UNVERIFIED_SIZE));
        }
        let m =
            ChannelManifest::parse_verified(manifest_bytes, signature, trust.pubkey, &trust.floor)
                .map_err(|e| {
                    debug_detail(UNVERIFIED_SIGNATURE, &e);
                    AdmitError::Unverified(UNVERIFIED_SIGNATURE)
                })?;
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
        // A blob already held counts only once hashed again (bounded by this manifest's pins
        // for the served targets; a mismatch is removed and its target reported missing).
        let copied = |p: &Pin| kept.iter().any(|c| c.sha256 == p.sha256);
        let mut on_disk: BTreeSet<&str> = BTreeSet::new();
        for p in &pins {
            if !copied(p) && !on_disk.contains(p.sha256.as_str()) && verify_held(&blob_dir, p)? {
                on_disk.insert(&p.sha256);
            }
        }
        let held_after = |p: &Pin| copied(p) || on_disk.contains(p.sha256.as_str());
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
        publish_pair(&self.dir, manifest_bytes, signature.as_bytes(), fault)?;
        // The pair just published verified (under the node's floor too): no second verify.
        *lock(&self.pair) = Some((self.pair_stamp(), true));
        let keep: BTreeSet<String> = pins.iter().map(|p| p.sha256.clone()).collect();
        prune(&blob_dir, &keep);
        prune_tmp(&self.dir);
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

    /// Is the held pair one to serve (§9.1; row D5c item 4)? The manifest and its signature
    /// must verify together under the compiled key (`parse_verified`: the keychain walk, the
    /// signature over these bytes, the channel). A crash between the two renames of
    /// [`publish_pair`] leaves the new signature beside the old manifest: that torn pair is
    /// never served (every route answers 404, as when no release is held) and the next
    /// admission replaces it. No pair held ⇒ `false`.
    ///
    /// Checked where the routes first open the origin and then only when either file changed
    /// (their device, inode, length, mtime and ctime, compared for equality only): one verify
    /// of a pair the routes read anyway, no task, no timer. An admission here records its own
    /// pair as verified. The node's floor is not applied: it rises when this node updates and
    /// gates admission (§9.5 T4), not what is already held. Blocking (two `lstat`s; the verify
    /// reads the pair).
    pub fn pair_served(&self) -> bool {
        let stamp = self.pair_stamp();
        if let Some((held, ok)) = *lock(&self.pair) {
            if held == stamp {
                return ok;
            }
        }
        let ok = self.pair_verifies();
        if self.pair_stamp() != stamp {
            return false; // changed while read: fail closed, the next call checks again
        }
        *lock(&self.pair) = Some((stamp, ok));
        ok
    }

    fn pair_verifies(&self) -> bool {
        #[cfg(test)]
        self.pair_checks.fetch_add(1, Ordering::Relaxed);
        let man = read_capped(&self.dir.join(MANIFEST_NAME), MANIFEST_MAX_BYTES as u64);
        let sig = read_capped(&self.dir.join(SIGNATURE_NAME), SIGNATURE_MAX_BYTES as u64);
        let (Some(man), Some(sig)) = (man, sig) else {
            return false;
        };
        let Ok(sig) = std::str::from_utf8(&sig) else {
            return false;
        };
        ChannelManifest::parse_verified(&man, sig, self.pubkey, &Floor::initial()).is_ok()
    }

    fn pair_stamp(&self) -> PairStamp {
        let meta = |name: &str| {
            std::fs::symlink_metadata(self.dir.join(name))
                .ok()
                .map(|m| FileStamp {
                    dev: m.dev(),
                    ino: m.ino(),
                    len: m.len(),
                    mtime: (m.mtime(), m.mtime_nsec()),
                    ctime: (m.ctime(), m.ctime_nsec()),
                })
        };
        PairStamp([meta(MANIFEST_NAME), meta(SIGNATURE_NAME)])
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

/// One file of the held pair as last seen: compared for equality only (no ordering decision
/// rides a file time).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

/// The manifest's and the signature's [`FileStamp`] (`None`: absent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PairStamp([Option<FileStamp>; 2]);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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

/// Open `path` for reading as every file here is opened ([`READ_FLAGS`]).
fn open_read(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(READ_FLAGS)
        .open(path)
}

/// The sha256 (lowercase hex) of exactly `size` bytes read from `f`: `Ok(None)` when it holds
/// more or fewer bytes (a complete read proved it) or `sink` refused a chunk; `Err` when a read
/// failed (an interrupted read is retried), so the caller can tell a proven mismatch from an
/// I/O error (row D5c item 2).
fn sha256_exact(
    f: &mut impl Read,
    size: u64,
    mut sink: impl FnMut(&[u8]) -> bool,
) -> std::io::Result<Option<String>> {
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut total: u64 = 0;
    loop {
        let n = match f.read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > size {
            return Ok(None); // grew while read
        }
        hasher.update(&buf[..n]);
        if !sink(&buf[..n]) {
            return Ok(None);
        }
    }
    Ok((total == size).then(|| {
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }))
}

/// Is the pin's blob held, and do its bytes still hash to the pin (row D5b: a held blob is
/// hashed again before it counts)? Blocking.
///
/// The file is removed only on a proven mismatch (row D5c item 2): a symlink at the pinned
/// name (never followed), a file that is not regular, a wrong size, or a complete read whose
/// sha256 differs. An open or read error (EMFILE, EIO, EACCES, …) proves nothing about the
/// bytes: it is returned as [`AdmitError::Io`] and the file stays (a good blob the served
/// manifest may pin too is never destroyed by a transient error). Absent ⇒ `Ok(false)`.
fn verify_held(blob_dir: &Path, p: &Pin) -> Result<bool, AdmitError> {
    verify_held_with(blob_dir, p, |path| {
        let f = open_read(path)?;
        let m = f.metadata()?;
        Ok((m, f))
    })
}

/// [`verify_held`] with the open step given (`open` returns the opened file's metadata and a
/// reader; a test fails it or its reads).
fn verify_held_with<R: Read>(
    blob_dir: &Path,
    p: &Pin,
    open: impl FnOnce(&Path) -> std::io::Result<(std::fs::Metadata, R)>,
) -> Result<bool, AdmitError> {
    let path = blob_dir.join(&p.sha256);
    let (meta, mut f) = match open(&path) {
        Ok(opened) => opened,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        // O_NOFOLLOW on a symlink: proven not the pinned regular file
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) && is_symlink(&path) => {
            let _ = std::fs::remove_file(&path);
            return Ok(false);
        }
        Err(e) => return Err(io_err("open", &path, e)),
    };
    let proven_match = meta.is_file()
        && meta.len() == p.size
        && sha256_exact(&mut f, p.size, |_| true)
            .map_err(|e| io_err("read", &path, e))?
            .is_some_and(|h| h == p.sha256);
    drop(f);
    if !proven_match {
        let _ = std::fs::remove_file(&path);
    }
    Ok(proven_match)
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// Copy `src` into `blob_dir` under a temporary name, hashing the bytes as they are written.
/// `Some` when the copy's sha256 and size match one of `pins`; `None` (the copy removed) when
/// the file is not regular, its size matches no pin, it changed while read or its hash matches
/// none. Only an I/O failure in `blob_dir` is an error.
fn copy_hashed(src: &Path, blob_dir: &Path, pins: &[Pin]) -> Result<Option<Copied>, AdmitError> {
    // As the serving side opens: no symlink, no blocking open (a FIFO), regular files only.
    let Ok(mut from) = open_read(src) else {
        return Ok(None);
    };
    let Ok(meta) = from.metadata() else {
        return Ok(None);
    };
    let size = meta.len();
    if !meta.is_file() || !pins.iter().any(|p| p.size == size) {
        return Ok(None);
    }
    let (mut to, tmp) = create_tmp(blob_dir)?;
    let mut guard = TmpGuard(vec![tmp.clone()]);
    let mut write_err = None;
    let hashed = sha256_exact(&mut from, size, |chunk| match to.write_all(chunk) {
        Ok(()) => true,
        Err(e) => {
            write_err = Some(e);
            false
        }
    });
    if let Some(e) = write_err {
        return Err(io_err("write", &tmp, e));
    }
    // A source that cannot be read whole is not admitted (a foreign file's error is not the
    // store's): reported unpinned, as before.
    let Ok(Some(sha256)) = hashed else {
        return Ok(None);
    };
    if !pins.iter().any(|p| p.sha256 == sha256 && p.size == size) {
        return Ok(None);
    }
    to.sync_all().map_err(|e| io_err("fsync", &tmp, e))?;
    guard.0.clear(); // the caller's guard owns it now
    Ok(Some(Copied { tmp, sha256 }))
}

/// The steps of [`publish_pair`] a test may fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PairStep {
    /// Both temporary files are written and synced; nothing renamed yet.
    Written,
    /// The previous signature is about to be kept under a hard link (a test fails it as a
    /// state directory that refuses `link()` would, so the copy is taken instead).
    Link,
    /// The new signature is in place; the manifest is not.
    SignatureRenamed,
}

/// A new file under `dir` with a temporary name (`.tmp-…`, never one of the served names),
/// created exclusively, 0600, never through a symlink.
fn create_tmp(dir: &Path) -> Result<(std::fs::File, PathBuf), AdmitError> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!("{TMP_PREFIX}{}-{n}", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(FILE_MODE)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&tmp)
        {
            Ok(f) => return Ok((f, tmp)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_err("create", &tmp, e)),
        }
    }
}

/// `bytes` in a synced temporary file under `dir` (removed by `guard` unless renamed).
fn write_tmp(dir: &Path, bytes: &[u8], guard: &mut TmpGuard) -> Result<PathBuf, AdmitError> {
    let (mut f, tmp) = create_tmp(dir)?;
    guard.0.push(tmp.clone());
    f.write_all(bytes).map_err(|e| io_err("write", &tmp, e))?;
    f.sync_all().map_err(|e| io_err("fsync", &tmp, e))?;
    Ok(tmp)
}

/// Publish the manifest and its signature as one pair (§9.1, row D5b): both are written to
/// temporary names and synced first; then the signature is renamed into place, then the
/// manifest. The previous signature is kept until the manifest's rename succeeded: a failure
/// before it puts the previous signature back, so the previous pair (or none) stays served. A
/// reader between the two renames may meet the new signature beside the old manifest;
/// `parse_verified` refuses that pair and the reader tries again.
///
/// The previous signature is kept under a hard link where the state directory allows one, and
/// otherwise (row D5c item 1: Android app storage under Termux, FUSE, vfat or exFAT refuse
/// `link()` with EPERM, EACCES, ENOTSUP, …) as a copy: its bytes (at most
/// [`SIGNATURE_MAX_BYTES`], read `O_NOFOLLOW`) written to a temporary name in the same
/// directory (exclusive, `O_NOFOLLOW`, 0600) and synced before the first rename, so putting it
/// back is again one `rename` on the same filesystem. Only "already exists" (a race for the
/// temporary name) is an error. Either way the crash-safety argument is the same: until the
/// signature's rename the served pair is untouched; after it, an in-process failure renames the
/// kept signature back (the kept bytes are the previous signature's, since this process's
/// write mutex is held and another writer takes `state/staging.lock`); a crash between the two
/// renames leaves a torn pair that is never served ([`Origin::pair_served`]) and that the next
/// admission replaces.
fn publish_pair(
    dir: &Path,
    manifest: &[u8],
    signature: &[u8],
    fault: &dyn Fn(PairStep) -> std::io::Result<()>,
) -> Result<(), AdmitError> {
    let (sig_path, man_path) = (dir.join(SIGNATURE_NAME), dir.join(MANIFEST_NAME));
    let mut guard = TmpGuard(Vec::new());
    let tmp_man = write_tmp(dir, manifest, &mut guard)?;
    let tmp_sig = write_tmp(dir, signature, &mut guard)?;
    fault(PairStep::Written).map_err(|e| io_err("publish", dir, e))?;
    // The previous signature, kept until the manifest is in place.
    let previous = match std::fs::symlink_metadata(&sig_path) {
        Ok(m) if m.is_file() => Some(keep_previous(dir, &sig_path, &mut guard, fault)?),
        _ => None,
    };
    std::fs::rename(&tmp_sig, &sig_path).map_err(|e| io_err("rename", &sig_path, e))?;
    let second =
        fault(PairStep::SignatureRenamed).and_then(|()| std::fs::rename(&tmp_man, &man_path));
    if let Err(e) = second {
        // Put the previous signature back (or remove the new one when there was none).
        let _ = match &previous {
            Some(keep) => std::fs::rename(keep, &sig_path),
            None => std::fs::remove_file(&sig_path),
        };
        return Err(io_err("rename", &man_path, e));
    }
    drop(guard); // the kept signature and nothing else is left to remove
    crate::util::fs::fsync_dir(dir).map_err(|e| AdmitError::Io(format!("{e:#}")))
}

/// The previous signature at `sig_path`, kept under a temporary name in `dir` (removed by
/// `guard` unless renamed back): a hard link, or a synced copy where `link()` is refused
/// ([`publish_pair`]).
fn keep_previous(
    dir: &Path,
    sig_path: &Path,
    guard: &mut TmpGuard,
    fault: &dyn Fn(PairStep) -> std::io::Result<()>,
) -> Result<PathBuf, AdmitError> {
    let (f, keep) = create_tmp(dir)?;
    drop(f);
    guard.0.push(keep.clone());
    std::fs::remove_file(&keep).map_err(|e| io_err("remove", &keep, e))?;
    match fault(PairStep::Link).and_then(|()| std::fs::hard_link(sig_path, &keep)) {
        Ok(()) => Ok(keep),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(io_err("link", &keep, e)),
        Err(_) => {
            let bytes = read_capped(sig_path, SIGNATURE_MAX_BYTES as u64).ok_or_else(|| {
                AdmitError::Io(format!(
                    "keep {}: the previous signature cannot be read",
                    sig_path.display()
                ))
            })?;
            write_tmp(dir, &bytes, guard)
        }
    }
}

/// Remove the temporary entries (`.tmp-…`) directly in `state/origin/` that a crash left
/// (row D5c item 4; `blob/` has [`prune`]). Called after a publication, under the write mutex.
fn prune_tmp(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name();
        if name.to_str().is_some_and(|n| n.starts_with(TMP_PREFIX))
            && e.file_type().is_ok_and(|t| !t.is_dir())
        {
            let _ = std::fs::remove_file(e.path());
        }
    }
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
    let mut f = open_read(path).ok()?;
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

/// The held pair verifies ([`Origin::pair_served`]); checked after [`serving`], so a node that
/// does not serve never reads it.
async fn pair_served(api: &ApiState) -> bool {
    let origin = Origin::shared(api);
    tokio::task::spawn_blocking(move || origin.pair_served())
        .await
        .unwrap_or(false)
}

async fn manifest(State(api): State<Arc<ApiState>>) -> Response {
    if !serving(&api) || !pair_served(&api).await {
        return not_found();
    }
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
    if !serving(&api) || !pair_served(&api).await {
        return not_found();
    }
    let path = dir(&api.layout.state_dir()).join(SIGNATURE_NAME);
    let ctype = "text/plain; charset=utf-8";
    serve_file(&api, &path, None, SIGNATURE_MAX_BYTES as u64, ctype).await
}

async fn blob(State(api): State<Arc<ApiState>>, UrlPath(sha256): UrlPath<String>) -> Response {
    if !serving(&api) || !is_sha256_hex(&sha256) || !pair_served(&api).await {
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
        .custom_flags(READ_FLAGS)
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

    /// Row D5b: a FIFO handed as a source blob is skipped at once (opened `O_NONBLOCK`, not a
    /// regular file) instead of blocking the store with its write mutex held; a symlink is not
    /// followed. Without the flags the FIFO's open waits for a writer forever.
    #[test]
    fn a_fifo_or_a_symlink_source_never_blocks_or_redirects() {
        let d = tempfile::tempdir().unwrap();
        let fifo = d.path().join("fifo");
        let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path; mkfifo only creates the node.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let p = payload(R1, "x86_64");
        let link = d.path().join("link.bin");
        std::os::unix::fs::symlink(&p, &link).unwrap();
        let state = d.path().join("state");
        let (tx, rx) = std::sync::mpsc::channel();
        let (f, l, pp) = (fifo.clone(), link.clone(), p.clone());
        std::thread::spawn(move || {
            let o = Origin::new(&state);
            let (m, s) = release(R1);
            let _ = tx.send(o.admit_with(&trust(), &own(), &m, &s, &[&f, &l, &pp]));
        });
        let a = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("admit returned (a FIFO source blocked it)")
            .unwrap();
        assert_eq!(a.unpinned, vec![fifo, link]);
        assert_eq!(a.held, own());
    }

    /// Row D5b: the pair is published signature first, manifest second; a failure before the
    /// manifest's rename leaves the previous pair served byte-exact (the previous signature is
    /// put back), and no temporary file stays behind. With no previous pair, nothing is left.
    #[test]
    fn a_failed_publish_leaves_the_previous_pair() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        let (m1, s1) = release(R1);
        let (m2, s2) = release(R2);
        let p1 = payload(R1, "x86_64");
        let p2 = payload(R2, "x86_64");
        let listing = |o: &Origin| -> BTreeSet<String> {
            std::fs::read_dir(o.dir())
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        };
        // No previous pair: a failure after the signature's rename leaves nothing served.
        let fail_at = |at: PairStep| {
            move |step: PairStep| {
                if step == at {
                    Err(std::io::Error::other("injected"))
                } else {
                    Ok(())
                }
            }
        };
        let e = o
            .admit_faulty(
                &trust(),
                &own(),
                &m1,
                &s1,
                &[&p1],
                &fail_at(PairStep::SignatureRenamed),
            )
            .unwrap_err();
        assert!(matches!(e, AdmitError::Io(_)), "{e}");
        assert!(!o.dir().join(MANIFEST_NAME).exists());
        assert!(
            !o.dir().join(SIGNATURE_NAME).exists(),
            "the new signature removed"
        );
        o.admit_with(&trust(), &own(), &m1, &s1, &[&p1]).unwrap();
        for at in [PairStep::Written, PairStep::SignatureRenamed] {
            let e = o
                .admit_faulty(&trust(), &own(), &m2, &s2, &[&p2], &fail_at(at))
                .unwrap_err();
            assert!(matches!(e, AdmitError::Io(_)), "{at:?}: {e}");
            assert_eq!(
                std::fs::read(o.dir().join(MANIFEST_NAME)).unwrap(),
                m1,
                "{at:?}"
            );
            assert_eq!(
                std::fs::read_to_string(o.dir().join(SIGNATURE_NAME)).unwrap(),
                s1,
                "{at:?}: the previous signature is served"
            );
            assert_eq!(
                listing(&o),
                BTreeSet::from([BLOB_DIR.into(), MANIFEST_NAME.into(), SIGNATURE_NAME.into()]),
                "{at:?}: no temporary file left"
            );
            // The previous release's payload is still held (no prune ran).
            assert!(blobs(&o).contains(&sha(&p1)), "{at:?}");
        }
        o.admit_with(&trust(), &own(), &m2, &s2, &[&p2]).unwrap();
        assert_eq!(std::fs::read(o.dir().join(MANIFEST_NAME)).unwrap(), m2);
        assert_eq!(
            std::fs::read_to_string(o.dir().join(SIGNATURE_NAME)).unwrap(),
            s2
        );
        assert_eq!(
            listing(&o),
            BTreeSet::from([BLOB_DIR.into(), MANIFEST_NAME.into(), SIGNATURE_NAME.into()])
        );
    }

    /// The names directly in `state/origin/`.
    fn listing(o: &Origin) -> BTreeSet<String> {
        std::fs::read_dir(o.dir())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect()
    }

    fn served_names() -> BTreeSet<String> {
        BTreeSet::from([BLOB_DIR.into(), MANIFEST_NAME.into(), SIGNATURE_NAME.into()])
    }

    /// Row D5c item 1: a state directory that refuses `link()` (EPERM, as Android app storage
    /// under Termux does) still admits a release after the first: the previous signature is
    /// kept as a synced copy instead. A failure after the signature's rename then puts the
    /// copy back byte-exact; only "already exists" is an error. Without the fallback the
    /// second admission fails with "link".
    #[test]
    fn a_refused_link_keeps_a_copy_and_a_second_admission_lands() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        let (m1, s1) = release(R1);
        let (m2, s2) = release(R2);
        let (p1, p2) = (payload(R1, "x86_64"), payload(R2, "x86_64"));
        let refuse = |kind: std::io::ErrorKind, also: Option<PairStep>| {
            move |step: PairStep| {
                if step == PairStep::Link {
                    Err(match kind {
                        std::io::ErrorKind::AlreadyExists => std::io::Error::from(kind),
                        _ => std::io::Error::from_raw_os_error(libc::EPERM),
                    })
                } else if Some(step) == also {
                    Err(std::io::Error::other("injected"))
                } else {
                    Ok(())
                }
            }
        };
        let eperm = std::io::ErrorKind::PermissionDenied;
        // The first admission links nothing (no previous signature).
        o.admit_faulty(&trust(), &own(), &m1, &s1, &[&p1], &refuse(eperm, None))
            .unwrap();
        // A failure after the signature's rename: the copy is put back byte-exact.
        let e = o
            .admit_faulty(
                &trust(),
                &own(),
                &m2,
                &s2,
                &[&p2],
                &refuse(eperm, Some(PairStep::SignatureRenamed)),
            )
            .unwrap_err();
        assert!(matches!(e, AdmitError::Io(_)), "{e}");
        assert_eq!(std::fs::read(o.dir().join(MANIFEST_NAME)).unwrap(), m1);
        assert_eq!(
            std::fs::read_to_string(o.dir().join(SIGNATURE_NAME)).unwrap(),
            s1,
            "the kept copy is the previous signature"
        );
        assert_eq!(listing(&o), served_names(), "no temporary file left");
        assert!(o.pair_served());
        // "already exists" is a race for the temporary name: an error, nothing changed.
        let exists = std::io::ErrorKind::AlreadyExists;
        let e = o
            .admit_faulty(&trust(), &own(), &m2, &s2, &[&p2], &refuse(exists, None))
            .unwrap_err();
        assert!(
            matches!(e, AdmitError::Io(ref w) if w.contains("link")),
            "{e}"
        );
        assert_eq!(std::fs::read(o.dir().join(MANIFEST_NAME)).unwrap(), m1);
        assert_eq!(listing(&o), served_names());
        // The second admission lands with link() refused.
        let a = o
            .admit_faulty(&trust(), &own(), &m2, &s2, &[&p2], &refuse(eperm, None))
            .unwrap();
        assert_eq!(a.release.to_string(), R2);
        assert_eq!(std::fs::read(o.dir().join(MANIFEST_NAME)).unwrap(), m2);
        assert_eq!(
            std::fs::read_to_string(o.dir().join(SIGNATURE_NAME)).unwrap(),
            s2
        );
        assert_eq!(listing(&o), served_names(), "the copy is removed");
        assert_eq!(blobs(&o), BTreeSet::from([sha(&p2)]));
        assert!(o.pair_served());
    }

    /// A reader that fails its first `fail` reads with `err`, then reads `inner`.
    struct Flaky<R> {
        inner: R,
        fail: u32,
        err: fn() -> std::io::Error,
    }

    impl<R: Read> Read for Flaky<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.fail > 0 {
                self.fail -= 1;
                return Err((self.err)());
            }
            self.inner.read(buf)
        }
    }

    /// Row D5c item 2: a held blob is removed only on a proven mismatch. An open error
    /// (EMFILE) and a read error (EIO) are returned and the file stays; an interrupted read is
    /// retried and the blob counts; a wrong size, a changed byte (a complete read) and a
    /// symlink are proven and removed.
    #[test]
    fn a_held_blob_is_removed_only_on_a_proven_mismatch() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        let (m, s) = release(R1);
        let x = payload(R1, "x86_64");
        o.admit_with(&trust(), &own(), &m, &s, &[&x]).unwrap();
        let blob_dir = o.dir().join(BLOB_DIR);
        let pin = Pin {
            target: X86.into(),
            sha256: sha(&x),
            size: std::fs::metadata(&x).unwrap().len(),
        };
        let at = blob_dir.join(&pin.sha256);
        let good = std::fs::read(&at).unwrap();
        let opened = |fail: u32, err: fn() -> std::io::Error| {
            move |path: &Path| {
                let f = open_read(path)?;
                Ok((
                    f.metadata()?,
                    Flaky {
                        inner: f,
                        fail,
                        err,
                    },
                ))
            }
        };
        // EMFILE at open: an error, the file stays
        let e = verify_held_with(&blob_dir, &pin, |_: &Path| {
            Err::<(std::fs::Metadata, std::fs::File), _>(std::io::Error::from_raw_os_error(
                libc::EMFILE,
            ))
        })
        .unwrap_err();
        assert!(
            matches!(e, AdmitError::Io(ref w) if w.starts_with("open")),
            "{e}"
        );
        assert_eq!(std::fs::read(&at).unwrap(), good, "kept after EMFILE");
        // EIO on a read: an error, the file stays
        let eio = || std::io::Error::from_raw_os_error(libc::EIO);
        let e = verify_held_with(&blob_dir, &pin, opened(1, eio)).unwrap_err();
        assert!(
            matches!(e, AdmitError::Io(ref w) if w.starts_with("read")),
            "{e}"
        );
        assert_eq!(std::fs::read(&at).unwrap(), good, "kept after EIO");
        // an interrupted read is retried
        let eintr = || std::io::Error::from(std::io::ErrorKind::Interrupted);
        assert!(verify_held_with(&blob_dir, &pin, opened(3, eintr)).unwrap());
        assert!(verify_held(&blob_dir, &pin).unwrap());
        // a proven mismatch is removed: one byte changed (a complete read) ...
        let mut bad = good.clone();
        bad[7] ^= 0x40;
        std::fs::write(&at, &bad).unwrap();
        assert!(!verify_held(&blob_dir, &pin).unwrap());
        assert!(!at.exists(), "a changed byte is removed");
        // ... a wrong size ...
        std::fs::write(&at, &good[1..]).unwrap();
        assert!(!verify_held(&blob_dir, &pin).unwrap());
        assert!(!at.exists(), "a wrong size is removed");
        // ... a symlink (never followed); absent is Ok(false)
        std::os::unix::fs::symlink(&x, &at).unwrap();
        assert!(!verify_held(&blob_dir, &pin).unwrap());
        assert!(std::fs::symlink_metadata(&at).is_err() && x.exists());
        assert!(!verify_held(&blob_dir, &pin).unwrap());
    }

    /// Row D5c item 4: a crash between the signature's rename and the manifest's leaves the new
    /// signature beside the old manifest. That pair is not served (by this store and by a
    /// fresh one, as at the next start); the next admission repairs it and removes the
    /// temporary files the crash left in `state/origin/`. Checked once per pair seen.
    #[test]
    fn a_torn_pair_is_not_served_until_the_next_admission() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        assert!(!o.pair_served(), "nothing held");
        let (m1, s1) = release(R1);
        let (m2, s2) = release(R2);
        o.admit_with(&trust(), &own(), &m1, &s1, &[&payload(R1, "x86_64")])
            .unwrap();
        assert!(o.pair_served());
        assert_eq!(
            o.pair_checks.load(Ordering::Relaxed),
            1,
            "the empty store's one check; the admitted pair needs none"
        );
        // the crash: R2's signature renamed into place, R1's manifest still there, the
        // temporary copies left behind
        let sig = o.dir().join(SIGNATURE_NAME);
        let tmp = o.dir().join(format!("{TMP_PREFIX}1-0"));
        std::fs::write(&tmp, &s2).unwrap();
        std::fs::rename(&tmp, &sig).unwrap();
        std::fs::write(o.dir().join(format!("{TMP_PREFIX}1-1")), &m2).unwrap();
        assert!(!o.pair_served(), "the torn pair, seen by the running store");
        let fresh = Origin::new(d.path());
        assert!(!fresh.pair_served(), "the torn pair, at the next start");
        assert!(!fresh.pair_served());
        assert_eq!(fresh.pair_checks.load(Ordering::Relaxed), 1, "one verify");
        // the next admission repairs it
        let p2 = payload(R2, "x86_64");
        fresh
            .admit_with(&trust(), &own(), &m2, &s2, &[&p2])
            .unwrap();
        assert!(fresh.pair_served());
        assert!(o.pair_served(), "a write by another store is seen too");
        assert_eq!(
            listing(&fresh),
            served_names(),
            "the crash's leftovers removed"
        );
        // the old manifest beside the new signature the other way round is refused too
        std::fs::write(o.dir().join(MANIFEST_NAME), &m1).unwrap();
        assert!(!o.pair_served());
    }

    /// Row D5b: a blob already held is hashed again before it counts. A held blob whose bytes
    /// changed (same name, same size) is removed and its target reported missing; for the own
    /// target the admission is refused (and the corrupt bytes are gone, so never served).
    #[test]
    fn a_held_blob_is_hashed_again() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        let (m, s) = release(R1);
        let (x, a) = (payload(R1, "x86_64"), payload(R1, "aarch64"));
        let both = vec![X86.to_string(), ARM.to_string()];
        o.admit_with(&trust(), &both, &m, &s, &[&x, &a]).unwrap();
        let corrupt = |p: &Path| {
            let at = o.dir().join(BLOB_DIR).join(sha(p));
            let mut b = std::fs::read(&at).unwrap();
            b[7] ^= 0x40;
            std::fs::write(&at, b).unwrap();
        };
        // Another target's held blob changed: reported missing and removed.
        corrupt(&a);
        let r = o.admit_with(&trust(), &both, &m, &s, &[&x]).unwrap();
        assert_eq!((r.held, r.missing), (own(), vec![ARM.to_string()]));
        assert_eq!(blobs(&o), BTreeSet::from([sha(&x)]));
        // The own target's held blob changed: refused, and the bad bytes removed.
        corrupt(&x);
        let e = o.admit_with(&trust(), &both, &m, &s, &[]).unwrap_err();
        assert!(matches!(e, AdmitError::OwnPayloadMissing(_)), "{e}");
        assert!(
            blobs(&o).is_empty(),
            "the corrupt blob is not left to be served"
        );
        // An intact held blob still counts without being given again.
        o.admit_with(&trust(), &own(), &m, &s, &[&x]).unwrap();
        let r = o.admit_with(&trust(), &own(), &m, &s, &[]).unwrap();
        assert_eq!(r.held, own());
        // A symlink at a pinned name is removed, never followed.
        let at = o.dir().join(BLOB_DIR).join(sha(&x));
        std::fs::remove_file(&at).unwrap();
        std::os::unix::fs::symlink(&x, &at).unwrap();
        let e = o.admit_with(&trust(), &own(), &m, &s, &[]).unwrap_err();
        assert!(matches!(e, AdmitError::OwnPayloadMissing(_)), "{e}");
        assert!(std::fs::symlink_metadata(&at).is_err());
        assert!(x.exists(), "the target of the link is untouched");
    }

    /// Row D5b: `AdmitError::Unverified` carries a fixed reason per failure class, never the
    /// parser's text (which can quote the offered bytes): whatever was offered, the error's
    /// text is one of the three fixed sentences.
    #[test]
    fn unverified_carries_a_fixed_reason_only() {
        let d = tempfile::tempdir().unwrap();
        let o = Origin::new(d.path());
        let (m, s) = release(R1);
        let p = payload(R1, "x86_64");
        let marker = "\u{1b}[31mEVIL-MARKER";
        let mut bad_m = String::from_utf8(m.clone()).unwrap();
        bad_m.insert_str(0, &format!("evil = \"{marker}\"\n"));
        let bad_s = format!("untrusted comment: {marker}\n{marker}\n");
        let cases: [(&[u8], &str, &str); 4] = [
            (bad_m.as_bytes(), &s, UNVERIFIED_SIGNATURE),
            (&m, &bad_s, UNVERIFIED_SIGNATURE),
            (marker.as_bytes(), marker, UNVERIFIED_SIGNATURE),
            (&m, &"x".repeat(SIGNATURE_MAX_BYTES + 1), UNVERIFIED_SIZE),
        ];
        for (mb, sb, want) in cases {
            let e = o.admit_with(&trust(), &own(), mb, sb, &[&p]).unwrap_err();
            assert!(matches!(e, AdmitError::Unverified(r) if r == want), "{e:?}");
            let shown = format!("{e} {e:?}");
            assert!(
                !shown.contains("EVIL") && !shown.contains('\u{1b}'),
                "{shown}"
            );
            assert_eq!(
                e.to_string(),
                format!("the manifest does not verify: {want}")
            );
        }
        // The debug-only detail is escaped and capped.
        let shown = log_safe(&format!("{marker}\n{}", "y".repeat(400)));
        assert!(
            !shown.contains('\u{1b}') && !shown.contains('\n'),
            "{shown}"
        );
        assert_eq!(shown.chars().count(), DETAIL_MAX + 1);
    }

    /// Row D5b: a body of several frames streams byte-exact, each frame at most [`CHUNK`]
    /// bytes, and a length shorter than the file stops exactly there, across a frame boundary.
    #[tokio::test]
    async fn a_body_streams_over_several_frames() {
        use futures_util::StreamExt;
        async fn frames(path: &Path, len: u64) -> Vec<Bytes> {
            let f = tokio::fs::File::open(path).await.unwrap();
            stream_body(f, len)
                .into_data_stream()
                .map(|r| r.unwrap())
                .collect()
                .await
        }
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("big.bin");
        let want: Vec<u8> = (0..5 * CHUNK + 1_234).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &want).unwrap();
        let got = frames(&path, want.len() as u64).await;
        assert!(got.len() >= 6, "{} frames", got.len());
        assert!(got.iter().all(|f| !f.is_empty() && f.len() <= CHUNK));
        assert_eq!(got.concat(), want, "byte-exact");
        let cut = CHUNK as u64 + 10;
        let got = frames(&path, cut).await;
        assert!(got.len() >= 2, "{} frames", got.len());
        assert_eq!(got.concat(), want[..cut as usize].to_vec());
    }

    #[test]
    fn sha_names() {
        assert!(is_sha256_hex(&"a".repeat(64)));
        assert!(!is_sha256_hex(&"A".repeat(64)));
        assert!(!is_sha256_hex(&"a".repeat(63)));
        assert!(!is_sha256_hex(&format!("../{}", "a".repeat(61))));
    }
}
