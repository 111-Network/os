//! The bridge core (Layer 6 `6-transport/README.md` §2, §3, §4, §10): the adapter
//! registry, the adapter manifests, the recv tasks, the monotonic tick and the typed API
//! surface the routes (C1) call.
//!
//! - **Registry.** [`Bridge::enable`] attaches an `Arc<dyn Adapter>` under its network id
//!   and spawns **one recv task** for it; [`Bridge::disable`] aborts that task and drops
//!   the adapter. With no adapter enabled there is no task, and [`Bridge::tick_once`]
//!   returns after one read-lock and one length check: nothing runs, polls, allocates or
//!   listens (§2 "inert with no adapter enabled"; `AGENTS.md`, the Raspberry Pi floor).
//! - **Recv task.** Awaits `Adapter::recv`, then drains whatever else is ready without
//!   waiting (`now_or_never`: adapters' `recv` is cancel-safe, as the stub's is) into one
//!   bounded batch of at most [`BridgeConfig::recv_batch`] (≤ 256) items, and hands the
//!   batch to the §3 ingest: texts go through the one ingest function and §8 guard 2, then
//!   **one** `FabricHandle::foreign_ingest` call, awaited (the back-pressure on the actor);
//!   conversation upserts, Nearby items (to the daemon's [`NearbySink`]), link state and
//!   the adapter's dropped-text counter are applied in the same pass.
//! - **Tick.** No timer of its own: the daemon's existing tick calls
//!   [`Bridge::tick_once`] (the `Meshtastic::tick_once` shape). One pass drains send
//!   events, runs the relay edges (§8, [`super::edges`]) and the delivery queue (§4,
//!   [`super::state`]). The tick counter is the only clock the core reads; no wall clock
//!   orders or decides anything (retry back-off counts ticks).
//! - **Manifests** (`assets/adapters/<id>.toml`, embedded like `assets/components/`;
//!   `deny_unknown_fields`, size-checked before parse, validated; an invalid file is
//!   logged and not registered): the honest copy, the relay TAG, caps for a network whose
//!   adapter is not running, the recipient syntax. A stub network without a manifest uses
//!   its `caps().copy`.
//! - **Errors** ([`BridgeError`]) map onto the contract's problems
//!   ([`BridgeError::problem`]): `404 not_found`, `400 bad_request`, `409 oversize`,
//!   `409 consent_required` with the `consent` member, `409` per-target refusals with the
//!   `targets` member, `409 component_unavailable`, and the fabric's own mapping.

use super::{
    ingest, settings, state, Adapter, AdapterFacts, AdapterState, Auth, Caps, ConvKind, ConvRef,
    ForeignMessageState, Inbound, LinkState, PerKind,
};
use crate::daemon::components::consent::{ConsentRequired, ProblemWith};
use crate::daemon::components::Components;
use crate::daemon::dashboard::api_v1::{ApiState, Problem};
use crate::daemon::fabric::{self as fab, json as fjson, FabricHandle};
use axum::http::StatusCode;
use futures_util::FutureExt;
use mesh111::fabric::{Fabric, FabricError};
use mesh111::store::{ForeignConversationRow, ForeignKind};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};
use tokio::task::JoinHandle;

/// Largest manifest accepted (checked before parsing).
pub const MAX_MANIFEST_BYTES: usize = 16 * 1024;
/// Items per recv batch at most (§3: a bounded batch per adapter tick; the store's cap).
pub const MAX_RECV_BATCH: usize = mesh111::store::MAX_FOREIGN_BATCH;
/// The networks in the order the release declares them (the preferred order is the
/// user's, `GET /transport` `order`; this is the fallback).
pub const NETWORK_ORDER: &[&str] = &[
    super::NETWORK_MESHTASTIC,
    super::NETWORK_LXMF,
    super::NETWORK_DELTACHAT,
    super::NETWORK_SIMPLEX,
];
/// Conversation ids cached per adapter (cleared when full).
const CONV_CACHE: usize = 4096;
/// The account recipient lists are stored under (local objects, no account of ours).
pub const LIST_ACCOUNT: &str = "local";
/// The native-id prefix of a recipient list's group conversation.
pub const LIST_PREFIX: &str = "list:";

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------- manifests

/// The embedded folder (`assets/adapters/`). Debug builds read it from disk; release
/// builds embed it.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets/adapters/"]
struct AdapterFiles;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    id: String,
    label: String,
    chip: String,
    tag: String,
    #[serde(default)]
    kinds: Option<Vec<ConvKind>>,
    #[serde(default)]
    evidence: Option<Vec<ForeignMessageState>>,
    #[serde(default)]
    splittable: Option<bool>,
    #[serde(default)]
    auth: BTreeMap<String, String>,
    copy: RawCopy,
    #[serde(default)]
    limits: RawLimits,
    #[serde(default)]
    recipient: Option<RecipientSyntax>,
    #[serde(default)]
    airtime_rule: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCopy {
    banner: String,
    send_warning: String,
    #[serde(default)]
    uplink_warning: Option<String>,
    #[serde(default)]
    dm_hint: Option<String>,
    #[serde(default)]
    service: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLimits {
    #[serde(default)]
    channel: Option<usize>,
    #[serde(default)]
    dm: Option<usize>,
    #[serde(default)]
    group: Option<usize>,
    #[serde(default)]
    channel_licensed: Option<usize>,
    #[serde(default)]
    dm_licensed: Option<usize>,
    #[serde(default)]
    group_licensed: Option<usize>,
}

/// A network's recipient syntax as the contract shows it (`NetworkCaps.recipient_syntax`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipientSyntax {
    pub label: String,
    /// Anchored, the [`ingest::syntax_match`] subset.
    pub pattern: String,
    pub example: String,
}

/// One validated adapter manifest (§4 honest copy, so the UI holds no per-network code).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterManifest {
    pub id: String,
    pub label: String,
    pub chip: String,
    /// The relay marker TAG (§8): one to three ASCII letters.
    pub tag: String,
    pub kinds: Vec<ConvKind>,
    pub evidence: Vec<ForeignMessageState>,
    pub splittable: bool,
    /// `auth` level → its copy.
    pub auth: Vec<(Auth, String)>,
    pub banner: String,
    pub send_warning: String,
    /// "this channel is published to the internet by your board" (§5).
    pub uplink_warning: Option<String>,
    pub dm_hint: Option<String>,
    pub service: Option<String>,
    pub limits: PerKind,
    /// Caps on a licensed account where they differ (§4: Meshtastic DM 232 B).
    pub licensed: PerKind,
    pub recipient: Option<RecipientSyntax>,
    /// The preview's estimate rule in words (§9), when the network has one.
    pub airtime_rule: Option<String>,
}

