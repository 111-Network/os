//! The runtime (spec §4–§9, §18.1): links → [`open_envelope`] → L2 window
//! verdict (`observe_counter`) → `msg_seq` dedup / the kind's idempotency
//! rule → store → events. Owner: F1 (library side); the daemon wiring is
//! node111 `daemon/fabric/` (F2).
//!
//! ### Shape
//!
//! One task (an actor) owns the store, the connection table and every piece
//! of sender/receiver bookkeeping; nothing else touches them. Inputs reach it
//! over two channels:
//!
//! - **frames** from each link's `recv` loop, on a **bounded** queue
//!   ([`FRAME_QUEUE`]) filled with `send().await`: when the actor falls
//!   behind (every sealed frame costs it an X25519 + AEAD open), a link's
//!   `recv` loop stops, the adapter's own bounded inbound queue fills
//!   (`TcpLanConfig::inbound_queue`), its readers stop reading, and TCP flow
//!   control pushes back on the sender — end to end. An unauthenticated LAN
//!   peer (hello + any self-signed announce) streaming frames at line rate
//!   gets a stalled socket, never the daemon's memory; nothing is dropped
//!   for want of room. ([`Link::send`] must therefore never wait on its own
//!   `recv` being drained: the actor awaits `send` while frames wait for it.)
//! - **control**: [`LinkEvent`]s from the adapters ([`Fabric::link_events`];
//!   connection facts, at most two per completed handshake) and commands
//!   from the typed API ([`Fabric`], what the daemon's routes call). Within
//!   a tick control inputs are taken first, so the API stays responsive
//!   while a peer floods.
//!
//! The actor works in **ticks**: it takes every input that is ready (at most
//! `TICK_BATCH`), then flushes owed ACKs and mailbox commits, pumps the
//! outbox of every touched peer, writes the queued frames to their links,
//! and emits the tick's events — **coalesced per message per tick** (§15: a
//! 16-fragment transfer never floods the SSE ring).
//!
//! **No timers** (§1, §9): nothing here sleeps or schedules. Retries happen on
//! link events (a connection's announce, a disconnect), contact events (a key
//! arrives, a request is accepted), ACK evidence (§5 loss inference), and the
//! explicit [`Fabric::retry`] / [`Fabric::retry_all`] (what the daemon calls on
//! `fabric` events). Keepalive is the adapter's transport detail.
//!
//! ### Verify before state (§18 rule 1)
//!
//! A sealed frame is opened by [`open_envelope`] (`pair_mac` before anything
//! else; the INTRO card first, then `pair_mac`), then **every** verified
//! packet is offered to the L2 window (`Sealer::observe_counter`, A11). The
//! verdict is bookkeeping for `msg_seq` kinds (their content is processed
//! either way — the store's receive state is the replay authority), but it
//! gates the three things §8.1 lists:
//!
//! - **Attribution** — a connection is attributed to node X only after a
//!   `pair_mac`-verified packet from X that the window judged `Fresh` arrived
//!   on it. A replayed packet verifies but is never fresh, so it never
//!   attributes a connection.
//! - **Proactive pushes** — the hub streams `MAILBOX_ITEM`s only on a
//!   connection attributed to their `dest` that fetched on it.
//! - **Card answers** — a `node1…` card request is answered only to an
//!   enrolled member on a connection attributed to it.
//!
//! Beyond §8.1's list this module answers control requests (`CUSTODY_PUT`,
//! `MAILBOX_FETCH`, `ENROLL`) only on a connection attributed to their
//! sender, and sends its own traffic only on a connection attributed to the
//! peer **or** one it dialed itself to an address configured for that peer
//! (an outbound connection whose announce names the peer). An inbound,
//! unattributed connection — hello and announce authenticate nothing — gets
//! nothing but a silent socket.
//!
//! ### Delivery (§9)
//!
//! Own messages are numbered only by `Store::queue_own_message` (FULL
//! commit, before any fragment is sealed, §4) and fragmented when queued
//! (§5: the count is fixed then, from the destination path's profile —
//! module [`crate::frag`]). The route to a peer is, in order: a direct
//! connection (above), else the home hub's mailbox (`CUSTODY_PUT` of the
//! sealed item, wrapped and sealed to the hub), else none (`queued`). States
//! move only forward: `queued` → `sent` (a frame was written to a link) →
//! `relayed` (the home hub's `CUSTODY_ACK` ACCEPTED/DUPLICATE for every
//! fragment) → `delivered` (the recipient's own ACK, the only event that
//! releases the sender's copy). Custody rejects keep `sent` with a reason.
//!
//! ### Readings recorded for the lead
//!
//! - Fragment planning uses the current route's MTU, else the smallest MTU
//!   among the registered links, else §5's 160 B default; stream links keep
//!   [`PLAN_RESERVE`] bytes for the INTRO card, a piggybacked ACK and the
//!   custody wrap.
//! - The window fills **fresh fragments first, then retransmissions**: under
//!   "no timers" the receiver's SACKs of fresh fragments are the only loss
//!   evidence, so keeping fresh data moving keeps that evidence flowing.
//! - `node1…` contacts waiting for their card are **stored** (§7.2 (2): the
//!   store's `pending_node_ids`, schema v2 — the contacts table needs key
//!   material) and read back at start, together with every outbox peer that
//!   still lacks a key. A row leaves when the card is pinned or the contact
//!   is forgotten.
//! - Read marks (`ContactPatch::read_seq`) live in memory only and reset to
//!   "nothing read" at every start (v1, accepted by the lead's Wave 1 triage;
//!   a `read_seq` column is a C3 wish if they must survive restarts).
//! - A home-hub hint in a contact's announce (`appdata`, 8 B) is a locator
//!   only (§7.3): with the home hub reachable the item is handed to it and
//!   the hub's `CUSTODY_ACK` decides (`REJECT_UNKNOWN_DEST` ⇒ `sent` /
//!   `hub_not_yours`); the hint only explains a `queued` message while the
//!   hub is unreachable. A stale hint never keeps a message from a hub that
//!   holds its recipient.
//! - `relayed` evidence is the home hub's authenticated `CUSTODY_ACK`
//!   `ACCEPTED` **or `DUPLICATE`** for every fragment: §6.2 names ACCEPTED,
//!   but a DUPLICATE from the home hub means the same `pkt_ref` is already
//!   held there for that member (a retried `CUSTODY_PUT` whose first ACK was
//!   lost), which is the same fact §9 states ("a mailbox node saved it and
//!   will hold it for them").
//! - A successful **direct** write of a message already `sent` with a
//!   custody reject's reason clears that reason: the hub's verdict no longer
//!   explains where the message is.
//! - Hub push: a `MAILBOX_ITEM` counts as streamed only once its frame is
//!   queued for the link, and a send error rolls the connection's push
//!   window back to just below it. Items stream in `custody_seq` order with
//!   no gaps (the member's cursor is a high-water mark, so a gap would be
//!   committed unseen): an item too large for the member's path stops that
//!   connection's stream until the member fetches over a path that carries
//!   it (unreachable with v1 `tcp-lan` sizes; L6 small-MTU hub links).
//! - `ENROLL_OK`'s `custody_seq_floor` is just below the lowest custody
//!   sequence still held uncommitted for the member (`custody_high` when
//!   none): a member that re-enrolls — a new device after a restore, or a
//!   fresh offer — fetches what waits for it. The member keeps its cursor's
//!   `max()` semantics.
//! - The member re-sends `MAILBOX_COMMIT {cursor}` beside every
//!   `MAILBOX_FETCH` on hub link-up (the hub keeps `max(cursor)`, so it is
//!   idempotent), and a commit owed while no hub connection exists stays
//!   owed: a commit lost to a dropped socket or a restart is repaired at the
//!   next link-up.
//! - A crash between numbering an own message and writing its outbox rows
//!   (an orphan, §4) re-plans it for the destination path's profile at start
//!   (no link is registered yet then, so §5's 160 B default) — or, when the
//!   text exceeds Tier 1 at that size, for a stream link (the only path that
//!   could have accepted it).
//! - Thread listings read the newest page and SQL aggregates
//!   (`Store::messages_newest`, `Store::thread_counts`): no call loads a
//!   whole history.
//! - A message request (INTRO from an unknown sender) is filed in the store's
//!   `requests` table; its messages are stored but never ACKed until the user
//!   accepts; accepting ACKs exactly the receive state, i.e. what is shown.
//! - A member commits its mailbox cursor past every item it processed,
//!   including one its receive state deferred (range list full): the hub
//!   keeps the item (evict-first, not deleted) and the sender retransmits it
//!   directly when the two meet (DTN residual, recorded).
//! - An ACK sent through the hub is fire-and-forget: if it is lost, the
//!   sender's next direct retransmission is re-ACKed.
//! - Custody rejects keep `sent` with a reason: `REJECT_FULL` ⇒
//!   `mailbox_full`; `REJECT_UNKNOWN_DEST` ⇒ `hub_not_yours` (the closest
//!   contract code for §9's "your hub does not know them");
//!   `REJECT_UNENROLLED` ⇒ `not_enrolled`. `not_enrolled` and `cancelled`
//!   are not yet in the contract's emitted `reason_code` set (clients render
//!   `unknown`); the contract gains both with F2 (Wave 2, lead's triage).
//! - Forgetting a contact leaves its undelivered messages queued
//!   (`waiting_for_key`; numbers are never abandoned); after a restart such
//!   a peer is listed again as a key-less contact, because the outbox still
//!   names it. Forgetting the **home hub** also ends the membership here:
//!   `settings.home_hub` is cleared (a `Settings` event) and its maintained
//!   dial released.
//! - [`Fabric::enroll`] resolves on `ENROLL_OK`; it has no timeout of its
//!   own (no timers here) — the route that calls it bounds the wait.
//! - All handles dropped does not stop the actor: call
//!   [`Fabric::shutdown`] (clean close) or [`Fabric::abort`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};

use id111::announce::Announce;
use id111::keyset::NodeId;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::ack::{strong_hash, AckBlock, AckPayload, StrongReceipt};
use crate::contacts::{
    card_text, classify_announce, contact_row, parse_card_text, parse_node_id, verify_card,
    AnnounceVerdict, CardError, HubBundle, KeyCache,
};
use crate::custody::{pkt_ref, CustodyAck, CustodyReceipt, CustodyStatus};
use crate::enroll::{Code, EnrollError, Offer as EnrollOffer, OfferState};
use crate::envelope::{
    open_envelope, payload_budget, seal_envelope, FragPos, Inner, Kind, Opened, PeerKeys, Priority,
    MAX_PACKET_LEN, OUTER_OVERHEAD,
};
use crate::frag::{frag_slice, plan, FragError, TxTracker, CUSTODY_WRAP, MAX_OUTSTANDING};
use crate::ids::{MsgSeq, STREAM_1TO1};
use crate::link::{
    AnnounceGate, Dialer, EventSink, Frame, FrameType, Link, LinkError, LinkEvent, LinkId,
    LinkKind, LinkState, PathProfile, PeerAddr, DEFAULT_MTU,
};
use crate::mailbox::{
    decode_cursor, decode_item, decode_put, encode_cursor, encode_enroll, encode_item, encode_put,
    mint_code, normalize_code, EnrollOk, EnrollStatus, PushWindow,
};
use crate::outbox::{build_ack_block, highest_seen, MessageBody, Reason};
use crate::sealer::{Freshness, Sealer, SealerError};
use crate::store::{
    Class, ContactRow, CustodyPut, Dir, EpochDecision, EpochState, FragOffer, MessageRow, MsgState,
    NewMessage, RequestPut, Seq, Store, StoreError, Trust,
};

/// Bytes a stream link's fragment plan keeps free for the INTRO card
/// (≤ 170 B), a piggybacked ACK block and the 120 B custody wrap.
pub const PLAN_RESERVE: usize = 512;
/// Links at or above this MTU are "stream" links for planning.
const STREAM_MTU: usize = 4096;
/// Inputs handled per tick before the flush (bounded work per tick).
const TICK_BATCH: usize = 256;
/// Frames buffered between the links' `recv` loops and the actor (module
/// doc: a full queue stops the loops, and back-pressure reaches the sender).
pub const FRAME_QUEUE: usize = 64;
/// Flush/drain rounds per tick (a failed send may re-route once or twice).
const DRAIN_ROUNDS: usize = 8;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Fabric failures surfaced to the API (typed; nothing panics on input).
#[derive(Debug, thiserror::Error)]
pub enum FabricError {
    /// The fabric task is not running.
    #[error("fabric stopped")]
    Stopped,
    /// No such message or contact.
    #[error("not found")]
    NotFound,
    /// A card's keys conflict with the pinned keys for that NodeId (§7.1).
    #[error("key material conflicts with the pinned contact")]
    KeyConflict,
    /// The request does not apply (one plain phrase).
    #[error("invalid: {0}")]
    Invalid(&'static str),
    /// `retry` on a message that is not `queued` or `sent`, or an incoming one.
    #[error("not retryable in its current state")]
    NotRetryable,
    /// The text needs more than Tier 0/1 can carry (§5; Tier 2 is v1.1).
    #[error("{0}")]
    TooLarge(FragError),
    /// The identity holder reports `RestoreFloorRequired` (§4): nothing is
    /// numbered or sealed until it clears.
    #[error("restore pending: nothing is sealed until the identity restore completes")]
    RestorePending,
    /// A card or NodeId that does not parse or verify.
    #[error("card: {0}")]
    Card(#[from] CardError),
    /// The hub refused the enrollment (`ENROLL_OK` status).
    #[error("enrollment refused ({0:?})")]
    Enroll(EnrollStatus),
    /// The enrollment code does not parse.
    #[error("enrollment code: {0}")]
    Code(#[from] EnrollError),
    /// No registered link can dial that address.
    #[error("no link can reach that address")]
    NoLink,
    /// Store failure.
    #[error("store: {0}")]
    Store(#[from] StoreError),
    /// Identity-handle failure.
    #[error("sealer: {0}")]
    Sealer(#[from] SealerError),
}

/// Result alias.
pub type Result<T, E = FabricError> = std::result::Result<T, E>;

/// The node's home hub (member side, §9; one per node in v1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HomeHub {
    /// The hub's NodeId.
    pub node_id: NodeId,
    /// Its `tcp-lan` address (dialed and kept up when a dialer is registered).
    pub address: Option<SocketAddr>,
}

/// `fabric/settings` (§15 `PATCH /fabric/settings`). The daemon persists
/// them; the fabric emits [`FabricEvent::Settings`] whenever they change
/// (enrollment sets `home_hub`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FabricSettings {
    /// Accept custody for enrolled members (hub role, §9).
    pub hub_role: bool,
    /// Bind the `tcp-lan` listener on the LAN (the daemon acts on it).
    pub listen_lan: bool,
    /// An always-on laptop or a phone opts in to hold custody.
    pub custody_opt_in: bool,
    /// The home hub, once enrolled.
    pub home_hub: Option<HomeHub>,
}

impl FabricSettings {
    /// This node holds custody (and runs enrollment) for members.
    pub fn holds_custody(&self) -> bool {
        self.hub_role || self.custody_opt_in
    }
}

/// A partial merge of [`FabricSettings`] plus the store's capacity shaping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingsPatch {
    /// Hub role.
    pub hub_role: Option<bool>,
    /// LAN listener.
    pub listen_lan: Option<bool>,
    /// Custody opt-in.
    pub custody_opt_in: Option<bool>,
    /// Per-class quotas (refusable classes only; `None` drops a quota).
    pub quotas: Vec<(Class, Option<u64>)>,
    /// The pending-request quota (`Some(None)` restores the default).
    pub request_quota: Option<Option<u64>>,
}

/// What changed about a message (§15 SSE `message.change`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageChange {
    /// A message new to the client.
    New,
    /// A state or reason change.
    State,
    /// A message held under a message request (§7.2 (3)).
    Request,
}

/// What changed about a contact (§15 SSE `contact.change`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactChange {
    /// Added (a card, a `node1…`, an enrollment, an accepted request).
    Added,
    /// Updated (alias, pin, key arrival, refreshed announce, accept).
    Updated,
    /// Forgotten.
    Removed,
    /// A new message request.
    Request,
}

