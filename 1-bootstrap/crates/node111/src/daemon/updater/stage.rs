//! Payload staging: verify tarball pin → extract to `state/staging/<ver>.tmp` →
//! verify the extracted tree against `share/payload.manifest` → fsync → rename
//! → drop `state/update-request.json`. The daemon's staging area is the ONLY
//! place it may write; promotion to releases/ is the root helper's job (D12).
//!
//! The whole fetch–stage–request sequence runs under [`StagingLock`]
//! (`state/staging.lock`, Layer 7 §9.2, S1), so the daemon's check and a
//! `111 update --from` run never write `state/staging/<ver>/` or the request
//! file at the same time.

use crate::calver::Calver;
use crate::manifest::PinnedArtifact;
use flate2::read::GzDecoder;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};

/// The lock file's name under `state/` (Layer 7 §9.2, S1; §18).
pub const STAGING_LOCK: &str = "staging.lock";

/// How [`StagingLock::acquire`] treats a lock another run holds (§9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockWait {
    /// Block until it is free: the daemon's loop, inside its `spawn_blocking`.
    Wait,
    /// Return [`LockTake::Held`] at once: `111 update --from` (exit 75, L4).
    NoWait,
}

/// The result of [`StagingLock::acquire`].
#[derive(Debug)]
pub enum LockTake {
    Taken(StagingLock),
    /// Another run (the daemon's check, or a `--from` run) holds the lock.
    Held,
}

/// `state/staging.lock`: an exclusive `flock(2)` on a file opened `O_CLOEXEC`
/// (so no child the daemon spawns inherits it) and `O_NOFOLLOW` (a planted link
/// is refused, never followed), held for the whole fetch–stage–request sequence
/// (Layer 7 §9.2, S1). Dropping it closes the descriptor, which releases the
/// lock; a crashed holder releases it the same way.
#[derive(Debug)]
pub struct StagingLock {
    _file: File,
    waited: bool,
}

impl StagingLock {
    /// `state/staging.lock` under `state_dir`.
    pub fn path(state_dir: &Path) -> PathBuf {
        state_dir.join(STAGING_LOCK)
    }

    /// Take the lock (creating the file, mode 0600, when missing).
    pub fn acquire(state_dir: &Path, wait: LockWait) -> std::io::Result<LockTake> {
        std::fs::create_dir_all(state_dir)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(Self::path(state_dir))?;
        // A non-blocking try first, so a waiting take knows it waited (the run
        // it waited for may have staged the same release: `already_requested`).
        match flock(&file, libc::LOCK_EX | libc::LOCK_NB)? {
            true => {
                return Ok(LockTake::Taken(StagingLock {
                    _file: file,
                    waited: false,
                }))
            }
            false if wait == LockWait::NoWait => return Ok(LockTake::Held),
            false => {}
        }
        flock(&file, libc::LOCK_EX)?;
        Ok(LockTake::Taken(StagingLock {
            _file: file,
            waited: true,
        }))
    }

    /// Whether this take had to wait for another holder (§9.2: the daemon's
    /// check skips re-staging what that holder just requested).
    pub fn waited(&self) -> bool {
        self.waited
    }
}

/// `flock(2)` with EINTR retried: `Ok(false)` when a `LOCK_NB` take finds the lock
/// held.
fn flock(file: &File, op: libc::c_int) -> std::io::Result<bool> {
    loop {
        // SAFETY: `file` owns a valid open descriptor for this call's duration.
        let rc = unsafe { libc::flock(file.as_raw_fd(), op) };
        if rc == 0 {
            return Ok(true);
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(libc::EWOULDBLOCK) if op & libc::LOCK_NB != 0 => return Ok(false),
            _ => return Err(err),
        }
    }
}

