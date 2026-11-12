//! The `gateway` service row's facts (Layer 7 §3.1, §3.2, §3.5, §3.8; D30 Q14).
//!
//! The 60 s WAN probe is gone (§3.5 item 1): nothing here probes, polls, schedules or sends a
//! request. The row is the gateway badge's home and shows one of four states, never "online"
//! (§3.8): `gateway_none` (no gateway configured), `carrier_ok` (the carrier answered at its last
//! check), `carrier_pending` (the last check failed, none ran yet, or items wait) and
//! `gateway_off` (the owner turned it off). Its `detail` is the contract's `GatewayDetail`
//! `{carrier, endpoint, mode, checked_ago_s, last_flush_ago_s, pending, error}`; every age is a
//! monotonic [`Instant`] difference taken when the row is read, never a wall-clock stamp.
//!
//! **What "internet" rests on (§3.2).** Either the owner's carrier (ntfy, §6.2: its checks and
//! flushes are D11's and arrive through [`record_check`], [`record_flush`] and [`set_pending`]),
//! or a paired hub reached over the internet: a whole `tcp` pair whose daemon half is not marked
//! LAN (§4.1 rule 1), whose fabric `reachable` (a connection attributed to it, M4) is the check.
//!
//! **Weight (§1 rule 2, §5.11; the Pi floor).** Nothing exists until [`apply`] meets a switch that
//! is on (`gateway.enabled`, or a carrier configured with its consent): until then [`on_event`]
//! is one `OnceLock` read and [`facts`] returns the `gateway_none` default. Once armed, a fabric
//! event only marks the facts stale (one atomic store); the services registry's existing 2 s
//! tick ([`refresh`]) re-reads them on its next pass: the pair halves from the fabric handle's
//! in-memory copy, and one `Fabric::status` call only while the gateway is on, this node holds
//! the hub role and an internet pair exists. No task, timer, socket or file of its own.
//!
//! **Never a member's view of its hub (§3.8, I5).** Only this node's own switch and its own
//! pairs are read; a member learns its hub's gateway state only as §7.5 says.

use crate::daemon::fabric::settings::Persisted;
use crate::daemon::fabric::FabricHandle;
use crate::daemon::relay::settings::{CarrierKind, ConsentKind, RelaySettings};
use id111::encoding::NodeIdDisplay;
use id111::keyset::NodeId;
use mesh111::fabric::{FabricEvent, PairVia, PairView};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Instant;

/// `detail.carrier` for the ntfy carrier (§6.2).
pub const CARRIER_NTFY: &str = "ntfy";
/// `detail.carrier` for a paired hub reached over the internet (§3.2, §4.1).
pub const CARRIER_PAIRED_HUB: &str = "paired-hub";
/// The mode a paired hub runs in: its `tcp-lan` connection exists anyway, so it is the held
/// form (§3.1 "5B ... or the paired hub's `tcp-lan` connection").
pub const PAIRED_HUB_MODE: &str = "5B";

/// The badge's four states (§3.8), which are also the row's reason codes (§3.5 item 2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GatewayState {
    /// "Not a gateway": no gateway configured (state `unset`).
    #[default]
    None,
    /// "Gateway: carrier reachable at last check, N min ago" (state `live`).
    CarrierOk,
    /// "Gateway: last flush N min ago, K waiting": the last check failed, none ran yet, or items
    /// wait (state `unset`).
    CarrierPending,
    /// "Gateway off (by you)" (state `unset`).
    Off,
}

impl GatewayState {
    /// The `reason_code` (contract `RelayGatewayState`).
    pub fn code(self) -> &'static str {
        match self {
            GatewayState::None => "gateway_none",
            GatewayState::CarrierOk => "carrier_ok",
            GatewayState::CarrierPending => "carrier_pending",
            GatewayState::Off => "gateway_off",
        }
    }
}

