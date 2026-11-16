//! Layer 9 (§18.1, §18.4, §14.1, §17.1): T0 stub; filled by rows D22, D24, D25, D26, D31 to D36,
//! D40, D42, D46, D47, D49 and D53.
//!
//! Serves (once its rows land, §18.1): `GET /api/v1/repairs` (`RepairsStatus`, D22; its `needs_you`
//! from D25, D26, D31 and D53), `GET /api/v1/repairs/incidents` and `/{id}`, `POST
//! /api/v1/repairs/incidents/{id}/dismiss` and `GET /api/v1/repairs/audit` (D22); `POST
//! /api/v1/repairs/incidents/{id}/task` (D24; its `diagnose` re-send D42, its `patch` mode D46, its
//! `kb_draft` mode D49) and `GET /api/v1/repairs/playbooks` (D24); `POST
//! /api/v1/repairs/attempts/{id}/confirm` (D31), `/restore` and `/dismiss` (D32); `GET
//! /api/v1/repairs/changes` and `POST /api/v1/repairs/changes/{id}/undo` (D33; the ledger clear,
//! D34); `GET` and `PATCH /api/v1/repairs/settings` (D35; the write loopback only); `GET
//! /api/v1/repairs/fixes` and `/{id}` (D40) and `POST /api/v1/repairs/fixes/{id}/send` (D36); `GET
//! /api/v1/repairs/drafts`, `/{id}/mbox` and `/{id}/pr` (D47). The agent's bearer is refused (401)
//! on every non-GET.
//!
//! T0: an empty router, merged in `api_v1/mod.rs`; until the rows land these paths match no route
//! and answer the shared problem+json `404` (`routes::api_problems`), like any unknown `/api/*`
//! path.
#![allow(dead_code)]

use super::ApiRouter;

/// The Layer 9 Repairs routes of this file (§18.1). T0: none.
pub fn router() -> ApiRouter {
    axum::Router::new()
}
