//! The narrow apply → verify → undo cut (`os/60-transport/README.md` §12
//! "Proposals" and its table; D17 #7, P4; DEV §3 "Apply loop: snapshot →
//! apply → probe within 30 s → keep or revert, result written to the
//! proposal. Unknown action = refused, never best-effort").
//!
//! A `kind: device` proposal carries one JSON object `{"why", "actions":
//! [{"action": name, …args}]}` (`40-interface/api/openapi.yaml`
//! `Proposal.kind`). [`ApplierRequest::from_body`] parses it against the
//! closed vocabulary:
//!
//! | Action | Apply (through `Spine::update`, never a render or restart of our own) | Probe within [`PROBE_WINDOW`] (monotonic) |
//! |---|---|---|
//! | `rns.interface.upsert {device_id, type, params}` | the RNode entry for the device's port is written (a region must be in effect: else refused `region_unset`), the spine renders and `rnstatus --attach`es it | the interface is listed by `rnstatus` **and up** (an RNode that cannot open shows down while rnsd runs: that is a revert) |
//! | `rns.interface.remove {name}` | the entry is dropped, the spine `--detach`es it | absent from `rnstatus` |
//! | `rns.posture.set {share_instance?, enable_transport?}` | the spine renders, stops the helper(s), restarts rnsd, starts the helper(s) | rnsd up, the shared instance listed and up (when sharing), the helpers re-attached |
//! | `radio.region.set {region}` | `[radio] region` written (acceptance is the user's confirmation) | the file re-parses with that region; no radio effect by itself |
//! | `identify {device_id}` | the consented identify ([`OneShots::identify`]) | — (nothing to revert) |
//! | `service.restart {service_id}` | a supervised child restarts ([`OneShots::restart`], `409 not_supervised` otherwise) | — |
//!
//! `meshtastic.set`, `ble.pair`, `ble.forget`, `firmware.flash` and any
//! other name are refused `409 unsupported_action` before anything is
//! written; so is a malformed action (`400`).
//!
//! **The loop.** Every action of a proposal is first checked against a copy
//! of the file as the earlier actions leave it (a region set by an earlier
//! action counts for a later upsert); a refusal there writes nothing and the
//! proposal stays open. Then the decision record is written **pending**
//! (`applied: {pending: true, actions}`, the Layer 5 §15 pattern) before the
//! first effect, and per action: **snapshot** (`interfaces.toml` and
//! `rns/config` bytes plus the parsed file) → **apply** → **probe** →
//! **keep**, or **revert** (the snapshot restored through the spine — which
//! `--detach`es / `--attach`es back or restarts rnsd again — and the TOML
//! bytes put back byte-exact). The first action that is not kept stops the
//! proposal (later ones are `skipped`); the result `{applied, reverted,
//! reason, actions}` replaces the pending record. A crash mid-apply leaves
//! the pending record on disk: the proposal reads decided and is **never
//! applied a second time** (a retry is `409 already_decided`).
//!
//! One proposal applies at a time on a node. Nothing here runs on a timer:
//! the probe polls only inside an accepted apply.
//!
//! **Daemon wiring (task C1).** `N/mod.rs` builds the production applier
//! once the spine, the rnsd child and the components exist and inserts it
//! into `ApiState` ext:
//! `api.insert_ext(Applier::production(&api, transport.spine(),
//! transport.control().map(|c| Arc::new(c) as Arc<dyn RnsdChild>),
//! Some(Arc::new(components.clone()) as Arc<dyn HelperCheck>)))`. The
//! device proposal accept arm in `N/dashboard/api_v1/proposals.rs` then
//! calls, after its own checks (kind, node stamp, the agent bearer refused):
//!
//! ```ignore
//! let applier = api.ext::<Arc<devices::applier::Applier>>()?;
//! let req = devices::applier::ApplierRequest::from_body(&p.id, &p.body)?; // BodyError::problem()
//! let report = devices::applier::apply(&applier, req).await?;             // ApplyError::problem()
//! // 200 with the proposal; `report` is what the decision record holds
//! ```

use super::registry::{DeviceRegistry, TargetError};
use super::{valid_id, MAX_FACT};
use crate::daemon::components::consent::ProblemWith;
use crate::daemon::components::registry::ComponentState;
use crate::daemon::components::Components;
use crate::daemon::dashboard::api_v1::Problem;
use crate::daemon::dashboard::ApiState;
use crate::daemon::ports::{port_key, Owner, PortBusy};
use crate::daemon::services::ServiceId;
use crate::daemon::supervise::ChildState;
use crate::daemon::transport::config;
use crate::daemon::transport::interfaces::{
    self, IfKind, IfOwner, Interface, InterfacesFile, Invalid,
};
use crate::daemon::transport::posture::{ApplyError as SpineError, RnsdChild, Spine};
use crate::daemon::transport::rnstatus::InterfaceFacts;
use crate::daemon::DaemonLayout;
use axum::http::StatusCode;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::time::Duration;

/// How long a probe may take to see the change (§12: "within 30 s").
pub const PROBE_WINDOW: Duration = Duration::from_secs(30);
/// Pause between probe readings.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(1);
/// Actions in one proposal (the agent's ceiling too).
pub const MAX_ACTIONS: usize = 8;
/// Largest device proposal body parsed (checked before the parse).
pub const MAX_BODY: usize = 16 * 1024;
/// The four applied actions (§12).
pub const APPLIED_ACTIONS: [&str; 4] = [
    "rns.interface.upsert",
    "rns.interface.remove",
    "rns.posture.set",
    "radio.region.set",
];
/// The two one-shot actions (§12).
pub const ONE_SHOT_ACTIONS: [&str; 2] = ["identify", "service.restart"];
/// Vocabulary entries refused `unsupported_action` (§12).
pub const REFUSED_ACTIONS: [&str; 4] =
    ["meshtastic.set", "ble.pair", "ble.forget", "firmware.flash"];
/// The only stanza type a device upsert writes in v1.
pub const UPSERT_TYPE: &str = "RNodeInterface";

/// A boxed `Send` future (object-safe async hooks without a macro crate).
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ---------------------------------------------------------------- vocabulary

/// One action of the closed vocabulary (DEV §3, §12).
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    InterfaceUpsert {
        device_id: String,
        kind: String,
        params: Map<String, Value>,
    },
    InterfaceRemove {
        name: String,
    },
    PostureSet {
        share_instance: Option<bool>,
        enable_transport: Option<bool>,
    },
    RegionSet {
        region: String,
    },
    Identify {
        device_id: String,
    },
    ServiceRestart {
        service_id: String,
    },
}

