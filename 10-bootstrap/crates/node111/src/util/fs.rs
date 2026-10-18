//! Atomic filesystem primitives (tmp file + fsync + rename; fsync parent dir).

use anyhow::Context;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Write `data` to `path` atomically with the given mode (e.g. 0o600).
///
/// The tmp file is created in the same directory with `O_EXCL` (never clobbers a
/// sibling), fsynced, then renamed over `path`; the parent directory is fsynced
/// after the rename so the change survives a crash. The final file's permission
/// bits are exactly `mode` (the process umask is overridden via `fchmod`).
pub fn atomic_write(path: &std::path::Path, data: &[u8], mode: u32) -> anyhow::Result<()> {
    atomic_write_owned(path, data, mode, None)
}

/// [`atomic_write`], and when `owner` is `Some((uid, gid))` the tmp file is
/// `fchown`ed through its open descriptor *before* the rename — never chown by
/// path afterwards (a path chown follows a symlink planted at `path`).
pub fn atomic_write_owned(
    path: &std::path::Path,
    data: &[u8],
    mode: u32,
    owner: Option<(u32, u32)>,
) -> anyhow::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("atomic_write: no file name in {}", path.display()))?;

    // O_EXCL tmpfile in the same directory; loop defeats PID/counter collisions.
    let pid = std::process::id();
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let (mut file, tmp_path) = loop {
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp: PathBuf = dir.join(format!(".{}.tmp-{pid}-{n}", file_name.to_string_lossy()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true) // O_EXCL
            .mode(mode)
            .open(&tmp)
        {
            Ok(f) => break (f, tmp),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(e).context(format!("atomic_write: create tmp for {}", path.display()))
            }
        }
    };

    // Inner closure so every failure path can unlink the tmpfile.
    let result = (|| -> anyhow::Result<()> {
        // fchmod defeats umask: final mode is exactly `mode`.
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        if let Some((uid, gid)) = owner {
            std::os::unix::fs::fchown(&file, Some(uid), Some(gid))?;
        }
        file.write_all(data)?;
        file.sync_all()?; // fsync file data + metadata before rename
        drop(file);
        std::fs::rename(&tmp_path, path)?;
        fsync_dir(dir)?; // fsync parent dir so the rename is durable
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path); // best-effort cleanup
    }
    result.with_context(|| format!("atomic_write {}", path.display()))
}

/// fsync a directory (after renames inside it).
pub fn fsync_dir(path: &std::path::Path) -> anyhow::Result<()> {
    std::fs::File::open(path)
        .and_then(|f| f.sync_all())
        .with_context(|| format!("fsync_dir {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn unit_smoke_content_and_mode() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("out.bin");
        atomic_write(&p, b"hello", 0o640).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"hello");
        assert_eq!(p.metadata().unwrap().permissions().mode() & 0o777, 0o640);
    }

    #[test]
    fn owned_write_replaces_a_planted_symlink_and_sets_owner() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"keep").unwrap();
        let p = dir.path().join("out.json");
        std::os::unix::fs::symlink(&victim, &p).unwrap();
        // our own uid/gid: fchown to them needs no privilege
        let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
        atomic_write_owned(&p, b"new", 0o600, Some((uid, gid))).unwrap();
        let meta = std::fs::symlink_metadata(&p).unwrap();
        assert!(meta.file_type().is_file(), "the symlink itself is replaced");
        assert_eq!((meta.uid(), meta.gid()), (uid, gid));
        assert_eq!(meta.mode() & 0o777, 0o600);
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep", "target untouched");
    }
}
