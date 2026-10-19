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
//! explicit [`Fabric::retry`] (the UI's per-message retry) and
//! [`Fabric::retry_all`] (an operator's retry-all). The daemon never calls
//! `retry_all` on its own: it presumes every outstanding fragment lost, so an
//! automatic call on a link event re-sent a 16-fragment transfer in flight
//! (the lock's three-node wire count read 31 frames); members re-fetch on
//! link-up by themselves. Keepalive is the adapter's transport detail.
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
//! - A `MAILBOX_FETCH` is always answered (L5 m3, as built at the lock):
//!   the items above the cursor, or — when the hub holds none for the
//!   member — one item-less `MAILBOX_ITEM {custody_seq = cursor}`. Only an
//!   enrolled member's fetch on a connection attributed to it is answered,
//!   so the answer is the member's evidence that its hub serves it there:
//!   [`HubView::reachable`] is true only while a connection is attributed
//!   to the home hub (a fresh, verified hub packet arrived on it since it
//!   came up), never from this node's own dialed socket. A member that
//!   predates the marker drops it as a malformed item, harmlessly.
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
//!   own (no timers here) — the route that calls it bounds the wait, and
//!   dropping the wait (that bound elapsed, or its client went away)
//!   abandons the attempt.
//! - An enrollment attempt that ends without `ENROLL_OK` (§7.3) — refused
//!   (any `ENROLL_OK` status but OK, or an answer that does not verify or
//!   pin), no link, superseded by a newer attempt, its hub forgotten, or
//!   abandoned — leaves nothing behind on the member: the dial it started
//!   is released unless the home hub or a configured peer
//!   ([`Fabric::dial_peer`]) holds that address, and a hub card it pinned
//!   is forgotten unless the contact existed before the attempt (or became
//!   the home hub, or was used or edited since: an alias, a pin, a stronger
//!   trust, a message to it). A refused member therefore neither redials
//!   the hub (which would keep counting it as an attributed peer) nor keeps
//!   a hub card it never had before.
//! - All handles dropped does not stop the actor: call
//!   [`Fabric::shutdown`] (clean close) or [`Fabric::abort`].
//!
//! ### Foreign calls (Layer 6 `60-transport/README.md` §3, §8, §9, §10)
//!
//! The bridge core and the daemon's routes reach the foreign tables (store
//! schema v3, [`crate::store::foreign`]) only through the typed
//! `Fabric::foreign_*` calls, never through [`Fabric::with_store`] (which
//! emits no event). Each call runs in the actor between frame pumps as
//! **one** synchronous SQLCipher transaction, like `send` and `threads`:
//!
//! - [`Fabric::foreign_ingest`] takes at most [`MAX_FOREIGN_INGEST`] rows
//!   (256; refused above, before the actor is asked). The bridge core
//!   awaits each call, so an adapter tick waits for its batch to commit —
//!   that wait is the back-pressure on the actor's input channel, and a
//!   1,000-row burst is four jobs interleaved with 111 frames, never one
//!   long stall (`tests/foreign_burst.rs` measures the 111 latency).
//! - Every write emits [`FabricEvent::Foreign`] beside `Message`, `Contact`,
//!   `Fabric` and `Settings`, **coalesced per tick** (§10): one event per
//!   own row, DM row or pinned-channel row (`id` set), and one per
//!   **conversation** for inbound traffic of an unpinned channel (`id`
//!   `None`; the daemon carries the conversation's highest `seq` in the
//!   frame), so 50 channel rows in one batch are one event. As for every
//!   API job, the events of a call are emitted before the next call's
//!   reply (`Core::handle`), so "per tick" is at most "per call".
//! - Nothing here touches 111 message numbering, the outbox or the L2
//!   window: foreign rows only draw `local_seq` from the store-wide counter
//!   (one order, §3).
//! - Readings: a cancel deletes an own row still `queued` that was never
//!   handed to a carrier (no pending handoff mark, no native id; the
//!   store's `foreign_cancel`); the handoff mark is set only through
//!   [`Fabric::foreign_handoff_begin`] / [`Fabric::foreign_accepted`];
//!   edge changes emit no event (the `bridge` SSE snapshot is the bridge
//!   core's), except a disable or a delete, which drop the destination's
//!   never-handed copies (a `Conversation` event for it).

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
    Class, ContactRow, CustodyPut, Dir, EdgeConsent, EdgeCounter, EdgeNew, EdgeOversize, EdgeRow,
    EpochDecision, EpochState, ForeignConversationRow, ForeignDir, ForeignInsert, ForeignKind,
    ForeignMessageRow, ForeignSend, ForeignState, FragOffer, GroupPart, MessageRow, MetaValue,
    MsgState, NewMessage, RequestPut, SendGroupRow, Seq, StateChange, Store, StoreError, Trust,
    MAX_FOREIGN_BATCH,
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

/// What changed about a foreign row or conversation (Layer 6 §10 SSE
/// `network_message.change`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignChange {
    /// Rows new to the client: inbound, a queued own row, a relay copy.
    New,
    /// One row's state, evidence, reason or `meta` changed.
    State,
    /// The conversation itself changed: new to this run, a new title, a
    /// pin, its read mark, or queued copies dropped with an edge.
    Conversation,
}