fn valid_network_id(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 32
        && v != super::NETWORK_111
        && v.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_:".contains(&b))
}

fn parse_auth(level: &str) -> Option<Auth> {
    serde_json::from_value(Value::String(level.to_string())).ok()
}

/// Parse and validate one manifest named `file` (`<id with : as ->.toml`).
pub fn parse_manifest(file: &str, bytes: &[u8]) -> Result<AdapterManifest, String> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err("larger than 16 KiB".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "not UTF-8".to_string())?;
    let raw: RawManifest = toml::from_str(text).map_err(|e| e.to_string())?;
    if !valid_network_id(&raw.id) {
        return Err(format!("id {:?} is not a network id", raw.id));
    }
    let stem = file.strip_suffix(".toml").unwrap_or(file);
    if stem != raw.id.replace(':', "-") {
        return Err(format!("id {:?} does not match the file name", raw.id));
    }
    let short = |what: &str, v: &str| {
        if v.trim().is_empty() || v.len() > 64 {
            Err(format!("{what} must be 1..=64 bytes"))
        } else {
            Ok(())
        }
    };
    short("label", &raw.label)?;
    short("chip", &raw.chip)?;
    if !super::marker::valid_tag(&raw.tag) {
        return Err(format!(
            "tag {:?} is not one to three ASCII letters",
            raw.tag
        ));
    }
    let l = &raw.limits;
    for v in [
        l.channel,
        l.dm,
        l.group,
        l.channel_licensed,
        l.dm_licensed,
        l.group_licensed,
    ]
    .into_iter()
    .flatten()
    {
        if v == 0 || v > ingest::MAX_BODY_BYTES {
            return Err("limits must be 1..=65536 bytes".into());
        }
    }
    let limits = PerKind {
        channel: l.channel,
        dm: l.dm,
        group: l.group,
    };
    let kinds = match raw.kinds {
        Some(k) if !k.is_empty() => k,
        Some(_) => return Err("kinds is empty".into()),
        None => [ConvKind::Channel, ConvKind::Dm, ConvKind::Group]
            .into_iter()
            .filter(|k| limits.get(*k).is_some())
            .collect(),
    };
    let mut auth = Vec::new();
    for (level, copy) in raw.auth {
        let a = parse_auth(&level).ok_or_else(|| format!("auth level {level:?} unknown"))?;
        auth.push((a, copy));
    }
    if let Some(r) = &raw.recipient {
        if !(r.pattern.starts_with('^') && r.pattern.ends_with('$'))
            || !ingest::syntax_match(&r.pattern, &r.example)
        {
            return Err("recipient pattern must be anchored and match its example".into());
        }
    }
    Ok(AdapterManifest {
        id: raw.id,
        label: raw.label,
        chip: raw.chip,
        tag: raw.tag,
        kinds,
        evidence: raw.evidence.unwrap_or_else(|| {
            vec![
                ForeignMessageState::Queued,
                ForeignMessageState::Sent,
                ForeignMessageState::Relayed,
                ForeignMessageState::Delivered,
            ]
        }),
        splittable: raw.splittable.unwrap_or(true),
        auth,
        banner: raw.copy.banner,
        send_warning: raw.copy.send_warning,
        uplink_warning: raw.copy.uplink_warning,
        dm_hint: raw.copy.dm_hint,
        service: raw.copy.service,
        limits,
        licensed: PerKind {
            channel: l.channel_licensed,
            dm: l.dm_licensed,
            group: l.group_licensed,
        },
        recipient: raw.recipient,
        airtime_rule: raw.airtime_rule,
    })
}

/// Every embedded manifest, invalid or duplicate ones logged and left out, in
/// [`NETWORK_ORDER`] then by id.
pub fn embedded_manifests() -> Vec<AdapterManifest> {
    let mut files: Vec<(String, Vec<u8>)> = AdapterFiles::iter()
        .filter(|n| n.ends_with(".toml") && !n.contains('/'))
        .filter_map(|n| AdapterFiles::get(&n).map(|f| (n.to_string(), f.data.into_owned())))
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    load_manifests(&files)
}

/// Parse `files` (`(name, bytes)`); see [`embedded_manifests`].
pub fn load_manifests(files: &[(String, Vec<u8>)]) -> Vec<AdapterManifest> {
    let mut out: Vec<AdapterManifest> = Vec::new();
    for (name, bytes) in files {
        match parse_manifest(name, bytes) {
            Ok(m) if out.iter().any(|o| o.id == m.id || o.tag == m.tag) => {
                eprintln!("111 bridge: adapter {name}: not registered: duplicate id or tag");
            }
            Ok(m) => out.push(m),
            Err(e) => eprintln!("111 bridge: adapter {name}: not registered: {e}"),
        }
    }
    out.sort_by_key(|m| order_key(&m.id));
    out
}

fn order_key(id: &str) -> (usize, String) {
    let i = NETWORK_ORDER
        .iter()
        .position(|n| *n == id)
        .unwrap_or(NETWORK_ORDER.len());
    (i, id.to_string())
}

// ---------------------------------------------------------------- errors

