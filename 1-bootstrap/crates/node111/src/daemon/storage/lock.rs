//! Layer 8 (§13.2 "The lock"; §1 rule 1; §3 J2): T0 stub; filled by row D16.
//!
//! `<root>/node/lock` (0600): `111d` holds an exclusive `flock` on it for its lifetime, one
//! descriptor opened `O_CLOEXEC` at start before any store opens, taken without waiting (the
//! `state/staging.lock` pattern of `daemon/updater/stage.rs`). The store verbs take the same lock
//! without waiting and refuse while it is held ("Stop the node first: sudo systemctl stop 111node";
//! D16 adds their `try_take`). A `111d` that starts while a verb holds it exits with status 75 and
//! the message "a store verb is running" (the arm in `daemon/mod.rs`), and the unit restarts it
//! after `RestartSec=5`.
//!
//! T0: [`acquire`] returns `Ok` and opens nothing; no descriptor is held.
#![allow(dead_code)]

use std::fmt;
use std::path::Path;

/// The daemon's hold on `node/lock`, kept for its lifetime (§13.2). T0: no descriptor.
pub struct NodeLock;

/// Why [`acquire`] did not take the lock.
#[derive(Debug)]
pub enum LockError {
    /// A store verb holds the lock: the daemon exits with status 75.
    Held,
    /// The lock file could not be created or opened.
    Io(std::io::Error),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::Held => f.write_str("a store verb is running"),
            LockError::Io(e) => write!(f, "node/lock: {e}"),
        }
    }
}

impl std::error::Error for LockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LockError::Held => None,
            LockError::Io(e) => Some(e),
        }
    }
}

/// Take `<root>/node/lock` for the daemon's lifetime, without waiting (§13.2). T0: returns `Ok`
/// at once; nothing is created, opened or locked.
pub fn acquire(_root: &Path) -> Result<NodeLock, LockError> {
    Ok(NodeLock)
}