/// Events the daemon subscribes to (SSE `message`, `contact`, `fabric`),
/// coalesced per message / contact per tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FabricEvent {
    /// A new message, a state change, or a request (first change this tick).
    Message {
        /// Store row id.
        id: i64,
        /// The other party.
        peer: NodeId,
        /// First change since the previous frame.
        change: MessageChange,
    },
    /// A contact change.
    Contact {
        /// The contact.
        node_id: NodeId,
        /// What changed.
        change: ContactChange,
    },
    /// Links, hub, custody or store changed (re-read [`Fabric::status`]).
    Fabric,
    /// The settings changed; persist them.
    Settings(FabricSettings),
}

/// One message as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageView {
    /// Store row id (the daemon mints the `msg-…` string from it).
    pub id: i64,
    /// The other party (recipient for own, sender for in).
    pub peer: NodeId,
    /// `Own` or `In`.
    pub dir: Dir,
    /// §3.2 kind byte.
    pub kind: u8,
    /// The body text (`MSG` only; untrusted for `In`).
    pub text: Option<String>,
    /// An empty body: a cancelled message's tombstone (§4).
    pub tombstone: bool,
    /// §9 state (`In` is always delivered).
    pub state: MsgState,
    /// Why the state needs explaining.
    pub reason: Option<Reason>,
    /// Thread order.
    pub local_seq: u64,
    /// Local receipt sequence (`In` only).
    pub received_seq: Option<u64>,
    /// The sender's unverified `sent_at` claim (`In` only).
    pub sent_at_claim: Option<u64>,
    /// `msg_id` stream.
    pub stream: u64,
    /// `msg_id` (epoch, n).
    pub seq: MsgSeq,
}

/// Counts of a thread's own messages per state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StateCounts {
    /// `queued`.
    pub queued: u64,
    /// `sent`.
    pub sent: u64,
    /// `relayed`.
    pub relayed: u64,
    /// `delivered`.
    pub delivered: u64,
}

/// One thread in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadView {
    /// The other party.
    pub peer: NodeId,
    /// Local alias.
    pub alias: Option<String>,
    /// An unaccepted message request.
    pub request: bool,
    /// Incoming messages above the read mark.
    pub unread: u64,
    /// The most recent message.
    pub last: Option<MessageView>,
    /// Own messages per state.
    pub states: StateCounts,
}

/// One page of a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPage {
    /// The other party.
    pub peer: NodeId,
    /// An unaccepted message request.
    pub request: bool,
    /// The newest `limit` messages, oldest first.
    pub messages: Vec<MessageView>,
    /// Older messages exist.
    pub more: bool,
}

/// A contact as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactView {
    /// NodeId.
    pub node_id: NodeId,
    /// Local alias.
    pub alias: Option<String>,
    /// How the keys were learned (a `node1…` contact reads `oob`).
    pub trust: Trust,
    /// A pending message request.
    pub request: bool,
    /// Never pruned.
    pub pinned: bool,
    /// False while only the NodeId is known.
    pub has_key: bool,
    /// `appdata` role bits.
    pub roles: u16,
    /// `appdata` home-hub hint (8 B, a locator).
    pub home_hub: Option<Vec<u8>>,
}

/// A patch to a contact (§15 `PATCH /contacts/{id}`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactPatch {
    /// `Some(None)` clears the alias.
    pub alias: Option<Option<String>>,
    /// Accept a pending message request.
    pub accept: bool,
    /// Pin or unpin.
    pub pinned: Option<bool>,
    /// Mark the thread read up to this `local_seq` (moves forward only).
    pub read_seq: Option<u64>,
}

/// This node's card (§7.2 (1)): the stored announce bytes and their text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardView {
    /// Base32 text (the UI draws the QR from it).
    pub text: String,
    /// The announce bytes.
    pub announce: Vec<u8>,
}

/// A minted enrollment offer (§7.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferView {
    /// The code, `XXXX-XXXX` (shown once).
    pub code: String,
    /// This hub's card text.
    pub card: String,
    /// The `111HUB:` bundle (card, code, address), when an address is known.
    pub bundle: Option<String>,
    /// The `tcp-lan` address.
    pub address: Option<SocketAddr>,
}

/// One link in `GET /fabric`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkView {
    /// Id.
    pub id: LinkId,
    /// Kind.
    pub kind: LinkKind,
    /// State.
    pub state: LinkState,
    /// Attributed peers on this link.
    pub peers: usize,
    /// Path facts.
    pub profile: PathProfile,
}

/// Both ends of the hub relationship (`GET /fabric` `hub`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubView {
    /// This node has a home hub.
    pub enrolled: bool,
    /// A connection to the home hub is up.
    pub reachable: bool,
    /// The committed mailbox cursor.
    pub cursor: u64,
    /// The home hub.
    pub node_id: Option<NodeId>,
    /// Its address.
    pub address: Option<SocketAddr>,
    /// Hub role: enrolled members.
    pub members: u64,
    /// Hub role: an offer is open.
    pub offer_active: bool,
}

/// The store gauge (`GET /fabric` `store`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreView {
    /// Live bytes.
    pub used: u64,
    /// High watermark.
    pub watermark: u64,
    /// New custody and bulk are refused.
    pub store_full: bool,
    /// The run's epoch (`None` while a mint is owed or a restore is pending).
    pub epoch: Option<u64>,
    /// The identity holder reports a restore floor.
    pub restore_pending: bool,
}

/// Bounded diagnostic counters (§18 rule 1: a failure leaves no trace but a
/// counter).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// Frames that failed to open or verify.
    pub dropped: u64,
    /// Verified packets the L2 window judged stale (never attributing).
    pub stale: u64,
    /// INTROs refused by capacity (dropped without an ACK, §7.2 (3)).
    pub refused_requests: u64,
    /// Fragment retransmissions (re-sealed).
    pub retransmits: u64,
    /// Fragments presumed lost from ACK evidence (§5 loss inference: a
    /// later-sent fragment was acknowledged first), never from an event.
    pub inferred_lost: u64,
    /// The most fragments ever outstanding to one peer (§5: ≤ 16).
    pub max_outstanding: u64,
    /// SACK bits regressed after the receiver evicted a partial.
    pub regressed: u64,
    /// ACK blocks that rode a data packet (§6.1 piggyback).
    pub piggybacked: u64,
    /// Hub: `MAILBOX_ITEM`s streamed.
    pub items_pushed: u64,
    /// Member: `MAILBOX_ITEM`s received.
    pub items_received: u64,
    /// Announces or cards conflicting with pinned keys (dropped, surfaced).
    pub conflicts: u64,
    /// Store or sealer failures inside handlers.
    pub errors: u64,
}

/// `GET /fabric`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FabricStatus {
    /// Registered links.
    pub links: Vec<LinkView>,
    /// Hub relationship.
    pub hub: HubView,
    /// Custody held: (count, bytes).
    pub custody: (u64, u64),
    /// Store gauge.
    pub store: StoreView,
    /// Settings.
    pub settings: FabricSettings,
    /// Counters.
    pub counters: Counters,
}

// ---------------------------------------------------------------------------
// The handle
// ---------------------------------------------------------------------------

/// The typed handle the daemon's routes call (cheap to clone).
#[derive(Clone)]
pub struct Fabric {
    inner: Arc<Handle>,
}

struct Handle {
    input: mpsc::UnboundedSender<Input>,
    events: broadcast::Sender<FabricEvent>,
    sink: EventSink,
    gate: Arc<FabricGate>,
    task: Mutex<Option<JoinHandle<()>>>,
    own: NodeId,
}

impl std::fmt::Debug for Fabric {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fabric")
            .field("own", &self.inner.own)
            .finish_non_exhaustive()
    }
}

impl Fabric {
    /// Start the runtime over an opened store (`Store::open` /
    /// `Store::open_with`, whose [`EpochDecision`] is passed in). Mints the
    /// run's epoch through the handle when owed (§4; a refusal means
    /// `RestoreFloorRequired`: nothing is sealed until it clears), finishes
    /// complete partials (a crash between the last fragment and delivery),
    /// rebuilds the outbox index, and spawns the actor on the current Tokio
    /// runtime.
    pub fn start(
        sealer: Arc<dyn Sealer>,
        mut store: Store,
        decision: EpochDecision,
        settings: FabricSettings,
    ) -> Result<Fabric> {
        let unclean = matches!(decision, EpochDecision::Mint(r) if r.unclean);
        if let EpochDecision::Mint(_) = decision {
            match sealer.next_announce_counter() {
                Ok(e) => store.record_epoch(e)?,
                Err(_) => store.set_restore_pending(),
            }
        }
        let own = sealer.node_id();
        let own_announce = sealer.own_announce();
        let (input, rx) = mpsc::unbounded_channel();
        let (frames, frames_rx) = mpsc::channel(FRAME_QUEUE);
        let (events, _) = broadcast::channel(1024);
        let (sink, mut sink_rx) = mpsc::unbounded_channel::<LinkEvent>();
        let keys = KeyCache::rebuild(&store.contacts()?);
        let gate = Arc::new(FabricGate {
            own,
            cache: RwLock::new(keys.clone()),
        });
        let mut core = Core {
            sealer,
            own,
            own_announce,
            store,
            settings,
            links: Vec::new(),
            link_index: HashMap::new(),
            early: Vec::new(),
            conns: BTreeMap::new(),
            keys,
            gate: gate.clone(),
            pending_keys: BTreeSet::new(),
            tracker: TxTracker::new(),
            tx: BTreeMap::new(),
            custody_pending: HashMap::new(),
            custody_inflight: HashSet::new(),
            ack_owed: BTreeMap::new(),
            pump_set: BTreeSet::new(),
            commit_owed: None,
            heard: HashSet::new(),
            offer: None,
            enrolling: None,
            read_marks: HashMap::new(),
            counters: Counters::default(),
            events: events.clone(),
            pending_events: BTreeMap::new(),
            outq: Vec::new(),
            _control: input.clone(),
            frames,
            tasks: Tasks(Vec::new()),
        };
        core.recover(unclean)?;
        let fwd_input = input.clone();
        let fwd = tokio::spawn(async move {
            while let Some(ev) = sink_rx.recv().await {
                if fwd_input.send(Input::Link(ev)).is_err() {
                    return;
                }
            }
        });
        core.tasks.0.push(fwd);
        let task = tokio::spawn(run(core, rx, frames_rx));
        Ok(Fabric {
            inner: Arc::new(Handle {
                input,
                events,
                sink,
                gate,
                task: Mutex::new(Some(task)),
                own,
            }),
        })
    }

    /// This node's NodeId.
    pub fn node_id(&self) -> NodeId {
        self.inner.own
    }

    /// Subscribe to coalesced events.
    pub fn subscribe(&self) -> broadcast::Receiver<FabricEvent> {
        self.inner.events.subscribe()
    }

    /// The sink adapters report [`LinkEvent`]s to (pass it when building a
    /// link, before [`Fabric::add_link`]).
    pub fn link_events(&self) -> EventSink {
        self.inner.sink.clone()
    }

    /// The §8.1 pinned-keys gate for adapters, answered from the contacts
    /// table (authoritative, A7).
    pub fn gate(&self) -> Arc<dyn AnnounceGate> {
        self.inner.gate.clone()
    }

    async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Core) -> T + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = oneshot::channel();
        self.inner
            .input
            .send(Input::Run(Box::new(move |core: &mut Core| {
                let _ = tx.send(f(core));
            })))
            .map_err(|_| FabricError::Stopped)?;
        rx.await.map_err(|_| FabricError::Stopped)
    }

    /// Register a link (and, for `tcp-lan`, its dialer). Returns its index.
    /// When a home hub with an address is configured, the first dialer
    /// registered keeps a connection to it up.
    pub async fn add_link(
        &self,
        link: Arc<dyn Link>,
        dialer: Option<Arc<dyn Dialer>>,
    ) -> Result<usize> {
        self.call(move |c| c.add_link(link, dialer)).await
    }

    /// Queue a text message to a contact (§15 `POST /messages`). It stays
    /// `queued` until a path exists. `sent_at` is the caller's optional
    /// claim (§3.3; the fabric reads no clock).
    pub async fn send(&self, to: NodeId, text: &str, sent_at: Option<u64>) -> Result<i64> {
        let text = text.to_owned();
        self.call(move |c| c.send(to, &text, sent_at)).await?
    }

    /// Cancel an own message that has not left yet: it becomes an empty
    /// tombstone send (§4: an allocated `n` is never abandoned).
    pub async fn cancel(&self, id: i64) -> Result<MessageView> {
        self.call(move |c| c.cancel(id)).await?
    }

    /// Re-queue an own `queued` or `sent` message now (§15 retry).
    pub async fn retry(&self, id: i64) -> Result<MessageView> {
        self.call(move |c| c.retry(id)).await?
    }

    /// A fabric event: retry a pending mint, fetch from the home hub,
    /// presume every outstanding fragment lost, clear custody rejects, and
    /// pump every peer.
    pub async fn retry_all(&self) -> Result<()> {
        self.call(|c| c.retry_all()).await
    }

    /// Thread list, most recent first.
    pub async fn threads(&self) -> Result<Vec<ThreadView>> {
        self.call(|c| c.threads()).await?
    }

    /// The newest `limit` messages with `peer`.
    pub async fn thread(&self, peer: NodeId, limit: usize) -> Result<ThreadPage> {
        self.call(move |c| c.thread(peer, limit)).await?
    }

    /// One message.
    pub async fn message(&self, id: i64) -> Result<MessageView> {
        self.call(move |c| c.message(id)).await?
    }

    /// Contacts, including pending requests and `node1…` contacts.
    pub async fn contacts(&self) -> Result<Vec<ContactView>> {
        self.call(|c| c.contacts()).await?
    }

    /// One contact.
    pub async fn contact(&self, id: NodeId) -> Result<ContactView> {
        self.call(move |c| c.contact_view(&id)).await?
    }

    /// Add a contact by card text (§7.2 (1), `trust = oob`).
    pub async fn add_contact_card(&self, text: &str) -> Result<ContactView> {
        let text = text.to_owned();
        self.call(move |c| c.add_card(&text)).await?
    }

    /// Add a contact by NodeId only (§7.2 (2)); the card is requested from
    /// the home hub.
    pub async fn add_contact_node_id(&self, text: &str) -> Result<ContactView> {
        let text = text.to_owned();
        self.call(move |c| c.add_node_id(&text)).await?
    }

    /// Alias, accept a request, pin, mark read.
    pub async fn patch_contact(&self, id: NodeId, patch: ContactPatch) -> Result<ContactView> {
        self.call(move |c| c.patch_contact(id, patch)).await?
    }

    /// Forget a contact (or discard a request); its pair key is zeroized.
    pub async fn forget_contact(&self, id: NodeId) -> Result<()> {
        self.call(move |c| c.forget(id)).await?
    }

    /// This node's card.
    pub async fn card(&self) -> Result<CardView> {
        self.call(|c| CardView {
            text: card_text(&c.own_announce),
            announce: c.own_announce.clone(),
        })
        .await
    }

    /// `GET /fabric`.
    pub async fn status(&self) -> Result<FabricStatus> {
        self.call(|c| c.status()).await?
    }

    /// Member side (§7.3): enroll with the hub at `address` using its code;
    /// `card` (the hub's card text) is needed unless the hub is already a
    /// contact. Resolves when `ENROLL_OK` arrives.
    pub async fn enroll(&self, address: SocketAddr, code: &str, card: Option<&str>) -> Result<()> {
        let code = code.to_owned();
        let card = card.map(str::to_owned);
        let (tx, rx) = oneshot::channel();
        self.call(move |c| c.enroll(address, &code, card.as_deref(), tx))
            .await??;
        rx.await.map_err(|_| FabricError::Stopped)?
    }

    /// Hub side (§7.3): mint a one-time enrollment code (replacing any open
    /// one). `address` is this hub's `tcp-lan` address for the bundle.
    pub async fn offer(&self, address: Option<SocketAddr>) -> Result<OfferView> {
        self.call(move |c| c.offer(address)).await?
    }

    /// Hub side: dismiss the outstanding offer. Returns whether one was open.
    pub async fn dismiss_offer(&self) -> Result<bool> {
        self.call(|c| c.dismiss_offer()).await
    }

    /// The settings.
    pub async fn settings(&self) -> Result<FabricSettings> {
        self.call(|c| c.settings.clone()).await
    }

    /// Patch the settings and the store's capacity shaping.
    pub async fn patch_settings(&self, patch: SettingsPatch) -> Result<FabricSettings> {
        self.call(move |c| c.patch_settings(patch)).await?
    }

    /// Run `f` against the store inside the actor (capacity passes,
    /// diagnostics). Do not mutate sequence or outbox state through it.
    pub async fn with_store<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> T + Send + 'static,
    ) -> Result<T> {
        self.call(move |c| f(&mut c.store)).await
    }

    /// Clean stop: the store writes its clean-shutdown record (§10.1).
    pub async fn shutdown(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.inner
            .input
            .send(Input::Shutdown(tx))
            .map_err(|_| FabricError::Stopped)?;
        let r = rx.await.map_err(|_| FabricError::Stopped)?;
        if let Ok(mut t) = self.inner.task.lock() {
            t.take();
        }
        r
    }

    /// Unclean stop (a crash): the actor is dropped without closing the
    /// store, so the next open mints a new epoch (§4).
    pub fn abort(&self) {
        if let Ok(mut t) = self.inner.task.lock() {
            if let Some(h) = t.take() {
                h.abort();
            }
        }
    }
}

