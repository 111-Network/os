//! The delivery state machine (Layer 6 `60-transport/README.md` §4 "Monotone states", "At
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
//!   confirmed" and is never re-sent automatically; `delivered` never regresses.
//! - **`Retry::Later` before acceptance** (queue full, duty cycle, rate limited, key
//!   unknown, peer unannounced) keeps the row `queued` with the reason and retries it on
//!   the tick with capped back-off — attempt `a` waits `min(2^(a-1), backoff_cap_ticks)`
//!   ticks — and at once on an adapter event (link up, a heard announce, a conversation
//!   upsert). The pending mark stays durable while it waits (the store has no handoff
//!   abort): a crash during the wait leaves "may have been sent", never a resend.
//! - **Recipient lists** (§6): a row in a list's conversation is handed as one DM per
//!   member (legs); each leg retries on its own, an accepted leg is never sent again, and
//!   the row shows the lowest state among its legs.

use super::core::{conv_ref, is_list_conv, Bridge, BridgeError};
use super::{
    AdapterState, ConvKind, ConvRef, ForeignMessageState as S, Reason, Retry, SendEvent, SendOpts,
    SizeMode,
};
use crate::daemon::fabric::json::{self as fjson, GroupTarget};
use mesh111::fabric::{FabricError, ForeignGroupPart, ForeignMeta};
use mesh111::store::{
    EdgeCounter, ForeignConversationRow, ForeignDir, ForeignMessageRow, ForeignState,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::mpsc::{self, error::TryRecvError};

/// The evidence a post-air NAK leaves (§4, §5).
pub const TRANSMITTED_NOT_CONFIRMED: &str = "transmitted, not confirmed";
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
        }
    }

    /// What this leg contributes to the row: `(state, evidence, reason)`.
    fn effective(&self) -> (S, Option<String>, Option<String>) {
        if self.failed && self.state.rank() < S::Relayed.rank() {
            return (S::Failed, None, self.reason.clone());
        }
        if matches!(self.io, LegIo::Waiting { .. }) {
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
                    // on air without proof: keep the state, never re-sent automatically
                    if self.state.rank() < S::Sent.rank() {
                        self.state = S::Sent;
                    }
                    self.evidence = Some(TRANSMITTED_NOT_CONFIRMED.to_string());
                    self.reason = Some(code);
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
                } else {
                    self.failed = true;
                    self.reason = Some(code);
                }
            }
        }
    }
}

/// A row this run handed to an adapter.
struct Track {
    network: String,
    conv: i64,
    account: String,
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
        self.legs.iter().all(|l| matches!(l.io, LegIo::Closed))
    }

    fn waiting_due(&self, tick: u64) -> bool {
        self.legs
            .iter()
            .any(|l| matches!(l.io, LegIo::Waiting { due, .. } if due <= tick))
    }
}

/// What the state machine holds in memory (rows handed this run; nothing durable).
#[derive(Default)]
pub(super) struct Delivery {
    tracks: HashMap<i64, Track>,
}

impl Delivery {
    /// Rows tracked (in flight or waiting for a retry).
    pub(super) fn len(&self) -> usize {
        self.tracks.len()
    }
}

enum Polled {
    Ev(SendEvent),
    Closed,
}

/// Forget the in-flight handles of `network` (its adapter was disabled): their rows keep
/// the pending mark ("may have been sent"); legs waiting for a retry stay for the next
/// enable.
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

/// Drain every open handle's events (non-blocking) and write the rows' states.
pub(super) async fn drain_events(b: &Bridge) {
    let ids: Vec<i64> = lock(&b.delivery).tracks.keys().copied().collect();
    for id in ids {
        drain_row(b, id).await;
    }
}

