//! `state/relay.json`: every Layer 7 setting and the state kept beside it (§6.3, S19; §2.4 item 4;
//! §6.2 "Drain"; §12).
//!
//! **What is here.** Every §6.3 key at its default, all off: `gateway.enabled` `false`,
//! `gateway.termux_poll_min` `15` (≥ [`MIN_TERMUX_POLL_MIN`], §3.4), `carrier.kind` `"none"`,
//! `carrier.ntfy.server` / `.token` / `.topic_secret` `null`, `carrier.mode` `"5A"` (5B refused
//! on a Termux hub), `carrier.consent` `null`, `presence.heartbeat.<transport>` `0` (events only,
//! §2.4), `discovery.*` `false`, `origin.serve` `false`, `origin.targets` = the hub's own target
//! (§9.1), `updates.check_internet` `true` (§9.3), `metered` `[]` (§12). Beside them, state that
//! is not a setting (§6.3 last paragraph): the consent hashes of `gateway_bit` and `origin_serve`
//! (§12; `carrier_enable`'s is `carrier.consent`), the 5A drain's last id per topic (§6.2, saved
//! only once the fabric took the batch, C13) and the metered window (§12, `CLOCK_BOOTTIME`
//! seconds, never the wall clock).
//!
//! **What is not here.** `quota.transit` is a store quota (`Store::set_quota`, the store's quota
//! table, §5.8); the pairs and `pair_plain`'s warning hash live in `fabric/settings` (§4.6).
//!
//! **The file** (the `fabric/settings` rule, `N/fabric/settings.rs:17-21`): JSON
//! `{"schema": 1, …}`, written atomically with mode 0600. Absent ⇒ the defaults, and **no file
//! exists until the first write** ([`load`] never writes; [`update`] writes only on a change). A
//! file that is not a small regular file (a symlink, a directory, over [`MAX_FILE`]), is not JSON,
//! has another schema, an unknown key or any value outside its range reads as **all defaults (all
//! off: no carrier, no consent, no serving)** and is **moved aside** to `relay.json.invalid-<n>`
//! (never deleted; a symlink is moved itself, never followed), its path logged once. Fail closed:
//! no consent survives an invalid file, so every switch asks again.
//!
//! **Secrets are write-only (§6.3, §13.1).** `carrier.ntfy.token` and `carrier.ntfy.topic_secret`
//! are held in [`Secret`] (zeroized on drop, `Debug` prints only whether one is set); the file is
//! the only place they are serialised. Every view ([`SettingsView`]) carries `{"set": bool}`
//! instead, and no log line here carries a value read from the file (a parse failure logs its
//! category and position only).
//!
//! **Weight (§1 rule 2, §5.11; the Pi floor).** Nothing here runs on its own: no watcher, timer,
//! thread or task. The file is read when the daemon calls [`load`] and written when a caller
//! changes a value through [`update`] (or [`save`]).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zeroize::Zeroizing;

use crate::daemon::carrier::ntfy::{TOPIC_LEN, TOPIC_PREFIX};
use crate::daemon::updater::fetch::TARGET_TRIPLE;

/// File name inside the daemon's `state/` directory (§6.3).
pub const RELAY_FILE: &str = "relay.json";
/// File schema.
pub const SCHEMA: u32 = 1;
/// Largest file read back; anything larger is invalid (checked before it is read).
pub const MAX_FILE: u64 = 64 * 1024;
/// `gateway.termux_poll_min` default (§3.4: above the 9 min Doze floor).
pub const DEFAULT_TERMUX_POLL_MIN: u32 = 15;
/// `gateway.termux_poll_min` floor (§3.4; Doze fires exact alarms at most once per nine minutes).
pub const MIN_TERMUX_POLL_MIN: u32 = 9;
/// `gateway.termux_poll_min` ceiling (one day).
pub const MAX_TERMUX_POLL_MIN: u32 = 24 * 60;
/// Longest `carrier.ntfy.server` (the contract's `maxLength`).
pub const MAX_SERVER_URL: usize = 512;
/// Longest `carrier.ntfy.token` (the contract's `maxLength`).
pub const MAX_TOKEN: usize = 512;
/// `carrier.ntfy.topic_secret` bytes (§6.2: 32 B).
pub const TOPIC_SECRET_LEN: usize = 32;
/// The Layer 7 heartbeat transports (§2.4 item 2): the v1 set every write to answers
/// `409 no_such_transport`. `rns` is Layer 6's setting (`owner: layer6`), shown by the status
/// route from `PATCH /api/v1/bridge/settings`, never stored here.
pub const HEARTBEAT_TRANSPORTS: [&str; 3] = ["tcp-lan", "kiss-modem", "meshtastic-client"];
/// Largest heartbeat interval accepted from the file (a week).
pub const MAX_HEARTBEAT_MIN: u32 = 7 * 24 * 60;
/// `origin.targets` entries kept (the own target included).
pub const MAX_TARGETS: usize = 8;
/// Longest target triple.
pub const MAX_TARGET_LEN: usize = 64;
/// `metered` rules kept.
pub const MAX_METERED: usize = 16;
/// Longest interface name (Linux `IFNAMSIZ` − 1).
pub const MAX_IFACE_LEN: usize = 15;
/// Longest SSID (IEEE 802.11: 32 octets).
pub const MAX_SSID_LEN: usize = 32;
/// Drain cursors kept (one inbound topic per carrier pair, `MAX_PAIRED_HUBS` = 32, and room for
/// the topics of a re-key until the caller forgets the old ones).
pub const MAX_DRAIN_TOPICS: usize = 64;
/// Longest ntfy message id kept (ids are 12 alphanumerics upstream; the carrier's own bound).
pub const MAX_DRAIN_ID: usize = 64;
/// Slots kept per metered link in the window.
pub const MAX_WINDOW_SLOTS: usize = 288;
/// Longest boot id the window is tagged with (`/proc/sys/kernel/random/boot_id` is 36).
pub const MAX_BOOT_ID: usize = 64;
/// Attempts at a free `relay.json.invalid-<n>` name.
const MOVE_ASIDE_TRIES: u32 = 1000;

/// Serialises every read-modify-write of the file in this process ([`update`]).
static WRITE_LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------- errors

/// Why a value was refused (a setter, or a field of the file at load). The text never carries a
/// secret's value; the caller (row D10) maps it to `400 bad_request` (or, for
/// [`SettingsError::ModeRefusedOnTermux`], the code its route documents).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    /// `carrier.ntfy.server` is not `https://`, or `http://` to a host that is not a private,
    /// loopback or link-local IP literal (§6.3, §9.5 T6, S12).
    BadServer,
    /// `carrier.ntfy.token` is empty, too long or not printable ASCII.
    BadToken,
    /// `carrier.ntfy.topic_secret` is not 32 B of RFC 4648 base32, or is all zero.
    BadTopicSecret,
    /// `gateway.termux_poll_min` is under [`MIN_TERMUX_POLL_MIN`] or over a day.
    BadTermuxPoll,
    /// `carrier.kind` is not `none` or `ntfy` (`chatmail` joins after §6.5).
    BadCarrierKind,
    /// `carrier.mode` is not `5A` or `5B`.
    BadMode,
    /// `carrier.mode = "5B"` on a Termux hub (§3.1, §6.3).
    ModeRefusedOnTermux,
    /// A heartbeat key is not a Layer 7 transport, or its interval is out of range.
    BadHeartbeat,
    /// An `origin.targets` entry is not a target triple, or there are too many.
    BadTarget,
    /// A `metered` rule is malformed (the text names how).
    BadMetered(&'static str),
    /// A consent hash is not lowercase hex SHA-256.
    BadHash,
    /// A drain topic is not a carrier topic name (§6.2), or a drain id not an ntfy id.
    BadDrain,
    /// More than [`MAX_DRAIN_TOPICS`] drain cursors.
    TooManyTopics,
    /// The metered window is malformed (the text names how).
    BadWindow(&'static str),
    /// The schema is not [`SCHEMA`].
    Schema,
    /// The file is not the expected JSON (its category and position, never its content).
    Parse(String),
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::BadServer => f.write_str(
                "carrier.ntfy.server must be https:// (http:// only for a private address)",
            ),
            SettingsError::BadToken => {
                f.write_str("carrier.ntfy.token must be 1 to 512 printable ASCII characters")
            }
            SettingsError::BadTopicSecret => f.write_str(
                "carrier.ntfy.topic_secret must be 32 bytes in RFC 4648 base32, not all zero",
            ),
            SettingsError::BadTermuxPoll => {
                f.write_str("gateway.termux_poll_min must be 9 to 1440 minutes")
            }
            SettingsError::BadCarrierKind => f.write_str("carrier.kind must be none or ntfy"),
            SettingsError::BadMode => f.write_str("carrier.mode must be 5A or 5B"),
            SettingsError::ModeRefusedOnTermux => {
                f.write_str("carrier.mode 5B is refused on a Termux hub (§3.1)")
            }
            SettingsError::BadHeartbeat => f.write_str(
                "presence.heartbeat names a transport that takes no Layer 7 beat, or an interval out of range",
            ),
            SettingsError::BadTarget => f.write_str(
                "origin.targets must be at most 8 target triples (lowercase, digits, '_' and '-')",
            ),
            SettingsError::BadMetered(why) => write!(f, "metered: {why}"),
            SettingsError::BadHash => f.write_str("a consent hash must be 64 lowercase hex"),
            SettingsError::BadDrain => f.write_str("a drain cursor is not a carrier topic and id"),
            SettingsError::TooManyTopics => f.write_str("too many drain cursors"),
            SettingsError::BadWindow(why) => write!(f, "metered window: {why}"),
            SettingsError::Schema => f.write_str("unknown schema"),
            SettingsError::Parse(why) => write!(f, "not the relay settings JSON ({why})"),
        }
    }
}

