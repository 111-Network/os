//! `GET /api/v1/status` — the `Status` schema (`os/interface/api/openapi.yaml`):
//! a strict superset of `/api/status` (every key it has, same values) plus
//! `node.label/bech32/alias`, `update`, `intelligence.agent_state/model_id`,
//! `bind`, `lan_enabled`, `auth` and `layers`. Owned by Layer 4 task R2.
//!
//! Field sources:
//! - `node.label` / `node.bech32` — `id111::encoding::NodeIdDisplay` over the
//!   public NodeId (display forms only; no key material, ever).
//! - `node.alias` — `state/settings.json` (`PATCH /api/v1/settings`).
//! - `update` — derived per request from state files + the live updater:
//!   `state` = `reverting` (`state/revert-request.json` present) ›
//!   `probation` (`state/update.json` journal pending) › `staged`
//!   (`state/update-request.json` present) › `checking` (a check is running) ›
//!   `failed` (last check this lifetime was refused/degraded) › `idle`;
//!   `pending_version` from the request file (else the probation journal);
//!   `last_check` from `state/last-update-check.json`; `last_result` from the
//!   updater's last outcome (null before the first check); `clock_fault` =
//!   `state/clock-fault.json` present.
//! - `intelligence.agent_state` / `model_id` — the services registry's cached
//!   `GET :11182/v1/agent/status` (10 s), null while the agent is down.
//! - `lan_enabled` — the *configured* value (settings.json `lan_enabled`, unless
//!   `DASHBOARD_LAN` in the env file overrides it — `bind::lan_config`; takes
//!   effect on restart); `bind.lan` is the
//!   effective one.
//! - `auth` — this request: `loopback` = loopback peer, `paired` = carries a
//!   valid session cookie.
//! - `layers` — layers *working* right now (contract `Status.layers`): `0` iff
//!   the KB payload verified at start and `current/share/kb` is still
//!   readable, `1` always (this daemon), `2` iff the identity loaded ok, `3`
//!   iff `current/bin/111-agent` exists, `4` always (this dashboard answers).
//!
//! **Heartbeat** ([`spawn_heartbeat`]): every 2 s the snapshot is rebuilt and a
//! `status` frame is published on the events bus when it changed (diffed as
//! serialized JSON without the ever-ticking `node.uptime_sec`) and never more
//! than 30 s after the previous frame (the heartbeat fires one tick early). Event frames are broadcast to every subscriber, so their `auth`
//! block cannot describe one client: it is always `{loopback:false,
//! paired:false}` — clients keep `auth` from their own `GET /status`.

use super::{auth, settings, ApiRouter, ApiState};
use crate::daemon::dashboard::bind::BindInfo;
use crate::daemon::dashboard::{DashboardState, ErrorState, OfferState, TransportState};
use crate::daemon::health::{evaluate, HealthStatus, TransportHealth};
use crate::daemon::intelligence::ChildDisplay;
use crate::daemon::services::Registry;
use crate::daemon::updater::{LastResult, UpdaterStatus};
use axum::extract::State;
use axum::http::{Extensions, HeaderMap};
use axum::routing::get;
use axum::Json;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Heartbeat rebuild cadence and the maximum gap between `status` frames.
pub const STATUS_TICK: Duration = Duration::from_secs(2);
pub const HEARTBEAT: Duration = Duration::from_secs(30);

pub fn router() -> ApiRouter {
    axum::Router::new().route("/api/v1/status", get(get_status))
}

// ---------------------------------------------------------------- schema

#[derive(Debug, Serialize)]
pub struct Status {
    pub schema: u32,
    pub node: Node,
    pub kb: Option<Kb>,
    pub transport_state: TransportState,
    pub health: Health,
    pub update: Update,
    pub offer: Option<OfferState>,
    pub error_state: Option<ErrorState>,
    pub intelligence: Intelligence,
    pub bind: BindInfo,
    pub lan_enabled: bool,
    pub auth: AuthView,
    pub layers: Vec<u8>,
    /// `/api/status` compat (superset): same object as there.
    pub updates: Updates,
    /// `/api/status` compat (superset); equals `update.clock_fault`.
    pub clock_fault: bool,
}

