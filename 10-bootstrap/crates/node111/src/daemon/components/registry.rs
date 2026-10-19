//! What is on disk for each optional component, and the disk-side steps of
//! enable and "remove files" (Layer 6 §2 "State", §2.1 rules 2–5).
//!
//! Paths, under the daemon root:
//! - `cache/packages/<id>/` — the install-time payload (root-owned; a USB drop
//!   into it is the offline alternative). Only [`remove_cache`] deletes it.
//! - `venvs/<id>` — a link to one complete build `venvs/<id>.d/<build>/` (the
//!   L1 presence-venv pattern: build in a new directory, flip the link with a
//!   tmp link + `rename`, then prune older builds; a failed build never
//!   touches the venv in place).
//! - `components/<id>/` — an installed binary payload (0755 program).
//! - `bridge/<adapter dir>/` — the adapter's state (0700).
//!
//! **State from disk facts** ([`state_of`]): `running` (the runner says so),
//! else installed files and the user's switch (`enabled` ⇒ `installed`,
//! otherwise `disabled`), else `cached` (bytes in `cache/packages/<id>/`),
//! else `declared`. Zero weight (§2.1 rule 2) is runtime weight: none of these
//! reads starts a process or keeps a file open.
//!
//! **Fail closed.** [`verify_cache`] checks every cached file against the pin
//! before anything is installed: for a pip closure every file in
//! `cache/packages/<id>/` must be a regular file whose SHA-256 the lock lists
//! (pip's `--require-hashes --no-index` checks again); for a binary the one
//! pinned file must match its size and SHA-256, and nothing else may sit
//! beside it.

use super::manifest::{lock_hashes, url_file_name, Component, Payload};
use crate::util::run::Runner;
use std::path::{Path, PathBuf};

/// Integrity marker of a build or an installed tree: the pin it was built
/// from (the lock's sha256, or the binary's).
pub const MARKER: &str = ".111-integrity";
/// The lock a venv build was installed from, kept inside the build.
pub const BUILD_LOCK: &str = "111-lock.txt";
/// Most entries a cache directory may hold.
const MAX_CACHE_ENTRIES: usize = 4096;
/// Longest tool output kept in an error.
const DETAIL_MAX: usize = 200;

/// The component's paths under `root`.
#[derive(Debug, Clone)]
pub struct Paths {
    pub cache: PathBuf,
    pub install: PathBuf,
    pub venv: PathBuf,
    pub builds: PathBuf,
    pub bridge: PathBuf,
}

impl Paths {
    pub fn of(root: &Path, c: &Component) -> Self {
        Paths {
            cache: root.join("cache/packages").join(&c.id),
            install: root.join("components").join(&c.id),
            venv: root.join("venvs").join(&c.id),
            builds: root.join("venvs").join(format!("{}.d", c.id)),
            bridge: root.join("bridge").join(c.adapter_dir()),
        }
    }
}

/// The contract's `ComponentState` (§2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentState {
    /// In the release, no payload cached.
    Declared,
    /// Payload in `cache/packages/<id>/` (its size), nothing installed.
    Cached { bytes: u64 },
    /// Files installed and enabled, the child not running (waiting for its
    /// dependency, backing off).
    Installed,
    /// The supervised child is up.
    Running,
    /// Installed files remain; the user disabled it (or never finished
    /// enabling it).
    Disabled,
}

impl ComponentState {
    /// The contract string.
    pub fn as_str(self) -> &'static str {
        match self {
            ComponentState::Declared => "declared",
            ComponentState::Cached { .. } => "cached",
            ComponentState::Installed => "installed",
            ComponentState::Running => "running",
            ComponentState::Disabled => "disabled",
        }
    }
}

impl serde::Serialize for ComponentState {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// What is on disk for one component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskFacts {
    /// Bytes in `cache/packages/<id>/`; `None` when nothing is cached.
    pub cached_bytes: Option<u64>,
    /// `venvs/<id>` (pip) or `components/<id>/` (binary) exists.
    pub installed: bool,
    /// Size of the installed tree; `None` when not installed.
    pub installed_bytes: Option<u64>,
}