/// Extract a gzipped tarball with strict path safety: no absolute paths, no
/// `..`, no symlink/hardlink entries (payload is a plain file tree).
pub(crate) fn safe_unpack(tarball: &[u8], dest: &Path) -> anyhow::Result<()> {
    let mut archive = tar::Archive::new(GzDecoder::new(tarball));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        anyhow::ensure!(
            path.components().all(|c| matches!(c, Component::Normal(_))),
            "unsafe path in payload: {}",
            path.display()
        );
        let header = entry.header();
        let kind = header.entry_type();
        anyhow::ensure!(
            kind == tar::EntryType::Regular || kind == tar::EntryType::Directory,
            "payload carries non-regular entry ({}: {:?}) — symlinks/hardlinks/devices are refused",
            path.display(),
            kind
        );
        let out = dest.join(&path);
        if kind == tar::EntryType::Directory {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mode = header.mode().unwrap_or(0o644);
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut buf)?;
            crate::util::fs::atomic_write(
                &out,
                &buf,
                if mode & 0o111 != 0 { 0o755 } else { 0o644 },
            )?;
        }
    }
    Ok(())
}

/// Verify every regular file in the extracted tree against the manifest the
/// tree itself carries (`share/payload.manifest`: `sha256␠␠relpath` lines).
pub fn verify_tree(tree: &Path) -> anyhow::Result<()> {
    let manifest = std::fs::read_to_string(tree.join("share/payload.manifest"))
        .map_err(|e| anyhow::anyhow!("payload missing share/payload.manifest: {e}"))?;
    let mut pinned = 0usize;
    for line in manifest.lines() {
        let mut parts = line.split_whitespace();
        let (Some(sha), Some(rel)) = (parts.next(), parts.next()) else {
            continue;
        };
        pinned += 1;
        let got = crate::util::hash::sha256_file(&tree.join(rel))
            .map_err(|e| anyhow::anyhow!("payload tree file unreadable ({rel}): {e}"))?;
        anyhow::ensure!(got == sha, "payload tree hash mismatch on {rel}");
    }
    anyhow::ensure!(pinned > 0, "payload manifest pins nothing");
    Ok(())
}

/// Stage a downloaded payload: pin-check the tarball bytes, unpack safely,
/// verify the tree, atomic rename into `staging/<ver>/`.
pub fn stage_payload(
    tarball: &[u8],
    expected: &PinnedArtifact,
    staging_root: &Path,
    version: &Calver,
) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        tarball.len() as u64 == expected.size,
        "payload size {} != manifest {}",
        tarball.len(),
        expected.size
    );
    let got_sha = crate::util::hash::sha256_bytes(tarball);
    anyhow::ensure!(
        got_sha == expected.sha256,
        "payload sha256 {got_sha} != manifest {}",
        expected.sha256
    );

    std::fs::create_dir_all(staging_root)?;
    let tmp = staging_root.join(format!(".{version}.tmp-{}", std::process::id()));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir(&tmp)?;

    let result = (|| -> anyhow::Result<()> {
        // Staging bundle (D12): the raw pinned tarball + the extracted tree/ +
        // (fetch.rs adds) the signed channel manifest + minisig. The root
        // helper re-verifies everything from these bytes alone.
        crate::util::fs::atomic_write(&tmp.join("payload.tar.gz"), tarball, 0o644)?;
        let tree = tmp.join("tree");
        std::fs::create_dir(&tree)?;
        safe_unpack(tarball, &tree)?;
        verify_tree(&tree)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }

    let dest = staging_root.join(version.to_string());
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?; // re-stage of same version (content-verified identical)
    }
    std::fs::rename(&tmp, &dest)?;
    crate::util::fs::fsync_dir(staging_root)?;
    Ok(dest)
}

/// Whether `state/update-request.json` already asks for `version` with this
/// manifest's sha256, and its staging bundle is in place: the run that held the
/// staging lock while this one waited staged it (Layer 7 §9.2, S1: a concurrent
/// run stages once). The file is the daemon's own; an unreadable or oversized
/// one answers `false` (stage again).
pub fn already_requested(state_dir: &Path, version: &Calver, manifest_sha256: &str) -> bool {
    const REQUEST_MAX_BYTES: u64 = 64 * 1024;
    let path = state_dir.join("update-request.json");
    let Ok(meta) = std::fs::metadata(&path) else {
        return false;
    };
    if !meta.is_file() || meta.len() > REQUEST_MAX_BYTES {
        return false;
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return false;
    };
    let Ok(req) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    let staged = state_dir.join("staging").join(version.to_string());
    req["version"].as_str() == Some(version.to_string().as_str())
        && req["manifest_sha256"].as_str() == Some(manifest_sha256)
        && staged.join("payload.tar.gz").is_file()
        && staged.join("channel-111-stable.toml.minisig").is_file()
}

