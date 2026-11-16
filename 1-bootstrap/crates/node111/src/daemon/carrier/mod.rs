//! Layer 7 §5.9, §6.1, §6.2 (rows P1, D11): the carrier on the forwarder.
//!
//! **The contract (§6.1).** A carrier moves opaque sealed hub-to-hub frames (the same
//! `CUSTODY_PUT` and `CUSTODY_ACK` packets a pair exchanges on `tcp-lan`) between two paired hubs,
//! batched in custody order, never one message per fragment; it declares `max_frame`; it is not a
//! `Link` in v1; its accept is a "posted" mark and only the far hub's `CUSTODY_ACK` commits;
//! before every flush it consults the consent record, the metered budget and the licensed-board
//! rule. In v1 it runs in-process behind the hand-off API (§5.9; §19 D12): [`HandOff`] over the
//! fabric's `forward_take`, `forward_done`, `carrier_inbound` and `forward_repost` (mesh111,
//! row M4), whose [`CarrierFrame`] and [`ForwardDone`] this module re-exports. Row P1 implements
//! [`Carrier`] for ntfy in `ntfy.rs`.
//!
//! **The runtime (row D11).** [`CarrierRuntime`] is the daemon's [`relay::CarrierHook`] (it
//! replaces the no-op hook in `ApiState` ext at `relay::start`, [`install`]). It holds nothing
//! until [`relay::CarrierHook::reconfigure`] meets `gateway.enabled`, `carrier.kind = "ntfy"` with
//! a server, the `carrier_enable` consent owed now (`consent::holds`; a metered cap set or raised
//! owes a new one) and the hub role (§5.11); otherwise nothing is constructed: no socket, task,
//! thread, file write or netlink subscription. `reconfigure` runs after every settings write, also
//! a no-op one: it compares what the switches ask for with what is built ([`BuildKey`]) and
//! builds, rebuilds or tears down only when they differ (cheap and idempotent). While built:
//!
//! - **The carrier bound** (§5.2, §6.1 item 3): `Fabric::set_carrier_max_frame(Some(3,068))`
//!   through `FabricHandle::fabric()` when built, `None` when torn down (no new handle method);
//!   the writes are serialised and each writes the state current when it runs.
//! - **One driver task** that waits on the debounced netlink subscription (row D4,
//!   `netwatch::Consumer::Carrier`; on a Termux hub whose bind failed, its coarse poll), the
//!   fabric's events, and in 5B the held stream; no timer of its own.
//! - **5A (§3.1, §6.2 "5A flush")**: a flush on a debounced address or route change (the §3.2
//!   check is the flush's first drain: one request when nothing is owed), on
//!   `FabricEvent::Forward` for a `carrier` pair, on "Flush now", on the Termux poll. A flush is:
//!   drain (acks first, so acked rows are not posted again), post what `forward_take` returns (in
//!   custody order, batched; `forward_done` for each put), drain once (skipped when nothing was
//!   posted: nothing waits for an answer), stop. Between flushes there is no connection, request
//!   task or timer (the 5A agent keeps no idle connection, P1).
//! - **5B (§3.1, §6.2 "Held")**: the held stream on the one thread `111d-ntfy-5b` for the first
//!   whole `carrier` pair (one held stream per carrier, P1), plus one post per `Forward` event for
//!   it; the stream's frames go to `carrier_inbound` and `forward_take` runs only at a drain
//!   boundary (the frames ready at once, mesh111's note). Refused on Termux (the settings run it
//!   as 5A there). Further `carrier` pairs run as 5A. Every connection the server accepts is a
//!   check (§3.2 (b): `record_check(Ok)`). The pair is assigned under the flush lock and a new
//!   stream opens only once the thread of a dropped one has ended. A delivery the fabric did not
//!   take whole leaves the cursor at the last commit, and the stream is held again from there
//!   (C13). No hold while a refusal holds; past the metered cap the thread stops by itself (its
//!   byte limit is the budget left) and the hold is looked at again on the next event or
//!   settings write.
//! - **Re-posts (§5.9, C6)**: `Taken` is the fabric's in-memory posted mark; the row commits only
//!   on the far `CUSTODY_ACK` through `carrier_inbound`. Rule (1) is the fabric's own (it re-arms in
//!   `forward_take` after a verified far frame); this runtime asks `forward_repost` on (2) "Flush
//!   now" and (3) a `since` miss; (4) after a restart the posted marks are gone with the process
//!   and the bound set at build raises the `Forward` events that post them once again. An ack
//!   `forward_take` returned is consumed on take and never posted again (C7).
//! - **Before every flush (§6.1 rule 6, §12)**, before each pair of a round, and before a 5B
//!   hold: the consent owed now, the metered budget ([`crate::daemon::relay::metered`]: carrier
//!   bytes both ways in a rolling 24 h `CLOCK_BOOTTIME` window, persisted, plus the bytes moved
//!   and not yet saved; past the cap no flush) and the licensed-board rule (no override; v1 has
//!   no sealed LoRa path, so the daemon's rule is structural and refuses nothing, a seam in
//!   [`RuntimeOptions::licensed`]).
//! - **What it saves (§6.2, C13)**: the last id per inbound topic in `state/relay.json`, only after
//!   the fabric took every frame read (the cursors of topics no current pair reads removed), and
//!   the metered window, also after a refused round and at a teardown; one
//!   `relay::settings::update` per round that moved either, under the settings cell's write lock,
//!   then the routes' settings cell follows (`RelayCell::replace`). A round stops between its
//!   steps once the carrier is torn down.
//! - **The `gateway` row (§3.5)**: `record_check` after every check (a fixed sentence on a
//!   failure, never a URL, topic, token or server's words), `record_flush` after a flush that
//!   moved frames (a failed drain or a bare check is none),
//!   `set_pending` from the fabric's per-pair `pending`; then `relay::changed()` (the `relay`
//!   frame).
//!
//! A frame from anyone but the pair fails `pair_mac` in the fabric, is dropped and counted there
//! (`counters.dropped`); a frame whose size no sealed packet has is dropped here and counted
//! ([`FabricHandOff::dropped`]); either way the drain's cursor moves past it.

pub mod ntfy;

pub use mesh111::fabric::{CarrierFrame, ForwardDone};

use crate::daemon::dashboard::api_v1::relay7;
use crate::daemon::dashboard::ApiState;
use crate::daemon::fabric::FabricHandle;
use crate::daemon::relay::netwatch::{self, NetEvent, NetWatch, Subscription};
use crate::daemon::relay::settings::{
    self as rs, CarrierKind, CarrierMode, ConsentKind, MeteredMatch, RelaySettings, SettingsError,
};
use crate::daemon::relay::{self, consent, gateway, metered};
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use id111::keyset::NodeId;
use mesh111::envelope::{MAX_PACKET_LEN, MIN_PACKET_LEN};
use mesh111::fabric::{FabricError, FabricEvent, FabricSettings, PairVia};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;
use tokio::sync::broadcast;
use zeroize::Zeroizing;

use self::ntfy::{HoldEvent, NtfyCarrier, NtfyConfig, NtfyStats};

/// Most frames one `forward_take` hands a flush (§5.9; the batches are packed from them).
pub const TAKE_MAX: usize = 256;

