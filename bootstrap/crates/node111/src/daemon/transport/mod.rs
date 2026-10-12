//! Conditional transport (Bundle B): hash-locked rnsd child of the daemon when
//! python3+venv exist; detection-only stub otherwise. rns identity is independent
//! of node identity in v1.

pub mod config;
pub mod netlink;
pub mod venv;

use crate::daemon::health::TransportHealth;

pub struct TransportHandle {
    // child handle + config watcher — construction detail is T9's.
    _priv: (),
}

impl TransportHandle {
    pub fn health(&self) -> TransportHealth {
        todo!("T9")
    }
}

/// Bring transport up per spec (venv integrity check → build/rebuild from cache →
/// rnsd child → AutoInterface → config regen on netlink). Degrades, never panics
/// the daemon: no python3 ⇒ `no transport` stub + degrade branch in receipt.
pub async fn start() -> TransportHandle {
    todo!("T9")
}