/// Events the daemon subscribes to (SSE `message`, `contact`, `fabric`,
/// `network_message`), coalesced per message / contact per tick.
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
    /// A foreign row or conversation changed (Layer 6 §3, §10), coalesced
    /// per row per tick for own rows, DMs, groups and pinned channels, and
    /// per **conversation** per tick (`id: None`) for inbound traffic of an
    /// unpinned channel (module doc, "Foreign calls").
    Foreign {
        /// `foreign_conversations.id`.
        conv: i64,
        /// `foreign_messages.id`; `None` on a coalesced channel event and on
        /// a conversation change.
        id: Option<i64>,
        /// First change since the previous frame.
        change: ForeignChange,
    },
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
    /// A connection to the home hub is up that this node may send on (it
    /// dialed the hub's address and the hub's announce answered, or the
    /// connection is attributed). Hello and announce authenticate nothing
    /// (§8.1): this alone promises no delivery.
    pub connected: bool,
    /// The home hub answered on the current connection: a connection is
    /// attributed to it (a `pair_mac`-verified packet from the hub that the
    /// L2 window judged fresh arrived on it since it came up — `ENROLL_OK`,
    /// a `MAILBOX_ITEM`, a `CUSTODY_ACK`, any sealed packet of the hub
    /// node). Never self-judged from this node's own sendable connection.
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
            peer_dials: HashSet::new(),
            read_marks: HashMap::new(),
            counters: Counters::default(),
            events: events.clone(),
            pending_events: BTreeMap::new(),
            foreign_convs: HashMap::new(),
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

    /// Re-queue an own `queued` or `sent` message now (§15 retry): its
    /// outstanding fragments are presumed lost and its in-flight
    /// `CUSTODY_PUT`s forgotten, so the next pump re-sends both.
    pub async fn retry(&self, id: i64) -> Result<MessageView> {
        self.call(move |c| c.retry(id)).await?
    }

    /// A fabric event: retry a pending mint, fetch from the home hub,
    /// presume every outstanding fragment lost, clear custody rejects and
    /// in-flight `CUSTODY_PUT`s, and pump every peer. An operator action:
    /// never called automatically (module doc, "No timers").
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
    /// contact. Resolves when `ENROLL_OK` arrives: `Ok` once enrolled, else
    /// the typed refusal (`FabricError::Enroll(status)`, `NoLink`, …). An
    /// attempt that ends without `ENROLL_OK` — including this future being
    /// dropped before the answer — leaves nothing behind (module doc).
    pub async fn enroll(&self, address: SocketAddr, code: &str, card: Option<&str>) -> Result<()> {
        let code = code.to_owned();
        let card = card.map(str::to_owned);
        let (tx, rx) = oneshot::channel();
        // Armed before the attempt opens: dropping this future at any await
        // point abandons it.
        let mut wait = EnrollWait {
            rx: Some(rx),
            input: self.inner.input.clone(),
        };
        if let Err(e) = self
            .call(move |c| c.enroll(address, &code, card.as_deref(), tx))
            .await?
        {
            wait.rx = None; // refused before any attempt opened
            return Err(e);
        }
        wait.answer().await
    }

    /// Keep a hand-configured direct peer dialed (the daemon's
    /// `fabric/settings` `peers`, §8.1): the first registered dialer
    /// maintains `address`, expecting `node_id` there. The fabric records
    /// the address, so an enrollment that ends without `ENROLL_OK` never
    /// releases a dial a configured peer holds (module doc). `NoLink` when
    /// no dialer is registered.
    pub async fn dial_peer(&self, address: SocketAddr, node_id: NodeId) -> Result<()> {
        self.call(move |c| c.dial_peer(address, node_id)).await?
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

/// The member's open enrollment attempt (§7.3; one at a time).
struct Enrolling {
    hub: NodeId,
    code: Code,
    address: SocketAddr,
    reply: Option<oneshot::Sender<Result<()>>>,
    /// This attempt started the dial to `address` (neither the home hub nor
    /// a configured peer held it then): an end without `ENROLL_OK` releases
    /// it.
    dialed: bool,
    /// This attempt pinned the hub's card for a contact that did not exist
    /// before: an end without `ENROLL_OK` forgets it.
    pinned: bool,
}

/// How an enrollment attempt ended without `ENROLL_OK` (module doc): every
/// end undoes the same things ([`Core::end_enroll`]) and tells its caller
/// why.
enum EnrollEnd {
    /// The hub refused (`ENROLL_OK` with a status other than OK), or its
    /// answer did not verify or pin.
    Refused(FabricError),
    /// No registered link can dial the address.
    NoLink,
    /// A newer attempt replaced it.
    Superseded,
    /// The hub's contact was forgotten meanwhile.
    HubForgotten,
    /// Its caller stopped waiting (the route's bound elapsed, or its client
    /// went away).
    Abandoned,
}

impl EnrollEnd {
    fn error(self) -> FabricError {
        match self {
            EnrollEnd::Refused(e) => e,
            EnrollEnd::NoLink => FabricError::NoLink,
            EnrollEnd::Superseded => FabricError::Invalid("superseded by a new enrollment"),
            EnrollEnd::HubForgotten => FabricError::Invalid("the hub was forgotten"),
            EnrollEnd::Abandoned => FabricError::Invalid("the enrollment was abandoned"),
        }
    }
}

/// The member's wait for `ENROLL_OK` ([`Fabric::enroll`]). Dropped before
/// the answer, it closes the reply and abandons the attempt
/// ([`Core::abandon_enroll`]), which then leaves nothing behind.
struct EnrollWait {
    rx: Option<oneshot::Receiver<Result<()>>>,
    input: mpsc::UnboundedSender<Input>,
}

impl EnrollWait {
    async fn answer(&mut self) -> Result<()> {
        let Some(rx) = self.rx.as_mut() else {
            return Err(FabricError::Stopped);
        };
        let answer = rx.await;
        self.rx = None;
        answer.map_err(|_| FabricError::Stopped)?
    }
}

impl Drop for EnrollWait {
    fn drop(&mut self) {
        if let Some(rx) = self.rx.take() {
            // Closed first: the actor ends only the attempt nobody awaits.
            drop(rx);
            let job: Job = Box::new(|c: &mut Core| c.abandon_enroll());
            let _ = self.input.send(Input::Run(job));
        }
    }
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
    /// One foreign row (Layer 6 §10).
    ForeignMsg(i64),
    /// One foreign conversation: its coalesced channel traffic and its own
    /// changes share the key (the first change wins, as for messages).
    ForeignConv(i64),
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
    /// Addresses kept dialed for configured peers ([`Fabric::dial_peer`]).
    peer_dials: HashSet<SocketAddr>,
    read_marks: HashMap<NodeId, u64>,
    counters: Counters,
    events: broadcast::Sender<FabricEvent>,
    pending_events: BTreeMap<EventKey, FabricEvent>,
    /// Kind, pin and title of the foreign conversations seen this run (the
    /// §10 coalescing decision; a bounded cache refilled from the store).
    foreign_convs: HashMap<i64, ConvFacts>,
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

    /// `hub.reachable` (§15): a connection is attributed to `hub` — the hub
    /// itself answered on it since it came up (a `pair_mac`-verified packet
    /// the L2 window judged fresh, §8.1). A connection entry lives from its
    /// link-up to its disconnect, so an answer heard on an earlier
    /// connection never counts.
    fn hub_answered(&self, hub: &NodeId) -> bool {
        self.conns
            .values()
            .any(|c| c.attributed.as_ref() == Some(hub))
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
        // §9 as built (L5 m3): a fetch is always answered. When nothing is
        // held above the cursor, an item-less `MAILBOX_ITEM {custody_seq =
        // cursor}` says so — the member's proof that its hub serves it on
        // this connection (`hub.reachable`). Items held but not streamable
        // on this path get no such claim.
        let streamed = self
            .conns
            .get(ck)
            .is_some_and(|c| !c.push.inflight.is_empty());
        if !streamed {
            match self.store.custody_fetch(sender.as_bytes(), cursor, 1) {
                Ok(held) if held.is_empty() => {
                    self.send_control(ck, sender, Kind::MailboxItem, encode_item(cursor, &[]));
                }
                Ok(_) => {}
                Err(_) => self.counters.errors += 1,
            }
        }
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
        // The hub's answer to a fetch when it holds nothing above the cursor
        // (an item-less `MAILBOX_ITEM {custody_seq}`, `on_fetch`): its outer
        // packet already attributed the connection; nothing to store, and
        // the cursor stays where it is.
        if decode_cursor(payload).is_ok() {
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

    /// §7.3 member side: the hub's answer to the open attempt. Anything but
    /// a verified, pinned OK ends the attempt as refused ([`Self::end_enroll`]:
    /// the dial it started released, a card it pinned forgotten).
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
        if ok.status != EnrollStatus::Ok {
            let why = EnrollEnd::Refused(FabricError::Enroll(ok.status));
            return self.end_enroll(en, why);
        }
        let (ann, card) = match ok.card.as_deref().map(|c| (verify_card(c), c)) {
            Some((Ok(ann), card)) if ann.node_id() == *sender => (ann, card),
            _ => {
                let why = EnrollEnd::Refused(FabricError::Enroll(EnrollStatus::Refused));
                return self.end_enroll(en, why);
            }
        };
        if let Err(e) = self.pin(&ann, card, Trust::Enrolled) {
            return self.end_enroll(en, EnrollEnd::Refused(e));
        }
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
        if let Some(r) = en.reply.take() {
            let _ = r.send(Ok(()));
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
        // A `CUSTODY_PUT` the hub ignored or lost holds its fragments in
        // flight until a disconnect: the retry re-PUTs them now (the same
        // stored bytes, so the hub dedups by `pkt_ref`; an ACK of the
        // earlier PUT still matches the new entry).
        self.custody_inflight.retain(|(i, _)| *i != id);
        self.custody_pending.retain(|_, (i, _)| *i != id);
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
                e.pinned = false; // forgotten just now
                self.end_enroll(e, EnrollEnd::HubForgotten);
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
        // A configured direct peer at the same address keeps its dial (the
        // rule `dial_held` applies to enrollment attempts).
        if let (Some(addr), Some(d)) = (h.address, self.dialer()) {
            if !self.peer_dials.contains(&addr) {
                d.release(addr);
            }
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
                connected: hub.is_some_and(|h| self.sendable_conn(&h.node_id).is_some()),
                reachable: hub.is_some_and(|h| self.hub_answered(&h.node_id)),
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

    /// §7.3 member side: open an attempt (replacing any open one). An `Err`
    /// opens nothing and pins nothing; once open, the attempt answers
    /// `reply` exactly once — on `ENROLL_OK`, or through
    /// [`Self::end_enroll`].
    fn enroll(
        &mut self,
        address: SocketAddr,
        code: &str,
        card: Option<&str>,
        reply: oneshot::Sender<Result<()>>,
    ) -> Result<()> {
        let code = normalize_code(code)?;
        let card = match card {
            Some(text) => {
                let bytes = parse_card_text(text)?;
                let ann = verify_card(&bytes)?;
                if ann.node_id() == self.own {
                    return Err(CardError::OwnCard.into());
                }
                Some((ann, bytes))
            }
            None => None,
        };
        let hub = match (&card, self.settings.home_hub) {
            (Some((ann, _)), _) => ann.node_id(),
            (None, Some(h)) => h.node_id,
            (None, None) => return Err(FabricError::Invalid("the hub's card is needed")),
        };
        // What the attempt this one replaces holds for the same hub or the
        // same address passes on to it (and is not undone with it).
        let (mut pinned, inherit_dial) = match &self.enrolling {
            Some(o) => (o.hub == hub && o.pinned, o.address == address && o.dialed),
            None => (false, false),
        };
        if let Some((ann, bytes)) = &card {
            let known = self.contact_known(&hub);
            self.pin(ann, bytes, Trust::Oob)?;
            self.contact_event(hub, ContactChange::Added);
            pinned |= !known;
        }
        if let Some(mut old) = self.enrolling.take() {
            old.pinned &= old.hub != hub;
            old.dialed &= old.address != address;
            self.end_enroll(old, EnrollEnd::Superseded);
        }
        let mut en = Enrolling {
            hub,
            code,
            address,
            reply: Some(reply),
            dialed: inherit_dial,
            pinned,
        };
        match self.sendable_conn(&hub) {
            Some(ck) => {
                self.enrolling = Some(en);
                self.send_enroll(&ck);
            }
            None => match self.dialer() {
                Some(d) => {
                    en.dialed |= !self.dial_held(address);
                    d.maintain(address, Some(hub));
                    self.enrolling = Some(en);
                }
                None => self.end_enroll(en, EnrollEnd::NoLink),
            },
        }
        Ok(())
    }

    /// §7.3 member side: an attempt that ends without `ENROLL_OK` leaves
    /// nothing behind (module doc). The dial it started is released unless
    /// the home hub or a configured peer holds that address now; the card it
    /// pinned for a new contact is forgotten unless that contact became the
    /// home hub or was used or edited since (an alias, a pin, a stronger
    /// trust, an own message to it). Then its caller hears why.
    fn end_enroll(&mut self, mut en: Enrolling, why: EnrollEnd) {
        if en.dialed && !self.dial_held(en.address) {
            if let Some(d) = self.dialer() {
                d.release(en.address);
            }
        }
        if en.pinned && Some(en.hub) != self.home_hub() {
            let untouched = matches!(
                self.store.contact(en.hub.as_bytes()),
                Ok(Some(c)) if c.trust == Trust::Oob && c.alias.is_none() && !c.pinned
            ) && !self.tx.values().any(|m| m.peer == en.hub);
            if untouched && self.forget(en.hub).is_err() {
                self.counters.errors += 1;
            }
        }
        self.fabric_changed();
        if let Some(r) = en.reply.take() {
            let _ = r.send(Err(why.error()));
        }
    }

    /// [`Fabric::enroll`]'s wait was dropped: end the open attempt if its
    /// reply has no receiver any more (a newer attempt's caller still
    /// waits, so that one is left alone).
    fn abandon_enroll(&mut self) {
        let abandoned = self
            .enrolling
            .as_ref()
            .is_some_and(|e| e.reply.as_ref().is_none_or(|r| r.is_closed()));
        if abandoned {
            if let Some(en) = self.enrolling.take() {
                self.end_enroll(en, EnrollEnd::Abandoned);
            }
        }
    }

    /// A dial at `address` that an enrollment attempt must not release: the
    /// home hub's, or a configured peer's.
    fn dial_held(&self, address: SocketAddr) -> bool {
        self.peer_dials.contains(&address)
            || self.settings.home_hub.and_then(|h| h.address) == Some(address)
    }

    /// Anything is known about `id` (a contact row, a `node1…` entry waiting
    /// for its card, a message request); a store error counts as known, so
    /// an attempt never forgets what it could not see.
    fn contact_known(&self, id: &NodeId) -> bool {
        !matches!(self.store.contact(id.as_bytes()), Ok(None))
            || self.pending_keys.contains(id)
            || self.request_pending(id)
    }

    /// [`Fabric::dial_peer`].
    fn dial_peer(&mut self, address: SocketAddr, node_id: NodeId) -> Result<()> {
        if node_id == self.own {
            return Err(FabricError::Invalid("cannot dial this node itself"));
        }
        let d = self.dialer().ok_or(FabricError::NoLink)?;
        self.peer_dials.insert(address);
        d.maintain(address, Some(node_id));
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
// Layer 6 foreign calls (`60-transport/README.md` §3, §8, §9, §10)
// ---------------------------------------------------------------------------

/// Rows per [`Fabric::foreign_ingest`] call (§3: one bounded batch per
/// adapter tick; the store's own cap, checked before the actor is asked).
pub const MAX_FOREIGN_INGEST: usize = MAX_FOREIGN_BATCH;
/// The largest page a foreign read returns at once
/// ([`Fabric::foreign_thread`], [`Fabric::foreign_queued`],
/// [`Fabric::foreign_edge_inputs`]); larger limits are clamped.
pub const MAX_FOREIGN_PAGE: usize = 10_000;
/// Conversations whose kind, pin and title the actor remembers (a cache for
/// the §10 coalescing decision: cleared when full, refilled from the store).
const FOREIGN_CONV_CACHE: usize = 4096;

/// [`Fabric::foreign_set_state`]'s outcome: `Changed`, or `Unchanged` when
/// the state would move back (§4 monotone) or nothing differs.
pub type ForeignStateOutcome = StateChange;

/// One own foreign row to queue ([`Fabric::foreign_queue_send`]; the owned
/// form of [`ForeignSend`]). Exactly one of `group_id` (user-composed; a
/// direct reply is a one-target send group, [`Fabric::foreign_group_create`])
/// or `edge_id` + `origin_msg` (a §8 relay copy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignQueue {
    /// Destination conversation.
    pub conv: i64,
    /// The text of this part (≤ 64 KiB; the marker already applied).
    pub text: String,
    /// The send group of a user-composed row.
    pub group_id: Option<i64>,
    /// Part `k` of `n` (1-based; `1/1` for a whole text).
    pub part_k: u32,
    /// Parts of the split send.
    pub part_n: u32,
    /// The inbound row a relay copy copies.
    pub origin_msg: Option<i64>,
    /// The edge a relay copy belongs to.
    pub edge_id: Option<i64>,
}

impl ForeignQueue {
    /// A §8 relay copy (part `k` of `n`) of `origin` on `edge` into `conv`.
    pub fn relay_copy(conv: i64, text: String, edge: i64, origin: i64, k: u32, n: u32) -> Self {
        ForeignQueue {
            conv,
            text,
            group_id: None,
            part_k: k,
            part_n: n,
            origin_msg: Some(origin),
            edge_id: Some(edge),
        }
    }

    fn borrow(&self) -> ForeignSend<'_> {
        ForeignSend {
            conv: self.conv,
            text: &self.text,
            group_id: self.group_id,
            part_k: self.part_k,
            part_n: self.part_n,
            origin_msg: self.origin_msg,
            edge_id: self.edge_id,
        }
    }
}

/// One foreign part of a send group ([`Fabric::foreign_group_create`]; the
/// owned form of [`GroupPart`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignGroupPart {
    /// Target conversation.
    pub conv: i64,
    /// This part's text.
    pub text: String,
    /// Part `k` of `n` (1-based).
    pub part_k: u32,
    /// Parts of this target's send.
    pub part_n: u32,
}

/// The recorded human consent of an edge (§8; the owned form of
/// [`EdgeConsent`]): the warning text's hash and who accepted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignConsent {
    /// Lowercase hex SHA-256 of the exact warning text confirmed.
    pub consent_hash: String,
    /// The session that confirmed (`loopback`, a paired session's label).
    pub enabled_by: String,
}

impl ForeignConsent {
    fn borrow(&self) -> EdgeConsent<'_> {
        EdgeConsent {
            consent_hash: &self.consent_hash,
            enabled_by: &self.enabled_by,
        }
    }
}

/// A new relay edge ([`Fabric::foreign_edge_create`]; the owned form of
/// [`EdgeNew`]): created enabled with `cursor_seq` = the current
/// `local_seq`, so nothing already stored is relayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignEdgeNew {
    /// Source conversation (`channel` or `group`).
    pub src_conv: i64,
    /// Destination conversation (`channel` or `group`).
    pub dst_conv: i64,
    /// Drop Meshtastic-origin rows without the ok-to-MQTT bit (off by default).
    pub honour_ok_to_mqtt: bool,
    /// Too-long copies: `refuse` (default) or `split`.
    pub oversize: EdgeOversize,
    /// The confirmed warning.
    pub consent: ForeignConsent,
}

/// A partial change of an edge ([`Fabric::foreign_edge_patch`],
/// `BridgeEdgePatch`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForeignEdgePatch {
    /// Pause, or resume (resuming records `consent`).
    pub paused: Option<bool>,
    /// The new confirmation a resume records.
    pub consent: Option<ForeignConsent>,
    /// Must equal the stored value: the store keeps it as created.
    pub honour_ok_to_mqtt: Option<bool>,
    /// Must equal the stored value: the store keeps it as created.
    pub oversize: Option<EdgeOversize>,
}