/// The §8.1 gate over the contacts cache: refuse this node's own NodeId and
/// any announce whose keys differ from a pinned contact's.
struct FabricGate {
    own: NodeId,
    cache: RwLock<KeyCache>,
}

impl AnnounceGate for FabricGate {
    fn admit(&self, announce: &Announce) -> bool {
        let id = announce.node_id();
        if id == self.own {
            return false;
        }
        match self.cache.read() {
            Ok(c) => c
                .get(&id)
                .is_none_or(|k| k.enc_pubkey == *announce.enc_pubkey()),
            Err(_) => false,
        }
    }
}

// ---------------------------------------------------------------------------
// The actor
// ---------------------------------------------------------------------------

type Job = Box<dyn FnOnce(&mut Core) + Send>;

/// One frame from a link's `recv` loop (the bounded queue, module doc).
struct Inbound {
    link: usize,
    peer: PeerAddr,
    frame: Frame,
}

enum Input {
    Frame(Inbound),
    Link(LinkEvent),
    Run(Job),
    Shutdown(oneshot::Sender<Result<()>>),
}

/// Aborts the link `recv` loops and the event forwarder when the actor goes.
struct Tasks(Vec<JoinHandle<()>>);

impl Drop for Tasks {
    fn drop(&mut self) {
        for t in &self.0 {
            t.abort();
        }
    }
}

/// The actor loop: control inputs and frames (module doc "Shape"). Both
/// senders live in `core`, so neither channel closes while it runs.
async fn run(
    mut core: Core,
    mut rx: mpsc::UnboundedReceiver<Input>,
    mut frames: mpsc::Receiver<Inbound>,
) {
    core.flush();
    core.drain_out().await;
    core.emit();
    loop {
        let first = tokio::select! {
            Some(input) = rx.recv() => input,
            Some(f) = frames.recv() => Input::Frame(f),
            else => return,
        };
        let mut stop = None;
        let mut next = Some(first);
        let mut handled = 0usize;
        while let Some(input) = next.take() {
            match input {
                Input::Shutdown(reply) => {
                    stop = Some(reply);
                    break;
                }
                other => core.handle(other),
            }
            handled += 1;
            if handled >= TICK_BATCH {
                break;
            }
            // Control first: the API answers while a peer floods.
            next = match rx.try_recv() {
                Ok(input) => Some(input),
                Err(_) => frames.try_recv().ok().map(Input::Frame),
            };
        }
        core.flush();
        core.drain_out().await;
        core.emit();
        if let Some(reply) = stop {
            let r = core.close();
            let _ = reply.send(r);
            return;
        }
    }
}

type ConnKey = (usize, PeerAddr);

/// One connection (or a test link's far end).
#[derive(Debug, Default)]
struct Conn {
    /// This side dialed it (to an address configured for `expected`).
    outbound: bool,
    /// For a dial: the NodeId that should answer.
    expected: Option<NodeId>,
    /// The first announce's NodeId (authenticates nothing).
    claimed: Option<NodeId>,
    /// Set by a `pair_mac`-verified packet the window judged fresh.
    attributed: Option<NodeId>,
    /// Hub side: the push stream to the attributed member.
    push: PushWindow,
    /// The outbound link-up work ran.
    ready: bool,
}

impl Conn {
    fn sendable_to(&self, node: &NodeId) -> bool {
        self.attributed.as_ref() == Some(node)
            || (self.outbound && self.claimed.as_ref() == Some(node))
    }
}

/// An own message that is not delivered yet (the outbox index).
#[derive(Debug, Clone)]
struct TxMsg {
    peer: NodeId,
    seq: MsgSeq,
    count: u8,
    kind: u8,
    state: MsgState,
    reason: Option<Reason>,
    cancelled: bool,
    hub_rejected: bool,
}

#[derive(Debug, Default)]
struct Owed {
    acked: Option<MsgSeq>,
    strong: Vec<StrongReceipt>,
}

#[derive(Debug, Clone, Copy)]
enum Tag {
    None,
    Frag(i64, u8),
    Custody(i64, u8),
    /// Hub: a `MAILBOX_ITEM` (its `custody_seq`) on a push stream.
    Push(u64),
    /// Member: a `MAILBOX_COMMIT {cursor}`.
    Commit(u64),
}

struct Out {
    ck: ConnKey,
    frame: Frame,
    tag: Tag,
}

struct LinkSlot {
    link: Arc<dyn Link>,
    dialer: Option<Arc<dyn Dialer>>,
}

struct Enrolling {
    hub: NodeId,
    code: Code,
    address: SocketAddr,
    reply: Option<oneshot::Sender<Result<()>>>,
}

enum Route {
    Direct(ConnKey),
    Hub(ConnKey, NodeId),
    None(Option<Reason>),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum EventKey {
    Message(i64),
    Contact([u8; 32]),
    Fabric,
    Settings,
}

struct Core {
    sealer: Arc<dyn Sealer>,
    own: NodeId,
    own_announce: Vec<u8>,
    store: Store,
    settings: FabricSettings,
    links: Vec<LinkSlot>,
    link_index: HashMap<LinkId, usize>,
    early: Vec<LinkEvent>,
    conns: BTreeMap<ConnKey, Conn>,
    keys: KeyCache,
    gate: Arc<FabricGate>,
    pending_keys: BTreeSet<NodeId>,
    tracker: TxTracker,
    tx: BTreeMap<i64, TxMsg>,
    custody_pending: HashMap<[u8; 16], (i64, u8)>,
    custody_inflight: HashSet<(i64, u8)>,
    ack_owed: BTreeMap<NodeId, Owed>,
    pump_set: BTreeSet<NodeId>,
    commit_owed: Option<u64>,
    heard: HashSet<NodeId>,
    offer: Option<EnrollOffer>,
    enrolling: Option<Enrolling>,
    read_marks: HashMap<NodeId, u64>,
    counters: Counters,
    events: broadcast::Sender<FabricEvent>,
    pending_events: BTreeMap<EventKey, FabricEvent>,
    outq: Vec<Out>,
    /// Keeps the control channel open while the actor runs (dropping every
    /// handle does not stop it, module doc).
    _control: mpsc::UnboundedSender<Input>,
    /// The bounded frame queue's sender, cloned into each link's loop.
    frames: mpsc::Sender<Inbound>,
    tasks: Tasks,
}

impl Core {
    // -- loop plumbing -----------------------------------------------------

    fn handle(&mut self, input: Input) {
        match input {
            Input::Frame(f) => self.on_frame(f.link, f.peer, f.frame),
            Input::Link(ev) => self.on_link_event(ev),
            Input::Run(job) => {
                // An API reply never precedes the events describing the
                // state it reports: emit what this tick has so far first.
                self.emit();
                job(self)
            }
            Input::Shutdown(_) => {}
        }
    }

    /// Owed pumps, ACKs and mailbox commits.
    fn flush(&mut self) {
        let mut peers: BTreeSet<NodeId> = std::mem::take(&mut self.pump_set);
        peers.extend(self.ack_owed.keys().copied());
        for p in peers {
            self.pump(p);
        }
        for (peer, owed) in std::mem::take(&mut self.ack_owed) {
            self.send_ack(peer, &owed);
        }
        // A commit stays owed until its frame is queued (and comes back on
        // a send error): a commit owed while the hub is unreachable goes
        // out at the next link-up, beside the fetch.
        if let Some(cursor) = self.commit_owed {
            match self.home_hub() {
                None => self.commit_owed = None,
                Some(hub) => {
                    if let Some(ck) = self.sendable_conn(&hub) {
                        let payload = encode_cursor(cursor);
                        if self.send_control_tagged(
                            &ck,
                            &hub,
                            Kind::MailboxCommit,
                            payload,
                            Tag::Commit(cursor),
                        ) {
                            self.commit_owed = None;
                        }
                    }
                }
            }
        }
    }

    async fn drain_out(&mut self) {
        for _ in 0..DRAIN_ROUNDS {
            let outs = std::mem::take(&mut self.outq);
            if outs.is_empty() {
                return;
            }
            // Connections whose push stream failed this round: later items
            // would overtake the failed one (a gap the member's cursor would
            // commit unseen, §9), so they are rolled back unsent.
            let mut push_failed: HashSet<ConnKey> = HashSet::new();
            for o in outs {
                let Some(link) = self.links.get(o.ck.0).map(|s| s.link.clone()) else {
                    self.on_sent(&o.ck, o.tag, Err(LinkError::Closed));
                    continue;
                };
                let push = matches!(o.tag, Tag::Push(_));
                if push && push_failed.contains(&o.ck) {
                    let e = std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        "an earlier item on this connection was not written",
                    );
                    self.on_sent(&o.ck, o.tag, Err(LinkError::Io(e)));
                    continue;
                }
                let r = link.send(o.ck.1.clone(), o.frame).await;
                if push && r.is_err() {
                    push_failed.insert(o.ck.clone());
                }
                self.on_sent(&o.ck, o.tag, r);
            }
            self.flush();
        }
    }

    fn emit(&mut self) {
        for (_, e) in std::mem::take(&mut self.pending_events) {
            let _ = self.events.send(e);
        }
    }

    fn close(self) -> Result<()> {
        let Core {
            store,
            sealer,
            tasks,
            ..
        } = self;
        drop(tasks);
        store.close(sealer.announce_last())?;
        Ok(())
    }

    // -- events ------------------------------------------------------------

    fn message_event(&mut self, id: i64, peer: NodeId, change: MessageChange) {
        self.pending_events
            .entry(EventKey::Message(id))
            .or_insert(FabricEvent::Message { id, peer, change });
    }

    fn contact_event(&mut self, node_id: NodeId, change: ContactChange) {
        self.pending_events
            .entry(EventKey::Contact(node_id.to_bytes()))
            .or_insert(FabricEvent::Contact { node_id, change });
    }

    fn fabric_changed(&mut self) {
        self.pending_events
            .entry(EventKey::Fabric)
            .or_insert(FabricEvent::Fabric);
    }

    fn settings_changed(&mut self) {
        self.pending_events.insert(
            EventKey::Settings,
            FabricEvent::Settings(self.settings.clone()),
        );
    }

    // -- startup -----------------------------------------------------------

    fn recover(&mut self, unclean: bool) -> Result<()> {
        // §5: a crash between the last fragment and delivery.
        for p in self.store.complete_partials()? {
            let Ok(n) = u32::try_from(p.n) else { continue };
            self.finish_partial(
                NodeId::from(p.peer),
                p.stream,
                MsgSeq::new(p.epoch, n),
                Kind::Msg.byte(),
                MessageChange::New,
            );
        }
        // The outbox index: every own message with outbox rows.
        for peer in self.store.outbox_peers()? {
            let mut counts: BTreeMap<i64, u8> = BTreeMap::new();
            for f in self.store.outbox_for(&peer)? {
                counts.insert(f.message_id, f.frag_count);
            }
            for (id, count) in counts {
                if let Some(row) = self.store.message(id)? {
                    self.index_own(&row, count);
                }
            }
        }
        if unclean {
            self.requeue_orphans()?;
        }
        for (peer, _) in self.store.rx_streams()? {
            self.heard.insert(NodeId::from(peer));
        }
        // §7.2 (2): stored `node1…` contacts, plus every outbox peer that
        // still lacks a key (a forgotten contact's queued messages).
        for id in self.store.pending_node_ids()? {
            let id = NodeId::from(id);
            if self.keys.get(&id).is_some() {
                // Pinned in a crash window before the row was removed.
                self.store.pending_node_id_remove(id.as_bytes())?;
            } else {
                self.pending_keys.insert(id);
            }
        }
        let missing: Vec<NodeId> = self
            .tx
            .values()
            .map(|m| m.peer)
            .filter(|p| self.keys.get(p).is_none())
            .collect();
        self.pending_keys.extend(missing);
        self.pump_all();
        Ok(())
    }

    /// After a crash: an own message numbered and stored but whose outbox
    /// rows were not yet written is planned again (never abandoned, §4) —
    /// for the destination path's profile (§5; at start no link is
    /// registered, so the 160 B default), else for a stream link when the
    /// text exceeds Tier 1 at that size (only a stream path could have
    /// accepted it when it was queued).
    fn requeue_orphans(&mut self) -> Result<()> {
        for row in self.store.own_unplanned()? {
            if self.tx.contains_key(&row.id) {
                continue;
            }
            let peer = NodeId::from(row.peer);
            let len = row.body.as_ref().map_or(0, Vec::len);
            let seq = msg_seq(&row);
            let (mtu, shim) = self.plan_mtu(&peer);
            let count = plan(len, mtu, shim, seq)
                .or_else(|_| plan(len, MAX_PACKET_LEN - PLAN_RESERVE, false, seq))
                .unwrap_or(1);
            for idx in 0..count {
                self.store.outbox_put(row.id, idx, count, &row.peer, None)?;
            }
            self.index_own(&row, count);
        }
        Ok(())
    }

    fn index_own(&mut self, row: &MessageRow, count: u8) {
        if row.dir != Dir::Own || row.state == MsgState::Delivered {
            return;
        }
        let reason = row.reason.as_deref().and_then(Reason::from_code);
        self.tx.insert(
            row.id,
            TxMsg {
                peer: NodeId::from(row.peer),
                seq: msg_seq(row),
                count,
                kind: row.kind,
                state: row.state,
                reason,
                cancelled: reason == Some(Reason::Cancelled),
                hub_rejected: false,
            },
        );
    }

    fn pump_all(&mut self) {
        let peers: Vec<NodeId> = self.tx.values().map(|m| m.peer).collect();
        self.pump_set.extend(peers);
    }

    fn rebuild_keys(&mut self) {
        match self.store.contacts() {
            Ok(rows) => {
                self.keys = KeyCache::rebuild(&rows);
                if let Ok(mut g) = self.gate.cache.write() {
                    *g = self.keys.clone();
                }
            }
            Err(_) => self.counters.errors += 1,
        }
    }

    // -- links -------------------------------------------------------------

