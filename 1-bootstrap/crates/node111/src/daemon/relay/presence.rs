//! Presence from the fabric's events (Layer 7 `os/7-relay/README.md` §2.1–§2.5; I1, I8; §19 C3 and
//! code seam 2; D31 Q2 as recommended).
//!
//! **Facts (§2.1).** A presence fact is `(peer, transport, evidence, local_seq, heard_at)`: the
//! peer's NodeId, where it was heard (`tcp-lan`, `hub` or `carrier`, the contract's closed
//! `NearbyEvidenceTransport` set), the evidence wording of §2.2, the store sequence of the event
//! that proved it (`heard_seq`; `None` for the hub's and a pair's answer, which no store row
//! records), and a monotonic [`Instant`] of this node's own clock. Facts live in memory for this
//! session only: no stored byte, `contacts.last_seen_seq` stays upsert-only (§19 C3), and after a
//! restart a row returns with its next fact. A peer's clock is never evidence; the age is
//! computed when the source is read (`heard_ago_s`), never refreshed by a timer.
//!
//! **What bumps last heard (§2.2).** From the fabric's tap ([`on_event`], called by
//! `relay::tap` for every `Message`, `Contact`, `Fabric` and `Forward` event of a batch) the work
//! is noted, and [`settle`] (called by the fabric handle at the end of the same batch, after its
//! `fabric` frame refreshed the kept views) reads what it needs from the fabric actor:
//!
//! | event | fact |
//! |---|---|
//! | an inbound message from a contact (`New`, `dir = in`; never a message request) | the contact's row, "a message from them arrived (verified)" |
//! | an own message that is `delivered` (once per message, in any order: its `local_seq` not among the last [`CONFIRMED_KEPT`] counted for that peer) | the contact's row, "they confirmed your message" |
//! | `hub.reachable` (a connection attributed to the home hub) | the hub's row, `tcp-lan`, "your hub answered" |
//! | a whole `tcp` pair with an attributed connection | its row, `tcp-lan`, "paired hub answered" |
//! | a whole `carrier` pair's first verified carrier frame this session | its row, `carrier`, "paired hub answered" |
//! | `relayed`, a card, an announce, a hello, a keepalive, a message request | nothing (L2 A6) |
//!
//! A message's transport is §2.2's rule on [`Fabric::attributed_on`] (§19 code seam 2), read when
//! the daemon handles the event: `tcp-lan` while a connection is attributed to the peer (whatever
//! link holds it: v1 builds only `tcp-lan` links, and the contract's closed set has no other
//! direct transport), else `hub` ("through your hub"). The honest limit of §2.1 stands: a message
//! that came through the hub while a direct connection to its sender was also attributed counts
//! as `tcp-lan`. A delivered fact's `heard_seq` is the message's own `local_seq`: the store keeps
//! no sequence for the confirmation itself, and no byte is added for it (§19 C3).
//!
//! **A paired hub (a recorded deviation, the D33 candidate on row D2).** §2.2 names a pair's
//! `CUSTODY_ACK` or forward put; mesh111 exposes no per-pair evidence of either (no event, no
//! counter, no view field), only [`PairView::reachable`]: for `tcp`, a connection attributed to the
//! far hub (any fresh verified packet on it, as `hub.reachable` for the home hub); for `carrier`, a
//! verified carrier frame this session (the carrier carries only puts and acks, so that one is
//! §2.2's evidence, but sticky). So a `tcp` pair refreshes on each batch that sees it attributed,
//! and a `carrier` pair counts once per rising edge of its view; the edge is consumed only once
//! its row is written (or the far hub is known not to be a contact), so a failed contact read
//! retries on the next `Fabric` event.
//!
//! **The `111` source (§2.3).** One source, `kind` `111`, label "111 nodes heard by this node",
//! `device_id` null, listed whenever presence is installed (empty until a fact). A row exists only
//! for a contact with a fact this session (a forgotten contact's row goes): `id` the 64-hex
//! NodeId, `num` 0, `long_name` the local alias (cleaned, 40 bytes) or the L2 label,
//! `short_name` the label, `last_heard_hint` null, `heard_ago_s`, `heard_seq`, `evidence` (one
//! entry per transport heard this session, the contract's `NearbyEvidence` with no age of its
//! own: `heard_ago_s` is the row's, from its latest fact, §2.3), `roles` (the signed role names,
//! bits 0–5 through D9's map). At most [`MAX_BRIDGE_NODES`] rows, the oldest heard forgotten
//! first. A change publishes the SSE `nearby` frame once per batch; a refreshed age alone is a
//! change only once it moved [`FRAME_AGE_STEP`] past the age the last published frame carried
//! (the UI ages a row from `heard_ago_s` and its own clock between frames), so facts refreshed
//! every few seconds still move the frame once a minute.
//!
//! **Weight (§1 rule 2; the Pi floor).** Nothing here owns a task, a timer, a socket or a file:
//! [`install`] allocates the empty book, [`on_event`] is one lock, and [`settle`] runs inside the
//! fabric pump that exists anyway, reading the actor per message event: none for a message
//! request, one (`message`) for an event that proves nothing, two (`message`, `attributed_on`)
//! for a fact on an existing row, three with the `contact` read of a new row; per `Fabric` batch
//! one `attributed_on` for the home hub (plus a new row's contact); the pairs come from the
//! handle's kept views. Each is one round trip to the actor that runs anyway. **After a pump
//! lag** the lost message events are not replayed (M5b): the synthetic `Fabric` event re-reads
//! the hub and the pairs only.
//!
//! **The heartbeat switch (§2.4).** Visible and inert in v1: [`check_heartbeat`] is the pure
//! validator D10's `PATCH /relay/settings` and the `presence.interval` proposal action call. It
//! answers `409 settings_locked` for `rns` (Layer 6's setting), `409 no_such_transport` for every
//! transport that takes no beat (all of them in v1, [`BEAT_TRANSPORTS_V1`]), and only after that
//! check the bans: a duty-cycle-limited band or a licensed board ⇒ `409 licensed_board`, a phone
//! under [`PHONE_MIN_INTERVAL_MIN`] ⇒ `400 bad_request`.
//!
//! **One daemon per process.** Like the gateway row, the book is a process static ([`install`]
//! binds it to the daemon's fabric handle; [`settle`] answers only that handle), because the tap
//! is a free function with no handle (M5b).

use crate::daemon::dashboard::api_v1::nearby::MAX_BRIDGE_NODES;
use crate::daemon::dashboard::api_v1::{nearby, ApiState, Problem};
use crate::daemon::fabric::sealer::role_names;
use crate::daemon::fabric::FabricHandle;
use crate::daemon::meshtastic::presence::sanitize_name;
use axum::http::StatusCode;
use id111::encoding::NodeIdDisplay;
use id111::keyset::NodeId;
use mesh111::fabric::{
    ContactChange, ContactView, Fabric, FabricError, FabricEvent, MessageChange, MessageView,
    PairView,
};
use mesh111::link::LinkKind;
use mesh111::store::{Dir, MsgState};
use mesh111::PairVia;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

/// The `111` source's heading (§2.3, verbatim).
pub const LABEL: &str = "111 nodes heard by this node";
/// `NearbySource.kind` of the presence source (§2.3).
pub const KIND: &str = "111";
/// A name at most, in UTF-8 bytes (the contract's `NearbyNode`: 40 bytes).
pub const MAX_NAME: usize = 40;
/// A refreshed fact with the same evidence and no newer sequence publishes a `nearby` frame only
/// once its age moved this far (§2.3 "on a change"): the UI ages rows between frames itself.
pub const FRAME_AGE_STEP: Duration = Duration::from_secs(60);
/// Distinct message and contact events noted per batch at most (a pump batch is smaller; the
/// rest of an overfull batch is dropped and counted, never an unbounded queue).
pub const MAX_WORK: usize = 1024;
/// Own-message `local_seq`s a row keeps as counted "they confirmed your message". mesh111 emits
/// `delivered` once per message (`Core::delivered` removes it), but the tap reads a message's
/// state when it handles the event, so an earlier event of the same message handled in a later
/// batch (its `sent`, say) reads `delivered` too: that repeat comes within moments of the real
/// one, with few other deliveries for the peer in between. A delivery out of send order (a
/// message fetched from the hub after a later one went direct) is a fact of its own and counts.
pub const CONFIRMED_KEPT: usize = 4;

/// Where a node was heard (`NearbyEvidence.transport`, §2.3; the closed set
/// `nearby::EVIDENCE_TRANSPORTS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Transport {
    /// A connection attributed to the node (also the home hub's and a `tcp` pair's row).
    TcpLan,
    /// A contact heard through the home hub.
    Hub,
    /// A `carrier` pair (its frames arrive through `carrier_inbound`, no link).
    Carrier,
}

impl Transport {
    /// The contract string.
    pub fn code(self) -> &'static str {
        match self {
            Transport::TcpLan => "tcp-lan",
            Transport::Hub => "hub",
            Transport::Carrier => "carrier",
        }
    }
}