/// The fabric side of the hand-off API as the carrier sees it (§5.9, amendment 11):
/// [`FabricHandOff`] over the running fabric; P1's tests implement it with a fake.
pub trait HandOff: Send + Sync {
    /// Sealed frames owed to `target`, a `carrier` pair: forward puts and the `CUSTODY_ACK`s owed
    /// to it. An ack is consumed by this call and never returned again (C7).
    fn forward_take(
        &self,
        target: NodeId,
        max: usize,
    ) -> BoxFuture<'_, Result<Vec<CarrierFrame>, CarrierError>>;
    /// What the carrier did with a put it took.
    fn forward_done(
        &self,
        custody_seq: u64,
        done: ForwardDone,
    ) -> BoxFuture<'_, Result<(), CarrierError>>;
    /// A sealed frame from `from` (a `carrier` pair) that arrived through the carrier; verified
    /// like any sealed packet, processed with no connection the way a mailbox item is (INTRO
    /// allowed).
    fn carrier_inbound(
        &self,
        from: NodeId,
        frame: Vec<u8>,
    ) -> BoxFuture<'_, Result<(), CarrierError>>;
    /// Re-arm every posted, unacknowledged put to `target` (§5.9 re-post triggers (2) "Flush now"
    /// and (3) a `since` miss); how many were re-armed. A hand-off with no posted marks answers 0.
    fn forward_repost(&self, _target: NodeId) -> BoxFuture<'_, Result<u32, CarrierError>> {
        async { Ok(0) }.boxed()
    }
}

/// One `carrier` pair as a carrier sees it (§4.6, §6.2): this hub, the far hub, and the pairing's
/// carrier key.
pub struct Pair {
    /// This hub's NodeId (`NodeId_X` of `topic(X→Y)`, §6.2).
    pub own: NodeId,
    /// The far hub's NodeId.
    pub peer: NodeId,
    /// The ntfy `topic_key` (§6.2): BLAKE2b-256 keyed by the X25519 output behind the identity
    /// handle (M5), or keyed by `carrier.ntfy.topic_secret` when that is set. It names a mailbox;
    /// it protects no content. Never on any API.
    pub topic_key: Zeroizing<[u8; 32]>,
}

/// A carrier request that did not complete (§5.9, §6.2).
#[derive(Debug)]
pub enum CarrierError {
    /// Transient: no route, HTTP 429 or 5xx; the rows stay (`ForwardDone::Retry`).
    Retry(String),
    /// Permanent: HTTP 413, 401 or 403 (`ForwardDone::Rejected`).
    Rejected(&'static str),
    /// The saved `since` id is gone (the server lost its cache): read again with `since=all`, and
    /// re-post trigger (3) of §5.9.
    SinceMiss,
    /// The fabric side of the hand-off refused or failed.
    HandOff(String),
}

impl std::fmt::Display for CarrierError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CarrierError::Retry(why) => write!(f, "carrier: transient: {why}"),
            CarrierError::Rejected(why) => write!(f, "carrier: rejected: {why}"),
            CarrierError::SinceMiss => f.write_str("carrier: the saved since id is gone"),
            CarrierError::HandOff(why) => write!(f, "carrier: hand-off: {why}"),
        }
    }
}

impl std::error::Error for CarrierError {}

/// A carrier (§5.9, the node111 side): ntfy in v1 (`ntfy.rs`), chatmail after the G10 log (§6.5,
/// row D12).
pub trait Carrier: Send + Sync {
    /// `"ntfy"`.
    fn id(&self) -> &'static str;
    /// The largest sealed frame one carrier message holds (§6.1 rule 3; ntfy: 3,068 B, §6.2).
    fn max_frame(&self) -> usize;
    /// Post `out` in custody order, batched; one outcome per frame (`None`: an ack, logged only,
    /// C7).
    fn flush<'a>(
        &'a self,
        pair: &'a Pair,
        out: Vec<CarrierFrame>,
    ) -> BoxFuture<'a, Vec<(Option<u64>, ForwardDone)>>;
    /// 5A: one poll of the pair's inbound topic; the frames go to [`HandOff::carrier_inbound`].
    fn drain<'a>(&'a self, pair: &'a Pair) -> BoxFuture<'a, Result<Vec<Vec<u8>>, CarrierError>>;
    /// 5B: the held stream of the pair's inbound topic.
    fn hold<'a>(&'a self, pair: &'a Pair) -> BoxStream<'a, Result<Vec<u8>, CarrierError>>;
}

// ---------------------------------------------------------------- the hand-off over the fabric

/// [`HandOff`] over the daemon's running fabric (`FabricHandle::fabric()`, §5.9). Holds the
/// handle weakly: a fabric that stopped answers [`CarrierError::HandOff`].
pub struct FabricHandOff {
    fabric: Weak<FabricHandle>,
    dropped: AtomicU64,
    /// A test hook: the next this many frames are answered `Err` ([`CarrierRuntime::fail_next_inbound`]).
    fail_next: AtomicU64,
}

impl FabricHandOff {
    /// The hand-off of `fabric`.
    pub fn new(fabric: &Arc<FabricHandle>) -> FabricHandOff {
        FabricHandOff {
            fabric: Arc::downgrade(fabric),
            dropped: AtomicU64::new(0),
            fail_next: AtomicU64::new(0),
        }
    }

    /// Frames dropped here: a size no sealed packet has (foreign bytes; the fabric counts the
    /// frames that fail `pair_mac`).
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn handle(&self) -> Result<Arc<FabricHandle>, CarrierError> {
        self.fabric
            .upgrade()
            .filter(|h| h.running())
            .ok_or_else(|| CarrierError::HandOff("the fabric is not running".into()))
    }
}

fn handoff_error(e: FabricError) -> CarrierError {
    CarrierError::HandOff(e.to_string())
}

/// Run `$call` on the running fabric of `$self`.
macro_rules! on_fabric {
    ($self:ident, |$f:ident| $call:expr) => {{
        let h = $self.handle()?;
        let $f = h
            .fabric()
            .ok_or_else(|| CarrierError::HandOff("the fabric is not running".into()))?;
        $call.await.map_err(handoff_error)
    }};
}

impl HandOff for FabricHandOff {
    fn forward_take(
        &self,
        target: NodeId,
        max: usize,
    ) -> BoxFuture<'_, Result<Vec<CarrierFrame>, CarrierError>> {
        async move { on_fabric!(self, |f| f.forward_take(target, max)) }.boxed()
    }

    fn forward_done(
        &self,
        custody_seq: u64,
        done: ForwardDone,
    ) -> BoxFuture<'_, Result<(), CarrierError>> {
        async move { on_fabric!(self, |f| f.forward_done(custody_seq, done)) }.boxed()
    }

    /// A frame of a size no sealed packet has is dropped and counted here (the fabric would refuse
    /// it as invalid, and the drain's cursor would never move past it); every other frame goes to
    /// the fabric, which drops and counts what fails `pair_mac` or is not from `from`.
    fn carrier_inbound(
        &self,
        from: NodeId,
        frame: Vec<u8>,
    ) -> BoxFuture<'_, Result<(), CarrierError>> {
        async move {
            if !(MIN_PACKET_LEN..=MAX_PACKET_LEN).contains(&frame.len()) {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            if self.fail_next.load(Ordering::Relaxed) > 0
                && self
                    .fail_next
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_ok()
            {
                return Err(CarrierError::HandOff("a test fault".into()));
            }
            on_fabric!(self, |f| f.carrier_inbound(from, frame))
        }
        .boxed()
    }

    fn forward_repost(&self, target: NodeId) -> BoxFuture<'_, Result<u32, CarrierError>> {
        async move { on_fabric!(self, |f| f.forward_repost(target)) }.boxed()
    }
}