impl std::error::Error for SettingsError {}

// ---------------------------------------------------------------- typed values

/// A write-only secret (§6.3, §13.1): zeroized on drop; `Debug` never prints it; serialised only
/// into the file.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    /// The value, for the one consumer that needs it (the carrier's request, row D11).
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(set)")
    }
}

/// `carrier.kind` (§6.3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CarrierKind {
    /// No carrier: nothing is constructed (§6.3 last paragraph).
    #[default]
    None,
    /// The ntfy carrier (§6.2).
    Ntfy,
}

impl CarrierKind {
    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            CarrierKind::None => "none",
            CarrierKind::Ntfy => "ntfy",
        }
    }

    /// Parse the wire name (`chatmail` is refused until §6.5 lands).
    pub fn parse(s: &str) -> Result<Self, SettingsError> {
        match s {
            "none" => Ok(CarrierKind::None),
            "ntfy" => Ok(CarrierKind::Ntfy),
            _ => Err(SettingsError::BadCarrierKind),
        }
    }
}

/// `carrier.mode` (§3.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CarrierMode {
    /// Requests on a schedule the flush sets (the default).
    #[default]
    #[serde(rename = "5A")]
    FiveA,
    /// A held stream (never on a Termux hub).
    #[serde(rename = "5B")]
    FiveB,
}

impl CarrierMode {
    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            CarrierMode::FiveA => "5A",
            CarrierMode::FiveB => "5B",
        }
    }

    /// Parse the wire name.
    pub fn parse(s: &str) -> Result<Self, SettingsError> {
        match s {
            "5A" => Ok(CarrierMode::FiveA),
            "5B" => Ok(CarrierMode::FiveB),
            _ => Err(SettingsError::BadMode),
        }
    }
}

/// The Layer 7 consents whose hash this file keeps (§12). `pair_plain`'s hash lives with its pair
/// in `fabric/settings` (§4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentKind {
    /// `carrier_enable` (§6.2 copy): kept as `carrier.consent`.
    CarrierEnable,
    /// `gateway_bit` (the public gateway role bit, §12).
    GatewayBit,
    /// `origin_serve` (serving update files to the LAN, §12).
    OriginServe,
}

/// One `metered` rule's match (§12): exactly one of `iface` or `ssid`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeteredMatch {
    /// A network interface name (`wwan0`).
    Iface(String),
    /// A Wi-Fi SSID the user marks (a hint: Termux cannot read Android's metered flag).
    Ssid(String),
}

