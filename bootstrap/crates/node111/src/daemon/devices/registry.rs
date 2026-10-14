//! The registry tick (spec §7.1): every [`TICK`], detect hardware, read the
//! Reticulum config + rnsd liveness and the dashboard sessions, build
//! [`DeviceRecord`]s, merge the store (aliases, forget marks, remembered absent
//! devices), and publish a `devices` frame only after the new snapshot was
//! computed on [`DEBOUNCE_TICKS`] consecutive ticks (no flapping). The first
//! tick after start publishes at once (nothing to flap from) so
//! `GET /api/v1/devices` is correct right after boot. User mutations (alias,
//! forget) publish immediately.
//!
//! Identification (`match`, `firmware`): detection is sysfs-only — a serial
//! port is never opened (it can reset ESP32 boards). A USB VID:PID names the
//! serial chip, not the board or its firmware, so a LoRa-table hit is
//! `match.confidence: likely` ("Likely: Heltec WiFi LoRa 32 V3") with
//! `firmware.family: unknown`; anything else is `unknown`.
//!
//! What drives a device (`driver`):
//! - an enabled `RNodeInterface` stanza in `<root>/rns/config` whose `port` is
//!   this device's tty while the rnsd child runs → `{stack: reticulum,
//!   interface: <stanza>, live: true}`;
//! - an SPI-class radio (CH341 USB-to-SPI `1a86:5512`) while meshtasticd answers
//!   `127.0.0.1:4403` → `{stack: meshtastic, interface: meshtasticd, live: true}`
//!   (meshtasticd never drives a USB serial board);
//! - a companion → `{stack: dashboard, interface: session, live: true}`;
//! - otherwise `null`.
//!
//! Layer 5 §14 (Meshtastic): a LoRa radio with a serial port (or an SPI bridge
//! meshtasticd drives) offers the consented **`identify`** action while the
//! Meshtastic module says this node can run it ([`MeshtasticView`]: meshmud
//! and the presence reader installed; USB serial only off phones), nothing
//! else drives or holds its port, and no Reticulum stanza names it. The
//! identify result is persisted in `devices.json` ([`DeviceRegistry::record_identify`]):
//! `firmware` from the probe overrides the sysfs `unknown`, and the device that
//! holds the Meshtastic binding gets `service_id: meshtastic`, capability
//! `meshtastic-firmware` and `driver {stack: meshtastic, interface: meshmud,
//! live: <meshmud running>}` (so `stack_bound` / `stack_down` like any
//! stack). A device whose port another owner holds in the port-owner table
//! reads `port_busy`. The uplink a device would be identified on names its
//! device node under `/dev` (`/dev/serial/by-id/<name>` when that passes the
//! §14 path pattern, else `/dev/<tty>`), or `tcp://127.0.0.1:4403` for an SPI
//! bridge.
//!
//! States: `error` when the tty is unreadable (`no_port_access`; `access(R|W)`
//! denied — the unit grants `dialout`, so this is a real fault) or when a
//! driver that was live in the published snapshot stops while still configured
//! (`stack_down`, `driver.live: false`: rnsd stopped with the stanza in place,
//! or meshtasticd stopped answering for an SPI bridge); `needs_setup` when
//! present and nothing drives it (`driver == null`; `radio_unbound`, or
//! `unbound` for unknown hardware) — a stanza whose rnsd has not run yet is
//! unbound, not an error; `connected` when present and driven (companions:
//! seen in the last 10 min; Bluetooth adapters: always, `adapter_idle`, until
//! Layer 6 uses them); `disconnected` for remembered absent hardware and idle
//! companions. `since` = when the published `state` last changed (in memory; at
//! start it is the start time, or the remembered `last_seen` for absent devices).

use super::detect::{self, BtAdapter, PortAccess, Roots, Scan, SerialPort, UsbInfo};
use super::rns::{self, RnodeBinding};
use super::store::{Entry, MeshtasticBinding, Store, Summary};
use super::{
    board_base, cap_detail, clean, companion_name, device_id, kb_slug, Confidence, DeviceDriver,
    DeviceFirmware, DeviceKind, DeviceMatch, DeviceRecord, DeviceState, FirmwareFamily, Stack,
    Transport, MAX_ALIAS, MAX_FACT, MAX_PATH_FACT, MAX_USER_AGENT,
};
use crate::daemon::dashboard::api_v1::auth::SessionRec;
use crate::daemon::dashboard::ApiState;
use crate::daemon::meshtastic::meshmud::Uplink;
use crate::install::probe::{lora_table, RadioRule};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use time::OffsetDateTime;

pub const TICK: Duration = Duration::from_secs(5);
/// Consecutive identical ticks before a changed snapshot is published.
pub const DEBOUNCE_TICKS: u32 = 2;
/// Consecutive absent ticks before a forgotten device's record is dropped.
const FORGET_CLEAR_TICKS: u32 = 2;
/// meshtasticd's TCP API (drives SPI-class radios only).
pub const MESHTASTICD_PROBE: &str = "127.0.0.1:4403";
pub const RADIO_SERVICE: &str = "radio";

/// USB bridges whose radio sits on SPI (meshtasticd's Linux-native path), as
/// `(vid, pid, plain name)`. Not serial devices: no tty, transport `spi`.
const SPI_BRIDGES: &[(&str, &str, &str)] = &[("1a86", "5512", "CH341 USB-to-SPI LoRa adapter")];

const HARDWARE_ACTIONS: &[&str] = &["rename", "forget", "agent"];
const COMPANION_ACTIONS: &[&str] = &["rename", "forget"];
/// The consented identify step (Layer 5 §14; `POST /devices/{id}/identify`).
pub const IDENTIFY_ACTION: &str = "identify";
/// `service_id` of the device holding the Meshtastic binding.
pub const MESHTASTIC_SERVICE: &str = "meshtastic";
/// `driver.interface` of a meshmud-driven board.
pub const MESHMUD_INTERFACE: &str = "meshmud";

/// What the Meshtastic module (Layer 5 §14) tells the registry each tick. A
/// cheap cached snapshot: the registry reads it under its own lock.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeshtasticView {
    /// Identify can run here for a USB-serial board (meshmud + reader
    /// installed, Linux, not a phone).
    pub identify_usb: bool,
    /// Identify can run here for a TCP uplink (meshtasticd).
    pub identify_tcp: bool,
    /// meshmud is up (the bound board's driver is live).
    pub meshmud_running: bool,
    /// The port-owner table: `(normalized port, holder task)`.
    pub held: Vec<(String, String)>,
}

/// Why `identify` cannot run on a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetError {
    NotFound,
    /// The device does not offer `identify` (with a plain reason).
    NotOffered(&'static str),
    /// Its port has another owner (`409 port_busy`).
    Busy {
        port: String,
        owner: String,
    },
}

/// A device `identify` may run on.
#[derive(Debug, Clone, PartialEq)]
pub struct IdentifyTarget {
    pub id: String,
    pub uplink: Uplink,
    /// The record as published when the target was taken.
    pub record: DeviceRecord,
}

