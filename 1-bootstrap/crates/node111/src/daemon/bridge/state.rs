//! The delivery state machine (Layer 6 `6-transport/README.md` §4 "Monotone states", "At
//! most once per user act", "Queue order"; §5 "Reasons"; §6 (4)): own rows from the queue
//! to the adapter and the adapter's events back onto the rows.
//!
//! - **Order** ([`deliver_pass`], once per tick per enabled network, at most
//!   `sends_per_tick` handoffs): `foreign_queued(network)` — user-composed rows (a send
//!   group) ahead of relay copies, then `local_seq` — merged with the rows due for a retry
//!   in the same order. Ordering, not a cap: the rest wait a tick.
//! - **Gates before a handoff**, read live per row: the consent gates (a licensed account
//!   needs its confirmation, `ham_unconfirmed`; the channel's send switch, `uplink_off`,
//!   relay copies included), then the adapter's state (link down or not live ⇒
//!   `adapter_down`; region unset ⇒ `region_unset`: the board transmits nothing). A held
//!   row stays `queued` with the reason and is looked at again next tick.
//! - **At most once.** `foreign_handoff_begin` writes `meta.handoff = pending` (FULL)
//!   before `Adapter::send`; `Accepted` records the native id and clears the mark
//!   (`foreign_accepted`). A row that carries a native id or a pending mark this run does
//!   not know is **never** handed again: after a crash it shows "may have been sent;
//!   resend?" ([`Bridge::uncertain`]) and [`Bridge::resend`] mints a new row (a new send
//!   group, so a new packet id). A pending row never blocks later rows.
//! - **Events → states** (the store is monotone, `ForeignState::may_become`): `Evidence`
//!   sets its state and string; `Failed` before air with `Retry::Never` is `failed` with the
//!   reason; a NAK after air (`no_route`, `timeout`, `max_retransmit`, `no_response`, or
//!   any failure after `Accepted`) keeps `sent`/`relayed` with "transmitted, not
//!   confirmed"; `delivered` never regresses.
//! - **`Retry::Later` before air** (queue full, duty cycle, rate limited, key unknown, peer
//!   unannounced; no `Accepted` on the row): nothing was transmitted, so the core
//!   **withdraws the pending mark** (`Fabric::foreign_handoff_abort`, FULL), forgets the
//!   handle and the text and keeps only `(row id → attempts, due tick)`. The row is plain
//!   `queued` with the reason again: cancellable (§9 "while `queued` with nothing sent"),
//!   sheddable by the store's prune (2b), never "may have been sent", and after a restart
//!   or a disable it is handed again on the next tick (the crash rule covers only a mark
//!   that is still pending). [`deliver_pass`] re-hands it from `foreign_queued` once due —
//!   attempt `a` waits `min(2^(a-1), backoff_cap_ticks)` ticks — and at once on an adapter
//!   event (link up, a heard announce, a conversation upsert). A row that left the queue
//!   meanwhile (cancelled, shed, its edge paused or deleted) is not waited for.
//! - **`Retry::Later` after air** (a post-air NAK reason with `Later`; §4 "Reasons": a DM's
//!   post-air NAK is `Later`; §5 "`Later` on the next NodeInfo from that node"): the row
//!   keeps its state and "transmitted, not confirmed"; the leg waits — no tick back-off, no
//!   timer — for the adapter's [`super::Inbound::Conversation`] upsert of exactly the leg's
//!   conversation (the Meshtastic client emits `dm:!<node>` on that node's next NodeInfo),
//!   then the same text is handed once more (the adapter mints a new packet id) and that
//!   send's `Accepted` records the new native id. Each upsert re-sends at most once. A
//!   failure after `Accepted` with any other reason, or with `Retry::Never`, is kept and
//!   never re-sent. This is not the crash rule: a row still `pending` across a restart is
//!   never re-sent automatically. At most [`MAX_PEER_WAIT`] rows wait so (the oldest
//!   stops waiting and keeps its state).
//! - **Recipient lists** (§6): a row in a list's conversation is handed as one DM per
//!   member (legs); each leg retries on its own, an accepted leg is never sent again, and
//!   the row shows the lowest state among its legs. The pending mark is withdrawn only when
//!   every leg was refused before air; while one leg was accepted the waiting legs retry
//!   from memory (the row has a native id, so it is not handed again from the queue).

