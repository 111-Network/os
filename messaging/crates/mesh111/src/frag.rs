//! §5 — fragmentation Tier 0/1 (fragment-then-seal) and the sender's SACK
//! bookkeeping. Owner: F1.
//!
//! - **Tier 0**: the message fits one bearer frame; one sealed packet, no
//!   `HAS_FRAG`.
//! - **Tier 1**: ≤ 16 fragments (`frag_idx < frag_count ≤ 16`). Every
//!   fragment is a complete sealed packet (independently authenticated,
//!   resumable). The split is a pure function of `(payload, frag_count)`
//!   ([`frag_slice`]) so a fragment can be **re-sealed on every
//!   retransmit** (fresh `epk` + counter) from the stored message body; the
//!   count is fixed when the message is queued (the destination path's
//!   profile, §5 "default 160 B when unknown") and never changes, so the
//!   receiver's partial bitmap stays meaningful. INTRO never rides a
//!   fragment (§5 / envelope rule).
//! - **≤ 16 outstanding** unACKed fragments per peer ([`MAX_OUTSTANDING`]);
//!   reassembly happens only at the destination, through the store's
//!   partials (bytes persisted with the SACK bit, §10.3).
//!
//! ### SACK semantics after a receiver evicts a partial (F1 decision)
//!
//! The store keeps at most 16 partial reassemblies per epoch and evicts the
//! oldest; the fragments of an evicted partial were already SACKed. Both
//! halves are handled:
//!
//! 1. the receiver builds every ACK's partial list from `rx_snapshot`
//!    (`outbox::build_ack_block`), which no longer lists an evicted partial,
//!    so it never claims bits it no longer holds; and
//! 2. the sender ([`TxTracker::apply_partial`]) treats a **listed** partial
//!    as the receiver's whole truth for that message: bits it had recorded
//!    as SACKed but which the newest listing omits are **regressed** to
//!    unacknowledged and become eligible for retransmission. A message the
//!    ACK does not list is left as it was (an ACK on a small MTU is
//!    truncated, so absence carries no information).
//!
//! Together these recover every evicted partial: the sender always holds at
//! least one unSACKed fragment of an undelivered message (a partial whose
//! bits all arrive is complete and delivered in the same transaction), its
//! retransmission re-creates the partial at the receiver, and the ACK that
//! answers it lists that message first (the §6.1 ordering rule for the
//! acknowledged packet), which regresses the stale bits. The sticky store
//! column `outbox.acked` cannot express a regression, so SACK bits live in
//! this tracker (in memory) and the store's `outstanding` flag mirrors the
//! window; after a restart every unacknowledged fragment is simply resent
//! once and the receiver re-ACKs what it holds (`DuplicateFragment`).
//!
//! Loss is inferred without timers: when an ACK evidences a fragment (or a
//! whole message) that was **sent after** an outstanding fragment, the
//! earlier one is presumed lost and leaves the window ([`TxTracker::infer_losses`]).
//! Tail losses wait for the next link or contact event (§9: retries never
//! run on wall-clock timers).

use std::collections::HashMap;

use crate::envelope::{payload_budget, FragPos, MAX_FRAGMENTS, OUTER_OVERHEAD, PAIR_MAC_LEN};
use crate::ids::MsgSeq;

/// §5: at most 16 outstanding unACKed fragments per peer.
pub const MAX_OUTSTANDING: usize = 16;

/// Bytes a `CUSTODY_PUT` adds around its item (§9): outer 64 + inner header
/// `sender_ref 4 ‖ kind 1 ‖ flags 1 ‖ msg_seq 0‖0 (2)` + `dest` 32 +
/// `pair_mac` 16 = 120. A fragment that may travel through the hub is sized
/// so its sealed item still fits the hub link's frame.
pub const CUSTODY_WRAP: usize = OUTER_OVERHEAD + 4 + 1 + 1 + 2 + 32 + PAIR_MAC_LEN;