impl MeteredMatch {
    fn check(&self) -> Result<(), SettingsError> {
        match self {
            MeteredMatch::Iface(n) => {
                if n.is_empty()
                    || n.len() > MAX_IFACE_LEN
                    || n == "."
                    || n == ".."
                    || n.bytes().any(|b| !(0x21..=0x7e).contains(&b) || b == b'/')
                {
                    return Err(SettingsError::BadMetered(
                        "iface must be 1 to 15 printable characters without '/'",
                    ));
                }
            }
            MeteredMatch::Ssid(s) => {
                if s.is_empty() || s.len() > MAX_SSID_LEN || s.chars().any(char::is_control) {
                    return Err(SettingsError::BadMetered(
                        "ssid must be 1 to 32 bytes without control characters",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// One metered link (§12; `MeteredLink` in the contract).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeteredLink {
    /// The match: `{"iface": …}` or `{"ssid": …}`.
    #[serde(rename = "match")]
    pub matcher: MeteredMatch,
    /// Carrier bytes, both ways, allowed in a rolling 24 h window (≥ 1).
    pub daily_cap_bytes: u64,
}

/// One slot of the metered window: carrier bytes counted from `at_s` (§12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeteredSlot {
    /// `CLOCK_BOOTTIME` seconds of the boot named by [`MeteredWindow::boot_id`]; never the wall
    /// clock.
    pub at_s: u64,
    /// Carrier bytes (bodies and headers, both ways) counted in this slot.
    pub bytes: u64,
}

/// The counted slots of one metered link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeteredUsage {
    /// The rule this usage counts against (the same match as a `metered` entry).
    #[serde(rename = "match")]
    pub matcher: MeteredMatch,
    /// Oldest first, at most [`MAX_WINDOW_SLOTS`].
    #[serde(default)]
    pub slots: Vec<MeteredSlot>,
}

/// The persisted metered window (§12): bytes counted on `CLOCK_BOOTTIME`, which restarts at a
/// boot, so the slots are tagged with the boot they were counted in. How slots are bucketed and
/// aged is the budget's (`relay::metered`, row D11); this file only keeps them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeteredWindow {
    /// The kernel boot id the slots' `at_s` belong to (`None`: no slot counted yet).
    #[serde(default)]
    pub boot_id: Option<String>,
    /// One entry per metered link with counted bytes.
    #[serde(default)]
    pub links: Vec<MeteredUsage>,
}

impl MeteredWindow {
    fn check(&self) -> Result<(), SettingsError> {
        if let Some(b) = &self.boot_id {
            if b.is_empty()
                || b.len() > MAX_BOOT_ID
                || !b.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-')
            {
                return Err(SettingsError::BadWindow("boot_id is not a boot id"));
            }
        }
        if self.links.len() > MAX_METERED {
            return Err(SettingsError::BadWindow("too many links"));
        }
        for (i, l) in self.links.iter().enumerate() {
            l.matcher
                .check()
                .map_err(|_| SettingsError::BadWindow("a link's match is malformed"))?;
            if self.links[..i].iter().any(|o| o.matcher == l.matcher) {
                return Err(SettingsError::BadWindow("a link is listed twice"));
            }
            if l.slots.len() > MAX_WINDOW_SLOTS {
                return Err(SettingsError::BadWindow("too many slots"));
            }
            if l.slots.windows(2).any(|w| w[0].at_s > w[1].at_s) {
                return Err(SettingsError::BadWindow("slots are not oldest first"));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- the settings

/// The `state/relay.json` document (§6.3): every Layer 7 setting and the state beside it. Built
/// at its defaults by [`Default`] (all off); changed only through the checked setters, so a
/// value held here is always one the file accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelaySettings {
    gateway_enabled: bool,
    termux_poll_min: u32,
    carrier_kind: CarrierKind,
    ntfy_server: Option<String>,
    ntfy_token: Option<Secret>,
    ntfy_topic_secret: Option<Secret>,
    carrier_mode: CarrierMode,
    carrier_consent: Option<String>,
    heartbeat: BTreeMap<String, u32>,
    lan_record: bool,
    public_gateway_bit: bool,
    offload_bit: bool,
    origin_serve: bool,
    origin_targets: Vec<String>,
    check_internet: bool,
    metered: Vec<MeteredLink>,
    consent_gateway_bit: Option<String>,
    consent_origin_serve: Option<String>,
    drain: BTreeMap<String, String>,
    metered_window: MeteredWindow,
}

impl Default for RelaySettings {
    /// Every §6.3 default: all off, `termux_poll_min` 15, mode 5A, the own target, the internet
    /// check on.
    fn default() -> Self {
        RelaySettings {
            gateway_enabled: false,
            termux_poll_min: DEFAULT_TERMUX_POLL_MIN,
            carrier_kind: CarrierKind::None,
            ntfy_server: None,
            ntfy_token: None,
            ntfy_topic_secret: None,
            carrier_mode: CarrierMode::FiveA,
            carrier_consent: None,
            heartbeat: BTreeMap::new(),
            lan_record: false,
            public_gateway_bit: false,
            offload_bit: false,
            origin_serve: false,
            origin_targets: vec![TARGET_TRIPLE.to_string()],
            check_internet: true,
            metered: Vec::new(),
            consent_gateway_bit: None,
            consent_origin_serve: None,
            drain: BTreeMap::new(),
            metered_window: MeteredWindow::default(),
        }
    }
}

impl RelaySettings {
    // ---- gateway (§3.1, §3.4)

    /// `gateway.enabled`. The "true only on a hub" rule is the route's (`409 needs_hub`, D10)
    /// and the builder's (D11), which know the category.
    pub fn gateway_enabled(&self) -> bool {
        self.gateway_enabled
    }
    /// Set `gateway.enabled`.
    pub fn set_gateway_enabled(&mut self, on: bool) {
        self.gateway_enabled = on;
    }
    /// `gateway.termux_poll_min` (§3.4; read only on a Termux hub whose netlink bind failed).
    pub fn termux_poll_min(&self) -> u32 {
        self.termux_poll_min
    }
    /// Set `gateway.termux_poll_min` (9 to 1440).
    pub fn set_termux_poll_min(&mut self, min: u32) -> Result<(), SettingsError> {
        check_termux_poll(min)?;
        self.termux_poll_min = min;
        Ok(())
    }

    // ---- carrier (§6.2, §6.3)

    /// `carrier.kind`.
    pub fn carrier_kind(&self) -> CarrierKind {
        self.carrier_kind
    }
    /// Set `carrier.kind`.
    pub fn set_carrier_kind(&mut self, kind: CarrierKind) {
        self.carrier_kind = kind;
    }
    /// `carrier.ntfy.server` (as stored, without a trailing `/`).
    pub fn ntfy_server(&self) -> Option<&str> {
        self.ntfy_server.as_deref()
    }
    /// Set `carrier.ntfy.server`: `https://`, or `http://` to a private, loopback or link-local IP
    /// literal (or `localhost`) only; `None` clears it.
    pub fn set_ntfy_server(&mut self, url: Option<&str>) -> Result<(), SettingsError> {
        self.ntfy_server = url.map(check_server).transpose()?;
        Ok(())
    }
    /// Is `carrier.ntfy.server` an `http://` URL (the token then crosses that LAN in clear, S12)?
    pub fn ntfy_server_plain(&self) -> bool {
        self.ntfy_server
            .as_deref()
            .is_some_and(|s| s.starts_with("http://"))
    }
    /// `carrier.ntfy.token` (write-only: never put it in a view, log or frame).
    pub fn ntfy_token(&self) -> Option<&Secret> {
        self.ntfy_token.as_ref()
    }
    /// Set `carrier.ntfy.token` (1 to 512 printable ASCII); `None` clears it. The argument is
    /// moved in and zeroized with the settings.
    pub fn set_ntfy_token(
        &mut self,
        token: Option<Zeroizing<String>>,
    ) -> Result<(), SettingsError> {
        self.ntfy_token = token.map(check_token).transpose()?;
        Ok(())
    }
    /// Set `carrier.ntfy.topic_secret` (32 B of RFC 4648 base32, any case, padding optional;
    /// stored lowercase unpadded); `None` clears it, and the X25519 output is the key again
    /// (§6.2).
    pub fn set_ntfy_topic_secret(&mut self, b32: Option<&str>) -> Result<(), SettingsError> {
        self.ntfy_topic_secret = b32.map(check_topic_secret).transpose()?;
        Ok(())
    }
    /// Is `carrier.ntfy.topic_secret` set?
    pub fn ntfy_topic_secret_set(&self) -> bool {
        self.ntfy_topic_secret.is_some()
    }
    /// `carrier.ntfy.topic_secret` as the 32-byte key that replaces the X25519 output (§6.2).
    pub fn ntfy_topic_secret(&self) -> Option<Zeroizing<[u8; TOPIC_SECRET_LEN]>> {
        let s = self.ntfy_topic_secret.as_ref()?;
        base32_decode_32(s.expose())
    }
    /// `carrier.mode` as stored.
    pub fn carrier_mode(&self) -> CarrierMode {
        self.carrier_mode
    }
    /// The mode to run: a 5B read on a Termux hub (a file edited by hand) runs as 5A, the lighter
    /// one (§3.1; fail closed).
    pub fn carrier_mode_for(&self, termux: bool) -> CarrierMode {
        if termux {
            CarrierMode::FiveA
        } else {
            self.carrier_mode
        }
    }
    /// Set `carrier.mode`; 5B is refused on a Termux hub (§3.1, §6.3).
    pub fn set_carrier_mode(
        &mut self,
        mode: CarrierMode,
        termux: bool,
    ) -> Result<(), SettingsError> {
        if termux && mode == CarrierMode::FiveB {
            return Err(SettingsError::ModeRefusedOnTermux);
        }
        self.carrier_mode = mode;
        Ok(())
    }

    // ---- consents (§12)

    /// The recorded hash of `kind`'s consent text (`carrier.consent` for `carrier_enable`).
    pub fn consent_hash(&self, kind: ConsentKind) -> Option<&str> {
        match kind {
            ConsentKind::CarrierEnable => self.carrier_consent.as_deref(),
            ConsentKind::GatewayBit => self.consent_gateway_bit.as_deref(),
            ConsentKind::OriginServe => self.consent_origin_serve.as_deref(),
        }
    }
    /// Record (or, with `None`, forget) the hash of the text the human accepted (lowercase hex
    /// SHA-256, the contract's `Sha256Hex`).
    pub fn set_consent_hash(
        &mut self,
        kind: ConsentKind,
        hash: Option<&str>,
    ) -> Result<(), SettingsError> {
        let h = hash.map(check_hash).transpose()?;
        match kind {
            ConsentKind::CarrierEnable => self.carrier_consent = h,
            ConsentKind::GatewayBit => self.consent_gateway_bit = h,
            ConsentKind::OriginServe => self.consent_origin_serve = h,
        }
        Ok(())
    }

    // ---- presence (§2.4)

    /// `presence.heartbeat.<transport>` in minutes; `0` (events only) for every transport by
    /// default. There is no setter: every v1 write is refused (§2.4 item 2), and nothing reads
    /// this to build a timer in v1.
    pub fn heartbeat_min(&self, transport: &str) -> u32 {
        self.heartbeat.get(transport).copied().unwrap_or(0)
    }

    // ---- discovery (§7.1, §7.2)

    /// `discovery.lan_record`.
    pub fn lan_record(&self) -> bool {
        self.lan_record
    }
    /// Set `discovery.lan_record`.
    pub fn set_lan_record(&mut self, on: bool) {
        self.lan_record = on;
    }
    /// `discovery.public_gateway_bit` (needs its `gateway_bit` consent; the route's check).
    pub fn public_gateway_bit(&self) -> bool {
        self.public_gateway_bit
    }
    /// Set `discovery.public_gateway_bit`.
    pub fn set_public_gateway_bit(&mut self, on: bool) {
        self.public_gateway_bit = on;
    }
    /// `discovery.offload_bit`.
    pub fn offload_bit(&self) -> bool {
        self.offload_bit
    }
    /// Set `discovery.offload_bit`.
    pub fn set_offload_bit(&mut self, on: bool) {
        self.offload_bit = on;
    }

    // ---- origin (§9.1) and updates (§9.3)

    /// `origin.serve` (needs `hub_role`, LAN sharing and its `origin_serve` consent; the route's
    /// and the server's checks).
    pub fn origin_serve(&self) -> bool {
        self.origin_serve
    }
    /// Set `origin.serve`.
    pub fn set_origin_serve(&mut self, on: bool) {
        self.origin_serve = on;
    }
    /// `origin.targets`: the hub's own target first, always (it holds its own payload, §9.1),
    /// then the others listed.
    pub fn origin_targets(&self) -> &[String] {
        &self.origin_targets
    }
    /// Set `origin.targets` (target triples, at most [`MAX_TARGETS`] with the own one); the own
    /// target is put first when missing, duplicates are dropped.
    pub fn set_origin_targets<S: AsRef<str>>(
        &mut self,
        targets: &[S],
    ) -> Result<(), SettingsError> {
        self.origin_targets = normalise_targets(targets.iter().map(AsRef::as_ref))?;
        Ok(())
    }
    /// `updates.check_internet` (`false`: the hourly loop contacts no internet base, §9.3).
    pub fn check_internet(&self) -> bool {
        self.check_internet
    }
    /// Set `updates.check_internet`.
    pub fn set_check_internet(&mut self, on: bool) {
        self.check_internet = on;
    }

    // ---- metered (§12)

    /// `metered`: the user's metered links.
    pub fn metered(&self) -> &[MeteredLink] {
        &self.metered
    }
    /// Replace `metered` (at most [`MAX_METERED`], each match once, caps ≥ 1). The window keeps
    /// only the counts of links still listed.
    pub fn set_metered(&mut self, links: Vec<MeteredLink>) -> Result<(), SettingsError> {
        check_metered(&links)?;
        self.metered_window
            .links
            .retain(|u| links.iter().any(|l| l.matcher == u.matcher));
        self.metered = links;
        Ok(())
    }
    /// The persisted metered window.
    pub fn metered_window(&self) -> &MeteredWindow {
        &self.metered_window
    }
    /// Replace the metered window (row D11's budget, after it counted bytes).
    pub fn set_metered_window(&mut self, window: MeteredWindow) -> Result<(), SettingsError> {
        window.check()?;
        self.metered_window = window;
        Ok(())
    }

    // ---- the drain's last id per topic (§6.2, C13)

    /// The last ntfy id the fabric took on `topic`.
    pub fn drain_cursor(&self, topic: &str) -> Option<&str> {
        self.drain.get(topic).map(String::as_str)
    }
    /// Every drain cursor, by topic.
    pub fn drain_cursors(&self) -> impl Iterator<Item = (&str, &str)> {
        self.drain.iter().map(|(t, i)| (t.as_str(), i.as_str()))
    }
    /// Save (or, with `None`, forget) the last id the fabric took on `topic`; call it only once
    /// the fabric took the batch (C13). At most [`MAX_DRAIN_TOPICS`] topics.
    pub fn set_drain_cursor(&mut self, topic: &str, id: Option<&str>) -> Result<(), SettingsError> {
        if !valid_topic(topic) {
            return Err(SettingsError::BadDrain);
        }
        match id {
            None => {
                self.drain.remove(topic);
            }
            Some(i) => {
                if !valid_drain_id(i) {
                    return Err(SettingsError::BadDrain);
                }
                if !self.drain.contains_key(topic) && self.drain.len() >= MAX_DRAIN_TOPICS {
                    return Err(SettingsError::TooManyTopics);
                }
                self.drain.insert(topic.to_string(), i.to_string());
            }
        }
        Ok(())
    }

    // ---- the view

    /// The settings as every API, log and SSE frame may show them (§13.1): the secrets as
    /// `{"set": bool}`, never their value. Row D10 builds `RelayStatus` around it (the badge's
    /// live facts, the pairs, the counters, `origin.bytes`, the `rns` heartbeat row).
    pub fn view(&self) -> SettingsView {
        SettingsView {
            gateway: GatewayView {
                enabled: self.gateway_enabled,
                termux_poll_min: self.termux_poll_min,
            },
            carrier: CarrierView {
                kind: self.carrier_kind,
                server: self.ntfy_server.clone(),
                mode: self.carrier_mode,
                token: SecretView {
                    set: self.ntfy_token.is_some(),
                },
                topic_secret: SecretView {
                    set: self.ntfy_topic_secret.is_some(),
                },
                consent_hash: self.carrier_consent.clone(),
            },
            presence: PresenceView {
                heartbeat: HEARTBEAT_TRANSPORTS
                    .iter()
                    .map(|t| HeartbeatView {
                        transport: (*t).to_string(),
                        interval_min: self.heartbeat_min(t),
                        owner: "layer7",
                    })
                    .collect(),
            },
            discovery: DiscoveryView {
                lan_record: self.lan_record,
                public_gateway_bit: self.public_gateway_bit,
                offload_bit: self.offload_bit,
            },
            origin: OriginView {
                serve: self.origin_serve,
                targets: self.origin_targets.clone(),
            },
            updates: UpdatesView {
                check_internet: self.check_internet,
            },
            metered: self.metered.clone(),
        }
    }
}

// ---------------------------------------------------------------- the view (§13.1)

/// The secret-free view of the settings (the contract's `RelayStatus` keys that come from this
/// file: `gateway.enabled`, `gateway.termux_poll_min`, `carrier`, `presence`, `discovery`,
/// `origin.serve`, `origin.targets`, `updates`, `metered`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsView {
    /// `gateway.*`.
    pub gateway: GatewayView,
    /// The contract's `RelayCarrier`.
    pub carrier: CarrierView,
    /// `presence.heartbeat`, the Layer 7 transports (the `rns` row is the status route's).
    pub presence: PresenceView,
    /// The contract's `RelayDiscovery`.
    pub discovery: DiscoveryView,
    /// `origin.serve`, `origin.targets` (the status adds `bytes`).
    pub origin: OriginView,
    /// `updates.*`.
    pub updates: UpdatesView,
    /// `metered`.
    pub metered: Vec<MeteredLink>,
}

/// `gateway.*` settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct GatewayView {
    /// `gateway.enabled`.
    pub enabled: bool,
    /// `gateway.termux_poll_min`.
    pub termux_poll_min: u32,
}

/// The contract's `RelaySecret`: whether a secret is stored, never its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SecretView {
    /// A value is stored.
    pub set: bool,
}

/// The contract's `RelayCarrier`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CarrierView {
    /// `carrier.kind`.
    pub kind: CarrierKind,
    /// `carrier.ntfy.server` (untrusted: rendered as text).
    pub server: Option<String>,
    /// `carrier.mode`.
    pub mode: CarrierMode,
    /// `carrier.ntfy.token`: `{"set": …}` only.
    pub token: SecretView,
    /// `carrier.ntfy.topic_secret`: `{"set": …}` only.
    pub topic_secret: SecretView,
    /// `carrier.consent`.
    pub consent_hash: Option<String>,
}

/// `presence.*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresenceView {
    /// One row per Layer 7 transport (the contract's `RelayHeartbeat`).
    pub heartbeat: Vec<HeartbeatView>,
}

/// The contract's `RelayHeartbeat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeartbeatView {
    /// The transport.
    pub transport: String,
    /// Minutes; `0` = events only.
    pub interval_min: u32,
    /// `layer7` (the `rns` row the status route adds says `layer6`).
    pub owner: &'static str,
}