use super::core::{conv_ref, is_list_conv, Bridge, BridgeError};
use super::{
    ingest, AdapterState, ConvKind, ConvRef, ForeignMessageState as S, Reason, Retry, SendEvent,
    SendOpts, SizeMode,
};
use crate::daemon::fabric::json::{self as fjson, GroupTarget};
use mesh111::fabric::{FabricError, ForeignGroupPart, ForeignMeta};
use mesh111::store::{
    EdgeCounter, ForeignConversationRow, ForeignDir, ForeignMessageRow, ForeignState,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc::{self, error::TryRecvError};

/// The evidence a post-air NAK leaves (§4, §5).
pub const TRANSMITTED_NOT_CONFIRMED: &str = "transmitted, not confirmed";
/// Rows whose leg waits for its peer's next conversation upsert (§5), at most; past it the
/// oldest (`local_seq`) stops waiting and keeps its state and evidence.
pub const MAX_PEER_WAIT: usize = 1024;
/// Extra queued rows looked at per pass beyond the budget (rows accepted but still
/// `queued` are skipped, never re-sent).
const QUEUE_SLACK: usize = 256;

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Is `r` a NAK after the board transmitted (§5)?
pub fn post_air(r: Reason) -> bool {
    matches!(
        r,
        Reason::NoRoute | Reason::Timeout | Reason::MaxRetransmit | Reason::NoResponse
    )
}

/// The `reason_code` spelling.
pub fn reason_code(r: Reason) -> String {
    match serde_json::to_value(r) {
        Ok(Value::String(s)) => s,
        _ => "adapter_down".into(),
    }
}

fn store_state(s: S) -> ForeignState {
    match s {
        S::Queued => ForeignState::Queued,
        S::Sent => ForeignState::Sent,
        S::Relayed => ForeignState::Relayed,
        S::Delivered => ForeignState::Delivered,
        S::Failed => ForeignState::Failed,
    }
}

fn contract_state(s: ForeignState) -> S {
    match s {
        ForeignState::Queued => S::Queued,
        ForeignState::Sent => S::Sent,
        ForeignState::Relayed => S::Relayed,
        ForeignState::Delivered => S::Delivered,
        ForeignState::Failed => S::Failed,
    }
}

enum LegIo {
    Open(mpsc::Receiver<SendEvent>),
    Closed,
    /// `Retry::Later` before acceptance: send again at tick `due`.
    Waiting {
        attempts: u32,
        due: u64,
    },
}

/// One handoff of a row (one per list member; one for any other row).
struct Leg {
    to: ConvRef,
    io: LegIo,
    attempts: u32,
    native: Option<String>,
    state: S,
    evidence: Option<String>,
    reason: Option<String>,
    /// Refused before air with `Retry::Never`.
    failed: bool,
    /// On air, and the adapter asked for a later try (a post-air NAK with `Retry::Later`,
    /// §5): `Some(false)` waits for the next upsert of `to`, `Some(true)` once it came (the
    /// next pass re-sends). The handle stays open meanwhile (a real ack may still come).
    peer: Option<bool>,
    /// Re-sent after air: the next `Accepted` records its new native id.
    rerecord: bool,
    /// Re-sent after air at least once: what the first send reached stays (a refusal of
    /// a re-send never makes the row `failed` or `queued` again).
    resent: bool,
}

impl Leg {
    fn new(to: ConvRef, rx: mpsc::Receiver<SendEvent>) -> Self {
        Leg {
            to,
            io: LegIo::Open(rx),
            attempts: 1,
            native: None,
            state: S::Queued,
            evidence: None,
            reason: None,
            failed: false,
            peer: None,
            rerecord: false,
            resent: false,
        }
    }

    /// What this leg contributes to the row: `(state, evidence, reason)`.
    fn effective(&self) -> (S, Option<String>, Option<String>) {
        if self.failed && self.state.rank() < S::Relayed.rank() {
            return (S::Failed, None, self.reason.clone());
        }
        if matches!(self.io, LegIo::Waiting { .. }) && !self.resent {
            return (S::Queued, None, self.reason.clone());
        }
        (self.state, self.evidence.clone(), self.reason.clone())
    }

    fn apply(&mut self, ev: SendEvent, tick: u64, cap: u64) {
        match ev {
            SendEvent::Accepted { native_id } => self.native = Some(native_id),
            SendEvent::Evidence { state, evidence } => {
                let delivered = state == S::Delivered;
                if (self.failed && !delivered) || state.rank() < self.state.rank() {
                    return;
                }
                if delivered {
                    self.failed = false;
                    // confirmed: nothing to try again
                    self.peer = None;
                }
                self.state = state;
                self.evidence = Some(evidence);
                self.reason = None;
            }
            SendEvent::Failed { reason, retry } => {
                let code = reason_code(reason);
                if post_air(reason)
                    || self.native.is_some()
                    || self.state.rank() >= S::Relayed.rank()
                {
                    // on air without proof: keep the state
                    if self.state.rank() < S::Sent.rank() {
                        self.state = S::Sent;
                    }
                    if self.state != S::Delivered {
                        self.evidence = Some(TRANSMITTED_NOT_CONFIRMED.to_string());
                        self.reason = Some(code);
                        // §4 Reasons, §5: a post-air NAK with `Later` waits for that
                        // conversation's next upsert; any other failure after air is kept
                        // and never re-sent
                        if retry == Retry::Later && post_air(reason) {
                            self.peer = Some(false);
                        }
                    }
                } else if retry == Retry::Later {
                    let wait = 1u64
                        .checked_shl(self.attempts.saturating_sub(1))
                        .unwrap_or(u64::MAX)
                        .min(cap);
                    self.io = LegIo::Waiting {
                        attempts: self.attempts,
                        due: tick.saturating_add(wait),
                    };
                    self.reason = Some(code);
                } else if self.resent {
                    // a re-send refused for good: the first send stays what it was
                    self.reason = Some(code);
                } else {
                    self.failed = true;
                    self.reason = Some(code);
                }
            }
        }
    }

    /// Due for a send now: its pre-air back-off ran out, or its peer was heard again.
    fn due(&self, tick: u64) -> bool {
        matches!(self.io, LegIo::Waiting { due, .. } if due <= tick) || self.peer == Some(true)
    }
}

/// A row this run handed to an adapter.
struct Track {
    network: String,
    conv: i64,
    edge: Option<i64>,
    part_k: u32,
    local_seq: u64,
    user: bool,
    text: String,
    legs: Vec<Leg>,
    recorded: bool,
    applied: Option<(S, Option<String>, Option<String>)>,
}

impl Track {
    /// The row's state: the lowest among its legs (§9 "the lowest state among its parts").
    fn aggregate(&self) -> (S, Option<String>, Option<String>) {
        self.legs
            .iter()
            .map(Leg::effective)
            .min_by_key(|(s, _, _)| s.rank())
            .unwrap_or((S::Queued, None, None))
    }

    fn done(&self) -> bool {
        self.legs
            .iter()
            .all(|l| matches!(l.io, LegIo::Closed) && l.peer.is_none())
    }

    fn retry_due(&self, tick: u64) -> bool {
        self.legs.iter().any(|l| l.due(tick))
    }

    fn peer_waiting(&self) -> bool {
        self.legs.iter().any(|l| l.peer.is_some())
    }

    /// Nothing of the row went on air and every leg waits for a pre-air retry: the pending
    /// mark can be withdrawn (F1).
    fn withdrawable(&self) -> Option<(u32, u64)> {
        if self.recorded || self.legs.is_empty() {
            return None;
        }
        let mut attempts = 0u32;
        let mut due = u64::MAX;
        for l in &self.legs {
            match l.io {
                LegIo::Waiting {
                    attempts: a,
                    due: d,
                } if l.native.is_none() => {
                    attempts = attempts.max(a);
                    due = due.min(d);
                }
                _ => return None,
            }
        }
        Some((attempts, due))
    }
}

/// A row refused before air and withdrawn from the store (`queued`, no pending mark): the
/// next handoff waits for tick `due` (or an adapter event).
struct LaterWait {
    network: String,
    attempts: u32,
    due: u64,
}

/// What the state machine holds in memory (rows handed this run; nothing durable).
#[derive(Default)]
pub(super) struct Delivery {
    tracks: HashMap<i64, Track>,
    /// `Retry::Later` before air (module doc): row id → its wait. No handle, no text.
    later: HashMap<i64, LaterWait>,
}

impl Delivery {
    /// Rows tracked (in flight, waiting for a retry, or waiting for their peer).
    pub(super) fn len(&self) -> usize {
        self.tracks.len() + self.later.len()
    }
}

enum Polled {
    Ev(SendEvent),
    Closed,
}

/// Forget the in-flight handles of `network` (its adapter was disabled): their rows keep
/// the pending mark ("may have been sent"); legs waiting for a retry or for their peer, and
/// withdrawn rows' waits, stay for the next enable.
pub(super) fn forget_in_flight(b: &Bridge, network: &str) {
    let mut d = lock(&b.delivery);
    d.tracks.retain(|_, t| {
        if t.network != network {
            return true;
        }
        for l in &mut t.legs {
            if matches!(l.io, LegIo::Open(_)) {
                l.io = LegIo::Closed;
            }
        }
        !t.done()
    });
}

/// Forget row `id` entirely (it was cancelled: nothing of it is sent or waited for).
pub(super) fn forget_row(b: &Bridge, id: i64) {
    let mut d = lock(&b.delivery);
    d.tracks.remove(&id);
    d.later.remove(&id);
}

/// The adapter upserted conversation `conv` (cleaned coordinates): a leg waiting for that
/// peer after a post-air NAK is re-sent on the next pass (§5). Returns the legs woken.
pub(super) fn wake_peer(b: &Bridge, conv: &ConvRef) -> usize {
    let mut d = lock(&b.delivery);
    let mut n = 0;
    for t in d.tracks.values_mut().filter(|t| t.network == conv.network) {
        for l in &mut t.legs {
            if l.peer == Some(false) && l.to == *conv {
                l.peer = Some(true);
                n += 1;
            }
        }
    }
    n
}

/// Drain every open handle's events (non-blocking) and write the rows' states.
pub(super) async fn drain_events(b: &Bridge) {
    let ids: Vec<i64> = lock(&b.delivery).tracks.keys().copied().collect();
    for id in ids {
        drain_row(b, id).await;
    }
    cap_peer_wait(b);
}

/// Keep at most [`MAX_PEER_WAIT`] rows waiting for their peer (oldest stop waiting).
fn cap_peer_wait(b: &Bridge) {
    let mut d = lock(&b.delivery);
    let n = d.tracks.values().filter(|t| t.peer_waiting()).count();
    if n <= MAX_PEER_WAIT {
        return;
    }
    let mut waiting: Vec<(u64, i64)> = d
        .tracks
        .iter()
        .filter(|(_, t)| t.peer_waiting())
        .map(|(id, t)| (t.local_seq, *id))
        .collect();
    waiting.sort_unstable();
    for (_, id) in waiting.into_iter().take(n - MAX_PEER_WAIT) {
        let done = match d.tracks.get_mut(&id) {
            Some(t) => {
                for l in &mut t.legs {
                    l.peer = None;
                }
                t.done()
            }
            None => false,
        };
        if done {
            d.tracks.remove(&id);
        }
    }
}

/// Drain one row's handles and apply what arrived.
async fn drain_row(b: &Bridge, id: i64) {
    let tick = b.tick();
    let cap = b.cfg.backoff_cap_ticks;
    // (native id, the row's first acceptance)
    let mut accepts: Vec<(String, bool)> = Vec::new();
    let (write, edge, part_k, withdraw) = {
        let mut d = lock(&b.delivery);
        let Some(t) = d.tracks.get_mut(&id) else {
            return;
        };
        let mut polled: Vec<(usize, Polled)> = Vec::new();
        for (i, leg) in t.legs.iter_mut().enumerate() {
            if let LegIo::Open(rx) = &mut leg.io {
                loop {
                    match rx.try_recv() {
                        Ok(ev) => polled.push((i, Polled::Ev(ev))),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            polled.push((i, Polled::Closed));
                            break;
                        }
                    }
                }
            }
        }
        if polled.is_empty() {
            return;
        }
        for (i, p) in polled {
            match p {
                Polled::Closed => {
                    let leg = &mut t.legs[i];
                    if matches!(leg.io, LegIo::Open(_)) {
                        leg.io = LegIo::Closed;
                    }
                }
                Polled::Ev(ev) => {
                    if let SendEvent::Accepted { native_id } = &ev {
                        // §8 guard 2 compares cleaned inbound ids
                        let clean = ingest::native(native_id).unwrap_or_else(|| native_id.clone());
                        lock(&b.echo).record(&t.network, &clean);
                        let leg = &mut t.legs[i];
                        if !t.recorded {
                            t.recorded = true;
                            accepts.push((native_id.clone(), true));
                        } else if leg.rerecord {
                            // a re-send after air: the new packet id is the row's now
                            accepts.push((native_id.clone(), false));
                        }
                        leg.rerecord = false;
                    }
                    t.legs[i].apply(ev, tick, cap);
                }
            }
        }
        let agg = t.aggregate();
        let write = (t.applied.as_ref() != Some(&agg)).then(|| agg.clone());
        t.applied = Some(agg);
        let withdraw = t
            .withdrawable()
            .map(|(attempts, due)| (t.network.clone(), attempts, due));
        (write, t.edge, t.part_k, withdraw)
    };
    let Ok(f) = b.fab() else { return };
    for (native, first) in accepts {
        if let Err(e) = f.foreign_accepted(id, &native).await {
            eprintln!(
                "111 bridge: {}: accepted not recorded: {e}",
                fjson::fm_id(id)
            );
        }
        if let (true, Some(edge), 1) = (first, edge, part_k) {
            let _ = f.foreign_edge_count(edge, EdgeCounter::Relayed, 1).await;
        }
    }
    if let Some((s, ev, reason)) = write {
        let _ = f
            .foreign_set_state(id, store_state(s), ev.as_deref(), reason.as_deref())
            .await;
    }
    if let Some((network, attempts, due)) = withdraw {
        // F1: refused before air, nothing transmitted — the row is plain `queued` again
        match f.foreign_handoff_abort(id).await {
            Ok(()) => {
                let mut d = lock(&b.delivery);
                d.tracks.remove(&id);
                d.later.insert(
                    id,
                    LaterWait {
                        network,
                        attempts,
                        due,
                    },
                );
                return;
            }
            Err(FabricError::NotFound) => {
                lock(&b.delivery).tracks.remove(&id);
                return;
            }
            Err(e) => {
                // kept: the legs retry from memory with the mark still durable
                eprintln!(
                    "111 bridge: {}: handoff not withdrawn: {e}",
                    fjson::fm_id(id)
                );
            }
        }
    }
    let mut d = lock(&b.delivery);
    if d.tracks.get(&id).is_some_and(Track::done) {
        d.tracks.remove(&id);
    }
}

