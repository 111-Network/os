//! Layer 5 fabric wiring (`os/5-messaging/README.md` §2, §4, §9, §10.2, §15,
//! §17; task F2): `mesh111` running inside `111d`.
//!
//! ## What [`start`] does, in order
//!
//! 1. **Identity handle** ([`sealer::IdentitySealer`]): the keyset loaded at
//!    daemon start stays alive here for the daemon's lifetime (L2 §13 signer
//!    pattern) with the identity dir's L2 counter store; `mesh111` gets the
//!    handle, never key bytes. A missing `counters` file is L2 §8.4's
//!    state-absent case: nothing is sealed (§4 `restore_pending`) and
//!    nothing is written to the identity dir (see step 4 and
//!    [`FabricHandle::shutdown`]).
//! 2. **Store** (§10): `fabric/` (0700, created at runtime like `agent/`),
//!    `fabric/store.db` opened with the Mode S key (§10.2: `store_secret`
//!    behind the handle → `mesh111::keys::ModeS`), the §4 epoch decision fed
//!    the live `announce_last`. Opens are serialized process-wide. When
//!    `fabric/settings` was absent or invalid (moved aside), the home hub is
//!    rebuilt from the store's `enrolled` contact here
//!    ([`settings::rebuild_home_hub`]) and the file rewritten.
//! 3. **Own announce** (§7, A8): `fabric/announce`, re-minted only when its
//!    `appdata` (roles from the settings, the home-hub hint) changed — at
//!    start, and again on every `Settings` event, when the re-minted bytes
//!    go to `tcp-lan` for new connections ([`TcpLan::set_announce`]). The
//!    fabric's own copy (card answers inside INTROs) is the one read at
//!    start until the next start; `GET /contacts/me/card` and the hub offer
//!    (`POST /fabric/hub/offer`: its card and `111HUB:` bundle) carry the
//!    current one. While restore-pending no announce is presented at all,
//!    not even a valid stored one.
//! 4. **Fabric** (`Fabric::start`: epoch mint, `complete_partials` and
//!    orphan recovery are the library's own), then the **`tcp-lan`** adapter
//!    registered with its dialer (the fabric keeps the home hub dialed;
//!    configured direct `peers` from `fabric/settings` are dialed here,
//!    through `Fabric::dial_peer`, so the fabric knows those dials and an
//!    enrollment that ends without `ENROLL_OK` never releases one). No
//!    link is registered while the node has no announce or the identity
//!    holder is restore-pending: no listener, no dial, no `peers` — so no
//!    inbound packet ever reaches the L2 window (§4, L2 §8.4).
//! 5. **Listener** (§2, §8.1): `:11183` is bound on the LAN only while
//!    `hub_role` or `listen_lan` is on (re-evaluated on every settings
//!    change, so `PATCH /fabric/settings` takes effect immediately); with
//!    both off nothing listens (loopback is never a peer, so no loopback
//!    listener is bound either).
//! 6. **Events** (§15 SSE): one task drains the fabric's subscription —
//!    already coalesced per message per tick — in batches, coalesces again
//!    per key (a message keeps its first change, a contact its last), and
//!    publishes `message` / `contact` / `fabric` frames; `Settings` events
//!    are persisted to `fabric/settings` (0600) and reconcile the listener.
//!    Layer 6 `Foreign` events (`6-transport/README.md` §10) become
//!    `network_message` frames `{network, conv, id?, change, seq}`,
//!    coalesced again per `(conv, id)` key (first change wins): one
//!    conversation read per distinct conversation in the batch gives the
//!    network and the highest `seq`; a coalesced channel event carries no
//!    `id`. Foreign display times ("received" / "queued") live in the second
//!    instance, `fabric/display-times.foreign`, opened beside the first
//!    ([`FabricHandle::foreign_ingest`] records them for the bridge core).
//! 7. **Housekeeping**: every few seconds the `fabric` service facts
//!    ([`FabricHandle::provider`] → the registry rule) are refreshed from
//!    `Fabric::status`; every minute a §10.4 capacity pass runs with the
//!    filesystem's free/total bytes (`statvfs`; unknown ⇒ the store's
//!    `DiskSpace::UNKNOWN`, the floor never triggers). This is capacity
//!    monitoring, not a protocol timer: the fabric itself has none.
//!
//! ## Layer 7 seams (`os/7-relay/README.md`; board row M5)
//!
//! Each is inert without a pair: no dial, socket, task or file byte is
//! added while `paired_hubs` is empty (§5.11; the Pi floor).
//!
//! - **Pairs** (§4.6, amendment 10): `fabric/settings` carries
//!   `paired_hubs` ([`settings`]). The fabric gets its records
//!   `{node_id, role, via}` at start and sends them back in every
//!   `Settings` event; the daemon keeps its half beside them
//!   ([`settings::PairLink`]: the far address, the pinned link key, `lan`,
//!   the warning hash; [`FabricHandle::set_pair_link`], for D6). A record
//!   the fabric drops (an unpairing) takes the daemon's half with it — only
//!   once the fabric's **current** settings lack it (a `Settings` event can
//!   be older than what a route applied; one re-read then, M5b). Every
//!   change of the persisted settings is serialized: the pairing table is
//!   published and the file bytes built under one lock, the file written
//!   in that order. An address the home hub, a peer or another pair uses is
//!   refused for a pair ([`settings::pair_address_refusal`]).
//! - **Dials** (§4.6 step 5, one connection per pair): the **whole** pairs
//!   [`settings::pair_dials`] names are dialled through `Fabric::dial_peer`,
//!   as `peers` are, at start and after every pair change; one it no longer
//!   names is released (unless the home hub or a peer uses the address). A
//!   half pair rides the fabric's own pairing dial (kept while half). A row
//!   written by an M5 build of this branch (before `ours`/`theirs`) reads
//!   as half: it neither forwards nor is dialled until paired again.
//! - **The stream wrap** (§4.4): `TcpLanConfig.wrap` is
//!   `relay::tls::wrap` (D5), handed the live pairing table
//!   ([`settings::PairTable`]); `None` (until D5, and always without a
//!   non-private pair) keeps `tcp-lan` byte-identical. It is read once,
//!   when `tcp-lan` is built.
//! - **The tap** (§2.1, §3.5, §5.9): every `Message`, `Contact`, `Fabric`
//!   and `Forward` event of a batch goes to `relay::tap` as the batch is
//!   published, after the batch's settings are applied; a no-op while no
//!   Layer 7 consumer is on. `Forward` (the carrier's trigger, D11) has no
//!   SSE frame of its own. **After a pump lag** the lost events are not
//!   replayed: the settings are re-read (`sync_settings`) and the tap sees
//!   one `Fabric` event, which means "re-read everything" (D11: re-take
//!   every carrier pair's owed frames; D2/D3: re-read the status).
//! - **Lost hub** (§7.7, for D8): [`FabricHandle::replace_home_hub_address`]
//!   keeps a new home-hub address only after a handshake there named the
//!   hub (the dial's expected NodeId; refused where a connection is up
//!   already). The override is voided by any sign the membership ended
//!   (the hub forgotten or enrolled at again; `Rehomed`). A pump lag alone
//!   never voids it: the current settings and the hub's contact row are
//!   re-read, and only a changed membership ends it (M5c). No void path
//!   releases the address the home hub is shown at.
//! - **The carrier topic's X25519** (§6.2) stays behind the identity handle
//!   ([`IdentitySealer::carrier_topic_x25519`]), on no API.
//! - **Role bits 3–5** (§7.2, §19 code seam 4; row D9): `appdata` carries
//!   [`sealer::ROLE_GATEWAY`], [`sealer::ROLE_OFFLOAD`] and
//!   [`sealer::ROLE_ORIGIN`] beside mesh111's bits 0–2, from
//!   `state/relay.json` ([`RelayRoles`]): read once at start (before the
//!   first announce is ensured, so a restart re-mints nothing), then only
//!   when the relay settings route hands the new settings to
//!   [`FabricHandle::set_relay_settings`] (D10). A change re-mints the
//!   announce once (one counter value committed with an fsync before it
//!   exists, `id111` `next_announce_counter`); the same roles never re-mint
//!   (`ensure_announce` keeps the stored bytes). Every re-mint computes the
//!   `appdata` from the current fabric settings and relay roles under the
//!   re-mint lock, so concurrent changes settle on the latest. The layout
//!   is unchanged (`[1][roles u16 LE][0]` plus the hint, no TLV, §19 C8).
//! - **The offer's gateway fact** (§7.5): [`FabricHandle::gateway_offer`]
//!   gives `POST /fabric/hub/offer` its additive `gateway {carrier, mode}`;
//!   the `111HUB:` bundle, INTRO and cards carry nothing new.
//!
//! [`FabricHandle::shutdown`] stops the link, closes the fabric cleanly
//! (`Fabric::shutdown`: the store writes its clean-shutdown record with
//! `announce_last`, §10.1) — `Fabric::abort` (unclean: the next open mints a
//! new epoch) only when that does not finish within the grace — and then
//! commits the L2 counters clean (skipped while restore-pending: a counters
//! file written then would carry `announce_last = 0` and read as state
//! present at the next start — a counter reset, never a floor).
//!
//! Any failure leaves the handle **down** (`running() == false`, the reason
//! logged once): routes answer `503`, `GET /fabric` reports `store.open:
//! false`, and the `fabric` service row reads ERROR.

pub mod cli;
pub mod json;
pub mod proposal;
pub mod sealer;
pub mod settings;
pub mod times;

use crate::daemon::dashboard::api_v1::{ApiState, Problem};
use crate::daemon::relay::settings::{
    self as relay_settings, CarrierKind, CarrierMode, ConsentKind,
};
use crate::daemon::services::probe::Provider;
use crate::daemon::services::FabricFacts;
use crate::daemon::DaemonLayout;
use crate::identity::Identity;
use axum::http::StatusCode;
use id111::keyset::NodeId;
use mesh111::contacts::{ROLE_ALWAYS_ON_HUB, ROLE_MAILBOX};
use mesh111::fabric::{
    ContactChange, Fabric, FabricError, FabricEvent, FabricSettings, FabricStatus, ForeignChange,
    ForeignGroupPart, ForeignQueue, MessageChange, MessageView, PairView, MAX_PAIRED_HUBS,
};
use mesh111::keys::ModeS;
use mesh111::link::tcp_lan::{TcpLan, TcpLanConfig, DEFAULT_PORT};
use mesh111::link::{Dialer, LinkId, PeerAddr};
use mesh111::sealer::Sealer;
use mesh111::store::{
    Class, DiskSpace, ForeignInsert, ForeignMessageRow, OpenParams, Store, StoreError, Trust,
};
use sealer::{AnnounceOutcome, IdentitySealer};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

/// The fabric directory under the node root (§2).
pub const FABRIC_DIR: &str = "fabric";
/// Housekeeping ticks between capacity passes (≈ 1 min at 5 s).
const PRUNE_EVERY: u64 = 12;
/// Fabric events taken per SSE batch.
const BATCH: usize = 1024;

/// `tcp-lan` transport details (never protocol: §8.1).
#[derive(Debug, Clone)]
pub struct TcpTuning {
    /// §2: loopback is never a peer (tests on one host turn it off).
    pub refuse_loopback: bool,
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    pub handshake_timeout: Duration,
    pub keepalive: Duration,
}

impl Default for TcpTuning {
    fn default() -> Self {
        let d = TcpLanConfig::new(Vec::new());
        TcpTuning {
            refuse_loopback: d.refuse_loopback,
            backoff_min: d.backoff_min,
            backoff_max: d.backoff_max,
            handshake_timeout: d.handshake_timeout,
            keepalive: d.keepalive,
        }
    }
}

/// Where and how the fabric runs.
#[derive(Debug, Clone)]
pub struct FabricConfig {
    /// `<root>/fabric` (0700).
    pub fabric_dir: PathBuf,
    /// `<root>/identity` (the L2 `counters` file lives there).
    pub identity_dir: PathBuf,
    /// `<root>/state`: `relay.json` (Layer 7 §6.3) is read there once at
    /// start for the role bits 3–5 ([`RelayRoles`]).
    pub state_dir: PathBuf,
    /// The `tcp-lan` listener address when the settings ask for one
    /// (production: `0.0.0.0:11183`).
    pub listen: SocketAddr,
    pub tcp: TcpTuning,
    /// `hub_role` when `fabric/settings` does not exist yet (Cat 4).
    pub hub_default: bool,
    /// `POST /fabric/hub/enroll` waits this long for `ENROLL_OK`.
    pub enroll_timeout: Duration,
    /// Facts refresh interval (capacity passes every [`PRUNE_EVERY`]).
    pub housekeeping: Duration,
    /// `Fabric::shutdown` grace before `Fabric::abort`.
    pub shutdown_grace: Duration,
    /// The host advertised in hub offers when the listener is bound to an
    /// unspecified address (`None`: the first LAN IPv4).
    pub offer_host: Option<IpAddr>,
}

impl FabricConfig {
    /// The daemon's configuration for `layout` (Cat 4 ⇒ hub by default).
    pub fn production(layout: &DaemonLayout, category: Option<&str>) -> FabricConfig {
        FabricConfig {
            fabric_dir: layout.root.join(FABRIC_DIR),
            identity_dir: layout.identity_dir(),
            state_dir: layout.state_dir(),
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), DEFAULT_PORT),
            tcp: TcpTuning::default(),
            hub_default: settings::is_hub_category(category),
            enroll_timeout: Duration::from_secs(20),
            housekeeping: Duration::from_secs(5),
            shutdown_grace: Duration::from_secs(5),
            offer_host: None,
        }
    }
}

struct Live {
    fabric: Fabric,
    sealer: Arc<IdentitySealer>,
    tcp: Option<TcpLan>,
}

/// The running (or down) fabric, shared by routes, SSE, the registry and
/// the proposal path (`ApiState` ext: `Arc<FabricHandle>`).
pub struct FabricHandle {
    cfg: FabricConfig,
    live: Option<Live>,
    down: Option<String>,
    persisted: Mutex<settings::Persisted>,
    facts: RwLock<FabricFacts>,
    times: Mutex<times::DisplayTimes>,
    /// The second instance (Layer 6 §3): `foreign_messages` row ids.
    foreign_times: Mutex<times::DisplayTimes>,
    contacts: Mutex<HashMap<[u8; 32], Value>>,
    listener: tokio::sync::Mutex<Option<SocketAddr>>,
    /// Serializes announce re-mints on settings changes.
    remint: tokio::sync::Mutex<()>,
    /// Layer 7 §7.2, §7.5: the relay switches behind role bits 3–5 and the
    /// offer's gateway fact (read at start, then set by D10).
    relay: Mutex<RelayRoles>,
    /// LAN sharing this lifetime (`ApiState::lan_enabled`; role bit 5).
    lan: bool,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    /// Layer 7 §4.6: the daemon's pairing rows, shared with the stream wrap.
    pairs: settings::PairTable,
    /// Layer 7 §3.2 (row D3b): the per-pair views as last read from
    /// `Fabric::status`, for readers that must not wait on the actor.
    pair_views: Mutex<PairViews>,
    /// Layer 7 §4.6 step 5: the pair dials this handle started (far hub →
    /// address); also serializes their reconciliation.
    pair_dials: tokio::sync::Mutex<HashMap<NodeId, SocketAddr>>,
    /// Layer 7 §7.7: the home-hub address replaced in this run.
    rehomed: Mutex<Option<Rehomed>>,
    /// Serializes every change of `persisted` with its `fabric/settings`
    /// write (M5b): the pairing table is published and the file bytes are
    /// built under the `persisted` lock, then written before this one is
    /// released, so the table and the file follow the changes in one order.
    /// A tokio mutex: the current settings may be re-read under it; no
    /// await ever runs under the std `persisted` lock.
    persist: tokio::sync::Mutex<()>,
    /// Serializes home-hub address replacements.
    rehome: tokio::sync::Mutex<()>,
}

/// A home-hub address replaced in this run (Layer 7 §7.7): the fabric still
/// holds `from` (it has no setter), and the daemon persists and shows `to`
/// over it until the fabric's settings name the hub differently.
///
/// The override is **voided** (`void`; shown nowhere, then dropped and its
/// dial released as the fabric's current settings are applied, at once) by
/// any sign that the membership it
/// was taken for may have ended: a `Settings` event in a batch that shows
/// the hub neither at `from` nor at `to` (the pump applies only a batch's
/// last one), or a `Contact` event `Added` or `Removed` for the hub (it was
/// forgotten, or enrolled at again: an enrollment with a card always emits
/// `Added`). A forget-and-re-enrol at `from` therefore never has `to`
/// written over it (M5b F5); one during the trial wait, while no override
/// exists to catch its events, is caught by re-reading the hub and its
/// marker just before the override is recorded (M5c re-review).
///
/// A **pump lag** (events were lost) does not void it on its own (M5c S1:
/// the void would release `to`, the one verified connection to the hub,
/// and nothing would dial the hub again). After a lag the hub's contact
/// row is read: the override is voided when the row is gone or its
/// `marker` (the row's `first_seen_seq`, a store sequence taken before the
/// override was recorded: a forget deletes the row, a re-enrolment after
/// it inserts a new one) differs; the re-read settings then void it as a
/// `Settings` event would (the hub at neither address). No void path
/// releases the address the home hub is shown at ([`FabricHandle::releasable`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rehomed {
    hub: NodeId,
    from: Option<SocketAddr>,
    to: SocketAddr,
    /// The hub's contact row (`first_seen_seq`) when the override was
    /// taken; `None` (no row) never confirms the membership after a lag.
    marker: Option<u64>,
    void: bool,
}

/// The per-pair views (Layer 7 §3.2, §5.3 amendment 11: `reachable`,
/// `pending`) as this handle last read them from `Fabric::status`: at every
/// `fabric` frame it publishes for a fabric event, every `GET /fabric` and
/// every housekeeping pass (module doc, step 7). Kept so a reader on a tick
/// (the `gateway` row, row D3b) never waits on the fabric actor: reading them
/// is one lock, no round trip.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PairViews {
    /// Bumped whenever the views changed, the daemon's pairing halves were
    /// published ([`FabricHandle::pair_table`]: an address or `lan` set by
    /// [`FabricHandle::set_pair_link`] emits no fabric event) or a settings
    /// change was applied. A reader compares it to skip an unchanged read.
    pub generation: u64,
    /// When the views were last read from the fabric (monotonic); `None`
    /// before the first read.
    pub at: Option<Instant>,
    /// `FabricStatus::paired` as then read.
    pub views: Vec<PairView>,
}

