//! Layer 7 (§1, §6.3, §5.11): the relay and presence subsystem of `os/7-relay/README.md`.
//!
//! Every switch defaults off (§6.3) and, while it is off, nothing is constructed: no socket, task,
//! thread, timer, file write or netlink subscription (§1 rule 2, §5.11; the Pi floor,
//! `os/AGENTS.md`). This module names the submodules and the entry points the daemon, the fabric
//! and the other rows call:
//!
//! - [`start`]: called once from `daemon/mod.rs` after the fabric and the Layer 6 parts are in
//!   `ApiState` ext. It reads `state/relay.json` once (row D1, into the settings cell of the
//!   `/relay` routes, row D10) and builds only what a switch turned on: the presence book (no
//!   switch, nothing spawned; D2), the registry's record (D8), the `gateway` row's facts (only
//!   once a gateway or carrier is configured; D3), the carrier hook ([`CarrierHook`]; D11 replaces
//!   the no-op one), and the binding of the `relay` frame ([`changed`]). Nothing is spawned,
//!   scheduled, bound or subscribed by D10's part of it.
//! - [`tap`]: hands each fabric event to the presence (§2.1, D2) and gateway (§3.5, D3) consumers
//!   and to the `relay` frame (§13.1, D10). It is called from the fabric's fan-out (Message,
//!   Contact, Fabric and Forward events, `FabricHandle::publish`); each consumer returns at once
//!   while it is off.
//! - [`changed`]: the `relay` frame's change notifier for the other modules (the carrier's check
//!   and flush outcomes, D11; a pairing attempt that ends later, D6): it publishes a full
//!   `RelayStatus` frame (§13.1). No timer: a frame is built only when something calls this or a
//!   fabric event moved the pair set.
//! - [`carrier_hook`]: the seam the `/relay/carrier/check` and `/flush` routes and the relay
//!   proposals hand to (§13.1). Until row D11 registers the carrier's own hook, the registered
//!   [`NoopCarrier`] records each request and starts nothing.

pub mod consent;
pub mod gateway;
pub mod linkkey;
pub mod metered;
pub mod netwatch;
pub mod pair;
// The T0 stub's module-wide `allow(dead_code)` is gone; it stays only here, where row D2's
// `presence::Fact.heard_at` is written and never read (D2's file).
#[allow(dead_code)]
pub mod presence;
pub mod settings;
pub mod tls;

use crate::daemon::dashboard::api_v1::relay7;
use crate::daemon::dashboard::ApiState;
use crate::daemon::fabric::FabricHandle;
use mesh111::fabric::FabricEvent;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Start Layer 7 (§6.3): read `state/relay.json` (D1) and build only what a switch turned on.
pub fn start(api: &Arc<ApiState>, fabric: &Arc<FabricHandle>) {
    // §6.3 (D10): the settings as the routes serve them, read once here (an invalid file is
    // moved aside by D1's load, here at start rather than on a later request).
    let s = relay7::cell(api).settings();
    // §2.1–§2.3 (D2): presence has no switch (event-derived on every node); this binds the empty
    // book to the fabric handle and lists the `111` Nearby source. Nothing spawned or scheduled.
    presence::install(api, fabric);
    // §7.1 (D8): the registry binds the fabric and syncs the `_n111._tcp` record once; it holds
    // an mDNS daemon only where the dashboard's announcer runs (nothing started here).
    crate::daemon::registry::install(api, fabric);
    // §3.5 (D3): the `gateway` row's switches; nothing is constructed while none is on.
    gateway::apply(fabric, &s);
    // §6.1–§6.2 (D11): the carrier is built here by its hook, only while its switches hold.
    carrier_hook(api).reconfigure(&s);
    // §13.1 (D10): the `relay` frame follows this daemon's fabric (no task until a change).
    relay7::bind(api, fabric);
}

/// One fabric event to the Layer 7 consumers: the presence facts (§2.1, §2.2; D2), the `gateway`
/// row (§3.5; D3) and the `relay` frame (§13.1; D10). Called from the fabric's fan-out; each
/// consumer returns at once while it is off.
pub fn tap(event: &FabricEvent) {
    presence::on_event(event);
    gateway::on_event(event);
    relay7::on_event(event);
}

