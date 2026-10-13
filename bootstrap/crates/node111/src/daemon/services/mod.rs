//! Services registry (Layer 4 spec §7, `os/interface/README.md`): the seven
//! `ServiceRecord`s behind `GET /api/v1/services`, computed by `111d` from
//! deterministic probes. The UI and the agent only read it.
//!
//! Split so the rules are testable without processes or sockets:
//! - [`Facts`] — everything one tick observed (child states, health probes,
//!   radio/WAN/PATH facts). Production fills it in [`probe`]; tests build it.
//! - [`compute`] — the §7 table as a pure function `Facts → [Computed; 7]`.
//! - [`Registry`] — the debounce: a `(state, reason_code, can_run_here)` change
//!   is published only after **two consecutive identical ticks**; `since` is
//!   when the published state last changed. The first tick publishes as-is.
//! - [`probe::spawn`] — the daemon task: a 2 s tick, the WAN probe every 60 s,
//!   the agent status probe every 10 s; publishes `services` on the events bus
//!   on every published change.
//!
//! `starting` grace: a child whose health probe has not passed yet is
//! `starting` for [`AGENT_GRACE`] (90 s) after its current PID first appeared
//! ([`MODEL_GRACE`], 120 s, for `llama-server`, which loads the model first). A
//! child that has not spawned yet at daemon start is `starting` for the same
//! window. After the grace a failing probe is `error`.
//!
//! `radio` is the LoRa radio *stack* row (spec §7, devices brief): live when a
//! stack is bound to a board — rnsd running with an enabled `RNodeInterface`
//! stanza in `<root>/rns/config` whose port is a detected serial device
//! (`radio_bound_rnode`), or meshtasticd answering `127.0.0.1:4403`
//! (`radio_bound_meshtastic`; meshtasticd drives SPI HATs / CH341 adapters and
//! never a USB board). A bound stack that stops answering is `starting`
//! (`stack_waiting`) for [`STACK_DOWN_GRACE`], then `error` (`stack_down`).
//! Unbound: `radio_unbound` (a LoRa-table board is present) or `no_radio`.
//! Board detection reuses Layer 4 R4's enumeration (`devices::detect::scan`,
//! `devices::rns::bindings`) with the installer's LoRa-table rule; it does not
//! read R4's registry snapshot, so the two never wait on each other's ticks.

pub mod log;
pub mod probe;

use crate::daemon::dashboard::{OfferState, TransportState};
use crate::daemon::intelligence::ChildDisplay;
use crate::daemon::kb::KbStatus;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::RwLock;
use std::time::Duration;

/// `starting` window after a child spawn (spec §7: ≤ 90 s).
pub const AGENT_GRACE: Duration = Duration::from_secs(90);
/// `llama-server` health grace (spec §7 table: non-200 after 120 s ⇒ error).
pub const MODEL_GRACE: Duration = Duration::from_secs(120);
/// A bound radio stack not answering this long ⇒ `error` (`stack_down`).
pub const STACK_DOWN_GRACE: Duration = Duration::from_secs(60);
/// meshtasticd's TCP API port (it drives SPI HATs / CH341 adapters only).
pub const MESHTASTIC_PORT: u16 = 4403;
/// `requires` of the radio row once a board is present.
pub const RADIO_STACK_REQUIRES: &str =
    "a mesh stack bound to the board — Reticulum RNode interface or Meshtastic";

/// `ServiceId` (contract enum), in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceId {
    Agent,
    Model,
    Transport,
    Radio,
    Kb,
    Gateway,
    Codeagent,
}

