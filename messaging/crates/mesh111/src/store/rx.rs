//! §4 receiver state — `rx_state` keyed `(peer, stream, epoch)`: a
//! contiguous delivered prefix, a bounded list of delivered ranges above it,
//! and partial-reassembly bitmaps (§5, for the §6.1 ACK partials).
//!
//! Rules implemented (all after `pair_mac`, §18.1 — this is the replay
//! authority for `msg_seq` kinds, L2 amendment A11):
//!
//! - **Below a prefix nothing is ever forgotten:** a duplicate answers
//!   [`Offer::BelowPrefix`] / [`Offer::Duplicate`] — re-ACK, never redeliver.
//! - **Bounded ranges:** at most [`MAX_RANGES`] ranges per epoch. A packet
//!   that would open one more isolated range is [`Offer::Deferred`]: not
//!   recorded, not delivered, not ACKed; the sender keeps its copy and
//!   retransmits, and it is accepted once the gap below closes. Forgetting a
//!   range instead would redeliver — never done.
//! - **Frozen epochs:** a packet whose epoch exceeds the newest opens a new
//!   entry; older entries are frozen but kept and still deliver (custody and
//!   multi-path legitimately deliver an earlier epoch after a later one).
//!   The newest [`RX_EPOCHS_KEPT`] epochs per `(peer, stream)` are kept; an
//!   evicted epoch raises the `(peer, stream)` floor, and anything at or
//!   below the floor (or older than all kept epochs once 4 are kept) is
//!   [`Offer::StaleEpoch`] — dropped, so an evicted epoch can never be
//!   re-opened and redelivered.
//! - **Fragments (§5 Tier 1):** each fragment's bytes are written to
//!   `rx_frags` in the **same** FULL transaction that sets its bit, so a
//!   fragment the ACK SACKs (and the sender therefore never resends) survives
//!   a crash. Reassembly reads them back ([`Store::fragments`],
//!   [`Store::reassemble`]); [`Store::complete_partials`] lists the messages
//!   whose fragments are all in but which are not delivered yet (resume
//!   after a restart). The bytes go with their partial: delivery of `n`,
//!   oldest-first eviction ([`MAX_PARTIALS`] per epoch) and epoch eviction.
//! - `n` runs 1 ..= 2^32 − 1 (see `store::tx`); `n = 0` is invalid input.

use rusqlite::{params, Connection, OptionalExtension};

use super::rows::{insert_message, Dir, MsgState, NewMessage};
use super::tx::Seq;
use super::{id32, to_i64, to_u64, Durability, Result, Store, StoreError, N_MAX};

/// Epochs kept per `(peer, stream)` (§4).
pub const RX_EPOCHS_KEPT: usize = 4;
/// Delivered ranges kept above the prefix per epoch (§4 "bounded range
/// list ≈ 24–48 B"; matches the §6.1 `n_ranges ≤ 8`).
pub const MAX_RANGES: usize = 8;
/// Partial reassemblies tracked per epoch; the oldest is evicted (§5).
pub const MAX_PARTIALS: usize = 16;
/// Tier 1 fragment bound (§5).
pub const MAX_FRAGS: u8 = 16;
/// Largest fragment kept for reassembly: one `tcp-lan` frame (§8.1). With
/// [`MAX_FRAGS`] this bounds one partial message to 16 × 65,535 B.
pub const MAX_FRAG_BYTES: usize = 65_535;

const RANGE_LEN: usize = 8; // start u32 LE ‖ len u32 LE
const PARTIAL_LEN: usize = 7; // n u32 LE ‖ frag_count u8 ‖ bitmap u16 LE

/// The verdict on one `(peer, stream, epoch, n)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// First sight: recorded; deliver and ACK.
    New,
    /// Already delivered (in a range above the prefix): re-ACK only.
    Duplicate,
    /// At or below the delivered prefix: re-ACK only.
    BelowPrefix,
    /// The epoch is older than every kept epoch or was evicted: drop.
    StaleEpoch,
    /// The range list is full: not recorded, not ACKed; delivered later.
    Deferred,
}

