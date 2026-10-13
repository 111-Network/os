//! Device registry (Layer 4 R4, spec `os/interface/README.md` §6.3, §7.1):
//! hardware the user attached to THIS node and devices they paired with its
//! control panel — never other people's nodes (those are Nearby, Layer 7).
//!
//! - [`detect`] — USB serial (`/dev/serial/by-id`), USB devices (`/sys/bus/usb`),
//!   Bluetooth adapters (`/sys/class/bluetooth`); fake-tree injectable.
//! - [`store`] — `<root>/state/devices.json`: aliases, forget marks, first/last
//!   seen, a summary of the last record (so absent hardware is remembered).
//! - [`rns`] — enabled `RNodeInterface` stanzas of `<root>/rns/config` (which
//!   ports the Reticulum stack drives → `driver`).
//! - [`registry`] — the 5 s tick: detection + dashboard sessions (companions) →
//!   [`DeviceRecord`]s, 2-tick debounce, `devices` frames on the events bus,
//!   alias/forget mutations for `/api/v1/devices*`.
//!
//! ## Stable ids
//! `id = hex(sha256(kind + ":" + stable_key))[..16]` where `stable_key` is, in
//! order: the `/dev/serial/by-id` symlink name; `vendor:product:serial`;
//! `vendor:product:<sysfs bus id>` (port-dependent — `detail.id_basis` says
//! which); the adapter MAC for Bluetooth (`hci` name when no MAC); the session
//! hash for companions (the hash itself is never exported).
//!
//! ## Forget semantics
//! `DELETE /devices/{id}` drops the alias and hides the device. Hardware that is
//! still plugged in stays hidden (`forgotten: true` in `devices.json`, across
//! restarts). Once it has been absent for 2 consecutive ticks the record is
//! dropped entirely, so when it is plugged in again it reappears as a new
//! device (no alias, fresh `first_seen`) — the forget mark clears itself.
//! Forgetting a companion revokes its dashboard session (that browser must pair
//! again — allowed from that very browser, which is then logged out). Companions live exactly as long as their session: a session that is
//! revoked (logout, `111 dashboard rotate`, expiry) drops the companion.

pub mod detect;
pub mod registry;
pub mod rns;
pub mod store;

pub use detect::Roots;
pub use registry::{DeviceRegistry, RegistryConfig};

use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// Cap for untrusted sysfs strings (chars).
pub const MAX_FACT: usize = 64;
/// Cap for path-like facts (by-id names are often > 64 chars).
pub const MAX_PATH_FACT: usize = 128;
/// Cap for a stored/exported browser user agent (chars).
pub const MAX_USER_AGENT: usize = 256;
/// Cap for a device alias (chars, after trimming).
pub const MAX_ALIAS: usize = 64;
/// `detail` bound (bytes of compact JSON).
pub const MAX_DETAIL_BYTES: usize = 4096;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Radio,
    Bluetooth,
    Companion,
    Node,
    Analog,
    Unknown,
}

impl DeviceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceKind::Radio => "radio",
            DeviceKind::Bluetooth => "bluetooth",
            DeviceKind::Companion => "companion",
            DeviceKind::Node => "node",
            DeviceKind::Analog => "analog",
            DeviceKind::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Connected,
    Disconnected,
    NeedsSetup,
    Error,
}

impl DeviceState {
    /// Present on the node now (anything but `disconnected`).
    pub fn present(self) -> bool {
        self != DeviceState::Disconnected
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    UsbSerial,
    Spi,
    Bluetooth,
    Wifi,
    Lan,
    Tcp,
    Audio,
    Unknown,
}

/// How sure the identification is (`DeviceMatch.confidence`). A USB VID:PID
/// names the serial chip, not the board or its firmware, so a LoRa-table hit
/// is only ever `likely` (`os/plan/devices.md` §0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Exact,
    Likely,
    Unknown,
}

/// `DeviceMatch`: how the record was identified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct DeviceMatch {
    pub confidence: Confidence,
    /// e.g. `lora-table 303a:1001`, `usb-ids 0403:6001`, `session`.
    pub rule: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FirmwareFamily {
    Rnode,
    Meshtastic,
    Meshcore,
    Kiss,
    None,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FirmwareSource {
    Probe,
    User,
    Agent,
    Unknown,
}

/// `DeviceFirmware`. Detection is sysfs-only (a serial port is never opened —
/// it can reset ESP32 boards), so v1 only ever says `unknown`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct DeviceFirmware {
    pub family: FirmwareFamily,
    pub version: Option<String>,
    pub source: FirmwareSource,
}

