//! Layer 7 (§13.1, §9.3, §9.5 T4): T0 stub; filled by row L4.
//!
//! Serves (once its row lands, §13.1): `GET /api/v1/update/origins` (the origin order, each
//! origin's last answer and newest seen). The `/origin/v1/*` files are outside `/api/v1` (row D7,
//! `dashboard/routes.rs`). T0: an empty router, merged in `api_v1/mod.rs`; until the row lands
//! these paths match no route and answer the shared problem+json `404` (`routes::api_problems`),
//! like any unknown `/api/*` path.
#![allow(dead_code)]

use super::ApiRouter;

/// The Layer 7 routes of this file (§13.1). T0: none.
pub fn router() -> ApiRouter {
    axum::Router::new()
}