impl Offer {
    /// Deliver the content (and ACK).
    pub fn deliver(self) -> bool {
        self == Offer::New
    }

    /// Re-ACK without redelivering (§6.1 "duplicates are re-ACKed").
    pub fn re_ack(self) -> bool {
        matches!(self, Offer::Duplicate | Offer::BelowPrefix)
    }
}

/// The verdict on one fragment `(…, n, frag_idx, frag_count)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragOffer {
    /// Recorded in the partial bitmap; `complete` once every fragment is in
    /// (then reassemble and call [`Store::receive_message`]).
    Stored { complete: bool },
    /// This fragment was already recorded: re-ACK (SACK bitmap).
    DuplicateFragment,
    /// The whole message is already delivered: re-ACK only.
    Delivered(Offer),
    /// Stale epoch: drop.
    StaleEpoch,
    /// The message could not be recorded yet (range list full): drop now.
    Deferred,
    /// `frag_count` differs from the one recorded for this `n`: drop.
    Mismatch,
}

/// One in-flight message's fragments (§6.1 `[epoch, n, frag_bitmap]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Partial {
    pub n: u32,
    pub frag_count: u8,
    pub bitmap: u16,
}

impl Partial {
    fn full_mask(&self) -> u16 {
        if self.frag_count >= 16 {
            u16::MAX
        } else {
            (1u16 << self.frag_count) - 1
        }
    }

    /// Every fragment received.
    pub fn is_complete(&self) -> bool {
        self.bitmap == self.full_mask()
    }
}

/// A message whose fragments are all stored but which is not delivered yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartialRef {
    pub peer: [u8; 32],
    pub stream: u64,
    pub epoch: u64,
    pub n: u64,
}

/// Receive state of one `(peer, stream, epoch)` — what an ACK reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RxEpoch {
    pub epoch: u64,
    /// Every `n` in `1..=prefix` is delivered (`0` = none yet).
    pub prefix: u32,
    /// Delivered `(start, len)` ranges above `prefix + 1`, ascending,
    /// disjoint and non-adjacent.
    pub ranges: Vec<(u32, u32)>,
    /// Partial reassemblies, oldest first.
    pub partials: Vec<Partial>,
}

impl RxEpoch {
    fn new(epoch: u64) -> Self {
        RxEpoch {
            epoch,
            prefix: 0,
            ranges: Vec::new(),
            partials: Vec::new(),
        }
    }

    fn end(r: (u32, u32)) -> u64 {
        u64::from(r.0) + u64::from(r.1) - 1
    }

    /// `Some` when `n` is already delivered.
    fn seen(&self, n: u32) -> Option<Offer> {
        if n <= self.prefix {
            return Some(Offer::BelowPrefix);
        }
        let i = self.ranges.partition_point(|r| r.0 <= n);
        if i > 0 && u64::from(n) <= Self::end(self.ranges[i - 1]) {
            return Some(Offer::Duplicate);
        }
        None
    }

    /// Whether recording `n` (not yet seen) fits the range bound.
    fn can_insert(&self, n: u32) -> bool {
        if u64::from(n) == u64::from(self.prefix) + 1 || self.ranges.len() < MAX_RANGES {
            return true;
        }
        let i = self.ranges.partition_point(|r| r.0 < n);
        let left = i > 0 && Self::end(self.ranges[i - 1]) + 1 == u64::from(n);
        let right = i < self.ranges.len() && u64::from(self.ranges[i].0) == u64::from(n) + 1;
        left || right
    }

