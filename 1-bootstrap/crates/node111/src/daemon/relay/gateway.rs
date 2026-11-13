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
//! LAN and whose address is known and not private (§4.1 rule 1: the one
//! [`super::tls::is_private`] the TLS path uses; loopback is never a peer), whose fabric
//! `reachable` (a connection attributed to it, M4) is the check. A pair with no address claims
//! nothing (fail closed).
//!
//! **Weight (§1 rule 2, §5.11; the Pi floor).** Nothing exists until [`apply`] meets a switch that
//! is on (`gateway.enabled`, or a carrier configured with its consent): until then [`on_event`]
//! is one `OnceLock` read and [`facts`] returns the `gateway_none` default. Once armed, a fabric
//! event only marks the facts stale (one atomic store); the services registry's existing 2 s
//! tick ([`refresh`]) re-reads them on its next pass. [`refresh`] is synchronous and never
//! reaches the fabric actor (row D3b): the per-pair views (`reachable`, `pending`) are the copy
//! the fabric handle keeps of its last `Fabric::status` read (`FabricHandle::pair_views`, taken
//! when it publishes a `fabric` frame and at its housekeeping pass), one lock; the pair set
//! (the persisted settings and the daemon's halves) is re-read only when that copy's generation
//! moved (a changed view, a published pairing half, an applied settings change) or a fabric
//! event marked the facts stale. No task, timer, socket, file or status call of its own.
//!
//! **Never a member's view of its hub (§3.8, I5).** Only this node's own switch and its own
//! pairs are read; a member learns its hub's gateway state only as §7.5 says.
//!
//! **What the agent's bearer can read.** The row's `detail` is on `GET
//! /api/v1/status/services`, which the agent's bearer may read. So `detail.error` is a fixed
//! sentence: [`record_check`]'s caller (D11) passes a fixed reason naming the failure, never a
//! URL, a topic, a token or a server's own words, and [`scrub_reason`] replaces anything that
//! still looks like one.

use crate::daemon::fabric::settings::Persisted;
use crate::daemon::fabric::{FabricHandle, PairViews};
use crate::daemon::relay::settings::{CarrierKind, ConsentKind, RelaySettings};
use id111::encoding::NodeIdDisplay;
use id111::keyset::NodeId;
use mesh111::fabric::{FabricEvent, PairVia, PairView};
use serde_json::{json, Value};
use std::net::IpAddr;
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
/// The mode shown for a paired hub on a Termux hub: 5B is a held stream and never runs on a
/// Termux hub (§3.1, contract `GatewayDetail.mode`), so it reads as the intermittent 5A there.
pub const PAIRED_HUB_MODE_TERMUX: &str = "5A";
/// Longest `detail.error` shown ([`scrub_reason`]).
pub const ERROR_MAX: usize = 160;

/// The paired-hub mode as this node runs it ([`PAIRED_HUB_MODE`], 5A on Termux).
pub fn paired_hub_mode(termux: bool) -> &'static str {
    if termux {
        PAIRED_HUB_MODE_TERMUX
    } else {
        PAIRED_HUB_MODE
    }
}

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
    /// Why the last check failed (a fixed sentence, [`scrub_reason`]).
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
    /// The server as configured (never displayed): a changed server is another carrier, whose
    /// facts start from nothing (row D3b).
    pub server: String,
    /// The recorded `carrier_enable` consent hash (never displayed): a new consent is another
    /// carrier too.
    pub consent: String,
}

/// What the switches say (§6.3), from `state/relay.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GatewayConfig {
    /// `gateway.enabled`.
    pub enabled: bool,
    /// A configured carrier.
    pub carrier: Option<CarrierSwitch>,
    /// This node runs on Termux (the paired-hub mode reads 5A, [`paired_hub_mode`]).
    pub termux: bool,
}