/// One `meta` value ([`Fabric::foreign_set_meta`]; the owned form of
/// [`MetaValue`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForeignMeta {
    /// A string (≤ 256 bytes).
    Str(String),
    /// An integer.
    Int(i64),
    /// A boolean.
    Bool(bool),
}

/// One page of a foreign conversation ([`Fabric::foreign_thread`], §10
/// `ForeignThread`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignThreadPage {
    /// The conversation with its counts.
    pub conversation: ForeignConversationRow,
    /// Rows ascending by `local_seq`.
    pub messages: Vec<ForeignMessageRow>,
    /// Rows exist beyond the window: older ones without `since_seq`, newer
    /// ones with it.
    pub more: bool,
}

/// A send group with its foreign rows ([`Fabric::foreign_group`], §9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignGroupView {
    /// The group as stored.
    pub group: SendGroupRow,
    /// Every part of every foreign target, by `local_seq`.
    pub rows: Vec<ForeignMessageRow>,
}

impl Fabric {
    /// §3 ingest: insert up to [`MAX_FOREIGN_INGEST`] inbound rows (after
    /// the bridge core's one ingest function) in **one** transaction.
    /// Returns, per input, the new row id or `None` for a §3 duplicate. A
    /// larger batch is refused here, before the actor is asked; any invalid
    /// row writes nothing. Await each call before the next (back-pressure,
    /// module doc).
    pub async fn foreign_ingest(&self, batch: Vec<ForeignInsert>) -> Result<Vec<Option<i64>>> {
        if batch.len() > MAX_FOREIGN_INGEST {
            return Err(FabricError::Invalid("foreign ingest over 256 rows"));
        }
        if batch.is_empty() {
            return Ok(Vec::new());
        }
        self.call(move |c| c.foreign_ingest(batch)).await?
    }

    /// §4 monotone state update of an own row (a lower state is ignored:
    /// [`StateChange::Unchanged`]). `evidence` is the adapter's string the
    /// UI prints; `reason` a §4 reason code.
    pub async fn foreign_set_state(
        &self,
        id: i64,
        state: ForeignState,
        evidence: Option<&str>,
        reason: Option<&str>,
    ) -> Result<ForeignStateOutcome> {
        let evidence = evidence.map(str::to_owned);
        let reason = reason.map(str::to_owned);
        self.call(move |c| c.foreign_set_state(id, state, evidence, reason))
            .await?
    }

    /// Set (`Some`) or remove (`None`) one `meta` field of a row (`[a-z_]`,
    /// ≤ 32 bytes; never `handoff`, which has its own calls). Returns
    /// whether the row exists.
    pub async fn foreign_set_meta(
        &self,
        id: i64,
        key: &str,
        value: Option<ForeignMeta>,
    ) -> Result<bool> {
        let key = key.to_owned();
        self.call(move |c| c.foreign_set_meta(id, &key, value))
            .await?
    }

    /// Queue one own row (a relay copy, or a part of an existing send
    /// group). A relay copy advances its edge's `relayed_seq` in the same
    /// transaction (§8).
    pub async fn foreign_queue_send(&self, send: ForeignQueue) -> Result<i64> {
        self.foreign_queue_sends(vec![send])
            .await?
            .into_iter()
            .next()
            .ok_or(FabricError::Invalid("nothing was queued"))
    }

    /// Queue up to 256 own rows in one transaction (every part of a split
    /// relay copy together, so a crash never leaves half a copy).
    pub async fn foreign_queue_sends(&self, sends: Vec<ForeignQueue>) -> Result<Vec<i64>> {
        if sends.len() > MAX_FOREIGN_BATCH {
            return Err(FabricError::Invalid("foreign batch over 256 rows"));
        }
        self.call(move |c| c.foreign_queue_sends(sends)).await?
    }

    /// Cancel an own row (§9): only while `queued` and never handed to a
    /// carrier (no pending handoff, no recorded native id); the store
    /// deletes it ([`Store::foreign_cancel`], FULL), so it can never be
    /// sent. Anything else is [`FabricError::NotRetryable`]. Other parts of
    /// the same send are not touched (cancel drops unsent parts only, one
    /// call per part).
    pub async fn foreign_cancel(&self, id: i64) -> Result<()> {
        self.call(move |c| c.foreign_cancel(id)).await?
    }

    /// §9: create a send group (`text`, its `targets` JSON) and queue its
    /// foreign parts in one transaction (atomic: a bad part queues nothing;
    /// `parts` may be empty for 111-only targets). Returns the group id and
    /// the row ids in `parts` order.
    pub async fn foreign_group_create(
        &self,
        text: &str,
        targets: &str,
        parts: Vec<ForeignGroupPart>,
    ) -> Result<(i64, Vec<i64>)> {
        if parts.len() > MAX_FOREIGN_BATCH {
            return Err(FabricError::Invalid("foreign batch over 256 rows"));
        }
        let (text, targets) = (text.to_owned(), targets.to_owned());
        self.call(move |c| c.foreign_group_create(&text, &targets, &parts))
            .await?
    }