impl Action {
    /// The vocabulary name.
    pub fn name(&self) -> &'static str {
        match self {
            Action::InterfaceUpsert { .. } => "rns.interface.upsert",
            Action::InterfaceRemove { .. } => "rns.interface.remove",
            Action::PostureSet { .. } => "rns.posture.set",
            Action::RegionSet { .. } => "radio.region.set",
            Action::Identify { .. } => "identify",
            Action::ServiceRestart { .. } => "service.restart",
        }
    }

    /// Parse one `{"action": name, …args}` (unknown fields refused; `null`
    /// counts as absent, as the agent writes it).
    pub fn parse(v: &Value) -> Result<Action, Refusal> {
        let obj = v
            .as_object()
            .ok_or_else(|| Refusal::Bad("an action must be an object".into()))?;
        let name = obj
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| Refusal::Bad("an action needs a string \"action\"".into()))?;
        let allowed: &[&str] = match name {
            "rns.interface.upsert" => &["device_id", "type", "params"],
            "rns.interface.remove" => &["name"],
            "rns.posture.set" => &["share_instance", "enable_transport"],
            "radio.region.set" => &["region"],
            "identify" => &["device_id"],
            "service.restart" => &["service_id"],
            other => return Err(Refusal::Unsupported(shown(other))),
        };
        for (k, val) in obj {
            if k != "action" && !val.is_null() && !allowed.contains(&k.as_str()) {
                return Err(Refusal::Bad(format!("{name} has no field {:?}", shown(k))));
            }
        }
        let get = |k: &str| obj.get(k).filter(|v| !v.is_null());
        let text = |k: &str, max: usize| -> Result<String, Refusal> {
            let s = get(k)
                .and_then(Value::as_str)
                .ok_or_else(|| Refusal::Bad(format!("{name} needs a string {k:?}")))?;
            if s.is_empty() || s.len() > max || s.chars().any(char::is_control) {
                return Err(Refusal::Bad(format!("{name}: {k} is not acceptable")));
            }
            Ok(s.to_string())
        };
        let device = |k: &str| -> Result<String, Refusal> {
            let id = text(k, 64)?;
            if !valid_id(&id) {
                return Err(Refusal::Bad(format!("{name}: {k} is not a device id")));
            }
            Ok(id)
        };
        Ok(match name {
            "rns.interface.upsert" => {
                let params = get("params")
                    .and_then(Value::as_object)
                    .cloned()
                    .ok_or_else(|| Refusal::Bad("rns.interface.upsert needs \"params\"".into()))?;
                Action::InterfaceUpsert {
                    device_id: device("device_id")?,
                    kind: text("type", 64)?,
                    params,
                }
            }
            "rns.interface.remove" => Action::InterfaceRemove {
                name: text("name", interfaces::MAX_NAME_BYTES)?,
            },
            "rns.posture.set" => {
                let flag = |k: &str| -> Result<Option<bool>, Refusal> {
                    match get(k) {
                        None => Ok(None),
                        Some(Value::Bool(b)) => Ok(Some(*b)),
                        Some(_) => Err(Refusal::Bad(format!("rns.posture.set: {k} is a boolean"))),
                    }
                };
                let (s, e) = (flag("share_instance")?, flag("enable_transport")?);
                if s.is_none() && e.is_none() {
                    return Err(Refusal::Bad(
                        "rns.posture.set needs share_instance and/or enable_transport".into(),
                    ));
                }
                Action::PostureSet {
                    share_instance: s,
                    enable_transport: e,
                }
            }
            "radio.region.set" => Action::RegionSet {
                region: text("region", 32)?,
            },
            "identify" => Action::Identify {
                device_id: device("device_id")?,
            },
            _ => {
                let id = text("service_id", 32)?;
                if !id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                {
                    return Err(Refusal::Bad(
                        "service.restart: service_id is not an id".into(),
                    ));
                }
                Action::ServiceRestart { service_id: id }
            }
        })
    }

    fn summary(&self) -> String {
        match self {
            Action::InterfaceUpsert { device_id, .. } => format!("{} {device_id}", self.name()),
            Action::InterfaceRemove { name } => format!("{} {}", self.name(), shown(name)),
            Action::RegionSet { region } => format!("{} {}", self.name(), shown(region)),
            Action::Identify { device_id } => format!("{} {device_id}", self.name()),
            Action::ServiceRestart { service_id } => format!("{} {service_id}", self.name()),
            Action::PostureSet { .. } => self.name().to_string(),
        }
    }
}

/// Untrusted text in a message: control characters dropped, ≤ 64 chars.
fn shown(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(MAX_FACT)
        .collect()
}

/// Why an action (or a proposal) was refused before anything was applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Not one of the six actions applied here (`409 unsupported_action`).
    Unsupported(String),
    /// A malformed or unacceptable action (`400 bad_request`).
    Bad(String),
    /// An RNode entry with no region in effect (`400`, `reason_code:
    /// region_unset`; D17 #10: a human sets the region first).
    RegionUnset(String),
    /// The device or the interface does not exist (`404`).
    NotFound(String),
    /// The device's port has another owner (`409 port_busy`).
    Busy(PortBusy),
    /// `interfaces.toml` belongs to a newer release (`409 settings_locked`).
    Locked,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Unsupported(n) => write!(
                f,
                "{n:?} is not applied by 111d (supported: {}, {})",
                APPLIED_ACTIONS.join(", "),
                ONE_SHOT_ACTIONS.join(", ")
            ),
            Refusal::Bad(e) | Refusal::NotFound(e) => f.write_str(e),
            Refusal::RegionUnset(name) => write!(
                f,
                "interface {name:?}: no radio region is set; a human sets it first"
            ),
            Refusal::Busy(b) => b.fmt(f),
            Refusal::Locked => {
                f.write_str("interfaces.toml belongs to a newer 111 release; nothing is changed")
            }
        }
    }
}

impl Refusal {
    /// The contract problem.
    pub fn problem(&self) -> ProblemWith {
        let detail = self.to_string();
        match self {
            Refusal::Unsupported(_) => ProblemWith::new(
                Problem::new(
                    StatusCode::CONFLICT,
                    "unsupported_action",
                    "This change is not applied by the node.",
                )
                .detail(detail),
            ),
            Refusal::Bad(_) => ProblemWith::new(
                Problem::new(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    "The proposed change was refused.",
                )
                .detail(detail),
            ),
            Refusal::RegionUnset(_) => ProblemWith::new(
                Problem::new(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    "Set the radio region first.",
                )
                .detail(detail),
            )
            .with("reason_code", Value::from("region_unset")),
            Refusal::NotFound(_) => ProblemWith::new(
                Problem::new(StatusCode::NOT_FOUND, "not_found", "Nothing by that name.")
                    .detail(detail),
            ),
            Refusal::Busy(b) => ProblemWith::new(b.problem()),
            Refusal::Locked => ProblemWith::new(
                Problem::new(
                    StatusCode::CONFLICT,
                    "settings_locked",
                    "The transport settings are read-only here.",
                )
                .detail(detail),
            ),
        }
    }
}