    /// Record `n` (precondition: `!seen(n) && can_insert(n)`).
    fn insert(&mut self, n: u32) {
        if u64::from(n) == u64::from(self.prefix) + 1 {
            self.prefix = n;
            if let Some(&first) = self.ranges.first() {
                if u64::from(first.0) == u64::from(self.prefix) + 1 {
                    // Bounded: `end` of a stored range never exceeds N_MAX.
                    self.prefix = Self::end(first) as u32;
                    self.ranges.remove(0);
                }
            }
        } else {
            let i = self.ranges.partition_point(|r| r.0 < n);
            let left = i > 0 && Self::end(self.ranges[i - 1]) + 1 == u64::from(n);
            let right = i < self.ranges.len() && u64::from(self.ranges[i].0) == u64::from(n) + 1;
            match (left, right) {
                (true, true) => {
                    let r = self.ranges.remove(i);
                    self.ranges[i - 1].1 += 1 + r.1;
                }
                (true, false) => self.ranges[i - 1].1 += 1,
                (false, true) => {
                    self.ranges[i].0 = n;
                    self.ranges[i].1 += 1;
                }
                (false, false) => self.ranges.insert(i, (n, 1)),
            }
        }
        self.partials.retain(|p| p.n != n);
    }

    fn encode_ranges(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.ranges.len() * RANGE_LEN);
        for (s, l) in &self.ranges {
            out.extend_from_slice(&s.to_le_bytes());
            out.extend_from_slice(&l.to_le_bytes());
        }
        out
    }

    fn encode_partials(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.partials.len() * PARTIAL_LEN);
        for p in &self.partials {
            out.extend_from_slice(&p.n.to_le_bytes());
            out.push(p.frag_count);
            out.extend_from_slice(&p.bitmap.to_le_bytes());
        }
        out
    }

    /// Decode and validate a stored row (sizes checked before parse; every
    /// invariant re-checked — a bad row is `Corrupt`, never trusted).
    fn decode(epoch: u64, prefix: i64, ranges: &[u8], partials: &[u8]) -> Result<RxEpoch> {
        let prefix = u32::try_from(prefix).map_err(|_| StoreError::Corrupt("rx_state.prefix"))?;
        if !ranges.len().is_multiple_of(RANGE_LEN) || ranges.len() > MAX_RANGES * RANGE_LEN {
            return Err(StoreError::Corrupt("rx_state.ranges"));
        }
        if !partials.len().is_multiple_of(PARTIAL_LEN)
            || partials.len() > MAX_PARTIALS * PARTIAL_LEN
        {
            return Err(StoreError::Corrupt("rx_state.partials"));
        }
        let mut e = RxEpoch::new(epoch);
        e.prefix = prefix;
        let mut floor = u64::from(prefix) + 1; // next start must exceed this
        for c in ranges.as_chunks::<RANGE_LEN>().0 {
            let s = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            let l = u32::from_le_bytes([c[4], c[5], c[6], c[7]]);
            if l == 0 || u64::from(s) <= floor || u64::from(s) + u64::from(l) - 1 > N_MAX {
                return Err(StoreError::Corrupt("rx_state.ranges"));
            }
            floor = u64::from(s) + u64::from(l); // end + 1: the next must be beyond it
            e.ranges.push((s, l));
        }
        for c in partials.as_chunks::<PARTIAL_LEN>().0 {
            let p = Partial {
                n: u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                frag_count: c[4],
                bitmap: u16::from_le_bytes([c[5], c[6]]),
            };
            if p.n == 0
                || p.frag_count == 0
                || p.frag_count > MAX_FRAGS
                || p.bitmap & !p.full_mask() != 0
            {
                return Err(StoreError::Corrupt("rx_state.partials"));
            }
            e.partials.push(p);
        }
        Ok(e)
    }
}

/// `(peer, stream)` state loaded for one decision.
struct Loaded {
    /// Newest epoch first.
    entries: Vec<RxEpoch>,
    floor: Option<u64>,
}

enum Slot {
    Existing(usize),
    Fresh,
}

