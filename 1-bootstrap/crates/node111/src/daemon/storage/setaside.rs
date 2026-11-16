//! Layer 8 (§13.7, §13.2, §13.6 (3); §2.7 S3; §16.2 `set_aside_store`): T0 stub; filled by row D20.
//!
//! The set-aside store files beside both stores, `<file>.rolled-back-v<N>`, `<file>.damaged-<n>`
//! and `<file>.replaced-<n>`, listed as `set_aside_store` notices `{file, bytes}` (one `readdir` of
//! `fabric/` and `node/` per request); `DELETE /api/v1/storage/set-aside/{file}` and `POST
//! /api/v1/storage/start-empty`, both loopback only, are served from `api_v1/storage.rs`. While a
//! set-aside fabric store exists the open's blob sweep removes no blob file (mesh111's S3).
//!
//! T0: [`notices`] is empty: nothing is listed.
#![allow(dead_code)]

/// One set-aside store file (§16.2 `set_aside_store`). T0: the name only.
pub struct SetAsideStore;

/// The `set_aside_store` notices (§16.2). T0: none.
pub fn notices() -> Vec<SetAsideStore> {
    Vec::new()
}
