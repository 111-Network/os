//! Layer 7 (§2.1–§2.5): T0 stub; filled by row D2.
//!
//! Presence facts from the fabric's events (§2.1, §2.2): `(peer, transport, evidence, local_seq,
//! heard_at)` in memory for the session, feeding the `111` Nearby source (§2.3) and the inert
//! heartbeat validator (§2.4). No timer, no task per contact, no stored byte (§19 C3).
#![allow(dead_code)]

use mesh111::fabric::FabricEvent;

/// The session's presence facts (§2.1). T0: the name only.
pub struct Presence;

/// One fabric event from [`super::tap`] (§2.2: what bumps last heard). T0: does nothing.
pub fn on_event(_event: &FabricEvent) {}
