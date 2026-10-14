//! §10.4 — pruning by capacity only (never by policy, never by clock).
//!
//! **Trigger:** live bytes (DB pages minus freelist, plus present blob files)
//! above the high watermark, **or** free disk below a 1 % floor. The caller
//! supplies the disk figures ([`DiskSpace`]; `mesh111` has no `statvfs`).
//! Pruning then frees down to the low watermark (and back above the floor).
//!
//! **Normative order:**
//! 1. transit custody that is `committed`, public chunks that are
//!    re-fetchable (and delivered transit copies) — weight = size × age, old
//!    and large first; age = local-sequence distance;
//! 2. superseded release/KB objects (below the Layer 1 floor), fully fetched
//!    deltas — marked by their consumer ([`Store::blob_mark_superseded`]);
//! 3. own conversation history oldest-first — **only at hard capacity**
//!    (the store itself still short after 1–2; never to make room for an
//!    incoming custody or bulk item, which is refused instead) **and only
//!    after an export offer** ([`Store::set_export_offered`]); otherwise
//!    `export_offer_needed`;
//! 4. **never** undelivered outbound, uncommitted custody, pending requests,
//!    pinned items or contacts — instead `store_full` is raised and new
//!    custody, new bulk and new message requests are refused (messages are
//!    never refused).
//!
//! **Per-class quotas** (settings, configurable) shape capacity for the
//! refusable classes (custody and bulk): a class above its quota loses its
//! step-1/2 items first; if still above, new items of that class are refused.
//! Pending message requests have a count quota ([`Store::request_quota`],
//! default [`DEFAULT_REQUEST_QUOTA`]).
//!
//! **Persistence:** `store_full`, the classes over quota and the last disk
//! figures are written to `settings` by every pass and read back at open, so
//! a full store stays full across a restart until a pass with room clears it.

use rusqlite::{params, Connection};

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
    /// Step 3: own conversation history (after an export offer).
    History,
}

/// What was removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrunedRef {
    Custody(u64),
    Blob(BlobHash),
    Message(i64),
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

#[derive(Debug, Clone, Copy)]
struct Candidate {
    item: PrunedRef,
    bytes: u64,
    local_seq: u64,
    class: Class,
}

/// `size × age` (u128, no overflow); larger first, then older, then stable.
fn sort_by_weight(v: &mut [Candidate], now: u64) {
    v.sort_by(|a, b| {
        let wa = u128::from(a.bytes) * u128::from(now.saturating_sub(a.local_seq) + 1);
        let wb = u128::from(b.bytes) * u128::from(now.saturating_sub(b.local_seq) + 1);
        wb.cmp(&wa)
            .then(a.local_seq.cmp(&b.local_seq))
            .then(key(&a.item).cmp(&key(&b.item)))
    });
}

fn key(r: &PrunedRef) -> (u8, i128, [u8; 32]) {
    match r {
        PrunedRef::Custody(s) => (0, i128::from(*s), [0; 32]),
        PrunedRef::Blob(h) => (1, 0, h.0),
        PrunedRef::Message(id) => (2, i128::from(*id), [0; 32]),
    }
}

/// Step 1 candidates.
fn evictable(c: &Connection) -> Result<Vec<Candidate>> {
    let mut out = Vec::new();
    {
        let mut st = c.prepare(
            "SELECT custody_seq, length(bytes), local_seq FROM custody WHERE committed = 1",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for r in rows {
            let (seq, len, local) = r?;
            out.push(Candidate {
                item: PrunedRef::Custody(to_u64(seq, "custody_seq")?),
                bytes: to_u64(len, "custody.len")? + ROW_OVERHEAD,
                local_seq: to_u64(local, "custody.local_seq")?,
                class: Class::Custody,
            });
        }
    }
    out.extend(blob_candidates(
        c,
        "public = 1 AND refetchable = 1 AND present = 1 AND pinned = 0",
    )?);
    out.extend(message_candidates(
        c,
        "dir = 2 AND state = 3 AND pinned = 0",
        "local_seq",
    )?);
    Ok(out)
}

/// Step 2 candidates.
fn superseded(c: &Connection) -> Result<Vec<Candidate>> {
    blob_candidates(
        c,
        "superseded = 1 AND present = 1 AND pinned = 0 AND NOT (public = 1 AND refetchable = 1)",
    )
}

/// Step 3 candidates, oldest first: delivered own/inbound history, never
/// pinned, never with outbox fragments, never from a pending request.
fn history(c: &Connection) -> Result<Vec<Candidate>> {
    message_candidates(
        c,
        "dir IN (0, 1) AND state = 3 AND pinned = 0
         AND NOT EXISTS (SELECT 1 FROM outbox o WHERE o.message_id = messages.id)
         AND peer NOT IN (SELECT node_id FROM requests)
         AND peer NOT IN (SELECT node_id FROM contacts WHERE request = 1)",
        "local_seq",
    )
}

fn blob_candidates(c: &Connection, filter: &str) -> Result<Vec<Candidate>> {
    let mut st = c.prepare(&format!(
        "SELECT hash, disk_bytes, local_seq, class FROM blobs WHERE {filter}"
    ))?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, Vec<u8>>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (h, disk, local, class) = r?;
        out.push(Candidate {
            item: PrunedRef::Blob(BlobHash(id32(h, "blobs.hash")?)),
            bytes: to_u64(disk, "blobs.disk_bytes")? + ROW_OVERHEAD,
            local_seq: to_u64(local, "blobs.local_seq")?,
            class: Class::from_i64(class)?,
        });
    }
    Ok(out)
}