fn load(c: &Connection, peer: &[u8; 32], stream: i64) -> Result<Loaded> {
    let mut st = c.prepare(
        "SELECT epoch, prefix, ranges, partials FROM rx_state
         WHERE peer = ?1 AND stream = ?2 ORDER BY epoch DESC",
    )?;
    let rows = st.query_map(params![&peer[..], stream], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Vec<u8>>(2)?,
            r.get::<_, Vec<u8>>(3)?,
        ))
    })?;
    let mut entries = Vec::new();
    for r in rows {
        let (epoch, prefix, ranges, partials) = r?;
        entries.push(RxEpoch::decode(
            to_u64(epoch, "rx_state.epoch")?,
            prefix,
            &ranges,
            &partials,
        )?);
    }
    if entries.len() > RX_EPOCHS_KEPT {
        return Err(StoreError::Corrupt("rx_state epochs"));
    }
    let floor: Option<i64> = c
        .query_row(
            "SELECT floor_epoch FROM rx_floor WHERE peer = ?1 AND stream = ?2",
            params![&peer[..], stream],
            |r| r.get(0),
        )
        .optional()?;
    Ok(Loaded {
        entries,
        floor: floor.map(|f| to_u64(f, "rx_floor")).transpose()?,
    })
}

fn locate(l: &Loaded, epoch: u64) -> Option<Slot> {
    if l.floor.is_some_and(|f| epoch <= f) {
        return None;
    }
    if let Some(i) = l.entries.iter().position(|e| e.epoch == epoch) {
        return Some(Slot::Existing(i));
    }
    if l.entries.len() >= RX_EPOCHS_KEPT {
        // Full: only an epoch newer than the oldest kept may enter.
        let oldest = l.entries.last().map(|e| e.epoch).unwrap_or(0);
        if epoch < oldest {
            return None;
        }
    }
    Some(Slot::Fresh)
}

fn save(c: &Connection, peer: &[u8; 32], stream: i64, e: &RxEpoch) -> Result<()> {
    c.execute(
        "INSERT INTO rx_state (peer, stream, epoch, prefix, ranges, partials)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (peer, stream, epoch) DO UPDATE SET
             prefix = excluded.prefix, ranges = excluded.ranges, partials = excluded.partials",
        params![
            &peer[..],
            stream,
            to_i64(e.epoch, "epoch")?,
            i64::from(e.prefix),
            e.encode_ranges(),
            e.encode_partials()
        ],
    )?;
    Ok(())
}

/// After adding a fresh epoch entry: keep the newest 4, raise the floor to
/// the newest evicted epoch.
fn evict_old(c: &Connection, peer: &[u8; 32], stream: i64, l: &Loaded, added: u64) -> Result<()> {
    let mut epochs: Vec<u64> = l.entries.iter().map(|e| e.epoch).collect();
    epochs.push(added);
    epochs.sort_unstable_by(|a, b| b.cmp(a));
    if epochs.len() <= RX_EPOCHS_KEPT {
        return Ok(());
    }
    let mut floor = l.floor.unwrap_or(0);
    for old in &epochs[RX_EPOCHS_KEPT..] {
        c.execute(
            "DELETE FROM rx_state WHERE peer = ?1 AND stream = ?2 AND epoch = ?3",
            params![&peer[..], stream, to_i64(*old, "epoch")?],
        )?;
        drop_frags(c, peer, stream, *old, None)?;
        floor = floor.max(*old);
    }
    c.execute(
        "INSERT INTO rx_floor (peer, stream, floor_epoch) VALUES (?1, ?2, ?3)
         ON CONFLICT (peer, stream) DO UPDATE SET floor_epoch = max(floor_epoch, excluded.floor_epoch)",
        params![&peer[..], stream, to_i64(floor, "epoch")?],
    )?;
    Ok(())
}

/// Delete the stored fragment bytes of `(peer, stream, epoch)` — of one `n`,
/// or of the whole epoch.
fn drop_frags(
    c: &Connection,
    peer: &[u8; 32],
    stream: i64,
    epoch: u64,
    n: Option<u32>,
) -> Result<()> {
    let epoch = to_i64(epoch, "epoch")?;
    match n {
        Some(n) => c.execute(
            "DELETE FROM rx_frags WHERE peer = ?1 AND stream = ?2 AND epoch = ?3 AND n = ?4",
            params![&peer[..], stream, epoch, i64::from(n)],
        )?,
        None => c.execute(
            "DELETE FROM rx_frags WHERE peer = ?1 AND stream = ?2 AND epoch = ?3",
            params![&peer[..], stream, epoch],
        )?,
    };
    Ok(())
}

