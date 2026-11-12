//! §10.4 — pruning by capacity only (never by policy, never by clock).
//!
//! **Trigger:** live bytes (DB pages minus freelist, plus present blob files)
//! above the high watermark, **or** free disk below a 1 % floor. The caller
//! supplies the disk figures ([`DiskSpace`]; `mesh111` has no `statvfs`).
//! Pruning then frees down to the low watermark (and back above the floor).
//!
//! **Normative order:**
//! 1. transit custody that is `committed` — member rows a member committed
//!    and (Layer 7 §5.8) forward rows the next custodian acknowledged or
//!    every target refused (the refused mark) — public chunks that are
//!    re-fetchable (and delivered transit copies) — weight = size × age, old
//!    and large first; age = local-sequence distance;
//! 2. superseded release/KB objects (below the Layer 1 floor), fully fetched
//!    deltas — marked by their consumer ([`Store::blob_mark_superseded`]);
//!
//!    **2a** (Layer 6 §3) foreign inbound rows of conversations that are not
//!    pinned, weight = size × `local_seq` distance (as step 1), together
//!    with relay copies already handed to a carrier (`edge_id` set, no
//!    longer `queued`, or `queued` with `meta.handoff = "pending"`; module
//!    note below);
//!
//!    **2b** (Layer 6 §3) relay backlog copies (`dir own`, `edge_id` set,
//!    `queued`, no pending handoff) oldest first; the queued parts of one
//!    copy go together, the
//!    shed is counted on its edge (`shed += 1`) and the edge's `relayed_seq`
//!    is advanced past the copy's origin, so shedding never makes history
//!    relayable again;
//! 3. own conversation history oldest-first — **only at hard capacity**
//!    (the store itself still short after 1–2; never to make room for an
//!    incoming custody or bulk item, which is refused instead) **and only
//!    after an export offer** ([`Store::set_export_offered`]); otherwise
//!    `export_offer_needed`;
//! 4. **never** undelivered outbound, uncommitted custody (forward rows
//!    included, Layer 7 §5.3 amendment 6: a forward row marked refused is
//!    committed, so it is a step-1 row), pending requests,
//!    pinned items or contacts — nor (Layer 6 §3) a user-composed foreign
//!    send (`group_id` set) while undelivered, the rows of a pinned foreign
//!    conversation, or the bounded `foreign_seen` keys — instead
//!    `store_full` is raised and new custody, new bulk and new message
//!    requests are refused (messages, foreign rows included, are never
//!    refused).
//!
//! **Per-class quotas** (settings, configurable) shape capacity for the
//! refusable classes (custody and bulk): a class above its quota loses its
//! step-1/2 items first; if still above, new items of that class are refused.
//! [`Class::Foreign`] has a **shaping quota only** (Layer 6 §3, settings key
//! `quota.foreign`, unset = unlimited): over it, its step-2a/2b items are
//! evicted **first**, before any other quota or step, and it never refuses
//! (it is never in `full_classes`).
//!
//! **Transit (Layer 7 §5.8).** `custody` rows split by `fwd`:
//! [`Class::Custody`] counts the member rows (`fwd = 0`) only and
//! [`Class::Transit`] the forward rows (`fwd != 0`), so strangers' items
//! never count against members' custody quota. Transit always has a quota:
//! `quota.transit` when set, else a quarter of the custody quota when one is
//! set, else a quarter of the high watermark (128 MiB at the 512 MiB
//! default). Inside it each origin (`fwd_from`) holds at most
//! [`Store::transit_share`] (`quota.transit / 8`, 16 MiB of the default) of
//! protected forward rows; over it, that origin's forward puts are refused
//! while other origins are admitted ([`Store::custody_put_fwd`]). Protected
//! forward rows are DB rows, so they count in live bytes toward the high
//! watermark like every other row: at the cap, member custody is refused too.
//! The Custody and Transit totals and an origin's protected bytes are read
//! from the schema v5 `custody_totals` ([`custody_total`]), one row by
//! primary key; the answers are the sums the rows give.
//!
//! **What a pass and an admission cost (the Pi floor).** Every input that
//! decides whether a pass prunes is read without a candidate list: the
//! settings by primary key, live bytes ([`live_bytes`]: three pragmas and
//! a `SUM(disk_bytes)` over the present blobs, so O(blobs)), and the total
//! of each class with a quota (Custody and Transit: one `custody_totals`
//! row; a bulk class or Foreign with a quota set: a sum over its blobs and
//! messages, or over `foreign_messages`). A pass **at rest** — no quota
//! over counting the incoming item, live bytes with it not over the high
//! watermark, free disk not under the floor — prunes nothing, reads no
//! candidate, and writes only when its persisted outcome changes (below).
//! A pass that must prune reads each step list when it reaches it, for the
//! one class a quota step prunes or for every class, and keeps in memory
//! only the head of the list that frees what is still needed (the same
//! items in the same order as the whole sorted list gives). Reading a step
//! costs: step 1, the committed `custody` rows of the class through the v5
//! partial index `custody_committed` (never a protected row; bounded by
//! the committed rows of that class, which every pruning pass evicts
//! first), plus a pass over the blobs and the messages; step 2, a pass
//! over the blobs; steps 2a and 2b (Foreign only), a pass over
//! `foreign_messages`; step 3, at hard capacity only, the history. The
//! weight order is a product of size and age, so no index serves it: a
//! step's rows are read in full and ranked in memory. An admission
//! ([`Store::admit`]) reads its class's quota and total and the live bytes
//! (O(blobs)) and runs a pass only when one of them is over or
//! `store_full` is set.
//!
//! **Live bytes** are DB pages, so the v3 foreign tables count as soon as
//! they hold rows; [`Store::class_bytes`] of `Foreign` is the rows' body and
//! `meta` bytes plus the per-row overhead.
//!
//! *Note (as-built reading of Layer 6 §3):* relay copies that a carrier has
//! already taken (`sent` and later) are not in the "never" list and are not
//! 2b's backlog; they are evicted with 2a (same weight), so an active edge
//! cannot grow the store without bound. A copy still `queued` but with
//! `meta.handoff = "pending"` may have been transmitted, so it counts as
//! handed too (a 2a row, never a shed counted on the edge) — the same line
//! [`Store::edge_delete`] and [`Store::edge_set_enabled`] draw, which drop
//! only never-handed copies and keep pending ones. Delivered user-composed rows are in
//! no step (human-rate bounded, like own history before an export offer).
//! Pending message requests have a count quota ([`Store::request_quota`],
//! default [`DEFAULT_REQUEST_QUOTA`]).
//!
//! **Persistence:** `store_full`, the classes over quota and the last disk
//! figures are persisted in `settings` by every pass (a pass at rest writes
//! them only when they differ from what is stored) and read back at open,
//! so a full store stays full across a restart until a pass with room
//! clears it.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension};

use crate::blobs::BlobHash;

use super::rows::Class;
use super::{
    current_local_seq, get_u64, id32, set_i64, set_u64, to_u64, Durability, Result, Store,
    StoreError,
};

/// Default high watermark (configurable in settings).
pub const DEFAULT_HIGH_WATERMARK: u64 = 512 * 1024 * 1024;
/// Default cap on pending message requests (§7.2 (3); configurable).
pub const DEFAULT_REQUEST_QUOTA: u64 = 256;
/// Rough per-row overhead counted for DB items (indexes, row header).
pub(crate) const ROW_OVERHEAD: u64 = 64;