/// The contract's `RelayDiscovery`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DiscoveryView {
    /// `discovery.lan_record`.
    pub lan_record: bool,
    /// `discovery.public_gateway_bit`.
    pub public_gateway_bit: bool,
    /// `discovery.offload_bit`.
    pub offload_bit: bool,
}

/// `origin.serve` and `origin.targets`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OriginView {
    /// `origin.serve`.
    pub serve: bool,
    /// `origin.targets`, the own target first.
    pub targets: Vec<String>,
}

/// `updates.*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct UpdatesView {
    /// `updates.check_internet`.
    pub check_internet: bool,
}

// ---------------------------------------------------------------- checks

fn check_termux_poll(min: u32) -> Result<(), SettingsError> {
    if (MIN_TERMUX_POLL_MIN..=MAX_TERMUX_POLL_MIN).contains(&min) {
        Ok(())
    } else {
        Err(SettingsError::BadTermuxPoll)
    }
}

/// `carrier.ntfy.server` (§6.3): `https://`; `http://` only for a private address (§9.5 T6,
/// S12), which here means an IP literal in a private, loopback or link-local range, or
/// `localhost`; a host name over `http://` is refused (it could resolve anywhere). No query or
/// fragment, no userinfo. Returns the URL without a trailing `/`. (The carrier checks it again
/// when it is built, `N/carrier/ntfy.rs`.)
fn check_server(url: &str) -> Result<String, SettingsError> {
    if url.is_empty()
        || url.len() > MAX_SERVER_URL
        || url.bytes().any(|b| !(0x21..=0x7e).contains(&b))
        || url.contains(['?', '#', '@'])
    {
        return Err(SettingsError::BadServer);
    }
    let uri: ureq::http::Uri = url.parse().map_err(|_| SettingsError::BadServer)?;
    let host = uri.host().ok_or(SettingsError::BadServer)?;
    if host.is_empty() {
        return Err(SettingsError::BadServer);
    }
    match uri.scheme_str() {
        Some("https") => {}
        Some("http") if is_private_host(host) => {}
        _ => return Err(SettingsError::BadServer),
    }
    Ok(url.trim_end_matches('/').to_string())
}

fn is_private_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let v4_private =
        |v4: std::net::Ipv4Addr| v4.is_private() || v4.is_loopback() || v4.is_link_local();
    match bare.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4_private(v4),
        Ok(IpAddr::V6(v6)) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4_private(v4);
            }
            let s0 = v6.segments()[0];
            v6.is_loopback() || (s0 & 0xfe00) == 0xfc00 || (s0 & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

fn check_token(t: Zeroizing<String>) -> Result<Secret, SettingsError> {
    if t.is_empty() || t.len() > MAX_TOKEN || t.bytes().any(|b| !(0x21..=0x7e).contains(&b)) {
        return Err(SettingsError::BadToken);
    }
    Ok(Secret(t))
}

fn check_topic_secret(b32: &str) -> Result<Secret, SettingsError> {
    let key = base32_decode_32(b32).ok_or(SettingsError::BadTopicSecret)?;
    if key.iter().all(|&b| b == 0) {
        return Err(SettingsError::BadTopicSecret);
    }
    Ok(Secret(Zeroizing::new(base32_encode_lower(&key[..]))))
}

fn check_hash(h: &str) -> Result<String, SettingsError> {
    if h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(h.to_string())
    } else {
        Err(SettingsError::BadHash)
    }
}

fn valid_target(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= MAX_TARGET_LEN
        && t.contains('-')
        && !t.starts_with('-')
        && !t.ends_with('-')
        && t.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn normalise_targets<'a>(
    targets: impl Iterator<Item = &'a str>,
) -> Result<Vec<String>, SettingsError> {
    let mut out = vec![TARGET_TRIPLE.to_string()];
    for t in targets {
        if !valid_target(t) {
            return Err(SettingsError::BadTarget);
        }
        if !out.iter().any(|o| o == t) {
            if out.len() >= MAX_TARGETS {
                return Err(SettingsError::BadTarget);
            }
            out.push(t.to_string());
        }
    }
    Ok(out)
}

fn check_metered(links: &[MeteredLink]) -> Result<(), SettingsError> {
    if links.len() > MAX_METERED {
        return Err(SettingsError::BadMetered("at most 16 metered links"));
    }
    for (i, l) in links.iter().enumerate() {
        l.matcher.check()?;
        if l.daily_cap_bytes == 0 {
            return Err(SettingsError::BadMetered(
                "daily_cap_bytes must be at least 1",
            ));
        }
        if links[..i].iter().any(|o| o.matcher == l.matcher) {
            return Err(SettingsError::BadMetered("a link is listed twice"));
        }
    }
    Ok(())
}

/// A carrier topic (§6.2): `"111"` and 32 lowercase base32 characters.
fn valid_topic(t: &str) -> bool {
    t.len() == TOPIC_LEN
        && t.starts_with(TOPIC_PREFIX)
        && t[TOPIC_PREFIX.len()..]
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'))
}

/// An ntfy message id (the carrier's rule: it goes into a URL).
fn valid_drain_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_DRAIN_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

