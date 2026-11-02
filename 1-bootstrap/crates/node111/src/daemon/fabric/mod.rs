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
//!   the fabric drops (an unpairing) takes the daemon's half with it.
//! - **Dials** (§4.6 step 5, one connection per pair): the pairs
//!   [`settings::pair_dials`] names are dialled through `Fabric::dial_peer`,
//!   as `peers` are, at start and after every pair change; one it no longer
//!   names is released (unless the home hub or a peer uses the address).
//! - **The stream wrap** (§4.4): `TcpLanConfig.wrap` is
//!   `relay::tls::wrap` (D5), handed the live pairing table
//!   ([`settings::PairTable`]); `None` (until D5, and always without a
//!   non-private pair) keeps `tcp-lan` byte-identical. It is read once,
//!   when `tcp-lan` is built.
//! - **The tap** (§2.1, §3.5, §5.9): every `Message`, `Contact`, `Fabric`
//!   and `Forward` event of a batch goes to `relay::tap` as the batch is
//!   published, a no-op while no Layer 7 consumer is on. `Forward` (the
//!   carrier's trigger, D11) has no SSE frame of its own.
//! - **Lost hub** (§7.7, for D8): [`FabricHandle::replace_home_hub_address`]
//!   keeps a new home-hub address only after a handshake there named the
//!   hub (the dial's expected NodeId).
//! - **The carrier topic's X25519** (§6.2) stays behind the identity handle
//!   ([`IdentitySealer::carrier_topic_x25519`]), on no API.
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
use crate::daemon::services::probe::Provider;
use crate::daemon::services::FabricFacts;
use crate::daemon::DaemonLayout;
use crate::identity::Identity;
use axum::http::StatusCode;
use id111::keyset::NodeId;
use mesh111::contacts::{ROLE_ALWAYS_ON_HUB, ROLE_MAILBOX};
use mesh111::fabric::{
    ContactChange, Fabric, FabricError, FabricEvent, FabricSettings, FabricStatus, ForeignChange,
    ForeignGroupPart, ForeignQueue, MessageChange, MessageView, MAX_PAIRED_HUBS,
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
use std::time::Duration;
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
    tasks: Mutex<Vec<JoinHandle<()>>>,
    /// Layer 7 §4.6: the daemon's pairing rows, shared with the stream wrap.
    pairs: settings::PairTable,
    /// Layer 7 §4.6 step 5: the pair dials this handle started (far hub →
    /// address); also serializes their reconciliation.
    pair_dials: tokio::sync::Mutex<HashMap<NodeId, SocketAddr>>,
    /// Layer 7 §7.7: the home-hub address replaced in this run.
    rehomed: Mutex<Option<Rehomed>>,
    /// Serializes home-hub address replacements.
    rehome: tokio::sync::Mutex<()>,
}

/// A home-hub address replaced in this run (Layer 7 §7.7): the fabric still
/// holds `from` (it has no setter), and the daemon persists and shows `to`
/// over it until the fabric's settings name the hub differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rehomed {
    hub: NodeId,
    from: Option<SocketAddr>,
    to: SocketAddr,
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

