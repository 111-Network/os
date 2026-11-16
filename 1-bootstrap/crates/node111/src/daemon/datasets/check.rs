//! Layer 8 (§12.4 "The check"; §6.2): T0 stub; filled by row D38.
//!
//! The catalogue check, with no timer and no gossip: a member asks its home hub for the hub's
//! catalogue with one GET on Layer 7's link-attribution event, only while the hub's signed announce
//! carries `ROLE_DATASETS`, and whenever the user opens the Datasets view or taps Check now (`POST
//! /api/v1/datasets/check`); each source is asked for the home hub's and every followed publisher's
//! catalogue with `If-None-Match`, its bytes counted in the metered window, and no GET is sent
//! while that window's cap is reached.
#![allow(dead_code)]

/// One catalogue check (§12.4). T0: the name only.
pub struct CatalogueCheck;