impl ServiceId {
    pub const ALL: [ServiceId; 7] = [
        ServiceId::Agent,
        ServiceId::Model,
        ServiceId::Transport,
        ServiceId::Radio,
        ServiceId::Kb,
        ServiceId::Gateway,
        ServiceId::Codeagent,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ServiceId::Agent => "agent",
            ServiceId::Model => "model",
            ServiceId::Transport => "transport",
            ServiceId::Radio => "radio",
            ServiceId::Kb => "kb",
            ServiceId::Gateway => "gateway",
            ServiceId::Codeagent => "codeagent",
        }
    }

    pub fn parse(s: &str) -> Option<ServiceId> {
        ServiceId::ALL.into_iter().find(|id| id.as_str() == s)
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// `ServiceState` (contract enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceState {
    Live,
    Unset,
    Error,
    Starting,
}

/// `ServiceRecord.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Child,
    Detected,
    Builtin,
}

/// `ServiceRecord.actions` items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// In the contract enum but never emitted until a per-child restart route
    /// exists (Layer 9).
    Restart,
    Log,
}

/// One `ServiceRecord` (contract schema; optional fields omitted when unset).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServiceRecord {
    pub id: ServiceId,
    pub name: &'static str,
    pub plain_name: &'static str,
    pub kind: Kind,
    pub state: ServiceState,
    /// RFC 3339 UTC: when the published state last changed.
    pub since: String,
    pub reason_code: &'static str,
    pub reason: String,
    pub can_run_here: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires: Option<&'static str>,
    pub description: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kb_entry: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_url: Option<String>,
    pub actions: Vec<Action>,
    pub detail: Value,
}

/// The static half of a record (spec §7 table + plain copy).
pub struct Catalogue {
    pub name: &'static str,
    pub plain_name: &'static str,
    pub kind: Kind,
    pub requires: Option<&'static str>,
    pub description: &'static str,
    pub kb_entry: Option<&'static str>,
}

pub fn catalogue(id: ServiceId) -> Catalogue {
    match id {
        ServiceId::Agent => Catalogue {
            name: "111-agent",
            plain_name: "Local AI agent",
            kind: Kind::Child,
            requires: Some("The standard tier: the 111-agent program in this release."),
            description: "The node's own AI assistant. It answers questions from the built-in \
                knowledge base and can write proposals for you to accept. It runs on this node; \
                your chats are not sent to the internet.",
            kb_entry: Some("models/qwen3-4b.md"),
        },
        ServiceId::Model => Catalogue {
            name: "llama-server",
            plain_name: "Local model",
            kind: Kind::Child,
            requires: Some(
                "The standard tier: the llama-server program and a model file under models/current.",
            ),
            description: "The language model that powers the agent, served by llama-server on \
                this node. It needs the model file and enough memory to run it.",
            kb_entry: Some("projects/llama-cpp.md"),
        },
        ServiceId::Transport => Catalogue {
            name: "rnsd",
            plain_name: "Mesh transport",
            kind: Kind::Child,
            requires: Some("python3 on this machine (the Reticulum environment is built from it)."),
            description: "Reticulum, the mesh network layer that lets this node reach other 111 \
                nodes over any link: Wi-Fi, Ethernet or radio. It runs as the rnsd process.",
            kb_entry: Some("projects/reticulum.md"),
        },
        ServiceId::Radio => Catalogue {
            name: "RNode / meshtasticd",
            plain_name: "LoRa radio",
            kind: Kind::Detected,
            requires: Some(RADIO_STACK_REQUIRES),
            description: "Long-range radio for the mesh. A LoRa board must be bound to a stack \
                before it carries traffic.",
            kb_entry: Some("projects/reticulum.md"),
        },
        ServiceId::Kb => Catalogue {
            name: "share/kb",
            plain_name: "Knowledge base",
            kind: Kind::Builtin,
            requires: Some("A release that ships the knowledge base (share/kb)."),
            description: "An offline library of guides and reference pages that ships with every \
                release. The agent reads it to answer questions, and you can browse it here.",
            kb_entry: None,
        },
        ServiceId::Gateway => Catalogue {
            name: "wan",
            plain_name: "Internet gateway",
            kind: Kind::Detected,
            requires: Some("An internet connection: Wi-Fi, Ethernet or a phone hotspot."),
            description: "Whether this node can reach the internet right now. Nothing needs \
                setting up here; the node checks once a minute.",
            kb_entry: None,
        },
        ServiceId::Codeagent => Catalogue {
            name: "opencode",
            plain_name: "Code agent",
            kind: Kind::Detected,
            requires: Some("A hub-class node (category Hub4A or Hub4B) with opencode on the PATH."),
            description: "opencode, a coding agent that can work on software on this node. It \
                runs on hub-class machines that have the opencode tool installed.",
            kb_entry: Some("projects/opencode.md"),
        },
    }
}

