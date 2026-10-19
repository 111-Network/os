//! Bridge core (Layer 6 `os/60-transport/README.md` §2, §3, §4, §8, §9): the part of `111d`
//! that joins the node to the networks it does not own. It owns consent, the loop guard,
//! fan-out, delivery state and the foreign tables (mesh111 store, schema V3); **adapters**
//! move native text and report facts, nothing more (§4).
//!
//! T0 scaffold (`60-transport/TASKS.md`): the §4 adapter contract and the type names the
//! build tasks share. Wave 1 B2 fills `core`, `ingest`, `edges`, `fanout`, `state` and the
//! scripted [`StubAdapter`] (the G1 fixture); B3 adds the Meshtastic client, C3 the LXMF
//! adapter, C5 the app peers. The bridge core is always present and inert with no adapter
//! enabled (§2): nothing here runs, polls or listens until an adapter is enabled by a human
//! (`AGENTS.md`: the Raspberry Pi is the floor).
//!
//! Hard rules carried in (§1): foreign text is untrusted everywhere · a 111 message enters a
//! foreign network only by the user's explicit act · no wall clock in any ordering decision
//! (`local_seq` and the monotonic tick only) · an adapter's `send` never blocks the core.

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::{mpsc, Notify};

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
    Accepted {
        native_id: String,
    },
    /// A state with the evidence string the UI prints (§4 table).
    Evidence {
        state: ForeignMessageState,
        evidence: String,
    },
    Failed {
        reason: Reason,
        retry: Retry,
    },
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
}

/// What the stub answers for a send (scripted per test, §4 "StubAdapter").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StubOutcome {
    /// `Accepted {native_id}` then the listed evidence states in order.
    Accept {
        native_id: String,
        evidence: Vec<(ForeignMessageState, String)>,
    },
    Fail {
        reason: Reason,
        retry: Retry,
    },
}

/// In-process adapter with scripted accept, evidence, failures, airtime and inbound feed:
/// the G1 fixture (modelled on `mesh111::link::lossy`). T0 ships the minimal form; B2
/// extends it (airtime, queue-full pauses, link flaps).
pub struct StubAdapter {
    id: &'static str,
    caps: Mutex<Caps>,
    inbound: Mutex<VecDeque<Inbound>>,
    wake: Notify,
    outcomes: Mutex<VecDeque<StubOutcome>>,
    sent: Mutex<Vec<(ConvRef, String)>>,
    next_native_id: Mutex<u64>,
}

impl StubAdapter {
    pub fn new(id: &'static str, caps: Caps) -> Self {
        Self {
            id,
            caps: Mutex::new(caps),
            inbound: Mutex::new(VecDeque::new()),
            wake: Notify::new(),
            outcomes: Mutex::new(VecDeque::new()),
            sent: Mutex::new(Vec::new()),
            next_native_id: Mutex::new(1),
        }
    }

    /// Default caps for a stub network: channel + DM, 232/220 B, every evidence state.
    pub fn default_caps(label: &str, tag: &str) -> Caps {
        Caps {
            kinds: vec![ConvKind::Channel, ConvKind::Dm],
            max_text_bytes: PerKind {
                channel: Some(232),
                dm: Some(220),
                group: None,
            },
            evidence: vec![
                ForeignMessageState::Queued,
                ForeignMessageState::Sent,
                ForeignMessageState::Relayed,
                ForeignMessageState::Delivered,
            ],
            airtime: None,
            auth_levels: vec![Auth::None, Auth::ChannelKey],
            copy: Honest {
                label: label.to_string(),
                chip: tag.to_string(),
                tag: tag.to_string(),
                banner: format!("{label}: a stub network (tests only)"),
                send_warning: String::new(),
            },
            recipient_syntax: None,
            splittable: true,
            state: AdapterState::Live,
        }
    }

    /// Feed one inbound item (text, upsert, nearby, link state); wakes a pending `recv`.
    pub fn feed(&self, item: Inbound) {
        self.inbound.lock().expect("stub inbound").push_back(item);
        self.wake.notify_one();
    }

