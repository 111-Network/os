//! Layer 8 (§13.6 (2); §15.1 `update_without_copy`; §2.5): T0 stub; filled by row D17.
//!
//! The end of the migration window: once the probation journal (`state/update.json`) shows the
//! running release confirmed, `111d` deletes every `*.pre-v<N>` store copy, `agent/legacy-v0/` and
//! `state/heal/legacy-v0.jsonl`, and forgets this release's `update_without_copy` entries; at the
//! first start under another release it forgets the reverted release's entries. The journal is read
//! at start, and on the pass only while a copy, a legacy file or an entry exists (one `stat`). The
//! copy itself is mesh111's (`store/copy.rs`, row M6).
#![allow(dead_code)]

/// What this release's migrations left to delete or forget (§13.6 (2)). T0: the name only.
pub struct MigrationLeftovers;
