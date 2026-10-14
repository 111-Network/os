//! Services registry (Layer 4 spec §7, `os/interface/README.md`; grown to nine
//! ids by Layer 5 §15/§19): the `ServiceRecord`s behind
//! `GET /api/v1/services`, computed by `111d` from deterministic probes. The UI
//! and the agent only read it.
//!
//! Split so the rules are testable without processes or sockets:
//! - [`Facts`] — everything one tick observed (child states, health probes,
//!   radio/WAN/PATH facts, and the Layer 5 `fabric` / `meshmud` / panel facts
//!   from optional providers). Production fills it in [`probe`]; tests build
//!   it.
//! - [`compute`] — the §7 table as a pure function
//!   `Facts → [Computed; ServiceId::COUNT]`.
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
//! (`radio_bound_rnode`), or meshtasticd listening on local port 4403
//! (`radio_bound_meshtastic`; meshtasticd drives SPI HATs / CH341 adapters and
//! never a USB board). A bound stack that stops answering is `starting`
//! (`stack_waiting`) for [`STACK_DOWN_GRACE`], then `error` (`stack_down`).
//! Unbound: `radio_unbound` (a LoRa-table board is present) or `no_radio`.
//! Board detection reuses Layer 4 R4's enumeration (`devices::detect::scan`,
//! `devices::rns::bindings`) with the installer's LoRa-table rule; it does not
//! read R4's registry snapshot, so the two never wait on each other's ticks.
//! Layer 5 §14: a board **meshmud** is bound to ([`RadioFacts::meshmud_bound`])
//! counts as bound too — live (`radio_bound_meshtastic`, "driven by meshmud")
//! while meshmud runs — so `radio` and `meshtastic` never contradict.
//!
//! Layer 5 rows: `fabric` ("Messaging", §15) over [`FabricFacts`] and
//! `meshtastic` (§14) over [`MeshmudFacts`]. Without a provider the facts are
//! `None` and both rows read NOT SET honestly. A service with a hosted panel
//! (§13, [`PanelInfo`] by service id) carries it in `panel`; `embed_url` is
//! then never set.

pub mod log;
pub mod probe;

use crate::daemon::dashboard::{OfferState, TransportState};
use crate::daemon::intelligence::ChildDisplay;
use crate::daemon::kb::KbStatus;
use crate::daemon::supervise::ChildControl;
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
/// meshmud's TCP downlink (Layer 5 §2/§14: loopback only; LISTEN-checked,
/// never connected to by a probe).
pub const MESHMUD_DOWNLINK_PORT: u16 = 11230;
/// meshmud without its uplink this long ⇒ `meshtastic` is `error` (§14:
/// "uplink lost > 60 s").
pub const UPLINK_LOST_GRACE: Duration = Duration::from_secs(60);

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
    /// Layer 5 §15: the messaging fabric ("Messaging").
    Fabric,
    /// Layer 5 §14: meshmud + the hosted Meshtastic web client.
    Meshtastic,
}

impl ServiceId {
    /// Number of services (the fixed size of every per-service array).
    pub const COUNT: usize = 9;

    pub const ALL: [ServiceId; ServiceId::COUNT] = [
        ServiceId::Agent,
        ServiceId::Model,
        ServiceId::Transport,
        ServiceId::Radio,
        ServiceId::Kb,
        ServiceId::Gateway,
        ServiceId::Codeagent,
        ServiceId::Fabric,
        ServiceId::Meshtastic,
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
            ServiceId::Fabric => "fabric",
            ServiceId::Meshtastic => "meshtastic",
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
    /// `POST /api/v1/services/{id}/restart` — offered exactly for the children
    /// this daemon supervises right now (agent, model, transport while rnsd is
    /// a child); see [`Supervised`].
    Restart,
    Log,
}

/// The children 111d supervises, by service. The daemon start hook inserts an
/// `Arc<Supervised>` into `ApiState` ext; `POST /services/{id}/restart` reads
/// it (no entry ⇒ `409 not_supervised`). A service may own several children,
/// restarted in registration order (Layer 5 §14: `meshtastic` = meshmud, then
/// the presence reader).
#[derive(Clone, Default)]
pub struct Supervised {
    children: Vec<(ServiceId, Vec<ChildControl>)>,
}

impl Supervised {
    /// Add `id`'s child when there is one (`None`: absent, stub).
    pub fn with(self, id: ServiceId, child: Option<ChildControl>) -> Self {
        self.with_children(id, child.into_iter().collect())
    }

    /// Set `id`'s children, first = primary (the one whose state decides
    /// whether the service is restartable); an empty list registers nothing.
    pub fn with_children(mut self, id: ServiceId, children: Vec<ChildControl>) -> Self {
        if !children.is_empty() {
            self.children.retain(|(i, _)| *i != id);
            self.children.push((id, children));
        }
        self
    }

