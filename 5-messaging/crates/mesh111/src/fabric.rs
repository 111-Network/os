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
//! ### Foreign calls (Layer 6 `6-transport/README.md` §3, §8, §9, §10)
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
//! - Readings: a cancel deletes a user-composed own row still `queued`
//!   that was never handed to a carrier (no pending handoff mark, no native
//!   id; the store's `foreign_cancel`; a relay copy is not cancelled) and
//!   emits a `Conversation` event for its conversation (the row is gone, so
//!   no per-row event names it); the handoff mark is set, cleared and
//!   withdrawn only through [`Fabric::foreign_handoff_begin`] /
//!   [`Fabric::foreign_accepted`] / [`Fabric::foreign_handoff_abort`],
//!   which emit no event (bookkeeping of the at-most-once rule; the
//!   visible change is the state the carrier reports next); a conversation
//!   upsert emits only when the store reports a new row or a new title;
//!   edge changes emit no event (the `bridge` SSE snapshot is the bridge
//!   core's), except a disable or a delete, which drop the destination's
//!   never-handed copies (a `Conversation` event for it).
//!
//! ### Pairing and cross-hub forwarding (Layer 7 §4.6, §5)
//!
//! Layer 7's amendments 1, 7 (sender side), 8, 10, 11 and 12
//! (`7-relay/README.md` §5.3). Every pairing and forwarding path starts
//! behind the hub role ([`FabricSettings::hub_role`], Layer 7 §5.11); with
//! no paired hub a hub's gate answers, frames and rows are Layer 5's.
//!
//! - **Pairing** (hub role only; §4.6 steps 1–4) is two enrollments, one in
//!   each direction, and **wholeness is recorded, never inferred from a
//!   contact's trust**. [`Fabric::offer_pair`] mints a pairing code (one
//!   open offer at a time, member or pairing); the far hub's `ENROLL` that
//!   redeems it records its [`PairedHub`] with `theirs` set and pins its
//!   card at `oob`, never `enrolled`. This hub's [`Fabric::pair`] with the
//!   far hub's pairing code records `ours` on its `ENROLL_OK`; it sets no
//!   home hub and writes no cursor. A pair is **whole** when both bools are
//!   set ([`PairedHub::whole`], persisted by the daemon through
//!   [`FabricEvent::Settings`], which is also how it learns a pair became
//!   whole); only a whole pair that did not answer `REJECT_UNENROLLED` this
//!   session is a forward target or an admitted sender, so a half pair's
//!   puts meet `REJECT_UNENROLLED` and it is offered nothing. A member
//!   offer's code ([`Fabric::offer`]) never records a pair; a recorded pair
//!   presenting one, this node's home hub presenting a pairing code
//!   (amendment 12), and a pairing code presented for a hub recorded with
//!   another role or link are refused before the code is checked (the offer
//!   is neither spent nor locked). The second `ENROLL` rides the connection
//!   the first hub opened (an attributed connection is sendable whatever its
//!   direction, S22); `ENROLL` never rides a carrier. A paired hub is never
//!   a member: not counted, never a `dest` member row, never fetched for,
//!   its commits ignored, never answered a card; a hub that is both an
//!   enrolled member and a recorded pair is a pair. [`Fabric::unpair`] drops
//!   the record (both bools) and everything held for the pair and leaves the
//!   far hub's contact as it is.
//! - **Pairing dials** (§4.6 steps 3, 5): the dial a pairing attempt
//!   started is kept only while the pair is half, so the far hub's `ENROLL`
//!   can ride it; once whole, the fabric keeps no dial for any link (the
//!   daemon's rule holds a `tcp` pair's one connection; a `carrier` pair is
//!   never dialed again). The side whose `ENROLL_OK` makes the pair whole
//!   releases its own dial and sends one `MAILBOX_FETCH {0}` on the
//!   connection; a whole pair's fetch (or its connection dropping) releases
//!   the other side's. The side whose answer makes the pair whole releases
//!   nothing at once: that `ENROLL_OK` rides the connection.
//! - **The gate** (`Core::custody_gate`, Layer 7 §5.2): the member branch
//!   is today's (a member `dest` ⇒ a `fwd = 0` row from any origin); a
//!   non-member `dest` meets `forward_ok`, a synchronous arm of this actor:
//!   it decides only whether a target **exists** under the §5.1 table,
//!   holds the row and answers `CUSTODY_ACK` before it returns; the
//!   forwarder's puts are queued after the ack (it never awaits a pair).
//! - **The forwarder** (`Core::run_forwarder`, §5.5, §5.9) runs at the
//!   end of a tick when a forward row was inserted, a pair's connection came
//!   up or attributed, a contact event, a pair's `CUSTODY_ACK` or a carrier
//!   call asked for it; never on a timer. A row's targets are computed from
//!   the table in `paired_hubs` order (hint first, then the hit cache, then
//!   the probe order); its **turn** is the first target that has not
//!   refused it, and a target that is down keeps its turn until it is up.
//!   The work per trigger is bounded by what changed: the rows sit in
//!   per-target queues by state (custody order), a row's targets are
//!   recomputed only when its inputs changed (a new or paged-in row, a
//!   refusal, an orphaning, the pair set or the carrier bound, a changed
//!   cache entry for its dest), and the put stage visits only the `tcp`
//!   pairs that are up and have window room; a row is forgotten in one
//!   lookup (its `pkt_ref` keys the pending map). One put per row at a
//!   time, at most [`FORWARD_WINDOW`] in flight per `tcp` pair (ACK-clocked
//!   flow control).
//! - **The resident window:** at most [`FORWARD_RESIDENT`] protected
//!   forward rows live in memory (about 250 B each with the indexes: ≈ 1 MB
//!   at most, hubs only). The first window is read at start; rows beyond it
//!   wait in the store, "not yet offered", and are paged in custody order
//!   from the paging floor (`Store::forward_pending_lens`: row heads with
//!   the item's length, never the blobs; one read per `FORWARD_LOAD_PAGE`
//!   rows) as resident rows leave; a page brings every row in, whatever its
//!   turn. A row held while the window is full (or while rows wait before
//!   it) goes to the store, so it is offered after the rows ahead of it,
//!   without its hint, as after a restart. **The window is shared fairly**
//!   (§5.5: a target that is down never holds back another's rows): a
//!   target is *up* when it can take a row now (a `tcp` pair with a
//!   sendable connection, a `carrier` pair while the carrier bound is set)
//!   and no `REJECT_FULL` row of it waits (a full target counts as down
//!   until its next trigger, C16, clears or accepts those rows).
//!   While the window is full and rows wait in the store, the **swap
//!   probe** reads one page of row heads per pass, in custody order; each
//!   stored row whose turn (the table, the hit cache, the probe order; no
//!   hint) is up enters in place of the **newest** idle (or `REJECT_FULL`)
//!   resident row whose turn is down or that has no target (evicted: it is
//!   in the store already, leaves memory without a refusal, and the paging
//!   floor moves below it), and the others stay in the store. Newest first
//!   per target, so every target's resident rows stay older than its
//!   stored ones and it is offered its rows in custody order when it comes
//!   up; across targets a row with no hint, no refusal and not orphaned
//!   goes first, since eviction drops all three (residual, beside the
//!   paged rows' lost hint above: an evicted row that had them comes back
//!   without, so a `fwd = 1` row whose hint named no pair may also be
//!   offered to the near `tcp` pairs, a carrier pair that refused it is
//!   offered it once more, and an orphaned one waits instead of being
//!   refused). A tick runs at most four passes; when the last read was a
//!   full page (the probe, or a page in, has more to read) the actor
//!   queues **one self-wake** (`Input::Continue`) to its own mailbox, so
//!   the next tick reads on while every other input interleaves: no timer,
//!   at most one queued, the work per tick bounded. The probe reads
//!   nothing while no target is up or nothing can be evicted, and once it
//!   reached the end of the store with nothing to move it stops until a
//!   stored row's turn can have changed: a target comes up, the pair set
//!   changes (a pairing, an unpairing, a pair break), the carrier bound
//!   changes, or a hit-cache entry is added or changed (each one restarts
//!   it from the floor, so stored rows move before a newer row for the
//!   same target); meanwhile a new row whose turn is up enters at once,
//!   with its hint, in place of an evicted one. So an all-down hub, or a
//!   store of rows for down targets only, does not churn, and each pass
//!   does at most one read and one page of evictions. The view's `pending`
//!   counts resident rows; `Store::forward_stats` has the totals. A non-hub
//!   node holds the empty structure only.
//! - **The hub role off** (a `custody_opt_in` node keeps member custody):
//!   the records stay in `paired_hubs` and forward nothing, the resident
//!   rows leave memory and wait in the store (never refused), no pair is
//!   answered or admitted, the pairing dials are released, and the view
//!   shows each pair not whole, not reachable, nothing pending. On again,
//!   the rows page in from the store.
//! - **Readings recorded for the lead:** a refusal is a fact of its
//!   connection (§5.5): a `tcp` pair's refusals are forgotten when its
//!   connection drops, so the row is offered to it again on its next
//!   link-up; with every target refused at once the row is marked refused.
//!   Carrier bound unset ([`Fabric::set_carrier_max_frame`] with `None`):
//!   no carrier pair is a target at the gate, and a row already held for a
//!   carrier pair waits for the bound instead of being refused. A whole
//!   `tcp` pair's link-up (an outbound connection that announced it) sends
//!   one `MAILBOX_FETCH {0}`, which the far hub answers with one item-less
//!   `MAILBOX_ITEM`, never items: both sides of the one connection
//!   attribute, so either may forward on it (§8.1: an inbound connection
//!   gets nothing until a fresh sealed packet attributes it). A pair's
//!   `CUSTODY_ACK` is read only from the pair the put went to: over `tcp`
//!   only when fresh on a connection attributed to it (a replay changes
//!   nothing), through the carrier only from a whole `carrier` pair.
//!   `REJECT_FULL` rows are retried on the pair's next link-up or
//!   attribution and on any other `CUSTODY_ACK` from it or a row inserted
//!   for it, never on the `REJECT_FULL` itself (that would ping-pong); each
//!   trigger releases at most one window of them, oldest first, so a pair
//!   with room again drains them ACK-clocked. The carrier's posted marks
//!   and the re-post rules live here ([`Fabric::forward_done`],
//!   [`Fabric::forward_repost`]): §5.9 rule (1) counts **far-hub rounds**
//!   per post: a verified frame from the far hub that arrives after a post
//!   (its `Taken`) and carries no ack for it marks that post once (a
//!   `FabricEvent::Forward` tells the carrier it is owed); the next
//!   [`Fabric::forward_take`] re-arms the marked posts (after the whole
//!   drain was handed in, so a drain's own acks commit before anything is
//!   re-armed). A re-post needs a far frame after it to be re-armed again:
//!   one re-post per row per far-hub round, never two within one (C6, §15
//!   G10 item 3); flushes while the far hub is silent post nothing again.
//!   The round is counted per verified far frame, not per drain (§5.9 "a
//!   drain that began after its post"): under a drain, take, flush, drain
//!   cycle the two agree, but a held stream hands in one far flush frame by
//!   frame, so the daemon's held-stream driver (D11) must call
//!   [`Fabric::forward_take`] only at a drain boundary.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};

use id111::announce::Announce;
use id111::keyset::NodeId;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::ack::{strong_hash, AckBlock, AckPayload, StrongReceipt};
use crate::contacts::{
    card_text, classify_announce, contact_row, parse_card_text, parse_node_id, verify_card,
    AnnounceVerdict, CardError, HubBundle, KeyCache, ROLE_ALWAYS_ON_HUB, ROLE_MAILBOX,
};
use crate::custody::{pkt_ref, CustodyAck, CustodyReceipt, CustodyStatus};
use crate::enroll::{Code, EnrollError, Offer as EnrollOffer, OfferState};
use crate::envelope::{
    open_envelope, payload_budget, seal_envelope, FragPos, Inner, Kind, Opened, PeerKeys, Priority,
    DEST_HUB_ALONE_LEN, DEST_HUB_LEN, MAX_PACKET_LEN, MIN_PACKET_LEN, OUTER_OVERHEAD,
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
    ForeignMessageRow, ForeignSend, ForeignState, ForwardHead, ForwardItem, FragOffer, Fwd,
    GroupPart, MessageRow, MetaValue, MsgState, NewMessage, RequestPut, SendGroupRow, Seq,
    StateChange, Store, StoreError, Trust, MAX_FOREIGN_BATCH,
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
/// Paired hubs one hub records at most (Layer 7 §4.6): capacity, not
/// policy (each is a held connection and a probe target).
pub const MAX_PAIRED_HUBS: usize = 32;
/// The forwarder's hit cache `dest → pair` (Layer 7 §5.5): at most 256
/// entries (≈ 16 KB), oldest evicted, in memory only (empty at start).
pub const FORWARD_CACHE: usize = 256;
/// Forward puts in flight to one `tcp` pair at once (Layer 7 §5.9): the
/// forwarder's flow control, clocked by the pair's `CUSTODY_ACK`s.
pub const FORWARD_WINDOW: usize = 32;
/// `CUSTODY_ACK`s held for one `carrier` pair until its next
/// [`Fabric::forward_take`] (oldest dropped: a lost ack heals by the far
/// hub's re-post, Layer 7 §5.9 C7).
pub const CARRIER_ACK_QUEUE: usize = 1024;
/// Bytes a forward put adds around its item (Layer 7 §5.2, §6.1 item 3):
/// [`CUSTODY_WRAP`] and the 11 B hint TLV ([`DEST_HUB_ALONE_LEN`]), 131 B.
pub const FORWARD_PUT_WRAP: usize = CUSTODY_WRAP + DEST_HUB_ALONE_LEN;
/// Forward rows read from the store per page (Layer 7 §5.9: the rows
/// resume after a restart, and rows beyond the resident window come in as
/// it drains): one `forward_pending_lens` read of row heads per 256 rows
/// (the swap probe reads at most one such page per pass).
const FORWARD_LOAD_PAGE: usize = 256;
/// Protected forward rows the forwarder holds in memory at most (the
/// resident window, module doc): about 250 B each, ≈ 1 MB at most, hubs
/// only. Rows beyond it wait in the store, not yet offered, and are paged
/// in custody order as the window drains.
pub const FORWARD_RESIDENT: usize = 4_096;
/// Forwarder passes per run (`Core::run_forwarder`): at most one store read
/// each, so a tick reads at most this many pages of row heads before the
/// forwarder's self-wake hands the rest to the next tick.
const FORWARD_PASSES: usize = 4;

/// Whether an item of `item_len` sealed bytes fits a carrier whose largest
/// frame is `max_frame` (Layer 7 §5.2, §6.1 item 3): item +
/// [`FORWARD_PUT_WRAP`] ≤ the bound. `None` (no carrier built) fits
/// nothing. At ntfy's 3,068 B a 2,937 B item fits and 2,938 B does not.
pub fn carrier_fits(max_frame: Option<usize>, item_len: usize) -> bool {
    max_frame.is_some_and(|m| item_len.saturating_add(FORWARD_PUT_WRAP) <= m)
}

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
    /// Hubs paired with this one (Layer 7 §4.6 steps 3–4, amendment 10):
    /// the one record the gate reads, each with the two enrollments it
    /// holds ([`PairedHub::whole`]). Only a hub-role node records pairs;
    /// with the hub role off they stay here and forward nothing. A paired
    /// hub is never the home hub (module doc).
    pub paired_hubs: Vec<PairedHub>,
}

impl FabricSettings {
    /// This node holds custody (and runs enrollment) for members.
    pub fn holds_custody(&self) -> bool {
        self.hub_role || self.custody_opt_in
    }
}

/// A paired hub's role, from this hub's side (Layer 7 §4.6 step 2, §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PairRole {
    /// A peer hub (`tcp` or `carrier`).
    Peer,
    /// This hub is its gateway (`tcp` only).
    Leaf,
    /// It is this hub's upstream (`tcp` only).
    Gateway,
}

impl PairRole {
    /// Every role.
    pub const ALL: [PairRole; 3] = [PairRole::Peer, PairRole::Leaf, PairRole::Gateway];

    /// The pairing bundle's and the settings file's word.
    pub fn code(self) -> &'static str {
        match self {
            PairRole::Peer => "peer",
            PairRole::Leaf => "leaf",
            PairRole::Gateway => "gateway",
        }
    }

    /// The inverse of [`PairRole::code`].
    pub fn from_code(s: &str) -> Option<PairRole> {
        PairRole::ALL.into_iter().find(|r| r.code() == s)
    }

    /// The role the far hub records for this one (Layer 7 §4.6 step 2, C3):
    /// `peer` ↔ `peer`, `leaf` ↔ `gateway`.
    pub fn complement(self) -> PairRole {
        match self {
            PairRole::Peer => PairRole::Peer,
            PairRole::Leaf => PairRole::Gateway,
            PairRole::Gateway => PairRole::Leaf,
        }
    }
}

/// How a paired hub is reached (Layer 7 §4.6 step 2, §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PairVia {
    /// A `tcp-lan` connection (LAN, tunnel or TLS 1.3 RPK).
    Tcp,
    /// This hub's carrier (Layer 7 §6), through the hand-off API.
    Carrier,
}

impl PairVia {
    /// Every link.
    pub const ALL: [PairVia; 2] = [PairVia::Tcp, PairVia::Carrier];

    /// The pairing bundle's and the settings file's word.
    pub fn code(self) -> &'static str {
        match self {
            PairVia::Tcp => "tcp",
            PairVia::Carrier => "carrier",
        }
    }

    /// The inverse of [`PairVia::code`].
    pub fn from_code(s: &str) -> Option<PairVia> {
        PairVia::ALL.into_iter().find(|v| v.code() == s)
    }
}