// ---------------------------------------------------------------- facts

/// Result of one HTTP health probe.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Probe {
    /// Not probed this tick (child not running).
    #[default]
    NotRun,
    /// HTTP status received.
    Status(u16),
    /// Connect/read failure or timeout.
    Unreachable(String),
}

impl Probe {
    fn detail(&self) -> Value {
        match self {
            Probe::NotRun => Value::Null,
            Probe::Status(c) => json!({ "status": c }),
            Probe::Unreachable(e) => json!({ "error": e }),
        }
    }
}

/// One supervised child as observed this tick.
#[derive(Debug, Clone, Default)]
pub struct ChildFacts {
    pub display: ChildDisplay,
    pub pid: Option<u32>,
    /// Time since the current (or last) PID first appeared; `None` = never
    /// spawned this daemon lifetime.
    pub since_spawn: Option<Duration>,
    pub health: Probe,
    /// The child's log file, when it exists.
    pub log: Option<String>,
}

/// One detected LoRa board (USB facts matched against `lora-table.toml`; a
/// VID:PID names the serial chip, so this is a *likely* board).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Radio {
    pub vid: String,
    pub pid: String,
    pub product: String,
    /// Table row name (e.g. "Heltec WiFi LoRa 32 V3 (CP2102)").
    pub name: String,
    /// Kernel tty (`ttyACM0`); `None` for a board without a serial port
    /// (bootloader mode).
    pub tty: Option<String>,
    /// `/dev/serial/by-id` name, when udev made one.
    pub by_id: Option<String>,
}

/// An enabled `RNodeInterface` stanza whose port is a present serial device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RnodeBound {
    /// Stanza name.
    pub interface: String,
    /// `port` as configured.
    pub port: String,
    pub tty: String,
}

#[derive(Debug, Clone, Default)]
pub struct RadioFacts {
    /// LoRa-table boards present now; `None` = USB sysfs not readable here
    /// (phones, containers).
    pub boards: Option<Vec<Radio>>,
    /// RNode bindings to present serial devices (configuration).
    pub rnode: Vec<RnodeBound>,
    /// The rnsd child is running (the RNode binding's stack).
    pub rnsd_running: bool,
    /// TCP connect to 127.0.0.1:4403 succeeded this tick.
    pub meshtastic_open: bool,
    /// meshtasticd answered at some point this daemon lifetime (its binding
    /// is only knowable by it answering).
    pub meshtastic_seen: bool,
    /// How long a bound stack has not been answering (`None` while live or
    /// when nothing is bound).
    pub down_for: Option<Duration>,
}

#[derive(Debug, Clone, Default)]
pub struct GatewayFacts {
    /// `None` until the first WAN probe completes.
    pub online: Option<bool>,
    /// The URL that answered 204 (or the last one tried).
    pub url: Option<String>,
    pub checked_at: Option<String>,
    pub error: Option<String>,
}

/// The agent's own view (`GET :11182/v1/agent/status`, cached 10 s).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentStatusProbe {
    /// `local | offloaded | unavailable`.
    pub state: Option<String>,
    pub model_id: Option<String>,
}

