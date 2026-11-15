//! Layer 7 §13.1, §6.3, §12 (row D10): the relay status, its settings and their consents. Not
//! `api_v1/relay.rs`, which is Layer 4's agent relay.
//!
//! Routes (contract `getRelay`, `patchRelaySettings`, `checkRelayCarrier`, `flushRelayCarrier`):
//!
//! - `GET /api/v1/relay` → `RelayStatus`: every Layer 7 setting (row D1's view: the carrier's
//!   secrets only as `{set}`), the `gateway` badge (row D3's facts), the paired hubs (the fabric's
//!   records, the daemon's halves and the per-pair views, row D6's `paired_hub_json`), the forward
//!   counters (`forward_refused`, `forward_too_large`), the `rns` heartbeat row from Layer 6's
//!   setting (`owner: layer6`, §2.4 item 2), `origin.bytes` (row D7). A session or loopback; the
//!   agent's bearer reads it (no secret is in it). It reads one fabric status and the settings
//!   held in memory: **no task is started and no file is written on a GET**.
//! - `PATCH /api/v1/relay/settings` → `200 RelayStatus` (a human; the agent's bearer is refused
//!   by the global write gate, `401`). A partial merge of `RelaySettingsPatch` (the carrier keys
//!   flat: `carrier.{kind, server, mode, token, topic_secret}`; a secret is a write-only string,
//!   `null` clears it). The order: the body's shape (`400`); a heartbeat write (§2.4 item 2:
//!   `409 settings_locked` for `rns`, its `detail` naming `PATCH /api/v1/bridge/settings`, else
//!   `409 no_such_transport`, through row D2's validator; nothing written); each value through
//!   D1's setters (`400`; `5B` on a Termux hub `409 bad_request`; `ntfy` without a server `400`);
//!   the refusals (`gateway.enabled` or `origin.serve` on a node without the hub role `409
//!   needs_hub`; `origin.serve` without LAN sharing `409 lan_off`; an `origin.targets` list
//!   without this node's own target `409 bad_request`, whose `detail` names it); then the
//!   consents (below); then one write of `state/relay.json`, and after it the gateway row
//!   (`gateway::apply`, D3), the role bits and the offer's gateway fact
//!   (`FabricHandle::set_relay_settings`, D9), the DNS-SD record when `discovery.lan_record`
//!   moved (D8), the served blobs when `origin.targets` moved (`Origin::retain_targets`, D7), the
//!   carrier hook (`reconfigure`, D11) and the `relay` frame.
//! - `POST /api/v1/relay/carrier/check`, `/flush` → `202 RelayAccepted` (a human; bearer `401`):
//!   `409 bad_request` with no carrier configured, `409 consent_required` while the recorded
//!   `carrier_enable` hash is not the hash of the text owed now (the human accepts through `PATCH
//!   /relay/settings`, below), `409 metered_cap`, `409 licensed_board`; then the request goes to
//!   the registered carrier hook (`relay::carrier_hook`). Until row D11 registers the carrier's
//!   own, the no-op hook records the request and starts nothing (`started: false`).
//!
//! **Consents (§12; I2), in the pinned round trip.** Three gates, asked one per round in this
//! order: `gateway_bit` (turning `discovery.public_gateway_bit` on, or, while it is on and its
//! consent is owed, any write to `discovery`), `origin_serve` (turning `origin.serve` on, or,
//! while it is on and its consent is owed, any write to `origin`), `carrier_enable` (subject
//! `<kind>:<server>`; the carrier turned on or its server set, or a metered cap set or raised,
//! on any link, while the carrier is on: the text names every metered link's match and cap, and
//! `metered` is in `addons`). The texts and the hash rule are `relay::consent`'s. A gate is
//! owed when the recorded hash is not the hash of its text now (role bits 3 and 5 read only a
//! consent that holds, `consent::holds`, so one recorded under an older text is off there);
//! the request carries `accept_warning` = the hash to pass it, and the hash is recorded with the
//! write. When a round passes one gate and the next is owed, the hash it passed is recorded
//! (`gateway_bit` and `origin_serve` only: their texts never change with the request) and the
//! next gate answers, so a request with two gates converges (a hash recorded so for a switch
//! already on turns its role bit on: the fabric follows the file then too, and a frame is
//! published when the announce moved); `carrier_enable` is asked last and
//! recorded only with the write itself, so its hash always names the server written beside it.
//! Turning a switch off forgets its hash: the copy is shown again before it turns on (§6.2).
//! Caps lowered or removed keep a valid carrier consent valid (the hash of the new text is
//! recorded with the write; §12 re-asks only for a cap set or raised).
//!
//! **A write the file already holds.** When the write changes nothing in `state/relay.json`,
//! every follower still runs, each idempotent (the gateway row, `set_relay_settings`, a no-op
//! when the announce carries them, the DNS-SD record, the served blobs, the carrier hook): a
//! re-mint that failed put the old switches back (row D4b), and a write cut short after the file
//! (the daemon stopping part-way) left the rest as before, so "Save again to retry" (and a relay
//! proposal decided again) reconciles them. A frame follows only when the announce moved. The
//! `PATCH` runs in a task of its own, so a client that goes away mid-write cuts nothing short.
//!
//! **The lead's rulings (D33 candidates).** (a) A `PATCH` whose `accept_warning` is the hash of
//! the `carrier_enable` text owed now for the carrier the node holds records that consent even
//! when the body changes no stored value (Check now and Flush now carry no `accept_warning`); a
//! hash that is the hash of no text owed or recorded now answers `409 consent_required` with the
//! current text. (b) An `origin.targets` list without this node's own target is refused (`409
//! bad_request`, the detail names the target). (c) A paired hub's `tls` is [`tls_of`]: `{via:
//! tcp, lan: false, tls: none}` is a pair met at a private address, which runs plain `tcp-lan`
//! by §4.1 rule 1 (no TLS, no warning, no owner mark); `plain` only with a recorded warning.
//!
//! **The `relay` frame (§13.1).** A full `RelayStatus`, published after every write here, after a
//! relay proposal's acceptance, when another module calls `relay::changed()` (the carrier, D11),
//! and when a fabric event may have moved what the status shows: a `Forward` event, or a `Fabric`
//! event on a node that holds paired hubs or whose pair set moved (`pair_generation`). Builds are
//! coalesced (one in flight, one more queued, in a task that ends when nothing is queued); a
//! fabric-driven build publishes only when the status differs from the last frame in more than
//! its ages; a build begun before a write never publishes after that write's frame (a write
//! generation, [`LastFrame::admit`]). No timer, no resident task: a node with no pair and no write publishes nothing.
//!
//! **Secrets (§6.3, §13.1).** The token and the topic secret are moved into zeroizing buffers
//! when the body is parsed, written only into `state/relay.json`, and never echoed, logged or
//! framed: every problem here names a key, never a value.

use super::{ApiRouter, ApiState, Problem};
use crate::daemon::fabric::settings::PairLink;
use crate::daemon::fabric::FabricHandle;
use crate::daemon::origin::Origin;
use crate::daemon::relay::consent::{self, ConsentRequired, ProblemWith};
use crate::daemon::relay::presence::{self, BeatContext, HeartbeatRefusal, RNS_TRANSPORT};
use crate::daemon::relay::settings::{
    self as rs, CarrierKind, CarrierMode, ConsentKind, MeteredLink, RelaySettings, SettingsError,
};
use crate::daemon::relay::{self, gateway, pair};
use crate::daemon::updater::fetch::TARGET_TRIPLE;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::Json;
use mesh111::fabric::{FabricEvent, PairVia};
use serde_json::{json, Map, Value};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};
use zeroize::Zeroizing;

/// `RelayStatus.schema` and `RelayAccepted.schema`.
pub const SCHEMA: u32 = 1;

/// The Layer 7 routes of this file (§13.1).
pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/relay", get(get_relay))
        .route("/api/v1/relay/settings", patch(patch_settings))
        .route("/api/v1/relay/carrier/check", post(carrier_check))
        .route("/api/v1/relay/carrier/flush", post(carrier_flush))
}

// ---------------------------------------------------------------- the settings cell

/// The relay settings as this daemon last read or wrote them (`ApiState` ext): read once at
/// `relay::start` (row D1's `load`), replaced after every write here. The routes read this, never
/// the file, so a GET reads no file and writes none (an invalid file is moved aside by the load at
/// start). The drain cursors and the metered window, which the carrier (D11) writes itself, are
/// as of the last write here: D11 reads them from the file, or [`RelayCell::replace`]s the cell
/// after its own write.
pub struct RelayCell {
    settings: RwLock<RelaySettings>,
    /// Serialises every read-plan-write of the settings (a PATCH, a relay proposal).
    write: Arc<tokio::sync::Mutex<()>>,
    /// The last `relay` frame: its status without its ages (the fabric-driven builds compare)
    /// and the write generation its build began at.
    last_frame: Mutex<LastFrame>,
    /// The write generation: bumped by every write before it publishes its frame, so a build
    /// begun before a write never publishes after that write's frame ([`publish`]).
    writes: AtomicU64,
}

/// The last published `relay` frame (module doc; [`LastFrame::admit`]).
#[derive(Debug, Default)]
struct LastFrame {
    /// The write generation the last published build began at.
    generation: u64,
    /// Its status without its ages.
    status: Option<Value>,
}

impl LastFrame {
    /// Whether a build begun at write generation `generation`, whose status without ages is
    /// `status`, publishes (and, if so, it becomes the last frame): never when a build begun
    /// after a newer write has published (its status may predate that write); unless `force`,
    /// only when the status differs from the last frame's.
    fn admit(&mut self, generation: u64, status: Value, force: bool) -> bool {
        if generation < self.generation {
            return false;
        }
        if !force && self.status.as_ref() == Some(&status) {
            return false;
        }
        self.generation = generation;
        self.status = Some(status);
        true
    }
}

impl RelayCell {
    fn load(api: &ApiState) -> RelayCell {
        RelayCell {
            settings: RwLock::new(rs::load(&api.layout.state_dir()).settings),
            write: Arc::new(tokio::sync::Mutex::new(())),
            last_frame: Mutex::new(LastFrame::default()),
            writes: AtomicU64::new(0),
        }
    }

    /// The settings held now (a copy).
    pub fn settings(&self) -> RelaySettings {
        self.settings
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Hold `s` as the settings (after a write of `state/relay.json` outside these routes).
    pub fn replace(&self, s: RelaySettings) {
        *self.settings.write().unwrap_or_else(|p| p.into_inner()) = s;
    }
}

/// The daemon's settings cell (`ApiState` ext `Arc<RelayCell>`), loaded at the first use
/// (`relay::start` makes that first use at daemon start).
pub fn cell(api: &ApiState) -> Arc<RelayCell> {
    api.ext_or_insert_with(|| Arc::new(RelayCell::load(api)))
}

// ---------------------------------------------------------------- problems

fn bad_request(detail: impl Into<String>) -> Problem {
    Problem::new(
        StatusCode::BAD_REQUEST,
        "bad_request",
        "The request was refused.",
    )
    .detail(detail)
}

fn conflict(code: &'static str, title: &str, detail: impl Into<String>) -> Problem {
    Problem::new(StatusCode::CONFLICT, code, title).detail(detail)
}

fn needs_hub(what: &str) -> Problem {
    conflict(
        "needs_hub",
        "Only a hub does this.",
        format!(
            "{what} needs the hub role: turn on \"Let other nodes use this one as their home \
             hub\" (Settings → Messaging & Data) first."
        ),
    )
}

fn internal(detail: impl Into<String>) -> Problem {
    Problem::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
        "The relay settings could not be saved.",
    )
    .detail(detail)
}

