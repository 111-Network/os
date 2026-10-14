//! §6.1 — the delivery ACK block (standalone `kind = ACK` payload, or the
//! `PIGGYBACK_ACK` block placed before `payload`, §3) and the transferable
//! `ack_sig`. Owner: C2.
//!
//! ```text
//! ack block = ack_len(varint) ‖ ack_epoch(varint) ‖ ack_prefix(varint)
//!             ‖ n_ranges(1, ≤ 8) ‖ [epoch, start_n, len](varint ×3)*
//!             ‖ n_partial(1)     ‖ [epoch, n (varint ×2), frag_bitmap(u16 LE)]*
//!             ‖ n_strong(1)      ‖ [epoch, n (varint ×2), BLAKE2b-128(payload)]*
//! ```
//!
//! `ack_len` covers everything after it. The block acknowledges the
//! (sender, stream) of the packet that carries it — the stream is the ACK's
//! own stream (0 for 1:1 in v1). Authentication is that packet's own
//! `pair_mac`; a **transferable** receipt is the standalone `ACK` form only:
//! payload = block ‖ `ack_sig`(64), with
//! `ack_sig = Ed25519("111/ack-sig/v1" ‖ acker ‖ sender ‖ stream(varint) ‖ block bytes)`
//! where "block bytes" are the exact wire bytes **including** `ack_len`.
//!
//! Canonical-form rules this codec enforces on both encode and decode (fail
//! closed; an ACK is cumulative and repeatable, so a refused ACK costs one
//! re-send, never a hole):
//! - `ack_epoch ≥ 1`; every listed epoch is `≤ ack_epoch`;
//! - ranges: `len ≥ 1`, `start_n ≥ 1`, no `u32` overflow, strictly ascending by
//!   `(epoch, start_n)` without overlap, current-epoch ranges start above
//!   `ack_prefix`; at most [`MAX_RANGES`];
//! - partials: `n ≥ 1`, `frag_bitmap ≠ 0`, strictly ascending by `(epoch, n)`,
//!   and never for a message the block already acknowledges;
//! - strong receipts: `n ≥ 1`, strictly ascending by `(epoch, n)`, and only for
//!   messages the block acknowledges (the hash proves *delivered* content);
//! - `ack_len` matches the body exactly; at most 255 entries per list.

use id111::keyset::{verify_strict_tagged, NodeId, VerifyError};

use crate::envelope::blake2b_128;
use crate::ids::{put_varint, MsgSeq, ReadError, Reader};
use crate::sealer::Sealer;

/// §3.1 domain tag of the transferable ACK signature.
pub const ACK_SIG_TAG: &[u8] = b"111/ack-sig/v1";

/// §6.1: `n_ranges ≤ 8`.
pub const MAX_RANGES: usize = 8;

/// `ack_sig` length (Ed25519).
pub const ACK_SIG_LEN: usize = 64;

/// One acknowledged run `[start_n, start_n + len)` of `epoch` (absolute).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AckRange {
    /// The epoch the run belongs to (current or frozen).
    pub epoch: u64,
    /// First `n` of the run (≥ 1).
    pub start: u32,
    /// Run length (≥ 1).
    pub len: u32,
}

impl AckRange {
    /// Last `n` of the run, `None` on an empty or overflowing run.
    pub fn last(&self) -> Option<u32> {
        self.len
            .checked_sub(1)
            .and_then(|l| self.start.checked_add(l))
    }

    /// True when `seq` lies in this run.
    pub fn contains(&self, seq: MsgSeq) -> bool {
        match self.last() {
            Some(last) => seq.epoch == self.epoch && seq.n >= self.start && seq.n <= last,
            None => false,
        }
    }
}

/// Fragments received of an in-flight (Tier 1, §5) message: bit `i` of
/// `frag_bitmap` = fragment `i` held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PartialAck {
    /// The message's `msg_seq`.
    pub seq: MsgSeq,
    /// SACK bitmap (LE u16 on the wire; ≤ 16 fragments, §5).
    pub frag_bitmap: u16,
}

/// A `RECEIPT_STRONG` answer: the recipient's `BLAKE2b-128(payload)` of the
/// delivered message (proof it decrypted *this* content).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StrongReceipt {
    /// The message's `msg_seq`.
    pub seq: MsgSeq,
    /// [`strong_hash`] of the message payload.
    pub payload_hash: [u8; 16],
}

