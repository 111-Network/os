//! Bridge core (Layer 6 `os/6-transport/README.md` §2, §3, §4, §8, §9): the part of `111d`
//! that joins the node to the networks it does not own. It owns consent, the loop guard,
//! fan-out, delivery state and the foreign tables (mesh111 store, schema V3); **adapters**
//! move native text and report facts, nothing more (§4).
//!
//! This file is the §4 adapter contract and the type names the build tasks share (T0); the
//! core lives beside it (B2):
//!
//! - [`core`] — [`Bridge`]: the adapter registry (one recv task per enabled adapter), the
//!   adapter manifests (`assets/adapters/<id>.toml`), the monotonic tick, the networks view
//!   and the typed API surface the routes call, with [`BridgeError`] mapped onto problems.
//! - [`ingest`] — the §3 one ingest function (Cc + Cf strip, NFC, caps), the dedupe flag and
//!   §8 guard 2 (own-echo suppression).
//! - [`marker`] — the §8 relay marker `[<n>/<TAG>] `: build and parse.
//! - [`edges`] — §8 relay edges: guards 1–6, live consent with pause, `honour_ok_to_mqtt`,
//!   oversize refuse / split, `relayed_seq`.
//! - [`fanout`] — §9 compose fan-out: per-target plans (bytes, cap, parts, airtime estimate,
//!   refusals), atomic validation, send groups.
//! - [`state`] — the §4 delivery state machine: crash-safe handoff, monotone states, `Later`
//!   retries on the tick with capped back-off (the pending mark withdrawn while nothing was
//!   sent), a post-air NAK's `Later` re-sent when its peer is heard again, user rows first.
//! - [`settings`] — the bridge-core sections of `state/bridge.json` (HAM confirmations,
//!   per-channel send switches, recipient lists, announce), the text + hash records.
//! - [`stub`] — the scripted [`StubAdapter`] (the G1 fixture).
//!
//! B3 adds the Meshtastic client, C3 the LXMF adapter, C5 the app peers. The bridge core is
//! always present and inert with no adapter enabled (§2): nothing here runs, polls or listens
//! until an adapter is enabled by a human (`AGENTS.md`: the Raspberry Pi is the floor) — with
//! none enabled there is no task and [`Bridge::tick_once`] returns before touching anything.
//!
//! Hard rules carried in (§1): foreign text is untrusted everywhere · a 111 message enters a
//! foreign network only by the user's explicit act · no wall clock in any ordering decision
//! (`local_seq` and the monotonic tick only) · an adapter's `send` never blocks the core.

pub mod attest;
pub mod core;
pub mod deltachat;
pub mod edges;
pub mod fanout;
pub mod ingest;
pub mod lxmf;
pub mod marker;
pub mod settings;
pub mod simplex;
pub mod state;
pub mod stub;

pub use self::core::{AdapterManifest, Bridge, BridgeConfig, BridgeError, NearbySink};
pub use self::stub::{StubAdapter, StubOutcome};

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Network ids (§3, defined once): `111` is the fabric; the rest are foreign.
pub const NETWORK_111: &str = "111";
pub const NETWORK_MESHTASTIC: &str = "meshtastic";
pub const NETWORK_LXMF: &str = "lxmf";
pub const NETWORK_DELTACHAT: &str = "app:deltachat";
pub const NETWORK_SIMPLEX: &str = "app:simplex";

/// §3 conversation kinds. A DM can never be a relay-edge endpoint (§8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConvKind {
    Channel,
    Dm,
    Group,
}

/// §4 `ForeignMessageState`: closed, monotone (`delivered` never regresses; `failed` only
/// for refusals before air). 111's own `MessageState` stays at four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForeignMessageState {
    Queued,
    Sent,
    Relayed,
    Delivered,
    Failed,
}

impl ForeignMessageState {
    /// Monotone rank: a state only moves to a higher rank, except that a later
    /// `delivered` overrides `failed` (§4).
    pub fn rank(self) -> u8 {
        match self {
            ForeignMessageState::Queued => 0,
            ForeignMessageState::Sent => 1,
            ForeignMessageState::Relayed => 2,
            ForeignMessageState::Failed => 3,
            ForeignMessageState::Delivered => 4,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ForeignMessageState::Queued => "queued",
            ForeignMessageState::Sent => "sent",
            ForeignMessageState::Relayed => "relayed",
            ForeignMessageState::Delivered => "delivered",
            ForeignMessageState::Failed => "failed",
        }
    }
}