// ---------------------------------------------------------------- the runtime (row D11)

/// The runtime's environment (production: [`RuntimeOptions::production`]; tests choose).
#[derive(Clone)]
pub struct RuntimeOptions {
    /// The netlink address and route subscription (row D4); `None`: no netlink trigger.
    pub netwatch: Option<&'static NetWatch>,
    /// This node runs on Termux (5B runs as 5A, §3.1).
    pub termux: bool,
    /// The server's keepalive interval (5B read timeout = 3 ×, §6.2).
    pub keepalive: Duration,
    /// Connect timeout (5A and 5B).
    pub connect_timeout: Duration,
    /// 5A global timeout per request.
    pub request_timeout: Duration,
    /// The licensed-board rule (§12): `true` refuses every flush (no override). The daemon's is
    /// structural in v1 (no Layer 7 path rides a board: no sealed LoRa path), so it never refuses.
    pub licensed: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Which metered rules match the carrier's path now (§12;
    /// [`metered::active_by_default_route`] in the daemon).
    pub metered_active: Arc<dyn Fn(&MeteredMatch) -> bool + Send + Sync>,
}

impl RuntimeOptions {
    /// The daemon's: the process's netlink watch, this host's Termux detection, the P1 timeouts,
    /// no licensed path, metered rules matched by the default route.
    pub fn production() -> RuntimeOptions {
        RuntimeOptions {
            netwatch: Some(netwatch::global()),
            termux: crate::config::on_termux(),
            keepalive: ntfy::DEFAULT_KEEPALIVE,
            connect_timeout: ntfy::DEFAULT_CONNECT_TIMEOUT,
            request_timeout: ntfy::DEFAULT_REQUEST_TIMEOUT,
            licensed: Arc::new(|| false),
            metered_active: Arc::new(metered::active_by_default_route),
        }
    }
}

/// What a built carrier was built from (§6.3): a change of any of these rebuilds it; the same
/// key keeps it as it is.
#[derive(Clone, PartialEq, Eq)]
pub struct BuildKey {
    server: String,
    token: Option<rs::Secret>,
    topic_secret: Option<Zeroizing<[u8; rs::TOPIC_SECRET_LEN]>>,
    five_b: bool,
    termux_poll_min: u32,
}

impl std::fmt::Debug for BuildKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildKey")
            .field("server", &self.server)
            .field("token", &self.token.is_some())
            .field("topic_secret", &self.topic_secret.is_some())
            .field("five_b", &self.five_b)
            .finish()
    }
}

/// What the runtime did since it was made (tests, diagnostics; no secret, no address).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeStats {
    /// A carrier is built now.
    pub built: bool,
    /// Carriers built.
    pub builds: u64,
    /// Carriers torn down.
    pub teardowns: u64,
    /// Rounds run (each one or more flushes, posts or a check).
    pub rounds: u64,
    /// 5A flushes run (a drain, a post, a drain).
    pub flushes: u64,
    /// Rounds refused before any request (consent, metered cap, licensed board).
    pub refused: u64,
    /// Re-posts asked (`forward_repost` on Flush now and on a `since` miss).
    pub reposts_asked: u64,
}

#[derive(Default)]
struct Counters {
    builds: AtomicU64,
    teardowns: AtomicU64,
    rounds: AtomicU64,
    flushes: AtomicU64,
    refused: AtomicU64,
    reposts_asked: AtomicU64,
}

fn bump(c: &AtomicU64) {
    c.fetch_add(1, Ordering::Relaxed);
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The daemon's carrier hook (module doc).
pub struct CarrierRuntime {
    me: Weak<CarrierRuntime>,
    api: Weak<ApiState>,
    fabric: Weak<FabricHandle>,
    opts: RuntimeOptions,
    state: Mutex<Option<Arc<Built>>>,
    /// Serialises the carrier bound's writes (each writes the state current when it runs).
    bound: Arc<tokio::sync::Mutex<()>>,
    counters: Counters,
}

/// Register the daemon's carrier runtime as the `ApiState` carrier hook (`relay::start`, before
/// its first `reconfigure`), unless a hook is registered already. Builds nothing.
pub fn install(api: &Arc<ApiState>, fabric: &Arc<FabricHandle>) {
    if api.ext::<Arc<dyn relay::CarrierHook>>().is_some() {
        return;
    }
    let rt = CarrierRuntime::new(api, fabric, RuntimeOptions::production());
    api.insert_ext(rt.clone());
    api.insert_ext(rt as Arc<dyn relay::CarrierHook>);
}

/// Why a round runs no request (§6.1 rule 6, §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The `carrier_enable` consent owed now is not recorded.
    Consent,
    /// A metered link's daily cap is reached.
    Metered,
    /// The path would ride a board marked licensed.
    Licensed,
}

impl CarrierRuntime {
    /// A runtime for `api` and `fabric` (nothing built until [`relay::CarrierHook::reconfigure`]).
    pub fn new(
        api: &Arc<ApiState>,
        fabric: &Arc<FabricHandle>,
        opts: RuntimeOptions,
    ) -> Arc<CarrierRuntime> {
        Arc::new_cyclic(|me| CarrierRuntime {
            me: me.clone(),
            api: Arc::downgrade(api),
            fabric: Arc::downgrade(fabric),
            opts,
            state: Mutex::new(None),
            bound: Arc::new(tokio::sync::Mutex::new(())),
            counters: Counters::default(),
        })
    }

    /// The counters.
    pub fn stats(&self) -> RuntimeStats {
        let c = &self.counters;
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        RuntimeStats {
            built: lock(&self.state).is_some(),
            builds: g(&c.builds),
            teardowns: g(&c.teardowns),
            rounds: g(&c.rounds),
            flushes: g(&c.flushes),
            refused: g(&c.refused),
            reposts_asked: g(&c.reposts_asked),
        }
    }

    /// The built carrier's counters (`None` while nothing is built).
    pub fn carrier_stats(&self) -> Option<NtfyStats> {
        lock(&self.state).as_ref().map(|b| b.carrier.stats())
    }

    /// The 5B thread of the built carrier is alive.
    pub fn holding(&self) -> bool {
        lock(&self.state)
            .as_ref()
            .is_some_and(|b| b.carrier.holding())
    }

    /// The pair the 5B stream holds now.
    pub fn held(&self) -> Option<NodeId> {
        lock(&self.state).as_ref().and_then(|b| *lock(&b.held))
    }

    /// Frames dropped by the hand-off for their size ([`FabricHandOff::dropped`]).
    pub fn dropped(&self) -> u64 {
        lock(&self.state)
            .as_ref()
            .map_or(0, |b| b.handoff.dropped())
    }

