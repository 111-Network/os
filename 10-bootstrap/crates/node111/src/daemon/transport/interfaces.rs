//! `state/interfaces.toml`: the transport spine's config home and the
//! **single source of truth for interface ownership** (`os/60-transport/
//! README.md` §11; D17 #6, D19 c). `111d` renders rnsd's config from it
//! (`config.rs`, renderer v2), the agent proposes typed actions, the human
//! decides.
//!
//! - **Shape (schema 1, §11).** `[radio] region/callsign`, `[posture]
//!   share_instance/instance_name/enable_transport`, `[posture.lan] backbone =
//!   {listen, port, ifac}, stock_autointerface, propagation_node`, `[order]
//!   networks`, `[[interface]] name/owner/type/device/port/region/params/
//!   uplink`. Parsed with `deny_unknown_fields`; the size is checked before
//!   the parse ([`MAX_FILE_BYTES`]).
//! - **Validation (§11 "Renderer v2", L5 §14).** Stanza names are sanitized
//!   ([`sanitize_name`]; a stored name must already be in that form), `port`
//!   matches `^/dev/(serial/by-id/)?[A-Za-z0-9_.:-]+$` or is `tcp://host:port`,
//!   region ids come from the region table ([`REGIONS`]; unknown ⇒ refused),
//!   every `params` key is on the stanza type's allowlist with a typed range,
//!   and **one owner per port** (a second entry on a held port is refused).
//! - **Files.** 0600, written atomically ([`save`]); the file is opened with
//!   `O_NOFOLLOW`, must be a regular file, and a wrong mode is repaired to
//!   0600 on load (logged). An invalid file is **moved aside** to
//!   `interfaces.toml.invalid-<n>` (never deleted) and the defaults are
//!   written in its place; a file of a **newer** schema is left alone (this
//!   binary runs on the defaults, read-only: rollback safety, the same rule
//!   as the rns config's `schema-version`).
//! - **Migration (integration M7).** On the first start with Layer 6 (file
//!   absent) the Meshtastic binding from `devices.json` becomes an `owner =
//!   "111d"`, `type = "meshtastic"` entry and every enabled `RNodeInterface`
//!   stanza of `rns/config` an `owner = "rnsd"` entry ([`migrate`]). From
//!   then on this file wins: the binding is read from it
//!   ([`InterfacesFile::meshtastic_binding`], [`meshtastic_binding_at`]), the
//!   copy in `devices.json` is a write-only mirror
//!   (`devices::store::mirror_meshtastic_binding`), and a divergence found at
//!   start is resolved for this file with one log line naming both values.

use crate::daemon::devices::store::{board_name, MeshtasticBinding};
use crate::daemon::devices::valid_id;
use crate::daemon::meshtastic::meshmud::{valid_serial_path, Uplink};
use crate::daemon::ports::port_key;
use crate::install::categorize::Category;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::io::Read;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

/// The file under `state/` (§2 "State").
pub const FILE: &str = "interfaces.toml";
/// The schema this binary reads and writes (§11).
pub const SCHEMA: u32 = 1;
/// Read cap, checked before the parse (the file is a few KB).
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
/// Most `[[interface]]` entries accepted.
pub const MAX_INTERFACES: usize = 32;
/// Longest stanza name (bytes, after sanitizing).
pub const MAX_NAME_BYTES: usize = 48;
/// Most entries in `[order] networks`.
pub const MAX_ORDER: usize = 16;
/// Longest callsign (contract `TransportStatus.callsign`).
pub const MAX_CALLSIGN: usize = 16;
/// rnsd's Backbone listener port on the LAN (§2 "Ports", §11).
pub const BACKBONE_PORT: u16 = 4242;
/// The shared-instance name (§6 "rnsd posture v2").
pub const DEFAULT_INSTANCE: &str = "111";
/// README §6's preferred networks, the default `[order]`.
pub const DEFAULT_ORDER: [&str; 5] = ["111", "lxmf", "meshtastic", "app:deltachat", "app:simplex"];
/// The `111.network` AutoInterface stanza (kept from v1, §6).
pub const AUTO_NAME: &str = "Auto Interface";
/// The opt-in second AutoInterface in the stock `reticulum` group (§6).
pub const STOCK_AUTO_NAME: &str = "Reticulum Auto Interface";
/// The Backbone listener stanza (§11 "Phone relay v1").
pub const BACKBONE_NAME: &str = "111 LAN Backbone";
/// Stanza names 111d renders itself; refused for `[[interface]]` entries.
pub const RESERVED_NAMES: [&str; 3] = [AUTO_NAME, STOCK_AUTO_NAME, BACKBONE_NAME];
/// `PortTable` task of rnsd-owned ports (§12 "Port ownership").
pub const TASK_RNSD: &str = "rnsd";
/// `PortTable` task of the 111d-owned Meshtastic uplink (meshmud drives it,
/// `N/meshtastic/mod.rs` claims under the same task).
pub const TASK_MESHMUD: &str = "meshmud";
/// Attempts at a free `interfaces.toml.invalid-<n>` name.
const MOVE_ASIDE_TRIES: u32 = 1000;
/// Comment head of a saved file.
const HEADER: &str = "# 111 transport spine (os/60-transport/README.md §11): interface ownership and the rnsd\n\
# posture. Written by 111d (0600); change it in Settings → Layer 6 or through an accepted proposal.\n";

// ---------------------------------------------------------------- regions

/// One region's frequency plan (§11: "the frequency plan comes from the KB
/// region table, generated from the pinned `config.proto`", R05 §3.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionPlan {
    /// `config.proto` `LoRaConfig.RegionCode` name.
    pub id: &'static str,
    /// Lowest permitted frequency (Hz).
    pub start_hz: u64,
    /// Highest permitted frequency (Hz).
    pub end_hz: u64,
    /// Duty-cycle limit in percent (100 = none).
    pub duty_cycle_pct: u8,
    /// Transmit power limit (dBm).
    pub power_limit_dbm: u8,
}

const fn plan(id: &'static str, start_khz: u64, end_khz: u64, duty: u8, power: u8) -> RegionPlan {
    RegionPlan {
        id,
        start_hz: start_khz * 1000,
        end_hz: end_khz * 1000,
        duty_cycle_pct: duty,
        power_limit_dbm: power,
    }
}

/// The v1 region table: every `LoRaConfig.RegionCode` but `UNSET` (names
/// *src*: the `config.proto` vendored in the `meshtastic` 0.1.9 crate), with
/// the frequency plan of the Meshtastic firmware's `RDEF` table (*src*:
/// `meshtastic/firmware` v2.7.15 `src/mesh/RadioInterface.cpp`: start, end,
/// duty cycle, power limit). Shipped in this file until the KB region table
/// generated from the pinned 2.8.1 `config.proto` (37 codes, R05 §3.16)
/// replaces it; a region missing here is refused (fail closed).
pub const REGIONS: &[RegionPlan] = &[
    plan("US", 902_000, 928_000, 100, 30),
    plan("EU_433", 433_000, 434_000, 10, 10),
    plan("EU_868", 869_400, 869_650, 10, 27),
    plan("CN", 470_000, 510_000, 100, 19),
    plan("JP", 920_500, 923_500, 100, 13),
    plan("ANZ", 915_000, 928_000, 100, 30),
    plan("KR", 920_000, 923_000, 100, 23),
    plan("TW", 920_000, 925_000, 100, 27),
    plan("RU", 868_700, 869_200, 100, 20),
    plan("IN", 865_000, 867_000, 100, 30),
    plan("NZ_865", 864_000, 868_000, 100, 36),
    plan("TH", 920_000, 925_000, 100, 16),
    plan("LORA_24", 2_400_000, 2_483_500, 100, 10),
    plan("UA_433", 433_000, 434_700, 10, 10),
    plan("UA_868", 868_000, 868_600, 1, 14),
    plan("MY_433", 433_000, 435_000, 100, 20),
    plan("MY_919", 919_000, 924_000, 100, 27),
    plan("SG_923", 917_000, 925_000, 100, 20),
    plan("PH_433", 433_000, 434_700, 100, 10),
    plan("PH_868", 868_000, 869_400, 100, 14),
    plan("PH_915", 915_000, 918_000, 100, 24),
    plan("ANZ_433", 433_050, 434_790, 100, 14),
    plan("KZ_433", 433_075, 434_775, 100, 10),
    plan("KZ_863", 863_000, 868_000, 100, 30),
    plan("NP_865", 865_000, 868_000, 100, 30),
    plan("BR_902", 902_000, 907_500, 100, 30),
];

