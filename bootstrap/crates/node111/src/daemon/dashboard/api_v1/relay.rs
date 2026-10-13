//! POST /api/v1/chat (streamed) · GET /chat/history · GET /agent/status · POST /agent/task — relay to 111-agent :11182.
//!
//! Owned by task R3 (`os/interface/TASKS.md`). Stub until then: the contract
//! paths are registered so auth + CSRF already apply, each answering
//! `501 problem+json` (`code: internal`, `title: not built yet`). Replace the
//! handlers; keep the paths (see `api_v1/mod.rs` for the router pattern).

use super::{not_built, ApiRouter};
use axum::routing::{get, post};

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/chat", post(not_built))
        .route("/api/v1/chat/history", get(not_built))
        .route("/api/v1/agent/status", get(not_built))
        .route("/api/v1/agent/task", post(not_built))
}
