//! Dashboard v1 (deliberately minimal — Layer 4 owns the real one): single embedded
//! GET-only page on 127.0.0.1:11180, /health + /api/status per pinned HTTP contracts.

pub mod routes;

use crate::daemon::health::Components;
use serde::Serialize;
use std::sync::{Arc, RwLock};
use std::time::Instant;

pub const DEFAULT_PORT: u16 = 11180;

/// Transport display state (pinned enum — Layer 5/6 seam).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TransportState {
    #[serde(rename = "no transport")]
    NoTransport,
    #[serde(rename = "isolated")]
    Isolated,
    #[serde(rename = "joined")]
    Joined,
    #[serde(rename = "transport degraded")]
    Degraded,
}

/// Standard-tier offer stub (v1 records intent only; Layer 3 replaces the hook).
#[derive(Debug, Clone, Serialize)]
pub struct OfferState {
    pub tier: String,
    pub state: String, // "offered" | "accepted" | "dismissed"
}

/// Distinct error surfaces (identity corruption, …) — never silent.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorState {
    pub kind: String,
    pub detail: String,
}

/// Everything the dashboard/API serves. Writers: daemon core (identity, kb,
/// transport, health), updater (last-successful-check, pending), offer stub.
pub struct DashboardState {
    /// NodeId, §7 machine string (lowercase hex of the 32B Ed25519 public key).
    pub node_id: Option<String>,
    pub category: Option<String>,
    pub version: String,
    pub started: Instant,
    pub kb: Option<crate::daemon::kb::KbStatus>,
    pub transport_state: TransportState,
    pub components: Components,
    pub last_successful_update_check: Option<String>, // RFC3339
    pub update_pending: bool,
    pub floor_release: Option<String>,
    pub clock_fault: bool,
    pub offer: Option<OfferState>,
    pub error_state: Option<ErrorState>,
}

impl DashboardState {
    pub fn new(version: &str) -> Self {
        DashboardState {
            node_id: None,
            category: None,
            version: version.to_string(),
            started: Instant::now(),
            kb: None,
            transport_state: TransportState::NoTransport,
            components: Components {
                identity: false,
                kb: false,
                transport: crate::daemon::health::TransportHealth::Down,
                updater_idle: true,
            },
            last_successful_update_check: None,
            update_pending: false,
            floor_release: None,
            clock_fault: false,
            offer: None,
            error_state: None,
        }
    }

    pub fn uptime_sec(&self) -> u64 {
        self.started.elapsed().as_secs()
    }
}

pub type SharedState = Arc<RwLock<DashboardState>>;

/// Serve on `bind` (127.0.0.1:11180 by default). Asserts loopback unless
/// `lan_opt_in` — LAN binding is an explicit config choice with a printed
/// warning until Layer 4 auth exists (spec §Dashboard).
pub async fn serve(
    state: SharedState,
    bind: std::net::SocketAddr,
    lan_opt_in: bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        lan_opt_in || bind.ip().is_loopback(),
        "dashboard refuses non-loopback bind {bind} without explicit LAN opt-in (111 config dashboard.lan true)"
    );
    let app = routes::router(state);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