    /// A test hook, not an API: the built carrier's hand-off answers the next `n` frames with an
    /// error (as the fabric does for a pair that stopped being whole), so the C13 path of a frame
    /// the fabric did not take can be driven. Nothing while nothing is built.
    #[doc(hidden)]
    pub fn fail_next_inbound(&self, n: u64) {
        if let Some(b) = lock(&self.state).as_ref() {
            b.handoff.fail_next.store(n, Ordering::SeqCst);
        }
    }

    /// No round is queued or running (tests wait on it).
    pub fn idle(&self) -> bool {
        lock(&self.state)
            .as_ref()
            .is_none_or(|b| !b.running.load(Ordering::SeqCst))
    }

    fn settings(&self) -> Option<RelaySettings> {
        self.api.upgrade().map(|a| relay7::cell(&a).settings())
    }

    /// What the switches ask to be built (§6.3, §5.11): `None` unless the gateway is on, the
    /// carrier is ntfy with a server, its consent owed now is recorded and the hub role is on.
    fn wanted(&self, s: &RelaySettings) -> Option<BuildKey> {
        if !s.gateway_enabled() || s.carrier_kind() != CarrierKind::Ntfy {
            return None;
        }
        let server = s.ntfy_server()?;
        if !consent::holds(s, ConsentKind::CarrierEnable) {
            return None;
        }
        let fh = self.fabric.upgrade()?;
        if !fh.running() || !fh.persisted().fabric.hub_role {
            return None;
        }
        Some(BuildKey {
            server: server.to_string(),
            token: s.ntfy_token().cloned(),
            topic_secret: s.ntfy_topic_secret(),
            five_b: s.carrier_mode_for(self.opts.termux) == CarrierMode::FiveB,
            termux_poll_min: s.termux_poll_min(),
        })
    }

