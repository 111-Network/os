//! Dashboard v1 (deliberately minimal — Layer 4 owns the real one): single embedded
//! GET-only page on 127.0.0.1:11180, /health + /api/status per pinned HTTP contracts.

pub mod routes;

use crate::daemon::health::Components;

pub struct DashboardState {
    // npub, category, version, kb status, transport state, uptime,
    // last-successful-update-check, clock_fault, offer — wiring is T8's.
    _priv: (),
}

/// Serve on 127.0.0.1:11180 (loopback; LAN only via explicit config opt-in).
pub async fn serve(state: std::sync::Arc<std::sync::RwLock<Components>>) -> anyhow::Result<()> {
    let _ = state;
    todo!("T8")
}