impl std::fmt::Debug for FabricHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FabricHandle")
            .field("running", &self.live.is_some())
            .field("down", &self.down)
            .finish_non_exhaustive()
    }
}

/// `503` while the fabric is not running.
pub fn not_running() -> Problem {
    Problem::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "internal",
        "Messaging is not running on this node.",
    )
    .detail("The message store did not open; see the Messaging service page.")
}

/// The generic mapping of fabric failures onto contract problems (routes
/// refine where the contract names a code).
pub fn problem(e: &FabricError) -> Problem {
    use mesh111::mailbox::EnrollStatus;
    let p = |s: StatusCode, code: &'static str, title: &str| Problem::new(s, code, title);
    match e {
        FabricError::Stopped => not_running(),
        FabricError::NotFound => p(StatusCode::NOT_FOUND, "not_found", "Not found."),
        FabricError::KeyConflict => p(
            StatusCode::CONFLICT,
            "bad_request",
            "Those keys conflict with the ones pinned for this node.",
        )
        .detail("A card with different keys for a known NodeId never replaces them (§7.1)."),
        FabricError::Invalid(why) => p(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "The request was refused.",
        )
        .detail(*why),
        FabricError::NotRetryable => p(
            StatusCode::CONFLICT,
            "bad_request",
            "Not retryable in its current state.",
        ),
        FabricError::TooLarge(e) => p(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "The text is too long to send.",
        )
        .detail(format!("{e} (attachments arrive in v1.1).")),
        FabricError::RestorePending => p(
            StatusCode::CONFLICT,
            "restore_pending",
            "Nothing is sealed until the identity restore completes.",
        )
        .detail("The identity holder reports a restore floor (§4); messages wait."),
        FabricError::Card(e) => p(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "That card or NodeId is not valid.",
        )
        .detail(e.to_string()),
        FabricError::Enroll(EnrollStatus::LockedOut) => p(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "The hub locked this offer after 5 wrong codes.",
        )
        .detail("Ask the hub's owner for a new code."),
        FabricError::Enroll(s) => p(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            "The hub refused the enrollment.",
        )
        .detail(format!("{s:?}: the code is wrong, spent or dismissed.")),
        FabricError::Code(e) => p(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "That enrollment code is not valid.",
        )
        .detail(e.to_string()),
        FabricError::NoLink => p(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            "No link can reach that address.",
        ),
        FabricError::Store(StoreError::StoreFull) => p(
            StatusCode::CONFLICT,
            "bad_request",
            "The message store is full.",
        )
        .detail("New custody and bulk are refused until capacity is freed (§10.4)."),
        FabricError::Store(e) => p(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "The message store failed.",
        )
        .detail(e.to_string()),
        FabricError::Sealer(e) => p(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "The identity holder failed.",
        )
        .detail(e.to_string()),
    }
}

/// Serializes store opens process-wide (one store per daemon; tests open
/// several — mesh111's harness saw a rare SQLCipher failure on concurrent
/// first opens).
static OPEN_LOCK: Mutex<()> = Mutex::new(());

/// Free/total bytes of the filesystem holding `path` (`statvfs`).
pub fn disk_space(path: &Path) -> Option<DiskSpace> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let frsize = st.f_frsize as u64;
    Some(DiskSpace {
        free: (st.f_bavail as u64).saturating_mul(frsize),
        total: (st.f_blocks as u64).saturating_mul(frsize),
    })
}

/// The Layer 7 switches behind role bits 3–5 and the hub offer's gateway
/// fact (§7.2, §7.5), taken from `state/relay.json`
/// ([`RelayRoles::from_settings`]). Each is fail closed: a switch whose
/// consent is not recorded counts as off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelayRoles {
    /// `discovery.public_gateway_bit` with its `gateway_bit` consent
    /// recorded (§12).
    pub gateway_bit: bool,
    /// `discovery.offload_bit`.
    pub offload_bit: bool,
    /// `origin.serve` with its `origin_serve` consent recorded (§12).
    pub origin_serve: bool,
    /// The carrier while the gateway is on (`gateway.enabled`, a carrier
    /// kind, its server and the `carrier_enable` consent recorded): the
    /// carrier kind and the mode it runs in (5B reads as 5A on Termux,
    /// §3.1). `None` otherwise.
    pub carrier: Option<GatewayOffer>,
}

/// The hub offer's `gateway {carrier, mode}` (§7.5; contract `GatewayOffer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayOffer {
    /// `carrier.kind` (never `none`).
    pub carrier: CarrierKind,
    /// The mode the carrier runs in.
    pub mode: CarrierMode,
}

impl GatewayOffer {
    /// The contract's JSON object.
    pub fn json(&self) -> Value {
        json!({ "carrier": self.carrier.as_str(), "mode": self.mode.as_str() })
    }
}

/// Why the own announce does not carry the role bits just set (Layer 7
/// §7.2; row D4b): [`FabricHandle::set_relay_settings`] puts the previous
/// switches back and returns it, so the same call tries again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReannounceError {
    /// No announce counter could be committed (the identity `counters` file
    /// cannot be written, or the counter space is exhausted, §6.2): the
    /// previous announce is still presented (or none).
    NoCounter,
    /// The blocking mint task failed (it panicked or was cancelled).
    Task(String),
}

impl std::fmt::Display for ReannounceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReannounceError::NoCounter => f.write_str(
                "the role bits are not announced yet: no announce counter could be committed",
            ),
            ReannounceError::Task(e) => {
                write!(
                    f,
                    "the role bits are not announced yet: the re-mint failed ({e})"
                )
            }
        }
    }
}

impl std::error::Error for ReannounceError {}

impl RelayRoles {
    /// The switches as `r` holds them (`termux`: this node runs on Termux,
    /// where 5B runs as 5A).
    pub fn from_settings(r: &relay_settings::RelaySettings, termux: bool) -> RelayRoles {
        let consent = |k| r.consent_hash(k).is_some();
        let carrier_ready = match r.carrier_kind() {
            CarrierKind::None => false,
            CarrierKind::Ntfy => r.ntfy_server().is_some(),
        };
        RelayRoles {
            gateway_bit: r.public_gateway_bit() && consent(ConsentKind::GatewayBit),
            offload_bit: r.offload_bit(),
            origin_serve: r.origin_serve() && consent(ConsentKind::OriginServe),
            carrier: (r.gateway_enabled() && carrier_ready && consent(ConsentKind::CarrierEnable))
                .then(|| GatewayOffer {
                    carrier: r.carrier_kind(),
                    mode: r.carrier_mode_for(termux),
                }),
        }
    }

    /// The gateway fact while this node can act on it: the carrier is on
    /// and the node holds the hub role (§3.1: a gateway is a hub; with the
    /// role off no pairing or forwarding path runs, M4b).
    fn gateway(&self, s: &FabricSettings) -> Option<GatewayOffer> {
        self.carrier.filter(|_| s.hub_role)
    }
}

/// The `appdata` role bits for the fabric settings `s`, the Layer 7
/// switches `relay` and LAN sharing `lan` (`ApiState::lan_enabled`, fixed
/// for the daemon's lifetime). Bits 0–1 as Layer 5 sets them (§7.3 A8);
/// bits 3–5 (Layer 7 §7.2) only while what they promise holds:
/// - 3 `ROLE_GATEWAY`: the public bit consented **and** the gateway on
///   with the hub role (the offer's gateway fact, [`RelayRoles::carrier`]);
/// - 4 `ROLE_OFFLOAD`: `discovery.offload_bit`;
/// - 5 `ROLE_ORIGIN`: `origin.serve` consented, the hub role and LAN sharing
///   (§9.1: the origin is served only while all three are on; otherwise it
///   answers 404, and the bit would promise files nobody can fetch).
fn roles(s: &FabricSettings, relay: &RelayRoles, lan: bool) -> u16 {
    let mut r = 0;
    if s.holds_custody() {
        r |= ROLE_MAILBOX;
    }
    if s.hub_role {
        r |= ROLE_ALWAYS_ON_HUB;
    }
    if relay.gateway_bit && relay.gateway(s).is_some() {
        r |= sealer::ROLE_GATEWAY;
    }
    if relay.offload_bit {
        r |= sealer::ROLE_OFFLOAD;
    }
    if relay.origin_serve && s.hub_role && lan {
        r |= sealer::ROLE_ORIGIN;
    }
    r
}

/// The relay switches at start (Layer 7 §6.3: absent ⇒ all off; an invalid
/// file reads all off and is moved aside by `relay::settings::load`).
fn load_relay_roles(state_dir: &Path) -> RelayRoles {
    let loaded = relay_settings::load(state_dir);
    RelayRoles::from_settings(&loaded.settings, crate::config::on_termux())
}

fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

async fn boot(
    cfg: &FabricConfig,
    identity: Option<Identity>,
    persisted: &mut settings::Persisted,
    recover: Option<&settings::Recover>,
    pairs: &settings::PairTable,
    extra: (&RelayRoles, bool),
) -> Result<Live, String> {
    let identity = identity.ok_or("no identity is loaded")?;
    let (counters, how) = sealer::open_counters(&cfg.identity_dir)
        .map_err(|e| format!("identity counter store: {e}"))?;
    if how == sealer::CounterOpen::RestorePending {
        eprintln!(
            "111 fabric: the identity counter state is absent (restore without a floor): \
             nothing is sealed until it is set (§4 restore_pending)"
        );
    }
    let sealer = Arc::new(IdentitySealer::new(identity, counters));
    match boot_with(cfg, &sealer, persisted, recover, pairs, extra).await {
        Ok(l) => Ok(l),
        Err(e) => {
            // a no-op while restore-pending (no counter reset, L2 §8.4)
            let _ = sealer.shutdown_counters();
            Err(e)
        }
    }
}

/// The store's `enrolled` contacts, for [`settings::rebuild_home_hub`].
fn enrolled_contacts(store: &Store) -> Vec<settings::Enrolled> {
    match store.contacts() {
        Ok(rows) => rows
            .into_iter()
            .filter(|c| c.trust == Trust::Enrolled)
            .map(|c| settings::Enrolled {
                node_id: NodeId::from(c.node_id),
                custody: c.roles & ROLE_MAILBOX != 0,
                cursor: store.cursor(&c.node_id).unwrap_or(0),
            })
            .collect(),
        Err(e) => {
            eprintln!("111 fabric: contacts unreadable for the home-hub rebuild: {e}");
            Vec::new()
        }
    }
}

/// Settings file absent or invalid: rebuild the home hub from the store and
/// rewrite the file when there is something to keep (module doc, step 2).
///
/// `announced` is what the own announce as stored before this start
/// re-mints it says about the home hub ([`IdentitySealer::stored_hub_hint`]:
/// authoritative whenever a verified one is stored); the pairs salvaged
/// from an invalid file are never rebuilt as the home hub (Layer 7 §19 C7).
fn recover_settings(
    cfg: &FabricConfig,
    persisted: &mut settings::Persisted,
    recover: &settings::Recover,
    enrolled: &[settings::Enrolled],
    announced: sealer::StoredHubHint,
) {
    if persisted.fabric.home_hub.is_none() {
        if let Some(h) =
            settings::rebuild_home_hub(enrolled, recover.hint.as_ref(), announced, &recover.pairs)
        {
            eprintln!(
                "111 fabric: home hub {} rebuilt from its enrolled contact{}",
                json::hex(&h.node_id),
                match h.address {
                    Some(a) => format!(" at {a}"),
                    None => " (its address is unknown: nothing dials it until a fresh offer)"
                        .to_string(),
                }
            );
            persisted.fabric.home_hub = Some(h);
        } else if announced == sealer::StoredHubHint::NoHub && !enrolled.is_empty() {
            eprintln!("111 fabric: no home hub rebuilt: this node's stored announce names none");
        } else if !enrolled.is_empty() && recover.invalid {
            eprintln!(
                "111 fabric: no home hub could be identified among {} enrolled contact(s)",
                enrolled.len()
            );
        }
    }
    // An invalid file that was moved aside is replaced now; an absent one
    // is written only when it gained a home hub (absent = the defaults).
    let rewrite = if recover.invalid {
        recover.moved_to.is_some()
    } else {
        persisted.fabric.home_hub.is_some()
    };
    if rewrite {
        if let Err(e) = settings::save(&cfg.fabric_dir, persisted) {
            eprintln!("111 fabric: settings not saved: {e:#}");
        }
    }
}

async fn boot_with(
    cfg: &FabricConfig,
    sealer: &Arc<IdentitySealer>,
    persisted: &mut settings::Persisted,
    recover: Option<&settings::Recover>,
    pairs: &settings::PairTable,
    (relay, lan): (&RelayRoles, bool),
) -> Result<Live, String> {
    let node_id = sealer.node_id();
    let secret = Zeroizing::new(
        sealer
            .store_secret()
            .map_err(|e| format!("store secret: {e}"))?,
    );
    let provider = ModeS::new(secret, node_id.to_bytes());
    let params = OpenParams {
        announce_last: sealer.announce_last(),
        restore_floor_set: false,
    };
    let dir = cfg.fabric_dir.clone();
    let want_hubs = recover.is_some() && persisted.fabric.home_hub.is_none();
    let (store, decision, enrolled) = tokio::task::spawn_blocking(move || {
        let _one = OPEN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (store, decision) = Store::open_with(&dir, &provider, &params)?;
        let enrolled = if want_hubs {
            enrolled_contacts(&store)
        } else {
            Vec::new()
        };
        Ok::<_, StoreError>((store, decision, enrolled))
    })
    .await
    .map_err(|e| format!("store open task: {e}"))?
    .map_err(|e| format!("store open: {e}"))?;
    if let Some(r) = recover {
        // Read before `ensure_announce` below re-mints it.
        let announced = if want_hubs {
            sealer.stored_hub_hint(&cfg.fabric_dir)
        } else {
            sealer::StoredHubHint::NoAnnounce
        };
        recover_settings(cfg, persisted, r, &enrolled, announced);
    }

    let app = sealer::appdata(
        roles(&persisted.fabric, relay, lan),
        persisted.fabric.home_hub.as_ref().map(|h| &h.node_id),
    );
    // Restore-pending ⇒ `Missing`, even with a valid stored announce.
    let announce = sealer.ensure_announce(&cfg.fabric_dir, &app);
    let fabric = Fabric::start(
        Arc::clone(sealer) as Arc<dyn Sealer>,
        store,
        decision,
        persisted.fabric.clone(),
    )
    .map_err(|e| format!("fabric start: {e}"))?;
    // Restore-pending is checked on its own too (belt and braces): no link
    // ⇒ no inbound packet can ever tick the L2 window (L2 §8.4).
    let tcp = if announce == AnnounceOutcome::Missing || sealer.restore_pending() {
        eprintln!(
            "111 fabric: no own announce (restore pending): tcp-lan not started — \
             no listener, no dial, no peers"
        );
        None
    } else {
        let mut tc = TcpLanConfig::new(sealer.own_announce());
        tc.refuse_loopback = cfg.tcp.refuse_loopback;
        tc.backoff_min = cfg.tcp.backoff_min;
        tc.backoff_max = cfg.tcp.backoff_max;
        tc.handshake_timeout = cfg.tcp.handshake_timeout;
        tc.keepalive = cfg.tcp.keepalive;
        // Layer 7 §4.4 (D5): `None` keeps every connection plain and
        // byte-identical (module doc, "Layer 7 seams").
        tc.wrap = crate::daemon::relay::tls::wrap(&cfg.fabric_dir, pairs);
        let t = TcpLan::new(
            LinkId::new("tcp-lan"),
            tc,
            fabric.link_events(),
            fabric.gate(),
        );
        let link: Arc<dyn mesh111::link::Link> = Arc::new(t.clone());
        let dialer: Arc<dyn Dialer> = Arc::new(t.clone());
        if let Err(e) = fabric.add_link(link, Some(dialer)).await {
            fabric.abort();
            return Err(format!("tcp-lan link: {e}"));
        }
        for p in &persisted.peers {
            if p.node_id != node_id {
                if let Err(e) = fabric.dial_peer(p.address, p.node_id).await {
                    eprintln!("111 fabric: peer {} not dialed: {e}", p.address);
                }
            }
        }
        Some(t)
    };
    Ok(Live {
        fabric,
        sealer: Arc::clone(sealer),
        tcp,
    })
}

