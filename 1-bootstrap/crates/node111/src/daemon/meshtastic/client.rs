//! The Meshtastic client (Layer 6 `6-transport/README.md` §5; §4 "Delivery states per
//! network"; §3 "Conversation identity"): the Meshtastic [`Adapter`] of the bridge core.
//!
//! **One more downlink client.** [`MeshtasticAdapter`] connects to meshmud's TCP downlink
//! (`127.0.0.1:11230`, `meshmud::DOWNLINK`) beside the presence reader and the panel shim's
//! per-session downlinks; no new port claim. [`super::Meshtastic`] starts it while service
//! `meshtastic` runs (meshmud up, downlink listening, uplink present) and stops it otherwise:
//! with the service off it has no task, socket or timer (`AGENTS.md`: the Raspberry Pi is
//! the floor). The account is the board's own node (`!<node num hex>`).
//!
//! **Protobuf types, our client.** The `meshtastic` crate (GPL-3.0, 0.1.9) is linked with no
//! features: only its generated prost types (`meshtastic::protobufs`, one snapshot). Its
//! stream API needs the crate's `tokio` feature, which also links `tokio-serial` and
//! `serialport` (MPL-2.0) and tokio `full`, and it spawns four tasks per connection over
//! unbounded queues; the §5 fallback applies: this client over the crate's types.
//!
//! **Handshake once per connection** (§5): one `want_config_id`, random, nonzero, never 1,
//! 69420 or 69421; `MyNodeInfo`, `NodeInfo`, `Channel` and `Config` (region, preset) are
//! ingested until a `config_complete_id`. Another client's `want_config_id` replays the
//! config stream to every downlink (meshmud forwards verbatim, R01 §3.10) and can pre-empt
//! ours, so **any** `config_complete_id` ends a config stream, replays are ingested
//! idempotently and `want_config_id` is never re-sent inside a session. Text before the
//! first completion is buffered (bounded, [`PREBUFFER`]) and mapped once the channel table
//! is in. A nonce-0 heartbeat every 5 minutes on the injectable monotonic clock keeps the
//! board's 15-minute API session alive (R01 §3.9). `rebooted` or a closed downlink ends the
//! session; the client reconnects with capped back-off.
//!
//! **The allowlist is the lowest layer** ([`super::allowlist`]): the write half is an
//! `AllowlistWriter`; this file holds the only three `ToRadio` constructors in `src/`.
//!
//! **One send queue** (§5): texts at least [`PACING`] (2.1 s) apart on the monotonic clock,
//! random nonzero packet ids never reused, `QueueStatus.free = 0` pauses the queue ("waiting
//! for the radio"), `want_ack = true` for channel text and DMs (the firmware clears it on
//! air for broadcasts but keeps the local reliable record that yields the implicit ack).
//!
//! **Send events** (§4; one handle per `send`):
//! - `Failed { reason, Never }` before air: `region_unset`, `no_channel`, `too_large`,
//!   `not_authorized`, `pki_failed`, `got_nak` (terminal).
//! - `Failed { reason, Later }`: the job goes back to the core, which retries on its tick
//!   (and earlier on an adapter event): `adapter_down` (no configured session), `rate_limited`,
//!   `duty_cycle`, `key_unknown` (PKI key of the destination unknown). The adapter events
//!   are `Inbound::Link(Up)` once a session is configured and an `Inbound::Conversation`
//!   upsert of `dm:!<node>` when that node's NodeInfo with a key is heard (key learned) or,
//!   after a DM's post-air NAK, when that node's next NodeInfo is heard.
//! - `Accepted { native_id }` once **the radio** took it (`native_id` = the packet id, 8
//!   lowercase hex digits, the form inbound `native_msg_id`s use): `QueueStatus.res = 0` for
//!   our id after a short settle in which a `RATE_LIMIT_EXCEEDED` still counts (the firmware
//!   queues a res-0 status before that NAK), or any ack or post-air NAK for it. Every
//!   refusal above comes before it, so the core's "a failure after `Accepted` was on air"
//!   rule holds.
//! - `Evidence`: `queued` "waiting for the radio" while the queue is paused (the job stays);
//!   `sent` "accepted by the radio" (right after `Accepted`); `relayed` "a neighbour
//!   rebroadcast it" (a routing ack from our own node: the
//!   implicit ack; "relayed, not confirmed" for a DM); `delivered` "the recipient confirmed"
//!   (a DM's real ack); `sent`/`relayed` "transmitted, not confirmed" after a post-air NAK
//!   (`no_route`, `timeout`, `max_retransmit`, `no_response`), which for a DM is followed by
//!   `Failed { reason, Later }` (the state stays where it was: states are monotone).
//! - The handle closes after a terminal event, a DM's `delivered`, a channel text's
//!   `relayed` or post-air NAK, or [`IN_FLIGHT_TTL`]. A handle that closes with neither
//!   `Accepted` nor `Failed` means a write broke mid-frame: "may have been sent".
//!
//! **Receive** (§5): portnum 1 becomes an [`InboundText`]; `emoji != 0` with `reply_id` is a
//! tapback, counted and not a message; channel text is `channel-key` (`none` on a channel
//! without a key); a DM to the board is `pki` when `pki_encrypted`; `via_mqtt`, snr, rssi,
//! hops, `ok_to_mqtt` and an own echo (`from` = the board) go in `meta`. Every configured
//! channel is upserted read-only from the first configured session; DMs on their first text.
//!
//! **Conversation identity** (§3): channel `ch:<name>:<kh>`, `kh` = 16 hex digits of a keyed
//! BLAKE2b-256 (the key the daemon derives under `111/bridge-conv/v1`, handed in) over the
//! effective name and a keyed digest of the PSK. **The PSK is hashed and dropped**: never
//! kept, persisted, logged, serialized or sent; its bytes and the frame carrying it are
//! zeroized after hashing. A re-keyed channel is a new conversation; the channel index is
//! board-local and never part of an id. DM `dm:!<node hex>`.
//!
//! **Facts** ([`MeshtasticAdapter::mesh_facts`], [`Adapter::facts`]): the board's node,
//! `User.is_licensed`, each channel's `uplink_enabled` and default-key fact, region and
//! preset, read live (§5 "HAM and MQTT: live consent").
//!
//! **Nearby** (§5 "Nearby folds into the client"): the node DB follows the presence
//! reader's rules (meshtastic-python's node dict updates and `normalize_node`), names cut to
//! 24 B; [`MeshtasticAdapter::nearby_nodes`] runs the reader-format rows through the
//! daemon's own ingest (`presence::parse`), so both feeds publish the same `NearbySource`.

use super::allowlist::{self, AllowStats, AllowlistWriter, LogRing, BROADCAST};
use super::presence::{self, NearbyNode, NearbySource};
use super::shim::{self, Deframer};
use crate::daemon::bridge::{
    AccountFacts, Adapter, AdapterFacts, AdapterState, AirtimeProfile, Auth, Caps, ChannelFacts,
    ConvKind, ConvRef, ForeignMessageState, Honest, Inbound, InboundText, LinkState, PerKind,
    Reason, Retry, SendEvent, SendHandle, SendOpts, NETWORK_MESHTASTIC,
};
use futures_util::future::BoxFuture;
use meshtastic::protobufs as pb;
use meshtastic::Message;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch, Notify};
use zeroize::{Zeroize, Zeroizing};

/// Heartbeat cadence (§5; the crate's `CLIENT_HEARTBEAT_INTERVAL`; the board's API session
/// lapses after 15 minutes without a `ToRadio`).
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(5 * 60);
/// Text spacing (§5: the firmware answers a text within 2 s of the previous one with
/// `RATE_LIMIT_EXCEEDED`, per `PhoneAPI`, shared with the panel user).
pub const PACING: Duration = Duration::from_millis(2100);
/// How long a `QueueStatus.res = 0` waits for a `RATE_LIMIT_EXCEEDED` before it reads as
/// "accepted by the radio" (the firmware sends both for a rate-limited text).
pub const SENT_SETTLE: Duration = Duration::from_secs(1);
/// Channel text cap (§4/§5: 232 B, the firmware's 239 B `Data` with the 2-byte `bitfield`).
pub const TEXT_CHANNEL_MAX: usize = 232;
/// PKI DM cap (220 B: 227 B `Data`, 12 B PKI overhead).
pub const TEXT_PKI_DM_MAX: usize = 220;
/// DM cap on a licensed board (the firmware skips PKI there).
pub const TEXT_LICENSED_DM_MAX: usize = 232;
/// Names are cut to the protobuf limit (`User.long_name`: 24 B of UTF-8, R01 §3.3).
pub const NAME_BYTES: usize = 24;
/// Text packets buffered before the first `config_complete_id`.
pub const PREBUFFER: usize = 256;
/// Inbound queue bound (texts, upserts, link state); an overflow drops the oldest, counted.
pub const INBOUND: usize = 1024;
/// Jobs waiting in the send queue.
pub const SEND_QUEUE: usize = 256;
/// Written texts awaiting evidence.
pub const IN_FLIGHT: usize = 256;
/// How long a written text waits for evidence before its handle closes.
pub const IN_FLIGHT_TTL: Duration = Duration::from_secs(10 * 60);
/// Most nodes kept (the reader's and the daemon's cap).
pub const MAX_NODES: usize = presence::MAX_NODES;
/// Recent packet ids never reused (the firmware's own window is 20).
const USED_IDS: usize = 4096;
/// Conversation upserts remembered (to emit each once).
const EMITTED_CONVS: usize = 8192;
/// Domain tags inside the conv key (the key itself is derived under `111/bridge-conv/v1`).
const TAG_PSK: &[u8] = b"meshtastic/psk/v1";
const TAG_CHANNEL: &[u8] = b"meshtastic/channel/v1";
/// The firmware's public default channel key (`Channels.h` `defaultpsk`); a 1-byte PSK
/// 1..=10 is a published variant of it, 0 or empty is no encryption.
const DEFAULT_PSK: [u8; 16] = [
    0xd4, 0xf1, 0xbb, 0x3a, 0x20, 0x29, 0x07, 0x59, 0xf0, 0xbc, 0xff, 0xab, 0xcf, 0x4e, 0x69, 0x01,
];

// ------------------------------------------------------------------ the three constructors

/// `ToRadio.want_config_id` (once per connection). One of the three `ToRadio` constructors
/// in `src/` (§5; the source-scan test pins the set).
fn to_radio_want_config(id: u32) -> Vec<u8> {
    pb::ToRadio {
        payload_variant: Some(pb::to_radio::PayloadVariant::WantConfigId(id)),
    }
    .encode_to_vec()
}