/// Fragmentation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FragError {
    /// More than 16 fragments would be needed: Tier 2 (bulk objects, v1.1).
    #[error("{len} B needs more than 16 fragments (max {max} B at this MTU)")]
    TooLarge {
        /// Payload length.
        len: usize,
        /// Largest Tier-1 payload at this MTU.
        max: usize,
    },
    /// The MTU cannot carry even an empty packet.
    #[error("MTU too small for a sealed packet")]
    MtuTooSmall,
}

/// Payload bytes one Tier-0 packet carries on a path of `mtu` bytes.
pub fn tier0_budget(mtu: usize, shim: bool, seq: MsgSeq) -> Option<usize> {
    payload_budget(mtu, shim, seq, None, false)
}

/// Payload bytes one Tier-1 fragment carries (frag fields included; both
/// are one-byte varints for `count ≤ 16`).
pub fn tier1_budget(mtu: usize, shim: bool, seq: MsgSeq) -> Option<usize> {
    payload_budget(
        mtu,
        shim,
        seq,
        Some(FragPos {
            idx: MAX_FRAGMENTS - 1,
            count: MAX_FRAGMENTS,
        }),
        false,
    )
}

/// §5: how many fragments a `len`-byte payload needs on a path of `mtu`
/// bytes: 1 (Tier 0) when it fits one packet, else `⌈len / tier1⌉ ≤ 16`.
pub fn plan(len: usize, mtu: usize, shim: bool, seq: MsgSeq) -> Result<u8, FragError> {
    let b0 = tier0_budget(mtu, shim, seq).ok_or(FragError::MtuTooSmall)?;
    if len <= b0 {
        return Ok(1);
    }
    let b1 = tier1_budget(mtu, shim, seq)
        .filter(|b| *b > 0)
        .ok_or(FragError::MtuTooSmall)?;
    let count = len.div_ceil(b1);
    if count > usize::from(MAX_FRAGMENTS) {
        return Err(FragError::TooLarge {
            len,
            max: b1 * usize::from(MAX_FRAGMENTS),
        });
    }
    Ok(count as u8) // ≤ 16
}

/// Fragment `idx` of `count` of `payload`: an even split into
/// `⌈len / count⌉`-byte pieces (the last may be shorter). Pure, so every
/// retransmission re-derives identical bytes. `None` when `idx ≥ count` or
/// `count = 0`.
pub fn frag_slice(payload: &[u8], count: u8, idx: u8) -> Option<&[u8]> {
    if count == 0 || idx >= count {
        return None;
    }
    let size = payload.len().div_ceil(usize::from(count));
    let start = (usize::from(idx) * size).min(payload.len());
    let end = (start + size).min(payload.len());
    Some(&payload[start..end])
}

/// One fragment's sender-side state (in memory; module doc).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FragTx {
    /// The receiver's newest listing holds this fragment.
    pub sacked: bool,
    /// Written to a link and not yet evidenced or presumed lost.
    pub outstanding: bool,
    /// Send order of the last transmission (a counter, never a clock).
    pub tick: u64,
    /// The home hub holds this fragment's custody item (§6.2 ACCEPTED or
    /// DUPLICATE).
    pub held: bool,
}

/// What [`TxTracker::apply_partial`] changed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PartialApply {
    /// Fragments newly SACKed.
    pub sacked: Vec<u8>,
    /// Fragments previously SACKed that the receiver no longer holds.
    pub regressed: Vec<u8>,
    /// Highest send tick among the newly SACKed fragments (loss evidence).
    pub evidence: u64,
}

/// Sender-side SACK + window bookkeeping, keyed by `(message row id,
/// frag_idx)`.
#[derive(Debug, Default)]
pub struct TxTracker {
    frags: HashMap<(i64, u8), FragTx>,
    tick: u64,
}

impl TxTracker {
    /// A fresh tracker (after a restart everything unacknowledged is resent).
    pub fn new() -> Self {
        Self::default()
    }

