//! GET /api/v1/proposals · /proposals/{id} · POST /proposals/{id}/decision.
//!
//! Owned by task R3 (`os/interface/TASKS.md`). Stub until then: the contract
//! paths are registered so auth + CSRF already apply, each answering
//! `501 problem+json` (`code: internal`, `title: not built yet`). Replace the
//! handlers; keep the paths (see `api_v1/mod.rs` for the router pattern).

use super::{not_built, ApiRouter};
use axum::routing::{get, post};

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/proposals", get(not_built))
        .route("/api/v1/proposals/{id}", get(not_built))
        .route("/api/v1/proposals/{id}/decision", post(not_built))
}