/// A setter's refusal (D1): the text names the key and the rule, never the value.
fn setting_problem(e: &SettingsError) -> Problem {
    match e {
        SettingsError::ModeRefusedOnTermux => conflict(
            "bad_request",
            "5B is not available on a phone.",
            e.to_string(),
        ),
        _ => bad_request(e.to_string()),
    }
}

// ---------------------------------------------------------------- the patch (RelaySettingsPatch)

/// A parsed `RelaySettingsPatch` (§13.1): every key optional; `Some(None)` clears a nullable one.
/// The secrets are held in zeroizing buffers from the parse on; `Debug` prints only whether one
/// is present.
#[derive(Clone, Default)]
pub struct RelayPatch {
    /// `gateway.enabled`.
    pub gateway_enabled: Option<bool>,
    /// `gateway.termux_poll_min`.
    pub termux_poll_min: Option<u32>,
    /// `carrier.kind`.
    pub carrier_kind: Option<CarrierKind>,
    /// `carrier.server` (`Some(None)` clears it).
    pub server: Option<Option<String>>,
    /// `carrier.mode`.
    pub mode: Option<CarrierMode>,
    /// `carrier.token` (write-only; `Some(None)` clears it).
    pub token: Option<Option<Zeroizing<String>>>,
    /// `carrier.topic_secret` (write-only; `Some(None)` clears it).
    pub topic_secret: Option<Option<Zeroizing<String>>>,
    /// `presence.heartbeat` (every v1 write is refused, §2.4).
    pub heartbeat: Option<Vec<(String, u64)>>,
    /// `discovery.lan_record`.
    pub lan_record: Option<bool>,
    /// `discovery.public_gateway_bit`.
    pub public_gateway_bit: Option<bool>,
    /// `discovery.offload_bit`.
    pub offload_bit: Option<bool>,
    /// `origin.serve`.
    pub origin_serve: Option<bool>,
    /// `origin.targets` (replaces the list).
    pub origin_targets: Option<Vec<String>>,
    /// `updates.check_internet`.
    pub check_internet: Option<bool>,
    /// `metered` (replaces the list).
    pub metered: Option<Vec<MeteredLink>>,
    /// `accept_warning`: the hash of the consent text the human was shown.
    pub accept_warning: Option<String>,
}

impl std::fmt::Debug for RelayPatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayPatch")
            .field("gateway_enabled", &self.gateway_enabled)
            .field("termux_poll_min", &self.termux_poll_min)
            .field("carrier_kind", &self.carrier_kind)
            .field("server", &self.server)
            .field("mode", &self.mode)
            .field("token", &self.token.as_ref().map(|t| t.is_some()))
            .field(
                "topic_secret",
                &self.topic_secret.as_ref().map(|t| t.is_some()),
            )
            .field("heartbeat", &self.heartbeat)
            .field("lan_record", &self.lan_record)
            .field("public_gateway_bit", &self.public_gateway_bit)
            .field("offload_bit", &self.offload_bit)
            .field("origin_serve", &self.origin_serve)
            .field("origin_targets", &self.origin_targets)
            .field("check_internet", &self.check_internet)
            .field("metered", &self.metered)
            .field("accept_warning", &self.accept_warning)
            .finish()
    }
}

/// 64 lowercase hex characters (the contract's `Sha256Hex`).
pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn object(v: Value, key: &str) -> Result<Map<String, Value>, Problem> {
    match v {
        Value::Object(o) => Ok(o),
        _ => Err(bad_request(format!("{key} must be an object."))),
    }
}

fn boolean(v: &Value, key: &str) -> Result<bool, Problem> {
    v.as_bool()
        .ok_or_else(|| bad_request(format!("{key} must be true or false.")))
}

fn unknown(key: &str) -> Problem {
    bad_request(format!(
        "{key} is not a relay setting (RelaySettingsPatch)."
    ))
}

/// A nullable string of at most `max` characters; the error names the key only (a secret's value
/// never reaches a message).
fn nullable_string(v: Value, key: &str, max: usize) -> Result<Option<String>, Problem> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) if s.chars().count() <= max => Ok(Some(s)),
        Value::String(_) => Err(bad_request(format!(
            "{key} is longer than {max} characters."
        ))),
        _ => Err(bad_request(format!("{key} must be a string or null."))),
    }
}

/// A nullable write-only secret, moved into a zeroizing buffer at once.
fn nullable_secret(v: Value, key: &str, max: usize) -> Result<Option<Zeroizing<String>>, Problem> {
    Ok(nullable_string(v, key, max)?.map(Zeroizing::new))
}

fn parse_gateway(v: Value, p: &mut RelayPatch) -> Result<(), Problem> {
    for (k, v) in object(v, "gateway")? {
        match k.as_str() {
            "enabled" => p.gateway_enabled = Some(boolean(&v, "gateway.enabled")?),
            "termux_poll_min" => {
                let n = v.as_u64().ok_or_else(|| {
                    bad_request("gateway.termux_poll_min must be a whole number of minutes.")
                })?;
                p.termux_poll_min = Some(u32::try_from(n).unwrap_or(u32::MAX));
            }
            other => return Err(unknown(&format!("gateway.{other}"))),
        }
    }
    Ok(())
}

fn parse_carrier(v: Value, p: &mut RelayPatch) -> Result<(), Problem> {
    for (k, v) in object(v, "carrier")? {
        match k.as_str() {
            "kind" => {
                let k = v.as_str().and_then(|s| CarrierKind::parse(s).ok());
                p.carrier_kind =
                    Some(k.ok_or_else(|| bad_request("carrier.kind must be none or ntfy."))?);
            }
            "server" => {
                p.server = Some(nullable_string(v, "carrier.server", rs::MAX_SERVER_URL)?);
            }
            "mode" => {
                let m = v.as_str().and_then(|s| CarrierMode::parse(s).ok());
                p.mode = Some(m.ok_or_else(|| bad_request("carrier.mode must be 5A or 5B."))?);
            }
            "token" => p.token = Some(nullable_secret(v, "carrier.token", rs::MAX_TOKEN)?),
            "topic_secret" => {
                p.topic_secret = Some(nullable_secret(v, "carrier.topic_secret", 64)?);
            }
            other => return Err(unknown(&format!("carrier.{other}"))),
        }
    }
    Ok(())
}

fn parse_heartbeat(v: Value) -> Result<Vec<(String, u64)>, Problem> {
    let shape = "presence.heartbeat must be a list of {\"transport\": string, \"interval_min\": \
                 whole minutes}.";
    let Value::Array(items) = v else {
        return Err(bad_request(shape));
    };
    let mut out = Vec::with_capacity(items.len().min(16));
    for item in items {
        let o = object(item, "presence.heartbeat[]")?;
        let (mut t, mut m) = (None, None);
        for (k, v) in o {
            match (k.as_str(), v) {
                ("transport", Value::String(s)) => t = Some(s),
                ("interval_min", v) if v.as_u64().is_some() => m = v.as_u64(),
                _ => return Err(bad_request(shape)),
            }
        }
        match (t, m) {
            (Some(t), Some(m)) => out.push((t, m)),
            _ => return Err(bad_request(shape)),
        }
    }
    Ok(out)
}

fn parse_flags(v: Value, prefix: &str, keys: &[&str]) -> Result<Vec<(usize, bool)>, Problem> {
    let mut out = Vec::new();
    for (k, v) in object(v, prefix)? {
        let Some(i) = keys.iter().position(|x| *x == k) else {
            return Err(unknown(&format!("{prefix}.{k}")));
        };
        out.push((i, boolean(&v, &format!("{prefix}.{k}"))?));
    }
    Ok(out)
}

fn parse_origin(v: Value, p: &mut RelayPatch) -> Result<(), Problem> {
    for (k, v) in object(v, "origin")? {
        match k.as_str() {
            "serve" => p.origin_serve = Some(boolean(&v, "origin.serve")?),
            "targets" => {
                let shape = "origin.targets must be a list of target triples.";
                let Value::Array(items) = v else {
                    return Err(bad_request(shape));
                };
                let mut list = Vec::with_capacity(items.len().min(rs::MAX_TARGETS));
                for i in items {
                    match i {
                        Value::String(s) if s.len() <= rs::MAX_TARGET_LEN => list.push(s),
                        _ => return Err(bad_request(shape)),
                    }
                }
                p.origin_targets = Some(list);
            }
            other => return Err(unknown(&format!("origin.{other}"))),
        }
    }
    Ok(())
}

/// Parse a `RelaySettingsPatch` body (§13.1): an object of the contract's keys only, each of its
/// type; anything else is `400 bad_request`. Values are checked later, by D1's setters.
pub fn parse_patch(body: &[u8]) -> Result<RelayPatch, Problem> {
    let top = match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(o)) => o,
        _ => {
            return Err(bad_request(
                "Body must be a JSON object of relay settings (RelaySettingsPatch).",
            ))
        }
    };
    let mut p = RelayPatch::default();
    for (k, v) in top {
        match k.as_str() {
            "gateway" => parse_gateway(v, &mut p)?,
            "carrier" => parse_carrier(v, &mut p)?,
            "presence" => {
                for (k, v) in object(v, "presence")? {
                    match k.as_str() {
                        "heartbeat" => p.heartbeat = Some(parse_heartbeat(v)?),
                        other => return Err(unknown(&format!("presence.{other}"))),
                    }
                }
            }
            "discovery" => {
                let keys = ["lan_record", "public_gateway_bit", "offload_bit"];
                for (i, on) in parse_flags(v, "discovery", &keys)? {
                    match i {
                        0 => p.lan_record = Some(on),
                        1 => p.public_gateway_bit = Some(on),
                        _ => p.offload_bit = Some(on),
                    }
                }
            }
            "origin" => parse_origin(v, &mut p)?,
            "updates" => {
                for (_, on) in parse_flags(v, "updates", &["check_internet"])? {
                    p.check_internet = Some(on);
                }
            }
            "metered" => {
                let list: Vec<MeteredLink> = serde_json::from_value(v).map_err(|_| {
                    bad_request(
                        "metered must be a list of {\"match\": {\"iface\" | \"ssid\": string}, \
                         \"daily_cap_bytes\": whole bytes ≥ 1}.",
                    )
                })?;
                p.metered = Some(list);
            }
            "accept_warning" => match v {
                Value::String(h) if is_sha256_hex(&h) => p.accept_warning = Some(h),
                _ => {
                    return Err(bad_request(
                        "accept_warning must be the 64 lowercase hex the consent carried.",
                    ))
                }
            },
            other => return Err(unknown(other)),
        }
    }
    Ok(p)
}

// ---------------------------------------------------------------- planning a write

/// What the refusals read about this node.
#[derive(Debug, Clone, Copy)]
pub struct NodeFacts {
    /// The fabric holds the hub role (`hub_role`).
    pub hub: bool,
    /// LAN sharing is on for this daemon's lifetime.
    pub lan: bool,
    /// This node runs on Termux (5B refused; the heartbeat's phone floor).
    pub termux: bool,
}