    /// Script the outcome of the next `send`s, in order. With no script, a send is
    /// accepted and reaches `sent` ("accepted by the radio").
    pub fn script(&self, outcome: StubOutcome) {
        self.outcomes
            .lock()
            .expect("stub outcomes")
            .push_back(outcome);
    }

    pub fn set_state(&self, state: AdapterState) {
        self.caps.lock().expect("stub caps").state = state;
    }

    /// Every `(conv, text)` handed to `send`, in order.
    pub fn sent(&self) -> Vec<(ConvRef, String)> {
        self.sent.lock().expect("stub sent").clone()
    }
}

impl Adapter for StubAdapter {
    fn id(&self) -> &'static str {
        self.id
    }

    fn caps(&self) -> Caps {
        self.caps.lock().expect("stub caps").clone()
    }

    fn recv(&self) -> BoxFuture<'_, Inbound> {
        Box::pin(async move {
            loop {
                if let Some(item) = self.inbound.lock().expect("stub inbound").pop_front() {
                    return item;
                }
                self.wake.notified().await;
            }
        })
    }

    fn send(&self, conv: &ConvRef, text: &str, _o: SendOpts) -> BoxFuture<'_, SendHandle> {
        self.sent
            .lock()
            .expect("stub sent")
            .push((conv.clone(), text.to_string()));
        let outcome = self
            .outcomes
            .lock()
            .expect("stub outcomes")
            .pop_front()
            .unwrap_or_else(|| {
                let mut n = self.next_native_id.lock().expect("stub ids");
                let native_id = format!("stub-{}", *n);
                *n += 1;
                StubOutcome::Accept {
                    native_id,
                    evidence: vec![(
                        ForeignMessageState::Sent,
                        "accepted by the radio".to_string(),
                    )],
                }
            });
        Box::pin(async move {
            let (tx, rx) = mpsc::channel(8);
            match outcome {
                StubOutcome::Accept {
                    native_id,
                    evidence,
                } => {
                    let _ = tx.send(SendEvent::Accepted { native_id }).await;
                    for (state, evidence) in evidence {
                        let _ = tx.send(SendEvent::Evidence { state, evidence }).await;
                    }
                }
                StubOutcome::Fail { reason, retry } => {
                    let _ = tx.send(SendEvent::Failed { reason, retry }).await;
                }
            }
            SendHandle { events: rx }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv() -> ConvRef {
        ConvRef {
            network: "stub-a".into(),
            account: "board-1".into(),
            kind: ConvKind::Channel,
            native_id: "ch:LongFast:abcd".into(),
        }
    }

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

    #[tokio::test]
    async fn stub_send_returns_at_once_and_streams_events() {
        let stub = StubAdapter::new("stub-a", StubAdapter::default_caps("Stub A", "A"));
        let mut h = stub.send(&conv(), "hello", SendOpts::default()).await;
        assert_eq!(
            h.events.recv().await,
            Some(SendEvent::Accepted {
                native_id: "stub-1".into()
            })
        );
        assert!(matches!(
            h.events.recv().await,
            Some(SendEvent::Evidence {
                state: ForeignMessageState::Sent,
                ..
            })
        ));
        stub.script(StubOutcome::Fail {
            reason: Reason::RegionUnset,
            retry: Retry::Later,
        });
        let mut h = stub.send(&conv(), "again", SendOpts::default()).await;
        assert_eq!(
            h.events.recv().await,
            Some(SendEvent::Failed {
                reason: Reason::RegionUnset,
                retry: Retry::Later
            })
        );
        assert_eq!(stub.sent().len(), 2);
    }

    #[tokio::test]
    async fn stub_recv_waits_for_feed() {
        let stub = std::sync::Arc::new(StubAdapter::new(
            "stub-b",
            StubAdapter::default_caps("Stub B", "B"),
        ));
        let s2 = stub.clone();
        let waiter = tokio::spawn(async move { s2.recv().await });
        tokio::task::yield_now().await;
        stub.feed(Inbound::Link(LinkState::Up));
        assert_eq!(waiter.await.unwrap(), Inbound::Link(LinkState::Up));
    }
}