/// Why a device proposal body could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyError {
    /// Not one JSON object `{"why", "actions": [..]}` (`502 upstream_error`:
    /// the agent wrote something the contract does not allow).
    Shape(String),
    /// An action was refused (its problem).
    Refused(Refusal),
}

impl BodyError {
    pub fn problem(&self) -> ProblemWith {
        match self {
            BodyError::Shape(e) => ProblemWith::new(
                Problem::new(
                    StatusCode::BAD_GATEWAY,
                    "upstream_error",
                    "The proposal is not a device proposal 111d can read.",
                )
                .detail(e.clone()),
            ),
            BodyError::Refused(r) => r.problem(),
        }
    }
}

/// What C1's accept arm hands the applier: the proposal id and its actions.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplierRequest {
    /// `^[A-Za-z0-9_-]{1,128}$` (the decision record's name).
    pub proposal_id: String,
    pub actions: Vec<Action>,
}

/// `^[A-Za-z0-9_-]{1,128}$` (the proposals' id rule).
fn valid_proposal_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl ApplierRequest {
    /// A request over already-parsed actions (1..=[`MAX_ACTIONS`]).
    pub fn new(proposal_id: &str, actions: Vec<Action>) -> Result<Self, Refusal> {
        if !valid_proposal_id(proposal_id) {
            return Err(Refusal::Bad("not a proposal id".into()));
        }
        if actions.is_empty() || actions.len() > MAX_ACTIONS {
            return Err(Refusal::Bad(format!(
                "a device proposal holds 1 to {MAX_ACTIONS} actions"
            )));
        }
        Ok(ApplierRequest {
            proposal_id: proposal_id.to_string(),
            actions,
        })
    }

    /// Parse a `kind: device` body (module doc; size checked first).
    pub fn from_body(proposal_id: &str, body: &str) -> Result<Self, BodyError> {
        if body.len() > MAX_BODY {
            return Err(BodyError::Shape(format!(
                "the body is over {MAX_BODY} bytes"
            )));
        }
        let v: Value = serde_json::from_str(body)
            .map_err(|_| BodyError::Shape("the body is not one JSON object".into()))?;
        let obj = v
            .as_object()
            .ok_or_else(|| BodyError::Shape("the body is not one JSON object".into()))?;
        if obj.keys().any(|k| k != "why" && k != "actions") {
            return Err(BodyError::Shape(
                "the body has keys besides why and actions".into(),
            ));
        }
        if !obj.get("why").is_some_and(Value::is_string) {
            return Err(BodyError::Shape("the body has no why".into()));
        }
        let list = obj
            .get("actions")
            .and_then(Value::as_array)
            .ok_or_else(|| BodyError::Shape("the body has no actions list".into()))?;
        let actions = list
            .iter()
            .map(Action::parse)
            .collect::<Result<Vec<_>, _>>()
            .map_err(BodyError::Refused)?;
        Self::new(proposal_id, actions).map_err(BodyError::Refused)
    }
}

// ---------------------------------------------------------------- results

/// One action's result in the decision record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionResult {
    pub action: String,
    /// Applied and kept (a one-shot: ran).
    pub applied: bool,
    /// Applied, the probe failed, the snapshot restored.
    pub reverted: bool,
    /// Not reached: an earlier action was not kept.
    pub skipped: bool,
    /// Why it was not kept (stable words first: `probe_failed`, …).
    pub reason: Option<String>,
    /// A one-shot's answer (identify: the family found).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

/// The whole result (`applied` in the decision record).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApplyReport {
    /// Every action applied and kept.
    pub applied: bool,
    /// Some action was reverted.
    pub reverted: bool,
    /// The first reason an action was not kept.
    pub reason: Option<String>,
    pub actions: Vec<ActionResult>,
}

/// Why nothing (or not all) was applied, as the accept arm answers it.
#[derive(Debug, Clone)]
pub enum ApplyError {
    /// A decision record exists (a retry, or a crash mid-apply): never a
    /// second apply (`409 already_decided`).
    AlreadyDecided,
    /// Refused before anything was written (the proposal stays open).
    Refused(Refusal),
    /// A one-shot refused before any effect (the proposal stays open).
    OneShot(ProblemWith),
    /// The decision record could not be written (`500`).
    Record(String),
}

impl ApplyError {
    pub fn problem(&self) -> ProblemWith {
        match self {
            ApplyError::AlreadyDecided => ProblemWith::new(Problem::new(
                StatusCode::CONFLICT,
                "already_decided",
                "This proposal was already decided.",
            )),
            ApplyError::Refused(r) => r.problem(),
            ApplyError::OneShot(p) => p.clone(),
            ApplyError::Record(e) => ProblemWith::new(
                Problem::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "The decision could not be saved.",
                )
                .detail(e.clone()),
            ),
        }
    }
}

// ---------------------------------------------------------------- seams

/// Why a one-shot did not complete.
#[derive(Debug, Clone)]
pub enum OneShotError {
    /// Nothing happened (busy, not offered, not supervised, rate limited):
    /// the proposal reopens with this problem.
    Refused(ProblemWith),
    /// It ran and failed (recorded as not applied).
    Failed(String),
}

/// The two one-shots (§12). Production: [`DaemonOneShots`].
pub trait OneShots: Send + Sync {
    fn identify<'a>(&'a self, device_id: &'a str) -> BoxFut<'a, Result<Value, OneShotError>>;
    fn restart<'a>(&'a self, service_id: &'a str) -> BoxFut<'a, Result<Value, OneShotError>>;
}

/// What the probes read. Production: [`SpineProbe`].
pub trait ApplyProbe: Send + Sync {
    /// One `rnstatus` reading (`Err`: no answer, with why).
    fn interfaces(&self) -> BoxFut<'_, Result<Vec<InterfaceFacts>, String>>;
    /// The rnsd child runs.
    fn rnsd_running(&self) -> bool;
    /// Every helper that depends on rnsd is back (the posture probe).
    fn helpers_attached(&self) -> BoxFut<'_, bool>;
    /// `interfaces.toml` re-read from disk: its `[radio] region` (`Err`:
    /// it does not re-parse).
    fn file_region(&self) -> BoxFut<'_, Result<Option<String>, String>>;
}

/// The helpers that hang off rnsd (S4 components). `Components`
/// implements it: every enabled dependant of `transport` is `running`.
pub trait HelperCheck: Send + Sync {
    fn attached(&self) -> BoxFut<'_, bool>;
}