/// A failure of a bridge call, as the contract's problem.
#[derive(Debug)]
pub enum BridgeError {
    /// `404 not_found` (what was not found).
    NotFound(&'static str),
    /// `400 bad_request`.
    BadRequest(String),
    /// `409 oversize` (refuse mode): the detail ("312 B > 232 B per packet").
    Oversize(String),
    /// `409 consent_required` with the exact text and hash (`consent`).
    Consent(ConsentRequired),
    /// `409` with the per-target report (`targets`): nothing was queued (§9).
    Refused {
        code: &'static str,
        targets: Vec<Value>,
    },
    /// `409 component_unavailable`: the network's adapter is not enabled.
    Unavailable(String),
    /// A fabric failure (its own mapping).
    Fabric(FabricError),
    /// `state/bridge.json` could not be written (nothing changed).
    Io(String),
}

impl From<FabricError> for BridgeError {
    fn from(e: FabricError) -> Self {
        BridgeError::Fabric(e)
    }
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::NotFound(w) => write!(f, "not found: {w}"),
            BridgeError::BadRequest(w) => write!(f, "bad request: {w}"),
            BridgeError::Oversize(w) => write!(f, "oversize: {w}"),
            BridgeError::Consent(c) => write!(f, "consent required: {}", c.action),
            BridgeError::Refused { code, targets } => {
                write!(f, "refused ({code}): {} target(s)", targets.len())
            }
            BridgeError::Unavailable(w) => write!(f, "unavailable: {w}"),
            BridgeError::Fabric(e) => write!(f, "fabric: {e}"),
            BridgeError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for BridgeError {}

impl BridgeError {
    /// The contract problem (`application/problem+json` with its extension members).
    pub fn problem(&self) -> ProblemWith {
        let p = |s: StatusCode, code: &'static str, title: &str| Problem::new(s, code, title);
        match self {
            BridgeError::NotFound(what) => ProblemWith::new(
                p(StatusCode::NOT_FOUND, "not_found", "Not found.")
                    .detail(format!("No such {what}.")),
            ),
            BridgeError::BadRequest(why) => ProblemWith::new(
                p(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    "The request does not apply.",
                )
                .detail(why.clone()),
            ),
            BridgeError::Oversize(detail) => ProblemWith::new(
                p(
                    StatusCode::CONFLICT,
                    "oversize",
                    "The text is longer than this network carries.",
                )
                .detail(format!(
                    "{detail}. Shorten it, or send it split into numbered parts."
                )),
            ),
            BridgeError::Consent(c) => ProblemWith::new(
                p(
                    StatusCode::CONFLICT,
                    "consent_required",
                    "Read and confirm the warning first.",
                )
                .detail("Show the text, then repeat the request with accept_warning = its hash."),
            )
            .with("consent", serde_json::to_value(c).unwrap_or_default()),
            BridgeError::Refused { code, targets } => ProblemWith::new(
                p(
                    StatusCode::CONFLICT,
                    code,
                    "Some targets were refused; nothing was queued.",
                )
                .detail("See targets: each refused target with its reason."),
            )
            .with("targets", Value::Array(targets.clone())),
            BridgeError::Unavailable(why) => ProblemWith::new(
                p(
                    StatusCode::CONFLICT,
                    "component_unavailable",
                    "This network's adapter is not enabled.",
                )
                .detail(why.clone()),
            ),
            BridgeError::Fabric(FabricError::Invalid(why)) => ProblemWith::new(
                p(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    "The request does not apply.",
                )
                .detail(*why),
            ),
            BridgeError::Fabric(e) => ProblemWith::new(fab::problem(e)),
            BridgeError::Io(e) => ProblemWith::new(
                p(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "The bridge settings could not be saved.",
                )
                .detail(e.clone()),
            ),
        }
    }
}

impl axum::response::IntoResponse for BridgeError {
    fn into_response(self) -> axum::response::Response {
        self.problem().into_response()
    }
}

// ---------------------------------------------------------------- the bridge

/// Where the bridge keeps its file and how much one tick does.
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// The daemon root (`state/bridge.json` lives under it).
    pub root: PathBuf,
    /// Items per recv batch (≤ [`MAX_RECV_BATCH`]).
    pub recv_batch: usize,
    /// Rows handed to one adapter per tick (ordering, not a cap: the rest wait a tick).
    pub sends_per_tick: usize,
    /// Edge inputs looked at per edge per tick.
    pub relay_per_tick: usize,
    /// The largest retry back-off, in ticks (§4: capped back-off; a mechanism, not a policy).
    pub backoff_cap_ticks: u64,
}

impl BridgeConfig {
    pub fn new(root: &Path) -> Self {
        BridgeConfig {
            root: root.to_path_buf(),
            recv_batch: MAX_RECV_BATCH,
            sends_per_tick: 16,
            relay_per_tick: 64,
            backoff_cap_ticks: 64,
        }
    }
}

/// Where Nearby items an adapter reports go (`(network, NearbySource JSON)`); the daemon
/// wires it (the LXMF helper's heard announces; B3's client feeds its own).
pub type NearbySink = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// One enabled adapter and its recv task.
pub(super) struct Attached {
    pub(super) adapter: Arc<dyn Adapter>,
    task: Mutex<Option<JoinHandle<()>>>,
    pub(super) dropped: AtomicU64,
    pub(super) echoes: AtomicU64,
    pub(super) link: Mutex<Option<LinkState>>,
    /// An adapter event (link up, heard, key learned) since the last tick: `Later` rows of
    /// this network retry now (§4).
    pub(super) woken: AtomicBool,
    convs: Mutex<HashMap<ConvRef, i64>>,
}

impl Attached {
    /// The link is not usable (reported down or partitioned).
    pub(super) fn link_down(&self) -> bool {
        matches!(
            *lock(&self.link),
            Some(LinkState::Down) | Some(LinkState::Partitioned)
        )
    }
}

/// The bridge core (one per daemon; shared as `Arc<Bridge>`).
pub struct Bridge {
    pub(super) cfg: BridgeConfig,
    pub(super) handle: Arc<FabricHandle>,
    api: Weak<ApiState>,
    pub(super) manifests: Vec<AdapterManifest>,
    attached: RwLock<BTreeMap<String, Arc<Attached>>>,
    tick: AtomicU64,
    gate: tokio::sync::Mutex<()>,
    pub(super) delivery: Mutex<state::Delivery>,
    pub(super) echo: Mutex<ingest::OwnEcho>,
    pub(super) sections: Mutex<settings::Sections>,
    components: OnceLock<Arc<Components>>,
    nearby: Mutex<Option<NearbySink>>,
    ingest_batches: AtomicU64,
    noise: AtomicU64,
    /// Edges this run paused because a consent fact changed (§8).
    pub(super) consent_paused: Mutex<HashSet<i64>>,
}

impl std::fmt::Debug for Bridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bridge")
            .field("enabled", &self.enabled())
            .field("tick", &self.tick())
            .finish_non_exhaustive()
    }
}

/// Lock a bridge mutex (poison is not a reason to stop delivering).
pub(super) fn lock_set<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    lock(m)
}

/// The cached `state/bridge.json` sections (settings, gates).
pub(super) fn lock_sections(b: &Bridge) -> std::sync::MutexGuard<'_, settings::Sections> {
    lock(&b.sections)
}

/// The store's kind for a contract kind.
pub fn store_kind(k: ConvKind) -> ForeignKind {
    match k {
        ConvKind::Channel => ForeignKind::Channel,
        ConvKind::Dm => ForeignKind::Dm,
        ConvKind::Group => ForeignKind::Group,
    }
}

/// The contract kind of a stored conversation.
pub fn conv_kind(k: ForeignKind) -> ConvKind {
    match k {
        ForeignKind::Channel => ConvKind::Channel,
        ForeignKind::Dm => ConvKind::Dm,
        ForeignKind::Group => ConvKind::Group,
    }
}

