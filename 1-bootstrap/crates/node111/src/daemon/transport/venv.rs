//! Hash-locked Python venvs under `/var/lib/111/venvs/`, built with
//! `pip install --require-hashes` from a lockfile whose sha256 the signed
//! channel manifest pins, and marked with that sha256 (`.111-integrity`).
//!
//! - **rnsd venv** (`venvs/rns`, the functions at the top of this module):
//!   integrity check at daemon start; torn venvs rebuild from
//!   `cache/packages/python/` (offline path; index allowed only at install
//!   time).
//! - **presence venv** (`venvs/presence`, [`presence`]): the
//!   `meshtastic-presence` reader's own venv (`os/5-messaging/README.md` §2,
//!   §14, §19 L1), built from the lock the release payload carries and the
//!   manifest pins (`manifest::layer5::PRESENCE_LOCK`); fail closed — a failed
//!   build never removes the venv in place.

use crate::daemon::DaemonLayout;
use crate::util::run::Runner;
use std::path::{Path, PathBuf};

pub fn venv_dir(layout: &DaemonLayout) -> PathBuf {
    layout.root.join("venvs/rns")
}

fn integrity_marker(venv: &Path) -> PathBuf {
    venv.join(".111-integrity")
}

/// The venv's integrity marker holds `want` (a lockfile sha256).
fn marker_is(venv: &Path, want: &str) -> bool {
    std::fs::read_to_string(integrity_marker(venv)).is_ok_and(|got| got.trim() == want)
}

/// `pip install` arguments for a strictly offline, hash-locked install from
/// the prefetched wheel cache.
fn offline_install_args<'a>(lock: &'a str, cache: &'a str) -> [&'a str; 7] {
    [
        "install",
        "--require-hashes",
        "-r",
        lock,
        "--no-index",
        "--find-links",
        cache,
    ]
}

/// Integrity: venv exists, rnsd present, marker sha256 == lockfile sha256.
pub fn integrity_ok(layout: &DaemonLayout, lockfile: &Path) -> bool {
    let venv = venv_dir(layout);
    if !venv.join("bin/rnsd").exists() {
        return false;
    }
    let Ok(want) = crate::util::hash::sha256_file(lockfile) else {
        return false;
    };
    marker_is(&venv, &want)
}

/// Build (or rebuild) the venv. Offline-first: `--no-index --find-links` the
/// prefetched wheel cache; `allow_index` additionally permits PyPI (install
/// time only — the daemon rebuilds strictly from cache).
pub fn build(
    layout: &DaemonLayout,
    runner: &dyn Runner,
    lockfile: &Path,
    cache_py: &Path,
    allow_index: bool,
) -> anyhow::Result<()> {
    let venv = venv_dir(layout);
    if venv.exists() {
        std::fs::remove_dir_all(&venv)?;
    }
    if let Some(parent) = venv.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let venv_s = venv.to_string_lossy().into_owned();
    let out = runner.run("python3", &["-m", "venv", &venv_s])?;
    anyhow::ensure!(out.ok(), "python3 -m venv failed: {}", out.stderr.trim());

    let lock_s = lockfile.to_string_lossy().into_owned();
    let cache_s = cache_py.to_string_lossy().into_owned();
    let pip = format!("{venv_s}/bin/pip");
    let mut args: Vec<&str> = offline_install_args(&lock_s, &cache_s).to_vec();
    if allow_index {
        args = vec!["install", "--require-hashes", "-r", &lock_s];
    }
    let out = runner.run(&pip, &args)?;
    if !out.ok() && !allow_index {
        anyhow::bail!(
            "venv rebuild from cache failed (torn cache?): {}",
            out.stderr.trim()
        );
    }
    if !out.ok() && allow_index {
        anyhow::bail!("venv build failed: {}", out.stderr.trim());
    }

    let lock_sha = crate::util::hash::sha256_file(lockfile)?;
    std::fs::create_dir_all(&venv)?; // defensive (mocked venv runs create nothing)
    crate::util::fs::atomic_write(&integrity_marker(&venv), lock_sha.as_bytes(), 0o644)?;
    Ok(())
}

