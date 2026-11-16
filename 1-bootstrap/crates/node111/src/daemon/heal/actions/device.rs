//! Layer 9 (§7.1; §7.2 the device rows; §7.4 steps 4 and 5): T0 stub; filled by row D19 (the four
//! device actions ported), row D30 (the `device` vocabulary's three bridge actions) and row D52
//! (the records of an attempt with no incident).
//!
//! The Layer 6 applier's device half moved behind the engine's trait: `rns.interface.upsert`,
//! `rns.interface.remove`, `rns.posture.set` and `radio.region.set`, ported byte for byte, each
//! with a `restore_cold` (a file write at daemon start, before any child starts and before the
//! store opens); `identify` and a `device` proposal's `service.restart` stay Layer 6's one-shots.
//! `kind: device` proposals keep their route, body and results.
//!
//! T0: the name only.
#![allow(dead_code)]

/// The device actions behind the engine's trait (§7.1). T0: the name only.
pub struct DeviceAction;