/// One tick's view of the `gateway` row (`services::Facts::gateway`): what [`facts`] read, ages
/// already taken. Tests build it directly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GatewayFacts {
    /// This node can be a gateway: it holds the hub role and has something to carry on (a
    /// configured carrier, or with the gateway on an internet pair) (§3.1).
    pub configured: bool,
    /// `gateway.enabled` (§6.3).
    pub enabled: bool,
    /// The carrier answered at its last check; `None` before one ran (§3.2).
    pub answered: Option<bool>,
    /// `ntfy`, `paired-hub` or `None` (contract: open vocabulary).
    pub carrier: Option<&'static str>,
    /// The carrier's host, or the paired hub's label. Display only.
    pub endpoint: Option<String>,
    /// `5A`, `5B` or `None`.
    pub mode: Option<&'static str>,
    /// Seconds since the last check (monotonic); `None` before one ran.
    pub checked_ago_s: Option<u64>,
    /// Seconds since the last flush (monotonic); `None` before one ran.
    pub last_flush_ago_s: Option<u64>,
    /// Items waiting for the carrier.
    pub pending: u64,
    /// Why the last check failed.
    pub error: Option<String>,
}

impl GatewayFacts {
    /// The badge state (§3.5 item 2, §3.8).
    pub fn state(&self) -> GatewayState {
        if !self.configured {
            GatewayState::None
        } else if !self.enabled {
            GatewayState::Off
        } else if self.answered == Some(true) && self.pending == 0 {
            GatewayState::CarrierOk
        } else {
            GatewayState::CarrierPending
        }
    }

    /// The row's `detail`: the contract's `GatewayDetail` (§3.5 item 3).
    pub fn detail(&self) -> Value {
        json!({
            "carrier": self.carrier,
            "endpoint": self.endpoint,
            "mode": self.mode,
            "checked_ago_s": self.checked_ago_s,
            "last_flush_ago_s": self.last_flush_ago_s,
            "pending": self.pending,
            "error": self.error,
        })
    }
}

/// The configured carrier the claim may rest on (§6.3): `carrier.kind` set, its server set and
/// its `carrier_enable` consent recorded (a carrier without its consent never runs, so it is not
/// configured; fail closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarrierSwitch {
    /// `detail.carrier`.
    pub kind: &'static str,
    /// The server's host (display only).
    pub endpoint: Option<String>,
    /// `5A` or `5B` as this node runs it (5B runs as 5A on Termux).
    pub mode: &'static str,
}

/// What the switches say (§6.3), from `state/relay.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GatewayConfig {
    /// `gateway.enabled`.
    pub enabled: bool,
    /// A configured carrier.
    pub carrier: Option<CarrierSwitch>,
}

impl GatewayConfig {
    /// The switches as `r` holds them (`termux`: this node runs on Termux).
    pub fn from_settings(r: &RelaySettings, termux: bool) -> GatewayConfig {
        let carrier = match r.carrier_kind() {
            CarrierKind::None => None,
            CarrierKind::Ntfy => r
                .ntfy_server()
                .filter(|_| r.consent_hash(ConsentKind::CarrierEnable).is_some())
                .map(|server| CarrierSwitch {
                    kind: CARRIER_NTFY,
                    endpoint: url_host(server),
                    mode: r.carrier_mode_for(termux).as_str(),
                }),
        };
        GatewayConfig {
            enabled: r.gateway_enabled(),
            carrier,
        }
    }

    /// Anything here needs the facts kept: the gateway is on, or a carrier is configured (so
    /// "off by you" can be told from "not a gateway").
    pub fn armed(&self) -> bool {
        self.enabled || self.carrier.is_some()
    }
}

/// The host of an `http(s)://` URL the settings already checked (display only).
pub fn url_host(url: &str) -> Option<String> {
    let uri: ureq::http::Uri = url.parse().ok()?;
    uri.host().filter(|h| !h.is_empty()).map(str::to_string)
}