/// Where the registry looks (all injectable for tests).
#[derive(Debug, Clone)]
pub struct RegistryConfig {
    pub roots: Roots,
    /// Installed KB root (`<root>/current/share/kb`): `kb_entry` is set when
    /// `devices/<slug>.md` exists there.
    pub kb_dir: PathBuf,
    /// `<root>/state/devices.json`.
    pub store_path: PathBuf,
    /// `<root>/rns/config` (RNodeInterface bindings).
    pub rns_config: PathBuf,
    /// meshtasticd probe target (SPI-class radios only).
    pub probe_addr: SocketAddr,
    pub probe_timeout: Duration,
    /// A companion whose session was not seen for longer is `disconnected`.
    pub companion_idle: Duration,
}

impl RegistryConfig {
    pub fn production(layout: &crate::daemon::DaemonLayout) -> Self {
        RegistryConfig {
            roots: Roots::default(),
            kb_dir: layout.current_release().join("share/kb"),
            store_path: layout.state_dir().join(super::store::FILE),
            rns_config: crate::daemon::transport::config::config_path(layout),
            probe_addr: MESHTASTICD_PROBE.parse().expect("static addr"),
            probe_timeout: Duration::from_secs(1),
            companion_idle: Duration::from_secs(600),
        }
    }
}

/// Error for the mutation API (`404` at the route).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotFound;

/// Outcome of the meshtasticd probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Open,
    Refused,
    Timeout,
    Failed,
    /// No SPI-class radio present, so nothing was probed.
    Skipped,
}

impl Probe {
    fn as_str(self) -> &'static str {
        match self {
            Probe::Open => "open",
            Probe::Refused => "refused",
            Probe::Timeout => "timeout",
            Probe::Failed => "error",
            Probe::Skipped => "skipped",
        }
    }
}

/// Is meshtasticd answering? For a local address this is a LISTEN check in
/// `/proc/net/tcp{,6}` (`services::probe::listening_under`): meshtasticd
/// serves one API client, so connecting every tick would evict the real one.
/// A connect probe (bounded) runs only for a non-local address or when procfs
/// is unreadable.
pub async fn probe(addr: SocketAddr, timeout: Duration) -> Probe {
    let ip = addr.ip().to_canonical();
    if ip.is_loopback() || ip.is_unspecified() {
        if let Some(listening) = crate::daemon::services::probe::listening_under(
            std::path::Path::new("/proc/net"),
            addr.port(),
        ) {
            return if listening {
                Probe::Open
            } else {
                Probe::Refused
            };
        }
    }
    match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await {
        Ok(Ok(_)) => Probe::Open,
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => Probe::Refused,
        Ok(Err(_)) => Probe::Failed,
        Err(_) => Probe::Timeout,
    }
}

/// Everything one tick observed (besides sessions).
#[derive(Debug, Clone)]
pub struct TickFacts {
    pub scan: Scan,
    pub rnode: Vec<RnodeBinding>,
    pub rnsd_running: bool,
    pub probe: Probe,
}

/// One device seen this tick (before alias/store merge).
struct Detected {
    id: String,
    record: DeviceRecord,
    summary: Summary,
    /// companion: the session's `created` (first_seen); hardware: None (store).
    first_seen_hint: Option<String>,
    /// companion: the session hash (for revoke on forget).
    session_hash: Option<String>,
    /// The stack configured for this device even though it is not live now
    /// (an rns stanza while rnsd is down; meshtasticd for an SPI bridge).
    hint: Option<Hint>,
    /// Layer 5 §14: the uplink an identify would open.
    identify_port: Option<Uplink>,
}

/// `(stack, interface)` a device is configured for.
type Hint = (Stack, Option<String>);

/// A hardware record before id/summary bookkeeping.
struct Draft {
    kind: DeviceKind,
    stable_key: String,
    id_basis: &'static str,
    name: String,
    transport: Transport,
    state: (DeviceState, &'static str, String),
    matched: DeviceMatch,
    firmware: Option<DeviceFirmware>,
    driver: Option<DeviceDriver>,
    hint: Option<Hint>,
    board: Option<String>,
    identity: Map<String, Value>,
    detail: Map<String, Value>,
    identify_port: Option<Uplink>,
}

struct Inner {
    store: Store,
    published: Option<Vec<DeviceRecord>>,
    published_fp: String,
    pending: Option<(String, u32)>,
    absent_ticks: HashMap<String, u32>,
    /// companion id → session hash (from the last tick).
    sessions: HashMap<String, String>,
    /// present device id → the uplink an identify would open (last tick).
    identify_ports: HashMap<String, Uplink>,
}

type RunningFn = Arc<dyn Fn() -> bool + Send + Sync>;
type ViewFn = Arc<dyn Fn() -> MeshtasticView + Send + Sync>;

pub struct DeviceRegistry {
    cfg: RegistryConfig,
    rules: Vec<RadioRule>,
    rnsd_running: RwLock<Option<RunningFn>>,
    meshtastic: RwLock<Option<ViewFn>>,
    inner: Mutex<Inner>,
}

fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

fn parse_rfc3339(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
}

fn put(m: &mut Map<String, Value>, k: &str, v: Option<impl Into<Value>>) {
    if let Some(v) = v {
        m.insert(k.into(), v.into());
    }
}

fn rule_matches(rule: &RadioRule, u: &UsbInfo) -> bool {
    if rule.vid != u.vendor_id || (rule.pid != "*" && rule.pid != u.product_id) {
        return false;
    }
    match &rule.product_contains {
        Some(want) => u
            .product
            .as_deref()
            .is_some_and(|p| p.to_lowercase().contains(want.as_str())),
        None => true,
    }
}

fn spi_bridge(u: &UsbInfo) -> Option<&'static str> {
    SPI_BRIDGES
        .iter()
        .find(|(v, p, _)| *v == u.vendor_id && *p == u.product_id)
        .map(|(_, _, name)| *name)
}

fn usb_detail(m: &mut Map<String, Value>, u: &UsbInfo) {
    put(m, "usb_bus_id", Some(u.bus_id.clone()));
    put(m, "usb_vendor_id", Some(u.vendor_id.clone()));
    put(m, "usb_product_id", Some(u.product_id.clone()));
    put(m, "usb_manufacturer", u.manufacturer.clone());
    put(m, "usb_product", u.product.clone());
    put(m, "usb_serial", u.serial.clone());
    put(m, "usb_class", u.device_class.clone());
    if !u.interface_classes.is_empty() {
        put(
            m,
            "usb_interface_classes",
            Some(u.interface_classes.join(",")),
        );
    }
}

fn usb_identity(m: &mut Map<String, Value>, u: &UsbInfo) {
    put(m, "vendor_id", Some(u.vendor_id.clone()));
    put(m, "product_id", Some(u.product_id.clone()));
    put(m, "product", u.product.clone());
    put(m, "serial", u.serial.clone());
}