    /// `id`'s primary child.
    pub fn get(&self, id: ServiceId) -> Option<&ChildControl> {
        self.children(id).first()
    }

    /// Every child of `id`, in restart order (empty when unsupervised).
    pub fn children(&self, id: ServiceId) -> &[ChildControl] {
        self.children
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, c)| c.as_slice())
            .unwrap_or(&[])
    }
}

/// `PanelState.health` (contract enum): the upstream's child state, never a
/// connect (§13 health).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelHealth {
    Up,
    Down,
    Unknown,
}

/// `PanelState` (contract schema, Layer 5 §13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PanelState {
    /// Only enabled panels hold a listener.
    pub enabled: bool,
    pub listening: bool,
    pub health: PanelHealth,
    /// Shim panels: `FromRadio` frames dropped by the bounded drop-oldest
    /// buffer (§14 "drop count surfaced in `panel.state`").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped: Option<u64>,
}

/// A hosted panel attached to a service record (`Panel`, Layer 5 §13:
/// `ServiceRecord.panel = {id, port, entry, needs_secure_context, allow[],
/// optional_hosts[], state} | null`). Filled by the panels module through the
/// registry's panel provider; attached to the service whose id it carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PanelInfo {
    /// Panel id = its service id (manifest `id`).
    pub id: ServiceId,
    /// The panel listener's port (from the 11190–11229 block); the UI builds
    /// the origin from `location.hostname` + this, never from a snapshot.
    pub port: u16,
    /// Manifest `[entry] path`.
    pub entry: String,
    pub needs_secure_context: bool,
    /// Manifest `[frame] allow` (iframe `allow` features).
    pub allow: Vec<String>,
    /// Manifest `[offline] optional_hosts` (shown to the user).
    pub optional_hosts: Vec<String>,
    pub state: PanelState,
    /// Manifest `title` (frame chrome).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Manifest `[frame] secure_note`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secure_note: Option<String>,
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
    /// Layer 5 §13: the hosted panel, `null` when the service has none.
    pub panel: Option<PanelInfo>,
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
            requires: Some("A hub-class node (category Hub4A or Hub4B) with opencode installed."),
            description: "opencode, a coding agent that can work on software on this node. It \
                runs on hub-class machines that have the opencode tool installed.",
            kb_entry: Some("projects/opencode.md"),
        },
        ServiceId::Fabric => Catalogue {
            name: "mesh111",
            plain_name: "Messaging",
            kind: Kind::Builtin,
            requires: Some(
                "A home hub to enroll with, or another 111 node reachable on this network.",
            ),
            description: "Sealed messages between your nodes and your contacts. A message goes \
                straight to the other node when it can be reached, or waits with your home hub \
                until it comes back. It runs inside the node; only the two ends can read a \
                message.",
            kb_entry: None,
        },
        ServiceId::Meshtastic => Catalogue {
            name: "meshmud",
            plain_name: "Meshtastic",
            kind: Kind::Child,
            requires: Some(
                "A Meshtastic board identified and bound to this node: plugged in by USB (Linux), \
                 or a Wi-Fi board reached by its address.",
            ),
            description: "Meshtastic's own web client for the Meshtastic board bound to this \
                node, and the list of Meshtastic nodes it hears. What you do in that client goes \
                to your radio and out over Meshtastic; 111 does not see or store it, and 111 \
                itself never transmits on Meshtastic.",
            kb_entry: Some("projects/meshtastic-web.md"),
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
    /// Something LISTENs on local port 4403 this tick (`/proc/net/tcp{,6}`;
    /// never a connect — meshtasticd serves one client at a time).
    pub meshtastic_open: bool,
    /// meshtasticd answered at some point this daemon lifetime (its binding
    /// is only knowable by it answering).
    pub meshtastic_seen: bool,
    /// Ports meshmud is bound to (Layer 5 §14, from the `devices.json`
    /// binding): such a board counts as bound.
    pub meshmud_bound: Vec<String>,
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

/// The messaging fabric as `111d` sees it this tick (Layer 5 §15; filled by
/// the fabric module's provider).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FabricFacts {
    /// The store opened (§10.1). `false` with facts present = it failed.
    pub store_open: bool,
    /// The fabric is down because no identity loaded (the node's identity
    /// check failed: `111 identity restore`), not because of the store.
    pub no_identity: bool,
    /// The identity holder reports a restore floor (§4, L2 §8.4): nothing
    /// is sealed and no link runs until it is set.
    pub restore_pending: bool,
    /// Capacity pruning cannot make room (§10.4 `store_full`).
    pub store_full: bool,
    /// Home hub (§7.3): `None` = not enrolled with one; `Some(reachable)`.
    pub hub_reachable: Option<bool>,
    /// Hub role (§9): `None` = this node is not a hub; `Some(n)` = a hub with
    /// `n` enrolled members.
    pub members: Option<u32>,
    /// Peer links attributed (a `pair_mac`-verified packet seen, §8.1) now.
    pub attributed_links: u32,
}