/// One paired hub reached over the internet, as the last check saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairFact {
    /// The far hub.
    pub node_id: NodeId,
    /// A connection is attributed to it (M4 `PairView::reachable`, not broken).
    pub reachable: bool,
    /// Forward rows waiting for it (`PairView::pending`).
    pub pending: u32,
}

impl PairFact {
    /// The far hub's L2 label (`xxxx-xxxx-xx`), the contract's `endpoint` for a paired hub.
    pub fn label(&self) -> String {
        NodeIdDisplay::new(self.node_id.as_bytes()).label()
    }
}

/// The pairs a paired-hub claim rests on (§3.2, §4.1): whole `tcp` pairs whose daemon half is
/// present and not marked LAN, with the hub role on (with it off no pairing path runs, M4b). A
/// `carrier` pair is the carrier's (D11); a pair without its daemon half is skipped (fail closed).
pub fn internet_pairs(p: &Persisted) -> Vec<NodeId> {
    if !p.fabric.hub_role {
        return Vec::new();
    }
    p.fabric
        .paired_hubs
        .iter()
        .filter(|h| h.via == PairVia::Tcp && h.whole())
        .filter(|h| p.pair_link(&h.node_id).is_some_and(|l| !l.lan))
        .map(|h| h.node_id)
        .collect()
}

/// The fabric's view of `ids` (`FabricStatus::paired`): a pair the view lacks, or holds as not
/// whole or broken, is not reachable and holds nothing.
pub fn pair_facts(ids: &[NodeId], paired: &[PairView]) -> Vec<PairFact> {
    ids.iter()
        .map(|id| {
            let v = paired
                .iter()
                .find(|v| v.hub.node_id == *id && v.whole && !v.broken);
            PairFact {
                node_id: *id,
                reachable: v.is_some_and(|v| v.reachable),
                pending: v.map_or(0, |v| v.pending),
            }
        })
        .collect()
}

/// The ntfy carrier's facts (D11 feeds them).
#[derive(Debug, Default)]
struct CarrierFacts {
    checked: Option<Instant>,
    answered: Option<bool>,
    error: Option<String>,
    last_flush: Option<Instant>,
    pending: u32,
}

#[derive(Debug, Default)]
struct Inner {
    config: GatewayConfig,
    hub_role: bool,
    pairs: Vec<PairFact>,
    pairs_checked: Option<Instant>,
    pairs_error: Option<String>,
    carrier: CarrierFacts,
}

/// The state holder behind the free functions (one per daemon, in a process static; tests build
/// their own).
#[derive(Debug, Default)]
pub struct Gateway {
    /// [`GatewayConfig::armed`]: the one load [`Gateway::on_event`] makes.
    armed: AtomicBool,
    /// A fabric event arrived since the last [`Gateway::refresh`].
    stale: AtomicBool,
    inner: Mutex<Inner>,
    fabric: Mutex<Option<Weak<FabricHandle>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn ago(at: Option<Instant>, now: Instant) -> Option<u64> {
    at.map(|t| now.saturating_duration_since(t).as_secs())
}

impl Gateway {
    /// Nothing configured.
    pub fn new() -> Gateway {
        Gateway::default()
    }

    /// Take the switches (§6.3) and the fabric handle the facts are read from. Disarming drops
    /// every fact held (a later arm starts from nothing).
    pub fn configure(&self, cfg: GatewayConfig, fabric: Option<&Arc<FabricHandle>>) {
        let armed = cfg.armed();
        {
            let mut i = lock(&self.inner);
            if !armed {
                *i = Inner::default();
            }
            i.config = cfg;
        }
        *lock(&self.fabric) = fabric.filter(|_| armed).map(Arc::downgrade);
        self.armed.store(armed, Ordering::SeqCst);
        self.stale.store(armed, Ordering::SeqCst);
    }