/// §3 authentication label (closed; copy from the adapter manifest).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Auth {
    None,
    ChannelKey,
    Pki,
    Signed,
    /// Reserved for the second release (§14, §17); never produced in v1.
    Attested111,
}

/// Which conversation an adapter is asked to send into or reports text from (§3): the
/// native coordinates, never an `fc-` id (the core maps between them).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConvRef {
    pub network: String,
    /// Which board or identity of ours (§3 `account`).
    pub account: String,
    pub kind: ConvKind,
    pub native_id: String,
}

/// §9 per-target size mode: oversize is refused by default; split is an explicit choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SizeMode {
    #[default]
    Refuse,
    Split,
}

/// Options for one `send` (§4). The adapter does not decide policy; the core has already
/// validated size and consent.
#[derive(Debug, Clone, Default)]
pub struct SendOpts {
    pub size_mode: SizeMode,
    /// §5: `want_ack = true` for channel text and DMs on Meshtastic.
    pub want_ack: bool,
}

/// §4 `Retry`: `Later` rows retry on the monotonic tick with capped back-off and earlier on
/// an adapter event; `Never` leaves the retry to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Retry {
    Later,
    Never,
}

/// §4 and §5 reason codes (closed; extends Layer 5's). The string form is the API's
/// `reason_code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    RegionUnset,
    NoChannel,
    TooLarge,
    NotAuthorized,
    PkiFailed,
    GotNak,
    AdapterDown,
    NoRoute,
    Timeout,
    MaxRetransmit,
    NoResponse,
    DutyCycle,
    RateLimited,
    KeyUnknown,
    PeerUnannounced,
    UplinkOff,
    HamUnconfirmed,
    ConsentPaused,
    Cancelled,
    Oversize,
}

/// Events an adapter reports for one send (§4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendEvent {
    /// The carrier took it; `native_id` is the Meshtastic packet id or the LXMF hash
    /// (recorded for own-echo suppression, §8 guard 2). Clears `meta.handoff = pending`.
    Accepted { native_id: String },
    /// A state with the evidence string the UI prints (§4 table).
    Evidence {
        state: ForeignMessageState,
        evidence: String,
    },
    /// The carrier did not (or not provably) deliver it. What the core does (§4, §5;
    /// [`state`]):
    ///
    /// - **Before air** (no `Accepted` on the send, and `reason` not a post-air NAK):
    ///   `Retry::Never` makes the row `failed`; `Retry::Later` keeps it `queued` with the
    ///   reason, withdraws its pending handoff mark and hands it again on the tick with
    ///   capped back-off, earlier on an adapter event (link up, a Nearby item, a conversation
    ///   upsert). Nothing was transmitted, so the retry is still the one send of the act.
    /// - **After air** (after `Accepted`, or a post-air NAK reason `no_route`, `timeout`,
    ///   `max_retransmit`, `no_response`): the row keeps `sent`/`relayed` with "transmitted,
    ///   not confirmed". With a post-air NAK reason and `Retry::Later` (a Meshtastic DM's
    ///   NAK) the core re-hands the **same text** once when the adapter next yields
    ///   [`Inbound::Conversation`] for exactly that send's [`ConvRef`] (the peer was heard
    ///   again: the Meshtastic client upserts `dm:!<node>` on that node's NodeInfo); the
    ///   adapter does not re-send by itself and mints a new native id, which the re-send's
    ///   `Accepted` records on the row. No tick back-off applies. Any other failure after
    ///   `Accepted`, or `Retry::Never`, is kept and never re-sent automatically.
    /// - A row whose pending mark survived a restart is never re-sent automatically
    ///   ("may have been sent; resend?"), whatever the adapter reports later.
    Failed { reason: Reason, retry: Retry },
}

/// Returned by `send` at once; events follow on the channel (§4: `send` never blocks).
pub struct SendHandle {
    pub events: mpsc::Receiver<SendEvent>,
}