/// Everything one registry tick observed.
#[derive(Debug, Clone)]
pub struct Facts {
    pub category: Option<String>,
    /// Phone-class node (Node2A / Android): no USB host for a LoRa board.
    pub phone_like: bool,
    /// Time since the registry started (grace for children not yet spawned).
    pub boot_elapsed: Duration,
    pub agent: ChildFacts,
    pub agent_binary: bool,
    pub agent_status: Option<AgentStatusProbe>,
    pub model: ChildFacts,
    pub model_binary: bool,
    /// `models/current/*.gguf` file name.
    pub model_file: Option<String>,
    pub offer: Option<OfferState>,
    pub transport_state: TransportState,
    pub transport_child: ChildDisplay,
    pub transport_log: Option<String>,
    /// `python3` found on PATH (splits "no python" from "venv unavailable"
    /// when the transport is the detection-only stub).
    pub python3: bool,
    pub radio: RadioFacts,
    pub kb: Option<KbStatus>,
    /// `current/share/kb/` exists (distinguishes "absent" from "failed verify").
    pub kb_payload_present: bool,
    pub gateway: GatewayFacts,
    /// opencode path when found on PATH.
    pub opencode: Option<String>,
}

impl Default for Facts {
    fn default() -> Self {
        Facts {
            category: None,
            phone_like: false,
            boot_elapsed: Duration::ZERO,
            agent: ChildFacts::default(),
            agent_binary: false,
            agent_status: None,
            model: ChildFacts::default(),
            model_binary: false,
            model_file: None,
            offer: None,
            transport_state: TransportState::NoTransport,
            transport_child: ChildDisplay::Absent,
            transport_log: None,
            python3: false,
            radio: RadioFacts::default(),
            kb: None,
            kb_payload_present: false,
            gateway: GatewayFacts::default(),
            opencode: None,
        }
    }
}

// ---------------------------------------------------------------- rules

/// The dynamic half of a record (what [`compute`] decides each tick).
#[derive(Debug, Clone, PartialEq)]
pub struct Computed {
    pub id: ServiceId,
    pub state: ServiceState,
    pub reason_code: &'static str,
    pub reason: String,
    pub can_run_here: bool,
    pub actions: Vec<Action>,
    pub embed_url: Option<String>,
    /// Overrides the catalogue's `requires` / `kb_entry` for this state.
    pub requires: Option<&'static str>,
    pub kb_entry: Option<&'static str>,
    pub detail: Value,
}

impl Computed {
    fn new(
        id: ServiceId,
        state: ServiceState,
        code: &'static str,
        reason: impl Into<String>,
    ) -> Self {
        Computed {
            id,
            state,
            reason_code: code,
            reason: reason.into(),
            can_run_here: true,
            actions: Vec::new(),
            embed_url: None,
            requires: None,
            kb_entry: None,
            detail: json!({}),
        }
    }
}

fn display_str(d: ChildDisplay) -> &'static str {
    match d {
        ChildDisplay::Absent => "absent",
        ChildDisplay::Running => "running",
        ChildDisplay::Backoff => "backoff",
        ChildDisplay::Degraded => "degraded",
        ChildDisplay::Stopped => "stopped",
    }
}