    /// One send group with its rows.
    pub async fn foreign_group(&self, id: i64) -> Result<ForeignGroupView> {
        self.call(move |c| c.foreign_group(id)).await?
    }

    /// Create an enabled relay edge with its recorded consent (§8; DM
    /// endpoints, `src = dst` and a duplicate pair are refused).
    pub async fn foreign_edge_create(&self, edge: ForeignEdgeNew) -> Result<EdgeRow> {
        self.call(move |c| c.foreign_edge_create(&edge)).await?
    }

    /// Every edge, by id.
    pub async fn foreign_edge_list(&self) -> Result<Vec<EdgeRow>> {
        self.call(|c| c.store.edge_list().map_err(fx)).await?
    }

    /// One edge.
    pub async fn foreign_edge_get(&self, id: i64) -> Result<EdgeRow> {
        self.call(move |c| c.store.edge(id).map_err(fx)?.ok_or(FabricError::NotFound))
            .await?
    }

    /// Enable (recording `consent`; `cursor_seq` moves to now: what arrived
    /// while off is history) or disable an edge (the store drops its copies
    /// never handed to a carrier: a `Conversation` event for the
    /// destination). Returns whether it changed.
    pub async fn foreign_edge_set_enabled(
        &self,
        id: i64,
        enabled: bool,
        consent: Option<ForeignConsent>,
    ) -> Result<bool> {
        self.call(move |c| c.foreign_edge_set_enabled(id, enabled, consent.as_ref()))
            .await?
    }

    /// Pause, or resume with the new confirmation (§8 `consent_paused`).
    /// Returns whether it changed.
    pub async fn foreign_edge_set_paused(
        &self,
        id: i64,
        paused: bool,
        consent: Option<ForeignConsent>,
    ) -> Result<bool> {
        self.call(move |c| {
            let consent = consent.as_ref().map(ForeignConsent::borrow);
            c.store
                .edge_set_paused(id, paused, consent.as_ref())
                .map_err(fx)
        })
        .await?
    }

    /// `PATCH /bridge/edges/{id}`: pause or resume. `honour_ok_to_mqtt` and
    /// `oversize` are kept as created by this store version: a patch that
    /// would change either is refused before anything is written.
    pub async fn foreign_edge_patch(&self, id: i64, patch: ForeignEdgePatch) -> Result<EdgeRow> {
        self.call(move |c| c.foreign_edge_patch(id, &patch)).await?
    }

    /// Advance an edge's `relayed_seq` to at least `seq` (a shed, §8;
    /// never backwards). Returns the stored value.
    pub async fn foreign_edge_advance_relayed(&self, id: i64, seq: u64) -> Result<u64> {
        self.call(move |c| c.store.edge_advance_relayed_seq(id, seq).map_err(fx))
            .await?
    }

    /// Add `n` to one of an edge's §8 counters (`relayed`, `shed`,
    /// `refused`).
    pub async fn foreign_edge_count(&self, id: i64, counter: EdgeCounter, n: u64) -> Result<()> {
        self.call(move |c| c.store.edge_count(id, counter, n).map_err(fx))
            .await?
    }

    /// Delete an edge with its loop-guard keys and its copies never handed
    /// to a carrier. Returns whether it existed.
    pub async fn foreign_edge_delete(&self, id: i64) -> Result<bool> {
        self.call(move |c| c.foreign_edge_delete(id)).await?
    }

    /// §8 "what it relays": inbound rows of the edge's source above
    /// `max(cursor_seq, relayed_seq)` that are not relayed copies, oldest
    /// first, at most `limit` (clamped to [`MAX_FOREIGN_PAGE`]).
    pub async fn foreign_edge_inputs(
        &self,
        edge: i64,
        limit: usize,
    ) -> Result<Vec<ForeignMessageRow>> {
        let limit = limit.min(MAX_FOREIGN_PAGE);
        self.call(move |c| c.store.edge_inputs(edge, limit).map_err(fx))
            .await?
    }

    /// Record a loop-guard key on `edge` (§3 `foreign_seen`, bounded per
    /// edge, oldest evicted).
    pub async fn foreign_seen_insert(
        &self,
        edge: i64,
        conv: i64,
        sender_native: &str,
        native_msg_id: &str,
        local_seq: u64,
    ) -> Result<()> {
        let (sender, native) = (sender_native.to_owned(), native_msg_id.to_owned());
        self.call(move |c| {
            c.store
                .foreign_seen_insert(edge, conv, &sender, &native, local_seq)
                .map_err(fx)
        })
        .await?
    }

    /// Whether a loop-guard key is held on `edge` (any edge when `None`).
    pub async fn foreign_seen_contains(
        &self,
        edge: Option<i64>,
        conv: i64,
        sender_native: &str,
        native_msg_id: &str,
    ) -> Result<bool> {
        let (sender, native) = (sender_native.to_owned(), native_msg_id.to_owned());
        self.call(move |c| {
            c.store
                .foreign_seen_contains(edge, conv, &sender, &native)
                .map_err(fx)
        })
        .await?
    }

    /// Insert or find a conversation by `(network, account, kind,
    /// native_id)` (§3 conversation identity); a `title` replaces the
    /// stored one. A conversation new to this run, or a new title, emits a
    /// `Conversation` event.
    pub async fn foreign_conversation_upsert(
        &self,
        network: &str,
        account: &str,
        kind: ForeignKind,
        native_id: &str,
        title: Option<&str>,
    ) -> Result<i64> {
        let (network, account, native) =
            (network.to_owned(), account.to_owned(), native_id.to_owned());
        let title = title.map(str::to_owned);
        self.call(move |c| {
            c.foreign_conversation_upsert(&network, &account, kind, &native, title.as_deref())
        })
        .await?
    }

    /// Pin or unpin a conversation (pinned rows are never pruned, and a
    /// pinned channel's rows are no longer coalesced).
    pub async fn foreign_conversation_pin(&self, id: i64, pinned: bool) -> Result<()> {
        self.call(move |c| c.foreign_conversation_pin(id, pinned))
            .await?
    }

    /// Clear a conversation's unread counter (an event only when it was
    /// not zero).
    pub async fn foreign_conversation_mark_read(&self, id: i64) -> Result<()> {
        self.call(move |c| c.foreign_conversation_mark_read(id))
            .await?
    }

    /// The conversations of `network` (every network when `None`), most
    /// recent first.
    pub async fn foreign_conversations(
        &self,
        network: Option<&str>,
    ) -> Result<Vec<ForeignConversationRow>> {
        let network = network.map(str::to_owned);
        self.call(move |c| {
            c.store
                .foreign_conversations(network.as_deref())
                .map_err(fx)
        })
        .await?
    }

    /// One conversation with its counts.
    pub async fn foreign_conversation(&self, id: i64) -> Result<ForeignConversationRow> {
        self.call(move |c| c.foreign_conversation(id)).await?
    }

    /// One page of a conversation (§10 `?limit&since_seq`): without
    /// `since_seq` the newest `limit` rows, with it the first `limit` rows
    /// above it; ascending by `local_seq` either way.
    pub async fn foreign_thread(
        &self,
        conv: i64,
        since_seq: Option<u64>,
        limit: usize,
    ) -> Result<ForeignThreadPage> {
        self.call(move |c| c.foreign_thread(conv, since_seq, limit))
            .await?
    }

    /// The adapter tick's queue for `network` (§4 order): own `queued` rows
    /// without a pending handoff, user-composed rows first, then
    /// `local_seq`; copies of a disabled or paused edge are held back.
    pub async fn foreign_queued(
        &self,
        network: &str,
        limit: usize,
    ) -> Result<Vec<ForeignMessageRow>> {
        let network = network.to_owned();
        let limit = limit.min(MAX_FOREIGN_PAGE);
        self.call(move |c| c.store.foreign_queued(&network, limit).map_err(fx))
            .await?
    }

    /// One foreign row.
    pub async fn foreign_message(&self, id: i64) -> Result<ForeignMessageRow> {
        self.call(move |c| {
            c.store
                .foreign_message(id)
                .map_err(fx)?
                .ok_or(FabricError::NotFound)
        })
        .await?
    }

    /// §4 at most once: mark a `queued` own row `handoff = pending`
    /// durably (FULL) before it is handed to a carrier.
    pub async fn foreign_handoff_begin(&self, id: i64) -> Result<()> {
        self.call(move |c| c.store.foreign_handoff_begin(id).map_err(fx))
            .await?
    }

    /// §4 `Accepted { native_id }`: record the carrier's native id (own-echo
    /// suppression, §8 guard 2) and clear the handoff mark.
    pub async fn foreign_accepted(&self, id: i64, native_msg_id: &str) -> Result<()> {
        let native = native_msg_id.to_owned();
        self.call(move |c| c.store.foreign_accepted(id, &native).map_err(fx))
            .await?
    }
}

/// What the actor remembers about a foreign conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ConvFacts {
    kind: ForeignKind,
    pinned: bool,
    title: Option<String>,
}

impl ConvFacts {
    fn of(c: &ForeignConversationRow) -> ConvFacts {
        ConvFacts {
            kind: c.kind,
            pinned: c.pinned,
            title: c.title.clone(),
        }
    }
}

/// Store failures of the typed foreign calls: a missing row is the API's
/// `NotFound`, a refused input its `Invalid` (a `400`, not a store fault).
fn fx(e: StoreError) -> FabricError {
    match e {
        StoreError::NotFound => FabricError::NotFound,
        StoreError::Invalid(why) => FabricError::Invalid(why),
        e => FabricError::Store(e),
    }
}

impl Core {
    // -- foreign events (§10 coalescing) -------------------------------------

    fn foreign_event(&mut self, conv: i64, id: Option<i64>, change: ForeignChange) {
        let key = match id {
            Some(id) => EventKey::ForeignMsg(id),
            None => EventKey::ForeignConv(conv),
        };
        self.pending_events
            .entry(key)
            .or_insert(FabricEvent::Foreign { conv, id, change });
    }