/// A candidate for this pass.
enum Cand {
    /// A queued row and the attempts it already made (a withdrawn `Later` row's).
    Fresh(Box<ForeignMessageRow>, u32),
    /// `(row, conversation)` with a leg due for its retry.
    Retry(i64, i64),
}

/// A conversation row, read once per pass.
async fn conv_cached(
    b: &Bridge,
    cache: &mut HashMap<i64, ForeignConversationRow>,
    id: i64,
) -> Option<ForeignConversationRow> {
    if let Some(c) = cache.get(&id) {
        return Some(c.clone());
    }
    let c = b.conv_row(id).await.ok()?;
    cache.insert(id, c.clone());
    Some(c)
}

/// The reason nothing can be handed to an adapter now, if any.
fn hold_reason(b: &Bridge, network: &str) -> Option<&'static str> {
    let a = b.attached(network)?;
    if a.link_down() || a.adapter.caps().state != AdapterState::Live {
        return Some("adapter_down");
    }
    if a.adapter.facts().region_set == Some(false) {
        return Some("region_unset");
    }
    None
}

/// One delivery pass over every enabled network (module doc).
pub(super) async fn deliver_pass(b: &Bridge) {
    let Ok(f) = b.fab() else { return };
    let tick = b.tick();
    for (net, att) in b.attached_all() {
        if att.woken.swap(false, std::sync::atomic::Ordering::SeqCst) {
            let mut d = lock(&b.delivery);
            for t in d.tracks.values_mut().filter(|t| t.network == net) {
                for l in &mut t.legs {
                    if let LegIo::Waiting { due, .. } = &mut l.io {
                        *due = (*due).min(tick);
                    }
                }
            }
            for w in d.later.values_mut().filter(|w| w.network == net) {
                w.due = w.due.min(tick);
            }
        }
        let hold = hold_reason(b, &net);
        let mut budget = b.cfg.sends_per_tick;
        let (tracked, mut due): (usize, Vec<(bool, u64, i64, i64)>) = {
            let d = lock(&b.delivery);
            let tracked = d.tracks.values().filter(|t| t.network == net).count()
                + d.later.values().filter(|w| w.network == net).count();
            (
                tracked,
                d.tracks
                    .iter()
                    .filter(|(_, t)| t.network == net && t.retry_due(tick))
                    .map(|(id, t)| (!t.user, t.local_seq, *id, t.conv))
                    .collect(),
            )
        };
        let limit = budget + tracked + QUEUE_SLACK;
        let fresh = match f.foreign_queued(&net, limit).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("111 bridge: {net}: queue read failed: {e}");
                continue;
            }
        };
        let mut cands: Vec<((bool, u64), Cand)> = Vec::with_capacity(fresh.len() + due.len());
        {
            let mut d = lock(&b.delivery);
            if fresh.len() < limit && d.later.values().any(|w| w.network == net) {
                // the whole queue was read: a withdrawn row no longer in it (cancelled,
                // shed, its edge paused or deleted) is not waited for
                let queued: HashSet<i64> = fresh.iter().map(|r| r.id).collect();
                d.later
                    .retain(|id, w| w.network != net || queued.contains(id));
            }
            for r in fresh {
                if d.tracks.contains_key(&r.id)
                    || r.native_msg_id.is_some()
                    || r.handoff_pending
                    || r.dir != ForeignDir::Own
                {
                    continue;
                }
                let prior = match d.later.get(&r.id) {
                    Some(w) if w.due > tick => continue,
                    Some(w) => w.attempts,
                    None => 0,
                };
                cands.push((
                    (r.group_id.is_none(), r.local_seq),
                    Cand::Fresh(Box::new(r), prior),
                ));
            }
        }
        for (relay, seq, id, conv) in due.drain(..) {
            cands.push(((relay, seq), Cand::Retry(id, conv)));
        }
        cands.sort_by_key(|(k, _)| *k);
        let mut convs: HashMap<i64, ForeignConversationRow> = HashMap::new();
        for (_, cand) in cands {
            if budget == 0 {
                break;
            }
            match cand {
                Cand::Retry(id, conv_id) => {
                    if hold.is_some() {
                        continue;
                    }
                    // a retry is a send: the consent gates are read again (a switch turned
                    // off while it waited holds it)
                    let Some(conv) = conv_cached(b, &mut convs, conv_id).await else {
                        continue;
                    };
                    if b.gate_for(&conv, &att.adapter.facts()).is_some() {
                        continue;
                    }
                    budget = budget.saturating_sub(retry_row(b, &att.adapter, id, tick).await);
                    drain_row(b, id).await;
                }
                Cand::Fresh(row, prior) => {
                    let Some(conv) = conv_cached(b, &mut convs, row.conv).await else {
                        continue;
                    };
                    let facts = att.adapter.facts();
                    let reason = b.gate_for(&conv, &facts).map(|g| g.reason()).or(hold);
                    if let Some(reason) = reason {
                        if row.reason_code.as_deref() != Some(reason) {
                            let _ = f
                                .foreign_set_state(row.id, ForeignState::Queued, None, Some(reason))
                                .await;
                        }
                        continue;
                    }
                    let n = hand_off(b, &att.adapter, &row, &conv, &facts, prior).await;
                    budget = budget.saturating_sub(n.max(1));
                    drain_row(b, row.id).await;
                }
            }
        }
    }
}

