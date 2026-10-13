//! GET /api/v1/devices · GET/PATCH/DELETE /api/v1/devices/{id} (device registry, spec §7.1).
//!
//! Owned by task R4 (`os/interface/TASKS.md`). Stub until then: the contract
//! paths are registered so auth + CSRF already apply, each answering
//! `501 problem+json` (`code: internal`, `title: not built yet`). Replace the
//! handlers; keep the paths (see `api_v1/mod.rs` for the router pattern).

use super::{not_built, ApiRouter};
use axum::routing::get;

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/devices", get(not_built))
        .route(
            "/api/v1/devices/{id}",
            get(not_built).patch(not_built).delete(not_built),
        )
}