/// `ToRadio.heartbeat` with nonce 0 (the keep-alive; nonce 1 would broadcast our NodeInfo).
fn to_radio_heartbeat() -> Vec<u8> {
    pb::ToRadio {
        payload_variant: Some(pb::to_radio::PayloadVariant::Heartbeat(pb::Heartbeat {
            nonce: 0,
        })),
    }
    .encode_to_vec()
}

/// `ToRadio.packet` carrying one text built by [`text_packet`].
fn to_radio_text(p: pb::MeshPacket) -> Vec<u8> {
    pb::ToRadio {
        payload_variant: Some(pb::to_radio::PayloadVariant::Packet(p)),
    }
    .encode_to_vec()
}

/// The one packet shape the client sends: `TEXT_MESSAGE_APP` to `to` (`0xFFFFFFFF` or a
/// node) on a board-local channel index, `want_ack = true` (§5), `from` left 0 (the
/// firmware writes the board's node).
pub fn text_packet(to: u32, channel: u32, id: u32, hop_limit: u32, text: &[u8]) -> pb::MeshPacket {
    pb::MeshPacket {
        to,
        channel,
        id,
        hop_limit,
        want_ack: true,
        payload_variant: Some(pb::mesh_packet::PayloadVariant::Decoded(pb::Data {
            portnum: pb::PortNum::TextMessageApp as i32,
            payload: text.to_vec(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

// ------------------------------------------------------------------ clock

/// The monotonic clock the client decides with (heartbeat, pacing, settle, back-off).
/// Never the wall clock; tests inject a [`ManualClock`].
pub trait MonoClock: Send + Sync + 'static {
    /// Time since an arbitrary origin.
    fn now(&self) -> Duration;
    /// Resolves once [`MonoClock::now`] has reached `t`.
    fn sleep_until(&self, t: Duration) -> BoxFuture<'static, ()>;
}

/// tokio's monotonic clock.
#[derive(Debug)]
pub struct SystemClock {
    origin: tokio::time::Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock {
            origin: tokio::time::Instant::now(),
        }
    }
}

impl MonoClock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn sleep_until(&self, t: Duration) -> BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep_until(self.origin + t))
    }
}

/// A clock that moves only when told (tests).
#[derive(Debug)]
pub struct ManualClock {
    now: watch::Sender<Duration>,
}

impl ManualClock {
    pub fn new() -> Arc<Self> {
        Arc::new(ManualClock {
            now: watch::channel(Duration::ZERO).0,
        })
    }

    /// Move the clock forward by `d`, waking every sleeper whose time has come.
    pub fn advance(&self, d: Duration) {
        self.now.send_modify(|t| *t += d);
    }
}

impl MonoClock for ManualClock {
    fn now(&self) -> Duration {
        *self.now.borrow()
    }

    fn sleep_until(&self, t: Duration) -> BoxFuture<'static, ()> {
        let mut rx = self.now.subscribe();
        Box::pin(async move {
            loop {
                if *rx.borrow_and_update() >= t {
                    return;
                }
                if rx.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        })
    }
}

// ------------------------------------------------------------------ config

/// Client tuning (production: [`ClientConfig::production`]).
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// meshmud's TCP downlink.
    pub downlink: SocketAddr,
    pub heartbeat_every: Duration,
    pub pacing: Duration,
    pub sent_settle: Duration,
    /// Reconnect back-off: first wait, doubled up to `reconnect_max`.
    pub reconnect_initial: Duration,
    pub reconnect_max: Duration,
    /// Probe cadence while `QueueStatus.free = 0` (a nonce-0 heartbeat asks for a status).
    pub pause_probe: Duration,
    pub pause_probe_max: Duration,
    pub connect_timeout: Duration,
    pub write_timeout: Duration,
    pub inbound_capacity: usize,
    pub prebuffer_capacity: usize,
}

impl ClientConfig {
    pub fn production(downlink: SocketAddr) -> Self {
        ClientConfig {
            downlink,
            heartbeat_every: HEARTBEAT_EVERY,
            pacing: PACING,
            sent_settle: SENT_SETTLE,
            reconnect_initial: Duration::from_secs(1),
            reconnect_max: Duration::from_secs(60),
            pause_probe: Duration::from_secs(5),
            pause_probe_max: Duration::from_secs(60),
            connect_timeout: Duration::from_secs(3),
            write_timeout: Duration::from_secs(5),
            inbound_capacity: INBOUND,
            prebuffer_capacity: PREBUFFER,
        }
    }
}

/// The conversation key (handed in by the daemon, derived under `111/bridge-conv/v1`).
struct ConvKey(Zeroizing<[u8; 32]>);

impl ConvKey {
    fn mac(&self, parts: &[&[u8]]) -> [u8; 32] {
        use blake2::digest::consts::U32;
        use blake2::digest::{FixedOutput, KeyInit, Update};
        // a 32-byte key is always accepted by BLAKE2b-MAC (keys up to 64 bytes)
        let mut mac = match <blake2::Blake2bMac<U32> as KeyInit>::new_from_slice(&self.0[..]) {
            Ok(m) => m,
            Err(_) => return [0u8; 32],
        };
        for p in parts {
            Update::update(&mut mac, &(p.len() as u32).to_le_bytes());
            Update::update(&mut mac, p);
        }
        let out = mac.finalize_fixed();
        let mut a = [0u8; 32];
        a.copy_from_slice(&out);
        a
    }

    /// A keyed digest of a PSK (the PSK itself is never kept).
    fn psk_digest(&self, psk: &[u8]) -> [u8; 32] {
        self.mac(&[TAG_PSK, psk])
    }

    /// `kh` of `ch:<name>:<kh>`: 16 hex digits.
    fn kh(&self, name: &[u8], psk_digest: &[u8; 32]) -> String {
        let d = self.mac(&[TAG_CHANNEL, name, psk_digest]);
        d[..8].iter().map(|b| format!("{b:02x}")).collect()
    }
}

// ------------------------------------------------------------------ the reader's rules