impl NodeFacts {
    /// The facts of `api`'s daemon now (no fabric running ⇒ not a hub: fail closed).
    pub fn of(api: &ApiState) -> NodeFacts {
        NodeFacts {
            hub: fabric_of(api).is_some_and(|h| h.persisted().fabric.hub_role),
            lan: api.lan_enabled,
            termux: crate::config::on_termux(),
        }
    }
}

fn fabric_of(api: &ApiState) -> Option<Arc<FabricHandle>> {
    api.ext::<Arc<FabricHandle>>().filter(|h| h.running())
}

/// The `carrier_enable` consent the carrier `s` holds is owed now (§6.2, §12): `None` while no
/// carrier is configured (`carrier.kind` `none`, or no server). The text names every metered
/// link's match and cap (`consent::carrier_enable_text`).
pub fn carrier_consent(s: &RelaySettings) -> Option<ConsentRequired> {
    consent::carrier_enable_for(s)
}

/// The carrier `s` holds is configured and its recorded consent is the one owed now (§6.2: the
/// carrier never runs without it).
pub fn carrier_consented(s: &RelaySettings) -> bool {
    consent::holds(s, ConsentKind::CarrierEnable)
}

/// A metered cap of `next` is set or raised against `pre` (§12): a link `pre` does not list, or
/// a cap above the one `pre` holds for the same match. Lowering or removing caps is neither.
pub fn cap_set_or_raised(pre: &[MeteredLink], next: &[MeteredLink]) -> bool {
    next.iter().any(|l| {
        pre.iter()
            .find(|x| x.matcher == l.matcher)
            .is_none_or(|x| l.daily_cap_bytes > x.daily_cap_bytes)
    })
}

/// Apply the patch's values to `s` through D1's setters (the same calls for the plan and for the
/// write, so the write cannot refuse what the plan took).
fn apply_values(s: &mut RelaySettings, p: &RelayPatch, termux: bool) -> Result<(), SettingsError> {
    if let Some(on) = p.gateway_enabled {
        s.set_gateway_enabled(on);
    }
    if let Some(m) = p.termux_poll_min {
        s.set_termux_poll_min(m)?;
    }
    if let Some(k) = p.carrier_kind {
        s.set_carrier_kind(k);
        if k == CarrierKind::None {
            // §6.2: the copy is shown again before the switch turns on
            s.set_consent_hash(ConsentKind::CarrierEnable, None)?;
        }
    }
    if let Some(server) = &p.server {
        s.set_ntfy_server(server.as_deref())?;
    }
    if let Some(m) = p.mode {
        s.set_carrier_mode(m, termux)?;
    }
    if let Some(t) = &p.token {
        s.set_ntfy_token(t.clone())?;
    }
    if let Some(t) = &p.topic_secret {
        s.set_ntfy_topic_secret(t.as_deref().map(String::as_str))?;
    }
    if let Some(on) = p.lan_record {
        s.set_lan_record(on);
    }
    if let Some(on) = p.public_gateway_bit {
        s.set_public_gateway_bit(on);
        if !on {
            s.set_consent_hash(ConsentKind::GatewayBit, None)?;
        }
    }
    if let Some(on) = p.offload_bit {
        s.set_offload_bit(on);
    }
    if let Some(on) = p.origin_serve {
        s.set_origin_serve(on);
        if !on {
            s.set_consent_hash(ConsentKind::OriginServe, None)?;
        }
    }
    if let Some(t) = &p.origin_targets {
        s.set_origin_targets(t)?;
    }
    if let Some(on) = p.check_internet {
        s.set_check_internet(on);
    }
    if let Some(m) = &p.metered {
        s.set_metered(m.clone())?;
    }
    Ok(())
}

/// The three consent kinds this file records.
const CONSENT_KINDS: [ConsentKind; 3] = [
    ConsentKind::GatewayBit,
    ConsentKind::OriginServe,
    ConsentKind::CarrierEnable,
];

/// A refused plan: the problem, and the consent hashes the round passed that are recorded anyway
/// (`gateway_bit`, `origin_serve`: module doc).
#[derive(Debug)]
pub struct Refused {
    /// What the route answers (boxed: a `Result` with it stays small).
    pub problem: Box<ProblemWith>,
    /// `(kind, hash)` to record although the write is refused.
    pub record: Vec<(ConsentKind, String)>,
}

impl From<Problem> for Refused {
    fn from(p: Problem) -> Self {
        Refused {
            problem: Box::new(ProblemWith::new(p)),
            record: Vec::new(),
        }
    }
}

/// One owed consent gate of a plan.
struct Gate {
    kind: ConsentKind,
    consent: ConsentRequired,
    /// The request turns this switch on or touches its section (`discovery` for `gateway_bit`,
    /// `origin` for `origin_serve`: a consent recorded under another text is asked again there),
    /// or, for the carrier, sets its kind or server, or sets or raises any metered cap: only the
    /// human's `accept_warning` passes it.
    material: bool,
    /// Passed: by the request's `accept_warning`, or (a cap lowered under a valid carrier
    /// consent) renewed.
    passed: bool,
}

/// Plan the write of `p` onto `pre` (§13.1; module doc for the order): the settings to write, or
/// the refusal. Writes nothing.
pub fn plan(pre: &RelaySettings, p: &RelayPatch, cx: &NodeFacts) -> Result<RelaySettings, Refused> {
    // §2.4 item 2: every heartbeat write is refused first; `rns` is Layer 6's setting
    if let Some(hb) = p.heartbeat.as_ref().filter(|h| !h.is_empty()) {
        let bx = BeatContext {
            offered: &presence::BEAT_TRANSPORTS_V1,
            phone: cx.termux,
            ..BeatContext::default()
        };
        let order = hb
            .iter()
            .filter(|(t, _)| t == RNS_TRANSPORT)
            .chain(hb.iter().filter(|(t, _)| t != RNS_TRANSPORT));
        for (t, m) in order {
            if let Err(r) = presence::check_heartbeat(t, u32::try_from(*m).unwrap_or(u32::MAX), &bx)
            {
                return Err(heartbeat_problem(r).into());
            }
        }
        // no transport takes a beat in v1, so nothing reaches here; fail closed all the same
        return Err(HeartbeatRefusal::NoSuchTransport.problem().into());
    }
    // the values (D1's setters)
    let mut next = pre.clone();
    apply_values(&mut next, p, cx.termux).map_err(|e| setting_problem(&e))?;
    if next.carrier_kind() == CarrierKind::Ntfy && next.ntfy_server().is_none() {
        return Err(bad_request(
            "carrier.server is needed with carrier.kind ntfy: an https:// URL (http:// only for \
             a private address).",
        )
        .into());
    }
    // the refusals
    if p.gateway_enabled == Some(true) && !cx.hub {
        return Err(needs_hub("Turning the gateway on").into());
    }
    if p.origin_serve == Some(true) {
        if !cx.hub {
            return Err(needs_hub("Serving update files").into());
        }
        if !cx.lan {
            return Err(conflict(
                "lan_off",
                "LAN sharing is off.",
                "Serving update files to this network needs LAN sharing: turn it on first \
                 (Settings → Dashboard).",
            )
            .into());
        }
    }
    if let Some(t) = &p.origin_targets {
        if !t.iter().any(|x| x == TARGET_TRIPLE) {
            return Err(conflict(
                "bad_request",
                "This hub's own target stays.",
                format!(
                    "origin.targets must keep {TARGET_TRIPLE}: this hub holds and serves its \
                     own release first (§9.1). Remove the others only."
                ),
            )
            .into());
        }
    }
    // the consents: gateway_bit, origin_serve, carrier_enable (module doc)
    let mut gates: Vec<Gate> = Vec::new();
    let owed = |next: &RelaySettings, kind: ConsentKind, c: &ConsentRequired| {
        next.consent_hash(kind) != Some(c.hash.as_str())
    };
    // a gate owed on a switch that is on (no consent, or one recorded under another text) is
    // material when the request turns the switch on or touches its section, so a stale consent
    // is asked again on the next write there (role bits 3 and 5 read only a current one, D9)
    let discovery =
        p.lan_record.is_some() || p.public_gateway_bit.is_some() || p.offload_bit.is_some();
    let origin = p.origin_serve.is_some() || p.origin_targets.is_some();
    if next.public_gateway_bit() {
        let c = consent::gateway_bit();
        if owed(&next, ConsentKind::GatewayBit, &c) {
            gates.push(Gate {
                kind: ConsentKind::GatewayBit,
                consent: c,
                material: discovery,
                passed: false,
            });
        }
    }
    if next.origin_serve() {
        let c = consent::origin_serve();
        if owed(&next, ConsentKind::OriginServe, &c) {
            gates.push(Gate {
                kind: ConsentKind::OriginServe,
                consent: c,
                material: origin,
                passed: false,
            });
        }
    }
    if let Some(c) = carrier_consent(&next) {
        if owed(&next, ConsentKind::CarrierEnable, &c) {
            // the text names every link's cap: any cap set or raised, on any link, asks
            let raised = cap_set_or_raised(pre.metered(), next.metered());
            let material =
                p.carrier_kind == Some(CarrierKind::Ntfy) || p.server.is_some() || raised;
            // caps only lowered or removed under a valid consent: the new text is recorded with
            // the write (§12 re-asks only for a cap set or raised)
            let renew = !material && carrier_consented(pre);
            gates.push(Gate {
                kind: ConsentKind::CarrierEnable,
                consent: c,
                material,
                passed: renew,
            });
        }
    }
    let accept = p.accept_warning.as_deref();
    if let Some(a) = accept {
        if let Some(g) = gates.iter_mut().find(|g| g.consent.hash == a) {
            g.passed = true;
        } else {
            // a hash of no text owed or recorded now (ruling (a)): the current text answers
            let known = [
                Some(consent::hash(consent::GATEWAY_BIT_TEXT)),
                Some(consent::hash(consent::ORIGIN_SERVE_TEXT)),
                carrier_consent(&next).map(|c| c.hash),
            ];
            if !known.iter().flatten().any(|h| h == a) {
                if let Some(g) = gates.iter().find(|g| !g.passed) {
                    return Err(Refused {
                        problem: Box::new(consent::problem(&g.consent)),
                        record: Vec::new(),
                    });
                }
            }
        }
    }
    if let Some(g) = gates.iter().find(|g| g.material && !g.passed) {
        let record = gates
            .iter()
            .filter(|x| x.passed && x.kind != ConsentKind::CarrierEnable)
            .map(|x| (x.kind, x.consent.hash.clone()))
            .collect();
        return Err(Refused {
            problem: Box::new(consent::problem(&g.consent)),
            record,
        });
    }
    for g in gates.iter().filter(|g| g.passed) {
        next.set_consent_hash(g.kind, Some(&g.consent.hash))
            .map_err(|e| setting_problem(&e))?;
    }
    Ok(next)
}

fn heartbeat_problem(r: HeartbeatRefusal) -> Problem {
    let mut p = r.problem();
    if r == HeartbeatRefusal::SettingsLocked {
        // §2.4 item 2: the detail names the route that writes it
        p = p.detail(
            "The rns heartbeat is the LXMF helper's announce interval, Layer 6's setting: change \
             it with PATCH /api/v1/bridge/settings (announce.interval_min). Nothing was written.",
        );
    }
    p
}