/// Shared child rule (agent + model): display state × health × grace.
fn child_rule(
    id: ServiceId,
    what: &str,
    c: &ChildFacts,
    grace: Duration,
    boot_elapsed: Duration,
) -> (ServiceState, &'static str, String) {
    let in_grace = match c.since_spawn {
        Some(d) => d < grace,
        None => boot_elapsed < grace,
    };
    match c.display {
        ChildDisplay::Absent => unreachable!("absent handled by the caller ({id:?})"),
        ChildDisplay::Degraded => (
            ServiceState::Error,
            "child_degraded",
            format!("The {what} kept crashing, so the node stopped restarting it."),
        ),
        ChildDisplay::Stopped => (
            ServiceState::Error,
            "child_stopped",
            format!("The {what} was stopped."),
        ),
        ChildDisplay::Backoff if c.since_spawn.is_none() && in_grace => (
            ServiceState::Starting,
            "spawn_pending",
            format!("The {what} is starting."),
        ),
        ChildDisplay::Backoff => (
            ServiceState::Error,
            "child_backoff",
            format!("The {what} crashed and is being restarted."),
        ),
        ChildDisplay::Running => match &c.health {
            Probe::Status(200) => (
                ServiceState::Live,
                "healthy",
                format!("The {what} is running and healthy."),
            ),
            _ if in_grace => (
                ServiceState::Starting,
                "health_pending",
                format!("The {what} is starting up."),
            ),
            Probe::Status(503) => (
                ServiceState::Error,
                "health_503",
                format!("The {what} is running but reports it is not ready."),
            ),
            Probe::Status(code) => (
                ServiceState::Error,
                "health_error",
                format!("The {what} is running but its health check answered {code}."),
            ),
            Probe::Unreachable(_) | Probe::NotRun => (
                ServiceState::Error,
                "health_unreachable",
                format!("The {what} is running but does not answer its health check."),
            ),
        },
    }
}

/// `log` when the child's log file exists (no `restart` until a per-child
/// restart route exists, Layer 9).
fn child_actions(c: &ChildFacts) -> Vec<Action> {
    if c.log.is_some() {
        vec![Action::Log]
    } else {
        Vec::new()
    }
}

fn agent(f: &Facts) -> Computed {
    let id = ServiceId::Agent;
    let c = &f.agent;
    let mut out = if c.display == ChildDisplay::Absent {
        Computed::new(
            id,
            ServiceState::Unset,
            "binary_absent",
            "The agent is not installed on this node; it comes with the standard tier.",
        )
    } else {
        let (s, code, reason) = child_rule(id, "agent", c, AGENT_GRACE, f.boot_elapsed);
        Computed::new(id, s, code, reason)
    };
    out.actions = child_actions(c);
    out.detail = json!({
        "child": display_str(c.display),
        "pid": c.pid,
        "port": crate::daemon::intelligence::AGENT_PORT,
        "health": c.health.detail(),
        "binary_present": f.agent_binary,
        "agent_state": f.agent_status.as_ref().and_then(|s| s.state.clone()),
        "model_id": f.agent_status.as_ref().and_then(|s| s.model_id.clone()),
        "log": c.log,
    });
    out
}

fn model(f: &Facts) -> Computed {
    let id = ServiceId::Model;
    let c = &f.model;
    let mut out = if c.display == ChildDisplay::Absent {
        if !f.model_binary {
            Computed::new(
                id,
                ServiceState::Unset,
                "binary_absent",
                "The model server is not installed on this node; it comes with the standard tier.",
            )
        } else {
            Computed::new(
                id,
                ServiceState::Unset,
                "no_model",
                "No model file is installed yet (models/current has no .gguf file).",
            )
        }
    } else {
        let (s, code, reason) = child_rule(id, "model server", c, MODEL_GRACE, f.boot_elapsed);
        let reason = if code == "health_pending" {
            "The model server is loading the model.".to_string()
        } else {
            reason
        };
        Computed::new(id, s, code, reason)
    };
    out.actions = child_actions(c);
    out.detail = json!({
        "child": display_str(c.display),
        "pid": c.pid,
        "port": crate::daemon::intelligence::MODEL_PORT,
        "health": c.health.detail(),
        "binary_present": f.model_binary,
        "model_file": f.model_file,
        "model_id": f.agent_status.as_ref().and_then(|s| s.model_id.clone()),
        "offer": f.offer,
        "log": c.log,
    });
    out
}