/// `max_text_bytes` per kind (§4; `None` = kind not supported by this adapter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PerKind {
    pub channel: Option<usize>,
    pub dm: Option<usize>,
    pub group: Option<usize>,
}

impl PerKind {
    pub fn get(&self, kind: ConvKind) -> Option<usize> {
        match kind {
            ConvKind::Channel => self.channel,
            ConvKind::Dm => self.dm,
            ConvKind::Group => self.group,
        }
    }
}

/// §4 airtime profile in Layer 5's `Airtime` shape (`mesh111::link::Airtime`: duty cycle in
/// permille, raw bit rate) plus the preview's estimate rule (§9). Facts shown, never a limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AirtimeProfile {
    /// Regulatory duty cycle in permille (10 = 1 %).
    pub duty_cycle_permille: u16,
    /// Raw bearer bit rate.
    pub bits_per_s: u32,
    /// The board's modem preset (Meshtastic) or `None` ("depends on the path", LXMF).
    pub preset: Option<String>,
}

impl From<mesh111::link::Airtime> for AirtimeProfile {
    fn from(a: mesh111::link::Airtime) -> Self {
        AirtimeProfile {
            duty_cycle_permille: a.duty_cycle_permille,
            bits_per_s: a.bits_per_s,
            preset: None,
        }
    }
}

/// §4 honest copy from `assets/adapters/<id>.toml`, so the UI holds no per-network code.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Honest {
    pub label: String,
    pub chip: String,
    /// Relay marker TAG: one to three ASCII letters (§8).
    pub tag: String,
    pub banner: String,
    pub send_warning: String,
}

/// §4 recipient syntax for networks with chosen-peer targets (LXMF: 32 hex).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Syntax {
    pub pattern: String,
    pub hint: String,
}

/// Live adapter state as `GET /networks` reports it (§10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "detail")]
pub enum AdapterState {
    Live,
    NotSet,
    Error(String),
    Disabled,
}

/// §4 `caps()`: static facts plus live state; read by the preview, the UI and the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Caps {
    pub kinds: Vec<ConvKind>,
    pub max_text_bytes: PerKind,
    pub evidence: Vec<ForeignMessageState>,
    pub airtime: Option<AirtimeProfile>,
    pub auth_levels: Vec<Auth>,
    pub copy: Honest,
    pub recipient_syntax: Option<Syntax>,
    pub splittable: bool,
    pub state: AdapterState,
}

/// Adapter link state (§4 `recv`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkState {
    Up,
    Down,
    Partitioned,
}

/// One native text as the adapter heard it (§3 columns; the core runs the ingest function).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundText {
    pub conv: ConvRef,
    pub sender_native: String,
    pub sender_name: String,
    pub auth: Auth,
    pub native_msg_id: String,
    pub body: String,
    /// The foreign hint (display only, "sender's claim").
    pub claim_ts: Option<i64>,
    /// snr, rssi, hops, `via_mqtt`, `ok_to_mqtt`, … (§3 `meta`).
    pub meta: serde_json::Value,
}

/// What `recv` yields (§4): texts, conversation upserts, Nearby updates, link state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Inbound {
    Text(InboundText),
    Conversation {
        conv: ConvRef,
        title: String,
    },
    /// A `NearbySource` JSON object (Layer 5 §14 schema; `kind: meshtastic | reticulum`).
    Nearby(serde_json::Value),
    Link(LinkState),
    /// Inbound queue overflow: `n` oldest texts dropped (shown, never silent).
    Dropped {
        n: u64,
    },
}

/// §4 adapter contract (object-safe; futures are boxed). An adapter moves native text and
/// reports facts. It never decides consent, relaying, loop guard, size policy or
/// persistence, never touches the store, and its `send` returns at once.
pub trait Adapter: Send + Sync {
    /// `"meshtastic" | "lxmf" | "app:deltachat" | "app:simplex"`.
    fn id(&self) -> &'static str;
    fn caps(&self) -> Caps;
    /// The next native text, conversation upsert, Nearby update or link state.
    fn recv(&self) -> BoxFuture<'_, Inbound>;
    /// Returns a handle at once; events follow.
    fn send(&self, conv: &ConvRef, text: &str, o: SendOpts) -> BoxFuture<'_, SendHandle>;
    /// Live facts the core reads per send and per relay copy (§5 "HAM and MQTT: live
    /// consent", §8 "Consent facts are read live, per copy"): this node's accounts, licensed
    /// mode, channel uplink and key facts, the region. A provided method (additive to the T0
    /// contract): an adapter that reports none reads as "no accounts, nothing licensed,
    /// nothing uplinked" — the Meshtastic client (B3) and the LXMF adapter (C3) override it.
    fn facts(&self) -> AdapterFacts {
        AdapterFacts::default()
    }
}

