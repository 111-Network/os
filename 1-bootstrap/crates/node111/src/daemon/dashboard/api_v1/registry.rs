//! Layer 7 (§13.1, §7.1, §7.4, §7.6, §7.7; row D8): the Service Discovery Registry routes.
//!
//! - `GET /api/v1/registry` → `{schema, rows: [RegistryRow]}` (a session; the agent's bearer
//!   reads, like every GET). Empty until a browse ran. Every string is untrusted text.
//! - `POST /api/v1/registry/browse` → 202 `{schema, started}` (`RelayAccepted`; `started: false`
//!   when a browse task runs already: during its browse its frame is still to come, during its
//!   lost-hub phase this call queues one more browse after it, so a frame always follows); a
//!   DNS-SD browse of `_n111._tcp` for at most 5 s, then `stop_browse`; the rows arrive on the
//!   SSE `registry` frame. `409 lan_off` where no mDNS
//!   daemon runs (LAN sharing off: nothing is started) and on Termux. A human: the agent's
//!   bearer is refused (401) by the global write gate (`auth::require_session`). On a member
//!   with a home hub this is also "Find my hub" (§7.7): the registry matches the rows against
//!   the hub's `k` and keeps an address only after a handshake there named the hub.

use super::{ApiRouter, ApiState, Problem};
use crate::daemon::registry::{self, Refused, Why, SCHEMA};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use serde_json::json;
use std::sync::Arc;

async fn list(State(api): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let rows = registry::get(&api)
        .map(|r| r.rows_json())
        .unwrap_or_default();
    Json(json!({ "schema": SCHEMA, "rows": rows }))
}

/// The `409 lan_off` problem (§13.1).
pub fn lan_off() -> Problem {
    Problem::new(
        StatusCode::CONFLICT,
        "lan_off",
        "Looking for 111 nodes needs LAN sharing",
    )
    .detail(
        "The browse runs only on the mDNS daemon of a LAN-shared dashboard (never on Termux); nothing was started.",
    )
}

async fn browse(State(api): State<Arc<ApiState>>) -> Response {
    let Some(reg) = registry::get(&api) else {
        return lan_off().into_response();
    };
    match reg.browse(Why::Asked) {
        Ok(started) => (
            StatusCode::ACCEPTED,
            Json(json!({ "schema": SCHEMA, "started": started })),
        )
            .into_response(),
        Err(Refused::LanOff) => lan_off().into_response(),
    }
}

/// The Layer 7 routes of this file (§13.1).
pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/registry", get(list))
        .route("/api/v1/registry/browse", post(browse))
}