/// Hand a queued row to `adapter` (module doc); `prior` is the attempts a withdrawn
/// `Later` row already made (0 for a fresh one). Returns the sends made.
async fn hand_off(
    b: &Bridge,
    adapter: &std::sync::Arc<dyn super::Adapter>,
    row: &ForeignMessageRow,
    conv: &ForeignConversationRow,
    facts: &super::AdapterFacts,
    prior: u32,
) -> usize {
    let Ok(f) = b.fab() else { return 0 };
    let targets: Vec<ConvRef> = if is_list_conv(conv) {
        let Some(account) = facts.accounts.first().map(|a| a.id.clone()) else {
            if row.reason_code.as_deref() != Some("adapter_down") {
                let _ = f
                    .foreign_set_state(row.id, ForeignState::Queued, None, Some("adapter_down"))
                    .await;
            }
            return 0;
        };
        match b.list_of(conv) {
            Some(l) if !l.members.is_empty() => l
                .members
                .iter()
                .map(|m| ConvRef {
                    network: conv.network.clone(),
                    account: account.clone(),
                    kind: ConvKind::Dm,
                    native_id: format!("dm:{m}"),
                })
                .collect(),
            _ => {
                // the list is gone: nothing to send to (refused before air)
                let _ = f
                    .foreign_set_state(row.id, ForeignState::Failed, None, Some("no_channel"))
                    .await;
                lock(&b.delivery).later.remove(&row.id);
                return 0;
            }
        }
    } else {
        vec![conv_ref(conv)]
    };
    if let Err(e) = f.foreign_handoff_begin(row.id).await {
        eprintln!("111 bridge: {}: not handed: {e}", fjson::fm_id(row.id));
        return 0;
    }
    let opts = || SendOpts {
        size_mode: SizeMode::Refuse,
        want_ack: true,
    };
    let mut legs = Vec::with_capacity(targets.len());
    for to in targets {
        let h = adapter.send(&to, &row.body, opts()).await;
        let mut leg = Leg::new(to, h.events);
        leg.attempts = prior.saturating_add(1);
        legs.push(leg);
    }
    let n = legs.len();
    let mut d = lock(&b.delivery);
    d.later.remove(&row.id);
    d.tracks.insert(
        row.id,
        Track {
            network: conv.network.clone(),
            conv: conv.id,
            edge: row.edge_id,
            part_k: row.part_k,
            local_seq: row.local_seq,
            user: row.group_id.is_some(),
            text: row.body.clone(),
            legs,
            recorded: false,
            applied: None,
        },
    );
    n
}