const B32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// RFC 4648 base32, lower case, unpadded.
fn base32_encode_lower(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &b in data {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32[((acc >> bits) & 31) as usize] as char);
        }
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(B32[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Exactly 32 bytes of RFC 4648 base32: 52 characters, any case, optionally padded to 56 with
/// `=`; the unused low bits of the last character must be zero (one spelling per key).
fn base32_decode_32(text: &str) -> Option<Zeroizing<[u8; TOPIC_SECRET_LEN]>> {
    let body = match text.len() {
        52 => text,
        56 if text.ends_with("====") => &text[..52],
        _ => return None,
    };
    let mut out = Zeroizing::new([0u8; TOPIC_SECRET_LEN]);
    let (mut acc, mut bits, mut n) = (0u32, 0u32, 0usize);
    for c in body.bytes() {
        let v = match c {
            b'a'..=b'z' => c - b'a',
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        };
        acc = (acc << 5) | u32::from(v);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            *out.get_mut(n)? = (acc >> bits) as u8;
            n += 1;
        }
        acc &= (1 << bits) - 1;
    }
    (n == TOPIC_SECRET_LEN && acc == 0).then_some(out)
}

// ---------------------------------------------------------------- the file

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GatewayFile {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_poll")]
    termux_poll_min: u32,
}

impl Default for GatewayFile {
    fn default() -> Self {
        GatewayFile {
            enabled: false,
            termux_poll_min: DEFAULT_TERMUX_POLL_MIN,
        }
    }
}

fn default_poll() -> u32 {
    DEFAULT_TERMUX_POLL_MIN
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NtfyFile {
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    topic_secret: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CarrierFile {
    #[serde(default)]
    kind: CarrierKind,
    #[serde(default)]
    mode: CarrierMode,
    #[serde(default)]
    consent: Option<String>,
    #[serde(default)]
    ntfy: NtfyFile,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PresenceFile {
    #[serde(default)]
    heartbeat: BTreeMap<String, u32>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscoveryFile {
    #[serde(default)]
    lan_record: bool,
    #[serde(default)]
    public_gateway_bit: bool,
    #[serde(default)]
    offload_bit: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OriginFile {
    #[serde(default)]
    serve: bool,
    #[serde(default)]
    targets: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdatesFile {
    #[serde(default = "default_true")]
    check_internet: bool,
}

impl Default for UpdatesFile {
    fn default() -> Self {
        UpdatesFile {
            check_internet: true,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConsentsFile {
    #[serde(default)]
    gateway_bit: Option<String>,
    #[serde(default)]
    origin_serve: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelayFile {
    schema: u32,
    #[serde(default)]
    gateway: GatewayFile,
    #[serde(default)]
    carrier: CarrierFile,
    #[serde(default)]
    presence: PresenceFile,
    #[serde(default)]
    discovery: DiscoveryFile,
    #[serde(default)]
    origin: OriginFile,
    #[serde(default)]
    updates: UpdatesFile,
    #[serde(default)]
    metered: Vec<MeteredLink>,
    #[serde(default)]
    consents: ConsentsFile,
    #[serde(default)]
    drain: BTreeMap<String, String>,
    #[serde(default)]
    metered_window: MeteredWindow,
}

/// A serde failure as a log-safe text: its category and position, never the value it quotes.
fn parse_error(e: &serde_json::Error) -> SettingsError {
    let what = match e.classify() {
        serde_json::error::Category::Io => "io",
        serde_json::error::Category::Syntax => "syntax",
        serde_json::error::Category::Data => "a key or value out of shape",
        serde_json::error::Category::Eof => "truncated",
    };
    SettingsError::Parse(format!("{what} at line {} column {}", e.line(), e.column()))
}

fn decode(text: &str) -> Result<RelaySettings, SettingsError> {
    let f: RelayFile = serde_json::from_str(text).map_err(|e| parse_error(&e))?;
    if f.schema != SCHEMA {
        return Err(SettingsError::Schema);
    }
    let mut s = RelaySettings {
        gateway_enabled: f.gateway.enabled,
        carrier_kind: f.carrier.kind,
        carrier_mode: f.carrier.mode,
        lan_record: f.discovery.lan_record,
        public_gateway_bit: f.discovery.public_gateway_bit,
        offload_bit: f.discovery.offload_bit,
        origin_serve: f.origin.serve,
        check_internet: f.updates.check_internet,
        ..RelaySettings::default()
    };
    s.set_termux_poll_min(f.gateway.termux_poll_min)?;
    s.set_ntfy_server(f.carrier.ntfy.server.as_deref())?;
    s.set_ntfy_token(f.carrier.ntfy.token.map(Zeroizing::new))?;
    let secret = f.carrier.ntfy.topic_secret.map(Zeroizing::new);
    s.set_ntfy_topic_secret(secret.as_ref().map(|z| z.as_str()))?;
    s.set_consent_hash(ConsentKind::CarrierEnable, f.carrier.consent.as_deref())?;
    s.set_consent_hash(ConsentKind::GatewayBit, f.consents.gateway_bit.as_deref())?;
    s.set_consent_hash(ConsentKind::OriginServe, f.consents.origin_serve.as_deref())?;
    for (t, m) in &f.presence.heartbeat {
        if !HEARTBEAT_TRANSPORTS.contains(&t.as_str()) || *m > MAX_HEARTBEAT_MIN {
            return Err(SettingsError::BadHeartbeat);
        }
    }
    s.heartbeat = f.presence.heartbeat;
    if let Some(t) = &f.origin.targets {
        s.set_origin_targets(t)?;
    }
    s.set_metered(f.metered)?;
    if f.drain.len() > MAX_DRAIN_TOPICS {
        return Err(SettingsError::TooManyTopics);
    }
    for (topic, id) in &f.drain {
        s.set_drain_cursor(topic, Some(id))?;
    }
    s.set_metered_window(f.metered_window)?;
    // A window entry for a link no longer listed is dropped, as `set_metered` does.
    s.metered_window
        .links
        .retain(|u| s.metered.iter().any(|l| l.matcher == u.matcher));
    Ok(s)
}

fn encode(s: &RelaySettings) -> Zeroizing<String> {
    let f = RelayFile {
        schema: SCHEMA,
        gateway: GatewayFile {
            enabled: s.gateway_enabled,
            termux_poll_min: s.termux_poll_min,
        },
        carrier: CarrierFile {
            kind: s.carrier_kind,
            mode: s.carrier_mode,
            consent: s.carrier_consent.clone(),
            ntfy: NtfyFile {
                server: s.ntfy_server.clone(),
                token: s.ntfy_token.as_ref().map(|t| t.expose().to_string()),
                topic_secret: s.ntfy_topic_secret.as_ref().map(|t| t.expose().to_string()),
            },
        },
        presence: PresenceFile {
            heartbeat: s.heartbeat.clone(),
        },
        discovery: DiscoveryFile {
            lan_record: s.lan_record,
            public_gateway_bit: s.public_gateway_bit,
            offload_bit: s.offload_bit,
        },
        origin: OriginFile {
            serve: s.origin_serve,
            targets: Some(s.origin_targets.clone()),
        },
        updates: UpdatesFile {
            check_internet: s.check_internet,
        },
        metered: s.metered.clone(),
        consents: ConsentsFile {
            gateway_bit: s.consent_gateway_bit.clone(),
            origin_serve: s.consent_origin_serve.clone(),
        },
        drain: s.drain.clone(),
        metered_window: s.metered_window.clone(),
    };
    let text = Zeroizing::new(serde_json::to_string_pretty(&f).unwrap_or_default());
    // The copies of the secrets in `f` are wiped before it is dropped.
    let RelayFile { carrier, .. } = f;
    let _ = carrier.ntfy.token.map(Zeroizing::new);
    let _ = carrier.ntfy.topic_secret.map(Zeroizing::new);
    let mut out = Zeroizing::new(String::with_capacity(text.len() + 1));
    out.push_str(&text);
    out.push('\n');
    out
}

/// The file path under the daemon's `state/` directory.
pub fn path(state_dir: &Path) -> PathBuf {
    state_dir.join(RELAY_FILE)
}

/// What [`load`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// The settings to run with (the defaults when the file was absent or invalid).
    pub settings: RelaySettings,
    /// `Some` when a file was present but invalid.
    pub invalid: Option<Invalid>,
}

/// An invalid file (module doc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    /// Why (log-safe: no value from the file).
    pub why: String,
    /// Where it was moved (`None`: the move failed, or not even its metadata could be read; it
    /// stays in place and the next write replaces it).
    pub moved_to: Option<PathBuf>,
}

/// Move the invalid file aside (never deleted; a symlink is moved itself, never followed).
fn move_aside(state_dir: &Path) -> Option<PathBuf> {
    let from = path(state_dir);
    for n in 1..=MOVE_ASIDE_TRIES {
        let to = state_dir.join(format!("{RELAY_FILE}.invalid-{n}"));
        if std::fs::symlink_metadata(&to).is_ok() {
            continue;
        }
        return match std::fs::rename(&from, &to) {
            Ok(()) => Some(to),
            Err(e) => {
                eprintln!("111 relay: cannot move {} aside: {e}", from.display());
                None
            }
        };
    }
    eprintln!(
        "111 relay: cannot move {} aside: no free {RELAY_FILE}.invalid-<n> name",
        from.display()
    );
    None
}

/// Load `state/relay.json`: absent ⇒ the defaults (nothing is written); invalid ⇒ the defaults
/// and the file moved aside (module doc). Never creates the file.
pub fn load(state_dir: &Path) -> Loaded {
    let p = path(state_dir);
    let invalid = |why: String| {
        let moved_to = move_aside(state_dir);
        match &moved_to {
            Some(to) => eprintln!(
                "111 relay: {} invalid ({why}); moved to {}; everything off",
                p.display(),
                to.display()
            ),
            None => eprintln!("111 relay: {} invalid ({why}); everything off", p.display()),
        }
        Loaded {
            settings: RelaySettings::default(),
            invalid: Some(Invalid { why, moved_to }),
        }
    };
    let meta = match std::fs::symlink_metadata(&p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Loaded {
                settings: RelaySettings::default(),
                invalid: None,
            }
        }
        Err(e) => {
            // Not even its metadata: leave it in place, run with all off.
            eprintln!(
                "111 relay: {} unreadable ({e}); everything off",
                p.display()
            );
            return Loaded {
                settings: RelaySettings::default(),
                invalid: Some(Invalid {
                    why: e.to_string(),
                    moved_to: None,
                }),
            };
        }
    };
    if !meta.file_type().is_file() || meta.len() > MAX_FILE {
        return invalid("not a small regular file".into());
    }
    let text = match read_capped(&p) {
        Ok(t) => t,
        Err(e) => return invalid(e),
    };
    match decode(&text) {
        Ok(settings) => Loaded {
            settings,
            invalid: None,
        },
        Err(e) => invalid(e.to_string()),
    }
}

/// Read at most [`MAX_FILE`] bytes (the file may grow between the metadata check and the read),
/// without following a symlink swapped in meanwhile.
fn read_capped(p: &Path) -> Result<Zeroizing<String>, String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(p)
        .map_err(|e| e.to_string())?;
    let mut buf = Zeroizing::new(Vec::new());
    f.take(MAX_FILE + 1)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if buf.len() as u64 > MAX_FILE {
        return Err("not a small regular file".into());
    }
    let text = std::str::from_utf8(&buf).map_err(|_| "not UTF-8".to_string())?;
    Ok(Zeroizing::new(text.to_string()))
}

/// Write `state/relay.json` atomically (0600). Prefer [`update`], which writes only on a change.
pub fn save(state_dir: &Path, s: &RelaySettings) -> anyhow::Result<()> {
    let text = encode(s);
    crate::util::fs::atomic_write(&path(state_dir), text.as_bytes(), 0o600)
}

/// Read-modify-write under the process's write lock: load the file (absent or invalid ⇒ the
/// defaults, as [`load`]), apply `change`, and write only when the settings differ from what was
/// loaded (and no file is created while they are still the defaults of an absent file). Returns
/// the settings after the change and whether a write happened. A `change` that fails leaves the
/// file untouched.
pub fn update<E>(
    state_dir: &Path,
    change: impl FnOnce(&mut RelaySettings) -> Result<(), E>,
) -> Result<(RelaySettings, bool), UpdateError<E>> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let before = load(state_dir).settings;
    let mut after = before.clone();
    change(&mut after).map_err(UpdateError::Change)?;
    if after == before {
        return Ok((after, false));
    }
    save(state_dir, &after).map_err(|e| UpdateError::Write(e.to_string()))?;
    Ok((after, true))
}

/// Why [`update`] failed.
#[derive(Debug)]
pub enum UpdateError<E> {
    /// The change refused itself (nothing written).
    Change(E),
    /// The write failed (the file is as it was).
    Write(String),
}

impl<E: std::fmt::Display> std::fmt::Display for UpdateError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::Change(e) => e.fmt(f),
            UpdateError::Write(e) => write!(f, "cannot write {RELAY_FILE}: {e}"),
        }
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for UpdateError<E> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const TOPIC: &str = "111abcdefghijklmnopqrstuvwxyz234567";

    fn secret_b32(byte: u8) -> String {
        base32_encode_lower(&[byte; 32])
    }

    fn entries(d: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn defaults_are_all_off() {
        let s = RelaySettings::default();
        assert!(!s.gateway_enabled());
        assert_eq!(s.termux_poll_min(), 15);
        assert_eq!(s.carrier_kind(), CarrierKind::None);
        assert_eq!(s.ntfy_server(), None);
        assert!(s.ntfy_token().is_none());
        assert!(!s.ntfy_topic_secret_set());
        assert!(s.ntfy_topic_secret().is_none());
        assert_eq!(s.carrier_mode(), CarrierMode::FiveA);
        for k in [
            ConsentKind::CarrierEnable,
            ConsentKind::GatewayBit,
            ConsentKind::OriginServe,
        ] {
            assert_eq!(s.consent_hash(k), None);
        }
        for t in HEARTBEAT_TRANSPORTS.iter().chain(["rns"].iter()) {
            assert_eq!(s.heartbeat_min(t), 0, "{t}: events only");
        }
        assert!(!s.lan_record() && !s.public_gateway_bit() && !s.offload_bit());
        assert!(!s.origin_serve());
        assert_eq!(s.origin_targets(), [TARGET_TRIPLE.to_string()]);
        assert!(s.check_internet());
        assert!(s.metered().is_empty());
        assert_eq!(s.drain_cursors().count(), 0);
        assert_eq!(s.metered_window(), &MeteredWindow::default());
    }

    #[test]
    fn absent_file_reads_defaults_and_creates_nothing() {
        let t = tempfile::tempdir().unwrap();
        let l = load(t.path());
        assert_eq!(l.settings, RelaySettings::default());
        assert!(l.invalid.is_none());
        assert!(entries(t.path()).is_empty(), "load never writes");
        // An update that changes nothing writes nothing.
        let (s, wrote) = update::<SettingsError>(t.path(), |s| s.set_termux_poll_min(15)).unwrap();
        assert!(!wrote);
        assert_eq!(s, RelaySettings::default());
        assert!(
            entries(t.path()).is_empty(),
            "no file until the first write"
        );
        // A refused change writes nothing either.
        let e = update(t.path(), |s| s.set_termux_poll_min(8)).unwrap_err();
        assert!(matches!(
            e,
            UpdateError::Change(SettingsError::BadTermuxPoll)
        ));
        assert!(entries(t.path()).is_empty());
    }

    #[test]
    fn first_write_is_0600_schema_1_and_round_trips() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        let (_, wrote) = update::<SettingsError>(d, |s| {
            s.set_gateway_enabled(true);
            s.set_carrier_kind(CarrierKind::Ntfy);
            s.set_ntfy_server(Some("https://ntfy.example.org/"))?;
            s.set_ntfy_token(Some(Zeroizing::new("tk_abc123".into())))?;
            s.set_ntfy_topic_secret(Some(&secret_b32(0x42).to_uppercase()))?;
            s.set_carrier_mode(CarrierMode::FiveB, false)?;
            s.set_consent_hash(ConsentKind::CarrierEnable, Some(HASH))?;
            s.set_consent_hash(ConsentKind::GatewayBit, Some(HASH))?;
            s.set_public_gateway_bit(true);
            s.set_lan_record(true);
            s.set_offload_bit(true);
            s.set_origin_serve(true);
            s.set_origin_targets(&["aarch64-unknown-linux-musl", "aarch64-unknown-linux-musl"])?;
            s.set_check_internet(false);
            s.set_termux_poll_min(30)?;
            s.set_metered(vec![MeteredLink {
                matcher: MeteredMatch::Iface("wwan0".into()),
                daily_cap_bytes: 1_000_000,
            }])?;
            s.set_drain_cursor(TOPIC, Some("aBc123XyZ789"))?;
            s.set_metered_window(MeteredWindow {
                boot_id: Some("0f8e2d1c-aaaa-bbbb-cccc-0123456789ab".into()),
                links: vec![MeteredUsage {
                    matcher: MeteredMatch::Iface("wwan0".into()),
                    slots: vec![
                        MeteredSlot {
                            at_s: 10,
                            bytes: 400,
                        },
                        MeteredSlot {
                            at_s: 70,
                            bytes: 900,
                        },
                    ],
                }],
            })
        })
        .unwrap();
        assert!(wrote);
        let p = path(d);
        let meta = std::fs::metadata(&p).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        let text = std::fs::read_to_string(&p).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["schema"], 1);
        assert_eq!(v["carrier"]["ntfy"]["server"], "https://ntfy.example.org");
        assert_eq!(v["carrier"]["ntfy"]["topic_secret"], secret_b32(0x42));
        let l = load(d);
        assert!(l.invalid.is_none());
        let s = l.settings;
        assert!(s.gateway_enabled());
        assert_eq!(s.ntfy_token().unwrap().expose(), "tk_abc123");
        assert_eq!(*s.ntfy_topic_secret().unwrap(), [0x42u8; 32]);
        assert_eq!(s.carrier_mode(), CarrierMode::FiveB);
        assert_eq!(
            s.carrier_mode_for(true),
            CarrierMode::FiveA,
            "never 5B on Termux"
        );
        assert_eq!(s.consent_hash(ConsentKind::CarrierEnable), Some(HASH));
        assert_eq!(s.consent_hash(ConsentKind::GatewayBit), Some(HASH));
        assert_eq!(s.consent_hash(ConsentKind::OriginServe), None);
        let mut want = vec![TARGET_TRIPLE.to_string()];
        if TARGET_TRIPLE != "aarch64-unknown-linux-musl" {
            want.push("aarch64-unknown-linux-musl".into());
        }
        assert_eq!(s.origin_targets(), want, "own first, no duplicate");
        assert!(!s.check_internet());
        assert_eq!(s.termux_poll_min(), 30);
        assert_eq!(s.drain_cursor(TOPIC), Some("aBc123XyZ789"));
        assert_eq!(s.metered_window().links[0].slots.len(), 2);
        // Writing the same settings again is no write.
        let before = std::fs::metadata(&p).unwrap().modified().unwrap();
        let (_, wrote) = update::<SettingsError>(d, |_| Ok(())).unwrap();
        assert!(!wrote);
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), before);
        assert_eq!(entries(d), [RELAY_FILE.to_string()], "no tmp left behind");
    }

    #[test]
    fn a_partial_file_takes_defaults_for_missing_keys() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(
            path(t.path()),
            r#"{"schema":1,"discovery":{"lan_record":true}}"#,
        )
        .unwrap();
        let l = load(t.path());
        assert!(l.invalid.is_none());
        let mut want = RelaySettings::default();
        want.set_lan_record(true);
        assert_eq!(l.settings, want);
    }

    #[test]
    fn invalid_files_are_moved_aside_and_read_all_off() {
        let cases: Vec<(&str, String)> = vec![
            ("not json", "{".into()),
            ("schema 2", r#"{"schema":2}"#.into()),
            ("no schema", r#"{"gateway":{"enabled":true}}"#.into()),
            ("unknown key", r#"{"schema":1,"gatewy":{}}"#.into()),
            ("unknown nested key", r#"{"schema":1,"carrier":{"knd":"ntfy"}}"#.into()),
            ("chatmail before §6.5", r#"{"schema":1,"carrier":{"kind":"chatmail"}}"#.into()),
            ("mode 5C", r#"{"schema":1,"carrier":{"mode":"5C"}}"#.into()),
            ("poll under 9", r#"{"schema":1,"gateway":{"termux_poll_min":8}}"#.into()),
            (
                "http to a public host",
                r#"{"schema":1,"carrier":{"ntfy":{"server":"http://ntfy.sh"}}}"#.into(),
            ),
            ("bad consent hash", r#"{"schema":1,"consents":{"gateway_bit":"ABC"}}"#.into()),
            (
                "heartbeat on rns",
                r#"{"schema":1,"presence":{"heartbeat":{"rns":60}}}"#.into(),
            ),
            ("bad target", r#"{"schema":1,"origin":{"targets":["../x"]}}"#.into()),
            (
                "metered without a match",
                r#"{"schema":1,"metered":[{"match":{},"daily_cap_bytes":1}]}"#.into(),
            ),
            (
                "metered with both",
                r#"{"schema":1,"metered":[{"match":{"iface":"a","ssid":"b"},"daily_cap_bytes":1}]}"#
                    .into(),
            ),
            (
                "metered cap 0",
                r#"{"schema":1,"metered":[{"match":{"iface":"a"},"daily_cap_bytes":0}]}"#.into(),
            ),
            ("drain topic", r#"{"schema":1,"drain":{"x":"abc"}}"#.into()),
            (
                "drain id",
                format!(r#"{{"schema":1,"drain":{{"{TOPIC}":"a/b"}}}}"#),
            ),
            (
                "window out of order",
                r#"{"schema":1,"metered_window":{"links":[{"match":{"iface":"a"},"slots":[{"at_s":5,"bytes":1},{"at_s":4,"bytes":1}]}]}}"#
                    .into(),
            ),
            (
                "zero topic secret",
                format!(
                    r#"{{"schema":1,"carrier":{{"ntfy":{{"topic_secret":"{}"}}}}}}"#,
                    secret_b32(0)
                ),
            ),
        ];
        for (name, text) in cases {
            let t = tempfile::tempdir().unwrap();
            let d = t.path();
            std::fs::write(path(d), &text).unwrap();
            let l = load(d);
            assert_eq!(l.settings, RelaySettings::default(), "{name}");
            let inv = l.invalid.unwrap_or_else(|| panic!("{name}: not refused"));
            assert_eq!(inv.moved_to, Some(d.join("relay.json.invalid-1")), "{name}");
            assert_eq!(
                std::fs::read_to_string(d.join("relay.json.invalid-1")).unwrap(),
                text
            );
            assert!(!path(d).exists(), "{name}: moved, not copied");
        }
    }

    #[test]
    fn a_parse_failure_never_logs_a_value() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(
            path(t.path()),
            r#"{"schema":1,"carrier":{"ntfy":{"token":"tk_SECRETVALUE","topic_secret":7}}}"#,
        )
        .unwrap();
        let inv = load(t.path()).invalid.unwrap();
        assert!(!inv.why.contains("SECRETVALUE"), "{}", inv.why);
        let e = decode(r#"{"schema":1,"carrier":{"kind":"tk_SECRETVALUE"}}"#).unwrap_err();
        assert!(!e.to_string().contains("SECRETVALUE"), "{e}");
    }

    #[test]
    fn symlinks_directories_and_oversize_files_are_refused() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        let target = d.join("elsewhere");
        std::fs::write(&target, r#"{"schema":1,"gateway":{"enabled":true}}"#).unwrap();
        std::os::unix::fs::symlink(&target, path(d)).unwrap();
        let l = load(d);
        assert_eq!(l.settings, RelaySettings::default());
        assert!(l.invalid.is_some());
        assert!(std::fs::symlink_metadata(d.join("relay.json.invalid-1"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(target.exists(), "the link's target is never touched");

        std::fs::create_dir(path(d)).unwrap();
        assert!(load(d).invalid.is_some());
        assert!(d.join("relay.json.invalid-2").is_dir());

        let mut big = String::from(r#"{"schema":1,"drain":{},"x":""#);
        big.push_str(&"a".repeat(MAX_FILE as usize));
        big.push_str("\"}");
        std::fs::write(path(d), big).unwrap();
        let l = load(d);
        assert_eq!(
            l.invalid.unwrap().moved_to,
            Some(d.join("relay.json.invalid-3"))
        );
    }

    #[test]
    fn an_update_after_an_invalid_file_starts_from_all_off() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        std::fs::write(
            path(d),
            format!(r#"{{"schema":1,"carrier":{{"consent":"{HASH}","mode":"5Z"}}}}"#),
        )
        .unwrap();
        let (s, wrote) = update::<SettingsError>(d, |s| {
            s.set_lan_record(true);
            Ok(())
        })
        .unwrap();
        assert!(wrote);
        assert_eq!(
            s.consent_hash(ConsentKind::CarrierEnable),
            None,
            "no consent survives"
        );
        assert!(d.join("relay.json.invalid-1").exists());
        assert!(load(d).settings.lan_record());
    }

    #[test]
    fn server_rule() {
        let mut s = RelaySettings::default();
        for ok in [
            "https://ntfy.example.org",
            "https://ntfy.example.org:8443/base/",
            "http://192.168.1.10:8080",
            "http://10.0.0.1",
            "http://172.16.5.5",
            "http://127.0.0.1:2586",
            "http://localhost:2586",
            "http://169.254.1.1",
            "http://[fd00::1]:80",
            "http://[fe80::1]",
            "http://[::1]",
            "http://[::ffff:192.168.0.1]",
        ] {
            assert!(s.set_ntfy_server(Some(ok)).is_ok(), "{ok}");
            assert!(!s.ntfy_server().unwrap().ends_with('/'));
        }
        assert!(s.ntfy_server_plain());
        for bad in [
            "",
            "ntfy.example.org",
            "ftp://ntfy.example.org",
            "http://ntfy.example.org",
            "http://8.8.8.8",
            "http://[2001:db8::1]",
            "http://172.32.0.1",
            "https://ntfy.example.org/?x=1",
            "https://ntfy.example.org/#f",
            "https://user:pw@ntfy.example.org",
            "http://user@192.168.1.1",
            "https://ntfy example.org",
            "https://",
        ] {
            assert_eq!(
                s.set_ntfy_server(Some(bad)),
                Err(SettingsError::BadServer),
                "{bad}"
            );
        }
        let long = format!("https://{}.org", "a".repeat(MAX_SERVER_URL));
        assert_eq!(
            s.set_ntfy_server(Some(&long)),
            Err(SettingsError::BadServer)
        );
        s.set_ntfy_server(None).unwrap();
        assert_eq!(s.ntfy_server(), None);
    }

    #[test]
    fn token_and_topic_secret_rules() {
        let mut s = RelaySettings::default();
        for bad in ["", "tk with space", "tk\r\nX-Injected: 1", "tk\u{e9}"] {
            assert_eq!(
                s.set_ntfy_token(Some(Zeroizing::new(bad.into()))),
                Err(SettingsError::BadToken),
                "{bad:?}"
            );
        }
        assert_eq!(
            s.set_ntfy_token(Some(Zeroizing::new("a".repeat(MAX_TOKEN + 1)))),
            Err(SettingsError::BadToken)
        );
        s.set_ntfy_token(Some(Zeroizing::new("tk_ok".into())))
            .unwrap();
        s.set_ntfy_token(None).unwrap();
        assert!(s.ntfy_token().is_none());

        let b = secret_b32(0x5a);
        assert_eq!(b.len(), 52);
        s.set_ntfy_topic_secret(Some(&b)).unwrap();
        s.set_ntfy_topic_secret(Some(&format!("{}====", b.to_uppercase())))
            .unwrap();
        assert_eq!(*s.ntfy_topic_secret().unwrap(), [0x5a; 32]);
        for bad in [
            String::new(),
            b[..51].to_string(),
            format!("{b}a"),
            format!("{}1", &b[..51]),
            format!("{}===", b),
            secret_b32(0),
        ] {
            assert_eq!(
                s.set_ntfy_topic_secret(Some(&bad)),
                Err(SettingsError::BadTopicSecret),
                "{bad:?}"
            );
        }
        // Non-canonical trailing bits (the last character carries 4 unused bits).
        let mut nc = b.clone().into_bytes();
        let last = nc.last_mut().unwrap();
        let v = B32.iter().position(|c| c == last).unwrap();
        *last = B32[v | 1];
        let nc = String::from_utf8(nc).unwrap();
        assert!(base32_decode_32(&nc).is_none(), "a non-canonical spelling");
    }

    #[test]
    fn base32_matches_rfc4648() {
        // RFC 4648 §10 vectors, lowercased and unpadded.
        for (input, want) in [
            (&b""[..], ""),
            (b"f", "my"),
            (b"fo", "mzxq"),
            (b"foo", "mzxw6"),
            (b"foob", "mzxw6yq"),
            (b"fooba", "mzxw6ytb"),
            (b"foobar", "mzxw6ytboi"),
        ] {
            assert_eq!(base32_encode_lower(input), want);
        }
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        let enc = base32_encode_lower(&key);
        assert_eq!(*base32_decode_32(&enc).unwrap(), key);
    }

    #[test]
    fn secrets_never_reach_a_view_or_debug() {
        let mut s = RelaySettings::default();
        s.set_ntfy_token(Some(Zeroizing::new("tk_SECRETVALUE".into())))
            .unwrap();
        let ts = secret_b32(0x77);
        s.set_ntfy_topic_secret(Some(&ts)).unwrap();
        let v = s.view();
        assert!(v.carrier.token.set && v.carrier.topic_secret.set);
        let json = serde_json::to_string(&v).unwrap();
        assert!(
            !json.contains("SECRETVALUE") && !json.contains(&ts),
            "{json}"
        );
        let dbg = format!("{s:?}");
        assert!(!dbg.contains("SECRETVALUE") && !dbg.contains(&ts), "{dbg}");
        let jv: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(jv["carrier"]["token"], serde_json::json!({"set": true}));
        assert_eq!(
            jv["carrier"]["topic_secret"],
            serde_json::json!({"set": true})
        );
    }

    #[test]
    fn the_view_has_the_contract_shape() {
        let v = serde_json::to_value(RelaySettings::default().view()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "gateway": {"enabled": false, "termux_poll_min": 15},
                "carrier": {"kind": "none", "server": null, "mode": "5A",
                            "token": {"set": false}, "topic_secret": {"set": false},
                            "consent_hash": null},
                "presence": {"heartbeat": [
                    {"transport": "tcp-lan", "interval_min": 0, "owner": "layer7"},
                    {"transport": "kiss-modem", "interval_min": 0, "owner": "layer7"},
                    {"transport": "meshtastic-client", "interval_min": 0, "owner": "layer7"}
                ]},
                "discovery": {"lan_record": false, "public_gateway_bit": false, "offload_bit": false},
                "origin": {"serve": false, "targets": [TARGET_TRIPLE]},
                "updates": {"check_internet": true},
                "metered": []
            })
        );
        let mut s = RelaySettings::default();
        s.set_metered(vec![MeteredLink {
            matcher: MeteredMatch::Ssid("Phone hotspot".into()),
            daily_cap_bytes: 5,
        }])
        .unwrap();
        let v = serde_json::to_value(s.view()).unwrap();
        assert_eq!(
            v["metered"],
            serde_json::json!([{"match": {"ssid": "Phone hotspot"}, "daily_cap_bytes": 5}])
        );
    }

    #[test]
    fn mode_5b_is_refused_on_termux() {
        let mut s = RelaySettings::default();
        assert_eq!(
            s.set_carrier_mode(CarrierMode::FiveB, true),
            Err(SettingsError::ModeRefusedOnTermux)
        );
        s.set_carrier_mode(CarrierMode::FiveA, true).unwrap();
        s.set_carrier_mode(CarrierMode::FiveB, false).unwrap();
        assert_eq!(s.carrier_mode_for(false), CarrierMode::FiveB);
        assert_eq!(CarrierMode::parse("5B"), Ok(CarrierMode::FiveB));
        assert_eq!(CarrierMode::parse("5b"), Err(SettingsError::BadMode));
        assert_eq!(CarrierKind::parse("ntfy"), Ok(CarrierKind::Ntfy));
        assert_eq!(
            CarrierKind::parse("chatmail"),
            Err(SettingsError::BadCarrierKind)
        );
    }

    #[test]
    fn targets_keep_the_own_one_first_and_are_bounded() {
        let mut s = RelaySettings::default();
        s.set_origin_targets::<&str>(&[]).unwrap();
        assert_eq!(s.origin_targets(), [TARGET_TRIPLE.to_string()]);
        s.set_origin_targets(&["armv7-unknown-linux-musleabihf", TARGET_TRIPLE])
            .unwrap();
        assert_eq!(s.origin_targets()[0], TARGET_TRIPLE);
        assert_eq!(s.origin_targets().len(), 2);
        for bad in ["", "x86_64", "X86_64-linux", "-a-b", "a/b-c", "a b-c"] {
            assert_eq!(
                s.set_origin_targets(&[bad]),
                Err(SettingsError::BadTarget),
                "{bad:?}"
            );
        }
        let many: Vec<String> = (0..MAX_TARGETS).map(|i| format!("arch{i}-linux")).collect();
        assert_eq!(s.set_origin_targets(&many), Err(SettingsError::BadTarget));
        assert!(s.set_origin_targets(&many[..MAX_TARGETS - 1]).is_ok());
    }

    #[test]
    fn metered_rules_and_window() {
        let mut s = RelaySettings::default();
        let link = |m: MeteredMatch, cap| MeteredLink {
            matcher: m,
            daily_cap_bytes: cap,
        };
        let wwan = MeteredMatch::Iface("wwan0".into());
        let hot = MeteredMatch::Ssid("hotspot".into());
        assert!(matches!(
            s.set_metered(vec![link(wwan.clone(), 0)]),
            Err(SettingsError::BadMetered(_))
        ));
        assert!(matches!(
            s.set_metered(vec![link(wwan.clone(), 1), link(wwan.clone(), 2)]),
            Err(SettingsError::BadMetered(_))
        ));
        for bad in ["", "a/b", "sixteen-chars-xx", "a b", ".."] {
            assert!(
                s.set_metered(vec![link(MeteredMatch::Iface(bad.into()), 1)])
                    .is_err(),
                "{bad:?}"
            );
        }
        assert!(s
            .set_metered(vec![link(MeteredMatch::Ssid("a".repeat(33)), 1)])
            .is_err());
        assert!(s
            .set_metered(vec![link(MeteredMatch::Ssid("a\nb".into()), 1)])
            .is_err());
        let too_many: Vec<MeteredLink> = (0..=MAX_METERED)
            .map(|i| link(MeteredMatch::Iface(format!("if{i}")), 1))
            .collect();
        assert!(s.set_metered(too_many).is_err());

        s.set_metered(vec![link(wwan.clone(), 10), link(hot.clone(), 20)])
            .unwrap();
        let slots = vec![MeteredSlot { at_s: 1, bytes: 2 }];
        s.set_metered_window(MeteredWindow {
            boot_id: None,
            links: vec![
                MeteredUsage {
                    matcher: wwan.clone(),
                    slots: slots.clone(),
                },
                MeteredUsage {
                    matcher: hot.clone(),
                    slots: slots.clone(),
                },
            ],
        })
        .unwrap();
        // Removing a rule drops its counts.
        s.set_metered(vec![link(hot.clone(), 20)]).unwrap();
        assert_eq!(s.metered_window().links.len(), 1);
        assert_eq!(s.metered_window().links[0].matcher, hot);

        let bad_boot = MeteredWindow {
            boot_id: Some("not a boot id".into()),
            links: vec![],
        };
        assert!(matches!(
            s.set_metered_window(bad_boot),
            Err(SettingsError::BadWindow(_))
        ));
        let over = MeteredWindow {
            boot_id: None,
            links: vec![MeteredUsage {
                matcher: hot.clone(),
                slots: vec![MeteredSlot { at_s: 0, bytes: 0 }; MAX_WINDOW_SLOTS + 1],
            }],
        };
        assert!(s.set_metered_window(over).is_err());
    }

    #[test]
    fn drain_cursors_are_bounded_and_checked() {
        let mut s = RelaySettings::default();
        assert_eq!(
            s.set_drain_cursor("111short", Some("a")),
            Err(SettingsError::BadDrain)
        );
        assert_eq!(
            s.set_drain_cursor(&TOPIC.to_uppercase(), Some("a")),
            Err(SettingsError::BadDrain)
        );
        assert_eq!(
            s.set_drain_cursor(TOPIC, Some("")),
            Err(SettingsError::BadDrain)
        );
        assert_eq!(
            s.set_drain_cursor(TOPIC, Some("a?b")),
            Err(SettingsError::BadDrain)
        );
        assert_eq!(
            s.set_drain_cursor(TOPIC, Some(&"a".repeat(MAX_DRAIN_ID + 1))),
            Err(SettingsError::BadDrain)
        );
        s.set_drain_cursor(TOPIC, Some("id1")).unwrap();
        s.set_drain_cursor(TOPIC, Some("id2")).unwrap();
        assert_eq!(s.drain_cursor(TOPIC), Some("id2"));
        s.set_drain_cursor(TOPIC, None).unwrap();
        assert_eq!(s.drain_cursor(TOPIC), None);
        let topic = |i: usize| format!("111{}", base32_encode_lower(&[i as u8; 20]));
        for i in 0..MAX_DRAIN_TOPICS {
            s.set_drain_cursor(&topic(i), Some("x")).unwrap();
        }
        assert_eq!(
            s.set_drain_cursor(&topic(MAX_DRAIN_TOPICS), Some("x")),
            Err(SettingsError::TooManyTopics)
        );
        s.set_drain_cursor(&topic(0), Some("y")).unwrap(); // an existing topic still moves
    }

    #[test]
    fn consent_hashes_are_sha256_hex() {
        let mut s = RelaySettings::default();
        for k in [
            ConsentKind::CarrierEnable,
            ConsentKind::GatewayBit,
            ConsentKind::OriginServe,
        ] {
            assert_eq!(
                s.set_consent_hash(k, Some("abc")),
                Err(SettingsError::BadHash)
            );
            assert_eq!(
                s.set_consent_hash(k, Some(&HASH.to_uppercase())),
                Err(SettingsError::BadHash)
            );
            s.set_consent_hash(k, Some(HASH)).unwrap();
            assert_eq!(s.consent_hash(k), Some(HASH));
            s.set_consent_hash(k, None).unwrap();
            assert_eq!(s.consent_hash(k), None);
        }
    }

    #[test]
    fn a_file_heartbeat_is_read_but_only_for_layer7_transports() {
        let s = decode(r#"{"schema":1,"presence":{"heartbeat":{"tcp-lan":0,"kiss-modem":30}}}"#)
            .unwrap();
        assert_eq!(s.heartbeat_min("kiss-modem"), 30);
        assert_eq!(s.heartbeat_min("tcp-lan"), 0);
        assert_eq!(
            decode(r#"{"schema":1,"presence":{"heartbeat":{"kiss-modem":20000}}}"#),
            Err(SettingsError::BadHeartbeat)
        );
    }
}