/// Size of the regular files under `p` (links not followed); `None` when `p`
/// is absent or holds nothing.
pub fn tree_bytes(p: &Path) -> Option<u64> {
    let meta = std::fs::symlink_metadata(p).ok()?;
    if meta.file_type().is_file() {
        return Some(meta.len());
    }
    if !meta.file_type().is_dir() {
        return None;
    }
    let mut total = 0u64;
    let mut any = false;
    let mut stack = vec![p.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(m) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if m.file_type().is_dir() {
                stack.push(entry.path());
            } else if m.file_type().is_file() {
                total += m.len();
                any = true;
            }
        }
    }
    any.then_some(total)
}

/// The facts for `c` under `root`.
pub fn facts(root: &Path, c: &Component) -> DiskFacts {
    let p = Paths::of(root, c);
    let (installed, installed_bytes) = match c.payload {
        Payload::Pip { .. } => {
            let present = std::fs::symlink_metadata(&p.venv).is_ok();
            let bytes = std::fs::read_link(&p.venv)
                .ok()
                .map(|t| p.venv.parent().unwrap_or(root).join(t))
                .and_then(|t| tree_bytes(&t))
                .or_else(|| {
                    std::fs::symlink_metadata(&p.venv)
                        .is_ok_and(|m| m.file_type().is_dir())
                        .then(|| tree_bytes(&p.venv))
                        .flatten()
                });
            (present, present.then_some(bytes.unwrap_or(0)))
        }
        Payload::Binary { .. } => {
            let present = std::fs::symlink_metadata(&p.install).is_ok();
            (
                present,
                present.then(|| tree_bytes(&p.install).unwrap_or(0)),
            )
        }
    };
    DiskFacts {
        cached_bytes: tree_bytes(&p.cache),
        installed,
        installed_bytes,
    }
}

/// The state from the disk facts, the user's switch and the runner.
pub fn state_of(f: &DiskFacts, enabled: bool, running: bool) -> ComponentState {
    if running {
        ComponentState::Running
    } else if f.installed && enabled {
        ComponentState::Installed
    } else if f.installed {
        ComponentState::Disabled
    } else if let Some(bytes) = f.cached_bytes {
        ComponentState::Cached { bytes }
    } else {
        ComponentState::Declared
    }
}

/// Why the cache does not satisfy the pin (nothing was installed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// Nothing (or not everything) to install from: `payload_missing`.
    NotCached(String),
    /// A cached file is not the pinned one: `pin_mismatch`.
    Mismatch(String),
}

/// Check every cached file of `c` against its pin (module doc, "Fail closed").
pub fn verify_cache(root: &Path, c: &Component) -> Result<(), VerifyError> {
    let p = Paths::of(root, c);
    let entries = cache_entries(&p.cache)?;
    match &c.payload {
        Payload::Pip { lock_bytes, .. } => {
            let lock = lock_bytes.as_deref().ok_or_else(|| {
                VerifyError::NotCached(format!("this release carries no lock for {}", c.id))
            })?;
            let pinned = lock_hashes(lock);
            if entries.is_empty() {
                return Err(VerifyError::NotCached(format!(
                    "{} is empty",
                    p.cache.display()
                )));
            }
            for (name, path) in &entries {
                let sha = crate::util::hash::sha256_file(path).map_err(|e| {
                    VerifyError::Mismatch(format!("{name}: cannot be read ({e:#})"))
                })?;
                if !pinned.contains(&sha) {
                    return Err(VerifyError::Mismatch(format!(
                        "{name} is not pinned by the lock"
                    )));
                }
            }
            Ok(())
        }
        Payload::Binary { url, sha256, size } => {
            let want = url_file_name(url).unwrap_or_default();
            let Some((_, path)) = entries.iter().find(|(n, _)| *n == want) else {
                return Err(VerifyError::NotCached(format!(
                    "{want} is not in {}",
                    p.cache.display()
                )));
            };
            if let Some((other, _)) = entries.iter().find(|(n, _)| *n != want) {
                return Err(VerifyError::Mismatch(format!(
                    "{other} is not the pinned file"
                )));
            }
            let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            if len != *size {
                return Err(VerifyError::Mismatch(format!(
                    "{want} is {len} bytes, the pin says {size}"
                )));
            }
            let sha = crate::util::hash::sha256_file(path)
                .map_err(|e| VerifyError::Mismatch(format!("{want}: cannot be read ({e:#})")))?;
            if sha != *sha256 {
                return Err(VerifyError::Mismatch(format!(
                    "{want} does not match its pinned sha256"
                )));
            }
            Ok(())
        }
    }
}