fn message_candidates(c: &Connection, filter: &str, order: &str) -> Result<Vec<Candidate>> {
    let mut st = c.prepare(&format!(
        "SELECT id, coalesce(length(body), 0) + coalesce(length(blob_refs), 0), local_seq, class
         FROM messages WHERE {filter} ORDER BY {order}"
    ))?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (id, len, local, class) = r?;
        out.push(Candidate {
            item: PrunedRef::Message(id),
            bytes: to_u64(len, "messages.len")? + ROW_OVERHEAD,
            local_seq: to_u64(local, "messages.local_seq")?,
            class: Class::from_i64(class)?,
        });
    }
    Ok(out)
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

/// Bytes a class accounts for.
pub(crate) fn class_bytes(c: &Connection, class: Class) -> Result<u64> {
    let v: i64 = if class == Class::Custody {
        c.query_row(
            "SELECT coalesce(sum(length(bytes) + ?1), 0) FROM custody",
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
    pub fn quota(&self, class: Class) -> Result<Option<u64>> {
        get_u64(&self.conn, &quota_key(class))
    }

    /// Set or clear the quota of a refusable class (custody or bulk;
    /// messages are never refused, so quotas on them are invalid).
    pub fn set_quota(&mut self, class: Class, quota: Option<u64>) -> Result<()> {
        if !class.is_refusable() {
            return Err(StoreError::Invalid(
                "quotas apply to custody and bulk classes",
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
    fn prune_inner(
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
            let mut step1 = evictable(tx)?;
            sort_by_weight(&mut step1, now);
            let mut step2 = superseded(tx)?;
            sort_by_weight(&mut step2, now);
            let mut gone: Vec<PrunedRef> = Vec::new();

            // Quotas: per refusable class, steps 1–2 restricted to it.
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
                    .map(|c| (PruneStep::Quota(*class), c))
                    .chain(step2.iter().map(|c| (PruneStep::Quota(*class), c)))
                {
                    if used.saturating_add(extra_here) <= *quota {
                        break;
                    }
                    let (step, cand) = cand;
                    if cand.class != *class || gone.contains(&cand.item) {
                        continue;
                    }
                    remove(tx, &cand.item)?;
                    gone.push(cand.item);
                    used = used.saturating_sub(cand.bytes);
                    pruned.push(PrunedItem {
                        step,
                        item: cand.item,
                        bytes: cand.bytes,
                    });
                }
                if used > *quota {
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
            let need_for = |extra: u64| -> u64 {
                let mut need = 0;
                if live.saturating_add(extra) > w.high {
                    need = live.saturating_add(extra) - w.low;
                }
                if disk.below_floor() {
                    need = need.max((disk.floor() - disk.free).saturating_add(extra));
                }
                need
            };
            let need_base = need_for(0);
            let need_all = need_for(incoming);
            triggered = need_base > 0 || live_before > w.high || disk.below_floor();
            let mut freed = 0u64;
            if need_all > 0 {
                for (step, list) in [
                    (PruneStep::Evictable, &step1),
                    (PruneStep::Superseded, &step2),
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
                // Hard capacity: steps 1–2 exhausted and the store itself is
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

/// Persist one pass's capacity outcome (inside the pass's transaction).
fn save_capacity(
    c: &Connection,
    store_full: bool,
    full_classes: &[Class],
    disk: DiskSpace,
) -> Result<()> {
    set_i64(c, K_STORE_FULL, i64::from(store_full))?;
    let mask = full_classes
        .iter()
        .fold(0u64, |m, cl| m | (1u64 << (*cl as u8)));
    set_u64(c, K_FULL_CLASSES, mask)?;
    if disk == DiskSpace::UNKNOWN {
        super::del(c, K_DISK_FREE)?;
        super::del(c, K_DISK_TOTAL)
    } else {
        // Saturate: settings hold i64; a figure above i64::MAX is "plenty".
        set_u64(c, K_DISK_FREE, disk.free.min(i64::MAX as u64))?;
        set_u64(c, K_DISK_TOTAL, disk.total.min(i64::MAX as u64))
    }
}