    /// One fabric event from the tap (§3.5): a change of links, contacts or custody marks the
    /// pair facts stale for the next [`Gateway::refresh`]. Returns at once while disarmed.
    pub fn on_event(&self, event: &FabricEvent) {
        if !self.armed.load(Ordering::Relaxed) {
            return;
        }
        if matches!(event, FabricEvent::Fabric | FabricEvent::Contact { .. }) {
            self.stale.store(true, Ordering::SeqCst);
        }
    }

    /// The facts are stale and something is configured.
    pub fn needs_refresh(&self) -> bool {
        self.armed.load(Ordering::SeqCst) && self.stale.load(Ordering::SeqCst)
    }

    /// Re-read the pair facts when stale (called from the services registry's existing tick; no
    /// timer of its own). The pair halves come from the handle's in-memory copy; reachability
    /// from one `Fabric::status` call, made only while the gateway is on, the hub role is held
    /// and an internet pair exists.
    pub async fn refresh(&self) {
        if !self.armed.load(Ordering::SeqCst) || !self.stale.swap(false, Ordering::SeqCst) {
            return;
        }
        let handle = lock(&self.fabric).as_ref().and_then(Weak::upgrade);
        let Some(fh) = handle else { return };
        let persisted = fh.persisted();
        let ids = internet_pairs(&persisted);
        let enabled = lock(&self.inner).config.enabled;
        let paired = if enabled && !ids.is_empty() {
            match fh.fabric() {
                Some(f) => Some(
                    f.status()
                        .await
                        .map(|st| st.paired)
                        .map_err(|e| e.to_string()),
                ),
                None => Some(Err("the message fabric is not running".to_string())),
            }
        } else {
            None
        };
        self.observe(persisted.fabric.hub_role, &ids, paired, Instant::now());
    }

    /// Record one read of the pair facts: `paired` is `None` when no status call was made (the
    /// gateway off, or no internet pair), else the fabric's answer.
    pub fn observe(
        &self,
        hub_role: bool,
        ids: &[NodeId],
        paired: Option<Result<Vec<PairView>, String>>,
        now: Instant,
    ) {
        let mut i = lock(&self.inner);
        i.hub_role = hub_role;
        match paired {
            None => {
                // nothing checked: the pair set is known, its reachability is not
                let old = std::mem::take(&mut i.pairs);
                i.pairs = ids
                    .iter()
                    .map(|id| {
                        old.iter()
                            .find(|p| p.node_id == *id)
                            .cloned()
                            .unwrap_or(PairFact {
                                node_id: *id,
                                reachable: false,
                                pending: 0,
                            })
                    })
                    .collect();
            }
            Some(Ok(views)) => {
                i.pairs = pair_facts(ids, &views);
                i.pairs_checked = Some(now);
                i.pairs_error = (!i.pairs.iter().any(|p| p.reachable))
                    .then(|| "No paired hub is connected.".to_string());
            }
            Some(Err(e)) => {
                i.pairs = pair_facts(ids, &[]);
                i.pairs_checked = Some(now);
                i.pairs_error = Some(e);
            }
        }
    }

    /// D11: one carrier check ended (§3.2: the drain is the check); `Err` carries why it failed.
    pub fn record_check(&self, outcome: Result<(), String>, now: Instant) {
        let mut i = lock(&self.inner);
        i.carrier.checked = Some(now);
        i.carrier.answered = Some(outcome.is_ok());
        i.carrier.error = outcome.err();
    }

    /// D11: one flush ended (§6.2).
    pub fn record_flush(&self, now: Instant) {
        lock(&self.inner).carrier.last_flush = Some(now);
    }

    /// D11: the items waiting for the carrier (`FabricEvent::Forward::pending`, summed).
    pub fn set_pending(&self, pending: u32) {
        lock(&self.inner).carrier.pending = pending;
    }