// ---------------------------------------------------------------- writing

/// Write `planned` (the plan of `p` onto `pre`) and run what follows a write (module doc):
/// the gateway row, the role bits, the DNS-SD record, the served blobs, the carrier hook, the
/// `relay` frame. The caller holds the cell's write lock.
async fn commit(
    api: &ApiState,
    pre: &RelaySettings,
    p: &RelayPatch,
    planned: &RelaySettings,
) -> Result<RelaySettings, ProblemWith> {
    let termux = crate::config::on_termux();
    let state_dir = api.layout.state_dir();
    let (p2, consents) = (
        p.clone(),
        CONSENT_KINDS.map(|k| (k, planned.consent_hash(k).map(str::to_string))),
    );
    let written = tokio::task::spawn_blocking(move || {
        rs::update(&state_dir, |s| {
            apply_values(s, &p2, termux)?;
            for (k, h) in &consents {
                s.set_consent_hash(*k, h.as_deref())?;
            }
            Ok::<(), SettingsError>(())
        })
    })
    .await;
    let (after, changed) = match written {
        Ok(Ok(x)) => x,
        Ok(Err(rs::UpdateError::Change(e))) => return Err(ProblemWith::new(setting_problem(&e))),
        Ok(Err(e @ rs::UpdateError::Write(_))) => {
            eprintln!("111 relay: {e}");
            return Err(ProblemWith::new(internal(e.to_string())));
        }
        Err(e) => return Err(ProblemWith::new(internal(e.to_string()))),
    };
    let c = cell(api);
    c.replace(after.clone());
    // what follows a write: on a changed file what the change moves; on an unchanged one every
    // follower, each idempotent, since the file may hold these settings while the rest does not
    // (a re-mint that failed put the old switches back, row D4b; or the request that wrote the
    // file was cut short after the write): the same write again reconciles them
    let announce = reconcile(api, &after).await;
    if !changed || pre.lan_record() != after.lan_record() {
        if let Some(reg) = crate::daemon::registry::get(api) {
            reg.set_lan_record(after.lan_record());
            reg.sync_record().await;
        }
    }
    if !changed || pre.origin_targets() != after.origin_targets() {
        let origin = Origin::shared(api);
        let targets = after.origin_targets().to_vec();
        let _ = tokio::task::spawn_blocking(move || origin.retain_targets(&targets)).await;
    }
    relay::carrier_hook(api).reconfigure(&after);
    // a frame for a changed file, or for switches the fabric announces only now
    if changed || matches!(announce, Ok(true)) {
        publish_written(api).await;
    }
    match announce {
        Ok(_) => Ok(after),
        Err(e) => Err(not_announced(&e)),
    }
}

/// The gateway row and the role bits follow `after` (no fabric: nothing to follow). `Ok(true)`
/// when the announce changed.
async fn reconcile(
    api: &ApiState,
    after: &RelaySettings,
) -> Result<bool, crate::daemon::fabric::ReannounceError> {
    let Some(h) = fabric_of(api) else {
        return Ok(false);
    };
    gateway::apply(&h, after);
    h.set_relay_settings(after).await
}

/// A write's frame: the write generation bumped first, so no build begun before this write
/// publishes after this frame ([`publish`]).
async fn publish_written(api: &ApiState) {
    cell(api).writes.fetch_add(1, Ordering::SeqCst);
    publish(api, true).await;
}

/// `500`: the settings are written but the role bits are not announced (row D4b: the fabric's
/// switches are back as announced before; the same write tries again, [`commit`]).
fn not_announced(e: &crate::daemon::fabric::ReannounceError) -> ProblemWith {
    eprintln!("111 relay: settings saved; {e}");
    ProblemWith::new(internal(format!(
        "The settings were saved, but {e}. Save again to retry."
    )))
}

/// Record the consent hashes a refused round passed (module doc); the cell follows the file.
async fn record_consents(api: &ApiState, record: Vec<(ConsentKind, String)>) {
    if record.is_empty() {
        return;
    }
    let state_dir = api.layout.state_dir();
    let written = tokio::task::spawn_blocking(move || {
        rs::update(&state_dir, |s| {
            for (k, h) in &record {
                s.set_consent_hash(*k, Some(h))?;
            }
            Ok::<(), SettingsError>(())
        })
    })
    .await;
    let after = match written {
        Ok(Ok((after, _))) => after,
        Ok(Err(e)) => {
            eprintln!("111 relay: a consent was not recorded: {e}");
            return;
        }
        Err(e) => {
            eprintln!("111 relay: a consent was not recorded: {e}");
            return;
        }
    };
    cell(api).replace(after.clone());
    // a consent recorded for a switch already on turns its role bit on (D9 reads only a consent
    // that holds): the fabric follows the file as after any write
    match reconcile(api, &after).await {
        Ok(true) => publish_written(api).await,
        Ok(false) => {}
        Err(e) => eprintln!("111 relay: a consent was recorded; {e}"),
    }
}

/// `PATCH /relay/settings` and a relay proposal's `gateway.enable`: plan, then write (module doc).
/// Holds the cell's write lock throughout.
pub async fn apply_patch(api: &ApiState, p: &RelayPatch) -> Result<RelaySettings, ProblemWith> {
    let c = cell(api);
    let _one = c.write.lock().await;
    let pre = c.settings();
    match plan(&pre, p, &NodeFacts::of(api)) {
        Ok(next) => commit(api, &pre, p, &next).await,
        Err(r) => {
            record_consents(api, r.record).await;
            Err(*r.problem)
        }
    }
}

async fn patch_settings(State(api): State<Arc<ApiState>>, body: Bytes) -> Response {
    let p = match parse_patch(&body) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    // the write and what follows it run to their end in a task of their own (as V5-1's
    // transport writes): a request dropped part-way (its client gone) never leaves the file
    // written and the followers not run
    let task = {
        let api = Arc::clone(&api);
        tokio::spawn(async move { apply_patch(&api, &p).await.map(|_| ()) })
    };
    match task.await {
        Ok(Ok(())) => Json(status(&api).await).into_response(),
        Ok(Err(e)) => e.into_response(),
        Err(e) => {
            ProblemWith::new(internal(format!("the settings task failed: {e}"))).into_response()
        }
    }
}

// ---------------------------------------------------------------- the carrier routes

/// What a carrier request asks (§13.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarrierRequest {
    /// `POST /relay/carrier/check`.
    Check,
    /// `POST /relay/carrier/flush` (and the proposal's `carrier.flush {}`).
    Flush,
}

/// The refusals before a carrier request (§13.1, §12), against the settings `s`: no carrier
/// (`409 bad_request`), its consent not the one owed now (`409 consent_required`; accepted through
/// `PATCH /relay/settings`), the metered cap (`409 metered_cap`), a licensed board (`409
/// licensed_board`).
pub fn carrier_refusal(s: &RelaySettings, hook: &dyn relay::CarrierHook) -> Option<ProblemWith> {
    let Some(c) = carrier_consent(s) else {
        return Some(ProblemWith::new(conflict(
            "bad_request",
            "No carrier is set up.",
            "Set up a carrier first (Settings → Relay & presence → Gateway).",
        )));
    };
    if s.consent_hash(ConsentKind::CarrierEnable) != Some(c.hash.as_str()) {
        return Some(consent::problem(&c));
    }
    if hook.metered_cap_reached() {
        return Some(ProblemWith::new(conflict(
            "metered_cap",
            "Today's metered allowance is used up.",
            "The carrier has used the daily cap of a link you marked metered; nothing is sent \
             until the 24 hours pass.",
        )));
    }
    if hook.licensed_board() {
        return Some(ProblemWith::new(conflict(
            "licensed_board",
            "Not on a licensed board.",
            "No Layer 7 path runs over a board marked licensed (§12).",
        )));
    }
    None
}

/// Run `what` through the refusals, then hand it to the carrier hook (§13.1): `RelayAccepted`.
pub async fn carrier_request(api: &ApiState, what: CarrierRequest) -> Result<Value, ProblemWith> {
    let s = cell(api).settings();
    let hook = relay::carrier_hook(api);
    if let Some(p) = carrier_refusal(&s, hook.as_ref()) {
        return Err(p);
    }
    let started = match what {
        CarrierRequest::Check => hook.check(),
        CarrierRequest::Flush => hook.flush(),
    };
    Ok(json!({ "schema": SCHEMA, "started": started }))
}