/// The cache directory's entries as (name, path); every entry must be a
/// regular file (a link or a directory is a mismatch).
fn cache_entries(dir: &Path) -> Result<Vec<(String, PathBuf)>, VerifyError> {
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_dir() => {}
        Ok(_) => {
            return Err(VerifyError::Mismatch(format!(
                "{} is not a directory",
                dir.display()
            )))
        }
        Err(_) => {
            return Err(VerifyError::NotCached(format!(
                "nothing is cached in {}",
                dir.display()
            )))
        }
    }
    let rd = std::fs::read_dir(dir)
        .map_err(|e| VerifyError::NotCached(format!("{}: {e}", dir.display())))?;
    let mut out = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|e| VerifyError::Mismatch(format!("{}: {e}", dir.display())))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = std::fs::symlink_metadata(entry.path())
            .map_err(|e| VerifyError::Mismatch(format!("{name}: {e}")))?;
        if !meta.file_type().is_file() {
            return Err(VerifyError::Mismatch(format!(
                "{name} is not a regular file"
            )));
        }
        out.push((name, entry.path()));
        if out.len() > MAX_CACHE_ENTRIES {
            return Err(VerifyError::Mismatch("too many cached files".into()));
        }
    }
    out.sort();
    Ok(out)
}

/// What [`install`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// The tree in place was built from this pin: nothing ran.
    Unchanged,
    /// A fresh tree was built and swapped in.
    Built,
}

/// Why nothing new was installed (the tree in place is untouched).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallError(pub String);

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InstallError {}

fn ierr(what: &str, path: &Path, e: impl std::fmt::Display) -> InstallError {
    InstallError(format!("{what} {}: {e}", path.display()))
}

/// The installed tree in place was built from the current pin.
pub fn installed_from_pin(root: &Path, c: &Component) -> bool {
    let p = Paths::of(root, c);
    match &c.payload {
        Payload::Pip { lock_bytes, .. } => lock_bytes.as_deref().is_some_and(|lock| {
            let want = crate::util::hash::sha256_bytes(lock);
            p.venv.join("bin/python").is_file() && marker_is(&p.venv, &want)
        }),
        Payload::Binary { sha256, .. } => {
            p.install.join(&c.run.program).is_file() && marker_is(&p.install, sha256)
        }
    }
}

fn marker_is(dir: &Path, want: &str) -> bool {
    std::fs::read_to_string(dir.join(MARKER)).is_ok_and(|got| got.trim() == want)
}

/// Install `c` from its verified cache (call [`verify_cache`] first):
/// a pip closure into a fresh `venvs/<id>.d/<build>/` (`python3 -m venv`,
/// then that venv's pip with `--require-hashes --no-index --find-links
/// cache/packages/<id>`), then the link swap; a binary into
/// `components/<id>/` (unpacked when a `.tar.gz`, else copied 0755). An
/// unchanged pin runs nothing. Blocking.
pub fn install(root: &Path, c: &Component, cmd: &dyn Runner) -> Result<Installed, InstallError> {
    if installed_from_pin(root, c) {
        return Ok(Installed::Unchanged);
    }
    match &c.payload {
        Payload::Pip { lock_bytes, .. } => {
            let lock = lock_bytes.as_deref().ok_or_else(|| {
                InstallError(format!("this release carries no lock for {}", c.id))
            })?;
            install_venv(root, c, lock, cmd).map(|()| Installed::Built)
        }
        Payload::Binary { url, sha256, .. } => {
            install_binary(root, c, url, sha256).map(|()| Installed::Built)
        }
    }
}