impl HelperCheck for Components {
    fn attached(&self) -> BoxFut<'_, bool> {
        Box::pin(async move {
            for id in self.dependants(ServiceId::Transport).await {
                match self.status(&id).await {
                    Some(s) if s.state == ComponentState::Running => {}
                    _ => return false,
                }
            }
            true
        })
    }
}

/// The device → port lookup an upsert needs. `DeviceRegistry` implements it.
pub trait DeviceLookup: Send + Sync {
    fn serial_port(&self, device_id: &str) -> Result<String, TargetError>;
}

impl DeviceLookup for DeviceRegistry {
    fn serial_port(&self, device_id: &str) -> Result<String, TargetError> {
        self.serial_port_for(device_id)
    }
}

/// Where the decision is recorded (the Layer 5 `applied: {pending: true}`
/// pattern). Production: [`DecisionFile`].
pub trait DecisionRecord: Send + Sync {
    /// Record the acceptance with `applied = pending`; `Err(None)` when a
    /// decision already exists, `Err(Some(why))` when it cannot be written.
    fn begin(&self, proposal_id: &str, pending: &Value) -> Result<(), Option<String>>;
    /// Replace the pending record with the result.
    fn finish(&self, proposal_id: &str, applied: &Value) -> Result<(), String>;
    /// Remove the pending record (a refusal before any effect).
    fn reopen(&self, proposal_id: &str);
}

/// `agent/decisions/<id>.json` in the shape `proposals.rs` reads
/// (`{schema: 1, decision, at, applied}`; dir 0700, file 0600, atomic). A
/// present file — valid or not — is a decision: fail closed, never a
/// second apply.
pub struct DecisionFile {
    dir: PathBuf,
}

impl DecisionFile {
    pub fn new(dir: PathBuf) -> Self {
        DecisionFile { dir }
    }

    /// The daemon's decisions dir (`proposals::decisions_dir`).
    pub fn production(layout: &DaemonLayout) -> Self {
        Self::new(crate::daemon::dashboard::api_v1::proposals::decisions_dir(
            layout,
        ))
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn write(&self, id: &str, at: &str, applied: &Value) -> Result<(), String> {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)
            .map_err(|e| e.to_string())?;
        let body = json!({"schema": 1, "decision": "accepted", "at": at, "applied": applied});
        crate::util::fs::atomic_write(&self.path(id), format!("{body}\n").as_bytes(), 0o600)
            .map_err(|e| format!("{e:#}"))
    }

    fn at_of(&self, id: &str) -> Option<String> {
        let f = std::fs::File::open(self.path(id)).ok()?;
        let mut buf = Vec::new();
        f.take(64 * 1024).read_to_end(&mut buf).ok()?;
        let v: Value = serde_json::from_slice(&buf).ok()?;
        v.get("at")?.as_str().map(str::to_string)
    }
}

fn rfc3339_now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

impl DecisionRecord for DecisionFile {
    fn begin(&self, proposal_id: &str, pending: &Value) -> Result<(), Option<String>> {
        if std::fs::symlink_metadata(self.path(proposal_id)).is_ok() {
            return Err(None);
        }
        // display only: no ordering decision reads it
        self.write(proposal_id, &rfc3339_now(), pending)
            .map_err(Some)
    }

    fn finish(&self, proposal_id: &str, applied: &Value) -> Result<(), String> {
        let at = self.at_of(proposal_id).unwrap_or_else(rfc3339_now);
        self.write(proposal_id, &at, applied)
    }

    fn reopen(&self, proposal_id: &str) {
        if let Err(e) = std::fs::remove_file(self.path(proposal_id)) {
            eprintln!("111 applier: {proposal_id}: the pending record stays: {e}");
        }
    }
}

/// The production probe: `rnstatus` through the spine, the rnsd child, the
/// components.
pub struct SpineProbe {
    spine: Arc<Spine>,
    rnsd: Option<Arc<dyn RnsdChild>>,
    helpers: Option<Arc<dyn HelperCheck>>,
}

impl SpineProbe {
    pub fn new(
        spine: Arc<Spine>,
        rnsd: Option<Arc<dyn RnsdChild>>,
        helpers: Option<Arc<dyn HelperCheck>>,
    ) -> Self {
        SpineProbe {
            spine,
            rnsd,
            helpers,
        }
    }
}

impl ApplyProbe for SpineProbe {
    fn interfaces(&self) -> BoxFut<'_, Result<Vec<InterfaceFacts>, String>> {
        Box::pin(async move {
            let rn = self.spine.rnstatus();
            tokio::task::spawn_blocking(move || rn.read())
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())
        })
    }

    fn rnsd_running(&self) -> bool {
        self.rnsd.as_ref().is_some_and(|c| c.running())
    }

    fn helpers_attached(&self) -> BoxFut<'_, bool> {
        Box::pin(async move {
            match &self.helpers {
                Some(h) => h.attached().await,
                None => true,
            }
        })
    }

    fn file_region(&self) -> BoxFut<'_, Result<Option<String>, String>> {
        Box::pin(async move {
            let path = interfaces::path(&self.spine.layout().state_dir());
            tokio::task::spawn_blocking(move || interfaces::read(&path))
                .await
                .map_err(|e| e.to_string())?
                .map(|f| f.radio.region)
                .map_err(|e| format!("interfaces.toml does not re-parse: {e}"))
        })
    }
}

/// The production one-shots: the identify orchestrator in `ApiState` ext
/// (consent = the acceptance) and a restart of the children 111d supervises
/// for the service (`services::Supervised`, as `POST /services/{id}/restart`).
pub struct DaemonOneShots {
    api: Weak<ApiState>,
}

impl DaemonOneShots {
    pub fn new(api: &Arc<ApiState>) -> Self {
        DaemonOneShots {
            api: Arc::downgrade(api),
        }
    }

    fn api(&self) -> Result<Arc<ApiState>, OneShotError> {
        self.api.upgrade().ok_or_else(|| {
            OneShotError::Refused(ProblemWith::new(Problem::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "internal",
                "The node is shutting down.",
            )))
        })
    }
}