/// Send the due legs of a tracked row again: a pre-air wait that ran out, or a leg whose
/// peer was heard again after a post-air NAK (the same text; the adapter mints a new
/// packet id). Returns the sends made.
async fn retry_row(
    b: &Bridge,
    adapter: &std::sync::Arc<dyn super::Adapter>,
    id: i64,
    tick: u64,
) -> usize {
    let due: Vec<(usize, ConvRef, String, u32, bool)> = {
        let d = lock(&b.delivery);
        let Some(t) = d.tracks.get(&id) else { return 0 };
        t.legs
            .iter()
            .enumerate()
            .filter(|(_, l)| l.due(tick))
            .map(|(i, l)| {
                let attempts = match l.io {
                    LegIo::Waiting { attempts, .. } => attempts,
                    _ => l.attempts,
                };
                (
                    i,
                    l.to.clone(),
                    t.text.clone(),
                    attempts,
                    l.peer == Some(true),
                )
            })
            .collect()
    };
    let mut sent = 0;
    for (i, to, text, attempts, after_air) in due {
        let h = adapter
            .send(
                &to,
                &text,
                SendOpts {
                    size_mode: SizeMode::Refuse,
                    want_ack: true,
                },
            )
            .await;
        sent += 1;
        let mut d = lock(&b.delivery);
        if let Some(leg) = d.tracks.get_mut(&id).and_then(|t| t.legs.get_mut(i)) {
            leg.io = LegIo::Open(h.events);
            leg.attempts = attempts.saturating_add(1);
            if after_air {
                // the old packet id stays recorded as an own echo; the new one replaces
                // it on the row once accepted
                leg.peer = None;
                leg.native = None;
                leg.resent = true;
            }
            leg.rerecord = leg.resent;
        }
    }
    sent
}

