//! Layer 7 (§3.1, §3.2, §3.5, §3.8): T0 stub; filled by row D3 (the row's codes and `detail`; the
//! ntfy facts with D11).
//!
//! The `gateway` service row's facts: `gateway_none`, `carrier_ok`, `carrier_pending`,
//! `gateway_off` and the `detail` object (§3.5), the badge's four states, never "online" (§3.8).
//! Ages from a monotonic `Instant`; no probe, timer or request of its own (§3.5, D30 Q14).
#![allow(dead_code)]

use mesh111::fabric::FabricEvent;

/// The `gateway` row's facts (§3.5). T0: the name only.
pub struct GatewayFacts;

/// One fabric event from [`super::tap`] (a paired hub's `reachable`, §3.5 item 3). T0: does
/// nothing.
pub fn on_event(_event: &FabricEvent) {}