/// meshtastic-presence `sanitize_text`: drop Cc, Zl, Zp and the bidi/invisible set, strip,
/// cut to `max` UTF-8 bytes on a character boundary, strip again.
pub fn reader_name(s: &str, max: usize) -> String {
    fn dropped(c: char) -> bool {
        c.is_control()
            || matches!(
                c as u32,
                0x2028 | 0x2029 | 0x061C | 0x200E | 0x200F | 0x2060 | 0xFEFF | 0x202A..=0x202E
                    | 0x2066..=0x2069
            )
    }
    let kept: String = s.chars().filter(|c| !dropped(*c)).collect();
    let mut out = String::new();
    for c in kept.trim().chars() {
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out.trim().to_string()
}

/// A float32 field as `MessageToDict` hands it to Python (the shortest repr), or `None`
/// for NaN / infinity (the reader drops those).
fn py_f32(x: f32) -> Option<f64> {
    if !x.is_finite() {
        return None;
    }
    format!("{x}").parse().ok()
}

/// Python's `round(x, digits)` (exact value, half to even).
fn py_round(x: f64, digits: usize) -> f64 {
    format!("{x:.digits$}").parse().unwrap_or(x)
}

fn node_id(num: u32) -> String {
    format!("!{num:08x}")
}

fn dm_native(num: u32) -> String {
    format!("dm:{}", node_id(num))
}

/// `dm:!<8 hex>` → the node number.
fn parse_dm(native: &str) -> Option<u32> {
    let hex = native.strip_prefix("dm:!")?;
    if hex.len() != 8 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// The firmware's display name of a modem preset (`DisplayFormatters.cpp`): an unnamed
/// channel is called this on air.
fn preset_display(preset: i32, use_preset: bool) -> &'static str {
    if !use_preset {
        return "Custom";
    }
    match preset {
        0 => "LongFast",
        1 => "LongSlow",
        2 => "VLongSlow",
        3 => "MediumSlow",
        4 => "MediumFast",
        5 => "ShortSlow",
        6 => "ShortFast",
        7 => "LongMod",
        8 => "ShortTurbo",
        9 => "LongTurbo",
        _ => "Invalid",
    }
}

/// Nominal LoRa bit rate of a preset (SF × BW / 2^SF × CR; display only, §9 preview).
fn preset_bits_per_s(preset: i32) -> Option<u32> {
    Some(match preset {
        8 => 21_875,
        6 => 10_937,
        5 => 6_250,
        4 => 3_515,
        3 => 1_953,
        9 => 1_342,
        0 => 1_074,
        7 => 335,
        1 => 183,
        2 => 91,
        _ => return None,
    })
}

/// Regulatory duty cycle of a region in permille (the firmware's region table; 1000 = no
/// limit). Display only.
fn region_duty_permille(region: i32) -> u16 {
    match region {
        2 | 3 | 12 | 14 => 100, // EU_433, EU_868, TH, UA_433: 10 %
        15 => 10,               // UA_868: 1 %
        _ => 1000,
    }
}

fn region_name(region: i32) -> String {
    pb::config::lo_ra_config::RegionCode::try_from(region)
        .map(|r| r.as_str_name().to_string())
        .unwrap_or_else(|_| format!("REGION_{region}"))
}

fn preset_name(preset: i32) -> String {
    pb::config::lo_ra_config::ModemPreset::try_from(preset)
        .map(|r| r.as_str_name().to_string())
        .unwrap_or_else(|_| format!("PRESET_{preset}"))
}

fn rand_u32() -> Option<u32> {
    let mut b = [0u8; 4];
    getrandom::getrandom(&mut b).ok()?;
    Some(u32::from_le_bytes(b))
}

/// A config id for this connection (§5): random, nonzero, never 1, 69420 or 69421.
pub fn random_config_id() -> Option<u32> {
    (0..64)
        .filter_map(|_| rand_u32())
        .find(|id| allowlist::config_id_ok(*id))
}

// ------------------------------------------------------------------ the node DB

#[derive(Debug, Clone, Default, PartialEq)]
struct PosRaw {
    lat_i: Option<i32>,
    lon_i: Option<i32>,
    alt: Option<i32>,
}

impl From<&pb::Position> for PosRaw {
    fn from(p: &pb::Position) -> Self {
        PosRaw {
            lat_i: p.latitude_i,
            lon_i: p.longitude_i,
            alt: p.altitude,
        }
    }
}

/// One node as the reader's library keeps it (only the fields the schema reads).
#[derive(Debug, Clone, Default)]
struct Node {
    num: u32,
    long_name: String,
    short_name: String,
    /// A `User` was heard (else the names are the library's presumptive ones).
    user_known: bool,
    pubkey: Option<String>,
    licensed: bool,
    snr: Option<f64>,
    rssi: Option<i32>,
    hops_away: Option<u32>,
    last_heard: Option<u32>,
    pos: Option<PosRaw>,
    util: Option<f64>,
}

impl Node {
    /// meshtastic-python `_getOrCreateByNum`: a presumptive user from the node id.
    fn presumptive(num: u32) -> Node {
        let id = node_id(num);
        let last4 = &id[id.len() - 4..];
        Node {
            num,
            long_name: format!("Meshtastic {last4}"),
            short_name: last4.to_string(),
            ..Default::default()
        }
    }

    fn set_user(&mut self, u: &pb::User) {
        self.long_name = reader_name(&u.long_name, NAME_BYTES);
        self.short_name = reader_name(&u.short_name, NAME_BYTES);
        self.pubkey = (u.public_key.len() == 32)
            .then(|| crate::daemon::dashboard::routes::base64(&u.public_key));
        self.licensed = u.is_licensed;
        self.user_known = true;
    }

    /// `_receiveInfoUpdate`: text and NodeInfo packets overwrite rssi, last heard and snr
    /// (with nothing when the packet has none).
    fn info_update(&mut self, p: &pb::MeshPacket) {
        self.rssi = (p.rx_rssi != 0).then_some(p.rx_rssi);
        self.last_heard = (p.rx_time != 0).then_some(p.rx_time);
        self.snr = if p.rx_snr != 0.0 {
            py_f32(p.rx_snr)
        } else {
            None
        };
    }

    /// The reader's `normalize_node` row (`meshtastic.json` schema 1).
    fn reader_row(&self) -> Value {
        let mut m = Map::new();
        m.insert("id".into(), json!(node_id(self.num)));
        m.insert("num".into(), json!(self.num));
        m.insert("long_name".into(), json!(self.long_name));
        m.insert("short_name".into(), json!(self.short_name));
        if let Some(k) = &self.pubkey {
            m.insert("pubkey".into(), json!(k));
        }
        if let Some(s) = self.snr.filter(|s| (-100.0..=100.0).contains(s)) {
            m.insert("snr".into(), json!(py_round(s, 2)));
        }
        if let Some(r) = self.rssi.filter(|r| (-200..=0).contains(r)) {
            m.insert("rssi".into(), json!(r));
        }
        if let Some(h) = self.hops_away.filter(|h| *h <= 7) {
            m.insert("hops_away".into(), json!(h));
        }
        if let Some(t) = self.last_heard.filter(|t| *t > 0) {
            m.insert("last_heard_hint".into(), json!(t));
        }
        if let Some(p) = self.pos.as_ref().and_then(position_row) {
            m.insert("position".into(), p);
        }
        if let Some(u) = self.util.filter(|u| (0.0..=100.0).contains(u)) {
            m.insert("channel_utilization".into(), json!(py_round(u, 2)));
        }
        Value::Object(m)
    }
}

/// The reader's `_position`: a fix needs both integer coordinates in range and not 0,0.
fn position_row(p: &PosRaw) -> Option<Value> {
    let (lat_i, lon_i) = (p.lat_i?, p.lon_i?);
    let (lat, lon) = (f64::from(lat_i) / 1e7, f64::from(lon_i) / 1e7);
    if !((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)) {
        return None;
    }
    if lat == 0.0 && lon == 0.0 {
        return None;
    }
    let mut o = Map::new();
    o.insert("lat".into(), json!(py_round(lat, 7)));
    o.insert("lon".into(), json!(py_round(lon, 7)));
    if let Some(a) = p.alt.filter(|a| (-1000..=100_000).contains(a)) {
        o.insert("alt".into(), json!(a));
    }
    Some(Value::Object(o))
}

// ------------------------------------------------------------------ channels

/// One active channel. The PSK is gone: only its keyed digest stays (for a secondary with
/// an empty PSK, which uses the primary's key).
#[derive(Clone)]
struct Chan {
    index: u32,
    /// The name as the board sent it (may be empty: the preset name is used on air).
    raw_name: String,
    psk_digest: [u8; 32],
    psk_empty: bool,
    /// Own key is the public default (empty, one byte, or `defaultpsk`).
    default_key_own: bool,
    /// Own key encrypts (not empty, not the 1-byte 0).
    keyed_own: bool,
    uplink: bool,
    primary: bool,
}

/// Facts of one channel (`MeshFacts.channels`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelFact {
    /// Board-local index (0 = primary); never part of an id.
    pub index: u32,
    /// The effective name (the preset's when unnamed); untrusted text.
    pub name: String,
    /// The conversation's native id `ch:<name>:<kh>`.
    pub conv_id: String,
    /// The board publishes this channel to MQTT.
    pub uplink_enabled: bool,
    /// The channel uses the public default key (or none).
    pub default_key: bool,
    /// The channel's key encrypts at all.
    pub encrypted: bool,
    pub primary: bool,
}

/// Counters (`/networks`, the service page, tests).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ClientCounters {
    pub sessions: u64,
    pub configured: u64,
    /// Frames that do not decode as `FromRadio` (the JSON stub harness, debug noise).
    pub noise_frames: u64,
    /// Inbound items dropped by the bounded queue.
    pub inbound_dropped: u64,
    /// Texts dropped from the pre-config buffer.
    pub prebuffer_dropped: u64,
    pub tapbacks: u64,
    /// Channel text on an index not in the table.
    pub unknown_channel: u64,
    /// Text addressed to another node.
    pub not_for_us: u64,
    pub allowlist_passed: u64,
    pub allowlist_dropped: u64,
    pub texts_written: u64,
    /// `QueueStatus` frames seen.
    pub queue_statuses: u64,
    /// Reconnect back-off waits started.
    pub backoffs: u64,
}

/// Live facts of the board (§5 "HAM and MQTT: live consent").
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MeshFacts {
    pub running: bool,
    pub connected: bool,
    /// A `config_complete_id` arrived in this session.
    pub configured: bool,
    /// The board's own node id (`!0a0a0a0a`): the account.
    pub my_node: Option<String>,
    pub long_name: Option<String>,
    pub short_name: Option<String>,
    /// `User.is_licensed` of the board's own node; `None` until its NodeInfo arrived.
    pub licensed: Option<bool>,
    /// `Config.lora.region` (`UNSET` = the board transmits nothing).
    pub region: Option<String>,
    pub region_set: Option<bool>,
    pub preset: Option<String>,
    pub channels: Vec<ChannelFact>,
    pub nodes: u32,
    /// Jobs in the send queue, written texts awaiting evidence.
    pub queued: u32,
    pub in_flight: u32,
    /// `QueueStatus.free = 0` paused the send queue.
    pub paused: bool,
    pub counters: ClientCounters,
}

// ------------------------------------------------------------------ the send side

struct Job {
    to: u32,
    channel: u32,
    dm: bool,
    text: Vec<u8>,
    tx: mpsc::Sender<SendEvent>,
    /// "waiting for the radio" already reported for the current pause.
    wait_noted: bool,
}

impl Job {
    fn emit(&self, e: SendEvent) {
        // never blocks; the core drains handles promptly and a job emits few events
        let _ = self.tx.try_send(e);
    }
}

impl Flight {
    /// `Accepted` once, before any evidence that the radio took it.
    fn accept(&mut self) {
        if !self.accepted {
            self.accepted = true;
            self.job.emit(SendEvent::Accepted {
                native_id: format!("{:08x}", self.id),
            });
        }
    }
}

struct Flight {
    id: u32,
    job: Job,
    /// `Accepted` went out on this handle.
    accepted: bool,
    written_at: Duration,
    /// Highest state reported on this handle.
    state: Option<ForeignMessageState>,
    /// `QueueStatus.res = 0` for this id arrived at.
    qs_ok_at: Option<Duration>,
}

struct Pause {
    probe_at: Duration,
    step: Duration,
}

/// What the session task writes next.
enum Work {
    Frame(Vec<u8>),
    Text { id: u32, frame: Vec<u8>, job: Job },
}

/// §5 Reasons: how one `Routing.Error` maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingOutcome {
    /// `NONE`: an ack (implicit when from our own node).
    Ack,
    /// Refused before air: `failed`, `Retry::Never`.
    PreAir(Reason),
    /// Not sent now, retry later: `queued`, `Retry::Later`.
    Later(Reason),
    /// After the board transmitted: keep `sent`/`relayed`, "transmitted, not confirmed".
    PostAir(Reason),
}

/// The §5 table. Codes outside it fail closed: `NO_INTERFACE` (the board has no radio up)
/// is `adapter_down` later; `BAD_REQUEST`, the admin codes and unknown values are `got_nak`
/// before air.
pub fn map_routing_error(code: i32) -> RoutingOutcome {
    use pb::routing::Error as E;
    match E::try_from(code) {
        Ok(E::None) => RoutingOutcome::Ack,
        Ok(E::NoRoute) => RoutingOutcome::PostAir(Reason::NoRoute),
        Ok(E::Timeout) => RoutingOutcome::PostAir(Reason::Timeout),
        Ok(E::MaxRetransmit) => RoutingOutcome::PostAir(Reason::MaxRetransmit),
        Ok(E::NoResponse) => RoutingOutcome::PostAir(Reason::NoResponse),
        Ok(E::GotNak) => RoutingOutcome::PreAir(Reason::GotNak),
        Ok(E::NoChannel) => RoutingOutcome::PreAir(Reason::NoChannel),
        Ok(E::TooLarge) => RoutingOutcome::PreAir(Reason::TooLarge),
        Ok(E::NotAuthorized) => RoutingOutcome::PreAir(Reason::NotAuthorized),
        Ok(E::PkiFailed) => RoutingOutcome::PreAir(Reason::PkiFailed),
        Ok(E::DutyCycleLimit) => RoutingOutcome::Later(Reason::DutyCycle),
        Ok(E::RateLimitExceeded) => RoutingOutcome::Later(Reason::RateLimited),
        Ok(E::PkiUnknownPubkey) | Ok(E::PkiSendFailPublicKey) => {
            RoutingOutcome::Later(Reason::KeyUnknown)
        }
        Ok(E::NoInterface) => RoutingOutcome::Later(Reason::AdapterDown),
        _ => RoutingOutcome::PreAir(Reason::GotNak),
    }
}

const EV_WAITING: &str = "waiting for the radio";
const EV_ACCEPTED: &str = "accepted by the radio";
const EV_RELAYED: &str = "a neighbour rebroadcast it";
const EV_RELAYED_DM: &str = "relayed, not confirmed";
const EV_DELIVERED: &str = "the recipient confirmed";
const EV_UNCONFIRMED: &str = "transmitted, not confirmed";