    /// The row's facts at `now` (ages taken here). The configured carrier is the claim when one
    /// is set (the owner's chosen endpoint, §3.2); else the internet pairs.
    pub fn facts(&self, now: Instant) -> GatewayFacts {
        let i = lock(&self.inner);
        let pairs_on = i.config.enabled && !i.pairs.is_empty();
        let configured = i.hub_role && (i.config.carrier.is_some() || pairs_on);
        let mut f = GatewayFacts {
            configured,
            enabled: i.config.enabled,
            ..GatewayFacts::default()
        };
        if !configured {
            return f;
        }
        if let Some(c) = &i.config.carrier {
            f.carrier = Some(c.kind);
            f.endpoint = c.endpoint.clone();
            f.mode = Some(c.mode);
            f.answered = i.carrier.answered;
            f.checked_ago_s = ago(i.carrier.checked, now);
            f.last_flush_ago_s = ago(i.carrier.last_flush, now);
            f.pending = u64::from(i.carrier.pending);
            f.error = i.carrier.error.clone();
        } else {
            let shown = i
                .pairs
                .iter()
                .find(|p| p.reachable)
                .or_else(|| i.pairs.first());
            f.carrier = Some(CARRIER_PAIRED_HUB);
            f.endpoint = shown.map(PairFact::label);
            f.mode = Some(PAIRED_HUB_MODE);
            f.answered = i.pairs_checked.map(|_| i.pairs.iter().any(|p| p.reachable));
            f.checked_ago_s = ago(i.pairs_checked, now);
            f.pending = i.pairs.iter().map(|p| u64::from(p.pending)).sum();
            f.error = i.pairs_error.clone();
        }
        f
    }
}

static GATEWAY: OnceLock<Gateway> = OnceLock::new();

/// Hand the relay settings to the row (D10 calls it at start, after `state/relay.json` is read,
/// and after every successful write of it). While nothing is on and nothing was ever on, nothing
/// is constructed.
pub fn apply(fabric: &Arc<FabricHandle>, settings: &RelaySettings) {
    let cfg = GatewayConfig::from_settings(settings, crate::config::on_termux());
    if !cfg.armed() && GATEWAY.get().is_none() {
        return;
    }
    GATEWAY
        .get_or_init(Gateway::new)
        .configure(cfg, Some(fabric));
}

/// One fabric event from [`super::tap`] (§3.5 item 3): returns at once while nothing is on.
pub fn on_event(event: &FabricEvent) {
    if let Some(g) = GATEWAY.get() {
        g.on_event(event);
    }
}

/// Re-read stale facts (the services registry's tick; a no-op while nothing is on).
pub async fn refresh() {
    if let Some(g) = GATEWAY.get() {
        if g.needs_refresh() {
            g.refresh().await;
        }
    }
}

/// The row's facts now (`gateway_none` while nothing is on).
pub fn facts() -> GatewayFacts {
    GATEWAY
        .get()
        .map(|g| g.facts(Instant::now()))
        .unwrap_or_default()
}

/// D11: one carrier check ended (see [`Gateway::record_check`]).
pub fn record_check(outcome: Result<(), String>) {
    if let Some(g) = GATEWAY.get() {
        g.record_check(outcome, Instant::now());
    }
}

/// D11: one flush ended (see [`Gateway::record_flush`]).
pub fn record_flush() {
    if let Some(g) = GATEWAY.get() {
        g.record_flush(Instant::now());
    }
}

/// D11: the items waiting for the carrier (see [`Gateway::set_pending`]).
pub fn set_pending(pending: u32) {
    if let Some(g) = GATEWAY.get() {
        g.set_pending(pending);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::fabric::settings::PairLink;
    use crate::daemon::relay::settings::CarrierMode;
    use mesh111::fabric::{PairRole, PairedHub};
    use std::time::Duration;

    fn nid(b: u8) -> NodeId {
        NodeId::from([b; 32])
    }

    fn view(id: NodeId, reachable: bool, pending: u32) -> PairView {
        PairView {
            hub: PairedHub {
                node_id: id,
                role: PairRole::Peer,
                via: PairVia::Tcp,
                ours: true,
                theirs: true,
            },
            whole: true,
            broken: false,
            reachable,
            pending,
            reposts: 0,
        }
    }

    fn on() -> GatewayConfig {
        GatewayConfig {
            enabled: true,
            carrier: None,
        }
    }

    fn ntfy() -> CarrierSwitch {
        CarrierSwitch {
            kind: CARRIER_NTFY,
            endpoint: Some("ntfy.example.net".into()),
            mode: "5A",
        }
    }

    #[test]
    fn nothing_configured_is_gateway_none_and_the_tap_returns_at_once() {
        let g = Gateway::new();
        g.on_event(&FabricEvent::Fabric);
        assert!(!g.needs_refresh(), "a disarmed row marks nothing");
        let f = g.facts(Instant::now());
        assert_eq!(f, GatewayFacts::default());
        assert_eq!(f.state(), GatewayState::None);
        assert_eq!(f.state().code(), "gateway_none");
        // the process static stays empty until a switch is on
        assert_eq!(facts().state(), GatewayState::None);
    }

    #[test]
    fn the_detail_is_the_contract_shape() {
        let d = GatewayFacts::default().detail();
        let mut keys: Vec<&str> = d.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "carrier",
                "checked_ago_s",
                "endpoint",
                "error",
                "last_flush_ago_s",
                "mode",
                "pending"
            ]
        );
        for k in [
            "carrier",
            "endpoint",
            "mode",
            "checked_ago_s",
            "last_flush_ago_s",
            "error",
        ] {
            assert!(d[k].is_null(), "{k}");
        }
        assert_eq!(d["pending"], 0);
        for gone in ["online", "probe_url", "checked_at", "interval_sec"] {
            assert!(d.get(gone).is_none(), "{gone} retired");
        }
    }