/// One paired hub (Layer 7 §4.6 steps 3–4, amendment 10; §19 code seam 3:
/// `{node_id, role, via}`, the record the gate reads, plus the two
/// enrollments it holds, `ours` and `theirs`: wholeness is recorded, never
/// inferred from a contact's trust; an additive deviation from amendment
/// 10's shape, D33). The daemon persists all five in its `paired_hubs` row
/// and keeps the address, link key, `lan` and the warning hash beside them
/// in `fabric/settings`; none of those is the fabric's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PairedHub {
    /// The far hub.
    pub node_id: NodeId,
    /// Its role from this hub's side.
    pub role: PairRole,
    /// Its link.
    pub via: PairVia,
    /// This hub's pairing `ENROLL` got `ENROLL_OK` at the far hub
    /// ([`Fabric::pair`]).
    pub ours: bool,
    /// The far hub redeemed this hub's pairing offer
    /// ([`Fabric::offer_pair`]).
    pub theirs: bool,
}

impl PairedHub {
    /// Layer 7 §4.6 step 2: a `carrier` pair is `peer` only, and a `leaf`
    /// or `gateway` pair is `tcp` only (row 3's final put and the §5.5 probe
    /// need a connection).
    pub fn valid(&self) -> bool {
        self.via == PairVia::Tcp || self.role == PairRole::Peer
    }

    /// Both enrollments are recorded (§4.6 step 4: "a pair forwards only
    /// when both sides hold it"): only a whole pair is a forward target or
    /// origin.
    pub fn whole(&self) -> bool {
        self.ours && self.theirs
    }
}

/// One sealed frame owed to a `carrier` pair (Layer 7 §5.9): a forward put
/// (`custody_seq = Some`) or a `CUSTODY_ACK` owed to it (`None`; consumed
/// by the [`Fabric::forward_take`] that returned it, C7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarrierFrame {
    /// The forward row this put carries; `None` for an ack.
    pub custody_seq: Option<u64>,
    /// One sealed hub-to-hub packet.
    pub bytes: Vec<u8>,
}

/// What the carrier did with a put it took (Layer 7 §5.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardDone {
    /// The carrier's mailbox accepted it: an in-memory "posted" mark; the
    /// row stays `committed = 0` until the far hub's `CUSTODY_ACK` arrives
    /// through [`Fabric::carrier_inbound`] (§5.6; §19 C5).
    Taken,
    /// Permanent (HTTP 413, 401, 403 for ntfy): the row returns to the
    /// other targets; with none left it is refused and counted (§5.8).
    Rejected {
        /// One plain phrase for the log.
        why: &'static str,
    },
    /// Transient (no route, HTTP 429 or 5xx): the row stays for the next
    /// flush.
    Retry,
}

/// One paired hub in [`FabricStatus::paired`] (Layer 7 §5.3 amendment 11;
/// the daemon's `RelayStatus.paired_hubs[]`, §13.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairView {
    /// The record.
    pub hub: PairedHub,
    /// [`PairedHub::whole`] with the hub role on: a forward target and
    /// origin (`false` with the hub role off, whatever the record holds).
    pub whole: bool,
    /// It answered `REJECT_UNENROLLED` this session ("the other hub no
    /// longer lists this hub", §5.9): no forward target until paired again.
    pub broken: bool,
    /// `tcp`: a connection is attributed to it (it answered on it since it
    /// came up); `carrier`: a verified frame from it arrived through the
    /// carrier this session. `false` with the hub role off.
    pub reachable: bool,
    /// Resident forward rows whose turn is this pair, plus the
    /// `CUSTODY_ACK`s owed to it through the carrier. Rows beyond the
    /// resident window ([`FORWARD_RESIDENT`]) are in the store, not yet
    /// offered, and counted by no pair; `Store::forward_stats` has the
    /// totals.
    pub pending: u32,
    /// Carrier re-posts this session (§5.9 C6).
    pub reposts: u32,
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
    /// Forward pending / target up (Layer 7 §5.9): frames are owed to the
    /// `carrier` pair `target` ([`Fabric::forward_take`] would return
    /// `pending` of them now). Coalesced per target per tick (the actor's
    /// input batch); emitted when a row's turn or an ack reaches the pair,
    /// its `REJECT_FULL` rows are retried, [`Fabric::forward_repost`]
    /// re-arms a post, or the carrier bound is set; never on a timer, never
    /// for a row that merely waits after a `Retry`, and never for re-post
    /// rule (1), which the next [`Fabric::forward_take`] applies itself.
    Forward {
        /// The `carrier` pair.
        target: NodeId,
        /// Frames owed to it.
        pending: u32,
    },
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
    /// Hub: forward rows marked refused this session (Layer 7 §5.8: every
    /// target refused, or none left). A session count: it restarts at zero.
    pub forward_refused: u64,
    /// Hub: puts answered `REJECT_UNKNOWN_DEST` because their only targets
    /// were carrier pairs they do not fit (Layer 7 §5.2, C15).
    pub forward_too_large: u64,
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
    /// Hub: the paired hubs, in `paired_hubs` order (Layer 7 §5.3
    /// amendment 11: per-pair `reachable` and `pending`).
    pub paired: Vec<PairView>,
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
        let mut settings = settings;
        normalize_pairs(&mut settings, &own);
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
            offer_pair: None,
            enrolling: None,
            peer_dials: HashSet::new(),
            read_marks: HashMap::new(),
            counters: Counters::default(),
            fwd: Forwarder::default(),
            events: events.clone(),
            pending_events: BTreeMap::new(),
            foreign_convs: HashMap::new(),
            outq: Vec::new(),
            control: input.clone(),
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
            .call(move |c| c.enroll(Some(address), &code, card.as_deref(), None, tx))
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

/// Layer 7 (§4.6, §5.9): pairing, the carrier bound and the hand-off API.
/// Every call is a synchronous arm of the actor (it never awaits a pair);
/// pairing and the hand-off are refused without the hub role (§5.11).
impl Fabric {
    /// Hub side, Layer 7 §4.6 steps 3–4: enroll at the hub whose card is
    /// `card` with its one-time pairing `code` (minted there by
    /// [`Fabric::offer_pair`]), recording a pairing (`role` and `via` from
    /// this hub's side) instead of a home hub. The `ENROLL` rides a
    /// connection already attributed to that hub when one exists (the
    /// second hub of a pairing answers on the first hub's connection, S22),
    /// else a dial to `address`. Resolves on `ENROLL_OK` like
    /// [`Fabric::enroll`] (same refusals; dropping the wait abandons the
    /// attempt) with the record, whose `ours` is now set; the pair is whole
    /// (it forwards) once `theirs` is set too, when the far hub redeems this
    /// hub's pairing offer. Refused without the hub role, for this node or
    /// its home hub, for a `role`/`via` pair §4.6 forbids, for a hub
    /// recorded with another role or link, and beyond [`MAX_PAIRED_HUBS`].
    pub async fn pair(
        &self,
        address: Option<SocketAddr>,
        code: &str,
        card: &str,
        role: PairRole,
        via: PairVia,
    ) -> Result<PairedHub> {
        let code = code.to_owned();
        let card = card.to_owned();
        let (tx, rx) = oneshot::channel();
        let mut wait = EnrollWait {
            rx: Some(rx),
            input: self.inner.input.clone(),
        };
        let node = match self
            .call(move |c| c.pair(address, &code, &card, role, via, tx))
            .await?
        {
            Ok(n) => n,
            Err(e) => {
                wait.rx = None; // refused before any attempt opened
                return Err(e);
            }
        };
        wait.answer().await?;
        self.call(move |c| c.find_pair(&node))
            .await?
            .ok_or(FabricError::NotFound)
    }

    /// Hub side, Layer 7 §4.6 step 1: mint a one-time **pairing** code
    /// (replacing any open offer, member or pairing: one at a time). `role`
    /// and `via` are the far hub's, from this hub's side. The far hub's
    /// `ENROLL` that redeems it records (or completes) its [`PairedHub`]
    /// with `theirs` set and pins its card at `oob`, never `enrolled`: a
    /// paired hub is never a member. A member offer's code
    /// ([`Fabric::offer`]) never records a pair, and a recorded pair that
    /// presents one is refused; this node's home hub redeeming a pairing
    /// code is refused (amendment 12). `bundle` is `None`: the `111PAIR:`
    /// bundle (link key, role, via) is the daemon's (D6). Refused without
    /// the hub role and for a `role`/`via` pair §4.6 forbids.
    pub async fn offer_pair(
        &self,
        address: Option<SocketAddr>,
        role: PairRole,
        via: PairVia,
    ) -> Result<OfferView> {
        self.call(move |c| c.offer_pair(address, role, via)).await?
    }

    /// Layer 7 §13.1 `DELETE /relay/paired/{node_id}`: drop the pairing
    /// record (both enrollments) and everything held for the pair; its
    /// forward rows return to the other targets (refused when none is
    /// left). The far hub's contact is left as it is: a pairing pinned it
    /// at `oob`, so its puts meet `REJECT_UNENROLLED` here afterwards, the
    /// far hub's sign that this hub no longer lists it. `false` when no such
    /// pair is recorded.
    pub async fn unpair(&self, node_id: NodeId) -> Result<bool> {
        self.call(move |c| c.unpair(node_id)).await?
    }

    /// The carrier bound (Layer 7 §5.2, §6.1 item 3; set by the daemon's
    /// carrier, D11): the largest sealed frame one carrier message holds.
    /// In memory, never persisted. `None` (the default, and whenever no
    /// carrier is built): no `carrier` pair is a target for a new put.
    pub async fn set_carrier_max_frame(&self, max: Option<usize>) -> Result<()> {
        self.call(move |c| c.set_carrier_max_frame(max)).await
    }

    /// Sealed frames owed to `target`, a whole `carrier` pair (Layer 7
    /// §5.9), at most `max`: first the `CUSTODY_ACK`s owed to it
    /// (`custody_seq = None`, consumed by this call and never returned
    /// again, C7), then forward puts whose turn it is (`Some`), in custody
    /// order, each to be answered with [`Fabric::forward_done`]. A put
    /// already posted is returned again only once re-armed (§5.9 C6): by
    /// [`Fabric::forward_repost`], or by re-post rule (1), applied here
    /// when a verified frame from `target` came in through
    /// [`Fabric::carrier_inbound`] since the last take (at most once per
    /// row). A forwarder trigger. `Invalid` for anything but a whole
    /// `carrier` pair, or with the hub role off.
    pub async fn forward_take(&self, target: NodeId, max: usize) -> Result<Vec<CarrierFrame>> {
        self.call(move |c| c.forward_take(target, max)).await?
    }

    /// What the carrier did with a put it took (Layer 7 §5.9). A row that
    /// was committed or refused meanwhile, or never taken, is ignored.
    pub async fn forward_done(&self, custody_seq: u64, done: ForwardDone) -> Result<()> {
        self.call(move |c| c.forward_done(custody_seq, done)).await
    }

    /// A sealed frame from `from`, a `carrier` pair, that arrived through
    /// the carrier (Layer 7 §5.9): verified like any sealed packet (size
    /// first, then `pair_mac`, then the L2 window: an already-processed
    /// frame is stale and dropped), processed with no connection the way a
    /// mailbox item is (INTRO allowed). Only `CUSTODY_PUT` (the `fwd = 3`
    /// branch of the gate) and `CUSTODY_ACK` are read; anything else, and
    /// any frame not from `from`, is dropped and counted. `Invalid` for a
    /// sender that is not a whole `carrier` pair, or with the hub role off.
    pub async fn carrier_inbound(&self, from: NodeId, frame: Vec<u8>) -> Result<()> {
        if !(MIN_PACKET_LEN..=MAX_PACKET_LEN).contains(&frame.len()) {
            return Err(FabricError::Invalid("carrier frame size"));
        }
        self.call(move |c| c.carrier_inbound(from, &frame)).await?
    }

    /// Re-arm every posted, unacknowledged put to `target` (Layer 7 §5.9
    /// re-post triggers (2) "Flush now" and (3) a `since` miss; rule (1)
    /// is the fabric's own, rule (4) is a restart): the next
    /// [`Fabric::forward_take`] returns them again, and each counts as a
    /// re-post. Returns how many were re-armed.
    pub async fn forward_repost(&self, target: NodeId) -> Result<u32> {
        self.call(move |c| c.forward_repost(target)).await?
    }

    /// A test hook, not an API: the §5.1 target computations this session
    /// (the gate's and the forwarder's). The forwarder recomputes a row's
    /// targets only when its inputs changed, so a drain of N rows costs
    /// O(N) of them (`tests/forward.rs` asserts ≤ 3 N).
    #[doc(hidden)]
    pub async fn forward_target_computations(&self) -> Result<u64> {
        self.call(|c| c.fwd.computed).await
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
    /// The forwarder's self-wake (module doc "The resident window"): a
    /// tick that left the swap probe with more of the store to read queues
    /// one of these to its own mailbox (never a timer; at most one queued),
    /// so the next tick reads on while every other input interleaves.
    Continue,
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
    /// Hub: a forward put (its `custody_seq`) to a `tcp` pair (Layer 7 §5.9).
    Forward(u64),
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

/// The member's open enrollment attempt (§7.3; one at a time), or a hub's
/// pairing attempt (Layer 7 §4.6 step 3: `pair` set).
struct Enrolling {
    hub: NodeId,
    code: Code,
    /// The address dialed when no connection to `hub` was sendable (`None`
    /// only for a pairing that rides an existing connection).
    address: Option<SocketAddr>,
    /// A pairing: the record `ENROLL_OK` writes instead of a home hub.
    pair: Option<PairedHub>,
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
    /// One `carrier` pair's "forward pending" (Layer 7 §5.9; the latest
    /// count of the tick wins).
    Forward([u8; 32]),
}

/// Where a `CUSTODY_PUT` arrived (Layer 7 §5.1: a put against the pair's
/// recorded link meets today's gate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrival {
    /// On a `tcp-lan` connection attributed to the sender.
    Tcp,
    /// Through the carrier ([`Fabric::carrier_inbound`]).
    Carrier,
}

/// The gate's verdict on a put for a `dest` (Layer 7 §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gate {
    /// A member here: today's row (`fwd = 0`).
    Member,
    /// A forward row (a target exists under the §5.1 table).
    Forward(Fwd),
    /// `REJECT_UNKNOWN_DEST`; `too_large` when the only targets were carrier
    /// pairs the item does not fit (counted, C15).
    Unknown { too_large: bool },
}

/// Admission (the gate) or a held row (the forwarder): a carrier pair with
/// no bound set is no target for a new put, and a down target for a row
/// already held (module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetMode {
    Admit,
    Held,
}

/// [`forward_targets`]'s answer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Targets {
    /// The §5.1 targets in offer order.
    list: Vec<NodeId>,
    /// Empty because every carrier target was too small (counted, C15).
    too_large: bool,
}

/// The §5.1 targets of a forward row in offer order (Layer 7 §5.2, §5.5).
/// `pairs` are the whole, unbroken pairs in `paired_hubs` order; `fwd` the
/// row's table row (1 from a member, 2 from a leaf, 3 from the carrier);
/// `hint` the received `dest_hub`; `size` the item's sealed bytes;
/// `cached` the hit cache's pair for the dest.
///
/// - `fwd = 1`: a hint naming a pair ⇒ that pair first, then the probe
///   order; a hint naming no pair ⇒ this hub's `gateway` pairs, then the
///   fitting `carrier` pairs (upstream only); no hint ⇒ the probe order:
///   `tcp` `peer` and `leaf` pairs, then `gateway`, then fitting `carrier`
///   pairs, a cache hit first.
/// - `fwd = 2`: the fitting `carrier` pairs, the hint's first.
/// - `fwd = 3`: the `leaf` pairs, the hint's (or the cache's) first; a hint
///   naming no leaf ⇒ none.
fn forward_targets(
    pairs: &[PairedHub],
    fwd: u8,
    hint: Option<[u8; DEST_HUB_LEN]>,
    size: usize,
    max_frame: Option<usize>,
    mode: TargetMode,
    cached: Option<NodeId>,
) -> Targets {
    let named = |p: &PairedHub| hint.is_some_and(|h| h[..] == p.node_id.as_bytes()[..DEST_HUB_LEN]);
    // A carrier pair is a target only for a put that fits its bound (§5.2).
    let mut too_large = false;
    let mut carriers: Vec<NodeId> = Vec::new();
    for p in pairs.iter().filter(|p| p.via == PairVia::Carrier) {
        let ok = match max_frame {
            None => mode == TargetMode::Held,
            Some(_) => carrier_fits(max_frame, size),
        };
        if ok {
            carriers.push(p.node_id);
        } else if max_frame.is_some() {
            too_large = true;
        }
    }
    let near =
        |p: &PairedHub| p.via == PairVia::Tcp && matches!(p.role, PairRole::Peer | PairRole::Leaf);
    let upstream = |p: &PairedHub| p.via == PairVia::Tcp && p.role == PairRole::Gateway;
    let pick = |f: &dyn Fn(&PairedHub) -> bool| -> Vec<NodeId> {
        pairs.iter().filter(|p| f(p)).map(|p| p.node_id).collect()
    };
    let (mut list, first) = match fwd {
        1 => {
            let named_pair = pairs
                .iter()
                .find(|p| named(p) && (p.via == PairVia::Tcp || carriers.contains(&p.node_id)))
                .map(|p| p.node_id);
            if hint.is_some() && named_pair.is_none() {
                let mut l = pick(&upstream);
                l.extend(carriers.iter().copied());
                (l, None)
            } else {
                let mut l = pick(&near);
                l.extend(pick(&upstream));
                l.extend(carriers.iter().copied());
                (l, named_pair.or(cached))
            }
        }
        2 => {
            let first = pairs
                .iter()
                .find(|p| named(p) && carriers.contains(&p.node_id))
                .map(|p| p.node_id);
            (carriers.clone(), first)
        }
        3 => {
            let leaves = pick(&|p: &PairedHub| p.via == PairVia::Tcp && p.role == PairRole::Leaf);
            match hint {
                Some(_) => match pairs
                    .iter()
                    .find(|p| named(p) && leaves.contains(&p.node_id))
                {
                    Some(p) => (leaves, Some(p.node_id)),
                    None => (Vec::new(), None),
                },
                None => (leaves, cached),
            }
        }
        _ => (Vec::new(), None),
    };
    if let Some(f) = first {
        if let Some(i) = list.iter().position(|n| *n == f) {
            let n = list.remove(i);
            list.insert(0, n);
        }
    }
    let mut seen = HashSet::new();
    list.retain(|n| seen.insert(*n));
    let too_large = list.is_empty() && too_large && mode == TargetMode::Admit;
    Targets { list, too_large }
}

