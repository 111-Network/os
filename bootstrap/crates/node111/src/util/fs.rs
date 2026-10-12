//! Atomic filesystem primitives (tmp file + fsync + rename; fsync parent dir).

/// Write `data` to `path` atomically with the given mode (e.g. 0o600).
pub fn atomic_write(path: &std::path::Path, data: &[u8], mode: u32) -> anyhow::Result<()> {
    let _ = (path, data, mode);
    todo!("T2")
}

/// fsync a directory (after renames inside it).
pub fn fsync_dir(path: &std::path::Path) -> anyhow::Result<()> {
    let _ = path;
    todo!("T2")
}