/// The stored fragments of a complete partial, in `frag_idx` order; every
/// row is re-checked against the bitmap (a gap or a stray row is `Corrupt`).
fn read_frags(
    c: &Connection,
    peer: &[u8; 32],
    stream: i64,
    epoch: u64,
    p: &Partial,
) -> Result<Vec<Vec<u8>>> {
    let mut st = c.prepare(
        "SELECT frag_idx, frag_count, bytes FROM rx_frags
         WHERE peer = ?1 AND stream = ?2 AND epoch = ?3 AND n = ?4 ORDER BY frag_idx",
    )?;
    let rows = st.query_map(
        params![&peer[..], stream, to_i64(epoch, "epoch")?, i64::from(p.n)],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        },
    )?;
    let mut out: Vec<Vec<u8>> = Vec::with_capacity(usize::from(p.frag_count));
    for r in rows {
        let (idx, count, bytes) = r?;
        if idx != out.len() as i64
            || count != i64::from(p.frag_count)
            || bytes.len() > MAX_FRAG_BYTES
        {
            return Err(StoreError::Corrupt("rx_frags"));
        }
        out.push(bytes);
    }
    if out.len() != usize::from(p.frag_count) {
        return Err(StoreError::Corrupt("rx_frags"));
    }
    Ok(out)
}

fn check_n(n: u64) -> Result<u32> {
    if n == 0 || n > N_MAX {
        return Err(StoreError::Invalid("msg_seq n out of range"));
    }
    Ok(n as u32)
}

/// The §4 decision for `(peer, stream, epoch, n)`, recorded on `New`.
pub(crate) fn offer_in(
    c: &Connection,
    peer: &[u8; 32],
    stream: u64,
    epoch: u64,
    n: u64,
) -> Result<Offer> {
    let n = check_n(n)?;
    let stream = to_i64(stream, "stream")?;
    to_i64(epoch, "epoch")?;
    let l = load(c, peer, stream)?;
    match locate(&l, epoch) {
        None => Ok(Offer::StaleEpoch),
        Some(Slot::Existing(i)) => {
            let mut e = l.entries[i].clone();
            if let Some(o) = e.seen(n) {
                return Ok(o);
            }
            if !e.can_insert(n) {
                return Ok(Offer::Deferred);
            }
            e.insert(n);
            save(c, peer, stream, &e)?;
            // The partial (if any) is gone with `insert`; so are its bytes.
            drop_frags(c, peer, stream, epoch, Some(n))?;
            Ok(Offer::New)
        }
        Some(Slot::Fresh) => {
            let mut e = RxEpoch::new(epoch);
            e.insert(n);
            save(c, peer, stream, &e)?;
            evict_old(c, peer, stream, &l, epoch)?;
            Ok(Offer::New)
        }
    }
}

/// The §5 fragment decision; on `Stored` the bit **and** the fragment's
/// bytes are recorded (same transaction).
#[allow(clippy::too_many_arguments)]
pub(crate) fn offer_fragment_in(
    c: &Connection,
    peer: &[u8; 32],
    stream: u64,
    epoch: u64,
    n: u64,
    frag_idx: u8,
    frag_count: u8,
    bytes: &[u8],
) -> Result<FragOffer> {
    let n = check_n(n)?;
    if frag_count == 0 || frag_count > MAX_FRAGS || frag_idx >= frag_count {
        return Err(StoreError::Invalid("fragment bounds"));
    }
    if bytes.len() > MAX_FRAG_BYTES {
        return Err(StoreError::Invalid("fragment too large"));
    }
    let stream = to_i64(stream, "stream")?;
    to_i64(epoch, "epoch")?;
    let l = load(c, peer, stream)?;
    let (mut e, fresh) = match locate(&l, epoch) {
        None => return Ok(FragOffer::StaleEpoch),
        Some(Slot::Existing(i)) => (l.entries[i].clone(), false),
        Some(Slot::Fresh) => (RxEpoch::new(epoch), true),
    };
    if let Some(o) = e.seen(n) {
        return Ok(FragOffer::Delivered(o));
    }
    if !e.can_insert(n) {
        return Ok(FragOffer::Deferred);
    }
    let bit = 1u16 << frag_idx;
    let complete = match e.partials.iter_mut().find(|p| p.n == n) {
        Some(p) => {
            if p.frag_count != frag_count {
                return Ok(FragOffer::Mismatch);
            }
            if p.bitmap & bit != 0 {
                return Ok(FragOffer::DuplicateFragment);
            }
            p.bitmap |= bit;
            p.is_complete()
        }
        None => {
            if e.partials.len() >= MAX_PARTIALS {
                // Oldest-first eviction (§5), bytes included.
                let old = e.partials.remove(0);
                drop_frags(c, peer, stream, epoch, Some(old.n))?;
            }
            let p = Partial {
                n,
                frag_count,
                bitmap: bit,
            };
            e.partials.push(p);
            p.is_complete()
        }
    };
    c.execute(
        "INSERT INTO rx_frags (peer, stream, epoch, n, frag_idx, frag_count, bytes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            &peer[..],
            stream,
            to_i64(epoch, "epoch")?,
            i64::from(n),
            i64::from(frag_idx),
            i64::from(frag_count),
            bytes
        ],
    )?;
    save(c, peer, stream, &e)?;
    if fresh {
        evict_old(c, peer, stream, &l, epoch)?;
    }
    Ok(FragOffer::Stored { complete })
}