/// `vendor:product:serial`, else `vendor:product:<bus id>`.
fn usb_key(u: &UsbInfo) -> (String, &'static str) {
    match &u.serial {
        Some(s) => (
            format!("{}:{}:{s}", u.vendor_id, u.product_id),
            "usb-serial-number",
        ),
        None => (
            format!("{}:{}:{}", u.vendor_id, u.product_id, u.bus_id),
            "usb-sysfs-path",
        ),
    }
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// The uplink an identify would open for a serial port: the by-id node when
/// its name passes the §14 pattern, else the kernel tty (always under the
/// real `/dev`, whatever tree detection read).
fn serial_uplink(tty: &str, by_id: Option<&str>) -> Option<Uplink> {
    by_id
        .and_then(|b| Uplink::serial(&format!("/dev/serial/by-id/{b}")).ok())
        .or_else(|| Uplink::serial(&format!("/dev/{tty}")).ok())
}

fn push_unique(v: &mut Vec<String>, s: &str) {
    if !v.iter().any(|x| x == s) {
        v.push(s.to_string());
    }
}

/// Layer 5 §14: merge the persisted identify result and the Meshtastic
/// binding into a hardware record, and decide whether `identify` is offered.
/// Pure (the port-table lookup normalizes names through
/// `daemon::ports::port_key`).
fn apply_meshtastic(
    r: &mut DeviceRecord,
    summary: Option<&mut Summary>,
    entry: Option<&Entry>,
    view: &MeshtasticView,
    identify_port: Option<&Uplink>,
    rns_hint: bool,
) {
    if matches!(r.kind, DeviceKind::Companion | DeviceKind::Bluetooth) {
        return;
    }
    let firmware = entry.and_then(|e| e.firmware.clone());
    let binding: Option<&MeshtasticBinding> = entry.and_then(|e| e.meshtastic.as_ref());
    if let Some(fw) = &firmware {
        r.firmware = Some(fw.clone());
    }
    let meshtastic_fw = firmware
        .as_ref()
        .is_some_and(|f| f.family == FirmwareFamily::Meshtastic);
    if meshtastic_fw {
        push_unique(&mut r.capabilities, "meshtastic-firmware");
    }
    let mut actions = strs(HARDWARE_ACTIONS);
    if let Some(b) = binding {
        r.service_id = Some(MESHTASTIC_SERVICE.to_string());
        r.detail
            .insert("meshtastic_uplink".into(), b.uplink.display().into());
        r.detail
            .insert("meshtastic_node_id".into(), b.node_id.clone().into());
        r.detail = cap_detail(std::mem::take(&mut r.detail));
        if r.state.present() && r.reason_code != "no_port_access" {
            let driver = DeviceDriver {
                stack: Stack::Meshtastic,
                interface: Some(MESHMUD_INTERFACE.into()),
                live: view.meshmud_running,
            };
            let (state, code, reason) = driven_state(Some(&driver), ("", ""));
            r.state = state;
            r.reason_code = code.into();
            r.reason = reason;
            r.driver = Some(driver);
        }
    } else if r.kind == DeviceKind::Radio && r.state.present() {
        if let Some(up) = identify_port {
            let key = crate::daemon::ports::port_key(&up.port_name());
            let self_task = format!("identify:{}", r.id);
            let holder = view
                .held
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, t)| t.clone());
            let driver_ok = match up {
                // a USB board: nothing may drive it yet
                Uplink::Serial { .. } => r.driver.is_none() && view.identify_usb,
                // an SPI bridge: identify goes through the meshtasticd that drives it
                Uplink::Tcp { .. } => {
                    view.identify_tcp
                        && r.driver.as_ref().is_some_and(|d| {
                            d.stack == Stack::Meshtastic
                                && d.live
                                && d.interface.as_deref() == Some("meshtasticd")
                        })
                }
            };
            match holder {
                Some(task) if task != self_task => {
                    if r.state == DeviceState::NeedsSetup {
                        r.reason_code = "port_busy".into();
                        r.reason = format!("Its port is in use by {task}.");
                    }
                }
                Some(_) => {} // being identified right now
                None if driver_ok && !rns_hint && r.reason_code != "no_port_access" => {
                    actions.push(IDENTIFY_ACTION.to_string());
                }
                None => {}
            }
        }
    }
    r.actions = actions;
    if let Some(s) = summary {
        s.firmware = r.firmware.clone();
        s.capabilities = r.capabilities.clone();
        s.service_id = r.service_id.clone();
    }
}

fn no_port_access() -> (DeviceState, &'static str, String) {
    (
        DeviceState::Error,
        "no_port_access",
        "Plugged in, but 111 cannot open its serial port (permission denied).".into(),
    )
}