const K_HIGH: &str = "watermark.high";
const K_LOW: &str = "watermark.low";
const K_EXPORT_OFFERED: &str = "export_offered";
const K_REQUEST_QUOTA: &str = "quota.requests";
const K_STORE_FULL: &str = "capacity.store_full";
const K_FULL_CLASSES: &str = "capacity.full_classes";
const K_DISK_FREE: &str = "capacity.disk_free";
const K_DISK_TOTAL: &str = "capacity.disk_total";

fn quota_key(c: Class) -> String {
    format!("quota.{}", c.name())
}

/// Free/total bytes of the filesystem holding the store (from the caller).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskSpace {
    pub free: u64,
    pub total: u64,
}

impl DiskSpace {
    /// No disk figures known: the floor never triggers.
    pub const UNKNOWN: DiskSpace = DiskSpace {
        free: u64::MAX,
        total: 0,
    };

    /// The 1 % floor in bytes.
    pub fn floor(&self) -> u64 {
        self.total.div_ceil(100)
    }

    /// Free space is below the 1 % floor.
    pub fn below_floor(&self) -> bool {
        self.free < self.floor()
    }
}

/// High/low watermarks of live bytes (`low ≤ high`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watermarks {
    pub high: u64,
    pub low: u64,
}

impl Watermarks {
    /// `high` with the default hysteresis (`low = high − high/8`).
    pub fn with_high(high: u64) -> Self {
        Watermarks {
            high,
            low: high - high / 8,
        }
    }
}

/// Which §10.4 step removed an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneStep {
    /// A class over its quota (steps 1–2 candidates of that class).
    Quota(Class),
    /// Step 1: committed custody, re-fetchable public objects, delivered
    /// transit copies.
    Evictable,
    /// Step 2: superseded objects.
    Superseded,
    /// Step 2a (Layer 6 §3): foreign inbound rows (and handed relay copies).
    ForeignInbound,
    /// Step 2b (Layer 6 §3): relay backlog copies (queued), oldest first.
    RelayBacklog,
    /// Step 3: own conversation history (after an export offer).
    History,
}

/// What was removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrunedRef {
    Custody(u64),
    Blob(BlobHash),
    Message(i64),
    /// A `foreign_messages` row (step 2a).
    Foreign(i64),
    /// A queued relay copy (step 2b): the id of its first queued part; its
    /// other queued parts went with it.
    RelayCopy(i64),
}

/// One removed item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrunedItem {
    pub step: PruneStep,
    pub item: PrunedRef,
    /// Bytes the item accounted for.
    pub bytes: u64,
}

/// The outcome of one pruning pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneReport {
    /// Live bytes or free disk crossed a limit.
    pub triggered: bool,
    pub live_before: u64,
    pub live_after: u64,
    /// In removal order.
    pub pruned: Vec<PrunedItem>,
    /// Only protected items remain over the limit: new custody and new bulk
    /// are refused (surfaced on the `fabric` service and to the agent).
    pub store_full: bool,
    /// Own history would be pruned next, but no export was offered yet.
    pub export_offer_needed: bool,
    /// Refusable classes still above their quota.
    pub full_classes: Vec<Class>,
}

/// One item a step may remove: what it is, the bytes it accounts for, its
/// age (`local_seq`) and its class (the read selects the list's class,
/// [`Only`]; [`Head::push`] keeps only that class).
#[derive(Debug, Clone, Copy)]
struct Candidate {
    item: PrunedRef,
    bytes: u64,
    local_seq: u64,
    class: Class,
}

/// One candidate with its §10.4 weight, `size × age` (u128, no overflow),
/// ordered as the step lists are: larger first, then older, then by item.
/// The order is total (no two candidates share an item), so any subset of
/// a list keeps its relative order however it was read.
#[derive(Debug, Clone, Copy)]
struct Ranked {
    weight: u128,
    cand: Candidate,
}

impl Ranked {
    fn new(cand: Candidate, now: u64) -> Self {
        let age = u128::from(now.saturating_sub(cand.local_seq)) + 1;
        Ranked {
            weight: u128::from(cand.bytes) * age,
            cand,
        }
    }
}

impl Ord for Ranked {
    /// `Less` comes first in a step list.
    fn cmp(&self, o: &Self) -> Ordering {
        o.weight
            .cmp(&self.weight)
            .then(self.cand.local_seq.cmp(&o.cand.local_seq))
            .then(key(&self.cand.item).cmp(&key(&o.cand.item)))
    }
}

impl PartialOrd for Ranked {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl PartialEq for Ranked {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}

impl Eq for Ranked {}

fn key(r: &PrunedRef) -> (u8, i128, [u8; 32]) {
    match r {
        PrunedRef::Custody(s) => (0, i128::from(*s), [0; 32]),
        PrunedRef::Blob(h) => (1, 0, h.0),
        PrunedRef::Message(id) => (2, i128::from(*id), [0; 32]),
        PrunedRef::Foreign(id) => (3, i128::from(*id), [0; 32]),
        PrunedRef::RelayCopy(id) => (4, i128::from(*id), [0; 32]),
    }
}

/// The head of one weight-ordered step list: the candidates a pass can
/// reach before it has freed `need` bytes (§10.4: it takes items in order
/// while it is still short, so it never reaches past the first whose bytes
/// add up to `need`). Fed every candidate of the step in any order, it
/// keeps exactly that head, in order: a candidate the head no longer needs
/// (the heads before it already free `need`) leaves at once, so memory is
/// bounded by the bytes to free, not the rows held. `need = None` keeps
/// every candidate (a quota the class cannot get under, where the pass
/// takes them all).
struct Head {
    only: Only,
    need: Option<u128>,
    now: u64,
    held: u128,
    /// A max-heap: its top is the last candidate of the head.
    heap: BinaryHeap<Ranked>,
}

impl Head {
    fn new(only: Only, need: Option<u64>, now: u64) -> Self {
        Head {
            only,
            need: need.map(u128::from),
            now,
            held: 0,
            heap: BinaryHeap::new(),
        }
    }

    /// Offer one candidate; one of another class than the list's is not
    /// the list's (the reads select by class already: this keeps the rule
    /// the pass states, `class` decides).
    fn push(&mut self, cand: Candidate) {
        if !self.only.has(cand.class) {
            return;
        }
        self.held += u128::from(cand.bytes);
        self.heap.push(Ranked::new(cand, self.now));
        let Some(need) = self.need else { return };
        while let Some(last) = self.heap.peek() {
            let b = u128::from(last.cand.bytes);
            if self.held - b < need {
                break;
            }
            self.held -= b;
            self.heap.pop();
        }
    }

    /// The head in step order.
    fn into_vec(self) -> Vec<Candidate> {
        self.heap
            .into_sorted_vec()
            .into_iter()
            .map(|r| r.cand)
            .collect()
    }
}

/// The steps whose candidates a pass reads (step 3 is [`history`], read
/// only at hard capacity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Step 1 ([`evictable`]).
    One,
    /// Step 2 ([`superseded`]).
    Two,
    /// Step 2a ([`foreign_inbound`]).
    TwoA,
    /// Step 2b ([`relay_backlog`]).
    TwoB,
}

/// Steps 1–2b in the normative order, with the label a watermark or floor
/// pass reports.
const STEPS: [(Step, PruneStep); 4] = [
    (Step::One, PruneStep::Evictable),
    (Step::Two, PruneStep::Superseded),
    (Step::TwoA, PruneStep::ForeignInbound),
    (Step::TwoB, PruneStep::RelayBacklog),
];