/// Ensure the venv is intact; rebuild when torn.
pub fn ensure(
    layout: &DaemonLayout,
    runner: &dyn Runner,
    lockfile: &Path,
    cache_py: &Path,
    allow_index: bool,
) -> anyhow::Result<PathBuf> {
    if integrity_ok(layout, lockfile) {
        return Ok(venv_dir(layout));
    }
    build(layout, runner, lockfile, cache_py, allow_index)?;
    Ok(venv_dir(layout))
}

pub fn python3_present(runner: &dyn Runner) -> bool {
    runner
        .run("python3", &["--version"])
        .map(|o| o.ok())
        .unwrap_or(false)
}

/// The `meshtastic-presence` reader's venv (Layer 5 §14: "own source dir +
/// hash-locked venv like `rns-lock.txt`"; §2: venvs under `venvs/`; §19 L1:
/// the presence venv lock is a signed-manifest entry).
///
/// **Inputs.** The lock is `<release>/share/meshtastic-presence/
/// meshtastic-presence-lock.txt` ([`lockfile`]): it rides in the payload and
/// equals the manifest's `[presence-lock]` pin, checked by
/// `ChannelManifest::verify_layer5_tree` wherever a payload is staged or
/// promoted. The wheels come from `cache/packages/presence/` ([`cache_dir`]),
/// root-owned like `cache/packages/python/`.
///
/// **When.** `111 install` (stage step, [`Mode::Install`]): the build venv's
/// own pip downloads the locked closure into the cache — the host's
/// `python3 -m pip` is not needed (Debian's `python3-venv` ships none) — then
/// installs from the cache only. After that, whenever the release in
/// `current/` carries a lock the venv in place was not built from (an update
/// whose lock changed was promoted), [`ensure_current`] rebuilds offline from
/// the cache ([`Mode::Offline`]: nothing downloads at runtime). An unchanged
/// lock runs nothing ([`Outcome::Unchanged`]).
///
/// **Layout and fail-closed swap.** `venvs/presence` is a link to one
/// complete build `venvs/presence.d/<sha16>-<pid>-<n>/` (the reader's
/// `venvs/presence/bin/python` path stays as it was). A build happens in a
/// new directory; only after `pip install --require-hashes` succeeded,
/// `bin/python` exists and the marker is written does the link flip
/// (tmp link + `rename`), and only then are older builds pruned. Any failure
/// — no `python3`, `venv` failing, a hash mismatch, a wheel missing from the
/// cache — removes the unfinished build and leaves the venv in place
/// untouched ([`Outcome::Unavailable`]).
///
/// **Status.** Every ensure records its result in
/// `venvs/presence.status.json` ([`Status`], written only when it changes):
/// the meshtastic service reads [`unavailable_reason`] so a missing or stale
/// venv surfaces as its reason, never as a crash.
///
/// **Links.** The installer runs this as root inside the node111-owned
/// `venvs/` tree, so `venvs` and `venvs/presence.d` must be real
/// directories (never followed when they are links), the build lock is
/// opened `O_NOFOLLOW`, and pruning removes a link itself, never its target.
pub mod presence {
    use super::{integrity_marker, marker_is, offline_install_args, python3_present};
    use crate::daemon::DaemonLayout;
    use crate::manifest::layer5;
    use crate::util::run::Runner;
    use serde::{Deserialize, Serialize};
    use std::path::{Path, PathBuf};

    /// Directory of the complete builds the `venvs/presence` link points into.
    pub const BUILDS: &str = "venvs/presence.d";
    /// [`BUILDS`]' own name: the link target is `presence.d/<id>`, relative.
    const BUILDS_NAME: &str = "presence.d";
    /// Last ensure's result ([`Status`]).
    pub const STATUS: &str = "venvs/presence.status.json";
    /// Wheel cache of the presence closure (root-owned, like `cache/packages/python`).
    pub const CACHE: &str = "cache/packages/presence";
    /// `flock` serialising builds (installer vs daemon), inside `venvs/`.
    const BUILD_LOCK: &str = ".presence.lock";
    /// Prefix of the tmp link renamed over `venvs/presence`.
    const TMP_LINK: &str = ".presence.tmp-";
    /// Longest status file read back.
    const STATUS_MAX: u64 = 64 * 1024;
    /// Longest tool output kept in a reason.
    const DETAIL_MAX: usize = 200;