async fn carrier_check(State(api): State<Arc<ApiState>>) -> Response {
    match carrier_request(&api, CarrierRequest::Check).await {
        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn carrier_flush(State(api): State<Arc<ApiState>>) -> Response {
    match carrier_request(&api, CarrierRequest::Flush).await {
        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(e) => e.into_response(),
    }
}

// ---------------------------------------------------------------- the status (RelayStatus)

/// The paired hubs and the forward counters, from one fabric status (§13.1): the fabric's
/// records, the daemon's halves, the per-pair views.
async fn pairs_and_counters(h: &FabricHandle) -> (Vec<Value>, u64, u64) {
    let Some(f) = h.fabric() else {
        return (Vec::new(), 0, 0);
    };
    let persisted = h.persisted();
    let (records, views, refused, too_large) = match f.status().await {
        Ok(st) => (
            st.settings.paired_hubs,
            st.paired,
            st.counters.forward_refused,
            st.counters.forward_too_large,
        ),
        Err(_) => (
            persisted.fabric.paired_hubs.clone(),
            h.pair_views().views,
            0,
            0,
        ),
    };
    let mut own: Option<Option<SocketAddr>> = None;
    let mut out = Vec::with_capacity(records.len());
    for r in &records {
        let half = persisted.pair_link(&r.node_id).cloned();
        // where the two hubs meet: the far bundle's address, else the one this hub shows
        let meet = match half.as_ref().and_then(|l| l.address) {
            Some(a) => Some(a),
            None if r.via == PairVia::Tcp => match own {
                Some(o) => o,
                None => {
                    let o = h.offer_address().await;
                    own = Some(o);
                    o
                }
            },
            None => None,
        };
        let view = views.iter().find(|v| v.hub.node_id == r.node_id);
        let link = half.clone().unwrap_or_else(|| PairLink::new(r.node_id));
        let mut v = pair::paired_hub_json(&link, r.role, r.via, meet, view);
        v["tls"] = json!(tls_of(half.as_ref(), r.via, meet));
        out.push(v);
    }
    (out, refused, too_large)
}

/// A paired hub's `PairTls` in the status (§4.1, §12; ruling (c)): row D6's `tls_view` for a pair
/// this hub scanned (`none` for a carrier pair, a `lan` pair or a private meeting address, `rpk`
/// with the far link key pinned, `plain` with the warning recorded). For a record this hub holds
/// no half of (the far hub enrolled here from this hub's bundle; this hub has not scanned the
/// far hub's yet): `none` when they meet at a private address (plain `tcp-lan`, §4.1 rule 1),
/// else `rpk` (the far hub pinned this hub's key from that bundle and dials it inside TLS; the
/// link comes up once this hub pins the far key). So `{via: tcp, lan: false, tls: none}` is a
/// pair met at a private address, running plain with no TLS and no warning (or, after this hub's
/// own address stopped being private, one it no longer dials); `plain` is never shown without a
/// recorded `pair_plain` warning, nor `rpk` without a pinned key on one side.
pub fn tls_of(half: Option<&PairLink>, via: PairVia, meet: Option<SocketAddr>) -> &'static str {
    match half {
        Some(link) => {
            let t = pair::tls_view(link, via, meet);
            if t == "plain" && link.warning_hash.is_none() {
                // no key and no warning at an address that is not private now (the scan route
                // asks the warning otherwise, so it was private when the pair was made): this
                // hub dials nothing there (§4.1 rule 4); never claim TLS or an accepted warning
                "none"
            } else {
                t
            }
        }
        None if via == PairVia::Carrier => "none",
        None if meet.is_some_and(|a| crate::daemon::relay::tls::is_private(a.ip())) => "none",
        None => "rpk",
    }
}

/// The `rns` heartbeat row (§2.4 item 2): the LXMF helper's announce interval, Layer 6's setting,
/// shown here and never written through.
fn rns_row(api: &ApiState) -> Value {
    let minutes = api
        .ext::<Arc<crate::daemon::bridge::Bridge>>()
        .map(|b| b.announce().interval_min)
        .unwrap_or(crate::daemon::bridge::settings::DEFAULT_ANNOUNCE_MIN);
    json!({ "transport": RNS_TRANSPORT, "interval_min": minutes, "owner": "layer6" })
}

/// `GET /relay`'s `RelayStatus` (§13.1): no secret, no task, no file write.
pub async fn status(api: &ApiState) -> Value {
    let s = cell(api).settings();
    let view = s.view();
    let (paired, refused, too_large) = match fabric_of(api) {
        Some(h) => pairs_and_counters(&h).await,
        None => (Vec::new(), 0, 0),
    };
    gateway::refresh();
    let g = gateway::facts();
    let hook = relay::carrier_hook(api);
    let mut heartbeat = serde_json::to_value(&view.presence.heartbeat).unwrap_or_default();
    if let Value::Array(rows) = &mut heartbeat {
        rows.push(rns_row(api));
    }
    let bytes = Origin::shared(api).bytes();
    json!({
        "schema": SCHEMA,
        "gateway": {
            "enabled": view.gateway.enabled,
            "state": g.state().code(),
            "carrier": g.carrier,
            "endpoint": g.endpoint,
            "mode": g.mode,
            "checked_ago_s": g.checked_ago_s,
            "last_flush_ago_s": g.last_flush_ago_s,
            "pending": g.pending,
            "metered_cap_reached": hook.metered_cap_reached(),
            "termux_poll_min": view.gateway.termux_poll_min,
        },
        "carrier": view.carrier,
        "paired_hubs": paired,
        "forward": { "refused": refused, "too_large": too_large },
        "presence": { "heartbeat": heartbeat },
        "discovery": view.discovery,
        "origin": { "serve": view.origin.serve, "targets": view.origin.targets, "bytes": bytes },
        "updates": view.updates,
        "metered": view.metered,
    })
}

async fn get_relay(State(api): State<Arc<ApiState>>) -> Response {
    Json(status(&api).await).into_response()
}

// ---------------------------------------------------------------- the relay frame (§13.1)

/// The status without its ages (what a fabric-driven build compares with the last frame).
fn without_ages(v: &Value) -> Value {
    let mut v = v.clone();
    if let Some(g) = v.get_mut("gateway").and_then(Value::as_object_mut) {
        g.remove("checked_ago_s");
        g.remove("last_flush_ago_s");
    }
    v
}

/// Build the status and publish it as the `relay` frame; unless `force`, only when it differs
/// from the last frame in more than its ages; never when a write committed and published its own
/// frame after this build began ([`LastFrame::admit`]: a stale status never follows a newer one).
pub async fn publish(api: &ApiState, force: bool) {
    let built = build(api).await;
    emit(api, built, force);
}

/// A `relay` frame built and not yet published: its status and the write generation its build
/// began at ([`build`], [`emit`]).
#[doc(hidden)]
pub struct Built {
    generation: u64,
    status: Value,
}

/// Build a frame: the write generation is read before the status, so a write that lands while
/// the status is read makes this build older than that write's frame ([`LastFrame::admit`]).
#[doc(hidden)]
pub async fn build(api: &ApiState) -> Built {
    let generation = cell(api).writes.load(Ordering::SeqCst);
    let status = status(api).await;
    Built { generation, status }
}

/// Publish a built frame if [`LastFrame::admit`] admits it.
#[doc(hidden)]
pub fn emit(api: &ApiState, built: Built, force: bool) {
    let fp = without_ages(&built.status);
    // admit and publish under one lock, so two builds' frames go out in the order admitted
    let c = cell(api);
    let mut last = c.last_frame.lock().unwrap_or_else(|p| p.into_inner());
    if last.admit(built.generation, fp, force) {
        api.events.publish("relay", &built.status);
    }
}

/// The frame's binding to the daemon (one per process, like the presence book and the gateway
/// row: the fabric's tap is a free function with no handle) and its coalescing state.
#[derive(Default)]
struct Frames {
    bound: Mutex<Option<(Weak<ApiState>, Weak<FabricHandle>)>>,
    /// A build is queued.
    dirty: AtomicBool,
    /// The queued build publishes even an unchanged status.
    force: AtomicBool,
    /// A build task runs.
    running: AtomicBool,
    /// The bound node holds paired hubs: every `Fabric` event may move the status.
    watch: AtomicBool,
    /// The pair generation the last build saw.
    seen: AtomicU64,
    /// The daemon's runtime (taken at [`bind`]), for a [`changed`] called from a thread of its
    /// own (the carrier's 5B stream, D11).
    rt: Mutex<Option<tokio::runtime::Handle>>,
}

static FRAMES: OnceLock<Frames> = OnceLock::new();

impl Frames {
    fn upgrade(&self) -> Option<(Arc<ApiState>, Arc<FabricHandle>)> {
        let b = self.bound.lock().unwrap_or_else(|p| p.into_inner());
        let (a, f) = b.as_ref()?;
        Some((a.upgrade()?, f.upgrade()?))
    }

    fn observe(&self, h: &FabricHandle) {
        self.watch.store(
            !h.persisted().fabric.paired_hubs.is_empty(),
            Ordering::SeqCst,
        );
        self.seen.store(h.pair_generation(), Ordering::SeqCst);
    }

    fn schedule(&'static self, force: bool) {
        if force {
            self.force.store(true, Ordering::SeqCst);
        }
        self.dirty.store(true, Ordering::SeqCst);
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let rt = tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.rt.lock().unwrap_or_else(|p| p.into_inner()).clone());
        let Some(rt) = rt else {
            self.running.store(false, Ordering::SeqCst);
            return;
        };
        rt.spawn(self.run());
    }

    async fn run(&'static self) {
        /// Clears `running` if the build is dropped part-way (its runtime shut down, or the
        /// build panicked), so a later [`Frames::schedule`] starts a task again.
        struct Unstick(&'static AtomicBool);
        impl Drop for Unstick {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        loop {
            self.dirty.store(false, Ordering::SeqCst);
            let force = self.force.swap(false, Ordering::SeqCst);
            let unstick = Unstick(&self.running);
            if let Some((api, h)) = self.upgrade() {
                self.observe(&h);
                publish(&api, force).await;
            }
            std::mem::forget(unstick);
            self.running.store(false, Ordering::SeqCst);
            // a request that came while this one built: run again (unless another task took it)
            if !self.dirty.load(Ordering::SeqCst) || self.running.swap(true, Ordering::SeqCst) {
                return;
            }
        }
    }
}

/// Bind the `relay` frame to the daemon's api and fabric (`relay::start`). Nothing is spawned.
pub fn bind(api: &Arc<ApiState>, fabric: &Arc<FabricHandle>) {
    let f = FRAMES.get_or_init(Frames::default);
    *f.bound.lock().unwrap_or_else(|p| p.into_inner()) =
        Some((Arc::downgrade(api), Arc::downgrade(fabric)));
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        *f.rt.lock().unwrap_or_else(|p| p.into_inner()) = Some(rt);
    }
    f.observe(fabric);
}

/// A fabric event from `relay::tap`: a `Forward` event, or a `Fabric` event while the node holds
/// pairs or after its pair set moved, queues a build (module doc). Other events, and every event
/// before [`bind`], return at once.
pub fn on_event(event: &FabricEvent) {
    let forward = match event {
        FabricEvent::Forward { .. } => true,
        FabricEvent::Fabric => false,
        _ => return,
    };
    let Some(f) = FRAMES.get() else {
        return;
    };
    if !forward && !f.watch.load(Ordering::SeqCst) {
        let Some((_, h)) = f.upgrade() else {
            return;
        };
        if f.seen.load(Ordering::SeqCst) == h.pair_generation() {
            return;
        }
    }
    f.schedule(false);
}

/// `relay::changed()`: publish the frame soon, even when only its ages moved. Callable from any
/// thread (the build runs on the daemon's runtime).
pub fn changed() {
    if let Some(f) = FRAMES.get() {
        f.schedule(true);
    }
}

// ---------------------------------------------------------------- relay proposals (§13.2)

/// The proposal kind (§13.2).
pub const RELAY_KIND: &str = "relay";

/// One action of a relay proposal's closed vocabulary (§13.2; row A1's writer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayAction {
    /// `gateway.enable {carrier, mode}`: the gateway on with this carrier (its server is the
    /// one configured; the agent never types one).
    GatewayEnable {
        /// `ntfy` (the only carrier in v1).
        carrier: CarrierKind,
        /// `5A` or `5B`.
        mode: CarrierMode,
    },
    /// `hub.pair {}`: opens the pairing screen (the agent never types a code).
    HubPair,
    /// `carrier.flush {}`.
    CarrierFlush,
}

impl RelayAction {
    /// The vocabulary's name.
    pub fn name(&self) -> &'static str {
        match self {
            RelayAction::GatewayEnable { .. } => "gateway.enable",
            RelayAction::HubPair => "hub.pair",
            RelayAction::CarrierFlush => "carrier.flush",
        }
    }
}

/// `502 upstream_error`: the agent wrote a relay body that is not the §13.2 shape.
fn not_the_shape(why: &str) -> ProblemWith {
    ProblemWith::new(
        Problem::new(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            "The local AI agent wrote a relay proposal this node cannot read.",
        )
        .detail(format!(
            "{why}; the body must be {{\"why\": string, \"actions\": [Action]}} from the closed \
             vocabulary. Dismiss it and ask the agent again."
        )),
    )
}

/// `409 unsupported_action` (the proposal stays open).
fn unsupported(action: &str, why: &str) -> ProblemWith {
    ProblemWith::new(conflict(
        "unsupported_action",
        "This node does not apply that action.",
        format!("{action:?}: {why}"),
    ))
}

