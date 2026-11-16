//! Layer 8 (§6.4, §6.1; §5.1 `drafts/`): T0 stub; filled by row D28.
//!
//! Drafts and publishing: a draft `state/origin/drafts/<id>@<version>.toml` is written only after
//! its bytes and its sidecar passed; a human's publish builds the new catalogue from the previous
//! one, the drafts named and the removals named, mints its `seq` from the L2 announce counter
//! through the identity handle (each new entry takes the same `seq`) and signs it under
//! `111/catalogue/v1`: one counter value and one signature per publish. The release key never signs
//! a dataset.
#![allow(dead_code)]

/// One entry not yet published (§6.4). T0: the name only.
pub struct Draft;