/// The native coordinates of a stored conversation.
pub fn conv_ref(c: &ForeignConversationRow) -> ConvRef {
    ConvRef {
        network: c.network.clone(),
        account: c.account.clone(),
        kind: conv_kind(c.kind),
        native_id: c.native_id.clone(),
    }
}

/// Is `c` a recipient list's conversation (§6: a local destination-only group)?
pub fn is_list_conv(c: &ForeignConversationRow) -> bool {
    c.kind == ForeignKind::Group
        && c.account == LIST_ACCOUNT
        && c.native_id.starts_with(LIST_PREFIX)
}

impl Bridge {
    /// A bridge on the running fabric `handle`, publishing SSE on `api`'s bus. Inert until
    /// an adapter is enabled.
    pub fn new(cfg: BridgeConfig, handle: Arc<FabricHandle>, api: &Arc<ApiState>) -> Arc<Bridge> {
        Self::with_manifests(cfg, handle, api, embedded_manifests())
    }

    /// The same with explicit manifests (tests).
    pub fn with_manifests(
        mut cfg: BridgeConfig,
        handle: Arc<FabricHandle>,
        api: &Arc<ApiState>,
        manifests: Vec<AdapterManifest>,
    ) -> Arc<Bridge> {
        cfg.recv_batch = cfg.recv_batch.clamp(1, MAX_RECV_BATCH);
        cfg.sends_per_tick = cfg.sends_per_tick.max(1);
        cfg.relay_per_tick = cfg.relay_per_tick.max(1);
        cfg.backoff_cap_ticks = cfg.backoff_cap_ticks.max(1);
        let sections = settings::Sections::load(&cfg.root);
        Arc::new(Bridge {
            cfg,
            handle,
            api: Arc::downgrade(api),
            manifests,
            attached: RwLock::new(BTreeMap::new()),
            tick: AtomicU64::new(0),
            gate: tokio::sync::Mutex::new(()),
            delivery: Mutex::new(state::Delivery::default()),
            echo: Mutex::new(ingest::OwnEcho::default()),
            sections: Mutex::new(sections),
            components: OnceLock::new(),
            nearby: Mutex::new(None),
            ingest_batches: AtomicU64::new(0),
            noise: AtomicU64::new(0),
            consent_paused: Mutex::new(HashSet::new()),
        })
    }

    /// The optional-component registry (S4), for `GET /networks` component facts.
    pub fn set_components(&self, c: Arc<Components>) {
        let _ = self.components.set(c);
    }

    /// Where Nearby items go (the daemon's Nearby list).
    pub fn set_nearby_sink(&self, sink: NearbySink) {
        *lock(&self.nearby) = Some(sink);
    }

    pub fn config(&self) -> &BridgeConfig {
        &self.cfg
    }

    /// The registered manifests.
    pub fn manifests(&self) -> &[AdapterManifest] {
        &self.manifests
    }

    pub fn manifest(&self, network: &str) -> Option<&AdapterManifest> {
        self.manifests.iter().find(|m| m.id == network)
    }

    /// Enable `adapter`: attach it under its network id and spawn its one recv task.
    pub async fn enable(self: &Arc<Self>, adapter: Arc<dyn Adapter>) -> Result<(), BridgeError> {
        let id = adapter.id().to_string();
        if !valid_network_id(&id) {
            return Err(BridgeError::BadRequest(format!(
                "{id:?} is not a network id"
            )));
        }
        {
            let mut map = self.attached.write().unwrap_or_else(|e| e.into_inner());
            if map.contains_key(&id) {
                return Err(BridgeError::BadRequest(format!("{id} is already enabled")));
            }
            let att = Arc::new(Attached {
                adapter: Arc::clone(&adapter),
                task: Mutex::new(None),
                dropped: AtomicU64::new(0),
                echoes: AtomicU64::new(0),
                link: Mutex::new(None),
                woken: AtomicBool::new(true),
                convs: Mutex::new(HashMap::new()),
            });
            let task = tokio::spawn(recv_loop(
                Arc::downgrade(self),
                Arc::clone(&adapter),
                self.cfg.recv_batch,
            ));
            *lock(&att.task) = Some(task);
            map.insert(id, att);
        }
        self.publish_networks().await;
        Ok(())
    }

    /// Disable `network`: abort its recv task and drop the adapter. Rows handed and not yet
    /// answered keep their pending mark ("may have been sent", §4); rows waiting for a
    /// `Later` retry wait for the next enable. Returns whether it was enabled.
    pub async fn disable(&self, network: &str) -> bool {
        let att = self
            .attached
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(network);
        let Some(att) = att else { return false };
        if let Some(t) = lock(&att.task).take() {
            t.abort();
        }
        state::forget_in_flight(self, network);
        self.publish_networks().await;
        true
    }

    /// The enabled networks.
    pub fn enabled(&self) -> Vec<String> {
        self.attached
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// Recv tasks alive (one per enabled adapter; none with nothing enabled).
    pub fn recv_tasks(&self) -> usize {
        self.attached
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|a| lock(&a.task).as_ref().is_some_and(|t| !t.is_finished()))
            .count()
    }

    /// The enabled adapter of `network`.
    pub fn adapter(&self, network: &str) -> Option<Arc<dyn Adapter>> {
        self.attached(network).map(|a| Arc::clone(&a.adapter))
    }

    pub(super) fn attached(&self, network: &str) -> Option<Arc<Attached>> {
        self.attached
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(network)
            .cloned()
    }

    pub(super) fn attached_all(&self) -> Vec<(String, Arc<Attached>)> {
        self.attached
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect()
    }

    /// The monotonic tick (the only clock the core reads).
    pub fn tick(&self) -> u64 {
        self.tick.load(Ordering::SeqCst)
    }

    /// Recv batches handed to the fabric so far (diagnostics, tests).
    pub fn ingest_batches(&self) -> u64 {
        self.ingest_batches.load(Ordering::SeqCst)
    }

    /// Inbound items refused as malformed (noise, §15.9).
    pub fn noise(&self) -> u64 {
        self.noise.load(Ordering::SeqCst)
    }

    /// Own echoes dropped on `network` (§8 guard 2).
    pub fn echoes(&self, network: &str) -> u64 {
        self.attached(network)
            .map_or(0, |a| a.echoes.load(Ordering::SeqCst))
    }

    /// The last link state `network`'s adapter reported (`None`: none yet, or disabled).
    pub fn link(&self, network: &str) -> Option<LinkState> {
        self.attached(network).and_then(|a| *lock(&a.link))
    }

    /// Texts the adapter of `network` dropped while 111 was busy (§4).
    pub fn dropped(&self, network: &str) -> u64 {
        self.attached(network)
            .map_or(0, |a| a.dropped.load(Ordering::SeqCst))
    }