impl DeviceFirmware {
    pub fn unknown() -> Self {
        DeviceFirmware {
            family: FirmwareFamily::Unknown,
            version: None,
            source: FirmwareSource::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stack {
    Reticulum,
    Meshtastic,
    Meshcore,
    Dashboard,
}

/// `DeviceDriver`: what drives the device now. `None` on the record (JSON
/// `null`) = nothing bound → a present device `needs_setup`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct DeviceDriver {
    pub stack: Stack,
    /// rnsd interface (stanza) name, `meshtasticd`, `session`.
    pub interface: Option<String>,
    /// The bound stack is up (rnsd child running / meshtasticd answering).
    pub live: bool,
}

/// `DeviceRecord` (openapi.yaml).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeviceRecord {
    pub id: String,
    pub kind: DeviceKind,
    pub alias: Option<String>,
    pub name: String,
    pub state: DeviceState,
    pub transport: Transport,
    pub first_seen: String,
    pub last_seen: String,
    pub since: String,
    pub reason_code: String,
    pub reason: String,
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kb_entry: Option<String>,
    #[serde(rename = "match")]
    pub matched: DeviceMatch,
    pub firmware: Option<DeviceFirmware>,
    pub driver: Option<DeviceDriver>,
    /// Companions only (`controller` in v1); `null` otherwise.
    pub role: Option<String>,
    pub identity: Map<String, Value>,
    pub actions: Vec<String>,
    pub detail: Map<String, Value>,
}

/// Untrusted string → display-safe: control and bidi-override characters
/// removed, trimmed, capped at `max` chars. `None` when nothing is left.
pub fn clean(s: &str, max: usize) -> Option<String> {
    let filtered: String = s
        .chars()
        .filter(|c| !c.is_control() && !is_bidi_control(*c))
        .collect();
    let capped: String = filtered.trim().chars().take(max).collect();
    let out = capped.trim_end().to_string();
    (!out.is_empty()).then_some(out)
}

/// Bidi embedding/override/isolate controls (Trojan-Source class).
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// The stable device id (see module docs).
pub fn device_id(kind: DeviceKind, stable_key: &str) -> String {
    let digest = Sha256::digest(format!("{}:{stable_key}", kind.as_str()).as_bytes());
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Path parameter validation (`^[A-Za-z0-9_-]{4,64}$`).
pub fn valid_id(id: &str) -> bool {
    (4..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Bound `detail` to [`MAX_DETAIL_BYTES`]: drop the longest values first, then
/// mark `truncated: true`.
pub fn cap_detail(mut detail: Map<String, Value>) -> Map<String, Value> {
    let size = |m: &Map<String, Value>| Value::Object(m.clone()).to_string().len();
    if size(&detail) <= MAX_DETAIL_BYTES {
        return detail;
    }
    detail.insert("truncated".into(), Value::Bool(true));
    while size(&detail) > MAX_DETAIL_BYTES {
        let longest = detail
            .iter()
            .filter(|(k, _)| *k != "truncated" && *k != "id_basis")
            .max_by_key(|(_, v)| v.to_string().len())
            .map(|(k, _)| k.clone());
        match longest {
            Some(k) => {
                detail.remove(&k);
            }
            None => break,
        }
    }
    detail
}

/// A LoRa-table row name without its chip note: "Heltec WiFi LoRa 32 V3
/// (CP2102)" → "Heltec WiFi LoRa 32 V3".
pub fn board_base(name: &str) -> &str {
    name.split('(').next().unwrap_or(name).trim()
}

/// KB slug for a board name: the part before any `(`, lowercased, runs of
/// non-alphanumerics → `-` ("Heltec WiFi LoRa 32 V3 (CP2102)" →
/// "heltec-wifi-lora-32-v3").
pub fn kb_slug(name: &str) -> String {
    let base = board_base(name);
    let mut out = String::new();
    for c in base.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// A plain companion name from a browser user agent (display only).
pub fn companion_name(user_agent: Option<&str>) -> String {
    let ua = user_agent.unwrap_or("");
    let platform = if ua.contains("iPhone") {
        "iPhone"
    } else if ua.contains("iPad") {
        "iPad"
    } else if ua.contains("Android") {
        "Android device"
    } else if ua.contains("CrOS") {
        "Chromebook"
    } else if ua.contains("Macintosh") || ua.contains("Mac OS X") {
        "Mac"
    } else if ua.contains("Windows") {
        "Windows computer"
    } else if ua.contains("Linux") {
        "Linux computer"
    } else {
        return "Paired browser".into();
    };
    format!("Browser on {platform}")
}

/// Create the registry for `api` (production paths from its layout), insert it
/// into `api.ext`, and spawn the 5 s tick. Called once from `daemon/mod.rs`.
///
/// `rnsd_running` reports whether the rnsd child is up (the Reticulum stack's
/// liveness for devices bound by an `RNodeInterface` stanza).
pub fn start(
    api: std::sync::Arc<crate::daemon::dashboard::ApiState>,
    rnsd_running: impl Fn() -> bool + Send + Sync + 'static,
) -> std::sync::Arc<DeviceRegistry> {
    let reg = install(&api, RegistryConfig::production(&api.layout));
    reg.set_rnsd_running(rnsd_running);
    let task_reg = std::sync::Arc::clone(&reg);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(registry::TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            task_reg.tick(&api).await;
        }
    });
    reg
}

/// Create a registry with `cfg` and insert it into `api.ext` (no task: tests
/// drive [`DeviceRegistry::tick`] themselves).
pub fn install(
    api: &crate::daemon::dashboard::ApiState,
    cfg: RegistryConfig,
) -> std::sync::Arc<DeviceRegistry> {
    let reg = std::sync::Arc::new(DeviceRegistry::new(cfg));
    api.insert_ext(std::sync::Arc::clone(&reg));
    reg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_controls_bidi_and_caps() {
        assert_eq!(clean("  CP2102\n", 64).as_deref(), Some("CP2102"));
        assert_eq!(
            clean("a\u{7}b\u{1b}[31mc\0", 64).as_deref(),
            Some("ab[31mc")
        );
        assert_eq!(clean("x\u{202e}y\u{2066}z", 64).as_deref(), Some("xyz"));
        assert_eq!(clean(" \n\t ", 64), None);
        let long = "é".repeat(300);
        assert_eq!(clean(&long, 64).unwrap().chars().count(), 64);
        // cap then trim: no trailing space left at the cut
        assert_eq!(clean("abc def", 4).as_deref(), Some("abc"));
    }

    #[test]
    fn ids_are_stable_16_hex_and_kind_scoped() {
        let a = device_id(DeviceKind::Radio, "usb-Espressif_X-if00");
        assert_eq!(a, device_id(DeviceKind::Radio, "usb-Espressif_X-if00"));
        assert_eq!(a.len(), 16);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(valid_id(&a));
        assert_ne!(a, device_id(DeviceKind::Unknown, "usb-Espressif_X-if00"));
    }

    #[test]
    fn id_validation() {
        for ok in ["abcd", "A-b_9", &"x".repeat(64)] {
            assert!(valid_id(ok), "{ok}");
        }
        for bad in ["abc", "ab/cd", "../..", "abcd.", &"x".repeat(65), "ab cd"] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn detail_is_capped() {
        let mut m = Map::new();
        m.insert("id_basis".into(), "serial-by-id".into());
        m.insert("small".into(), "x".into());
        for i in 0..10 {
            m.insert(format!("big{i}"), "y".repeat(1000).into());
        }
        let capped = cap_detail(m);
        assert!(Value::Object(capped.clone()).to_string().len() <= MAX_DETAIL_BYTES);
        assert_eq!(capped["truncated"], true);
        assert_eq!(capped["id_basis"], "serial-by-id");
        assert_eq!(capped["small"], "x");
        // already small → untouched, no marker
        let mut s = Map::new();
        s.insert("a".into(), 1.into());
        assert_eq!(cap_detail(s.clone()), s);
    }

    #[test]
    fn slugs() {
        assert_eq!(
            kb_slug("Heltec WiFi LoRa 32 V3 (CP2102)"),
            "heltec-wifi-lora-32-v3"
        );
        assert_eq!(kb_slug("LILYGO T-Echo (nRF52 DFU)"), "lilygo-t-echo");
        assert_eq!(kb_slug("RAK4631"), "rak4631");
        assert_eq!(kb_slug("ESP32-S3 native USB"), "esp32-s3-native-usb");
    }

    #[test]
    fn companion_names() {
        assert_eq!(
            companion_name(Some("Mozilla/5.0 (Linux; Android 14; Pixel 7)")),
            "Browser on Android device"
        );
        assert_eq!(
            companion_name(Some(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)"
            )),
            "Browser on iPhone"
        );
        assert_eq!(companion_name(None), "Paired browser");
    }
}