fn transport(f: &Facts) -> Computed {
    let id = ServiceId::Transport;
    let mut out = match f.transport_state {
        TransportState::NoTransport if !f.python3 => Computed::new(
            id,
            ServiceState::Unset,
            "no_python",
            "Mesh transport is off: python3 is not installed on this machine.",
        ),
        TransportState::NoTransport => Computed::new(
            id,
            ServiceState::Unset,
            "venv_unavailable",
            "Mesh transport is off: python3 is installed, but the Reticulum environment could not be set up.",
        ),
        TransportState::Degraded => Computed::new(
            id,
            ServiceState::Error,
            "transport_degraded",
            "The Reticulum process (rnsd) keeps failing.",
        ),
        TransportState::Isolated | TransportState::Joined
            if f.transport_child == ChildDisplay::Backoff =>
        {
            if f.boot_elapsed < AGENT_GRACE {
                Computed::new(
                    id,
                    ServiceState::Starting,
                    "spawn_pending",
                    "The Reticulum process (rnsd) is starting.",
                )
            } else {
                Computed::new(
                    id,
                    ServiceState::Error,
                    "child_backoff",
                    "The Reticulum process (rnsd) crashed and is being restarted.",
                )
            }
        }
        TransportState::Joined => Computed::new(
            id,
            ServiceState::Live,
            "joined",
            "rnsd is running and has heard other nodes recently.",
        ),
        TransportState::Isolated => Computed::new(
            id,
            ServiceState::Live,
            "isolated",
            "rnsd is running; no other nodes heard yet.",
        ),
    };
    let child = f.transport_child;
    if f.transport_log.is_some() {
        out.actions.push(Action::Log);
    }
    let peers = match f.transport_state {
        TransportState::Joined => Some("joined"),
        TransportState::Isolated => Some("isolated"),
        _ => None,
    };
    out.detail = json!({
        "child": display_str(child),
        "python3": f.python3,
        "transport_state": f.transport_state,
        "peers": peers,
        "log": f.transport_log,
    });
    out
}

fn radio(f: &Facts) -> Computed {
    let id = ServiceId::Radio;
    let r = &f.radio;
    let boards: &[Radio] = r.boards.as_deref().unwrap_or(&[]);
    let rnode_live = r.rnsd_running && !r.rnode.is_empty();
    let bound = !r.rnode.is_empty() || r.meshtastic_seen;
    let mut out = if rnode_live {
        Computed::new(
            id,
            ServiceState::Live,
            "radio_bound_rnode",
            "The LoRa board is bound to the mesh transport (Reticulum RNode interface).",
        )
    } else if r.meshtastic_open {
        let mut c = Computed::new(
            id,
            ServiceState::Live,
            "radio_bound_meshtastic",
            "meshtasticd drives the LoRa radio and answers on port 4403.",
        );
        c.kb_entry = Some("projects/meshtastic.md");
        c
    } else if bound {
        let what = if r.rnode.is_empty() {
            "meshtasticd"
        } else {
            "The Reticulum process (rnsd)"
        };
        if r.down_for.unwrap_or_default() >= STACK_DOWN_GRACE {
            Computed::new(
                id,
                ServiceState::Error,
                "stack_down",
                format!("{what}, bound to the LoRa radio, has not answered for over a minute."),
            )
        } else {
            Computed::new(
                id,
                ServiceState::Starting,
                "stack_waiting",
                format!("{what}, bound to the LoRa radio, is not answering yet."),
            )
        }
    } else if !boards.is_empty() {
        Computed::new(
            id,
            ServiceState::Unset,
            "radio_unbound",
            "A LoRa board is plugged in, but no mesh stack is bound to it yet.",
        )
    } else {
        let mut c = Computed::new(
            id,
            ServiceState::Unset,
            "no_radio",
            "No LoRa board is plugged in.",
        );
        c.requires = Some("a LoRa board plugged into this node");
        if f.phone_like {
            c.can_run_here = false;
            c.reason = "This device cannot drive a LoRa board.".into();
        }
        c
    };
    out.detail = json!({
        "boards": boards,
        "usb_readable": r.boards.is_some(),
        "rnode_bindings": r.rnode,
        "rnsd_running": r.rnsd_running,
        "meshtastic": {
            "port": MESHTASTIC_PORT,
            "open": r.meshtastic_open,
            "seen": r.meshtastic_seen,
        },
        "down_for_sec": r.down_for.map(|d| d.as_secs()),
    });
    out
}