    /// One pass of the core, driven by the daemon's tick: send events, relay edges,
    /// delivery. With no adapter enabled it returns at once (no task, no fabric call, no
    /// allocation).
    pub async fn tick_once(&self) {
        if self
            .attached
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
        {
            return;
        }
        let Ok(_pass) = self.gate.try_lock() else {
            return;
        };
        self.tick.fetch_add(1, Ordering::SeqCst);
        state::drain_events(self).await;
        super::edges::relay_pass(self).await;
        state::deliver_pass(self).await;
    }

    // -- helpers shared by the other modules --------------------------------

    /// The running fabric.
    pub(super) fn fab(&self) -> Result<Fabric, BridgeError> {
        self.handle
            .fabric()
            .cloned()
            .ok_or(BridgeError::Fabric(FabricError::Stopped))
    }

    /// The registered TAGs (§8): every manifest's and every enabled adapter's.
    pub(super) fn tags(&self) -> Vec<String> {
        let mut t: Vec<String> = self.manifests.iter().map(|m| m.tag.clone()).collect();
        for (_, a) in self.attached_all() {
            let tag = a.adapter.caps().copy.tag;
            if super::marker::valid_tag(&tag) && !t.contains(&tag) {
                t.push(tag);
            }
        }
        t
    }

    /// The TAG of `network` (manifest first).
    pub(super) fn tag_of(&self, network: &str) -> Option<String> {
        if let Some(m) = self.manifest(network) {
            return Some(m.tag.clone());
        }
        self.adapter(network)
            .map(|a| a.caps().copy.tag)
            .filter(|t| super::marker::valid_tag(t))
    }