/// The §2.2 evidence (the strings are verbatim).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// An inbound message from a contact.
    MessageIn,
    /// An own message turned `delivered`.
    Delivered,
    /// The home hub answered on its connection.
    HubAnswered,
    /// A paired hub answered.
    PairAnswered,
}

impl Evidence {
    /// The evidence string (§2.2, verbatim).
    pub fn text(self) -> &'static str {
        match self {
            Evidence::MessageIn => "a message from them arrived (verified)",
            Evidence::Delivered => "they confirmed your message",
            Evidence::HubAnswered => "your hub answered",
            Evidence::PairAnswered => "paired hub answered",
        }
    }
}

/// A row's names and roles (§2.3), from the contact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Names {
    /// The local alias (cleaned, at most [`MAX_NAME`] bytes) or the L2 label.
    pub long_name: String,
    /// The L2 label.
    pub short_name: String,
    /// The signed role names (D9's map, `sealer::role_names`).
    pub roles: Vec<&'static str>,
}

impl Names {
    /// The names a contact shows: an alias that cleans to nothing falls back to the label.
    pub fn of(c: &ContactView) -> Names {
        let label = NodeIdDisplay::new(c.node_id.as_bytes()).label();
        let alias = c
            .alias
            .as_deref()
            .map(|a| sanitize_name(a, MAX_NAME))
            .filter(|a| !a.is_empty());
        Names {
            long_name: alias.unwrap_or_else(|| label.clone()),
            short_name: label,
            roles: role_names(c.roles),
        }
    }
}

#[derive(Debug, Clone)]
struct Fact {
    transport: Transport,
    evidence: Evidence,
    heard_at: Instant,
}

#[derive(Debug, Clone)]
struct Row {
    peer: NodeId,
    names: Names,
    /// One per transport.
    facts: Vec<Fact>,
    /// The highest store sequence of a proving event (monotone; `None` until a message fact).
    heard_seq: Option<u64>,
    /// The latest fact's time.
    heard_at: Instant,
    /// The `heard_at` the last published source carried for this row: a refreshed age is a
    /// change once `heard_at` moved [`FRAME_AGE_STEP`] past it.
    shown_at: Instant,
    /// The last [`CONFIRMED_KEPT`] own-message `local_seq`s counted as "they confirmed your
    /// message", newest first: a delivered message among them is a repeat, no new fact.
    confirmed: [Option<u64>; CONFIRMED_KEPT],
}

impl Row {
    fn note_confirmed(&mut self, seq: u64) {
        if !self.confirmed.contains(&Some(seq)) {
            self.confirmed.rotate_right(1);
            self.confirmed[0] = Some(seq);
        }
    }
}

/// The session's rows, oldest heard first, at most [`MAX_BRIDGE_NODES`].
#[derive(Debug, Default)]
struct Book {
    rows: VecDeque<Row>,
}

impl Book {
    fn position(&self, peer: &NodeId) -> Option<usize> {
        self.rows.iter().position(|r| r.peer == *peer)
    }

    fn has(&self, peer: &NodeId) -> bool {
        self.position(peer).is_some()
    }

    /// Whether the own message `seq` to `peer` was already counted as delivered.
    fn confirmed(&self, peer: &NodeId, seq: u64) -> bool {
        self.rows
            .iter()
            .find(|r| r.peer == *peer)
            .is_some_and(|r| r.confirmed.contains(&Some(seq)))
    }

    /// The rows as a published frame shows them: each row's age is current there.
    fn mark_shown(&mut self) {
        for r in self.rows.iter_mut() {
            r.shown_at = r.heard_at;
        }
    }

    /// Record one fact at `now`. A new row needs `names` (a contact); without them nothing is
    /// created. Returns whether the shown source changed (module doc: a new row or transport,
    /// other evidence, a higher `heard_seq`, other names, or the row's age moved
    /// [`FRAME_AGE_STEP`] past the one last shown).
    fn record(
        &mut self,
        peer: NodeId,
        names: Option<Names>,
        fact: (Transport, Evidence),
        seq: Option<u64>,
        now: Instant,
    ) -> bool {
        let (transport, evidence) = fact;
        let fresh = Fact {
            transport,
            evidence,
            heard_at: now,
        };
        let Some(i) = self.position(&peer) else {
            let Some(names) = names else {
                return false;
            };
            self.rows.push_back(Row {
                peer,
                names,
                facts: vec![fresh],
                heard_seq: seq,
                heard_at: now,
                shown_at: now,
                confirmed: [None; CONFIRMED_KEPT],
            });
            while self.rows.len() > MAX_BRIDGE_NODES {
                self.rows.pop_front();
            }
            return true;
        };
        let Some(mut row) = self.rows.remove(i) else {
            return false;
        };
        let mut changed = false;
        match row.facts.iter_mut().find(|f| f.transport == transport) {
            Some(f) => {
                changed |= f.evidence != evidence;
                *f = fresh;
            }
            None => {
                row.facts.push(fresh);
                changed = true;
            }
        }
        if let Some(s) = seq {
            if row.heard_seq.is_none_or(|h| s > h) {
                row.heard_seq = Some(s);
                changed = true;
            }
        }
        if let Some(n) = names {
            if n != row.names {
                row.names = n;
                changed = true;
            }
        }
        row.heard_at = now;
        changed |= now.saturating_duration_since(row.shown_at) >= FRAME_AGE_STEP;
        if changed {
            row.shown_at = now;
        }
        self.rows.push_back(row);
        changed
    }

    /// New names for a row (a renamed contact, a refreshed announce's roles).
    fn rename(&mut self, peer: &NodeId, names: Names) -> bool {
        match self.rows.iter_mut().find(|r| r.peer == *peer) {
            Some(r) if r.names != names => {
                r.names = names;
                true
            }
            _ => false,
        }
    }

    /// A forgotten contact's row goes (a row only for a contact, §2.3).
    fn forget(&mut self, peer: &NodeId) -> bool {
        let before = self.rows.len();
        self.rows.retain(|r| r.peer != *peer);
        self.rows.len() != before
    }

    /// The rows as `NearbyNode`s, most recently heard first, ages taken at `now`.
    fn nodes(&self, now: Instant) -> Vec<Value> {
        self.rows.iter().rev().map(|r| node_json(r, now)).collect()
    }
}

/// One `111` row (`NearbyNode`, §2.3, §13.2): the RF fields are null, `num` 0.
fn node_json(r: &Row, now: Instant) -> Value {
    let mut facts: Vec<&Fact> = r.facts.iter().collect();
    facts.sort_by_key(|f| f.transport);
    json!({
        "id": NodeIdDisplay::new(r.peer.as_bytes()).hex(),
        "num": 0,
        "long_name": r.names.long_name,
        "short_name": r.names.short_name,
        "pubkey": null,
        "snr": null,
        "rssi": null,
        "hops_away": null,
        "last_heard_hint": null,
        "position": null,
        "channel_utilization": null,
        "heard_ago_s": now.saturating_duration_since(r.heard_at).as_secs(),
        "heard_seq": r.heard_seq,
        "evidence": facts
            .iter()
            .map(|f| json!({ "transport": f.transport.code(), "text": f.evidence.text() }))
            .collect::<Vec<_>>(),
        "roles": r.names.roles,
        "position_precision_bits": null,
    })
}

/// What the tap noted for the next [`settle`] (bounded by [`MAX_WORK`]).
#[derive(Debug, Default)]
struct Work {
    /// Distinct message ids with the event's peer and their first change.
    messages: Vec<(i64, NodeId, MessageChange)>,
    /// The last change per contact.
    contacts: Vec<(NodeId, ContactChange)>,
    /// A `Fabric` event (links, hub, custody; after a lag "re-read everything").
    fabric: bool,
}

impl Work {
    fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.contacts.is_empty() && !self.fabric
    }
}

#[derive(Default)]
struct State {
    /// The daemon's fabric handle (the only one [`settle`] answers).
    fabric: Weak<FabricHandle>,
    book: Book,
    work: Work,
    /// `carrier` pairs whose first verified frame this session is recorded (a rising edge).
    carrier_heard: Vec<NodeId>,
    /// Events dropped from an overfull batch (diagnostics).
    dropped: u64,
}

/// The session's presence facts (§2.1) and the `111` source (§2.3). One per process
/// ([`install`]); in `ApiState` ext for `GET /nearby`.
#[derive(Default)]
pub struct Presence {
    state: Mutex<State>,
}

impl std::fmt::Debug for Presence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.lock();
        f.debug_struct("Presence")
            .field("rows", &s.book.rows.len())
            .field("dropped", &s.dropped)
            .finish_non_exhaustive()
    }
}