    fn build(&self, key: BuildKey) -> Result<Arc<Built>, String> {
        let rt = tokio::runtime::Handle::try_current().map_err(|_| "no runtime".to_string())?;
        let fh = self
            .fabric
            .upgrade()
            .ok_or_else(|| "the fabric is gone".to_string())?;
        let f = fh
            .fabric()
            .ok_or_else(|| "the fabric is not running".to_string())?;
        let mut cfg = NtfyConfig::new(key.server.clone());
        cfg.token = key
            .token
            .as_ref()
            .map(|t| Zeroizing::new(t.expose().to_string()));
        cfg.keepalive = self.opts.keepalive;
        cfg.connect_timeout = self.opts.connect_timeout;
        cfg.request_timeout = self.opts.request_timeout;
        let carrier = NtfyCarrier::new(cfg).map_err(|e| e.to_string())?;
        // subscribed before the bound is set, so the `Forward` events it raises are seen
        let events = f.subscribe();
        let net = match self.opts.netwatch {
            Some(w) => match w.subscribe(netwatch::Consumer::Carrier {
                termux_poll_min: key.termux_poll_min,
            }) {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!("111 carrier: no address-change trigger: {e}");
                    None
                }
            },
            None => None,
        };
        let b = Arc::new(Built {
            own: f.node_id(),
            five_b: key.five_b,
            key,
            carrier,
            handoff: FabricHandOff::new(&fh),
            fabric: Arc::downgrade(&fh),
            api: self.api.clone(),
            rt: self.me.clone(),
            alive: AtomicBool::new(true),
            pairs: Mutex::new(HashMap::new()),
            held: Mutex::new(None),
            queue: Mutex::new(Queue::default()),
            running: AtomicBool::new(false),
            flush_lock: tokio::sync::Mutex::new(()),
            counted: AtomicU64::new(0),
            rehold: tokio::sync::Notify::new(),
            driver: Mutex::new(None),
        });
        let task = rt.spawn(drive(b.clone(), net, events));
        *lock(&b.driver) = Some(task);
        bump(&self.counters.builds);
        Ok(b)
    }

    /// Write the carrier bound the state now calls for (serialised; module doc).
    fn sync_bound(&self) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let me = self.me.clone();
        let one = self.bound.clone();
        rt.spawn(async move {
            let _one = one.lock().await;
            let Some(me) = me.upgrade() else { return };
            let want = lock(&me.state).as_ref().map(|b| b.carrier.max_frame());
            let Some(fh) = me.fabric.upgrade() else {
                return;
            };
            if let Some(f) = fh.fabric() {
                if let Err(e) = f.set_carrier_max_frame(want).await {
                    eprintln!("111 carrier: the carrier bound was not set: {e}");
                }
            }
        });
    }

    /// The refusal owed before a flush now (§6.1 rule 6), if any.
    pub fn refusal(&self) -> Option<Refusal> {
        let b = lock(&self.state).clone();
        self.refusal_of(b.as_deref())
    }

    /// [`CarrierRuntime::refusal`] with `b`'s bytes not yet in the saved window counted (§12):
    /// a cap reached by the flush before stops the next one, in the same round too.
    fn refusal_of(&self, b: Option<&Built>) -> Option<Refusal> {
        let s = self.settings()?;
        if !consent::holds(&s, ConsentKind::CarrierEnable) {
            return Some(Refusal::Consent);
        }
        if self.remaining(&s, b) == Some(0) {
            return Some(Refusal::Metered);
        }
        if (self.opts.licensed)() {
            return Some(Refusal::Licensed);
        }
        None
    }

    /// The carrier bytes left before a metered cap (§12; `None`: no active rule, no limit),
    /// counting `b`'s bytes not yet in the saved window. No clock or file read with no rule.
    fn remaining(&self, s: &RelaySettings, b: Option<&Built>) -> Option<u64> {
        if s.metered().is_empty() {
            return None;
        }
        let unsaved = b.map_or(0, |b| {
            b.carrier
                .stats()
                .wire_bytes
                .saturating_sub(b.counted.load(Ordering::SeqCst))
        });
        metered::remaining(
            s.metered(),
            s.metered_window(),
            metered::Clock::now().as_ref(),
            &*self.opts.metered_active,
            unsaved,
        )
    }

    /// Hand the 5B thread the byte count it stops at (§12: the budget left; module doc).
    fn update_wire_limit(&self, b: &Built) {
        let left = self.settings().and_then(|s| self.remaining(&s, Some(b)));
        let limit = left.map_or(u64::MAX, |l| b.carrier.stats().wire_bytes.saturating_add(l));
        b.carrier.set_wire_limit(limit);
    }

    fn request(&self, r: Request) -> bool {
        let b = lock(&self.state).clone();
        b.is_some_and(|b| b.request(r))
    }

    /// One round of queued work (module doc), under the built carrier's flush lock.
    async fn round(&self, b: &Arc<Built>, q: Queue) {
        bump(&self.counters.rounds);
        let pairs = b.carrier_pairs();
        b.forget_unpaired(&pairs);
        let held = *lock(&b.held);
        let mut work: Vec<(NodeId, bool)> = Vec::new();
        for p in &pairs {
            // the 5B pair is only posted to (its stream drains it); a 5A pair asked only for a
            // post (never in v1's requests) is flushed whole
            if q.all || q.flush.contains(p) || q.post.contains(p) {
                work.push((*p, held == Some(*p)));
            }
        }
        let health = q.check && work.iter().all(|(_, post)| *post);
        if work.is_empty() && !health {
            return;
        }
        let held_pair = held.and_then(|h| lock(&b.pairs).get(&h).cloned());
        if let Some(r) = self.refusal_of(Some(b)) {
            bump(&self.counters.refused);
            eprintln!("111 carrier: no flush ({r:?})");
            // what the held stream moved is still saved and counted (§12, C13)
            let touched: Vec<Arc<Pair>> = held_pair.into_iter().collect();
            self.persist(b, &touched).await;
            relay::changed();
            return;
        }
        let mut touched: Vec<Arc<Pair>> = Vec::new();
        let mut moved = false;
        if health {
            let outcome = b.carrier.health().await;
            gateway::record_check(outcome.map_err(|e| reason(&e).to_string()));
        }
        for (peer, post_only) in work {
            // torn down meanwhile (the gateway or the carrier switched off): nothing more is sent
            if !b.alive.load(Ordering::SeqCst) {
                break;
            }
            // before every flush (§6.1 rule 6): a cap reached by the flush before stops the next
            // (its bytes are counted though not yet saved, `refusal_of`)
            if self.refusal_of(Some(b)).is_some() {
                bump(&self.counters.refused);
                break;
            }
            let pair = match b.pair(peer).await {
                Ok(p) => p,
                Err(why) => {
                    gateway::record_check(Err(why.to_string()));
                    continue;
                }
            };
            if q.repost {
                self.repost(b, peer).await;
            }
            let m = if post_only {
                b.post(&pair).await.posted > 0
            } else {
                bump(&self.counters.flushes);
                self.flush_5a(b, &pair).await
            };
            moved |= m;
            touched.push(pair);
        }
        if let Some(p) = held_pair {
            if !touched.iter().any(|t| t.peer == p.peer) {
                touched.push(p);
            }
        }
        self.persist(b, &touched).await;
        // a flush is one that moved frames: a failed drain or a bare check is no flush (§3.5)
        if moved {
            gateway::record_flush();
        }
        if let Some(n) = b.pending().await {
            gateway::set_pending(n);
        }
        relay::changed();
    }

    async fn repost(&self, b: &Built, peer: NodeId) {
        bump(&self.counters.reposts_asked);
        if let Err(e) = b.handoff.forward_repost(peer).await {
            eprintln!("111 carrier: re-post not armed: {e}");
        }
    }

    /// The §6.2 5A flush of one pair: drain (the §3.2 check), post, drain once, stop. Stops
    /// between its steps once the carrier is torn down. Returns whether frames moved (read in or
    /// posted).
    async fn flush_5a(&self, b: &Built, pair: &Pair) -> bool {
        let mut moved = match b.carrier.drain_into(pair, &b.handoff).await {
            Ok(d) => {
                gateway::record_check(Ok(()));
                if d.since_miss {
                    self.repost(b, pair.peer).await;
                }
                d.frames > 0
            }
            Err(e) => {
                gateway::record_check(Err(reason(&e).to_string()));
                return false;
            }
        };
        if !b.alive.load(Ordering::SeqCst) {
            return moved;
        }
        let posted = b.post(pair).await;
        moved |= posted.posted > 0;
        if posted.taken > 0 && b.alive.load(Ordering::SeqCst) {
            match b.carrier.drain_into(pair, &b.handoff).await {
                Ok(d) => moved |= d.frames > 0,
                Err(e) => gateway::record_check(Err(reason(&e).to_string())),
            }
        }
        moved
    }

    /// Save what the round moved (§6.2, C13; §12): the drain cursors of `touched` the fabric took,
    /// the cursors of topics no current `carrier` pair reads removed (an unpair, a re-pair or a
    /// `topic_secret` change leaves one; the file holds at most `MAX_DRAIN_TOPICS`), and the
    /// carrier's bytes since the last count in the metered window (only with a metered rule set).
    /// One `relay.json` update, none when nothing moved, under the settings cell's write lock
    /// (the PATCH route's), so the cell never goes back to an older snapshot. A cursor the file
    /// refuses is skipped, never the rest; bytes whose write failed are counted next time.
    async fn persist(&self, b: &Built, touched: &[Arc<Pair>]) {
        let Some(api) = self.api.upgrade() else {
            return;
        };
        let live = if b.alive.load(Ordering::SeqCst) {
            b.live_topics().await
        } else {
            None
        };
        let cell = relay7::cell(&api);
        let write = cell.write_lock().await;
        let s = cell.settings();
        let mut cursors: Vec<(String, Option<String>)> = touched
            .iter()
            .filter_map(|p| {
                let topic = ntfy::inbound_topic(p);
                let now = b.carrier.cursor(p);
                (s.drain_cursor(&topic) != now.as_deref()).then_some((topic, now))
            })
            .collect();
        if let Some(live) = &live {
            for (topic, _) in s.drain_cursors() {
                if !live.contains(topic) && !cursors.iter().any(|(t, _)| t == topic) {
                    cursors.push((topic.to_string(), None));
                }
            }
            // forgotten first: room for the live ones
            cursors.sort_by_key(|(_, id)| id.is_some());
        }
        let wire = b.carrier.stats().wire_bytes;
        let delta = wire.saturating_sub(b.counted.load(Ordering::SeqCst));
        let count = delta > 0 && !s.metered().is_empty();
        if s.metered().is_empty() {
            // no rule: nothing is counted (§12), and nothing moved now counts later
            b.counted.store(wire, Ordering::SeqCst);
        }
        if !cursors.is_empty() || count {
            let state_dir = api.layout.state_dir();
            let active = self.opts.metered_active.clone();
            let written = tokio::task::spawn_blocking(move || {
                rs::update(&state_dir, |s| {
                    for (topic, id) in &cursors {
                        if let Err(e) = s.set_drain_cursor(topic, id.as_deref()) {
                            eprintln!("111 carrier: a drain position was not saved: {e}");
                        }
                    }
                    if count {
                        if let Some(clock) = metered::Clock::now() {
                            let mut w = s.metered_window().clone();
                            let links = s.metered().to_vec();
                            if metered::count(&links, &mut w, &clock, &*active, delta) {
                                s.set_metered_window(w)?;
                            }
                        }
                    }
                    Ok::<(), SettingsError>(())
                })
            })
            .await;
            match written {
                Ok(Ok((after, _))) => {
                    cell.replace(after);
                    if count {
                        b.counted.fetch_add(delta, Ordering::SeqCst);
                    }
                }
                Ok(Err(e)) => eprintln!("111 carrier: the drain position was not saved: {e}"),
                Err(e) => eprintln!("111 carrier: the drain position was not saved: {e}"),
            }
        }
        drop(write);
        self.update_wire_limit(b);
    }
}