    /// `venvs/presence` (`manifest::layer5::PRESENCE_VENV`).
    pub fn venv_dir(root: &Path) -> PathBuf {
        root.join(layer5::PRESENCE_VENV)
    }

    /// The reader's interpreter (`MeshtasticConfig::reader_python`).
    pub fn python(root: &Path) -> PathBuf {
        venv_dir(root).join("bin/python")
    }

    /// The lock a release carries (`manifest::layer5::PRESENCE_LOCK`).
    pub fn lockfile(release: &Path) -> PathBuf {
        release.join(layer5::PRESENCE_LOCK)
    }

    /// `cache/packages/presence`.
    pub fn cache_dir(root: &Path) -> PathBuf {
        root.join(CACHE)
    }

    /// `venvs/presence.d`.
    pub fn builds_dir(root: &Path) -> PathBuf {
        root.join(BUILDS)
    }

    /// `venvs/presence.status.json`.
    pub fn status_path(root: &Path) -> PathBuf {
        root.join(STATUS)
    }

    /// The venv in place has `bin/python` and was built from `lock`.
    pub fn integrity_ok(root: &Path, lock: &Path) -> bool {
        crate::util::hash::sha256_file(lock).is_ok_and(|want| built_from(root, &want))
    }

    fn built_from(root: &Path, want: &str) -> bool {
        python(root).is_file() && marker_is(&venv_dir(root), want)
    }