/// Parse a relay proposal's body (§13.2): one JSON object `{"why": string, "actions": [Action]}`,
/// each action `{"action": name, …args}` at most once. Not that shape (and an action's arguments
/// not its own): `502 upstream_error`. `presence.interval` is refused `409 unsupported_action`
/// before its arguments or any transport are looked at (§2.4, I4), and so is a name outside the
/// vocabulary.
pub fn parse_relay_body(body: &str) -> Result<Vec<RelayAction>, ProblemWith> {
    let top = match serde_json::from_str::<Value>(body) {
        Ok(Value::Object(o)) => o,
        _ => return Err(not_the_shape("the body is not one JSON object")),
    };
    if top.len() != 2 || !top.get("why").is_some_and(Value::is_string) {
        return Err(not_the_shape("the object is not exactly {why, actions}"));
    }
    let Some(Value::Array(items)) = top.get("actions") else {
        return Err(not_the_shape("actions is not a list"));
    };
    if items.is_empty() {
        return Err(not_the_shape("actions is empty"));
    }
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let Some(name) = item.get("action").and_then(Value::as_str) else {
            return Err(not_the_shape("an action is not {\"action\": name, …}"));
        };
        names.push(name);
    }
    for name in &names {
        match *name {
            "presence.interval" => {
                return Err(unsupported(
                    name,
                    "no transport takes a presence heartbeat in this version (events only)",
                ))
            }
            "gateway.enable" | "hub.pair" | "carrier.flush" => {}
            _ => return Err(unsupported(name, "not in the relay vocabulary")),
        }
    }
    let mut out: Vec<RelayAction> = Vec::with_capacity(items.len());
    for item in items {
        let Some(o) = item.as_object() else {
            return Err(not_the_shape("an action is not an object"));
        };
        let args = |keys: &[&str]| {
            o.keys()
                .all(|k| k == "action" || keys.contains(&k.as_str()))
        };
        let a = match o.get("action").and_then(Value::as_str) {
            Some("gateway.enable") => {
                let carrier = o
                    .get("carrier")
                    .and_then(Value::as_str)
                    .and_then(|s| CarrierKind::parse(s).ok())
                    .filter(|k| *k == CarrierKind::Ntfy);
                let mode = o
                    .get("mode")
                    .and_then(Value::as_str)
                    .and_then(|s| CarrierMode::parse(s).ok());
                match (carrier, mode, args(&["carrier", "mode"])) {
                    (Some(carrier), Some(mode), true) => {
                        RelayAction::GatewayEnable { carrier, mode }
                    }
                    _ => return Err(not_the_shape(
                        "gateway.enable is not {\"carrier\": \"ntfy\", \"mode\": \"5A\" | \"5B\"}",
                    )),
                }
            }
            Some("hub.pair") if args(&[]) => RelayAction::HubPair,
            Some("carrier.flush") if args(&[]) => RelayAction::CarrierFlush,
            _ => {
                return Err(not_the_shape(
                    "an action carries arguments it does not take",
                ))
            }
        };
        if out.iter().any(|x| x.name() == a.name()) {
            return Err(not_the_shape("an action is listed twice"));
        }
        out.push(a);
    }
    Ok(out)
}

/// A relay proposal checked and ready to apply (§13.2): every validation, consent and refusal
/// has passed; the settings write lock is held until it is applied or dropped.
pub struct PreparedRelay {
    _one: tokio::sync::OwnedMutexGuard<()>,
    actions: Vec<RelayAction>,
    pre: RelaySettings,
    patch: Option<RelayPatch>,
    planned: RelaySettings,
}

/// A refusal on the decision route: `400` there is `409 bad_request` (the decision's body was
/// fine; the node's state refused the action).
fn on_decision(mut p: ProblemWith) -> ProblemWith {
    if p.problem.status == StatusCode::BAD_REQUEST.as_u16() {
        p.problem.status = StatusCode::CONFLICT.as_u16();
    }
    p
}

/// Run a relay proposal's actions through the same validation, consent and refusal paths as the
/// routes (§13.2), writing nothing: `gateway.enable` is `PATCH /relay/settings {gateway:
/// {enabled: true}, carrier: {kind, mode}}` with the decision's `accept_warning`; `carrier.flush`
/// is `POST /relay/carrier/flush`'s refusals against the settings the proposal would leave;
/// `hub.pair` needs the hub role, like the pairing routes.
pub async fn prepare_relay(
    api: &ApiState,
    actions: Vec<RelayAction>,
    accept: Option<String>,
) -> Result<PreparedRelay, ProblemWith> {
    let c = cell(api);
    let one = Arc::clone(&c.write).lock_owned().await;
    let pre = c.settings();
    let cx = NodeFacts::of(api);
    let mut patch = None;
    let mut planned = pre.clone();
    for a in &actions {
        match a {
            RelayAction::GatewayEnable { carrier, mode } => {
                let p = RelayPatch {
                    gateway_enabled: Some(true),
                    carrier_kind: Some(*carrier),
                    mode: Some(*mode),
                    accept_warning: accept.clone(),
                    ..RelayPatch::default()
                };
                match plan(&pre, &p, &cx) {
                    Ok(next) => planned = next,
                    Err(r) => {
                        record_consents(api, r.record).await;
                        return Err(on_decision(*r.problem));
                    }
                }
                patch = Some(p);
            }
            RelayAction::HubPair if !cx.hub => {
                return Err(ProblemWith::new(needs_hub("Pairing with another hub")))
            }
            RelayAction::HubPair | RelayAction::CarrierFlush => {}
        }
    }
    if actions.contains(&RelayAction::CarrierFlush) {
        let hook = relay::carrier_hook(api);
        if let Some(p) = carrier_refusal(&planned, hook.as_ref()) {
            return Err(on_decision(p));
        }
    }
    Ok(PreparedRelay {
        _one: one,
        actions,
        pre,
        patch,
        planned,
    })
}

