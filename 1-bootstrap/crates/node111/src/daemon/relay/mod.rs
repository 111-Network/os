//! Layer 7 (§1, §6.3, §5.11): T0 stub; filled by row D10 (status, settings and consents; §13.1) and
//! row D11 (the carrier on the forwarder; §5.9, §6.1–§6.2).
//!
//! The relay and presence subsystem of `os/7-relay/README.md`. Every switch defaults off (§6.3)
//! and, while it is off, nothing is constructed: no socket, task, thread, timer, file read or
//! write, netlink subscription (§1 rule 2, §5.11; the Pi floor, `os/AGENTS.md`). This T0 stub names
//! the submodules and the two entry points the daemon and the fabric call:
//!
//! - [`start`]: called once from `daemon/mod.rs` after the fabric and the Layer 6 parts are in
//!   `ApiState` ext; it returns at once while every switch is off (and, in T0, always).
//! - [`tap`]: hands each fabric event to the presence (§2.1, row D2) and gateway (§3.5, row D3)
//!   consumers. T0 does not wire it: the fan-out of `FabricEvent` batches is
//!   `FabricHandle::publish` in `daemon/fabric/mod.rs`, a file row M5 owns, and M5 adds the one tap
//!   there (Message, Contact, Fabric and Forward events; a no-op while no Layer 7 consumer is on).
#![allow(dead_code)]

pub mod consent;
pub mod gateway;
pub mod linkkey;
pub mod metered;
pub mod netwatch;
pub mod pair;
pub mod presence;
pub mod settings;
pub mod tls;

use crate::daemon::dashboard::ApiState;
use crate::daemon::fabric::FabricHandle;
use mesh111::fabric::FabricEvent;
use std::sync::Arc;

/// Start Layer 7 (§6.3): read `state/relay.json` (D1) and build only what a switch turned on (D10,
/// D11). T0 stub: returns at once; nothing is read, spawned, bound or scheduled.
pub fn start(_api: &Arc<ApiState>, _fabric: &Arc<FabricHandle>) {}

/// One fabric event to the Layer 7 consumers: the presence facts (§2.1, §2.2; D2) and the `gateway`
/// row (§3.5; D3). Called from the fabric's fan-out once M5 hooks it (see the module doc); each
/// consumer returns at once while it is off. T0: both are no-ops.
pub fn tap(event: &FabricEvent) {
    presence::on_event(event);
    gateway::on_event(event);
}