impl Bridge {
    /// Rows this run is delivering (handed, or waiting for a retry).
    pub fn tracked(&self) -> usize {
        lock(&self.delivery).len()
    }

    /// Whether this run is delivering row `id` (handed and unanswered, waiting for a
    /// retry — withdrawn rows included — or waiting for its peer).
    pub fn is_tracked(&self, id: i64) -> bool {
        let d = lock(&self.delivery);
        d.tracks.contains_key(&id) || d.later.contains_key(&id)
    }

    /// "May have been sent; resend?" (§4): an own row handed to a carrier that never
    /// answered — its pending mark survived a crash (or a disable), so it is never handed
    /// again automatically.
    pub async fn uncertain(&self, id: i64) -> Result<bool, BridgeError> {
        let row = self.fab()?.foreign_message(id).await?;
        Ok(row.dir == ForeignDir::Own
            && row.state == ForeignState::Queued
            && row.handoff_pending
            && !self.is_tracked(id))
    }

    /// Resend a user-composed row that is uncertain or `failed` (§4): a new one-target send
    /// group with the same text and part numbering (so a new packet id); the old row keeps
    /// its state and records `meta.resent_as`. A relay copy is never resent (the edge's mark
    /// has passed its origin). Returns `ForeignSendAccepted`.
    pub async fn resend(&self, id: i64) -> Result<Value, BridgeError> {
        let f = self.fab()?;
        let row = match f.foreign_message(id).await {
            Ok(r) => r,
            Err(FabricError::NotFound) => return Err(BridgeError::NotFound("message")),
            Err(e) => return Err(e.into()),
        };
        if row.dir != ForeignDir::Own || row.group_id.is_none() {
            return Err(BridgeError::BadRequest(
                "only a message you composed is resent; a relay copy is not".into(),
            ));
        }
        let uncertain =
            row.state == ForeignState::Queued && row.handoff_pending && !self.is_tracked(id);
        if !(uncertain || row.state == ForeignState::Failed) {
            return Err(BridgeError::BadRequest(
                "this message is not waiting for a decision".into(),
            ));
        }
        let conv = self.conv_row(row.conv).await?;
        let plan = self.plan_conv(&conv, &row.body, SizeMode::Refuse);
        if let Some(r) = &plan.refusal {
            return Err(match &r.consent {
                Some(c) if r.code != "oversize" => BridgeError::Consent(c.clone()),
                _ => BridgeError::Oversize(r.detail.clone()),
            });
        }
        let target = json!({ "network": conv.network, "conv": fjson::fc_id(conv.id) });
        let targets = fjson::send_group_targets(&[GroupTarget {
            target,
            plan: plan.json(),
            msg: None,
        }]);
        let (_, ids) = self
            .handle
            .foreign_group_create(
                &row.body,
                &targets,
                vec![ForeignGroupPart {
                    conv: conv.id,
                    text: row.body.clone(),
                    part_k: row.part_k,
                    part_n: row.part_n,
                }],
            )
            .await?;
        let new = *ids.first().ok_or(BridgeError::Fabric(FabricError::Invalid(
            "nothing was queued",
        )))?;
        let _ = f
            .foreign_set_meta(id, "resent_as", Some(ForeignMeta::Int(new)))
            .await;
        Ok(json!({ "schema": 1, "id": fjson::fm_id(new), "plan": plan.json() }))
    }

