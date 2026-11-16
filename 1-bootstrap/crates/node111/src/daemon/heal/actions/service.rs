//! Layer 9 (§7.2 `service.restart`; §3.4 rule 2; §7.5): T0 stub; filled by row D27.
//!
//! `service.restart {service_id}` as a repair: `restart_service`, which also clears `Degraded`; its
//! probe, within 30 s, asserts that the service's published row is out of `error` (the registry's
//! two-tick confirm) and that no new exit with `requested: false` arrived. A restart cannot be
//! undone and arms no undo; being requested, it opens no incident.
//!
//! T0: the name only.
#![allow(dead_code)]

/// `service.restart` (§7.2). T0: the name only.
pub struct ServiceRestart;