    /// Where the closure's wheels may come from.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Mode {
        /// `111 install` (online): the build venv's pip downloads the locked
        /// closure into the cache, then installs from the cache only.
        Install,
        /// The daemon: the cache only (`--no-index`); nothing downloads.
        Offline,
    }

    /// Why no venv was built for the wanted lock.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum PresenceVenvError {
        /// `python3 --version` did not run.
        NoPython3,
        /// A directory the build writes through is a link or not a directory.
        Unsafe(String),
        /// `python3 -m venv` failed.
        Venv(String),
        /// `pip install --require-hashes` failed: a hash mismatch, a wheel the
        /// cache lacks. `download`: the install-time download's own failure.
        Pip {
            detail: String,
            offline: bool,
            download: Option<String>,
        },
        /// pip succeeded but the build has no `bin/python`.
        Incomplete,
        /// A filesystem step failed.
        Io(String),
    }

    impl PresenceVenvError {
        /// Stable code (`Status::code`).
        pub fn code(&self) -> &'static str {
            match self {
                PresenceVenvError::NoPython3 => "presence_no_python",
                _ => "presence_venv_failed",
            }
        }
    }

    impl std::fmt::Display for PresenceVenvError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                PresenceVenvError::NoPython3 => {
                    write!(f, "python3 is not installed on this machine")
                }
                PresenceVenvError::Unsafe(what) => write!(f, "{what}"),
                PresenceVenvError::Venv(e) => {
                    write!(f, "python3 could not create a virtual environment ({e})")
                }
                PresenceVenvError::Pip {
                    detail,
                    offline: true,
                    ..
                } => write!(
                    f,
                    "pip could not install its locked packages from the offline cache \
                     ({detail}); running `111 install` again downloads them"
                ),
                PresenceVenvError::Pip {
                    detail,
                    offline: false,
                    download,
                } => {
                    write!(f, "pip could not install its locked packages ({detail})")?;
                    match download {
                        Some(d) => write!(f, "; downloading them failed too ({d})"),
                        None => Ok(()),
                    }
                }
                PresenceVenvError::Incomplete => {
                    write!(f, "the build produced no bin/python")
                }
                PresenceVenvError::Io(e) => write!(f, "{e}"),
            }
        }
    }

    impl std::error::Error for PresenceVenvError {}

    fn io(what: &str, path: &Path, e: impl std::fmt::Display) -> PresenceVenvError {
        PresenceVenvError::Io(format!("{what} {}: {e}", path.display()))
    }

    /// One ensure's result.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Outcome {
        /// The release ships no presence reader (no lock): nothing to build.
        NotShipped,
        /// The venv in place was built from this lock: nothing ran.
        Unchanged,
        /// A fresh venv was built from this lock and swapped in.
        Built,
        /// No venv for this lock. `previous_kept`: an earlier venv is still in
        /// place (the reader keeps running on it).
        Unavailable {
            error: PresenceVenvError,
            previous_kept: bool,
        },
    }

    /// `venvs/presence.status.json`: what the last ensure found.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Status {
        pub schema: u32,
        /// A venv built from `lock_sha256` is in place.
        pub ok: bool,
        /// `presence_venv_ready` · `presence_no_python` · `presence_venv_failed`.
        pub code: String,
        /// Plain-language reason when `!ok` (the meshtastic service shows it).
        pub reason: Option<String>,
        /// sha256 of the lock the release in `current/` carries.
        pub lock_sha256: String,
        /// `!ok` and an earlier venv is still in place.
        pub previous_kept: bool,
    }

    impl Status {
        fn ready(lock_sha256: &str) -> Self {
            Status {
                schema: 1,
                ok: true,
                code: "presence_venv_ready".into(),
                reason: None,
                lock_sha256: lock_sha256.into(),
                previous_kept: false,
            }
        }

        fn failed(lock_sha256: &str, error: &PresenceVenvError, previous_kept: bool) -> Self {
            let reason = if previous_kept {
                format!(
                    "The presence reader's environment for this release could not be built \
                     ({error}), so the reader runs on the one built earlier."
                )
            } else {
                format!("The presence reader is off: its environment could not be built ({error}).")
            };
            Status {
                schema: 1,
                ok: false,
                code: error.code().into(),
                reason: Some(reason),
                lock_sha256: lock_sha256.into(),
                previous_kept,
            }
        }
    }

    /// The last recorded status (`None`: never recorded, unreadable or
    /// malformed).
    pub fn status(root: &Path) -> Option<Status> {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(status_path(root))
            .ok()?;
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(STATUS_MAX + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 > STATUS_MAX {
            return None;
        }
        serde_json::from_slice::<Status>(&bytes)
            .ok()
            .filter(|s| s.schema == 1)
    }

    /// Why the reader has no venv for the current release, when the last
    /// ensure recorded a failure (`None` when it is ready or nothing was
    /// recorded).
    pub fn unavailable_reason(root: &Path) -> Option<String> {
        status(root).filter(|s| !s.ok).and_then(|s| s.reason)
    }

    /// Daemon start (and so after every promotion, which restarts the
    /// daemon): ensure the venv matches the lock `current/` carries, offline
    /// from [`cache_dir`]. A release without the reader is
    /// [`Outcome::NotShipped`] and leaves everything as it is.
    pub fn ensure_current(layout: &DaemonLayout, runner: &dyn Runner) -> Outcome {
        let lock = lockfile(&layout.current_release());
        if !lock.is_file() {
            return Outcome::NotShipped;
        }
        ensure(
            &layout.root,
            runner,
            &lock,
            &cache_dir(&layout.root),
            Mode::Offline,
        )
    }

    /// Ensure `venvs/presence` was built from `lock`; build it (fail closed)
    /// when not. Never panics; the outcome is also recorded ([`status`]).
    pub fn ensure(
        root: &Path,
        runner: &dyn Runner,
        lock: &Path,
        cache: &Path,
        mode: Mode,
    ) -> Outcome {
        let want = match crate::util::hash::sha256_file(lock) {
            Ok(sha) => sha,
            Err(e) => {
                let error = io("cannot read the lock", lock, format!("{e:#}"));
                let previous_kept = python(root).is_file();
                return Outcome::Unavailable {
                    error,
                    previous_kept,
                };
            }
        };
        if built_from(root, &want) {
            record(root, &Status::ready(&want));
            return Outcome::Unchanged;
        }
        let built = (|| -> Result<bool, PresenceVenvError> {
            if !python3_present(runner) {
                return Err(PresenceVenvError::NoPython3);
            }
            let venvs = real_dir(root, "venvs")?;
            let _lock = BuildLock::acquire(&venvs)?;
            // another builder may have finished while this one waited
            if built_from(root, &want) {
                return Ok(false);
            }
            build_and_swap(root, &venvs, runner, lock, &want, cache, mode)?;
            Ok(true)
        })();
        match built {
            Ok(fresh) => {
                record(root, &Status::ready(&want));
                if fresh {
                    Outcome::Built
                } else {
                    Outcome::Unchanged
                }
            }
            Err(error) => {
                let previous_kept = python(root).is_file();
                record(root, &Status::failed(&want, &error, previous_kept));
                Outcome::Unavailable {
                    error,
                    previous_kept,
                }
            }
        }
    }

    /// Write `status` unless it is already what is on disk. Best effort: a
    /// status that cannot be written is logged, never fatal.
    fn record(root: &Path, next: &Status) {
        if status(root).as_ref() == Some(next) {
            return;
        }
        let write = || -> Result<(), PresenceVenvError> {
            real_dir(root, "venvs")?;
            let json = serde_json::to_vec(next)
                .map_err(|e| PresenceVenvError::Io(format!("status: {e}")))?;
            let path = status_path(root);
            crate::util::fs::atomic_write(&path, &json, 0o644)
                .map_err(|e| io("cannot write", &path, format!("{e:#}")))
        };
        if let Err(e) = write() {
            eprintln!("111 presence venv: status not recorded: {e}");
        }
    }

    /// `root/<rel>` as a real directory: created when missing, refused when
    /// it is a link or another kind of file (never followed).
    fn real_dir(root: &Path, rel: &str) -> Result<PathBuf, PresenceVenvError> {
        let path = root.join(rel);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_dir() => return Ok(path),
            Ok(_) => {
                return Err(PresenceVenvError::Unsafe(format!(
                    "{} is a link or not a directory; refusing to build there",
                    path.display()
                )))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io("cannot inspect", &path, e)),
        }
        if let Err(e) = std::fs::create_dir(&path) {
            if e.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(io("cannot create", &path, e));
            }
        }
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_dir() => Ok(path),
            _ => Err(PresenceVenvError::Unsafe(format!(
                "{} is a link or not a directory; refusing to build there",
                path.display()
            ))),
        }
    }

    /// Exclusive `flock` on `venvs/.presence.lock` for one build (released on
    /// drop, when the descriptor closes).
    struct BuildLock {
        _file: std::fs::File,
    }

    impl BuildLock {
        fn acquire(venvs: &Path) -> Result<Self, PresenceVenvError> {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::OpenOptionsExt;
            let path = venvs.join(BUILD_LOCK);
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o644)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)
                .map_err(|e| io("cannot open", &path, e))?;
            // SAFETY: `file` owns a valid open descriptor for the whole call.
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if rc != 0 {
                return Err(io("cannot lock", &path, std::io::Error::last_os_error()));
            }
            Ok(BuildLock { _file: file })
        }
    }

    /// [`detail`] of a finished tool: its stderr, else its stdout (`venv`
    /// reports a missing `ensurepip` on stdout).
    fn detail_of(out: &crate::util::run::Output) -> String {
        if out.stderr.trim().is_empty() {
            detail(&out.stdout, out.status)
        } else {
            detail(&out.stderr, out.status)
        }
    }

    /// First non-empty `ERROR:` line of a tool's output (pip's hash mismatch
    /// report is several lines), else its last non-empty line; control
    /// characters dropped, at most [`DETAIL_MAX`] bytes.
    fn detail(text: &str, status: i32) -> String {
        let lines: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let line = lines
            .iter()
            .find(|l| l.starts_with("ERROR:") || l.starts_with("Error:"))
            .or(lines.last())
            .copied();
        let Some(line) = line else {
            return format!("exit status {status}");
        };
        let mut out = String::new();
        for c in line.chars().filter(|c| !c.is_control()) {
            if out.len() + c.len_utf8() > DETAIL_MAX {
                out.push('…');
                break;
            }
            out.push(c);
        }
        out
    }

    /// A fresh, unique build directory under `venvs/presence.d/`.
    fn new_build_dir(builds: &Path, want: &str) -> Result<(PathBuf, String), PresenceVenvError> {
        let pid = std::process::id();
        let tag = want.get(..16).unwrap_or(want);
        for n in 0..64u32 {
            let id = format!("{tag}-{pid}-{n}");
            let dir = builds.join(&id);
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok((dir, id)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io("cannot create", &dir, e)),
            }
        }
        Err(PresenceVenvError::Io(format!(
            "no free build directory under {}",
            builds.display()
        )))
    }

    /// Build into a new directory, then flip `venvs/presence` to it; the venv
    /// in place is touched only by that final atomic rename.
    fn build_and_swap(
        root: &Path,
        venvs: &Path,
        runner: &dyn Runner,
        lock: &Path,
        want: &str,
        cache: &Path,
        mode: Mode,
    ) -> Result<(), PresenceVenvError> {
        let builds = real_dir(root, BUILDS)?;
        sweep_tmp_links(venvs);
        let (dir, id) = new_build_dir(&builds, want)?;
        let result = (|| -> Result<(), PresenceVenvError> {
            let dir_s = dir.to_string_lossy().into_owned();
            let out = runner
                .run("python3", &["-m", "venv", &dir_s])
                .map_err(|e| PresenceVenvError::Venv(format!("{e:#}")))?;
            if !out.ok() {
                return Err(PresenceVenvError::Venv(detail_of(&out)));
            }
            let pip = format!("{dir_s}/bin/pip");
            let lock_s = lock.to_string_lossy().into_owned();
            let cache_s = cache.to_string_lossy().into_owned();
            let mut download = None;
            if mode == Mode::Install {
                std::fs::create_dir_all(cache).map_err(|e| io("cannot create", cache, e))?;
                let args = [
                    "download",
                    "--require-hashes",
                    "-r",
                    &lock_s,
                    "-d",
                    &cache_s,
                ];
                match runner.run(&pip, &args) {
                    Ok(o) if o.ok() => {}
                    Ok(o) => download = Some(detail_of(&o)),
                    Err(e) => download = Some(format!("{e:#}")),
                }
            }
            let out = runner
                .run(&pip, &offline_install_args(&lock_s, &cache_s))
                .map_err(|e| PresenceVenvError::Pip {
                    detail: format!("{e:#}"),
                    offline: mode == Mode::Offline,
                    download: download.clone(),
                })?;
            if !out.ok() {
                return Err(PresenceVenvError::Pip {
                    detail: detail_of(&out),
                    offline: mode == Mode::Offline,
                    download,
                });
            }
            if !dir.join("bin/python").is_file() {
                return Err(PresenceVenvError::Incomplete);
            }
            let marker = integrity_marker(&dir);
            crate::util::fs::atomic_write(&marker, want.as_bytes(), 0o644)
                .map_err(|e| io("cannot write", &marker, format!("{e:#}")))?;
            flip(root, venvs, &builds, &id)
        })();
        match result {
            Ok(()) => {
                prune(&builds, &id);
                Ok(())
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                Err(e)
            }
        }
    }

    /// Point `venvs/presence` at `presence.d/<id>` atomically (tmp link +
    /// `rename`). A real directory in the way (an older hand-made venv) is
    /// first moved into `presence.d/` so the rename can replace it, and moved
    /// back should the swap fail.
    fn flip(root: &Path, venvs: &Path, builds: &Path, id: &str) -> Result<(), PresenceVenvError> {
        let link = venv_dir(root);
        let target = Path::new(BUILDS_NAME).join(id);
        let pid = std::process::id();
        let tmp = venvs.join(format!("{TMP_LINK}{pid}-{id}"));
        let _ = std::fs::remove_file(&tmp);
        std::os::unix::fs::symlink(&target, &tmp).map_err(|e| io("cannot link", &tmp, e))?;
        let mut aside = None;
        if std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_dir()) {
            let to = builds.join(format!("legacy-{pid}-{id}"));
            if let Err(e) = std::fs::rename(&link, &to) {
                let _ = std::fs::remove_file(&tmp);
                return Err(io("cannot move aside", &link, e));
            }
            aside = Some(to);
        }
        if let Err(e) = std::fs::rename(&tmp, &link) {
            let _ = std::fs::remove_file(&tmp);
            if let Some(to) = aside {
                let _ = std::fs::rename(to, &link);
            }
            return Err(io("cannot swap in", &link, e));
        }
        // the swap is done: a failed directory sync must not undo it
        if let Err(e) = crate::util::fs::fsync_dir(venvs) {
            eprintln!("111 presence venv: {e:#}");
        }
        Ok(())
    }

    /// Remove every build but `keep` (a link is removed itself, never its
    /// target). Best effort.
    fn prune(builds: &Path, keep: &str) {
        let Ok(entries) = std::fs::read_dir(builds) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_str() == Some(keep) {
                continue;
            }
            let path = entry.path();
            let removed = match std::fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_dir() => std::fs::remove_dir_all(&path),
                Ok(_) => std::fs::remove_file(&path),
                Err(e) => Err(e),
            };
            if let Err(e) = removed {
                eprintln!("111 presence venv: cannot prune {}: {e}", path.display());
            }
        }
    }

    /// Tmp links a crash left between `symlink` and `rename`.
    fn sweep_tmp_links(venvs: &Path) {
        let Ok(entries) = std::fs::read_dir(venvs) else {
            return;
        };
        for entry in entries.flatten() {
            let is_tmp = entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(TMP_LINK));
            let is_link =
                std::fs::symlink_metadata(entry.path()).is_ok_and(|m| m.file_type().is_symlink());
            if is_tmp && is_link {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn builds_sit_beside_the_venv_link() {
            let venv = Path::new(layer5::PRESENCE_VENV);
            assert_eq!(venv.parent(), Some(Path::new("venvs")));
            assert_eq!(Path::new(BUILDS), Path::new("venvs").join(BUILDS_NAME));
            assert_eq!(
                Path::new(STATUS).parent(),
                Some(Path::new("venvs")),
                "the status survives every build"
            );
        }

        #[test]
        fn detail_prefers_the_error_line_and_bounds_it() {
            let pip = "Collecting x\nERROR: THESE PACKAGES DO NOT MATCH THE HASHES FROM THE REQUIREMENTS FILE.\n    x==1 from y:\n        Expected sha256 a\n             Got        b\n";
            assert_eq!(
                detail(pip, 1),
                "ERROR: THESE PACKAGES DO NOT MATCH THE HASHES FROM THE REQUIREMENTS FILE."
            );
            assert_eq!(detail("warn\nlast line\n", 1), "last line");
            assert_eq!(detail("", 3), "exit status 3");
            let long = format!("ERROR: {}\u{7}", "é".repeat(300));
            let d = detail(&long, 1);
            assert!(d.len() <= DETAIL_MAX + '…'.len_utf8(), "{}", d.len());
            assert!(d.ends_with('…') && !d.contains('\u{7}'));
        }

        #[test]
        fn reasons_say_whether_the_previous_venv_stays() {
            let kept = Status::failed("s", &PresenceVenvError::NoPython3, true);
            assert_eq!(kept.code, "presence_no_python");
            assert!(kept
                .reason
                .as_deref()
                .is_some_and(|r| r.contains("runs on the one built earlier")
                    && r.contains("python3 is not installed")));
            let off = Status::failed(
                "s",
                &PresenceVenvError::Pip {
                    detail: "ERROR: no wheel".into(),
                    offline: true,
                    download: None,
                },
                false,
            );
            assert_eq!(off.code, "presence_venv_failed");
            let r = off.reason.unwrap();
            assert!(r.starts_with("The presence reader is off"), "{r}");
            assert!(
                r.contains("offline cache") && r.contains("111 install"),
                "{r}"
            );
        }

        #[test]
        fn status_round_trips_and_rejects_links() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            assert!(status(root).is_none());
            record(root, &Status::ready("abc"));
            assert_eq!(status(root), Some(Status::ready("abc")));
            assert_eq!(unavailable_reason(root), None);
            let failed = Status::failed("abc", &PresenceVenvError::Incomplete, false);
            record(root, &failed);
            assert_eq!(unavailable_reason(root), failed.reason);
            // a status file that is a link is never followed
            let real = root.join("elsewhere.json");
            std::fs::rename(status_path(root), &real).unwrap();
            std::os::unix::fs::symlink(&real, status_path(root)).unwrap();
            assert!(status(root).is_none());
        }
    }
}