/// `root/<rel>` as a real directory: created when missing, refused when it
/// is a link or another kind of file (never followed).
fn real_dir(path: &Path) -> Result<(), InstallError> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_dir() => return Ok(()),
        Ok(_) => {
            return Err(InstallError(format!(
                "{} is a link or not a directory; refusing to install there",
                path.display()
            )))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(ierr("cannot inspect", path, e)),
    }
    if let Err(e) = std::fs::create_dir(path) {
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(ierr("cannot create", path, e));
        }
    }
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_dir() => Ok(()),
        _ => Err(InstallError(format!(
            "{} is a link or not a directory; refusing to install there",
            path.display()
        ))),
    }
}

/// First `ERROR:` line of a tool's output, else its last line; control
/// characters dropped, bounded.
fn detail(out: &crate::util::run::Output) -> String {
    let text = if out.stderr.trim().is_empty() {
        &out.stdout
    } else {
        &out.stderr
    };
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
        return format!("exit status {}", out.status);
    };
    let mut s = String::new();
    for ch in line.chars().filter(|c| !c.is_control()) {
        if s.len() + ch.len_utf8() > DETAIL_MAX {
            s.push('…');
            break;
        }
        s.push(ch);
    }
    s
}

fn new_build_dir(builds: &Path, tag: &str) -> Result<(PathBuf, String), InstallError> {
    let pid = std::process::id();
    for n in 0..64u32 {
        let id = format!("{tag}-{pid}-{n}");
        let dir = builds.join(&id);
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok((dir, id)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(ierr("cannot create", &dir, e)),
        }
    }
    Err(InstallError(format!(
        "no free build directory under {}",
        builds.display()
    )))
}