impl relay::CarrierHook for CarrierRuntime {
    /// Build, rebuild or tear down what the switches say (module doc); cheap when nothing changed.
    fn reconfigure(&self, settings: &RelaySettings) {
        let want = self.wanted(settings);
        let mut st = lock(&self.state);
        if st.as_ref().map(|b| &b.key) == want.as_ref() {
            // the same carrier: a 5B hold looks again at the refusal (a cap raised and consented)
            if let Some(b) = st.as_ref().filter(|b| b.five_b) {
                b.rehold.notify_one();
            }
            return;
        }
        let old = st.take();
        let built = want.and_then(|k| match self.build(k) {
            Ok(b) => Some(b),
            Err(e) => {
                eprintln!("111 carrier: not built: {e}");
                None
            }
        });
        *st = built;
        drop(st);
        if let Some(o) = old {
            o.teardown();
            bump(&self.counters.teardowns);
            // the bytes it moved since the last count still count (§12)
            if let (Some(me), Ok(h)) = (self.me.upgrade(), tokio::runtime::Handle::try_current()) {
                h.spawn(async move { me.persist(&o, &[]).await });
            }
        }
        self.sync_bound();
        relay::changed();
    }

    /// "Check now" (§3.2 (c)): the drain of every `carrier` pair (one request each when nothing
    /// is owed; a check that returns frames is the first step of a flush), or, with none paired,
    /// the server's health route. `false` while nothing is built.
    fn check(&self) -> bool {
        self.request(Request::Check)
    }

    /// "Flush now" (§5.9 re-post trigger (2), §6.2): every `carrier` pair flushed, each posted and
    /// unacknowledged put posted once more. `false` while nothing is built.
    fn flush(&self) -> bool {
        self.request(Request::FlushNow)
    }

    fn metered_cap_reached(&self) -> bool {
        let b = lock(&self.state).clone();
        self.settings()
            .is_some_and(|s| self.remaining(&s, b.as_deref()) == Some(0))
    }

    fn licensed_board(&self) -> bool {
        (self.opts.licensed)()
    }
}

/// A fixed sentence for the `gateway` row's `detail.error` (the agent's bearer reads it): never a
/// URL, a topic, a token or the server's words.
pub fn reason(e: &CarrierError) -> &'static str {
    match e {
        CarrierError::Retry(_) => "The carrier did not answer.",
        CarrierError::Rejected(w) if w.contains("401") || w.contains("403") => {
            "The carrier refused the token."
        }
        CarrierError::Rejected(w) if w.contains("413") => {
            "The carrier refused a message as too large."
        }
        CarrierError::Rejected(_) => "The carrier refused the request.",
        CarrierError::SinceMiss => "The carrier lost the messages it held.",
        CarrierError::HandOff(_) => "The fabric did not take the carrier's frames.",
    }
}

/// What is asked of a built carrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Request {
    /// "Check now".
    Check,
    /// "Flush now".
    FlushNow,
    /// A debounced address or route change, or the Termux poll.
    Net,
    /// `FabricEvent::Forward` for a 5A pair, or the fabric's events lost (`pair = None`: all).
    Pair(Option<NodeId>),
    /// One post for the 5B pair (a `Forward` event, or a drain boundary of its stream).
    Post(NodeId),
}

/// Requests coalesced until the next round.
#[derive(Debug, Default)]
struct Queue {
    all: bool,
    check: bool,
    repost: bool,
    flush: HashSet<NodeId>,
    post: HashSet<NodeId>,
}

impl Queue {
    fn add(&mut self, r: Request) {
        match r {
            Request::Check | Request::Net => {
                self.all = true;
                self.check = true;
            }
            Request::FlushNow => {
                self.all = true;
                self.check = true;
                self.repost = true;
            }
            Request::Pair(None) => self.all = true,
            Request::Pair(Some(p)) => {
                self.flush.insert(p);
            }
            Request::Post(p) => {
                self.post.insert(p);
            }
        }
    }

    fn is_empty(&self) -> bool {
        !self.all && !self.check && self.flush.is_empty() && self.post.is_empty()
    }
}

/// One built carrier (module doc).
struct Built {
    key: BuildKey,
    own: NodeId,
    five_b: bool,
    carrier: NtfyCarrier,
    handoff: FabricHandOff,
    fabric: Weak<FabricHandle>,
    api: Weak<ApiState>,
    rt: Weak<CarrierRuntime>,
    alive: AtomicBool,
    /// The `carrier` pairs' topic keys, made at their first flush.
    pairs: Mutex<HashMap<NodeId, Arc<Pair>>>,
    /// The pair the 5B stream holds.
    held: Mutex<Option<NodeId>>,
    queue: Mutex<Queue>,
    running: AtomicBool,
    /// One round or 5B delivery at a time (a drain boundary is one unit).
    flush_lock: tokio::sync::Mutex<()>,
    /// `NtfyStats::wire_bytes` already counted in the metered window (moved on only by
    /// `persist`, under the settings cell's write lock).
    counted: AtomicU64,
    /// A settings write that changed nothing built: the driver looks at the 5B hold again.
    rehold: tokio::sync::Notify,
    driver: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// What one post did ([`Built::post`]).
#[derive(Debug, Clone, Copy, Default)]
struct Posted {
    /// Frames `forward_take` returned.
    taken: usize,
    /// Of those, accepted by the server.
    posted: usize,
}

impl Built {
    /// Queue `r` and start a round task unless one runs (it picks the request up).
    fn request(self: &Arc<Self>, r: Request) -> bool {
        if !self.alive.load(Ordering::SeqCst) {
            return false;
        }
        lock(&self.queue).add(r);
        if self.running.swap(true, Ordering::SeqCst) {
            return true;
        }
        let (Some(rt), Ok(h)) = (self.rt.upgrade(), tokio::runtime::Handle::try_current()) else {
            self.running.store(false, Ordering::SeqCst);
            return false;
        };
        h.spawn(run_queue(rt, self.clone()));
        true
    }

    fn teardown(&self) {
        self.alive.store(false, Ordering::SeqCst);
        if let Some(t) = lock(&self.driver).take() {
            t.abort();
        }
        *lock(&self.held) = None;
        lock(&self.pairs).clear();
        *lock(&self.queue) = Queue::default();
    }

    /// The whole `carrier` pairs, in `paired_hubs` order (none with the hub role off).
    fn carrier_pairs(&self) -> Vec<NodeId> {
        let Some(fh) = self.fabric.upgrade() else {
            return Vec::new();
        };
        carrier_pairs_of(&fh.persisted().fabric)
    }

    fn forget_unpaired(&self, pairs: &[NodeId]) {
        lock(&self.pairs).retain(|id, _| pairs.contains(id));
    }

    /// The inbound topics of every whole `carrier` pair now; `None` (prune nothing) while the hub
    /// role is off or a pair's keys are not known here.
    async fn live_topics(&self) -> Option<HashSet<String>> {
        let fh = self.fabric.upgrade()?;
        if !fh.persisted().fabric.hub_role {
            return None;
        }
        let mut out = HashSet::new();
        for peer in self.carrier_pairs() {
            let p = self.pair(peer).await.ok()?;
            out.insert(ntfy::inbound_topic(&p));
        }
        Some(out)
    }