impl Store {
    /// §4 dedup for `(peer, stream, epoch, n)` **without** content (tests
    /// only). Recording `n` apart from its content would re-ACK and lose the
    /// message after a crash, so the only production path that records `n`
    /// is [`Store::receive_message`].
    #[cfg(test)]
    pub(crate) fn offer(
        &mut self,
        peer: &[u8; 32],
        stream: u64,
        epoch: u64,
        n: u64,
    ) -> Result<Offer> {
        self.with_tx(Durability::Full, |tx| offer_in(tx, peer, stream, epoch, n))
    }

    /// §5: record fragment `frag_idx` of `frag_count` for message `n`
    /// together with its (opened) payload `bytes` (≤ [`MAX_FRAG_BYTES`]), in
    /// one FULL transaction — the ACK that SACKs it promises the bytes.
    /// On `Stored { complete: true }` read them back with
    /// [`Store::reassemble`] and finish with [`Store::receive_message`].
    #[allow(clippy::too_many_arguments)]
    pub fn offer_fragment(
        &mut self,
        peer: &[u8; 32],
        stream: u64,
        epoch: u64,
        n: u64,
        frag_idx: u8,
        frag_count: u8,
        bytes: &[u8],
    ) -> Result<FragOffer> {
        self.with_tx(Durability::Full, |tx| {
            offer_fragment_in(tx, peer, stream, epoch, n, frag_idx, frag_count, bytes)
        })
    }

    /// The stored fragments of message `n`, in `frag_idx` order — `None`
    /// unless every fragment is in (and the message is not delivered yet).
    pub fn fragments(
        &self,
        peer: &[u8; 32],
        stream: u64,
        epoch: u64,
        n: u64,
    ) -> Result<Option<Vec<Vec<u8>>>> {
        let n = check_n(n)?;
        let stream = to_i64(stream, "stream")?;
        let l = load(&self.conn, peer, stream)?;
        let Some(e) = l.entries.iter().find(|e| e.epoch == epoch) else {
            return Ok(None);
        };
        let Some(p) = e.partials.iter().find(|p| p.n == n) else {
            return Ok(None);
        };
        if !p.is_complete() {
            return Ok(None);
        }
        read_frags(&self.conn, peer, stream, epoch, p).map(Some)
    }

    /// §5 reassembly at the destination: the fragments of `n` concatenated in
    /// order (≤ 16 × 65,535 B), read back from the store; `None` while any
    /// is missing. Pass the result to [`Store::receive_message`], which
    /// records `n`, stores the content and drops the fragments atomically.
    pub fn reassemble(
        &self,
        peer: &[u8; 32],
        stream: u64,
        epoch: u64,
        n: u64,
    ) -> Result<Option<Vec<u8>>> {
        Ok(self
            .fragments(peer, stream, epoch, n)?
            .map(|frags| frags.concat()))
    }