/// meshmud and the presence reader as `111d` sees them this tick (Layer 5
/// §14; filled by the Meshtastic module's provider).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshmudFacts {
    /// meshmud's process is up now (a live PID).
    pub running: bool,
    /// meshmud's supervisor state; `Absent` = no child (no binding, or a
    /// binding meshmud cannot run for — see `bound`, `installed`).
    pub child_state: ChildDisplay,
    /// A board is bound (`devices.json` holds a Meshtastic binding).
    pub bound: bool,
    /// meshmud's binary (`current/bin/meshmud`) is installed.
    pub installed: bool,
    /// Something LISTENs on [`MESHMUD_DOWNLINK_PORT`] (`/proc/net/tcp`,
    /// never a connect — §13 health).
    pub downlink_listening: bool,
    /// The presence reader produced a valid node DB (§14 ingest).
    pub node_db_present: bool,
    /// The identified Meshtastic board (alias or model); `None` = none.
    pub board: Option<String>,
    pub firmware_version: Option<String>,
    /// Nodes in the current node DB.
    pub nodes_heard: u32,
    /// Seconds since meshmud lost its uplink to the board; `None` while
    /// connected.
    pub uplink_lost_secs: Option<u64>,
    /// Board-reported channel utilization (%), display only.
    pub channel_utilization: Option<f64>,
    /// The binding's uplink is a USB serial port (phones cannot open one).
    pub usb_uplink: bool,
    /// Another owner holds the bound port in the port-owner table
    /// (`daemon::ports`), so meshmud cannot open it: that holder's task.
    pub port_held_by: Option<String>,
    /// meshmud's log file, when it exists.
    pub log: Option<String>,
}