// ------------------------------------------------------------------ state

#[derive(Default)]
struct State {
    running: bool,
    connected: bool,
    configured: bool,
    ever_configured: bool,
    link_up: bool,
    end_session: bool,
    config_id: Option<u32>,
    my_num: Option<u32>,
    region: Option<i32>,
    preset: Option<i32>,
    use_preset: bool,
    hop_limit: u32,
    channels: BTreeMap<u32, Chan>,
    nodes: HashMap<u32, Node>,
    inbound: VecDeque<Inbound>,
    dropped_texts: u64,
    prebuffer: VecDeque<pb::MeshPacket>,
    emitted_convs: HashMap<String, String>,
    nearby_dirty: bool,
    nearby_version: u64,
    source: Option<(String, String)>,
    queue: VecDeque<Job>,
    in_flight: HashMap<u32, Flight>,
    used_ids: VecDeque<u32>,
    used_set: HashSet<u32>,
    next_send_at: Duration,
    next_heartbeat: Duration,
    pause: Option<Pause>,
    /// Nodes whose NodeInfo with a key resumes a `key_unknown` DM.
    key_wait: HashSet<u32>,
    /// Nodes whose next NodeInfo resumes a DM after a post-air NAK.
    nack_wait: HashSet<u32>,
    counters: ClientCounters,
}

struct Inner {
    cfg: ClientConfig,
    clock: Arc<dyn MonoClock>,
    key: ConvKey,
    st: Mutex<State>,
    recv_wake: Notify,
    work_wake: Notify,
    log: Arc<LogRing>,
    allow: Arc<AllowStats>,
    run: Mutex<Option<RunHandle>>,
}

struct RunHandle {
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.st
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The adapter manifest's honest copy (`assets/adapters/meshtastic.toml`).
fn manifest() -> &'static toml::Value {
    static M: OnceLock<toml::Value> = OnceLock::new();
    M.get_or_init(|| {
        toml::from_str::<toml::Table>(include_str!("../../../assets/adapters/meshtastic.toml"))
            .map(toml::Value::Table)
            .unwrap_or(toml::Value::Table(Default::default()))
    })
}

fn manifest_str(path: &[&str]) -> String {
    let mut v = manifest();
    for p in path {
        match v.get(p) {
            Some(n) => v = n,
            None => return String::new(),
        }
    }
    v.as_str().unwrap_or_default().to_string()
}

impl State {
    fn my_account(&self) -> Option<String> {
        self.my_num.map(node_id)
    }

    fn primary(&self) -> Option<&Chan> {
        self.channels.values().find(|c| c.primary)
    }

    /// The key facts a channel effectively uses (an empty secondary PSK means the primary's).
    fn effective_key(&self, c: &Chan) -> ([u8; 32], bool, bool) {
        if c.psk_empty && !c.primary {
            if let Some(p) = self.primary() {
                return (p.psk_digest, p.default_key_own, p.keyed_own);
            }
        }
        (c.psk_digest, c.default_key_own, c.keyed_own)
    }

    fn effective_name(&self, c: &Chan) -> String {
        if c.raw_name.is_empty() {
            preset_display(self.preset.unwrap_or(0), self.use_preset).to_string()
        } else {
            c.raw_name.clone()
        }
    }

    fn channel_title(&self, c: &Chan) -> String {
        presence::sanitize_name(&self.effective_name(c), NAME_BYTES)
    }

    fn channel_native(&self, key: &ConvKey, c: &Chan) -> String {
        let (digest, _, _) = self.effective_key(c);
        let kh = key.kh(self.effective_name(c).as_bytes(), &digest);
        format!("ch:{}:{kh}", self.channel_title(c))
    }

    fn conv(&self, kind: ConvKind, native_id: String) -> Option<ConvRef> {
        Some(ConvRef {
            network: NETWORK_MESHTASTIC.to_string(),
            account: self.my_account()?,
            kind,
            native_id,
        })
    }

    fn push_inbound(&mut self, cap: usize, item: Inbound) {
        while self.inbound.len() >= cap.max(1) {
            if let Some(old) = self.inbound.pop_front() {
                self.counters.inbound_dropped += 1;
                if matches!(old, Inbound::Text(_)) {
                    self.dropped_texts += 1;
                }
            }
        }
        self.inbound.push_back(item);
    }

    /// Upsert a conversation once per (native id, title).
    fn upsert(&mut self, cap: usize, conv: ConvRef, title: String) {
        if self.emitted_convs.get(&conv.native_id) == Some(&title) {
            return;
        }
        if self.emitted_convs.len() >= EMITTED_CONVS {
            // bounded memory: forget them all; the core upserts idempotently
            self.emitted_convs.clear();
        }
        self.emitted_convs
            .insert(conv.native_id.clone(), title.clone());
        self.push_inbound(cap, Inbound::Conversation { conv, title });
    }

    /// Force one DM upsert (the "key learned" / "heard again" adapter event).
    fn nudge_dm(&mut self, cap: usize, num: u32) {
        let title = self.sender_name(num);
        if let Some(conv) = self.conv(ConvKind::Dm, dm_native(num)) {
            self.emitted_convs.remove(&conv.native_id);
            self.upsert(cap, conv, title);
        }
    }

    fn sender_name(&self, num: u32) -> String {
        self.nodes
            .get(&num)
            .filter(|n| n.user_known)
            .map(|n| n.long_name.clone())
            .unwrap_or_default()
    }

    fn upsert_channels(&mut self, cap: usize, key: &ConvKey) {
        let list: Vec<(ConvRef, String)> = self
            .channels
            .values()
            .filter_map(|c| {
                let conv = self.conv(ConvKind::Channel, self.channel_native(key, c))?;
                Some((conv, self.channel_title(c)))
            })
            .collect();
        for (conv, title) in list {
            self.upsert(cap, conv, title);
        }
    }

    fn node_mut(&mut self, num: u32) -> Option<&mut Node> {
        if num == 0 || num == BROADCAST {
            return None;
        }
        if !self.nodes.contains_key(&num) && self.nodes.len() >= MAX_NODES {
            // evict the least recently heard other node (the reader's ranking)
            let my = self.my_num;
            if let Some(victim) = self
                .nodes
                .values()
                .filter(|n| Some(n.num) != my)
                .min_by_key(|n| (n.last_heard.unwrap_or(0), std::cmp::Reverse(n.num)))
                .map(|n| n.num)
            {
                self.nodes.remove(&victim);
            }
        }
        self.nearby_dirty = true;
        self.nearby_version += 1;
        Some(
            self.nodes
                .entry(num)
                .or_insert_with(|| Node::presumptive(num)),
        )
    }

    /// A key or a NodeInfo was heard for `num`: resume what waits on it (§5).
    fn node_heard(&mut self, cap: usize, num: u32, has_key: bool) {
        if has_key && self.key_wait.remove(&num) {
            self.nudge_dm(cap, num);
        }
        if self.nack_wait.remove(&num) {
            self.nudge_dm(cap, num);
        }
    }

    /// Fail every job the adapter still holds (no configured session to send it on).
    fn give_back_queue(&mut self) {
        for job in self.queue.drain(..) {
            job.emit(SendEvent::Failed {
                reason: Reason::AdapterDown,
                retry: Retry::Later,
            });
        }
    }

    fn new_packet_id(&mut self) -> Option<u32> {
        for _ in 0..64 {
            let id = rand_u32()?;
            if id != 0 && !self.used_set.contains(&id) && !self.in_flight.contains_key(&id) {
                if self.used_ids.len() >= USED_IDS {
                    if let Some(old) = self.used_ids.pop_front() {
                        self.used_set.remove(&old);
                    }
                }
                self.used_ids.push_back(id);
                self.used_set.insert(id);
                return Some(id);
            }
        }
        None
    }
}

// ------------------------------------------------------------------ the adapter

/// The Meshtastic adapter (see the module docs).
pub struct MeshtasticAdapter {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for MeshtasticAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.inner.lock();
        f.debug_struct("MeshtasticAdapter")
            .field("downlink", &self.inner.cfg.downlink)
            .field("running", &st.running)
            .field("connected", &st.connected)
            .field("configured", &st.configured)
            .field("my_node", &st.my_account())
            .field("channels", &st.channels.len())
            .field("nodes", &st.nodes.len())
            .field("queued", &st.queue.len())
            .field("in_flight", &st.in_flight.len())
            .field("counters", &st.counters)
            .finish_non_exhaustive()
    }
}

impl MeshtasticAdapter {
    /// A stopped client. `conv_key` is the 32-byte key the daemon derives under
    /// `111/bridge-conv/v1` (§3); it is kept zeroizing and never shown.
    pub fn new(cfg: ClientConfig, conv_key: [u8; 32]) -> Arc<Self> {
        Self::with_clock(cfg, conv_key, Arc::new(SystemClock::default()))
    }

    /// As [`MeshtasticAdapter::new`] with an injected monotonic clock (tests).
    pub fn with_clock(
        cfg: ClientConfig,
        conv_key: [u8; 32],
        clock: Arc<dyn MonoClock>,
    ) -> Arc<Self> {
        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(&conv_key);
        let mut conv_key = conv_key;
        conv_key.zeroize();
        let log = Arc::new(LogRing::new());
        let allow = Arc::new(AllowStats::new(Some(Arc::clone(&log))));
        Arc::new(MeshtasticAdapter {
            inner: Arc::new(Inner {
                cfg,
                clock,
                key: ConvKey(key),
                st: Mutex::new(State {
                    hop_limit: 3,
                    use_preset: true,
                    ..Default::default()
                }),
                recv_wake: Notify::new(),
                work_wake: Notify::new(),
                log,
                allow,
                run: Mutex::new(None),
            }),
        })
    }

    pub fn config(&self) -> &ClientConfig {
        &self.inner.cfg
    }

    /// The Nearby source identity (the bound device id and the board's label).
    pub fn set_source(&self, device_id: &str, label: &str) {
        self.inner.lock().source = Some((device_id.to_string(), label.to_string()));
    }

