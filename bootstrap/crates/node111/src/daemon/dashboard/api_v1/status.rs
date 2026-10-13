//! GET /api/v1/status (superset of /api/status: label/bech32/alias, bind, auth, update, layers).
//!
//! Owned by task R2 (`os/interface/TASKS.md`). Stub until then: the contract
//! paths are registered so auth + CSRF already apply, each answering
//! `501 problem+json` (`code: internal`, `title: not built yet`). Replace the
//! handlers; keep the paths (see `api_v1/mod.rs` for the router pattern).

use super::{not_built, ApiRouter};
use axum::routing::get;

pub fn router() -> ApiRouter {
    axum::Router::new().route("/api/v1/status", get(not_built))
}