/// Drain one row's handles and apply what arrived.
async fn drain_row(b: &Bridge, id: i64) {
    let tick = b.tick();
    let cap = b.cfg.backoff_cap_ticks;
    let mut accepts = Vec::new();
    let (write, facts) = {
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
            let leg = &mut t.legs[i];
            match p {
                Polled::Closed => {
                    if matches!(leg.io, LegIo::Open(_)) {
                        leg.io = LegIo::Closed;
                    }
                }
                Polled::Ev(ev) => {
                    if let SendEvent::Accepted { native_id } = &ev {
                        lock(&b.echo).record(&t.network, native_id);
                        if !t.recorded {
                            t.recorded = true;
                            accepts.push(native_id.clone());
                        }
                    }
                    leg.apply(ev, tick, cap);
                }
            }
        }
        let agg = t.aggregate();
        let write = (t.applied.as_ref() != Some(&agg)).then(|| agg.clone());
        t.applied = Some(agg);
        (
            write,
            (
                t.network.clone(),
                t.edge,
                t.part_k,
                t.conv,
                t.account.clone(),
                t.local_seq,
            ),
        )
    };
    let Ok(f) = b.fab() else { return };
    let (_, edge, part_k, conv, account, local_seq) = facts;
    for native in accepts {
        if let Err(e) = f.foreign_accepted(id, &native).await {
            eprintln!(
                "111 bridge: {}: accepted not recorded: {e}",
                fjson::fm_id(id)
            );
        }
        if let Some(edge) = edge {
            if part_k == 1 {
                let _ = f.foreign_edge_count(edge, EdgeCounter::Relayed, 1).await;
            }
            let _ = f
                .foreign_seen_insert(edge, conv, &account, &native, local_seq)
                .await;
        }
    }
    if let Some((s, ev, reason)) = write {
        let _ = f
            .foreign_set_state(id, store_state(s), ev.as_deref(), reason.as_deref())
            .await;
    }
    let mut d = lock(&b.delivery);
    if d.tracks.get(&id).is_some_and(Track::done) {
        d.tracks.remove(&id);
    }
}

/// A candidate for this pass.
enum Cand {
    Fresh(Box<ForeignMessageRow>),
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
        }
        let hold = hold_reason(b, &net);
        let mut budget = b.cfg.sends_per_tick;
        let (tracked, mut due): (usize, Vec<(bool, u64, i64, i64)>) = {
            let d = lock(&b.delivery);
            let mine: Vec<&Track> = d.tracks.values().filter(|t| t.network == net).collect();
            (
                mine.len(),
                d.tracks
                    .iter()
                    .filter(|(_, t)| t.network == net && t.waiting_due(tick))
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
            let d = lock(&b.delivery);
            for r in fresh {
                if d.tracks.contains_key(&r.id)
                    || r.native_msg_id.is_some()
                    || r.handoff_pending
                    || r.dir != ForeignDir::Own
                {
                    continue;
                }
                cands.push((
                    (r.group_id.is_none(), r.local_seq),
                    Cand::Fresh(Box::new(r)),
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
                Cand::Fresh(row) => {
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
                    let n = hand_off(b, &att.adapter, &row, &conv, &facts).await;
                    budget = budget.saturating_sub(n.max(1));
                    drain_row(b, row.id).await;
                }
            }
        }
    }
}

/// Hand a fresh row to `adapter` (module doc). Returns the sends made.
async fn hand_off(
    b: &Bridge,
    adapter: &std::sync::Arc<dyn super::Adapter>,
    row: &ForeignMessageRow,
    conv: &ForeignConversationRow,
    facts: &super::AdapterFacts,
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
        legs.push(Leg::new(to, h.events));
    }
    let n = legs.len();
    lock(&b.delivery).tracks.insert(
        row.id,
        Track {
            network: conv.network.clone(),
            conv: conv.id,
            account: conv.account.clone(),
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

/// Send the due legs of a waiting row again. Returns the sends made.
async fn retry_row(
    b: &Bridge,
    adapter: &std::sync::Arc<dyn super::Adapter>,
    id: i64,
    tick: u64,
) -> usize {
    let due: Vec<(usize, ConvRef, String, u32)> = {
        let d = lock(&b.delivery);
        let Some(t) = d.tracks.get(&id) else { return 0 };
        t.legs
            .iter()
            .enumerate()
            .filter_map(|(i, l)| match l.io {
                LegIo::Waiting { attempts, due } if due <= tick => {
                    Some((i, l.to.clone(), t.text.clone(), attempts))
                }
                _ => None,
            })
            .collect()
    };
    let mut sent = 0;
    for (i, to, text, attempts) in due {
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
            leg.attempts = attempts + 1;
        }
    }
    sent
}

impl Bridge {
    /// Rows this run is delivering (handed, or waiting for a retry).
    pub fn tracked(&self) -> usize {
        lock(&self.delivery).len()
    }

    /// Whether this run is delivering row `id`.
    pub fn is_tracked(&self, id: i64) -> bool {
        lock(&self.delivery).tracks.contains_key(&id)
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
            account: "local".into(),
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