/// The plan of region `id` (exact, case-sensitive id), if known.
pub fn region(id: &str) -> Option<&'static RegionPlan> {
    REGIONS.iter().find(|r| r.id == id)
}

/// RNode LoRa bandwidths (Hz) the firmware accepts (SX127x/SX126x set, plus
/// the SX128x 2.4 GHz set).
pub const RNODE_BANDWIDTHS: &[i64] = &[
    7_800, 10_400, 15_600, 20_800, 31_250, 41_700, 62_500, 125_000, 250_000, 500_000, 203_125,
    406_250, 812_500, 1_625_000,
];

// ---------------------------------------------------------------- shape

/// The whole file (§11 shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterfacesFile {
    pub schema: u32,
    #[serde(default)]
    pub radio: Radio,
    #[serde(default)]
    pub posture: Posture,
    #[serde(default)]
    pub order: Order,
    #[serde(default, rename = "interface", skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<Interface>,
}

/// `[radio]` (D17 #10: asked once at first radio setup; the agent suggests,
/// never sets).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Radio {
    /// A [`REGIONS`] id; absent until a human sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// Optional, licensed operators (empty = none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callsign: Option<String>,
}

impl Radio {
    /// The callsign, `None` when absent or empty.
    pub fn callsign(&self) -> Option<&str> {
        self.callsign.as_deref().filter(|c| !c.is_empty())
    }
}

/// `[posture]` (D17 #5; Settings → Layer 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Posture {
    #[serde(default = "yes")]
    pub share_instance: bool,
    #[serde(default = "default_instance")]
    pub instance_name: String,
    /// `None` = the category default ([`Posture::enable_transport_for`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_transport: Option<bool>,
    #[serde(default)]
    pub lan: Lan,
}

impl Default for Posture {
    fn default() -> Self {
        Posture {
            share_instance: true,
            instance_name: DEFAULT_INSTANCE.into(),
            enable_transport: None,
            lan: Lan::default(),
        }
    }
}

impl Posture {
    /// `enable_transport` as rendered: the file's value, else true on Cat
    /// 3/4 and false elsewhere (§11, §6 "rnsd posture v2"); an unknown
    /// category takes the non-routing default.
    pub fn enable_transport_for(&self, category: Option<Category>) -> bool {
        self.enable_transport.unwrap_or(matches!(
            category,
            Some(Category::Bridge3 | Category::Hub4A | Category::Hub4B)
        ))
    }
}

/// `[posture.lan]`: rendered only while LAN sharing is on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lan {
    /// Second AutoInterface, group `reticulum`: opt-in.
    #[serde(default)]
    pub stock_autointerface: bool,
    /// LXMF propagation role: opt-in.
    #[serde(default)]
    pub propagation_node: bool,
    #[serde(default)]
    pub backbone: Backbone,
}

/// `backbone = { listen, port, ifac }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backbone {
    #[serde(default = "yes")]
    pub listen: bool,
    #[serde(default = "default_backbone_port")]
    pub port: u16,
    #[serde(default)]
    pub ifac: bool,
}

impl Default for Backbone {
    fn default() -> Self {
        Backbone {
            listen: true,
            port: BACKBONE_PORT,
            ifac: false,
        }
    }
}

/// `[order]`: the preferred networks in the user's order (README §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Order {
    #[serde(default = "default_order")]
    pub networks: Vec<String>,
}

impl Default for Order {
    fn default() -> Self {
        Order {
            networks: default_order(),
        }
    }
}

/// Who drives an interface (D19 c): one owner per port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IfOwner {
    /// A board 111d drives itself (Meshtastic via meshmud).
    #[serde(rename = "111d")]
    D111,
    /// A stanza in rnsd's config.
    #[serde(rename = "rnsd")]
    Rnsd,
}

/// The stanza types v1 knows (anything else ⇒ the file is invalid).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IfKind {
    #[serde(rename = "RNodeInterface")]
    RNode,
    /// The phone-relay client (§11, [`crate::daemon::transport::join`]).
    #[serde(rename = "TCPClientInterface")]
    TcpClient,
    #[serde(rename = "meshtastic")]
    Meshtastic,
}

impl IfKind {
    /// rnsd's `type =` value (`None`: not an rnsd stanza).
    pub fn rnsd_type(self) -> Option<&'static str> {
        match self {
            IfKind::RNode => Some("RNodeInterface"),
            IfKind::TcpClient => Some("TCPClientInterface"),
            IfKind::Meshtastic => None,
        }
    }

    /// The owner this type requires.
    pub fn owner(self) -> IfOwner {
        match self {
            IfKind::RNode | IfKind::TcpClient => IfOwner::Rnsd,
            IfKind::Meshtastic => IfOwner::D111,
        }
    }
}

/// One `[[interface]]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interface {
    pub name: String,
    pub owner: IfOwner,
    #[serde(rename = "type")]
    pub kind: IfKind,
    /// Stable `DeviceRecord` id (DEV §7.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// rnsd RNode port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    /// Per-device override of `[radio] region` (D17 #10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// The Meshtastic board end (a `/dev/…` path or `tcp://host:port`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uplink: Option<String>,
    /// Typed per-type parameters ([`param_specs`]).
    #[serde(default, skip_serializing_if = "toml::Table::is_empty")]
    pub params: toml::Table,
}

impl Interface {
    /// The effective region: the entry's override, else `[radio]`.
    pub fn effective_region<'a>(&'a self, radio: &'a Radio) -> Option<&'a str> {
        self.region.as_deref().or(radio.region.as_deref())
    }

    /// The radio port this entry owns (RNode `port`, Meshtastic `uplink`),
    /// as given.
    pub fn owned_port(&self) -> Option<String> {
        match self.kind {
            IfKind::RNode => self.port.clone(),
            IfKind::Meshtastic => self
                .uplink
                .as_deref()
                .and_then(|u| parse_uplink(u).ok())
                .map(|u| u.port_name()),
            IfKind::TcpClient => None,
        }
    }

    /// The `PortTable` task that claims this entry's port (§12).
    pub fn claim_task(&self) -> &'static str {
        match self.owner {
            IfOwner::Rnsd => TASK_RNSD,
            IfOwner::D111 => TASK_MESHMUD,
        }
    }

    /// An integer parameter.
    pub fn int(&self, key: &str) -> Option<i64> {
        self.params.get(key).and_then(toml::Value::as_integer)
    }

    /// A string parameter.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.params.get(key).and_then(toml::Value::as_str)
    }

    /// A boolean parameter (absent = false).
    pub fn flag(&self, key: &str) -> bool {
        self.params
            .get(key)
            .and_then(toml::Value::as_bool)
            .unwrap_or(false)
    }
}

fn yes() -> bool {
    true
}
fn default_instance() -> String {
    DEFAULT_INSTANCE.into()
}
fn default_backbone_port() -> u16 {
    BACKBONE_PORT
}
fn default_order() -> Vec<String> {
    DEFAULT_ORDER.iter().map(|s| s.to_string()).collect()
}