    /// The connection task exists.
    pub fn running(&self) -> bool {
        self.inner
            .run
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|r| !r.task.is_finished())
    }

    /// Start the connection task (no-op while running). Needs a tokio runtime.
    pub fn start(&self) {
        let mut run = self.inner.run.lock().unwrap_or_else(|e| e.into_inner());
        if run.as_ref().is_some_and(|r| !r.task.is_finished()) {
            return;
        }
        let (stop, rx) = watch::channel(false);
        self.inner.lock().running = true;
        let inner = Arc::clone(&self.inner);
        let task = tokio::spawn(run_loop(inner, rx));
        *run = Some(RunHandle { stop, task });
        self.inner.log.line("started");
    }

    /// Stop the task and wait for it; jobs not yet written go back to the core
    /// (`adapter_down`, later).
    pub async fn stop(&self) {
        let handle = self
            .inner
            .run
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(h) = handle {
            let _ = h.stop.send(true);
            let _ = h.task.await;
            self.inner.log.line("stopped");
        }
        let mut st = self.inner.lock();
        st.running = false;
        st.connected = false;
        st.configured = false;
        st.give_back_queue();
        st.in_flight.clear();
        st.pause = None;
        if st.link_up {
            st.link_up = false;
            let cap = self.inner.cfg.inbound_capacity;
            st.push_inbound(cap, Inbound::Link(LinkState::Down));
        }
        drop(st);
        self.inner.recv_wake.notify_one();
    }

    /// Live board facts (§5).
    pub fn mesh_facts(&self) -> MeshFacts {
        let st = self.inner.lock();
        let own = st.my_num.and_then(|n| st.nodes.get(&n));
        let channels = st
            .channels
            .values()
            .map(|c| {
                let (_, default_key, keyed) = st.effective_key(c);
                ChannelFact {
                    index: c.index,
                    name: st.channel_title(c),
                    conv_id: st.channel_native(&self.inner.key, c),
                    uplink_enabled: c.uplink,
                    default_key,
                    encrypted: keyed,
                    primary: c.primary,
                }
            })
            .collect();
        let mut counters = st.counters.clone();
        counters.allowlist_passed = self.inner.allow.passed();
        counters.allowlist_dropped = self.inner.allow.dropped();
        MeshFacts {
            running: st.running,
            connected: st.connected,
            configured: st.configured,
            my_node: st.my_account(),
            long_name: own.filter(|n| n.user_known).map(|n| n.long_name.clone()),
            short_name: own.filter(|n| n.user_known).map(|n| n.short_name.clone()),
            licensed: own.filter(|n| n.user_known).map(|n| n.licensed),
            region: st.region.map(region_name),
            region_set: st.region.map(|r| r != 0),
            preset: st.preset.map(preset_name),
            channels,
            nodes: st
                .nodes
                .keys()
                .filter(|n| Some(**n) != st.my_num)
                .count()
                .min(u32::MAX as usize) as u32,
            queued: st.queue.len() as u32,
            in_flight: st.in_flight.len() as u32,
            paused: st.pause.is_some(),
            counters,
        }
    }

    /// The allowlist's counters.
    pub fn allow_stats(&self) -> &Arc<AllowStats> {
        &self.inner.allow
    }

    /// Recent log lines (they never carry content, keys or names).
    pub fn recent_log(&self) -> Vec<String> {
        self.inner.log.recent()
    }

    /// The client's node DB has been through a config stream.
    pub fn nearby_ready(&self) -> bool {
        let st = self.inner.lock();
        st.connected && st.ever_configured && st.my_num.is_some()
    }

    /// Bumped on every node DB change (the module publishes SSE `nearby` on a change).
    pub fn nearby_version(&self) -> u64 {
        self.inner.lock().nearby_version
    }

    /// The reader-format document (`meshtastic.json` schema 1) of the node DB: the board's
    /// own node excluded, at most 4096 nodes (the least recently heard go first), sorted by
    /// node number.
    pub fn reader_document(&self) -> Value {
        let st = self.inner.lock();
        reader_document(&st)
    }

    /// Nearby nodes as `GET /nearby` publishes them (the reader-format rows through the
    /// daemon's own ingest, `presence::parse`).
    pub fn nearby_nodes(&self) -> Vec<NearbyNode> {
        let doc = self.reader_document();
        let bytes = serde_json::to_vec(&doc).unwrap_or_default();
        match presence::parse(&bytes) {
            Ok(n) => n,
            Err(e) => {
                self.inner
                    .log
                    .line(format!("node list refused by the Nearby ingest ({e})"));
                Vec::new()
            }
        }
    }

    /// The `NearbySource` (`kind: meshtastic`) of the board.
    pub fn nearby_source(&self) -> NearbySource {
        let (id, label) = {
            let st = self.inner.lock();
            st.source.clone().unwrap_or_else(|| {
                let id = st.my_account().unwrap_or_else(|| "board".into());
                (id.clone(), id)
            })
        };
        NearbySource::meshtastic(&id, &label, self.nearby_nodes())
    }

    fn caps_now(&self) -> Caps {
        let st = self.inner.lock();
        // live once configured with a region; an error only after a session dropped;
        // otherwise a human fact is missing (no board running, region unset)
        let state = if st.running && st.configured && st.region != Some(0) {
            AdapterState::Live
        } else if st.running && !st.configured && st.ever_configured {
            AdapterState::Error("the board's downlink dropped; reconnecting".into())
        } else {
            AdapterState::NotSet
        };
        let airtime = st.preset.and_then(|p| {
            Some(AirtimeProfile {
                duty_cycle_permille: region_duty_permille(st.region.unwrap_or(0)),
                bits_per_s: preset_bits_per_s(p)?,
                preset: Some(preset_name(p)),
            })
        });
        Caps {
            kinds: vec![ConvKind::Channel, ConvKind::Dm],
            max_text_bytes: PerKind {
                channel: Some(TEXT_CHANNEL_MAX),
                dm: Some(TEXT_PKI_DM_MAX),
                group: None,
            },
            evidence: vec![
                ForeignMessageState::Queued,
                ForeignMessageState::Sent,
                ForeignMessageState::Relayed,
                ForeignMessageState::Delivered,
            ],
            airtime,
            auth_levels: vec![Auth::None, Auth::ChannelKey, Auth::Pki],
            copy: Honest {
                label: manifest_str(&["label"]),
                chip: manifest_str(&["chip"]),
                tag: manifest_str(&["tag"]),
                banner: manifest_str(&["copy", "banner"]),
                send_warning: manifest_str(&["copy", "send_warning"]),
            },
            recipient_syntax: None,
            splittable: true,
            state,
        }
    }

    /// Validate and queue one text (see the module docs for the events).
    fn enqueue(
        &self,
        conv: &ConvRef,
        text: &str,
        tx: &mpsc::Sender<SendEvent>,
    ) -> Result<(), (Reason, Retry)> {
        let mut st = self.inner.lock();
        if !st.running || !st.connected || !st.configured {
            return Err((Reason::AdapterDown, Retry::Later));
        }
        if conv.network != NETWORK_MESHTASTIC || Some(conv.account.clone()) != st.my_account() {
            return Err((Reason::NoChannel, Retry::Never));
        }
        if st.region == Some(0) {
            return Err((Reason::RegionUnset, Retry::Never));
        }
        let licensed = st
            .my_num
            .and_then(|n| st.nodes.get(&n))
            .is_some_and(|n| n.user_known && n.licensed);
        let (to, channel, dm, cap) = match conv.kind {
            ConvKind::Channel => {
                let ch = st
                    .channels
                    .values()
                    .find(|c| st.channel_native(&self.inner.key, c) == conv.native_id)
                    .ok_or((Reason::NoChannel, Retry::Never))?;
                (BROADCAST, ch.index, false, TEXT_CHANNEL_MAX)
            }
            ConvKind::Dm => {
                let num = parse_dm(&conv.native_id)
                    .filter(|n| *n != 0 && *n != BROADCAST && Some(*n) != st.my_num)
                    .ok_or((Reason::NoChannel, Retry::Never))?;
                let cap = if licensed {
                    TEXT_LICENSED_DM_MAX
                } else {
                    TEXT_PKI_DM_MAX
                };
                (num, 0, true, cap)
            }
            ConvKind::Group => return Err((Reason::NoChannel, Retry::Never)),
        };
        if text.len() > cap {
            return Err((Reason::TooLarge, Retry::Never));
        }
        if st.queue.len() >= SEND_QUEUE {
            return Err((Reason::AdapterDown, Retry::Later));
        }
        let paused = st.pause.is_some();
        let job = Job {
            to,
            channel,
            dm,
            text: text.as_bytes().to_vec(),
            tx: tx.clone(),
            wait_noted: paused,
        };
        if paused {
            job.emit(SendEvent::Evidence {
                state: ForeignMessageState::Queued,
                evidence: EV_WAITING.into(),
            });
        }
        st.queue.push_back(job);
        Ok(())
    }
}

/// The reader's own file cap (`meshtastic_presence.py` `MAX_FILE_BYTES`, below the daemon's
/// 1 MiB ingest limit).
const READER_MAX_FILE_BYTES: usize = 1_000_000;

/// The reader-format document of a state (see [`MeshtasticAdapter::reader_document`]): the
/// reader's `encode_document` (rank by most recently heard, keep 4096, then cut whole rows
/// until the compact JSON fits its file cap; rows in node-number order).
fn reader_document(st: &State) -> Value {
    let mut ranked: Vec<&Node> = st
        .nodes
        .values()
        .filter(|n| Some(n.num) != st.my_num)
        .collect();
    ranked.sort_by_key(|n| {
        (
            std::cmp::Reverse(n.last_heard.filter(|t| *t > 0).unwrap_or(0)),
            n.num,
        )
    });
    ranked.truncate(MAX_NODES);
    let dump = |keep: &[&Node]| {
        let mut rows: Vec<&Node> = keep.to_vec();
        rows.sort_by_key(|n| n.num);
        json!({
            "schema": presence::SCHEMA,
            "nodes": rows.iter().map(|n| n.reader_row()).collect::<Vec<_>>(),
        })
    };
    let mut doc = dump(&ranked);
    loop {
        let len = serde_json::to_vec(&doc).map(|v| v.len() + 1).unwrap_or(0);
        if len <= READER_MAX_FILE_BYTES || ranked.is_empty() {
            return doc;
        }
        let avg = (len / ranked.len()).max(1);
        let over = len - READER_MAX_FILE_BYTES;
        let cut = over.div_ceil(avg).clamp(1, ranked.len());
        ranked.truncate(ranked.len() - cut);
        doc = dump(&ranked);
    }
}