impl MeshmudFacts {
    /// meshmud is up: its process runs and its supervisor agrees.
    pub fn up(&self) -> bool {
        self.running && self.child_state == ChildDisplay::Running
    }
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
    /// opencode path when found (PATH, then the usual install directories:
    /// `probe::opencode_dirs`).
    pub opencode: Option<String>,
    /// Layer 5 §15; `None` = no fabric provider (the row reads NOT SET).
    pub fabric: Option<FabricFacts>,
    /// Layer 5 §14; `None` = no Meshtastic provider (NOT SET).
    pub meshmud: Option<MeshmudFacts>,
    /// Layer 5 §13: hosted panels, matched to services by `PanelInfo::id`.
    pub panels: Vec<PanelInfo>,
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
            fabric: None,
            meshmud: None,
            panels: Vec::new(),
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
    /// Layer 5 §13 hosted panel (attached by [`compute`]; when set,
    /// `embed_url` is `None`).
    pub panel: Option<PanelInfo>,
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
            panel: None,
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

/// `restart` while the daemon supervises the child (not absent, not stopped
/// for shutdown), `log` when its log file exists.
fn child_actions(display: ChildDisplay, log: bool) -> Vec<Action> {
    let mut out = Vec::new();
    if !matches!(display, ChildDisplay::Absent | ChildDisplay::Stopped) {
        out.push(Action::Restart);
    }
    if log {
        out.push(Action::Log);
    }
    out
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
    out.actions = child_actions(c.display, c.log.is_some());
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
    out.actions = child_actions(c.display, c.log.is_some());
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
    out.actions = child_actions(child, f.transport_log.is_some());
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
    let meshmud_bound = !r.meshmud_bound.is_empty();
    let meshmud_live = meshmud_bound && f.meshmud.as_ref().is_some_and(MeshmudFacts::up);
    let bound = !r.rnode.is_empty() || r.meshtastic_seen || meshmud_bound;
    let driven_by = if rnode_live {
        Some("rnsd")
    } else if meshmud_live {
        Some("meshmud")
    } else if r.meshtastic_open {
        Some("meshtasticd")
    } else {
        None
    };
    let mut out = if rnode_live {
        Computed::new(
            id,
            ServiceState::Live,
            "radio_bound_rnode",
            "The LoRa board is bound to the mesh transport (Reticulum RNode interface).",
        )
    } else if meshmud_live {
        // §14 / review J2: one rule, so `radio` and `meshtastic` never disagree
        let mut c = Computed::new(
            id,
            ServiceState::Live,
            "radio_bound_meshtastic",
            "The LoRa board runs Meshtastic, driven by meshmud.",
        );
        c.kb_entry = Some("projects/meshtastic.md");
        c
    } else if r.meshtastic_open {
        let mut c = Computed::new(
            id,
            ServiceState::Live,
            "radio_bound_meshtastic",
            "meshtasticd drives the LoRa radio and listens on port 4403.",
        );
        c.kb_entry = Some("projects/meshtastic.md");
        c
    } else if bound {
        let what = if !r.rnode.is_empty() {
            "The Reticulum process (rnsd)"
        } else if meshmud_bound {
            "meshmud"
        } else {
            "meshtasticd"
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
        "meshmud_bound": r.meshmud_bound,
        "driven_by": driven_by,
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
            "opencode is not installed (it was not found on the PATH or in the usual install folders).",
        ),
    };
    out.detail = json!({
        "category": f.category,
        "hub": hub,
        "opencode": f.opencode,
    });
    out
}

/// Layer 5 §15 `fabric` ("Messaging"): LIVE when the store is open and an
/// enrolled hub is reachable, or this hub has a member, or a peer link is
/// attributed; NOT SET when the store is open but nothing is reachable (a hub
/// without members is "waiting for the first node to enroll"; a node with no
/// hub is pointed at enrollment); ERROR on store failure or `store_full`,
/// and — each named for what it is, with the way out — when no identity
/// loaded (`no_identity`) or the identity holder is restore-pending
/// (`restore_pending`: its counters file is missing, so nothing is sealed).
fn fabric(f: &Facts) -> Computed {
    let id = ServiceId::Fabric;
    let Some(x) = &f.fabric else {
        let mut c = Computed::new(
            id,
            ServiceState::Unset,
            "fabric_off",
            "Messaging has not started on this node.",
        );
        c.detail = json!({ "started": false });
        return c;
    };
    let plural = |n: u32, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let mut out = if !x.store_open && x.no_identity {
        Computed::new(
            id,
            ServiceState::Error,
            "no_identity",
            "Messaging cannot start: this node's identity did not load. Run `111 identity restore` on the node.",
        )
    } else if !x.store_open {
        Computed::new(
            id,
            ServiceState::Error,
            "store_failed",
            "The message store could not be opened.",
        )
    } else if x.restore_pending {
        Computed::new(
            id,
            ServiceState::Error,
            "restore_pending",
            "Identity restore floor missing: nothing is sent or received until it is set. Put this node's identity/counters file back, or restart the node and run `111 identity restore` when it asks.",
        )
    } else if x.store_full {
        Computed::new(
            id,
            ServiceState::Error,
            "store_full",
            "The message store is full; new messages cannot be saved until space is freed.",
        )
    } else if x.hub_reachable == Some(true) {
        Computed::new(
            id,
            ServiceState::Live,
            "hub_reachable",
            "Your home hub is reachable; messages are delivered or held for you.",
        )
    } else if let Some(n) = x.members.filter(|n| *n > 0) {
        Computed::new(
            id,
            ServiceState::Live,
            "hub_serving",
            format!(
                "This node is a home hub for {}.",
                plural(n, "member node", "member nodes")
            ),
        )
    } else if x.attributed_links > 0 {
        Computed::new(
            id,
            ServiceState::Live,
            "peer_linked",
            format!(
                "Connected directly to {}.",
                plural(x.attributed_links, "other 111 node", "other 111 nodes")
            ),
        )
    } else if x.members == Some(0) {
        Computed::new(
            id,
            ServiceState::Unset,
            "hub_waiting",
            "This node is a home hub, waiting for the first node to enroll.",
        )
    } else if x.hub_reachable == Some(false) {
        Computed::new(
            id,
            ServiceState::Unset,
            "hub_unreachable",
            "Your home hub cannot be reached right now; messages wait until it can.",
        )
    } else {
        Computed::new(
            id,
            ServiceState::Unset,
            "not_enrolled",
            "No home hub and no other node reachable yet. Set up with the agent to enroll with a hub.",
        )
    };
    out.detail = json!({
        "started": true,
        "store": { "open": x.store_open, "full": x.store_full },
        "restore_pending": x.restore_pending,
        "hub": {
            "enrolled": x.hub_reachable.is_some(),
            "reachable": x.hub_reachable,
        },
        "hub_role": x.members.is_some(),
        "members": x.members,
        "attributed_links": x.attributed_links,
    });
    out
}

/// Layer 5 §14 `meshtastic`: LIVE when meshmud runs (downlink listening) and
/// the presence reader has produced a node DB; NOT SET when no Meshtastic
/// board is bound (identified-but-unbound boards point at the agent); ERROR
/// when meshmud crash-loops, its uplink is lost for more than
/// [`UPLINK_LOST_GRACE`], another owner holds the bound port (`port_busy`),
/// or a board is bound but meshmud's binary is not installed
/// (`binary_absent`: an install or upgrade problem, not a set-up step). A USB
/// board cannot be driven from a phone.
fn meshtastic(f: &Facts) -> Computed {
    let id = ServiceId::Meshtastic;
    let none = || {
        Computed::new(
            id,
            ServiceState::Unset,
            "no_meshtastic",
            "No Meshtastic board is set up on this node.",
        )
    };
    let Some(m) = &f.meshmud else {
        let mut c = none();
        c.detail = json!({ "child": display_str(ChildDisplay::Absent), "board": null });
        return c;
    };
    let lost = m.uplink_lost_secs;
    // a running meshmud owns its port; otherwise another holder blocks it
    let held_by = m
        .port_held_by
        .as_deref()
        .filter(|_| m.child_state != ChildDisplay::Running);
    let mut out = if let Some(holder) = held_by {
        Computed::new(
            id,
            ServiceState::Error,
            "port_busy",
            format!("The board's port is in use by {holder}, so meshmud cannot open it."),
        )
    } else {
        match m.child_state {
            ChildDisplay::Absent
                if m.board.is_some()
                    && m.bound
                    && !m.installed
                    && !(f.phone_like && m.usb_uplink) =>
            {
                Computed::new(
                    id,
                    ServiceState::Error,
                    "binary_absent",
                    "A Meshtastic board is set up, but meshmud is not installed on this node, so \
                     it cannot run (reinstall or update 111).",
                )
            }
            ChildDisplay::Absent if m.board.is_some() => {
                let mut c = Computed::new(
                    id,
                    ServiceState::Unset,
                    "meshtastic_unbound",
                    "A Meshtastic board is identified but not set up yet. Set up with the agent.",
                );
                if f.phone_like && m.usb_uplink {
                    c.can_run_here = false;
                    c.reason =
                        "This device cannot drive a USB board; a Wi-Fi Meshtastic board can be \
                    set up by its address."
                            .into();
                }
                c
            }
            ChildDisplay::Absent => none(),
            ChildDisplay::Degraded => Computed::new(
                id,
                ServiceState::Error,
                "child_degraded",
                "meshmud kept crashing, so the node stopped restarting it.",
            ),
            ChildDisplay::Stopped => Computed::new(
                id,
                ServiceState::Error,
                "child_stopped",
                "meshmud was stopped.",
            ),
            ChildDisplay::Backoff => Computed::new(
                id,
                ServiceState::Error,
                "child_backoff",
                "meshmud crashed and is being restarted.",
            ),
            ChildDisplay::Running if lost.is_some_and(|s| s > UPLINK_LOST_GRACE.as_secs()) => {
                Computed::new(
                    id,
                    ServiceState::Error,
                    "uplink_lost",
                    "meshmud lost its connection to the board over a minute ago.",
                )
            }
            ChildDisplay::Running if lost.is_some() => Computed::new(
                id,
                ServiceState::Starting,
                "uplink_waiting",
                "meshmud lost its connection to the board and is reconnecting.",
            ),
            ChildDisplay::Running if !m.running || !m.downlink_listening => Computed::new(
                id,
                ServiceState::Starting,
                "downlink_pending",
                "meshmud is starting.",
            ),
            ChildDisplay::Running if !m.node_db_present => Computed::new(
                id,
                ServiceState::Starting,
                "node_db_pending",
                "meshmud is connected; waiting for the board's list of nodes.",
            ),
            ChildDisplay::Running => Computed::new(
                id,
                ServiceState::Live,
                "meshtastic_live",
                "meshmud is connected to the board and its list of nodes is in.",
            ),
        }
    };
    out.actions = child_actions(m.child_state, m.log.is_some());
    out.detail = json!({
        "child": display_str(m.child_state),
        "running": m.running,
        "downlink": { "port": MESHMUD_DOWNLINK_PORT, "listening": m.downlink_listening },
        "node_db_present": m.node_db_present,
        "board": m.board,
        "firmware_version": m.firmware_version,
        "nodes_heard": m.nodes_heard,
        "channel_utilization": m.channel_utilization,
        "uplink_lost_sec": m.uplink_lost_secs,
        "usb_uplink": m.usb_uplink,
        "port_held_by": m.port_held_by,
        "bound": m.bound,
        "meshmud_installed": m.installed,
        "log": m.log,
    });
    out
}

/// The §7 table: one [`Computed`] per service, in [`ServiceId::ALL`] order.
/// A panel in `f.panels` attaches to the service of the same id (Layer 5
/// §13) and clears that record's `embed_url`.
pub fn compute(f: &Facts) -> [Computed; ServiceId::COUNT] {
    let mut out = [
        agent(f),
        model(f),
        transport(f),
        radio(f),
        kb(f),
        gateway(f),
        codeagent(f),
        fabric(f),
        meshtastic(f),
    ];
    for c in &mut out {
        c.panel = f.panels.iter().find(|p| p.id == c.id).cloned();
        if c.panel.is_some() {
            c.embed_url = None;
        }
    }
    out
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
    pub fn tick(&self, computed: [Computed; ServiceId::COUNT], now: &str) -> bool {
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
                slot.published.panel = c.panel;
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
        panel: c.panel.clone(),
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
                "codeagent",
                "fabric",
                "meshtastic"
            ]
        );
        assert_eq!(ServiceId::ALL.len(), ServiceId::COUNT);
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