impl Default for InterfacesFile {
    fn default() -> Self {
        InterfacesFile {
            schema: SCHEMA,
            radio: Radio::default(),
            posture: Posture::default(),
            order: Order::default(),
            interfaces: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------- validation

/// Why a file or a change was refused (nothing is written or rendered).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    Schema(u32),
    TooMany(usize),
    BadName(String),
    ReservedName(String),
    DuplicateName(String),
    /// `owner` does not match the type (`meshtastic` is 111d's, the rest rnsd's).
    OwnerKind(String),
    /// A second Meshtastic entry (one meshmud, one downlink: L5 §14).
    SecondMeshtastic(String),
    BadDevice(String),
    MissingPort(String),
    MissingUplink(String),
    /// A field this type does not take.
    UnexpectedField {
        name: String,
        field: &'static str,
    },
    BadPort {
        name: String,
        port: String,
    },
    BadUplink {
        name: String,
        why: String,
    },
    /// One owner per port (§11, D19 c).
    PortHeld {
        name: String,
        port: String,
        by: String,
    },
    UnknownRegion(String),
    BadParam {
        name: String,
        key: String,
        why: &'static str,
    },
    MissingParam {
        name: String,
        key: &'static str,
    },
    /// An RNode entry without a region (D17 #10, DEV §0.6): refused for a
    /// proposal; a stored one is left out of the render.
    NoRegion(String),
    BadInstanceName(String),
    BadBackbonePort(u16),
    BadCallsign,
    BadOrder(String),
}

/// Untrusted text in a message: control characters dropped, ≤ 80 chars.
fn shown(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(80).collect()
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Invalid::Schema(v) => write!(f, "schema {v} is not {SCHEMA}"),
            Invalid::TooMany(n) => write!(f, "{n} interfaces (at most {MAX_INTERFACES})"),
            Invalid::BadName(n) => write!(
                f,
                "interface name {:?} is not a sanitized stanza name",
                shown(n)
            ),
            Invalid::ReservedName(n) => {
                write!(f, "interface name {n:?} is reserved for 111d's own stanzas")
            }
            Invalid::DuplicateName(n) => write!(f, "two interfaces are named {:?}", shown(n)),
            Invalid::OwnerKind(n) => write!(
                f,
                "interface {:?}: the owner does not match its type",
                shown(n)
            ),
            Invalid::SecondMeshtastic(n) => write!(
                f,
                "interface {:?}: only one Meshtastic board can be bound",
                shown(n)
            ),
            Invalid::BadDevice(n) => {
                write!(f, "interface {:?}: device is not a device id", shown(n))
            }
            Invalid::MissingPort(n) => write!(f, "interface {:?}: an RNode needs a port", shown(n)),
            Invalid::MissingUplink(n) => write!(
                f,
                "interface {:?}: a Meshtastic board needs an uplink and a device",
                shown(n)
            ),
            Invalid::UnexpectedField { name, field } => write!(
                f,
                "interface {:?}: {field} does not apply to this type",
                shown(name)
            ),
            Invalid::BadPort { name, port } => write!(
                f,
                "interface {:?}: port {:?} is not an accepted /dev path or tcp://host:port",
                shown(name),
                shown(port)
            ),
            Invalid::BadUplink { name, why } => write!(f, "interface {:?}: {why}", shown(name)),
            Invalid::PortHeld { name, port, by } => write!(
                f,
                "interface {:?}: port {:?} is already owned by {:?}",
                shown(name),
                shown(port),
                shown(by)
            ),
            Invalid::UnknownRegion(r) => {
                write!(f, "region {:?} is not in the region table", shown(r))
            }
            Invalid::BadParam { name, key, why } => write!(
                f,
                "interface {:?}: parameter {:?} {why}",
                shown(name),
                shown(key)
            ),
            Invalid::MissingParam { name, key } => write!(
                f,
                "interface {:?}: parameter {key} is required",
                shown(name)
            ),
            Invalid::NoRegion(n) => write!(
                f,
                "interface {:?}: no radio region is set (a human sets it first)",
                shown(n)
            ),
            Invalid::BadInstanceName(n) => write!(
                f,
                "instance name {:?} is not 1-32 of A-Z a-z 0-9 . _ -",
                shown(n)
            ),
            Invalid::BadBackbonePort(p) => write!(f, "backbone port {p} is not 1024-65535"),
            Invalid::BadCallsign => write!(f, "callsign is not 0-{MAX_CALLSIGN} of A-Z 0-9 / -"),
            Invalid::BadOrder(n) => write!(
                f,
                "network order entry {:?} is not a network id (or repeats)",
                shown(n)
            ),
        }
    }
}

impl std::error::Error for Invalid {}

/// A stanza name in its sanitized form (§11: "stanza names are sanitized"):
/// ASCII letters, digits, space, `-`, `_`, `.`, `(`, `)` kept, everything
/// else dropped (no `[`, `]`, `#`, `=`, quotes, `,` or `/`: rnsd's
/// ConfigObj reads them as syntax and `rnstatus` prints `/` as the address
/// separator), runs of spaces collapsed, trimmed, ≤ [`MAX_NAME_BYTES`].
/// `None` when nothing is left.
pub fn sanitize_name(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut space = false;
    for c in raw.chars() {
        let keep = c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '(' | ')');
        if keep {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            if out.len() >= MAX_NAME_BYTES {
                break;
            }
            out.push(c);
        } else if c == ' ' || c == '\t' {
            space = true;
        }
    }
    let out = out.trim().to_string();
    (!out.is_empty()).then_some(out)
}

/// `name` is already sanitized and not one of 111d's own stanza names.
pub fn valid_name(name: &str) -> bool {
    sanitize_name(name).as_deref() == Some(name) && !reserved(name)
}

fn reserved(name: &str) -> bool {
    RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(name))
}

/// An RNode `port` (§11, L5 §14): `^/dev/(serial/by-id/)?[A-Za-z0-9_.:-]+$`
/// (minus `.`/`..`), or `tcp://host:port` with an IP literal or DNS name.
pub fn valid_port(port: &str) -> bool {
    if port.starts_with("/dev/") {
        return valid_serial_path(port);
    }
    port.strip_prefix("tcp://")
        .is_some_and(|rest| !rest.is_empty() && !rest.contains('/') && Uplink::tcp(port).is_ok())
}

/// A Meshtastic uplink from its file form (`/dev/…` or `[tcp://]host:port`).
pub fn parse_uplink(s: &str) -> Result<Uplink, String> {
    let r = if s.starts_with('/') {
        Uplink::serial(s)
    } else {
        Uplink::tcp(s)
    };
    r.map_err(|e| e.to_string())
}

/// The file form of an uplink (`/dev/…` or `tcp://host:port`).
pub fn uplink_text(u: &Uplink) -> String {
    u.display()
}

/// An IP literal or a DNS host name (letters, digits, `-`, dots).
pub fn valid_host(host: &str) -> bool {
    if host.parse::<IpAddr>().is_ok() {
        return true;
    }
    !host.contains(':') && Uplink::tcp(&format!("{host}:1")).is_ok()
}

/// An IFAC network name: 1-32 of `A-Z a-z 0-9 . _ -`.
pub fn valid_netname(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn valid_instance(s: &str) -> bool {
    valid_netname(s)
}

/// A callsign: 0-16 of `A-Z a-z 0-9 / -` (empty = none).
pub fn valid_callsign(s: &str) -> bool {
    s.len() <= MAX_CALLSIGN
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-'))
}

fn valid_network_id(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=40).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b':' | b'_' | b'-')
        })
}

fn valid_board_text(s: &str) -> bool {
    board_name(s).as_deref() == Some(s)
}

fn non_empty_callsign(s: &str) -> bool {
    !s.is_empty() && valid_callsign(s)
}

/// A parameter's type and range.
#[derive(Debug, Clone, Copy)]
pub enum ParamKind {
    Int {
        min: i64,
        max: i64,
    },
    OneOf(&'static [i64]),
    Bool,
    /// A percentage 0-100 (integer or float).
    Percent,
    Text(fn(&str) -> bool),
}

/// One allowlisted parameter.
#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    pub key: &'static str,
    pub kind: ParamKind,
    /// Needed to render the stanza (a stored entry without it is left out
    /// of the render; a proposal without it is refused).
    pub required: bool,
}

const fn p(key: &'static str, kind: ParamKind, required: bool) -> ParamSpec {
    ParamSpec {
        key,
        kind,
        required,
    }
}

/// RNodeInterface parameters, in render order.
pub const RNODE_PARAMS: &[ParamSpec] = &[
    p(
        "frequency",
        ParamKind::Int {
            min: 100_000_000,
            max: 3_000_000_000,
        },
        true,
    ),
    p("bandwidth", ParamKind::OneOf(RNODE_BANDWIDTHS), true),
    p("txpower", ParamKind::Int { min: 0, max: 37 }, true),
    p("spreadingfactor", ParamKind::Int { min: 5, max: 12 }, true),
    p("codingrate", ParamKind::Int { min: 5, max: 8 }, true),
    p("flow_control", ParamKind::Bool, false),
    p(
        "id_interval",
        ParamKind::Int {
            min: 1,
            max: 86_400,
        },
        false,
    ),
    p("id_callsign", ParamKind::Text(non_empty_callsign), false),
    p("airtime_limit_short", ParamKind::Percent, false),
    p("airtime_limit_long", ParamKind::Percent, false),
];