    fn add_link(&mut self, link: Arc<dyn Link>, dialer: Option<Arc<dyn Dialer>>) -> usize {
        let idx = self.links.len();
        self.link_index.insert(link.id(), idx);
        let frames = self.frames.clone();
        let rx_link = link.clone();
        // Waits while the frame queue is full: the link's own bounded queue
        // then fills and its readers stop (back-pressure, module doc).
        let task = tokio::spawn(async move {
            loop {
                let (peer, frame) = rx_link.recv().await;
                let inbound = Inbound {
                    link: idx,
                    peer,
                    frame,
                };
                if frames.send(inbound).await.is_err() {
                    return;
                }
            }
        });
        self.tasks.0.push(task);
        let dial_hub = dialer.is_some() && !self.links.iter().any(|s| s.dialer.is_some());
        self.links.push(LinkSlot {
            link: link.clone(),
            dialer: dialer.clone(),
        });
        if dial_hub {
            if let (Some(d), Some(h)) = (dialer, self.settings.home_hub) {
                if let Some(addr) = h.address {
                    d.maintain(addr, Some(h.node_id));
                }
            }
        }
        let id = link.id();
        let early: Vec<LinkEvent> = std::mem::take(&mut self.early);
        for ev in early {
            let mine = match &ev {
                LinkEvent::Connected { link, .. } | LinkEvent::Disconnected { link, .. } => {
                    *link == id
                }
            };
            if mine {
                self.on_link_event(ev);
            } else {
                self.early.push(ev);
            }
        }
        self.fabric_changed();
        idx
    }

    fn dialer(&self) -> Option<Arc<dyn Dialer>> {
        self.links.iter().find_map(|s| s.dialer.clone())
    }

    fn hang_up(&mut self, ck: &ConnKey) {
        if let Some(d) = self.links.get(ck.0).and_then(|s| s.dialer.clone()) {
            d.hang_up(&ck.1);
        }
        self.drop_conn(ck);
    }

    fn on_link_event(&mut self, ev: LinkEvent) {
        match ev {
            LinkEvent::Connected {
                link,
                peer,
                outbound,
                expected,
            } => {
                let Some(&idx) = self.link_index.get(&link) else {
                    self.early.push(LinkEvent::Connected {
                        link,
                        peer,
                        outbound,
                        expected,
                    });
                    return;
                };
                let ck = (idx, peer);
                let c = self.conns.entry(ck.clone()).or_default();
                c.outbound = outbound;
                c.expected = expected;
                if let (Some(exp), Some(cl)) = (c.expected, c.claimed) {
                    if exp != cl {
                        self.hang_up(&ck);
                        return;
                    }
                }
                self.conn_ready(&ck);
                self.fabric_changed();
            }
            LinkEvent::Disconnected { link, peer } => match self.link_index.get(&link) {
                Some(&idx) => self.drop_conn(&(idx, peer)),
                None => self.early.retain(|e| {
                    !matches!(e, LinkEvent::Connected { link: l, peer: p, .. } if *l == link && *p == peer)
                }),
            },
        }
    }

    /// Outbound link-up work, once the connection's claim and direction are
    /// both known: fetch from the home hub, re-ask for missing cards, send a
    /// pending ENROLL, pump.
    fn conn_ready(&mut self, ck: &ConnKey) {
        let Some(c) = self.conns.get_mut(ck) else {
            return;
        };
        let Some(node) = c.claimed else { return };
        if !c.outbound || c.ready {
            return;
        }
        c.ready = true;
        if self.enrolling.as_ref().is_some_and(|e| e.hub == node) {
            self.send_enroll(ck);
        }
        if Some(node) == self.home_hub() {
            self.hub_link_up(ck);
        }
        self.pump_set.insert(node);
    }

    fn hub_link_up(&mut self, ck: &ConnKey) {
        let Some(hub) = self.home_hub() else { return };
        let cursor = self.store.cursor(hub.as_bytes()).unwrap_or(0);
        self.send_control(ck, &hub, Kind::MailboxFetch, encode_cursor(cursor));
        // §9: the hub keeps max(cursor), so committing the stored cursor
        // beside every fetch is idempotent and repairs a commit lost with a
        // socket or a restart.
        if cursor > 0
            && self.send_control_tagged(
                ck,
                &hub,
                Kind::MailboxCommit,
                encode_cursor(cursor),
                Tag::Commit(cursor),
            )
            && self.commit_owed.is_some_and(|c| c <= cursor)
        {
            self.commit_owed = None;
        }
        let wanted: Vec<NodeId> = self.pending_keys.iter().copied().collect();
        for w in wanted {
            self.outq.push(Out {
                ck: ck.clone(),
                frame: Frame::card_request(&w),
                tag: Tag::None,
            });
        }
        for m in self.tx.values_mut() {
            m.hub_rejected = false;
        }
        self.pump_all();
        self.fabric_changed();
    }

    fn drop_conn(&mut self, ck: &ConnKey) {
        let Some(c) = self.conns.remove(ck) else {
            return;
        };
        let mut nodes: Vec<NodeId> = c.attributed.into_iter().collect();
        nodes.extend(c.claimed.filter(|n| Some(*n) != c.attributed));
        for node in nodes {
            self.presume_lost(&node);
            if Some(node) == self.home_hub() {
                self.custody_inflight.clear();
                self.custody_pending.clear();
                self.pump_all();
            }
            self.pump_set.insert(node);
        }
        self.fabric_changed();
    }

    fn presume_lost(&mut self, peer: &NodeId) {
        for (id, idx) in self.frags_of(peer) {
            if self.tracker.get(id, idx).outstanding {
                self.tracker.set_outstanding(id, idx, false);
                let _ = self.store.outbox_set_outstanding(id, idx, false);
            }
        }
    }

    fn frags_of(&self, peer: &NodeId) -> Vec<(i64, u8)> {
        let mut out = Vec::new();
        for (id, m) in &self.tx {
            if m.peer == *peer {
                out.extend((0..m.count).map(|i| (*id, i)));
            }
        }
        out
    }

    fn sendable_conn(&self, node: &NodeId) -> Option<ConnKey> {
        let mut dialed: Option<&ConnKey> = None;
        for (k, c) in &self.conns {
            if c.attributed.as_ref() == Some(node) {
                return Some(k.clone());
            }
            if dialed.is_none() && c.sendable_to(node) {
                dialed = Some(k);
            }
        }
        dialed.cloned()
    }

    fn home_hub(&self) -> Option<NodeId> {
        self.settings.home_hub.map(|h| h.node_id)
    }

    fn profile(&self, ck: &ConnKey) -> (PathProfile, LinkKind) {
        match self.links.get(ck.0) {
            Some(s) => (s.link.profile(), s.link.kind()),
            None => (PathProfile::unknown(), LinkKind::TcpLan),
        }
    }

    /// Sealed bytes that fit one frame on `ck`'s path.
    fn room(&self, ck: &ConnKey) -> usize {
        let (p, _) = self.profile(ck);
        usize::from(p.mtu_bytes).saturating_sub(p.shim_len())
    }

    fn route(&self, peer: &NodeId) -> Route {
        if let Some(ck) = self.sendable_conn(peer) {
            return Route::Direct(ck);
        }
        let Some(hub) = self.home_hub() else {
            return Route::None(None);
        };
        if hub == *peer {
            return Route::None(None);
        }
        match self.sendable_conn(&hub) {
            // The hub decides (`REJECT_UNKNOWN_DEST`): a contact's hint is a
            // locator only (§7.3) and may be stale (module doc).
            Some(ck) => Route::Hub(ck, hub),
            // §9 "their hub is not yours": the hint explains a queued one.
            None => Route::None(
                self.hinted_elsewhere(peer, &hub)
                    .then_some(Reason::HubNotYours),
            ),
        }
    }

    /// The contact's announce hints at a home hub that is not `hub`.
    fn hinted_elsewhere(&self, peer: &NodeId, hub: &NodeId) -> bool {
        matches!(
            self.store.contact(peer.as_bytes()),
            Ok(Some(c)) if c.home_hub.as_ref().is_some_and(|h| h.len() == 8 && h[..] != hub.as_bytes()[..8])
        )
    }

    /// §5: the MTU a new message to `peer` is planned for, and the shim.
    fn plan_mtu(&self, peer: &NodeId) -> (usize, bool) {
        let path = match self.route(peer) {
            Route::Direct(ck) => Some((self.profile(&ck).0, 0)),
            Route::Hub(ck, _) => Some((self.profile(&ck).0, CUSTODY_WRAP)),
            Route::None(_) => None,
        };
        let (mtu, shim) = match path {
            Some((p, wrap)) => (usize::from(p.mtu_bytes).saturating_sub(wrap), p.broadcast),
            None => match self
                .links
                .iter()
                .map(|s| s.link.profile())
                .min_by_key(|p| p.mtu_bytes)
            {
                Some(p) => (usize::from(p.mtu_bytes), p.broadcast),
                None => (usize::from(DEFAULT_MTU), false),
            },
        };
        let reserve = if mtu >= STREAM_MTU { PLAN_RESERVE } else { 0 };
        (mtu - reserve, shim)
    }

    fn can_seal(&self) -> bool {
        matches!(self.store.epoch_state(), EpochState::Ready { .. })
    }

    fn try_mint(&mut self) {
        if self.can_seal() {
            return;
        }
        if let Ok(e) = self.sealer.next_announce_counter() {
            if self.store.record_epoch(e).is_ok() {
                self.pump_all();
                self.fabric_changed();
            }
        }
    }

    // -- sealing -----------------------------------------------------------

    /// Seal a control payload to `to` and queue it on `ck`. Returns whether
    /// the frame was queued (keys known, sealing allowed, it fits the path).
    fn send_control(&mut self, ck: &ConnKey, to: &NodeId, kind: Kind, payload: Vec<u8>) -> bool {
        self.send_control_tagged(ck, to, kind, payload, Tag::None)
    }

    fn send_control_tagged(
        &mut self,
        ck: &ConnKey,
        to: &NodeId,
        kind: Kind,
        payload: Vec<u8>,
        tag: Tag,
    ) -> bool {
        match self.keys.get(to) {
            Some(keys) => self.send_control_keys(ck, &keys, kind, payload, tag),
            None => false,
        }
    }

    fn send_control_keys(
        &mut self,
        ck: &ConnKey,
        to: &PeerKeys,
        kind: Kind,
        payload: Vec<u8>,
        tag: Tag,
    ) -> bool {
        if !self.can_seal() {
            return false;
        }
        let inner = Inner::new(&self.own, kind, MsgSeq::CONTROL, payload);
        match seal_envelope(&*self.sealer, to, Priority::Control, &inner) {
            Ok(sealed) if sealed.len() <= self.room(ck) => {
                self.outq.push(Out {
                    ck: ck.clone(),
                    frame: Frame::sealed(sealed),
                    tag,
                });
                true
            }
            Ok(_) => false,
            Err(_) => {
                self.counters.errors += 1;
                false
            }
        }
    }

    fn send_enroll(&mut self, ck: &ConnKey) {
        let Some(e) = &self.enrolling else { return };
        let (hub, payload) = (e.hub, encode_enroll(&e.code));
        let Some(keys) = self.keys.get(&hub) else {
            return;
        };
        if !self.can_seal() {
            return;
        }
        let mut inner = Inner::new(&self.own, Kind::Enroll, MsgSeq::CONTROL, payload);
        inner.card = Some(self.own_announce.clone());
        match seal_envelope(&*self.sealer, &keys, Priority::Control, &inner) {
            Ok(sealed) => self.outq.push(Out {
                ck: ck.clone(),
                frame: Frame::sealed(sealed),
                tag: Tag::None,
            }),
            Err(_) => self.counters.errors += 1,
        }
    }

    /// Seal fragment `idx` of own message `id` within `room` sealed bytes.
    /// Tier 0 may carry the INTRO card (`intro_ok`: a `tcp-lan` path to a
    /// peer not yet heard from), `seen_peer_seq`, and — when `piggyback` —
    /// an owed ACK block for the peer; extras are dropped before the payload
    /// is ever refused. Re-sealing is every retransmission's fresh epk +
    /// counter (§5).
    fn seal_data(
        &mut self,
        keys: &PeerKeys,
        id: i64,
        idx: u8,
        room: usize,
        intro_ok: bool,
        piggyback: bool,
    ) -> Option<Vec<u8>> {
        let m = self.tx.get(&id)?.clone();
        let row = self.store.message(id).ok()??;
        let body = if m.cancelled {
            Vec::new()
        } else {
            row.body.unwrap_or_default()
        };
        let kind = Kind::from_byte(m.kind)?;
        let payload = if m.count == 1 {
            body
        } else {
            frag_slice(&body, m.count, idx)?.to_vec()
        };
        let mut inner = Inner::new(&self.own, kind, m.seq, payload);
        let mut used_ack = false;
        if m.count > 1 {
            inner.frag = Some(FragPos {
                idx,
                count: m.count,
            });
        } else {
            if intro_ok && kind.intro_allowed() && !self.heard.contains(&m.peer) {
                inner.card = Some(self.own_announce.clone());
                if sealed_len(&inner).is_none_or(|l| l > room) {
                    inner.card = None;
                }
            }
            if kind == Kind::Msg {
                if let Ok(snap) = self.store.rx_snapshot(m.peer.as_bytes(), STREAM_1TO1) {
                    inner.ext.seen_peer_seq = highest_seen(&snap);
                    if sealed_len(&inner).is_none_or(|l| l > room) {
                        inner.ext = Default::default();
                    }
                }
            }
            if piggyback && !self.request_pending(&m.peer) {
                if let (Some(owed), Some(base)) = (self.ack_owed.get(&m.peer), sealed_len(&inner)) {
                    // ack_len varint (≤ 3 B) is inside the block's encoding.
                    let budget = room.saturating_sub(base);
                    if let Ok(snap) = self.store.rx_snapshot(m.peer.as_bytes(), STREAM_1TO1) {
                        if let Some(block) =
                            build_ack_block(&snap, owed.acked, &owed.strong, budget)
                        {
                            inner.ack = Some(block);
                            used_ack = sealed_len(&inner).is_some_and(|l| l <= room);
                            if !used_ack {
                                inner.ack = None;
                            }
                        }
                    }
                }
            }
        }
        if sealed_len(&inner).is_none_or(|l| l > room) {
            return None;
        }
        let prio = kind.default_priority().unwrap_or(Priority::Interactive);
        match seal_envelope(&*self.sealer, keys, prio, &inner) {
            Ok(sealed) => {
                if used_ack {
                    self.ack_owed.remove(&m.peer);
                    self.counters.piggybacked += 1;
                }
                Some(sealed)
            }
            Err(_) => {
                self.counters.errors += 1;
                None
            }
        }
    }

    // -- outbox ------------------------------------------------------------

    fn set_state(&mut self, id: i64, state: MsgState, reason: Option<Reason>) {
        let Some(m) = self.tx.get_mut(&id) else {
            return;
        };
        let reason = if m.cancelled {
            Some(Reason::Cancelled)
        } else {
            reason
        };
        if state < m.state || (state == m.state && reason == m.reason) {
            return;
        }
        let peer = m.peer;
        match self
            .store
            .set_message_state(id, state, reason.map(Reason::code))
        {
            Ok(_) => {
                m.state = state;
                m.reason = reason;
                self.message_event(id, peer, MessageChange::State);
            }
            Err(_) => self.counters.errors += 1,
        }
    }

    fn set_reason_if_queued(&mut self, id: i64, reason: Option<Reason>) {
        if self
            .tx
            .get(&id)
            .is_some_and(|m| m.state == MsgState::Queued)
        {
            self.set_state(id, MsgState::Queued, reason);
        }
    }

    fn delivered(&mut self, id: i64) {
        let Some(m) = self.tx.remove(&id) else {
            return;
        };
        self.tracker.forget(id);
        self.custody_inflight.retain(|(i, _)| *i != id);
        self.custody_pending.retain(|_, (i, _)| *i != id);
        let reason = m.cancelled.then_some(Reason::Cancelled);
        match self
            .store
            .set_message_state(id, MsgState::Delivered, reason.map(Reason::code))
        {
            Ok(_) => self.message_event(id, m.peer, MessageChange::State),
            Err(_) => self.counters.errors += 1,
        }
    }