    fn row(f: &Facts, id: ServiceId) -> Computed {
        compute(f).into_iter().find(|c| c.id == id).unwrap()
    }

    fn st(f: &Facts, id: ServiceId) -> (ServiceState, &'static str, bool) {
        let c = row(f, id);
        (c.state, c.reason_code, c.can_run_here)
    }

    use ServiceState::{Error as E, Live as L, Starting as S, Unset as U};

    #[test]
    fn fabric_rule_matrix() {
        let id = ServiceId::Fabric;
        let mut f = Facts::default();
        // no provider: NOT SET, honestly
        assert_eq!(st(&f, id), (U, "fabric_off", true));
        assert_eq!(row(&f, id).detail["started"], false);
        let base = FabricFacts {
            store_open: true,
            ..FabricFacts::default()
        };
        let with = |x: FabricFacts| Facts {
            fabric: Some(x),
            ..Facts::default()
        };
        // store open, nothing reachable, no hub: point at enrollment
        f = with(base.clone());
        assert_eq!(st(&f, id), (U, "not_enrolled", true));
        assert!(row(&f, id).reason.contains("Set up with the agent"));
        // a hub with no members: waiting for the first node to enroll
        f = with(FabricFacts {
            members: Some(0),
            ..base.clone()
        });
        assert_eq!(st(&f, id), (U, "hub_waiting", true));
        assert!(row(&f, id)
            .reason
            .contains("waiting for the first node to enroll"));
        // enrolled, hub unreachable: NOT SET (nothing reachable)
        f = with(FabricFacts {
            hub_reachable: Some(false),
            ..base.clone()
        });
        assert_eq!(st(&f, id), (U, "hub_unreachable", true));
        // LIVE: hub reachable · a member · an attributed link
        f = with(FabricFacts {
            hub_reachable: Some(true),
            ..base.clone()
        });
        assert_eq!(st(&f, id), (L, "hub_reachable", true));
        f = with(FabricFacts {
            members: Some(2),
            ..base.clone()
        });
        assert_eq!(st(&f, id), (L, "hub_serving", true));
        assert!(row(&f, id).reason.contains("2 member nodes"));
        f = with(FabricFacts {
            hub_reachable: Some(false),
            attributed_links: 1,
            ..base.clone()
        });
        assert_eq!(st(&f, id), (L, "peer_linked", true));
        assert!(row(&f, id).reason.contains("1 other 111 node."));
        // ERROR: store failure, store_full (wins over reachability)
        f = with(FabricFacts {
            store_open: false,
            hub_reachable: Some(true),
            ..FabricFacts::default()
        });
        assert_eq!(st(&f, id), (E, "store_failed", true));
        f = with(FabricFacts {
            store_full: true,
            hub_reachable: Some(true),
            ..base.clone()
        });
        let c = row(&f, id);
        assert_eq!((c.state, c.reason_code), (E, "store_full"));
        assert_eq!(c.detail["store"]["full"], true);
        assert_eq!(c.detail["hub"]["enrolled"], true);
        assert!(c.actions.is_empty(), "built in: nothing to restart");
        // F2 review m8: the down and restore-pending states are named, with
        // the way out — never the generic store failure
        f = with(FabricFacts {
            store_open: false,
            no_identity: true,
            ..FabricFacts::default()
        });
        let c = row(&f, id);
        assert_eq!((c.state, c.reason_code), (E, "no_identity"));
        assert!(c.reason.contains("111 identity restore"), "{}", c.reason);
        f = with(FabricFacts {
            restore_pending: true,
            hub_reachable: Some(true),
            ..base.clone()
        });
        let c = row(&f, id);
        assert_eq!((c.state, c.reason_code), (E, "restore_pending"));
        assert!(c.reason.contains("restore floor missing"), "{}", c.reason);
        assert!(c.reason.contains("identity/counters"), "{}", c.reason);
        assert_eq!(c.detail["restore_pending"], true);
    }