impl OneShots for DaemonOneShots {
    fn identify<'a>(&'a self, device_id: &'a str) -> BoxFut<'a, Result<Value, OneShotError>> {
        Box::pin(async move {
            use super::identify::{Consent, Identify, IdentifyError};
            let api = self.api()?;
            let Some(ident) = api.ext::<Arc<Identify>>() else {
                return Err(OneShotError::Refused(ProblemWith::new(Problem::new(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    "Identify is not available on this node.",
                ))));
            };
            // the step runs to completion in its own task (a dropped request
            // must not leave a child running or a port claimed)
            let id = device_id.to_string();
            let a2 = Arc::clone(&api);
            let run = tokio::spawn(async move { ident.run(&a2, &id, Consent::Given).await });
            match run.await {
                Ok(Ok(report)) => Ok(json!({
                    "firmware": report.record.firmware,
                    "steps": report.steps.iter().map(|s| s.line()).collect::<Vec<_>>(),
                })),
                Ok(Err(e @ (IdentifyError::Inconclusive(_) | IdentifyError::Stopped(_)))) => {
                    Err(OneShotError::Failed(e.to_string()))
                }
                Ok(Err(e)) => Err(OneShotError::Refused(ProblemWith::new(e.problem()))),
                Err(e) => Err(OneShotError::Failed(format!(
                    "the identify step failed: {e}"
                ))),
            }
        })
    }

    fn restart<'a>(&'a self, service_id: &'a str) -> BoxFut<'a, Result<Value, OneShotError>> {
        Box::pin(async move {
            let api = self.api()?;
            let Some(sid) = ServiceId::parse(service_id) else {
                return Err(OneShotError::Refused(ProblemWith::new(
                    Problem::new(StatusCode::NOT_FOUND, "not_found", "No such service.")
                        .detail(shown(service_id)),
                )));
            };
            // supervised children only; the primary child decides (stopped
            // for good, at shutdown, is not supervised)
            let children = api
                .ext::<Arc<crate::daemon::services::Supervised>>()
                .map(|s| s.children(sid).to_vec())
                .unwrap_or_default();
            if children
                .first()
                .is_none_or(|c| c.state() == ChildState::Stopped)
            {
                return Err(OneShotError::Refused(ProblemWith::new(
                    Problem::new(
                        StatusCode::CONFLICT,
                        "not_supervised",
                        "This service is not run by the node itself.",
                    )
                    .detail(format!(
                        "{} has no child 111d supervises here.",
                        sid.as_str()
                    )),
                )));
            }
            eprintln!(
                "111 applier: restart of {} requested by an accepted proposal",
                sid.as_str()
            );
            tokio::spawn(async move {
                for child in children {
                    if child.state() != ChildState::Stopped {
                        child.restart(crate::daemon::CHILD_STOP_GRACE).await;
                    }
                }
            });
            Ok(json!({"schema": 1, "id": sid.as_str(), "restarting": true}))
        })
    }
}

// ---------------------------------------------------------------- applier

/// Timing (tests shorten it).
#[derive(Debug, Clone, Copy)]
pub struct ApplierConfig {
    pub probe_window: Duration,
    pub probe_interval: Duration,
}

impl Default for ApplierConfig {
    fn default() -> Self {
        ApplierConfig {
            probe_window: PROBE_WINDOW,
            probe_interval: PROBE_INTERVAL,
        }
    }
}

/// One action resolved against the simulated file.
#[derive(Debug, Clone)]
enum Planned {
    Upsert(Interface),
    Remove(String),
    Posture {
        share_instance: Option<bool>,
        enable_transport: Option<bool>,
    },
    Region(String),
    Identify(String),
    Restart(String),
}

/// What an action left behind to restore.
struct Snapshot {
    file: InterfacesFile,
    toml: Option<Vec<u8>>,
    config: Option<Vec<u8>>,
}

fn read_bytes(p: &Path) -> Option<Vec<u8>> {
    let f = std::fs::File::open(p).ok()?;
    let mut buf = Vec::new();
    f.take(config::MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut buf)
        .ok()?;
    Some(buf)
}

/// `Spine::update` saved `interfaces.toml` before it failed (the render or
/// the `rns/config` write failed): the change is on disk and must be undone.
fn saved_anyway(e: &SpineError) -> bool {
    matches!(e, SpineError::Write(_) | SpineError::NewerConfig)
}

/// JSON params (flat strings, numbers, booleans) → the entry's TOML table.
fn toml_params(params: &Map<String, Value>) -> Result<toml::Table, Refusal> {
    let mut out = toml::Table::new();
    for (k, v) in params {
        let tv = match v {
            Value::Bool(b) => toml::Value::Boolean(*b),
            Value::Number(n) => match (n.as_i64(), n.as_f64()) {
                (Some(i), _) => toml::Value::Integer(i),
                (None, Some(f)) if f.is_finite() => toml::Value::Float(f),
                _ => return Err(Refusal::Bad(format!("params.{} is not a number", shown(k)))),
            },
            Value::String(s) => toml::Value::String(s.clone()),
            _ => {
                return Err(Refusal::Bad(format!(
                    "params.{} must be a string, number or boolean",
                    shown(k)
                )))
            }
        };
        out.insert(k.clone(), tv);
    }
    Ok(out)
}

fn invalid(e: Invalid) -> Refusal {
    match e {
        Invalid::NoRegion(name) => Refusal::RegionUnset(name),
        other => Refusal::Bad(other.to_string()),
    }
}

/// The applier (module doc). Shared as `Arc<Applier>` in `ApiState` ext.
pub struct Applier {
    spine: Arc<Spine>,
    probe: Arc<dyn ApplyProbe>,
    devices: Arc<dyn DeviceLookup>,
    one_shots: Arc<dyn OneShots>,
    record: Arc<dyn DecisionRecord>,
    cfg: ApplierConfig,
    one: tokio::sync::Mutex<()>,
}

/// C1's call from the device proposal accept arm (module doc).
pub async fn apply(applier: &Applier, req: ApplierRequest) -> Result<ApplyReport, ApplyError> {
    applier.apply(req).await
}

impl Applier {
    pub fn new(
        spine: Arc<Spine>,
        probe: Arc<dyn ApplyProbe>,
        devices: Arc<dyn DeviceLookup>,
        one_shots: Arc<dyn OneShots>,
        record: Arc<dyn DecisionRecord>,
        cfg: ApplierConfig,
    ) -> Arc<Self> {
        Arc::new(Applier {
            spine,
            probe,
            devices,
            one_shots,
            record,
            cfg,
            one: tokio::sync::Mutex::new(()),
        })
    }

    /// The production applier (module doc, "Daemon wiring"): the registry
    /// from `api` ext, [`DaemonOneShots`], [`DecisionFile`], [`SpineProbe`].
    /// `None` without a device registry.
    pub fn production(
        api: &Arc<ApiState>,
        spine: Arc<Spine>,
        rnsd: Option<Arc<dyn RnsdChild>>,
        helpers: Option<Arc<dyn HelperCheck>>,
    ) -> Option<Arc<Self>> {
        let reg = api.ext::<Arc<DeviceRegistry>>()?;
        Some(Self::new(
            Arc::clone(&spine),
            Arc::new(SpineProbe::new(spine, rnsd, helpers)),
            reg,
            Arc::new(DaemonOneShots::new(api)),
            Arc::new(DecisionFile::production(&api.layout)),
            ApplierConfig::default(),
        ))
    }