/// The §6.1 block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AckBlock {
    /// The receiver's current epoch for this (sender, stream).
    pub epoch: u64,
    /// Delivered prefix of the current epoch: `1..=prefix` delivered.
    pub prefix: u32,
    /// Current-epoch ranges above the prefix plus frozen epochs, absolute.
    pub ranges: Vec<AckRange>,
    /// Fragment bitmaps of in-flight messages.
    pub partials: Vec<PartialAck>,
    /// Payload hashes for messages that asked `RECEIPT_STRONG`.
    pub strong: Vec<StrongReceipt>,
}

/// ACK codec failures (typed; never a panic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AckError {
    /// Low-level read failure (truncation, non-minimal varint, …).
    #[error("ack block: {0}")]
    Read(#[from] ReadError),
    /// `ack_len` disagrees with the body it prefixes.
    #[error("ack_len does not match the block body")]
    LengthMismatch,
    /// `ack_epoch = 0` (epochs are announce-counter values ≥ 1).
    #[error("ack_epoch is zero")]
    ZeroEpoch,
    /// More than [`MAX_RANGES`] ranges.
    #[error("{0} ranges (max 8)")]
    TooManyRanges(usize),
    /// A list longer than its 1-byte count can carry.
    #[error("{0} entries exceed a 1-byte count")]
    TooManyEntries(usize),
    /// A range with `len = 0`, `start_n = 0`, a `u32` overflow, or an epoch
    /// above `ack_epoch`.
    #[error("malformed range")]
    BadRange,
    /// Ranges out of order, overlapping, or overlapping the prefix.
    #[error("ranges not strictly ascending / overlapping")]
    RangeOrder,
    /// A partial with `n = 0`, an empty bitmap, an epoch above `ack_epoch`,
    /// out of order, or for an already-acknowledged message.
    #[error("malformed partial entry")]
    BadPartial,
    /// A strong receipt with `n = 0`, out of order, or for a message the
    /// block does not acknowledge.
    #[error("malformed strong receipt")]
    BadStrong,
    /// A standalone ACK payload whose tail after the block is neither empty
    /// nor exactly one 64 B `ack_sig`.
    #[error("ACK payload tail is {0} B (0 or 64 allowed)")]
    BadTail(usize),
    /// `ack_sig` failed the strict Ed25519 predicate.
    #[error("ack_sig: {0}")]
    BadSignature(VerifyError),
}

impl AckBlock {
    /// True when the block acknowledges `seq` (current-epoch prefix or any
    /// range). Partials never count as acknowledged.
    pub fn acks(&self, seq: MsgSeq) -> bool {
        if seq.n == 0 {
            return false;
        }
        (seq.epoch == self.epoch && seq.n <= self.prefix)
            || self.ranges.iter().any(|r| r.contains(seq))
    }

    /// Check every canonical-form rule (module doc).
    pub fn validate(&self) -> Result<(), AckError> {
        if self.epoch == 0 {
            return Err(AckError::ZeroEpoch);
        }
        if self.ranges.len() > MAX_RANGES {
            return Err(AckError::TooManyRanges(self.ranges.len()));
        }
        for len in [self.partials.len(), self.strong.len()] {
            if len > usize::from(u8::MAX) {
                return Err(AckError::TooManyEntries(len));
            }
        }
        let mut prev: Option<(u64, u32)> = None; // (epoch, last n)
        for r in &self.ranges {
            let last = r.last().ok_or(AckError::BadRange)?;
            if r.start == 0 || r.epoch == 0 || r.epoch > self.epoch {
                return Err(AckError::BadRange);
            }
            if r.epoch == self.epoch && r.start <= self.prefix {
                return Err(AckError::RangeOrder);
            }
            if let Some((pe, pl)) = prev {
                if r.epoch < pe || (r.epoch == pe && r.start <= pl) {
                    return Err(AckError::RangeOrder);
                }
            }
            prev = Some((r.epoch, last));
        }
        let mut prev: Option<MsgSeq> = None;
        for p in &self.partials {
            if p.seq.n == 0
                || p.seq.epoch == 0
                || p.seq.epoch > self.epoch
                || p.frag_bitmap == 0
                || prev.is_some_and(|q| p.seq <= q)
                || self.acks(p.seq)
            {
                return Err(AckError::BadPartial);
            }
            prev = Some(p.seq);
        }
        let mut prev: Option<MsgSeq> = None;
        for s in &self.strong {
            if s.seq.n == 0 || prev.is_some_and(|q| s.seq <= q) || !self.acks(s.seq) {
                return Err(AckError::BadStrong);
            }
            prev = Some(s.seq);
        }
        Ok(())
    }