impl Adapter for MeshtasticAdapter {
    fn id(&self) -> &'static str {
        NETWORK_MESHTASTIC
    }

    fn caps(&self) -> Caps {
        self.caps_now()
    }

    fn recv(&self) -> BoxFuture<'_, Inbound> {
        Box::pin(async move {
            loop {
                let notified = self.inner.recv_wake.notified();
                {
                    let mut st = self.inner.lock();
                    if st.dropped_texts > 0 {
                        let n = std::mem::take(&mut st.dropped_texts);
                        return Inbound::Dropped { n };
                    }
                    if let Some(item) = st.inbound.pop_front() {
                        return item;
                    }
                    if st.nearby_dirty && st.ever_configured {
                        st.nearby_dirty = false;
                        drop(st);
                        let src = self.nearby_source();
                        if let Ok(v) = serde_json::to_value(&src) {
                            return Inbound::Nearby(v);
                        }
                        continue;
                    }
                }
                notified.await;
            }
        })
    }

    fn send(&self, conv: &ConvRef, text: &str, _o: SendOpts) -> BoxFuture<'_, SendHandle> {
        // `want_ack` is always true on Meshtastic (§5), whatever the options say
        let (tx, rx) = mpsc::channel(16);
        match self.enqueue(conv, text, &tx) {
            Ok(()) => self.inner.work_wake.notify_one(),
            Err((reason, retry)) => {
                let _ = tx.try_send(SendEvent::Failed { reason, retry });
            }
        }
        drop(tx);
        Box::pin(async move { SendHandle { events: rx } })
    }

    fn facts(&self) -> AdapterFacts {
        let f = self.mesh_facts();
        let account = f.my_node.clone().unwrap_or_default();
        AdapterFacts {
            accounts: f
                .my_node
                .iter()
                .map(|id| AccountFacts {
                    id: id.clone(),
                    label: Some(match &f.long_name {
                        Some(n) if !n.is_empty() => format!("{n} ({id})"),
                        _ => id.clone(),
                    }),
                    licensed: f.licensed.unwrap_or(false),
                })
                .collect(),
            channels: f
                .channels
                .iter()
                .map(|c| ChannelFacts {
                    account: account.clone(),
                    native_id: c.conv_id.clone(),
                    uplink: c.uplink_enabled,
                    default_key: c.default_key,
                })
                .collect(),
            region_set: f.region_set,
            ids_globally_unique: false,
            parts_may_reorder: true,
            licensed_caps: Some(PerKind {
                channel: Some(TEXT_CHANNEL_MAX),
                dm: Some(TEXT_LICENSED_DM_MAX),
                group: None,
            }),
        }
    }
}

// ------------------------------------------------------------------ receive

impl Inner {
    fn on_connect(&self, config_id: u32, now: Duration) {
        let mut st = self.lock();
        st.connected = true;
        st.configured = false;
        st.end_session = false;
        st.config_id = Some(config_id);
        st.next_heartbeat = now + self.cfg.heartbeat_every;
        st.next_send_at = st.next_send_at.max(now);
        st.pause = None;
        st.counters.sessions += 1;
    }

    fn on_session_end(&self, why: &str) {
        let mut st = self.lock();
        let was_configured = st.configured;
        st.connected = false;
        st.configured = false;
        st.pause = None;
        st.give_back_queue();
        // texts heard before a completion that never came: map them with the table we
        // have, or count them
        let pre: Vec<pb::MeshPacket> = st.prebuffer.drain(..).collect();
        let cap = self.cfg.inbound_capacity;
        for p in pre {
            if st.ever_configured {
                self.text(&mut st, &p);
            } else {
                st.counters.prebuffer_dropped += 1;
            }
        }
        if st.link_up {
            st.link_up = false;
            st.push_inbound(cap, Inbound::Link(LinkState::Down));
        }
        drop(st);
        self.recv_wake.notify_one();
        self.log.line(format!(
            "downlink session ended ({why}; it was {}configured)",
            if was_configured { "" } else { "not " }
        ));
    }

    /// Ingest one read's frames (each zeroized after use).
    fn on_frames(&self, frames: Vec<Vec<u8>>, now: Duration) {
        let mut st = self.lock();
        for mut f in frames {
            if f.is_empty() {
                continue;
            }
            match pb::FromRadio::decode(&f[..]) {
                Ok(msg) => self.handle_from_radio(&mut st, msg, now),
                Err(_) => {
                    st.counters.noise_frames += 1;
                    if st.counters.noise_frames == 1
                        || st.counters.noise_frames.is_multiple_of(1000)
                    {
                        self.log.line(format!(
                            "{} downlink frame(s) did not decode as FromRadio (counted as noise)",
                            st.counters.noise_frames
                        ));
                    }
                }
            }
            f.zeroize();
        }
        drop(st);
        self.recv_wake.notify_one();
        self.work_wake.notify_one();
    }

    fn handle_from_radio(&self, st: &mut State, msg: pb::FromRadio, now: Duration) {
        use pb::from_radio::PayloadVariant as V;
        let cap = self.cfg.inbound_capacity;
        match msg.payload_variant {
            Some(V::MyInfo(mi)) => {
                let num = mi.my_node_num;
                if num == 0 || num == BROADCAST {
                    return;
                }
                if st.my_num.is_some_and(|m| m != num) {
                    // another board behind the same binding: nothing of the old one stands
                    self.log
                        .line("the board's node number changed; its tables were reset");
                    st.channels.clear();
                    st.nodes.clear();
                    st.emitted_convs.clear();
                    st.key_wait.clear();
                    st.nack_wait.clear();
                    st.give_back_queue();
                    st.in_flight.clear();
                    st.nearby_dirty = true;
                    st.nearby_version += 1;
                }
                st.my_num = Some(num);
            }
            Some(V::NodeInfo(ni)) => self.node_info(st, &ni),
            Some(V::Channel(mut ch)) => {
                self.channel(st, &mut ch);
                if st.configured {
                    st.upsert_channels(cap, &self.key);
                }
            }
            Some(V::Config(mut c)) => match &mut c.payload_variant {
                Some(pb::config::PayloadVariant::Lora(l)) => {
                    st.region = Some(l.region);
                    st.preset = Some(l.modem_preset);
                    st.use_preset = l.use_preset;
                    st.hop_limit = if (1..=allowlist::MAX_HOP_LIMIT).contains(&l.hop_limit) {
                        l.hop_limit
                    } else {
                        3
                    };
                }
                Some(pb::config::PayloadVariant::Security(s)) => {
                    s.private_key.zeroize();
                }
                _ => {}
            },
            Some(V::ConfigCompleteId(id)) => self.config_complete(st, id),
            Some(V::Packet(p)) => self.packet(st, p),
            Some(V::QueueStatus(qs)) => self.queue_status(st, &qs, now),
            Some(V::Rebooted(_)) => {
                // the board's API session restarted: a new connection asks for config once
                st.end_session = true;
            }
            _ => {}
        }
    }

    fn node_info(&self, st: &mut State, ni: &pb::NodeInfo) {
        let cap = self.cfg.inbound_capacity;
        let Some(n) = st.node_mut(ni.num) else { return };
        // meshtastic-python `node.update(MessageToDict(nodeInfo))`: only fields present
        if let Some(u) = &ni.user {
            n.set_user(u);
        }
        if let Some(p) = &ni.position {
            n.pos = Some(PosRaw::from(p));
        }
        if ni.snr != 0.0 {
            n.snr = py_f32(ni.snr);
        }
        if ni.last_heard != 0 {
            n.last_heard = Some(ni.last_heard);
        }
        if let Some(dm) = &ni.device_metrics {
            n.util = dm.channel_utilization.and_then(py_f32);
        }
        if let Some(h) = ni.hops_away {
            n.hops_away = Some(h);
        }
        let has_key = n.pubkey.is_some();
        let num = ni.num;
        st.node_heard(cap, num, has_key);
    }

    fn channel(&self, st: &mut State, ch: &mut pb::Channel) {
        let Ok(index) = u32::try_from(ch.index) else {
            return;
        };
        if index > allowlist::MAX_CHANNEL_INDEX {
            return;
        }
        let role = ch.role;
        let Some(s) = ch.settings.as_mut() else {
            st.channels.remove(&index);
            return;
        };
        let psk = std::mem::take(&mut s.psk);
        let psk = Zeroizing::new(psk);
        if role == pb::channel::Role::Disabled as i32 {
            st.channels.remove(&index);
            return;
        }
        let chan = Chan {
            index,
            raw_name: s.name.clone(),
            psk_digest: self.key.psk_digest(&psk),
            psk_empty: psk.is_empty(),
            default_key_own: psk.len() <= 1 || psk[..] == DEFAULT_PSK,
            keyed_own: !(psk.is_empty() || psk[..] == [0]),
            uplink: s.uplink_enabled,
            primary: role == pb::channel::Role::Primary as i32,
        };
        drop(psk);
        st.channels.insert(index, chan);
    }

    fn config_complete(&self, st: &mut State, id: u32) {
        let cap = self.cfg.inbound_capacity;
        let ours = st.config_id == Some(id);
        let first = !st.configured;
        st.configured = true;
        st.ever_configured = true;
        st.upsert_channels(cap, &self.key);
        let pre: Vec<pb::MeshPacket> = st.prebuffer.drain(..).collect();
        for p in pre {
            self.text(st, &p);
        }
        st.nearby_dirty = true;
        st.nearby_version += 1;
        if first {
            st.counters.configured += 1;
            if !st.link_up {
                st.link_up = true;
                st.push_inbound(cap, Inbound::Link(LinkState::Up));
            }
            self.log.line(format!(
                "config stream complete ({}; {} channel(s), {} node(s))",
                if ours { "ours" } else { "another client's" },
                st.channels.len(),
                st.nodes.len()
            ));
        }
    }

    fn packet(&self, st: &mut State, p: pb::MeshPacket) {
        let cap = self.cfg.inbound_capacity;
        let Some(pb::mesh_packet::PayloadVariant::Decoded(d)) = &p.payload_variant else {
            return;
        };
        const RANGE_TEST: i32 = 66;
        const DETECTION_SENSOR: i32 = 10;
        let port = d.portnum;
        if port == pb::PortNum::TextMessageApp as i32
            || port == RANGE_TEST
            || port == DETECTION_SENSOR
        {
            if let Some(n) = st.node_mut(p.from) {
                n.info_update(&p);
            }
            if port == pb::PortNum::TextMessageApp as i32 {
                if st.configured {
                    self.text(st, &p);
                } else {
                    if st.prebuffer.len() >= self.cfg.prebuffer_capacity.max(1) {
                        st.prebuffer.pop_front();
                        st.counters.prebuffer_dropped += 1;
                    }
                    st.prebuffer.push_back(p.clone());
                }
            }
        } else if port == pb::PortNum::PositionApp as i32 {
            if let (Ok(pos), Some(n)) = (pb::Position::decode(&d.payload[..]), st.node_mut(p.from))
            {
                n.pos = Some(PosRaw::from(&pos));
            }
        } else if port == pb::PortNum::NodeinfoApp as i32 {
            if let Ok(u) = pb::User::decode(&d.payload[..]) {
                if let Some(n) = st.node_mut(p.from) {
                    n.set_user(&u);
                    n.info_update(&p);
                    let has_key = n.pubkey.is_some();
                    st.node_heard(cap, p.from, has_key);
                }
            }
        } else if port == pb::PortNum::TelemetryApp as i32 {
            if let Ok(t) = pb::Telemetry::decode(&d.payload[..]) {
                if let Some(pb::telemetry::Variant::DeviceMetrics(dm)) = t.variant {
                    if let (Some(u), Some(n)) = (dm.channel_utilization, st.node_mut(p.from)) {
                        n.util = py_f32(u);
                    }
                }
            }
        } else if port == pb::PortNum::RoutingApp as i32 {
            self.routing(st, &p, d);
        }
    }