impl GatewayConfig {
    /// The switches as `r` holds them (`termux`: this node runs on Termux).
    pub fn from_settings(r: &RelaySettings, termux: bool) -> GatewayConfig {
        let carrier = match r.carrier_kind() {
            CarrierKind::None => None,
            CarrierKind::Ntfy => r.ntfy_server().and_then(|server| {
                let consent = r.consent_hash(ConsentKind::CarrierEnable)?;
                Some(CarrierSwitch {
                    kind: CARRIER_NTFY,
                    endpoint: url_host(server),
                    mode: r.carrier_mode_for(termux).as_str(),
                    server: server.to_string(),
                    consent: consent.to_string(),
                })
            }),
        };
        GatewayConfig {
            enabled: r.gateway_enabled(),
            carrier,
            termux,
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

/// `detail.error` as shown (the agent's bearer can read it): control characters dropped,
/// whitespace collapsed, every word that looks like a URL or carries one of its parts (a scheme,
/// a path, a query, credentials, a fragment) or a long opaque string (a topic, a token) replaced
/// by "(address removed)", the whole capped at [`ERROR_MAX`] characters. The caller's rule comes
/// first: pass a fixed sentence ([`record_check`]); this is the backstop.
pub fn scrub_reason(reason: &str) -> String {
    const REMOVED: &str = "(address removed)";
    let looks_like_address = |w: &str| {
        let opaque = w.len() >= 24
            && w.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        w.contains("://")
            || w.contains('/')
            || w.contains('?')
            || w.contains('=')
            || w.contains('@')
            || w.contains('#')
            || w.contains('\\')
            || w.to_ascii_lowercase().starts_with("www.")
            || opaque
    };
    let mut out = String::new();
    for word in reason
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
    {
        let w = if looks_like_address(word) {
            REMOVED
        } else {
            word
        };
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    if out.chars().count() > ERROR_MAX {
        out = out.chars().take(ERROR_MAX - 1).collect();
        out.push('…');
    }
    out
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

/// An address a paired hub claims the internet at (§4.1 rule 1): not private (RFC 1918, RFC
/// 6598, ULA, link-local: [`super::tls::is_private`], the TLS path's own test), not loopback
/// (never a peer), not unspecified and not multicast. An IPv4-mapped IPv6 address is judged by
/// its IPv4 address.
pub fn internet_address(ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    !super::tls::is_private(ip) && !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast()
}

/// The pairs a paired-hub claim rests on (§3.2, §4.1): whole `tcp` pairs whose daemon half is
/// present, not marked LAN and holds an [`internet_address`], with the hub role on (with it off
/// no pairing path runs, M4b). A `carrier` pair is the carrier's (D11); a pair without its
/// daemon half or without an address is skipped (fail closed).
pub fn internet_pairs(p: &Persisted) -> Vec<NodeId> {
    if !p.fabric.hub_role {
        return Vec::new();
    }
    p.fabric
        .paired_hubs
        .iter()
        .filter(|h| h.via == PairVia::Tcp && h.whole())
        .filter(|h| {
            p.pair_link(&h.node_id)
                .is_some_and(|l| !l.lan && l.address.is_some_and(|a| internet_address(a.ip())))
        })
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
    /// The internet pairs as last derived ([`internet_pairs`]).
    ids: Vec<NodeId>,
    /// The [`PairViews::generation`] `ids` was derived at; `None` forces a re-read.
    seen: Option<u64>,
    pairs: Vec<PairFact>,
    pairs_checked: Option<Instant>,
    pairs_error: Option<String>,
    carrier: CarrierFacts,
}

impl Inner {
    fn reset_pairs(&mut self) {
        self.ids.clear();
        self.seen = None;
        self.pairs.clear();
        self.pairs_checked = None;
        self.pairs_error = None;
    }
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
    /// every fact held (a later arm starts from nothing). While armed, the carrier's facts
    /// (its check, answer, error, last flush, pending) are dropped whenever the configured
    /// carrier differs (another kind, server, endpoint, mode or consent: they were about the
    /// old one), and the pair facts whenever `gateway.enabled` flips (row D3b).
    pub fn configure(&self, cfg: GatewayConfig, fabric: Option<&Arc<FabricHandle>>) {
        let armed = cfg.armed();
        {
            let mut i = lock(&self.inner);
            if !armed {
                *i = Inner::default();
            } else {
                if i.config.carrier != cfg.carrier {
                    i.carrier = CarrierFacts::default();
                }
                if i.config.enabled != cfg.enabled {
                    i.reset_pairs();
                }
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

    /// Re-read the pair facts (called from the services registry's existing tick; no timer of
    /// its own). Synchronous: it never waits on the fabric actor (row D3b). The views are the
    /// fabric handle's kept copy ([`FabricHandle::pair_views`], one lock); the pair set is
    /// re-derived from the handle's in-memory settings only when that copy's generation moved
    /// or a fabric event marked the facts stale. A no-op while disarmed.
    pub fn refresh(&self) {
        if !self.armed.load(Ordering::SeqCst) {
            return;
        }
        let handle = lock(&self.fabric).as_ref().and_then(Weak::upgrade);
        let Some(fh) = handle else { return };
        let views = fh.pair_views();
        let stale = self.stale.swap(false, Ordering::SeqCst);
        let moved = stale || lock(&self.inner).seen != Some(views.generation);
        let pair_set = moved.then(|| {
            let p = fh.persisted();
            (p.fabric.hub_role, internet_pairs(&p))
        });
        self.observe(pair_set, &views);
    }

    /// Record one read: `pair_set` is `(hub role, internet pairs)` when it was re-derived (else
    /// the last one stands), `views` the fabric's kept per-pair views. The check is the views'
    /// read (`views.at`); before the first one nothing is checked.
    pub fn observe(&self, pair_set: Option<(bool, Vec<NodeId>)>, views: &PairViews) {
        let mut i = lock(&self.inner);
        if let Some((hub_role, ids)) = pair_set {
            i.hub_role = hub_role;
            i.ids = ids;
            i.seen = Some(views.generation);
        }
        match views.at {
            Some(at) => {
                i.pairs = pair_facts(&i.ids, &views.views);
                i.pairs_checked = Some(at);
                i.pairs_error = (!i.pairs.iter().any(|p| p.reachable))
                    .then(|| "No paired hub is connected.".to_string());
            }
            None => {
                i.pairs = pair_facts(&i.ids, &[]);
                i.pairs_checked = None;
                i.pairs_error = None;
            }
        }
    }

    /// D11: one carrier check ended (§3.2: the drain is the check); `Err` carries why it failed.
    /// The reason reaches the row's `detail.error`, which the agent's bearer can read: pass a
    /// fixed sentence naming the failure ("The carrier did not answer.", "The carrier refused
    /// the token."), never a URL, a topic, a token or the server's own words. Anything that
    /// still looks like one is replaced ([`scrub_reason`]).
    pub fn record_check(&self, outcome: Result<(), String>, now: Instant) {
        let mut i = lock(&self.inner);
        i.carrier.checked = Some(now);
        i.carrier.answered = Some(outcome.is_ok());
        i.carrier.error = outcome.err().map(|e| scrub_reason(&e));
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
            f.mode = Some(paired_hub_mode(i.config.termux));
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

/// Re-read the facts (the services registry's tick; a no-op while nothing is on). Synchronous:
/// no actor round trip, no await (row D3b).
pub fn refresh() {
    if let Some(g) = GATEWAY.get() {
        g.refresh();
    }
}

/// The row's facts now (`gateway_none` while nothing is on).
pub fn facts() -> GatewayFacts {
    GATEWAY
        .get()
        .map(|g| g.facts(Instant::now()))
        .unwrap_or_default()
}

/// D11: one carrier check ended (see [`Gateway::record_check`]: a fixed reason, never a URL).
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
    use crate::daemon::fabric::FabricConfig;
    use crate::daemon::relay::settings::CarrierMode;
    use mesh111::fabric::{PairRole, PairedHub};
    use std::net::SocketAddr;
    use std::path::PathBuf;
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

    /// The fabric's views as read at `at`.
    fn read(at: Instant, views: Vec<PairView>) -> PairViews {
        PairViews {
            generation: 1,
            at: Some(at),
            views,
        }
    }

    fn on() -> GatewayConfig {
        GatewayConfig {
            enabled: true,
            carrier: None,
            termux: false,
        }
    }

    fn ntfy() -> CarrierSwitch {
        CarrierSwitch {
            kind: CARRIER_NTFY,
            endpoint: Some("ntfy.example.net".into()),
            mode: "5A",
            server: "https://ntfy.example.net".into(),
            consent: "ab".repeat(32),
        }
    }

    fn rec(id: NodeId, via: PairVia, theirs: bool) -> PairedHub {
        PairedHub {
            node_id: id,
            role: PairRole::Peer,
            via,
            ours: true,
            theirs,
        }
    }

    fn link(id: NodeId, address: Option<&str>, lan: bool) -> PairLink {
        let mut l = PairLink::new(id);
        l.address = address.map(|a| a.parse::<SocketAddr>().unwrap());
        l.lan = lan;
        l
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
        g.observe(Some((false, vec![])), &read(t0, vec![]));
        assert_eq!(g.facts(t0).state(), GatewayState::None);
        // a hub with an internet pair, not reachable at the check
        g.observe(Some((true, vec![a, b])), &read(t0, vec![view(a, false, 0)]));
        let f = g.facts(t0);
        assert_eq!(f.state(), GatewayState::CarrierPending);
        assert_eq!(f.carrier, Some(CARRIER_PAIRED_HUB));
        assert_eq!(f.mode, Some(PAIRED_HUB_MODE));
        assert_eq!(f.answered, Some(false));
        assert!(f.error.is_some());
        // b's connection is attributed: the claim rests on it (the pair set stands)
        g.observe(None, &read(t0, vec![view(a, false, 0), view(b, true, 0)]));
        let f = g.facts(t0);
        assert_eq!(f.state(), GatewayState::CarrierOk);
        assert_eq!(f.endpoint, Some(NodeIdDisplay::new(b.as_bytes()).label()));
        assert_eq!(f.error, None);
        assert_eq!(f.last_flush_ago_s, None);
        // rows waiting for a pair: pending
        g.observe(None, &read(t0, vec![view(a, false, 2), view(b, true, 1)]));
        let f = g.facts(t0);
        assert_eq!((f.state(), f.pending), (GatewayState::CarrierPending, 3));
        // a broken pair is not reachable
        let mut v = view(b, true, 0);
        v.broken = true;
        g.observe(Some((true, vec![b])), &read(t0, vec![v]));
        assert_eq!(g.facts(t0).answered, Some(false));
        // the fabric not read yet: nothing checked, no error invented
        g.observe(Some((true, vec![b])), &PairViews::default());
        let f = g.facts(t0);
        assert_eq!(
            (f.state(), f.answered, f.checked_ago_s, f.error.as_deref()),
            (GatewayState::CarrierPending, None, None, None)
        );
    }

    /// G5: 5B is a held stream and never runs on a Termux hub; the paired-hub claim reads 5A
    /// there (fails without the fix: the mode was the 5B constant everywhere).
    #[test]
    fn the_paired_hub_mode_is_5a_on_termux() {
        let g = Gateway::new();
        let t0 = Instant::now();
        let mut cfg = on();
        cfg.termux = true;
        g.configure(cfg, None);
        g.observe(
            Some((true, vec![nid(9)])),
            &read(t0, vec![view(nid(9), true, 0)]),
        );
        assert_eq!(g.facts(t0).mode, Some("5A"));
        g.configure(on(), None);
        g.observe(
            Some((true, vec![nid(9)])),
            &read(t0, vec![view(nid(9), true, 0)]),
        );
        assert_eq!(g.facts(t0).mode, Some("5B"));
        assert_eq!(paired_hub_mode(true), PAIRED_HUB_MODE_TERMUX);
    }

    #[test]
    fn ages_are_monotonic_and_taken_when_read() {
        let g = Gateway::new();
        g.configure(on(), None);
        let t0 = Instant::now();
        g.observe(
            Some((true, vec![nid(3)])),
            &read(t0, vec![view(nid(3), true, 0)]),
        );
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
        // a re-read with no new status read keeps the last check and its age
        g.observe(None, &read(t0, vec![view(nid(3), true, 0)]));
        let f = g.facts(t0 + Duration::from_secs(100));
        assert_eq!((f.checked_ago_s, f.answered), (Some(100), Some(true)));
    }

    #[test]
    fn the_configured_carrier_is_the_claim_and_off_is_by_choice() {
        let g = Gateway::new();
        let mut cfg = GatewayConfig {
            enabled: true,
            carrier: Some(ntfy()),
            termux: false,
        };
        g.configure(cfg.clone(), None);
        let t0 = Instant::now();
        g.observe(
            Some((true, vec![nid(4)])),
            &read(t0, vec![view(nid(4), true, 0)]),
        );
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

    /// G2: the facts of an old carrier are never reported against a new one (fails without the
    /// fix: a changed server or consent kept the old check, answer, error, flush and pending).
    #[test]
    fn a_changed_carrier_starts_from_nothing() {
        let t0 = Instant::now();
        let base = GatewayConfig {
            enabled: true,
            carrier: Some(ntfy()),
            termux: false,
        };
        let primed = |g: &Gateway| {
            g.record_check(Err("The carrier did not answer.".into()), t0);
            g.record_flush(t0);
            g.set_pending(4);
        };
        type Change = (&'static str, fn(&mut CarrierSwitch));
        let changes: [Change; 4] = [
            ("server", |c| c.server = "https://other.example.org".into()),
            ("endpoint", |c| {
                c.endpoint = Some("other.example.org".into())
            }),
            ("mode", |c| c.mode = "5B"),
            ("consent", |c| c.consent = "cd".repeat(32)),
        ];
        for (what, change) in changes {
            let g = Gateway::new();
            g.configure(base.clone(), None);
            g.observe(Some((true, vec![])), &PairViews::default());
            primed(&g);
            // the same carrier again (a relay settings write that changed something else)
            g.configure(base.clone(), None);
            let f = g.facts(t0);
            assert_eq!(
                (f.checked_ago_s, f.last_flush_ago_s, f.pending),
                (Some(0), Some(0), 4),
                "{what}: an unchanged carrier keeps its facts"
            );
            let mut cfg = base.clone();
            change(cfg.carrier.as_mut().unwrap());
            g.configure(cfg, None);
            let f = g.facts(t0);
            assert_eq!(
                (f.answered, f.checked_ago_s, f.last_flush_ago_s, f.pending),
                (None, None, None, 0),
                "{what}: a changed carrier's facts start from nothing"
            );
            assert_eq!(f.error, None, "{what}");
        }
    }

    /// G2: the pair facts reset when `gateway.enabled` flips.
    #[test]
    fn the_pair_facts_reset_when_the_switch_flips() {
        let t0 = Instant::now();
        let g = Gateway::new();
        let mut cfg = GatewayConfig {
            enabled: true,
            carrier: Some(ntfy()),
            termux: false,
        };
        g.configure(cfg.clone(), None);
        g.observe(
            Some((true, vec![nid(7)])),
            &read(t0, vec![view(nid(7), true, 0)]),
        );
        cfg.carrier = None;
        g.configure(cfg.clone(), None);
        assert_eq!(g.facts(t0).state(), GatewayState::CarrierOk, "same switch");
        let mut off = cfg.clone();
        off.enabled = false;
        off.carrier = Some(ntfy());
        g.configure(off, None);
        g.configure(cfg, None);
        let f = g.facts(t0);
        assert_eq!(
            (f.state(), f.checked_ago_s),
            (GatewayState::None, None),
            "the pair set is re-read before anything is claimed"
        );
    }

    #[test]
    fn off_without_a_carrier_is_not_a_gateway() {
        // pairs alone make a gateway only while it is on (§3.1); with it off and no carrier the
        // row is disarmed and reads gateway_none
        let g = Gateway::new();
        g.configure(on(), None);
        let t0 = Instant::now();
        g.observe(
            Some((true, vec![nid(5)])),
            &read(t0, vec![view(nid(5), true, 0)]),
        );
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
        let (wan, lan, half, carrier, nohalf) = (nid(10), nid(11), nid(12), nid(13), nid(14));
        p.fabric.paired_hubs = vec![
            rec(wan, PairVia::Tcp, true),
            rec(lan, PairVia::Tcp, true),
            rec(half, PairVia::Tcp, false),
            rec(carrier, PairVia::Carrier, true),
            rec(nohalf, PairVia::Tcp, true),
        ];
        for id in [wan, lan, half, carrier] {
            p.pairs.push(link(id, Some("203.0.113.7:11183"), id == lan));
        }
        assert!(internet_pairs(&p).is_empty(), "hub role off");
        p.fabric.hub_role = true;
        assert_eq!(internet_pairs(&p), vec![wan]);
    }

    /// G1: §4.1 rule 1 — a pair at a private address is a LAN pair whatever its `lan` mark; an
    /// unknown address claims nothing (fails without the fix: every unmarked pair claimed).
    #[test]
    fn a_pair_claims_the_internet_only_at_a_known_public_address() {
        let cases: [(Option<&str>, bool); 12] = [
            (Some("192.168.1.20:11183"), false),
            (Some("10.1.2.3:11183"), false),
            (Some("172.16.0.9:11183"), false),
            (Some("100.64.0.1:11183"), false),
            (Some("100.127.255.254:11183"), false),
            (Some("169.254.3.4:11183"), false),
            (Some("[fd12:3456::1]:11183"), false),
            (Some("[fe80::1]:11183"), false),
            (Some("[::ffff:192.168.1.20]:11183"), false),
            (Some("127.0.0.1:11183"), false),
            (None, false),
            (Some("203.0.113.7:11183"), true),
        ];
        for (address, claims) in cases {
            let mut p = Persisted::default();
            p.fabric.hub_role = true;
            p.fabric.paired_hubs = vec![rec(nid(20), PairVia::Tcp, true)];
            p.pairs.push(link(nid(20), address, false));
            assert_eq!(!internet_pairs(&p).is_empty(), claims, "{address:?}");
        }
        for (ip, claims) in [
            ("100.63.255.255", true),
            ("100.128.0.0", true),
            ("2001:db8::1", true),
            ("::ffff:198.51.100.2", true),
            ("0.0.0.0", false),
            ("224.0.0.1", false),
            ("::1", false),
        ] {
            assert_eq!(internet_address(ip.parse().unwrap()), claims, "{ip}");
        }
    }

    #[test]
    fn a_failure_reason_never_carries_an_address() {
        assert_eq!(
            scrub_reason("The carrier did not answer."),
            "The carrier did not answer."
        );
        for leak in [
            "GET https://ntfy.example.net/abc failed",
            "error at ntfy.example.net/x1y2z3",
            "token=tk_abcdef",
            "user:pass@host",
            "see #frag",
            "topic Qm9vbGVhbkFuZFRoZVRvcGljU2VjcmV0",
        ] {
            let s = scrub_reason(leak);
            assert!(s.contains("(address removed)"), "{leak} -> {s}");
            for bit in [
                "://",
                "ntfy.example.net/",
                "tk_abcdef",
                "pass@",
                "#frag",
                "Qm9v",
            ] {
                assert!(!s.contains(bit), "{leak} -> {s}");
            }
        }
        let s = scrub_reason("line one\r\nline\u{7}two");
        assert_eq!(s, "line one line two");
        let long = scrub_reason(&"word ".repeat(100));
        assert_eq!(long.chars().count(), ERROR_MAX);
        // through record_check
        let g = Gateway::new();
        g.configure(
            GatewayConfig {
                enabled: true,
                carrier: Some(ntfy()),
                termux: false,
            },
            None,
        );
        g.observe(Some((true, vec![])), &PairViews::default());
        g.record_check(
            Err("refused by https://ntfy.example.net/t".into()),
            Instant::now(),
        );
        let e = g.facts(Instant::now()).error.unwrap();
        assert_eq!(e, "refused by (address removed)");
    }

    fn handle_with(pairs: Vec<PairLink>, hubs: Vec<PairedHub>) -> Arc<FabricHandle> {
        let none = PathBuf::from("/nonexistent/111-gateway-test");
        let cfg = FabricConfig {
            fabric_dir: none.join("fabric"),
            identity_dir: none.join("identity"),
            state_dir: none.join("state"),
            listen: "127.0.0.1:0".parse().unwrap(),
            tcp: Default::default(),
            hub_default: false,
            enroll_timeout: Duration::from_secs(1),
            housekeeping: Duration::from_secs(3600),
            shutdown_grace: Duration::from_secs(1),
            offer_host: None,
        };
        let mut p = Persisted::default();
        p.fabric.hub_role = true;
        p.fabric.paired_hubs = hubs;
        p.pairs = pairs;
        FabricHandle::detached(cfg, p)
    }

    /// G3 + G6: from a fabric handle with an internet pair, through `refresh()` to the row. The
    /// handle has no fabric behind it (any actor round trip would fail as "not running"):
    /// `refresh()` is synchronous and reads only the views the handle keeps. A changed pairing
    /// half (`lan`, the address) emits no fabric event; the views' generation carries it.
    #[test]
    fn refresh_reads_the_handles_kept_views_and_reaches_the_row() {
        let far = nid(30);
        let fh = handle_with(
            vec![link(far, Some("203.0.113.9:11183"), false)],
            vec![rec(far, PairVia::Tcp, true)],
        );
        let g = Gateway::new();
        g.configure(on(), Some(&fh));
        g.refresh();
        let f = g.facts(Instant::now());
        assert_eq!(
            (f.state(), f.answered, f.checked_ago_s),
            (GatewayState::CarrierPending, None, None),
            "no status read kept yet: nothing checked"
        );
        // the handle read the fabric (a `fabric` frame or its housekeeping pass)
        fh.observe_pair_views(&[view(far, true, 0)]);
        assert!(!g.needs_refresh(), "no fabric event reached the tap");
        g.refresh();
        let f = g.facts(Instant::now());
        assert_eq!(f.state(), GatewayState::CarrierOk);
        assert_eq!(f.state().code(), "carrier_ok");
        let d = f.detail();
        assert_eq!(d["carrier"], CARRIER_PAIRED_HUB);
        assert_eq!(d["endpoint"], NodeIdDisplay::new(far.as_bytes()).label());
        assert_eq!(d["mode"], PAIRED_HUB_MODE);
        assert_eq!(d["checked_ago_s"], 0);
        // and the services registry's `gateway` row built from these facts
        let sf = crate::daemon::services::Facts {
            gateway: f.clone(),
            ..Default::default()
        };
        let rows = crate::daemon::services::compute(&sf);
        let row = rows
            .iter()
            .find(|r| r.id == crate::daemon::services::ServiceId::Gateway)
            .unwrap();
        assert_eq!(
            (row.state, row.reason_code),
            (crate::daemon::services::ServiceState::Live, "carrier_ok")
        );
        assert_eq!(row.detail["carrier"], CARRIER_PAIRED_HUB);
        assert_eq!(row.detail["error"], Value::Null);
        // the owner marks the pair LAN: no fabric event, the generation moves
        let before = fh.pair_generation();
        fh.replace_pair_links(vec![link(far, Some("203.0.113.9:11183"), true)]);
        assert_ne!(fh.pair_generation(), before);
        g.refresh();
        assert_eq!(g.facts(Instant::now()).state(), GatewayState::None);
        // a new address at a private LAN: still no claim
        fh.replace_pair_links(vec![link(far, Some("192.168.4.4:11183"), false)]);
        g.refresh();
        assert_eq!(g.facts(Instant::now()).state(), GatewayState::None);
        // back at a public address
        fh.replace_pair_links(vec![link(far, Some("198.51.100.4:11183"), false)]);
        g.refresh();
        assert_eq!(g.facts(Instant::now()).state(), GatewayState::CarrierOk);
        // the far hub's connection drops: the next kept read says so
        fh.observe_pair_views(&[view(far, false, 2)]);
        g.refresh();
        let f = g.facts(Instant::now());
        assert_eq!((f.state(), f.pending), (GatewayState::CarrierPending, 2));
        // the handle is gone: the last facts stand, nothing panics
        drop(fh);
        g.refresh();
        assert_eq!(g.facts(Instant::now()).pending, 2);
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
        assert!(GatewayConfig::from_settings(&r, true).termux);
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
        assert_eq!(c.server, "https://ntfy.example.net:8443/base");
        assert_eq!(c.consent, "ab".repeat(32));
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