    /// The state of one fragment.
    pub fn get(&self, msg: i64, idx: u8) -> FragTx {
        self.frags.get(&(msg, idx)).copied().unwrap_or_default()
    }

    /// Record a transmission; returns its tick.
    pub fn on_sent(&mut self, msg: i64, idx: u8) -> u64 {
        self.tick += 1;
        let e = self.frags.entry((msg, idx)).or_default();
        e.outstanding = true;
        e.tick = self.tick;
        self.tick
    }

    /// Set or clear the outstanding flag.
    pub fn set_outstanding(&mut self, msg: i64, idx: u8, outstanding: bool) {
        if outstanding || self.frags.contains_key(&(msg, idx)) {
            self.frags.entry((msg, idx)).or_default().outstanding = outstanding;
        }
    }

    /// The home hub holds this fragment.
    pub fn set_held(&mut self, msg: i64, idx: u8) {
        self.frags.entry((msg, idx)).or_default().held = true;
    }

    /// Every fragment `0..count` of `msg` is held by the hub.
    pub fn all_held(&self, msg: i64, count: u8) -> bool {
        (0..count).all(|i| self.get(msg, i).held)
    }

    /// Apply the receiver's listed partial bitmap for `msg` (`count`
    /// fragments) with **replace** semantics (module doc).
    pub fn apply_partial(&mut self, msg: i64, count: u8, bitmap: u16) -> PartialApply {
        let mut out = PartialApply::default();
        for idx in 0..count.min(MAX_FRAGMENTS) {
            let held = bitmap & (1u16 << idx) != 0;
            let e = self.frags.entry((msg, idx)).or_default();
            match (e.sacked, held) {
                (false, true) => {
                    e.sacked = true;
                    e.outstanding = false;
                    out.sacked.push(idx);
                    out.evidence = out.evidence.max(e.tick);
                }
                (true, false) => {
                    e.sacked = false;
                    e.outstanding = false;
                    out.regressed.push(idx);
                }
                _ => {}
            }
        }
        out
    }

    /// The highest send tick of any fragment of `msg` (evidence when the
    /// whole message is acknowledged).
    pub fn last_tick(&self, msg: i64) -> u64 {
        self.frags
            .iter()
            .filter(|((m, _), _)| *m == msg)
            .map(|(_, f)| f.tick)
            .max()
            .unwrap_or(0)
    }

    /// §5 loss inference without timers: every outstanding, unSACKed
    /// fragment in `frags` sent **before** `evidence` leaves the window.
    /// Returns the fragments presumed lost.
    pub fn infer_losses(&mut self, frags: &[(i64, u8)], evidence: u64) -> Vec<(i64, u8)> {
        let mut lost = Vec::new();
        for k in frags {
            if let Some(e) = self.frags.get_mut(k) {
                if e.outstanding && !e.sacked && e.tick < evidence {
                    e.outstanding = false;
                    lost.push(*k);
                }
            }
        }
        lost
    }

    /// Outstanding fragments among `frags`.
    pub fn outstanding(&self, frags: &[(i64, u8)]) -> usize {
        frags
            .iter()
            .filter(|k| {
                self.frags
                    .get(k)
                    .is_some_and(|f| f.outstanding && !f.sacked)
            })
            .count()
    }