/// Where a forward row is with its current target (Layer 7 §5.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    /// Nothing out: offered to its turn when that is up (and has room).
    Idle,
    /// A put written to a `tcp` pair, awaiting its `CUSTODY_ACK`.
    InFlight,
    /// The turn answered `REJECT_FULL`: the row stays with it until a retry
    /// trigger (module doc; C16).
    Full,
    /// A carrier took it ([`Fabric::forward_take`]) and has not answered.
    Taken,
    /// The carrier posted it (`Taken`); `repost` once re-armed (§5.9 C6).
    Posted { repost: bool },
}

/// The set of its turn's [`TargetQ`] a resident row sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Idle,
    InFlight,
    Full,
    Taken,
    /// Posted, no verified far frame since the post.
    Posted,
    /// Posted, and a verified frame from the far hub came after the post
    /// with no ack for it (a far-hub round): §5.9 rule (1) re-arms it at
    /// the next [`Fabric::forward_take`].
    Heard,
    /// Re-armed: owed to the carrier again.
    Armed,
}

/// One protected forward row the forwarder holds in memory (one of the at
/// most [`FORWARD_RESIDENT`] of the resident window, module doc).
#[derive(Debug, Clone)]
struct FwdRow {
    dest: [u8; 32],
    /// The stored `pkt_ref`: the pending map's key, so forgetting a row is
    /// one lookup.
    pkt_ref: [u8; 16],
    /// The `dest_hub` TLV it arrived with, repeated on every forward put
    /// (§5.4; lost at a restart and for a row paged in from the store).
    hint: Option<[u8; DEST_HUB_LEN]>,
    /// The §5.1 table row (1–3).
    fwd: u8,
    /// The item's sealed bytes.
    size: usize,
    /// Its current target.
    turn: Option<NodeId>,
    state: Turn,
    /// Targets that refused it (a `tcp` pair's only for its connection).
    refused: Vec<NodeId>,
    /// Not yet given a turn (its first turn is a retry trigger for that
    /// pair's `REJECT_FULL` rows: "a row insert").
    fresh: bool,
    /// Its turn's pair was unpaired or broken: with no target left it is
    /// refused rather than kept waiting.
    orphaned: bool,
    /// §5.9 re-post rule (1): a verified frame from its turn came after
    /// its (re-)post. Set once per far-hub round (only a row in the posted
    /// set takes it, at the first far frame after its post), cleared by the
    /// re-arm and by every new post: one re-post per row per far-hub round,
    /// never two within one (C6, §15 G10 item 3).
    heard: bool,
}

impl FwdRow {
    fn slot(&self) -> Slot {
        match self.state {
            Turn::Idle => Slot::Idle,
            Turn::InFlight => Slot::InFlight,
            Turn::Full => Slot::Full,
            Turn::Taken => Slot::Taken,
            Turn::Posted { repost: true } => Slot::Armed,
            Turn::Posted { repost: false } if self.heard => Slot::Heard,
            Turn::Posted { repost: false } => Slot::Posted,
        }
    }
}

/// The resident rows whose turn is one target, by where they stand, each
/// set in custody order: the per-target queues the forwarder works from
/// (no pass over every row; counts are set sizes).
#[derive(Debug, Default)]
struct TargetQ {
    idle: BTreeSet<u64>,
    inflight: BTreeSet<u64>,
    full: BTreeSet<u64>,
    taken: BTreeSet<u64>,
    posted: BTreeSet<u64>,
    heard: BTreeSet<u64>,
    armed: BTreeSet<u64>,
}

impl TargetQ {
    fn set(&mut self, s: Slot) -> &mut BTreeSet<u64> {
        match s {
            Slot::Idle => &mut self.idle,
            Slot::InFlight => &mut self.inflight,
            Slot::Full => &mut self.full,
            Slot::Taken => &mut self.taken,
            Slot::Posted => &mut self.posted,
            Slot::Heard => &mut self.heard,
            Slot::Armed => &mut self.armed,
        }
    }

    fn len(&self) -> usize {
        self.idle.len()
            + self.inflight.len()
            + self.full.len()
            + self.taken.len()
            + self.posted.len()
            + self.heard.len()
            + self.armed.len()
    }

    fn all(&self) -> Vec<u64> {
        let mut v: Vec<u64> = Vec::with_capacity(self.len());
        for s in [
            &self.idle,
            &self.inflight,
            &self.full,
            &self.taken,
            &self.posted,
            &self.heard,
            &self.armed,
        ] {
            v.extend(s.iter().copied());
        }
        v
    }
}

/// The hit cache `dest → pair` (Layer 7 §5.5): at most [`FORWARD_CACHE`]
/// entries, oldest evicted, in memory only. A map for the lookup and an
/// insertion log for the eviction order (stale log entries are skipped and
/// compacted), so a lookup and an insert are O(1) amortized.
#[derive(Debug, Default)]
struct HitCache {
    map: HashMap<[u8; 32], (NodeId, u64)>,
    order: VecDeque<([u8; 32], u64)>,
    stamp: u64,
}

impl HitCache {
    fn get(&self, dest: &[u8; 32]) -> Option<NodeId> {
        self.map.get(dest).map(|(p, _)| *p)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.map.len()
    }

    /// Record a hit; `true` when the dest's entry changed (new, or another
    /// pair).
    fn put(&mut self, dest: [u8; 32], pair: NodeId) -> bool {
        self.stamp += 1;
        let changed = self.map.get(&dest).is_none_or(|(p, _)| *p != pair);
        self.map.insert(dest, (pair, self.stamp));
        self.order.push_back((dest, self.stamp));
        while self.map.len() > FORWARD_CACHE {
            let Some((d, s)) = self.order.pop_front() else {
                break;
            };
            if self.map.get(&d).is_some_and(|(_, st)| *st == s) {
                self.map.remove(&d);
            }
        }
        if self.order.len() > 2 * FORWARD_CACHE {
            let map = &self.map;
            self.order
                .retain(|(d, s)| map.get(d).is_some_and(|(_, st)| st == s));
        }
        changed
    }

    /// Drop every entry naming `pair`.
    fn forget_pair(&mut self, pair: &NodeId) {
        self.map.retain(|_, (p, _)| p != pair);
        let map = &self.map;
        self.order
            .retain(|(d, s)| map.get(d).is_some_and(|(_, st)| st == s));
    }
}

/// The forwarder's in-memory state (Layer 7 §5.5, §5.9): lost at a
/// restart; the store keeps the rows (module doc). Every structure is
/// bounded by the resident window ([`FORWARD_RESIDENT`] rows) or by
/// [`MAX_PAIRED_HUBS`].
#[derive(Debug, Default)]
struct Forwarder {
    /// The carrier bound ([`Fabric::set_carrier_max_frame`]).
    carrier_max: Option<usize>,
    /// The resident window: protected forward rows by `custody_seq`.
    rows: BTreeMap<u64, FwdRow>,
    /// The resident rows by turn and state (the per-target queues).
    targets: HashMap<NodeId, TargetQ>,
    /// Resident idle rows with no turn (no target exists for them now:
    /// they wait, and are evictable like a down target's).
    unturned: BTreeSet<u64>,
    /// Resident rows by dest (a cache hit re-targets that dest's rows).
    by_dest: HashMap<[u8; 32], BTreeSet<u64>>,
    /// Resident rows each target refused (a `tcp` pair's refusals end with
    /// its connection).
    refused_by: HashMap<NodeId, BTreeSet<u64>>,
    /// Rows whose targets the next pass recomputes (their inputs changed).
    dirty: BTreeSet<u64>,
    /// The pair set or the carrier bound changed: every idle row is
    /// recomputed by the next pass.
    all_dirty: bool,
    /// Protected rows above `loaded` may wait in the store, not resident
    /// (the window was full when they came, or they were evicted): paged
    /// in custody order.
    more: bool,
    /// The paging floor: every protected row at or below it is resident
    /// (an eviction moves it back below the evicted row; a page moves it
    /// up past what it read).
    loaded: u64,
    /// The swap probe (module doc "The resident window"): rows of the
    /// store above `loaded` and at or below `probe` were read while the
    /// window was full and found unable to move.
    probe: u64,
    /// The probe read to the end of the store and found nothing it could
    /// swap in: no store read for it until a target comes up or a stored
    /// row's turn can have changed ([`Forwarder::restart_probe`]).
    probe_done: bool,
    /// The targets that were up when the probe last looked: one coming up
    /// restarts the probe from `loaded`.
    up: HashSet<NodeId>,
    /// The last store read of this tick's runs (a page in or a probe page)
    /// was full: more of the store is left to read.
    read_on: bool,
    /// A self-wake ([`Input::Continue`]) is queued: at most one at a time.
    wake: bool,
    /// The pending map `pkt_ref → (custody_seq, target)` of puts out.
    pending: HashMap<[u8; 16], (u64, NodeId)>,
    /// The hit cache `dest → pair`.
    cache: HitCache,
    /// Sealed `CUSTODY_ACK`s owed to each `carrier` pair.
    acks: HashMap<NodeId, VecDeque<Vec<u8>>>,
    /// Pairs that answered `REJECT_UNENROLLED` this session.
    broken: HashSet<NodeId>,
    /// `carrier` pairs heard through the carrier this session.
    heard: HashSet<NodeId>,
    /// Re-posts per `carrier` pair this session.
    reposts: HashMap<NodeId, u32>,
    /// Dials a pairing attempt started, kept while the pair is not whole
    /// (the far hub's `ENROLL` rides them, §4.6 step 3).
    dials: HashMap<NodeId, SocketAddr>,
    /// A run is owed at the end of the tick.
    run: bool,
    /// Pairs whose `REJECT_FULL` rows the next run retries.
    retry_full: HashSet<NodeId>,
    /// `carrier` pairs whose owed frames changed this tick.
    notify: BTreeSet<NodeId>,
    /// §5.1 target computations this session (a diagnostic: the forwarder
    /// recomputes only rows whose inputs changed;
    /// [`Fabric::forward_target_computations`]).
    computed: u64,
}

impl Forwarder {
    /// Take row `seq` out of its turn's queue.
    fn unplace(&mut self, seq: u64) {
        let Some((turn, slot)) = self.rows.get(&seq).map(|r| (r.turn, r.slot())) else {
            return;
        };
        let Some(t) = turn else {
            self.unturned.remove(&seq);
            return;
        };
        if let Some(q) = self.targets.get_mut(&t) {
            q.set(slot).remove(&seq);
            if q.len() == 0 {
                self.targets.remove(&t);
            }
        }
    }

    /// Put row `seq` into its turn's queue (a row with no turn is idle and
    /// sits in `unturned` once its targets were computed; a fresh row sits
    /// in none until then).
    fn place(&mut self, seq: u64) {
        match self.rows.get(&seq).map(|r| (r.turn, r.slot(), r.fresh)) {
            Some((Some(t), slot, _)) => {
                self.targets.entry(t).or_default().set(slot).insert(seq);
            }
            Some((None, _, false)) => {
                self.unturned.insert(seq);
            }
            _ => {}
        }
    }

    /// Change row `seq`; its queue entry follows.
    fn mutate(&mut self, seq: u64, f: impl FnOnce(&mut FwdRow)) {
        self.unplace(seq);
        if let Some(r) = self.rows.get_mut(&seq) {
            f(r);
        }
        self.place(seq);
    }

    fn set_state(&mut self, seq: u64, state: Turn) {
        self.mutate(seq, |r| r.state = state);
    }

    /// The put of row `seq` is no longer out: its pending entry goes.
    fn unpend(&mut self, seq: u64) {
        if let Some(r) = self.rows.get(&seq) {
            if self.pending.get(&r.pkt_ref).is_some_and(|(s, _)| *s == seq) {
                self.pending.remove(&r.pkt_ref);
            }
        }
    }

    /// A new resident row (its targets are computed by the next pass).
    fn insert(&mut self, seq: u64, row: FwdRow) {
        self.by_dest.entry(row.dest).or_default().insert(seq);
        self.rows.insert(seq, row);
        self.place(seq);
        self.dirty.insert(seq);
        self.run = true;
    }

    /// The resident row the window gives up first when a row that can move
    /// must enter (module doc "The resident window"): the newest idle (or
    /// `REJECT_FULL`) row whose turn is a target not in `up`, or that has no
    /// target now, with its targets settled (not dirty). Newest first per
    /// target, so each target's resident rows stay older than its rows in
    /// the store: it is offered its rows in custody order when it comes up.
    /// Among those candidates (one per target, one with no target) a row
    /// that loses nothing by leaving memory goes first: no hint, no
    /// refusal, not orphaned (eviction drops all three); with none such,
    /// the newest candidate. O(targets).
    fn evictable(&self, up: &HashSet<NodeId>) -> Option<u64> {
        let settled = |s: &&u64| !self.dirty.contains(s);
        let plain = |s: u64| {
            self.rows
                .get(&s)
                .is_some_and(|r| r.hint.is_none() && r.refused.is_empty() && !r.orphaned)
        };
        let (mut best, mut best_plain) = (None, None);
        let mut see = |cand: Option<u64>| {
            if let Some(s) = cand {
                best = best.max(Some(s));
                if plain(s) {
                    best_plain = best_plain.max(Some(s));
                }
            }
        };
        see(self.unturned.iter().rev().find(settled).copied());
        for (t, q) in &self.targets {
            if up.contains(t) {
                continue;
            }
            // Idle, or kept after a `REJECT_FULL` (retried on the pair's
            // next link-up either way, C16): the target's newest of both.
            let idle = q.idle.iter().rev().find(settled).copied();
            let full = q.full.iter().rev().find(settled).copied();
            see(idle.max(full));
        }
        best_plain.or(best)
    }

    /// Every stored row is judged again by the swap probe (module doc "The
    /// resident window"): a change that can turn a stored row's turn up (the
    /// pair set, a pair break, the carrier bound, a changed hit-cache entry)
    /// restarts it from the paging floor, so the rows already in the store
    /// move before a newer row for the same target is admitted.
    fn restart_probe(&mut self) {
        self.probe = self.loaded;
        self.probe_done = false;
    }

    /// The pair set or the carrier bound changed: every idle row is
    /// recomputed by the next pass, and the probe judges the store again.
    fn all_changed(&mut self) {
        self.all_dirty = true;
        self.restart_probe();
    }

    /// Evict the idle row `seq` from the window: it is in the store already
    /// and leaves memory without a refusal; paging resumes below it.
    fn evict(&mut self, seq: u64) {
        if self.untrack(seq).is_some() {
            self.loaded = self.loaded.min(seq.saturating_sub(1));
            self.more = true;
        }
    }

    /// Forget a row (committed, refused, or gone from the store): every
    /// index entry goes with it, each one lookup.
    fn untrack(&mut self, seq: u64) -> Option<FwdRow> {
        self.unpend(seq);
        self.unplace(seq);
        let r = self.rows.remove(&seq)?;
        if let Some(s) = self.by_dest.get_mut(&r.dest) {
            s.remove(&seq);
            if s.is_empty() {
                self.by_dest.remove(&r.dest);
            }
        }
        for t in &r.refused {
            if let Some(s) = self.refused_by.get_mut(t) {
                s.remove(&seq);
                if s.is_empty() {
                    self.refused_by.remove(t);
                }
            }
        }
        self.dirty.remove(&seq);
        Some(r)
    }

    /// `target` refused row `seq` (§5.5, §5.8): noted, the row loses its
    /// turn and the next pass gives it the next one.
    fn refused(&mut self, seq: u64, target: NodeId) {
        if !self.rows.contains_key(&seq) {
            return;
        }
        self.unpend(seq);
        self.mutate(seq, |r| {
            if !r.refused.contains(&target) {
                r.refused.push(target);
            }
            r.turn = None;
            r.state = Turn::Idle;
        });
        self.refused_by.entry(target).or_default().insert(seq);
        self.dirty.insert(seq);
        self.run = true;
    }

    /// Rows whose turn was `node` lose it (every state) and return to the
    /// other targets at the next pass (refused there when none is left).
    fn orphan(&mut self, node: &NodeId) {
        let seqs = self
            .targets
            .remove(node)
            .map(|q| q.all())
            .unwrap_or_default();
        for seq in seqs {
            if let Some(r) = self.rows.get_mut(&seq) {
                if self.pending.get(&r.pkt_ref).is_some_and(|(s, _)| *s == seq) {
                    self.pending.remove(&r.pkt_ref);
                }
                r.turn = None;
                r.state = Turn::Idle;
                r.orphaned = true;
                self.dirty.insert(seq);
                self.unturned.insert(seq);
            }
        }
        self.run = true;
    }