/// Apply a prepared relay proposal: the settings write (and what follows it) for
/// `gateway.enable`, then the flush for `carrier.flush`. The decision record's `applied`.
pub async fn apply_relay(api: &ApiState, prepared: PreparedRelay) -> Result<Value, ProblemWith> {
    if let Some(p) = &prepared.patch {
        commit(api, &prepared.pre, p, &prepared.planned)
            .await
            .map_err(on_decision)?;
    }
    let mut applied = json!({
        "actions": prepared.actions.iter().map(RelayAction::name).collect::<Vec<_>>(),
    });
    if prepared.actions.contains(&RelayAction::CarrierFlush) {
        applied["flush_started"] = json!(relay::carrier_hook(api).flush());
    }
    drop(prepared);
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::relay::settings::MeteredMatch;

    fn hub() -> NodeFacts {
        NodeFacts {
            hub: true,
            lan: true,
            termux: false,
        }
    }

    fn patch(body: &str) -> RelayPatch {
        parse_patch(body.as_bytes()).unwrap()
    }

    fn code(r: &Refused) -> (u16, &'static str) {
        (r.problem.problem.status, r.problem.problem.code)
    }

    fn consent_of(r: &Refused) -> Value {
        r.problem.body()["consent"].clone()
    }

    fn ntfy(server: &str) -> RelaySettings {
        let mut s = RelaySettings::default();
        s.set_carrier_kind(CarrierKind::Ntfy);
        s.set_ntfy_server(Some(server)).unwrap();
        let h = consent::carrier_enable("ntfy", server, &[]).hash;
        s.set_consent_hash(ConsentKind::CarrierEnable, Some(&h))
            .unwrap();
        s
    }

    #[test]
    fn the_patch_body_is_strict_and_never_echoes_a_secret() {
        let p = patch(
            r#"{"gateway":{"enabled":true,"termux_poll_min":20},
                "carrier":{"kind":"ntfy","server":"https://n.example","mode":"5B",
                           "token":"tok-SECRET-1","topic_secret":null},
                "presence":{"heartbeat":[]},
                "discovery":{"lan_record":true,"public_gateway_bit":false,"offload_bit":true},
                "origin":{"serve":false,"targets":["x86_64-unknown-linux-musl"]},
                "updates":{"check_internet":false},
                "metered":[{"match":{"iface":"wwan0"},"daily_cap_bytes":1000}]}"#,
        );
        assert_eq!(p.gateway_enabled, Some(true));
        assert_eq!(p.termux_poll_min, Some(20));
        assert_eq!(p.carrier_kind, Some(CarrierKind::Ntfy));
        assert_eq!(p.mode, Some(CarrierMode::FiveB));
        assert_eq!(
            p.token.as_ref().unwrap().as_deref().map(|t| t.as_str()),
            Some("tok-SECRET-1")
        );
        assert!(matches!(p.topic_secret, Some(None)));
        assert_eq!(p.check_internet, Some(false));
        assert_eq!(
            p.metered.as_ref().unwrap()[0].matcher,
            MeteredMatch::Iface("wwan0".into())
        );
        assert!(
            !format!("{p:?}").contains("SECRET"),
            "Debug prints no secret"
        );
        for bad in [
            &br#"[]"#[..],
            br#"{"gateway":{"enabled":"yes"}}"#,
            br#"{"gateway":{"other":true}}"#,
            br#"{"carrier":{"kind":"chatmail"}}"#,
            br#"{"carrier":{"mode":"5C"}}"#,
            br#"{"carrier":{"ntfy":{"server":"x"}}}"#,
            br#"{"presence":{"heartbeat":[{"transport":"rns"}]}}"#,
            br#"{"presence":{"heartbeat":[{"transport":"rns","interval_min":-1}]}}"#,
            br#"{"origin":{"targets":"x86_64-unknown-linux-musl"}}"#,
            br#"{"metered":[{"match":{"iface":"a","ssid":"b"},"daily_cap_bytes":1}]}"#,
            br#"{"accept_warning":"ABAB"}"#,
            br#"{"quota":{"transit":1}}"#,
        ] {
            assert!(
                parse_patch(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        let long = format!(r#"{{"carrier":{{"token":"{}"}}}}"#, "S".repeat(513));
        let e = parse_patch(long.as_bytes()).unwrap_err();
        assert!(!e.detail.unwrap_or_default().contains("SSS"));
    }

    #[test]
    fn heartbeat_writes_are_refused_first_and_write_nothing() {
        let pre = RelaySettings::default();
        for (body, want) in [
            (
                r#"{"presence":{"heartbeat":[{"transport":"tcp-lan","interval_min":5}]}}"#,
                "no_such_transport",
            ),
            (
                r#"{"presence":{"heartbeat":[{"transport":"kiss-modem","interval_min":0}]}}"#,
                "no_such_transport",
            ),
            (
                r#"{"presence":{"heartbeat":[{"transport":"meshtastic-client","interval_min":60}]}}"#,
                "no_such_transport",
            ),
            (
                r#"{"presence":{"heartbeat":[{"transport":"tcp-lan","interval_min":1},{"transport":"rns","interval_min":30}]}}"#,
                "settings_locked",
            ),
            // a heartbeat beside a refusal of its own is still the heartbeat's answer
            (
                r#"{"gateway":{"enabled":true},"presence":{"heartbeat":[{"transport":"rns","interval_min":30}]}}"#,
                "settings_locked",
            ),
        ] {
            let r = plan(
                &pre,
                &patch(body),
                &NodeFacts {
                    hub: false,
                    ..hub()
                },
            )
            .unwrap_err();
            assert_eq!(code(&r), (409, want), "{body}");
            assert!(r.record.is_empty());
            if want == "settings_locked" {
                let d = r.problem.problem.detail.clone().unwrap();
                assert!(d.contains("PATCH /api/v1/bridge/settings"), "{d}");
            }
        }
    }

    #[test]
    fn the_refusals_follow_the_values() {
        let pre = RelaySettings::default();
        let member = NodeFacts {
            hub: false,
            ..hub()
        };
        let r = plan(&pre, &patch(r#"{"gateway":{"enabled":true}}"#), &member).unwrap_err();
        assert_eq!(code(&r), (409, "needs_hub"));
        let r = plan(&pre, &patch(r#"{"origin":{"serve":true}}"#), &member).unwrap_err();
        assert_eq!(code(&r), (409, "needs_hub"));
        let no_lan = NodeFacts {
            lan: false,
            ..hub()
        };
        let r = plan(&pre, &patch(r#"{"origin":{"serve":true}}"#), &no_lan).unwrap_err();
        assert_eq!(code(&r), (409, "lan_off"));
        // a value refused by D1 comes before the hub check
        let r = plan(
            &pre,
            &patch(r#"{"gateway":{"enabled":true,"termux_poll_min":5}}"#),
            &member,
        )
        .unwrap_err();
        assert_eq!(code(&r), (400, "bad_request"));
        let r = plan(&pre, &patch(r#"{"carrier":{"kind":"ntfy"}}"#), &hub()).unwrap_err();
        assert_eq!(code(&r), (400, "bad_request"), "ntfy without a server");
        let r = plan(
            &pre,
            &patch(r#"{"carrier":{"kind":"ntfy","server":"http://ntfy.example.org"}}"#),
            &hub(),
        )
        .unwrap_err();
        assert_eq!(code(&r), (400, "bad_request"), "http:// to a public name");
        let termux = NodeFacts {
            termux: true,
            ..hub()
        };
        let r = plan(&pre, &patch(r#"{"carrier":{"mode":"5B"}}"#), &termux).unwrap_err();
        assert_eq!(code(&r), (409, "bad_request"));
        // ruling (b): the own target stays
        let r = plan(
            &pre,
            &patch(r#"{"origin":{"targets":["aarch64-unknown-linux-musl"]}}"#),
            &hub(),
        )
        .unwrap_err();
        assert_eq!(code(&r), (409, "bad_request"));
        assert!(r
            .problem
            .problem
            .detail
            .clone()
            .unwrap()
            .contains(TARGET_TRIPLE));
        let ok = plan(
            &pre,
            &patch(&format!(
                r#"{{"origin":{{"targets":["aarch64-unknown-linux-musl","{TARGET_TRIPLE}"]}}}}"#
            )),
            &hub(),
        )
        .unwrap();
        assert_eq!(
            ok.origin_targets()[0],
            TARGET_TRIPLE,
            "the own target first"
        );
        assert_eq!(ok.origin_targets().len(), 2);
    }

    #[test]
    fn the_carrier_asks_once_and_records_the_hash() {
        let pre = RelaySettings::default();
        let body = r#"{"carrier":{"kind":"ntfy","server":"https://n.example","mode":"5A"}}"#;
        let r = plan(&pre, &patch(body), &hub()).unwrap_err();
        assert_eq!(code(&r), (409, "consent_required"));
        let c = consent_of(&r);
        assert_eq!(c["action"], "carrier_enable");
        assert_eq!(c["subject"], "ntfy:https://n.example");
        assert_eq!(c["addons"], json!([]));
        let text = c["warning_text"].as_str().unwrap();
        assert_eq!(text, consent::carrier_enable_text("https://n.example", &[]));
        assert_eq!(c["hash"], consent::hash(text).as_str());
        assert!(
            r.record.is_empty(),
            "the carrier's hash is recorded only with the write"
        );
        // a stale hash is refused with the current text
        let stale = format!(
            r#"{{"carrier":{{"kind":"ntfy","server":"https://n.example"}},"accept_warning":"{}"}}"#,
            "0".repeat(64)
        );
        let r = plan(&pre, &patch(&stale), &hub()).unwrap_err();
        assert_eq!(consent_of(&r)["hash"], c["hash"]);
        let ok = format!(
            r#"{{"carrier":{{"kind":"ntfy","server":"https://n.example"}},"accept_warning":"{}"}}"#,
            c["hash"].as_str().unwrap()
        );
        let next = plan(&pre, &patch(&ok), &hub()).unwrap();
        assert!(carrier_consented(&next));
        // the same carrier again asks nothing; a mode change asks nothing
        plan(&next, &patch(body), &hub()).unwrap();
        plan(&next, &patch(r#"{"carrier":{"mode":"5B"}}"#), &hub()).unwrap();
        // another server asks again
        let r = plan(
            &next,
            &patch(r#"{"carrier":{"server":"https://m.example"}}"#),
            &hub(),
        )
        .unwrap_err();
        assert_eq!(consent_of(&r)["subject"], "ntfy:https://m.example");
        // off forgets the hash: the copy is shown again before it turns on
        let off = plan(&next, &patch(r#"{"carrier":{"kind":"none"}}"#), &hub()).unwrap();
        assert_eq!(off.consent_hash(ConsentKind::CarrierEnable), None);
        let r = plan(&off, &patch(r#"{"carrier":{"kind":"ntfy"}}"#), &hub()).unwrap_err();
        assert_eq!(code(&r), (409, "consent_required"));
    }

    #[test]
    fn a_cap_set_or_raised_asks_again_and_a_lower_one_does_not() {
        let pre = ntfy("https://n.example");
        assert!(carrier_consented(&pre));
        let cap = |n: u64| {
            format!(r#"{{"metered":[{{"match":{{"ssid":"Phone"}},"daily_cap_bytes":{n}}}]}}"#)
        };
        let r = plan(&pre, &patch(&cap(5000)), &hub()).unwrap_err();
        let c = consent_of(&r);
        assert_eq!(c["action"], "carrier_enable");
        assert_eq!(c["addons"], json!(["metered"]));
        assert!(c["warning_text"]
            .as_str()
            .unwrap()
            .contains("A daily cap of 5000 bytes"));
        let with = format!(
            r#"{{"metered":[{{"match":{{"ssid":"Phone"}},"daily_cap_bytes":5000}}],"accept_warning":"{}"}}"#,
            c["hash"].as_str().unwrap()
        );
        let capped = plan(&pre, &patch(&with), &hub()).unwrap();
        assert!(carrier_consented(&capped));
        let r = plan(&capped, &patch(&cap(6000)), &hub()).unwrap_err();
        assert_eq!(code(&r), (409, "consent_required"), "raised");
        let lower = plan(&capped, &patch(&cap(4000)), &hub()).unwrap();
        assert!(
            carrier_consented(&lower),
            "a lower cap keeps a valid consent valid"
        );
        let none = plan(&lower, &patch(r#"{"metered":[]}"#), &hub()).unwrap();
        assert!(
            carrier_consented(&none),
            "no cap: the text without its sentence"
        );
        // with the carrier off a cap asks nothing
        plan(&RelaySettings::default(), &patch(&cap(5000)), &hub()).unwrap();
    }

    /// Review finding 2: the text names every metered link, so a second capped link (at or under
    /// the largest cap) or a raised cap that is not the largest asks again; lowering one does not.
    #[test]
    fn any_cap_set_or_raised_on_any_link_asks_again() {
        let pre = ntfy("https://n.example");
        let links = |wwan: u64, phone: Option<u64>| {
            let mut v = vec![format!(
                r#"{{"match":{{"iface":"wwan0"}},"daily_cap_bytes":{wwan}}}"#
            )];
            if let Some(n) = phone {
                v.push(format!(
                    r#"{{"match":{{"ssid":"Phone"}},"daily_cap_bytes":{n}}}"#
                ));
            }
            format!(r#""metered":[{}]"#, v.join(","))
        };
        let accept = |pre: &RelaySettings, body: &str| {
            let r = plan(pre, &patch(&format!("{{{body}}}")), &hub()).unwrap_err();
            assert_eq!(code(&r), (409, "consent_required"), "{body}");
            let h = consent_of(&r)["hash"].as_str().unwrap().to_string();
            plan(
                pre,
                &patch(&format!(r#"{{{body},"accept_warning":"{h}"}}"#)),
                &hub(),
            )
            .unwrap()
        };
        let one = accept(&pre, &links(5000, None));
        // a second capped link under the largest cap
        let r = plan(
            &one,
            &patch(&format!("{{{}}}", links(5000, Some(1000)))),
            &hub(),
        )
        .unwrap_err();
        assert_eq!(code(&r), (409, "consent_required"), "a second link");
        let text = consent_of(&r)["warning_text"].as_str().unwrap().to_string();
        assert!(
            text.contains("5000 bytes") && text.contains("1000 bytes"),
            "{text}"
        );
        let two = accept(&one, &links(5000, Some(1000)));
        assert!(carrier_consented(&two));
        // the cap that is not the largest, raised
        let r = plan(
            &two,
            &patch(&format!("{{{}}}", links(5000, Some(2000)))),
            &hub(),
        )
        .unwrap_err();
        assert_eq!(code(&r), (409, "consent_required"), "a non-max cap raised");
        // one lowered, the other unchanged: no question, the new text recorded
        let lower = plan(
            &two,
            &patch(&format!("{{{}}}", links(5000, Some(500)))),
            &hub(),
        )
        .unwrap();
        assert!(carrier_consented(&lower));
        // one lowered and the other raised in the same write asks
        let r = plan(
            &two,
            &patch(&format!("{{{}}}", links(6000, Some(500)))),
            &hub(),
        )
        .unwrap_err();
        assert_eq!(code(&r), (409, "consent_required"), "lowered and raised");
        assert!(
            !cap_set_or_raised(two.metered(), &[]),
            "removing caps raises none"
        );
    }

    /// Review finding 3(b): a `gateway_bit` or `origin_serve` consent recorded under another text
    /// (an update changed the copy) is asked again on the next write that touches its section;
    /// a write elsewhere passes and leaves it owed.
    #[test]
    fn a_stale_switch_consent_is_asked_on_the_next_write_there() {
        let mut pre = RelaySettings::default();
        pre.set_public_gateway_bit(true);
        pre.set_origin_serve(true);
        let old = "ab".repeat(32);
        pre.set_consent_hash(ConsentKind::GatewayBit, Some(&old))
            .unwrap();
        pre.set_consent_hash(ConsentKind::OriginServe, Some(&old))
            .unwrap();
        // elsewhere: passes, nothing recorded
        let next = plan(
            &pre,
            &patch(r#"{"updates":{"check_internet":false}}"#),
            &hub(),
        )
        .unwrap();
        assert_eq!(
            next.consent_hash(ConsentKind::GatewayBit),
            Some(old.as_str())
        );
        // the discovery section: gateway_bit asked
        let r = plan(
            &pre,
            &patch(r#"{"discovery":{"offload_bit":true}}"#),
            &hub(),
        )
        .unwrap_err();
        assert_eq!(code(&r), (409, "consent_required"));
        assert_eq!(consent_of(&r)["action"], "gateway_bit");
        // the origin section: origin_serve asked
        let r = plan(
            &pre,
            &patch(&format!(
                r#"{{"origin":{{"targets":["{TARGET_TRIPLE}"]}}}}"#
            )),
            &hub(),
        )
        .unwrap_err();
        assert_eq!(consent_of(&r)["action"], "origin_serve");
        // accepted: recorded with the write
        let gb = consent::gateway_bit().hash;
        let next = plan(
            &pre,
            &patch(&format!(
                r#"{{"discovery":{{"offload_bit":true}},"accept_warning":"{gb}"}}"#
            )),
            &hub(),
        )
        .unwrap();
        assert!(consent::holds(&next, ConsentKind::GatewayBit));
        // turning it off asks nothing
        plan(
            &pre,
            &patch(r#"{"discovery":{"public_gateway_bit":false}}"#),
            &hub(),
        )
        .unwrap();
    }

    /// Review finding 4: a frame build begun before a write never publishes after that write's
    /// frame; builds of the same generation still compare their status.
    #[test]
    fn a_stale_build_never_follows_a_newer_frame() {
        let mut last = LastFrame::default();
        let (old, new) = (json!({"v": 1}), json!({"v": 2}));
        // a fabric-driven build begins at generation 0; a write commits (generation 1) and its
        // forced frame publishes first
        assert!(last.admit(1, new.clone(), true));
        // the stale build finishes: refused, even forced
        assert!(!last.admit(0, old.clone(), false));
        assert!(!last.admit(0, old.clone(), true));
        assert_eq!(last.status.as_ref(), Some(&new));
        // the same generation: an unchanged status is not published again, a changed one is
        assert!(!last.admit(1, new.clone(), false));
        assert!(last.admit(1, old.clone(), false));
        assert!(
            last.admit(1, old, true),
            "forced publishes an unchanged one"
        );
        assert!(last.admit(2, new, false));
    }

    #[test]
    fn ruling_a_records_the_owed_carrier_consent_on_its_own() {
        // a carrier held without its consent (a file edited by hand, say)
        let mut pre = RelaySettings::default();
        pre.set_carrier_kind(CarrierKind::Ntfy);
        pre.set_ntfy_server(Some("https://n.example")).unwrap();
        assert!(!carrier_consented(&pre));
        // an unrelated write passes and leaves it unconsented
        let next = plan(
            &pre,
            &patch(r#"{"discovery":{"offload_bit":true}}"#),
            &hub(),
        )
        .unwrap();
        assert!(!carrier_consented(&next));
        let owed = carrier_consent(&pre).unwrap().hash;
        // the owed hash alone records it, with no other key
        let next = plan(
            &pre,
            &patch(&format!(r#"{{"accept_warning":"{owed}"}}"#)),
            &hub(),
        )
        .unwrap();
        assert!(carrier_consented(&next));
        // a stale hash is refused with the current text
        let stale = consent::carrier_enable("ntfy", "https://old.example", &[]).hash;
        let r = plan(
            &pre,
            &patch(&format!(r#"{{"accept_warning":"{stale}"}}"#)),
            &hub(),
        )
        .unwrap_err();
        assert_eq!(code(&r), (409, "consent_required"));
        assert_eq!(consent_of(&r)["hash"], owed.as_str());
        // with nothing owed, an extra hash is a no-op
        plan(
            &next,
            &patch(&format!(r#"{{"accept_warning":"{stale}"}}"#)),
            &hub(),
        )
        .unwrap();
    }

    #[test]
    fn two_gates_converge_one_per_round() {
        let pre = RelaySettings::default();
        let body = |accept: Option<&str>| {
            let a = accept
                .map(|h| format!(r#","accept_warning":"{h}""#))
                .unwrap_or_default();
            format!(r#"{{"discovery":{{"public_gateway_bit":true}},"origin":{{"serve":true}}{a}}}"#)
        };
        let r = plan(&pre, &patch(&body(None)), &hub()).unwrap_err();
        assert_eq!(consent_of(&r)["action"], "gateway_bit");
        assert_eq!(consent_of(&r)["warning_text"], consent::GATEWAY_BIT_TEXT);
        let gb = consent::hash(consent::GATEWAY_BIT_TEXT);
        let r = plan(&pre, &patch(&body(Some(&gb))), &hub()).unwrap_err();
        assert_eq!(consent_of(&r)["action"], "origin_serve");
        assert_eq!(consent_of(&r)["warning_text"], consent::ORIGIN_SERVE_TEXT);
        assert_eq!(r.record, vec![(ConsentKind::GatewayBit, gb.clone())]);
        // the route records it; the next round passes the second gate
        let mut held = pre.clone();
        held.set_consent_hash(ConsentKind::GatewayBit, Some(&gb))
            .unwrap();
        let os = consent::hash(consent::ORIGIN_SERVE_TEXT);
        let next = plan(&held, &patch(&body(Some(&os))), &hub()).unwrap();
        assert!(next.public_gateway_bit() && next.origin_serve());
        assert_eq!(
            next.consent_hash(ConsentKind::OriginServe),
            Some(os.as_str())
        );
        // off forgets: on again asks again
        let off = plan(&next, &patch(r#"{"origin":{"serve":false}}"#), &hub()).unwrap();
        assert_eq!(off.consent_hash(ConsentKind::OriginServe), None);
        let r = plan(&off, &patch(r#"{"origin":{"serve":true}}"#), &hub()).unwrap_err();
        assert_eq!(consent_of(&r)["action"], "origin_serve");
    }

    #[test]
    fn the_carrier_refusals_come_in_order() {
        struct Hook(bool, bool);
        impl relay::CarrierHook for Hook {
            fn reconfigure(&self, _: &RelaySettings) {}
            fn check(&self) -> bool {
                true
            }
            fn flush(&self) -> bool {
                true
            }
            fn metered_cap_reached(&self) -> bool {
                self.0
            }
            fn licensed_board(&self) -> bool {
                self.1
            }
        }
        let c = |s: &RelaySettings, h: &Hook| carrier_refusal(s, h).map(|p| p.problem.code);
        let none = RelaySettings::default();
        assert_eq!(c(&none, &Hook(true, true)), Some("bad_request"));
        let mut unconsented = ntfy("https://n.example");
        unconsented
            .set_consent_hash(ConsentKind::CarrierEnable, None)
            .unwrap();
        assert_eq!(c(&unconsented, &Hook(true, true)), Some("consent_required"));
        let ok = ntfy("https://n.example");
        assert_eq!(c(&ok, &Hook(true, true)), Some("metered_cap"));
        assert_eq!(c(&ok, &Hook(false, true)), Some("licensed_board"));
        assert_eq!(c(&ok, &Hook(false, false)), None);
        let p = carrier_refusal(&none, &Hook(false, false)).unwrap();
        assert_eq!(p.problem.status, 409);
    }

    #[test]
    fn the_relay_body_is_the_closed_vocabulary() {
        let a = parse_relay_body(
            r#"{"why":"w","actions":[{"action":"gateway.enable","carrier":"ntfy","mode":"5A"},{"action":"hub.pair"},{"action":"carrier.flush"}]}"#,
        )
        .unwrap();
        assert_eq!(
            a,
            vec![
                RelayAction::GatewayEnable {
                    carrier: CarrierKind::Ntfy,
                    mode: CarrierMode::FiveA
                },
                RelayAction::HubPair,
                RelayAction::CarrierFlush
            ]
        );
        let code = |b: &str| parse_relay_body(b).unwrap_err().problem.code;
        // presence.interval before anything about its arguments
        assert_eq!(
            code(r#"{"why":"w","actions":[{"action":"presence.interval","transport":"x"}]}"#),
            "unsupported_action"
        );
        assert_eq!(
            code(
                r#"{"why":"w","actions":[{"action":"presence.interval","transport":"tcp-lan","minutes":30}]}"#
            ),
            "unsupported_action"
        );
        assert_eq!(
            code(r#"{"why":"w","actions":[{"action":"carrier.flush"},{"action":"rns.restart"}]}"#),
            "unsupported_action"
        );
        for bad in [
            "not json",
            "[]",
            r#"{"actions":[{"action":"hub.pair"}]}"#,
            r#"{"why":"w","actions":[]}"#,
            r#"{"why":"w","actions":{}}"#,
            r#"{"why":"w","actions":[{"action":"hub.pair"}],"extra":1}"#,
            r#"{"why":"w","actions":["hub.pair"]}"#,
            r#"{"why":"w","actions":[{"action":"hub.pair","code":"X"}]}"#,
            r#"{"why":"w","actions":[{"action":"gateway.enable","carrier":"chatmail","mode":"5A"}]}"#,
            r#"{"why":"w","actions":[{"action":"gateway.enable","carrier":"ntfy"}]}"#,
            r#"{"why":"w","actions":[{"action":"gateway.enable","carrier":"ntfy","mode":"5A","server":"x"}]}"#,
            r#"{"why":"w","actions":[{"action":"carrier.flush"},{"action":"carrier.flush"}]}"#,
        ] {
            let e = parse_relay_body(bad).unwrap_err();
            assert_eq!(e.problem.status, 502, "{bad}");
            assert_eq!(e.problem.code, "upstream_error", "{bad}");
        }
    }

    #[test]
    fn the_status_tls_says_what_the_link_runs() {
        use crate::daemon::fabric::settings::{LinkSpki, ED25519_SPKI_PREFIX};
        let mut der = ED25519_SPKI_PREFIX.to_vec();
        der.extend_from_slice(&[7u8; 32]);
        let key = LinkSpki::from_der(&der).unwrap();
        let far = id111::keyset::NodeId::from([9u8; 32]);
        let private: SocketAddr = "192.168.1.7:11183".parse().unwrap();
        let public: SocketAddr = "203.0.113.7:11183".parse().unwrap();
        let link = |lan, key: Option<LinkSpki>, warned: bool| PairLink {
            lan,
            link_spki: key,
            warning_hash: warned.then_some([1u8; 32]),
            ..PairLink::new(far)
        };
        let tcp = PairVia::Tcp;
        // ruling (c): a private meeting address, no mark: plain by §4.1 rule 1, shown as none
        assert_eq!(
            tls_of(Some(&link(false, Some(key), false)), tcp, Some(private)),
            "none"
        );
        assert_eq!(
            tls_of(Some(&link(true, Some(key), false)), tcp, Some(public)),
            "none"
        );
        assert_eq!(
            tls_of(Some(&link(false, Some(key), false)), tcp, Some(public)),
            "rpk"
        );
        assert_eq!(
            tls_of(Some(&link(false, None, true)), tcp, Some(public)),
            "plain"
        );
        // never plain without a recorded warning, never rpk without a key
        assert_eq!(
            tls_of(Some(&link(false, None, false)), tcp, Some(public)),
            "none"
        );
        assert_eq!(
            tls_of(
                Some(&link(false, Some(key), false)),
                PairVia::Carrier,
                Some(public)
            ),
            "none"
        );
        // no half here (the far hub enrolled from this hub's bundle)
        assert_eq!(tls_of(None, tcp, Some(private)), "none");
        assert_eq!(tls_of(None, tcp, Some(public)), "rpk");
        assert_eq!(tls_of(None, PairVia::Carrier, None), "none");
    }

    #[test]
    fn without_ages_drops_only_the_ages() {
        let v = json!({"gateway": {"state": "carrier_ok", "checked_ago_s": 3, "last_flush_ago_s": 4}, "x": 1});
        assert_eq!(
            without_ages(&v),
            json!({"gateway": {"state": "carrier_ok"}, "x": 1})
        );
    }
}