    fn encode_body(&self, out: &mut Vec<u8>) {
        put_varint(out, self.epoch);
        put_varint(out, u64::from(self.prefix));
        out.push(self.ranges.len() as u8); // ≤ 8, validated
        for r in &self.ranges {
            put_varint(out, r.epoch);
            put_varint(out, u64::from(r.start));
            put_varint(out, u64::from(r.len));
        }
        out.push(self.partials.len() as u8); // ≤ 255, validated
        for p in &self.partials {
            p.seq.encode(out);
            out.extend_from_slice(&p.frag_bitmap.to_le_bytes());
        }
        out.push(self.strong.len() as u8); // ≤ 255, validated
        for s in &self.strong {
            s.seq.encode(out);
            out.extend_from_slice(&s.payload_hash);
        }
    }

    /// The full wire block, `ack_len` included.
    pub fn encode(&self) -> Result<Vec<u8>, AckError> {
        self.validate()?;
        let mut body = Vec::new();
        self.encode_body(&mut body);
        let mut out = Vec::with_capacity(body.len() + 3);
        put_varint(&mut out, body.len() as u64);
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// Read one block (with its `ack_len`) from `r`. Returns the block and
    /// the exact wire bytes it occupied (the `ack_sig` input).
    pub fn decode<'a>(r: &mut Reader<'a>) -> Result<(AckBlock, &'a [u8]), AckError> {
        let start = r.position();
        let mut whole = r.clone();
        let ack_len = r.varint()?;
        let ack_len = usize::try_from(ack_len).map_err(|_| AckError::LengthMismatch)?;
        if ack_len > r.remaining() {
            return Err(AckError::LengthMismatch);
        }
        let body = r.take(ack_len)?;
        let wire = whole.take(r.position() - start)?;
        let mut b = Reader::new(body);
        let block = Self::decode_body(&mut b).map_err(|e| match e {
            AckError::Read(ReadError::Truncated) => AckError::LengthMismatch,
            other => other,
        })?;
        if !b.is_empty() {
            return Err(AckError::LengthMismatch);
        }
        block.validate()?;
        Ok((block, wire))
    }

    /// Decode a buffer that holds exactly one block.
    pub fn decode_exact(bytes: &[u8]) -> Result<AckBlock, AckError> {
        let mut r = Reader::new(bytes);
        let (block, _) = Self::decode(&mut r)?;
        if !r.is_empty() {
            return Err(AckError::LengthMismatch);
        }
        Ok(block)
    }

    fn decode_body(b: &mut Reader<'_>) -> Result<AckBlock, AckError> {
        // Epochs are bounded like every msg_seq epoch (≤ i64::MAX, §18.2): a
        // hostile ACK must never become a store error downstream.
        let epoch = b.varint_epoch()?;
        let prefix = b.varint_u32()?;
        let n_ranges = usize::from(b.u8()?);
        if n_ranges > MAX_RANGES {
            return Err(AckError::TooManyRanges(n_ranges));
        }
        let mut ranges = Vec::with_capacity(n_ranges);
        for _ in 0..n_ranges {
            let epoch = b.varint_epoch()?;
            let start = b.varint_u32()?;
            let len = b.varint_u32()?;
            ranges.push(AckRange { epoch, start, len });
        }
        let n_partial = usize::from(b.u8()?);
        let mut partials = Vec::with_capacity(n_partial.min(b.remaining() / 4));
        for _ in 0..n_partial {
            let seq = MsgSeq::decode(b)?;
            let frag_bitmap = b.u16_le()?;
            partials.push(PartialAck { seq, frag_bitmap });
        }
        let n_strong = usize::from(b.u8()?);
        let mut strong = Vec::with_capacity(n_strong.min(b.remaining() / 18));
        for _ in 0..n_strong {
            let seq = MsgSeq::decode(b)?;
            let payload_hash = b.array::<16>()?;
            strong.push(StrongReceipt { seq, payload_hash });
        }
        Ok(AckBlock {
            epoch,
            prefix,
            ranges,
            partials,
            strong,
        })
    }
}