    fn pump(&mut self, peer: NodeId) {
        let ids: Vec<i64> = self
            .tx
            .iter()
            .filter(|(_, m)| m.peer == peer)
            .map(|(id, _)| *id)
            .collect();
        if ids.is_empty() {
            return;
        }
        if !self.can_seal() {
            for id in ids {
                self.set_reason_if_queued(id, Some(Reason::RestorePending));
            }
            return;
        }
        let Some(keys) = self.keys.get(&peer) else {
            for id in ids {
                self.set_reason_if_queued(id, Some(Reason::WaitingForKey));
            }
            return;
        };
        match self.route(&peer) {
            Route::Direct(ck) => self.pump_direct(&keys, &ck, &ids),
            Route::Hub(ck, hub) => self.pump_hub(&keys, &ck, &hub, &ids),
            Route::None(reason) => {
                let reason =
                    reason.or_else(|| self.store.store_full().then_some(Reason::StoreFull));
                for id in ids {
                    self.set_reason_if_queued(id, reason);
                }
            }
        }
    }

    /// §5 direct path: ≤ 16 outstanding fragments per peer, fresh fragments
    /// first, then retransmissions (re-sealed).
    fn pump_direct(&mut self, keys: &PeerKeys, ck: &ConnKey, ids: &[i64]) {
        let peer = keys.node_id;
        let room = self.room(ck);
        let tcp = self.profile(ck).1 == LinkKind::TcpLan;
        let frags = self.frags_of(&peer);
        let wanted: HashSet<i64> = ids.iter().copied().collect();
        let mut outstanding = self.tracker.outstanding(&frags);
        for fresh_pass in [true, false] {
            for &(id, idx) in &frags {
                if outstanding >= MAX_OUTSTANDING {
                    break;
                }
                if !wanted.contains(&id) {
                    continue;
                }
                let f = self.tracker.get(id, idx);
                if f.sacked || f.outstanding || (f.tick == 0) != fresh_pass {
                    continue;
                }
                let Some(bytes) = self.seal_data(keys, id, idx, room, tcp, true) else {
                    continue;
                };
                self.tracker.on_sent(id, idx);
                let _ = self.store.outbox_set_outstanding(id, idx, true);
                outstanding += 1;
                if !fresh_pass {
                    self.counters.retransmits += 1;
                }
                self.outq.push(Out {
                    ck: ck.clone(),
                    frame: Frame::sealed(bytes),
                    tag: Tag::Frag(id, idx),
                });
            }
        }
        self.counters.max_outstanding = self.counters.max_outstanding.max(outstanding as u64);
    }