    fn remember_conv(&mut self, id: i64, facts: ConvFacts) {
        if self.foreign_convs.len() >= FOREIGN_CONV_CACHE && !self.foreign_convs.contains_key(&id) {
            self.foreign_convs.clear();
        }
        self.foreign_convs.insert(id, facts);
    }

    /// Inbound rows of this conversation coalesce per conversation: an
    /// unpinned channel. Unknown facts (a store failure) coalesce too — the
    /// bounded choice; the client refetches the conversation either way.
    fn coalesces(&mut self, conv: i64) -> bool {
        if let Some(f) = self.foreign_convs.get(&conv) {
            return f.kind == ForeignKind::Channel && !f.pinned;
        }
        match self.store.foreign_conversation(conv) {
            Ok(Some(c)) => {
                let f = ConvFacts::of(&c);
                let per_conv = f.kind == ForeignKind::Channel && !f.pinned;
                self.remember_conv(conv, f);
                per_conv
            }
            Ok(None) => true,
            Err(_) => {
                self.counters.errors += 1;
                true
            }
        }
    }

    /// The §10 rule for one row: per message for own rows, DMs, groups and
    /// pinned channels; per conversation for inbound unpinned-channel rows.
    fn foreign_row_event(&mut self, conv: i64, id: i64, dir: ForeignDir, change: ForeignChange) {
        if dir == ForeignDir::In && self.coalesces(conv) {
            self.foreign_event(conv, None, change);
        } else {
            self.foreign_event(conv, Some(id), change);
        }
    }

    /// [`Core::foreign_row_event`] for a row known by id only.
    fn foreign_id_event(&mut self, id: i64, change: ForeignChange) {
        match self.store.foreign_message(id) {
            Ok(Some(r)) => self.foreign_row_event(r.conv, id, r.dir, change),
            Ok(None) => {}
            Err(_) => self.counters.errors += 1,
        }
    }

    // -- writes ---------------------------------------------------------------

    fn foreign_ingest(&mut self, batch: Vec<ForeignInsert>) -> Result<Vec<Option<i64>>> {
        let convs: Vec<i64> = batch.iter().map(|r| r.conv).collect();
        let ids = self.store.foreign_insert_batch(batch).map_err(fx)?;
        for (conv, id) in convs.into_iter().zip(&ids) {
            if let Some(id) = *id {
                self.foreign_row_event(conv, id, ForeignDir::In, ForeignChange::New);
            }
        }
        Ok(ids)
    }

    fn foreign_set_state(
        &mut self,
        id: i64,
        state: ForeignState,
        evidence: Option<String>,
        reason: Option<String>,
    ) -> Result<StateChange> {
        let ch = self
            .store
            .foreign_set_state(id, state, evidence.as_deref(), reason.as_deref())
            .map_err(fx)?;
        if ch == StateChange::Changed {
            self.foreign_id_event(id, ForeignChange::State);
        }
        Ok(ch)
    }

    fn foreign_set_meta(&mut self, id: i64, key: &str, value: Option<ForeignMeta>) -> Result<bool> {
        if key == "handoff" {
            return Err(FabricError::Invalid(
                "the handoff mark is set only by foreign_handoff_begin / foreign_accepted",
            ));
        }
        let v = value.as_ref().map(|v| match v {
            ForeignMeta::Str(s) => MetaValue::Str(s),
            ForeignMeta::Int(i) => MetaValue::Int(*i),
            ForeignMeta::Bool(b) => MetaValue::Bool(*b),
        });
        let found = self.store.foreign_set_meta(id, key, v).map_err(fx)?;
        if found {
            self.foreign_id_event(id, ForeignChange::State);
        }
        Ok(found)
    }

    fn foreign_queue_sends(&mut self, sends: Vec<ForeignQueue>) -> Result<Vec<i64>> {
        let rows: Vec<ForeignSend<'_>> = sends.iter().map(ForeignQueue::borrow).collect();
        let ids = self.store.foreign_queue_sends(&rows).map_err(fx)?;
        for (s, id) in sends.iter().zip(&ids) {
            self.foreign_event(s.conv, Some(*id), ForeignChange::New);
        }
        Ok(ids)
    }

    fn foreign_cancel(&mut self, id: i64) -> Result<()> {
        let row = self
            .store
            .foreign_message(id)
            .map_err(fx)?
            .ok_or(FabricError::NotFound)?;
        // The store refuses a pending handoff; a recorded native id (the
        // carrier accepted it) is refused here too: fail closed.
        let untouched = row.dir == ForeignDir::Own
            && row.state == ForeignState::Queued
            && !row.handoff_pending
            && row.native_msg_id.is_none();
        if !untouched {
            return Err(FabricError::NotRetryable);
        }
        self.store.foreign_cancel(id).map_err(fx)?;
        self.foreign_event(row.conv, Some(id), ForeignChange::State);
        Ok(())
    }

    fn foreign_group_create(
        &mut self,
        text: &str,
        targets: &str,
        parts: &[ForeignGroupPart],
    ) -> Result<(i64, Vec<i64>)> {
        let gp: Vec<GroupPart<'_>> = parts
            .iter()
            .map(|p| GroupPart {
                conv: p.conv,
                text: &p.text,
                part_k: p.part_k,
                part_n: p.part_n,
            })
            .collect();
        let (group, ids) = self
            .store
            .send_group_create(text, targets, &gp)
            .map_err(fx)?;
        for (p, id) in parts.iter().zip(&ids) {
            self.foreign_event(p.conv, Some(*id), ForeignChange::New);
        }
        Ok((group, ids))
    }

    fn foreign_group(&self, id: i64) -> Result<ForeignGroupView> {
        let group = self
            .store
            .send_group(id)
            .map_err(fx)?
            .ok_or(FabricError::NotFound)?;
        let rows = self.store.send_group_rows(id).map_err(fx)?;
        Ok(ForeignGroupView { group, rows })
    }

    fn foreign_edge_create(&mut self, e: &ForeignEdgeNew) -> Result<EdgeRow> {
        let id = self
            .store
            .edge_create(&EdgeNew {
                src_conv: e.src_conv,
                dst_conv: e.dst_conv,
                honour_ok_to_mqtt: e.honour_ok_to_mqtt,
                oversize: e.oversize,
                consent: e.consent.borrow(),
            })
            .map_err(fx)?;
        self.store
            .edge(id)
            .map_err(fx)?
            .ok_or(FabricError::Store(StoreError::Corrupt("edge just created")))
    }

    fn foreign_edge_patch(&mut self, id: i64, p: &ForeignEdgePatch) -> Result<EdgeRow> {
        let e = self
            .store
            .edge(id)
            .map_err(fx)?
            .ok_or(FabricError::NotFound)?;
        if p.honour_ok_to_mqtt
            .is_some_and(|v| v != e.honour_ok_to_mqtt)
            || p.oversize.is_some_and(|v| v != e.oversize)
        {
            return Err(FabricError::Invalid(
                "an edge keeps honour_ok_to_mqtt and oversize as created; \
                 delete it and create a new one to change them",
            ));
        }
        if let Some(paused) = p.paused {
            let consent = p.consent.as_ref().map(ForeignConsent::borrow);
            self.store
                .edge_set_paused(id, paused, consent.as_ref())
                .map_err(fx)?;
        }
        self.store
            .edge(id)
            .map_err(fx)?
            .ok_or(FabricError::NotFound)
    }

    fn foreign_edge_set_enabled(
        &mut self,
        id: i64,
        enabled: bool,
        consent: Option<&ForeignConsent>,
    ) -> Result<bool> {
        let dst = self.store.edge(id).map_err(fx)?.map(|e| e.dst_conv);
        let consent = consent.map(ForeignConsent::borrow);
        let changed = self
            .store
            .edge_set_enabled(id, enabled, consent.as_ref())
            .map_err(fx)?;
        if let (true, false, Some(dst)) = (changed, enabled, dst) {
            // a disabled edge has no backlog: its never-handed copies left
            self.foreign_event(dst, None, ForeignChange::Conversation);
        }
        Ok(changed)
    }

    fn foreign_edge_delete(&mut self, id: i64) -> Result<bool> {
        let Some(e) = self.store.edge(id).map_err(fx)? else {
            return Ok(false);
        };
        let gone = self.store.edge_delete(id).map_err(fx)?;
        if gone {
            // queued copies of the edge left the destination
            self.foreign_event(e.dst_conv, None, ForeignChange::Conversation);
        }
        Ok(gone)
    }

    fn foreign_conversation_upsert(
        &mut self,
        network: &str,
        account: &str,
        kind: ForeignKind,
        native_id: &str,
        title: Option<&str>,
    ) -> Result<i64> {
        let id = self
            .store
            .foreign_conversation_upsert(network, account, kind, native_id, title)
            .map_err(fx)?;
        let unchanged = self
            .foreign_convs
            .get(&id)
            .is_some_and(|f| title.is_none() || f.title.as_deref() == title);
        if !unchanged {
            match self.store.foreign_conversation(id) {
                Ok(Some(c)) => self.remember_conv(id, ConvFacts::of(&c)),
                _ => {
                    self.foreign_convs.remove(&id);
                }
            }
            self.foreign_event(id, None, ForeignChange::Conversation);
        }
        Ok(id)
    }

    fn foreign_conversation_pin(&mut self, id: i64, pinned: bool) -> Result<()> {
        self.store
            .foreign_conversation_set_pinned(id, pinned)
            .map_err(fx)?;
        if let Some(f) = self.foreign_convs.get_mut(&id) {
            f.pinned = pinned;
        }
        self.foreign_event(id, None, ForeignChange::Conversation);
        Ok(())
    }

    fn foreign_conversation_mark_read(&mut self, id: i64) -> Result<()> {
        let c = self.foreign_conversation(id)?;
        if c.unread == 0 {
            return Ok(());
        }
        self.store.foreign_conversation_mark_read(id).map_err(fx)?;
        self.foreign_event(id, None, ForeignChange::Conversation);
        Ok(())
    }

    // -- reads ----------------------------------------------------------------