    #[test]
    fn the_four_states() {
        let mut f = GatewayFacts::default();
        assert_eq!(f.state(), GatewayState::None);
        f.configured = true;
        assert_eq!(f.state(), GatewayState::Off);
        f.enabled = true;
        assert_eq!(f.state(), GatewayState::CarrierPending, "no check yet");
        f.answered = Some(false);
        assert_eq!(f.state(), GatewayState::CarrierPending);
        f.answered = Some(true);
        assert_eq!(f.state(), GatewayState::CarrierOk);
        f.pending = 3;
        assert_eq!(f.state(), GatewayState::CarrierPending, "items wait");
        let codes: Vec<&str> = [
            GatewayState::None,
            GatewayState::CarrierOk,
            GatewayState::CarrierPending,
            GatewayState::Off,
        ]
        .iter()
        .map(|s| s.code())
        .collect();
        assert_eq!(
            codes,
            [
                "gateway_none",
                "carrier_ok",
                "carrier_pending",
                "gateway_off"
            ]
        );
    }

    #[test]
    fn the_paired_hub_carrier_fact() {
        let g = Gateway::new();
        g.configure(on(), None);
        assert!(g.needs_refresh(), "arming marks the facts stale");
        let (a, b) = (nid(1), nid(2));
        let t0 = Instant::now();
        // not a hub: nothing to carry on
        g.observe(false, &[], None, t0);
        assert_eq!(g.facts(t0).state(), GatewayState::None);
        // a hub with an internet pair, not reachable at the check
        g.observe(true, &[a, b], Some(Ok(vec![view(a, false, 0)])), t0);
        let f = g.facts(t0);
        assert_eq!(f.state(), GatewayState::CarrierPending);
        assert_eq!(f.carrier, Some(CARRIER_PAIRED_HUB));
        assert_eq!(f.mode, Some(PAIRED_HUB_MODE));
        assert_eq!(f.answered, Some(false));
        assert!(f.error.is_some());
        // b's connection is attributed: the claim rests on it
        g.observe(
            true,
            &[a, b],
            Some(Ok(vec![view(a, false, 0), view(b, true, 0)])),
            t0,
        );
        let f = g.facts(t0);
        assert_eq!(f.state(), GatewayState::CarrierOk);
        assert_eq!(f.endpoint, Some(NodeIdDisplay::new(b.as_bytes()).label()));
        assert_eq!(f.error, None);
        assert_eq!(f.last_flush_ago_s, None);
        // rows waiting for a pair: pending
        g.observe(
            true,
            &[a, b],
            Some(Ok(vec![view(a, false, 2), view(b, true, 1)])),
            t0,
        );
        let f = g.facts(t0);
        assert_eq!((f.state(), f.pending), (GatewayState::CarrierPending, 3));
        // a broken pair is not reachable
        let mut v = view(b, true, 0);
        v.broken = true;
        g.observe(true, &[b], Some(Ok(vec![v])), t0);
        assert_eq!(g.facts(t0).answered, Some(false));
        // the fabric did not answer: the check failed
        g.observe(true, &[b], Some(Err("stopped".into())), t0);
        let f = g.facts(t0);
        assert_eq!(
            (f.state(), f.error.as_deref()),
            (GatewayState::CarrierPending, Some("stopped"))
        );
    }

