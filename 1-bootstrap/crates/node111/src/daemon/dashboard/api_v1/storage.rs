//! Layer 8 (§16.1, §16.2, §16.3, §15.1): T0 stub; filled by rows D14, D20, D21 and D26.
//!
//! Serves (once its rows land, §16.1): `GET /api/v1/storage` (`StorageStatus`, §16.2; a session or
//! loopback, the agent's bearer reads), `PATCH /api/v1/storage/settings` (a human on loopback only,
//! 403 `loopback_only` otherwise) and `POST /api/v1/storage/export-offer` (D14); `DELETE
//! /api/v1/storage/set-aside/{file}` and `POST /api/v1/storage/start-empty` (loopback only; D20);
//! `POST /api/v1/storage/update-without-copy` (D21); the `origin.*` serving switches with their
//! `datasets_serve` and `components_serve` consents (D26); the SSE `storage` frame (§16.3). The
//! agent's bearer is refused (401) on every non-GET.
//!
//! T0: an empty router, merged in `api_v1/mod.rs`; until the rows land these paths match no route
//! and answer the shared problem+json `404` (`routes::api_problems`), like any unknown `/api/*`
//! path.
#![allow(dead_code)]

use super::ApiRouter;

/// The Layer 8 storage routes of this file (§16.1). T0: none.
pub fn router() -> ApiRouter {
    axum::Router::new()
}
