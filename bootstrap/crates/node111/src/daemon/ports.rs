//! Daemon-wide port-owner table (Layer 5 §14 "Identify"; reused by Layer 6's
//! full identify sequence, §19).
//!
//! A radio port (a serial device such as `/dev/ttyACM0`, or a `host:port`
//! uplink) is opened by **one** process at a time: meshmud, an identify
//! probe, rnsd's RNode interface. Before anything opens a port it claims it
//! here; a port someone else holds answers [`PortBusy`], which the API maps to
//! `409 port_busy` ([`PortBusy::problem`]). Nothing here opens a port.
//!
//! Keys are normalized by [`port_key`]: an absolute device path is resolved
//! through its symlinks (so `/dev/serial/by-id/usb-…` and the `/dev/ttyACM0`
//! it points at are one port); a network uplink (`host:port`, `tcp://` optional)
//! is trimmed and lowercased.
//! Ownership stamps are daemon ticks (a caller-supplied monotonic counter),
//! never wall-clock time.

use crate::daemon::dashboard::api_v1::Problem;
use axum::http::StatusCode;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::Mutex;

/// Who holds a port (§14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    /// The holder, e.g. `identify:<device id>`, `meshmud`, `rnsd`.
    pub task: String,
    /// Daemon tick at which the current holder claimed (or took over) it.
    pub since_tick: u64,
}

/// The port is held by someone else (`409 port_busy`, §14/§15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortBusy {
    /// The normalized port key ([`port_key`]).
    pub port: String,
    pub owner: Owner,
}

impl fmt::Display for PortBusy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "port {} is in use by {} (since tick {})",
            self.port, self.owner.task, self.owner.since_tick
        )
    }
}

impl std::error::Error for PortBusy {}

impl PortBusy {
    /// The contract problem: `409 port_busy` (Layer 5 §15 `Problem.code`).
    pub fn problem(&self) -> Problem {
        Problem::new(StatusCode::CONFLICT, "port_busy", "This port is in use.").detail(format!(
            "{} is held by {}; stop that first, then try again.",
            self.port, self.owner.task
        ))
    }
}

/// Why a release or hand-over did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    /// Nobody holds the port.
    NotHeld { port: String },
    /// Another task holds it; only the holder may release or hand it over.
    NotOwner { port: String, owner: Owner },
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PortError::NotHeld { port } => write!(f, "port {port} is not held"),
            PortError::NotOwner { port, owner } => {
                write!(f, "port {port} is held by {}, not the caller", owner.task)
            }
        }
    }
}

impl std::error::Error for PortError {}

/// Normalize a port name to its table key: absolute paths resolve through
/// symlinks (falling back to the path as given when it does not resolve —
/// e.g. the device was unplugged); network uplinks (`host:port`, optionally
/// spelled `tcp://host:port`, §14) are trimmed and lowercased.
pub fn port_key(raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with('/') {
        return match std::fs::canonicalize(Path::new(raw)) {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(_) => raw.to_string(),
        };
    }
    let lower = raw.to_ascii_lowercase();
    match lower.strip_prefix("tcp://") {
        Some(rest) => rest.to_string(),
        None => lower,
    }
}

/// The table (one per daemon; shared as `Arc<PortTable>`).
#[derive(Debug, Default)]
pub struct PortTable {
    owners: Mutex<HashMap<String, Owner>>,
}

impl PortTable {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Owner>> {
        // A panic while holding this lock cannot leave the map half-updated
        // (every critical section is one insert/remove), so poison is benign.
        self.owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Claim `port` for `task` at `tick`. Idempotent for the current holder
    /// (its `since_tick` is kept); [`PortBusy`] when another task holds it.
    pub fn claim(&self, port: &str, task: &str, tick: u64) -> Result<Owner, PortBusy> {
        let key = port_key(port);
        let mut owners = self.lock();
        match owners.get(&key) {
            Some(owner) if owner.task == task => Ok(owner.clone()),
            Some(owner) => Err(PortBusy {
                port: key,
                owner: owner.clone(),
            }),
            None => {
                let owner = Owner {
                    task: task.to_string(),
                    since_tick: tick,
                };
                owners.insert(key, owner.clone());
                Ok(owner)
            }
        }
    }

    /// Release `port`; only its holder may.
    pub fn release(&self, port: &str, task: &str) -> Result<(), PortError> {
        let key = port_key(port);
        let mut owners = self.lock();
        match owners.get(&key) {
            None => Err(PortError::NotHeld { port: key }),
            Some(owner) if owner.task != task => Err(PortError::NotOwner {
                port: key,
                owner: owner.clone(),
            }),
            Some(_) => {
                owners.remove(&key);
                Ok(())
            }
        }
    }