    #[test]
    fn ages_are_monotonic_and_taken_when_read() {
        let g = Gateway::new();
        g.configure(on(), None);
        let t0 = Instant::now();
        g.observe(true, &[nid(3)], Some(Ok(vec![view(nid(3), true, 0)])), t0);
        assert_eq!(g.facts(t0).checked_ago_s, Some(0));
        assert_eq!(
            g.facts(t0 + Duration::from_secs(90)).checked_ago_s,
            Some(90)
        );
        // a reading taken before the check never goes negative
        assert_eq!(
            g.facts(t0 - Duration::from_millis(1)).checked_ago_s,
            Some(0)
        );
        // a re-read with no status call keeps the last check and its age
        g.observe(true, &[nid(3)], None, t0 + Duration::from_secs(5));
        let f = g.facts(t0 + Duration::from_secs(100));
        assert_eq!((f.checked_ago_s, f.answered), (Some(100), Some(true)));
    }

    #[test]
    fn the_configured_carrier_is_the_claim_and_off_is_by_choice() {
        let g = Gateway::new();
        let mut cfg = GatewayConfig {
            enabled: true,
            carrier: Some(ntfy()),
        };
        g.configure(cfg.clone(), None);
        let t0 = Instant::now();
        g.observe(true, &[nid(4)], Some(Ok(vec![view(nid(4), true, 0)])), t0);
        let f = g.facts(t0);
        assert_eq!(f.carrier, Some(CARRIER_NTFY));
        assert_eq!(f.endpoint.as_deref(), Some("ntfy.example.net"));
        assert_eq!(f.mode, Some("5A"));
        assert_eq!(
            f.state(),
            GatewayState::CarrierPending,
            "the carrier has not been checked"
        );
        g.record_check(Err("connection refused".into()), t0);
        let f = g.facts(t0 + Duration::from_secs(7));
        assert_eq!(f.checked_ago_s, Some(7));
        assert_eq!(f.error.as_deref(), Some("connection refused"));
        assert_eq!(f.state(), GatewayState::CarrierPending);
        g.record_check(Ok(()), t0);
        g.record_flush(t0);
        let f = g.facts(t0 + Duration::from_secs(60));
        assert_eq!(
            (f.state(), f.error.clone()),
            (GatewayState::CarrierOk, None)
        );
        assert_eq!(f.last_flush_ago_s, Some(60));
        g.set_pending(2);
        assert_eq!(g.facts(t0).state(), GatewayState::CarrierPending);
        // off by choice: the carrier stays configured
        cfg.enabled = false;
        g.configure(cfg, None);
        assert_eq!(g.facts(t0).state(), GatewayState::Off);
        // nothing on: every fact dropped, the tap returns at once again
        g.configure(GatewayConfig::default(), None);
        g.on_event(&FabricEvent::Fabric);
        assert!(!g.needs_refresh());
        assert_eq!(g.facts(t0), GatewayFacts::default());
    }