    /// The pair's carrier view (§6.2 "Topic"), made once: the far hub's encryption key from its
    /// contact row, the X25519 output behind the identity handle (or `topic_secret`), the
    /// topic key; the saved drain position restored.
    async fn pair(&self, peer: NodeId) -> Result<Arc<Pair>, &'static str> {
        if let Some(p) = lock(&self.pairs).get(&peer) {
            return Ok(p.clone());
        }
        const NO_KEYS: &str = "The paired hub's keys are not known here.";
        let fh = self.fabric.upgrade().ok_or(NO_KEYS)?;
        let (Some(f), Some(sealer)) = (fh.fabric(), fh.sealer()) else {
            return Err(NO_KEYS);
        };
        let id = peer.to_bytes();
        let row = f
            .with_store(move |s| s.contact(&id))
            .await
            .map_err(|_| NO_KEYS)?
            .map_err(|_| NO_KEYS)?
            .ok_or(NO_KEYS)?;
        let peer_enc = row.enc_pubkey;
        let own_enc = sealer.enc_pubkey();
        let key = match &self.key.topic_secret {
            Some(k) => k.clone(),
            None => sealer
                .carrier_topic_x25519(&peer_enc)
                .map_err(|_| NO_KEYS)?,
        };
        let topic_key = ntfy::topic_key(&key, &own_enc, &peer_enc).ok_or(NO_KEYS)?;
        let pair = Arc::new(Pair {
            own: self.own,
            peer,
            topic_key,
        });
        let saved = self.api.upgrade().and_then(|a| {
            relay7::cell(&a)
                .settings()
                .drain_cursor(&ntfy::inbound_topic(&pair))
                .map(str::to_string)
        });
        if self.carrier.restore_cursor(&pair, saved).is_err() {
            let _ = self.carrier.restore_cursor(&pair, None);
        }
        Ok(lock(&self.pairs).entry(peer).or_insert(pair).clone())
    }

    /// Post what the fabric owes `pair` (`forward_take`, in custody order, batched), and report
    /// each put's outcome (`forward_done`). Nothing once torn down.
    async fn post(&self, pair: &Pair) -> Posted {
        if !self.alive.load(Ordering::SeqCst) {
            return Posted::default();
        }
        let out = match self.handoff.forward_take(pair.peer, TAKE_MAX).await {
            Ok(o) => o,
            Err(e) => {
                eprintln!("111 carrier: nothing taken: {e}");
                return Posted::default();
            }
        };
        let mut r = Posted {
            taken: out.len(),
            posted: 0,
        };
        if r.taken == 0 {
            return r;
        }
        for (seq, done) in self.carrier.flush(pair, out).await {
            if done == ForwardDone::Taken {
                r.posted += 1;
            }
            if let Some(s) = seq {
                if let Err(e) = self.handoff.forward_done(s, done).await {
                    eprintln!("111 carrier: a put's outcome was not recorded: {e}");
                }
            }
        }
        r
    }

    /// The frames waiting for the `carrier` pairs (the fabric's per-pair `pending`).
    async fn pending(&self) -> Option<u32> {
        let fh = self.fabric.upgrade()?;
        let st = fh.fabric()?.status().await.ok()?;
        Some(
            st.paired
                .iter()
                .filter(|v| v.hub.via == PairVia::Carrier)
                .fold(0u32, |a, v| a.saturating_add(v.pending)),
        )
    }
}

async fn run_queue(rt: Arc<CarrierRuntime>, b: Arc<Built>) {
    loop {
        let q = std::mem::take(&mut *lock(&b.queue));
        if q.is_empty() || !b.alive.load(Ordering::SeqCst) {
            b.running.store(false, Ordering::SeqCst);
            // a request that came after the take and before the store: run again (unless
            // another task took it)
            if lock(&b.queue).is_empty()
                || !b.alive.load(Ordering::SeqCst)
                || b.running.swap(true, Ordering::SeqCst)
            {
                return;
            }
            continue;
        }
        let _one = b.flush_lock.lock().await;
        rt.round(&b, q).await;
    }
}

// ---------------------------------------------------------------- the driver

/// The whole `carrier` pairs of `s`, in `paired_hubs` order (none with the hub role off).
fn carrier_pairs_of(s: &FabricSettings) -> Vec<NodeId> {
    if !s.hub_role {
        return Vec::new();
    }
    s.paired_hubs
        .iter()
        .filter(|h| h.via == PairVia::Carrier && h.whole())
        .map(|h| h.node_id)
        .collect()
}

/// What woke the driver.
enum Step {
    Net(NetEvent),
    Fabric(Result<FabricEvent, broadcast::error::RecvError>),
    /// New fabric settings (`FabricEvent::Settings`): the hold reads the pairs from them, since
    /// the daemon's persisted copy may not hold them yet when the event reaches this driver.
    Settings(Box<FabricSettings>),
    Held(Option<Result<HoldEvent, CarrierError>>),
    /// The 5B thread of a dropped stream ended: the pending hold opens.
    Released,
    /// A settings write that changed nothing built.
    Rehold,
}

async fn next_net(n: Option<&Subscription>) -> NetEvent {
    match n {
        Some(s) => s.next().await,
        None => std::future::pending().await,
    }
}

type Held = BoxStream<'static, Result<HoldEvent, CarrierError>>;

async fn next_held(h: &mut Option<Held>) -> Option<Result<HoldEvent, CarrierError>> {
    match h {
        Some(s) => s.next().await,
        None => std::future::pending().await,
    }
}

/// The driver's 5B state.
#[derive(Default)]
struct Hold {
    stream: Option<Held>,
    /// The pair is assigned (`Built::held`) but the thread of a dropped stream is still closing
    /// (one held stream per carrier): the stream opens once it has ended ([`Step::Released`]).
    pending: bool,
    /// The server refused the stream (401, 403, …): no hold until a rebuild.
    rejected: bool,
    /// Consecutive held deliveries the fabric did not take whole.
    failures: u32,
}

/// 5B: hold the first whole `carrier` pair's stream unless it is held (or about to be) already;
/// none while a refusal holds (§6.1 rule 6, §12: the stream is carrier traffic too). The pair is
/// assigned under the flush lock: a round either ran before or reads the new mark, so no 5A
/// drain reads the held topic beside the stream (C13).
async fn start_hold(b: &Built, h: &mut Hold, settings: Option<&FabricSettings>) {
    if !b.five_b || h.rejected {
        return;
    }
    let Some(rt) = b.rt.upgrade() else { return };
    let pairs = match settings {
        Some(s) => carrier_pairs_of(s),
        None => b.carrier_pairs(),
    };
    let first = pairs.first().copied();
    let want = first.filter(|_| rt.refusal_of(Some(b)).is_none());
    let active = (h.stream.is_some() || h.pending)
        .then(|| *lock(&b.held))
        .flatten();
    if want == active {
        if want.is_some() {
            rt.update_wire_limit(b);
        } else {
            *lock(&b.held) = None;
        }
        return;
    }
    let _one = b.flush_lock.lock().await;
    h.stream = None;
    h.pending = false;
    *lock(&b.held) = None;
    let Some(peer) = want else {
        relay::changed();
        return;
    };
    match b.pair(peer).await {
        Ok(p) => {
            *lock(&b.held) = Some(peer);
            rt.update_wire_limit(b);
            if b.carrier.holding() {
                h.pending = true;
            } else {
                h.stream = Some(b.carrier.hold_owned(&p));
            }
        }
        Err(why) => {
            gateway::record_check(Err(why.to_string()));
            relay::changed();
        }
    }
}