/// The `relay` frame's change notifier (§13.1): publish a full `RelayStatus` on the daemon's event
/// bus soon (coalesced: one build in flight, one more queued). For the modules whose facts are in
/// the status but reach it through no fabric event: the carrier's check and flush (D11), a pairing
/// attempt that ends after its route answered (D6). A no-op before [`start`] bound the frame and
/// outside a Tokio runtime.
pub fn changed() {
    relay7::changed();
}

// ---------------------------------------------------------------- the carrier seam (§13.1)

/// What the carrier routes and the relay proposals hand to (§13.1 `POST /relay/carrier/check`,
/// `/flush`; §13.2 `carrier.flush {}`), and what the status reads of the carrier's budget (§12).
/// Row D11 registers its implementation in `ApiState` ext (`Arc<dyn CarrierHook>`, inserted
/// before [`start`] or replacing the no-op one); the routes have already refused the request when
/// no carrier is configured, its `carrier_enable` consent is not the one owed now, the metered cap
/// is reached ([`CarrierHook::metered_cap_reached`]) or a licensed board would carry it
/// ([`CarrierHook::licensed_board`]).
pub trait CarrierHook: Send + Sync {
    /// The relay settings at [`start`] and after every write of them: build, rebuild or tear down
    /// what the switches say (§6.3: nothing while `carrier.kind` is `none` or `gateway.enabled`
    /// is off).
    fn reconfigure(&self, settings: &settings::RelaySettings);
    /// Run the carrier's check now (§3.1). Returns whether a request was started.
    fn check(&self) -> bool;
    /// Flush the carrier now (§5.9, §6.2 "Flush now"). Returns whether a flush was started.
    fn flush(&self) -> bool;
    /// The rolling 24 h cap of a metered link is reached (§12): no flush runs.
    fn metered_cap_reached(&self) -> bool;
    /// The carrier's path would ride a board marked licensed (§12: a hard block, no override).
    fn licensed_board(&self) -> bool;
}

/// The hook registered until row D11 lands: it records each request (a count) and starts nothing,
/// so `check` and `flush` answer `202` with `started: false`. No cap is ever reached and no board
/// is licensed: v1 has no sealed LoRa path (§12), and the metered budget is D11's.
#[derive(Debug, Default)]
pub struct NoopCarrier {
    checks: AtomicU64,
    flushes: AtomicU64,
    reconfigured: AtomicU64,
}

impl NoopCarrier {
    /// `check` requests recorded.
    pub fn checks(&self) -> u64 {
        self.checks.load(Ordering::Relaxed)
    }
    /// `flush` requests recorded.
    pub fn flushes(&self) -> u64 {
        self.flushes.load(Ordering::Relaxed)
    }
    /// `reconfigure` calls recorded.
    pub fn reconfigured(&self) -> u64 {
        self.reconfigured.load(Ordering::Relaxed)
    }
}

impl CarrierHook for NoopCarrier {
    fn reconfigure(&self, _settings: &settings::RelaySettings) {
        self.reconfigured.fetch_add(1, Ordering::Relaxed);
    }
    fn check(&self) -> bool {
        self.checks.fetch_add(1, Ordering::Relaxed);
        false
    }
    fn flush(&self) -> bool {
        self.flushes.fetch_add(1, Ordering::Relaxed);
        false
    }
    fn metered_cap_reached(&self) -> bool {
        false
    }
    fn licensed_board(&self) -> bool {
        false
    }
}

/// The registered carrier hook (`ApiState` ext `Arc<dyn CarrierHook>`), the no-op one
/// ([`NoopCarrier`]) inserted at the first use when none is registered.
pub fn carrier_hook(api: &ApiState) -> Arc<dyn CarrierHook> {
    api.ext_or_insert_with(|| Arc::new(NoopCarrier::default()) as Arc<dyn CarrierHook>)
}