    /// A hit for `dest` at `pair` (§5.5): when the entry changed, the idle
    /// rows of that dest that `pair` has not refused and whose turn is
    /// another target are recomputed (the cache goes first).
    fn cache_put(&mut self, dest: [u8; 32], pair: NodeId) {
        if !self.cache.put(dest, pair) {
            return;
        }
        // Stored rows of that dest may now have an up turn (rare per dest;
        // the re-sweep is bounded per tick and paced by the self-wake).
        self.restart_probe();
        let Some(seqs) = self.by_dest.get(&dest) else {
            return;
        };
        for seq in seqs {
            if let Some(r) = self.rows.get(seq) {
                if r.state == Turn::Idle && r.turn != Some(pair) && !r.refused.contains(&pair) {
                    self.dirty.insert(*seq);
                }
            }
        }
        self.run = true;
    }

    /// Frames [`Fabric::forward_take`] would return to `target` now (the
    /// posts §5.9 rule (1) re-arms at that take included).
    fn owed(&self, target: &NodeId) -> u32 {
        let acks = self.acks.get(target).map_or(0, VecDeque::len);
        let rows = self
            .targets
            .get(target)
            .map_or(0, |q| q.idle.len() + q.armed.len() + q.heard.len());
        u32::try_from(acks + rows).unwrap_or(u32::MAX)
    }

    /// Resident rows whose turn is `target` (any state), plus acks owed to
    /// it.
    fn pending_for(&self, target: &NodeId) -> u32 {
        let acks = self.acks.get(target).map_or(0, VecDeque::len);
        let rows = self.targets.get(target).map_or(0, TargetQ::len);
        u32::try_from(acks + rows).unwrap_or(u32::MAX)
    }
}