fn install_venv(
    root: &Path,
    c: &Component,
    lock: &[u8],
    cmd: &dyn Runner,
) -> Result<(), InstallError> {
    let p = Paths::of(root, c);
    let venvs = root.join("venvs");
    real_dir(&venvs)?;
    real_dir(&p.builds)?;
    sweep_tmp_links(&venvs, &c.id);
    let want = crate::util::hash::sha256_bytes(lock);
    let (dir, build) = new_build_dir(&p.builds, want.get(..16).unwrap_or(&want))?;
    let result = (|| -> Result<(), InstallError> {
        let dir_s = dir.to_string_lossy().into_owned();
        let out = cmd
            .run("python3", &["-m", "venv", &dir_s])
            .map_err(|e| InstallError(format!("python3 -m venv: {e:#}")))?;
        if !out.ok() {
            return Err(InstallError(format!(
                "python3 could not create a virtual environment ({})",
                detail(&out)
            )));
        }
        let lock_path = dir.join(BUILD_LOCK);
        crate::util::fs::atomic_write(&lock_path, lock, 0o644)
            .map_err(|e| ierr("cannot write", &lock_path, format!("{e:#}")))?;
        let pip = format!("{dir_s}/bin/pip");
        let lock_s = lock_path.to_string_lossy().into_owned();
        let cache_s = p.cache.to_string_lossy().into_owned();
        let out = cmd
            .run(
                &pip,
                &[
                    "install",
                    "--require-hashes",
                    "--no-index",
                    "--find-links",
                    &cache_s,
                    "-r",
                    &lock_s,
                ],
            )
            .map_err(|e| InstallError(format!("pip: {e:#}")))?;
        if !out.ok() {
            return Err(InstallError(format!(
                "pip could not install the locked packages from the cache ({})",
                detail(&out)
            )));
        }
        if !dir.join("bin/python").is_file() {
            return Err(InstallError("the build produced no bin/python".into()));
        }
        let marker = dir.join(MARKER);
        crate::util::fs::atomic_write(&marker, want.as_bytes(), 0o644)
            .map_err(|e| ierr("cannot write", &marker, format!("{e:#}")))?;
        flip(&venvs, &p.venv, &format!("{}.d", c.id), &build)
    })();
    match result {
        Ok(()) => {
            prune(&p.builds, &build);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

const TMP_LINK: &str = ".component.tmp-";

/// Point `link` at `<builds_name>/<build>` atomically (tmp link + `rename`).
/// A real directory in the way is moved into the builds dir first and moved
/// back should the swap fail.
fn flip(venvs: &Path, link: &Path, builds_name: &str, build: &str) -> Result<(), InstallError> {
    let target = Path::new(builds_name).join(build);
    let pid = std::process::id();
    let name = link.file_name().and_then(|n| n.to_str()).unwrap_or("x");
    let tmp = venvs.join(format!("{TMP_LINK}{name}-{pid}-{build}"));
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(&target, &tmp).map_err(|e| ierr("cannot link", &tmp, e))?;
    let mut aside = None;
    if std::fs::symlink_metadata(link).is_ok_and(|m| m.file_type().is_dir()) {
        let to = venvs
            .join(builds_name)
            .join(format!("legacy-{pid}-{build}"));
        if let Err(e) = std::fs::rename(link, &to) {
            let _ = std::fs::remove_file(&tmp);
            return Err(ierr("cannot move aside", link, e));
        }
        aside = Some(to);
    }
    if let Err(e) = std::fs::rename(&tmp, link) {
        let _ = std::fs::remove_file(&tmp);
        if let Some(to) = aside {
            let _ = std::fs::rename(to, link);
        }
        return Err(ierr("cannot swap in", link, e));
    }
    if let Err(e) = crate::util::fs::fsync_dir(venvs) {
        eprintln!("111 components: {e:#}");
    }
    Ok(())
}

/// Remove every build but `keep` (a link is removed itself). Best effort.
fn prune(builds: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(builds) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_str() == Some(keep) {
            continue;
        }
        if let Err(e) = remove_path(&entry.path()) {
            eprintln!(
                "111 components: cannot prune {}: {e}",
                entry.path().display()
            );
        }
    }
}

/// Tmp links a crash left between `symlink` and `rename` for `id`.
fn sweep_tmp_links(venvs: &Path, id: &str) {
    let Ok(entries) = std::fs::read_dir(venvs) else {
        return;
    };
    let prefix = format!("{TMP_LINK}{id}-");
    for entry in entries.flatten() {
        let is_tmp = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(&prefix));
        if is_tmp
            && std::fs::symlink_metadata(entry.path()).is_ok_and(|m| m.file_type().is_symlink())
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn install_binary(root: &Path, c: &Component, url: &str, sha256: &str) -> Result<(), InstallError> {
    use std::os::unix::fs::PermissionsExt;
    let p = Paths::of(root, c);
    let name = url_file_name(url).unwrap_or_default();
    let cached = p.cache.join(&name);
    let components = root.join("components");
    real_dir(&components)?;
    let pid = std::process::id();
    let tmp = components.join(format!(".{}.tmp-{pid}", c.id));
    let _ = remove_path(&tmp);
    std::fs::create_dir(&tmp).map_err(|e| ierr("cannot create", &tmp, e))?;
    let result = (|| -> Result<(), InstallError> {
        if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            let bytes = std::fs::read(&cached).map_err(|e| ierr("cannot read", &cached, e))?;
            // the bytes read are the verified ones only if they still hash to the pin
            if crate::util::hash::sha256_bytes(&bytes) != sha256 {
                return Err(InstallError(format!(
                    "{name} changed after it was verified"
                )));
            }
            crate::daemon::updater::stage::safe_unpack(&bytes, &tmp)
                .map_err(|e| InstallError(format!("cannot unpack {name}: {e:#}")))?;
        } else {
            let to = tmp.join(&name);
            std::fs::copy(&cached, &to).map_err(|e| ierr("cannot copy", &cached, e))?;
            if crate::util::hash::sha256_file(&to).ok().as_deref() != Some(sha256) {
                return Err(InstallError(format!(
                    "{name} changed after it was verified"
                )));
            }
        }
        let program = tmp.join(&c.run.program);
        if !std::fs::symlink_metadata(&program).is_ok_and(|m| m.file_type().is_file()) {
            return Err(InstallError(format!(
                "the payload carries no {}",
                c.run.program
            )));
        }
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| ierr("cannot chmod", &program, e))?;
        let marker = tmp.join(MARKER);
        crate::util::fs::atomic_write(&marker, sha256.as_bytes(), 0o644)
            .map_err(|e| ierr("cannot write", &marker, format!("{e:#}")))?;
        // swap in: the old tree (if any) moves aside, then goes
        let old = components.join(format!(".{}.old-{pid}", c.id));
        let _ = remove_path(&old);
        let had_old = std::fs::symlink_metadata(&p.install).is_ok();
        if had_old {
            std::fs::rename(&p.install, &old)
                .map_err(|e| ierr("cannot move aside", &p.install, e))?;
        }
        if let Err(e) = std::fs::rename(&tmp, &p.install) {
            if had_old {
                let _ = std::fs::rename(&old, &p.install);
            }
            return Err(ierr("cannot swap in", &p.install, e));
        }
        if had_old {
            let _ = remove_path(&old);
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = remove_path(&tmp);
    }
    result
}

/// Remove `p` without following it: a link or file is unlinked, a directory
/// removed recursively. Absent is fine.
pub fn remove_path(p: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// "Remove files" (§2.1 rule 5): the installed tree (`components/<id>/`,
/// `venvs/<id>` and `venvs/<id>.d/`) and the adapter's state
/// (`bridge/<adapter dir>/`). The cache stays ([`remove_cache`]). An empty
/// `components/` or `bridge/` goes too, so a node with every component
/// removed looks like one that never had any (G7). Returns what existed and
/// was removed.
pub fn remove_files(root: &Path, c: &Component) -> std::io::Result<Vec<PathBuf>> {
    let p = Paths::of(root, c);
    let mut removed = Vec::new();
    for path in [&p.install, &p.venv, &p.builds, &p.bridge] {
        if std::fs::symlink_metadata(path).is_ok() {
            remove_path(path)?;
            removed.push(path.clone());
        }
    }
    for dir in [root.join("components"), root.join("bridge")] {
        // only when empty: remove_dir refuses otherwise
        let _ = std::fs::remove_dir(&dir);
    }
    Ok(removed)
}

/// Delete the cached payload `cache/packages/<id>/` (only on this explicit
/// call). `cache/packages/` is root-owned on an installed node, so the
/// daemon may lack the permission; the error says so.
pub fn remove_cache(root: &Path, c: &Component) -> std::io::Result<bool> {
    let p = Paths::of(root, c);
    if std::fs::symlink_metadata(&p.cache).is_err() {
        return Ok(false);
    }
    remove_path(&p.cache)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_follows_the_disk_facts() {
        let none = DiskFacts {
            cached_bytes: None,
            installed: false,
            installed_bytes: None,
        };
        assert_eq!(state_of(&none, false, false), ComponentState::Declared);
        assert_eq!(state_of(&none, true, false), ComponentState::Declared);
        let cached = DiskFacts {
            cached_bytes: Some(7),
            ..none
        };
        assert_eq!(
            state_of(&cached, false, false),
            ComponentState::Cached { bytes: 7 }
        );
        let installed = DiskFacts {
            installed: true,
            installed_bytes: Some(3),
            ..cached
        };
        assert_eq!(state_of(&installed, true, false), ComponentState::Installed);
        assert_eq!(state_of(&installed, false, false), ComponentState::Disabled);
        assert_eq!(state_of(&installed, true, true), ComponentState::Running);
        assert_eq!(ComponentState::Cached { bytes: 1 }.as_str(), "cached");
        assert_eq!(
            serde_json::to_value(ComponentState::Disabled).unwrap(),
            "disabled"
        );
    }

    #[test]
    fn tree_bytes_counts_regular_files_and_never_follows_links() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(tree_bytes(&d.path().join("absent")), None);
        assert_eq!(tree_bytes(d.path()), None, "empty");
        std::fs::write(d.path().join("a"), b"12345").unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("sub/b"), b"12").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("big"), vec![0u8; 1000]).unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join("link")).unwrap();
        assert_eq!(tree_bytes(d.path()), Some(7));
    }

    #[test]
    fn detail_prefers_the_error_line() {
        let out = crate::util::run::Output {
            status: 1,
            stdout: String::new(),
            stderr: "x\nERROR: THESE PACKAGES DO NOT MATCH THE HASHES\n  more\n".into(),
        };
        assert_eq!(
            detail(&out),
            "ERROR: THESE PACKAGES DO NOT MATCH THE HASHES"
        );
    }
}