    /// Apply a device proposal (module doc).
    pub async fn apply(&self, req: ApplierRequest) -> Result<ApplyReport, ApplyError> {
        let _one = self.one.lock().await;
        let plan = self.plan(&req.actions).map_err(ApplyError::Refused)?;
        let names: Vec<&str> = req.actions.iter().map(Action::name).collect();
        let pending = json!({"pending": true, "actions": names});
        match self.record.begin(&req.proposal_id, &pending) {
            Ok(()) => {}
            Err(None) => return Err(ApplyError::AlreadyDecided),
            Err(Some(e)) => return Err(ApplyError::Record(e)),
        }
        let mut results: Vec<ActionResult> = Vec::new();
        let mut stopped = false;
        for (action, step) in req.actions.iter().zip(plan) {
            if stopped {
                results.push(ActionResult {
                    action: action.name().into(),
                    applied: false,
                    reverted: false,
                    skipped: true,
                    reason: Some("skipped: an earlier action was not kept".into()),
                    result: None,
                });
                continue;
            }
            let r = match self.run_one(step).await {
                Ok(r) => r,
                Err(problem) => {
                    if results.is_empty() {
                        // the first action refused before any effect: the
                        // proposal reopens, nothing was applied
                        self.record.reopen(&req.proposal_id);
                        return Err(ApplyError::OneShot(problem));
                    }
                    ActionResult {
                        action: action.name().into(),
                        applied: false,
                        reverted: false,
                        skipped: false,
                        reason: Some(format!(
                            "refused: {}",
                            problem
                                .problem
                                .detail
                                .as_deref()
                                .unwrap_or(problem.problem.code)
                        )),
                        result: None,
                    }
                }
            };
            let r = ActionResult {
                action: action.name().into(),
                ..r
            };
            if !r.applied {
                stopped = true;
                eprintln!(
                    "111 applier: {}: {} not kept: {}",
                    req.proposal_id,
                    action.summary(),
                    r.reason.as_deref().unwrap_or("?")
                );
            }
            results.push(r);
        }
        let report = ApplyReport {
            applied: results.iter().all(|r| r.applied),
            reverted: results.iter().any(|r| r.reverted),
            reason: results.iter().find_map(|r| r.reason.clone()),
            actions: results,
        };
        let applied = serde_json::to_value(&report).unwrap_or(Value::Null);
        if let Err(e) = self.record.finish(&req.proposal_id, &applied) {
            // applied once and recorded as accepted: the record keeps saying
            // pending, and a retry is still already_decided
            eprintln!(
                "111 applier: {}: applied, but the decision record still reads pending: {e}",
                req.proposal_id
            );
        }
        Ok(report)
    }

    /// Resolve and check every action against a copy of the file as the
    /// earlier actions leave it (nothing written).
    fn plan(&self, actions: &[Action]) -> Result<Vec<Planned>, Refusal> {
        let applies = actions
            .iter()
            .any(|a| !matches!(a, Action::Identify { .. } | Action::ServiceRestart { .. }));
        if applies && self.spine.read_only() {
            return Err(Refusal::Locked);
        }
        let mut sim = self.spine.file();
        let mut out = Vec::new();
        for a in actions {
            out.push(match a {
                Action::InterfaceUpsert {
                    device_id,
                    kind,
                    params,
                } => {
                    if kind != UPSERT_TYPE {
                        return Err(Refusal::Bad(format!(
                            "a device interface is an {UPSERT_TYPE} (not {:?})",
                            shown(kind)
                        )));
                    }
                    let port = self.devices.serial_port(device_id).map_err(|e| match e {
                        TargetError::NotFound => {
                            Refusal::NotFound(format!("no device {device_id}"))
                        }
                        TargetError::NotOffered(why) => {
                            Refusal::Bad(format!("device {device_id}: {why}"))
                        }
                        TargetError::Busy { port, owner } => Refusal::Busy(PortBusy {
                            port: port_key(&port),
                            owner: Owner {
                                task: owner,
                                since_tick: 0,
                            },
                        }),
                    })?;
                    let key = port_key(&port);
                    let name = sim
                        .interfaces
                        .iter()
                        .find(|i| {
                            i.kind == IfKind::RNode
                                && (i.device.as_deref() == Some(device_id.as_str())
                                    || i.port.as_deref().map(port_key).as_deref()
                                        == Some(key.as_str()))
                        })
                        .map(|i| i.name.clone())
                        .unwrap_or_else(|| {
                            format!("RNode {}", &device_id[..8.min(device_id.len())])
                        });
                    let entry = Interface {
                        name: name.clone(),
                        owner: IfOwner::Rnsd,
                        kind: IfKind::RNode,
                        device: Some(device_id.clone()),
                        port: Some(port),
                        region: None,
                        uplink: None,
                        params: toml_params(params)?,
                    };
                    sim.upsert(entry.clone());
                    sim.check_proposal(&name).map_err(invalid)?;
                    Planned::Upsert(entry)
                }
                Action::InterfaceRemove { name } => {
                    let i = sim.interface(name).ok_or_else(|| {
                        Refusal::NotFound(format!("no interface named {:?}", shown(name)))
                    })?;
                    if i.owner != IfOwner::Rnsd {
                        return Err(Refusal::Bad(
                            "the Meshtastic binding is removed by forgetting its board".into(),
                        ));
                    }
                    let stored = i.name.clone();
                    sim.remove(&stored);
                    sim.validate().map_err(invalid)?;
                    Planned::Remove(stored)
                }
                Action::PostureSet {
                    share_instance,
                    enable_transport,
                } => {
                    if let Some(s) = share_instance {
                        sim.posture.share_instance = *s;
                    }
                    if let Some(e) = enable_transport {
                        sim.posture.enable_transport = Some(*e);
                    }
                    sim.validate().map_err(invalid)?;
                    Planned::Posture {
                        share_instance: *share_instance,
                        enable_transport: *enable_transport,
                    }
                }
                Action::RegionSet { region } => {
                    if interfaces::region(region).is_none() {
                        return Err(Refusal::Bad(format!(
                            "region {:?} is not in the region table",
                            shown(region)
                        )));
                    }
                    sim.radio.region = Some(region.clone());
                    sim.validate().map_err(invalid)?;
                    Planned::Region(region.clone())
                }
                Action::Identify { device_id } => Planned::Identify(device_id.clone()),
                Action::ServiceRestart { service_id } => Planned::Restart(service_id.clone()),
            });
        }
        Ok(out)
    }