    fn foreign_conversation(&self, id: i64) -> Result<ForeignConversationRow> {
        self.store
            .foreign_conversation(id)
            .map_err(fx)?
            .ok_or(FabricError::NotFound)
    }

    fn foreign_thread(
        &self,
        conv: i64,
        since_seq: Option<u64>,
        limit: usize,
    ) -> Result<ForeignThreadPage> {
        let conversation = self.foreign_conversation(conv)?;
        let limit = limit.clamp(1, MAX_FOREIGN_PAGE);
        let mut messages = match since_seq {
            Some(s) => self.store.foreign_messages(conv, s, limit + 1),
            None => self.store.foreign_thread_page(conv, limit + 1),
        }
        .map_err(fx)?;
        let more = messages.len() > limit;
        if more {
            match since_seq {
                Some(_) => messages.truncate(limit),
                None => {
                    messages.remove(0);
                }
            }
        }
        Ok(ForeignThreadPage {
            conversation,
            messages,
            more,
        })
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

/// Layer 6 foreign calls through the actor (§3 typed writes, §10 SSE
/// coalescing): every call, the 256-row cap, and the coalescing rule.
#[cfg(test)]
mod foreign_tests {
    use super::*;
    use crate::keys::StoreKey;
    use crate::store::{ForeignAuth, OpenParams};
    use crate::test_double::TestSealer;

    struct Node {
        f: Fabric,
        _dir: tempfile::TempDir,
    }

    /// Store opens are serialized within the test process: concurrent first
    /// SQLCipher opens failed rarely under load (the integration harness
    /// and the daemon serialize them the same way).
    static OPEN: Mutex<()> = Mutex::new(());

    fn node() -> Node {
        let dir = tempfile::tempdir().expect("tempdir");
        let sealer = Arc::new(TestSealer::from_label("bridge"));
        let _one = OPEN.lock().unwrap_or_else(|e| e.into_inner());
        let (store, decision) = Store::open(
            dir.path(),
            &StoreKey::from_bytes([0x6f; 32]),
            &OpenParams {
                announce_last: sealer.announce_last(),
                restore_floor_set: false,
            },
        )
        .expect("store");
        let f = Fabric::start(sealer, store, decision, FabricSettings::default()).expect("fabric");
        Node { f, _dir: dir }
    }

    fn row(conv: i64, native: Option<&str>, body: &str) -> ForeignInsert {
        ForeignInsert {
            conv,
            sender_native: "!a1b2c3d4".into(),
            sender_name: Some("W1AW".into()),
            auth: ForeignAuth::ChannelKey,
            native_msg_id: native.map(str::to_owned),
            body: body.into(),
            claim_ts: None,
            meta: None,
            id_globally_unique: false,
        }
    }

    fn rows(conv: i64, tag: &str, n: usize) -> Vec<ForeignInsert> {
        (0..n)
            .map(|i| row(conv, Some(&format!("{tag}{i}")), &format!("{tag} {i}")))
            .collect()
    }

    fn consent(h: char) -> ForeignConsent {
        ForeignConsent {
            consent_hash: h.to_string().repeat(64),
            enabled_by: "loopback".into(),
        }
    }

    type Ev = (i64, Option<i64>, ForeignChange);

    /// Subscribe after a barrier call, so events of earlier calls (emitted
    /// at the end of their tick) are not seen.
    async fn subscribe(f: &Fabric) -> broadcast::Receiver<FabricEvent> {
        f.foreign_conversations(None).await.expect("barrier");
        f.subscribe()
    }

    /// The `Foreign` events emitted so far: a barrier call first (its job
    /// runs only after every earlier tick emitted its events).
    async fn events(f: &Fabric, rx: &mut broadcast::Receiver<FabricEvent>) -> Vec<Ev> {
        f.foreign_conversations(None).await.expect("barrier");
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            if let FabricEvent::Foreign { conv, id, change } = e {
                out.push((conv, id, change));
            }
        }
        out
    }

    async fn channel(f: &Fabric, account: &str, name: &str) -> i64 {
        f.foreign_conversation_upsert(
            "meshtastic",
            account,
            ForeignKind::Channel,
            &format!("ch:{name}:00ff"),
            Some(name),
        )
        .await
        .expect("channel")
    }

    async fn dm(f: &Fabric, native: &str) -> i64 {
        f.foreign_conversation_upsert("meshtastic", "!board", ForeignKind::Dm, native, None)
            .await
            .expect("dm")
    }

    #[tokio::test]
    async fn ingest_is_one_bounded_batch_per_call() {
        let n = node();
        let c = channel(&n.f, "!board", "LongFast").await;
        let over = rows(c, "o", MAX_FOREIGN_INGEST + 1);
        assert!(matches!(
            n.f.foreign_ingest(over).await,
            Err(FabricError::Invalid(_))
        ));
        assert_eq!(n.f.foreign_conversation(c).await.unwrap().messages, 0);
        assert!(n.f.foreign_ingest(Vec::new()).await.unwrap().is_empty());
        let ids = n.f.foreign_ingest(rows(c, "k", 256)).await.unwrap();
        assert_eq!(ids.len(), 256);
        assert!(ids.iter().all(Option::is_some));
        // one transaction: the batch drew contiguous local_seq
        let page = n.f.foreign_thread(c, None, 300).await.unwrap();
        let seqs: Vec<u64> = page.messages.iter().map(|m| m.local_seq).collect();
        assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "{seqs:?}");
        // a repeated (key, body) inside the window is a duplicate
        let tail: Vec<ForeignInsert> = rows(c, "k", 256).into_iter().skip(253).collect();
        let again = n.f.foreign_ingest(tail).await.unwrap();
        assert_eq!(again, vec![None, None, None]);
        // the same packet id with another body is a new row only beyond
        // the window: here k0's id is 256 rows back, so it is inserted
        let old = n.f.foreign_ingest(rows(c, "k", 1)).await.unwrap();
        assert!(old[0].is_some());
        // an unknown conversation writes nothing and names nothing
        assert!(matches!(
            n.f.foreign_ingest(vec![row(9_999, None, "x")]).await,
            Err(FabricError::NotFound)
        ));
        let mut big = rows(c, "q", 2);
        big[1].body = "x".repeat(crate::store::MAX_FOREIGN_BODY + 1);
        assert!(matches!(
            n.f.foreign_ingest(big).await,
            Err(FabricError::Invalid(_))
        ));
        assert_eq!(n.f.foreign_conversation(c).await.unwrap().messages, 257);
    }