    /// Hand `port` from `from` to `to` atomically (§14: the identify child
    /// becomes the service child in place — the port is never free in
    /// between). The new holder's `since_tick` is `tick`.
    pub fn transfer(&self, port: &str, from: &str, to: &str, tick: u64) -> Result<(), PortError> {
        let key = port_key(port);
        let mut owners = self.lock();
        match owners.get_mut(&key) {
            None => Err(PortError::NotHeld { port: key }),
            Some(owner) if owner.task != from => Err(PortError::NotOwner {
                port: key,
                owner: owner.clone(),
            }),
            Some(owner) => {
                *owner = Owner {
                    task: to.to_string(),
                    since_tick: tick,
                };
                Ok(())
            }
        }
    }

    /// The current holder of `port`, if any.
    pub fn is_held(&self, port: &str) -> Option<Owner> {
        self.lock().get(&port_key(port)).cloned()
    }

    /// Every held port (normalized key, holder), sorted by key.
    pub fn held(&self) -> Vec<(String, Owner)> {
        let mut out: Vec<_> = self
            .lock()
            .iter()
            .map(|(k, o)| (k.clone(), o.clone()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_is_exclusive_and_idempotent_for_the_holder() {
        let t = PortTable::new();
        let a = t.claim("/dev/ttyFAKE0", "identify:d1", 5).unwrap();
        assert_eq!(a.since_tick, 5);
        // the holder re-claiming keeps its stamp
        assert_eq!(t.claim("/dev/ttyFAKE0", "identify:d1", 9).unwrap(), a);
        // anyone else: PortBusy naming the holder
        let busy = t.claim("/dev/ttyFAKE0", "meshmud", 9).unwrap_err();
        assert_eq!(busy.port, "/dev/ttyFAKE0");
        assert_eq!(busy.owner, a);
        let p = busy.problem();
        assert_eq!((p.status, p.code), (409, "port_busy"));
        assert!(p.detail.unwrap().contains("identify:d1"));
        assert_eq!(t.is_held("/dev/ttyFAKE0"), Some(a));
        assert_eq!(t.is_held("/dev/ttyFAKE1"), None);
    }

    #[test]
    fn only_the_holder_releases_or_hands_over() {
        let t = PortTable::new();
        assert_eq!(
            t.release("10.0.0.7:4403", "meshmud"),
            Err(PortError::NotHeld {
                port: "10.0.0.7:4403".into()
            })
        );
        t.claim("10.0.0.7:4403", "identify:d2", 1).unwrap();
        assert!(matches!(
            t.release("10.0.0.7:4403", "meshmud"),
            Err(PortError::NotOwner { .. })
        ));
        assert!(matches!(
            t.transfer("10.0.0.7:4403", "meshmud", "x", 2),
            Err(PortError::NotOwner { .. })
        ));
        // identify → service child in place: never free in between
        t.transfer("10.0.0.7:4403", "identify:d2", "meshmud", 3)
            .unwrap();
        assert_eq!(
            t.is_held("10.0.0.7:4403"),
            Some(Owner {
                task: "meshmud".into(),
                since_tick: 3
            })
        );
        assert!(t.claim("10.0.0.7:4403", "identify:d2", 4).is_err());
        t.release("10.0.0.7:4403", "meshmud").unwrap();
        assert_eq!(t.is_held("10.0.0.7:4403"), None);
        assert!(t.held().is_empty());
        assert!(t.transfer("10.0.0.7:4403", "meshmud", "x", 5).is_err());
    }

    #[test]
    fn keys_are_normalized() {
        // host:port names: case, surrounding space and `tcp://` do not matter
        assert_eq!(port_key(" Board.LAN:4403 "), "board.lan:4403");
        assert_eq!(port_key("TCP://Board.lan:4403"), "board.lan:4403");
        let t = PortTable::new();
        t.claim("Board.lan:4403", "meshmud", 0).unwrap();
        assert!(t.claim("board.LAN:4403", "identify:d3", 1).is_err());

        // a by-id symlink and its target are one port
        let dir = tempfile::tempdir().unwrap();
        let dev = dir.path().join("ttyACM7");
        std::fs::write(&dev, b"").unwrap();
        let by_id = dir.path().join("usb-Board_1234-if00");
        std::os::unix::fs::symlink(&dev, &by_id).unwrap();
        assert_eq!(
            port_key(by_id.to_str().unwrap()),
            port_key(dev.to_str().unwrap())
        );
        t.claim(by_id.to_str().unwrap(), "meshmud", 2).unwrap();
        let busy = t.claim(dev.to_str().unwrap(), "rnsd", 3).unwrap_err();
        assert_eq!(busy.owner.task, "meshmud");
        // a path that does not resolve keeps its spelling
        assert_eq!(
            port_key("/dev/does-not-exist-111"),
            "/dev/does-not-exist-111"
        );
        assert_eq!(t.held().len(), 2);
    }
}