#[derive(Debug, Serialize)]
pub struct Node {
    pub node_id: Option<String>,
    pub label: Option<String>,
    pub bech32: Option<String>,
    pub alias: Option<String>,
    pub category: Option<String>,
    pub version: String,
    pub uptime_sec: u64,
}

#[derive(Debug, Serialize)]
pub struct Kb {
    pub sha256: String,
    pub entries: u32,
}

#[derive(Debug, Serialize)]
pub struct Health {
    pub schema: u32,
    pub status: &'static str,
    pub components: HealthComponents,
}

#[derive(Debug, Serialize)]
pub struct HealthComponents {
    pub identity: bool,
    pub kb: bool,
    pub transport: &'static str,
    pub updater_idle: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateState {
    Idle,
    Checking,
    Staged,
    Probation,
    Reverting,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Update {
    pub state: UpdateState,
    pub pending: bool,
    pub pending_version: Option<String>,
    pub last_check: Option<String>,
    pub last_result: Option<LastResult>,
    pub floor: Option<String>,
    pub clock_fault: bool,
}

#[derive(Debug, Serialize)]
pub struct Updates {
    pub last_successful_check: Option<String>,
    pub pending: bool,
    pub floor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Intelligence {
    pub agent: ChildDisplay,
    pub model_server: ChildDisplay,
    pub agent_url: Option<String>,
    pub agent_state: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct AuthView {
    pub loopback: bool,
    pub paired: bool,
}

// ---------------------------------------------------------------- derivation

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// The `update` block from `state/` files, the dashboard state and the live
/// updater (`None` when no updater runs in this process).
pub fn update_view(
    state_dir: &Path,
    dash: &DashboardState,
    updater: Option<&UpdaterStatus>,
) -> Update {
    let journal = crate::probation::Journal::read(&state_dir.join("update.json"))
        .ok()
        .flatten()
        .filter(|j| j.pending);
    let request = read_json(&state_dir.join("update-request.json"));
    let request_version = request
        .as_ref()
        .and_then(|r| r["version"].as_str().map(str::to_owned));
    let reverting = state_dir.join("revert-request.json").exists();
    let last_result = updater.and_then(UpdaterStatus::last_result);
    let state = if reverting {
        UpdateState::Reverting
    } else if journal.is_some() {
        UpdateState::Probation
    } else if request.is_some() {
        UpdateState::Staged
    } else if updater.is_some_and(UpdaterStatus::checking) {
        UpdateState::Checking
    } else if matches!(
        last_result,
        Some(LastResult::Refused | LastResult::Degraded)
    ) {
        UpdateState::Failed
    } else {
        UpdateState::Idle
    };
    let last_check = read_json(&state_dir.join("last-update-check.json"))
        .and_then(|v| v["at"].as_str().map(str::to_owned))
        .or_else(|| dash.last_successful_update_check.clone());
    Update {
        state,
        pending: dash.update_pending || request.is_some(),
        pending_version: request_version.or_else(|| journal.map(|j| j.version)),
        last_check,
        last_result,
        floor: dash.floor_release.clone(),
        clock_fault: state_dir.join("clock-fault.json").exists(),
    }
}

/// 64 lowercase hex → 32 bytes.
fn decode_hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in s.as_bytes().chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

/// `(label, bech32)` display forms of a NodeId hex string.
pub fn display_forms(node_id: Option<&str>) -> (Option<String>, Option<String>) {
    match node_id.and_then(decode_hex32) {
        Some(bytes) => {
            let d = id111::encoding::NodeIdDisplay::new(&bytes);
            (Some(d.label()), Some(d.bech32()))
        }
        None => (None, None),
    }
}

/// Layers working right now (see module docs for the rule). `kb_verified` =
/// the KB passed its integrity check at start; `identity_ok` = the identity
/// loaded (the `/health` identity component).
pub fn layers(
    layout: &crate::daemon::DaemonLayout,
    kb_verified: bool,
    identity_ok: bool,
) -> Vec<u8> {
    let mut l = Vec::new();
    if kb_verified && std::fs::read_dir(layout.current_release().join("share/kb")).is_ok() {
        l.push(0);
    }
    l.push(1);
    if identity_ok {
        l.push(2);
    }
    if crate::daemon::intelligence::paths::agent_binary(layout).is_file() {
        l.push(3);
    }
    l.push(4);
    l
}

fn transport_label(t: TransportHealth) -> &'static str {
    match t {
        TransportHealth::Alive => "alive",
        TransportHealth::DeliberatelyAbsent => "absent",
        TransportHealth::Degraded => "degraded",
        TransportHealth::Down => "down",
    }
}

/// Build the full snapshot (`auth` as given: per request, or the broadcast
/// placeholder).
pub fn build(api: &ApiState, auth: AuthView) -> Status {
    let state_dir = api.layout.state_dir();
    let updater = api.ext::<Arc<UpdaterStatus>>();
    let agent_status = api.ext::<Arc<Registry>>().and_then(|r| r.agent_status());
    let alias = settings::read_file(&state_dir).alias;
    let lan_enabled = settings::lan_configured(api);
    let bind = api.bind.read().unwrap().clone();
    let s = api.dash.read().unwrap();
    let layers = layers(&api.layout, s.kb.is_some(), s.components.identity);
    let update = update_view(&state_dir, &s, updater.as_deref());
    let (label, bech32) = display_forms(s.node_id.as_deref());
    let agent_running = s.intelligence.agent == ChildDisplay::Running;
    let agent_status = agent_status.filter(|_| agent_running);
    Status {
        schema: 1,
        node: Node {
            node_id: s.node_id.clone(),
            label,
            bech32,
            alias,
            category: s.category.clone(),
            version: s.version.clone(),
            uptime_sec: s.uptime_sec(),
        },
        kb: s.kb.as_ref().map(|k| Kb {
            sha256: k.sha256.clone(),
            entries: k.entries,
        }),
        transport_state: s.transport_state,
        health: Health {
            schema: 1,
            status: match evaluate(&s.components) {
                HealthStatus::Ok => "ok",
                HealthStatus::Error => "error",
            },
            components: HealthComponents {
                identity: s.components.identity,
                kb: s.components.kb,
                transport: transport_label(s.components.transport),
                updater_idle: s.components.updater_idle,
            },
        },
        updates: Updates {
            last_successful_check: update.last_check.clone(),
            pending: update.pending,
            floor: update.floor.clone(),
        },
        clock_fault: update.clock_fault,
        update,
        offer: s.offer.clone(),
        error_state: s.error_state.clone(),
        intelligence: Intelligence {
            agent: s.intelligence.agent,
            model_server: s.intelligence.model_server,
            agent_url: s.intelligence.agent_url.clone(),
            agent_state: agent_status.as_ref().and_then(|a| a.state.clone()),
            model_id: agent_status.and_then(|a| a.model_id),
        },
        bind,
        lan_enabled,
        auth,
        layers,
    }
}

async fn get_status(
    State(api): State<Arc<ApiState>>,
    ext: Extensions,
    headers: HeaderMap,
) -> Json<Status> {
    let view = AuthView {
        loopback: auth::peer_is_loopback(&ext),
        paired: api.auth.session_in(&headers).is_some(),
    };
    Json(build(&api, view))
}

// ---------------------------------------------------------------- heartbeat

/// Last published `status` frame (diff key + time), shared by the heartbeat
/// and on-demand publishes (settings changes).
pub struct StatusPublisher {
    last: Mutex<Option<(String, Instant)>>,
    /// The heartbeat's rebuild tick (the 30 s bound is kept within one tick).
    tick: Duration,
}

impl Default for StatusPublisher {
    fn default() -> Self {
        StatusPublisher {
            last: Mutex::new(None),
            tick: STATUS_TICK,
        }
    }
}

/// `auth` of broadcast `status` frames: a placeholder; clients keep the `auth`
/// from their own `GET /status` and ignore the frame's (UI store, commit 4b5ba16).
pub const FRAME_AUTH: AuthView = AuthView {
    loopback: false,
    paired: false,
};

/// Is a heartbeat frame due? Fires one tick early so that, with a tick of
/// `tick`, consecutive `status` frames are never more than [`HEARTBEAT`]
/// apart (the UI marks the stream stale after 45 s of silence).
pub fn heartbeat_due(since_last: Duration, tick: Duration) -> bool {
    since_last + tick >= HEARTBEAT
}

impl StatusPublisher {
    /// Publish a `status` frame when the snapshot changed, when `force`, or
    /// when the heartbeat is due ([`heartbeat_due`]). Returns whether it
    /// published.
    pub fn publish(&self, api: &ApiState, force: bool) -> bool {
        let snapshot = serde_json::to_value(build(api, FRAME_AUTH)).unwrap_or(Value::Null);
        let mut key = snapshot.clone();
        if let Some(node) = key.get_mut("node").and_then(Value::as_object_mut) {
            node.remove("uptime_sec");
        }
        let key = key.to_string();
        let mut last = self.last.lock().unwrap();
        let due = match last.as_ref() {
            None => true,
            Some((k, at)) => force || *k != key || heartbeat_due(at.elapsed(), self.tick),
        };
        if due {
            api.events.publish("status", &snapshot);
            *last = Some((key, Instant::now()));
        }
        due
    }
}

/// Publish a `status` frame now (after a settings change / update request).
pub fn publish_now(api: &ApiState) {
    match api.ext::<Arc<StatusPublisher>>() {
        Some(p) => {
            p.publish(api, true);
        }
        None => {
            let snapshot = serde_json::to_value(build(api, FRAME_AUTH)).unwrap_or(Value::Null);
            api.events.publish("status", &snapshot);
        }
    }
}

/// Start the status heartbeat (daemon start hook; tests may call it).
pub fn spawn_heartbeat(api: Arc<ApiState>) -> Arc<StatusPublisher> {
    spawn_heartbeat_every(api, STATUS_TICK)
}

/// [`spawn_heartbeat`] with a custom rebuild tick (tests).
pub fn spawn_heartbeat_every(api: Arc<ApiState>, tick: Duration) -> Arc<StatusPublisher> {
    let publisher = Arc::new(StatusPublisher {
        last: Mutex::new(None),
        tick,
    });
    api.insert_ext(Arc::clone(&publisher));
    let p = Arc::clone(&publisher);
    let weak = Arc::downgrade(&api);
    drop(api);
    tokio::spawn(async move {
        let mut t = tokio::time::interval(tick);
        t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            t.tick().await;
            let Some(api) = weak.upgrade() else { break };
            p.publish(&api, false);
        }
    });
    publisher
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_never_exceeds_30s() {
        // with a 2 s tick the frame goes out at 28 s, so the next tick (≤ 30 s
        // after the last frame) is never needed
        assert!(!heartbeat_due(Duration::from_secs(27), STATUS_TICK));
        assert!(heartbeat_due(Duration::from_secs(28), STATUS_TICK));
        assert!(heartbeat_due(Duration::from_secs(31), STATUS_TICK));
        assert!(!heartbeat_due(
            Duration::from_millis(29_900),
            Duration::ZERO
        ));
        assert!(heartbeat_due(HEARTBEAT, Duration::ZERO));
    }

    #[test]
    fn hex_decode() {
        assert!(decode_hex32("ab").is_none());
        assert!(decode_hex32(&"zz".repeat(32)).is_none());
        assert_eq!(decode_hex32(&"0f".repeat(32)), Some([0x0f; 32]));
        let (label, bech) = display_forms(Some(&"11".repeat(32)));
        assert!(bech.unwrap().starts_with("node1"));
        let label = label.unwrap();
        assert_eq!(label.len(), 12, "{label}");
        assert_eq!(display_forms(None), (None, None));
        assert_eq!(display_forms(Some("nothex")), (None, None));
    }
}