    /// Portnum 1 → an inbound text (configured sessions; earlier ones were buffered).
    fn text(&self, st: &mut State, p: &pb::MeshPacket) {
        let cap = self.cfg.inbound_capacity;
        let Some(pb::mesh_packet::PayloadVariant::Decoded(d)) = &p.payload_variant else {
            return;
        };
        if d.emoji != 0 && d.reply_id != 0 {
            st.counters.tapbacks += 1;
            return;
        }
        let Some(my) = st.my_num else { return };
        if p.from == 0 || p.from == BROADCAST {
            st.counters.not_for_us += 1;
            return;
        }
        let (conv, auth) = if p.to == BROADCAST {
            let Some(ch) = st.channels.get(&p.channel).cloned() else {
                st.counters.unknown_channel += 1;
                return;
            };
            let (_, _, keyed) = st.effective_key(&ch);
            let Some(conv) = st.conv(ConvKind::Channel, st.channel_native(&self.key, &ch)) else {
                return;
            };
            (conv, if keyed { Auth::ChannelKey } else { Auth::None })
        } else if p.to == my {
            let auth = if p.pki_encrypted {
                Auth::Pki
            } else if st.primary().is_some_and(|c| c.keyed_own) {
                Auth::ChannelKey
            } else {
                Auth::None
            };
            let Some(conv) = st.conv(ConvKind::Dm, dm_native(p.from)) else {
                return;
            };
            let title = st.sender_name(p.from);
            st.upsert(cap, conv.clone(), title);
            (conv, auth)
        } else {
            st.counters.not_for_us += 1;
            return;
        };
        let mut meta = Map::new();
        if let Some(s) = (p.rx_snr != 0.0).then(|| py_f32(p.rx_snr)).flatten() {
            meta.insert("snr".into(), json!(s));
        }
        if p.rx_rssi != 0 {
            meta.insert("rssi".into(), json!(p.rx_rssi));
        }
        if p.hop_start > 0 && p.hop_start >= p.hop_limit {
            meta.insert("hops".into(), json!(p.hop_start - p.hop_limit));
        }
        meta.insert("via_mqtt".into(), json!(p.via_mqtt));
        if let Some(b) = d.bitfield {
            meta.insert("ok_to_mqtt".into(), json!(b & 1 != 0));
        }
        if p.from == my {
            meta.insert("own_echo".into(), json!(true));
        }
        let item = InboundText {
            conv,
            sender_native: node_id(p.from),
            sender_name: st.sender_name(p.from),
            auth,
            native_msg_id: format!("{:08x}", p.id),
            body: String::from_utf8_lossy(&d.payload).into_owned(),
            claim_ts: (p.rx_time != 0).then_some(i64::from(p.rx_time)),
            meta: Value::Object(meta),
        };
        st.push_inbound(cap, Inbound::Text(item));
    }

    fn routing(&self, st: &mut State, p: &pb::MeshPacket, d: &pb::Data) {
        let req = d.request_id;
        if req == 0 || !st.in_flight.contains_key(&req) {
            return;
        }
        if st.my_num.is_some_and(|m| p.to != m) {
            return;
        }
        let Ok(r) = pb::Routing::decode(&d.payload[..]) else {
            return;
        };
        let code = match r.variant {
            Some(pb::routing::Variant::ErrorReason(e)) => e,
            None => 0,
            Some(_) => return,
        };
        let implicit = st.my_num == Some(p.from);
        let Some(mut fl) = st.in_flight.remove(&req) else {
            return;
        };
        let rank = |s: Option<ForeignMessageState>| s.map_or(0, |s| s.rank() + 1);
        match map_routing_error(code) {
            RoutingOutcome::Ack => {
                fl.accept();
                if fl.job.dm && !implicit && p.from == fl.job.to {
                    fl.job.emit(SendEvent::Evidence {
                        state: ForeignMessageState::Delivered,
                        evidence: EV_DELIVERED.into(),
                    });
                    return; // closes the handle
                }
                if rank(fl.state) < rank(Some(ForeignMessageState::Relayed)) {
                    fl.job.emit(SendEvent::Evidence {
                        state: ForeignMessageState::Relayed,
                        evidence: if fl.job.dm { EV_RELAYED_DM } else { EV_RELAYED }.into(),
                    });
                    fl.state = Some(ForeignMessageState::Relayed);
                }
                if fl.job.dm {
                    st.in_flight.insert(req, fl); // a real ack may still come
                }
            }
            RoutingOutcome::PreAir(reason) => fl.job.emit(SendEvent::Failed {
                reason,
                retry: Retry::Never,
            }),
            RoutingOutcome::Later(reason) => {
                if reason == Reason::KeyUnknown && fl.job.dm {
                    st.key_wait.insert(fl.job.to);
                }
                fl.job.emit(SendEvent::Failed {
                    reason,
                    retry: Retry::Later,
                });
            }
            RoutingOutcome::PostAir(reason) => {
                fl.accept();
                let state = match fl.state {
                    Some(ForeignMessageState::Relayed) => ForeignMessageState::Relayed,
                    _ => ForeignMessageState::Sent,
                };
                fl.job.emit(SendEvent::Evidence {
                    state,
                    evidence: EV_UNCONFIRMED.into(),
                });
                if fl.job.dm {
                    st.nack_wait.insert(fl.job.to);
                    fl.job.emit(SendEvent::Failed {
                        reason,
                        retry: Retry::Later,
                    });
                }
            }
        }
    }

    fn queue_status(&self, st: &mut State, qs: &pb::QueueStatus, now: Duration) {
        st.counters.queue_statuses += 1;
        if qs.maxlen > 0 {
            if qs.free == 0 {
                if st.pause.is_none() {
                    st.pause = Some(Pause {
                        probe_at: now + self.cfg.pause_probe,
                        step: self.cfg.pause_probe,
                    });
                    for job in st.queue.iter_mut() {
                        if !job.wait_noted {
                            job.wait_noted = true;
                            job.emit(SendEvent::Evidence {
                                state: ForeignMessageState::Queued,
                                evidence: EV_WAITING.into(),
                            });
                        }
                    }
                }
            } else if st.pause.take().is_some() {
                for job in st.queue.iter_mut() {
                    job.wait_noted = false;
                }
            }
        }
        let id = qs.mesh_packet_id;
        if id == 0 {
            return;
        }
        if qs.res == 0 {
            if let Some(fl) = st.in_flight.get_mut(&id) {
                fl.qs_ok_at.get_or_insert(now);
            }
        } else if let Some(fl) = st.in_flight.remove(&id) {
            self.log.line(format!(
                "the board refused to queue a text (status {}); handed back",
                qs.res
            ));
            fl.job.emit(SendEvent::Failed {
                reason: Reason::AdapterDown,
                retry: Retry::Later,
            });
        }
    }

    // -------------------------------------------------------------- send work

    /// What to write now (heartbeat, probe, at most one paced text) and the timers due.
    fn take_work(&self, now: Duration) -> Vec<Work> {
        let mut st = self.lock();
        let mut out = Vec::new();
        if now >= st.next_heartbeat {
            out.push(Work::Frame(to_radio_heartbeat()));
            st.next_heartbeat = now + self.cfg.heartbeat_every;
        }
        if let Some(p) = st.pause.as_mut() {
            if now >= p.probe_at {
                // a nonce-0 heartbeat makes the board answer with a QueueStatus
                out.push(Work::Frame(to_radio_heartbeat()));
                p.step = (p.step * 2).min(self.cfg.pause_probe_max);
                p.probe_at = now + p.step;
            }
        }
        // settle: res-0 statuses that saw no rate-limit NAK
        let settle = self.cfg.sent_settle;
        for fl in st.in_flight.values_mut() {
            if fl.state.is_none() && fl.qs_ok_at.is_some_and(|t| now >= t + settle) {
                fl.state = Some(ForeignMessageState::Sent);
                fl.accept();
                fl.job.emit(SendEvent::Evidence {
                    state: ForeignMessageState::Sent,
                    evidence: EV_ACCEPTED.into(),
                });
            }
        }
        // channel texts are final once relayed or sent-and-settled long enough; everything
        // ends at the TTL
        let ttl = IN_FLIGHT_TTL;
        st.in_flight.retain(|_, fl| now < fl.written_at + ttl);
        if st.configured && st.pause.is_none() && now >= st.next_send_at && !st.queue.is_empty() {
            if let Some(id) = st.new_packet_id() {
                if let Some(job) = st.queue.pop_front() {
                    let hop = st.hop_limit;
                    let frame = to_radio_text(text_packet(job.to, job.channel, id, hop, &job.text));
                    match shim::frame(&frame) {
                        Ok(frame) => {
                            st.next_send_at = now + self.cfg.pacing;
                            out.push(Work::Text { id, frame, job });
                        }
                        Err(_) => job.emit(SendEvent::Failed {
                            reason: Reason::TooLarge,
                            retry: Retry::Never,
                        }),
                    }
                }
            } else {
                st.next_send_at = now + self.cfg.pacing;
                self.log
                    .line("no packet id could be drawn; the queue waits");
            }
        }
        out
    }

    fn on_written(&self, id: u32, job: Job, now: Duration) {
        let mut st = self.lock();
        st.counters.texts_written += 1;
        if st.in_flight.len() >= IN_FLIGHT {
            if let Some(oldest) = st
                .in_flight
                .iter()
                .min_by_key(|(_, f)| f.written_at)
                .map(|(k, _)| *k)
            {
                st.in_flight.remove(&oldest);
            }
        }
        st.in_flight.insert(
            id,
            Flight {
                id,
                job,
                accepted: false,
                written_at: now,
                state: None,
                qs_ok_at: None,
            },
        );
    }