/// §6.1: `BLAKE2b-128(payload)` for a strong receipt (the delivered
/// message's payload; for a Tier-1 message, the reassembled payload).
pub fn strong_hash(payload: &[u8]) -> [u8; 16] {
    blake2b_128(&[payload])
}

/// A standalone `ACK` packet's payload: block ‖ [`ack_sig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckPayload {
    /// The block.
    pub block: AckBlock,
    /// Exact wire bytes of the block (incl. `ack_len`) — the `ack_sig` input.
    pub block_bytes: Vec<u8>,
    /// The transferable signature, when requested.
    pub ack_sig: Option<[u8; 64]>,
}

impl AckPayload {
    /// Parse an `ACK` payload: one block, then nothing or exactly 64 B.
    pub fn decode(payload: &[u8]) -> Result<AckPayload, AckError> {
        let mut r = Reader::new(payload);
        let (block, wire) = AckBlock::decode(&mut r)?;
        let tail = r.rest();
        let ack_sig = match tail.len() {
            0 => None,
            ACK_SIG_LEN => {
                let mut s = [0u8; 64];
                s.copy_from_slice(tail);
                Some(s)
            }
            n => return Err(AckError::BadTail(n)),
        };
        Ok(AckPayload {
            block,
            block_bytes: wire.to_vec(),
            ack_sig,
        })
    }

    /// The payload bytes: block ‖ [ack_sig].
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.block_bytes.clone();
        if let Some(s) = &self.ack_sig {
            out.extend_from_slice(s);
        }
        out
    }

    /// Build a payload, signing it when `transferable` (acker = this node).
    pub fn build(
        sealer: &dyn Sealer,
        block: AckBlock,
        sender: &NodeId,
        stream: u64,
        transferable: bool,
    ) -> Result<AckPayload, AckError> {
        let block_bytes = block.encode()?;
        let ack_sig = transferable.then(|| {
            sealer.sign(
                ACK_SIG_TAG,
                &ack_sig_input(&sealer.node_id(), sender, stream, &block_bytes),
            )
        });
        Ok(AckPayload {
            block,
            block_bytes,
            ack_sig,
        })
    }

    /// Verify the transferable signature (if any) as `acker` acknowledging
    /// `sender`'s `stream`. `Ok(false)` when unsigned.
    pub fn verify(&self, acker: &NodeId, sender: &NodeId, stream: u64) -> Result<bool, AckError> {
        match &self.ack_sig {
            None => Ok(false),
            Some(sig) => {
                verify_ack_sig(acker, sender, stream, &self.block_bytes, sig).map(|()| true)
            }
        }
    }
}

/// §6.1 `ack_sig` signed input **without** the domain tag (the signer and
/// the strict verifier prepend `"111/ack-sig/v1"`):
/// `acker(32) ‖ sender(32) ‖ stream(varint) ‖ block bytes`.
pub fn ack_sig_input(acker: &NodeId, sender: &NodeId, stream: u64, block_bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + 10 + block_bytes.len());
    out.extend_from_slice(acker.as_bytes());
    out.extend_from_slice(sender.as_bytes());
    put_varint(&mut out, stream);
    out.extend_from_slice(block_bytes);
    out
}