    /// §9 member side: hand every fragment the hub does not hold yet to it
    /// as `CUSTODY_PUT {dest ‖ item}`. The item's sealed bytes are kept in
    /// the outbox so a retry re-sends the same bytes (`pkt_ref` dedup).
    fn pump_hub(&mut self, keys: &PeerKeys, hub_ck: &ConnKey, hub: &NodeId, ids: &[i64]) {
        let Some(hub_keys) = self.keys.get(hub) else {
            return;
        };
        let peer = keys.node_id;
        let room = self.room(hub_ck);
        let tcp = self.profile(hub_ck).1 == LinkKind::TcpLan;
        let stored: HashMap<(i64, u8), Vec<u8>> = match self.store.outbox_for(peer.as_bytes()) {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|f| f.sealed.map(|s| ((f.message_id, f.frag_idx), s)))
                .collect(),
            Err(_) => {
                self.counters.errors += 1;
                return;
            }
        };
        for &id in ids {
            let Some(m) = self.tx.get(&id).cloned() else {
                continue;
            };
            if m.hub_rejected {
                continue;
            }
            for idx in 0..m.count {
                let f = self.tracker.get(id, idx);
                if f.held || f.sacked || self.custody_inflight.contains(&(id, idx)) {
                    continue;
                }
                let item = match stored.get(&(id, idx)) {
                    Some(b) => b.clone(),
                    None => {
                        let Some(b) = self.seal_data(
                            keys,
                            id,
                            idx,
                            room.saturating_sub(CUSTODY_WRAP),
                            tcp,
                            false,
                        ) else {
                            continue;
                        };
                        if self
                            .store
                            .outbox_put(id, idx, m.count, peer.as_bytes(), Some(b.as_slice()))
                            .is_err()
                        {
                            self.counters.errors += 1;
                            continue;
                        }
                        b
                    }
                };
                let inner = Inner::new(
                    &self.own,
                    Kind::CustodyPut,
                    MsgSeq::CONTROL,
                    encode_put(&peer, &item),
                );
                let Ok(sealed) =
                    seal_envelope(&*self.sealer, &hub_keys, Priority::Interactive, &inner)
                else {
                    self.counters.errors += 1;
                    continue;
                };
                if sealed.len() > room {
                    continue;
                }
                self.custody_pending.insert(pkt_ref(&item), (id, idx));
                self.custody_inflight.insert((id, idx));
                self.outq.push(Out {
                    ck: hub_ck.clone(),
                    frame: Frame::sealed(sealed),
                    tag: Tag::Custody(id, idx),
                });
            }
        }
    }

    fn on_sent(&mut self, ck: &ConnKey, tag: Tag, r: Result<(), LinkError>) {
        match (tag, r) {
            (Tag::Frag(id, _), Ok(())) => {
                // Written to the peer itself: `sent`, and a custody reject's
                // reason no longer explains it (module doc).
                let clear = self.tx.get(&id).is_some_and(|m| {
                    m.state == MsgState::Queued
                        || (m.state == MsgState::Sent && m.reason.is_some() && !m.cancelled)
                });
                if clear {
                    self.set_state(id, MsgState::Sent, None);
                }
            }
            (Tag::Custody(id, _), Ok(())) => {
                if self
                    .tx
                    .get(&id)
                    .is_some_and(|m| m.state == MsgState::Queued)
                {
                    self.set_state(id, MsgState::Sent, None);
                }
            }
            (Tag::None | Tag::Push(_) | Tag::Commit(_), Ok(())) => {}
            (Tag::Push(seq), Err(e)) => {
                // Not written: the stream restarts just below it.
                if let Some(c) = self.conns.get_mut(ck) {
                    c.push.unpush(seq);
                }
                self.on_send_error(ck, &e);
            }
            (Tag::Commit(cursor), Err(e)) => {
                self.commit_owed = Some(self.commit_owed.unwrap_or(0).max(cursor));
                self.on_send_error(ck, &e);
            }
            (Tag::Frag(id, idx), Err(e)) => {
                self.tracker.set_outstanding(id, idx, false);
                let _ = self.store.outbox_set_outstanding(id, idx, false);
                self.on_send_error(ck, &e);
            }
            (Tag::Custody(id, idx), Err(e)) => {
                self.custody_inflight.remove(&(id, idx));
                self.custody_pending.retain(|_, v| *v != (id, idx));
                self.on_send_error(ck, &e);
            }
            (Tag::None, Err(e)) => self.on_send_error(ck, &e),
        }
    }

    fn on_send_error(&mut self, ck: &ConnKey, e: &LinkError) {
        if matches!(e, LinkError::Closed) {
            self.drop_conn(ck);
        }
    }

    /// The standalone ACK (§6.1) for `peer`, unless a message request from
    /// it is pending (never ACKed until accepted, §7.2 (3)).
    fn send_ack(&mut self, peer: NodeId, owed: &Owed) {
        if !self.can_seal() || self.request_pending(&peer) {
            return;
        }
        let Some(keys) = self.keys.get(&peer) else {
            return;
        };
        let (ck, hub, room) = match self.route(&peer) {
            Route::Direct(ck) => {
                let room = self.room(&ck);
                (ck, None, room)
            }
            Route::Hub(ck, hub) => {
                let room = self.room(&ck).saturating_sub(CUSTODY_WRAP);
                (ck, Some(hub), room)
            }
            Route::None(_) => return,
        };
        let Ok(snap) = self.store.rx_snapshot(peer.as_bytes(), STREAM_1TO1) else {
            self.counters.errors += 1;
            return;
        };
        let budget = payload_budget(room, false, MsgSeq::CONTROL, None, false).unwrap_or(0);
        let Some(block) = build_ack_block(&snap, owed.acked, &owed.strong, budget) else {
            return;
        };
        let Ok(p) = AckPayload::build(&*self.sealer, block, &peer, STREAM_1TO1, false) else {
            return;
        };
        let inner = Inner::new(&self.own, Kind::Ack, MsgSeq::CONTROL, p.encode());
        let Ok(sealed) = seal_envelope(&*self.sealer, &keys, Priority::Control, &inner) else {
            self.counters.errors += 1;
            return;
        };
        match hub {
            None => self.outq.push(Out {
                ck,
                frame: Frame::sealed(sealed),
                tag: Tag::None,
            }),
            Some(hub) => {
                // Through the mailbox: the ACK rides as a custody item.
                self.send_control(&ck, &hub, Kind::CustodyPut, encode_put(&peer, &sealed));
            }
        }
    }

    // -- inbound -----------------------------------------------------------

    fn on_frame(&mut self, link: usize, peer: PeerAddr, frame: Frame) {
        let ck = (link, peer);
        if !self.conns.contains_key(&ck) {
            // A frame that raced its connection's disconnect re-creates no
            // entry for a peer the link no longer lists: its sealed content
            // still counts (data and ACKs), it attributes nothing.
            let live = self
                .links
                .get(link)
                .is_some_and(|s| s.link.peers().contains(&ck.1));
            if !live {
                if frame.frame_type == FrameType::Sealed {
                    self.on_sealed(None, &frame.bytes);
                }
                return;
            }
            self.conns.insert(ck.clone(), Conn::default());
        }
        match frame.frame_type {
            FrameType::Announce => self.on_announce(&ck, &frame.bytes),
            FrameType::CardRequest => self.on_card_request(&ck, &frame.bytes),
            FrameType::Sealed => self.on_sealed(Some(ck), &frame.bytes),
            FrameType::Chunk | FrameType::Foreign => {}
        }
    }

    /// A cleartext announce (§7, §8.1): the connection's claim when it is
    /// the first, a card answer when we asked for that NodeId, a refresh of
    /// a pinned contact; a key conflict drops the connection.
    fn on_announce(&mut self, ck: &ConnKey, bytes: &[u8]) {
        let Ok(ann) = verify_card(bytes) else {
            self.counters.dropped += 1;
            return;
        };
        let id = ann.node_id();
        let pinned = match self.store.contact(id.as_bytes()) {
            Ok(c) => c,
            Err(_) => {
                self.counters.errors += 1;
                return;
            }
        };
        match classify_announce(&self.own, pinned.as_ref(), &ann) {
            AnnounceVerdict::Own => {
                self.hang_up(ck);
                return;
            }
            AnnounceVerdict::Conflict => {
                self.counters.conflicts += 1;
                self.contact_event(id, ContactChange::Updated);
                self.hang_up(ck);
                return;
            }
            AnnounceVerdict::Refresh => {
                if let Some(c) = pinned {
                    self.refresh_contact(&c, &ann, bytes);
                }
            }
            AnnounceVerdict::Stale | AnnounceVerdict::Unknown => {}
        }
        if self.pending_keys.contains(&id) {
            // §7.2 (2): the card answer, verified against the expected NodeId.
            if self.pin(&ann, bytes, Trust::Oob).is_ok() {
                // `pin` cleared the pending entry.
                self.contact_event(id, ContactChange::Updated);
                self.pump_set.insert(id);
            }
        }
        let Some(c) = self.conns.get_mut(ck) else {
            return;
        };
        if c.claimed.is_none() {
            c.claimed = Some(id);
            if c.expected.is_some_and(|e| e != id) {
                self.hang_up(ck);
                return;
            }
            self.conn_ready(ck);
            self.fabric_changed();
        }
    }

    fn refresh_contact(&mut self, c: &ContactRow, ann: &Announce, bytes: &[u8]) {
        let mut row = contact_row(ann, bytes, c.trust);
        row.alias = c.alias.clone();
        if self.store.contact_upsert(&row).is_ok() {
            self.contact_event(ann.node_id(), ContactChange::Updated);
        }
    }

    /// Pin a verified announce at `trust` (monotone: the store keeps the
    /// stronger trust and the alias); a pending `node1…` entry for it is
    /// cleared.
    fn pin(&mut self, ann: &Announce, bytes: &[u8], trust: Trust) -> Result<()> {
        let id = ann.node_id();
        let mut row = contact_row(ann, bytes, trust);
        if let Some(existing) = self.store.contact(id.as_bytes())? {
            row.alias = existing.alias;
        }
        match self.store.contact_upsert(&row) {
            Ok(()) => {}
            Err(StoreError::KeyConflict) => {
                self.counters.conflicts += 1;
                return Err(FabricError::KeyConflict);
            }
            Err(e) => return Err(e.into()),
        }
        self.rebuild_keys();
        // Keys are known now: a `node1…` contact is no longer waiting (an
        // enrollment or an accepted request may pin it too).
        self.pending_clear(&id);
        Ok(())
    }

    /// §7.2 (2) hub side: answer a `node1…` card request only for an
    /// enrolled contact, only to an enrolled member on a connection
    /// attributed to it, from stored bytes; never forwarded.
    fn on_card_request(&mut self, ck: &ConnKey, body: &[u8]) {
        if !self.settings.holds_custody() {
            return;
        }
        let Ok(wanted) = <[u8; 32]>::try_from(body) else {
            return;
        };
        let Some(member) = self.conns.get(ck).and_then(|c| c.attributed) else {
            return;
        };
        if !self.is_enrolled(&member) || member.to_bytes() == wanted {
            return;
        }
        if let Ok(Some(c)) = self.store.contact(&wanted) {
            if c.trust == Trust::Enrolled && Some(NodeId::from(wanted)) != self.home_hub() {
                self.outq.push(Out {
                    ck: ck.clone(),
                    frame: Frame::announce(c.announce),
                    tag: Tag::None,
                });
            }
        }
    }

    fn is_enrolled(&self, node: &NodeId) -> bool {
        matches!(self.store.contact(node.as_bytes()), Ok(Some(c)) if c.trust == Trust::Enrolled)
    }

    fn request_pending(&self, node: &NodeId) -> bool {
        self.store.request_pending(node.as_bytes()).unwrap_or(false)
    }

    fn attribute(&mut self, ck: &ConnKey, sender: NodeId) {
        let Some(c) = self.conns.get_mut(ck) else {
            return;
        };
        if c.attributed != Some(sender) {
            if c.attributed.is_some() {
                c.push = PushWindow::default();
            }
            c.attributed = Some(sender);
            self.pump_set.insert(sender);
            self.fabric_changed();
        }
    }

    /// §18 rule 1 for one sealed packet. `ck = None`: an item that arrived
    /// through the mailbox (it attributes nothing and carries only data and
    /// ACKs).
    fn on_sealed(&mut self, ck: Option<ConnKey>, bytes: &[u8]) {
        let opened = {
            let keys = &self.keys;
            match open_envelope(&*self.sealer, bytes, &|p| keys.lookup(p)) {
                Ok(o) => o,
                Err(_) => {
                    self.counters.dropped += 1;
                    return;
                }
            }
        };
        let sender = opened.sender.node_id;
        // Every verified packet is offered to the L2 window (A11).
        let fresh = self
            .sealer
            .observe_counter(&sender, opened.counter())
            .unwrap_or(Freshness::Stale);
        if fresh == Freshness::Stale {
            self.counters.stale += 1;
        }
        // §7.2 (3): INTRO is `tcp-lan`-only in v1 (or a mailbox item, which
        // crossed the member's `tcp-lan` link to its hub).
        if opened.via_intro
            && !ck
                .as_ref()
                .is_none_or(|k| self.profile(k).1 == LinkKind::TcpLan)
        {
            self.counters.dropped += 1;
            self.forget_provisional(&opened);
            return;
        }
        if let Some(ck) = &ck {
            if fresh == Freshness::Fresh {
                self.attribute(ck, sender);
            }
        }
        // It sealed to our key: it holds our card (no INTRO needed to it).
        // Only a contact is remembered here; an INTRO sender only once its
        // request is filed (`on_data`) — a refused one leaves nothing but a
        // counter (§7.2 (3), §18.1).
        if self.keys.get(&sender).is_some() {
            self.heard.insert(sender);
        }
        let attributed = ck
            .as_ref()
            .and_then(|k| self.conns.get(k))
            .is_some_and(|c| c.attributed == Some(sender));
        let kind = opened.inner.kind;
        let via_mailbox = ck.is_none();
        if via_mailbox && !(kind.carries_msg_seq() || kind == Kind::Ack) {
            self.forget_provisional(&opened);
            return;
        }
        if let Some(block) = &opened.inner.ack {
            if !opened.via_intro {
                self.apply_ack(&sender, block);
            }
        }
        match kind {
            Kind::Ack => {
                if let Ok(p) = AckPayload::decode(&opened.inner.payload) {
                    self.apply_ack(&sender, &p.block);
                }
            }
            k if k.carries_msg_seq() => self.on_data(&opened),
            Kind::CustodyPut => {
                if let Some(ck) = ck.as_ref().filter(|_| attributed) {
                    self.on_custody_put(ck, &sender, &opened.inner.payload);
                }
            }
            Kind::CustodyAck => self.on_custody_ack(&sender, &opened.inner.payload),
            Kind::MailboxFetch => {
                if let Some(ck) = ck.as_ref().filter(|_| attributed) {
                    self.on_fetch(ck, &sender, &opened.inner.payload);
                }
            }
            Kind::MailboxItem => self.on_item(&sender, &opened.inner.payload),
            Kind::MailboxCommit => self.on_commit(&sender, &opened.inner.payload),
            Kind::Enroll => {
                if let Some(ck) = ck.as_ref().filter(|_| attributed) {
                    self.on_enroll(ck, &opened);
                }
            }
            Kind::EnrollOk => self.on_enroll_ok(&sender, &opened.inner.payload, ck.as_ref()),
            _ => {}
        }
        self.forget_provisional(&opened);
    }

    /// A provisional INTRO key that ended up neither pinned nor filed as a
    /// request leaves the handle's pair-key cache (zeroized).
    fn forget_provisional(&mut self, opened: &Opened) {
        let id = opened.sender.node_id;
        if opened.via_intro && self.keys.get(&id).is_none() && !self.request_pending(&id) {
            self.sealer.forget_pair_key(&opened.sender.enc_pubkey);
        }
    }

    /// §4 + §5 + §7.2 (3): a sequenced packet's content.
    fn on_data(&mut self, opened: &Opened) {
        let sender = opened.sender.node_id;
        let sb = sender.to_bytes();
        let mut held = self.request_pending(&sender);
        if self.keys.get(&sender).is_none() {
            // An INTRO from a never-contacted sender: a message request.
            if opened.inner.kind != Kind::Msg {
                return;
            }
            let Some(card) = opened.inner.card.as_deref() else {
                return;
            };
            match self.store.request_put(&sb, card) {
                Ok(RequestPut::Filed) => {
                    self.heard.insert(sender);
                    self.contact_event(sender, ContactChange::Request);
                }
                Ok(RequestPut::AlreadyPending) => {
                    self.heard.insert(sender);
                }
                Ok(RequestPut::Refused(_)) => {
                    // Dropped without an ACK and counted (§7.2 (3)).
                    self.counters.refused_requests += 1;
                    return;
                }
                Err(_) => {
                    self.counters.errors += 1;
                    return;
                }
            }
            held = true;
        }
        let seq = opened.inner.seq;
        let kind = opened.inner.kind.byte();
        let change = if held {
            MessageChange::Request
        } else {
            MessageChange::New
        };
        let mut ack = false;
        let mut strong = None;
        if let Some(f) = opened.inner.frag {
            match self.store.offer_fragment(
                &sb,
                opened.stream,
                seq.epoch,
                u64::from(seq.n),
                f.idx,
                f.count,
                &opened.inner.payload,
            ) {
                Ok(FragOffer::Stored { complete: true }) => {
                    ack = self.finish_partial(sender, opened.stream, seq, kind, change);
                }
                Ok(FragOffer::Stored { complete: false }) | Ok(FragOffer::DuplicateFragment) => {
                    ack = true;
                }
                Ok(FragOffer::Delivered(o)) => ack = o.re_ack(),
                Ok(FragOffer::StaleEpoch | FragOffer::Deferred | FragOffer::Mismatch) => {}
                Err(_) => self.counters.errors += 1,
            }
        } else {
            let msg = NewMessage {
                peer: sb,
                sender: sb,
                stream: opened.stream,
                kind,
                class: Class::from_kind(kind),
                body: &opened.inner.payload,
                blob_refs: None,
                reason: None,
                pinned: false,
            };
            match self.store.receive_message(
                &msg,
                Seq {
                    epoch: seq.epoch,
                    n: u64::from(seq.n),
                },
            ) {
                Ok((o, id)) => {
                    if let Some(id) = id {
                        self.message_event(id, sender, change);
                    }
                    ack = o.deliver() || o.re_ack();
                    if ack && opened.inner.receipt_strong {
                        strong = Some(StrongReceipt {
                            seq,
                            payload_hash: strong_hash(&opened.inner.payload),
                        });
                    }
                }
                Err(_) => self.counters.errors += 1,
            }
        }
        if ack && !held {
            let o = self.ack_owed.entry(sender).or_default();
            o.acked = Some(seq);
            if let Some(s) = strong {
                if !o.strong.iter().any(|x| x.seq == s.seq) {
                    o.strong.push(s);
                }
            }
        }
    }

    /// Reassemble a complete partial and deliver it (same FULL transaction
    /// as recording `n`, §4). Returns whether to ACK.
    fn finish_partial(
        &mut self,
        sender: NodeId,
        stream: u64,
        seq: MsgSeq,
        kind: u8,
        change: MessageChange,
    ) -> bool {
        let sb = sender.to_bytes();
        let whole = match self
            .store
            .reassemble(&sb, stream, seq.epoch, u64::from(seq.n))
        {
            Ok(Some(w)) => w,
            Ok(None) => return false,
            Err(_) => {
                self.counters.errors += 1;
                return false;
            }
        };
        let msg = NewMessage {
            peer: sb,
            sender: sb,
            stream,
            kind,
            class: Class::from_kind(kind),
            body: &whole,
            blob_refs: None,
            reason: None,
            pinned: false,
        };
        match self.store.receive_message(
            &msg,
            Seq {
                epoch: seq.epoch,
                n: u64::from(seq.n),
            },
        ) {
            Ok((o, id)) => {
                if let Some(id) = id {
                    self.message_event(id, sender, change);
                }
                o.deliver() || o.re_ack()
            }
            Err(_) => {
                self.counters.errors += 1;
                false
            }
        }
    }

    /// §6.1 at the sender: delivered messages leave the outbox; listed
    /// partials replace the SACK view (regressing evicted bits, `frag.rs`);
    /// earlier-sent outstanding fragments are presumed lost (§5, no timers).
    fn apply_ack(&mut self, sender: &NodeId, block: &AckBlock) {
        let ids: Vec<(i64, MsgSeq, u8)> = self
            .tx
            .iter()
            .filter(|(_, m)| m.peer == *sender)
            .map(|(id, m)| (*id, m.seq, m.count))
            .collect();
        let mut evidence = 0u64;
        for (id, seq, count) in ids {
            if block.acks(seq) {
                evidence = evidence.max(self.tracker.last_tick(id));
                self.delivered(id);
            } else if let Some(p) = block.partials.iter().find(|p| p.seq == seq) {
                let a = self.tracker.apply_partial(id, count, p.frag_bitmap);
                evidence = evidence.max(a.evidence);
                for idx in &a.sacked {
                    let _ = self.store.outbox_set_outstanding(id, *idx, false);
                }
                self.counters.regressed += a.regressed.len() as u64;
            }
        }
        let frags = self.frags_of(sender);
        for (id, idx) in self.tracker.infer_losses(&frags, evidence) {
            self.counters.inferred_lost += 1;
            let _ = self.store.outbox_set_outstanding(id, idx, false);
        }
        self.pump_set.insert(*sender);
    }

    // -- hub role (§9) -----------------------------------------------------

    fn on_custody_put(&mut self, ck: &ConnKey, sender: &NodeId, payload: &[u8]) {
        let Ok((dest, item)) = decode_put(payload) else {
            return;
        };
        let item = item.to_vec();
        let mut accepted = None;
        let (status, custody_seq) = if !self.settings.holds_custody() || !self.is_enrolled(sender) {
            (CustodyStatus::RejectUnenrolled, 0)
        } else if !self.is_enrolled(&dest) || Some(dest) == self.home_hub() {
            (CustodyStatus::RejectUnknownDest, 0)
        } else {
            match self.store.custody_put(dest.as_bytes(), &item) {
                Ok(CustodyPut::Accepted(s)) => {
                    accepted = Some(s);
                    (CustodyStatus::Accepted, s)
                }
                Ok(CustodyPut::Duplicate(s)) => (CustodyStatus::Duplicate, s),
                Ok(CustodyPut::RejectFull) => (CustodyStatus::RejectFull, 0),
                Err(_) => {
                    self.counters.errors += 1;
                    return;
                }
            }
        };
        let receipt = CustodyReceipt {
            status,
            custody_seq,
            pkt_ref: pkt_ref(&item),
        };
        if let Ok(bytes) =
            CustodyAck::build(&*self.sealer, receipt, &dest, false).and_then(|a| a.encode())
        {
            self.send_control(ck, sender, Kind::CustodyAck, bytes);
        }
        if accepted.is_some() {
            self.push_to(&dest);
            self.fabric_changed();
        }
    }

    /// Stream `dest`'s items on every connection attributed to it that has
    /// fetched (flow control: [`PushWindow`]).
    fn push_to(&mut self, dest: &NodeId) {
        let cks: Vec<ConnKey> = self
            .conns
            .iter()
            .filter(|(_, c)| c.attributed == Some(*dest) && c.push.active)
            .map(|(k, _)| k.clone())
            .collect();
        for ck in cks {
            self.stream_items(&ck, dest);
        }
    }

    fn stream_items(&mut self, ck: &ConnKey, dest: &NodeId) {
        let Some(c) = self.conns.get(ck) else { return };
        let (high, room) = (c.push.high, c.push.room());
        if room == 0 {
            return;
        }
        let items = match self.store.custody_fetch(dest.as_bytes(), high, room) {
            Ok(v) => v,
            Err(_) => {
                self.counters.errors += 1;
                return;
            }
        };
        for it in items {
            // Streamed only once queued; an item that cannot be queued stops
            // the stream (in order, no gaps — module doc).
            if !self.send_control_tagged(
                ck,
                dest,
                Kind::MailboxItem,
                encode_item(it.custody_seq, &it.bytes),
                Tag::Push(it.custody_seq),
            ) {
                break;
            }
            self.counters.items_pushed += 1;
            if let Some(c) = self.conns.get_mut(ck) {
                c.push.pushed(it.custody_seq);
            }
        }
    }

    fn on_fetch(&mut self, ck: &ConnKey, sender: &NodeId, payload: &[u8]) {
        if !self.settings.holds_custody() || !self.is_enrolled(sender) {
            return;
        }
        let Ok(cursor) = decode_cursor(payload) else {
            return;
        };
        if let Some(c) = self.conns.get_mut(ck) {
            c.push.fetch(cursor);
        }
        self.stream_items(ck, sender);
    }

    fn on_commit(&mut self, sender: &NodeId, payload: &[u8]) {
        if !self.settings.holds_custody() || !self.is_enrolled(sender) {
            return;
        }
        let Ok(cursor) = decode_cursor(payload) else {
            return;
        };
        if self
            .store
            .custody_commit(sender.as_bytes(), cursor)
            .is_err()
        {
            self.counters.errors += 1;
        }
        let cks: Vec<ConnKey> = self
            .conns
            .iter()
            .filter(|(_, c)| c.attributed == Some(*sender))
            .map(|(k, _)| k.clone())
            .collect();
        for ck in cks {
            if let Some(c) = self.conns.get_mut(&ck) {
                c.push.commit(cursor);
            }
            self.stream_items(&ck, sender);
        }
        self.fabric_changed();
    }

    /// §7.3 hub side: the code against the open offer (single use, lockout
    /// after 5); success pins the member `enrolled` and answers `ENROLL_OK
    /// {hub card, custody_seq_floor}`.
    fn on_enroll(&mut self, ck: &ConnKey, opened: &Opened) {
        let member = opened.sender;
        let status = if !self.settings.holds_custody() {
            EnrollStatus::Refused
        } else {
            match self.offer.as_mut() {
                None => EnrollStatus::Refused,
                Some(o) => match o.present(&member.node_id, &opened.inner.payload) {
                    Ok(_) => EnrollStatus::Ok,
                    Err(e) => EnrollStatus::from_error(&e),
                },
            }
        };
        let status = match (status, opened.inner.card.as_deref()) {
            (EnrollStatus::Ok, Some(card)) => match verify_card(card) {
                Ok(ann) if ann.node_id() == member.node_id => {
                    match self.pin(&ann, card, Trust::Enrolled) {
                        Ok(()) => {
                            // It sealed to our key: it holds our card.
                            self.heard.insert(member.node_id);
                            self.contact_event(member.node_id, ContactChange::Added);
                            self.fabric_changed();
                            EnrollStatus::Ok
                        }
                        Err(_) => EnrollStatus::Refused,
                    }
                }
                _ => EnrollStatus::Refused,
            },
            (EnrollStatus::Ok, None) => EnrollStatus::Refused,
            (s, _) => s,
        };
        let answer = if status == EnrollStatus::Ok {
            // §7.3: below what is still held for this member (module doc).
            let floor = match self.store.custody_floor(member.node_id.as_bytes()) {
                Ok(f) => f,
                Err(_) => {
                    self.counters.errors += 1;
                    0
                }
            };
            EnrollOk {
                status,
                card: Some(self.own_announce.clone()),
                custody_seq_floor: floor,
            }
        } else {
            EnrollOk::refusal(status)
        };
        if let Ok(bytes) = answer.encode() {
            self.send_control_keys(ck, &member, Kind::EnrollOk, bytes, Tag::None);
        }
    }

    // -- member side (§9) --------------------------------------------------

    fn on_custody_ack(&mut self, sender: &NodeId, payload: &[u8]) {
        if Some(*sender) != self.home_hub() {
            return;
        }
        let Ok(ack) = CustodyAck::decode(payload) else {
            return;
        };
        let Some((id, idx)) = self.custody_pending.remove(&ack.receipt.pkt_ref) else {
            return;
        };
        self.custody_inflight.remove(&(id, idx));
        let Some(count) = self.tx.get(&id).map(|m| m.count) else {
            return;
        };
        match ack.receipt.status {
            CustodyStatus::Accepted | CustodyStatus::Duplicate => {
                self.tracker.set_held(id, idx);
                if self.tracker.all_held(id, count) {
                    // §9: authenticated ACCEPTED from the home hub (a
                    // mailbox-role node) for a known member ⇒ relayed.
                    self.set_state(id, MsgState::Relayed, None);
                }
            }
            reject => {
                let reason = match reject {
                    CustodyStatus::RejectFull => Reason::MailboxFull,
                    CustodyStatus::RejectUnenrolled => Reason::NotEnrolled,
                    _ => Reason::HubNotYours,
                };
                if let Some(m) = self.tx.get_mut(&id) {
                    m.hub_rejected = true;
                }
                self.set_state(id, MsgState::Sent, Some(reason));
            }
        }
    }

    fn on_item(&mut self, sender: &NodeId, payload: &[u8]) {
        let Some(hub) = self.home_hub() else { return };
        if *sender != hub {
            return;
        }
        let Ok((seq, item)) = decode_item(payload) else {
            return;
        };
        let item = item.to_vec();
        self.counters.items_received += 1;
        self.on_sealed(None, &item);
        match self.store.advance_cursor(hub.as_bytes(), seq) {
            Ok(c) => self.commit_owed = Some(self.commit_owed.unwrap_or(0).max(c)),
            Err(_) => self.counters.errors += 1,
        }
    }

    fn on_enroll_ok(&mut self, sender: &NodeId, payload: &[u8], ck: Option<&ConnKey>) {
        if !self.enrolling.as_ref().is_some_and(|e| e.hub == *sender) {
            return;
        }
        let Ok(ok) = EnrollOk::decode(payload) else {
            return;
        };
        let Some(mut en) = self.enrolling.take() else {
            return;
        };
        let reply = en.reply.take();
        let result = if ok.status == EnrollStatus::Ok {
            match ok.card.as_deref().map(|c| (verify_card(c), c)) {
                Some((Ok(ann), card)) if ann.node_id() == *sender => {
                    match self.pin(&ann, card, Trust::Enrolled) {
                        Ok(()) => {
                            self.heard.insert(*sender);
                            self.settings.home_hub = Some(HomeHub {
                                node_id: *sender,
                                address: Some(en.address),
                            });
                            if self
                                .store
                                .advance_cursor(sender.as_bytes(), ok.custody_seq_floor)
                                .is_err()
                            {
                                self.counters.errors += 1;
                            }
                            self.settings_changed();
                            self.contact_event(*sender, ContactChange::Updated);
                            if let Some(ck) = ck {
                                self.hub_link_up(ck);
                            }
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                }
                _ => Err(FabricError::Enroll(EnrollStatus::Refused)),
            }
        } else {
            Err(FabricError::Enroll(ok.status))
        };
        if let Some(r) = reply {
            let _ = r.send(result);
        }
    }

    // -- API ---------------------------------------------------------------

    fn send(&mut self, to: NodeId, text: &str, sent_at: Option<u64>) -> Result<i64> {
        if to == self.own {
            return Err(FabricError::Invalid("cannot message this node itself"));
        }
        if text.is_empty() {
            return Err(FabricError::Invalid("empty text"));
        }
        let known = self.keys.get(&to).is_some()
            || self.pending_keys.contains(&to)
            || self.request_pending(&to);
        if !known {
            return Err(FabricError::NotFound);
        }
        let epoch = match self.store.epoch_state() {
            EpochState::Ready { epoch } => epoch,
            _ => return Err(FabricError::RestorePending),
        };
        let body = MessageBody {
            text: text.to_owned(),
            sent_at,
        }
        .encode();
        let next = self
            .store
            .peek_next_n(to.as_bytes(), STREAM_1TO1)?
            .map_or(MsgSeq::new(epoch, 1), |s| {
                MsgSeq::new(s.epoch, u32::try_from(s.n).unwrap_or(u32::MAX))
            });
        let (mtu, shim) = self.plan_mtu(&to);
        let count = plan(body.len(), mtu, shim, next).map_err(FabricError::TooLarge)?;
        let reason = self.store.store_full().then_some(Reason::StoreFull);
        let msg = NewMessage {
            peer: to.to_bytes(),
            sender: self.own.to_bytes(),
            stream: STREAM_1TO1,
            kind: Kind::Msg.byte(),
            class: Class::Human,
            body: &body,
            blob_refs: None,
            reason: reason.map(Reason::code),
            pinned: false,
        };
        let (id, seq) = self.store.queue_own_message(&msg)?;
        for idx in 0..count {
            self.store.outbox_put(id, idx, count, to.as_bytes(), None)?;
        }
        let n = u32::try_from(seq.n).map_err(|_| FabricError::Invalid("n out of range"))?;
        self.tx.insert(
            id,
            TxMsg {
                peer: to,
                seq: MsgSeq::new(seq.epoch, n),
                count,
                kind: Kind::Msg.byte(),
                state: MsgState::Queued,
                reason,
                cancelled: false,
                hub_rejected: false,
            },
        );
        self.message_event(id, to, MessageChange::New);
        self.pump_set.insert(to);
        Ok(id)
    }

    fn cancel(&mut self, id: i64) -> Result<MessageView> {
        let m = self.tx.get_mut(&id).ok_or(FabricError::NotFound)?;
        if m.state != MsgState::Queued || self.tracker.last_tick(id) != 0 {
            return Err(FabricError::NotRetryable);
        }
        m.cancelled = true;
        let peer = m.peer;
        self.store
            .set_message_state(id, MsgState::Queued, Some(Reason::Cancelled.code()))?;
        if let Some(m) = self.tx.get_mut(&id) {
            m.reason = Some(Reason::Cancelled);
        }
        self.message_event(id, peer, MessageChange::State);
        self.pump_set.insert(peer);
        self.message(id)
    }

    fn retry(&mut self, id: i64) -> Result<MessageView> {
        let Some(m) = self.tx.get_mut(&id) else {
            return match self.store.message(id)? {
                Some(_) => Err(FabricError::NotRetryable),
                None => Err(FabricError::NotFound),
            };
        };
        if !matches!(m.state, MsgState::Queued | MsgState::Sent) {
            return Err(FabricError::NotRetryable);
        }
        m.hub_rejected = false;
        let (peer, count) = (m.peer, m.count);
        for idx in 0..count {
            if self.tracker.get(id, idx).outstanding {
                self.tracker.set_outstanding(id, idx, false);
                let _ = self.store.outbox_set_outstanding(id, idx, false);
            }
        }
        self.try_mint();
        self.pump_set.insert(peer);
        self.message(id)
    }

    fn retry_all(&mut self) {
        self.try_mint();
        // §9: members fetch on fabric events too (and re-ask for cards).
        if let Some(hub) = self.home_hub() {
            if let Some(ck) = self.sendable_conn(&hub) {
                self.hub_link_up(&ck);
            }
        }
        let peers: BTreeSet<NodeId> = self.tx.values().map(|m| m.peer).collect();
        for p in &peers {
            self.presume_lost(p);
        }
        for m in self.tx.values_mut() {
            m.hub_rejected = false;
        }
        self.custody_inflight.clear();
        self.custody_pending.clear();
        self.pump_set.extend(peers);
    }

    fn message(&self, id: i64) -> Result<MessageView> {
        let row = self.store.message(id)?.ok_or(FabricError::NotFound)?;
        Ok(message_view(&row))
    }

    fn thread_peers(&self) -> Result<BTreeSet<NodeId>> {
        let mut peers: BTreeSet<NodeId> = self
            .store
            .contacts()?
            .iter()
            .map(|c| NodeId::from(c.node_id))
            .collect();
        peers.extend(self.store.requests()?.iter().map(|(n, _)| NodeId::from(*n)));
        peers.extend(self.pending_keys.iter().copied());
        peers.extend(self.tx.values().map(|m| m.peer));
        Ok(peers)
    }

    /// Thread list: per peer the newest message and SQL counts (never a
    /// whole history in memory).
    fn threads(&self) -> Result<Vec<ThreadView>> {
        let mut out = Vec::new();
        for peer in self.thread_peers()? {
            let Some(last) = self.store.messages_newest(peer.as_bytes(), 1)?.pop() else {
                continue;
            };
            let read = self.read_marks.get(&peer).copied().unwrap_or(0);
            let c = self.store.thread_counts(peer.as_bytes(), read)?;
            let alias = self.store.contact(peer.as_bytes())?.and_then(|c| c.alias);
            out.push(ThreadView {
                peer,
                alias,
                request: self.request_pending(&peer),
                unread: c.unread,
                last: Some(message_view(&last)),
                states: StateCounts {
                    queued: c.queued,
                    sent: c.sent,
                    relayed: c.relayed,
                    delivered: c.delivered,
                },
            });
        }
        out.sort_by_key(|t| std::cmp::Reverse(t.last.as_ref().map_or(0, |m| m.local_seq)));
        Ok(out)
    }

    /// The newest `limit` messages (one more is read to answer `more`).
    fn thread(&self, peer: NodeId, limit: usize) -> Result<ThreadPage> {
        let limit = limit.max(1);
        let mut rows = self
            .store
            .messages_newest(peer.as_bytes(), limit.saturating_add(1))?;
        if rows.is_empty() && !self.thread_peers()?.contains(&peer) {
            return Err(FabricError::NotFound);
        }
        let more = rows.len() > limit;
        if more {
            rows.remove(0);
        }
        Ok(ThreadPage {
            peer,
            request: self.request_pending(&peer),
            messages: rows.iter().map(message_view).collect(),
            more,
        })
    }

    fn contacts(&self) -> Result<Vec<ContactView>> {
        let mut out: Vec<ContactView> = Vec::new();
        let mut seen: HashSet<NodeId> = HashSet::new();
        let requests: HashMap<[u8; 32], Vec<u8>> = self.store.requests()?.into_iter().collect();
        for row in self.store.contacts()? {
            let id = NodeId::from(row.node_id);
            seen.insert(id);
            out.push(view_of_row(&row, requests.contains_key(&row.node_id)));
        }
        for (node, announce) in &requests {
            let id = NodeId::from(*node);
            if seen.insert(id) {
                out.push(request_view(id, announce));
            }
        }
        for id in &self.pending_keys {
            if seen.insert(*id) {
                out.push(pending_view(*id));
            }
        }
        Ok(out)
    }

    fn contact_view(&self, id: &NodeId) -> Result<ContactView> {
        let request = self.request_pending(id);
        if let Some(row) = self.store.contact(id.as_bytes())? {
            return Ok(view_of_row(&row, request));
        }
        if request {
            if let Some((_, a)) = self
                .store
                .requests()?
                .into_iter()
                .find(|(n, _)| n == id.as_bytes())
            {
                return Ok(request_view(*id, &a));
            }
        }
        if self.pending_keys.contains(id) {
            return Ok(pending_view(*id));
        }
        Err(FabricError::NotFound)
    }

    fn add_card(&mut self, text: &str) -> Result<ContactView> {
        let bytes = parse_card_text(text)?;
        let ann = verify_card(&bytes)?;
        let id = ann.node_id();
        if id == self.own {
            return Err(CardError::OwnCard.into());
        }
        let existing = self.store.contact(id.as_bytes())?;
        match classify_announce(&self.own, existing.as_ref(), &ann) {
            AnnounceVerdict::Conflict => {
                self.counters.conflicts += 1;
                return Err(FabricError::KeyConflict);
            }
            AnnounceVerdict::Stale => {}
            AnnounceVerdict::Refresh => {
                if let Some(c) = &existing {
                    self.refresh_contact(c, &ann, &bytes);
                }
            }
            AnnounceVerdict::Unknown => {
                self.pin(&ann, &bytes, Trust::Oob)?;
                self.contact_event(id, ContactChange::Added);
            }
            AnnounceVerdict::Own => return Err(CardError::OwnCard.into()),
        }
        if self.pending_clear(&id) {
            self.contact_event(id, ContactChange::Updated);
        }
        if self.request_pending(&id) {
            // Scanning the requester's card out of band accepts the request.
            self.accept(id)?;
        }
        self.rebuild_keys();
        self.pump_set.insert(id);
        self.contact_view(&id)
    }

    fn add_node_id(&mut self, text: &str) -> Result<ContactView> {
        let id = parse_node_id(text)?;
        if id == self.own {
            return Err(CardError::OwnCard.into());
        }
        if self.store.contact(id.as_bytes())?.is_some() || self.request_pending(&id) {
            return self.contact_view(&id);
        }
        // §7.2 (2): "the node stores the NodeId" (FULL commit) before it is
        // listed or asked for.
        let stored = self.store.pending_node_id_put(id.as_bytes())?;
        if self.pending_keys.insert(id) || stored {
            self.contact_event(id, ContactChange::Added);
            if let Some(hub) = self.home_hub() {
                if let Some(ck) = self.sendable_conn(&hub) {
                    self.outq.push(Out {
                        ck,
                        frame: Frame::card_request(&id),
                        tag: Tag::None,
                    });
                }
            }
        }
        self.contact_view(&id)
    }

    /// Accept a pending request: the contact is created (`tofu`) and an ACK
    /// for exactly the stored receive state goes out (never retroactively
    /// more than what is shown).
    fn accept(&mut self, id: NodeId) -> Result<()> {
        let reqs = self.store.requests()?;
        let Some((_, announce)) = reqs.into_iter().find(|(n, _)| n == id.as_bytes()) else {
            return Err(FabricError::Invalid("no pending request"));
        };
        let ann = verify_card(&announce)?;
        if ann.node_id() != id {
            return Err(FabricError::Invalid("request card mismatch"));
        }
        self.pin(&ann, &announce, Trust::Tofu)?;
        self.store.request_remove(id.as_bytes())?;
        self.ack_owed.entry(id).or_default();
        self.pump_set.insert(id);
        self.contact_event(id, ContactChange::Updated);
        Ok(())
    }

    fn patch_contact(&mut self, id: NodeId, patch: ContactPatch) -> Result<ContactView> {
        if patch.accept {
            self.accept(id)?;
        }
        if let Some(alias) = &patch.alias {
            if alias.as_ref().is_some_and(|a| a.len() > 64) {
                return Err(FabricError::Invalid("alias longer than 64 bytes"));
            }
            let mut row = self
                .store
                .contact(id.as_bytes())?
                .ok_or(FabricError::NotFound)?;
            row.alias = alias.clone();
            self.store.contact_upsert(&row)?;
            self.contact_event(id, ContactChange::Updated);
        }
        if let Some(p) = patch.pinned {
            match self.store.contact_set_pinned(id.as_bytes(), p) {
                Ok(()) => self.contact_event(id, ContactChange::Updated),
                Err(StoreError::NotFound) => return Err(FabricError::NotFound),
                Err(e) => return Err(e.into()),
            }
        }
        if let Some(seq) = patch.read_seq {
            let m = self.read_marks.entry(id).or_insert(0);
            *m = (*m).max(seq);
        }
        self.contact_view(&id)
    }

    fn forget(&mut self, id: NodeId) -> Result<()> {
        let mut found = false;
        if let Some(row) = self.store.contact(id.as_bytes())? {
            self.store.contact_forget(id.as_bytes())?;
            self.sealer.forget_pair_key(&row.enc_pubkey);
            found = true;
        }
        if self.request_pending(&id) {
            if let Some((_, a)) = self
                .store
                .requests()?
                .into_iter()
                .find(|(n, _)| n == id.as_bytes())
            {
                if let Ok(ann) = Announce::parse(&a) {
                    self.sealer.forget_pair_key(ann.enc_pubkey());
                }
            }
            self.store.request_remove(id.as_bytes())?;
            found = true;
        }
        found |= self.store.pending_node_id_remove(id.as_bytes())?;
        found |= self.pending_keys.remove(&id);
        if !found {
            return Err(FabricError::NotFound);
        }
        if Some(id) == self.home_hub() {
            self.leave_hub();
        }
        if self.enrolling.as_ref().is_some_and(|e| e.hub == id) {
            if let Some(mut e) = self.enrolling.take() {
                if let Some(r) = e.reply.take() {
                    let _ = r.send(Err(FabricError::Invalid("the hub was forgotten")));
                }
            }
        }
        self.rebuild_keys();
        self.contact_event(id, ContactChange::Removed);
        Ok(())
    }

    /// Forgetting the home hub ends the membership here: no home hub in the
    /// settings (persisted by the daemon), its dial released, nothing owed
    /// to it; queued messages wait for a direct path.
    fn leave_hub(&mut self) {
        let Some(h) = self.settings.home_hub.take() else {
            return;
        };
        if let (Some(addr), Some(d)) = (h.address, self.dialer()) {
            d.release(addr);
        }
        self.commit_owed = None;
        self.custody_inflight.clear();
        self.custody_pending.clear();
        for m in self.tx.values_mut() {
            m.hub_rejected = false;
        }
        self.pump_all();
        self.settings_changed();
        self.fabric_changed();
    }

    /// The `node1…` contact's card arrived (or it was scanned): it leaves
    /// the pending set, in memory and in the store (the in-memory set holds
    /// every stored row after `recover`). Returns whether it was pending.
    fn pending_clear(&mut self, id: &NodeId) -> bool {
        if !self.pending_keys.remove(id) {
            return false;
        }
        if self.store.pending_node_id_remove(id.as_bytes()).is_err() {
            self.counters.errors += 1;
        }
        true
    }

    fn status(&self) -> Result<FabricStatus> {
        let links = self
            .links
            .iter()
            .enumerate()
            .map(|(i, s)| LinkView {
                id: s.link.id(),
                kind: s.link.kind(),
                state: s.link.state(),
                peers: self
                    .conns
                    .iter()
                    .filter(|((l, _), c)| *l == i && c.attributed.is_some())
                    .count(),
                profile: s.link.profile(),
            })
            .collect();
        let hub = self.settings.home_hub;
        let members = if self.settings.holds_custody() {
            self.store
                .contacts()?
                .iter()
                .filter(|c| {
                    c.trust == Trust::Enrolled && Some(NodeId::from(c.node_id)) != self.home_hub()
                })
                .count() as u64
        } else {
            0
        };
        let (epoch, restore_pending) = match self.store.epoch_state() {
            EpochState::Ready { epoch } => (Some(epoch), false),
            EpochState::MintRequired(_) | EpochState::RestorePending => (None, true),
        };
        Ok(FabricStatus {
            links,
            hub: HubView {
                enrolled: hub.is_some(),
                reachable: hub.is_some_and(|h| self.sendable_conn(&h.node_id).is_some()),
                cursor: match hub {
                    Some(h) => self.store.cursor(h.node_id.as_bytes())?,
                    None => 0,
                },
                node_id: hub.map(|h| h.node_id),
                address: hub.and_then(|h| h.address),
                members,
                offer_active: self
                    .offer
                    .as_ref()
                    .is_some_and(|o| o.state() == OfferState::Open),
            },
            custody: self.store.custody_stats()?,
            store: StoreView {
                used: self.store.live_bytes()?,
                watermark: self.store.watermarks()?.high,
                store_full: self.store.store_full(),
                epoch,
                restore_pending,
            },
            settings: self.settings.clone(),
            counters: self.counters,
        })
    }

    fn enroll(
        &mut self,
        address: SocketAddr,
        code: &str,
        card: Option<&str>,
        reply: oneshot::Sender<Result<()>>,
    ) -> Result<()> {
        let code = normalize_code(code)?;
        let hub = match card {
            Some(text) => {
                let bytes = parse_card_text(text)?;
                let ann = verify_card(&bytes)?;
                if ann.node_id() == self.own {
                    return Err(CardError::OwnCard.into());
                }
                self.pin(&ann, &bytes, Trust::Oob)?;
                self.contact_event(ann.node_id(), ContactChange::Added);
                ann.node_id()
            }
            None => match self.settings.home_hub {
                Some(h) => h.node_id,
                None => return Err(FabricError::Invalid("the hub's card is needed")),
            },
        };
        if let Some(mut old) = self.enrolling.take() {
            if let Some(r) = old.reply.take() {
                let _ = r.send(Err(FabricError::Invalid("superseded by a new enrollment")));
            }
        }
        self.enrolling = Some(Enrolling {
            hub,
            code,
            address,
            reply: Some(reply),
        });
        match self.sendable_conn(&hub) {
            Some(ck) => self.send_enroll(&ck),
            None => match self.dialer() {
                Some(d) => d.maintain(address, Some(hub)),
                None => {
                    if let Some(mut e) = self.enrolling.take() {
                        if let Some(r) = e.reply.take() {
                            let _ = r.send(Err(FabricError::NoLink));
                        }
                    }
                }
            },
        }
        Ok(())
    }

    fn offer(&mut self, address: Option<SocketAddr>) -> Result<OfferView> {
        if !self.settings.holds_custody() {
            return Err(FabricError::Invalid("this node is not a hub"));
        }
        let (code, secret) = mint_code(&*self.sealer)?;
        let offer = EnrollOffer::new(code, secret);
        let card = card_text(&self.own_announce);
        let bundle = address.map(|a| {
            HubBundle {
                card: card.clone(),
                code: String::from_utf8_lossy(&code).into_owned(),
                address: a.to_string(),
            }
            .text()
        });
        let view = OfferView {
            code: offer.display(),
            card,
            bundle,
            address,
        };
        self.offer = Some(offer);
        self.fabric_changed();
        Ok(view)
    }

    fn dismiss_offer(&mut self) -> bool {
        let open = self
            .offer
            .as_ref()
            .is_some_and(|o| o.state() == OfferState::Open);
        if let Some(o) = self.offer.as_mut() {
            o.dismiss();
        }
        self.offer = None;
        self.fabric_changed();
        open
    }

    fn patch_settings(&mut self, patch: SettingsPatch) -> Result<FabricSettings> {
        for (class, q) in &patch.quotas {
            self.store.set_quota(*class, *q)?;
        }
        if let Some(q) = patch.request_quota {
            self.store.set_request_quota(q)?;
        }
        if let Some(v) = patch.hub_role {
            self.settings.hub_role = v;
        }
        if let Some(v) = patch.listen_lan {
            self.settings.listen_lan = v;
        }
        if let Some(v) = patch.custody_opt_in {
            self.settings.custody_opt_in = v;
        }
        self.settings_changed();
        self.fabric_changed();
        Ok(self.settings.clone())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Sealed length of `inner` without sealing (outer 64 + inner + auth).
fn sealed_len(inner: &Inner) -> Option<usize> {
    inner
        .encode_before_auth()
        .ok()
        .map(|b| OUTER_OVERHEAD + b.len() + inner.auth_len())
}

fn msg_seq(row: &MessageRow) -> MsgSeq {
    MsgSeq::new(row.seq.epoch, u32::try_from(row.seq.n).unwrap_or(u32::MAX))
}

fn message_view(row: &MessageRow) -> MessageView {
    let body = row.body.clone().unwrap_or_default();
    let (text, sent_at) = if row.kind == Kind::Msg.byte() {
        match MessageBody::decode(&body) {
            Ok(b) => (Some(b.text), b.sent_at),
            Err(_) => (None, None),
        }
    } else {
        (None, None)
    };
    MessageView {
        id: row.id,
        peer: NodeId::from(row.peer),
        dir: row.dir,
        kind: row.kind,
        text,
        tombstone: body.is_empty(),
        state: row.state,
        reason: row.reason.as_deref().and_then(Reason::from_code),
        local_seq: row.local_seq,
        received_seq: row.received_local_seq,
        sent_at_claim: if row.dir == Dir::In { sent_at } else { None },
        stream: row.stream,
        seq: msg_seq(row),
    }
}

fn view_of_row(row: &ContactRow, request: bool) -> ContactView {
    ContactView {
        node_id: NodeId::from(row.node_id),
        alias: row.alias.clone(),
        trust: row.trust,
        request,
        pinned: row.pinned,
        has_key: true,
        roles: row.roles,
        home_hub: row.home_hub.clone(),
    }
}

fn request_view(id: NodeId, announce: &[u8]) -> ContactView {
    let (roles, home_hub) = match Announce::parse(announce) {
        Ok(a) => {
            let app = crate::contacts::AppData::parse(a.appdata());
            (app.roles, app.home_hub.map(|h| h.to_vec()))
        }
        Err(_) => (0, None),
    };
    ContactView {
        node_id: id,
        alias: None,
        trust: Trust::Tofu,
        request: true,
        pinned: false,
        has_key: true,
        roles,
        home_hub,
    }
}

fn pending_view(id: NodeId) -> ContactView {
    ContactView {
        node_id: id,
        alias: None,
        trust: Trust::Oob,
        request: false,
        pinned: false,
        has_key: false,
        roles: 0,
        home_hub: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::MIN_PACKET_LEN;
    use crate::keys::StoreKey;
    use crate::link::lossy::{self, EndSpec, LossyConfig};
    use crate::store::OpenParams;
    use crate::test_double::TestSealer;

    struct TestNode {
        sealer: Arc<TestSealer>,
        f: Fabric,
        _dir: tempfile::TempDir,
    }

    fn test_node(label: &str, settings: FabricSettings) -> TestNode {
        let dir = tempfile::tempdir().expect("tempdir");
        let sealer = Arc::new(TestSealer::from_label(label));
        let (store, decision) = Store::open(
            dir.path(),
            &StoreKey::from_bytes([0x5e; 32]),
            &OpenParams {
                announce_last: sealer.announce_last(),
                restore_floor_set: false,
            },
        )
        .expect("store");
        let f = Fabric::start(sealer.clone(), store, decision, settings).expect("fabric");
        TestNode {
            sealer,
            f,
            _dir: dir,
        }
    }

    /// Pin `who`'s card on `node` at `trust`.
    async fn pin_on(node: &TestNode, who: &TestSealer, trust: Trust) {
        let card = who.own_announce();
        node.f
            .call(move |c| {
                let ann = verify_card(&card).expect("card");
                c.pin(&ann, &card, trust).expect("pinned");
            })
            .await
            .expect("actor");
    }

    #[tokio::test]
    async fn a_refused_intro_leaves_no_heard_entry() {
        let bob = test_node("bob", FabricSettings::default());
        let carol = TestSealer::from_label("carol");
        let intro = |n: u32| {
            let mut inner = Inner::new(
                &carol.node_id(),
                Kind::Msg,
                MsgSeq::new(1, n),
                MessageBody::text("hi").encode(),
            );
            inner.card = Some(carol.own_announce());
            seal_envelope(&carol, &bob.sealer.keys(), Priority::Interactive, &inner)
                .expect("sealed")
        };
        let (first, second) = (intro(1), intro(2));
        let id = carol.node_id();
        let (refused_heard, refused, filed_heard) = bob
            .f
            .call(move |c| {
                c.store.set_request_quota(Some(0)).expect("quota");
                c.on_sealed(None, &first);
                let after_refusal = c.heard.contains(&id);
                let refused = c.counters.refused_requests;
                c.store.set_request_quota(None).expect("quota");
                c.on_sealed(None, &second);
                (after_refusal, refused, c.heard.contains(&id))
            })
            .await
            .expect("actor");
        assert_eq!(refused, 1);
        assert!(
            !refused_heard,
            "a refused INTRO leaves nothing but a counter"
        );
        assert!(filed_heard, "a filed request's sender holds our card");
    }

    #[tokio::test]
    async fn a_commit_is_kept_until_queued_and_resent_beside_the_fetch() {
        let hub = TestSealer::from_label("hub");
        let settings = FabricSettings {
            home_hub: Some(HomeHub {
                node_id: hub.node_id(),
                address: None,
            }),
            ..FabricSettings::default()
        };
        let m = test_node("member", settings);
        pin_on(&m, &hub, Trust::Enrolled).await;
        let hub_id = hub.node_id();
        let ck: ConnKey = (7, PeerAddr::Opaque(b"hub".to_vec()));
        let (kept, frames, owed_after) =
            m.f.call(move |c| {
                // The hub socket died just before the flush: still owed.
                c.commit_owed = Some(5);
                c.flush();
                let kept = c.commit_owed;
                // Link-up: the stored cursor is committed beside the fetch.
                c.store
                    .advance_cursor(hub_id.as_bytes(), 9)
                    .expect("cursor");
                c.conns.insert(
                    ck.clone(),
                    Conn {
                        attributed: Some(hub_id),
                        ..Conn::default()
                    },
                );
                c.outq.clear();
                c.hub_link_up(&ck);
                c.flush();
                let frames: Vec<Vec<u8>> = std::mem::take(&mut c.outq)
                    .into_iter()
                    .filter(|o| o.ck == ck)
                    .map(|o| o.frame.bytes)
                    .collect();
                c.conns.remove(&ck);
                (kept, frames, c.commit_owed)
            })
            .await
            .expect("actor");
        assert_eq!(kept, Some(5), "an unsent commit stays owed");
        assert_eq!(owed_after, None);
        let mk = m.sealer.keys();
        let seen: Vec<(Kind, u64)> = frames
            .iter()
            .map(|b| {
                let o = open_envelope(&hub, b, &|_| vec![mk]).expect("opens");
                (
                    o.inner.kind,
                    decode_cursor(&o.inner.payload).expect("cursor"),
                )
            })
            .collect();
        assert_eq!(
            seen,
            vec![(Kind::MailboxFetch, 9), (Kind::MailboxCommit, 9)]
        );
    }

    #[tokio::test]
    async fn a_push_counts_only_once_queued_and_rolls_back_on_a_send_error() {
        let settings = FabricSettings {
            hub_role: true,
            ..FabricSettings::default()
        };
        let h = test_node("hub", settings);
        let (m1, m2) = (TestSealer::from_label("m1"), TestSealer::from_label("m2"));
        pin_on(&h, &m1, Trust::Enrolled).await;
        pin_on(&h, &m2, Trust::Enrolled).await;
        // A 400 B hub ↔ member path (an L6-sized link).
        let (far_events, _far_rx) = mpsc::unbounded_channel();
        let small = LossyConfig {
            profile: PathProfile {
                mtu_bytes: 400,
                ..PathProfile::unknown()
            },
            ..LossyConfig::default()
        };
        let (end, _far) = lossy::pair(
            EndSpec {
                id: LinkId::new("small"),
                announce: h.sealer.own_announce(),
                events: h.f.link_events(),
                cfg: small,
            },
            EndSpec {
                id: LinkId::new("far"),
                announce: m1.own_announce(),
                events: far_events,
                cfg: LossyConfig::default(),
            },
        );
        let idx = h.f.add_link(end, None).await.expect("link");
        let (id1, id2) = (m1.node_id(), m2.node_id());
        let (stuck, streamed, rolled) =
            h.f.call(move |c| {
                let window = |c: &Core, ck: &ConnKey| {
                    let w = &c.conns[ck].push;
                    (w.high, w.inflight.len())
                };
                // m1: an item too large for this path, then one that fits.
                let ck1: ConnKey = (idx, PeerAddr::Opaque(b"m1".to_vec()));
                c.store
                    .custody_put(id1.as_bytes(), &[1u8; 400])
                    .expect("put");
                c.store
                    .custody_put(id1.as_bytes(), &[2u8; MIN_PACKET_LEN])
                    .expect("put");
                let conn = c.conns.entry(ck1.clone()).or_default();
                conn.attributed = Some(id1);
                conn.push.fetch(0);
                c.outq.clear();
                c.stream_items(&ck1, &id1);
                let stuck = (window(c, &ck1), c.outq.len(), c.counters.items_pushed);
                // m2: two items that fit; the first frame's write fails.
                let ck2: ConnKey = (idx, PeerAddr::Opaque(b"m2".to_vec()));
                c.store
                    .custody_put(id2.as_bytes(), &[3u8; MIN_PACKET_LEN])
                    .expect("put");
                c.store
                    .custody_put(id2.as_bytes(), &[4u8; MIN_PACKET_LEN])
                    .expect("put");
                let conn = c.conns.entry(ck2.clone()).or_default();
                conn.attributed = Some(id2);
                conn.push.fetch(0);
                c.stream_items(&ck2, &id2);
                let streamed = (window(c, &ck2), c.outq.len());
                let e = std::io::Error::new(std::io::ErrorKind::WouldBlock, "write queue full");
                c.on_sent(&ck2, Tag::Push(3), Err(LinkError::Io(e)));
                let rolled = window(c, &ck2);
                c.outq.clear();
                c.conns.remove(&ck1);
                c.conns.remove(&ck2);
                (stuck, streamed, rolled)
            })
            .await
            .expect("actor");
        assert_eq!(
            stuck,
            ((0, 0), 0, 0),
            "nothing counts as streamed, and nothing overtakes the stuck item"
        );
        assert_eq!(streamed, ((4, 2), 2));
        assert_eq!(rolled, (2, 0), "the stream restarts just below item 3");
    }

    #[test]
    fn sealed_len_matches_the_size_table() {
        let id = NodeId::from([7u8; 32]);
        let inner = Inner::new(&id, Kind::Msg, MsgSeq::new(1, 1), vec![0; 72]);
        // §3 table: 160 B frame, Tier 0 payload 72 B with a 2 B msg_seq.
        assert_eq!(sealed_len(&inner), Some(160));
        let ctl = Inner::new(&id, Kind::MailboxFetch, MsgSeq::CONTROL, vec![0]);
        assert_eq!(sealed_len(&ctl), Some(64 + 4 + 1 + 1 + 2 + 1 + 16));
    }

    #[test]
    fn conn_send_rule() {
        let x = NodeId::from([1u8; 32]);
        let mut c = Conn {
            claimed: Some(x),
            ..Conn::default()
        };
        assert!(!c.sendable_to(&x), "an inbound claim authenticates nothing");
        c.outbound = true;
        assert!(c.sendable_to(&x), "our own dial to its configured address");
        let d = Conn {
            attributed: Some(x),
            ..Conn::default()
        };
        assert!(d.sendable_to(&x));
    }
}