    /// The next time the session must wake on its own.
    fn next_deadline(&self, now: Duration) -> Duration {
        let st = self.lock();
        let mut t = st.next_heartbeat;
        if let Some(p) = &st.pause {
            t = t.min(p.probe_at);
        }
        if st.configured && st.pause.is_none() && !st.queue.is_empty() {
            t = t.min(st.next_send_at);
        }
        for fl in st.in_flight.values() {
            if let (None, Some(q)) = (fl.state, fl.qs_ok_at) {
                t = t.min(q + self.cfg.sent_settle);
            }
            t = t.min(fl.written_at + IN_FLIGHT_TTL);
        }
        t.max(now)
    }
}

// ------------------------------------------------------------------ the task

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    Stopped,
    Eof,
    Read,
    Write,
    Rebooted,
    NoRandom,
}

async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(
    inner: &Inner,
    w: &mut W,
    bytes: &[u8],
) -> bool {
    matches!(
        tokio::time::timeout(
            inner.cfg.write_timeout,
            allowlist::write_all_flush(w, bytes)
        )
        .await,
        Ok(Ok(()))
    )
}

/// One downlink connection: one `want_config_id`, then frames in, work out.
async fn session(inner: &Arc<Inner>, stream: TcpStream, stop: &mut watch::Receiver<bool>) -> End {
    let _ = stream.set_nodelay(true);
    let (rd, wr) = stream.into_split();
    let mut w = AllowlistWriter::new(wr, Arc::clone(&inner.allow));
    let Some(config_id) = random_config_id() else {
        inner.log.line("no random config id could be drawn");
        return End::NoRandom;
    };
    inner.on_connect(config_id, inner.clock.now());
    let Ok(want) = shim::frame(&to_radio_want_config(config_id)) else {
        return End::Write;
    };
    if !write_frame(inner, &mut w, &want).await {
        return End::Write;
    }
    let mut deframer = Deframer::new();
    let mut buf = vec![0u8; 4096];
    loop {
        let now = inner.clock.now();
        for item in inner.take_work(now) {
            match item {
                Work::Frame(bytes) => {
                    let Ok(f) = shim::frame(&bytes) else { continue };
                    if !write_frame(inner, &mut w, &f).await {
                        return End::Write;
                    }
                }
                Work::Text { id, frame, job } => {
                    if write_frame(inner, &mut w, &frame).await {
                        inner.on_written(id, job, inner.clock.now());
                    } else {
                        // the frame may or may not have left: the handle closes with
                        // neither Accepted nor Failed ("may have been sent")
                        drop(job);
                        return End::Write;
                    }
                }
            }
        }
        inner.recv_wake.notify_one();
        if inner.lock().end_session {
            return End::Rebooted;
        }
        let deadline = inner.next_deadline(now);
        let sleep = inner.clock.sleep_until(deadline);
        tokio::select! {
            r = rd.readable() => {
                if r.is_err() {
                    return End::Read;
                }
                match rd.try_read(&mut buf) {
                    Ok(0) => return End::Eof,
                    Ok(n) => {
                        let frames = deframer.push(&buf[..n]);
                        buf[..n].zeroize();
                        inner.on_frames(frames, inner.clock.now());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => return End::Read,
                }
            }
            _ = inner.work_wake.notified() => {}
            _ = sleep => {}
            _ = stop.changed() => return End::Stopped,
        }
    }
}

/// Connect, run a session, back off (capped, on the injected clock), until stopped.
async fn run_loop(inner: Arc<Inner>, mut stop: watch::Receiver<bool>) {
    let mut backoff = inner.cfg.reconnect_initial;
    loop {
        if *stop.borrow() {
            break;
        }
        let connect = tokio::time::timeout(
            inner.cfg.connect_timeout,
            TcpStream::connect(inner.cfg.downlink),
        );
        let stream = tokio::select! {
            r = connect => r.ok().and_then(|r| r.ok()),
            _ = stop.changed() => break,
        };
        if let Some(stream) = stream {
            let configured_before = inner.lock().counters.configured;
            let end = session(&inner, stream, &mut stop).await;
            inner.on_session_end(match end {
                End::Stopped => "stopped",
                End::Eof => "meshmud closed the downlink",
                End::Read => "read error",
                End::Write => "write error",
                End::Rebooted => "the board rebooted",
                End::NoRandom => "no randomness",
            });
            if end == End::Stopped {
                break;
            }
            if inner.lock().counters.configured > configured_before {
                backoff = inner.cfg.reconnect_initial;
            }
        }
        let wake = inner.clock.now() + backoff;
        let sleep = inner.clock.sleep_until(wake);
        inner.lock().counters.backoffs += 1;
        tokio::select! {
            _ = sleep => {}
            _ = stop.changed() => break,
        }
        backoff = (backoff * 2).min(inner.cfg.reconnect_max);
    }
    inner.lock().connected = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ConvKey {
        ConvKey(Zeroizing::new([7u8; 32]))
    }

    #[test]
    fn reader_names_follow_the_python_rules() {
        assert_eq!(reader_name("  Hill\u{202e}top\u{7}  ", 40), "Hilltop");
        // zero-width joiners are kept by the reader (the daemon ingest strips them later)
        assert_eq!(reader_name("a\u{200d}b", 40), "a\u{200d}b");
        // Zl / Zp dropped
        assert_eq!(reader_name("a\u{2028}b\u{2029}c", 40), "abc");
        let long = "é".repeat(30);
        assert_eq!(reader_name(&long, NAME_BYTES).len(), 24);
        assert_eq!(
            reader_name(&format!("{}€", "x".repeat(23)), 24),
            "x".repeat(23)
        );
        assert_eq!(reader_name("abc   def", 5), "abc");
    }

    #[test]
    fn python_floats() {
        assert_eq!(py_f32(-7.3), Some(-7.3));
        assert_eq!(py_f32(5.25), Some(5.25));
        assert_eq!(py_f32(f32::NAN), None);
        assert_eq!(py_round(7.254_999, 2), 7.25);
        assert_eq!(py_round(0.125, 2), 0.12, "half to even on the exact value");
        assert_eq!(py_round(51.507_4, 7), 51.507_4);
    }

    #[test]
    fn conv_ids_are_keyed_and_rekeying_changes_them() {
        let k = key();
        let a = k.kh(b"LongFast", &k.psk_digest(&[1]));
        let b = k.kh(b"LongFast", &k.psk_digest(&[2]));
        let c = ConvKey(Zeroizing::new([8u8; 32])).kh(b"LongFast", &k.psk_digest(&[1]));
        assert_eq!(a.len(), 16);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(a, b, "a re-keyed channel is a new conversation");
        assert_ne!(a, c, "the daemon's key keys it");
        assert_eq!(a, k.kh(b"LongFast", &k.psk_digest(&[1])));
    }

    #[test]
    fn dm_ids_round_trip() {
        assert_eq!(dm_native(0x0a0b0c0d), "dm:!0a0b0c0d");
        assert_eq!(parse_dm("dm:!0a0b0c0d"), Some(0x0a0b0c0d));
        assert_eq!(parse_dm("dm:!0a0b0c0"), None);
        assert_eq!(parse_dm("dm:0a0b0c0d"), None);
        assert_eq!(parse_dm("ch:x:y"), None);
    }

    #[test]
    fn every_routing_error_of_section_5_maps() {
        use pb::routing::Error as E;
        use RoutingOutcome::*;
        let cases = [
            (E::None, Ack),
            (E::NoChannel, PreAir(Reason::NoChannel)),
            (E::TooLarge, PreAir(Reason::TooLarge)),
            (E::NotAuthorized, PreAir(Reason::NotAuthorized)),
            (E::PkiFailed, PreAir(Reason::PkiFailed)),
            (E::GotNak, PreAir(Reason::GotNak)),
            (E::NoRoute, PostAir(Reason::NoRoute)),
            (E::Timeout, PostAir(Reason::Timeout)),
            (E::MaxRetransmit, PostAir(Reason::MaxRetransmit)),
            (E::NoResponse, PostAir(Reason::NoResponse)),
            (E::DutyCycleLimit, Later(Reason::DutyCycle)),
            (E::RateLimitExceeded, Later(Reason::RateLimited)),
            (E::PkiSendFailPublicKey, Later(Reason::KeyUnknown)),
            (E::PkiUnknownPubkey, Later(Reason::KeyUnknown)),
            (E::NoInterface, Later(Reason::AdapterDown)),
            (E::BadRequest, PreAir(Reason::GotNak)),
            (E::AdminBadSessionKey, PreAir(Reason::GotNak)),
            (E::AdminPublicKeyUnauthorized, PreAir(Reason::GotNak)),
        ];
        for (e, want) in cases {
            assert_eq!(map_routing_error(e as i32), want, "{e:?}");
        }
        assert_eq!(map_routing_error(12_345), PreAir(Reason::GotNak));
    }

    #[test]
    fn config_ids_are_random_and_never_reserved() {
        for _ in 0..1000 {
            let id = random_config_id().unwrap();
            assert!(allowlist::config_id_ok(id));
        }
    }

    #[test]
    fn presets_and_regions() {
        assert_eq!(preset_display(0, true), "LongFast");
        assert_eq!(preset_display(0, false), "Custom");
        assert_eq!(region_name(3), "EU_868");
        assert_eq!(region_name(0), "UNSET");
        assert_eq!(preset_name(0), "LONG_FAST");
        assert_eq!(region_duty_permille(3), 100);
        assert_eq!(region_duty_permille(1), 1000);
    }

    #[test]
    fn the_manifest_copy_is_read() {
        assert_eq!(manifest_str(&["label"]), "Meshtastic");
        assert_eq!(manifest_str(&["tag"]), "M");
        assert!(manifest_str(&["copy", "banner"]).contains("not 111-sealed"));
        assert_eq!(
            manifest_str(&["copy", "service"]),
            super::super::SERVICE_COPY
        );
    }

    #[test]
    fn heartbeat_and_want_config_bytes() {
        assert_eq!(to_radio_heartbeat(), [0x3a, 0x00]);
        assert_eq!(
            allowlist::classify(&to_radio_heartbeat(), true),
            Ok(allowlist::Allowed::Heartbeat)
        );
        assert_eq!(
            allowlist::classify(&to_radio_want_config(77), false),
            Ok(allowlist::Allowed::WantConfig(77))
        );
        let t = to_radio_text(text_packet(BROADCAST, 1, 9, 3, b"hi"));
        assert_eq!(allowlist::classify(&t, true), Ok(allowlist::Allowed::Text));
    }
}