/// The fabric reads presence makes (the actor's `message`, `contact` and `attributed_on`); a
/// trait so the unit tests drive the batch logic without a store.
pub trait Reads: Sync {
    /// [`Fabric::message`].
    fn message(&self, id: i64) -> impl Future<Output = Result<MessageView, FabricError>> + Send;
    /// [`Fabric::contact`].
    fn contact(&self, id: NodeId) -> impl Future<Output = Result<ContactView, FabricError>> + Send;
    /// [`Fabric::attributed_on`] (§19 code seam 2).
    fn attributed_on(
        &self,
        peer: NodeId,
    ) -> impl Future<Output = Result<Option<LinkKind>, FabricError>> + Send;
}

impl Reads for Fabric {
    fn message(&self, id: i64) -> impl Future<Output = Result<MessageView, FabricError>> + Send {
        Fabric::message(self, id)
    }
    fn contact(&self, id: NodeId) -> impl Future<Output = Result<ContactView, FabricError>> + Send {
        Fabric::contact(self, id)
    }
    fn attributed_on(
        &self,
        peer: NodeId,
    ) -> impl Future<Output = Result<Option<LinkKind>, FabricError>> + Send {
        Fabric::attributed_on(self, peer)
    }
}

/// What [`Presence::lookup`] found for a peer.
enum Lookup {
    /// The row exists (its names stand).
    Row,
    /// A contact without a row yet.
    New(Names),
    /// Not a contact (or a message request): no row.
    NotContact,
    /// The read failed: no row now, the fact may come again.
    Failed,
}

/// What the fabric handle knows without the actor, read once per batch: the home hub and the
/// per-pair views it kept from its last `Fabric::status` (`FabricHandle::pair_views`).
#[derive(Debug, Clone, Default)]
pub struct Kept {
    /// The home hub's NodeId (a member).
    pub home_hub: Option<NodeId>,
    /// `FabricStatus::paired` as last read.
    pub pairs: Vec<PairView>,
}

impl Presence {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Note one tapped event (module doc): one lock, no read, no allocation beyond the bounded
    /// lists.
    pub fn note(&self, event: &FabricEvent) {
        let mut s = self.lock();
        let s = &mut *s;
        match event {
            FabricEvent::Message { id, peer, change } => {
                if s.work.messages.iter().any(|(m, _, _)| m == id) {
                    return;
                }
                if s.work.messages.len() >= MAX_WORK {
                    s.dropped += 1;
                    return;
                }
                s.work.messages.push((*id, *peer, *change));
            }
            FabricEvent::Contact { node_id, change } => {
                s.work.contacts.retain(|(n, _)| n != node_id);
                if s.work.contacts.len() >= MAX_WORK {
                    s.dropped += 1;
                    return;
                }
                s.work.contacts.push((*node_id, *change));
            }
            FabricEvent::Fabric => s.work.fabric = true,
            // the carrier's trigger (D11) proves nothing about the far hub; its verified
            // frames show in the pair's view on the `Fabric` event that follows them
            FabricEvent::Forward { .. }
            | FabricEvent::Settings(_)
            | FabricEvent::Foreign { .. } => {}
        }
    }

    /// Bind the book to the daemon's fabric handle (a different handle starts a new session).
    fn bind(&self, fabric: &Arc<FabricHandle>) {
        let mut s = self.lock();
        if !std::ptr::eq(s.fabric.as_ptr(), Arc::as_ptr(fabric)) {
            *s = State {
                fabric: Arc::downgrade(fabric),
                ..State::default()
            };
        }
    }

    /// The noted work, when `fh` is the bound handle and there is some.
    fn take_work(&self, fh: &FabricHandle) -> Option<Work> {
        let mut s = self.lock();
        if !std::ptr::eq(s.fabric.as_ptr(), fh) || s.work.is_empty() {
            return None;
        }
        Some(std::mem::take(&mut s.work))
    }

    /// Turn one batch's work into facts at `now` (module doc table). Every read failure leaves
    /// the fact out (fail closed); returns whether the source changed. No lock is held across a
    /// read.
    async fn apply<R: Reads>(&self, r: &R, work: Work, kept: &Kept, now: Instant) -> bool {
        let mut changed = false;
        for (id, change) in &work.contacts {
            match change {
                ContactChange::Removed => changed |= self.lock().book.forget(id),
                ContactChange::Updated => {
                    if !self.lock().book.has(id) {
                        continue;
                    }
                    match r.contact(*id).await {
                        Ok(c) if !c.request => {
                            changed |= self.lock().book.rename(id, Names::of(&c))
                        }
                        Ok(_) | Err(FabricError::NotFound) => {
                            changed |= self.lock().book.forget(id)
                        }
                        Err(_) => {}
                    }
                }
                // a card, an enrollment or a message request proves no liveness (L2 A6)
                ContactChange::Added | ContactChange::Request => {}
            }
        }
        for (id, peer, change) in &work.messages {
            // a message request proves nothing (§2.2): not even read
            if *change == MessageChange::Request {
                continue;
            }
            let Ok(v) = r.message(*id).await else {
                continue;
            };
            if v.peer != *peer {
                continue;
            }
            let (evidence, seq) = match (v.dir, change, v.state) {
                (Dir::In, MessageChange::New, _) => {
                    (Evidence::MessageIn, v.received_seq.unwrap_or(v.local_seq))
                }
                (Dir::Own, _, MsgState::Delivered) => {
                    // once per message, in any order ([`CONFIRMED_KEPT`])
                    if self.lock().book.confirmed(peer, v.local_seq) {
                        continue;
                    }
                    (Evidence::Delivered, v.local_seq)
                }
                // relayed, sent, queued, failed, a message request: nothing (§2.2)
                _ => continue,
            };
            // §2.2: `tcp-lan` when a connection is attributed to them, else `hub`
            let transport = match r.attributed_on(*peer).await {
                Ok(Some(_)) => Transport::TcpLan,
                Ok(None) => Transport::Hub,
                Err(_) => continue,
            };
            let names = match self.lookup(r, peer).await {
                Lookup::Row => None,
                Lookup::New(n) => Some(n),
                Lookup::NotContact | Lookup::Failed => continue,
            };
            let mut s = self.lock();
            changed |= s
                .book
                .record(*peer, names, (transport, evidence), Some(seq), now);
            if evidence == Evidence::Delivered {
                if let Some(row) = s.book.rows.iter_mut().find(|row| row.peer == *peer) {
                    row.note_confirmed(seq);
                }
            }
        }
        if work.fabric {
            changed |= self.apply_fabric(r, kept, now).await;
        }
        if changed {
            // the frame settle publishes now carries every row's current age
            self.lock().book.mark_shown();
        }
        changed
    }

    /// The hub's and the pairs' facts on a `Fabric` event (module doc table).
    async fn apply_fabric<R: Reads>(&self, r: &R, kept: &Kept, now: Instant) -> bool {
        let mut changed = false;
        if let Some(hub) = kept.home_hub {
            if let Ok(Some(_)) = r.attributed_on(hub).await {
                let names = match self.lookup(r, &hub).await {
                    Lookup::Row => Some(None),
                    Lookup::New(n) => Some(Some(n)),
                    Lookup::NotContact | Lookup::Failed => None,
                };
                if let Some(names) = names {
                    changed |= self.lock().book.record(
                        hub,
                        names,
                        (Transport::TcpLan, Evidence::HubAnswered),
                        None,
                        now,
                    );
                }
            }
        }
        for v in &kept.pairs {
            let id = v.hub.node_id;
            let answered = v.whole && !v.broken && v.reachable;
            let transport = match v.hub.via {
                PairVia::Tcp if answered => Transport::TcpLan,
                PairVia::Tcp => continue,
                PairVia::Carrier => {
                    let mut s = self.lock();
                    if !answered {
                        s.carrier_heard.retain(|n| *n != id);
                        continue;
                    }
                    if s.carrier_heard.contains(&id) {
                        continue;
                    }
                    Transport::Carrier
                }
            };
            // a carrier pair's rising edge is consumed only once it is settled: its row
            // written, or the far hub known not to be a contact; a failed read keeps it
            let names = match self.lookup(r, &id).await {
                Lookup::Row => None,
                Lookup::New(n) => Some(n),
                Lookup::NotContact => {
                    if transport == Transport::Carrier {
                        self.lock().carrier_heard.push(id);
                    }
                    continue;
                }
                Lookup::Failed => continue,
            };
            let mut s = self.lock();
            changed |= s
                .book
                .record(id, names, (transport, Evidence::PairAnswered), None, now);
            if transport == Transport::Carrier && !s.carrier_heard.contains(&id) {
                s.carrier_heard.push(id);
            }
        }
        // a pair that is no longer listed starts a new rising edge if it returns
        self.lock()
            .carrier_heard
            .retain(|n| kept.pairs.iter().any(|v| v.hub.node_id == *n));
        changed
    }