/// Which candidates a step list holds: every class (the watermark and
/// floor pass) or one class (that class's quota step, which skips every
/// other class's candidates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Only {
    All,
    Class(Class),
}

impl Only {
    fn has(self, class: Class) -> bool {
        match self {
            Only::All => true,
            Only::Class(c) => c == class,
        }
    }

    /// The `fwd` range of the committed `custody` rows of this filter
    /// (Layer 7 §5.3 amendment 6: `fwd = 0` is Custody, `fwd != 0`
    /// Transit; `fwd` is `CHECK`ed to 0–3); `None`: no custody row.
    fn fwd_range(self) -> Option<(i64, i64)> {
        match self {
            Only::All => Some((0, 3)),
            Only::Class(Class::Custody) => Some((0, 0)),
            Only::Class(Class::Transit) => Some((1, 3)),
            Only::Class(_) => None,
        }
    }

    /// `AND class = ?1` with its parameter, for a table with a `class`.
    fn class_filter(self) -> (&'static str, Option<i64>) {
        match self {
            Only::All => ("", None),
            Only::Class(c) => (" AND class = ?1", Some(c as i64)),
        }
    }
}

/// The candidates of `step` under `only`, in the step's order, cut to the
/// head that frees `need` bytes ([`Head`]; for step 2b, ordered by the
/// query, the first rows that do). Read when a pass reaches the step, so a
/// row an earlier step or quota removed is not read again.
fn step_list(
    c: &Connection,
    step: Step,
    only: Only,
    need: Option<u64>,
    now: u64,
) -> Result<Vec<Candidate>> {
    let mut head = Head::new(only, need, now);
    match step {
        Step::One => evictable(c, only, &mut head)?,
        Step::Two => superseded(c, only, &mut head)?,
        Step::TwoA => foreign_inbound(c, only, &mut head)?,
        Step::TwoB => return relay_backlog(c, only, need),
    }
    Ok(head.into_vec())
}

/// The committed `custody` rows of a `fwd` range (step 1): a bounded
/// search of the v5 partial index `custody_committed`, never a protected
/// row. The bytes are `length(bytes) + coalesce(length(fwd_from), 0)`:
/// the item plus, for a forward row, its 32 B `fwd_from` (NULL, so 0, for
/// a member row: member rows count exactly what they counted before v4).
/// The v5 `custody_totals` keep the same two parts per key (`item_bytes`,
/// `from_bytes`; [`CustodyTotal::bytes`]). Crate-visible so the tests can
/// show its plan.
pub(crate) const EVICTABLE_CUSTODY_SQL: &str =
    "SELECT custody_seq, length(bytes) + coalesce(length(fwd_from), 0), local_seq, fwd FROM custody
     WHERE committed = 1 AND fwd BETWEEN ?1 AND ?2";

