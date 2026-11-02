//! Layer 7 (§13.1, §4.6): T0 stub; filled by row D6.
//!
//! Serves (once its row lands, §13.1): `POST /api/v1/relay/pair/offer`, `POST /api/v1/relay/pair`,
//! `DELETE /api/v1/relay/paired/{node_id}`, all loopback only (I15). T0: an empty router, merged in
//! `api_v1/mod.rs`; until the row lands these paths match no route and answer the shared
//! problem+json `404` (`routes::api_problems`), like any unknown `/api/*` path.
#![allow(dead_code)]

use super::ApiRouter;

/// The Layer 7 routes of this file (§13.1). T0: none.
pub fn router() -> ApiRouter {
    axum::Router::new()
}