/// Start the fabric (see the module doc). Never fails: a fabric that cannot
/// run is returned down, with the reason logged.
pub async fn start(
    cfg: FabricConfig,
    identity: Option<Identity>,
    api: &Arc<ApiState>,
) -> Arc<FabricHandle> {
    let mut down = None;
    let mut persisted = settings::Persisted::default();
    let pairs: settings::PairTable = Arc::default();
    let no_identity = identity.is_none();
    let lan = api.lan_enabled;
    let mut relay = RelayRoles::default();
    let live = match ensure_dir(&cfg.fabric_dir) {
        Err(e) => {
            down = Some(format!("{}: {e}", cfg.fabric_dir.display()));
            None
        }
        Ok(()) => {
            let loaded = settings::load(&cfg.fabric_dir, cfg.hub_default, cfg.tcp.refuse_loopback);
            persisted = loaded.persisted;
            if let Ok(mut t) = pairs.write() {
                t.clone_from(&persisted.pairs);
            }
            // Layer 7 §7.2: read before the first announce is ensured, so
            // a restart with the same switches re-mints nothing.
            relay = load_relay_roles(&cfg.state_dir);
            match boot(
                &cfg,
                identity,
                &mut persisted,
                loaded.recover.as_ref(),
                &pairs,
                (&relay, lan),
            )
            .await
            {
                Ok(l) => Some(l),
                Err(e) => {
                    down = Some(e);
                    None
                }
            }
        }
    };
    if let Some(why) = &down {
        eprintln!("111 fabric: not running: {why}");
    }
    let (times, foreign_times) = if live.is_some() {
        (
            times::DisplayTimes::load(&cfg.fabric_dir),
            times::DisplayTimes::open(&cfg.fabric_dir, times::TimesFile::Foreign),
        )
    } else {
        (times::DisplayTimes::memory(), times::DisplayTimes::memory())
    };
    let handle = Arc::new(FabricHandle {
        facts: RwLock::new(FabricFacts {
            store_open: live.is_some(),
            no_identity: live.is_none() && no_identity,
            restore_pending: live.as_ref().is_some_and(|l| l.sealer.restore_pending()),
            ..FabricFacts::default()
        }),
        cfg,
        live,
        down,
        persisted: Mutex::new(persisted.clone()),
        times: Mutex::new(times),
        foreign_times: Mutex::new(foreign_times),
        contacts: Mutex::new(HashMap::new()),
        listener: tokio::sync::Mutex::new(None),
        remint: tokio::sync::Mutex::new(()),
        relay: Mutex::new(relay),
        lan,
        tasks: Mutex::new(Vec::new()),
        pairs,
        pair_views: Mutex::new(PairViews::default()),
        pair_dials: tokio::sync::Mutex::new(HashMap::new()),
        rehomed: Mutex::new(None),
        rehome: tokio::sync::Mutex::new(()),
        persist: tokio::sync::Mutex::new(()),
    });
    if let Some(l) = &handle.live {
        handle.reconcile_listener(&persisted.fabric).await;
        handle.reconcile_pair_dials().await;
        if let Ok(all) = l.fabric.contacts().await {
            if let Ok(mut c) = handle.contacts.lock() {
                for v in &all {
                    c.insert(v.node_id.to_bytes(), json::contact(v));
                }
            }
        }
        handle.refresh_facts().await;
        let rx = l.fabric.subscribe();
        let pump = tokio::spawn(pump(Arc::downgrade(&handle), Arc::downgrade(api), rx));
        let house = tokio::spawn(housekeeping(Arc::downgrade(&handle), Arc::downgrade(api)));
        if let Ok(mut t) = handle.tasks.lock() {
            t.push(pump);
            t.push(house);
        }
    }
    handle
}

impl FabricHandle {
    /// A handle with no fabric behind it (down, no actor, no task) holding
    /// `persisted` (tests of readers that must never reach the actor: a
    /// round trip here fails as "not running").
    #[cfg(test)]
    pub(crate) fn detached(cfg: FabricConfig, persisted: settings::Persisted) -> Arc<FabricHandle> {
        let pairs: settings::PairTable = Arc::new(RwLock::new(persisted.pairs.clone()));
        Arc::new(FabricHandle {
            facts: RwLock::new(FabricFacts::default()),
            cfg,
            live: None,
            down: Some("detached (test)".into()),
            persisted: Mutex::new(persisted),
            times: Mutex::new(times::DisplayTimes::memory()),
            foreign_times: Mutex::new(times::DisplayTimes::memory()),
            contacts: Mutex::new(HashMap::new()),
            listener: tokio::sync::Mutex::new(None),
            remint: tokio::sync::Mutex::new(()),
            relay: Mutex::new(RelayRoles::default()),
            lan: false,
            tasks: Mutex::new(Vec::new()),
            pairs,
            pair_views: Mutex::new(PairViews::default()),
            pair_dials: tokio::sync::Mutex::new(HashMap::new()),
            rehomed: Mutex::new(None),
            rehome: tokio::sync::Mutex::new(()),
            persist: tokio::sync::Mutex::new(()),
        })
    }

    /// Tests: record `views` as if `Fabric::status` had just returned them.
    #[cfg(test)]
    pub(crate) fn observe_pair_views(&self, views: &[PairView]) {
        self.store_pair_views(views);
    }

    /// Tests: publish `pairs` as the daemon's halves, as a pairing route does.
    #[cfg(test)]
    pub(crate) fn replace_pair_links(&self, pairs: Vec<settings::PairLink>) {
        let mut p = self.persisted.lock().unwrap_or_else(|p| p.into_inner());
        p.pairs = pairs;
        self.publish_pairs(&p.pairs);
    }

    /// The running fabric.
    ///
    /// Foreign **writes** (Layer 6 §3: ingest, send groups, relay copies)
    /// go through this handle's wrappers ([`FabricHandle::foreign_ingest`],
    /// [`FabricHandle::foreign_group_create`],
    /// [`FabricHandle::foreign_queue_sends`]): they record each new row's
    /// "received" / "queued" display time in `display-times.foreign` when
    /// the row is written. A caller writing through the raw [`Fabric`]
    /// leaves that to the first render: a channel row coalesced into a
    /// per-conversation frame (no `id`) is then stamped when it is first
    /// read, not when it arrived. Reads and state changes may use the raw
    /// fabric.
    pub fn fabric(&self) -> Option<&Fabric> {
        self.live.as_ref().map(|l| &l.fabric)
    }

    /// The identity handle (tests, diagnostics).
    pub fn sealer(&self) -> Option<&Arc<IdentitySealer>> {
        self.live.as_ref().map(|l| &l.sealer)
    }

    /// How long `POST /fabric/hub/enroll` waits for `ENROLL_OK`.
    pub fn enroll_timeout(&self) -> Duration {
        self.cfg.enroll_timeout
    }

    /// Is the fabric running?
    pub fn running(&self) -> bool {
        self.live.is_some()
    }