/// What one forward put to a `tcp` pair came to.
enum PutOutcome {
    /// Written to the out queue (in flight).
    Sent,
    /// The row left the target's idle queue (gone, or refused there).
    Skipped,
    /// Nothing can go to this target now (no keys, no sealing, a store or
    /// sealer error): the rest waits for the next run.
    Blocked,
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
    /// The open offer is a pairing offer (Layer 7 §4.6 step 1): the far
    /// hub's role and link from this hub's side.
    offer_pair: Option<(PairRole, PairVia)>,
    enrolling: Option<Enrolling>,
    /// Addresses kept dialed for configured peers ([`Fabric::dial_peer`]).
    peer_dials: HashSet<SocketAddr>,
    read_marks: HashMap<NodeId, u64>,
    counters: Counters,
    /// Layer 7 §5.9: the forwarder (empty and idle on a non-hub node).
    fwd: Forwarder,
    events: broadcast::Sender<FabricEvent>,
    pending_events: BTreeMap<EventKey, FabricEvent>,
    /// Kind, pin and title of the foreign conversations seen this run (the
    /// §10 coalescing decision; a bounded cache refilled from the store).
    foreign_convs: HashMap<i64, ConvFacts>,
    outq: Vec<Out>,
    /// Keeps the control channel open while the actor runs (dropping every
    /// handle does not stop it, module doc); the forwarder's self-wake
    /// ([`Input::Continue`]) is queued on it.
    control: mpsc::UnboundedSender<Input>,
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
            Input::Continue => {
                self.fwd.wake = false;
                self.fwd.run = true;
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
        // Layer 7 §5.2, §5.9: the forwarder's puts queue after this tick's
        // CUSTODY_ACKs (the gate answered before it ran).
        self.run_forwarder();
        self.forward_events();
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
        // Layer 7 §5.9: a contact event is a forwarder trigger.
        if !self.fwd.rows.is_empty() {
            self.fwd.run = true;
        }
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
        // Layer 7 §5.9 (§15 G9 item 6): protected forward rows resume (the
        // first window of them; the rest page in as it drains).
        if self.settings.hub_role {
            self.load_forward_rows()?;
        }
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
        if self
            .whole_pair(&node)
            .is_some_and(|p| p.via == PairVia::Tcp)
        {
            self.pair_link_up(ck, node);
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
            if self.pair_of(&node).is_some() {
                self.pair_link_down(&node);
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
        self.hint_elsewhere(peer, hub).is_some()
    }

    /// The contact's 8 B home-hub hint when it names a hub that is not
    /// `hub` (`hinted_elsewhere`): the value of the `dest_hub` TLV a put to
    /// `hub` carries (Layer 7 §5.4).
    fn hint_elsewhere(&self, peer: &NodeId, hub: &NodeId) -> Option<[u8; DEST_HUB_LEN]> {
        match self.store.contact(peer.as_bytes()) {
            Ok(Some(c)) => c
                .home_hub
                .as_deref()
                .and_then(|h| <[u8; DEST_HUB_LEN]>::try_from(h).ok())
                .filter(|h| h[..] != hub.as_bytes()[..DEST_HUB_LEN]),
            _ => None,
        }
    }

    /// Layer 7 §5.3 amendment 8 as built (reason `forwarding`): the dest's
    /// hint names another hub than `hub` **and** names a hub this node
    /// knows: a contact whose NodeId starts with the hint and whose announce
    /// carries a hub role bit. A hint that names no hub this node knows (a
    /// stale one, a hub never met) claims nothing: plain `relayed` (the
    /// home hub's ACCEPTED does not say whether it holds the item for a
    /// member or for forwarding).
    fn forwarding_evident(&self, peer: &NodeId, hub: &NodeId) -> bool {
        let Some(h) = self.hint_elsewhere(peer, hub) else {
            return false;
        };
        match self.store.contacts() {
            Ok(rows) => rows.iter().any(|c| {
                c.node_id[..DEST_HUB_LEN] == h[..]
                    && c.node_id != hub.to_bytes()
                    && c.roles & (ROLE_MAILBOX | ROLE_ALWAYS_ON_HUB) != 0
            }),
            Err(_) => false,
        }
    }

    /// §5: the MTU a new message to `peer` is planned for, and the shim.
    fn plan_mtu(&self, peer: &NodeId) -> (usize, bool) {
        let path = match self.route(peer) {
            Route::Direct(ck) => Some((self.profile(&ck).0, 0)),
            // Layer 7 §5.4: room for the 11 B hint TLV when the put carries it.
            Route::Hub(ck, hub) => {
                let tlv = if self.hint_elsewhere(peer, &hub).is_some() {
                    DEST_HUB_ALONE_LEN
                } else {
                    0
                };
                Some((self.profile(&ck).0, CUSTODY_WRAP + tlv))
            }
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
        // Layer 7 §5.4: the dest's hint rides the put when it names another
        // hub (`hinted_elsewhere`); the item keeps room for it.
        let hint = self.hint_elsewhere(&peer, hub);
        let tlv = if hint.is_some() {
            DEST_HUB_ALONE_LEN
        } else {
            0
        };
        let wrap = CUSTODY_WRAP + tlv;
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
                        let first =
                            self.seal_data(keys, id, idx, room.saturating_sub(wrap), tcp, false);
                        // Layer 7 §5.4: a fragment planned before the dest's
                        // hint was known (`plan_mtu` reserved no TLV) goes
                        // without it, a locator only, rather than not at all.
                        let first = match first {
                            None if hint.is_some() => self.seal_data(
                                keys,
                                id,
                                idx,
                                room.saturating_sub(CUSTODY_WRAP),
                                tcp,
                                false,
                            ),
                            b => b,
                        };
                        let Some(b) = first else {
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
                let mut inner = Inner::new(
                    &self.own,
                    Kind::CustodyPut,
                    MsgSeq::CONTROL,
                    encode_put(&peer, &item),
                );
                inner.ext.dest_hub = hint;
                let Ok(mut sealed) =
                    seal_envelope(&*self.sealer, &hub_keys, Priority::Interactive, &inner)
                else {
                    self.counters.errors += 1;
                    continue;
                };
                if sealed.len() > room && inner.ext.dest_hub.is_some() {
                    // A locator only (§7.3): an item stored before the hint
                    // was known goes without it rather than not at all.
                    inner.ext.dest_hub = None;
                    match seal_envelope(&*self.sealer, &hub_keys, Priority::Interactive, &inner) {
                        Ok(s) => sealed = s,
                        Err(_) => {
                            self.counters.errors += 1;
                            continue;
                        }
                    }
                }
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
            (Tag::None | Tag::Push(_) | Tag::Commit(_) | Tag::Forward(_), Ok(())) => {}
            (Tag::Forward(seq), Err(e)) => {
                // Not written: the row waits for its turn's next connection.
                if self
                    .fwd
                    .rows
                    .get(&seq)
                    .is_some_and(|r| r.state == Turn::InFlight)
                {
                    self.fwd.unpend(seq);
                    self.fwd.set_state(seq, Turn::Idle);
                }
                self.on_send_error(ck, &e);
            }
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
        if !self.is_member(&member) || member.to_bytes() == wanted {
            return;
        }
        if let Ok(Some(c)) = self.store.contact(&wanted) {
            let id = NodeId::from(wanted);
            // Layer 7: a paired hub is never a member, so never a card here.
            if c.trust == Trust::Enrolled
                && Some(id) != self.home_hub()
                && self.find_pair(&id).is_none()
            {
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
            // Layer 7 §5.9: a whole pair's attribution runs the forwarder.
            if self.whole_pair(&sender).is_some() {
                self.fwd.retry_full.insert(sender);
                self.fwd.run = true;
            }
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
                    let hint = opened.inner.ext.dest_hub;
                    self.on_custody_put(ck, &sender, &opened.inner.payload, hint);
                }
            }
            Kind::CustodyAck => {
                let live = fresh == Freshness::Fresh && attributed;
                self.on_custody_ack(&sender, &opened.inner.payload, live);
            }
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

    /// §9 hub side, widened by Layer 7 §5.2 (amendment 1): a put on a
    /// connection attributed to `sender`, answered on it before anything is
    /// forwarded.
    fn on_custody_put(
        &mut self,
        ck: &ConnKey,
        sender: &NodeId,
        payload: &[u8],
        hint: Option<[u8; DEST_HUB_LEN]>,
    ) {
        let Some(v) = self.custody_decide(sender, Arrival::Tcp, payload, hint) else {
            return;
        };
        if let Some(bytes) = self.ack_payload(&v) {
            self.send_control(ck, sender, Kind::CustodyAck, bytes);
        }
        self.custody_after(&v);
    }

    /// The gate on one `CUSTODY_PUT` and what it stored (synchronous: it
    /// decides whether a target **exists**, holds the row and returns the
    /// receipt; it never awaits a pair, Layer 7 §5.2). `None`: the payload
    /// did not parse, or the store failed (nothing is answered, as before).
    fn custody_decide(
        &mut self,
        sender: &NodeId,
        arrival: Arrival,
        payload: &[u8],
        hint: Option<[u8; DEST_HUB_LEN]>,
    ) -> Option<PutVerdict> {
        let Ok((dest, item)) = decode_put(payload) else {
            return None;
        };
        let item = item.to_vec();
        let mut member = false;
        let mut forward = false;
        // Layer 7 §5.2: a member, or a live (whole, unbroken) pair; a half
        // pair's put meets REJECT_UNENROLLED.
        let admitted = self.is_member(sender) || self.live_pair(sender).is_some();
        let (status, custody_seq) = if !self.settings.holds_custody() || !admitted {
            (CustodyStatus::RejectUnenrolled, 0)
        } else {
            let fwd = match self.custody_gate(sender, arrival, &dest, hint, item.len()) {
                Gate::Member => Some(Fwd::Member),
                Gate::Forward(f) => Some(f),
                Gate::Unknown { too_large } => {
                    if too_large {
                        self.counters.forward_too_large += 1;
                    }
                    None
                }
            };
            match fwd {
                None => (CustodyStatus::RejectUnknownDest, 0),
                Some(fwd) => match self.store.custody_put_fwd(dest.as_bytes(), &item, fwd) {
                    Ok(CustodyPut::Accepted(s)) => {
                        if fwd.is_forward() {
                            let row = Self::fwd_row(
                                dest.to_bytes(),
                                pkt_ref(&item),
                                item.len(),
                                fwd.code(),
                                hint,
                            );
                            self.track_forward(s, row);
                            forward = true;
                        } else {
                            member = true;
                        }
                        (CustodyStatus::Accepted, s)
                    }
                    Ok(CustodyPut::Duplicate(s)) => (CustodyStatus::Duplicate, s),
                    Ok(CustodyPut::RejectFull) => (CustodyStatus::RejectFull, 0),
                    Err(_) => {
                        self.counters.errors += 1;
                        return None;
                    }
                },
            }
        };
        Some(PutVerdict {
            dest,
            receipt: CustodyReceipt {
                status,
                custody_seq,
                pkt_ref: pkt_ref(&item),
            },
            member,
            forward,
        })
    }

    /// The `CUSTODY_ACK` payload for a verdict.
    fn ack_payload(&self, v: &PutVerdict) -> Option<Vec<u8>> {
        CustodyAck::build(&*self.sealer, v.receipt, &v.dest, false)
            .and_then(|a| a.encode())
            .ok()
    }

    /// After the answer: a new member row streams to its member; a new
    /// forward row runs the forwarder (already asked by `track_forward`).
    fn custody_after(&mut self, v: &PutVerdict) {
        if v.member {
            self.push_to(&v.dest);
        }
        if v.member || v.forward {
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
        if !self.settings.holds_custody() {
            return;
        }
        let Ok(cursor) = decode_cursor(payload) else {
            return;
        };
        if let Some(p) = self.whole_pair(sender) {
            // Layer 7 (module doc): a whole pair's link-up fetch, or the
            // ping that ends a pairing. Any pairing dial to it goes (the
            // daemon's §4.6 step 5 holds a `tcp` pair's one connection).
            self.release_pair_dial(sender);
            if p.via == PairVia::Tcp {
                // It attributed this connection; one item-less answer
                // attributes the other side's, never an item (a pair is
                // never a member). A `carrier` pair gets nothing over `tcp`.
                self.send_control(ck, sender, Kind::MailboxItem, encode_item(cursor, &[]));
                self.fwd.retry_full.insert(*sender);
                self.fwd.run = true;
            }
            return;
        }
        if !self.is_member(sender) {
            return;
        }
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
        if !self.settings.holds_custody() || !self.is_member(sender) {
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
        let node = member.node_id;
        // Layer 7 §4.6: a pairing offer's code records the far hub as a pair
        // (`theirs`), never a member; a member offer's code never records a
        // pair, and a recorded pair is refused one; this node's home hub is
        // refused a pairing code (amendment 12). Checked before the code is
        // presented: a refused presenter neither spends nor locks the offer.
        let pairing = self.offer_pair;
        let refused = !self.settings.holds_custody()
            || match pairing {
                Some((role, via)) => {
                    !self.settings.hub_role
                        || Some(node) == self.home_hub()
                        || self.pair_fits(&node, role, via).is_err()
                }
                None => self.find_pair(&node).is_some(),
            };
        let status = if refused {
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
                    let trust = if pairing.is_some() {
                        Trust::Oob
                    } else {
                        Trust::Enrolled
                    };
                    match self.pin(&ann, card, trust) {
                        Ok(()) => {
                            // It sealed to our key: it holds our card.
                            self.heard.insert(member.node_id);
                            self.contact_event(member.node_id, ContactChange::Added);
                            self.fabric_changed();
                            if let Some((role, via)) = pairing {
                                self.record_theirs(node, role, via);
                            }
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

    /// `live`: the packet was fresh and its connection is attributed to
    /// `sender` (only then is a pair's receipt read).
    fn on_custody_ack(&mut self, sender: &NodeId, payload: &[u8], live: bool) {
        // Layer 7 §5.3 amendment 11: a whole `tcp` pair's receipt for a
        // forward put (today only the home hub's acks are read). A replayed
        // (stale) one, or one on a connection not attributed to it, changes
        // nothing; a `carrier` pair's come through `carrier_inbound` only.
        if self.find_pair(sender).is_some() {
            let tcp = self
                .whole_pair(sender)
                .is_some_and(|p| p.via == PairVia::Tcp);
            if live && tcp {
                if let Ok(ack) = CustodyAck::decode(payload) {
                    self.on_forward_ack(sender, &ack);
                }
            }
            return;
        }
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
        let Some((count, peer)) = self.tx.get(&id).map(|m| (m.count, m.peer)) else {
            return;
        };
        match ack.receipt.status {
            CustodyStatus::Accepted | CustodyStatus::Duplicate => {
                self.tracker.set_held(id, idx);
                if self.tracker.all_held(id, count) {
                    // §9: authenticated ACCEPTED from the home hub (a
                    // mailbox-role node) for a known member, or held for
                    // forwarding (Layer 7 §5.3 amendment 8) ⇒ relayed.
                    let reason = self
                        .forwarding_evident(&peer, sender)
                        .then_some(Reason::Forwarding);
                    self.set_state(id, MsgState::Relayed, reason);
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
        // Layer 7 §4.6 step 4: a pairing pins the far hub at `oob`, never
        // `enrolled` (a paired hub is never a member); the record says
        // whether it is whole. The hub role and the record's role and link
        // are checked again (they may have changed while it was open).
        if let Some(p) = en.pair {
            let fits = if self.settings.hub_role {
                self.pair_fits(&p.node_id, p.role, p.via)
            } else {
                Err(FabricError::Invalid("pairing needs the hub role"))
            };
            if let Err(e) = fits {
                return self.end_enroll(en, EnrollEnd::Refused(e));
            }
        }
        let trust = if en.pair.is_some() {
            Trust::Oob
        } else {
            Trust::Enrolled
        };
        if let Err(e) = self.pin(&ann, card, trust) {
            return self.end_enroll(en, EnrollEnd::Refused(e));
        }
        self.heard.insert(*sender);
        if let Some(p) = en.pair {
            // A pairing records the pair: no home hub, no cursor (am. 10).
            let dial = en.address.filter(|_| en.dialed);
            self.record_ours(p, dial, ck);
            self.contact_event(*sender, ContactChange::Updated);
            if let Some(r) = en.reply.take() {
                let _ = r.send(Ok(()));
            }
            return;
        }
        self.settings.home_hub = Some(HomeHub {
            node_id: *sender,
            address: en.address,
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
        // Layer 7: a forgotten paired hub is no longer paired.
        if self.find_pair(&id).is_some() {
            self.drop_pair(&id);
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
                    let id = NodeId::from(c.node_id);
                    // Layer 7 §5.3 amendment 10: pairs are not members.
                    c.trust == Trust::Enrolled
                        && Some(id) != self.home_hub()
                        && self.find_pair(&id).is_none()
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
            paired: self
                .settings
                .paired_hubs
                .iter()
                .map(|p| PairView {
                    hub: *p,
                    whole: self.settings.hub_role && p.whole(),
                    broken: self.fwd.broken.contains(&p.node_id),
                    reachable: self.settings.hub_role
                        && match p.via {
                            PairVia::Tcp => self.hub_answered(&p.node_id),
                            PairVia::Carrier => self.fwd.heard.contains(&p.node_id),
                        },
                    pending: self.fwd.pending_for(&p.node_id),
                    reposts: self.fwd.reposts.get(&p.node_id).copied().unwrap_or(0),
                })
                .collect(),
        })
    }

    /// §7.3 member side: open an attempt (replacing any open one). An `Err`
    /// opens nothing and pins nothing; once open, the attempt answers
    /// `reply` exactly once — on `ENROLL_OK`, or through
    /// [`Self::end_enroll`].
    fn enroll(
        &mut self,
        address: Option<SocketAddr>,
        code: &str,
        card: Option<&str>,
        pair: Option<PairedHub>,
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
        // Layer 7 §5.3 amendment 12: a paired hub is never a home hub.
        if pair.is_none() && self.find_pair(&hub).is_some() {
            return Err(FabricError::Invalid("a paired hub is never the home hub"));
        }
        // What the attempt this one replaces holds for the same hub or the
        // same address passes on to it (and is not undone with it).
        let (mut pinned, inherit_dial) = match &self.enrolling {
            Some(o) => (
                o.hub == hub && o.pinned,
                address.is_some() && o.address == address && o.dialed,
            ),
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
            pair,
            reply: Some(reply),
            dialed: inherit_dial,
            pinned,
        };
        match self.sendable_conn(&hub) {
            Some(ck) => {
                self.enrolling = Some(en);
                self.send_enroll(&ck);
            }
            None => match (self.dialer(), address) {
                (Some(d), Some(address)) => {
                    en.dialed |= !self.dial_held(address);
                    d.maintain(address, Some(hub));
                    self.enrolling = Some(en);
                }
                _ => self.end_enroll(en, EnrollEnd::NoLink),
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
        if let Some(address) = en.address.filter(|a| en.dialed && !self.dial_held(*a)) {
            if let Some(d) = self.dialer() {
                d.release(address);
            }
        }
        // A pair's card stays while it is paired (a refused re-pairing).
        if en.pinned && Some(en.hub) != self.home_hub() && self.find_pair(&en.hub).is_none() {
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
            || self.fwd.dials.values().any(|a| *a == address)
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
        self.offer_pair = None;
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
        self.offer_pair = None;
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
            let was = self.settings.hub_role;
            self.settings.hub_role = v;
            if was != v {
                self.hub_role_changed();
            }
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
// Layer 7: pairing, the gate's forward branch, the forwarder, the hand-off
// (`7-relay/README.md` §4.6, §5.1–§5.9; module doc "Pairing and cross-hub
// forwarding")
// ---------------------------------------------------------------------------

/// The gate's answer on one put and what it stored.
struct PutVerdict {
    dest: NodeId,
    receipt: CustodyReceipt,
    /// A new member row (`fwd = 0`): stream it to `dest`.
    member: bool,
    /// A new forward row (the forwarder runs at the end of the tick).
    forward: bool,
}

/// Layer 7 §4.6: the pairing list the fabric trusts: one record per hub,
/// never this node or its home hub (a paired hub is never a home hub), only
/// the combinations §4.6 step 2 allows, at most [`MAX_PAIRED_HUBS`] (later
/// entries dropped); each record keeps its `ours` and `theirs`. Applied to
/// the settings the daemon starts the fabric with; a list it never wrote is
/// not rewritten back.
fn normalize_pairs(s: &mut FabricSettings, own: &NodeId) {
    let home = s.home_hub.map(|h| h.node_id);
    let mut seen = HashSet::new();
    s.paired_hubs.retain(|p| {
        p.valid() && p.node_id != *own && Some(p.node_id) != home && seen.insert(p.node_id)
    });
    s.paired_hubs.truncate(MAX_PAIRED_HUBS);
}

impl Core {
    // -- pairs (§4.6) ------------------------------------------------------

    /// The pairing record of `node`, whatever this node's role (record
    /// keeping: forget, unpair, the member count, the card answer).
    fn find_pair(&self, node: &NodeId) -> Option<PairedHub> {
        self.settings
            .paired_hubs
            .iter()
            .find(|p| p.node_id == *node)
            .copied()
    }

    /// The pairing record of `node` while the hub role is on: every pairing
    /// and forwarding path starts here (Layer 7 §5.11; with the hub role off
    /// the records stay in the file and their rows wait in the store).
    fn pair_of(&self, node: &NodeId) -> Option<PairedHub> {
        if !self.settings.hub_role {
            return None;
        }
        self.find_pair(node)
    }

    /// A whole pair (both enrollments recorded, §4.6 step 4: "a pair
    /// forwards only when both sides hold it"), hub role on.
    fn whole_pair(&self, node: &NodeId) -> Option<PairedHub> {
        self.pair_of(node).filter(PairedHub::whole)
    }

    /// A whole pair that did not answer `REJECT_UNENROLLED` this session:
    /// a forward target, and a sender the gate admits.
    fn live_pair(&self, node: &NodeId) -> Option<PairedHub> {
        self.whole_pair(node)
            .filter(|p| !self.fwd.broken.contains(&p.node_id))
    }

    /// A member: enrolled here and not a paired hub (§5.2: a paired hub is
    /// never treated as a member; a hub that is both is a pair). Without
    /// pairs, exactly `is_enrolled`.
    fn is_member(&self, node: &NodeId) -> bool {
        self.is_enrolled(node) && self.find_pair(node).is_none()
    }

    /// The forward targets that exist now: the live pairs in `paired_hubs`
    /// order (the order the owner sees, §5.5).
    fn eligible_pairs(&self) -> Vec<PairedHub> {
        if !self.settings.hub_role {
            return Vec::new();
        }
        self.settings
            .paired_hubs
            .iter()
            .filter(|p| p.whole() && !self.fwd.broken.contains(&p.node_id))
            .copied()
            .collect()
    }

    /// The record `node` would get for `role`/`via`: refused when it is
    /// recorded with another role or link (§4.6 step 2, C3: nothing is
    /// changed), or when a new record would pass [`MAX_PAIRED_HUBS`].
    fn pair_fits(&self, node: &NodeId, role: PairRole, via: PairVia) -> Result<()> {
        match self.find_pair(node) {
            Some(p) if p.role != role || p.via != via => Err(FabricError::Invalid(
                "the pair is recorded with another role or link",
            )),
            Some(_) => Ok(()),
            None if self.settings.paired_hubs.len() >= MAX_PAIRED_HUBS => {
                Err(FabricError::Invalid("too many paired hubs"))
            }
            None => Ok(()),
        }
    }

    /// [`Fabric::pair`]: an enrollment attempt that records a pairing.
    fn pair(
        &mut self,
        address: Option<SocketAddr>,
        code: &str,
        card: &str,
        role: PairRole,
        via: PairVia,
        reply: oneshot::Sender<Result<()>>,
    ) -> Result<NodeId> {
        // §5.11: pairing is refused unless the hub role is on.
        if !self.settings.hub_role {
            return Err(FabricError::Invalid("pairing needs the hub role"));
        }
        let ann = verify_card(&parse_card_text(card)?)?;
        let node = ann.node_id();
        if node == self.own {
            return Err(CardError::OwnCard.into());
        }
        if Some(node) == self.home_hub() {
            return Err(FabricError::Invalid("the home hub is never a paired hub"));
        }
        let hub = PairedHub {
            node_id: node,
            role,
            via,
            ours: false,
            theirs: false,
        };
        if !hub.valid() {
            return Err(FabricError::Invalid(
                "a carrier pair is a peer; leaf and gateway pairs are tcp",
            ));
        }
        self.pair_fits(&node, role, via)?;
        self.enroll(address, code, Some(card), Some(hub), reply)?;
        Ok(node)
    }

    /// [`Fabric::offer_pair`]: a member offer's mint, marked a pairing.
    fn offer_pair(
        &mut self,
        address: Option<SocketAddr>,
        role: PairRole,
        via: PairVia,
    ) -> Result<OfferView> {
        if !self.settings.hub_role {
            return Err(FabricError::Invalid("pairing needs the hub role"));
        }
        let probe = PairedHub {
            node_id: self.own,
            role,
            via,
            ours: false,
            theirs: false,
        };
        if !probe.valid() {
            return Err(FabricError::Invalid(
                "a carrier pair is a peer; leaf and gateway pairs are tcp",
            ));
        }
        let mut view = self.offer(address)?;
        // The `111PAIR:` bundle is the daemon's (D6); a `111HUB:` one would
        // offer a pairing code to a member.
        view.bundle = None;
        self.offer_pair = Some((role, via));
        Ok(view)
    }

    /// Hub side of a pairing (§4.6 step 3): the far hub redeemed this hub's
    /// pairing offer. The record gets `theirs` (created with the offer's
    /// role and link when new); the far hub's card was pinned at `oob`.
    /// Its `ENROLL_OK` rides the connection, so no dial is released here.
    fn record_theirs(&mut self, node: NodeId, role: PairRole, via: PairVia) {
        let rec = match self
            .settings
            .paired_hubs
            .iter_mut()
            .find(|x| x.node_id == node)
        {
            Some(x) => {
                x.theirs = true;
                *x
            }
            None => {
                let p = PairedHub {
                    node_id: node,
                    role,
                    via,
                    ours: false,
                    theirs: true,
                };
                self.settings.paired_hubs.push(p);
                p
            }
        };
        self.pair_changed(rec);
    }

    /// This hub's `ENROLL_OK` of a pairing (§4.6 step 4; `p`'s role and
    /// link were checked against the record, [`Self::pair_fits`]): the
    /// record gets `ours`. While the pair is half the dial the attempt
    /// started is kept, so the far hub's `ENROLL` can ride it; once whole,
    /// no pairing dial is kept for any link (the daemon's §4.6 step 5 rule
    /// holds a `tcp` pair's one connection; a `carrier` pair is never
    /// dialed), and a `MAILBOX_FETCH {0}` on `ck` tells the far hub to
    /// release its own (module doc).
    fn record_ours(&mut self, p: PairedHub, dial: Option<SocketAddr>, ck: Option<&ConnKey>) {
        let rec = match self
            .settings
            .paired_hubs
            .iter_mut()
            .find(|x| x.node_id == p.node_id)
        {
            Some(x) => {
                x.ours = true;
                *x
            }
            None => {
                let r = PairedHub {
                    ours: true,
                    theirs: false,
                    ..p
                };
                self.settings.paired_hubs.push(r);
                r
            }
        };
        if let Some(a) = dial {
            self.fwd.dials.insert(rec.node_id, a);
        }
        if rec.whole() {
            if let Some(ck) = ck {
                self.send_control(ck, &rec.node_id, Kind::MailboxFetch, encode_cursor(0));
            }
            self.release_pair_dial(&rec.node_id);
        }
        self.pair_changed(rec);
    }

    /// A record was written: persisted through [`FabricEvent::Settings`]
    /// (the two bools ride it: the daemon learns the pair is whole there),
    /// and the forwarder re-reads the pair set.
    fn pair_changed(&mut self, rec: PairedHub) {
        self.fwd.broken.remove(&rec.node_id);
        if rec.whole() {
            self.fwd.retry_full.insert(rec.node_id);
        }
        self.fwd.all_changed();
        self.fwd.run = true;
        self.settings_changed();
        self.fabric_changed();
    }

    /// The dial a pairing attempt started for `node` is no longer needed:
    /// released unless the home hub, a configured peer or another pair's
    /// attempt holds the address.
    fn release_pair_dial(&mut self, node: &NodeId) {
        if let Some(a) = self.fwd.dials.remove(node) {
            if !self.dial_held(a) {
                if let Some(d) = self.dialer() {
                    d.release(a);
                }
            }
        }
    }

    /// [`Fabric::unpair`]: the record and everything held for the pair go;
    /// the far hub's contact is left as it is (a pairing pinned it at `oob`,
    /// never `enrolled`; a member before stays a member).
    fn unpair(&mut self, node: NodeId) -> Result<bool> {
        if self.find_pair(&node).is_none() {
            return Ok(false);
        }
        self.drop_pair(&node);
        Ok(true)
    }

    /// The record goes; rows whose turn it was return to the other targets
    /// (§13.1 `DELETE /relay/paired`), everything held for it is dropped.
    fn drop_pair(&mut self, node: &NodeId) {
        self.settings.paired_hubs.retain(|p| p.node_id != *node);
        self.fwd.orphan(node);
        if let Some(seqs) = self.fwd.refused_by.remove(node) {
            for seq in seqs {
                if let Some(r) = self.fwd.rows.get_mut(&seq) {
                    r.refused.retain(|n| n != node);
                }
            }
        }
        self.fwd.broken.remove(node);
        self.fwd.heard.remove(node);
        self.fwd.reposts.remove(node);
        self.fwd.acks.remove(node);
        self.fwd.notify.remove(node);
        self.fwd.retry_full.remove(node);
        self.fwd.cache.forget_pair(node);
        self.fwd.all_changed();
        self.release_pair_dial(node);
        self.settings_changed();
        self.fabric_changed();
    }

    /// `REJECT_UNENROLLED` from a pair (§5.9): "the other hub no longer
    /// lists this hub". It is no target this session (until paired again);
    /// its rows return to the other targets.
    fn pair_broken(&mut self, node: NodeId) {
        self.fwd.broken.insert(node);
        self.fwd.orphan(&node);
        self.fwd.all_changed();
        self.fabric_changed();
    }

    /// A whole `tcp` pair's outbound connection came up (module doc): one
    /// `MAILBOX_FETCH {0}` so the far side attributes it, then a run.
    fn pair_link_up(&mut self, ck: &ConnKey, node: NodeId) {
        self.send_control(ck, &node, Kind::MailboxFetch, encode_cursor(0));
        self.fwd.retry_full.insert(node);
        self.fwd.run = true;
    }

    /// A pair's connection went: its puts in flight wait for the next one,
    /// and a `tcp` pair's refusals end with their connection (§5.5); a
    /// whole pair's pairing dial is released (never redialed).
    fn pair_link_down(&mut self, node: &NodeId) {
        let tcp = self.find_pair(node).is_some_and(|p| p.via == PairVia::Tcp);
        if tcp {
            if let Some(seqs) = self.fwd.refused_by.remove(node) {
                for seq in seqs {
                    if let Some(r) = self.fwd.rows.get_mut(&seq) {
                        r.refused.retain(|n| n != node);
                        if r.state == Turn::Idle {
                            self.fwd.dirty.insert(seq);
                        }
                    }
                }
            }
        }
        let out: Vec<u64> = self
            .fwd
            .targets
            .get(node)
            .map(|q| q.inflight.iter().copied().collect())
            .unwrap_or_default();
        for seq in out {
            self.fwd.unpend(seq);
            self.fwd.set_state(seq, Turn::Idle);
        }
        if self.whole_pair(node).is_some() {
            self.release_pair_dial(node);
        }
        self.fwd.run = true;
    }

    /// The hub role was switched (Layer 7 §5.11): off, every pairing
    /// and forwarding path stops, the resident rows leave memory (they wait
    /// in the store, never refused) and the pairing dials are released; on,
    /// the rows page in again from the store.
    fn hub_role_changed(&mut self) {
        if self.settings.hub_role {
            self.fwd.more = true;
            self.fwd.loaded = 0;
            self.fwd.probe = 0;
            self.fwd.probe_done = false;
            self.fwd.all_dirty = true;
            self.fwd.run = true;
            let whole: Vec<NodeId> = self
                .settings
                .paired_hubs
                .iter()
                .filter(|p| p.whole())
                .map(|p| p.node_id)
                .collect();
            self.fwd.retry_full.extend(whole);
        } else {
            let dials: Vec<SocketAddr> = self.fwd.dials.drain().map(|(_, a)| a).collect();
            let old = std::mem::take(&mut self.fwd);
            self.fwd = Forwarder {
                carrier_max: old.carrier_max,
                broken: old.broken,
                heard: old.heard,
                reposts: old.reposts,
                computed: old.computed,
                // A self-wake may still be queued: at most one at a time.
                wake: old.wake,
                ..Forwarder::default()
            };
            for a in dials {
                if !self.dial_held(a) {
                    if let Some(d) = self.dialer() {
                        d.release(a);
                    }
                }
            }
        }
        self.fabric_changed();
    }

    // -- the gate (§5.2) ---------------------------------------------------

    /// Layer 7 §5.1 and §5.2: where a put for `dest` from `sender` (already
    /// admitted: a member, or a live pair) goes. Synchronous: it reads the
    /// pairing record and the carrier bound only, so it decides whether a
    /// target **exists**.
    fn custody_gate(
        &mut self,
        sender: &NodeId,
        arrival: Arrival,
        dest: &NodeId,
        hint: Option<[u8; DEST_HUB_LEN]>,
        size: usize,
    ) -> Gate {
        // The member branch is today's, from any origin.
        if self.is_member(dest) && Some(*dest) != self.home_hub() {
            return Gate::Member;
        }
        let unknown = Gate::Unknown { too_large: false };
        // A stale hint (C18): one naming this hub while `dest` is not a
        // member here; the hint is a locator only and never pre-empts.
        if *dest == self.own || hint.is_some_and(|h| h[..] == self.own.as_bytes()[..DEST_HUB_LEN]) {
            return unknown;
        }
        if !self.settings.hub_role || self.settings.paired_hubs.is_empty() {
            return unknown;
        }
        let fwd = match (self.live_pair(sender), arrival) {
            // From a member (never the home hub: a hub's own upstream).
            (None, Arrival::Tcp) if Some(*sender) != self.home_hub() => {
                Fwd::FromMember(sender.to_bytes())
            }
            (Some(p), Arrival::Tcp) if p.role == PairRole::Leaf && p.via == PairVia::Tcp => {
                Fwd::FromLeaf(sender.to_bytes())
            }
            (Some(p), Arrival::Carrier)
                if p.role == PairRole::Peer && p.via == PairVia::Carrier =>
            {
                Fwd::FromCarrier(sender.to_bytes())
            }
            // A `tcp` peer, this hub's gateway, or against the pair's
            // record: today's gate (§5.1 last row).
            _ => return unknown,
        };
        let cached = self.fwd.cache.get(&dest.to_bytes());
        let pairs = self.eligible_pairs();
        self.fwd.computed += 1;
        let t = forward_targets(
            &pairs,
            fwd.code(),
            hint,
            size,
            self.fwd.carrier_max,
            TargetMode::Admit,
            cached,
        );
        if t.list.is_empty() {
            Gate::Unknown {
                too_large: t.too_large,
            }
        } else {
            Gate::Forward(fwd)
        }
    }

    /// A forward row was held: the forwarder offers it at the end of the
    /// tick, after the answer (§5.5, C2). While the resident window is full
    /// (or rows wait in the store before it) the row waits in the store and
    /// is paged in, or swapped in by the probe, in custody order without its
    /// hint (module doc "The resident window"). Once the probe found every
    /// stored row unable to move, a row that can move now enters at once
    /// with its hint, in place of an evicted one: no stored row is owed to
    /// its turn before it.
    fn track_forward(&mut self, seq: u64, it: FwdRow) {
        if !self.settings.hub_role {
            return;
        }
        if !self.fwd.more && self.fwd.rows.len() < FORWARD_RESIDENT {
            self.fwd.loaded = self.fwd.loaded.max(seq);
            self.fwd.insert(seq, it);
            return;
        }
        self.fwd.more = true;
        if self.fwd.probe_done {
            let pairs = self.eligible_pairs();
            let up = self.up_targets(&pairs);
            if self.fwd.probe_done {
                let turn = self.first_target(&pairs, &it.dest, it.fwd, it.hint, it.size);
                if turn.is_some_and(|t| up.contains(&t)) {
                    if self.fwd.rows.len() >= FORWARD_RESIDENT {
                        match self.fwd.evictable(&up) {
                            Some(v) => self.fwd.evict(v),
                            None => return,
                        }
                    }
                    self.fwd.insert(seq, it);
                }
                return;
            }
        }
        // The probe reads it in custody order (one page per trigger).
        self.fwd.run = true;
    }

    /// A resident row as the gate or the store gives it.
    fn fwd_row(
        dest: [u8; 32],
        pkt_ref: [u8; 16],
        size: usize,
        fwd: u8,
        hint: Option<[u8; DEST_HUB_LEN]>,
    ) -> FwdRow {
        FwdRow {
            dest,
            pkt_ref,
            hint,
            fwd,
            size,
            turn: None,
            state: Turn::Idle,
            refused: Vec::new(),
            fresh: true,
            orphaned: false,
            heard: false,
        }
    }

    /// A resident row from a stored row's head (no hint: memory only).
    fn head_row(h: &ForwardHead) -> FwdRow {
        let size = usize::try_from(h.len).unwrap_or(usize::MAX);
        Self::fwd_row(h.dest, h.pkt_ref, size, h.fwd.code(), None)
    }

    /// The turn a row with no refusal would get now (§5.1 table, hint,
    /// cache, probe order): what the probe and a held row under a full
    /// window are judged by. One target computation.
    fn first_target(
        &mut self,
        pairs: &[PairedHub],
        dest: &[u8; 32],
        fwd: u8,
        hint: Option<[u8; DEST_HUB_LEN]>,
        size: usize,
    ) -> Option<NodeId> {
        let cached = self.fwd.cache.get(dest);
        self.fwd.computed += 1;
        forward_targets(
            pairs,
            fwd,
            hint,
            size,
            self.fwd.carrier_max,
            TargetMode::Held,
            cached,
        )
        .list
        .first()
        .copied()
    }

    /// The targets that can take a row now (module doc "The resident
    /// window"): a `tcp` pair with a sendable connection, a `carrier` pair
    /// while the carrier bound is set, either one with no `REJECT_FULL` row
    /// waiting (a full target counts as down for admission until its next
    /// trigger clears or accepts its rows, C16). A target that was not up
    /// when the probe last looked restarts the probe from the paging floor
    /// (its rows in the store are read again). O(pairs × connections), only
    /// while the window is full.
    fn up_targets(&mut self, pairs: &[PairedHub]) -> HashSet<NodeId> {
        let up: HashSet<NodeId> = pairs
            .iter()
            .filter(|p| match p.via {
                PairVia::Tcp => self.sendable_conn(&p.node_id).is_some(),
                PairVia::Carrier => self.fwd.carrier_max.is_some(),
            })
            .filter(|p| {
                !self
                    .fwd
                    .targets
                    .get(&p.node_id)
                    .is_some_and(|q| !q.full.is_empty())
            })
            .map(|p| p.node_id)
            .collect();
        if up.iter().any(|t| !self.fwd.up.contains(t)) {
            self.fwd.probe = self.fwd.loaded;
            self.fwd.probe_done = false;
        }
        self.fwd.up.clone_from(&up);
        up
    }

    /// Page protected forward rows in from the store, in custody order from
    /// the paging floor, while the window has room for a page (one
    /// `forward_pending_lens` read of row heads per [`FORWARD_LOAD_PAGE`]
    /// rows that leave it; §15 G9 item 6 at start). A row already resident
    /// (swapped in above the floor) is skipped; every other row comes in,
    /// whatever its turn. Returns how many heads were read.
    fn page_in(&mut self) -> Result<usize> {
        if !self.fwd.more || !self.settings.hub_role {
            return Ok(0);
        }
        let resident = self.fwd.rows.len();
        let room = FORWARD_RESIDENT.saturating_sub(resident);
        if room == 0 || (room < FORWARD_LOAD_PAGE && resident > 0) {
            return Ok(0);
        }
        let want = room.min(FORWARD_LOAD_PAGE);
        let page = self.store.forward_pending_lens(self.fwd.loaded, want)?;
        if page.len() < want {
            self.fwd.more = false;
        }
        self.fwd.read_on = self.fwd.more;
        let n = page.len();
        for h in page {
            self.fwd.loaded = self.fwd.loaded.max(h.custody_seq);
            if !self.fwd.rows.contains_key(&h.custody_seq) {
                self.fwd.insert(h.custody_seq, Self::head_row(&h));
            }
        }
        Ok(n)
    }

    /// The window is full and rows wait in the store (module doc "The
    /// resident window"): one page of row heads from the probe; each stored
    /// row whose turn is up enters, in place of the newest idle row whose
    /// turn is down (evicted to the store), and the others stay in the
    /// store. Bounded: one read of at most [`FORWARD_LOAD_PAGE`] heads and
    /// as many evictions per pass; nothing is read while no target is up,
    /// nothing can be evicted, or the probe found nothing to move since it
    /// was last restarted (a target came up, or a stored row's turn can
    /// have changed: [`Forwarder::restart_probe`]); an all-down hub, or a
    /// store of rows for down targets only, does not churn. A full page
    /// leaves `read_on` set (the self-wake, [`Core::run_forwarder`]).
    /// Returns how many rows came in.
    fn swap_in(&mut self, pairs: &[PairedHub]) -> Result<usize> {
        let resident = self.fwd.rows.len();
        let room = FORWARD_RESIDENT.saturating_sub(resident);
        if !self.fwd.more || !self.settings.hub_role || room >= FORWARD_LOAD_PAGE {
            return Ok(0);
        }
        let up = self.up_targets(pairs);
        if up.is_empty() || self.fwd.probe_done {
            return Ok(0);
        }
        if room == 0 && self.fwd.evictable(&up).is_none() {
            return Ok(0);
        }
        let from = self.fwd.probe.max(self.fwd.loaded);
        let heads = self.store.forward_pending_lens(from, FORWARD_LOAD_PAGE)?;
        let mut pos = from;
        let mut moved = 0;
        let mut blocked = false;
        for h in &heads {
            if !self.fwd.rows.contains_key(&h.custody_seq) {
                let size = usize::try_from(h.len).unwrap_or(usize::MAX);
                let turn = self.first_target(pairs, &h.dest, h.fwd.code(), None, size);
                if turn.is_some_and(|t| up.contains(&t)) {
                    if self.fwd.rows.len() >= FORWARD_RESIDENT {
                        let Some(v) = self.fwd.evictable(&up) else {
                            blocked = true;
                            break;
                        };
                        self.fwd.evict(v);
                    }
                    self.fwd.insert(h.custody_seq, Self::head_row(h));
                    moved += 1;
                }
            }
            pos = h.custody_seq;
        }
        self.fwd.probe = pos;
        let full = !blocked && heads.len() == FORWARD_LOAD_PAGE;
        if full {
            // The next pass reads on (at most four a tick, then the
            // self-wake: `run_forwarder`).
            self.fwd.run = true;
        } else if !blocked {
            self.fwd.probe_done = true;
        }
        self.fwd.read_on = full;
        Ok(moved)
    }

    /// At start (§15 G9 item 6): the first window of protected forward rows,
    /// without the hints they arrived with (in memory only).
    fn load_forward_rows(&mut self) -> Result<()> {
        self.fwd.more = true;
        self.fwd.loaded = 0;
        self.fwd.probe = 0;
        self.fwd.probe_done = false;
        while self.page_in()? > 0 {}
        Ok(())
    }

    // -- the forwarder (§5.5, §5.9) ----------------------------------------

    /// Run while a trigger asked (module doc); a pass that changes a row's
    /// target asks again, so the tick settles (bounded: at most
    /// [`FORWARD_PASSES`] passes, each at most one store read). When the
    /// passes ran out with the last store read full (the swap probe, or a
    /// page in, has more to read), one [`Input::Continue`] is queued to the
    /// actor's own mailbox (module doc "The resident window"): the next tick
    /// reads on, every other input interleaves, and no timer runs. The
    /// mailbox is unbounded and at most one self-wake is queued at a time;
    /// were the send to fail (the actor is going), the next input resumes
    /// the probe.
    fn run_forwarder(&mut self) {
        self.fwd.read_on = false;
        for _ in 0..FORWARD_PASSES {
            if !std::mem::take(&mut self.fwd.run) {
                return;
            }
            self.forward_pass();
        }
        if self.fwd.run && self.fwd.read_on && !self.fwd.wake {
            self.fwd.wake = self.control.send(Input::Continue).is_ok();
        }
    }

    /// One pass, bounded by what changed: page rows in while the
    /// window has room, recompute the targets of the rows whose inputs
    /// changed only, swap rows that can move in for rows that cannot while
    /// the window is full, release the `REJECT_FULL` rows a trigger
    /// retried, and put what is due to the `tcp` pairs that are up and have
    /// window room; carrier turns wait for `forward_take`. At most one
    /// store read of row heads per pass.
    fn forward_pass(&mut self) {
        if !self.settings.hub_role {
            self.fwd.retry_full.clear();
            self.fwd.dirty.clear();
            self.fwd.all_dirty = false;
            return;
        }
        let paged = self.page_in().unwrap_or_else(|_| {
            self.counters.errors += 1;
            1
        });
        if self.fwd.rows.is_empty() {
            self.fwd.retry_full.clear();
            self.fwd.all_dirty = false;
            return;
        }
        let pairs = self.eligible_pairs();
        if std::mem::take(&mut self.fwd.all_dirty) {
            let idle: Vec<u64> = self
                .fwd
                .rows
                .iter()
                .filter(|(_, r)| r.state == Turn::Idle)
                .map(|(s, _)| *s)
                .collect();
            self.fwd.dirty.extend(idle);
        }
        let mut retry = std::mem::take(&mut self.fwd.retry_full);
        let mut refuse = Vec::new();
        for seq in std::mem::take(&mut self.fwd.dirty) {
            self.retarget(seq, &pairs, &mut retry, &mut refuse);
        }
        // After the retargets, so every resident row's turn is settled.
        if paged == 0 {
            match self.swap_in(&pairs) {
                Ok(0) => {}
                Ok(_) => {
                    for seq in std::mem::take(&mut self.fwd.dirty) {
                        self.retarget(seq, &pairs, &mut retry, &mut refuse);
                    }
                }
                Err(_) => self.counters.errors += 1,
            }
        }
        for seq in refuse {
            self.refuse_row(seq);
        }
        for t in retry {
            self.retry_full_rows(&t);
        }
        for p in pairs.iter().filter(|p| p.via == PairVia::Tcp) {
            self.put_due(p.node_id);
        }
    }

    /// Row `seq`'s §5.1 targets from the current inputs: its turn is the
    /// first that has not refused it; with none, it is refused (every
    /// target refused, or its turn's pair went) or waits (none ever).
    fn retarget(
        &mut self,
        seq: u64,
        pairs: &[PairedHub],
        retry: &mut HashSet<NodeId>,
        refuse: &mut Vec<u64>,
    ) {
        let Some(r) = self.fwd.rows.get(&seq) else {
            return;
        };
        if r.state != Turn::Idle {
            return;
        }
        let cached = self.fwd.cache.get(&r.dest);
        self.fwd.computed += 1;
        let t = forward_targets(
            pairs,
            r.fwd,
            r.hint,
            r.size,
            self.fwd.carrier_max,
            TargetMode::Held,
            cached,
        );
        let next = t.list.iter().find(|n| !r.refused.contains(n)).copied();
        let (fresh, waits) = (
            r.fresh,
            t.list.is_empty() && r.refused.is_empty() && !r.orphaned,
        );
        let old = r.turn;
        if let Some(r) = self.fwd.rows.get_mut(&seq) {
            r.fresh = false;
        }
        match next {
            Some(n) => {
                if fresh {
                    retry.insert(n);
                }
                if old != Some(n) {
                    self.fwd.mutate(seq, |r| r.turn = Some(n));
                    if pairs
                        .iter()
                        .any(|p| p.node_id == n && p.via == PairVia::Carrier)
                    {
                        self.fwd.notify.insert(n);
                    }
                }
            }
            // A row that never had a target waits for one.
            None if waits => self.fwd.mutate(seq, |r| r.turn = None),
            // Every target refused (§5.8), or none left after an unpair or
            // a break.
            None => refuse.push(seq),
        }
    }

    /// A retry trigger for `t`'s `REJECT_FULL` rows (C16): at most one
    /// window of them, oldest first, goes back to its queue. Each answer
    /// from the pair that is not `REJECT_FULL` is the next trigger, so a
    /// pair with room again drains them ACK-clocked, and a full one costs
    /// one window per trigger (module doc).
    fn retry_full_rows(&mut self, t: &NodeId) {
        let full: Vec<u64> = match self.fwd.targets.get(t) {
            Some(q) => q.full.iter().take(FORWARD_WINDOW).copied().collect(),
            None => return,
        };
        if full.is_empty() {
            return;
        }
        for seq in full {
            self.fwd.set_state(seq, Turn::Idle);
        }
        if self.carrier_pair(t).is_ok() {
            self.fwd.notify.insert(*t);
        }
    }

    /// The put stage for the `tcp` pair `t`: only when it is up and has
    /// window room, its idle rows in custody order until the window is full
    /// (at most [`FORWARD_WINDOW`] in flight, ACK-clocked).
    fn put_due(&mut self, t: NodeId) {
        let (empty, inflight) = match self.fwd.targets.get(&t) {
            Some(q) => (q.idle.is_empty(), q.inflight.len()),
            None => return,
        };
        if empty || inflight >= FORWARD_WINDOW {
            return;
        }
        // A pair that is down keeps its turn until its link-up (§5.5).
        let Some(ck) = self.sendable_conn(&t) else {
            return;
        };
        let mut room = FORWARD_WINDOW - inflight;
        while room > 0 {
            let Some(seq) = self
                .fwd
                .targets
                .get(&t)
                .and_then(|q| q.idle.first().copied())
            else {
                return;
            };
            match self.forward_put_tcp(seq, t, &ck) {
                PutOutcome::Sent => room -= 1,
                PutOutcome::Skipped => {}
                PutOutcome::Blocked => return,
            }
        }
    }

    /// The stored row, unless it is gone or no longer protected (then it
    /// leaves the forwarder). `Err`: the store failed (counted).
    fn forward_item_or_untrack(&mut self, seq: u64) -> Result<Option<ForwardItem>, ()> {
        match self.store.forward_item(seq) {
            Ok(Some(it)) if !it.committed => Ok(Some(it)),
            Ok(_) => {
                self.fwd.untrack(seq);
                Ok(None)
            }
            Err(_) => {
                self.counters.errors += 1;
                Err(())
            }
        }
    }

    /// §5.9: a `CUSTODY_PUT` to the next custodian with the stored item
    /// bytes (retransmits reuse them, §5.7 (d)) and the received hint.
    fn seal_forward_put(
        &mut self,
        keys: &PeerKeys,
        item: &ForwardItem,
        hint: Option<[u8; DEST_HUB_LEN]>,
    ) -> Option<Vec<u8>> {
        let mut inner = Inner::new(
            &self.own,
            Kind::CustodyPut,
            MsgSeq::CONTROL,
            encode_put(&NodeId::from(item.dest), &item.bytes),
        );
        inner.ext.dest_hub = hint;
        match seal_envelope(&*self.sealer, keys, Priority::Interactive, &inner) {
            Ok(s) => Some(s),
            Err(_) => {
                self.counters.errors += 1;
                None
            }
        }
    }

    /// A control payload sealed to `to` with no connection (a carrier ack).
    fn seal_control(&mut self, to: &PeerKeys, kind: Kind, payload: Vec<u8>) -> Option<Vec<u8>> {
        if !self.can_seal() {
            return None;
        }
        let inner = Inner::new(&self.own, kind, MsgSeq::CONTROL, payload);
        match seal_envelope(&*self.sealer, to, Priority::Control, &inner) {
            Ok(s) => Some(s),
            Err(_) => {
                self.counters.errors += 1;
                None
            }
        }
    }

    /// Put row `seq` to the `tcp` pair `target` on `ck`; records
    /// `pkt_ref → custody_seq` (the pending map).
    fn forward_put_tcp(&mut self, seq: u64, target: NodeId, ck: &ConnKey) -> PutOutcome {
        if !self.can_seal() {
            return PutOutcome::Blocked;
        }
        let Some(keys) = self.keys.get(&target) else {
            return PutOutcome::Blocked;
        };
        let item = match self.forward_item_or_untrack(seq) {
            Ok(Some(it)) => it,
            Ok(None) => return PutOutcome::Skipped,
            Err(()) => return PutOutcome::Blocked,
        };
        let hint = self.fwd.rows.get(&seq).and_then(|r| r.hint);
        let Some(sealed) = self.seal_forward_put(&keys, &item, hint) else {
            return PutOutcome::Blocked;
        };
        if sealed.len() > self.room(ck) {
            // No path to this target carries it: its refusal (§5.8).
            self.fwd.refused(seq, target);
            return PutOutcome::Skipped;
        }
        self.fwd.pending.insert(item.pkt_ref, (seq, target));
        self.fwd.set_state(seq, Turn::InFlight);
        self.outq.push(Out {
            ck: ck.clone(),
            frame: Frame::sealed(sealed),
            tag: Tag::Forward(seq),
        });
        PutOutcome::Sent
    }

    /// The refused mark (§5.8): `committed = 1`, step-1 evictable, counted.
    fn refuse_row(&mut self, seq: u64) {
        self.fwd.untrack(seq);
        match self.store.forward_refuse(seq) {
            Ok(true) => self.counters.forward_refused += 1,
            Ok(false) => {}
            Err(_) => self.counters.errors += 1,
        }
        self.fabric_changed();
    }

    /// A pair's `CUSTODY_ACK` for a forward put (§5.9), from the pair the
    /// put went to only (an ack from any other pair changes nothing):
    /// `ACCEPTED` or `DUPLICATE` commits (and is a cache hit),
    /// `REJECT_UNKNOWN_DEST` moves to the next target, `REJECT_FULL` keeps
    /// the row with that pair, `REJECT_UNENROLLED` breaks the pair.
    fn on_forward_ack(&mut self, sender: &NodeId, ack: &CustodyAck) {
        let pref = ack.receipt.pkt_ref;
        let Some(&(seq, target)) = self.fwd.pending.get(&pref) else {
            return;
        };
        if target != *sender {
            return;
        }
        let status = ack.receipt.status;
        if status != CustodyStatus::RejectFull {
            // A forwarder run for that pair (C16): its REJECT_FULL rows are
            // retried; never on the REJECT_FULL itself.
            self.fwd.retry_full.insert(*sender);
            self.fwd.run = true;
        }
        if status.is_held() {
            let row = self.fwd.untrack(seq);
            if self.store.forward_commit(seq).is_err() {
                self.counters.errors += 1;
            }
            if let Some(r) = row {
                self.fwd.cache_put(r.dest, *sender);
            }
            self.fabric_changed();
            return;
        }
        match status {
            CustodyStatus::RejectUnknownDest => self.fwd.refused(seq, target),
            CustodyStatus::RejectFull => {
                self.fwd.unpend(seq);
                self.fwd.set_state(seq, Turn::Full);
            }
            CustodyStatus::RejectUnenrolled => self.pair_broken(*sender),
            CustodyStatus::Accepted | CustodyStatus::Duplicate => {}
        }
        self.fabric_changed();
    }

    /// `FabricEvent::Forward` for each `carrier` pair whose owed frames
    /// changed this tick (coalesced: the latest count wins).
    fn forward_events(&mut self) {
        for t in std::mem::take(&mut self.fwd.notify) {
            if self.carrier_pair(&t).is_err() {
                continue;
            }
            let pending = self.fwd.owed(&t);
            if pending > 0 {
                self.pending_events.insert(
                    EventKey::Forward(t.to_bytes()),
                    FabricEvent::Forward { target: t, pending },
                );
            }
        }
    }

    // -- the hand-off (§5.9) -----------------------------------------------

    /// [`Fabric::set_carrier_max_frame`]: a changed bound recomputes the
    /// idle rows' targets; every call tells the carrier pairs what they are
    /// owed.
    fn set_carrier_max_frame(&mut self, max: Option<usize>) {
        if self.fwd.carrier_max != max {
            self.fwd.carrier_max = max;
            self.fwd.all_changed();
        }
        self.fwd.run = true;
        let carriers: Vec<NodeId> = self
            .settings
            .paired_hubs
            .iter()
            .filter(|p| p.via == PairVia::Carrier)
            .map(|p| p.node_id)
            .collect();
        self.fwd.notify.extend(carriers);
    }

    /// `target` is a whole `carrier` pair of this hub, hub role on.
    fn carrier_pair(&self, target: &NodeId) -> Result<PairedHub> {
        if !self.settings.hub_role {
            return Err(FabricError::Invalid("this node is not a hub"));
        }
        match self.whole_pair(target) {
            Some(p) if p.via == PairVia::Carrier => Ok(p),
            _ => Err(FabricError::Invalid("not a whole carrier pair")),
        }
    }

    /// [`Fabric::forward_take`].
    fn forward_take(&mut self, target: NodeId, max: usize) -> Result<Vec<CarrierFrame>> {
        self.carrier_pair(&target)?;
        // §5.9 re-post rule (1) (C6), after the whole drain was handed in
        // (its acks committed first): each post a verified far frame came
        // after, still unacknowledged, is re-armed once for that round.
        let heard: Vec<u64> = self
            .fwd
            .targets
            .get(&target)
            .map(|q| q.heard.iter().copied().collect())
            .unwrap_or_default();
        for seq in heard {
            self.fwd.mutate(seq, |r| {
                r.heard = false;
                r.state = Turn::Posted { repost: true };
            });
        }
        self.fwd.run = true;
        self.run_forwarder();
        let mut out = Vec::new();
        if let Some(q) = self.fwd.acks.get_mut(&target) {
            while out.len() < max {
                let Some(bytes) = q.pop_front() else { break };
                out.push(CarrierFrame {
                    custody_seq: None,
                    bytes,
                });
            }
            if q.is_empty() {
                self.fwd.acks.remove(&target);
            }
        }
        if self.fwd.carrier_max.is_none() || !self.can_seal() {
            return Ok(out);
        }
        let Some(keys) = self.keys.get(&target) else {
            return Ok(out);
        };
        let room = max.saturating_sub(out.len());
        let mut due: Vec<u64> = match self.fwd.targets.get(&target) {
            Some(q) => q
                .armed
                .iter()
                .take(room)
                .chain(q.idle.iter().take(room))
                .copied()
                .collect(),
            None => Vec::new(),
        };
        due.sort_unstable();
        due.truncate(room);
        for seq in due {
            let item = match self.forward_item_or_untrack(seq) {
                Ok(Some(it)) => it,
                Ok(None) => continue,
                Err(()) => break,
            };
            if !carrier_fits(self.fwd.carrier_max, item.bytes.len()) {
                // The bound shrank under it: this target refuses it.
                self.fwd.refused(seq, target);
                continue;
            }
            let hint = self.fwd.rows.get(&seq).and_then(|r| r.hint);
            let Some(bytes) = self.seal_forward_put(&keys, &item, hint) else {
                break;
            };
            let Some(r) = self.fwd.rows.get(&seq) else {
                continue;
            };
            if matches!(r.state, Turn::Posted { .. }) {
                *self.fwd.reposts.entry(target).or_default() += 1;
            }
            self.fwd.set_state(seq, Turn::Taken);
            self.fwd.pending.insert(item.pkt_ref, (seq, target));
            out.push(CarrierFrame {
                custody_seq: Some(seq),
                bytes,
            });
        }
        if !out.is_empty() {
            self.fabric_changed();
        }
        Ok(out)
    }

    /// [`Fabric::forward_done`].
    fn forward_done(&mut self, seq: u64, done: ForwardDone) {
        let Some(r) = self.fwd.rows.get(&seq) else {
            return;
        };
        if r.state != Turn::Taken {
            return;
        }
        let turn = r.turn;
        match done {
            // A new post: a far frame after it starts its round.
            ForwardDone::Taken => self.fwd.mutate(seq, |r| {
                r.heard = false;
                r.state = Turn::Posted { repost: false };
            }),
            // Stays for the next flush; no event (a carrier that is down
            // is not asked again in a loop).
            ForwardDone::Retry => {
                self.fwd.unpend(seq);
                self.fwd.set_state(seq, Turn::Idle);
            }
            ForwardDone::Rejected { .. } => {
                if let Some(t) = turn {
                    self.fwd.refused(seq, t);
                }
                self.fabric_changed();
            }
        }
    }

    /// [`Fabric::forward_repost`].
    fn forward_repost(&mut self, target: NodeId) -> Result<u32> {
        self.carrier_pair(&target)?;
        let seqs: Vec<u64> = match self.fwd.targets.get(&target) {
            Some(q) => q
                .posted
                .iter()
                .chain(q.heard.iter())
                .chain(q.taken.iter())
                .copied()
                .collect(),
            None => Vec::new(),
        };
        let n = u32::try_from(seqs.len()).unwrap_or(u32::MAX);
        for seq in seqs {
            self.fwd.mutate(seq, |r| {
                r.heard = false;
                r.state = Turn::Posted { repost: true };
            });
        }
        if n > 0 {
            self.fwd.notify.insert(target);
        }
        Ok(n)
    }

    /// [`Fabric::carrier_inbound`] (the size was checked by the handle).
    fn carrier_inbound(&mut self, from: NodeId, frame: &[u8]) -> Result<()> {
        // A whole carrier pair only (a `tcp` pair's frames never come this
        // way: §5.1, a frame against the record).
        self.carrier_pair(&from)?;
        let opened = {
            let keys = &self.keys;
            match open_envelope(&*self.sealer, frame, &|p| keys.lookup(p)) {
                Ok(o) => o,
                Err(_) => {
                    self.counters.dropped += 1;
                    return Ok(());
                }
            }
        };
        let sender = opened.sender.node_id;
        if sender != from {
            self.counters.dropped += 1;
            self.forget_provisional(&opened);
            return Ok(());
        }
        let fresh = self
            .sealer
            .observe_counter(&sender, opened.counter())
            .unwrap_or(Freshness::Stale);
        if fresh == Freshness::Stale {
            // Already processed (a re-read after a `since` miss, C13):
            // dropped before any `pkt_ref` check.
            self.counters.stale += 1;
            return Ok(());
        }
        self.heard.insert(sender);
        self.fwd.heard.insert(sender);
        match opened.inner.kind {
            Kind::CustodyPut => {
                let hint = opened.inner.ext.dest_hub;
                self.on_carrier_put(&from, &opened.inner.payload, hint);
            }
            Kind::CustodyAck => {
                if let Ok(ack) = CustodyAck::decode(&opened.inner.payload) {
                    self.on_forward_ack(&from, &ack);
                }
            }
            // ENROLL never rides a carrier (S22); nothing else is carried.
            _ => self.counters.dropped += 1,
        }
        // §5.9 re-post rule (1): a verified far frame after a post starts
        // that post's round (an ack this frame carried committed its row
        // above). Each post still unacknowledged is marked once (a post
        // already marked, or re-posted since, waits for the next round);
        // the re-arm waits for the next `forward_take`, after every frame
        // of this drain was handed in, and the carrier is told it is owed.
        let posted: Vec<u64> = self
            .fwd
            .targets
            .get(&from)
            .map(|q| q.posted.iter().copied().collect())
            .unwrap_or_default();
        if !posted.is_empty() {
            for seq in posted {
                self.fwd.mutate(seq, |r| r.heard = true);
            }
            self.fwd.notify.insert(from);
        }
        self.fwd.run = true;
        self.fabric_changed();
        Ok(())
    }

    /// A put through the carrier: the gate (the `fwd = 3` branch for a
    /// `carrier` pair), its answer queued for `from`'s next take.
    fn on_carrier_put(&mut self, from: &NodeId, payload: &[u8], hint: Option<[u8; DEST_HUB_LEN]>) {
        let Some(v) = self.custody_decide(from, Arrival::Carrier, payload, hint) else {
            return;
        };
        let sealed = match (self.ack_payload(&v), self.keys.get(from)) {
            (Some(bytes), Some(keys)) => self.seal_control(&keys, Kind::CustodyAck, bytes),
            _ => None,
        };
        if let Some(sealed) = sealed {
            let q = self.fwd.acks.entry(*from).or_default();
            q.push_back(sealed);
            while q.len() > CARRIER_ACK_QUEUE {
                q.pop_front();
            }
            self.fwd.notify.insert(*from);
        }
        self.custody_after(&v);
    }
}

// ---------------------------------------------------------------------------
// Layer 6 foreign calls (`6-transport/README.md` §3, §8, §9, §10)
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

    /// Cancel a user-composed own row (§9): only while `queued` and never
    /// handed to a carrier (no pending handoff, no recorded native id); the
    /// store deletes it ([`Store::foreign_cancel`], FULL), so it can never
    /// be sent, and a `Conversation` event tells the client to refetch the
    /// conversation (the row no longer exists). A row past that is
    /// [`FabricError::NotRetryable`]; a relay copy is
    /// [`FabricError::Invalid`] (copies leave by shed, disable or delete).
    /// Other parts of the same send are not touched (cancel drops unsent
    /// parts only, one call per part).
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

    /// Set an edge's `honour_ok_to_mqtt` and `oversize` (Layer 6 §8, C1:
    /// `PATCH /bridge/edges/{id}`; neither is part of the consent text).
    /// Edge-only changes emit no event (the bridge core publishes its
    /// `bridge` snapshot). Returns whether anything changed.
    pub async fn foreign_edge_set_options(
        &self,
        id: i64,
        honour_ok_to_mqtt: bool,
        oversize: EdgeOversize,
    ) -> Result<bool> {
        self.call(move |c| {
            c.store
                .edge_set_options(id, honour_ok_to_mqtt, oversize)
                .map_err(fx)
        })
        .await?
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

    /// §4 "Later retries": withdraw a row's pending handoff mark
    /// ([`Store::foreign_handoff_abort`], FULL) when the carrier reported a
    /// refusal **before air** and no `Accepted`, so the next tick offers
    /// the row again ([`Fabric::foreign_queued`]). Never after an
    /// `Accepted` or on a post-air outcome. Emits no event, like
    /// [`Fabric::foreign_handoff_begin`] (the mark is bookkeeping; the row's
    /// visible state did not change). A row that is not `queued` with a
    /// pending mark is [`FabricError::Invalid`]; an unknown id
    /// [`FabricError::NotFound`].
    pub async fn foreign_handoff_abort(&self, id: i64) -> Result<()> {
        self.call(move |c| c.store.foreign_handoff_abort(id).map_err(fx))
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
        // B1-bounce F3: the row is gone, so no per-row event may name it;
        // the conversation changed (its queued count), the client refetches.
        self.foreign_event(row.conv, None, ForeignChange::Conversation);
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
        // B1-bounce F4: the store compares against the stored row (cached or
        // not) and answers whether anything changed.
        let (id, changed) = self
            .store
            .foreign_conversation_upsert_changed(network, account, kind, native_id, title)
            .map_err(fx)?;
        if changed {
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

    fn pair_at(b: u8, role: PairRole, via: PairVia) -> PairedHub {
        PairedHub {
            node_id: NodeId::from([b; 32]),
            role,
            via,
            ours: true,
            theirs: true,
        }
    }

    fn ids(t: &Targets) -> Vec<u8> {
        t.list.iter().map(|n| n.as_bytes()[0]).collect()
    }

    /// Layer 7 §6.1 item 3 (the board's figures): item + 131 B ≤ the bound.
    #[test]
    fn the_carrier_bound() {
        assert_eq!(FORWARD_PUT_WRAP, 131);
        assert!(!carrier_fits(None, 100), "no carrier built: nothing fits");
        assert!(carrier_fits(Some(3_068), 2_937));
        assert!(!carrier_fits(Some(3_068), 2_938));
        assert!(!carrier_fits(Some(10), usize::MAX), "no overflow");
    }

    /// Layer 7 §4.6 step 2: the words, the complement, the allowed pairs.
    #[test]
    fn pair_roles_links_and_their_rules() {
        for r in PairRole::ALL {
            assert_eq!(PairRole::from_code(r.code()), Some(r));
            assert_eq!(r.complement().complement(), r);
        }
        for v in PairVia::ALL {
            assert_eq!(PairVia::from_code(v.code()), Some(v));
        }
        assert_eq!(PairRole::Leaf.complement(), PairRole::Gateway);
        assert_eq!(PairRole::Peer.complement(), PairRole::Peer);
        assert_eq!(PairRole::from_code("hub"), None);
        assert!(pair_at(1, PairRole::Peer, PairVia::Carrier).valid());
        assert!(!pair_at(1, PairRole::Leaf, PairVia::Carrier).valid());
        assert!(!pair_at(1, PairRole::Gateway, PairVia::Carrier).valid());
        assert!(pair_at(1, PairRole::Gateway, PairVia::Tcp).valid());
        let own = NodeId::from([9u8; 32]);
        let home = NodeId::from([8u8; 32]);
        let mut s = FabricSettings {
            home_hub: Some(HomeHub {
                node_id: home,
                address: None,
            }),
            paired_hubs: vec![
                pair_at(1, PairRole::Peer, PairVia::Tcp),
                pair_at(1, PairRole::Leaf, PairVia::Tcp),
                pair_at(9, PairRole::Peer, PairVia::Tcp),
                pair_at(8, PairRole::Peer, PairVia::Tcp),
                pair_at(2, PairRole::Leaf, PairVia::Carrier),
                pair_at(3, PairRole::Gateway, PairVia::Tcp),
            ],
            ..FabricSettings::default()
        };
        normalize_pairs(&mut s, &own);
        let kept: Vec<u8> = s
            .paired_hubs
            .iter()
            .map(|p| p.node_id.as_bytes()[0])
            .collect();
        assert_eq!(
            kept,
            vec![1, 3],
            "one per hub, never self or home, only §4.6 pairs"
        );
        s.paired_hubs = (10..60)
            .map(|b| pair_at(b, PairRole::Peer, PairVia::Tcp))
            .collect();
        normalize_pairs(&mut s, &own);
        assert_eq!(s.paired_hubs.len(), MAX_PAIRED_HUBS);
    }

    /// Layer 7 §5.1, §5.2, §5.5: the targets and their order, per table row.
    #[test]
    fn forward_targets_follow_the_table() {
        use PairRole::*;
        use PairVia::*;
        let pairs = [
            pair_at(1, Peer, Tcp),
            pair_at(2, Gateway, Tcp),
            pair_at(3, Leaf, Tcp),
            pair_at(4, Peer, Carrier),
            pair_at(5, Peer, Tcp),
        ];
        let hint = |b: u8| Some([b; DEST_HUB_LEN]);
        let bound = Some(3_068);
        let t = |fwd, h, size, max, mode, cached| {
            forward_targets(&pairs, fwd, h, size, max, mode, cached)
        };
        // From a member, no hint: tcp peers and leaves in order, then the
        // gateway, then a fitting carrier.
        let a = t(1, None, 100, bound, TargetMode::Admit, None);
        assert_eq!(ids(&a), vec![1, 3, 5, 2, 4]);
        // The hit cache goes first.
        let c = t(
            1,
            None,
            100,
            bound,
            TargetMode::Admit,
            Some(NodeId::from([5u8; 32])),
        );
        assert_eq!(ids(&c), vec![5, 1, 3, 2, 4]);
        // A hint naming a pair: that pair first.
        assert_eq!(
            ids(&t(1, hint(3), 100, bound, TargetMode::Admit, None)),
            vec![3, 1, 5, 2, 4]
        );
        assert_eq!(
            ids(&t(1, hint(4), 100, bound, TargetMode::Admit, None))[0],
            4
        );
        // A hint naming no pair: upstream only (gateway, fitting carriers).
        assert_eq!(
            ids(&t(1, hint(0xEE), 100, bound, TargetMode::Admit, None)),
            vec![2, 4]
        );
        // The carrier bound: unset ⇒ no carrier target at the gate, a down
        // target for a held row; too large ⇒ not a target, counted when it
        // leaves nothing.
        assert_eq!(
            ids(&t(1, None, 100, None, TargetMode::Admit, None)),
            vec![1, 3, 5, 2]
        );
        assert_eq!(
            ids(&t(1, None, 100, None, TargetMode::Held, None)),
            vec![1, 3, 5, 2, 4]
        );
        let big = t(1, None, 2_938, bound, TargetMode::Admit, None);
        assert_eq!(ids(&big), vec![1, 3, 5, 2]);
        assert!(!big.too_large, "other targets remain");
        let only_carrier = [pair_at(4, Peer, Carrier)];
        let tl = forward_targets(
            &only_carrier,
            1,
            None,
            2_938,
            bound,
            TargetMode::Admit,
            None,
        );
        assert!(tl.list.is_empty() && tl.too_large);
        let fits = forward_targets(
            &only_carrier,
            1,
            None,
            2_937,
            bound,
            TargetMode::Admit,
            None,
        );
        assert_eq!(ids(&fits), vec![4]);
        let unset = forward_targets(&only_carrier, 1, None, 100, None, TargetMode::Admit, None);
        assert!(
            unset.list.is_empty() && !unset.too_large,
            "no bound: no carrier, not too large"
        );
        // From a leaf: fitting carriers only.
        assert_eq!(
            ids(&t(2, None, 100, bound, TargetMode::Admit, None)),
            vec![4]
        );
        assert!(t(2, None, 2_938, bound, TargetMode::Admit, None).too_large);
        // From the carrier: leaf pairs only; a hint naming no leaf ⇒ none.
        assert_eq!(
            ids(&t(3, None, 100, bound, TargetMode::Admit, None)),
            vec![3]
        );
        assert_eq!(
            ids(&t(3, hint(3), 100, bound, TargetMode::Admit, None)),
            vec![3]
        );
        assert!(t(3, hint(1), 100, bound, TargetMode::Admit, None)
            .list
            .is_empty());
        assert!(t(0, None, 100, bound, TargetMode::Admit, None)
            .list
            .is_empty());
        assert!(
            forward_targets(&[], 1, None, 100, bound, TargetMode::Admit, None)
                .list
                .is_empty()
        );
    }

    /// Layer 7 §5.5: the hit cache holds at most 256 entries, oldest
    /// evicted, one per dest.
    #[test]
    fn the_hit_cache_is_bounded() {
        let mut f = Forwarder::default();
        let p = NodeId::from([1u8; 32]);
        let q = NodeId::from([2u8; 32]);
        for i in 0..300u32 {
            let mut d = [0u8; 32];
            d[..4].copy_from_slice(&i.to_le_bytes());
            f.cache_put(d, p);
        }
        assert_eq!(f.cache.len(), FORWARD_CACHE);
        let first = [0u8; 32];
        assert_eq!(f.cache.get(&first), None, "the oldest went");
        let mut last = [0u8; 32];
        last[..4].copy_from_slice(&299u32.to_le_bytes());
        assert_eq!(f.cache.get(&last), Some(p));
        f.cache_put(last, q);
        assert_eq!(
            f.cache.get(&last),
            Some(q),
            "one entry per dest, the newest"
        );
        assert_eq!(f.cache.len(), FORWARD_CACHE);
    }

    /// Layer 7 §5.11: without the hub role pairing (both directions) and
    /// the hand-off are refused; nothing is recorded.
    #[tokio::test]
    async fn no_pairing_and_no_hand_off_without_the_hub_role() {
        let hub = test_node("hub", FabricSettings::default());
        let other = test_node("other", FabricSettings::default());
        let card = card_text(&other.sealer.own_announce());
        let r = hub
            .f
            .pair(None, "ABCD-EFGH", &card, PairRole::Peer, PairVia::Tcp)
            .await;
        assert!(matches!(r, Err(FabricError::Invalid(_))), "{r:?}");
        let peer = other.f.node_id();
        assert!(hub.f.forward_take(peer, 8).await.is_err());
        assert!(hub.f.forward_repost(peer).await.is_err());
        let frame = vec![0u8; MIN_PACKET_LEN];
        assert!(hub.f.carrier_inbound(peer, frame).await.is_err());
        assert!(hub.f.carrier_inbound(peer, Vec::new()).await.is_err());
        hub.f.forward_done(7, ForwardDone::Taken).await.unwrap();
        assert!(!hub.f.unpair(peer).await.unwrap());
        let r = hub.f.offer_pair(None, PairRole::Peer, PairVia::Tcp).await;
        assert!(matches!(r, Err(FabricError::Invalid(_))), "{r:?}");
        let st = hub.f.status().await.unwrap();
        assert!(st.settings.paired_hubs.is_empty() && st.paired.is_empty());
        assert_eq!(st.counters.forward_refused, 0);
        // With the hub role: a pairing offer only for a pair §4.6 allows,
        // and no `111HUB:` bundle for it (the `111PAIR:` one is the daemon's).
        let real = test_node(
            "real-hub",
            FabricSettings {
                hub_role: true,
                ..FabricSettings::default()
            },
        );
        let at: SocketAddr = "192.0.2.1:11183".parse().unwrap();
        let r = real
            .f
            .offer_pair(Some(at), PairRole::Leaf, PairVia::Carrier)
            .await;
        assert!(matches!(r, Err(FabricError::Invalid(_))), "{r:?}");
        let v = real
            .f
            .offer_pair(Some(at), PairRole::Leaf, PairVia::Tcp)
            .await
            .unwrap();
        assert!(v.bundle.is_none() && v.address == Some(at));
        assert!(real.f.offer(Some(at)).await.unwrap().bundle.is_some());
    }

    /// The resident window's per-row cost (module doc: about 250 B a row
    /// with the indexes, ≈ 1 MB at [`FORWARD_RESIDENT`]).
    #[test]
    fn a_resident_row_is_small() {
        assert!(std::mem::size_of::<FwdRow>() <= 160);
        assert_eq!(FORWARD_RESIDENT % FORWARD_LOAD_PAGE, 0);
    }

    /// M4c (module doc "The resident window"): the window gives up the
    /// newest settled idle row whose turn is down or that has no target,
    /// never an up target's, a dirty row or a put out; an eviction moves
    /// the paging floor below the row and leaves rows in the store.
    #[test]
    fn eviction_takes_the_newest_idle_row_whose_turn_is_down() {
        let mut f = Forwarder::default();
        let (b, c) = (NodeId::from([1u8; 32]), NodeId::from([2u8; 32]));
        for seq in 1..=6u64 {
            let n = u8::try_from(seq).unwrap();
            f.insert(seq, Core::fwd_row([n; 32], [n; 16], 100, 1, None));
        }
        f.dirty.clear();
        assert!(f.unturned.is_empty(), "a fresh row sits in no queue");
        for (seq, t) in [(1, b), (2, b), (3, c), (4, b), (5, c)] {
            f.mutate(seq, |r| {
                r.fresh = false;
                r.turn = Some(t);
            });
        }
        f.mutate(6, |r| r.fresh = false);
        assert!(f.unturned.contains(&6), "no target: it waits");
        f.set_state(4, Turn::InFlight);
        f.loaded = 6;
        let up: HashSet<NodeId> = [c].into_iter().collect();
        assert_eq!(f.evictable(&up), Some(6), "the newest: no target");
        f.evict(6);
        assert_eq!((f.loaded, f.more, f.rows.len()), (5, true, 5));
        assert_eq!(f.evictable(&up), Some(2), "B's newest idle row");
        f.dirty.insert(2);
        assert_eq!(f.evictable(&up), Some(1), "a dirty row is not settled");
        f.dirty.clear();
        f.evict(2);
        assert_eq!(f.loaded, 1);
        let all: HashSet<NodeId> = [b, c].into_iter().collect();
        assert_eq!(f.evictable(&all), None, "every target up");
        f.set_state(1, Turn::Full);
        assert_eq!(f.evictable(&up), Some(1), "kept after REJECT_FULL");
    }

    /// M4d (module doc "The resident window"): each down target still gives
    /// up only its newest row, and among those candidates a row with no
    /// hint, no refusal and not orphaned goes first (eviction drops all
    /// three); with none such, the newest candidate.
    #[test]
    fn eviction_prefers_a_row_that_loses_nothing() {
        let mut f = Forwarder::default();
        let (b, c, d) = (
            NodeId::from([1u8; 32]),
            NodeId::from([2u8; 32]),
            NodeId::from([3u8; 32]),
        );
        for seq in 1..=5u64 {
            let n = u8::try_from(seq).unwrap();
            let hint = (seq == 4).then_some([9u8; DEST_HUB_LEN]);
            f.insert(seq, Core::fwd_row([n; 32], [n; 16], 100, 1, hint));
        }
        f.dirty.clear();
        for (seq, t) in [(1, b), (2, c), (3, c), (4, b)] {
            f.mutate(seq, |r| {
                r.fresh = false;
                r.turn = Some(t);
            });
        }
        f.mutate(5, |r| {
            r.fresh = false;
            r.orphaned = true;
        });
        f.loaded = 5;
        let none = HashSet::new();
        assert_eq!(
            f.evictable(&none),
            Some(3),
            "C's newest (plain) before B's newer hinted row or the orphan"
        );
        f.evict(3);
        f.mutate(2, |r| r.refused.push(d));
        assert_eq!(
            f.evictable(&none),
            Some(5),
            "no plain candidate (B's 4 hinted, C's 2 refused, 5 orphaned): the newest"
        );
        f.evict(5);
        assert_eq!(f.evictable(&none), Some(4), "B's newest, never its older 1");
        let up: HashSet<NodeId> = [b].into_iter().collect();
        assert_eq!(f.evictable(&up), Some(2), "B is up: C's only row");
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
        // B1-bounce F3: the row is gone, so the change is the
        // conversation's (no event names a row that no longer exists)
        assert_eq!(
            events(&n.f, &mut rx).await,
            vec![(d, None, ForeignChange::Conversation)]
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

    /// B1-bounce F8: a pre-air refusal without `Accepted` withdraws the
    /// pending mark through the actor; the row is offered again and is
    /// cancellable again. No event (as for `foreign_handoff_begin`).
    #[tokio::test]
    async fn a_pre_air_refusal_aborts_the_handoff_mark() {
        let n = node();
        let d = dm(&n.f, "dm:!0badc0de").await;
        let part = ForeignGroupPart {
            conv: d,
            text: "hello".into(),
            part_k: 1,
            part_n: 1,
        };
        let (_, ids) =
            n.f.foreign_group_create("hello", "[]", vec![part])
                .await
                .unwrap();
        let a = ids[0];
        let offered = |f: Fabric| async move {
            f.foreign_queued("meshtastic", 10)
                .await
                .unwrap()
                .iter()
                .any(|m| m.id == a)
        };
        assert!(matches!(
            n.f.foreign_handoff_abort(a).await,
            Err(FabricError::Invalid(_))
        ));
        n.f.foreign_handoff_begin(a).await.unwrap();
        assert!(!offered(n.f.clone()).await);
        let mut rx = subscribe(&n.f).await;
        n.f.foreign_handoff_abort(a).await.unwrap();
        assert!(events(&n.f, &mut rx).await.is_empty(), "bookkeeping only");
        assert!(!n.f.foreign_message(a).await.unwrap().handoff_pending);
        assert!(offered(n.f.clone()).await, "offered again");
        // twice is refused: no mark to withdraw
        assert!(matches!(
            n.f.foreign_handoff_abort(a).await,
            Err(FabricError::Invalid(_))
        ));
        assert!(matches!(
            n.f.foreign_handoff_abort(9_999).await,
            Err(FabricError::NotFound)
        ));
        // the meta setter cannot write the reserved mark (F7)
        assert!(matches!(
            n.f.foreign_set_meta(a, "handoff", Some(ForeignMeta::Str("pending".into())))
                .await,
            Err(FabricError::Invalid(_))
        ));
        assert!(offered(n.f.clone()).await);
        n.f.foreign_cancel(a).await.unwrap();
    }

    /// B1-bounce F4: an upsert of a conversation the actor does not have
    /// cached compares against the stored row: two identical upserts after
    /// a cache clear emit nothing; only the new row emitted.
    #[tokio::test]
    async fn an_uncached_identical_upsert_emits_nothing() {
        let n = node();
        let mut rx = subscribe(&n.f).await;
        let c = channel(&n.f, "!board", "LongFast").await;
        n.f.call(|core| core.foreign_convs.clear()).await.unwrap();
        assert_eq!(channel(&n.f, "!board", "LongFast").await, c);
        n.f.call(|core| core.foreign_convs.clear()).await.unwrap();
        assert_eq!(channel(&n.f, "!board", "LongFast").await, c);
        assert_eq!(
            events(&n.f, &mut rx).await,
            vec![(c, None, ForeignChange::Conversation)],
            "one event: the new row"
        );
        // a real difference while uncached still emits
        n.f.call(|core| core.foreign_convs.clear()).await.unwrap();
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
    async fn edge_options_are_set_without_touching_the_consent_or_the_marks() {
        let n = node();
        let src = channel(&n.f, "!board", "LongFast").await;
        let dst = channel(&n.f, "!board2", "Ops").await;
        let e =
            n.f.foreign_edge_create(ForeignEdgeNew {
                src_conv: src,
                dst_conv: dst,
                honour_ok_to_mqtt: false,
                oversize: EdgeOversize::Refuse,
                consent: consent('a'),
            })
            .await
            .unwrap();
        let mut rx = subscribe(&n.f).await;
        assert!(n
            .f
            .foreign_edge_set_options(e.id, true, EdgeOversize::Split)
            .await
            .unwrap());
        let after = n.f.foreign_edge_get(e.id).await.unwrap();
        assert!(after.honour_ok_to_mqtt);
        assert_eq!(after.oversize, EdgeOversize::Split);
        assert_eq!(
            (
                &after.consent_hash,
                after.cursor_seq,
                after.relayed_seq,
                after.paused
            ),
            (&e.consent_hash, e.cursor_seq, e.relayed_seq, e.paused),
            "only the two options change"
        );
        assert!(
            !n.f.foreign_edge_set_options(e.id, true, EdgeOversize::Split)
                .await
                .unwrap(),
            "the same values change nothing"
        );
        assert!(
            events(&n.f, &mut rx).await.is_empty(),
            "edge-only: no event"
        );
        assert!(matches!(
            n.f.foreign_edge_set_options(9_999, false, EdgeOversize::Refuse)
                .await,
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