    /// Every message whose fragments are all stored but which is not yet
    /// delivered — after a crash between the last fragment and
    /// [`Store::receive_message`], the fabric finishes these at start.
    pub fn complete_partials(&self) -> Result<Vec<PartialRef>> {
        let mut st = self.conn.prepare(
            "SELECT peer, stream, epoch, prefix, ranges, partials FROM rx_state
             ORDER BY peer, stream, epoch",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Vec<u8>>(4)?,
                r.get::<_, Vec<u8>>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (peer, stream, epoch, prefix, ranges, partials) = r?;
            let peer = id32(peer, "rx_state.peer")?;
            let stream = to_u64(stream, "rx_state.stream")?;
            let e = RxEpoch::decode(to_u64(epoch, "rx_state.epoch")?, prefix, &ranges, &partials)?;
            for p in e.partials.iter().filter(|p| p.is_complete()) {
                out.push(PartialRef {
                    peer,
                    stream,
                    epoch: e.epoch,
                    n: u64::from(p.n),
                });
            }
        }
        Ok(out)
    }

    /// §4 + §18.1: dedup an inbound message and, when [`Offer::New`], store
    /// it (state `delivered`, `received_local_seq` = receipt order) in the
    /// **same** FULL transaction — a crash can never leave `n` recorded
    /// without its content (which would re-ACK and lose it). `msg.peer` is
    /// the sender. For a reassembled message the stored fragments of `n` go
    /// in the same transaction. Returns the verdict and the new row id.
    pub fn receive_message(
        &mut self,
        msg: &NewMessage<'_>,
        seq: Seq,
    ) -> Result<(Offer, Option<i64>)> {
        if msg.body.len() > super::rows::MAX_BODY {
            return Err(StoreError::Invalid("message body too large"));
        }
        self.with_tx(Durability::Full, |tx| {
            let o = offer_in(tx, &msg.peer, msg.stream, seq.epoch, seq.n)?;
            if o != Offer::New {
                return Ok((o, None));
            }
            let id = insert_message(tx, msg, seq, Dir::In, MsgState::Delivered)?;
            Ok((o, Some(id)))
        })
    }

    /// Receive state for `(peer, stream)`, newest epoch first (the current
    /// epoch, then the frozen ones) — the input of a §6.1 ACK block.
    pub fn rx_snapshot(&self, peer: &[u8; 32], stream: u64) -> Result<Vec<RxEpoch>> {
        Ok(load(&self.conn, peer, to_i64(stream, "stream")?)?.entries)
    }

    /// The `(peer, stream)` stale floor: epochs at or below it are dropped.
    pub fn rx_floor(&self, peer: &[u8; 32], stream: u64) -> Result<Option<u64>> {
        Ok(load(&self.conn, peer, to_i64(stream, "stream")?)?.floor)
    }