    #[test]
    fn off_without_a_carrier_is_not_a_gateway() {
        // pairs alone make a gateway only while it is on (§3.1); with it off and no carrier the
        // row is disarmed and reads gateway_none
        let g = Gateway::new();
        g.configure(on(), None);
        let t0 = Instant::now();
        g.observe(true, &[nid(5)], Some(Ok(vec![view(nid(5), true, 0)])), t0);
        assert_eq!(g.facts(t0).state(), GatewayState::CarrierOk);
        g.configure(GatewayConfig::default(), None);
        assert_eq!(g.facts(t0).state(), GatewayState::None);
    }

    #[test]
    fn only_fabric_and_contact_events_mark_stale() {
        let g = Gateway::new();
        g.configure(on(), None);
        g.stale.store(false, Ordering::SeqCst);
        g.on_event(&FabricEvent::Forward {
            target: nid(6),
            pending: 1,
        });
        assert!(!g.needs_refresh());
        g.on_event(&FabricEvent::Fabric);
        assert!(g.needs_refresh());
    }

    #[test]
    fn internet_pairs_are_whole_tcp_and_not_lan() {
        let mut p = Persisted::default();
        let rec = |id: NodeId, via: PairVia, theirs: bool| PairedHub {
            node_id: id,
            role: PairRole::Peer,
            via,
            ours: true,
            theirs,
        };
        let (wan, lan, half, carrier, nohalf) = (nid(10), nid(11), nid(12), nid(13), nid(14));
        p.fabric.paired_hubs = vec![
            rec(wan, PairVia::Tcp, true),
            rec(lan, PairVia::Tcp, true),
            rec(half, PairVia::Tcp, false),
            rec(carrier, PairVia::Carrier, true),
            rec(nohalf, PairVia::Tcp, true),
        ];
        for id in [wan, lan, half, carrier] {
            let mut l = PairLink::new(id);
            l.lan = id == lan;
            p.pairs.push(l);
        }
        assert!(internet_pairs(&p).is_empty(), "hub role off");
        p.fabric.hub_role = true;
        assert_eq!(internet_pairs(&p), vec![wan]);
    }

    #[test]
    fn config_from_the_settings() {
        let mut r = RelaySettings::default();
        assert_eq!(
            GatewayConfig::from_settings(&r, false),
            GatewayConfig::default()
        );
        assert!(!GatewayConfig::default().armed());
        r.set_gateway_enabled(true);
        let c = GatewayConfig::from_settings(&r, false);
        assert!(c.enabled && c.carrier.is_none() && c.armed());
        r.set_carrier_kind(CarrierKind::Ntfy);
        r.set_ntfy_server(Some("https://ntfy.example.net:8443/base"))
            .unwrap();
        assert_eq!(
            GatewayConfig::from_settings(&r, false).carrier,
            None,
            "no consent: no carrier"
        );
        r.set_consent_hash(ConsentKind::CarrierEnable, Some(&"ab".repeat(32)))
            .unwrap();
        r.set_carrier_mode(CarrierMode::FiveB, false).unwrap();
        let c = GatewayConfig::from_settings(&r, false).carrier.unwrap();
        assert_eq!(c.kind, CARRIER_NTFY);
        assert_eq!(c.endpoint.as_deref(), Some("ntfy.example.net"));
        assert_eq!(c.mode, "5B");
        assert_eq!(
            GatewayConfig::from_settings(&r, true).carrier.unwrap().mode,
            "5A",
            "5B runs as 5A on Termux"
        );
    }

    #[test]
    fn hosts() {
        assert_eq!(url_host("https://ntfy.sh").as_deref(), Some("ntfy.sh"));
        assert_eq!(
            url_host("http://192.168.1.4:80/x").as_deref(),
            Some("192.168.1.4")
        );
        assert!(url_host("https://[fd00::1]:443/").is_some_and(|h| h.contains("fd00::1")));
        assert_eq!(url_host("not a url"), None);
    }
}