    fn mm(child: ChildDisplay) -> MeshmudFacts {
        MeshmudFacts {
            running: child == ChildDisplay::Running,
            child_state: child,
            bound: child != ChildDisplay::Absent,
            installed: true,
            downlink_listening: child == ChildDisplay::Running,
            node_db_present: true,
            board: Some("Heltec V3".into()),
            firmware_version: Some("2.6.11".into()),
            nodes_heard: 12,
            log: Some("/x/meshmud.log".into()),
            ..MeshmudFacts::default()
        }
    }

    #[test]
    fn meshtastic_rule_matrix() {
        let id = ServiceId::Meshtastic;
        let with = |m: MeshmudFacts| Facts {
            meshmud: Some(m),
            ..Facts::default()
        };
        // no provider / nothing identified: NOT SET
        assert_eq!(st(&Facts::default(), id), (U, "no_meshtastic", true));
        let mut m = mm(ChildDisplay::Absent);
        m.board = None;
        assert_eq!(st(&with(m), id), (U, "no_meshtastic", true));
        // identified, not bound: NOT SET, agent set-up
        let c = row(&with(mm(ChildDisplay::Absent)), id);
        assert_eq!((c.state, c.reason_code), (U, "meshtastic_unbound"));
        assert!(c.reason.contains("Set up with the agent"));
        assert!(c.actions.contains(&Action::Log) && !c.actions.contains(&Action::Restart));
        // a USB board on a phone cannot run here; a Wi-Fi one can
        let mut phone = with(MeshmudFacts {
            usb_uplink: true,
            ..mm(ChildDisplay::Absent)
        });
        phone.phone_like = true;
        assert_eq!(st(&phone, id), (U, "meshtastic_unbound", false));
        phone.meshmud.as_mut().unwrap().usb_uplink = false;
        assert_eq!(st(&phone, id), (U, "meshtastic_unbound", true));
        // bound, but meshmud's binary is missing: an install problem, not a
        // set-up step (never "set up with the agent")
        let missing = MeshmudFacts {
            bound: true,
            installed: false,
            ..mm(ChildDisplay::Absent)
        };
        let c = row(&with(missing.clone()), id);
        assert_eq!((c.state, c.reason_code), (E, "binary_absent"));
        assert!(
            c.reason.contains("meshmud is not installed"),
            "{}",
            c.reason
        );
        assert!(!c.reason.contains("agent"), "{}", c.reason);
        assert_eq!(c.detail["meshmud_installed"], false);
        assert_eq!(c.detail["bound"], true);
        // … identified but unbound stays the set-up step
        let unbound = MeshmudFacts {
            bound: false,
            ..missing.clone()
        };
        assert_eq!(st(&with(unbound), id), (U, "meshtastic_unbound", true));
        // … a USB binding on a phone still reads "cannot run here"
        let mut phone = with(MeshmudFacts {
            usb_uplink: true,
            ..missing
        });
        phone.phone_like = true;
        assert_eq!(st(&phone, id), (U, "meshtastic_unbound", false));
        // LIVE: running + downlink + node DB
        let c = row(&with(mm(ChildDisplay::Running)), id);
        assert_eq!((c.state, c.reason_code), (L, "meshtastic_live"));
        assert_eq!(c.actions, [Action::Restart, Action::Log]);
        assert_eq!(c.detail["downlink"]["port"], 11230);
        assert_eq!(c.detail["board"], "Heltec V3");
        assert_eq!(c.detail["nodes_heard"], 12);
        // starting: downlink not yet listening / no node DB yet / uplink blip
        let mut m = mm(ChildDisplay::Running);
        m.downlink_listening = false;
        assert_eq!(st(&with(m), id), (S, "downlink_pending", true));
        let mut m = mm(ChildDisplay::Running);
        m.running = false;
        assert_eq!(st(&with(m), id), (S, "downlink_pending", true));
        let mut m = mm(ChildDisplay::Running);
        m.node_db_present = false;
        assert_eq!(st(&with(m), id), (S, "node_db_pending", true));
        let mut m = mm(ChildDisplay::Running);
        m.uplink_lost_secs = Some(60);
        assert_eq!(st(&with(m), id), (S, "uplink_waiting", true));
        // ERROR: uplink lost > 60 s, crash loop, backoff, stopped
        let mut m = mm(ChildDisplay::Running);
        m.uplink_lost_secs = Some(61);
        assert_eq!(st(&with(m), id), (E, "uplink_lost", true));
        assert_eq!(
            st(&with(mm(ChildDisplay::Degraded)), id),
            (E, "child_degraded", true)
        );
        assert_eq!(
            st(&with(mm(ChildDisplay::Backoff)), id),
            (E, "child_backoff", true)
        );
        assert_eq!(
            st(&with(mm(ChildDisplay::Stopped)), id),
            (E, "child_stopped", true)
        );
        // the bound port has another owner: meshmud cannot open it
        let busy = |child| MeshmudFacts {
            port_held_by: Some("rnsd".into()),
            ..mm(child)
        };
        let c = row(&with(busy(ChildDisplay::Absent)), id);
        assert_eq!((c.state, c.reason_code), (E, "port_busy"));
        assert!(c.reason.contains("rnsd"), "{}", c.reason);
        assert_eq!(c.detail["port_held_by"], "rnsd");
        assert_eq!(
            st(&with(busy(ChildDisplay::Backoff)), id),
            (E, "port_busy", true)
        );
        // a running meshmud owns its port: the fact is stale, not a conflict
        assert_eq!(
            st(&with(busy(ChildDisplay::Running)), id),
            (L, "meshtastic_live", true)
        );
    }