/// State from the driver (after the port-access check).
fn driven_state(
    driver: Option<&DeviceDriver>,
    unbound: (&'static str, &str),
) -> (DeviceState, &'static str, String) {
    match driver {
        Some(d) if !d.live => (
            DeviceState::Error,
            "stack_down",
            match (&d.stack, &d.interface) {
                (Stack::Reticulum, Some(i)) => format!(
                    "Its Reticulum interface \u{201c}{i}\u{201d} is configured, but rnsd is not running."
                ),
                (Stack::Meshtastic, Some(i)) if i == MESHMUD_INTERFACE => {
                    "meshmud drives it and is not running.".into()
                }
                (Stack::Meshtastic, _) => "meshtasticd drove it and stopped answering.".into(),
                _ => "The stack that drives it is not running.".into(),
            },
        ),
        Some(d) => (
            DeviceState::Connected,
            "stack_bound",
            match (&d.stack, &d.interface) {
                (Stack::Reticulum, Some(i)) => {
                    format!("Driven by the mesh transport (Reticulum interface \u{201c}{i}\u{201d}).")
                }
                (Stack::Meshtastic, Some(i)) if i == MESHMUD_INTERFACE => {
                    "Driven by meshmud (Meshtastic).".into()
                }
                (Stack::Meshtastic, _) => "Driven by meshtasticd.".into(),
                _ => "Driven by a running stack.".into(),
            },
        ),
        None => (DeviceState::NeedsSetup, unbound.0, unbound.1.to_string()),
    }
}

/// Present-first, then kind, then display name, then id.
fn sort_records(records: &mut [DeviceRecord]) {
    records.sort_by(|a, b| {
        let key = |r: &DeviceRecord| {
            (
                !r.state.present(),
                r.kind,
                r.alias.as_deref().unwrap_or(&r.name).to_lowercase(),
                r.id.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
}

/// Snapshot identity for the debounce: everything except the volatile
/// timestamps (`last_seen` moves every tick; `since` is derived).
fn fingerprint(records: &[DeviceRecord]) -> String {
    let stripped: Vec<Value> = records
        .iter()
        .map(|r| {
            let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
            if let Some(o) = v.as_object_mut() {
                o.remove("last_seen");
                o.remove("since");
            }
            v
        })
        .collect();
    Value::Array(stripped).to_string()
}

impl DeviceRegistry {
    pub fn new(cfg: RegistryConfig) -> Self {
        let store = Store::load(&cfg.store_path);
        DeviceRegistry {
            rules: lora_table(),
            rnsd_running: RwLock::new(None),
            meshtastic: RwLock::new(None),
            inner: Mutex::new(Inner {
                store,
                published: None,
                published_fp: String::new(),
                pending: None,
                absent_ticks: HashMap::new(),
                sessions: HashMap::new(),
                identify_ports: HashMap::new(),
            }),
            cfg,
        }
    }

    pub fn config(&self) -> &RegistryConfig {
        &self.cfg
    }

    /// How to tell whether the rnsd child is running (daemon: the transport
    /// handle; default: never).
    pub fn set_rnsd_running(&self, f: impl Fn() -> bool + Send + Sync + 'static) {
        *self.rnsd_running.write().unwrap() = Some(Arc::new(f));
    }

    fn rnsd_running(&self) -> bool {
        let f = self.rnsd_running.read().unwrap().clone();
        f.is_some_and(|f| f())
    }

    /// Install the Meshtastic module's view (Layer 5 §14; default: identify
    /// is never offered and no board is meshmud-driven).
    pub fn set_meshtastic_view(&self, f: impl Fn() -> MeshtasticView + Send + Sync + 'static) {
        *self.meshtastic.write().unwrap() = Some(Arc::new(f));
    }

    fn meshtastic_view(&self) -> MeshtasticView {
        let f = self.meshtastic.read().unwrap().clone();
        f.map(|f| f()).unwrap_or_default()
    }

    fn rule_for(&self, u: &UsbInfo) -> Option<&RadioRule> {
        self.rules.iter().find(|r| rule_matches(r, u))
    }

    fn kb_entry(&self, board: &str) -> Option<String> {
        let slug = kb_slug(board);
        if slug.is_empty() {
            return None;
        }
        let rel = format!("devices/{slug}.md");
        self.cfg.kb_dir.join(&rel).is_file().then_some(rel)
    }

    // ------------------------------------------------------------ builders

    fn finish(&self, d: Draft, now: &str) -> Detected {
        let id = device_id(d.kind, &d.stable_key);
        let mut detail = d.detail;
        detail.insert("id_basis".into(), d.id_basis.into());
        let (capabilities, service_id) = match d.kind {
            DeviceKind::Radio => (strs(&["lora"]), Some(RADIO_SERVICE.to_string())),
            DeviceKind::Bluetooth => (strs(&["ble"]), None),
            _ => (Vec::new(), None),
        };
        let kb_entry = d.board.as_deref().and_then(|b| self.kb_entry(b));
        let (state, reason_code, reason) = d.state;
        let record = DeviceRecord {
            id: id.clone(),
            kind: d.kind,
            alias: None,
            name: d.name,
            state,
            transport: d.transport,
            first_seen: now.to_string(),
            last_seen: now.to_string(),
            since: now.to_string(),
            reason_code: reason_code.into(),
            reason,
            capabilities: capabilities.clone(),
            service_id: service_id.clone(),
            kb_entry: kb_entry.clone(),
            matched: d.matched.clone(),
            firmware: d.firmware.clone(),
            driver: d.driver,
            role: None,
            identity: d.identity.clone(),
            actions: strs(HARDWARE_ACTIONS),
            detail: cap_detail(detail),
        };
        let summary = Summary {
            kind: d.kind,
            name: record.name.clone(),
            transport: d.transport,
            capabilities,
            service_id,
            kb_entry,
            identity: d.identity,
            id_basis: d.id_basis.into(),
            matched: Some(d.matched),
            firmware: d.firmware,
        };
        Detected {
            id,
            record,
            summary,
            first_seen_hint: None,
            session_hash: None,
            hint: d.hint,
            identify_port: d.identify_port,
        }
    }

    /// The RNode stanza for `tty`: a live driver while rnsd runs, else only a
    /// hint (configured, not live).
    fn rnode_driver(
        facts: &TickFacts,
        tty: &str,
        detail: &mut Map<String, Value>,
    ) -> (Option<DeviceDriver>, Option<Hint>) {
        let Some(b) = facts.rnode.iter().find(|b| b.tty.as_deref() == Some(tty)) else {
            return (None, None);
        };
        put(detail, "rns_interface", Some(b.name.clone()));
        put(detail, "rns_port", Some(b.port.clone()));
        detail.insert("rnsd_running".into(), facts.rnsd_running.into());
        let driver = facts.rnsd_running.then(|| DeviceDriver {
            stack: Stack::Reticulum,
            interface: Some(b.name.clone()),
            live: true,
        });
        (driver, Some((Stack::Reticulum, Some(b.name.clone()))))
    }

    fn serial_record(&self, port: &SerialPort, facts: &TickFacts, now: &str) -> Detected {
        let u = port.usb.as_ref();
        let rule = u.and_then(|u| self.rule_for(u));
        let (stable_key, id_basis) = match (&port.by_id, u) {
            (Some(b), _) => (b.clone(), "serial-by-id"),
            (None, Some(u)) => usb_key(u),
            (None, None) => (format!("tty:{}", port.tty), "tty-name"),
        };
        let tty_path = self.cfg.roots.dev.join(&port.tty);
        let tty_display = clean(&tty_path.to_string_lossy(), MAX_PATH_FACT);
        let by_id_display = port.by_id.as_ref().and_then(|b| {
            clean(
                &self
                    .cfg
                    .roots
                    .dev
                    .join("serial/by-id")
                    .join(b)
                    .to_string_lossy(),
                MAX_PATH_FACT,
            )
        });
        let access: PortAccess = detect::port_access(&tty_path);

        let mut identity = Map::new();
        if let Some(u) = u {
            usb_identity(&mut identity, u);
        }
        put(&mut identity, "port", tty_display.clone());
        put(&mut identity, "by_id", by_id_display.clone());

        let mut detail = Map::new();
        put(&mut detail, "tty", tty_display);
        put(&mut detail, "serial_by_id", by_id_display);
        put(&mut detail, "port_access", Some(access.access));
        put(&mut detail, "port_mode", access.mode.clone());
        put(&mut detail, "port_group", access.group.clone());
        if let Some(u) = u {
            usb_detail(&mut detail, u);
        }
        let (driver, hint) = Self::rnode_driver(facts, &port.tty, &mut detail);
        let ids = u.map(|u| format!("{}:{}", u.vendor_id, u.product_id));

        let (kind, name, matched, firmware, unbound) = match rule {
            Some(r) => {
                put(&mut detail, "lora_table_match", Some(r.name.clone()));
                (
                    DeviceKind::Radio,
                    format!("Likely: {}", board_base(&r.name)),
                    DeviceMatch {
                        confidence: Confidence::Likely,
                        rule: format!("lora-table {}", ids.clone().unwrap_or_default()),
                    },
                    Some(DeviceFirmware::unknown()),
                    (
                        "radio_unbound",
                        "Probably a LoRa board; nothing drives it yet and its firmware is unknown.",
                    ),
                )
            }
            None => (
                DeviceKind::Unknown,
                match u.and_then(|u| u.product.as_deref()) {
                    Some(p) => format!("USB serial device ({p})"),
                    None if u.is_some() => "USB serial device".into(),
                    None => "Serial device".into(),
                },
                DeviceMatch {
                    confidence: Confidence::Unknown,
                    rule: match &ids {
                        Some(i) => format!("usb-ids {i}"),
                        None => "serial-port".into(),
                    },
                },
                Some(DeviceFirmware::unknown()),
                (
                    "unbound",
                    "Plugged in; not identified yet and nothing drives it.",
                ),
            ),
        };
        let configured;
        let unbound = match (&driver, &hint) {
            (None, Some((_, Some(i)))) => {
                configured = format!(
                    "Its Reticulum interface \u{201c}{i}\u{201d} is configured, but rnsd is not running."
                );
                (unbound.0, configured.as_str())
            }
            _ => unbound,
        };
        let state = if access.access == "denied" {
            no_port_access()
        } else {
            driven_state(driver.as_ref(), unbound)
        };
        self.finish(
            Draft {
                kind,
                stable_key,
                id_basis,
                name,
                transport: Transport::UsbSerial,
                state,
                matched,
                firmware,
                driver,
                hint,
                board: rule.map(|r| r.name.clone()),
                identity,
                detail,
                identify_port: serial_uplink(&port.tty, port.by_id.as_deref()),
            },
            now,
        )
    }

    fn usb_record(&self, u: &UsbInfo, facts: &TickFacts, now: &str) -> Detected {
        let (stable_key, id_basis) = usb_key(u);
        let mut identity = Map::new();
        usb_identity(&mut identity, u);
        let mut detail = Map::new();
        usb_detail(&mut detail, u);
        let ids = format!("{}:{}", u.vendor_id, u.product_id);
        let likely = |rule: String| DeviceMatch {
            confidence: Confidence::Likely,
            rule,
        };
        let unbound_radio = (
            "radio_unbound",
            "Probably a LoRa board; nothing drives it yet and its firmware is unknown.",
        );
        let (kind, name, transport, matched, firmware, driver, hint, board, unbound) =
            if let Some(spi) = spi_bridge(u) {
                put(
                    &mut detail,
                    "service_probe_addr",
                    Some(self.cfg.probe_addr.to_string()),
                );
                put(&mut detail, "service_probe", Some(facts.probe.as_str()));
                let driver = (facts.probe == Probe::Open).then(|| DeviceDriver {
                    stack: Stack::Meshtastic,
                    interface: Some("meshtasticd".into()),
                    live: true,
                });
                (
                    DeviceKind::Radio,
                    format!("Likely: {spi}"),
                    Transport::Spi,
                    likely(format!("spi-bridge {ids}")),
                    Some(DeviceFirmware::unknown()),
                    driver,
                    Some((Stack::Meshtastic, Some("meshtasticd".to_string()))),
                    None,
                    unbound_radio,
                )
            } else if let Some(r) = self.rule_for(u) {
                put(&mut detail, "lora_table_match", Some(r.name.clone()));
                (
                    DeviceKind::Radio,
                    format!("Likely: {}", board_base(&r.name)),
                    Transport::UsbSerial,
                    likely(format!("lora-table {ids}")),
                    Some(DeviceFirmware::unknown()),
                    None,
                    None,
                    Some(r.name.clone()),
                    unbound_radio,
                )
            } else {
                (
                    DeviceKind::Unknown,
                    match &u.product {
                        Some(p) => format!("USB device ({p})"),
                        None => "USB device".into(),
                    },
                    Transport::Unknown,
                    DeviceMatch {
                        confidence: Confidence::Unknown,
                        rule: format!("usb-ids {ids}"),
                    },
                    Some(DeviceFirmware::unknown()),
                    None,
                    None,
                    None,
                    (
                        "unbound",
                        "Plugged in; not identified yet and nothing drives it.",
                    ),
                )
            };
        let state = driven_state(driver.as_ref(), unbound);
        let identify_port = (transport == Transport::Spi).then(Uplink::meshtasticd);
        self.finish(
            Draft {
                kind,
                stable_key,
                id_basis,
                name,
                transport,
                state,
                matched,
                firmware,
                driver,
                hint,
                board,
                identity,
                detail,
                identify_port,
            },
            now,
        )
    }

    fn bluetooth_record(&self, bt: &BtAdapter, now: &str) -> Detected {
        let (stable_key, id_basis) = match &bt.address {
            Some(mac) => (mac.clone(), "mac"),
            None => (bt.hci.clone(), "adapter-name"),
        };
        let mut identity = Map::new();
        put(&mut identity, "mac", bt.address.clone());
        put(&mut identity, "adapter", Some(bt.hci.clone()));
        let mut detail = Map::new();
        put(&mut detail, "adapter", Some(bt.hci.clone()));
        put(&mut detail, "address", bt.address.clone());
        put(&mut detail, "adapter_name", bt.name.clone());
        if let Some(u) = &bt.usb {
            put(&mut identity, "vendor_id", Some(u.vendor_id.clone()));
            put(&mut identity, "product_id", Some(u.product_id.clone()));
            put(&mut identity, "product", u.product.clone());
            usb_detail(&mut detail, u);
        }
        let name = match bt.usb.as_ref().and_then(|u| u.product.as_deref()) {
            Some(p) => format!("Bluetooth adapter ({p})"),
            None => "Bluetooth adapter".into(),
        };
        self.finish(
            Draft {
                kind: DeviceKind::Bluetooth,
                stable_key,
                id_basis,
                name,
                transport: Transport::Bluetooth,
                // nothing to set up today: an idle adapter is not `needs_setup`
                state: (
                    DeviceState::Connected,
                    "adapter_idle",
                    "Present. Nothing on this node uses Bluetooth yet (arrives with Layer 6)."
                        .into(),
                ),
                matched: DeviceMatch {
                    confidence: Confidence::Exact,
                    rule: "sysfs bluetooth".into(),
                },
                firmware: None,
                driver: None,
                hint: None,
                board: None,
                identity,
                detail,
                identify_port: None,
            },
            now,
        )
    }

    fn companion_record(&self, s: &SessionRec, now: OffsetDateTime) -> Detected {
        let id = device_id(DeviceKind::Companion, &s.hash);
        let ua = s
            .user_agent
            .as_deref()
            .and_then(|u| clean(u, MAX_USER_AGENT));
        let ip = clean(&s.peer, MAX_FACT);
        let idle = parse_rfc3339(&s.last_seen)
            .is_none_or(|t| (now - t).whole_seconds() > self.cfg.companion_idle.as_secs() as i64);
        let (state, reason_code, reason) = if idle {
            (
                DeviceState::Disconnected,
                "session_idle",
                "Paired with the control panel; not seen in the last 10 minutes.",
            )
        } else {
            (
                DeviceState::Connected,
                "session_active",
                "Paired with the control panel; active in the last 10 minutes.",
            )
        };
        let mut identity = Map::new();
        identity.insert("ip".into(), ip.clone().map_or(Value::Null, Value::from));
        identity.insert(
            "user_agent".into(),
            ua.clone().map_or(Value::Null, Value::from),
        );
        let mut detail = Map::new();
        detail.insert("id_basis".into(), "session".into());
        put(&mut detail, "peer_ip", ip);
        put(&mut detail, "session_created", clean(&s.created, MAX_FACT));
        detail.insert(
            "idle_after_secs".into(),
            self.cfg.companion_idle.as_secs().into(),
        );
        let name = companion_name(ua.as_deref());
        let last_seen = clean(&s.last_seen, MAX_FACT).unwrap_or_else(|| rfc3339(now));
        let first_seen = clean(&s.created, MAX_FACT);
        let matched = DeviceMatch {
            confidence: Confidence::Exact,
            rule: "session".into(),
        };
        let record = DeviceRecord {
            id: id.clone(),
            kind: DeviceKind::Companion,
            alias: None,
            name: name.clone(),
            state,
            transport: Transport::Lan,
            first_seen: first_seen.clone().unwrap_or_else(|| last_seen.clone()),
            last_seen,
            since: rfc3339(now),
            reason_code: reason_code.into(),
            reason: reason.into(),
            capabilities: strs(&["dashboard-session"]),
            service_id: None,
            kb_entry: None,
            matched: matched.clone(),
            firmware: None,
            driver: Some(DeviceDriver {
                stack: Stack::Dashboard,
                interface: Some("session".into()),
                live: true,
            }),
            role: Some("controller".into()),
            identity: identity.clone(),
            actions: strs(COMPANION_ACTIONS),
            detail: cap_detail(detail),
        };
        Detected {
            id,
            summary: Summary {
                kind: DeviceKind::Companion,
                name,
                transport: Transport::Lan,
                capabilities: record.capabilities.clone(),
                service_id: None,
                kb_entry: None,
                identity,
                id_basis: "session".into(),
                matched: Some(matched),
                firmware: None,
            },
            record,
            first_seen_hint: first_seen,
            session_hash: Some(s.hash.clone()),
            hint: None,
            identify_port: None,
        }
    }

    /// A remembered device that is not present now (nothing drives it).
    fn remembered_record(id: &str, e: &super::store::Entry, s: &Summary) -> DeviceRecord {
        let (reason_code, reason) = match s.transport {
            Transport::Bluetooth => ("not_detected", "Not detected now."),
            _ => ("unplugged", "Not plugged in now."),
        };
        let mut detail = Map::new();
        detail.insert("id_basis".into(), s.id_basis.clone().into());
        detail.insert("remembered".into(), true.into());
        DeviceRecord {
            id: id.to_string(),
            kind: s.kind,
            alias: e.alias.clone(),
            name: s.name.clone(),
            state: DeviceState::Disconnected,
            transport: s.transport,
            first_seen: e.first_seen.clone(),
            last_seen: e.last_seen.clone(),
            since: e.last_seen.clone(),
            reason_code: reason_code.into(),
            reason: reason.into(),
            capabilities: s.capabilities.clone(),
            service_id: s.service_id.clone(),
            kb_entry: s.kb_entry.clone(),
            matched: s.matched.clone().unwrap_or(DeviceMatch {
                confidence: Confidence::Unknown,
                rule: "remembered".into(),
            }),
            firmware: s.firmware.clone(),
            driver: None,
            role: None,
            identity: s.identity.clone(),
            actions: strs(if s.kind == DeviceKind::Companion {
                COMPANION_ACTIONS
            } else {
                HARDWARE_ACTIONS
            }),
            detail,
        }
    }

    // ------------------------------------------------------------ tick

    /// Detect + build every record for this instant (no store/publish effects).
    fn detect_all(
        &self,
        facts: &TickFacts,
        sessions: &[SessionRec],
        now: OffsetDateTime,
    ) -> Vec<Detected> {
        let now_s = rfc3339(now);
        let mut out: Vec<Detected> = Vec::new();
        for p in &facts.scan.serial {
            out.push(self.serial_record(p, facts, &now_s));
        }
        for u in &facts.scan.usb_other {
            out.push(self.usb_record(u, facts, &now_s));
        }
        for b in &facts.scan.bluetooth {
            out.push(self.bluetooth_record(b, &now_s));
        }
        for s in sessions {
            out.push(self.companion_record(s, now));
        }
        // Two sources resolving to one id (e.g. duplicate by-id links): keep the first.
        let mut seen = HashSet::new();
        out.retain(|d| seen.insert(d.id.clone()));
        out
    }

    /// One registry tick: detect, read bindings, probe, merge, debounce,
    /// publish. Returns whether a `devices` frame was published.
    pub async fn tick(&self, api: &ApiState) -> bool {
        let roots = self.cfg.roots.clone();
        let rns_config = self.cfg.rns_config.clone();
        let (scan, rnode) = tokio::task::spawn_blocking(move || {
            (detect::scan(&roots), rns::bindings(&rns_config, &roots))
        })
        .await
        .unwrap_or_default();
        let spi_present = scan.usb_other.iter().any(|u| spi_bridge(u).is_some());
        let probe_result = if spi_present {
            probe(self.cfg.probe_addr, self.cfg.probe_timeout).await
        } else {
            Probe::Skipped
        };
        let facts = TickFacts {
            scan,
            rnode,
            rnsd_running: self.rnsd_running(),
            probe: probe_result,
        };
        let sessions = api.auth.sessions();
        self.apply(api, &facts, &sessions, OffsetDateTime::now_utc())
    }

    /// The synchronous half of [`Self::tick`] (unit-testable).
    pub fn apply(
        &self,
        api: &ApiState,
        facts: &TickFacts,
        sessions: &[SessionRec],
        now: OffsetDateTime,
    ) -> bool {
        let mut detected = self.detect_all(facts, sessions, now);
        let now_s = rfc3339(now);
        let mut guard = self.inner.lock().unwrap();
        let inner = &mut *guard;

        // A stack that was live in the published snapshot and is still
        // configured for the device but no longer answers → `stack_down`.
        let prev_drivers: HashMap<&str, &DeviceDriver> = inner
            .published
            .iter()
            .flatten()
            .filter_map(|r| r.driver.as_ref().map(|d| (r.id.as_str(), d)))
            .collect();
        for d in &mut detected {
            let r = &mut d.record;
            if r.driver.is_some() || r.state != DeviceState::NeedsSetup {
                continue;
            }
            let (Some((stack, interface)), Some(prev)) = (&d.hint, prev_drivers.get(d.id.as_str()))
            else {
                continue;
            };
            if prev.stack != *stack {
                continue;
            }
            let down = DeviceDriver {
                stack: *stack,
                interface: interface.clone(),
                live: false,
            };
            let (state, code, reason) = driven_state(Some(&down), ("", ""));
            r.state = state;
            r.reason_code = code.into();
            r.reason = reason;
            r.driver = Some(down);
        }
        drop(prev_drivers);

        // Layer 5 §14: identify results, the Meshtastic binding, `identify`
        let view = self.meshtastic_view();
        inner.identify_ports.clear();
        for d in &mut detected {
            let rns_hint = matches!(d.hint, Some((Stack::Reticulum, _)));
            apply_meshtastic(
                &mut d.record,
                Some(&mut d.summary),
                inner.store.devices.get(&d.id),
                &view,
                d.identify_port.as_ref(),
                rns_hint,
            );
            if let Some(up) = &d.identify_port {
                inner.identify_ports.insert(d.id.clone(), up.clone());
            }
        }

        let mut records = Vec::new();
        let mut present: HashSet<String> = HashSet::new();
        inner.sessions.clear();
        for d in detected {
            present.insert(d.id.clone());
            inner.absent_ticks.remove(&d.id);
            if let Some(h) = &d.session_hash {
                inner.sessions.insert(d.id.clone(), h.clone());
            }
            let entry = inner.store.devices.entry(d.id.clone()).or_insert_with(|| {
                Entry::new(
                    d.first_seen_hint.clone().unwrap_or_else(|| now_s.clone()),
                    d.record.last_seen.clone(),
                )
            });
            entry.last_seen = d.record.last_seen.clone();
            let mut changed = false;
            if entry.last_record_summary.as_ref() != Some(&d.summary) {
                entry.last_record_summary = Some(d.summary.clone());
                changed = true;
            }
            let (forgotten, alias, first_seen) = (
                entry.forgotten,
                entry.alias.clone(),
                entry.first_seen.clone(),
            );
            if changed {
                inner.store.mark_dirty();
            }
            if forgotten {
                continue;
            }
            let mut r = d.record;
            r.alias = alias;
            r.first_seen = first_seen;
            records.push(r);
        }

        let absent: Vec<String> = inner
            .store
            .devices
            .keys()
            .filter(|id| !present.contains(*id))
            .cloned()
            .collect();
        for id in absent {
            let e = inner.store.devices[&id].clone();
            let companion = e
                .last_record_summary
                .as_ref()
                .is_some_and(|s| s.kind == DeviceKind::Companion);
            if e.forgotten {
                let n = inner.absent_ticks.entry(id.clone()).or_insert(0);
                *n += 1;
                if *n >= FORGET_CLEAR_TICKS {
                    inner.absent_ticks.remove(&id);
                    inner.store.devices.remove(&id);
                    inner.store.mark_dirty();
                }
                continue;
            }
            if companion {
                // session revoked/expired: the pairing is over
                inner.store.devices.remove(&id);
                inner.store.mark_dirty();
                continue;
            }
            match &e.last_record_summary {
                Some(s) => records.push(Self::remembered_record(&id, &e, s)),
                // Layer 5 §14: a Meshtastic binding is configuration (e.g. a
                // Wi-Fi board bound by address is never detected): kept
                None if e.meshtastic.is_some() => {}
                None => {
                    inner.store.devices.remove(&id);
                    inner.store.mark_dirty();
                }
            }
        }
        inner.store.gc(&|id| present.contains(id));
        inner.store.flush(false);
        sort_records(&mut records);

        // ---- debounce + publish
        let fp = fingerprint(&records);
        let publish = match &inner.published {
            None => true,
            Some(_) if fp == inner.published_fp => {
                inner.pending = None;
                false
            }
            Some(_) => {
                let count = match &inner.pending {
                    Some((p, c)) if *p == fp => c + 1,
                    _ => 1,
                };
                inner.pending = Some((fp.clone(), count));
                count >= DEBOUNCE_TICKS
            }
        };
        if publish {
            let baseline = inner.published.is_none();
            let prev: HashMap<String, (DeviceState, String)> = inner
                .published
                .iter()
                .flatten()
                .map(|r| (r.id.clone(), (r.state, r.since.clone())))
                .collect();
            for r in &mut records {
                r.since = match prev.get(&r.id) {
                    Some((state, since)) if *state == r.state => since.clone(),
                    _ if baseline && r.state == DeviceState::Disconnected => r.last_seen.clone(),
                    _ => now_s.clone(),
                };
            }
            inner.published = Some(records);
            inner.published_fp = fp;
            inner.pending = None;
            Self::emit(api, inner);
        } else if let Some(published) = &mut inner.published {
            // no state change to announce: keep last_seen fresh for GETs
            let fresh: HashMap<&str, &str> = records
                .iter()
                .map(|r| (r.id.as_str(), r.last_seen.as_str()))
                .collect();
            for r in published.iter_mut() {
                if let Some(ls) = fresh.get(r.id.as_str()) {
                    r.last_seen = ls.to_string();
                }
            }
        }
        publish
    }

    fn emit(api: &ApiState, inner: &Inner) {
        let devices = inner.published.clone().unwrap_or_default();
        api.events
            .publish("devices", &json!({ "devices": devices }));
    }

    // ------------------------------------------------------------ reads

    /// The published snapshot (`None` until the first tick ran).
    pub fn list(&self) -> Option<Vec<DeviceRecord>> {
        self.inner.lock().unwrap().published.clone()
    }

    pub fn get(&self, id: &str) -> Option<DeviceRecord> {
        let inner = self.inner.lock().unwrap();
        inner
            .published
            .as_ref()?
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    // ------------------------------------------------------------ mutations

    /// Set (or clear with `None`) a device's alias; persists + publishes.
    /// `alias` must already be validated (trimmed, ≤ [`MAX_ALIAS`] chars).
    pub fn set_alias(
        &self,
        api: &ApiState,
        id: &str,
        alias: Option<String>,
    ) -> Result<DeviceRecord, NotFound> {
        debug_assert!(alias
            .as_ref()
            .is_none_or(|a| a.chars().count() <= MAX_ALIAS));
        let mut guard = self.inner.lock().unwrap();
        let inner = &mut *guard;
        let published = inner.published.as_mut().ok_or(NotFound)?;
        let rec = published.iter_mut().find(|r| r.id == id).ok_or(NotFound)?;
        let entry = inner.store.devices.get_mut(id).ok_or(NotFound)?;
        entry.alias = alias.clone();
        rec.alias = alias;
        let out = rec.clone();
        inner.store.mark_dirty();
        inner.store.flush(true);
        sort_records(published);
        inner.published_fp = fingerprint(published);
        Self::emit(api, inner);
        Ok(out)
    }

    /// Forget a device (see module docs of `devices`): hides it, drops its
    /// alias; a companion's session is revoked — including the session making
    /// the request (that browser's next call gets `401`). Persists + publishes.
    pub fn forget(&self, api: &ApiState, id: &str) -> Result<(), NotFound> {
        let mut guard = self.inner.lock().unwrap();
        let inner = &mut *guard;
        let published = inner.published.as_mut().ok_or(NotFound)?;
        let idx = published.iter().position(|r| r.id == id).ok_or(NotFound)?;
        let rec = published.remove(idx);
        if rec.kind == DeviceKind::Companion {
            if let Some(hash) = inner.sessions.remove(id) {
                api.auth.revoke_hash(&hash);
            }
            inner.store.devices.remove(id);
        } else if rec.state == DeviceState::Disconnected {
            // absent hardware: nothing left to hide
            inner.store.devices.remove(id);
        } else if let Some(e) = inner.store.devices.get_mut(id) {
            e.forgotten = true;
            e.alias = None;
            // forgetting a bound board unbinds it (the Meshtastic module stops
            // meshmud on its next tick)
            e.meshtastic = None;
        }
        inner.absent_ticks.remove(id);
        inner.store.mark_dirty();
        inner.store.flush(true);
        inner.published_fp = fingerprint(published);
        Self::emit(api, inner);
        Ok(())
    }

    // ------------------------------------------------------------ Layer 5 §14

    /// The device `identify` may run on now, from the published snapshot.
    /// A device driven by rnsd (or naming an RNode stanza) or held by meshmud
    /// is [`TargetError::Busy`]; one that does not offer the action is
    /// [`TargetError::NotOffered`].
    pub fn identify_target(&self, id: &str) -> Result<IdentifyTarget, TargetError> {
        let inner = self.inner.lock().unwrap();
        let rec = inner
            .published
            .as_ref()
            .and_then(|p| p.iter().find(|r| r.id == id))
            .cloned()
            .ok_or(TargetError::NotFound)?;
        let port = inner.identify_ports.get(id).cloned();
        let port_name = port.as_ref().map(Uplink::port_name).unwrap_or_default();
        if rec.actions.iter().any(|a| a == IDENTIFY_ACTION) {
            return match port {
                Some(uplink) => Ok(IdentifyTarget {
                    id: id.to_string(),
                    uplink,
                    record: rec,
                }),
                None => Err(TargetError::NotOffered("no port to probe")),
            };
        }
        let busy = |owner: &str| TargetError::Busy {
            port: port_name.clone(),
            owner: owner.to_string(),
        };
        if let Some(d) = &rec.driver {
            match (d.stack, d.interface.as_deref()) {
                (Stack::Reticulum, _) => return Err(busy("rnsd")),
                (Stack::Meshtastic, Some(MESHMUD_INTERFACE)) => return Err(busy("meshmud")),
                _ => {}
            }
        }
        if rec.detail.contains_key("rns_interface") {
            return Err(busy("rnsd"));
        }
        if rec.reason_code == "port_busy" {
            let owner = rec
                .reason
                .strip_prefix("Its port is in use by ")
                .and_then(|r| r.strip_suffix('.'))
                .unwrap_or("another program");
            return Err(busy(owner));
        }
        Err(TargetError::NotOffered(match rec.kind {
            DeviceKind::Radio if !rec.state.present() => "the board is not plugged in",
            DeviceKind::Radio if port.is_none() => "the board has no port to probe",
            DeviceKind::Radio => "identify cannot run on this node for this board",
            _ => "only LoRa boards can be identified",
        }))
    }

    /// Persist an identify result (Layer 5 §14): `firmware` (source probe)
    /// and, for a Meshtastic board, the binding — at most one device holds
    /// one, so any other binding is cleared. Publishes the updated record
    /// at once and returns it.
    pub fn record_identify(
        &self,
        api: &ApiState,
        id: &str,
        firmware: DeviceFirmware,
        binding: Option<MeshtasticBinding>,
    ) -> Result<DeviceRecord, NotFound> {
        let view = self.meshtastic_view();
        let mut guard = self.inner.lock().unwrap();
        let inner = &mut *guard;
        if !inner.store.devices.contains_key(id) {
            return Err(NotFound);
        }
        if binding.is_some() {
            for (other, e) in inner.store.devices.iter_mut() {
                if other != id {
                    e.meshtastic = None;
                }
            }
        }
        let entry = inner.store.devices.get_mut(id).ok_or(NotFound)?;
        entry.firmware = Some(firmware);
        entry.meshtastic = binding;
        let entry = entry.clone();
        inner.store.mark_dirty();
        inner.store.flush(true);
        let port = inner.identify_ports.get(id).cloned();
        let published = inner.published.as_mut().ok_or(NotFound)?;
        let rec = published.iter_mut().find(|r| r.id == id).ok_or(NotFound)?;
        let rns_hint = rec.detail.contains_key("rns_interface");
        apply_meshtastic(rec, None, Some(&entry), &view, port.as_ref(), rns_hint);
        let out = rec.clone();
        sort_records(published);
        inner.published_fp = fingerprint(published);
        Self::emit(api, inner);
        Ok(out)
    }

    /// The device holding the Meshtastic binding (from `devices.json`).
    pub fn meshtastic_binding(&self) -> Option<(String, MeshtasticBinding)> {
        self.inner.lock().unwrap().store.meshtastic_binding()
    }

    /// A device identified as Meshtastic but not bound (Layer 5 §14 "NOT SET
    /// when a Meshtastic board is identified but not bound"): `(id, uses a
    /// USB serial port)`.
    pub fn identified_unbound_meshtastic(&self) -> Option<(String, bool)> {
        let inner = self.inner.lock().unwrap();
        inner.store.devices.iter().find_map(|(id, e)| {
            (e.meshtastic.is_none()
                && !e.forgotten
                && e.firmware.as_ref().map(|f| f.family) == Some(FirmwareFamily::Meshtastic))
            .then(|| {
                let usb = e
                    .last_record_summary
                    .as_ref()
                    .is_some_and(|s| s.transport == Transport::UsbSerial);
                (id.clone(), usb)
            })
        })
    }

    /// A display label for a device: its alias, else its name (published
    /// snapshot first, then the remembered summary).
    pub fn label(&self, id: &str) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        if let Some(r) = inner
            .published
            .as_ref()
            .and_then(|p| p.iter().find(|r| r.id == id))
        {
            return Some(r.alias.clone().unwrap_or_else(|| r.name.clone()));
        }
        let e = inner.store.devices.get(id)?;
        e.alias
            .clone()
            .or_else(|| e.last_record_summary.as_ref().map(|s| s.name.clone()))
    }

    /// The stored identify result of a device.
    pub fn stored_firmware(&self, id: &str) -> Option<DeviceFirmware> {
        let inner = self.inner.lock().unwrap();
        inner.store.devices.get(id).and_then(|e| e.firmware.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, kind: DeviceKind, state: DeviceState, name: &str) -> DeviceRecord {
        DeviceRecord {
            id: id.into(),
            kind,
            alias: None,
            name: name.into(),
            state,
            transport: Transport::Unknown,
            first_seen: "a".into(),
            last_seen: "b".into(),
            since: "c".into(),
            reason_code: "x".into(),
            reason: "x".into(),
            capabilities: vec![],
            service_id: None,
            kb_entry: None,
            matched: DeviceMatch {
                confidence: Confidence::Unknown,
                rule: "t".into(),
            },
            firmware: None,
            driver: None,
            role: None,
            identity: Map::new(),
            actions: vec![],
            detail: Map::new(),
        }
    }

    #[test]
    fn sort_present_first_then_kind_then_name() {
        let mut v = vec![
            rec("1", DeviceKind::Radio, DeviceState::Disconnected, "A"),
            rec("2", DeviceKind::Unknown, DeviceState::Connected, "b"),
            rec("3", DeviceKind::Radio, DeviceState::NeedsSetup, "Zeta"),
            rec("4", DeviceKind::Radio, DeviceState::Connected, "alpha"),
            rec("5", DeviceKind::Bluetooth, DeviceState::Error, "c"),
        ];
        sort_records(&mut v);
        let ids: Vec<&str> = v.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["4", "3", "5", "2", "1"]);
    }

    #[test]
    fn fingerprint_ignores_volatile_timestamps() {
        let a = rec("1", DeviceKind::Radio, DeviceState::Connected, "A");
        let mut b = a.clone();
        b.last_seen = "later".into();
        b.since = "other".into();
        assert_eq!(
            fingerprint(std::slice::from_ref(&a)),
            fingerprint(std::slice::from_ref(&b))
        );
        b.state = DeviceState::NeedsSetup;
        assert_ne!(fingerprint(&[a]), fingerprint(&[b]));
    }

    #[test]
    fn state_follows_the_driver() {
        let unbound = ("radio_unbound", "nothing");
        assert_eq!(driven_state(None, unbound).0, DeviceState::NeedsSetup);
        let mut d = DeviceDriver {
            stack: Stack::Reticulum,
            interface: Some("Roof".into()),
            live: true,
        };
        let (s, code, reason) = driven_state(Some(&d), unbound);
        assert_eq!((s, code), (DeviceState::Connected, "stack_bound"));
        assert!(reason.contains("Roof"), "{reason}");
        d.live = false;
        let (s, code, _) = driven_state(Some(&d), unbound);
        assert_eq!((s, code), (DeviceState::Error, "stack_down"));
    }

    #[test]
    fn table_matching_rules() {
        let rules = lora_table();
        let usb = |vid: &str, pid: &str, product: Option<&str>| UsbInfo {
            vendor_id: vid.into(),
            product_id: pid.into(),
            product: product.map(String::from),
            ..Default::default()
        };
        let find = |u: &UsbInfo| {
            rules
                .iter()
                .find(|r| rule_matches(r, u))
                .map(|r| r.name.clone())
        };
        // product substring required for the CP2102 row
        assert_eq!(
            find(&usb(
                "10c4",
                "ea60",
                Some("CP2102 USB to UART Bridge Controller")
            ))
            .as_deref(),
            Some("Heltec WiFi LoRa 32 V3 (CP2102)")
        );
        assert_eq!(find(&usb("10c4", "ea60", Some("Other bridge"))), None);
        assert_eq!(find(&usb("10c4", "ea60", None)), None);
        // pid wildcard
        assert_eq!(
            find(&usb("303a", "1001", None)).as_deref(),
            Some("ESP32-S3 native USB")
        );
        assert_eq!(find(&usb("0403", "6001", Some("FT232R USB UART"))), None);
        // the SPI bridge is recognised before the table's 1a86:* row
        assert!(spi_bridge(&usb("1a86", "5512", None)).is_some());
        assert!(spi_bridge(&usb("1a86", "7523", None)).is_none());
    }
}