    #[tokio::test]
    async fn unpinned_channel_traffic_coalesces_per_conversation_dms_per_message() {
        let n = node();
        let mut rx = subscribe(&n.f).await;
        let c = channel(&n.f, "!board", "LongFast").await;
        assert_eq!(
            events(&n.f, &mut rx).await,
            vec![(c, None, ForeignChange::Conversation)]
        );
        // 50 channel rows in one call: ONE event, for the conversation
        let ids = n.f.foreign_ingest(rows(c, "a", 50)).await.unwrap();
        assert_eq!(ids.iter().flatten().count(), 50);
        assert_eq!(
            events(&n.f, &mut rx).await,
            vec![(c, None, ForeignChange::New)]
        );
        // a DM: one event per row
        let d = dm(&n.f, "dm:!0badc0de").await;
        assert_eq!(
            events(&n.f, &mut rx).await,
            vec![(d, None, ForeignChange::Conversation)]
        );
        let ids = n.f.foreign_ingest(rows(d, "d", 5)).await.unwrap();
        let got = events(&n.f, &mut rx).await;
        let want: Vec<Ev> = ids
            .iter()
            .map(|id| (d, Some(id.unwrap()), ForeignChange::New))
            .collect();
        assert_eq!(got, want);
        // one mixed batch: the channel once, the DM rows each
        let mut mixed = rows(c, "m", 10);
        mixed.extend(rows(d, "e", 3));
        n.f.foreign_ingest(mixed).await.unwrap();
        let got = events(&n.f, &mut rx).await;
        assert_eq!(got.iter().filter(|e| e.0 == c).count(), 1);
        assert_eq!(got.iter().filter(|e| e.0 == d && e.1.is_some()).count(), 3);
        // a pinned channel's rows are shown (and signalled) one by one
        n.f.foreign_conversation_pin(c, true).await.unwrap();
        assert_eq!(
            events(&n.f, &mut rx).await,
            vec![(c, None, ForeignChange::Conversation)]
        );
        n.f.foreign_ingest(rows(c, "p", 4)).await.unwrap();
        let got = events(&n.f, &mut rx).await;
        assert_eq!(got.len(), 4);
        assert!(got.iter().all(|e| e.0 == c && e.1.is_some()));
        n.f.foreign_conversation_pin(c, false).await.unwrap();
        events(&n.f, &mut rx).await;
        n.f.foreign_ingest(rows(c, "u", 7)).await.unwrap();
        assert_eq!(events(&n.f, &mut rx).await.len(), 1);
        // the same upsert again: no event; a new title: one
        assert_eq!(channel(&n.f, "!board", "LongFast").await, c);
        n.f.foreign_conversation_upsert(
            "meshtastic",
            "!board",
            ForeignKind::Channel,
            "ch:LongFast:00ff",
            None,
        )
        .await
        .unwrap();
        assert!(events(&n.f, &mut rx).await.is_empty());
        n.f.foreign_conversation_upsert(
            "meshtastic",
            "!board",
            ForeignKind::Channel,
            "ch:LongFast:00ff",
            Some("Long Fast"),
        )
        .await
        .unwrap();
        assert_eq!(
            events(&n.f, &mut rx).await,
            vec![(c, None, ForeignChange::Conversation)]
        );
        assert_eq!(
            n.f.foreign_conversation(c).await.unwrap().title.as_deref(),
            Some("Long Fast")
        );
        // the rail: every network, then one
        assert_eq!(n.f.foreign_conversations(None).await.unwrap().len(), 2);
        assert_eq!(
            n.f.foreign_conversations(Some("meshtastic"))
                .await
                .unwrap()
                .len(),
            2
        );
        assert!(n
            .f
            .foreign_conversations(Some("lxmf"))
            .await
            .unwrap()
            .is_empty());
        // a refused network id is a 400, never a store fault
        assert!(matches!(
            n.f.foreign_conversation_upsert("111", "a", ForeignKind::Dm, "x", None)
                .await,
            Err(FabricError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn own_rows_states_and_meta_are_per_message() {
        let n = node();
        let d = dm(&n.f, "dm:!0badc0de").await;
        let mut rx = subscribe(&n.f).await;
        let part = ForeignGroupPart {
            conv: d,
            text: "hello".into(),
            part_k: 1,
            part_n: 1,
        };
        let (g, ids) =
            n.f.foreign_group_create("hello", "[]", vec![part])
                .await
                .unwrap();
        let id = ids[0];
        assert_eq!(
            events(&n.f, &mut rx).await,
            vec![(d, Some(id), ForeignChange::New)]
        );
        let view = n.f.foreign_group(g).await.unwrap();
        assert_eq!(view.group.text, "hello");
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0].group_id, Some(g));
        assert!(matches!(
            n.f.foreign_group(g + 100).await,
            Err(FabricError::NotFound)
        ));
        // the adapter tick's queue
        let q = n.f.foreign_queued("meshtastic", 10).await.unwrap();
        assert_eq!(q.iter().map(|m| m.id).collect::<Vec<_>>(), vec![id]);
        // §4 monotone states
        let sent =
            n.f.foreign_set_state(id, ForeignState::Sent, Some("accepted by the radio"), None)
                .await
                .unwrap();
        assert_eq!(sent, StateChange::Changed);
        assert_eq!(
            events(&n.f, &mut rx).await,
            vec![(d, Some(id), ForeignChange::State)]
        );
        let back =
            n.f.foreign_set_state(id, ForeignState::Queued, None, None)
                .await
                .unwrap();
        assert_eq!(back, StateChange::Unchanged);
        assert!(events(&n.f, &mut rx).await.is_empty());
        let m = n.f.foreign_message(id).await.unwrap();
        assert_eq!(m.state, ForeignState::Sent);
        assert_eq!(m.evidence.as_deref(), Some("accepted by the radio"));
        // meta: one field, never the handoff mark
        assert!(matches!(
            n.f.foreign_set_meta(id, "handoff", Some(ForeignMeta::Str("pending".into())))
                .await,
            Err(FabricError::Invalid(_))
        ));
        assert!(n
            .f
            .foreign_set_meta(id, "hops", Some(ForeignMeta::Int(3)))
            .await
            .unwrap());
        assert!(!n
            .f
            .foreign_set_meta(9_999, "hops", Some(ForeignMeta::Int(3)))
            .await
            .unwrap());
        assert_eq!(
            events(&n.f, &mut rx).await,
            vec![(d, Some(id), ForeignChange::State)]
        );
        assert!(n
            .f
            .foreign_message(id)
            .await
            .unwrap()
            .meta
            .contains("\"hops\":3"));
        // a group of more than 256 parts never reaches the actor
        let many: Vec<ForeignGroupPart> = (0..257)
            .map(|_| ForeignGroupPart {
                conv: d,
                text: "x".into(),
                part_k: 1,
                part_n: 1,
            })
            .collect();
        assert!(matches!(
            n.f.foreign_group_create("x", "[]", many).await,
            Err(FabricError::Invalid(_))
        ));
        assert!(matches!(
            n.f.foreign_message(9_999).await,
            Err(FabricError::NotFound)
        ));
        // an inbound row has no send state
        let inbound = n.f.foreign_ingest(rows(d, "i", 1)).await.unwrap()[0].unwrap();
        assert!(matches!(
            n.f.foreign_set_state(inbound, ForeignState::Sent, None, None)
                .await,
            Err(FabricError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn cancel_only_while_queued_and_never_handed_off() {
        let n = node();
        let d = dm(&n.f, "dm:!0badc0de").await;
        let parts = (1..=3)
            .map(|k| ForeignGroupPart {
                conv: d,
                text: format!("({k}/3) part"),
                part_k: k,
                part_n: 3,
            })
            .collect();
        let (_, ids) =
            n.f.foreign_group_create("part part part", "[]", parts)
                .await
                .unwrap();
        let (a, b, c) = (ids[0], ids[1], ids[2]);
        n.f.foreign_handoff_begin(a).await.unwrap();
        assert!(matches!(
            n.f.foreign_cancel(a).await,
            Err(FabricError::NotRetryable)
        ));
        let mut rx = subscribe(&n.f).await;
        n.f.foreign_cancel(b).await.unwrap();
        assert_eq!(
            events(&n.f, &mut rx).await,
            vec![(d, Some(b), ForeignChange::State)]
        );
        // deleted: it can never be offered or sent
        assert!(matches!(
            n.f.foreign_message(b).await,
            Err(FabricError::NotFound)
        ));
        assert!(!n
            .f
            .foreign_queued("meshtastic", 10)
            .await
            .unwrap()
            .iter()
            .any(|m| m.id == b));
        assert!(matches!(
            n.f.foreign_cancel(b).await,
            Err(FabricError::NotFound)
        ));
        // a sent row is past cancelling
        n.f.foreign_set_state(c, ForeignState::Sent, None, None)
            .await
            .unwrap();
        assert!(matches!(
            n.f.foreign_cancel(c).await,
            Err(FabricError::NotRetryable)
        ));
        // accepted by a carrier (native id recorded): no longer cancellable
        n.f.foreign_accepted(a, "0x2a").await.unwrap();
        assert!(!n.f.foreign_message(a).await.unwrap().handoff_pending);
        assert!(matches!(
            n.f.foreign_cancel(a).await,
            Err(FabricError::NotRetryable)
        ));
        // the other parts keep their own state
        assert_eq!(
            n.f.foreign_message(c).await.unwrap().state,
            ForeignState::Sent
        );
        assert!(n
            .f
            .foreign_message(a)
            .await
            .unwrap()
            .native_msg_id
            .is_some());
        let inbound = n.f.foreign_ingest(rows(d, "i", 1)).await.unwrap()[0].unwrap();
        assert!(matches!(
            n.f.foreign_cancel(inbound).await,
            Err(FabricError::NotRetryable)
        ));
        assert!(matches!(
            n.f.foreign_cancel(9_999).await,
            Err(FabricError::NotFound)
        ));
        assert!(matches!(
            n.f.foreign_handoff_begin(b).await,
            Err(FabricError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn edges_through_the_actor() {
        let n = node();
        let src = channel(&n.f, "!board", "LongFast").await;
        let dst = channel(&n.f, "!board2", "Ops").await;
        let d = dm(&n.f, "dm:!0badc0de").await;
        n.f.foreign_ingest(rows(src, "h", 2)).await.unwrap(); // history
        let new = |s, t| ForeignEdgeNew {
            src_conv: s,
            dst_conv: t,
            honour_ok_to_mqtt: false,
            oversize: EdgeOversize::Refuse,
            consent: consent('a'),
        };
        let e = n.f.foreign_edge_create(new(src, dst)).await.unwrap();
        assert!(e.enabled && !e.paused && e.cursor_seq > 0);
        assert_eq!(
            (e.src_kind, e.dst_kind),
            (ForeignKind::Channel, ForeignKind::Channel)
        );
        for bad in [new(d, dst), new(src, d), new(src, src), new(src, dst)] {
            assert!(matches!(
                n.f.foreign_edge_create(bad).await,
                Err(FabricError::Invalid(_))
            ));
        }
        assert!(matches!(
            n.f.foreign_edge_create(new(src, 9_999)).await,
            Err(FabricError::NotFound)
        ));
        assert!(n.f.foreign_edge_inputs(e.id, 10).await.unwrap().is_empty());
        n.f.foreign_ingest(rows(src, "n", 3)).await.unwrap();
        let inputs = n.f.foreign_edge_inputs(e.id, 10).await.unwrap();
        assert_eq!(inputs.len(), 3, "history before cursor_seq is never input");
        let origin = &inputs[0];
        let copy =
            n.f.foreign_queue_send(ForeignQueue::relay_copy(
                dst,
                format!("[W1AW/M] {}", origin.body),
                e.id,
                origin.id,
                1,
                1,
            ))
            .await
            .unwrap();
        let e2 = n.f.foreign_edge_get(e.id).await.unwrap();
        assert_eq!(e2.relayed_seq, origin.local_seq, "advanced with the copy");
        assert_eq!(n.f.foreign_edge_inputs(e.id, 10).await.unwrap().len(), 2);
        let c = n.f.foreign_message(copy).await.unwrap();
        assert_eq!((c.edge_id, c.origin_msg), (Some(e.id), Some(origin.id)));
        // a copy to the wrong conversation, or one that is user-composed and a copy
        let mut wrong = ForeignQueue::relay_copy(src, "x".into(), e.id, inputs[1].id, 1, 1);
        assert!(matches!(
            n.f.foreign_queue_send(wrong.clone()).await,
            Err(FabricError::Invalid(_))
        ));
        wrong.conv = dst;
        wrong.group_id = Some(1);
        assert!(matches!(
            n.f.foreign_queue_send(wrong).await,
            Err(FabricError::Invalid(_))
        ));
        let q = |f: Fabric| async move { f.foreign_queued("meshtastic", 10).await.unwrap() };
        assert_eq!(q(n.f.clone()).await.len(), 1);
        // patch: pause and resume; options stay as created
        let refused = ForeignEdgePatch {
            oversize: Some(EdgeOversize::Split),
            paused: Some(true),
            ..ForeignEdgePatch::default()
        };
        assert!(matches!(
            n.f.foreign_edge_patch(e.id, refused).await,
            Err(FabricError::Invalid(_))
        ));
        assert!(
            !n.f.foreign_edge_get(e.id).await.unwrap().paused,
            "nothing written"
        );
        let same = ForeignEdgePatch {
            oversize: Some(EdgeOversize::Refuse),
            honour_ok_to_mqtt: Some(false),
            paused: Some(true),
            ..ForeignEdgePatch::default()
        };
        assert!(n.f.foreign_edge_patch(e.id, same).await.unwrap().paused);
        assert!(
            q(n.f.clone()).await.is_empty(),
            "a paused edge's copies wait"
        );
        let resume = ForeignEdgePatch {
            paused: Some(false),
            ..ForeignEdgePatch::default()
        };
        assert!(matches!(
            n.f.foreign_edge_patch(e.id, resume.clone()).await,
            Err(FabricError::Invalid(_))
        ));
        let resume = ForeignEdgePatch {
            consent: Some(consent('b')),
            ..resume
        };
        let r = n.f.foreign_edge_patch(e.id, resume).await.unwrap();
        assert!(!r.paused);
        assert_eq!(r.consent_hash, "b".repeat(64));
        assert!(n.f.foreign_edge_set_paused(e.id, true, None).await.unwrap());
        assert!(!n.f.foreign_edge_set_paused(e.id, true, None).await.unwrap());
        assert!(n
            .f
            .foreign_edge_set_paused(e.id, false, Some(consent('c')))
            .await
            .unwrap());
        // disable drops the never-handed backlog (the destination is told);
        // re-enable moves the cursor (what arrived while off is history)
        assert_eq!(q(n.f.clone()).await.len(), 1);
        let mut rx = subscribe(&n.f).await;
        assert!(n
            .f
            .foreign_edge_set_enabled(e.id, false, None)
            .await
            .unwrap());
        assert_eq!(
            events(&n.f, &mut rx).await,
            vec![(dst, None, ForeignChange::Conversation)]
        );
        let gone = n.f.foreign_message(copy).await;
        assert!(matches!(gone, Err(FabricError::NotFound)), "{gone:?}");
        assert!(!n
            .f
            .foreign_edge_set_enabled(e.id, false, None)
            .await
            .unwrap());
        assert!(
            events(&n.f, &mut rx).await.is_empty(),
            "no change, no event"
        );
        n.f.foreign_ingest(rows(src, "off", 2)).await.unwrap();
        assert!(matches!(
            n.f.foreign_edge_set_enabled(e.id, true, None).await,
            Err(FabricError::Invalid(_))
        ));
        assert!(n
            .f
            .foreign_edge_set_enabled(e.id, true, Some(consent('d')))
            .await
            .unwrap());
        assert!(n.f.foreign_edge_inputs(e.id, 10).await.unwrap().is_empty());
        // counters, the high-water mark, the loop guard
        n.f.foreign_edge_count(e.id, EdgeCounter::Relayed, 2)
            .await
            .unwrap();
        n.f.foreign_edge_count(e.id, EdgeCounter::Refused, 1)
            .await
            .unwrap();
        let cur = n.f.foreign_edge_get(e.id).await.unwrap().cursor_seq;
        let hw =
            n.f.foreign_edge_advance_relayed(e.id, cur + 1)
                .await
                .unwrap();
        assert_eq!(hw, cur + 1);
        assert_eq!(
            n.f.foreign_edge_advance_relayed(e.id, 1).await.unwrap(),
            cur + 1,
            "never backwards"
        );
        let e3 = n.f.foreign_edge_get(e.id).await.unwrap();
        assert_eq!((e3.relayed, e3.refused, e3.shed), (2, 1, 0));
        n.f.foreign_seen_insert(e.id, src, "!a1b2c3d4", "n0", 7)
            .await
            .unwrap();
        assert!(n
            .f
            .foreign_seen_contains(Some(e.id), src, "!a1b2c3d4", "n0")
            .await
            .unwrap());
        assert!(n
            .f
            .foreign_seen_contains(None, src, "!a1b2c3d4", "n0")
            .await
            .unwrap());
        assert!(!n
            .f
            .foreign_seen_contains(None, src, "!a1b2c3d4", "n1")
            .await
            .unwrap());
        assert!(matches!(
            n.f.foreign_seen_insert(9_999, src, "!a", "n", 1).await,
            Err(FabricError::NotFound)
        ));
        assert_eq!(n.f.foreign_edge_list().await.unwrap().len(), 1);
        // delete: the destination is told (its queued copies are gone)
        // (the first new row sits at the shed mark cur + 1: never input)
        let z = n.f.foreign_ingest(rows(src, "z", 2)).await.unwrap();
        assert_eq!(n.f.foreign_edge_inputs(e.id, 10).await.unwrap().len(), 1);
        let o2 = z[1].unwrap();
        let copy2 =
            n.f.foreign_queue_send(ForeignQueue::relay_copy(
                dst,
                "[W1AW/M] z".into(),
                e.id,
                o2,
                1,
                1,
            ))
            .await
            .unwrap();
        assert!(
            matches!(
                n.f.foreign_queue_send(ForeignQueue::relay_copy(
                    dst,
                    "again".into(),
                    e.id,
                    o2,
                    1,
                    1
                ))
                .await,
                Err(FabricError::Invalid(_))
            ),
            "one copy per origin per edge"
        );
        assert!(n.f.foreign_edge_delete(e.id).await.unwrap());
        let got = events(&n.f, &mut rx).await;
        let at_dst: Vec<Ev> = got.iter().copied().filter(|e| e.0 == dst).collect();
        assert_eq!(
            at_dst,
            vec![
                (dst, Some(copy2), ForeignChange::New),
                (dst, None, ForeignChange::Conversation)
            ]
        );
        // the source's two ingests since the subscription: one event each
        assert_eq!(got.iter().filter(|e| e.0 == src).count(), 2, "{got:?}");
        let gone = n.f.foreign_message(copy2).await;
        assert!(matches!(gone, Err(FabricError::NotFound)), "{gone:?}");
        assert!(!n.f.foreign_edge_delete(e.id).await.unwrap());
        assert!(n.f.foreign_edge_list().await.unwrap().is_empty());
        assert!(matches!(
            n.f.foreign_edge_get(e.id).await,
            Err(FabricError::NotFound)
        ));
    }

    #[tokio::test]
    async fn queue_order_puts_user_rows_ahead_of_relay_copies() {
        let n = node();
        let src = channel(&n.f, "!board", "LongFast").await;
        let dst = channel(&n.f, "!board2", "Ops").await;
        let e =
            n.f.foreign_edge_create(ForeignEdgeNew {
                src_conv: src,
                dst_conv: dst,
                honour_ok_to_mqtt: false,
                oversize: EdgeOversize::Split,
                consent: consent('a'),
            })
            .await
            .unwrap();
        let o = n.f.foreign_ingest(rows(src, "r", 1)).await.unwrap()[0].unwrap();
        let split = vec![
            ForeignQueue::relay_copy(dst, "[W1AW/M] (1/2) a".into(), e.id, o, 1, 2),
            ForeignQueue::relay_copy(dst, "[W1AW/M] (2/2) b".into(), e.id, o, 2, 2),
        ];
        let copies = n.f.foreign_queue_sends(split).await.unwrap();
        assert_eq!(copies.len(), 2);
        let part = ForeignGroupPart {
            conv: dst,
            text: "mine".into(),
            part_k: 1,
            part_n: 1,
        };
        let (_, mine) =
            n.f.foreign_group_create("mine", "[]", vec![part])
                .await
                .unwrap();
        let order: Vec<i64> =
            n.f.foreign_queued("meshtastic", 10)
                .await
                .unwrap()
                .iter()
                .map(|m| m.id)
                .collect();
        assert_eq!(order, vec![mine[0], copies[0], copies[1]]);
        assert_eq!(n.f.foreign_queued("meshtastic", 1).await.unwrap().len(), 1);
        assert!(matches!(
            n.f.foreign_queue_sends(Vec::new()).await,
            Err(FabricError::Invalid(_))
        ));
        let many = (0..257)
            .map(|_| ForeignQueue::relay_copy(dst, "x".into(), e.id, o, 1, 1))
            .collect();
        assert!(matches!(
            n.f.foreign_queue_sends(many).await,
            Err(FabricError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn thread_pages_read_marks_and_more() {
        let n = node();
        let d = dm(&n.f, "dm:!0badc0de").await;
        let bodies = |p: &ForeignThreadPage| -> Vec<String> {
            p.messages.iter().map(|m| m.body.clone()).collect()
        };
        n.f.foreign_ingest(rows(d, "t", 5)).await.unwrap();
        let newest = n.f.foreign_thread(d, None, 2).await.unwrap();
        assert_eq!(bodies(&newest), vec!["t 3", "t 4"]);
        assert!(newest.more, "older rows exist");
        assert_eq!(newest.conversation.unread, 5);
        let all = n.f.foreign_thread(d, None, 10).await.unwrap();
        assert_eq!(all.messages.len(), 5);
        assert!(!all.more);
        let s1 = all.messages[1].local_seq;
        let after = n.f.foreign_thread(d, Some(s1), 2).await.unwrap();
        assert_eq!(bodies(&after), vec!["t 2", "t 3"]);
        assert!(after.more, "newer rows exist");
        let s3 = all.messages[3].local_seq;
        let tail = n.f.foreign_thread(d, Some(s3), 5).await.unwrap();
        assert_eq!(bodies(&tail), vec!["t 4"]);
        assert!(!tail.more);
        // limit 0 reads one row, a huge limit is clamped
        assert_eq!(
            n.f.foreign_thread(d, None, 0).await.unwrap().messages.len(),
            1
        );
        assert_eq!(
            n.f.foreign_thread(d, None, usize::MAX)
                .await
                .unwrap()
                .messages
                .len(),
            5
        );
        assert!(matches!(
            n.f.foreign_thread(9_999, None, 5).await,
            Err(FabricError::NotFound)
        ));
        // the read mark: one event, then none while nothing is unread
        let mut rx = subscribe(&n.f).await;
        n.f.foreign_conversation_mark_read(d).await.unwrap();
        assert_eq!(
            events(&n.f, &mut rx).await,
            vec![(d, None, ForeignChange::Conversation)]
        );
        assert_eq!(n.f.foreign_conversation(d).await.unwrap().unread, 0);
        n.f.foreign_conversation_mark_read(d).await.unwrap();
        assert!(events(&n.f, &mut rx).await.is_empty());
        assert!(matches!(
            n.f.foreign_conversation_mark_read(9_999).await,
            Err(FabricError::NotFound)
        ));
        assert!(matches!(
            n.f.foreign_conversation_pin(9_999, true).await,
            Err(FabricError::NotFound)
        ));
    }
}
