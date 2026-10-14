//! `GET /api/v1/nearby` — presence per source (Layer 5 §14 "Nearby", §15;
//! contract `listNearby`). v1 has one kind of source: the bound Meshtastic
//! board's node DB, as validated by `daemon::meshtastic::presence`, grouped
//! as "heard via Meshtastic on <board alias>". Every node string is an
//! **untrusted RF-origin value** (the contract's `NearbyNode` description says
//! so): clients render it as text only and hand it to the agent only inside a
//! labelled untrusted block. A Meshtastic node is never a contact.
//!
//! Without the Meshtastic module, without a bound board, or before the reader
//! has produced a node DB the list is empty — no fake peers. Changes are also
//! pushed as SSE `nearby` frames (`{sources}`) by the module's monitor.

use super::{ApiRouter, ApiState};
use crate::daemon::meshtastic::Meshtastic;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use std::sync::Arc;

async fn list(State(api): State<Arc<ApiState>>) -> Response {
    let sources = api
        .ext::<Arc<Meshtastic>>()
        .map(|m| m.nearby_sources())
        .unwrap_or_default();
    Json(serde_json::json!({ "schema": 1, "sources": sources })).into_response()
}

pub fn router() -> ApiRouter {
    axum::Router::new().route("/api/v1/nearby", get(list))
}