    /// The label of `network` (manifest first).
    pub(super) fn label_of(&self, network: &str) -> String {
        if let Some(m) = self.manifest(network) {
            return m.label.clone();
        }
        self.adapter(network)
            .map(|a| a.caps().copy.label)
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| network.to_string())
    }

    /// Is `network` known (a manifest or an enabled adapter)?
    pub(super) fn known(&self, network: &str) -> bool {
        self.manifest(network).is_some() || self.attached(network).is_some()
    }

    /// The live facts of `network` (none while it is not enabled).
    pub(super) fn facts_of(&self, network: &str) -> Option<AdapterFacts> {
        self.adapter(network).map(|a| a.facts())
    }

    /// The text cap of a conversation: its kind and mode (licensed account) on the live
    /// adapter caps, the manifest's when not enabled. `None`: the carrier states none.
    pub(super) fn cap_for(
        &self,
        network: &str,
        kind: ConvKind,
        account: &str,
        caps: Option<&Caps>,
        facts: Option<&AdapterFacts>,
    ) -> Option<usize> {
        let licensed = facts
            .and_then(|f| f.account(account))
            .is_some_and(|a| a.licensed);
        if licensed {
            let l = facts
                .and_then(|f| f.licensed_caps)
                .and_then(|p| p.get(kind))
                .or_else(|| self.manifest(network).and_then(|m| m.licensed.get(kind)));
            if l.is_some() {
                return l;
            }
        }
        match caps {
            Some(c) => c.max_text_bytes.get(kind),
            None => self.manifest(network).and_then(|m| m.limits.get(kind)),
        }
    }

    /// One conversation row.
    pub(super) async fn conv_row(&self, id: i64) -> Result<ForeignConversationRow, BridgeError> {
        match self.fab()?.foreign_conversation(id).await {
            Ok(c) => Ok(c),
            Err(FabricError::NotFound) => Err(BridgeError::NotFound("conversation")),
            Err(e) => Err(e.into()),
        }
    }

    /// The conversation id of `conv` on `att` (upserted; cached per adapter).
    async fn conv_id(&self, att: &Attached, conv: &ConvRef) -> Option<i64> {
        if let Some(id) = lock(&att.convs).get(conv) {
            return Some(*id);
        }
        let account = ingest::native(&conv.account).unwrap_or_default();
        let native_id = ingest::native(&conv.native_id)?;
        let id = self
            .fab()
            .ok()?
            .foreign_conversation_upsert(
                &conv.network,
                &account,
                store_kind(conv.kind),
                &native_id,
                None,
            )
            .await
            .ok()?;
        let mut cache = lock(&att.convs);
        if cache.len() >= CONV_CACHE {
            cache.clear();
        }
        cache.insert(conv.clone(), id);
        Some(id)
    }

    /// Publish an SSE frame (no-op once the API is gone).
    pub(super) fn publish(&self, name: &str, data: &Value) {
        if let Some(api) = self.api.upgrade() {
            api.events.publish(name, data);
        }
    }

    /// Publish the `bridge` frame with the networks snapshot now (C1: the routes call it
    /// after an optional component changed state, which the core does not see itself).
    pub async fn publish_snapshot(&self) {
        self.publish_networks().await;
    }

    /// Publish the `bridge` frame with the networks snapshot.
    pub(super) async fn publish_networks(&self) {
        if let Ok(v) = self.networks().await {
            self.publish("bridge", &json!({ "networks": v["networks"] }));
        }
    }

    // -- the recv side -------------------------------------------------------

    /// Apply one recv batch of `network` (module doc): ≤ 256 items, one ingest call.
    async fn ingest_items(&self, network: &str, items: Vec<Inbound>) {
        let Some(att) = self.attached(network) else {
            return;
        };
        let facts = att.adapter.facts();
        let mut texts = Vec::new();
        let mut changed = false;
        for item in items {
            match item {
                Inbound::Text(t) => texts.push(t),
                Inbound::Conversation { conv, title } => {
                    if conv.network != network {
                        self.noise.fetch_add(1, Ordering::SeqCst);
                        continue;
                    }
                    let (Some(account), Some(native_id)) = (
                        ingest::native(&conv.account),
                        ingest::native(&conv.native_id),
                    ) else {
                        self.noise.fetch_add(1, Ordering::SeqCst);
                        continue;
                    };
                    let title = ingest::name(&title);
                    if let Ok(f) = self.fab() {
                        if let Ok(id) = f
                            .foreign_conversation_upsert(
                                network,
                                &account,
                                store_kind(conv.kind),
                                &native_id,
                                title.as_deref(),
                            )
                            .await
                        {
                            let mut cache = lock(&att.convs);
                            if cache.len() >= CONV_CACHE {
                                cache.clear();
                            }
                            cache.insert(conv, id);
                        }
                    }
                    att.woken.store(true, Ordering::SeqCst);
                }
                Inbound::Nearby(v) => {
                    // a heard announce wakes `Later` rows (set before anything observes it)
                    att.woken.store(true, Ordering::SeqCst);
                    let sink = lock(&self.nearby).clone();
                    if let Some(sink) = sink {
                        sink(network, v);
                    }
                }
                Inbound::Link(s) => {
                    if s == LinkState::Up {
                        att.woken.store(true, Ordering::SeqCst);
                    }
                    let prev = lock(&att.link).replace(s);
                    changed |= prev != Some(s);
                }
                Inbound::Dropped { n } => {
                    att.dropped.fetch_add(n, Ordering::SeqCst);
                    changed = true;
                }
            }
        }
        if !texts.is_empty() {
            let mut batch = Vec::with_capacity(texts.len());
            for t in &texts {
                if t.conv.network != network {
                    self.noise.fetch_add(1, Ordering::SeqCst);
                    continue;
                }
                let ours = ingest::is_own_echo(t, &facts.accounts, &lock(&self.echo));
                if ours {
                    att.echoes.fetch_add(1, Ordering::SeqCst);
                    continue;
                }
                let Some(conv) = self.conv_id(&att, &t.conv).await else {
                    self.noise.fetch_add(1, Ordering::SeqCst);
                    continue;
                };
                match ingest::prepare(t, conv, facts.ids_globally_unique) {
                    Some(row) => batch.push(row),
                    None => {
                        self.noise.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
            if !batch.is_empty() {
                self.ingest_batches.fetch_add(1, Ordering::SeqCst);
                if let Err(e) = self.handle.foreign_ingest(batch).await {
                    eprintln!("111 bridge: {network}: ingest failed: {e}");
                }
            }
        }
        if changed {
            self.publish_networks().await;
        }
    }

    // -- views ---------------------------------------------------------------

    /// Every network id the view lists: the manifests, then enabled adapters without one.
    pub fn network_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.manifests.iter().map(|m| m.id.clone()).collect();
        for n in self.enabled() {
            if !ids.contains(&n) {
                ids.push(n);
            }
        }
        ids
    }

    /// `GET /networks`: `{schema, networks: [Network]}` (§10).
    pub async fn networks(&self) -> Result<Value, BridgeError> {
        let f = self.fab()?;
        let convs = f.foreign_conversations(None).await?;
        let edges = f.foreign_edge_list().await?;
        let net_of: HashMap<i64, &str> = convs.iter().map(|c| (c.id, c.network.as_str())).collect();
        let components = match self.components.get() {
            Some(c) => c.list().await,
            None => Vec::new(),
        };
        let mut out = Vec::new();
        for id in self.network_ids() {
            let mine: Vec<&ForeignConversationRow> =
                convs.iter().filter(|c| c.network == id).collect();
            let edges_here = edges
                .iter()
                .filter(|e| {
                    net_of.get(&e.src_conv) == Some(&id.as_str())
                        || net_of.get(&e.dst_conv) == Some(&id.as_str())
                })
                .count();
            let component = components
                .iter()
                .find(|s| s.adapter == id)
                .and_then(|s| serde_json::to_value(s).ok())
                .map(|mut v| {
                    if let Value::Object(o) = &mut v {
                        o.remove("title");
                    }
                    v
                });
            out.push(self.network_json(&id, &mine, edges_here, component));
        }
        Ok(json!({ "schema": 1, "networks": out }))
    }

    fn network_json(
        &self,
        id: &str,
        convs: &[&ForeignConversationRow],
        edges: usize,
        component: Option<Value>,
    ) -> Value {
        let m = self.manifest(id);
        let att = self.attached(id);
        let caps = att.as_ref().map(|a| a.adapter.caps());
        let facts = att.as_ref().map(|a| a.adapter.facts());
        let (state, reason_code): (&str, Option<String>) = match (&att, &caps) {
            // the compiled-in adapter runs while service `meshtastic` is LIVE (a board bound)
            (None, _) | (_, None) if compiled_in(id) => ("not_set", Some("no_board".into())),
            (None, _) | (_, None) => ("disabled", None),
            (Some(a), Some(c)) => match (&*lock(&a.link), &c.state) {
                (Some(LinkState::Down), _) => ("error", Some("adapter_down".into())),
                (Some(LinkState::Partitioned), _) => ("error", Some("partitioned".into())),
                (_, AdapterState::Live)
                    if facts.as_ref().and_then(|f| f.region_set) == Some(false) =>
                {
                    ("not_set", Some("region_unset".into()))
                }
                (_, AdapterState::Live) => ("live", None),
                (_, AdapterState::NotSet) => ("not_set", Some("not_set".into())),
                (_, AdapterState::Error(why)) => ("error", Some(reason_slug(why))),
                (_, AdapterState::Disabled) => ("disabled", None),
            },
        };
        let reason = reason_code.as_deref().map(network_reason);
        let label = self.label_of(id);
        let chip = m
            .map(|m| m.chip.clone())
            .or_else(|| caps.as_ref().map(|c| c.copy.chip.clone()))
            .unwrap_or_else(|| id.to_string());
        let tag = self.tag_of(id).unwrap_or_default();
        let auth_levels: Vec<Auth> = match &caps {
            Some(c) => c.auth_levels.clone(),
            None => m
                .map(|m| m.auth.iter().map(|(a, _)| *a).collect())
                .unwrap_or_default(),
        };
        let mut auth_copy = serde_json::Map::new();
        for a in &auth_levels {
            let text = m
                .and_then(|m| m.auth.iter().find(|(x, _)| x == a).map(|(_, t)| t.clone()))
                .unwrap_or_else(|| default_auth_copy(*a).to_string());
            if let Ok(Value::String(k)) = serde_json::to_value(a) {
                auth_copy.insert(k, Value::String(text));
            }
        }
        let (banner, send_warning) = match (m, &caps) {
            (Some(m), _) => (m.banner.clone(), m.send_warning.clone()),
            (None, Some(c)) => (c.copy.banner.clone(), c.copy.send_warning.clone()),
            (None, None) => (String::new(), String::new()),
        };
        let kinds: Vec<ConvKind> = match (&caps, m) {
            (Some(c), _) => c.kinds.clone(),
            (None, Some(m)) => m.kinds.clone(),
            (None, None) => Vec::new(),
        };
        let mut max = serde_json::Map::new();
        for k in &kinds {
            let default = match &caps {
                Some(c) => c.max_text_bytes.get(*k),
                None => m.and_then(|m| m.limits.get(*k)),
            };
            let licensed = facts
                .as_ref()
                .and_then(|f| f.licensed_caps)
                .and_then(|p| p.get(*k))
                .or_else(|| m.and_then(|m| m.licensed.get(*k)));
            if let Some(d) = default {
                let mut modes = serde_json::Map::new();
                modes.insert("default".into(), json!(d));
                if let Some(l) = licensed.filter(|l| *l != d) {
                    modes.insert("licensed".into(), json!(l));
                }
                max.insert(kind_str(*k).into(), Value::Object(modes));
            }
        }
        let airtime = caps.as_ref().and_then(|c| c.airtime.as_ref()).map(|a| {
            json!({
                "duty_cycle_permille": a.duty_cycle_permille,
                "bits_per_s": a.bits_per_s,
                "rule": m.and_then(|m| m.airtime_rule.clone())
                    .unwrap_or_else(|| super::fanout::airtime_rule(a)),
            })
        });
        let recipient = m
            .and_then(|m| m.recipient.clone())
            .map(|r| json!({ "label": r.label, "pattern": r.pattern, "example": r.example }))
            .or_else(|| {
                caps.as_ref()
                    .and_then(|c| c.recipient_syntax.as_ref())
                    .map(|s| json!({ "label": s.hint, "pattern": s.pattern, "example": "" }))
            });
        let evidence: Vec<ForeignMessageState> = match (&caps, m) {
            (Some(c), _) => c.evidence.clone(),
            (None, Some(m)) => m.evidence.clone(),
            (None, None) => Vec::new(),
        };
        let splittable = caps
            .as_ref()
            .map(|c| c.splittable)
            .or(m.map(|m| m.splittable))
            .unwrap_or(false);
        let accounts = facts
            .as_ref()
            .map(|f| f.accounts.clone())
            .unwrap_or_default();
        let account = accounts.first().map(|a| match &a.label {
            Some(l) => format!("{l} ({})", a.id),
            None => a.id.clone(),
        });
        let licensed: Vec<&super::AccountFacts> = accounts.iter().filter(|a| a.licensed).collect();
        let ham_confirmed = if licensed.is_empty() {
            None
        } else {
            let s = lock(&self.sections);
            Some(
                licensed
                    .iter()
                    .all(|a| s.ham_confirmed(id, &a.id, &settings::ham_text(&label, &a.id))),
            )
        };
        let gate = json!({
            "region_set": facts.as_ref().and_then(|f| f.region_set),
            "licensed": (!accounts.is_empty()).then_some(!licensed.is_empty()),
            "ham_confirmed": ham_confirmed,
            "uplink_channels": facts.as_ref().map_or(0, |f| f.channels.iter().filter(|c| c.uplink).count()),
        });
        let counts = json!({
            "conversations": convs.len(),
            "unread": convs.iter().filter(|c| c.kind == ForeignKind::Dm).map(|c| c.unread).sum::<u64>(),
            "queued": convs.iter().map(|c| c.queued).sum::<u64>(),
            "edges": edges,
        });
        json!({
            "id": id,
            "label": label,
            "chip": chip,
            "state": state,
            "reason_code": reason_code,
            "reason": reason,
            "account": account,
            "copy": {
                "banner": banner,
                "send_warning": send_warning,
                "relay_tag": tag,
                "auth": auth_copy,
                "dm_note": m.and_then(|m| m.dm_hint.clone()),
            },
            "caps": {
                "kinds": kinds,
                "max_text_bytes": max,
                "evidence": evidence,
                "airtime": airtime,
                "auth_levels": auth_levels,
                "splittable": splittable,
                "recipient_syntax": recipient,
            },
            "gate": gate,
            "counts": counts,
            "dropped": att.as_ref().map_or(0, |a| a.dropped.load(Ordering::SeqCst)),
            "component": component,
        })
    }

    /// `GET /networks/{net}/conversations`.
    pub async fn conversations(&self, network: &str) -> Result<Value, BridgeError> {
        if !self.known(network) {
            return Err(BridgeError::NotFound("network"));
        }
        let rows = self.fab()?.foreign_conversations(Some(network)).await?;
        let facts = self.facts_of(network);
        let list: Vec<Value> = rows
            .iter()
            .map(|c| fjson::foreign_conversation(c, &self.conv_live(c, facts.as_ref())))
            .collect();
        Ok(json!({ "schema": 1, "network": network, "conversations": list }))
    }

    /// `GET /networks/{net}/conversations/{conv}` (`?since_seq&limit`, 1..=500).
    pub async fn conversation(
        &self,
        network: &str,
        conv: i64,
        since_seq: Option<u64>,
        limit: usize,
    ) -> Result<Value, BridgeError> {
        let row = self.conv_row(conv).await?;
        if row.network != network {
            return Err(BridgeError::NotFound("conversation"));
        }
        let page = self
            .fab()?
            .foreign_thread(conv, since_seq, limit.clamp(1, 500))
            .await?;
        let facts = self.facts_of(network);
        let live = self.conv_live(&page.conversation, facts.as_ref());
        let h = Arc::clone(&self.handle);
        let mut at_of = |id: i64| h.foreign_at_of(id);
        Ok(fjson::foreign_thread(
            &page.conversation,
            &live,
            &page.messages,
            page.more,
            &mut at_of,
        ))
    }

    /// `POST /networks/{net}/conversations` `{kind: dm, native_id}`: open (or find) a
    /// foreign DM from this node's first account. Idempotent. The address is validated
    /// against the network's recipient syntax; a foreign DM is never a 111 contact.
    pub async fn open_dm(&self, network: &str, native_id: &str) -> Result<Value, BridgeError> {
        if !self.known(network) {
            return Err(BridgeError::NotFound("network"));
        }
        let Some(adapter) = self.adapter(network) else {
            return Err(BridgeError::Unavailable(format!(
                "Enable {} first.",
                self.label_of(network)
            )));
        };
        if native_id.is_empty()
            || native_id.len() > 128
            || ingest::clean(native_id) != native_id
            || native_id.chars().any(char::is_whitespace)
        {
            return Err(BridgeError::BadRequest("that is not an address".into()));
        }
        let syntax = self
            .manifest(network)
            .and_then(|m| m.recipient.as_ref().map(|r| r.pattern.clone()))
            .or_else(|| adapter.caps().recipient_syntax.map(|s| s.pattern));
        if let Some(p) = syntax {
            if !ingest::syntax_match(&p, native_id) {
                return Err(BridgeError::BadRequest(format!(
                    "{native_id:?} is not a {} address",
                    self.label_of(network)
                )));
            }
        }
        if !adapter.caps().kinds.contains(&ConvKind::Dm) {
            return Err(BridgeError::BadRequest(format!(
                "{} has no direct messages",
                self.label_of(network)
            )));
        }
        let facts = adapter.facts();
        let Some(account) = facts.accounts.first().map(|a| a.id.clone()) else {
            return Err(BridgeError::Unavailable(
                "This network has no account of this node yet.".into(),
            ));
        };
        let f = self.fab()?;
        let id = f
            .foreign_conversation_upsert(
                network,
                &account,
                ForeignKind::Dm,
                &format!("dm:{native_id}"),
                None,
            )
            .await?;
        let row = f.foreign_conversation(id).await?;
        Ok(fjson::foreign_conversation(
            &row,
            &self.conv_live(&row, Some(&facts)),
        ))
    }
}

/// Is `network` a compiled-in adapter (§2.1 rule 2: Meshtastic is the one exception to
/// "every third-party program is an optional component")?
pub fn compiled_in(network: &str) -> bool {
    network == super::NETWORK_MESHTASTIC
}

/// The contract spelling of a kind.
pub fn kind_str(k: ConvKind) -> &'static str {
    match k {
        ConvKind::Channel => "channel",
        ConvKind::Dm => "dm",
        ConvKind::Group => "group",
    }
}