/// TCPClientInterface parameters (the phone-relay client, §11), in render
/// order. `ifac` says the IFAC passphrase is in `secrets/`
/// (`config::client_ifac_path`); it never sits in this file.
pub const TCP_CLIENT_PARAMS: &[ParamSpec] = &[
    p("target_host", ParamKind::Text(valid_host), true),
    p(
        "target_port",
        ParamKind::Int {
            min: 1,
            max: 65_535,
        },
        true,
    ),
    p("ifac", ParamKind::Bool, false),
    p("network_name", ParamKind::Text(valid_netname), false),
];

/// Meshtastic binding parameters (the board's identify facts, L5 §14).
pub const MESHTASTIC_PARAMS: &[ParamSpec] = &[
    p(
        "node_num",
        ParamKind::Int {
            min: 1,
            max: u32::MAX as i64 - 1,
        },
        true,
    ),
    p("hw_model", ParamKind::Text(valid_board_text), false),
    p("long_name", ParamKind::Text(valid_board_text), false),
];

/// The allowlist of `kind`.
pub fn param_specs(kind: IfKind) -> &'static [ParamSpec] {
    match kind {
        IfKind::RNode => RNODE_PARAMS,
        IfKind::TcpClient => TCP_CLIENT_PARAMS,
        IfKind::Meshtastic => MESHTASTIC_PARAMS,
    }
}

/// Check one value against its spec (`Err` = why).
pub fn check_value(kind: ParamKind, v: &toml::Value) -> Result<(), &'static str> {
    match (kind, v) {
        (ParamKind::Int { min, max }, toml::Value::Integer(i)) => {
            if (min..=max).contains(i) {
                Ok(())
            } else {
                Err("is out of range")
            }
        }
        (ParamKind::OneOf(set), toml::Value::Integer(i)) => {
            if set.contains(i) {
                Ok(())
            } else {
                Err("is not a supported value")
            }
        }
        (ParamKind::Bool, toml::Value::Boolean(_)) => Ok(()),
        (ParamKind::Percent, toml::Value::Integer(i)) if (0..=100).contains(i) => Ok(()),
        (ParamKind::Percent, toml::Value::Float(x)) if (0.0..=100.0).contains(x) => Ok(()),
        (ParamKind::Percent, _) => Err("is not a percentage 0-100"),
        (ParamKind::Text(ok), toml::Value::String(s)) => {
            if ok(s) {
                Ok(())
            } else {
                Err("is not an accepted value")
            }
        }
        _ => Err("has the wrong type"),
    }
}

/// The first missing required parameter of `iface`, if any.
pub fn missing_param(iface: &Interface) -> Option<&'static str> {
    param_specs(iface.kind)
        .iter()
        .find(|s| s.required && !iface.params.contains_key(s.key))
        .map(|s| s.key)
}

impl InterfacesFile {
    /// Every rule a stored file must pass (module doc). Required
    /// parameters and the region are **not** required here: a stored RNode
    /// entry without them is left out of the render with a reason (§11);
    /// [`InterfacesFile::check_proposal`] refuses them for a new entry.
    pub fn validate(&self) -> Result<(), Invalid> {
        if self.schema != SCHEMA {
            return Err(Invalid::Schema(self.schema));
        }
        if let Some(r) = &self.radio.region {
            region(r).ok_or_else(|| Invalid::UnknownRegion(r.clone()))?;
        }
        if let Some(c) = &self.radio.callsign {
            if !valid_callsign(c) {
                return Err(Invalid::BadCallsign);
            }
        }
        if !valid_instance(&self.posture.instance_name) {
            return Err(Invalid::BadInstanceName(self.posture.instance_name.clone()));
        }
        let bp = self.posture.lan.backbone.port;
        if bp < 1024 {
            return Err(Invalid::BadBackbonePort(bp));
        }
        let mut seen = BTreeSet::new();
        if self.order.networks.len() > MAX_ORDER {
            return Err(Invalid::BadOrder(format!(
                "{} entries",
                self.order.networks.len()
            )));
        }
        for n in &self.order.networks {
            if !valid_network_id(n) || !seen.insert(n.clone()) {
                return Err(Invalid::BadOrder(n.clone()));
            }
        }
        if self.interfaces.len() > MAX_INTERFACES {
            return Err(Invalid::TooMany(self.interfaces.len()));
        }
        let mut names = BTreeSet::new();
        let mut ports: Vec<(String, String)> = Vec::new();
        let mut meshtastic = false;
        for i in &self.interfaces {
            check_entry(i)?;
            if !names.insert(i.name.to_ascii_lowercase()) {
                return Err(Invalid::DuplicateName(i.name.clone()));
            }
            if i.kind == IfKind::Meshtastic {
                if meshtastic {
                    return Err(Invalid::SecondMeshtastic(i.name.clone()));
                }
                meshtastic = true;
            }
            if let Some(port) = i.owned_port() {
                let key = port_key(&port);
                if let Some((_, by)) = ports.iter().find(|(k, _)| *k == key) {
                    return Err(Invalid::PortHeld {
                        name: i.name.clone(),
                        port,
                        by: by.clone(),
                    });
                }
                ports.push((key, i.name.clone()));
            }
        }
        Ok(())
    }

    /// [`InterfacesFile::validate`] plus the proposal rules (§11, §12
    /// "`rns.interface.upsert`"): every required parameter of `name`'s entry
    /// present and, for an RNode, a region set.
    pub fn check_proposal(&self, name: &str) -> Result<(), Invalid> {
        self.validate()?;
        let Some(i) = self.interface(name) else {
            return Ok(());
        };
        if let Some(key) = missing_param(i) {
            return Err(Invalid::MissingParam {
                name: i.name.clone(),
                key,
            });
        }
        if i.kind == IfKind::RNode && i.effective_region(&self.radio).is_none() {
            return Err(Invalid::NoRegion(i.name.clone()));
        }
        Ok(())
    }

    /// The entry named `name` (case-insensitive).
    pub fn interface(&self, name: &str) -> Option<&Interface> {
        self.interfaces
            .iter()
            .find(|i| i.name.eq_ignore_ascii_case(name))
    }

    /// Insert or replace (by name) one entry. Not validated: the caller
    /// validates the whole file before saving.
    pub fn upsert(&mut self, iface: Interface) {
        match self
            .interfaces
            .iter_mut()
            .find(|i| i.name.eq_ignore_ascii_case(&iface.name))
        {
            Some(slot) => *slot = iface,
            None => self.interfaces.push(iface),
        }
    }

    /// Remove the entry named `name`; true when one was removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.interfaces.len();
        self.interfaces
            .retain(|i| !i.name.eq_ignore_ascii_case(name));
        self.interfaces.len() != before
    }

    /// The Meshtastic binding held here (§11: "interfaces.toml wins"), in
    /// the shape the Layer 5 readers take: `(device id, binding)`.
    /// Re-validated through [`MeshtasticBinding::new`]; `None` when there is
    /// no `type = "meshtastic"` entry or it does not form a valid binding.
    pub fn meshtastic_binding(&self) -> Option<(String, MeshtasticBinding)> {
        let i = self
            .interfaces
            .iter()
            .find(|i| i.kind == IfKind::Meshtastic && i.owner == IfOwner::D111)?;
        let device = i.device.clone().filter(|d| valid_id(d))?;
        let uplink = parse_uplink(i.uplink.as_deref()?).ok()?;
        let node_num = u32::try_from(i.int("node_num")?).ok()?;
        let b = MeshtasticBinding::new(uplink, node_num, i.text("hw_model"), i.text("long_name"))?;
        Some((device, b))
    }

    /// Replace the Meshtastic entry with `binding` (`None` removes it). The
    /// entry keeps its name when one exists. Not saved: callers go through
    /// the spine's save, which mirrors into `devices.json`.
    pub fn set_meshtastic_binding(&mut self, binding: Option<(&str, &MeshtasticBinding)>) {
        let name = self
            .interfaces
            .iter()
            .find(|i| i.kind == IfKind::Meshtastic)
            .map(|i| i.name.clone());
        self.interfaces.retain(|i| i.kind != IfKind::Meshtastic);
        if let Some((device, b)) = binding {
            self.interfaces
                .push(meshtastic_entry(name.as_deref(), device, b));
        }
    }
}