    /// The current state of an own row as the contract names it (tests, routes).
    pub async fn row_state(
        &self,
        id: i64,
    ) -> Result<(S, Option<String>, Option<String>), BridgeError> {
        let r = self.fab()?.foreign_message(id).await?;
        Ok((contract_state(r.state), r.evidence, r.reason_code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leg() -> Leg {
        let (_tx, rx) = mpsc::channel(1);
        Leg::new(
            ConvRef {
                network: "stub-a".into(),
                account: "a".into(),
                kind: ConvKind::Channel,
                native_id: "ch".into(),
            },
            rx,
        )
    }

    fn ev(s: S, e: &str) -> SendEvent {
        SendEvent::Evidence {
            state: s,
            evidence: e.into(),
        }
    }

    #[test]
    fn monotone_matrix_in_a_leg() {
        let mut l = leg();
        l.apply(ev(S::Sent, "accepted by the radio"), 1, 64);
        l.apply(ev(S::Delivered, "the recipient confirmed"), 1, 64);
        l.apply(ev(S::Sent, "late"), 1, 64);
        assert_eq!(l.effective().0, S::Delivered, "delivered never regresses");
        let mut l = leg();
        l.apply(
            SendEvent::Failed {
                reason: Reason::GotNak,
                retry: Retry::Never,
            },
            1,
            64,
        );
        assert_eq!(l.effective(), (S::Failed, None, Some("got_nak".into())));
        l.apply(ev(S::Sent, "x"), 1, 64);
        assert_eq!(l.effective().0, S::Failed, "failed is terminal …");
        l.apply(ev(S::Delivered, "the recipient confirmed"), 1, 64);
        assert_eq!(
            l.effective().0,
            S::Delivered,
            "… unless a later delivered arrives"
        );
    }

    #[test]
    fn post_air_naks_keep_sent_or_relayed() {
        for r in [
            Reason::NoRoute,
            Reason::Timeout,
            Reason::MaxRetransmit,
            Reason::NoResponse,
        ] {
            let mut l = leg();
            l.apply(ev(S::Relayed, "a neighbour rebroadcast it"), 1, 64);
            l.apply(
                SendEvent::Failed {
                    reason: r,
                    retry: Retry::Never,
                },
                1,
                64,
            );
            let (s, e, reason) = l.effective();
            assert_eq!(s, S::Relayed);
            assert_eq!(e.as_deref(), Some(TRANSMITTED_NOT_CONFIRMED));
            assert_eq!(reason, Some(reason_code(r)));
        }
        let mut l = leg();
        l.apply(
            SendEvent::Accepted {
                native_id: "1".into(),
            },
            1,
            64,
        );
        l.apply(
            SendEvent::Failed {
                reason: Reason::PeerUnannounced,
                retry: Retry::Later,
            },
            1,
            64,
        );
        assert_eq!(
            l.effective().0,
            S::Sent,
            "a failure after acceptance was on air: kept, never re-sent"
        );
        assert!(!matches!(l.io, LegIo::Waiting { .. }));
        assert_eq!(
            l.peer, None,
            "only a post-air NAK with Later waits for its peer"
        );
    }

    /// F2: a DM's post-air NAK (`Later`, §4 Reasons, §5) keeps `sent` with "transmitted,
    /// not confirmed" and waits for the peer — not for the tick; `Never` stays as built.
    #[test]
    fn f2_a_post_air_nak_with_later_waits_for_its_peer_not_the_tick() {
        let mut l = leg();
        l.apply(
            SendEvent::Accepted {
                native_id: "0a0b0c0d".into(),
            },
            1,
            64,
        );
        l.apply(ev(S::Sent, "accepted by the radio"), 1, 64);
        l.apply(
            SendEvent::Failed {
                reason: Reason::MaxRetransmit,
                retry: Retry::Later,
            },
            1,
            64,
        );
        assert_eq!(
            l.effective(),
            (
                S::Sent,
                Some(TRANSMITTED_NOT_CONFIRMED.into()),
                Some("max_retransmit".into())
            )
        );
        assert_eq!(l.peer, Some(false));
        assert!(!l.due(u64::MAX), "no tick back-off");
        // a real ack that still comes ends the wait
        l.apply(ev(S::Delivered, "the recipient confirmed"), 2, 64);
        assert_eq!(l.peer, None);
        let mut never = leg();
        never.apply(
            SendEvent::Accepted {
                native_id: "1".into(),
            },
            1,
            64,
        );
        never.apply(
            SendEvent::Failed {
                reason: Reason::Timeout,
                retry: Retry::Never,
            },
            1,
            64,
        );
        assert_eq!(never.peer, None, "Retry::Never post-air stays as built");
        // a re-send refused before air keeps what the first send reached
        let mut re = leg();
        re.state = S::Sent;
        re.resent = true;
        re.apply(
            SendEvent::Failed {
                reason: Reason::RateLimited,
                retry: Retry::Later,
            },
            5,
            64,
        );
        assert!(matches!(re.io, LegIo::Waiting { .. }));
        assert_eq!(re.effective().0, S::Sent, "never back to queued");
        re.io = LegIo::Closed;
        re.apply(
            SendEvent::Failed {
                reason: Reason::GotNak,
                retry: Retry::Never,
            },
            6,
            64,
        );
        assert_eq!(re.effective().0, S::Sent, "never failed after air");
    }

    #[test]
    fn later_before_acceptance_waits_with_doubling_capped_back_off() {
        let mut l = leg();
        let later = || SendEvent::Failed {
            reason: Reason::DutyCycle,
            retry: Retry::Later,
        };
        let mut dues = Vec::new();
        for attempt in 1..=8u32 {
            l.attempts = attempt;
            l.io = LegIo::Closed;
            l.apply(later(), 100, 16);
            match l.io {
                LegIo::Waiting { due, .. } => dues.push(due - 100),
                _ => panic!("waiting"),
            }
        }
        assert_eq!(dues, vec![1, 2, 4, 8, 16, 16, 16, 16]);
        assert_eq!(l.effective(), (S::Queued, None, Some("duty_cycle".into())));
    }

    #[test]
    fn a_list_row_shows_its_lowest_leg() {
        let mut t = Track {
            network: "lxmf".into(),
            conv: 1,
            edge: None,
            part_k: 1,
            local_seq: 1,
            user: true,
            text: "x".into(),
            legs: vec![leg(), leg()],
            recorded: false,
            applied: None,
        };
        t.legs[0].apply(ev(S::Delivered, "the recipient's node confirmed"), 1, 64);
        t.legs[1].apply(ev(S::Sent, "sent"), 1, 64);
        assert_eq!(t.aggregate().0, S::Sent);
        t.legs[1].apply(ev(S::Delivered, "the recipient's node confirmed"), 1, 64);
        assert_eq!(t.aggregate().0, S::Delivered);
        assert!(post_air(Reason::Timeout) && !post_air(Reason::GotNak));
    }
}