fn kb(f: &Facts) -> Computed {
    let id = ServiceId::Kb;
    let mut out = match &f.kb {
        Some(k) => Computed::new(
            id,
            ServiceState::Live,
            "verified",
            format!(
                "The knowledge base is installed and verified ({} entries).",
                k.entries
            ),
        ),
        None if f.kb_payload_present => Computed::new(
            id,
            ServiceState::Error,
            "verify_failed",
            "The knowledge base files do not match this release's manifest.",
        ),
        None => Computed::new(
            id,
            ServiceState::Unset,
            "kb_absent",
            "This release does not include the knowledge base.",
        ),
    };
    out.detail = json!({
        "sha256": f.kb.as_ref().map(|k| k.sha256.clone()),
        "entries": f.kb.as_ref().map(|k| k.entries),
        "payload_present": f.kb_payload_present,
        "path": "current/share/kb",
    });
    out
}

fn gateway(f: &Facts) -> Computed {
    let id = ServiceId::Gateway;
    let g = &f.gateway;
    let mut out = match g.online {
        Some(true) => Computed::new(
            id,
            ServiceState::Live,
            "wan_ok",
            "This node can reach the internet.",
        ),
        Some(false) => Computed::new(
            id,
            ServiceState::Unset,
            "wan_offline",
            "This node cannot reach the internet right now.",
        ),
        None => Computed::new(
            id,
            ServiceState::Unset,
            "wan_unknown",
            "The internet check has not finished yet.",
        ),
    };
    out.detail = json!({
        "online": g.online,
        "probe_url": g.url,
        "checked_at": g.checked_at,
        "error": g.error,
        "interval_sec": 60,
    });
    out
}

fn codeagent(f: &Facts) -> Computed {
    let id = ServiceId::Codeagent;
    let hub = matches!(f.category.as_deref(), Some("Hub4A" | "Hub4B"));
    let mut out = match (&f.opencode, hub) {
        (_, false) => {
            let mut c = Computed::new(
                id,
                ServiceState::Unset,
                "not_hub",
                "The code agent runs on hub-class nodes only.",
            );
            c.can_run_here = false;
            c
        }
        (Some(path), true) => Computed::new(
            id,
            ServiceState::Live,
            "opencode_found",
            format!("opencode is installed at {path}."),
        ),
        (None, true) => Computed::new(
            id,
            ServiceState::Unset,
            "opencode_absent",
            "opencode is not installed (it was not found on the PATH).",
        ),
    };
    out.detail = json!({
        "category": f.category,
        "hub": hub,
        "opencode": f.opencode,
    });
    out
}

/// The §7 table: one [`Computed`] per service, in [`ServiceId::ALL`] order.
pub fn compute(f: &Facts) -> [Computed; 7] {
    [
        agent(f),
        model(f),
        transport(f),
        radio(f),
        kb(f),
        gateway(f),
        codeagent(f),
    ]
}

// ---------------------------------------------------------------- registry