/// The 111d entry for a Meshtastic binding (§11 shape).
pub fn meshtastic_entry(name: Option<&str>, device: &str, b: &MeshtasticBinding) -> Interface {
    let mut params = toml::Table::new();
    params.insert(
        "node_num".into(),
        toml::Value::Integer(i64::from(b.node_num)),
    );
    if let Some(h) = &b.hw_model {
        params.insert("hw_model".into(), toml::Value::String(h.clone()));
    }
    if let Some(l) = &b.long_name {
        params.insert("long_name".into(), toml::Value::String(l.clone()));
    }
    Interface {
        name: name.unwrap_or("Meshtastic board").to_string(),
        owner: IfOwner::D111,
        kind: IfKind::Meshtastic,
        device: Some(device.to_string()),
        port: None,
        region: None,
        uplink: Some(uplink_text(&b.uplink)),
        params,
    }
}

fn check_entry(i: &Interface) -> Result<(), Invalid> {
    let name = || i.name.clone();
    if !valid_name(&i.name) {
        return Err(if reserved(&i.name) {
            Invalid::ReservedName(name())
        } else {
            Invalid::BadName(name())
        });
    }
    if i.owner != i.kind.owner() {
        return Err(Invalid::OwnerKind(name()));
    }
    if let Some(d) = &i.device {
        if !valid_id(d) {
            return Err(Invalid::BadDevice(name()));
        }
    }
    if let Some(r) = &i.region {
        region(r).ok_or_else(|| Invalid::UnknownRegion(r.clone()))?;
    }
    let unexpected = |field| Invalid::UnexpectedField {
        name: name(),
        field,
    };
    match i.kind {
        IfKind::RNode => {
            if i.uplink.is_some() {
                return Err(unexpected("uplink"));
            }
            let port = i
                .port
                .as_deref()
                .ok_or_else(|| Invalid::MissingPort(name()))?;
            if !valid_port(port) {
                return Err(Invalid::BadPort {
                    name: name(),
                    port: port.to_string(),
                });
            }
        }
        IfKind::TcpClient => {
            if i.port.is_some() {
                return Err(unexpected("port"));
            }
            if i.uplink.is_some() {
                return Err(unexpected("uplink"));
            }
            if i.region.is_some() {
                return Err(unexpected("region"));
            }
        }
        IfKind::Meshtastic => {
            if i.port.is_some() {
                return Err(unexpected("port"));
            }
            if i.region.is_some() {
                return Err(unexpected("region"));
            }
            let (Some(up), Some(_)) = (i.uplink.as_deref(), i.device.as_deref()) else {
                return Err(Invalid::MissingUplink(name()));
            };
            parse_uplink(up).map_err(|why| Invalid::BadUplink { name: name(), why })?;
        }
    }
    let specs = param_specs(i.kind);
    for (key, value) in &i.params {
        let spec = specs
            .iter()
            .find(|s| s.key == key)
            .ok_or_else(|| Invalid::BadParam {
                name: name(),
                key: key.clone(),
                why: "is not a parameter of this type",
            })?;
        check_value(spec.kind, value).map_err(|why| Invalid::BadParam {
            name: name(),
            key: key.clone(),
            why,
        })?;
    }
    Ok(())
}

// ---------------------------------------------------------------- files

/// The file's path under the state dir.
pub fn path(state_dir: &Path) -> PathBuf {
    state_dir.join(FILE)
}

/// Why [`read`] returned no file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    Absent,
    Io(String),
    /// A symlink, a directory or another non-regular file.
    NotRegular,
    TooLarge(u64),
    Parse(String),
    /// A newer schema owns the file: left alone.
    Newer(u32),
    Invalid(Invalid),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Absent => f.write_str("absent"),
            LoadError::Io(e) => write!(f, "unreadable: {e}"),
            LoadError::NotRegular => f.write_str("not a regular file"),
            LoadError::TooLarge(n) => write!(f, "{n} bytes (at most {MAX_FILE_BYTES})"),
            LoadError::Parse(e) => write!(f, "not valid: {e}"),
            LoadError::Newer(v) => write!(f, "schema {v} is newer than {SCHEMA}"),
            LoadError::Invalid(e) => write!(f, "invalid: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Parse and validate file text (size checked by the caller).
pub fn parse(text: &str) -> Result<InterfacesFile, LoadError> {
    // the schema first, leniently: a newer file may carry keys this binary
    // does not know and must be left alone, not moved aside
    let table: toml::Table =
        toml::from_str(text).map_err(|e| LoadError::Parse(short_err(&e.to_string())))?;
    match table.get("schema").and_then(toml::Value::as_integer) {
        Some(v) if v > i64::from(SCHEMA) => {
            return Err(LoadError::Newer(u32::try_from(v).unwrap_or(u32::MAX)))
        }
        Some(_) => {}
        None => return Err(LoadError::Parse("no schema".into())),
    }
    let f: InterfacesFile =
        toml::from_str(text).map_err(|e| LoadError::Parse(short_err(&e.to_string())))?;
    f.validate().map_err(LoadError::Invalid)?;
    Ok(f)
}

fn short_err(e: &str) -> String {
    e.lines().next().unwrap_or("").chars().take(200).collect()
}

/// Read `path` (no side effects but a mode repair): `O_NOFOLLOW`, a regular
/// file, ≤ [`MAX_FILE_BYTES`] checked before the parse, then [`parse`]. A
/// mode other than 0600 is set back to 0600 through the open descriptor
/// (logged once per load).
pub fn read(path: &Path) -> Result<InterfacesFile, LoadError> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(LoadError::Absent),
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => return Err(LoadError::NotRegular),
        Err(e) => return Err(LoadError::Io(e.to_string())),
    };
    let meta = file.metadata().map_err(|e| LoadError::Io(e.to_string()))?;
    if !meta.is_file() {
        return Err(LoadError::NotRegular);
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(LoadError::TooLarge(meta.len()));
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o600 {
        eprintln!(
            "111 transport: {} had mode {mode:o}; set back to 600",
            path.display()
        );
        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    let mut buf = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| LoadError::Io(e.to_string()))?;
    if buf.len() as u64 > MAX_FILE_BYTES {
        return Err(LoadError::TooLarge(buf.len() as u64));
    }
    let text = String::from_utf8(buf).map_err(|_| LoadError::Parse("not UTF-8".into()))?;
    parse(&text)
}

/// Why [`save`] wrote nothing.
#[derive(Debug)]
pub enum SaveError {
    Invalid(Invalid),
    Io(anyhow::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::Invalid(e) => write!(f, "refused: {e}"),
            SaveError::Io(e) => write!(f, "cannot write {FILE}: {e:#}"),
        }
    }
}

impl std::error::Error for SaveError {}

/// The text [`save`] writes (header + TOML).
pub fn to_text(f: &InterfacesFile) -> Result<String, SaveError> {
    let body = toml::to_string(f).map_err(|e| SaveError::Io(anyhow::anyhow!("serialize: {e}")))?;
    Ok(format!("{HEADER}{body}"))
}

/// Validate and write `f` atomically, 0600. The state dir is created 0700
/// when absent (an existing one is shared with Layers 1-5 and keeps its mode).
pub fn save(state_dir: &Path, f: &InterfacesFile) -> Result<(), SaveError> {
    f.validate().map_err(SaveError::Invalid)?;
    let text = to_text(f)?;
    ensure_dir(state_dir, false).map_err(SaveError::Io)?;
    crate::util::fs::atomic_write(&path(state_dir), text.as_bytes(), 0o600).map_err(SaveError::Io)
}

/// Create `dir` (parents too) with mode 0700 when absent; `force` also sets
/// 0700 on an existing one. Refuses a symlink.
pub fn ensure_dir(dir: &Path, force: bool) -> anyhow::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_symlink() => {
            anyhow::bail!("{} is a symlink", dir.display())
        }
        Ok(m) if !m.is_dir() => anyhow::bail!("{} is not a directory", dir.display()),
        Ok(m) => {
            if force && m.permissions().mode() & 0o777 != 0o700 {
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        }
        Err(_) => {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
            // umask may have narrowed the mode further; set it exactly
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
            Ok(())
        }
    }
}