    /// Whether `peer` has a row, or may get one: a row needs a contact (§2.3), read only when
    /// there is none yet.
    async fn lookup<R: Reads>(&self, r: &R, peer: &NodeId) -> Lookup {
        if self.lock().book.has(peer) {
            return Lookup::Row;
        }
        match r.contact(*peer).await {
            Ok(c) if !c.request => Lookup::New(Names::of(&c)),
            Ok(_) | Err(FabricError::NotFound) => Lookup::NotContact,
            Err(_) => Lookup::Failed,
        }
    }

    /// The `111` `NearbySource` (§2.3) at `now`; `own` names the source's id.
    pub fn source(&self, own: Option<NodeId>, now: Instant) -> Value {
        let id = match own {
            Some(n) => format!("{KIND}:{}", NodeIdDisplay::new(n.as_bytes()).hex()),
            None => KIND.to_string(),
        };
        let nodes = self.lock().book.nodes(now);
        json!({
            "id": id,
            "kind": KIND,
            "label": LABEL,
            "device_id": null,
            "nodes": nodes,
        })
    }

    /// The source as `GET /nearby` and the `nearby` frame list it (the bound fabric's NodeId).
    pub fn source_now(&self) -> Value {
        let own = self
            .lock()
            .fabric
            .upgrade()
            .and_then(|h| h.fabric().map(Fabric::node_id));
        self.source(own, Instant::now())
    }
}

static PRESENCE: OnceLock<Arc<Presence>> = OnceLock::new();

/// Install presence for the daemon (`relay::start`, once): bind the process's book to `fabric`
/// and put it in `api` ext, so `GET /nearby` lists the `111` source. Presence has no switch
/// (§1: event-derived on every category); nothing is spawned, scheduled, opened or read.
pub fn install(api: &Arc<ApiState>, fabric: &Arc<FabricHandle>) {
    let p = PRESENCE.get_or_init(|| Arc::new(Presence::default()));
    p.bind(fabric);
    api.insert_ext(Arc::clone(p));
}

/// One fabric event from `relay::tap` (§2.2): noted for this batch's [`settle`]; returns at once
/// until [`install`] ran.
pub fn on_event(event: &FabricEvent) {
    if let Some(p) = PRESENCE.get() {
        p.note(event);
    }
}

/// The end of one fabric batch (called by the fabric handle after the batch's frames, its
/// `fabric` frame included, so the kept pair views are this batch's): turn the noted work into
/// facts and publish the `nearby` frame when the source changed. A no-op until [`install`] ran,
/// for any handle but the bound one, and for a batch with nothing noted.
pub async fn settle(fh: &FabricHandle, api: &ApiState) {
    let Some(p) = PRESENCE.get() else {
        return;
    };
    let Some(work) = p.take_work(fh) else {
        return;
    };
    let Some(f) = fh.fabric() else {
        return;
    };
    let kept = Kept {
        home_hub: fh.persisted().fabric.home_hub.map(|h| h.node_id),
        pairs: if work.fabric {
            fh.pair_views().views
        } else {
            Vec::new()
        },
    };
    if p.apply(f, work, &kept, Instant::now()).await {
        nearby::publish(api);
    }
}

// ---------------------------------------------------------------- the heartbeat validator (§2.4)

/// The transports that take a beat in v1: none (§2.4 item 2; I4, §19 D15). A beat needs a sealed
/// LoRa link (v1.1); `tcp-lan`'s keepalive already proves its link.
pub const BEAT_TRANSPORTS_V1: [&str; 0] = [];
/// The lowest interval on a phone (Termux): Doze fires exact alarms at most once per nine minutes
/// (§2.4 item 3; R01 §3 finding 9).
pub const PHONE_MIN_INTERVAL_MIN: u32 = 9;
/// The `rns` row is Layer 6's: the LXMF helper's announce interval (§2.4 item 2).
pub const RNS_TRANSPORT: &str = "rns";

/// What the validator is told about this node (the caller reads it: the transports offered a
/// beat, the bound radio's region and board, the device category).
#[derive(Debug, Clone, Copy, Default)]
pub struct BeatContext<'a> {
    /// Transports that take a beat ([`BEAT_TRANSPORTS_V1`] in v1; unit tests mark one).
    pub offered: &'a [&'a str],
    /// The region table marks the transport's band duty-cycle limited.
    pub duty_cycle_limited: bool,
    /// The board is marked licensed.
    pub licensed_board: bool,
    /// A phone (the Termux category).
    pub phone: bool,
}

/// Why a heartbeat write is refused (§2.4; §13.1 `PATCH /relay/settings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatRefusal {
    /// `409 settings_locked`: `rns` is Layer 6's `PATCH /api/v1/bridge/settings`.
    SettingsLocked,
    /// `409 no_such_transport`: no beat on this transport (every one in v1).
    NoSuchTransport,
    /// `400 bad_request`: above the stored range (a week).
    OutOfRange,
    /// `409 licensed_board`: a duty-cycle-limited band or a licensed board.
    LicensedBoard,
    /// `400 bad_request`: a phone below [`PHONE_MIN_INTERVAL_MIN`].
    PhoneFloor,
}

impl HeartbeatRefusal {
    /// The contract problem (§13.1; the codes of G8).
    pub fn problem(self) -> Problem {
        match self {
            HeartbeatRefusal::SettingsLocked => Problem::new(
                StatusCode::CONFLICT,
                "settings_locked",
                "The rns announce interval is the bridge's setting.",
            )
            .detail(
                "It is the LXMF helper's announce interval: change it with PATCH \
                 /api/v1/bridge/settings.",
            ),
            HeartbeatRefusal::NoSuchTransport => Problem::new(
                StatusCode::CONFLICT,
                "no_such_transport",
                "This transport takes no heartbeat.",
            )
            .detail("No transport takes a heartbeat in this release: presence comes from events."),
            HeartbeatRefusal::OutOfRange => Problem::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "The heartbeat interval is out of range.",
            )
            .detail(format!(
                "Use 0 (events only) to {} minutes.",
                crate::daemon::relay::settings::MAX_HEARTBEAT_MIN
            )),
            HeartbeatRefusal::LicensedBoard => Problem::new(
                StatusCode::CONFLICT,
                "licensed_board",
                "No heartbeat on a duty-cycle-limited band or a licensed board.",
            ),
            HeartbeatRefusal::PhoneFloor => Problem::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "On a phone the heartbeat is at least 9 minutes.",
            )
            .detail("Android fires exact alarms at most once per nine minutes."),
        }
    }
}

