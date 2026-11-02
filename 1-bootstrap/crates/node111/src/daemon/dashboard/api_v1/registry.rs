//! Layer 7 (§13.1, §7.1, §7.4, §7.7): T0 stub; filled by row D8.
//!
//! Serves (once its row lands, §13.1): `GET /api/v1/registry` and `POST /api/v1/registry/browse` (a
//! human; bearer 401; `409 lan_off`); the SSE `registry` frame. T0: an empty router, merged in
//! `api_v1/mod.rs`; until the row lands these paths match no route and answer the shared
//! problem+json `404` (`routes::api_problems`), like any unknown `/api/*` path.
#![allow(dead_code)]

use super::ApiRouter;

/// The Layer 7 routes of this file (§13.1). T0: none.
pub fn router() -> ApiRouter {
    axum::Router::new()
}