/// Move an invalid file aside (never deleted; the link itself, not a target).
pub fn move_aside(state_dir: &Path) -> Option<PathBuf> {
    let from = path(state_dir);
    for n in 1..=MOVE_ASIDE_TRIES {
        let to = state_dir.join(format!("{FILE}.invalid-{n}"));
        if std::fs::symlink_metadata(&to).is_ok() {
            continue;
        }
        return std::fs::rename(&from, &to).ok().map(|()| to);
    }
    None
}

/// The binding as the Layer 5 readers take it, read-only from the file
/// under `root` (no move-aside, no write). The one-line switch for the
/// readers of `devices.json` (§11, §18): `N/meshtastic/mod.rs` `reconcile`.
pub fn meshtastic_binding_at(root: &Path) -> Option<(String, MeshtasticBinding)> {
    read(&path(&root.join("state"))).ok()?.meshtastic_binding()
}

// ---------------------------------------------------------------- startup

/// What [`startup`] settled on.
#[derive(Debug, Clone, PartialEq)]
pub struct Startup {
    pub file: InterfacesFile,
    /// A newer schema owns the file: run on the defaults, never save.
    pub read_only: bool,
    /// The log lines it printed (tests assert them).
    pub notes: Vec<String>,
}

/// Load `state/interfaces.toml`, or create it (module doc): present ⇒ the
/// `devices.json` mirror is checked and a divergence resolved for this file
/// (logged); absent ⇒ [`migrate`] from `devices.json` and `rns/config`;
/// invalid ⇒ moved aside and the defaults written; newer ⇒ left alone.
pub fn startup(state_dir: &Path, devices_json: &Path, rns_config: &Path) -> Startup {
    let mut notes = Vec::new();
    let mut note = |s: String| {
        eprintln!("111 transport: {s}");
        notes.push(s);
    };
    let file_path = path(state_dir);
    let (file, read_only) = match read(&file_path) {
        Ok(f) => {
            if let Some(line) = crate::daemon::devices::store::mirror_meshtastic_binding(
                devices_json,
                f.meshtastic_binding()
                    .as_ref()
                    .map(|(id, b)| (id.as_str(), b)),
            ) {
                note(line);
            }
            (f, false)
        }
        Err(LoadError::Absent) => {
            let binding =
                crate::daemon::devices::store::Store::load(devices_json).meshtastic_binding();
            let rns = read_capped(rns_config);
            let m = migrate(binding, rns.as_deref());
            for n in m.notes {
                note(n);
            }
            match save(state_dir, &m.file) {
                Ok(()) => note(format!(
                    "{FILE} created from devices.json and rns/config ({} interface(s)); it is the \
                     source of truth from now on",
                    m.file.interfaces.len()
                )),
                Err(e) => note(format!(
                    "{FILE}: {e}; running on the migrated values in memory"
                )),
            }
            (m.file, false)
        }
        Err(LoadError::Newer(v)) => {
            note(format!(
                "{FILE} has schema {v} (newer than {SCHEMA}): left alone, running on the defaults"
            ));
            (InterfacesFile::default(), true)
        }
        Err(e) => {
            let moved = move_aside(state_dir);
            note(format!(
                "{FILE} is {e}; moved aside to {} and replaced by the defaults",
                moved
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(nowhere: the move failed)".into())
            ));
            let f = InterfacesFile::default();
            if moved.is_some() {
                if let Err(e) = save(state_dir, &f) {
                    note(format!("{FILE}: {e}"));
                }
                if let Some(line) =
                    crate::daemon::devices::store::mirror_meshtastic_binding(devices_json, None)
                {
                    note(line);
                }
            }
            (f, moved.is_none())
        }
    };
    Startup {
        file,
        read_only,
        notes,
    }
}

/// The rns config text for the migration (≤ 256 KiB, like `devices::rns`).
fn read_capped(p: &Path) -> Option<String> {
    let f = std::fs::File::open(p).ok()?;
    let mut buf = Vec::new();
    f.take(256 * 1024).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

// ---------------------------------------------------------------- migration

/// What [`migrate`] produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Migrated {
    pub file: InterfacesFile,
    /// One line per dropped or renamed item.
    pub notes: Vec<String>,
}

/// One `[[stanza]]` of an rns config: name and `key = value` pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stanza {
    pub name: String,
    pub pairs: Vec<(String, String)>,
}