    fn snapshot(&self) -> Snapshot {
        let layout = self.spine.layout();
        Snapshot {
            file: self.spine.file(),
            toml: read_bytes(&interfaces::path(&layout.state_dir())),
            config: read_bytes(&config::config_path(layout)),
        }
    }

    /// Restore `snap` through the spine (which detaches, attaches back or
    /// restarts again), then put the TOML bytes back byte-exact. Returns a
    /// note when `rns/config` does not match its snapshot afterwards.
    async fn restore(&self, snap: &Snapshot) -> Result<Option<String>, String> {
        let file = snap.file.clone();
        let res = self.spine.update(move |f| *f = file).await;
        if let Err(e) = &res {
            if !saved_anyway(e) {
                // nothing was saved: the file on disk and in memory still
                // agree, so the bytes are not touched either
                return Err(e.to_string());
            }
        }
        let layout = self.spine.layout();
        let toml_path = interfaces::path(&layout.state_dir());
        if let Some(bytes) = &snap.toml {
            if read_bytes(&toml_path).as_deref() != Some(bytes.as_slice()) {
                crate::util::fs::atomic_write(&toml_path, bytes, 0o600)
                    .map_err(|e| format!("interfaces.toml not restored: {e:#}"))?;
            }
        }
        if let Err(e) = res {
            return Err(format!("interfaces.toml restored, but {e}"));
        }
        let now = read_bytes(&config::config_path(layout));
        Ok((now != snap.config)
            .then(|| "rns/config was re-rendered and differs from its snapshot".to_string()))
    }

    /// `Spine::update` refused or failed: when it had already saved the file
    /// (the render or its write failed after the save), the snapshot is
    /// restored; otherwise nothing changed.
    async fn not_applied(&self, snap: Snapshot, e: SpineError) -> ActionResult {
        if saved_anyway(&e) {
            return self
                .keep_or_revert(snap, Err(format!("not_applied: {e}")))
                .await;
        }
        ActionResult {
            action: String::new(),
            applied: false,
            reverted: false,
            skipped: false,
            reason: Some(format!("not_applied: {e}")),
            result: None,
        }
    }

    /// Keep or revert after a probe verdict.
    async fn keep_or_revert(&self, snap: Snapshot, verdict: Result<(), String>) -> ActionResult {
        let mut r = ActionResult {
            action: String::new(),
            applied: true,
            reverted: false,
            skipped: false,
            reason: None,
            result: None,
        };
        let Err(why) = verdict else {
            return r;
        };
        r.applied = false;
        match self.restore(&snap).await {
            Ok(note) => {
                r.reverted = true;
                r.reason = Some(match note {
                    Some(n) => format!("probe_failed: {why} (reverted; {n})"),
                    None => format!("probe_failed: {why} (reverted)"),
                });
            }
            Err(e) => {
                r.reason = Some(format!("probe_failed: {why}; revert_failed: {e}"));
            }
        }
        r
    }

    /// Poll `check` until it passes or the window closes (the last reason).
    async fn probe_until<F>(&self, mut check: F) -> Result<(), String>
    where
        F: FnMut() -> BoxFut<'static, Result<(), String>>,
    {
        let deadline = tokio::time::Instant::now() + self.cfg.probe_window;
        loop {
            let last = match check().await {
                Ok(()) => return Ok(()),
                Err(e) => e,
            };
            if tokio::time::Instant::now() + self.cfg.probe_interval > deadline {
                return Err(last);
            }
            tokio::time::sleep(self.cfg.probe_interval).await;
        }
    }