    /// Every `(peer, stream)` with receive state (diagnostics).
    pub fn rx_streams(&self) -> Result<Vec<([u8; 32], u64)>> {
        let mut st = self
            .conn
            .prepare("SELECT DISTINCT peer, stream FROM rx_state ORDER BY peer, stream")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            let (p, s) = r?;
            out.push((id32(p, "rx_state.peer")?, to_u64(s, "rx_state.stream")?));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ins(e: &mut RxEpoch, n: u32) -> Offer {
        if let Some(o) = e.seen(n) {
            return o;
        }
        if !e.can_insert(n) {
            return Offer::Deferred;
        }
        e.insert(n);
        Offer::New
    }

    #[test]
    fn prefix_and_range_merging() {
        let mut e = RxEpoch::new(1);
        assert_eq!(ins(&mut e, 2), Offer::New);
        assert_eq!(ins(&mut e, 4), Offer::New);
        assert_eq!(e.ranges, vec![(2, 1), (4, 1)]);
        assert_eq!(ins(&mut e, 3), Offer::New); // bridges both
        assert_eq!(e.ranges, vec![(2, 3)]);
        assert_eq!(ins(&mut e, 1), Offer::New); // prefix swallows the range
        assert_eq!((e.prefix, e.ranges.len()), (4, 0));
        assert_eq!(ins(&mut e, 4), Offer::BelowPrefix);
        assert_eq!(ins(&mut e, 7), Offer::New);
        assert_eq!(ins(&mut e, 6), Offer::New); // right-adjacent extends down
        assert_eq!(e.ranges, vec![(6, 2)]);
        assert_eq!(ins(&mut e, 8), Offer::New); // left-adjacent extends up
        assert_eq!(e.ranges, vec![(6, 3)]);
        assert_eq!(ins(&mut e, 7), Offer::Duplicate);
        assert_eq!(ins(&mut e, 5), Offer::New);
        assert_eq!((e.prefix, e.ranges.len()), (8, 0));
    }

    #[test]
    fn range_bound_defers_never_forgets() {
        let mut e = RxEpoch::new(1);
        for k in 0..MAX_RANGES as u32 {
            assert_eq!(ins(&mut e, 3 + 2 * k), Offer::New);
        }
        assert_eq!(e.ranges.len(), MAX_RANGES);
        let far = 3 + 2 * MAX_RANGES as u32 + 1;
        assert_eq!(ins(&mut e, far), Offer::Deferred);
        // Adjacent growth and gap filling still work while full.
        assert_eq!(ins(&mut e, 4), Offer::New);
        assert_eq!(ins(&mut e, far - 1), Offer::New);
        assert_eq!(ins(&mut e, 1), Offer::New);
        assert_eq!(ins(&mut e, 2), Offer::New);
        assert!(e.ranges.len() < MAX_RANGES);
        assert_eq!(ins(&mut e, far), Offer::New);
        assert_eq!(e.seen(far - 2), None); // the one real gap is still open
        for k in (1..=far).filter(|k| *k != far - 2 && *k % 2 == 1) {
            assert!(e.seen(k).is_some(), "{k}");
        }
    }

    #[test]
    fn top_of_n_space() {
        let mut e = RxEpoch::new(1);
        assert_eq!(ins(&mut e, u32::MAX), Offer::New);
        assert_eq!(ins(&mut e, u32::MAX), Offer::Duplicate);
        let enc = e.encode_ranges();
        assert_eq!(RxEpoch::decode(1, 0, &enc, &[]).unwrap(), e);
    }

    #[test]
    fn decode_rejects_bad_rows() {
        // Overlapping / unsorted / adjacent-to-prefix / zero-length ranges.
        let mk = |rs: &[(u32, u32)]| {
            let mut v = Vec::new();
            for (s, l) in rs {
                v.extend_from_slice(&s.to_le_bytes());
                v.extend_from_slice(&l.to_le_bytes());
            }
            v
        };
        assert!(RxEpoch::decode(1, 0, &mk(&[(5, 2), (6, 1)]), &[]).is_err());
        assert!(RxEpoch::decode(1, 0, &mk(&[(7, 1), (5, 1)]), &[]).is_err());
        assert!(RxEpoch::decode(1, 4, &mk(&[(5, 1)]), &[]).is_err());
        assert!(RxEpoch::decode(1, 0, &mk(&[(5, 0)]), &[]).is_err());
        assert!(RxEpoch::decode(1, 0, &mk(&[(5, 2), (7, 1)]), &[]).is_err());
        assert!(RxEpoch::decode(1, 0, &[0u8; 7], &[]).is_err());
        assert!(RxEpoch::decode(1, -1, &[], &[]).is_err());
        // Partials: bits beyond frag_count, zero count, too many fragments.
        assert!(RxEpoch::decode(1, 0, &[], &[5, 0, 0, 0, 2, 0b100, 0]).is_err());
        assert!(RxEpoch::decode(1, 0, &[], &[5, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(RxEpoch::decode(1, 0, &[], &[5, 0, 0, 0, 17, 1, 0]).is_err());
        assert!(
            RxEpoch::decode(1, 0, &mk(&[(5, 2), (8, 1)]), &[9, 0, 0, 0, 16, 0xff, 0xff]).is_ok()
        );
    }
}