/// A snake_case reason from an adapter's error text (fail closed to `adapter_down`).
fn reason_slug(why: &str) -> String {
    let ok = !why.is_empty()
        && why.len() <= 32
        && why
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if ok {
        why.to_string()
    } else {
        "adapter_down".to_string()
    }
}

/// One plain sentence for a network `reason_code`.
fn network_reason(code: &str) -> String {
    match code {
        "adapter_down" => "This network's adapter is not running.".into(),
        "partitioned" => {
            "The helper is not attached to this node's Reticulum instance; it retries once rnsd serves."
                .into()
        }
        "region_unset" => "The radio's region is not set, so it may not transmit yet.".into(),
        "not_set" => "This network needs a fact from you first (a board, a region).".into(),
        "no_board" => "No board is bound to this node.".into(),
        _ => "The adapter reported a condition this version does not describe.".into(),
    }
}

/// The copy of an auth level when the manifest has none.
fn default_auth_copy(a: Auth) -> &'static str {
    match a {
        Auth::None => "nothing vouches for the sender's name",
        Auth::ChannelKey => "anyone with this channel key can use any name",
        Auth::Pki => "the board's PKI proved the sender key, not the name",
        Auth::Signed => "the carrier validated the sender's signature; the name is self-declared",
        Auth::Attested111 => "attested by a 111 node (reserved)",
    }
}