type Key = (ServiceState, &'static str, bool);

fn key(c: &Computed) -> Key {
    (c.state, c.reason_code, c.can_run_here)
}

struct Slot {
    published: Computed,
    since: String,
    /// A different key seen on the previous tick (published on its 2nd tick).
    pending: Option<Key>,
}

/// The debounced registry (see module docs). Cheap to read from handlers.
#[derive(Default)]
pub struct Registry {
    slots: RwLock<Option<Vec<Slot>>>,
    agent_status: RwLock<Option<AgentStatusProbe>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one tick. Returns `true` when the published set changed (first
    /// tick, or a state flip confirmed by two identical ticks) — the caller
    /// publishes a `services` frame then. Detail refreshes every tick.
    pub fn tick(&self, computed: [Computed; 7], now: &str) -> bool {
        let mut slots = self.slots.write().unwrap();
        let Some(slots) = slots.as_mut() else {
            *slots = Some(
                computed
                    .into_iter()
                    .map(|c| Slot {
                        published: c,
                        since: now.to_string(),
                        pending: None,
                    })
                    .collect(),
            );
            return true;
        };
        let mut changed = false;
        for (slot, c) in slots.iter_mut().zip(computed) {
            let k = key(&c);
            if k == key(&slot.published) {
                slot.pending = None;
                slot.published = c;
            } else if slot.pending == Some(k) {
                slot.published = c;
                slot.since = now.to_string();
                slot.pending = None;
                changed = true;
            } else {
                slot.pending = Some(k);
                slot.published.detail = c.detail;
            }
        }
        changed
    }

    /// The published records (empty before the first tick).
    pub fn snapshot(&self) -> Vec<ServiceRecord> {
        let slots = self.slots.read().unwrap();
        slots
            .as_ref()
            .map(|s| s.iter().map(record).collect())
            .unwrap_or_default()
    }

    pub fn get(&self, id: ServiceId) -> Option<ServiceRecord> {
        let slots = self.slots.read().unwrap();
        slots.as_ref().map(|s| record(&s[id.index()]))
    }

    /// The `services` SSE payload (`SseServices`).
    pub fn frame(&self) -> Value {
        json!({ "services": self.snapshot() })
    }

    /// Cached `GET :11182/v1/agent/status` facts (`/status.intelligence`).
    pub fn agent_status(&self) -> Option<AgentStatusProbe> {
        self.agent_status.read().unwrap().clone()
    }

    pub fn set_agent_status(&self, s: Option<AgentStatusProbe>) {
        *self.agent_status.write().unwrap() = s;
    }
}

fn record(slot: &Slot) -> ServiceRecord {
    let c = &slot.published;
    let cat = catalogue(c.id);
    ServiceRecord {
        id: c.id,
        name: cat.name,
        plain_name: cat.plain_name,
        kind: cat.kind,
        state: c.state,
        since: slot.since.clone(),
        reason_code: c.reason_code,
        reason: c.reason.clone(),
        can_run_here: c.can_run_here,
        requires: c.requires.or(cat.requires),
        description: cat.description,
        kb_entry: c.kb_entry.or(cat.kb_entry),
        embed_url: c.embed_url.clone(),
        actions: c.actions.clone(),
        detail: c.detail.clone(),
    }
}

/// One registry tick plus its `services` frame when the published set changed
/// (the daemon task's body; tests drive it with injected [`Facts`]).
pub fn tick_and_publish(
    api: &crate::daemon::dashboard::ApiState,
    registry: &Registry,
    facts: &Facts,
    now: &str,
) -> bool {
    let changed = registry.tick(compute(facts), now);
    if changed {
        api.events.publish("services", &registry.frame());
    }
    changed
}

/// RFC 3339 UTC now (registry `since` stamps).
pub fn rfc3339_now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_in_contract_order() {
        let names: Vec<_> = ServiceId::ALL.iter().map(|i| i.as_str()).collect();
        assert_eq!(
            names,
            [
                "agent",
                "model",
                "transport",
                "radio",
                "kb",
                "gateway",
                "codeagent"
            ]
        );
        for id in ServiceId::ALL {
            assert_eq!(ServiceId::parse(id.as_str()), Some(id));
            assert_eq!(
                id.index(),
                ServiceId::ALL.iter().position(|x| *x == id).unwrap()
            );
        }
        assert_eq!(ServiceId::parse("Agent"), None);
    }

    #[test]
    fn compute_order_matches_ids() {
        let out = compute(&Facts::default());
        for (c, id) in out.iter().zip(ServiceId::ALL) {
            assert_eq!(c.id, id);
        }
    }
}
