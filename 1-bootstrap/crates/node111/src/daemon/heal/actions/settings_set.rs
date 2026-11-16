//! Layer 9 (§7.2 `settings.set`; §7.3 rules 2 and 3; §7.6): T0 stub; filled by row D28.
//!
//! `settings.set {key, value}` on `fabric.listen_lan`, `fabric.hub_role` and `fabric.quota.<class>`
//! only, through the `PATCH /fabric/settings` code path under its loopback-only gate (a
//! non-loopback accept answers `403 loopback_only`); a quota goes through Layer 8's `quota.set`.
//! The undo is the previous value; the first two keys are `may_affect`; a lowered quota that would
//! evict stored items asks `heal_remove` first.
//!
//! T0: the name only.
#![allow(dead_code)]

/// `settings.set` on the fabric keys (§7.2). T0: the name only.
pub struct SettingsSet;
