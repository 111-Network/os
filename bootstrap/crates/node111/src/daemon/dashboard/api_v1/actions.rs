//! POST /api/v1/node/restart · POST /api/v1/update/check (`ApiState::update_check`).
//!
//! Owned by task R2 (`os/interface/TASKS.md`). Stub until then: the contract
//! paths are registered so auth + CSRF already apply, each answering
//! `501 problem+json` (`code: internal`, `title: not built yet`). Replace the
//! handlers; keep the paths (see `api_v1/mod.rs` for the router pattern).

use super::{not_built, ApiRouter};
use axum::routing::post;

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/node/restart", post(not_built))
        .route("/api/v1/update/check", post(not_built))
}