impl Stanza {
    fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

fn unquote(v: &str) -> &str {
    let v = v.trim();
    v.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(v)
}

fn truthy(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "on" | "1" => Some(true),
        "no" | "false" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// The `[[…]]` stanzas under `[interfaces]` of a Reticulum config (the
/// ConfigObj form `devices::rns` reads; keys lowercased, values unquoted,
/// `#` comments dropped).
pub fn stanzas(text: &str) -> Vec<Stanza> {
    let mut out = Vec::new();
    let mut section = String::new();
    let mut cur: Option<Stanza> = None;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(inner) = line.strip_prefix("[[").and_then(|l| l.strip_suffix("]]")) {
            out.extend(cur.take());
            if section == "interfaces" {
                cur = Some(Stanza {
                    name: inner.trim().to_string(),
                    pairs: Vec::new(),
                });
            }
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            out.extend(cur.take());
            section = inner.trim().to_ascii_lowercase();
            continue;
        }
        if let (Some(s), Some((k, v))) = (cur.as_mut(), line.split_once('=')) {
            s.pairs
                .push((k.trim().to_ascii_lowercase(), unquote(v).to_string()));
        }
    }
    out.extend(cur.take());
    out
}

/// A config value as a typed TOML value for `spec` (`None`: not parseable).
fn typed(spec: &ParamSpec, raw: &str) -> Option<toml::Value> {
    let v = match spec.kind {
        ParamKind::Int { .. } | ParamKind::OneOf(_) => {
            toml::Value::Integer(raw.trim().parse::<i64>().ok()?)
        }
        ParamKind::Bool => toml::Value::Boolean(truthy(raw)?),
        ParamKind::Percent => match raw.trim().parse::<i64>() {
            Ok(i) => toml::Value::Integer(i),
            Err(_) => toml::Value::Float(raw.trim().parse::<f64>().ok()?),
        },
        ParamKind::Text(_) => toml::Value::String(raw.trim().to_string()),
    };
    check_value(spec.kind, &v).ok()?;
    Some(v)
}

/// Keys of an RNode stanza that are rendered by 111d, not parameters.
const STANZA_FIXED_KEYS: [&str; 5] = ["type", "enabled", "interface_enabled", "port", "name"];

/// Keys of an RNode stanza that protect or shape it — IFAC (Reticulum
/// reads `network_name`/`networkname`, `passphrase`/`pass_phrase`,
/// `ifac_size`) and the interface mode (`interface_mode`/`mode`). None is
/// on the allowlist, so a migrated copy would lose them and later render
/// onto an unprotected network with another mode: such a stanza is skipped
/// instead (B4 review F4).
const STANZA_GUARDED_KEYS: [&str; 7] = [
    "network_name",
    "networkname",
    "passphrase",
    "pass_phrase",
    "ifac_size",
    "interface_mode",
    "mode",
];

/// Build the first file (integration M7): the Meshtastic binding becomes an
/// `owner = "111d"` entry; every enabled `RNodeInterface` stanza of
/// `rns_config` an `owner = "rnsd"` entry (name sanitized and made unique,
/// port validated, parameters kept only when allowlisted and in range; no
/// region is inferred: a migrated RNode stays unrendered until a human sets
/// one). `RNodeMultiInterface` and other types are not migrated (logged);
/// nor is an RNode stanza carrying IFAC or interface-mode keys
/// ([`STANZA_GUARDED_KEYS`]; a weakened copy is never written, B4 review F4).
pub fn migrate(binding: Option<(String, MeshtasticBinding)>, rns_config: Option<&str>) -> Migrated {
    let mut file = InterfacesFile::default();
    let mut notes = Vec::new();
    if let Some((id, b)) = &binding {
        file.interfaces.push(meshtastic_entry(None, id, b));
    }
    let mut n = 0u32;
    for s in rns_config.map(stanzas).unwrap_or_default() {
        let kind = s.get("type").unwrap_or("");
        let enabled = s
            .get("enabled")
            .or_else(|| s.get("interface_enabled"))
            .and_then(truthy)
            .unwrap_or(false);
        if !enabled {
            continue;
        }
        if kind != "RNodeInterface" {
            if kind == "RNodeMultiInterface" {
                notes.push(format!(
                    "migration: stanza {:?} (RNodeMultiInterface) is not migrated; add it again in \
                     Settings → Layer 6",
                    shown(&s.name)
                ));
            }
            continue;
        }
        if let Some((k, _)) = s
            .pairs
            .iter()
            .find(|(k, _)| STANZA_GUARDED_KEYS.contains(&k.as_str()))
        {
            // the key is named, never its value (a passphrase)
            notes.push(format!(
                "migration: RNode stanza {:?} skipped: it sets {:?} (IFAC or interface mode), which \
                 a migrated copy cannot keep; add it again in Settings → Layer 6",
                shown(&s.name),
                shown(k)
            ));
            continue;
        }
        let Some(port) = s.get("port").filter(|p| valid_port(p)) else {
            notes.push(format!(
                "migration: RNode stanza {:?} skipped: its port is not an accepted /dev path or \
                 tcp://host:port",
                shown(&s.name)
            ));
            continue;
        };
        let key = port_key(port);
        if let Some(held) = file
            .interfaces
            .iter()
            .find(|i| i.owned_port().is_some_and(|p| port_key(&p) == key))
        {
            notes.push(format!(
                "migration: RNode stanza {:?} skipped: its port is owned by {:?}",
                shown(&s.name),
                held.name
            ));
            continue;
        }
        let taken = |name: &str| {
            reserved(name)
                || file
                    .interfaces
                    .iter()
                    .any(|i| i.name.eq_ignore_ascii_case(name))
        };
        let name = match sanitize_name(&s.name) {
            Some(nm) if !taken(&nm) => nm,
            _ => loop {
                n += 1;
                let nm = format!("RNode {n}");
                if !taken(&nm) {
                    notes.push(format!(
                        "migration: RNode stanza {:?} renamed {nm:?}",
                        shown(&s.name)
                    ));
                    break nm;
                }
            },
        };
        let mut params = toml::Table::new();
        for (k, v) in &s.pairs {
            if STANZA_FIXED_KEYS.contains(&k.as_str()) {
                continue;
            }
            match RNODE_PARAMS.iter().find(|spec| spec.key == k) {
                Some(spec) => match typed(spec, v) {
                    Some(tv) => {
                        params.insert(k.clone(), tv);
                    }
                    None => notes.push(format!(
                        "migration: {name:?}: parameter {:?} dropped (out of range)",
                        shown(k)
                    )),
                },
                None => notes.push(format!(
                    "migration: {name:?}: parameter {:?} dropped (not on the allowlist)",
                    shown(k)
                )),
            }
        }
        file.interfaces.push(Interface {
            name,
            owner: IfOwner::Rnsd,
            kind: IfKind::RNode,
            device: None,
            port: Some(port.to_string()),
            region: None,
            uplink: None,
            params,
        });
        if file.interfaces.len() >= MAX_INTERFACES {
            notes.push("migration: interface limit reached; later stanzas skipped".into());
            break;
        }
    }
    if let Err(e) = file.validate() {
        // fail closed: never write an invalid first file
        notes.push(format!(
            "migration: result refused ({e}); starting from the defaults"
        ));
        file = InterfacesFile::default();
    }
    Migrated { file, notes }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rnode(name: &str, port: &str) -> Interface {
        let mut params = toml::Table::new();
        params.insert("frequency".into(), toml::Value::Integer(869_525_000));
        params.insert("bandwidth".into(), toml::Value::Integer(125_000));
        params.insert("txpower".into(), toml::Value::Integer(14));
        params.insert("spreadingfactor".into(), toml::Value::Integer(8));
        params.insert("codingrate".into(), toml::Value::Integer(5));
        Interface {
            name: name.into(),
            owner: IfOwner::Rnsd,
            kind: IfKind::RNode,
            device: None,
            port: Some(port.into()),
            region: None,
            uplink: None,
            params,
        }
    }

    #[test]
    fn names_sanitize() {
        assert_eq!(sanitize_name("  Roof   RNode "), Some("Roof RNode".into()));
        assert_eq!(sanitize_name("a]b[c#d=e\"f,g/h"), Some("abcdefgh".into()));
        assert_eq!(sanitize_name("\u{202e}evil\nname"), Some("evilname".into()));
        assert_eq!(sanitize_name("]]"), None);
        assert_eq!(
            sanitize_name(&"x".repeat(100)).unwrap().len(),
            MAX_NAME_BYTES
        );
        assert!(valid_name("RNode 1"));
        assert!(!valid_name("RNode  1"));
        assert!(!valid_name("auto interface"), "reserved, any case");
    }

    #[test]
    fn ports_and_hosts() {
        assert!(valid_port("/dev/ttyACM0"));
        assert!(valid_port(
            "/dev/serial/by-id/usb-Silicon_Labs_CP2102-if00-port0"
        ));
        assert!(valid_port("tcp://10.0.0.5:4403"));
        assert!(valid_port("tcp://rnode.local:4403"));
        assert!(!valid_port("tcp://10.0.0.5"), "a port number is required");
        assert!(!valid_port("/dev/../etc/passwd"));
        assert!(!valid_port("/etc/shadow"));
        assert!(!valid_port("/dev/tty USB0"));
        assert!(!valid_port("ble://x"));
        assert!(valid_host("192.168.1.10"));
        assert!(valid_host("fe80::1"));
        assert!(valid_host("hub.local"));
        assert!(!valid_host("a b"));
        assert!(!valid_host("x,y"));
    }

    #[test]
    fn regions_known_and_unknown() {
        assert_eq!(region("EU_868").unwrap().start_hz, 869_400_000);
        assert_eq!(region("US").unwrap().power_limit_dbm, 30);
        assert!(region("UNSET").is_none());
        assert!(region("eu_868").is_none(), "ids are exact");
        assert_eq!(REGIONS.len(), 26);
    }

    #[test]
    fn defaults_round_trip_and_validate() {
        let f = InterfacesFile::default();
        f.validate().unwrap();
        let text = to_text(&f).unwrap();
        assert_eq!(parse(&text).unwrap(), f);
        assert!(f.posture.share_instance);
        assert_eq!(f.posture.lan.backbone.port, 4242);
        assert!(f.posture.enable_transport_for(Some(Category::Hub4A)));
        assert!(f.posture.enable_transport_for(Some(Category::Bridge3)));
        assert!(!f.posture.enable_transport_for(Some(Category::Node2A)));
        assert!(!f.posture.enable_transport_for(None));
    }

    #[test]
    fn spec_shape_parses() {
        let text = r#"
schema = 1
[radio]
region = "EU_868"
callsign = ""
[posture]
share_instance = true
instance_name = "111"
enable_transport = true
[posture.lan]
backbone = { listen = true, port = 4242, ifac = false }
stock_autointerface = false
propagation_node = false
[order]
networks = ["111", "lxmf", "meshtastic", "app:deltachat", "app:simplex"]
[[interface]]
name = "RNode 1"
owner = "rnsd"
type = "RNodeInterface"
device = "abcd1234abcd1234"
port = "/dev/serial/by-id/usb-X-if00"
region = "EU_868"
params = { frequency = 868100000, bandwidth = 125000, txpower = 14, spreadingfactor = 8, codingrate = 5 }
[[interface]]
name = "Bench radio"
owner = "111d"
type = "meshtastic"
device = "bbbb0000bbbb0000"
uplink = "/dev/serial/by-id/usb-Y-if00"
params = { node_num = 168430090 }
"#;
        let f = parse(text).unwrap();
        assert_eq!(f.interfaces.len(), 2);
        assert_eq!(f.radio.callsign(), None);
        let (id, b) = f.meshtastic_binding().unwrap();
        assert_eq!(id, "bbbb0000bbbb0000");
        assert_eq!(b.node_id, "!0a0a0a0a");
    }

    #[test]
    fn unknown_keys_and_types_are_refused() {
        let bad = [
            "schema = 1\nsurprise = 1\n",
            "schema = 1\n[posture]\nshare = true\n",
            "schema = 1\n[[interface]]\nname = \"X\"\nowner = \"rnsd\"\ntype = \"KISSInterface\"\nport = \"/dev/ttyACM0\"\n",
            "schema = 1\n[[interface]]\nname = \"X\"\nowner = \"rnsd\"\ntype = \"RNodeInterface\"\nport = \"/dev/ttyACM0\"\nextra = 1\n",
            "nothing = 1\n",
            "schema = 0\n",
            "not toml [",
        ];
        for t in bad {
            assert!(
                matches!(parse(t), Err(LoadError::Parse(_) | LoadError::Invalid(_))),
                "{t:?} → {:?}",
                parse(t)
            );
        }
        assert_eq!(
            parse("schema = 2\nfuture = { x = 1 }\n"),
            Err(LoadError::Newer(2))
        );
    }

    #[test]
    fn validation_refusals() {
        let mut f = InterfacesFile::default();
        f.interfaces.push(rnode("RNode 1", "/dev/ttyACM0"));
        f.validate().unwrap();
        // bad port
        let mut g = f.clone();
        g.interfaces[0].port = Some("/dev/../../etc/shadow".into());
        assert!(matches!(g.validate(), Err(Invalid::BadPort { .. })));
        // bad region
        let mut g = f.clone();
        g.interfaces[0].region = Some("MARS".into());
        assert_eq!(g.validate(), Err(Invalid::UnknownRegion("MARS".into())));
        let mut g = f.clone();
        g.radio.region = Some("XX_000".into());
        assert!(matches!(g.validate(), Err(Invalid::UnknownRegion(_))));
        // duplicate owner on a port
        let mut g = f.clone();
        g.interfaces.push(rnode("RNode 2", "/dev/ttyACM0"));
        assert!(matches!(g.validate(), Err(Invalid::PortHeld { .. })));
        // a 111d binding on rnsd's port
        let mut g = f.clone();
        let b =
            MeshtasticBinding::new(Uplink::serial("/dev/ttyACM0").unwrap(), 7, None, None).unwrap();
        g.set_meshtastic_binding(Some(("abcd1234abcd1234", &b)));
        assert!(matches!(g.validate(), Err(Invalid::PortHeld { .. })));
        // duplicate name (case-insensitive), reserved name, bad name
        let mut g = f.clone();
        g.interfaces.push(rnode("rnode 1", "/dev/ttyACM1"));
        assert!(matches!(g.validate(), Err(Invalid::DuplicateName(_))));
        let mut g = f.clone();
        g.interfaces[0].name = "Auto Interface".into();
        assert!(matches!(g.validate(), Err(Invalid::ReservedName(_))));
        let mut g = f.clone();
        g.interfaces[0].name = "X]]\n[reticulum".into();
        assert!(matches!(g.validate(), Err(Invalid::BadName(_))));
        // params: unknown key, out of range, wrong type
        let mut g = f.clone();
        g.interfaces[0]
            .params
            .insert("kiss_framing".into(), toml::Value::Boolean(true));
        assert!(matches!(g.validate(), Err(Invalid::BadParam { .. })));
        let mut g = f.clone();
        g.interfaces[0]
            .params
            .insert("spreadingfactor".into(), toml::Value::Integer(13));
        assert!(matches!(g.validate(), Err(Invalid::BadParam { .. })));
        let mut g = f.clone();
        g.interfaces[0]
            .params
            .insert("txpower".into(), toml::Value::String("14\n[x]".into()));
        assert!(matches!(g.validate(), Err(Invalid::BadParam { .. })));
        // owner/type mismatch
        let mut g = f.clone();
        g.interfaces[0].owner = IfOwner::D111;
        assert!(matches!(g.validate(), Err(Invalid::OwnerKind(_))));
        // a proposal needs a region and every required param
        assert_eq!(
            f.check_proposal("RNode 1"),
            Err(Invalid::NoRegion("RNode 1".into()))
        );
        let mut g = f.clone();
        g.radio.region = Some("EU_868".into());
        g.check_proposal("RNode 1").unwrap();
        g.interfaces[0].params.remove("codingrate");
        assert!(matches!(
            g.check_proposal("RNode 1"),
            Err(Invalid::MissingParam {
                key: "codingrate",
                ..
            })
        ));
        // posture fields
        let mut g = f.clone();
        g.posture.instance_name = "a b".into();
        assert!(matches!(g.validate(), Err(Invalid::BadInstanceName(_))));
        let mut g = f.clone();
        g.posture.lan.backbone.port = 80;
        assert_eq!(g.validate(), Err(Invalid::BadBackbonePort(80)));
        let mut g = f.clone();
        g.order.networks.push("111".into());
        assert!(matches!(g.validate(), Err(Invalid::BadOrder(_))));
        let mut g = f;
        g.radio.callsign = Some("W1AW\n".into());
        assert_eq!(g.validate(), Err(Invalid::BadCallsign));
    }

    #[test]
    fn stanzas_and_migration() {
        let cfg = "schema-version = 1\n[reticulum]\n  share_instance = no\n[interfaces]\n\
  [[Auto Interface]]\n    type = AutoInterface\n    enabled = yes\n\
  [[Roof RNode]]  # board\n    type = RNodeInterface\n    interface_enabled = True\n\
    port = /dev/serial/by-id/usb-X-if00\n    frequency = 867200000\n    bandwidth = 125000\n\
    txpower = 99\n    kiss_framing = yes\n\
  [[Off]]\n    type = RNodeInterface\n    enabled = no\n    port = /dev/ttyUSB3\n\
  [[Multi]]\n    type = RNodeMultiInterface\n    enabled = yes\n    port = /dev/ttyACM9\n\
  [[Bad port]]\n    type = RNodeInterface\n    enabled = yes\n    port = /etc/x\n\
  [[Auto Interface]]\n    type = RNodeInterface\n    enabled = yes\n    port = /dev/ttyACM2\n";
        let s = stanzas(cfg);
        assert_eq!(s.len(), 6);
        let m = migrate(None, Some(cfg));
        let names: Vec<_> = m.file.interfaces.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Roof RNode", "RNode 1"]);
        let roof = &m.file.interfaces[0];
        assert_eq!(roof.int("frequency"), Some(867_200_000));
        assert!(roof.params.get("txpower").is_none(), "out of range dropped");
        assert!(roof.params.get("kiss_framing").is_none(), "not allowlisted");
        assert!(roof.region.is_none(), "never inferred");
        let joined = m.notes.join("\n");
        assert!(joined.contains("RNodeMultiInterface"), "{joined}");
        assert!(joined.contains("Bad port"), "{joined}");
        assert!(joined.contains("txpower"), "{joined}");
        assert!(joined.contains("renamed \"RNode 1\""), "{joined}");
        m.file.validate().unwrap();
    }

    /// B4 review F4: an RNode stanza with IFAC or a mode is skipped with a
    /// note, never migrated as a weakened (unprotected, other-mode) copy.
    #[test]
    fn f4_migration_skips_ifac_and_mode_stanzas() {
        let base = "[interfaces]\n  [[Plain]]\n    type = RNodeInterface\n    enabled = yes\n    \
                    port = /dev/ttyACM0\n    frequency = 867200000\n";
        for (key, value) in [
            ("network_name", "secretnet"),
            ("networkname", "secretnet"),
            ("passphrase", "hunter2hunter2hunter2"),
            ("pass_phrase", "hunter2hunter2hunter2"),
            ("ifac_size", "16"),
            ("interface_mode", "access_point"),
            ("mode", "gateway"),
        ] {
            let cfg = format!(
                "{base}  [[Guarded]]\n    type = RNodeInterface\n    enabled = yes\n    \
                 port = /dev/ttyACM1\n    frequency = 867200000\n    {key} = {value}\n"
            );
            let m = migrate(None, Some(&cfg));
            let names: Vec<_> = m.file.interfaces.iter().map(|i| i.name.as_str()).collect();
            assert_eq!(
                names,
                ["Plain"],
                "{key}: the guarded stanza is not migrated"
            );
            let joined = m.notes.join("\n");
            assert!(joined.contains("\"Guarded\" skipped"), "{key}: {joined}");
            assert!(
                joined.contains("add it again in Settings → Layer 6"),
                "{joined}"
            );
            assert!(joined.contains(key), "{joined}");
            assert!(!joined.contains(value), "{key}: the value is never logged");
        }
    }
}