/// The recv task of one adapter (module doc).
async fn recv_loop(me: Weak<Bridge>, adapter: Arc<dyn Adapter>, batch_max: usize) {
    let network = adapter.id();
    loop {
        let first = adapter.recv().await;
        let mut batch = Vec::with_capacity(batch_max.min(64));
        batch.push(first);
        while batch.len() < batch_max {
            match adapter.recv().now_or_never() {
                Some(item) => batch.push(item),
                None => break,
            }
        }
        let Some(bridge) = me.upgrade() else { return };
        bridge.ingest_items(network, batch).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_manifests_parse_including_the_t0_example() {
        let m = embedded_manifests();
        let ids: Vec<&str> = m.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids, NETWORK_ORDER,
            "every declared adapter, in release order"
        );
        let mesh = &m[0];
        assert_eq!((mesh.tag.as_str(), mesh.chip.as_str()), ("M", "M"));
        assert_eq!(mesh.limits.channel, Some(232));
        assert_eq!(mesh.limits.dm, Some(220));
        assert_eq!(mesh.licensed.dm, Some(232));
        assert_eq!(mesh.kinds, vec![ConvKind::Channel, ConvKind::Dm]);
        assert!(mesh.uplink_warning.is_some());
        let tags: Vec<&str> = m.iter().map(|m| m.tag.as_str()).collect();
        assert_eq!(tags, ["M", "LX", "DC", "SX"]);
        let lx = &m[1];
        assert!(lx.recipient.is_some(), "LXMF addresses are 32 hex");
        assert!(lx.kinds.contains(&ConvKind::Group) && !lx.kinds.contains(&ConvKind::Channel));
    }

    #[test]
    fn invalid_manifests_are_not_registered() {
        let good = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/adapters/meshtastic.toml"
        ))
        .unwrap();
        let good = String::from_utf8(good).unwrap();
        assert!(parse_manifest("meshtastic.toml", good.as_bytes()).is_ok());
        assert!(
            parse_manifest("other.toml", good.as_bytes()).is_err(),
            "stem"
        );
        let unknown = format!("{good}\nsurprise = 1\n");
        assert!(parse_manifest("meshtastic.toml", unknown.as_bytes()).is_err());
        let bad_tag = good.replace("tag = \"M\"", "tag = \"M1\"");
        assert!(parse_manifest("meshtastic.toml", bad_tag.as_bytes()).is_err());
        let bad_auth = good.replace("pki =", "telepathy =");
        assert!(parse_manifest("meshtastic.toml", bad_auth.as_bytes()).is_err());
        let huge = vec![b'#'; MAX_MANIFEST_BYTES + 1];
        assert!(parse_manifest("meshtastic.toml", &huge).is_err());
        let dup = load_manifests(&[
            ("meshtastic.toml".into(), good.clone().into_bytes()),
            ("meshtastic.toml".into(), good.into_bytes()),
        ]);
        assert_eq!(dup.len(), 1, "a duplicate is left out");
    }

    #[test]
    fn errors_map_onto_contract_problems() {
        let b = BridgeError::Oversize("312 B > 232 B per packet".into())
            .problem()
            .body();
        assert_eq!(
            (b["status"].as_u64(), b["code"].as_str()),
            (Some(409), Some("oversize"))
        );
        let c = ConsentRequired {
            action: "edge_enable",
            subject: "fc-1>fc-2".into(),
            warning_text: "text".into(),
            hash: settings::sha256_hex("text"),
            addons: vec![],
        };
        let b = BridgeError::Consent(c).problem().body();
        assert_eq!(b["code"], "consent_required");
        assert_eq!(b["consent"]["hash"], settings::sha256_hex("text"));
        let b = BridgeError::Refused {
            code: "oversize",
            targets: vec![json!({"code": "oversize"})],
        }
        .problem()
        .body();
        assert_eq!(b["targets"][0]["code"], "oversize");
        let b = BridgeError::NotFound("conversation").problem().body();
        assert_eq!(b["status"], 404);
        let b = BridgeError::Fabric(FabricError::Invalid("x"))
            .problem()
            .body();
        assert_eq!(b["code"], "bad_request");
        let b = BridgeError::Unavailable("off".into()).problem().body();
        assert_eq!(b["code"], "component_unavailable");
    }
}
