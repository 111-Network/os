//! Layer 8 (§16.1, §16.2, §16.3): T0 stub; filled by rows D29, D30, D35, D38 and D43.
//!
//! Serves (once its rows land, §16.1): `GET /api/v1/datasets`, `GET /api/v1/datasets/drafts`, `POST
//! /api/v1/datasets/publish` and `DELETE /api/v1/datasets/entries/{publisher}/{id}/{version}`
//! (D29); `POST /api/v1/datasets/adopt`, loopback only, never a session or the bearer (D30); `POST
//! /api/v1/datasets/entries/{publisher}/{id}/{version}/fetch`, `DELETE
//! /api/v1/datasets/jobs/{job}`, `POST /api/v1/datasets/jobs/{job}/resume` and `POST
//! /api/v1/datasets/drafts` (D35); `POST /api/v1/datasets/check` (D38); `GET
//! /api/v1/datasets/search` (D43); the SSE `dataset` frame (§16.3). Entries live under `entries/`,
//! so no `id` meets a literal sibling; the agent's bearer is refused (401) on every non-GET.
//!
//! T0: an empty router, merged in `api_v1/mod.rs`; until the rows land these paths match no route
//! and answer the shared problem+json `404` (`routes::api_problems`), like any unknown `/api/*`
//! path.
#![allow(dead_code)]

use super::ApiRouter;

/// The Layer 8 datasets routes of this file (§16.1). T0: none.
pub fn router() -> ApiRouter {
    axum::Router::new()
}