/// The inert heartbeat validator (§2.4 items 2–3; a pure function). In order: `rns` is Layer 6's
/// (`settings_locked`); a transport not [`BeatContext::offered`] takes no beat
/// (`no_such_transport`, every transport in v1); an interval above the stored range is
/// `bad_request`; `0` (events only) sends nothing, so no ban applies; then the bans: a
/// duty-cycle-limited band or a licensed board (`licensed_board`), a phone below nine minutes
/// (`bad_request`).
pub fn check_heartbeat(
    transport: &str,
    minutes: u32,
    cx: &BeatContext<'_>,
) -> Result<(), HeartbeatRefusal> {
    if transport == RNS_TRANSPORT {
        return Err(HeartbeatRefusal::SettingsLocked);
    }
    if !cx.offered.contains(&transport) {
        return Err(HeartbeatRefusal::NoSuchTransport);
    }
    if minutes > crate::daemon::relay::settings::MAX_HEARTBEAT_MIN {
        return Err(HeartbeatRefusal::OutOfRange);
    }
    if minutes == 0 {
        return Ok(());
    }
    if cx.duty_cycle_limited || cx.licensed_board {
        return Err(HeartbeatRefusal::LicensedBoard);
    }
    if cx.phone && minutes < PHONE_MIN_INTERVAL_MIN {
        return Err(HeartbeatRefusal::PhoneFloor);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::dashboard::api_v1::nearby::EVIDENCE_TRANSPORTS;
    use mesh111::fabric::PairedHub;
    use mesh111::ids::MsgSeq;
    use mesh111::store::Trust;
    use mesh111::PairRole;
    use std::collections::HashMap;

    fn nid(n: u8) -> NodeId {
        NodeId::from([n; 32])
    }

    fn hex(n: NodeId) -> String {
        NodeIdDisplay::new(n.as_bytes()).hex()
    }

    fn label(n: NodeId) -> String {
        NodeIdDisplay::new(n.as_bytes()).label()
    }

    fn msg(id: i64, peer: NodeId, dir: Dir, state: MsgState, seq: u64) -> MessageView {
        MessageView {
            id,
            peer,
            dir,
            kind: 1,
            text: Some("hi".into()),
            tombstone: false,
            state,
            reason: None,
            local_seq: seq,
            received_seq: (dir == Dir::In).then_some(seq),
            sent_at_claim: None,
            stream: 0,
            seq: MsgSeq::new(1, 1),
        }
    }

    fn contact(id: NodeId, alias: Option<&str>, roles: u16) -> ContactView {
        ContactView {
            node_id: id,
            alias: alias.map(Into::into),
            trust: Trust::Oob,
            request: false,
            pinned: false,
            has_key: true,
            roles,
            home_hub: None,
        }
    }

    fn pair(id: NodeId, via: PairVia, whole: bool, reachable: bool) -> PairView {
        PairView {
            hub: PairedHub {
                node_id: id,
                role: PairRole::Peer,
                via,
                ours: whole,
                theirs: whole,
            },
            whole,
            broken: false,
            reachable,
            pending: 0,
            reposts: 0,
        }
    }

    /// The fabric's reads, scripted: every call is counted (the weight checks).
    #[derive(Default)]
    struct Fake {
        messages: HashMap<i64, MessageView>,
        contacts: HashMap<NodeId, ContactView>,
        attributed: HashMap<NodeId, LinkKind>,
        /// Every `contact` read fails (the actor is busy or stopping), not "not found".
        contact_fails: std::sync::atomic::AtomicBool,
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn count(&self, what: &str) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.starts_with(what))
                .count()
        }
        fn log(&self, what: String) {
            self.calls.lock().unwrap().push(what);
        }
    }

    impl Reads for Fake {
        fn message(
            &self,
            id: i64,
        ) -> impl Future<Output = Result<MessageView, FabricError>> + Send {
            self.log(format!("message {id}"));
            std::future::ready(self.messages.get(&id).cloned().ok_or(FabricError::NotFound))
        }
        fn contact(
            &self,
            id: NodeId,
        ) -> impl Future<Output = Result<ContactView, FabricError>> + Send {
            self.log("contact".into());
            if self.contact_fails.load(std::sync::atomic::Ordering::SeqCst) {
                return std::future::ready(Err(FabricError::Stopped));
            }
            std::future::ready(self.contacts.get(&id).cloned().ok_or(FabricError::NotFound))
        }
        fn attributed_on(
            &self,
            peer: NodeId,
        ) -> impl Future<Output = Result<Option<LinkKind>, FabricError>> + Send {
            self.log("attributed_on".into());
            std::future::ready(Ok(self.attributed.get(&peer).copied()))
        }
    }

    /// A fabric handle with nothing behind it (any actor read would fail).
    fn detached() -> Arc<FabricHandle> {
        let none = std::path::PathBuf::from("/nonexistent/111-presence-test");
        let cfg = crate::daemon::fabric::FabricConfig {
            fabric_dir: none.join("fabric"),
            identity_dir: none.join("identity"),
            state_dir: none.join("state"),
            listen: "127.0.0.1:0".parse().unwrap(),
            tcp: Default::default(),
            hub_default: false,
            enroll_timeout: Duration::from_secs(1),
            housekeeping: Duration::from_secs(3600),
            shutdown_grace: Duration::from_secs(1),
            offer_host: None,
        };
        FabricHandle::detached(cfg, crate::daemon::fabric::settings::Persisted::default())
    }

    fn messages(items: &[(i64, NodeId, MessageChange)]) -> Work {
        Work {
            messages: items.to_vec(),
            ..Work::default()
        }
    }

    fn fabric_event() -> Work {
        Work {
            fabric: true,
            ..Work::default()
        }
    }

    fn rows(p: &Presence, now: Instant) -> Vec<Value> {
        p.source(None, now)["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn evidence(row: &Value) -> Vec<(String, String)> {
        row["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["transport"].as_str().unwrap().to_string(),
                    e["text"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn the_evidence_strings_and_transports_are_the_specs() {
        assert_eq!(
            Evidence::MessageIn.text(),
            "a message from them arrived (verified)"
        );
        assert_eq!(Evidence::Delivered.text(), "they confirmed your message");
        assert_eq!(Evidence::HubAnswered.text(), "your hub answered");
        assert_eq!(Evidence::PairAnswered.text(), "paired hub answered");
        let ours: Vec<&str> = [Transport::TcpLan, Transport::Hub, Transport::Carrier]
            .iter()
            .map(|t| t.code())
            .collect();
        assert_eq!(ours, EVIDENCE_TRANSPORTS);
        let p = Presence::default();
        let s = p.source(Some(nid(9)), Instant::now());
        assert_eq!(s["kind"], "111");
        assert_eq!(s["label"], "111 nodes heard by this node");
        assert_eq!(s["device_id"], Value::Null);
        assert_eq!(s["id"], format!("111:{}", hex(nid(9))));
        assert_eq!(s["nodes"], json!([]), "listed and empty until a fact");
        assert_eq!(p.source(None, Instant::now())["id"], "111");
    }

    /// §2.2 row 1 and the LAN-vs-hub split (I1, G8): an inbound message on an attributed
    /// `tcp-lan` connection is a `tcp-lan` fact, the same contact's message through the hub is
    /// a `hub` fact, and both show on one row; the row's shape is §2.3's.
    #[tokio::test]
    async fn an_inbound_message_makes_a_row_on_the_lan_and_through_the_hub() {
        let (bob, hub) = (nid(2), nid(7));
        let mut r = Fake::default();
        r.contacts.insert(
            bob,
            contact(bob, Some("Bob's\u{202e} laptop"), 0x0001 | 0x0008),
        );
        r.messages
            .insert(10, msg(10, bob, Dir::In, MsgState::Delivered, 100));
        r.messages
            .insert(11, msg(11, bob, Dir::In, MsgState::Delivered, 140));
        r.attributed.insert(bob, LinkKind::TcpLan);
        let kept = Kept {
            home_hub: Some(hub),
            pairs: Vec::new(),
        };
        let p = Presence::default();
        let t0 = Instant::now();
        let w = messages(&[(10, bob, MessageChange::New)]);
        assert!(p.apply(&r, w, &kept, t0).await, "a new row is a change");
        let row = &rows(&p, t0)[0];
        assert_eq!(row["id"], hex(bob));
        assert_eq!(row["id"].as_str().unwrap().len(), 64);
        assert_eq!(row["num"], 0);
        assert_eq!(row["long_name"], "Bob's laptop", "alias cleaned");
        assert_eq!(row["short_name"], label(bob));
        assert_eq!(row["last_heard_hint"], Value::Null);
        assert_eq!(row["hops_away"], Value::Null);
        assert_eq!(row["heard_seq"], 100);
        assert_eq!(row["heard_ago_s"], 0);
        assert_eq!(row["roles"], json!(["mailbox", "gateway"]), "D9's map");
        assert_eq!(
            evidence(row),
            [(
                "tcp-lan".to_string(),
                "a message from them arrived (verified)".to_string()
            )]
        );
        // through the hub: no connection attributed to bob now
        r.attributed.clear();
        let t1 = t0 + Duration::from_secs(5);
        let w = messages(&[(11, bob, MessageChange::New)]);
        assert!(p.apply(&r, w, &kept, t1).await);
        let row = &rows(&p, t1 + Duration::from_secs(3))[0];
        assert_eq!(
            evidence(row),
            [
                (
                    "tcp-lan".to_string(),
                    "a message from them arrived (verified)".to_string()
                ),
                (
                    "hub".to_string(),
                    "a message from them arrived (verified)".to_string()
                ),
            ],
            "both facts show"
        );
        assert_eq!(row["heard_seq"], 140);
        assert_eq!(row["heard_ago_s"], 3, "the age grows from the latest fact");
        // the contact was read once (for the new row), the attribution once per message
        assert_eq!(r.count("contact"), 1);
        assert_eq!(r.count("attributed_on"), 2);
    }

    /// §2.2's mapping exactly (review finding 7): `tcp-lan` when a connection is attributed to
    /// the peer, whatever link holds it, else `hub`, with or without a home hub.
    #[tokio::test]
    async fn the_transport_is_the_specs_mapping_of_attributed_on() {
        let (bob, carol) = (nid(2), nid(3));
        let mut r = Fake::default();
        r.contacts.insert(bob, contact(bob, None, 0));
        r.contacts.insert(carol, contact(carol, None, 0));
        r.messages
            .insert(1, msg(1, bob, Dir::In, MsgState::Delivered, 5));
        r.messages
            .insert(2, msg(2, carol, Dir::In, MsgState::Delivered, 6));
        r.attributed.insert(carol, LinkKind::KissModem);
        let p = Presence::default();
        let now = Instant::now();
        let w = messages(&[(1, bob, MessageChange::New), (2, carol, MessageChange::New)]);
        assert!(p.apply(&r, w, &Kept::default(), now).await);
        let all = rows(&p, now);
        let row = |n: NodeId| all.iter().find(|row| row["id"] == hex(n)).unwrap();
        assert_eq!(
            evidence(row(bob))[0].0,
            "hub",
            "no home hub: still §2.2's else"
        );
        assert_eq!(row(bob)["long_name"], label(bob), "no alias: the label");
        assert_eq!(
            evidence(row(carol))[0].0,
            "tcp-lan",
            "attributed on another link: a connection is attributed"
        );
    }

    /// §2.2 rows 2 and 5: a delivered own message is "they confirmed your message"; relayed,
    /// sent and queued prove nothing; the same delivery seen again is no new fact.
    #[tokio::test]
    async fn delivered_counts_and_relayed_proves_nothing() {
        let (bob, hub) = (nid(2), nid(7));
        let mut r = Fake::default();
        r.contacts.insert(bob, contact(bob, None, 0));
        r.messages
            .insert(1, msg(1, bob, Dir::Own, MsgState::Relayed, 50));
        r.messages
            .insert(2, msg(2, bob, Dir::Own, MsgState::Sent, 51));
        r.messages
            .insert(3, msg(3, bob, Dir::Own, MsgState::Queued, 52));
        let kept = Kept {
            home_hub: Some(hub),
            pairs: Vec::new(),
        };
        let p = Presence::default();
        let now = Instant::now();
        let w = messages(&[
            (1, bob, MessageChange::State),
            (2, bob, MessageChange::State),
            (3, bob, MessageChange::New),
        ]);
        assert!(!p.apply(&r, w, &kept, now).await, "relayed is not heard");
        assert!(rows(&p, now).is_empty());
        r.messages
            .insert(1, msg(1, bob, Dir::Own, MsgState::Delivered, 50));
        let w = messages(&[(1, bob, MessageChange::State)]);
        assert!(p.apply(&r, w, &kept, now).await);
        let row = &rows(&p, now)[0];
        assert_eq!(
            evidence(row),
            [("hub".to_string(), "they confirmed your message".to_string())]
        );
        assert_eq!(row["heard_seq"], 50);
        // the same delivery again (a later frame of the same message): nothing new
        let reads = r.count("attributed_on");
        let later = now + FRAME_AGE_STEP * 2;
        let w = messages(&[(1, bob, MessageChange::State)]);
        assert!(!p.apply(&r, w, &kept, later).await);
        assert_eq!(r.count("attributed_on"), reads, "not even read");
        assert_eq!(rows(&p, later)[0]["heard_ago_s"], 120, "not refreshed");
    }

    /// Review finding 3 and its re-check: a delivered fact counts once per message, whatever
    /// order the deliveries come in: a repeat read of a counted message is no new fact; a newer
    /// message's delivery is one, and so is an older message delivered after a newer one.
    #[tokio::test]
    async fn a_delivery_counts_once_per_message_in_any_order() {
        let (bob, hub) = (nid(2), nid(7));
        let mut r = Fake::default();
        r.contacts.insert(bob, contact(bob, None, 0));
        r.messages
            .insert(1, msg(1, bob, Dir::Own, MsgState::Delivered, 50));
        r.messages
            .insert(2, msg(2, bob, Dir::Own, MsgState::Delivered, 60));
        r.messages
            .insert(3, msg(3, bob, Dir::Own, MsgState::Delivered, 70));
        let kept = Kept {
            home_hub: Some(hub),
            pairs: Vec::new(),
        };
        let p = Presence::default();
        let t0 = Instant::now();
        assert!(
            p.apply(&r, messages(&[(1, bob, MessageChange::State)]), &kept, t0)
                .await
        );
        let t1 = t0 + FRAME_AGE_STEP * 2;
        assert!(
            p.apply(&r, messages(&[(2, bob, MessageChange::State)]), &kept, t1)
                .await
        );
        // an event for message 1 again, read in a later batch (its earlier `sent` event
        // handled after it was delivered): a repeat, no new fact
        let t2 = t1 + FRAME_AGE_STEP * 2;
        assert!(
            !p.apply(&r, messages(&[(1, bob, MessageChange::State)]), &kept, t2)
                .await,
            "a repeat is no new fact"
        );
        assert_eq!(rows(&p, t2)[0]["heard_ago_s"], 120, "not refreshed");
        assert_eq!(rows(&p, t2)[0]["heard_seq"], 60);
        assert!(
            p.apply(&r, messages(&[(3, bob, MessageChange::State)]), &kept, t2)
                .await
        );
        assert_eq!(rows(&p, t2)[0]["heard_ago_s"], 0);
        assert_eq!(rows(&p, t2)[0]["heard_seq"], 70);
        // review of the fix pass: a message delivered out of send order (sent before 70, held
        // at the hub, fetched later) is that message's one confirmation: a fresh fact
        r.messages
            .insert(4, msg(4, bob, Dir::Own, MsgState::Delivered, 55));
        let t3 = t2 + FRAME_AGE_STEP * 10;
        assert!(
            p.apply(&r, messages(&[(4, bob, MessageChange::State)]), &kept, t3)
                .await,
            "an out-of-order delivery is a fact"
        );
        assert_eq!(rows(&p, t3)[0]["heard_ago_s"], 0, "refreshed");
        assert_eq!(rows(&p, t3)[0]["heard_seq"], 70, "heard_seq stays monotone");
        assert!(
            !p.apply(&r, messages(&[(4, bob, MessageChange::State)]), &kept, t3)
                .await,
            "and counts once"
        );
    }

    /// Review finding 1: a fact refreshed more often than [`FRAME_AGE_STEP`] (the hub
    /// attributed on every `Fabric` event, 20–50 s apart) still moves the frame once its age
    /// moved a step past the one the last frame carried, and not more often.
    #[tokio::test]
    async fn a_frequently_refreshed_age_still_moves_the_frame_once_a_step() {
        let hub = nid(7);
        let mut r = Fake::default();
        r.contacts.insert(hub, contact(hub, None, 0x0003));
        r.attributed.insert(hub, LinkKind::TcpLan);
        let kept = Kept {
            home_hub: Some(hub),
            pairs: Vec::new(),
        };
        let p = Presence::default();
        let t0 = Instant::now();
        assert!(p.apply(&r, fabric_event(), &kept, t0).await, "a new row");
        let mut frames = Vec::new();
        for i in 1..=8u64 {
            let t = t0 + Duration::from_secs(30 * i);
            if p.apply(&r, fabric_event(), &kept, t).await {
                frames.push(30 * i);
            }
        }
        assert_eq!(frames, [60, 120, 180, 240], "once per step of real change");
    }

    /// The tap's actor reads per message event (review finding 8, recorded on the board): a
    /// message request none, an event that proves nothing one, a fact on a row two, a new row
    /// three; a `Fabric` batch one for the home hub.
    #[tokio::test]
    async fn the_actor_reads_per_event_are_counted() {
        let (bob, hub) = (nid(2), nid(7));
        let mut r = Fake::default();
        r.contacts.insert(bob, contact(bob, None, 0));
        r.contacts.insert(hub, contact(hub, None, 0x0003));
        r.messages
            .insert(1, msg(1, bob, Dir::In, MsgState::Delivered, 1));
        r.messages
            .insert(2, msg(2, bob, Dir::In, MsgState::Delivered, 2));
        r.messages
            .insert(3, msg(3, bob, Dir::Own, MsgState::Relayed, 3));
        r.messages
            .insert(4, msg(4, bob, Dir::In, MsgState::Delivered, 4));
        let kept = Kept {
            home_hub: Some(hub),
            pairs: Vec::new(),
        };
        let p = Presence::default();
        let now = Instant::now();
        let reads = |r: &Fake| r.calls.lock().unwrap().len();
        p.apply(&r, messages(&[(1, bob, MessageChange::New)]), &kept, now)
            .await;
        assert_eq!(reads(&r), 3, "a new row: message, attributed_on, contact");
        p.apply(&r, messages(&[(2, bob, MessageChange::New)]), &kept, now)
            .await;
        assert_eq!(reads(&r), 5, "a fact on a row: two");
        p.apply(&r, messages(&[(3, bob, MessageChange::State)]), &kept, now)
            .await;
        assert_eq!(reads(&r), 6, "relayed: the message only");
        p.apply(
            &r,
            messages(&[(4, bob, MessageChange::Request)]),
            &kept,
            now,
        )
        .await;
        assert_eq!(reads(&r), 6, "a message request: nothing read");
        p.apply(&r, fabric_event(), &kept, now).await;
        assert_eq!(
            reads(&r),
            7,
            "the hub: attributed_on (not attributed: no row)"
        );
    }

    /// A row only for a contact (§2.3): a message request, a stranger, a request contact and a
    /// message whose store row names another peer create nothing; an inbound `State` change is
    /// not a new message.
    #[tokio::test]
    async fn only_a_contact_gets_a_row() {
        let (bob, eve, req) = (nid(2), nid(5), nid(6));
        let mut r = Fake::default();
        r.contacts.insert(bob, contact(bob, None, 0));
        let mut pending = contact(req, None, 0);
        pending.request = true;
        r.contacts.insert(req, pending);
        r.messages
            .insert(1, msg(1, bob, Dir::In, MsgState::Delivered, 1));
        r.messages
            .insert(2, msg(2, eve, Dir::In, MsgState::Delivered, 2));
        r.messages
            .insert(3, msg(3, req, Dir::In, MsgState::Delivered, 3));
        r.attributed.insert(bob, LinkKind::TcpLan);
        r.attributed.insert(eve, LinkKind::TcpLan);
        r.attributed.insert(req, LinkKind::TcpLan);
        let p = Presence::default();
        let now = Instant::now();
        let w = messages(&[
            (1, bob, MessageChange::Request),
            (2, eve, MessageChange::New),
            (3, req, MessageChange::New),
        ]);
        assert!(!p.apply(&r, w, &Kept::default(), now).await);
        assert!(rows(&p, now).is_empty());
        // the event's peer must be the row's (a stale id read elsewhere proves nothing)
        let w = messages(&[(1, eve, MessageChange::New)]);
        assert!(!p.apply(&r, w, &Kept::default(), now).await);
        let w = messages(&[(1, bob, MessageChange::State)]);
        assert!(!p.apply(&r, w, &Kept::default(), now).await);
        assert!(rows(&p, now).is_empty());
        // a message id the store no longer has
        let w = messages(&[(99, bob, MessageChange::New)]);
        assert!(!p.apply(&r, w, &Kept::default(), now).await);
        assert!(rows(&p, now).is_empty());
    }

    /// §2.2 row 6: a card, an announce, a contact update without a row, a request: nothing; a
    /// renamed contact renames its row; a forgotten contact's row goes.
    #[tokio::test]
    async fn contact_events_refresh_nothing_but_names_and_forget_rows() {
        let (bob, carol) = (nid(2), nid(3));
        let mut r = Fake::default();
        r.contacts.insert(bob, contact(bob, Some("Bob"), 0));
        r.contacts.insert(carol, contact(carol, Some("Carol"), 0));
        r.messages
            .insert(1, msg(1, bob, Dir::In, MsgState::Delivered, 1));
        let p = Presence::default();
        let now = Instant::now();
        let card = Work {
            contacts: vec![
                (carol, ContactChange::Added),
                (carol, ContactChange::Updated),
                (bob, ContactChange::Request),
            ],
            ..Work::default()
        };
        assert!(!p.apply(&r, card, &Kept::default(), now).await);
        assert!(rows(&p, now).is_empty(), "a card or an announce is no fact");
        assert_eq!(r.count("contact"), 0, "no row, no read");
        p.apply(
            &r,
            messages(&[(1, bob, MessageChange::New)]),
            &Kept::default(),
            now,
        )
        .await;
        r.contacts.insert(bob, contact(bob, Some("Robert"), 0x0020));
        let upd = Work {
            contacts: vec![(bob, ContactChange::Updated)],
            ..Work::default()
        };
        assert!(p.apply(&r, upd, &Kept::default(), now).await);
        let row = &rows(&p, now)[0];
        assert_eq!(row["long_name"], "Robert");
        assert_eq!(row["roles"], json!(["origin"]));
        assert_eq!(row["heard_seq"], 1, "a rename is no fact");
        let gone = Work {
            contacts: vec![(bob, ContactChange::Removed)],
            ..Work::default()
        };
        assert!(p.apply(&r, gone, &Kept::default(), now).await);
        assert!(rows(&p, now).is_empty());
    }

    /// §2.2 row 3: the home hub attributed ⇒ its row, `tcp-lan`, "your hub answered"; a
    /// `heard_seq` of null (no store row records the answer); not attributed ⇒ nothing.
    #[tokio::test]
    async fn the_home_hub_answered() {
        let hub = nid(7);
        let mut r = Fake::default();
        r.contacts
            .insert(hub, contact(hub, Some("Home hub"), 0x0003));
        let kept = Kept {
            home_hub: Some(hub),
            pairs: Vec::new(),
        };
        let p = Presence::default();
        let now = Instant::now();
        assert!(!p.apply(&r, fabric_event(), &kept, now).await);
        assert!(rows(&p, now).is_empty(), "not attributed: no fact");
        r.attributed.insert(hub, LinkKind::TcpLan);
        assert!(p.apply(&r, fabric_event(), &kept, now).await);
        let row = &rows(&p, now)[0];
        assert_eq!(row["id"], hex(hub));
        assert_eq!(row["heard_seq"], Value::Null);
        assert_eq!(row["roles"], json!(["mailbox", "always_on_hub"]), "{row}");
        assert_eq!(
            evidence(row),
            [("tcp-lan".to_string(), "your hub answered".to_string())]
        );
        // still attributed a little later: refreshed, but no frame for the same age bucket
        let soon = now + Duration::from_secs(10);
        assert!(!p.apply(&r, fabric_event(), &kept, soon).await);
        assert_eq!(rows(&p, soon)[0]["heard_ago_s"], 0, "the age was reset");
        // a minute on: a frame
        let later = soon + FRAME_AGE_STEP;
        assert!(p.apply(&r, fabric_event(), &kept, later).await);
        // without a Fabric event the hub is not read at all
        let reads = r.count("attributed_on");
        assert!(!p.apply(&r, Work::default(), &kept, later).await);
        assert_eq!(r.count("attributed_on"), reads);
    }

    /// §2.2 row 4: a whole `tcp` pair with its connection attributed ⇒ `tcp-lan`; a whole
    /// `carrier` pair's first verified frame ⇒ `carrier`, once per rising edge (the view is
    /// sticky); a half or broken or unreachable pair ⇒ nothing. The pairs are the kept views:
    /// no actor read beyond the new row's contact.
    #[tokio::test]
    async fn a_paired_hub_answered_on_its_via() {
        let (tcp, car, half, broken) = (nid(30), nid(31), nid(32), nid(33));
        let mut r = Fake::default();
        for n in [tcp, car, half, broken] {
            r.contacts.insert(n, contact(n, None, 0x0002));
        }
        let mut b = pair(broken, PairVia::Tcp, true, true);
        b.broken = true;
        let mut kept = Kept {
            home_hub: None,
            pairs: vec![
                pair(tcp, PairVia::Tcp, true, true),
                pair(car, PairVia::Carrier, true, true),
                pair(half, PairVia::Tcp, false, true),
                b,
            ],
        };
        let p = Presence::default();
        let now = Instant::now();
        assert!(p.apply(&r, fabric_event(), &kept, now).await);
        let all = rows(&p, now);
        assert_eq!(all.len(), 2, "{all:?}");
        let by_id = |n: NodeId| {
            all.iter()
                .find(|row| row["id"] == hex(n))
                .cloned()
                .unwrap_or(Value::Null)
        };
        assert_eq!(
            evidence(&by_id(tcp)),
            [("tcp-lan".to_string(), "paired hub answered".to_string())]
        );
        assert_eq!(
            evidence(&by_id(car)),
            [("carrier".to_string(), "paired hub answered".to_string())]
        );
        assert_eq!(r.count("attributed_on"), 0, "the kept views, no actor read");
        // the carrier view stays true for the session: no new fact on the next batch
        let later = now + FRAME_AGE_STEP * 3;
        p.apply(&r, fabric_event(), &kept, later).await;
        let ages: HashMap<String, u64> = rows(&p, later)
            .iter()
            .map(|row| {
                (
                    row["id"].as_str().unwrap().to_string(),
                    row["heard_ago_s"].as_u64().unwrap(),
                )
            })
            .collect();
        assert_eq!(ages[&hex(car)], 180, "sticky: only the rising edge counts");
        assert_eq!(ages[&hex(tcp)], 0, "attributed: refreshed");
        // the carrier pair goes quiet (unpaired or re-paired), then answers again
        kept.pairs[1].reachable = false;
        p.apply(&r, fabric_event(), &kept, later).await;
        kept.pairs[1].reachable = true;
        let again = later + Duration::from_secs(1);
        assert!(p.apply(&r, fabric_event(), &kept, again).await);
        let row = rows(&p, again)
            .into_iter()
            .find(|row| row["id"] == hex(car))
            .unwrap();
        assert_eq!(row["heard_ago_s"], 0, "a new rising edge");
    }

    /// Review finding 2: a carrier pair's rising edge is not lost to a failed contact read; it
    /// is consumed once the row is written, and only then.
    #[tokio::test]
    async fn a_carrier_edge_survives_a_failed_contact_read() {
        let car = nid(31);
        let mut r = Fake::default();
        r.contacts.insert(car, contact(car, None, 0x0002));
        let kept = Kept {
            home_hub: None,
            pairs: vec![pair(car, PairVia::Carrier, true, true)],
        };
        let p = Presence::default();
        let t0 = Instant::now();
        r.contact_fails
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(!p.apply(&r, fabric_event(), &kept, t0).await);
        assert!(rows(&p, t0).is_empty(), "the read failed: no row yet");
        r.contact_fails
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let t1 = t0 + Duration::from_secs(20);
        assert!(
            p.apply(&r, fabric_event(), &kept, t1).await,
            "the edge stayed"
        );
        let row = &rows(&p, t1)[0];
        assert_eq!(
            evidence(row),
            [("carrier".to_string(), "paired hub answered".to_string())]
        );
        // consumed now: the sticky view is no new fact
        let t2 = t1 + FRAME_AGE_STEP * 2;
        let contacts = r.count("contact");
        assert!(!p.apply(&r, fabric_event(), &kept, t2).await);
        assert_eq!(r.count("contact"), contacts, "no read once consumed");
        // a far hub that is not a contact settles the edge too (no read on every batch)
        let stranger = nid(40);
        let kept = Kept {
            home_hub: None,
            pairs: vec![pair(stranger, PairVia::Carrier, true, true)],
        };
        let before = r.count("contact");
        assert!(!p.apply(&r, fabric_event(), &kept, t2).await);
        assert!(!p.apply(&r, fabric_event(), &kept, t2).await);
        assert_eq!(r.count("contact"), before + 1, "read once");
    }

    /// The cap (§2.3): at most 512 rows, the oldest heard forgotten first; rows are listed most
    /// recently heard first.
    #[test]
    fn the_book_is_bounded_and_forgets_the_oldest_heard() {
        let mut b = Book::default();
        let t0 = Instant::now();
        let names = |n: NodeId| Names {
            long_name: label(n),
            short_name: label(n),
            roles: Vec::new(),
        };
        let total = MAX_BRIDGE_NODES + 3;
        for i in 0..total {
            let mut id = [0u8; 32];
            id[..8].copy_from_slice(&(i as u64).to_le_bytes());
            let n = NodeId::from(id);
            let at = t0 + Duration::from_millis(i as u64);
            assert!(b.record(
                n,
                Some(names(n)),
                (Transport::TcpLan, Evidence::MessageIn),
                Some(i as u64),
                at
            ));
        }
        assert_eq!(b.rows.len(), MAX_BRIDGE_NODES);
        let first_kept = {
            let mut id = [0u8; 32];
            id[..8].copy_from_slice(&3u64.to_le_bytes());
            NodeId::from(id)
        };
        assert_eq!(b.rows.front().map(|r| r.peer), Some(first_kept));
        // a fact moves its row to the newest end
        assert!(b.record(
            first_kept,
            None,
            (Transport::Hub, Evidence::MessageIn),
            None,
            t0 + Duration::from_secs(1),
        ));
        let shown = b.nodes(t0 + Duration::from_secs(1));
        assert_eq!(shown[0]["id"], hex(first_kept), "most recent first");
        // a fact for a peer without a row and without names creates nothing
        assert!(!b.record(
            nid(1),
            None,
            (Transport::TcpLan, Evidence::MessageIn),
            None,
            t0
        ));
        assert_eq!(b.rows.len(), MAX_BRIDGE_NODES);
    }

    /// The tap notes work without reading anything: message ids once (the first change kept),
    /// the last change per contact, a bounded list; a handle other than the bound one takes
    /// nothing.
    #[test]
    fn the_tap_notes_bounded_work_for_the_bound_handle_only() {
        let p = Presence::default();
        let (bob, carol) = (nid(2), nid(3));
        p.note(&FabricEvent::Message {
            id: 1,
            peer: bob,
            change: MessageChange::New,
        });
        p.note(&FabricEvent::Message {
            id: 1,
            peer: bob,
            change: MessageChange::State,
        });
        p.note(&FabricEvent::Contact {
            node_id: carol,
            change: ContactChange::Added,
        });
        p.note(&FabricEvent::Contact {
            node_id: carol,
            change: ContactChange::Removed,
        });
        p.note(&FabricEvent::Forward {
            target: carol,
            pending: 3,
        });
        let fh = detached();
        let other = detached();
        assert!(p.take_work(&fh).is_none(), "not bound yet");
        p.bind(&fh);
        assert!(p.take_work(&fh).is_none(), "binding starts a new session");
        p.note(&FabricEvent::Message {
            id: 1,
            peer: bob,
            change: MessageChange::New,
        });
        p.note(&FabricEvent::Message {
            id: 1,
            peer: bob,
            change: MessageChange::State,
        });
        p.note(&FabricEvent::Contact {
            node_id: carol,
            change: ContactChange::Added,
        });
        p.note(&FabricEvent::Contact {
            node_id: carol,
            change: ContactChange::Removed,
        });
        p.note(&FabricEvent::Fabric);
        assert!(
            p.take_work(&other).is_none(),
            "another handle takes nothing"
        );
        let w = p.take_work(&fh).expect("work");
        assert_eq!(w.messages, [(1, bob, MessageChange::New)]);
        assert_eq!(w.contacts, [(carol, ContactChange::Removed)]);
        assert!(w.fabric);
        assert!(p.take_work(&fh).is_none(), "taken once");
        for i in 0..(MAX_WORK as i64 + 10) {
            p.note(&FabricEvent::Message {
                id: i,
                peer: bob,
                change: MessageChange::New,
            });
        }
        assert_eq!(p.lock().dropped, 10);
        assert_eq!(p.take_work(&fh).expect("work").messages.len(), MAX_WORK);
    }

    // ------------------------------------------------------------ the heartbeat validator

    /// §2.4 item 2 and G8: in v1 no transport takes a beat, so every Layer 7 transport answers
    /// `409 no_such_transport` and `rns` `409 settings_locked` naming the bridge's route.
    #[test]
    fn every_v1_heartbeat_write_is_refused_by_transport() {
        let v1 = BeatContext {
            offered: &BEAT_TRANSPORTS_V1,
            ..BeatContext::default()
        };
        for t in crate::daemon::relay::settings::HEARTBEAT_TRANSPORTS {
            for m in [0, 9, 60] {
                assert_eq!(
                    check_heartbeat(t, m, &v1),
                    Err(HeartbeatRefusal::NoSuchTransport),
                    "{t} {m}"
                );
            }
        }
        assert_eq!(
            check_heartbeat("ble", 15, &v1),
            Err(HeartbeatRefusal::NoSuchTransport)
        );
        let p = check_heartbeat("tcp-lan", 15, &v1).unwrap_err().problem();
        assert_eq!((p.status, p.code), (409, "no_such_transport"));
        let rns = check_heartbeat("rns", 60, &v1).unwrap_err();
        assert_eq!(rns, HeartbeatRefusal::SettingsLocked);
        let p = rns.problem();
        assert_eq!((p.status, p.code), (409, "settings_locked"));
        assert!(p
            .detail
            .unwrap_or_default()
            .contains("PATCH /api/v1/bridge/settings"));
        // rns stays Layer 6's even where a test marks it offered
        let all = ["rns", "kiss-modem"];
        let marked = BeatContext {
            offered: &all,
            ..BeatContext::default()
        };
        assert_eq!(
            check_heartbeat("rns", 60, &marked),
            Err(HeartbeatRefusal::SettingsLocked)
        );
    }

    /// §2.4 item 3, G8 (I12): with a transport the test marks offered, the bans: a licensed
    /// board or a duty-cycle-limited band ⇒ `409 licensed_board`; a phone under nine minutes ⇒
    /// `400 bad_request`; `0` (events only) sends nothing, so no ban applies.
    #[test]
    fn the_bans_apply_after_the_transport_check() {
        let offered = ["kiss-modem"];
        let base = BeatContext {
            offered: &offered,
            ..BeatContext::default()
        };
        assert_eq!(check_heartbeat("kiss-modem", 15, &base), Ok(()));
        assert_eq!(
            check_heartbeat("kiss-modem", 1, &base),
            Ok(()),
            "not a phone"
        );
        let licensed = BeatContext {
            licensed_board: true,
            ..base
        };
        let e = check_heartbeat("kiss-modem", 180, &licensed).unwrap_err();
        assert_eq!(e, HeartbeatRefusal::LicensedBoard);
        let p = e.problem();
        assert_eq!((p.status, p.code), (409, "licensed_board"));
        let duty = BeatContext {
            duty_cycle_limited: true,
            ..base
        };
        assert_eq!(
            check_heartbeat("kiss-modem", 180, &duty),
            Err(HeartbeatRefusal::LicensedBoard)
        );
        let phone = BeatContext {
            phone: true,
            ..base
        };
        let e = check_heartbeat("kiss-modem", 8, &phone).unwrap_err();
        assert_eq!(e, HeartbeatRefusal::PhoneFloor);
        let p = e.problem();
        assert_eq!((p.status, p.code), (400, "bad_request"));
        assert_eq!(check_heartbeat("kiss-modem", 9, &phone), Ok(()));
        // events only passes every ban
        for cx in [licensed, duty, phone] {
            assert_eq!(check_heartbeat("kiss-modem", 0, &cx), Ok(()));
        }
        // the transport check comes first: an unoffered transport on a licensed board
        assert_eq!(
            check_heartbeat("meshtastic-client", 60, &licensed),
            Err(HeartbeatRefusal::NoSuchTransport)
        );
        let max = crate::daemon::relay::settings::MAX_HEARTBEAT_MIN;
        assert_eq!(check_heartbeat("kiss-modem", max, &base), Ok(()));
        let e = check_heartbeat("kiss-modem", max + 1, &base).unwrap_err();
        assert_eq!(e, HeartbeatRefusal::OutOfRange);
        assert_eq!(e.problem().status, 400);
    }
}