    #[test]
    fn radio_counts_a_meshmud_bound_board() {
        let id = ServiceId::Radio;
        let board = Radio {
            vid: "10c4".into(),
            pid: "ea60".into(),
            product: "CP2102".into(),
            name: "Heltec WiFi LoRa 32 V3 (CP2102)".into(),
            tty: Some("ttyUSB0".into()),
            by_id: None,
        };
        let mut f = Facts::default();
        f.radio.boards = Some(vec![board]);
        assert_eq!(st(&f, id), (U, "radio_unbound", true));
        // bound + meshmud up: live, driven by meshmud — agrees with `meshtastic`
        f.radio.meshmud_bound = vec!["/dev/ttyUSB0".into()];
        f.meshmud = Some(mm(ChildDisplay::Running));
        let c = row(&f, id);
        assert_eq!((c.state, c.reason_code), (L, "radio_bound_meshtastic"));
        assert!(c.reason.contains("driven by meshmud"));
        assert_eq!(c.detail["driven_by"], "meshmud");
        assert_eq!(c.detail["meshmud_bound"][0], "/dev/ttyUSB0");
        assert_eq!(c.kb_entry, Some("projects/meshtastic.md"));
        assert_eq!(st(&f, ServiceId::Meshtastic).0, L);
        // meshmud down: still bound — waiting, then stack_down
        f.meshmud = Some(mm(ChildDisplay::Backoff));
        f.radio.down_for = Some(Duration::from_secs(5));
        let c = row(&f, id);
        assert_eq!((c.state, c.reason_code), (S, "stack_waiting"));
        assert!(c.reason.starts_with("meshmud"), "{}", c.reason);
        f.radio.down_for = Some(STACK_DOWN_GRACE);
        assert_eq!(st(&f, id), (E, "stack_down", true));
        // meshmud running without a binding does not bind the radio
        f.radio.meshmud_bound.clear();
        f.radio.down_for = None;
        f.meshmud = Some(mm(ChildDisplay::Running));
        assert_eq!(st(&f, id), (U, "radio_unbound", true));
        assert_eq!(row(&f, id).detail["driven_by"], Value::Null);
    }