    fn facts_check(
        &self,
        verdict: impl Fn(&[InterfaceFacts]) -> Result<(), String> + Send + Sync + Clone + 'static,
    ) -> impl FnMut() -> BoxFut<'static, Result<(), String>> {
        let probe = Arc::clone(&self.probe);
        move || {
            let probe = Arc::clone(&probe);
            let verdict = verdict.clone();
            Box::pin(async move {
                let facts = probe
                    .interfaces()
                    .await
                    .map_err(|e| format!("rnstatus: {e}"))?;
                verdict(&facts)
            })
        }
    }

    /// One planned action (`Err`: a one-shot refused before any effect).
    async fn run_one(&self, step: Planned) -> Result<ActionResult, ProblemWith> {
        let failed = |reason: String| ActionResult {
            action: String::new(),
            applied: false,
            reverted: false,
            skipped: false,
            reason: Some(reason),
            result: None,
        };
        match step {
            Planned::Identify(id) => match self.one_shots.identify(&id).await {
                Ok(v) => Ok(ActionResult {
                    action: String::new(),
                    applied: true,
                    reverted: false,
                    skipped: false,
                    reason: None,
                    result: Some(v),
                }),
                Err(OneShotError::Refused(p)) => Err(p),
                Err(OneShotError::Failed(e)) => Ok(failed(format!("identify_failed: {e}"))),
            },
            Planned::Restart(id) => match self.one_shots.restart(&id).await {
                Ok(v) => Ok(ActionResult {
                    action: String::new(),
                    applied: true,
                    reverted: false,
                    skipped: false,
                    reason: None,
                    result: Some(v),
                }),
                Err(OneShotError::Refused(p)) => Err(p),
                Err(OneShotError::Failed(e)) => Ok(failed(format!("restart_failed: {e}"))),
            },
            Planned::Upsert(entry) => {
                let snap = self.snapshot();
                let name = entry.name.clone();
                let applied = match self.spine.update(move |f| f.upsert(entry)).await {
                    Ok(a) => a,
                    Err(e) => return Ok(self.not_applied(snap, e).await),
                };
                let early = applied
                    .left_out
                    .iter()
                    .find(|l| l.name.eq_ignore_ascii_case(&name))
                    .map(|l| format!("not rendered: {}", l.reason))
                    .or_else(|| applied.failures.first().cloned());
                let verdict = match early {
                    Some(e) => Err(e),
                    None => {
                        let n = name.clone();
                        self.probe_until(self.facts_check(move |facts| {
                            match facts.iter().find(|f| f.name.eq_ignore_ascii_case(&n)) {
                                Some(f) if f.up => Ok(()),
                                Some(_) => Err(format!("interface {n:?} is down")),
                                None => Err(format!("rnsd does not list interface {n:?}")),
                            }
                        }))
                        .await
                    }
                };
                Ok(self.keep_or_revert(snap, verdict).await)
            }
            Planned::Remove(name) => {
                let snap = self.snapshot();
                let n = name.clone();
                let applied = match self
                    .spine
                    .update(move |f| {
                        f.remove(&n);
                    })
                    .await
                {
                    Ok(a) => a,
                    Err(e) => return Ok(self.not_applied(snap, e).await),
                };
                let verdict = match applied.failures.first() {
                    Some(e) => Err(e.clone()),
                    None => {
                        let n = name.clone();
                        self.probe_until(self.facts_check(move |facts| {
                            if facts.iter().any(|f| f.name.eq_ignore_ascii_case(&n)) {
                                Err(format!("rnsd still lists interface {n:?}"))
                            } else {
                                Ok(())
                            }
                        }))
                        .await
                    }
                };
                Ok(self.keep_or_revert(snap, verdict).await)
            }
            Planned::Posture {
                share_instance,
                enable_transport,
            } => {
                let snap = self.snapshot();
                let applied = match self
                    .spine
                    .update(move |f| {
                        if let Some(s) = share_instance {
                            f.posture.share_instance = s;
                        }
                        if let Some(e) = enable_transport {
                            f.posture.enable_transport = Some(e);
                        }
                    })
                    .await
                {
                    Ok(a) => a,
                    Err(e) => return Ok(self.not_applied(snap, e).await),
                };
                let sharing = self.spine.file().posture.share_instance;
                let verdict = match applied.failures.first() {
                    Some(e) => Err(e.clone()),
                    None if applied.diff.is_empty() => Ok(()),
                    None => {
                        let probe = Arc::clone(&self.probe);
                        self.probe_until(move || {
                            let probe = Arc::clone(&probe);
                            Box::pin(async move {
                                if !probe.rnsd_running() {
                                    return Err("rnsd is not running".to_string());
                                }
                                if sharing {
                                    let facts = probe
                                        .interfaces()
                                        .await
                                        .map_err(|e| format!("rnstatus: {e}"))?;
                                    if !facts.iter().any(|f| f.kind == "Shared Instance" && f.up) {
                                        return Err("the shared instance is not up".to_string());
                                    }
                                }
                                if !probe.helpers_attached().await {
                                    return Err("a helper did not re-attach".to_string());
                                }
                                Ok(())
                            })
                        })
                        .await
                    }
                };
                Ok(self.keep_or_revert(snap, verdict).await)
            }
            Planned::Region(region) => {
                let snap = self.snapshot();
                let r = region.clone();
                if let Err(e) = self.spine.update(move |f| f.radio.region = Some(r)).await {
                    return Ok(self.not_applied(snap, e).await);
                }
                let probe = Arc::clone(&self.probe);
                let verdict = self
                    .probe_until(move || {
                        let probe = Arc::clone(&probe);
                        let region = region.clone();
                        Box::pin(async move {
                            match probe.file_region().await? {
                                Some(r) if r == region => Ok(()),
                                _ => Err("interfaces.toml does not hold the region".to_string()),
                            }
                        })
                    })
                    .await;
                Ok(self.keep_or_revert(snap, verdict).await)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vocabulary_is_closed() {
        let parse = |s: &str| Action::parse(&serde_json::from_str(s).unwrap());
        assert_eq!(
            parse(r#"{"action":"rns.interface.remove","name":"Roof"}"#).unwrap(),
            Action::InterfaceRemove {
                name: "Roof".into()
            }
        );
        assert_eq!(
            parse(r#"{"action":"rns.posture.set","share_instance":null,"enable_transport":false}"#)
                .unwrap(),
            Action::PostureSet {
                share_instance: None,
                enable_transport: Some(false)
            }
        );
        for refused in REFUSED_ACTIONS {
            let e = parse(&format!(r#"{{"action":"{refused}"}}"#)).unwrap_err();
            assert!(matches!(e, Refusal::Unsupported(_)), "{refused}: {e:?}");
            let p = e.problem();
            assert_eq!(
                (p.problem.status, p.problem.code),
                (409, "unsupported_action")
            );
        }
        assert!(matches!(
            parse(r#"{"action":"shell.exec","cmd":"rm"}"#).unwrap_err(),
            Refusal::Unsupported(_)
        ));
        for bad in [
            r#"{"action":"rns.posture.set"}"#,
            r#"{"action":"rns.posture.set","share_instance":"yes"}"#,
            r#"{"action":"rns.interface.remove","name":"x","extra":1}"#,
            r#"{"action":"identify","device_id":"../../etc"}"#,
            r#"{"action":"service.restart","service_id":"Agent!"}"#,
            r#"{"action":"rns.interface.upsert","device_id":"abcd1234","type":"RNodeInterface"}"#,
            r#"{"action":"radio.region.set","region":""}"#,
            r#""identify""#,
            r#"{"device_id":"abcd1234"}"#,
        ] {
            assert!(matches!(parse(bad), Err(Refusal::Bad(_))), "{bad}");
        }
    }

    #[test]
    fn bodies_are_one_object_with_why_and_actions() {
        let ok = r#"{"why":"identify this radio once (a consented probe)","actions":[{"action":"identify","device_id":"abcd1234abcd1234"}]}"#;
        let r = ApplierRequest::from_body("prop-1", ok).unwrap();
        assert_eq!(r.actions.len(), 1);
        for (body, shape) in [
            ("not json", true),
            ("[]", true),
            (r#"{"actions":[]}"#, true),
            (r#"{"why":"x","actions":{}}"#, true),
            (r#"{"why":"x","actions":[],"more":1}"#, true),
            (r#"{"why":"x","actions":[]}"#, false),
            (
                r#"{"why":"x","actions":[{"action":"ble.pair","mac":"00"}]}"#,
                false,
            ),
        ] {
            let e = ApplierRequest::from_body("prop-1", body).unwrap_err();
            assert_eq!(matches!(e, BodyError::Shape(_)), shape, "{body}: {e:?}");
        }
        let nine = format!(
            r#"{{"why":"x","actions":[{}]}}"#,
            [r#"{"action":"identify","device_id":"abcd1234"}"#; 9].join(",")
        );
        assert!(ApplierRequest::from_body("prop-1", &nine).is_err());
        assert!(ApplierRequest::from_body("../x", ok).is_err());
        assert_eq!(
            BodyError::Shape("x".into()).problem().problem.code,
            "upstream_error"
        );
    }

    #[test]
    fn params_convert_flat_only() {
        let m: Map<String, Value> = serde_json::from_str(
            r#"{"frequency":869525000,"airtime_limit_long":1.5,"flow_control":false,"id_callsign":"N0CALL"}"#,
        )
        .unwrap();
        let t = toml_params(&m).unwrap();
        assert_eq!(t["frequency"], toml::Value::Integer(869_525_000));
        assert_eq!(t["airtime_limit_long"], toml::Value::Float(1.5));
        let nested: Map<String, Value> = serde_json::from_str(r#"{"x":{"y":1}}"#).unwrap();
        assert!(toml_params(&nested).is_err());
        assert_eq!(
            Refusal::RegionUnset("R".into()).problem().body()["reason_code"],
            "region_unset"
        );
    }
}
