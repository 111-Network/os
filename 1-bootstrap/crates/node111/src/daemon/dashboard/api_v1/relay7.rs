//! Layer 7 (§13.1, §6.3, §12): T0 stub; filled by row D10 (the carrier routes go live with D11).
//!
//! Serves (once its row lands, §13.1): `GET /api/v1/relay` (`RelayStatus`), `PATCH
//! /api/v1/relay/settings`, `POST /api/v1/relay/carrier/check` and `POST
//! /api/v1/relay/carrier/flush`; the SSE `relay` frame. Not `api_v1/relay.rs`, which is Layer 4's
//! agent relay. T0: an empty router, merged in `api_v1/mod.rs`; until the row lands these paths
//! match no route and answer the shared problem+json `404` (`routes::api_problems`), like any
//! unknown `/api/*` path.
#![allow(dead_code)]

use super::ApiRouter;

/// The Layer 7 routes of this file (§13.1). T0: none.
pub fn router() -> ApiRouter {
    axum::Router::new()
}