    /// The identity holder reports a restore floor (§4): nothing is sealed,
    /// no link runs.
    pub fn restore_pending(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| l.sealer.restore_pending())
    }

    /// The `tcp-lan` adapter refuses loopback peers (production, §2).
    pub fn refuse_loopback(&self) -> bool {
        self.cfg.tcp.refuse_loopback
    }

    /// Why it is not running.
    pub fn down_reason(&self) -> Option<&str> {
        self.down.as_deref()
    }

    /// This node's stored announce (empty when none).
    pub fn own_announce(&self) -> Vec<u8> {
        self.live
            .as_ref()
            .map(|l| l.sealer.own_announce())
            .unwrap_or_default()
    }

    /// The `tcp-lan` adapter (tests, diagnostics).
    pub fn tcp(&self) -> Option<&TcpLan> {
        self.live.as_ref().and_then(|l| l.tcp.as_ref())
    }

    /// The bound `tcp-lan` listener, if any.
    pub async fn listen_addr(&self) -> Option<SocketAddr> {
        *self.listener.lock().await
    }

    /// The persisted settings as last written.
    pub fn persisted(&self) -> settings::Persisted {
        self.persisted.lock().map(|p| p.clone()).unwrap_or_default()
    }

    /// The `fabric` service facts provider (registry rule, §15 Services).
    pub fn provider(self: &Arc<Self>) -> Provider<FabricFacts> {
        let me = Arc::downgrade(self);
        Arc::new(move || {
            me.upgrade()
                .and_then(|h| h.facts.read().ok().map(|f| f.clone()))
        })
    }

    /// Bind or release the listener per `s` (§2: hub role or LAN listener).
    pub async fn reconcile_listener(&self, s: &FabricSettings) {
        let Some(tcp) = self.tcp() else { return };
        let want = s.hub_role || s.listen_lan;
        let mut bound = self.listener.lock().await;
        match (want, *bound) {
            (true, None) => match tcp.listen(self.cfg.listen).await {
                Ok(a) => *bound = Some(a),
                Err(e) => eprintln!(
                    "111 fabric: tcp-lan listener on {} failed: {e}",
                    self.cfg.listen
                ),
            },
            (false, Some(_)) => {
                tcp.stop_listening();
                *bound = None;
            }
            _ => {}
        }
    }

    /// The `tcp-lan` address a hub offer advertises: the bound listener,
    /// with an unspecified bind address replaced by the configured or first
    /// LAN IPv4 (D16 e); `None` while nothing listens on the LAN.
    pub async fn offer_address(&self) -> Option<SocketAddr> {
        let a = self.listen_addr().await?;
        if !a.ip().is_unspecified() {
            return Some(a);
        }
        let ip = self.cfg.offer_host.or_else(|| {
            crate::daemon::dashboard::bind::lan_ipv4s()
                .first()
                .map(|v4| IpAddr::V4(*v4))
        })?;
        Some(SocketAddr::new(ip, a.port()))
    }

    /// The display time of message `id` (recorded now when unseen).
    pub fn at_of(&self, id: i64) -> i64 {
        self.times
            .lock()
            .map(|mut t| t.get_or_record(id, times::now()))
            .unwrap_or_else(|_| times::now())
    }

    /// `Message` JSON for a view.
    pub fn message_json(&self, v: &MessageView) -> Value {
        json::message(v, self.at_of(v.id))
    }

    /// Queue a text message (`POST /messages`, an accepted proposal): the
    /// sender's `sent_at` claim is this node's clock (§3.3: display only,
    /// shown to the recipient as unverified).
    pub async fn send(&self, to: NodeId, text: &str) -> Result<i64, FabricError> {
        let fabric = self.fabric().ok_or(FabricError::Stopped)?;
        let now = times::now();
        let id = fabric.send(to, text, u64::try_from(now).ok()).await?;
        if let Ok(mut t) = self.times.lock() {
            t.record(id, now);
        }
        Ok(id)
    }

    /// The display time of foreign row `id` (Layer 6 §3: the second
    /// instance; recorded now when unseen).
    pub fn foreign_at_of(&self, id: i64) -> i64 {
        self.foreign_times
            .lock()
            .map(|mut t| t.get_or_record(id, times::now()))
            .unwrap_or_else(|_| times::now())
    }

    /// Record "now" as the display time of foreign rows first seen now
    /// (received, or queued); an id with a time keeps it.
    pub fn record_foreign_times(&self, ids: impl IntoIterator<Item = i64>) {
        let now = times::now();
        if let Ok(mut t) = self.foreign_times.lock() {
            for id in ids {
                t.record(id, now);
            }
        }
    }

    /// `ForeignMessage` JSON for a row (its display time from the foreign
    /// instance).
    pub fn foreign_message_json(&self, m: &ForeignMessageRow) -> Value {
        json::foreign_message(m, self.foreign_at_of(m.id))
    }

    /// The bridge core's ingest (Layer 6 §3): one bounded batch through the
    /// fabric ([`Fabric::foreign_ingest`]: ≤ 256 rows, one transaction,
    /// awaited), with each new row's "received" display time recorded now.
    pub async fn foreign_ingest(
        &self,
        batch: Vec<ForeignInsert>,
    ) -> Result<Vec<Option<i64>>, FabricError> {
        let fabric = self.fabric().ok_or(FabricError::Stopped)?;
        let ids = fabric.foreign_ingest(batch).await?;
        self.record_foreign_times(ids.iter().flatten().copied());
        Ok(ids)
    }

    /// A send group with its foreign parts (§9, [`Fabric::foreign_group_create`];
    /// `targets` from [`json::send_group_targets`]), each row's "queued"
    /// display time recorded now.
    pub async fn foreign_group_create(
        &self,
        text: &str,
        targets: &str,
        parts: Vec<ForeignGroupPart>,
    ) -> Result<(i64, Vec<i64>), FabricError> {
        let fabric = self.fabric().ok_or(FabricError::Stopped)?;
        let (group, ids) = fabric.foreign_group_create(text, targets, parts).await?;
        self.record_foreign_times(ids.iter().copied());
        Ok((group, ids))
    }

    /// Queue own rows (relay copies; [`Fabric::foreign_queue_sends`]), each
    /// row's "queued" display time recorded now.
    pub async fn foreign_queue_sends(
        &self,
        sends: Vec<ForeignQueue>,
    ) -> Result<Vec<i64>, FabricError> {
        let fabric = self.fabric().ok_or(FabricError::Stopped)?;
        let ids = fabric.foreign_queue_sends(sends).await?;
        self.record_foreign_times(ids.iter().copied());
        Ok(ids)
    }

    /// §4 at most once: withdraw a pending handoff mark
    /// ([`Fabric::foreign_handoff_abort`]) after the carrier reported a
    /// pre-air refusal and no `Accepted` (`Retry::Later`): the row is
    /// offered again by the next tick.
    pub async fn foreign_handoff_abort(&self, id: i64) -> Result<(), FabricError> {
        let fabric = self.fabric().ok_or(FabricError::Stopped)?;
        fabric.foreign_handoff_abort(id).await
    }

    /// The store's quota per refusable class (`None` = unset).
    pub async fn quotas(&self) -> Vec<(Class, Option<u64>)> {
        let Some(f) = self.fabric() else {
            return Vec::new();
        };
        f.with_store(|s| {
            json::quota_classes()
                .map(|c| (c, s.quota(c).ok().flatten()))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default()
    }

    /// `GET /fabric` (`FabricStatus`; the down shape when not running).
    pub async fn status_json(&self) -> Result<Value, FabricError> {
        let Some(f) = self.fabric() else {
            return Ok(json::status_down(&self.persisted().fabric));
        };
        let mut st = f.status().await?;
        // the store reports it after a refused epoch mint; the identity
        // holder is the authority (L2 §8.4)
        st.store.restore_pending |= self.restore_pending();
        // Layer 7 §7.7: the replaced home-hub address over the fabric's copy
        if let Some(x) = self
            .rehomed
            .lock()
            .ok()
            .and_then(|r| *r)
            .filter(|x| !x.void)
        {
            if let Some(h) = st.settings.home_hub.as_mut() {
                if h.node_id == x.hub && h.address == x.from {
                    h.address = Some(x.to);
                }
            }
            if st.hub.node_id == Some(x.hub) && st.hub.address == x.from {
                st.hub.address = Some(x.to);
            }
        }
        self.set_facts(&st);
        let q = self.quotas().await;
        Ok(json::status(&st, &q))
    }

    fn set_facts(&self, st: &FabricStatus) {
        let facts = FabricFacts {
            store_open: true,
            no_identity: false,
            restore_pending: st.store.restore_pending || self.restore_pending(),
            store_full: st.store.store_full,
            hub_reachable: st.hub.enrolled.then_some(st.hub.reachable),
            hub_connected: st.hub.enrolled && st.hub.connected,
            members: st
                .settings
                .holds_custody()
                .then(|| u32::try_from(st.hub.members).unwrap_or(u32::MAX)),
            attributed_links: st
                .links
                .iter()
                .map(|l| u32::try_from(l.peers).unwrap_or(u32::MAX))
                .fold(0u32, u32::saturating_add),
        };
        if let Ok(mut f) = self.facts.write() {
            *f = facts;
        }
        self.store_pair_views(&st.paired);
    }

    /// Keep `views` as the latest read (Layer 7 §3.2, row D3b): the time is
    /// always taken, the generation bumped only when they changed.
    fn store_pair_views(&self, views: &[PairView]) {
        let mut v = self.pair_views.lock().unwrap_or_else(|p| p.into_inner());
        v.at = Some(Instant::now());
        if v.views != views {
            v.views = views.to_vec();
            v.generation = v.generation.wrapping_add(1);
        }
    }

    /// Something a reader of [`FabricHandle::pair_views`] derives its pair
    /// set from changed (the pairing halves, the settings).
    fn bump_pair_generation(&self) {
        let mut v = self.pair_views.lock().unwrap_or_else(|p| p.into_inner());
        v.generation = v.generation.wrapping_add(1);
    }

    /// The per-pair views as last read ([`PairViews`]): one lock, never an
    /// actor round trip (Layer 7 §3.2, row D3b).
    pub fn pair_views(&self) -> PairViews {
        self.pair_views
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// [`PairViews::generation`] alone (one lock, nothing cloned).
    pub fn pair_generation(&self) -> u64 {
        self.pair_views
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .generation
    }

    /// Refresh the service facts from `Fabric::status`.
    pub async fn refresh_facts(&self) {
        let Some(f) = self.fabric() else { return };
        match f.status().await {
            Ok(st) => self.set_facts(&st),
            Err(e) => {
                eprintln!("111 fabric: status: {e}");
                if let Ok(mut x) = self.facts.write() {
                    x.store_open = false;
                }
            }
        }
    }

    /// Persist a settings change (the peers list and the daemon's pairing
    /// halves kept), apply it to the listener, re-mint the own announce when
    /// its `appdata` changed, and reconcile the pair dials (Layer 7 §4.6
    /// step 5). Returns the settings as persisted (a replaced home-hub
    /// address over the fabric's copy, §7.7).
    ///
    /// `s` may be older than what this handle last applied (a route's
    /// `sync_settings` and the pump race; M5b F2). So when `s` lacks a pair
    /// record this handle holds, the fabric's **current** settings are
    /// re-read (one actor call, only then) and applied instead: a record,
    /// and the daemon's half beside it, is dropped only when the fabric no
    /// longer holds it (an unpairing); a half whose record has not arrived
    /// yet stays (module doc, "Layer 7 seams"). The same re-read runs when
    /// `s` holds a record this handle lacks (M5c S2: a view older than an
    /// unpairing [`FabricHandle::set_pair_link`] already folded in must not
    /// bring the record back, or the unpairing's own event would then drop
    /// the fresh half); a new pairing's `ENROLL_OK` costs that one call.
    /// When that re-read fails
    /// (the fabric stopped) nothing is persisted. The pairing table and the
    /// file bytes are produced under the `persisted` lock and the file is
    /// written under `persist`, so both follow the changes in one order.
    /// A home-hub address the §7.7 override leaves is released under
    /// `persist`, and only when [`FabricHandle::releasable`].
    async fn on_settings(&self, s: FabricSettings) -> FabricSettings {
        let one = self.persist.lock().await;
        let mut s = s;
        let records_differ = self.persisted.lock().is_ok_and(|p| {
            let has =
                |v: &[mesh111::fabric::PairedHub], n: &NodeId| v.iter().any(|x| x.node_id == *n);
            p.fabric
                .paired_hubs
                .iter()
                .any(|h| !has(&s.paired_hubs, &h.node_id))
                || s.paired_hubs
                    .iter()
                    .any(|h| !has(&p.fabric.paired_hubs, &h.node_id))
        });
        if records_differ {
            let Some(f) = self.fabric() else { return s };
            match f.settings().await {
                Ok(current) => s = current,
                Err(e) => {
                    eprintln!("111 fabric: settings not re-read ({e}); nothing persisted");
                    return s;
                }
            }
        }
        let stale = self.apply_rehome(&mut s);
        let (snapshot, bytes) = match self.persisted.lock() {
            Ok(mut p) => {
                let gone: Vec<NodeId> = p
                    .fabric
                    .paired_hubs
                    .iter()
                    .map(|h| h.node_id)
                    .filter(|n| !s.paired_hubs.iter().any(|h| h.node_id == *n))
                    .collect();
                let before = p.pairs.len();
                p.pairs.retain(|l| !gone.contains(&l.node_id));
                if p.pairs.len() != before {
                    self.publish_pairs(&p.pairs);
                }
                p.fabric = s.clone();
                self.bump_pair_generation();
                (p.clone(), settings::encoded(&p))
            }
            Err(_) => return s,
        };
        self.write_settings(&bytes);
        if let (Some(a), Some(tcp)) = (stale, self.tcp()) {
            if self.releasable(&snapshot, a) {
                Dialer::release(tcp, a);
            }
        }
        drop(one);
        self.reconcile_listener(&s).await;
        // a failed re-mint is logged inside; the next settings change tries again
        let _ = self.refresh_announce().await;
        self.reconcile_pair_dials().await;
        s
    }

    /// Write `fabric/settings` from bytes built under the `persisted` lock;
    /// the caller holds `persist` (the write order is the change order).
    fn write_settings(&self, text: &str) {
        if let Err(e) = settings::save_encoded(&self.cfg.fabric_dir, text) {
            eprintln!("111 fabric: settings not saved: {e:#}");
        }
    }

    /// `s` with this run's replaced home-hub address put over the fabric's
    /// own copy (Layer 7 §7.7). When the replacement was voided
    /// ([`Rehomed`]), or `s` no longer names that hub at that address
    /// (forgotten, re-enrolled elsewhere or anew), the replacement is
    /// forgotten, and its address returned for release.
    fn apply_rehome(&self, s: &mut FabricSettings) -> Option<SocketAddr> {
        let Ok(mut r) = self.rehomed.lock() else {
            return None;
        };
        let x = (*r)?;
        match s.home_hub.as_mut() {
            _ if x.void => {
                *r = None;
                Some(x.to)
            }
            Some(h) if h.node_id == x.hub && h.address == x.from => {
                h.address = Some(x.to);
                None
            }
            Some(h) if h.node_id == x.hub && h.address == Some(x.to) => {
                *r = None;
                None
            }
            _ => {
                *r = None;
                Some(x.to)
            }
        }
    }

    /// Void this run's §7.7 override when `batch` may show the end of the
    /// membership it was taken for ([`Rehomed`]: a `Settings` event that
    /// shows the hub neither at `from` nor at `to`, a `Contact` `Added` or
    /// `Removed` for the hub). Returns whether it was voided just now.
    fn void_rehome_on(&self, batch: &[FabricEvent]) -> bool {
        let Ok(mut r) = self.rehomed.lock() else {
            return false;
        };
        let Some(x) = r.as_mut() else { return false };
        if x.void {
            return false;
        }
        let (hub, from, to) = (x.hub, x.from, x.to);
        let ended = batch.iter().any(|ev| match ev {
            FabricEvent::Settings(s) => !s
                .home_hub
                .is_some_and(|h| h.node_id == hub && (h.address == from || h.address == Some(to))),
            FabricEvent::Contact {
                node_id,
                change: ContactChange::Added | ContactChange::Removed,
            } => *node_id == hub,
            _ => false,
        });
        x.void = ended;
        ended
    }

    /// After a pump lag (the events that could have ended this run's §7.7
    /// override were lost): read the hub's contact row and void the
    /// override only when the row is gone or its marker differs from the
    /// one taken with the override ([`Rehomed`], M5c S1). The settings
    /// half of the check (the hub at neither address) is the re-read that
    /// follows ([`FabricHandle::apply_rehome`]). One store read, only while
    /// an override is live. A read that fails shows no membership change
    /// and keeps the override (its address is the hub's, verified by a
    /// handshake; a void would drop the one dial to it); an override taken
    /// without a marker is never confirmed (void).
    async fn recheck_rehome_after_lag(&self) {
        let Some(x) = self
            .rehomed
            .lock()
            .ok()
            .and_then(|r| *r)
            .filter(|x| !x.void)
        else {
            return;
        };
        match self.hub_marker(x.hub).await {
            Ok(now) if x.marker.is_some() && now == x.marker => return,
            Ok(_) => {}
            Err(e) => {
                eprintln!("111 fabric: home-hub contact not read after the lag ({e}); kept");
                if x.marker.is_some() {
                    return;
                }
            }
        }
        if let Ok(mut r) = self.rehomed.lock() {
            // the same override still (a replacement may have run meanwhile)
            if let Some(y) = r.as_mut().filter(|y| y.hub == x.hub && y.to == x.to) {
                y.void = true;
            }
        }
    }

    /// The membership marker of `hub` ([`Rehomed`]): its contact row's
    /// `first_seen_seq` (a store sequence, no clock); `Ok(None)` when there
    /// is no row, `Err` when it cannot be read.
    async fn hub_marker(&self, hub: NodeId) -> Result<Option<u64>, String> {
        let f = self.fabric().ok_or("the fabric is not running")?;
        f.with_store(move |s| {
            s.contact(hub.as_bytes())
                .map(|c| c.map(|c| c.first_seen_seq))
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
    }

    /// May a dial at `address` that the §7.7 override (or its trial) used
    /// be released beside `snapshot` (the persisted settings as just
    /// applied)? Not while the home hub is shown at it, a configured peer
    /// or a paired hub uses it, or a live override names it (M5c S1).
    fn releasable(&self, snapshot: &settings::Persisted, address: SocketAddr) -> bool {
        !settings::holds_address(snapshot, address)
            && !snapshot.pairs.iter().any(|l| l.address == Some(address))
            && !self
                .rehomed
                .lock()
                .ok()
                .and_then(|r| *r)
                .is_some_and(|x| !x.void && x.to == address)
    }

    /// Replace the live pairing table; called under the `persisted` lock
    /// (lock order: `persisted`, then the table).
    fn publish_pairs(&self, pairs: &[settings::PairLink]) {
        if let Ok(mut t) = self.pairs.write() {
            *t = pairs.to_vec();
        }
        // a changed address or `lan` emits no fabric event (row D3b)
        self.bump_pair_generation();
    }

    /// Layer 7 §4.6 step 5: dial the pairs [`settings::pair_dials`] names
    /// (through `Fabric::dial_peer`, so the fabric knows the dial and an
    /// enrollment attempt never releases it) and release the ones it no
    /// longer names, unless the home hub or a configured peer uses the
    /// address. Nothing without a running `tcp-lan`; nothing at all while
    /// no pair is recorded (no dial, no task).
    async fn reconcile_pair_dials(&self) {
        let Some(l) = &self.live else { return };
        let Some(tcp) = &l.tcp else { return };
        let mut held = self.pair_dials.lock().await;
        let p = self.persisted();
        let want = settings::pair_dials(&l.sealer.node_id(), &p);
        let gone: Vec<(NodeId, SocketAddr)> = held
            .iter()
            .map(|(n, a)| (*n, *a))
            .filter(|x| !want.contains(x))
            .collect();
        for (n, a) in gone {
            held.remove(&n);
            if !settings::holds_address(&p, a) && !want.iter().any(|(_, w)| *w == a) {
                Dialer::release(tcp, a);
            }
        }
        for (n, a) in want {
            if held.get(&n) == Some(&a) {
                continue;
            }
            match l.fabric.dial_peer(a, n).await {
                Ok(()) => {
                    held.insert(n, a);
                }
                Err(e) => eprintln!("111 fabric: paired hub {a} not dialed: {e}"),
            }
        }
    }

    /// The daemon's pairing rows (Layer 7 §4.6), live: the table the
    /// `tcp-lan` stream wrap reads (D5).
    pub fn pair_table(&self) -> settings::PairTable {
        Arc::clone(&self.pairs)
    }

    /// Layer 7 §4.6 (for D6): record or replace this hub's half of the
    /// pairing row of `link.node_id` (the far hub's address, its pinned link
    /// key, `lan`, the warning hash, `far_holds_address`). Called before the
    /// pairing's `Fabric::pair`, so the pin is in place for its handshake;
    /// the half reaches `fabric/settings` with the fabric's record: at once
    /// when the record exists, else with the `Settings` event of its
    /// `ENROLL_OK`. Refused (`Invalid`) without the hub role (§5.11), for
    /// this node, for an address that is no dial target or that the home
    /// hub, a configured peer or another paired hub already uses
    /// ([`settings::pair_address_refusal`]), and beyond [`MAX_PAIRED_HUBS`]
    /// halves. The §4.6 step 5 dials follow.
    ///
    /// When the persisted settings hold a record for `link.node_id`, the
    /// fabric's **current** settings are re-read first (one actor call,
    /// only then; under `persist`) and the records it no longer holds are
    /// dropped from the persisted ones with their halves
    /// ([`settings::fold_dropped_records`]): an unpairing whose `Settings`
    /// event the pump has not applied yet is applied here, so a fresh half
    /// for a re-pairing counts as not yet recorded (kept in memory, written
    /// with its record later) and the unpairing's late event drops nothing
    /// (M5c S2).
    pub async fn set_pair_link(&self, link: settings::PairLink) -> Result<(), FabricError> {
        let l = self.live.as_ref().ok_or(FabricError::Stopped)?;
        if let Some(why) =
            settings::pair_link_refusal(&link, &l.sealer.node_id(), self.cfg.tcp.refuse_loopback)
        {
            return Err(FabricError::Invalid(why));
        }
        let one = self.persist.lock().await;
        let held = self
            .persisted
            .lock()
            .map_err(|_| FabricError::Stopped)?
            .fabric
            .paired_hubs
            .iter()
            .any(|h| h.node_id == link.node_id);
        let current = match held {
            true => Some(l.fabric.settings().await?.paired_hubs),
            false => None,
        };
        let bytes = {
            let mut p = self.persisted.lock().map_err(|_| FabricError::Stopped)?;
            if !p.fabric.hub_role {
                return Err(FabricError::Invalid("pairing needs the hub role"));
            }
            let folded = current
                .as_deref()
                .is_some_and(|c| settings::fold_dropped_records(&mut p, c));
            if let Some(why) = settings::pair_address_refusal(&p, &link) {
                return Err(FabricError::Invalid(why));
            }
            match p.pairs.iter().position(|x| x.node_id == link.node_id) {
                Some(i) => p.pairs[i] = link.clone(),
                None if p.pairs.len() >= MAX_PAIRED_HUBS => {
                    return Err(FabricError::Invalid("too many paired hubs"))
                }
                None => p.pairs.push(link.clone()),
            }
            self.publish_pairs(&p.pairs);
            let recorded = p
                .fabric
                .paired_hubs
                .iter()
                .any(|h| h.node_id == link.node_id);
            (recorded || folded).then(|| settings::encoded(&p))
        };
        if let Some(b) = bytes {
            self.write_settings(&b);
        }
        drop(one);
        self.reconcile_pair_dials().await;
        Ok(())
    }

    /// Drop this hub's half of the row of `node_id` (an abandoned pairing,
    /// D6). A recorded pair keeps its fabric record (`Fabric::unpair` drops
    /// that, and the half with it). Returns whether a half was held.
    pub async fn forget_pair_link(&self, node_id: &NodeId) -> bool {
        let one = self.persist.lock().await;
        let bytes = match self.persisted.lock() {
            Ok(mut p) => {
                let before = p.pairs.len();
                p.pairs.retain(|x| x.node_id != *node_id);
                if p.pairs.len() == before {
                    return false;
                }
                self.publish_pairs(&p.pairs);
                let recorded = p.fabric.paired_hubs.iter().any(|h| h.node_id == *node_id);
                recorded.then(|| settings::encoded(&p))
            }
            Err(_) => return false,
        };
        if let Some(b) = bytes {
            self.write_settings(&b);
        }
        drop(one);
        self.reconcile_pair_dials().await;
        true
    }

    /// Layer 7 §7.7 lost-hub recovery (for D8): make `address` the home
    /// hub's once a handshake there names the hub. A trial dial expects the
    /// home hub's NodeId, so the adapter drops a connection whose first
    /// announce names another node (`tcp_lan.rs` module doc, "Reconnect");
    /// when a connection at `address` passes that handshake within `wait`
    /// (monotonic), the address is kept: dialled through `Fabric::dial_peer`,
    /// written to `fabric/settings`, shown by `GET /fabric`, and the previous
    /// address released (unless a peer or a pair uses it). `Ok(false)`: no
    /// handshake named the hub in time; the trial dial is released and
    /// nothing changed. Refused (`Invalid`) without a home hub, for an
    /// address that is no dial target or that a configured peer or a pair
    /// uses, and for an address a connection is already up at (M5b F4: the
    /// trial dial would be a no-op there and that connection's handshake
    /// named whichever node its own dial expected); `NoLink` while no
    /// `tcp-lan` runs. The trial dial is released only as the daemon's own
    /// (M5c S3, [`FabricHandle::release_trial`]): not when, after the wait,
    /// the shown home hub, a peer, a pair or a pair dial uses the address,
    /// or the fabric's own home-hub copy moved to it meanwhile.
    /// A dial target the fabric alone holds there (a pairing dial in
    /// back-off with no daemon half) is invisible to the daemon: the
    /// adapter keeps the first dial's expected NodeId and offers no
    /// "is this a target" query, so that residual is D8's (a mesh111
    /// `Dialer::is_target` seam). The membership marker ([`Rehomed`]) is
    /// read before the fabric's settings, so a forget-and-re-enrol between
    /// the two reads leaves an old marker (the override voids on a lag)
    /// rather than a new one over old settings. One bounded wait on the
    /// fabric's events, no poll. The fabric keeps its own copy of the old
    /// address until the next start (it has no setter); the daemon persists
    /// and shows the new one over it.
    pub async fn replace_home_hub_address(
        &self,
        address: SocketAddr,
        wait: Duration,
    ) -> Result<bool, FabricError> {
        let l = self.live.as_ref().ok_or(FabricError::Stopped)?;
        let tcp = l.tcp.as_ref().ok_or(FabricError::NoLink)?;
        if let Some(why) = settings::dial_refusal(&address, self.cfg.tcp.refuse_loopback) {
            return Err(FabricError::Invalid(why));
        }
        let _one = self.rehome.lock().await;
        let marked = match l.fabric.settings().await?.home_hub {
            Some(h) => (h.node_id, self.hub_marker(h.node_id).await.ok().flatten()),
            None => return Err(FabricError::Invalid("this node has no home hub")),
        };
        let fabric_hub = l
            .fabric
            .settings()
            .await?
            .home_hub
            .ok_or(FabricError::Invalid("this node has no home hub"))?;
        let p = self.persisted();
        let shown = p
            .fabric
            .home_hub
            .filter(|h| h.node_id == fabric_hub.node_id)
            .unwrap_or(fabric_hub);
        if shown.address == Some(address) {
            return Ok(true);
        }
        if p.peers.iter().any(|x| x.address == address)
            || p.pairs.iter().any(|x| x.address == Some(address))
        {
            return Err(FabricError::Invalid(
                "that address is a configured peer's or a paired hub's",
            ));
        }
        if mesh111::link::Link::peers(tcp).contains(&PeerAddr::Tcp(address)) {
            return Err(FabricError::Invalid(
                "a connection at that address is up already (another dial's)",
            ));
        }
        let mut events = l.fabric.subscribe();
        Dialer::maintain(tcp, address, Some(fabric_hub.node_id));
        if !connected_at(tcp, &mut events, address, wait).await {
            self.release_trial(address, fabric_hub.address).await;
            return Ok(false);
        }
        if let Err(e) = l.fabric.dial_peer(address, fabric_hub.node_id).await {
            self.release_trial(address, fabric_hub.address).await;
            return Err(e);
        }
        let one = self.persist.lock().await;
        // M5c re-review: during the wait no override existed to catch the
        // events of a forget-and-re-enrol, so the hub and its marker are
        // read again before anything is recorded (under `persist`, which
        // the pump's settings path holds too).
        let still = match l.fabric.settings().await {
            Ok(s) => s.home_hub,
            Err(e) => {
                drop(one);
                self.release_trial(address, fabric_hub.address).await;
                return Err(e);
            }
        };
        if still.is_some_and(|h| h.node_id == fabric_hub.node_id && h.address == Some(address)) {
            // the fabric's own copy moved here meanwhile: nothing to override
            drop(one);
            return Ok(true);
        }
        let forgotten = marked.1.is_some()
            && matches!(self.hub_marker(fabric_hub.node_id).await, Ok(m) if m != marked.1);
        if still != Some(fabric_hub) || forgotten {
            drop(one);
            self.release_trial(address, fabric_hub.address).await;
            return Ok(false);
        }
        if let Ok(mut r) = self.rehomed.lock() {
            *r = (fabric_hub.address != Some(address)).then_some(Rehomed {
                hub: fabric_hub.node_id,
                from: fabric_hub.address,
                to: address,
                marker: marked.1.filter(|_| marked.0 == fabric_hub.node_id),
                void: false,
            });
        }
        let (snapshot, bytes) = match self.persisted.lock() {
            Ok(mut p) => {
                if let Some(h) = p
                    .fabric
                    .home_hub
                    .as_mut()
                    .filter(|h| h.node_id == fabric_hub.node_id)
                {
                    h.address = Some(address);
                }
                (p.clone(), settings::encoded(&p))
            }
            Err(_) => return Ok(true),
        };
        self.write_settings(&bytes);
        if let Some(old) = shown.address {
            if self.releasable(&snapshot, old) {
                Dialer::release(tcp, old);
            }
        }
        drop(one);
        Ok(true)
    }

    /// Release the §7.7 trial dial at `address` (M5c S3) unless, now, the
    /// shown home hub, a configured peer, a paired hub or a pair dial uses
    /// it, or the fabric's own home-hub copy moved to it during the trial
    /// (`before`: that copy's address when the trial started; a
    /// re-enrolment there): then the trial's `maintain` was a no-op on
    /// that holder's target, or the holder came meanwhile, and a release
    /// would take its dial. One settings read, only on a failed trial.
    async fn release_trial(&self, address: SocketAddr, before: Option<SocketAddr>) {
        let Some(tcp) = self.tcp() else { return };
        let fabric_copy = match self.fabric() {
            Some(f) => match f.settings().await {
                Ok(s) => s.home_hub.and_then(|h| h.address),
                // the fabric stopped: its dials went with it
                Err(_) => None,
            },
            None => None,
        };
        if (fabric_copy == Some(address) && before != Some(address))
            || self.pair_dials.lock().await.values().any(|a| *a == address)
        {
            return;
        }
        let one = self.persist.lock().await;
        if self.releasable(&self.persisted(), address) {
            Dialer::release(tcp, address);
        }
        drop(one);
    }

    /// Re-mint the own announce for the current settings (§7.3 A8: roles,
    /// home-hub hint; Layer 7 §7.2: role bits 3–5 — `ensure_announce` keeps
    /// the stored one when its `appdata` already matches; a new one is
    /// counter-committed first, §6.2) and give it to `tcp-lan` for new
    /// connections. The `appdata` is computed under the re-mint lock from
    /// the persisted fabric settings and the relay switches as they are
    /// then, so two changes racing settle on the latest and an unchanged
    /// state never re-mints. Nothing without a link (no announce is
    /// presented while restore-pending).
    ///
    /// Reports its outcome (Layer 7 row D4b): `Ok` when the presented
    /// announce carries the current `appdata` (kept or minted), or when no
    /// announce is presented at all (no link, or restore-pending: the next
    /// one is minted from the then-current state); `Err` when the re-mint
    /// failed and the previous announce (or none) is still what peers see.
    async fn refresh_announce(&self) -> Result<(), ReannounceError> {
        let _one = self.remint.lock().await;
        self.refresh_announce_locked().await
    }

    /// [`FabricHandle::refresh_announce`] with the re-mint lock already held
    /// by the caller.
    async fn refresh_announce_locked(&self) -> Result<(), ReannounceError> {
        let Some(l) = &self.live else { return Ok(()) };
        let Some(tcp) = &l.tcp else { return Ok(()) };
        let app = self.current_appdata();
        let (handle, dir) = (Arc::clone(&l.sealer), self.cfg.fabric_dir.clone());
        let outcome = tokio::task::spawn_blocking(move || handle.ensure_announce(&dir, &app)).await;
        let result = match outcome {
            Ok(AnnounceOutcome::Missing) if l.sealer.restore_pending() => return Ok(()),
            Ok(AnnounceOutcome::Missing) => Err(ReannounceError::NoCounter),
            Ok(o) => {
                let bytes = l.sealer.own_announce();
                if !bytes.is_empty() {
                    tcp.set_announce(bytes);
                }
                match o {
                    AnnounceOutcome::Stale => Err(ReannounceError::NoCounter),
                    _ => Ok(()),
                }
            }
            Err(e) => Err(ReannounceError::Task(e.to_string())),
        };
        if let Err(e) = &result {
            eprintln!("111 fabric: announce re-mint: {e}");
        }
        result
    }

    /// The `appdata` the current state calls for (the persisted fabric
    /// settings, the relay switches, LAN sharing).
    fn current_appdata(&self) -> Vec<u8> {
        let s = self.persisted().fabric;
        let relay = self.relay_roles();
        sealer::appdata(
            roles(&s, &relay, self.lan),
            s.home_hub.as_ref().map(|h| &h.node_id),
        )
    }

    /// The relay switches behind role bits 3–5 as last set (Layer 7 §7.2).
    pub fn relay_roles(&self) -> RelayRoles {
        *self.relay.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Hand the relay settings just written to `state/relay.json` to the
    /// fabric (Layer 7 §7.2, §7.5; the relay settings route, D10, calls it
    /// after every successful write): the role bits 3–5 and the offer's
    /// gateway fact follow them. The own announce is re-minted once when the
    /// role bits changed (one counter value, fsynced before the announce
    /// exists) and not at all when they did not.
    ///
    /// `Ok(true)`: the bits changed and the announce carries them (or no
    /// announce is presented now and the next one will); `Ok(false)`: the
    /// bits did not change, nothing re-minted. `Err`: the bits changed but
    /// the re-mint failed ([`ReannounceError`]): the previous switches are
    /// put back (the offer's gateway fact with them), so the peers' view and
    /// this handle agree, and the same call with the same settings tries
    /// again (row D4b). The settings file keeps what was written; the
    /// caller tells the owner the bits are not announced yet.
    pub async fn set_relay_settings(
        &self,
        r: &relay_settings::RelaySettings,
    ) -> Result<bool, ReannounceError> {
        self.set_relay_roles(RelayRoles::from_settings(r, crate::config::on_termux()))
            .await
    }

    /// [`FabricHandle::set_relay_settings`] with the switches already read.
    ///
    /// The swap, the compare, the re-mint and the revert all run under the
    /// re-mint lock (row D4c), so two calls serialise: a second call never
    /// answers `Ok(false)` while the first's re-mint is still in flight and
    /// may yet fail and put the old switches back. "Changed" is decided
    /// against the role bits the presented announce carries (what peers
    /// see), not against the switches the handle held: after a failed
    /// re-mint the same call finds the bits unannounced and tries again.
    /// Without a presented announce (no link, restore-pending) the switches
    /// held before are the reference.
    pub async fn set_relay_roles(&self, new: RelayRoles) -> Result<bool, ReannounceError> {
        let _one = self.remint.lock().await;
        let before = std::mem::replace(
            &mut *self.relay.lock().unwrap_or_else(|p| p.into_inner()),
            new,
        );
        let s = self.persisted().fabric;
        let want = roles(&s, &new, self.lan);
        let reference = self
            .presented_roles()
            .unwrap_or_else(|| roles(&s, &before, self.lan));
        if reference == want {
            return Ok(false);
        }
        if let Err(e) = self.refresh_announce_locked().await {
            // put the previous switches back (no other call ran meanwhile:
            // the re-mint lock is held), so the peers' view and this handle agree
            *self.relay.lock().unwrap_or_else(|p| p.into_inner()) = before;
            return Err(e);
        }
        Ok(true)
    }

    /// The role bits of the announce `tcp-lan` presents now; `None` when
    /// none is presented (no link) or it cannot be read.
    fn presented_roles(&self) -> Option<u16> {
        self.tcp()?;
        let bytes = self.own_announce();
        let a = id111::announce::Announce::parse(&bytes).ok()?;
        Some(mesh111::contacts::AppData::parse(a.appdata()).roles)
    }

    /// The hub offer's `gateway {carrier, mode}` (Layer 7 §7.5): `Some`
    /// while the carrier is on and this node holds the hub role, else
    /// `None` (the offer then carries no `gateway` key).
    pub fn gateway_offer(&self) -> Option<GatewayOffer> {
        self.relay_roles().gateway(&self.persisted().fabric)
    }

    /// Persist and apply the fabric's current settings now — the enroll
    /// route calls it before answering, so `home_hub` is on disk when the
    /// member reports itself enrolled (the `Settings` event writes the same
    /// values again).
    pub async fn sync_settings(&self) -> Result<FabricSettings, FabricError> {
        let f = self.fabric().ok_or(FabricError::Stopped)?;
        let s = f.settings().await?;
        Ok(self.on_settings(s).await)
    }

    /// Apply a settings patch now (the route answers after the listener
    /// reflects it; the `Settings` event persists the same values again).
    pub async fn patch_settings(
        &self,
        patch: mesh111::fabric::SettingsPatch,
    ) -> Result<FabricSettings, FabricError> {
        let f = self.fabric().ok_or(FabricError::Stopped)?;
        let s = f.patch_settings(patch).await?;
        Ok(self.on_settings(s).await)
    }

    async fn contact_frame(&self, id: NodeId, change: ContactChange) -> Option<Value> {
        let key = id.to_bytes();
        let fetched = match change {
            ContactChange::Removed => None,
            _ => match self.fabric()?.contact(id).await {
                Ok(v) => Some(json::contact(&v)),
                Err(FabricError::NotFound) => None,
                Err(e) => {
                    eprintln!("111 fabric: contact event: {e}");
                    return None;
                }
            },
        };
        let mut cache = self.contacts.lock().ok()?;
        Some(match fetched {
            Some(v) => {
                cache.insert(key, v.clone());
                let c = match change {
                    ContactChange::Added => "added",
                    ContactChange::Request => "request",
                    _ => "updated",
                };
                json!({ "change": c, "contact": v })
            }
            None => {
                let was = cache
                    .remove(&key)
                    .unwrap_or_else(|| json::contact_stub(&id));
                json!({ "change": "removed", "contact": was })
            }
        })
    }

    async fn message_frame(&self, id: i64, change: MessageChange) -> Option<Value> {
        if matches!(change, MessageChange::New | MessageChange::Request) {
            if let Ok(mut t) = self.times.lock() {
                t.record(id, times::now());
            }
        }
        let v = match self.fabric()?.message(id).await {
            Ok(v) => v,
            Err(FabricError::NotFound) => return None,
            Err(e) => {
                eprintln!("111 fabric: message event: {e}");
                return None;
            }
        };
        if !json::shown(&v) {
            return None;
        }
        let c = match change {
            MessageChange::New => "new",
            MessageChange::State => "state",
            MessageChange::Request => "request",
        };
        Some(json!({
            "change": c,
            "peer": json::hex(&v.peer),
            "message": self.message_json(&v),
        }))
    }

    async fn publish_fabric(&self, api: &ApiState) {
        match self.status_json().await {
            Ok(v) => {
                api.events.publish("fabric", &v);
            }
            Err(e) => eprintln!("111 fabric: status: {e}"),
        }
    }

    /// `network_message` frames for coalesced foreign events (module doc,
    /// step 6): one conversation read per distinct conversation gives the
    /// network and the highest `seq`; a conversation that is gone is
    /// skipped. A per-row `new` records the row's display time now.
    ///
    /// When that read fails for another reason (a store fault, a stopped
    /// actor), the conversation's row frames cannot be built; instead ONE
    /// `conversation` frame with `seq` 0 and the network
    /// [`UNKNOWN_NETWORK`] is published for it, so the client refetches the
    /// conversation rather than missing its rows (B1-bounce F5).
    async fn foreign_frames(&self, items: Vec<ForeignItem>) -> Vec<Value> {
        let Some(f) = self.fabric() else {
            return Vec::new();
        };
        let mut convs: HashMap<i64, ConvFrame> = HashMap::new();
        let mut out = Vec::with_capacity(items.len());
        for (conv, id, change) in items {
            if let std::collections::hash_map::Entry::Vacant(v) = convs.entry(conv) {
                let facts = match f.foreign_conversation(conv).await {
                    Ok(c) => ConvFrame::Facts(c.network, c.last_seq),
                    Err(FabricError::NotFound) => ConvFrame::Gone,
                    Err(e) => {
                        eprintln!("111 fabric: network_message event: {e}");
                        out.push(json::network_message(
                            UNKNOWN_NETWORK,
                            conv,
                            None,
                            ForeignChange::Conversation,
                            0,
                        ));
                        ConvFrame::Gone
                    }
                };
                v.insert(facts);
            }
            let Some(ConvFrame::Facts(network, seq)) = convs.get(&conv) else {
                continue;
            };
            if let (ForeignChange::New, Some(id)) = (change, id) {
                self.record_foreign_times([id]);
            }
            out.push(json::network_message(network, conv, id, change, *seq));
        }
        out
    }

    /// One batch the pump took (module doc, step 6). After a lag (`lagged`:
    /// events were lost) the batch's own `Settings` events are dropped and
    /// the fabric's current settings re-read and applied instead
    /// ([`FabricHandle::sync_settings`]: a lost `Settings` event may have
    /// been an unpairing, whose half and dial must go), the §7.7 override
    /// is kept unless the hub's contact row shows the membership changed
    /// (M5c S1, [`Rehomed`]; the re-read settings void it when they show
    /// the hub at neither address), and one synthetic `Fabric` event is
    /// added: to the SSE clients and to the tap's consumers it means
    /// "re-read everything" (M5b F3).
    async fn pump_batch(&self, api: &ApiState, mut batch: Vec<FabricEvent>, lagged: bool) {
        if lagged {
            self.recheck_rehome_after_lag().await;
            batch.retain(|e| !matches!(e, FabricEvent::Settings(_)));
            if let Err(e) = self.sync_settings().await {
                eprintln!("111 fabric: settings not re-read after the lag: {e}");
            }
            batch.push(FabricEvent::Fabric);
        }
        self.publish(api, batch).await;
    }

    /// Publish one batch of fabric events (see the module doc, step 6).
    async fn publish(&self, api: &ApiState, batch: Vec<FabricEvent>) {
        let mut messages: BTreeMap<i64, MessageChange> = BTreeMap::new();
        let mut order: Vec<i64> = Vec::new();
        let mut contacts: Vec<(NodeId, ContactChange)> = Vec::new();
        let mut foreign = ForeignBatch::default();
        let mut fabric = false;
        // Layer 7 §7.7: a batch that may end the membership voids the
        // override before its settings are applied (`Rehomed`).
        let voided = self.void_rehome_on(&batch);
        // The batch's (last) settings are applied before the tap sees any of
        // its events, so a consumer reading `pair_table()` or `persisted()`
        // on this batch's events reads them applied (M5b F7b).
        let mut new_settings = batch.iter().rev().find_map(|ev| match ev {
            FabricEvent::Settings(s) => Some(s.clone()),
            _ => None,
        });
        if voided && new_settings.is_none() {
            // the voided address leaves the file and its dial now, not at
            // some later settings change (one actor call, only then)
            if let Some(f) = self.fabric() {
                new_settings = f.settings().await.ok();
            }
        }
        if let Some(s) = new_settings {
            self.on_settings(s).await;
            fabric = true;
        }
        for ev in batch {
            // Layer 7 (§2.1, §3.5, §5.9): the one tap; each consumer returns
            // at once while it is off (module doc, "Layer 7 seams").
            if matches!(
                ev,
                FabricEvent::Message { .. }
                    | FabricEvent::Contact { .. }
                    | FabricEvent::Fabric
                    | FabricEvent::Forward { .. }
            ) {
                crate::daemon::relay::tap(&ev);
            }
            match ev {
                FabricEvent::Foreign { conv, id, change } => foreign.push(conv, id, change),
                FabricEvent::Message { id, change, .. } => {
                    if let std::collections::btree_map::Entry::Vacant(e) = messages.entry(id) {
                        e.insert(change);
                        order.push(id);
                    }
                }
                FabricEvent::Contact { node_id, change } => {
                    contacts.retain(|(n, _)| *n != node_id);
                    contacts.push((node_id, change));
                }
                FabricEvent::Fabric => fabric = true,
                // applied above
                FabricEvent::Settings(_) => {}
                // Layer 7 §5.9: the carrier's trigger, handed to the tap
                // above (D11); no SSE frame of its own.
                FabricEvent::Forward { .. } => {}
            }
        }
        for (id, change) in contacts {
            if let Some(v) = self.contact_frame(id, change).await {
                api.events.publish("contact", &v);
            }
        }
        for id in order {
            if let Some(v) = self.message_frame(id, messages[&id]).await {
                api.events.publish("message", &v);
            }
        }
        let frames = self.foreign_frames(foreign.items).await;
        if !frames.is_empty() {
            // Layer 6 (FB1 item 6): the bridge's next tick re-publishes its snapshot when
            // a network's counts changed with these rows
            if let Some(b) = api.ext::<Arc<crate::daemon::bridge::Bridge>>() {
                b.note_counts_changed();
            }
        }
        for v in frames {
            api.events.publish("network_message", &v);
        }
        if fabric {
            self.publish_fabric(api).await;
        }
    }

    /// One capacity pass (§10.4) with the filesystem's figures; a change of
    /// `store_full` is published as a `fabric` frame.
    pub async fn capacity_pass(&self, api: &ApiState) {
        let Some(f) = self.fabric() else { return };
        let disk = disk_space(&self.cfg.fabric_dir).unwrap_or(DiskSpace::UNKNOWN);
        let r = f
            .with_store(move |s| {
                let before = s.store_full();
                let r = s.prune(disk).map(|_| ());
                (before, s.store_full(), r)
            })
            .await;
        match r {
            Ok((before, after, Ok(()))) => {
                if before != after {
                    self.publish_fabric(api).await;
                }
            }
            Ok((_, _, Err(e))) => eprintln!("111 fabric: capacity pass: {e}"),
            Err(e) => eprintln!("111 fabric: capacity pass: {e}"),
        }
    }

    /// Clean stop (module doc).
    pub async fn shutdown(&self) {
        if let Ok(mut t) = self.tasks.lock() {
            for h in t.drain(..) {
                h.abort();
            }
        }
        let Some(l) = &self.live else { return };
        if let Some(t) = &l.tcp {
            t.shutdown();
        }
        match tokio::time::timeout(self.cfg.shutdown_grace, l.fabric.shutdown()).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                eprintln!("111 fabric: clean close failed ({e}); stopping");
                l.fabric.abort();
            }
            Err(_) => {
                eprintln!("111 fabric: clean close timed out; stopping");
                l.fabric.abort();
            }
        }
        // A no-op while restore-pending: no counters file is written then.
        if let Err(e) = l.sealer.shutdown_counters() {
            eprintln!("111 fabric: identity counters not closed cleanly: {e}");
        }
    }
}

/// A foreign event's key and first change: `(conv, id, change)`.
type ForeignItem = (i64, Option<i64>, ForeignChange);

/// The `network` of a refetch frame whose conversation could not be read
/// (B1-bounce F5): the contract's open-vocabulary fallback (`NetworkId`: an
/// unknown id renders as `unknown`); the frame carries no `id` and `seq` 0,
/// and the client's refetch of the conversation names the real network.
const UNKNOWN_NETWORK: &str = "unknown";

/// What one batch learned about a conversation for its frames.
enum ConvFrame {
    /// Its network and highest `seq`.
    Facts(String, u64),
    /// Nothing more to publish: gone, or a refetch frame already pushed.
    Gone,
}

/// Foreign events of one SSE batch, coalesced again per `(conv, id)` key
/// (the first change wins, as for messages), in first-seen order.
#[derive(Debug, Default)]
struct ForeignBatch {
    seen: HashSet<(i64, Option<i64>)>,
    items: Vec<ForeignItem>,
}

impl ForeignBatch {
    fn push(&mut self, conv: i64, id: Option<i64>, change: ForeignChange) {
        if self.seen.insert((conv, id)) {
            self.items.push((conv, id, change));
        }
    }
}

/// Wait at most `wait` (monotonic) for a `tcp-lan` connection at `address`
/// to pass the adapter's handshake (Layer 7 §7.7), looking again on every
/// fabric event: a connection that comes up is a `Fabric` event. No poll.
async fn connected_at(
    tcp: &TcpLan,
    events: &mut broadcast::Receiver<FabricEvent>,
    address: SocketAddr,
    wait: Duration,
) -> bool {
    use mesh111::link::Link;
    let up = || Link::peers(tcp).contains(&PeerAddr::Tcp(address));
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        if up() {
            return true;
        }
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => return up(),
        }
    }
}

/// Fabric events → SSE (module doc, step 6).
async fn pump(
    me: Weak<FabricHandle>,
    api: Weak<ApiState>,
    mut rx: broadcast::Receiver<FabricEvent>,
) {
    loop {
        let first = match rx.recv().await {
            Ok(e) => Some(e),
            Err(broadcast::error::RecvError::Lagged(n)) => {
                eprintln!("111 fabric: SSE pump lagged by {n} events; sending a snapshot");
                None
            }
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let mut batch: Vec<FabricEvent> = first.into_iter().collect();
        let mut lagged = batch.is_empty();
        while batch.len() < BATCH {
            match rx.try_recv() {
                Ok(e) => batch.push(e),
                Err(broadcast::error::TryRecvError::Lagged(_)) => lagged = true,
                Err(_) => break,
            }
        }
        let (Some(me), Some(api)) = (me.upgrade(), api.upgrade()) else {
            return;
        };
        me.pump_batch(&api, batch, lagged).await;
    }
}

/// Facts refresh + capacity passes (module doc, step 7).
async fn housekeeping(me: Weak<FabricHandle>, api: Weak<ApiState>) {
    let every = match me.upgrade() {
        Some(h) => h.cfg.housekeeping,
        None => return,
    };
    let mut tick = tokio::time::interval(every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut n: u64 = 0;
    loop {
        tick.tick().await;
        let (Some(h), Some(api)) = (me.upgrade(), api.upgrade()) else {
            return;
        };
        if n.is_multiple_of(PRUNE_EVERY) {
            h.capacity_pass(&api).await;
        }
        h.refresh_facts().await;
        n = n.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_follow_settings() {
        use sealer::{ROLE_GATEWAY, ROLE_OFFLOAD, ROLE_ORIGIN};
        let off = RelayRoles::default();
        let mut s = FabricSettings::default();
        assert_eq!(roles(&s, &off, true), 0);
        s.custody_opt_in = true;
        assert_eq!(roles(&s, &off, true), ROLE_MAILBOX);
        s.hub_role = true;
        assert_eq!(roles(&s, &off, true), ROLE_MAILBOX | ROLE_ALWAYS_ON_HUB);

        // Layer 7 §7.2: bits 3–5, each only while what it promises holds
        let hub = ROLE_MAILBOX | ROLE_ALWAYS_ON_HUB;
        let carrier = Some(GatewayOffer {
            carrier: CarrierKind::Ntfy,
            mode: CarrierMode::FiveA,
        });
        let all = RelayRoles {
            gateway_bit: true,
            offload_bit: true,
            origin_serve: true,
            carrier,
        };
        assert_eq!(
            roles(&s, &all, true),
            hub | ROLE_GATEWAY | ROLE_OFFLOAD | ROLE_ORIGIN
        );
        // the public bit without the gateway on promises nothing
        let no_carrier = RelayRoles {
            carrier: None,
            ..all
        };
        assert_eq!(
            roles(&s, &no_carrier, true),
            hub | ROLE_OFFLOAD | ROLE_ORIGIN
        );
        // the gateway on without the public bit: no bit (members learn it
        // from the offer only, §7.5)
        let quiet = RelayRoles {
            gateway_bit: false,
            ..all
        };
        assert_eq!(roles(&s, &quiet, true), hub | ROLE_OFFLOAD | ROLE_ORIGIN);
        // the origin is served only with LAN sharing (§9.1)
        assert_eq!(roles(&s, &all, false), hub | ROLE_GATEWAY | ROLE_OFFLOAD);
        // without the hub role: no gateway, no origin; offload stays
        s.hub_role = false;
        assert_eq!(roles(&s, &all, true), ROLE_MAILBOX | ROLE_OFFLOAD);
        s.custody_opt_in = false;
        assert_eq!(roles(&s, &all, true), ROLE_OFFLOAD);
        assert_eq!(roles(&s, &off, true), 0);
    }

    #[test]
    fn relay_roles_read_the_switches_fail_closed() {
        use relay_settings::RelaySettings;
        const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let mut r = RelaySettings::default();
        assert_eq!(RelayRoles::from_settings(&r, false), RelayRoles::default());
        // a switch without its consent counts as off
        r.set_public_gateway_bit(true);
        r.set_origin_serve(true);
        r.set_offload_bit(true);
        let x = RelayRoles::from_settings(&r, false);
        assert!(!x.gateway_bit && !x.origin_serve && x.offload_bit);
        r.set_consent_hash(ConsentKind::GatewayBit, Some(HASH))
            .unwrap();
        r.set_consent_hash(ConsentKind::OriginServe, Some(HASH))
            .unwrap();
        let x = RelayRoles::from_settings(&r, false);
        assert!(x.gateway_bit && x.origin_serve && x.carrier.is_none());
        // the carrier: on, a kind, its server and its consent
        r.set_gateway_enabled(true);
        r.set_carrier_kind(CarrierKind::Ntfy);
        assert!(RelayRoles::from_settings(&r, false).carrier.is_none());
        r.set_ntfy_server(Some("https://ntfy.example.org")).unwrap();
        assert!(RelayRoles::from_settings(&r, false).carrier.is_none());
        r.set_consent_hash(ConsentKind::CarrierEnable, Some(HASH))
            .unwrap();
        r.set_carrier_mode(CarrierMode::FiveB, false).unwrap();
        let g = RelayRoles::from_settings(&r, false).carrier.unwrap();
        assert_eq!((g.carrier, g.mode), (CarrierKind::Ntfy, CarrierMode::FiveB));
        assert_eq!(g.json(), json!({"carrier": "ntfy", "mode": "5B"}));
        // 5B runs as 5A on Termux (§3.1)
        let g = RelayRoles::from_settings(&r, true).carrier.unwrap();
        assert_eq!(g.mode, CarrierMode::FiveA);
        r.set_gateway_enabled(false);
        assert!(RelayRoles::from_settings(&r, false).carrier.is_none());
    }

    #[test]
    fn statvfs_reports_something_for_tmp() {
        let d = disk_space(&std::env::temp_dir()).unwrap();
        assert!(d.total > 0 && d.free <= d.total);
        assert!(disk_space(Path::new("/definitely/not/here")).is_none());
    }

    #[test]
    fn foreign_batches_coalesce_per_key_and_keep_the_first_change() {
        use ForeignChange::{Conversation, New, State};
        let mut b = ForeignBatch::default();
        b.push(1, None, New);
        b.push(2, Some(9), New);
        b.push(1, None, Conversation);
        b.push(2, Some(9), State);
        b.push(2, Some(10), State);
        b.push(2, None, Conversation);
        assert_eq!(
            b.items,
            vec![
                (1, None, New),
                (2, Some(9), New),
                (2, Some(10), State),
                (2, None, Conversation)
            ]
        );
    }

    fn test_api(dir: &Path) -> Arc<ApiState> {
        test_api_lan(dir, false)
    }

    fn test_api_lan(dir: &Path, lan: bool) -> Arc<ApiState> {
        use crate::daemon::dashboard::api_v1::ApiConfig;
        use crate::daemon::dashboard::DashboardState;
        std::fs::create_dir_all(dir.join("state")).unwrap();
        let layout = DaemonLayout {
            root: dir.to_path_buf(),
            etc: dir.join("etc"),
        };
        let mut cfg = ApiConfig::new(layout, lan);
        cfg.persist_auth = false;
        ApiState::new(Arc::new(RwLock::new(DashboardState::new("t"))), cfg)
    }

    fn test_cfg(dir: &Path) -> FabricConfig {
        FabricConfig {
            fabric_dir: dir.join(FABRIC_DIR),
            identity_dir: dir.join("identity"),
            state_dir: dir.join("state"),
            listen: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            tcp: TcpTuning::default(),
            hub_default: false,
            enroll_timeout: Duration::from_secs(5),
            housekeeping: Duration::from_secs(3600),
            shutdown_grace: Duration::from_secs(5),
            offer_host: None,
        }
    }

    /// The `network_message` frames published since `head`.
    fn frames(api: &ApiState, head: u64) -> Vec<Value> {
        api.events
            .subscribe(Some(head))
            .replay
            .iter()
            .filter(|f| f.event == "network_message")
            .map(|f| serde_json::from_str(&f.data).unwrap())
            .collect()
    }

    async fn until_frames(api: &ApiState, head: u64, n: usize) -> Vec<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let got = frames(api, head);
            if got.len() >= n {
                return got;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{} of {n} frames: {got:?}",
                got.len()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn inbound(conv: i64, i: u32) -> ForeignInsert {
        ForeignInsert {
            conv,
            sender_native: "!a1b2c3d4".into(),
            sender_name: Some("W1AW".into()),
            auth: mesh111::store::ForeignAuth::ChannelKey,
            native_msg_id: Some(format!("{:08x}", 0x5000_0000 + i)),
            body: format!("text {i}"),
            claim_ts: None,
            meta: None,
            id_globally_unique: false,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn foreign_events_become_coalesced_network_message_frames() {
        use mesh111::store::ForeignKind;
        let dir = tempfile::tempdir().unwrap();
        let api = test_api(dir.path());
        let identity = crate::identity::generate(&dir.path().join("identity")).unwrap();
        let h = start(test_cfg(dir.path()), Some(identity), &api).await;
        assert!(h.running(), "{:?}", h.down_reason());
        let f = h.fabric().unwrap().clone();
        let head = api.events.subscribe(None).head;

        let ch = f
            .foreign_conversation_upsert(
                "meshtastic",
                "!0badc0de",
                ForeignKind::Channel,
                "ch:LongFast:9f3c",
                Some("LongFast"),
            )
            .await
            .unwrap();
        let dm = f
            .foreign_conversation_upsert(
                "meshtastic",
                "!0badc0de",
                ForeignKind::Dm,
                "dm:!a1b2c3d4",
                None,
            )
            .await
            .unwrap();
        let first = until_frames(&api, head, 2).await;
        assert_eq!(
            first,
            vec![
                json!({"network": "meshtastic", "conv": json::fc_id(ch), "change": "conversation", "seq": 0}),
                json!({"network": "meshtastic", "conv": json::fc_id(dm), "change": "conversation", "seq": 0}),
            ]
        );
        let head = api.events.subscribe(None).head;

        // 50 channel rows and 3 DM rows in one bounded call
        let mut batch: Vec<ForeignInsert> = (0..50).map(|i| inbound(ch, i)).collect();
        batch.extend((0..3).map(|i| inbound(dm, 100 + i)));
        let ids: Vec<i64> = h
            .foreign_ingest(batch)
            .await
            .unwrap()
            .into_iter()
            .map(Option::unwrap)
            .collect();
        assert_eq!(ids.len(), 53);
        let got = until_frames(&api, head, 4).await;
        let ch_seq = f.foreign_conversation(ch).await.unwrap().last_seq;
        let dm_seq = f.foreign_conversation(dm).await.unwrap().last_seq;
        let mut want = vec![json!({
            "network": "meshtastic", "conv": json::fc_id(ch), "change": "new", "seq": ch_seq,
        })];
        for id in &ids[50..] {
            want.push(json!({
                "network": "meshtastic", "conv": json::fc_id(dm), "id": json::fm_id(*id),
                "change": "new", "seq": dm_seq,
            }));
        }
        let mut got_sorted = got.clone();
        got_sorted.sort_by_key(|v| v.to_string());
        want.sort_by_key(|v| v.to_string());
        assert_eq!(
            got_sorted, want,
            "one frame for the channel, one per DM row"
        );

        // a state change of an own row: one per-row frame
        let head = api.events.subscribe(None).head;
        let part = ForeignGroupPart {
            conv: dm,
            text: "reply".into(),
            part_k: 1,
            part_n: 1,
        };
        let (_, own) = h
            .foreign_group_create("reply", "[]", vec![part])
            .await
            .unwrap();
        f.foreign_set_state(
            own[0],
            mesh111::store::ForeignState::Sent,
            Some("accepted by the radio"),
            None,
        )
        .await
        .unwrap();
        let got = until_frames(&api, head, 1).await;
        assert!(
            got.iter().all(|v| v["id"] == json::fm_id(own[0])),
            "{got:?}"
        );
        assert_eq!(
            got[0]["change"], "new",
            "the first change since the previous frame"
        );

        // display times: the foreign instance, beside the 111 one
        let at = h.foreign_at_of(ids[0]);
        assert!(at > 1_700_000_000);
        let file = dir.path().join(FABRIC_DIR).join(times::FOREIGN_TIMES_FILE);
        let len = std::fs::metadata(&file).unwrap().len();
        assert_eq!(len, 16 * 54, "53 received + 1 queued, each recorded once");
        let row = f.foreign_message(ids[0]).await.unwrap();
        let v = h.foreign_message_json(&row);
        assert_eq!(v["id"], json::fm_id(ids[0]));
        assert_eq!(v["at"], times::rfc3339(at));
        h.shutdown().await;
    }

    // ------------------------------------------------------------ Layer 7

    /// A loopback-friendly configuration (several nodes on one host).
    fn lo_cfg(dir: &Path, hub: bool) -> FabricConfig {
        let mut c = test_cfg(dir);
        c.hub_default = hub;
        c.tcp.refuse_loopback = false;
        c.tcp.backoff_min = Duration::from_millis(50);
        c.tcp.backoff_max = Duration::from_millis(400);
        c
    }

    async fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Layer 7 "never adds": with no pair, no pair dial, no connection and no
    /// `paired_hubs` key in `fabric/settings`; `Forward` events reach the tap
    /// and publish no frame.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nothing_is_dialled_or_written_without_a_pair() {
        let dir = tempfile::tempdir().unwrap();
        let api = test_api(dir.path());
        let identity = crate::identity::generate(&dir.path().join("identity")).unwrap();
        let h = start(lo_cfg(dir.path(), true), Some(identity), &api).await;
        assert!(h.running(), "{:?}", h.down_reason());
        assert!(h.pair_dials.lock().await.is_empty());
        assert!(h.pair_table().read().unwrap().is_empty());
        assert_eq!(h.tcp().unwrap().connection_count(), 0);
        h.patch_settings(mesh111::fabric::SettingsPatch {
            listen_lan: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        let file = std::fs::read_to_string(settings::path(&h.cfg.fabric_dir)).unwrap();
        assert!(!file.contains("paired_hubs"), "{file}");
        assert!(h.pair_dials.lock().await.is_empty());
        let head = api.events.subscribe(None).head;
        h.publish(
            &api,
            vec![FabricEvent::Forward {
                target: NodeId::from([9; 32]),
                pending: 1,
            }],
        )
        .await;
        assert!(api.events.subscribe(Some(head)).replay.is_empty());
        // a pairing half is refused for this node and without the hub role
        let own = h.sealer().unwrap().node_id();
        assert!(h.set_pair_link(settings::PairLink::new(own)).await.is_err());
        h.patch_settings(mesh111::fabric::SettingsPatch {
            hub_role: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
        let far = settings::PairLink::new(NodeId::from([9; 32]));
        assert!(matches!(
            h.set_pair_link(far).await,
            Err(FabricError::Invalid("pairing needs the hub role"))
        ));
        h.shutdown().await;
    }

    /// The own announce's role bits and counter (§7.2).
    fn announced(h: &FabricHandle) -> (u16, u64, Vec<u8>) {
        let b = h.own_announce();
        let a = id111::announce::Announce::parse(&b).unwrap();
        let roles = mesh111::contacts::AppData::parse(a.appdata()).roles;
        (roles, a.counter(), b)
    }

    /// Layer 7 §7.2 (row D9): role bits 3–5 follow `state/relay.json`; a
    /// change re-mints the announce once (exactly one announce counter
    /// value), the same switches never re-mint (a repeated set, an
    /// unrelated settings change, a restart), and the offer's gateway fact
    /// follows the carrier and the hub role (§7.5).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_role_bit_change_re_mints_the_announce_once() {
        use relay_settings::RelaySettings;
        use sealer::{ROLE_GATEWAY, ROLE_OFFLOAD, ROLE_ORIGIN};
        const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let dir = tempfile::tempdir().unwrap();
        let api = test_api_lan(dir.path(), true);
        let state = dir.path().join("state");
        // the switches written while the daemon is down: offload and origin
        let (mut r, wrote) = relay_settings::update(&state, |r: &mut RelaySettings| {
            r.set_offload_bit(true);
            r.set_origin_serve(true);
            r.set_consent_hash(ConsentKind::OriginServe, Some(HASH))
        })
        .unwrap();
        assert!(wrote);
        let identity = crate::identity::generate(&dir.path().join("identity")).unwrap();
        let h = start(lo_cfg(dir.path(), true), Some(identity), &api).await;
        assert!(h.running(), "{:?}", h.down_reason());
        let hub = ROLE_MAILBOX | ROLE_ALWAYS_ON_HUB;
        let (roles0, n0, bytes0) = announced(&h);
        assert_eq!(roles0, hub | ROLE_OFFLOAD | ROLE_ORIGIN);
        assert_eq!(h.gateway_offer(), None);
        let last = |h: &FabricHandle| h.sealer().unwrap().announce_last();
        // the announce counter at start (the epoch mint may take later
        // values of the same counter, so `l0` ≥ `n0`)
        let l0 = last(&h);
        assert!(n0 <= l0);

        // the same switches again: nothing re-minted
        assert!(!h.set_relay_settings(&r).await.unwrap());
        assert_eq!(last(&h), l0);
        assert_eq!(announced(&h).2, bytes0);
        // a switch whose bit is gated off changes no bit: the gateway bit
        // consented while the gateway is off
        r.set_public_gateway_bit(true);
        r.set_consent_hash(ConsentKind::GatewayBit, Some(HASH))
            .unwrap();
        assert!(!h.set_relay_settings(&r).await.unwrap());
        assert_eq!(last(&h), l0);
        // an unrelated fabric settings change: nothing re-minted
        h.patch_settings(mesh111::fabric::SettingsPatch {
            listen_lan: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(last(&h), l0);
        assert_eq!(announced(&h).2, bytes0);

        // the gateway on with its carrier: bit 3 and the offer fact, one
        // re-mint
        r.set_gateway_enabled(true);
        r.set_carrier_kind(CarrierKind::Ntfy);
        r.set_ntfy_server(Some("https://ntfy.example.org")).unwrap();
        r.set_consent_hash(ConsentKind::CarrierEnable, Some(HASH))
            .unwrap();
        assert!(h.set_relay_settings(&r).await.unwrap());
        let (roles1, n1, bytes1) = announced(&h);
        assert_eq!(roles1, hub | ROLE_GATEWAY | ROLE_OFFLOAD | ROLE_ORIGIN);
        assert_eq!(last(&h), l0 + 1);
        assert_eq!(n1, l0 + 1);
        assert_eq!(
            h.gateway_offer(),
            Some(GatewayOffer {
                carrier: CarrierKind::Ntfy,
                mode: CarrierMode::FiveA
            })
        );
        // the mode alone changes no bit: no re-mint, the offer follows
        r.set_carrier_mode(CarrierMode::FiveB, false).unwrap();
        let termux = crate::config::on_termux();
        assert!(!h.set_relay_settings(&r).await.unwrap());
        assert_eq!(last(&h), l0 + 1);
        let want = if termux {
            CarrierMode::FiveA
        } else {
            CarrierMode::FiveB
        };
        assert_eq!(h.gateway_offer().map(|g| g.mode), Some(want));

        // offload off: one more re-mint, one counter value
        r.set_offload_bit(false);
        assert!(h.set_relay_settings(&r).await.unwrap());
        assert_eq!(last(&h), l0 + 2);
        assert_eq!(announced(&h).0, hub | ROLE_GATEWAY | ROLE_ORIGIN);

        // the hub role off takes bits 1, 3, 5 and the offer fact with it
        // (bit 0 stays only with custody opt-in): one re-mint
        h.patch_settings(mesh111::fabric::SettingsPatch {
            hub_role: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(announced(&h).0, 0);
        assert_eq!(last(&h), l0 + 3);
        assert_eq!(h.gateway_offer(), None);
        h.patch_settings(mesh111::fabric::SettingsPatch {
            hub_role: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        let (roles2, n2, bytes2) = announced(&h);
        assert_eq!(roles2, hub | ROLE_GATEWAY | ROLE_ORIGIN);
        assert_eq!(n2, l0 + 4);
        assert_ne!(bytes2, bytes1);
        relay_settings::save(&state, &r).unwrap();
        h.shutdown().await;
        drop(h);

        // a restart with the same switches keeps the stored announce
        let identity = crate::identity::load(&dir.path().join("identity")).unwrap();
        let h = start(lo_cfg(dir.path(), true), Some(identity), &api).await;
        assert!(h.running(), "{:?}", h.down_reason());
        assert_eq!(announced(&h).2, bytes2);
        h.shutdown().await;
    }

    /// Layer 7 §7.2 (row D4b): a re-mint that fails (no announce counter can
    /// be committed: the identity `counters` file cannot be written) reports
    /// it, puts the previous switches back and leaves the old announce; the
    /// same call with the same settings then mints once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_re_mint_is_reported_and_the_same_call_tries_again() {
        use relay_settings::RelaySettings;
        use sealer::ROLE_OFFLOAD;
        let dir = tempfile::tempdir().unwrap();
        let api = test_api_lan(dir.path(), true);
        let identity_dir = dir.path().join("identity");
        let identity = crate::identity::generate(&identity_dir).unwrap();
        let h = start(lo_cfg(dir.path(), true), Some(identity), &api).await;
        assert!(h.running(), "{:?}", h.down_reason());
        let hub = ROLE_MAILBOX | ROLE_ALWAYS_ON_HUB;
        let (roles0, _, bytes0) = announced(&h);
        assert_eq!(roles0, hub);
        let last = |h: &FabricHandle| h.sealer().unwrap().announce_last();
        let l0 = last(&h);

        // the counter commit fails once: a directory where its temp file goes
        // (EISDIR, also for root)
        let blocker = identity_dir.join(format!("{}.tmp", id111::counters::COUNTERS_FILE));
        std::fs::create_dir(&blocker).unwrap();
        std::fs::write(blocker.join("keep"), b"x").unwrap();
        let mut r = RelaySettings::default();
        r.set_offload_bit(true);
        assert_eq!(
            h.set_relay_settings(&r).await,
            Err(ReannounceError::NoCounter)
        );
        assert_eq!(last(&h), l0, "no counter value committed");
        assert_eq!(announced(&h).2, bytes0, "the old announce stays");
        assert_eq!(
            h.relay_roles(),
            RelayRoles::default(),
            "the switches put back"
        );

        // the disk is fixed: the same call tries again and mints once
        std::fs::remove_dir_all(&blocker).unwrap();
        assert_eq!(h.set_relay_settings(&r).await, Ok(true));
        assert_eq!(last(&h), l0 + 1);
        assert_eq!(announced(&h).0, hub | ROLE_OFFLOAD);
        assert!(h.relay_roles().offload_bit);
        // and once only
        assert_eq!(h.set_relay_settings(&r).await, Ok(false));
        assert_eq!(last(&h), l0 + 1);
        h.shutdown().await;
    }

    /// Layer 7 §7.2 (row D4c item 4): two overlapping role writes, the first
    /// failing. The second never answers `Ok(false)` ("already announced")
    /// while the first's re-mint is in flight and then reverts: the calls
    /// serialise under the re-mint lock and "changed" is decided against
    /// the presented announce, so the second tries (and here fails) too, and
    /// the handle and the announce agree after both.
    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn overlapping_role_writes_serialise_and_agree_with_the_announce() {
        use relay_settings::RelaySettings;
        use sealer::ROLE_OFFLOAD;
        let dir = tempfile::tempdir().unwrap();
        let api = test_api_lan(dir.path(), true);
        let identity_dir = dir.path().join("identity");
        let identity = crate::identity::generate(&identity_dir).unwrap();
        let h = start(lo_cfg(dir.path(), true), Some(identity), &api).await;
        assert!(h.running(), "{:?}", h.down_reason());
        let hub = ROLE_MAILBOX | ROLE_ALWAYS_ON_HUB;
        let last = |h: &FabricHandle| h.sealer().unwrap().announce_last();
        let l0 = last(&h);
        let blocker = identity_dir.join(format!("{}.tmp", id111::counters::COUNTERS_FILE));
        std::fs::create_dir(&blocker).unwrap();
        std::fs::write(blocker.join("keep"), b"x").unwrap();
        let mut r = RelaySettings::default();
        r.set_offload_bit(true);

        // a re-mint is in flight (the lock held), then the two calls arrive
        let held = h.remint.lock().await;
        let first = tokio::spawn({
            let (h, r) = (Arc::clone(&h), r.clone());
            async move { h.set_relay_settings(&r).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let second = tokio::spawn({
            let (h, r) = (Arc::clone(&h), r.clone());
            async move { h.set_relay_settings(&r).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!second.is_finished(), "the second call waits its turn");
        drop(held);
        let (a, b) = (first.await.unwrap(), second.await.unwrap());
        assert_eq!(a, Err(ReannounceError::NoCounter));
        assert_eq!(
            b,
            Err(ReannounceError::NoCounter),
            "never Ok(false) for bits that are not announced"
        );
        assert_eq!(last(&h), l0);
        assert_eq!(announced(&h).0, hub);
        assert_eq!(h.relay_roles(), RelayRoles::default(), "they agree");

        // fixed: one call mints, the next finds the bits announced
        std::fs::remove_dir_all(&blocker).unwrap();
        assert_eq!(h.set_relay_settings(&r).await, Ok(true));
        assert_eq!(h.set_relay_settings(&r).await, Ok(false));
        assert_eq!((announced(&h).0, last(&h)), (hub | ROLE_OFFLOAD, l0 + 1));
        h.shutdown().await;
    }

    /// Layer 7 §4.6 step 5: two paired hubs that hold each other's address
    /// keep one connection, dialled by the lower NodeId; an unpairing
    /// releases it and takes the row out of `fabric/settings`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_pair_is_dialled_once_by_the_lower_node_id() {
        use mesh111::fabric::{PairRole, PairVia, PairedHub};
        let dirs = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
        let ids: Vec<Identity> = dirs
            .iter()
            .map(|d| crate::identity::generate(&d.path().join("identity")).unwrap())
            .collect();
        let node: Vec<NodeId> = ids.iter().map(|i| i.node_id).collect();
        // each file records the other as a `tcp` peer pair (no address yet)
        for (i, d) in dirs.iter().enumerate() {
            let fabric_dir = d.path().join(FABRIC_DIR);
            std::fs::create_dir_all(&fabric_dir).unwrap();
            let p = settings::Persisted {
                fabric: FabricSettings {
                    hub_role: true,
                    paired_hubs: vec![PairedHub {
                        node_id: node[1 - i],
                        role: PairRole::Peer,
                        via: PairVia::Tcp,
                        ours: true,
                        theirs: true,
                    }],
                    ..FabricSettings::default()
                },
                ..settings::Persisted::default()
            };
            settings::save(&fabric_dir, &p).unwrap();
        }
        let apis: Vec<Arc<ApiState>> = dirs.iter().map(|d| test_api(d.path())).collect();
        let mut hs = Vec::new();
        for ((d, id), api) in dirs.iter().zip(ids).zip(&apis) {
            let h = start(lo_cfg(d.path(), true), Some(id), api).await;
            assert!(h.running(), "{:?}", h.down_reason());
            assert_eq!(h.persisted().fabric.paired_hubs.len(), 1);
            assert!(h.pair_dials.lock().await.is_empty(), "no address: no dial");
            hs.push(h);
        }
        let addrs = [
            hs[0].listen_addr().await.unwrap(),
            hs[1].listen_addr().await.unwrap(),
        ];
        // both bundles carried an address: each holds the other's
        for i in 0..2 {
            let link = settings::PairLink {
                address: Some(addrs[1 - i]),
                link_spki: None,
                // loopback is not a private range (§4.1): marked LAN so the
                // pair runs plain once D5's wrap reads the table
                lan: true,
                far_holds_address: true,
                ..settings::PairLink::new(node[1 - i])
            };
            hs[i].set_pair_link(link).await.unwrap();
            assert_eq!(hs[i].pair_table().read().unwrap().len(), 1);
        }
        let (lo, hi) = if node[0].as_bytes() < node[1].as_bytes() {
            (0, 1)
        } else {
            (1, 0)
        };
        assert_eq!(
            hs[lo].pair_dials.lock().await.get(&node[hi]),
            Some(&addrs[hi])
        );
        assert!(
            hs[hi].pair_dials.lock().await.is_empty(),
            "the higher waits"
        );
        until("the pair connected", || {
            (hs[0].tcp().unwrap().connection_count() == 1
                && hs[1].tcp().unwrap().connection_count() == 1)
                .then_some(())
        })
        .await;
        // the row is on disk with the daemon's half beside the record
        let file = std::fs::read_to_string(settings::path(&hs[lo].cfg.fabric_dir)).unwrap();
        let v: Value = serde_json::from_str(&file).unwrap();
        assert_eq!(v["paired_hubs"][0]["address"], addrs[hi].to_string());
        assert_eq!(v["paired_hubs"][0]["far_holds_address"], true);
        // still one connection each a moment later (no second dial)
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(hs[0].tcp().unwrap().connection_count(), 1);
        assert_eq!(hs[1].tcp().unwrap().connection_count(), 1);
        // unpairing drops the row, the half and the dial
        assert!(hs[lo].fabric().unwrap().unpair(node[hi]).await.unwrap());
        until("the dial released", || {
            let file = std::fs::read_to_string(settings::path(&hs[lo].cfg.fabric_dir)).ok()?;
            (!file.contains("paired_hubs")
                && hs[lo].tcp().unwrap().connection_count() == 0
                && hs[lo].pair_table().read().unwrap().is_empty())
            .then_some(())
        })
        .await;
        assert!(hs[lo].pair_dials.lock().await.is_empty());
        for h in hs {
            h.shutdown().await;
        }
    }

    /// Layer 7 §7.7: a new home-hub address is kept only after a handshake
    /// there named the hub; it survives the fabric's own settings events.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_home_hub_address_is_replaced_only_after_a_handshake_names_it() {
        let (hd, md, od) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let (ha, ma, oa) = (
            test_api(hd.path()),
            test_api(md.path()),
            test_api(od.path()),
        );
        let hid = crate::identity::generate(&hd.path().join("identity")).unwrap();
        let hub = start(lo_cfg(hd.path(), true), Some(hid), &ha).await;
        let oid = crate::identity::generate(&od.path().join("identity")).unwrap();
        let other = start(lo_cfg(od.path(), true), Some(oid), &oa).await;
        let mid = crate::identity::generate(&md.path().join("identity")).unwrap();
        let m = start(lo_cfg(md.path(), false), Some(mid), &ma).await;
        assert!(hub.running() && other.running() && m.running());
        let at = hub.listen_addr().await.unwrap();
        let offer = hub.fabric().unwrap().offer(Some(at)).await.unwrap();
        m.fabric()
            .unwrap()
            .enroll(at, &offer.code, Some(&offer.card))
            .await
            .unwrap();
        m.sync_settings().await.unwrap();
        let hub_id = hub.sealer().unwrap().node_id();
        assert_eq!(
            m.persisted()
                .fabric
                .home_hub
                .map(|h| (h.node_id, h.address)),
            Some((hub_id, Some(at)))
        );
        // the hub "moves": a relay on another port reaches it
        let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let moved = relay.local_addr().unwrap();
        let forward = tokio::spawn(async move {
            while let Ok((mut inbound, _)) = relay.accept().await {
                tokio::spawn(async move {
                    if let Ok(mut out) = tokio::net::TcpStream::connect(at).await {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut out).await;
                    }
                });
            }
        });
        // another node's listener never passes for the hub
        let wrong = other.listen_addr().await.unwrap();
        let r = m
            .replace_home_hub_address(wrong, Duration::from_millis(800))
            .await;
        assert!(matches!(r, Ok(false)), "{r:?}");
        assert_eq!(m.persisted().fabric.home_hub.unwrap().address, Some(at));
        // the hub's own new address is kept once its handshake named it
        let r = m
            .replace_home_hub_address(moved, Duration::from_secs(5))
            .await;
        assert!(matches!(r, Ok(true)), "{r:?}");
        assert_eq!(m.persisted().fabric.home_hub.unwrap().address, Some(moved));
        let st = m.status_json().await.unwrap();
        assert_eq!(st["hub"]["address"], moved.to_string());
        // a later settings event from the fabric (which still holds `at`)
        // keeps the new address on disk
        m.patch_settings(mesh111::fabric::SettingsPatch {
            listen_lan: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        let file = std::fs::read_to_string(settings::path(&m.cfg.fabric_dir)).unwrap();
        assert!(file.contains(&moved.to_string()), "{file}");
        assert!(!file.contains(&format!("\"{at}\"")), "{file}");
        until("the hub reachable at its new address", || {
            use mesh111::link::Link;
            Link::peers(m.tcp().unwrap())
                .contains(&PeerAddr::Tcp(moved))
                .then_some(())
        })
        .await;
        // refusals: no dial target; no home hub
        assert!(m
            .replace_home_hub_address("0.0.0.0:1".parse().unwrap(), Duration::from_millis(10))
            .await
            .is_err());
        assert!(matches!(
            other
                .replace_home_hub_address(moved, Duration::from_millis(10))
                .await,
            Err(FabricError::Invalid("this node has no home hub"))
        ));
        // M5b F4: an address a connection is up at already (another dial's,
        // here one expecting the other node) is refused before any trial
        let other_id = other.sealer().unwrap().node_id();
        Dialer::maintain(m.tcp().unwrap(), wrong, Some(other_id));
        until("a connection to the other node", || {
            use mesh111::link::Link;
            Link::peers(m.tcp().unwrap())
                .contains(&PeerAddr::Tcp(wrong))
                .then_some(())
        })
        .await;
        let r = m
            .replace_home_hub_address(wrong, Duration::from_secs(2))
            .await;
        assert!(matches!(r, Err(FabricError::Invalid(_))), "{r:?}");
        assert_eq!(m.persisted().fabric.home_hub.unwrap().address, Some(moved));
        Dialer::release(m.tcp().unwrap(), wrong);
        // M5b F5: a batch that shows the hub enrolled at again (forgotten
        // and re-enrolled at `at`, the intermediate settings never seen; the
        // batch need not carry a settings event) voids the override: the
        // fabric's `at` is kept, not `moved`
        let current = m.fabric().unwrap().settings().await.unwrap();
        assert_eq!(current.home_hub.unwrap().address, Some(at));
        m.publish(
            &ma,
            vec![FabricEvent::Contact {
                node_id: hub_id,
                change: ContactChange::Added,
            }],
        )
        .await;
        assert_eq!(m.persisted().fabric.home_hub.unwrap().address, Some(at));
        let st = m.status_json().await.unwrap();
        assert_eq!(st["hub"]["address"], at.to_string());
        let file = std::fs::read_to_string(settings::path(&m.cfg.fabric_dir)).unwrap();
        assert!(file.contains(&format!("\"{at}\"")), "{file}");
        assert!(m.rehomed.lock().unwrap().is_none());
        forward.abort();
        for h in [m, hub, other] {
            h.shutdown().await;
        }
    }

    /// M5b F2, F7a: a pair's half follows the fabric's current settings, in
    /// one order for the table and the file: an older `Settings` event that
    /// lacks the record drops nothing; concurrent changes end with the table,
    /// the memory and the file agreeing; an unpairing drops the half; an
    /// address another dial uses is refused.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pair_halves_follow_the_current_settings_in_one_order() {
        use mesh111::fabric::{PairRole, PairVia, PairedHub};
        let dir = tempfile::tempdir().unwrap();
        let api = test_api(dir.path());
        let identity = crate::identity::generate(&dir.path().join("identity")).unwrap();
        let far = NodeId::from([0xb0; 32]);
        let peer: SocketAddr = "127.0.0.1:10".parse().unwrap();
        let fabric_dir = dir.path().join(FABRIC_DIR);
        std::fs::create_dir_all(&fabric_dir).unwrap();
        let file = settings::Persisted {
            fabric: FabricSettings {
                hub_role: true,
                paired_hubs: vec![PairedHub {
                    node_id: far,
                    role: PairRole::Peer,
                    via: PairVia::Tcp,
                    ours: true,
                    theirs: true,
                }],
                ..FabricSettings::default()
            },
            peers: vec![settings::Peer {
                node_id: NodeId::from([0xc0; 32]),
                address: peer,
            }],
            ..settings::Persisted::default()
        };
        settings::save(&fabric_dir, &file).unwrap();
        let h = start(lo_cfg(dir.path(), true), Some(identity), &api).await;
        assert!(h.running(), "{:?}", h.down_reason());
        let at = |n: NodeId, a: &str| settings::PairLink {
            address: Some(a.parse().unwrap()),
            ..settings::PairLink::new(n)
        };
        // F7a: a configured peer's address, then another pair's
        let third = NodeId::from([0xd0; 32]);
        assert!(matches!(
            h.set_pair_link(at(third, "127.0.0.1:10")).await,
            Err(FabricError::Invalid(_))
        ));
        h.set_pair_link(at(far, "127.0.0.1:9")).await.unwrap();
        assert!(matches!(
            h.set_pair_link(at(third, "127.0.0.1:9")).await,
            Err(FabricError::Invalid(_))
        ));
        let on_disk = || -> Value {
            serde_json::from_str(&std::fs::read_to_string(settings::path(&fabric_dir)).unwrap())
                .unwrap()
        };
        assert_eq!(on_disk()["paired_hubs"][0]["address"], "127.0.0.1:9");
        // an older settings view without the record keeps the record and
        // the half (the current settings are re-read)
        let current = h.fabric().unwrap().settings().await.unwrap();
        let older = FabricSettings {
            paired_hubs: Vec::new(),
            ..current.clone()
        };
        let applied = h.on_settings(older.clone()).await;
        assert_eq!(applied.paired_hubs, current.paired_hubs);
        assert_eq!(h.persisted().fabric.paired_hubs, current.paired_hubs);
        assert_eq!(h.pair_table().read().unwrap().len(), 1);
        assert_eq!(on_disk()["paired_hubs"][0]["address"], "127.0.0.1:9");
        // concurrent half changes and stale settings: one order for all
        let h2 = Arc::clone(&h);
        let mut jobs = Vec::new();
        for i in 0..8u16 {
            let (h, older) = (Arc::clone(&h2), older.clone());
            jobs.push(tokio::spawn(async move {
                h.set_pair_link(at(far, &format!("127.0.0.1:{}", 20 + i)))
                    .await
                    .unwrap();
            }));
            let h = Arc::clone(&h2);
            jobs.push(tokio::spawn(async move {
                h.on_settings(older).await;
            }));
        }
        for j in jobs {
            j.await.unwrap();
        }
        let table = h.pair_table().read().unwrap().clone();
        assert_eq!(table.len(), 1);
        let mem = h.persisted();
        assert_eq!(mem.pairs, table);
        assert_eq!(mem.fabric.paired_hubs, current.paired_hubs);
        let want = table[0].address.unwrap().to_string();
        assert_eq!(on_disk()["paired_hubs"][0]["address"], want.as_str());
        // the unpairing (the fabric no longer holds the record) drops it
        assert!(h.fabric().unwrap().unpair(far).await.unwrap());
        until("the half and its dial dropped", || {
            (h.pair_table().read().unwrap().is_empty()
                && on_disk().get("paired_hubs").is_none()
                && h.pair_dials.try_lock().is_ok_and(|d| d.is_empty()))
            .then_some(())
        })
        .await;
        h.shutdown().await;
    }

    /// M5c S1: a pump lag alone keeps the §7.7 override and its dial (the
    /// hub's contact row shows the same membership); a lag that hid a
    /// forget-and-re-enrol (a new contact row) voids it, and the fabric's
    /// own address is shown again.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_pump_lag_keeps_the_rehomed_address_unless_the_membership_changed() {
        use mesh111::link::Link;
        let (hd, md) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (ha, ma) = (test_api(hd.path()), test_api(md.path()));
        let hid = crate::identity::generate(&hd.path().join("identity")).unwrap();
        let hub = start(lo_cfg(hd.path(), true), Some(hid), &ha).await;
        let mid = crate::identity::generate(&md.path().join("identity")).unwrap();
        let m = start(lo_cfg(md.path(), false), Some(mid), &ma).await;
        assert!(hub.running() && m.running());
        let at = hub.listen_addr().await.unwrap();
        let offer = hub.fabric().unwrap().offer(Some(at)).await.unwrap();
        m.fabric()
            .unwrap()
            .enroll(at, &offer.code, Some(&offer.card))
            .await
            .unwrap();
        m.sync_settings().await.unwrap();
        let hub_id = hub.sealer().unwrap().node_id();
        let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let moved = relay.local_addr().unwrap();
        let forward = tokio::spawn(async move {
            while let Ok((mut inbound, _)) = relay.accept().await {
                tokio::spawn(async move {
                    if let Ok(mut out) = tokio::net::TcpStream::connect(at).await {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut out).await;
                    }
                });
            }
        });
        let r = m
            .replace_home_hub_address(moved, Duration::from_secs(5))
            .await;
        assert!(matches!(r, Ok(true)), "{r:?}");
        let marker = m.rehomed.lock().unwrap().unwrap().marker;
        assert!(marker.is_some(), "the membership marker is taken");
        // a lagged batch (events lost) with the membership unchanged
        m.pump_batch(&ma, Vec::new(), true).await;
        assert!(m.rehomed.lock().unwrap().is_some_and(|x| !x.void));
        assert_eq!(m.persisted().fabric.home_hub.unwrap().address, Some(moved));
        assert!(
            Link::peers(m.tcp().unwrap()).contains(&PeerAddr::Tcp(moved)),
            "the verified connection to the hub is kept"
        );
        let st = m.status_json().await.unwrap();
        assert_eq!(st["hub"]["address"], moved.to_string());
        let file = std::fs::read_to_string(settings::path(&m.cfg.fabric_dir)).unwrap();
        assert!(file.contains(&moved.to_string()), "{file}");
        // the hub forgotten and enrolled at again at `at`, all of it lost
        // in a lag (no event reaches the pump): a new contact row
        let hb = *hub_id.as_bytes();
        m.fabric()
            .unwrap()
            .with_store(move |s| {
                let row = s.contact(&hb).unwrap().unwrap();
                assert!(s.contact_forget(&hb).unwrap());
                s.contact_upsert(&row).unwrap();
            })
            .await
            .unwrap();
        assert_ne!(m.hub_marker(hub_id).await.unwrap(), marker);
        m.pump_batch(&ma, Vec::new(), true).await;
        assert!(m.rehomed.lock().unwrap().is_none());
        assert_eq!(m.persisted().fabric.home_hub.unwrap().address, Some(at));
        let st = m.status_json().await.unwrap();
        assert_eq!(st["hub"]["address"], at.to_string());
        let file = std::fs::read_to_string(settings::path(&m.cfg.fabric_dir)).unwrap();
        assert!(file.contains(&format!("\"{at}\"")), "{file}");
        until("the override's dial released", || {
            (!Link::peers(m.tcp().unwrap()).contains(&PeerAddr::Tcp(moved))).then_some(())
        })
        .await;
        forward.abort();
        for h in [m, hub] {
            h.shutdown().await;
        }
    }

    /// M5c S2: an unpairing the pump has not applied yet, then a fresh half
    /// for a re-pairing of the same hub: the half survives the unpairing's
    /// late `Settings` event (and an older view that still holds the
    /// record), kept in memory until its record arrives.
    /// M5c re-review: a forget-and-re-enrol of the hub during the trial
    /// wait (no override exists yet to catch its events) is caught before
    /// the override is recorded: the hub stays where the re-enrolment put
    /// it, nothing is written, the trial's dial is released.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_forget_during_the_trial_records_no_override() {
        use mesh111::link::Link;
        let (hd, md) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (ha, ma) = (test_api(hd.path()), test_api(md.path()));
        let hid = crate::identity::generate(&hd.path().join("identity")).unwrap();
        let hub = start(lo_cfg(hd.path(), true), Some(hid), &ha).await;
        let mid = crate::identity::generate(&md.path().join("identity")).unwrap();
        let m = start(lo_cfg(md.path(), false), Some(mid), &ma).await;
        assert!(hub.running() && m.running());
        let at = hub.listen_addr().await.unwrap();
        let offer = hub.fabric().unwrap().offer(Some(at)).await.unwrap();
        m.fabric()
            .unwrap()
            .enroll(at, &offer.code, Some(&offer.card))
            .await
            .unwrap();
        m.sync_settings().await.unwrap();
        let hub_id = hub.sealer().unwrap().node_id();
        let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let moved = relay.local_addr().unwrap();
        // the first proxied connection is held until the gate opens, so the
        // trial waits with its TCP connection up and no handshake through
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel::<()>();
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<()>();
        let forward = tokio::spawn(async move {
            let mut first = Some((accepted_tx, gate_rx));
            while let Ok((mut inbound, _)) = relay.accept().await {
                if let Some((accepted, gate)) = first.take() {
                    let _ = accepted.send(());
                    let _ = gate.await;
                }
                tokio::spawn(async move {
                    if let Ok(mut out) = tokio::net::TcpStream::connect(at).await {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut out).await;
                    }
                });
            }
        });
        let m2 = Arc::clone(&m);
        let trial = tokio::spawn(async move {
            m2.replace_home_hub_address(moved, Duration::from_secs(10))
                .await
        });
        accepted_rx.await.unwrap();
        let marker = m.hub_marker(hub_id).await.unwrap();
        assert!(marker.is_some());
        let hb = *hub_id.as_bytes();
        m.fabric()
            .unwrap()
            .with_store(move |s| {
                let row = s.contact(&hb).unwrap().unwrap();
                assert!(s.contact_forget(&hb).unwrap());
                s.contact_upsert(&row).unwrap();
            })
            .await
            .unwrap();
        assert_ne!(m.hub_marker(hub_id).await.unwrap(), marker);
        gate_tx.send(()).unwrap();
        let r = trial.await.unwrap();
        assert!(matches!(r, Ok(false)), "{r:?}");
        assert!(m.rehomed.lock().unwrap().is_none());
        assert_eq!(m.persisted().fabric.home_hub.unwrap().address, Some(at));
        let file = std::fs::read_to_string(settings::path(&m.cfg.fabric_dir)).unwrap();
        assert!(!file.contains(&moved.to_string()), "{file}");
        until("the trial's dial released", || {
            (!Link::peers(m.tcp().unwrap()).contains(&PeerAddr::Tcp(moved))).then_some(())
        })
        .await;
        forward.abort();
        for h in [m, hub] {
            h.shutdown().await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_re_pairing_half_survives_the_unpairings_late_event() {
        use mesh111::fabric::{PairRole, PairVia, PairedHub};
        let dir = tempfile::tempdir().unwrap();
        let api = test_api(dir.path());
        let identity = crate::identity::generate(&dir.path().join("identity")).unwrap();
        let far = NodeId::from([0xb0; 32]);
        let fabric_dir = dir.path().join(FABRIC_DIR);
        std::fs::create_dir_all(&fabric_dir).unwrap();
        let file = settings::Persisted {
            fabric: FabricSettings {
                hub_role: true,
                paired_hubs: vec![PairedHub {
                    node_id: far,
                    role: PairRole::Peer,
                    via: PairVia::Tcp,
                    ours: true,
                    theirs: true,
                }],
                ..FabricSettings::default()
            },
            ..settings::Persisted::default()
        };
        settings::save(&fabric_dir, &file).unwrap();
        let h = start(lo_cfg(dir.path(), true), Some(identity), &api).await;
        assert!(h.running(), "{:?}", h.down_reason());
        let at = |a: &str| settings::PairLink {
            address: Some(a.parse().unwrap()),
            ..settings::PairLink::new(far)
        };
        h.set_pair_link(at("127.0.0.1:9")).await.unwrap();
        let older = h.fabric().unwrap().settings().await.unwrap();
        // the pump is held back: no event of the fabric is applied
        if let Ok(mut t) = h.tasks.lock() {
            for j in t.drain(..) {
                j.abort();
            }
        }
        assert!(h.fabric().unwrap().unpair(far).await.unwrap());
        assert_eq!(h.persisted().fabric.paired_hubs.len(), 1, "not applied yet");
        h.set_pair_link(at("127.0.0.1:30")).await.unwrap();
        assert!(
            h.persisted().fabric.paired_hubs.is_empty(),
            "the unpairing folded in"
        );
        let on_disk = || std::fs::read_to_string(settings::path(&fabric_dir)).unwrap();
        assert!(!on_disk().contains("paired_hubs"), "{}", on_disk());
        let late = h.fabric().unwrap().settings().await.unwrap();
        assert!(late.paired_hubs.is_empty());
        h.on_settings(older).await;
        h.on_settings(late).await;
        let table = h.pair_table().read().unwrap().clone();
        assert_eq!(table, vec![at("127.0.0.1:30")], "the fresh half survives");
        assert_eq!(h.persisted().pairs, table);
        assert!(h.persisted().fabric.paired_hubs.is_empty());
        assert!(!on_disk().contains("paired_hubs"), "{}", on_disk());
        h.shutdown().await;
    }

    /// M5b F3: after a pump lag the batch's own (older) settings are not
    /// applied; the fabric's current ones are re-read, and one `fabric`
    /// frame tells every consumer to re-read.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pump_lag_re_reads_the_settings() {
        let dir = tempfile::tempdir().unwrap();
        let api = test_api(dir.path());
        let identity = crate::identity::generate(&dir.path().join("identity")).unwrap();
        let h = start(lo_cfg(dir.path(), true), Some(identity), &api).await;
        assert!(h.running(), "{:?}", h.down_reason());
        let f = h.fabric().unwrap().clone();
        let older = f.settings().await.unwrap();
        assert!(!older.listen_lan);
        // a change the daemon's pump applies (its event was not lost)
        f.patch_settings(mesh111::fabric::SettingsPatch {
            listen_lan: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        until("the pump applied it", || {
            h.persisted().fabric.listen_lan.then_some(())
        })
        .await;
        // a lagged batch whose only settings are the older ones
        let head = api.events.subscribe(None).head;
        h.pump_batch(&api, vec![FabricEvent::Settings(older)], true)
            .await;
        assert!(h.persisted().fabric.listen_lan, "the current settings won");
        let file = std::fs::read_to_string(settings::path(&h.cfg.fabric_dir)).unwrap();
        assert!(file.contains("\"listen_lan\": true"), "{file}");
        let sent: Vec<String> = api
            .events
            .subscribe(Some(head))
            .replay
            .iter()
            .map(|f| f.event.clone())
            .collect();
        assert!(sent.iter().any(|e| e == "fabric"), "{sent:?}");
        h.shutdown().await;
    }

    /// B1-bounce F5: a conversation read that fails for a reason other
    /// than NotFound (here: the actor stopped) turns the batch's frames for
    /// that conversation into ONE `conversation` refetch frame (seq 0);
    /// nothing is silently dropped.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_conversation_read_publishes_a_refetch_frame() {
        use mesh111::store::ForeignKind;
        let dir = tempfile::tempdir().unwrap();
        let api = test_api(dir.path());
        let identity = crate::identity::generate(&dir.path().join("identity")).unwrap();
        let h = start(test_cfg(dir.path()), Some(identity), &api).await;
        assert!(h.running(), "{:?}", h.down_reason());
        let f = h.fabric().unwrap().clone();
        let ch = f
            .foreign_conversation_upsert(
                "meshtastic",
                "!0badc0de",
                ForeignKind::Channel,
                "ch:LongFast:9f3c",
                Some("LongFast"),
            )
            .await
            .unwrap();
        // healthy: the frames are built from the conversation read
        let ok = h.foreign_frames(vec![(ch, None, ForeignChange::New)]).await;
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0]["network"], "meshtastic");
        // the actor stops: every read now fails with Stopped
        f.shutdown().await.unwrap();
        assert!(matches!(
            f.foreign_conversation(ch).await,
            Err(FabricError::Stopped)
        ));
        let got = h
            .foreign_frames(vec![
                (ch, Some(7), ForeignChange::New),
                (ch, Some(8), ForeignChange::State),
                (ch, None, ForeignChange::New),
                (ch + 1, None, ForeignChange::Conversation),
            ])
            .await;
        assert_eq!(
            got,
            vec![
                json!({"network": UNKNOWN_NETWORK, "conv": json::fc_id(ch), "change": "conversation", "seq": 0}),
                json!({"network": UNKNOWN_NETWORK, "conv": json::fc_id(ch + 1), "change": "conversation", "seq": 0}),
            ]
        );
        for v in &got {
            json::foreign_tests::assert_schema(v, "SseNetworkMessage");
        }
        h.shutdown().await;
    }
}