/// Verify an `ack_sig` under id111's strict Ed25519 predicate.
pub fn verify_ack_sig(
    acker: &NodeId,
    sender: &NodeId,
    stream: u64,
    block_bytes: &[u8],
    sig: &[u8; 64],
) -> Result<(), AckError> {
    verify_strict_tagged(
        acker,
        ACK_SIG_TAG,
        &ack_sig_input(acker, sender, stream, block_bytes),
        sig,
    )
    .map_err(AckError::BadSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_epoch() -> AckBlock {
        AckBlock {
            epoch: 9,
            prefix: 3,
            ranges: vec![
                AckRange {
                    epoch: 4,
                    start: 1,
                    len: 7,
                },
                AckRange {
                    epoch: 4,
                    start: 10,
                    len: 2,
                },
                AckRange {
                    epoch: 9,
                    start: 5,
                    len: 1,
                },
            ],
            partials: vec![PartialAck {
                seq: MsgSeq::new(9, 4),
                frag_bitmap: 0b101,
            }],
            strong: vec![StrongReceipt {
                seq: MsgSeq::new(4, 2),
                payload_hash: strong_hash(b"op"),
            }],
        }
    }

    #[test]
    fn round_trip_two_epochs() {
        let b = two_epoch();
        let wire = b.encode().unwrap();
        assert_eq!(AckBlock::decode_exact(&wire).unwrap(), b);
        assert!(b.acks(MsgSeq::new(9, 3)));
        assert!(b.acks(MsgSeq::new(4, 7)));
        assert!(!b.acks(MsgSeq::new(4, 8)));
        assert!(b.acks(MsgSeq::new(4, 11)));
        assert!(!b.acks(MsgSeq::new(9, 4)));
        assert!(b.acks(MsgSeq::new(9, 5)));
        assert!(!b.acks(MsgSeq::new(9, 0)));
    }

    #[test]
    fn ack_epoch_above_i64_max_is_rejected_before_any_state() {
        // Lead follow-up to review finding "epoch width": the ACK decoder bounds
        // epochs exactly like MsgSeq::decode (≤ i64::MAX).
        let mut bytes = Vec::new();
        let mut body = Vec::new();
        put_varint(&mut body, 1u64 << 63); // ack_epoch
        put_varint(&mut body, 1); // prefix
        body.push(0); // n_ranges
        body.push(0); // n_partial
        body.push(0); // n_strong
        put_varint(&mut bytes, body.len() as u64);
        bytes.extend_from_slice(&body);
        assert!(matches!(
            AckBlock::decode_exact(&bytes),
            Err(AckError::Read(ReadError::EpochOverflow))
        ));
    }

    #[test]
    fn canonical_rules() {
        let mut b = two_epoch();
        b.ranges.swap(0, 1);
        assert_eq!(b.encode(), Err(AckError::RangeOrder));
        let mut b = two_epoch();
        b.ranges[2].start = 3; // overlaps the prefix
        assert_eq!(b.encode(), Err(AckError::RangeOrder));
        let mut b = two_epoch();
        b.ranges[1].len = 0;
        assert_eq!(b.encode(), Err(AckError::BadRange));
        let mut b = two_epoch();
        b.ranges[0].epoch = 10; // above ack_epoch
        assert_eq!(b.encode(), Err(AckError::BadRange));
        let mut b = two_epoch();
        b.ranges = (0..9)
            .map(|i| AckRange {
                epoch: 9,
                start: 10 + 3 * i,
                len: 1,
            })
            .collect();
        assert_eq!(b.encode(), Err(AckError::TooManyRanges(9)));
        let mut b = two_epoch();
        b.partials[0].frag_bitmap = 0;
        assert_eq!(b.encode(), Err(AckError::BadPartial));
        let mut b = two_epoch();
        b.partials[0].seq = MsgSeq::new(9, 2); // already acknowledged
        assert_eq!(b.encode(), Err(AckError::BadPartial));
        let mut b = two_epoch();
        b.strong[0].seq = MsgSeq::new(4, 8); // not acknowledged
        assert_eq!(b.encode(), Err(AckError::BadStrong));
        let mut b = two_epoch();
        b.epoch = 0;
        assert_eq!(b.encode(), Err(AckError::ZeroEpoch));
    }

    #[test]
    fn ack_len_is_exact() {
        let wire = two_epoch().encode().unwrap();
        // short by one
        let mut short = wire.clone();
        short.pop();
        assert_eq!(
            AckBlock::decode_exact(&short),
            Err(AckError::LengthMismatch)
        );
        // ack_len one too large but data present after: body underrun
        let mut long = wire.clone();
        long[0] += 1;
        long.push(0);
        assert_eq!(AckBlock::decode_exact(&long), Err(AckError::LengthMismatch));
        // trailing byte after an exact block
        let mut trail = wire.clone();
        trail.push(0);
        assert_eq!(
            AckBlock::decode_exact(&trail),
            Err(AckError::LengthMismatch)
        );
    }

    #[test]
    fn payload_tail_is_zero_or_sig() {
        let wire = two_epoch().encode().unwrap();
        assert!(AckPayload::decode(&wire).unwrap().ack_sig.is_none());
        let mut with = wire.clone();
        with.extend_from_slice(&[7u8; 64]);
        assert_eq!(AckPayload::decode(&with).unwrap().ack_sig, Some([7u8; 64]));
        let mut bad = wire.clone();
        bad.extend_from_slice(&[7u8; 10]);
        assert_eq!(AckPayload::decode(&bad), Err(AckError::BadTail(10)));
    }
}