/// Drop the daemon→helper signal (`state/update-request.json`, schema 1).
/// The 111node-update.path unit watches this file.
pub fn write_update_request(
    state_dir: &Path,
    version: &Calver,
    manifest_sha256: &str,
    staged_path: &Path,
) -> anyhow::Result<()> {
    let body = serde_json::json!({
        "schema": 1,
        "version": version.to_string(),
        "manifest_sha256": manifest_sha256,
        "staged_path": staged_path.to_string_lossy(),
        "requested_at": time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "unknown".into()),
    });
    crate::util::fs::atomic_write(
        &state_dir.join("update-request.json"),
        body.to_string().as_bytes(),
        0o644,
    )
}

#[cfg(test)]
mod tests {
    // Fetch/stage coverage in tests/updater_stage.rs; the lock's own rules here.
    use super::*;

    #[test]
    fn a_held_lock_is_held_until_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let first = match StagingLock::acquire(dir.path(), LockWait::NoWait).unwrap() {
            LockTake::Taken(l) => l,
            LockTake::Held => panic!("a fresh lock is free"),
        };
        assert!(!first.waited());
        assert!(StagingLock::path(dir.path()).is_file());
        // flock is per open file description: a second open conflicts even in
        // the same process (the daemon's loop against a --from run).
        assert!(matches!(
            StagingLock::acquire(dir.path(), LockWait::NoWait).unwrap(),
            LockTake::Held
        ));
        drop(first);
        // A child another test forks shares the descriptor until its exec closes
        // it (O_CLOEXEC): the release can take that instant to show.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match StagingLock::acquire(dir.path(), LockWait::NoWait).unwrap() {
                LockTake::Taken(_) => break,
                LockTake::Held if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                LockTake::Held => panic!("the lock stayed held after its release"),
            }
        }
    }

    #[test]
    fn a_waiting_take_blocks_until_release() {
        let dir = tempfile::tempdir().unwrap();
        let LockTake::Taken(held) = StagingLock::acquire(dir.path(), LockWait::Wait).unwrap()
        else {
            panic!("a fresh lock is free");
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let path = dir.path().to_path_buf();
        let waiter = std::thread::spawn(move || {
            let take = StagingLock::acquire(&path, LockWait::Wait).unwrap();
            let LockTake::Taken(ref l) = take else {
                panic!("a waiting take never answers Held");
            };
            assert!(l.waited());
            tx.send(()).unwrap();
            drop(take);
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(200))
                .is_err(),
            "the waiter must block while the lock is held"
        );
        drop(held);
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("the waiter takes the lock once it is released");
        waiter.join().unwrap();
    }

    #[test]
    fn the_lock_file_is_cloexec_and_never_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let LockTake::Taken(l) = StagingLock::acquire(dir.path(), LockWait::NoWait).unwrap() else {
            panic!("a fresh lock is free");
        };
        // SAFETY: F_GETFD on a descriptor `l` owns.
        let flags = unsafe { libc::fcntl(l._file.as_raw_fd(), libc::F_GETFD) };
        assert!(flags & libc::FD_CLOEXEC != 0, "O_CLOEXEC");
        drop(l);

        let other = tempfile::tempdir().unwrap();
        let target = other.path().join("elsewhere");
        std::fs::write(&target, b"").unwrap();
        let planted = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(&target, StagingLock::path(planted.path())).unwrap();
        let err = StagingLock::acquire(planted.path(), LockWait::NoWait).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::ELOOP), "{err}");
    }
}