/// Stop the 5B hold (a refusal; the stream ended): the pair runs as 5A until it is held again.
fn stop_hold(b: &Built, h: &mut Hold) {
    h.stream = None;
    h.pending = false;
    *lock(&b.held) = None;
}

/// The driver of one built carrier (module doc): no timer; it waits on the netlink subscription,
/// the fabric's events and, in 5B, the held stream.
async fn drive(
    b: Arc<Built>,
    net: Option<Subscription>,
    mut events: broadcast::Receiver<FabricEvent>,
) {
    let mut h = Hold::default();
    start_hold(&b, &mut h, None).await;
    loop {
        let step = tokio::select! {
            ev = next_net(net.as_ref()) => Step::Net(ev),
            r = events.recv() => match r {
                Ok(FabricEvent::Settings(s)) => Step::Settings(Box::new(s)),
                r => Step::Fabric(r),
            },
            item = next_held(&mut h.stream) => Step::Held(item),
            _ = b.carrier.released(), if h.pending => Step::Released,
            _ = b.rehold.notified() => Step::Rehold,
        };
        match step {
            Step::Net(NetEvent::Changed | NetEvent::Poll) => {
                // the hold first: the round then knows which pair the stream reads
                start_hold(&b, &mut h, None).await;
                b.request(Request::Net);
            }
            // netwatch logs a failed bind once; on Termux its poll follows (§3.4)
            Step::Net(NetEvent::Failed) => {}
            Step::Fabric(Ok(FabricEvent::Forward { target, .. })) => {
                if h.stream.is_none() && !h.pending {
                    start_hold(&b, &mut h, None).await;
                }
                if *lock(&b.held) == Some(target) {
                    b.request(Request::Post(target));
                } else {
                    b.request(Request::Pair(Some(target)));
                }
            }
            Step::Settings(s) => start_hold(&b, &mut h, Some(&s)).await,
            Step::Rehold => start_hold(&b, &mut h, None).await,
            Step::Released => {
                h.pending = false;
                start_hold(&b, &mut h, None).await;
            }
            Step::Fabric(Ok(_)) => {}
            Step::Fabric(Err(broadcast::error::RecvError::Lagged(_))) => {
                b.request(Request::Pair(None));
            }
            Step::Fabric(Err(broadcast::error::RecvError::Closed)) => return,
            Step::Held(None) => {
                // ended: refused, or the metered budget spent; held again on the next event
                stop_hold(&b, &mut h);
            }
            Step::Held(Some(first)) => held_step(&b, &mut h, first).await,
        }
    }
}

/// One 5B delivery (a drain boundary, one unit under the flush lock): the frames go to
/// `carrier_inbound`; the cursor commits only when the fabric took them all (C13). When it did
/// not, the cursor stays at the last commit and the stream is dropped and held again from there
/// once its thread has ended, so the frames not taken are read again (the L2 window drops the
/// ones that were); after a second such delivery in a row, the hold waits for the next event.
/// Past a refusal (the metered cap) the stream is dropped.
async fn held_step(b: &Arc<Built>, h: &mut Hold, first: Result<HoldEvent, CarrierError>) {
    let Some(peer) = *lock(&b.held) else {
        return;
    };
    let Some(rt) = b.rt.upgrade() else { return };
    let one = b.flush_lock.lock().await;
    let mut items = vec![first];
    let mut ended = false;
    while let Some(s) = h.stream.as_mut() {
        match s.next().now_or_never() {
            Some(Some(x)) => items.push(x),
            Some(None) => {
                ended = true;
                break;
            }
            None => break,
        }
    }
    let mut got = 0usize;
    let mut failed = false;
    for item in items {
        match item {
            Ok(HoldEvent::Opened) => {
                // §3.2 (b): a held stream's (re)connect is a check
                gateway::record_check(Ok(()));
                relay::changed();
            }
            Ok(HoldEvent::Frame(frame)) => {
                if failed {
                    continue;
                }
                got += 1;
                if b.handoff.carrier_inbound(peer, frame).await.is_err() {
                    failed = true;
                }
            }
            Err(CarrierError::SinceMiss) => rt.repost(b, peer).await,
            Err(CarrierError::Rejected(w)) => {
                h.rejected = true;
                gateway::record_check(Err(reason(&CarrierError::Rejected(w)).into()));
                relay::changed();
            }
            Err(e) => {
                // the budget spent is a refusal, not a carrier that did not answer
                if rt.refusal_of(Some(b)).is_none() {
                    gateway::record_check(Err(reason(&e).into()));
                    relay::changed();
                }
            }
        }
    }
    let pair = lock(&b.pairs).get(&peer).cloned();
    if failed {
        if let Some(p) = &pair {
            let _ = b.carrier.restore_cursor(p, b.carrier.cursor(p));
        }
        h.failures = h.failures.saturating_add(1);
        h.stream = None;
        if h.failures == 1 {
            h.pending = true;
        } else {
            stop_hold(b, h);
        }
    } else if got > 0 {
        h.failures = 0;
        gateway::record_check(Ok(()));
        if let Some(p) = &pair {
            b.carrier.commit_drain(p);
        }
    }
    if ended && !h.pending {
        stop_hold(b, h);
    }
    if h.stream.is_some() && rt.refusal_of(Some(b)).is_some() {
        stop_hold(b, h);
        relay::changed();
    }
    drop(one);
    if got > 0 {
        // a drain boundary: the answer to what came (acks owed, re-posts re-armed) and the
        // cursor saved by that round
        b.request(Request::Post(peer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queue_coalesces_and_flush_now_reposts() {
        let mut q = Queue::default();
        assert!(q.is_empty());
        q.add(Request::Pair(Some(NodeId::from([1; 32]))));
        q.add(Request::Pair(Some(NodeId::from([1; 32]))));
        assert_eq!(q.flush.len(), 1);
        assert!(!q.all && !q.repost);
        q.add(Request::Post(NodeId::from([2; 32])));
        assert_eq!(q.post.len(), 1);
        q.add(Request::Net);
        assert!(q.all && q.check && !q.repost);
        q.add(Request::FlushNow);
        assert!(q.repost);
        let mut c = Queue::default();
        c.add(Request::Check);
        assert!(c.all && c.check && !c.repost);
    }

    #[test]
    fn the_check_reasons_are_fixed_sentences() {
        let cases = [
            CarrierError::Retry("ntfy: poll: https://x.example/abc failed".into()),
            CarrierError::Rejected("ntfy: 401 unauthorized"),
            CarrierError::Rejected("ntfy: 403 forbidden"),
            CarrierError::Rejected("ntfy: 413 message too large"),
            CarrierError::Rejected("ntfy: unexpected status"),
            CarrierError::SinceMiss,
            CarrierError::HandOff("fabric stopped".into()),
        ];
        for e in &cases {
            let r = reason(e);
            assert_eq!(gateway::scrub_reason(r), r, "{r}");
            assert!(!r.contains("x.example"));
        }
        assert_eq!(reason(&cases[1]), reason(&cases[2]));
    }
}
