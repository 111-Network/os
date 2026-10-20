//! `POST /api/v1/messages/fanout/preview` · `POST /api/v1/messages/fanout` (Layer 6
//! `os/60-transport/README.md` §9; contract `previewFanout`, `sendFanout`).
//!
//! - **Preview** → `FanoutPreview`: a `SendPlan` per target (bytes with the part
//!   overhead, the cap for its kind and mode, parts, the airtime estimate — always
//!   labelled one —, warnings, and the refusal a send would meet). It writes nothing,
//!   transmits nothing and **logs nothing of the text**: this route has no log line at
//!   all, and the bridge core's preview path has none either (a test captures the
//!   process's stderr around a preview and finds no trace of the text).
//! - **Send** → `202 FanoutResult {group_id, items}`: one fresh send group. Validation is
//!   atomic: any target refused for `oversize` in `refuse` mode or for a consent gate
//!   (`ham_unconfirmed`, `uplink_off`) answers `409` with the per-target report in
//!   `targets` and **nothing** is queued. Adapter state (region unset, board away, queue
//!   full, no path) is not a call error: rows queue with their reason. 111 targets are
//!   ordinary sealed messages (`msg-` ids); foreign targets are `fm-` rows, one per part.
//!
//! Both are writes of a human (the agent proposes `kind: bridge_message`): the global
//! write gate refuses the agent's bearer (`401`, §13).

use super::bridge::{body, bridge_of};
use super::{ApiRouter, ApiState};
use crate::daemon::bridge::fanout::FanoutRequest;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Json;
use std::sync::Arc;

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/messages/fanout/preview", post(preview))
        .route("/api/v1/messages/fanout", post(send))
}

async fn preview(State(api): State<Arc<ApiState>>, bytes: Bytes) -> Response {
    let req: FanoutRequest = match body(&bytes) {
        Ok(r) => r,
        Err(p) => return p.into_response(),
    };
    let b = match bridge_of(&api) {
        Ok(b) => b,
        Err(p) => return p.into_response(),
    };
    match b.fanout_preview(&req).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.problem().into_response(),
    }
}

async fn send(State(api): State<Arc<ApiState>>, bytes: Bytes) -> Response {
    let req: FanoutRequest = match body(&bytes) {
        Ok(r) => r,
        Err(p) => return p.into_response(),
    };
    let b = match bridge_of(&api) {
        Ok(b) => b,
        Err(p) => return p.into_response(),
    };
    match b.fanout(&req).await {
        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(e) => e.problem().into_response(),
    }
}