    /// A message is delivered (or gone): drop its state.
    pub fn forget(&mut self, msg: i64) {
        self.frags.retain(|(m, _), _| *m != msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_match_the_size_table() {
        // §3 table, 160 B row without shim: 160 − 64 − (4+1+1+seq) − 16.
        let seq = MsgSeq::new(1, 1); // 2-byte msg_seq
        assert_eq!(tier0_budget(160, false, seq), Some(72));
        assert_eq!(tier1_budget(160, false, seq), Some(70));
        // with the shim (broadcast bearer): one byte less
        assert_eq!(tier0_budget(160, true, seq), Some(71));
        // 233 B row
        assert_eq!(tier0_budget(233, true, seq), Some(144));
        assert_eq!(CUSTODY_WRAP, 120);
    }

    #[test]
    fn plan_tiers_and_bounds() {
        let seq = MsgSeq::new(1, 1);
        assert_eq!(plan(0, 160, false, seq), Ok(1));
        assert_eq!(plan(72, 160, false, seq), Ok(1));
        assert_eq!(plan(73, 160, false, seq), Ok(2));
        assert_eq!(plan(70 * 16, 160, false, seq), Ok(16));
        assert_eq!(
            plan(70 * 16 + 1, 160, false, seq),
            Err(FragError::TooLarge {
                len: 1121,
                max: 1120
            })
        );
        assert_eq!(plan(1, 80, false, seq), Err(FragError::MtuTooSmall));
        assert_eq!(plan(60_000, 65_535, false, seq), Ok(1));
    }

    #[test]
    fn slices_are_even_deterministic_and_cover() {
        let body: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        for count in 1..=16u8 {
            let parts: Vec<&[u8]> = (0..count)
                .map(|i| frag_slice(&body, count, i).unwrap())
                .collect();
            assert_eq!(parts.concat(), body);
            assert!(parts.iter().all(|p| !p.is_empty()));
            let max = parts.iter().map(|p| p.len()).max().unwrap();
            assert_eq!(max, body.len().div_ceil(usize::from(count)));
        }
        assert!(frag_slice(&body, 4, 4).is_none());
        assert!(frag_slice(&body, 0, 0).is_none());
        assert_eq!(frag_slice(&[], 1, 0), Some(&[][..]));
    }

    #[test]
    fn plan_never_leaves_an_empty_fragment() {
        let seq = MsgSeq::new(1, 1);
        for len in 0..=1120usize {
            let body = vec![1u8; len];
            let count = plan(len, 160, false, seq).unwrap();
            for i in 0..count {
                let s = frag_slice(&body, count, i).unwrap();
                assert!(len == 0 || !s.is_empty(), "len {len} count {count}");
                let budget = if count == 1 { 72 } else { 70 };
                assert!(s.len() <= budget);
            }
        }
    }

    #[test]
    fn replace_semantics_regress_evicted_bits() {
        let mut t = TxTracker::new();
        for i in 0..4 {
            t.on_sent(7, i);
        }
        let a = t.apply_partial(7, 4, 0b0111);
        assert_eq!(a.sacked, vec![0, 1, 2]);
        assert_eq!(a.evidence, 3);
        // the receiver evicted the partial and now holds only fragment 3
        let b = t.apply_partial(7, 4, 0b1000);
        assert_eq!(b.sacked, vec![3]);
        assert_eq!(b.regressed, vec![0, 1, 2]);
        for i in 0..3 {
            let f = t.get(7, i);
            assert!(!f.sacked && !f.outstanding, "fragment {i} must be resent");
        }
    }

    #[test]
    fn loss_inference_uses_send_order() {
        let mut t = TxTracker::new();
        t.on_sent(1, 0); // tick 1
        t.on_sent(1, 1); // tick 2
        t.on_sent(2, 0); // tick 3
        let frags = [(1, 0), (1, 1), (2, 0)];
        assert_eq!(t.outstanding(&frags), 3);
        let a = t.apply_partial(1, 2, 0b10); // frag 1 SACKed (tick 2)
        let lost = t.infer_losses(&frags, a.evidence);
        assert_eq!(lost, vec![(1, 0)]); // sent before tick 2, unSACKed
        assert_eq!(t.outstanding(&frags), 1); // (2,0) still in flight
        t.forget(1);
        assert_eq!(t.get(1, 1), FragTx::default());
        t.set_held(2, 0);
        assert!(t.all_held(2, 1));
        assert!(!t.all_held(2, 2));
    }
}