/// One account of this node on a network (§3 `account`): a board's node id, our LXMF
/// destination, an app-peer bridge account. Inbound from any of them is our own echo (§8
/// guard 2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountFacts {
    /// The native id (`!0badc0de`, a destination hash); the `account` of its conversations.
    pub id: String,
    /// What the UI shows (a board's long name and id); untrusted.
    pub label: Option<String>,
    /// The board reports an amateur-licensed user (`User.is_licensed`, read live, §5).
    pub licensed: bool,
}

/// Live facts of one channel (§5): the board publishes it to MQTT (`uplink_enabled`), and
/// whether it uses the default (public) key (§8 add-on).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelFacts {
    pub account: String,
    /// The conversation's native id (`ch:<name>:<kh>`).
    pub native_id: String,
    pub uplink: bool,
    pub default_key: bool,
}

/// What [`Adapter::facts`] reports (read live; never cached across a decision).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterFacts {
    pub accounts: Vec<AccountFacts>,
    pub channels: Vec<ChannelFacts>,
    /// The radio region is set (`None`: not applicable). Unset ⇒ nothing is handed to the
    /// adapter (a board transmits nothing before it, §5); rows queue with `region_unset`.
    pub region_set: Option<bool>,
    /// The carrier's message ids are globally unique (LXMF hash, app-peer ids): a repeat of
    /// the §3 key alone is a duplicate. `false` (Meshtastic's 32-bit packet ids): key and
    /// body within the last 32 rows.
    pub ids_globally_unique: bool,
    /// Parts of a split send can arrive out of order (Meshtastic; the preview says so, §9).
    pub parts_may_reorder: bool,
    /// `max_text_bytes` on a licensed account where it differs from [`Caps::max_text_bytes`]
    /// (Meshtastic DMs: 232 B instead of 220 B, the firmware skips PKI, §4).
    pub licensed_caps: Option<PerKind>,
}

impl AdapterFacts {
    /// The account `id`, if it is one of this node's on this network.
    pub fn account(&self, id: &str) -> Option<&AccountFacts> {
        self.accounts.iter().find(|a| a.id == id)
    }

    /// The live facts of a channel conversation.
    pub fn channel(&self, account: &str, native_id: &str) -> Option<&ChannelFacts> {
        self.channels
            .iter()
            .find(|c| c.account == account && c.native_id == native_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_are_monotone_with_delivered_on_top() {
        use ForeignMessageState::*;
        assert!(Queued.rank() < Sent.rank());
        assert!(Sent.rank() < Relayed.rank());
        assert!(
            Failed.rank() < Delivered.rank(),
            "a later delivered overrides failed (§4)"
        );
        assert_eq!(serde_json::to_string(&Delivered).unwrap(), "\"delivered\"");
        assert_eq!(
            serde_json::to_string(&Auth::ChannelKey).unwrap(),
            "\"channel-key\""
        );
        assert_eq!(
            serde_json::to_string(&Reason::PeerUnannounced).unwrap(),
            "\"peer_unannounced\""
        );
    }

    #[test]
    fn facts_default_to_nothing_and_look_up_by_native_coordinates() {
        let mut f = AdapterFacts::default();
        assert!(f.account("!0badc0de").is_none());
        f.accounts.push(AccountFacts {
            id: "!0badc0de".into(),
            label: None,
            licensed: true,
        });
        f.channels.push(ChannelFacts {
            account: "!0badc0de".into(),
            native_id: "ch:LongFast:9f3c".into(),
            uplink: true,
            default_key: true,
        });
        assert!(f.account("!0badc0de").unwrap().licensed);
        assert!(f.channel("!0badc0de", "ch:LongFast:9f3c").unwrap().uplink);
        assert!(f.channel("!other", "ch:LongFast:9f3c").is_none());
    }
}