fn roles(s: &FabricSettings) -> u16 {
    let mut r = 0;
    if s.holds_custody() {
        r |= ROLE_MAILBOX;
    }
    if s.hub_role {
        r |= ROLE_ALWAYS_ON_HUB;
    }
    r
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
    match boot_with(cfg, &sealer, persisted, recover, pairs).await {
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
/// `announced` is the home-hub hint of the own announce as stored before
/// this start re-mints it ([`IdentitySealer::stored_hub_hint`]); the pairs
/// salvaged from an invalid file are never rebuilt as the home hub (Layer 7
/// §19 C7).
fn recover_settings(
    cfg: &FabricConfig,
    persisted: &mut settings::Persisted,
    recover: &settings::Recover,
    enrolled: &[settings::Enrolled],
    announced: Option<[u8; 8]>,
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
            None
        };
        recover_settings(cfg, persisted, r, &enrolled, announced);
    }

    let app = sealer::appdata(
        roles(&persisted.fabric),
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
            match boot(
                &cfg,
                identity,
                &mut persisted,
                loaded.recover.as_ref(),
                &pairs,
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
        tasks: Mutex::new(Vec::new()),
        pairs,
        pair_dials: tokio::sync::Mutex::new(HashMap::new()),
        rehomed: Mutex::new(None),
        rehome: tokio::sync::Mutex::new(()),
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
        if let Some(x) = self.rehomed.lock().ok().and_then(|r| *r) {
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
    /// step 5). A pair record the fabric dropped takes the daemon's half
    /// with it; a half whose record has not arrived yet stays (module doc,
    /// "Layer 7 seams"). Returns the settings as persisted (a replaced
    /// home-hub address over the fabric's copy, §7.7).
    async fn on_settings(&self, s: FabricSettings) -> FabricSettings {
        let mut s = s;
        let stale = self.apply_rehome(&mut s);
        let (snapshot, pairs_changed) = match self.persisted.lock() {
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
                let changed = p.pairs.len() != before;
                p.fabric = s.clone();
                (p.clone(), changed)
            }
            Err(_) => return s,
        };
        if pairs_changed {
            self.publish_pairs(&snapshot.pairs);
        }
        self.save(&snapshot);
        if let (Some(a), Some(tcp)) = (stale, self.tcp()) {
            if !settings::holds_address(&snapshot, a)
                && !snapshot.pairs.iter().any(|l| l.address == Some(a))
            {
                Dialer::release(tcp, a);
            }
        }
        self.reconcile_listener(&s).await;
        self.refresh_announce(&s).await;
        self.reconcile_pair_dials().await;
        s
    }

    fn save(&self, p: &settings::Persisted) {
        if let Err(e) = settings::save(&self.cfg.fabric_dir, p) {
            eprintln!("111 fabric: settings not saved: {e:#}");
        }
    }

    /// `s` with this run's replaced home-hub address put over the fabric's
    /// own copy (Layer 7 §7.7). When `s` no longer names that hub at that
    /// address (forgotten, re-enrolled elsewhere or anew) the replacement
    /// is forgotten, and its address returned for release.
    fn apply_rehome(&self, s: &mut FabricSettings) -> Option<SocketAddr> {
        let Ok(mut r) = self.rehomed.lock() else {
            return None;
        };
        let x = (*r)?;
        match s.home_hub.as_mut() {
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

    fn publish_pairs(&self, pairs: &[settings::PairLink]) {
        if let Ok(mut t) = self.pairs.write() {
            *t = pairs.to_vec();
        }
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
    /// `ENROLL_OK`. Refused without the hub role (§5.11), for this node, for
    /// an address that is no dial target, and beyond [`MAX_PAIRED_HUBS`]
    /// halves. The §4.6 step 5 dials follow.
    pub async fn set_pair_link(&self, link: settings::PairLink) -> Result<(), FabricError> {
        let l = self.live.as_ref().ok_or(FabricError::Stopped)?;
        if let Some(why) =
            settings::pair_link_refusal(&link, &l.sealer.node_id(), self.cfg.tcp.refuse_loopback)
        {
            return Err(FabricError::Invalid(why));
        }
        let (snapshot, recorded) = {
            let mut p = self.persisted.lock().map_err(|_| FabricError::Stopped)?;
            if !p.fabric.hub_role {
                return Err(FabricError::Invalid("pairing needs the hub role"));
            }
            match p.pairs.iter().position(|x| x.node_id == link.node_id) {
                Some(i) => p.pairs[i] = link.clone(),
                None if p.pairs.len() >= MAX_PAIRED_HUBS => {
                    return Err(FabricError::Invalid("too many paired hubs"))
                }
                None => p.pairs.push(link.clone()),
            }
            let recorded = p
                .fabric
                .paired_hubs
                .iter()
                .any(|h| h.node_id == link.node_id);
            (p.clone(), recorded)
        };
        self.publish_pairs(&snapshot.pairs);
        if recorded {
            self.save(&snapshot);
        }
        self.reconcile_pair_dials().await;
        Ok(())
    }

    /// Drop this hub's half of the row of `node_id` (an abandoned pairing,
    /// D6). A recorded pair keeps its fabric record (`Fabric::unpair` drops
    /// that, and the half with it). Returns whether a half was held.
    pub async fn forget_pair_link(&self, node_id: &NodeId) -> bool {
        let (snapshot, recorded) = match self.persisted.lock() {
            Ok(mut p) => {
                let before = p.pairs.len();
                p.pairs.retain(|x| x.node_id != *node_id);
                if p.pairs.len() == before {
                    return false;
                }
                let recorded = p.fabric.paired_hubs.iter().any(|h| h.node_id == *node_id);
                (p.clone(), recorded)
            }
            Err(_) => return false,
        };
        self.publish_pairs(&snapshot.pairs);
        if recorded {
            self.save(&snapshot);
        }
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
    /// uses; `NoLink` while no `tcp-lan` runs. One bounded wait on the
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
        let mut events = l.fabric.subscribe();
        Dialer::maintain(tcp, address, Some(fabric_hub.node_id));
        if !connected_at(tcp, &mut events, address, wait).await {
            Dialer::release(tcp, address);
            return Ok(false);
        }
        if let Err(e) = l.fabric.dial_peer(address, fabric_hub.node_id).await {
            Dialer::release(tcp, address);
            return Err(e);
        }
        if let Ok(mut r) = self.rehomed.lock() {
            *r = (fabric_hub.address != Some(address)).then_some(Rehomed {
                hub: fabric_hub.node_id,
                from: fabric_hub.address,
                to: address,
            });
        }
        let snapshot = match self.persisted.lock() {
            Ok(mut p) => {
                if let Some(h) = p
                    .fabric
                    .home_hub
                    .as_mut()
                    .filter(|h| h.node_id == fabric_hub.node_id)
                {
                    h.address = Some(address);
                }
                p.clone()
            }
            Err(_) => return Ok(true),
        };
        self.save(&snapshot);
        if let Some(old) = shown.address {
            if !settings::holds_address(&snapshot, old)
                && !snapshot.pairs.iter().any(|x| x.address == Some(old))
            {
                Dialer::release(tcp, old);
            }
        }
        Ok(true)
    }

    /// Re-mint the own announce for `s` (§7.3 A8: roles, home-hub hint —
    /// `ensure_announce` keeps the stored one when its `appdata` already
    /// matches; a new one is counter-committed first, §6.2) and give it to
    /// `tcp-lan` for new connections. Nothing without a link (no announce
    /// is presented while restore-pending).
    async fn refresh_announce(&self, s: &FabricSettings) {
        let Some(l) = &self.live else { return };
        let Some(tcp) = &l.tcp else { return };
        let _one = self.remint.lock().await;
        let app = sealer::appdata(roles(s), s.home_hub.as_ref().map(|h| &h.node_id));
        let (handle, dir) = (Arc::clone(&l.sealer), self.cfg.fabric_dir.clone());
        match tokio::task::spawn_blocking(move || handle.ensure_announce(&dir, &app)).await {
            Ok(AnnounceOutcome::Missing) => {}
            Ok(_) => {
                let bytes = l.sealer.own_announce();
                if !bytes.is_empty() {
                    tcp.set_announce(bytes);
                }
            }
            Err(e) => eprintln!("111 fabric: announce re-mint: {e}"),
        }
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

    /// Publish one batch of fabric events (see the module doc, step 6).
    async fn publish(&self, api: &ApiState, batch: Vec<FabricEvent>) {
        let mut messages: BTreeMap<i64, MessageChange> = BTreeMap::new();
        let mut order: Vec<i64> = Vec::new();
        let mut contacts: Vec<(NodeId, ContactChange)> = Vec::new();
        let mut foreign = ForeignBatch::default();
        let mut fabric = false;
        let mut new_settings = None;
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
                FabricEvent::Settings(s) => new_settings = Some(s),
                // Layer 7 §5.9: the carrier's trigger, handed to the tap
                // above (D11); no SSE frame of its own.
                FabricEvent::Forward { .. } => {}
            }
        }
        if let Some(s) = new_settings {
            self.on_settings(s).await;
            fabric = true;
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
        if lagged {
            batch.push(FabricEvent::Fabric);
        }
        let (Some(me), Some(api)) = (me.upgrade(), api.upgrade()) else {
            return;
        };
        me.publish(&api, batch).await;
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
        let mut s = FabricSettings::default();
        assert_eq!(roles(&s), 0);
        s.custody_opt_in = true;
        assert_eq!(roles(&s), ROLE_MAILBOX);
        s.hub_role = true;
        assert_eq!(roles(&s), ROLE_MAILBOX | ROLE_ALWAYS_ON_HUB);
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
        use crate::daemon::dashboard::api_v1::ApiConfig;
        use crate::daemon::dashboard::DashboardState;
        std::fs::create_dir_all(dir.join("state")).unwrap();
        let layout = DaemonLayout {
            root: dir.to_path_buf(),
            etc: dir.join("etc"),
        };
        let mut cfg = ApiConfig::new(layout, false);
        cfg.persist_auth = false;
        ApiState::new(Arc::new(RwLock::new(DashboardState::new("t"))), cfg)
    }

    fn test_cfg(dir: &Path) -> FabricConfig {
        FabricConfig {
            fabric_dir: dir.join(FABRIC_DIR),
            identity_dir: dir.join("identity"),
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
        forward.abort();
        for h in [m, hub, other] {
            h.shutdown().await;
        }
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