    #[test]
    fn panels_attach_by_id_and_clear_embed_url() {
        let panel = PanelInfo {
            id: ServiceId::Meshtastic,
            port: 11190,
            entry: "/".into(),
            needs_secure_context: false,
            allow: vec!["serial".into(), "bluetooth".into()],
            optional_hosts: vec!["tiles.openfreemap.org".into()],
            state: PanelState {
                enabled: true,
                listening: true,
                health: PanelHealth::Up,
                dropped: Some(0),
            },
            title: Some("Meshtastic".into()),
            secure_note: None,
        };
        let f = Facts {
            panels: vec![panel.clone()],
            ..Facts::default()
        };
        let out = compute(&f);
        for c in &out {
            if c.id == ServiceId::Meshtastic {
                assert_eq!(c.panel.as_ref(), Some(&panel));
                assert_eq!(c.embed_url, None);
            } else {
                assert_eq!(c.panel, None, "{:?}", c.id);
            }
        }
        // the record carries it; `null` when absent (contract `| null`)
        let reg = Registry::new();
        reg.tick(out, "t0");
        let rec = serde_json::to_value(reg.get(ServiceId::Meshtastic).unwrap()).unwrap();
        assert_eq!(
            rec["panel"],
            json!({
                "id": "meshtastic",
                "port": 11190,
                "entry": "/",
                "needs_secure_context": false,
                "allow": ["serial", "bluetooth"],
                "optional_hosts": ["tiles.openfreemap.org"],
                "state": {"enabled": true, "listening": true, "health": "up", "dropped": 0},
                "title": "Meshtastic",
            })
        );
        assert!(rec.get("embed_url").is_none());
        let kb = serde_json::to_value(reg.get(ServiceId::Kb).unwrap()).unwrap();
        assert_eq!(kb["panel"], Value::Null);
    }

    #[test]
    fn layer5_catalogue_entries() {
        let f = catalogue(ServiceId::Fabric);
        assert_eq!((f.plain_name, f.kind), ("Messaging", Kind::Builtin));
        let m = catalogue(ServiceId::Meshtastic);
        assert_eq!((m.plain_name, m.kind), ("Meshtastic", Kind::Child));
        assert_eq!(m.kb_entry, Some("projects/meshtastic-web.md"));
        for id in ServiceId::ALL {
            let c = catalogue(id);
            assert!(c.description.len() > 40, "{id:?}");
            assert!(c.kb_entry.is_none_or(|k| k.ends_with(".md")), "{id:?}");
        }
    }

    #[test]
    fn supervised_children_keep_order_and_primary() {
        let s = Supervised::default().with(ServiceId::Model, None);
        assert!(s.get(ServiceId::Model).is_none());
        assert!(s.children(ServiceId::Meshtastic).is_empty());
        let s = s.with_children(ServiceId::Meshtastic, Vec::new());
        assert!(s.children(ServiceId::Meshtastic).is_empty());
    }
}