/// Step 1 candidates. A committed `custody` row is [`Class::Custody`] when
/// `fwd = 0` and [`Class::Transit`] otherwise (Layer 7 §5.3 amendment 6).
fn evictable(c: &Connection, only: Only, out: &mut Head) -> Result<()> {
    if let Some((lo, hi)) = only.fwd_range() {
        let mut st = c.prepare(EVICTABLE_CUSTODY_SQL)?;
        let rows = st.query_map([lo, hi], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        for r in rows {
            let (seq, len, local, fwd) = r?;
            out.push(Candidate {
                item: PrunedRef::Custody(to_u64(seq, "custody_seq")?),
                bytes: to_u64(len, "custody.len")? + ROW_OVERHEAD,
                local_seq: to_u64(local, "custody.local_seq")?,
                class: if fwd == 0 {
                    Class::Custody
                } else {
                    Class::Transit
                },
            });
        }
    }
    blob_candidates(
        c,
        "public = 1 AND refetchable = 1 AND present = 1 AND pinned = 0",
        only,
        |cand| out.push(cand),
    )?;
    message_candidates(
        c,
        "dir = 2 AND state = 3 AND pinned = 0",
        "local_seq",
        only,
        |cand| out.push(cand),
    )
}

/// Step 2 candidates.
fn superseded(c: &Connection, only: Only, out: &mut Head) -> Result<()> {
    blob_candidates(
        c,
        "superseded = 1 AND present = 1 AND pinned = 0 AND NOT (public = 1 AND refetchable = 1)",
        only,
        |cand| out.push(cand),
    )
}

/// Bytes one foreign row accounts for (SQL expression over alias `m`).
const FOREIGN_ROW_BYTES: &str = "octet_length(m.body) + octet_length(m.meta)";

/// Step 2a candidates (Layer 6 §3): foreign inbound of unpinned
/// conversations, plus relay copies already handed to a carrier (past
/// `queued`, or `queued` with a pending handoff: may have been sent).
fn foreign_inbound(c: &Connection, only: Only, out: &mut Head) -> Result<()> {
    if !only.has(Class::Foreign) {
        return Ok(());
    }
    let mut st = c.prepare(&format!(
        "SELECT m.id, {FOREIGN_ROW_BYTES}, m.local_seq FROM foreign_messages m
         JOIN foreign_conversations fc ON fc.id = m.conv
         WHERE fc.pinned = 0
           AND (m.dir = 'in'
                OR (m.edge_id IS NOT NULL
                    AND (m.state <> 'queued'
                         OR json_extract(m.meta, '$.handoff') IS 'pending')))"
    ))?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for r in rows {
        let (id, len, local) = r?;
        out.push(Candidate {
            item: PrunedRef::Foreign(id),
            bytes: to_u64(len, "foreign.len")? + ROW_OVERHEAD,
            local_seq: to_u64(local, "foreign.local_seq")?,
            class: Class::Foreign,
        });
    }
    Ok(())
}

/// Step 2b candidates (Layer 6 §3): queued relay copies of unpinned
/// conversations never handed (no pending handoff), one candidate per copy
/// (all such parts), oldest first; the first copies that free `need` bytes
/// (`None`: all).
fn relay_backlog(c: &Connection, only: Only, need: Option<u64>) -> Result<Vec<Candidate>> {
    let mut out = Vec::new();
    if !only.has(Class::Foreign) {
        return Ok(out);
    }
    let mut st = c.prepare(&format!(
        "SELECT min(m.id), sum({FOREIGN_ROW_BYTES} + ?1), min(m.local_seq)
         FROM foreign_messages m
         JOIN foreign_conversations fc ON fc.id = m.conv
         WHERE fc.pinned = 0 AND m.dir = 'own' AND m.edge_id IS NOT NULL
           AND m.state = 'queued' AND json_extract(m.meta, '$.handoff') IS NOT 'pending'
         GROUP BY m.edge_id, m.origin_msg
         ORDER BY min(m.local_seq), min(m.id)"
    ))?;
    let rows = st.query_map([ROW_OVERHEAD as i64], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    let mut held = 0u128;
    for r in rows {
        if need.is_some_and(|n| held >= u128::from(n)) {
            break;
        }
        let (id, len, local) = r?;
        let bytes = to_u64(len, "foreign.len")?;
        held += u128::from(bytes);
        out.push(Candidate {
            item: PrunedRef::RelayCopy(id),
            bytes,
            local_seq: to_u64(local, "foreign.local_seq")?,
            class: Class::Foreign,
        });
    }
    Ok(out)
}

/// Shed one queued relay copy (step 2b): delete its queued parts, count the
/// shed on its edge and advance the edge's `relayed_seq` past the origin
/// (when the origin row is still held; queueing the copy already advanced
/// it, Layer 6 §8).
fn shed_copy(c: &Connection, first: i64) -> Result<()> {
    let row: Option<(Option<i64>, Option<i64>)> = c
        .query_row(
            "SELECT edge_id, origin_msg FROM foreign_messages WHERE id = ?1",
            [first],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((Some(edge), Some(origin))) = row else {
        return Ok(());
    };
    let origin_seq: Option<i64> = c
        .query_row(
            "SELECT local_seq FROM foreign_messages WHERE id = ?1",
            [origin],
            |r| r.get(0),
        )
        .optional()?;
    let n = c.execute(
        "DELETE FROM foreign_messages
         WHERE edge_id = ?1 AND origin_msg = ?2 AND dir = 'own' AND state = 'queued'
           AND json_extract(meta, '$.handoff') IS NOT 'pending'",
        params![edge, origin],
    )?;
    if n > 0 {
        c.execute(
            "UPDATE relay_edges
             SET shed = shed + 1, relayed_seq = max(relayed_seq, coalesce(?2, relayed_seq))
             WHERE id = ?1",
            params![edge, origin_seq],
        )?;
    }
    Ok(())
}

/// Step 3 candidates, oldest first: delivered own/inbound history, never
/// pinned, never with outbox fragments, never from a pending request.
fn history(c: &Connection) -> Result<Vec<Candidate>> {
    let mut out = Vec::new();
    message_candidates(
        c,
        "dir IN (0, 1) AND state = 3 AND pinned = 0
         AND NOT EXISTS (SELECT 1 FROM outbox o WHERE o.message_id = messages.id)
         AND peer NOT IN (SELECT node_id FROM requests)
         AND peer NOT IN (SELECT node_id FROM contacts WHERE request = 1)",
        "local_seq",
        Only::All,
        |cand| out.push(cand),
    )?;
    Ok(out)
}

fn blob_candidates(
    c: &Connection,
    filter: &str,
    only: Only,
    mut sink: impl FnMut(Candidate),
) -> Result<()> {
    let (class, p) = only.class_filter();
    let mut st = c.prepare(&format!(
        "SELECT hash, disk_bytes, local_seq, class FROM blobs WHERE {filter}{class}"
    ))?;
    let rows = st.query_map(rusqlite::params_from_iter(p), |r| {
        Ok((
            r.get::<_, Vec<u8>>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    for r in rows {
        let (h, disk, local, class) = r?;
        sink(Candidate {
            item: PrunedRef::Blob(BlobHash(id32(h, "blobs.hash")?)),
            bytes: to_u64(disk, "blobs.disk_bytes")? + ROW_OVERHEAD,
            local_seq: to_u64(local, "blobs.local_seq")?,
            class: Class::from_i64(class)?,
        });
    }
    Ok(())
}

fn message_candidates(
    c: &Connection,
    filter: &str,
    order: &str,
    only: Only,
    mut sink: impl FnMut(Candidate),
) -> Result<()> {
    let (class, p) = only.class_filter();
    let mut st = c.prepare(&format!(
        "SELECT id, coalesce(length(body), 0) + coalesce(length(blob_refs), 0), local_seq, class
         FROM messages WHERE {filter}{class} ORDER BY {order}"
    ))?;
    let rows = st.query_map(rusqlite::params_from_iter(p), |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    for r in rows {
        let (id, len, local, class) = r?;
        sink(Candidate {
            item: PrunedRef::Message(id),
            bytes: to_u64(len, "messages.len")? + ROW_OVERHEAD,
            local_seq: to_u64(local, "messages.local_seq")?,
            class: Class::from_i64(class)?,
        });
    }
    Ok(())
}

/// Remove one candidate inside the caller's transaction. Blob files are
/// deleted after commit (`present = 0` first: a crash leaves an orphan
/// directory, swept at open — never a row pointing at a missing file).
fn remove(c: &Connection, item: &PrunedRef) -> Result<()> {
    match item {
        PrunedRef::Custody(seq) => {
            c.execute(
                "DELETE FROM custody WHERE custody_seq = ?1",
                [super::to_i64(*seq, "custody_seq")?],
            )?;
        }
        PrunedRef::Blob(h) => {
            c.execute("UPDATE blobs SET present = 0 WHERE hash = ?1", [&h.0[..]])?;
        }
        PrunedRef::Message(id) => {
            c.execute("DELETE FROM messages WHERE id = ?1", [id])?;
        }
        PrunedRef::Foreign(id) => {
            c.execute("DELETE FROM foreign_messages WHERE id = ?1", [id])?;
        }
        PrunedRef::RelayCopy(id) => shed_copy(c, *id)?,
    }
    Ok(())
}

/// Live bytes (§10.4): DB pages minus freelist, plus present blob files.
pub(crate) fn live_bytes(c: &Connection) -> Result<u64> {
    let pages = super::pragma_int(c, "page_count")?;
    let free = super::pragma_int(c, "freelist_count")?;
    let size = super::pragma_int(c, "page_size")?;
    let db = to_u64(pages.saturating_sub(free), "page_count")?
        .saturating_mul(to_u64(size, "page_size")?);
    let blobs: i64 = c.query_row(
        "SELECT coalesce(sum(disk_bytes), 0) FROM blobs WHERE present = 1",
        [],
        |r| r.get(0),
    )?;
    Ok(db.saturating_add(to_u64(blobs, "blobs.disk_bytes")?))
}

/// One `custody_totals` row (schema v5, [`super::schema`]): the count and
/// byte sums of the `custody` rows under one [`TotalKey`], kept by the v5
/// triggers in every writing transaction. Reading one costs a primary-key
/// lookup, whatever the rows held (the Pi floor: no work per put that grows
/// with the store).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CustodyTotal {
    /// Rows under the key.
    pub(crate) rows: u64,
    /// `Σ length(bytes)`: the items.
    pub(crate) item_bytes: u64,
    /// `Σ coalesce(length(fwd_from), 0)`: 32 B per forward row, 0 per
    /// member row.
    pub(crate) from_bytes: u64,
}

impl CustodyTotal {
    /// The bytes these rows account for (§10.4; Layer 7 §5.8): per row
    /// `length(bytes) + coalesce(length(fwd_from), 0)` (the step-1 read's
    /// bytes, [`EVICTABLE_CUSTODY_SQL`]) plus [`ROW_OVERHEAD`], summed —
    /// exactly what a `SUM` of that expression over the same rows returns. An overflow is
    /// a corrupt total (a store never holds 2^64 bytes), never a wrap.
    pub(crate) fn bytes(&self) -> Result<u64> {
        self.rows
            .checked_mul(ROW_OVERHEAD)
            .and_then(|o| o.checked_add(self.item_bytes))
            .and_then(|b| b.checked_add(self.from_bytes))
            .ok_or(StoreError::Corrupt("custody_totals"))
    }
}

/// The key of one `custody_totals` row (schema v5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TotalKey<'a> {
    /// `x'00'`: the member rows (`fwd = 0`) — [`Class::Custody`].
    Member,
    /// `x'01'`: every forward row (`fwd != 0`) — [`Class::Transit`].
    Forward,
    /// `x'02'`: the protected forward rows (`fwd != 0`, `committed = 0`).
    Protected,
    /// `x'03' ‖ origin`: the protected forward rows whose `fwd_from` is
    /// `origin` (Layer 7 §5.8 share).
    Origin(&'a [u8; 32]),
}

/// Read one `custody_totals` row by its primary key (crate-visible so the
/// tests can show its plan).
pub(crate) const CUSTODY_TOTAL_SQL: &str =
    "SELECT rows, item_bytes, from_bytes FROM custody_totals WHERE key = ?1";

/// The totals under `key`: a primary-key lookup. A key with no row holds
/// no rows (an origin with no protected forward row has none; the three
/// fixed keys always have one).
pub(crate) fn custody_total(c: &Connection, key: TotalKey<'_>) -> Result<CustodyTotal> {
    let mut origin_key = [0u8; 33];
    let k: &[u8] = match key {
        TotalKey::Member => &[0x00],
        TotalKey::Forward => &[0x01],
        TotalKey::Protected => &[0x02],
        TotalKey::Origin(o) => {
            origin_key[0] = 0x03;
            origin_key[1..].copy_from_slice(o);
            &origin_key
        }
    };
    let row: Option<(i64, i64, i64)> = c
        .query_row(CUSTODY_TOTAL_SQL, [k], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let Some((rows, item, from)) = row else {
        return Ok(CustodyTotal::default());
    };
    Ok(CustodyTotal {
        rows: to_u64(rows, "custody_totals.rows")?,
        item_bytes: to_u64(item, "custody_totals.item_bytes")?,
        from_bytes: to_u64(from, "custody_totals.from_bytes")?,
    })
}

/// Bytes a class accounts for. `custody` splits by `fwd` (Layer 7 §5.3
/// amendment 6): `Custody` is the member rows (`fwd = 0`), `Transit` the
/// forward rows (`fwd != 0`, each with its `fwd_from`); both read their
/// schema v5 total ([`custody_total`]), O(1) in the rows held.
pub(crate) fn class_bytes(c: &Connection, class: Class) -> Result<u64> {
    match class {
        Class::Custody => return custody_total(c, TotalKey::Member)?.bytes(),
        Class::Transit => return custody_total(c, TotalKey::Forward)?.bytes(),
        _ => {}
    }
    let v: i64 = if class == Class::Foreign {
        c.query_row(
            &format!("SELECT coalesce(sum({FOREIGN_ROW_BYTES} + ?1), 0) FROM foreign_messages m"),
            [ROW_OVERHEAD as i64],
            |r| r.get(0),
        )?
    } else {
        c.query_row(
            "SELECT (SELECT coalesce(sum(disk_bytes + ?2), 0) FROM blobs
                     WHERE class = ?1 AND present = 1)
                  + (SELECT coalesce(sum(coalesce(length(body), 0)
                                         + coalesce(length(blob_refs), 0) + ?2), 0)
                     FROM messages WHERE class = ?1)",
            params![class as i64, ROW_OVERHEAD as i64],
            |r| r.get(0),
        )?
    };
    to_u64(v, "class bytes")
}

/// Bytes a pass must free (§10.4 trigger): with `live` live bytes and an
/// incoming item of `extra` bytes, down to the low watermark once over the
/// high one, and back above the disk floor (the incoming item counted)
/// while free disk is under it; 0 when neither holds.
fn need_for(live: u64, extra: u64, disk: DiskSpace, w: Watermarks) -> u64 {
    let mut need = 0;
    if live.saturating_add(extra) > w.high {
        need = live.saturating_add(extra) - w.low;
    }
    if disk.below_floor() {
        need = need.max((disk.floor() - disk.free).saturating_add(extra));
    }
    need
}

impl Store {
    /// Live bytes now (§10.4 trigger input).
    pub fn live_bytes(&self) -> Result<u64> {
        live_bytes(&self.conn)
    }

    /// Bytes `class` accounts for (quota input; store gauge).
    pub fn class_bytes(&self, class: Class) -> Result<u64> {
        class_bytes(&self.conn, class)
    }

    /// The configured watermarks (defaults when unset).
    pub fn watermarks(&self) -> Result<Watermarks> {
        let high = get_u64(&self.conn, K_HIGH)?.unwrap_or(DEFAULT_HIGH_WATERMARK);
        let low = get_u64(&self.conn, K_LOW)?.unwrap_or(high - high / 8);
        Ok(Watermarks {
            high,
            low: low.min(high),
        })
    }

    /// Configure the watermarks (`low ≤ high`).
    pub fn set_watermarks(&mut self, w: Watermarks) -> Result<()> {
        if w.low > w.high {
            return Err(StoreError::Invalid("low watermark above high"));
        }
        self.with_tx(Durability::Normal, |tx| {
            set_u64(tx, K_HIGH, w.high)?;
            set_u64(tx, K_LOW, w.low)
        })
    }

    /// The quota of `class`, if any.
    ///
    /// [`Class::Transit`] always has one (Layer 7 §5.8): `quota.transit`
    /// when set, else one quarter of the custody quota when one is set,
    /// else one quarter of the configured high watermark — 128 MiB at the
    /// 512 MiB default ([`DEFAULT_HIGH_WATERMARK`]).
    pub fn quota(&self, class: Class) -> Result<Option<u64>> {
        let set = get_u64(&self.conn, &quota_key(class))?;
        if class != Class::Transit || set.is_some() {
            return Ok(set);
        }
        let base = match get_u64(&self.conn, &quota_key(Class::Custody))? {
            Some(custody) => custody,
            None => self.watermarks()?.high,
        };
        Ok(Some(base / 4))
    }

    /// The per-origin share of Transit (Layer 7 §5.8, D31 (6)): one eighth
    /// of the Transit quota ([`Store::quota`]), 16 MiB of the default;
    /// derived, no setting of its own. Each origin of forward rows (a
    /// member, a leaf hub or a carrier pair, by `fwd_from`) holds at most
    /// this much of protected forward rows ([`Store::forward_origin_bytes`]).
    pub fn transit_share(&self) -> Result<u64> {
        Ok(self.quota(Class::Transit)?.unwrap_or(0) / 8)
    }

    /// Set or clear the quota of a refusable class (custody, transit or
    /// bulk; messages are never refused, so quotas on them are invalid), or
    /// the shaping quota of [`Class::Foreign`] (evict first, never refuse;
    /// Layer 6 §3). Clearing `quota.transit` restores its derived default
    /// (Layer 7 §5.8), never "no cap".
    pub fn set_quota(&mut self, class: Class, quota: Option<u64>) -> Result<()> {
        if !class.has_quota() {
            return Err(StoreError::Invalid(
                "quotas apply to custody, transit, bulk and the foreign shaping quota",
            ));
        }
        let k = quota_key(class);
        match quota {
            Some(q) => set_u64(&self.conn, &k, q),
            None => super::del(&self.conn, &k),
        }
    }

    /// The pending-request quota (default [`DEFAULT_REQUEST_QUOTA`]).
    pub fn request_quota(&self) -> Result<u64> {
        Ok(get_u64(&self.conn, K_REQUEST_QUOTA)?.unwrap_or(DEFAULT_REQUEST_QUOTA))
    }

    /// Set the pending-request quota (`None` restores the default). Lowering
    /// it below the pending count refuses new requests; it never drops any.
    pub fn set_request_quota(&mut self, quota: Option<u64>) -> Result<()> {
        match quota {
            Some(q) => set_u64(&self.conn, K_REQUEST_QUOTA, q),
            None => super::del(&self.conn, K_REQUEST_QUOTA),
        }
    }

    /// Record that the user was offered a history export (§10.4 step 3
    /// gate; §21 `age`-based export).
    pub fn set_export_offered(&mut self, offered: bool) -> Result<()> {
        set_i64(&self.conn, K_EXPORT_OFFERED, i64::from(offered))
    }

    /// Whether an export was offered.
    pub fn export_offered(&self) -> Result<bool> {
        Ok(super::get_i64(&self.conn, K_EXPORT_OFFERED)? == Some(1))
    }

    /// `store_full` as of the last pruning pass (§10.4, §15 `GET /fabric`).
    pub fn store_full(&self) -> bool {
        self.store_full
    }

    /// Refusable classes above quota as of the last pass.
    pub fn full_classes(&self) -> &[Class] {
        &self.full_classes
    }

    /// One pruning pass with the configured watermarks.
    pub fn prune(&mut self, disk: DiskSpace) -> Result<PruneReport> {
        let w = self.watermarks()?;
        self.prune_with(disk, w)
    }

    /// One pruning pass with explicit watermarks.
    pub fn prune_with(&mut self, disk: DiskSpace, w: Watermarks) -> Result<PruneReport> {
        Ok(self.prune_inner(disk, w, None)?.0)
    }

    /// The pass. `extra = Some((class, bytes))` asks for room for an incoming
    /// item (`class = None`: an item no class quota covers, e.g. a message
    /// request); the bool answers whether it fits.
    ///
    /// **At rest** (no quota over counting the incoming item, live bytes
    /// with it not over the high watermark, free disk not under the floor)
    /// the pass prunes nothing, so it reads no candidate and opens no write
    /// transaction ([`Store::rest_pass`]). Otherwise each step list is read
    /// when the pass reaches it, for the class a quota step prunes or for
    /// every class, cut to the head that frees what is still needed
    /// ([`step_list`]): the same items in the same order as reading every
    /// list up front, because a removal only takes its own item out of a
    /// list and the order is total.
    pub(super) fn prune_inner(
        &mut self,
        disk: DiskSpace,
        w: Watermarks,
        extra: Option<(Option<Class>, u64)>,
    ) -> Result<(PruneReport, bool)> {
        if w.low > w.high {
            return Err(StoreError::Invalid("low watermark above high"));
        }
        let live_before = self.live_bytes()?;
        let incoming = extra.map_or(0, |e| e.1);
        let export_offered = self.export_offered()?;
        // The foreign shaping quota evicts first (Layer 6 §3), then the
        // refusable classes in `Class::ALL` order.
        let mut quotas = Vec::new();
        if let Some(q) = self.quota(Class::Foreign)? {
            quotas.push((Class::Foreign, q));
        }
        for class in Class::ALL.into_iter().filter(|c| c.is_refusable()) {
            if let Some(q) = self.quota(class)? {
                quotas.push((class, q));
            }
        }
        let extra_for = |class: Class| match extra {
            Some((Some(c), b)) if c == class => b,
            _ => 0,
        };
        if need_for(live_before, incoming, disk, w) == 0 {
            let mut over = false;
            for (class, quota) in &quotas {
                if class_bytes(&self.conn, *class)?.saturating_add(extra_for(*class)) > *quota {
                    over = true;
                    break;
                }
            }
            if !over {
                return self.rest_pass(disk, live_before);
            }
        }

        let mut pruned: Vec<PrunedItem> = Vec::new();
        let mut full_classes = Vec::new();
        let mut store_full = false;
        let mut export_offer_needed = false;
        let mut fits = true;
        let mut triggered = false;

        self.with_tx(Durability::Normal, |tx| {
            let now = current_local_seq(tx)?;

            // Quotas: per class with a quota, steps 1–2b restricted to it.
            for (class, quota) in &quotas {
                let extra_here = extra_for(*class);
                let mut used = class_bytes(tx, *class)?;
                for (step, _) in STEPS {
                    if used.saturating_add(extra_here) <= *quota {
                        break;
                    }
                    // Bytes still to free; `None` when even an empty class
                    // stays over (the incoming item alone is over): the
                    // pass then takes every candidate of the class.
                    let need = used
                        .checked_add(extra_here)
                        .filter(|_| extra_here <= *quota)
                        .map(|u| u - *quota);
                    for cand in step_list(tx, step, Only::Class(*class), need, now)? {
                        if used.saturating_add(extra_here) <= *quota {
                            break;
                        }
                        remove(tx, &cand.item)?;
                        used = used.saturating_sub(cand.bytes);
                        pruned.push(PrunedItem {
                            step: PruneStep::Quota(*class),
                            item: cand.item,
                            bytes: cand.bytes,
                        });
                    }
                }
                // A shaping-only quota (Foreign) never refuses.
                if used > *quota && class.is_refusable() {
                    full_classes.push(*class);
                }
                // Only the incoming item's own class can refuse it.
                if extra_here > 0 && used.saturating_add(extra_here) > *quota {
                    fits = false;
                }
            }

            // Global trigger: live bytes over the high watermark (counting
            // the incoming item) or free disk under the 1 % floor.
            let freed_by_quota: u64 = pruned.iter().map(|p| p.bytes).sum();
            let live = live_before.saturating_sub(freed_by_quota);
            let need_base = need_for(live, 0, disk, w);
            let need_all = need_for(live, incoming, disk, w);
            triggered = need_base > 0 || live_before > w.high || disk.below_floor();
            let mut freed = 0u64;
            if need_all > 0 {
                for (step, label) in STEPS {
                    if freed >= need_all {
                        break;
                    }
                    let need = Some(need_all - freed);
                    for cand in step_list(tx, step, Only::All, need, now)? {
                        if freed >= need_all {
                            break;
                        }
                        remove(tx, &cand.item)?;
                        freed = freed.saturating_add(cand.bytes);
                        pruned.push(PrunedItem {
                            step: label,
                            item: cand.item,
                            bytes: cand.bytes,
                        });
                    }
                }
                // Hard capacity: steps 1–2b exhausted and the store itself is
                // still short. Own history is pruned only for that pressure,
                // never to make room for incoming custody or bulk.
                if freed < need_base {
                    let hist = history(tx)?;
                    if hist.is_empty() {
                        // Nothing to offer for export: nothing more to free.
                    } else if export_offered {
                        for cand in hist {
                            if freed >= need_base {
                                break;
                            }
                            remove(tx, &cand.item)?;
                            freed = freed.saturating_add(cand.bytes);
                            pruned.push(PrunedItem {
                                step: PruneStep::History,
                                item: cand.item,
                                bytes: cand.bytes,
                            });
                        }
                    } else {
                        export_offer_needed = true;
                    }
                }
                store_full = freed < need_base;
                if freed < need_all {
                    fits = false;
                }
            }
            if pruned
                .iter()
                .any(|p| matches!(p.item, PrunedRef::Foreign(_) | PrunedRef::RelayCopy(_)))
            {
                clamp_unread(tx)?;
            }
            // Persist the outcome: a full store is still full after a restart.
            save_capacity(tx, store_full, &full_classes, disk)
        })?;

        // Blob files after the rows are committed.
        for p in &pruned {
            if let PrunedRef::Blob(h) = p.item {
                self.blobs.remove(&h)?;
            }
        }
        self.conn.execute_batch("PRAGMA incremental_vacuum;")?;
        let live_after = self.live_bytes()?;
        self.store_full = store_full;
        self.full_classes = full_classes.clone();
        self.last_disk = Some(disk);
        Ok((
            PruneReport {
                triggered,
                live_before,
                live_after,
                pruned,
                store_full,
                export_offer_needed,
                full_classes,
            },
            fits && !store_full,
        ))
    }

    /// A pass at rest ([`Store::prune_inner`]): nothing is over, so the
    /// report is the one a full pass gives (not triggered, nothing pruned,
    /// not full, no class full, the item fits) and the persisted outcome is
    /// `store_full = 0`, no full class and these disk figures. Those
    /// settings are written only when they differ from what is stored (a
    /// change from the last pass: a store leaving `store_full`, new disk
    /// figures), and the freelist is vacuumed only when it holds pages, so
    /// a store at rest with the same figures reads a few settings by
    /// primary key and the live-bytes inputs and writes nothing. `live_after`
    /// is measured again whenever either ran, as a full pass measures it.
    fn rest_pass(&mut self, disk: DiskSpace, live_before: u64) -> Result<(PruneReport, bool)> {
        let wrote = !capacity_saved(&self.conn, false, &[], disk)?;
        if wrote {
            self.with_tx(Durability::Normal, |tx| save_capacity(tx, false, &[], disk))?;
        }
        let vacuumed = super::pragma_int(&self.conn, "freelist_count")? > 0;
        if vacuumed {
            self.conn.execute_batch("PRAGMA incremental_vacuum;")?;
        }
        let live_after = if wrote || vacuumed {
            self.live_bytes()?
        } else {
            live_before
        };
        self.store_full = false;
        self.full_classes = Vec::new();
        self.last_disk = Some(disk);
        Ok((
            PruneReport {
                triggered: false,
                live_before,
                live_after,
                pruned: Vec::new(),
                store_full: false,
                export_offer_needed: false,
                full_classes: Vec::new(),
            },
            true,
        ))
    }

    /// Admission for new custody / new bulk of `bytes` in `class` (§10.4):
    /// refuses while `store_full`, or when the item cannot be made room for
    /// (pruning in the normative order first). Messages never come here.
    pub(crate) fn admit(&mut self, class: Class, bytes: u64) -> Result<bool> {
        self.admit_inner(Some(class), bytes)
    }

    /// Admission for an item no class quota covers (a message request):
    /// the same capacity rule as [`Store::admit`].
    pub(crate) fn admit_unclassed(&mut self, bytes: u64) -> Result<bool> {
        self.admit_inner(None, bytes)
    }

    fn admit_inner(&mut self, class: Option<Class>, bytes: u64) -> Result<bool> {
        let disk = self.last_disk.unwrap_or(DiskSpace::UNKNOWN);
        let w = self.watermarks()?;
        let quota_hit = match class {
            Some(class) => match self.quota(class)? {
                Some(q) => self.class_bytes(class)?.saturating_add(bytes) > q,
                None => false,
            },
            None => false,
        };
        let over = self.live_bytes()?.saturating_add(bytes) > w.high || disk.below_floor();
        if !self.store_full && !quota_hit && !over {
            return Ok(true);
        }
        let (_, fits) = self.prune_inner(disk, w, Some((class, bytes)))?;
        Ok(fits)
    }

    /// At open: the capacity state the last pass persisted.
    pub(crate) fn load_capacity(&mut self) -> Result<()> {
        self.store_full = super::get_i64(&self.conn, K_STORE_FULL)? == Some(1);
        let mask = get_u64(&self.conn, K_FULL_CLASSES)?.unwrap_or(0);
        self.full_classes = Class::ALL
            .into_iter()
            .filter(|c| mask & (1u64 << (*c as u8)) != 0)
            .collect();
        self.last_disk = match (
            get_u64(&self.conn, K_DISK_FREE)?,
            get_u64(&self.conn, K_DISK_TOTAL)?,
        ) {
            (Some(free), Some(total)) => Some(DiskSpace { free, total }),
            _ => None,
        };
        Ok(())
    }
}

/// After foreign rows were pruned: no conversation counts more unread rows
/// than it still holds inbound.
fn clamp_unread(c: &Connection) -> Result<()> {
    c.execute(
        "UPDATE foreign_conversations
         SET unread = min(unread, (SELECT count(*) FROM foreign_messages m
                                   WHERE m.conv = foreign_conversations.id AND m.dir = 'in'))
         WHERE unread > 0",
        [],
    )?;
    Ok(())
}

/// The settings one pass's capacity outcome is persisted as: `store_full`,
/// the mask of full classes and the disk figures (`None`: the key absent,
/// as for [`DiskSpace::UNKNOWN`]; saturated, since settings hold i64 and a
/// figure above `i64::MAX` is "plenty").
fn capacity_record(
    store_full: bool,
    full_classes: &[Class],
    disk: DiskSpace,
) -> [(&'static str, Option<i64>); 4] {
    let mask = full_classes
        .iter()
        .fold(0i64, |m, cl| m | (1i64 << (*cl as u8)));
    let (free, total) = if disk == DiskSpace::UNKNOWN {
        (None, None)
    } else {
        let sat = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
        (Some(sat(disk.free)), Some(sat(disk.total)))
    };
    [
        (K_STORE_FULL, Some(i64::from(store_full))),
        (K_FULL_CLASSES, Some(mask)),
        (K_DISK_FREE, free),
        (K_DISK_TOTAL, total),
    ]
}

/// Persist one pass's capacity outcome (inside the pass's transaction).
fn save_capacity(
    c: &Connection,
    store_full: bool,
    full_classes: &[Class],
    disk: DiskSpace,
) -> Result<()> {
    for (key, value) in capacity_record(store_full, full_classes, disk) {
        match value {
            Some(v) => set_i64(c, key, v)?,
            None => super::del(c, key)?,
        }
    }
    Ok(())
}

/// Whether the settings already hold exactly what [`save_capacity`] would
/// write (an integer per present key, no row per absent one): primary-key
/// reads, no write.
fn capacity_saved(
    c: &Connection,
    store_full: bool,
    full_classes: &[Class],
    disk: DiskSpace,
) -> Result<bool> {
    for (key, want) in capacity_record(store_full, full_classes, disk) {
        let got: Option<Value> = c
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        let same = match (got, want) {
            (None, None) => true,
            (Some(Value::Integer(g)), Some(w)) => g == w,
            _ => false,
        };
        if !same {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The pass as it was before the at-rest path and the lazy step lists:
/// every step list read up front for every class inside the write
/// transaction, a quota step skipping the other classes' candidates and
/// every step skipping what an earlier one removed. Kept only as the
/// oracle the tests hold [`Store::prune_inner`] to (byte-identical
/// reports, flags and rows).
#[cfg(test)]
impl Store {
    pub(super) fn prune_eager(
        &mut self,
        disk: DiskSpace,
        w: Watermarks,
        extra: Option<(Option<Class>, u64)>,
    ) -> Result<(PruneReport, bool)> {
        if w.low > w.high {
            return Err(StoreError::Invalid("low watermark above high"));
        }
        let live_before = self.live_bytes()?;
        let incoming = extra.map_or(0, |e| e.1);
        let export_offered = self.export_offered()?;
        let mut quotas = Vec::new();
        if let Some(q) = self.quota(Class::Foreign)? {
            quotas.push((Class::Foreign, q));
        }
        for class in Class::ALL.into_iter().filter(|c| c.is_refusable()) {
            if let Some(q) = self.quota(class)? {
                quotas.push((class, q));
            }
        }
        let mut pruned: Vec<PrunedItem> = Vec::new();
        let mut full_classes = Vec::new();
        let mut store_full = false;
        let mut export_offer_needed = false;
        let mut fits = true;
        let mut triggered = false;
        self.with_tx(Durability::Normal, |tx| {
            let now = current_local_seq(tx)?;
            let step1 = step_list(tx, Step::One, Only::All, None, now)?;
            let step2 = step_list(tx, Step::Two, Only::All, None, now)?;
            let step2a = step_list(tx, Step::TwoA, Only::All, None, now)?;
            let step2b = step_list(tx, Step::TwoB, Only::All, None, now)?;
            let mut gone: Vec<PrunedRef> = Vec::new();
            for (class, quota) in &quotas {
                let extra_here = match extra {
                    Some((Some(c), b)) if c == *class => b,
                    _ => 0,
                };
                let mut used = class_bytes(tx, *class)?;
                if used.saturating_add(extra_here) <= *quota {
                    continue;
                }
                for cand in step1
                    .iter()
                    .chain(step2.iter())
                    .chain(step2a.iter())
                    .chain(step2b.iter())
                {
                    if used.saturating_add(extra_here) <= *quota {
                        break;
                    }
                    if cand.class != *class || gone.contains(&cand.item) {
                        continue;
                    }
                    remove(tx, &cand.item)?;
                    gone.push(cand.item);
                    used = used.saturating_sub(cand.bytes);
                    pruned.push(PrunedItem {
                        step: PruneStep::Quota(*class),
                        item: cand.item,
                        bytes: cand.bytes,
                    });
                }
                if used > *quota && class.is_refusable() {
                    full_classes.push(*class);
                }
                if extra_here > 0 && used.saturating_add(extra_here) > *quota {
                    fits = false;
                }
            }
            let freed_by_quota: u64 = pruned.iter().map(|p| p.bytes).sum();
            let live = live_before.saturating_sub(freed_by_quota);
            let need_base = need_for(live, 0, disk, w);
            let need_all = need_for(live, incoming, disk, w);
            triggered = need_base > 0 || live_before > w.high || disk.below_floor();
            let mut freed = 0u64;
            if need_all > 0 {
                for (step, list) in [
                    (PruneStep::Evictable, &step1),
                    (PruneStep::Superseded, &step2),
                    (PruneStep::ForeignInbound, &step2a),
                    (PruneStep::RelayBacklog, &step2b),
                ] {
                    for cand in list.iter() {
                        if freed >= need_all {
                            break;
                        }
                        if gone.contains(&cand.item) {
                            continue;
                        }
                        remove(tx, &cand.item)?;
                        gone.push(cand.item);
                        freed = freed.saturating_add(cand.bytes);
                        pruned.push(PrunedItem {
                            step,
                            item: cand.item,
                            bytes: cand.bytes,
                        });
                    }
                }
                if freed < need_base {
                    let hist = history(tx)?;
                    if hist.is_empty() {
                    } else if export_offered {
                        for cand in hist {
                            if freed >= need_base {
                                break;
                            }
                            remove(tx, &cand.item)?;
                            freed = freed.saturating_add(cand.bytes);
                            pruned.push(PrunedItem {
                                step: PruneStep::History,
                                item: cand.item,
                                bytes: cand.bytes,
                            });
                        }
                    } else {
                        export_offer_needed = true;
                    }
                }
                store_full = freed < need_base;
                if freed < need_all {
                    fits = false;
                }
            }
            if pruned
                .iter()
                .any(|p| matches!(p.item, PrunedRef::Foreign(_) | PrunedRef::RelayCopy(_)))
            {
                clamp_unread(tx)?;
            }
            save_capacity(tx, store_full, &full_classes, disk)
        })?;
        for p in &pruned {
            if let PrunedRef::Blob(h) = p.item {
                self.blobs.remove(&h)?;
            }
        }
        self.conn.execute_batch("PRAGMA incremental_vacuum;")?;
        let live_after = self.live_bytes()?;
        self.store_full = store_full;
        self.full_classes = full_classes.clone();
        self.last_disk = Some(disk);
        Ok((
            PruneReport {
                triggered,
                live_before,
                live_after,
                pruned,
                store_full,
                export_offer_needed,
                full_classes,
            },
            fits && !store_full,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic stream of candidates: sizes, ages and items spread
    /// so that ties in weight and in age both occur.
    fn cands(n: u64, seed: u64) -> Vec<Candidate> {
        let mut x = seed | 1;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        (0..n)
            .map(|i| {
                let r = next();
                let item = match r % 3 {
                    0 => PrunedRef::Custody(i),
                    1 => PrunedRef::Message(i as i64),
                    _ => PrunedRef::Foreign(i as i64),
                };
                Candidate {
                    item,
                    bytes: 64 + (next() % 5) * 100,
                    local_seq: next() % 40,
                    class: if r % 2 == 0 {
                        Class::Custody
                    } else {
                        Class::Transit
                    },
                }
            })
            .collect()
    }

    /// The pre-M1c list: every candidate, sorted by weight.
    fn sorted(v: &[Candidate], now: u64) -> Vec<Candidate> {
        let mut r: Vec<Ranked> = v.iter().map(|c| Ranked::new(*c, now)).collect();
        r.sort();
        r.into_iter().map(|r| r.cand).collect()
    }

    /// What a pass takes from a list while it is short of `need` bytes.
    fn taken(list: &[Candidate], need: Option<u64>) -> Vec<PrunedRef> {
        let mut freed = 0u128;
        let mut out = Vec::new();
        for c in list {
            if need.is_some_and(|n| freed >= u128::from(n)) {
                break;
            }
            freed += u128::from(c.bytes);
            out.push(c.item);
        }
        out
    }

    /// §10.4: the head a step list is cut to is exactly what a pass takes
    /// from the whole sorted list, for every need (from one item's worth to
    /// more than the list holds), every class filter and any read order;
    /// with no need it is the whole sorted list of the class.
    #[test]
    fn a_head_holds_exactly_what_the_pass_takes_from_the_sorted_list() {
        let now = 50;
        for seed in [1u64, 7, 99, 12345] {
            let all = cands(200, seed);
            let total: u64 = all.iter().map(|c| c.bytes).sum();
            for only in [
                Only::All,
                Only::Class(Class::Custody),
                Only::Class(Class::Transit),
                Only::Class(Class::Kb),
            ] {
                let mine: Vec<Candidate> =
                    all.iter().copied().filter(|c| only.has(c.class)).collect();
                let want = sorted(&mine, now);
                for need in [
                    None,
                    Some(1),
                    Some(64),
                    Some(650),
                    Some(total / 3),
                    Some(total + 1),
                ] {
                    for reversed in [false, true] {
                        let mut h = Head::new(only, need, now);
                        let mut feed = all.clone();
                        if reversed {
                            feed.reverse();
                        }
                        for c in feed {
                            h.push(c);
                        }
                        let got = h.into_vec();
                        assert_eq!(
                            got.iter().map(|c| c.item).collect::<Vec<_>>(),
                            taken(&want, need),
                            "seed {seed}, {only:?}, need {need:?}"
                        );
                        if need.is_none() {
                            assert_eq!(got.len(), want.len());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn need_for_counts_the_watermark_and_the_floor() {
        let w = Watermarks { high: 100, low: 80 };
        let ok = DiskSpace {
            free: 50,
            total: 1000,
        };
        assert_eq!(need_for(100, 0, ok, w), 0);
        assert_eq!(need_for(100, 1, ok, w), 21);
        let low = DiskSpace {
            free: 5,
            total: 1000,
        };
        assert_eq!(need_for(0, 0, low, w), 5);
        assert_eq!(need_for(0, 7, low, w), 12);
        assert_eq!(need_for(200, 0, low, w), 120);
        assert_eq!(need_for(u64::MAX, 1, DiskSpace::UNKNOWN, w), u64::MAX - 80);
    }
}
