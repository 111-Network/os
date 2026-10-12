//! Node root resolution: `$NODE111_ROOT` (Termux + tests) or `/var/lib/111`.

use std::path::PathBuf;

/// The node root. Env override is honored every call (cheap; callers hold the
/// result in a DaemonLayout).
pub fn root() -> PathBuf {
    std::env::var("NODE111_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/var/lib/111"))
}

/// $HOME-based fallback for provisioning-host paths (~/.cache/111 etc.).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
