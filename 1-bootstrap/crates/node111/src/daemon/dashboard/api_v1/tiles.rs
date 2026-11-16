//! Layer 8 (§10, §16.1, §7.1): T0 stub; filled by row D44.
//!
//! Serves (once its row lands, §16.1): `GET /api/v1/tiles` (the held `pmtiles` sets) and `GET` and
//! `HEAD /api/v1/tiles/{publisher}/{id}/{version}`, the file through the Range arm as
//! `application/octet-stream` (a 416 carries a problem+json body, `range_not_satisfiable`); a
//! paired session or loopback, the agent's bearer reads; no stream cap on these session-gated
//! bodies; the CSP unchanged.
//!
//! T0: an empty router, merged in `api_v1/mod.rs`; until the row lands these paths match no route
//! and answer the shared problem+json `404` (`routes::api_problems`), like any unknown `/api/*`
//! path.
#![allow(dead_code)]

use super::ApiRouter;

/// The Layer 8 tiles routes of this file (§16.1). T0: none.
pub fn router() -> ApiRouter {
    axum::Router::new()
}
