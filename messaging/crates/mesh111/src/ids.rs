//! §4 — message identity: `msg_id = (sender, recipient, stream, epoch, n)`,
//! the two-varint `msg_seq` wire form, unsigned LEB128 (§3: "varints are
//! unsigned LEB128") and the receiver's epoch rule. Owner: C2 (codec/math);
//! C3 owns the persistence (`tx_state` / `rx_state`).
//!
//! Wire rules pinned here (fail closed, §18 rule 2):
//! - varints are **minimal** LEB128 of a `u64`: an over-long encoding
//!   (`0x80 0x00`), an 11th byte, or a value above `u64::MAX` is rejected —
//!   so every value has exactly one encoding and MAC/sig inputs are unique;
//! - `msg_seq = epoch(varint) ‖ n(varint)`; `n` is a `u32` (§4: a wrap of
//!   2^32 mints a new epoch), a larger `n` is rejected;
//! - control kinds carry `0 ‖ 0` ([`MsgSeq::CONTROL`]); `msg_seq` kinds carry
//!   `epoch ≥ 1` (an L2 announce-counter value, which starts at 1) and
//!   `n ≥ 1` ([`N_FIRST`]) — so `ack_prefix = P` reads "1..=P delivered" and
//!   `P = 0` reads "nothing yet".

use id111::keyset::NodeId;

/// §4: stream 0 is the pair's 1:1 stream. The v1 inner layout (§3) has no
/// stream field, so every v1 packet is on stream 0; the value still enters
/// the `sig` (§3) and `ack_sig` (§6.1) inputs as a varint.
pub const STREAM_1TO1: u64 = 0;

/// §4: the first `n` of every (peer, stream, epoch); `n = 0` only appears in
/// the control form `0 ‖ 0`.
pub const N_FIRST: u32 = 1;
/// The largest `epoch` the wire accepts: `i64::MAX`. The store keys epochs
/// as SQLite `INTEGER` (i64), so a larger value from a hostile contact must
/// fail here, in the codec, never as a store error after a valid `pair_mac`.
pub const MAX_EPOCH: u64 = i64::MAX as u64;

/// §4: `rx_state` keeps the newest 4 epochs per (peer, stream).
pub const RX_EPOCHS_KEPT: usize = 4;

/// Longest LEB128 encoding of a `u64` (10 bytes: 9 × 7 bits + 1 bit).
pub const MAX_VARINT_LEN: usize = 10;

/// Low-level read failures shared by every codec in the crate. Typed, never
/// a panic (L2 §12 rule 7: sizes checked before parse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// The buffer ended before the field did.
    #[error("truncated input")]
    Truncated,
    /// A varint ran past 10 bytes or above `u64::MAX`.
    #[error("varint overflows u64")]
    VarintOverflow,
    /// A varint used more bytes than its value needs (non-canonical).
    #[error("varint is not minimally encoded")]
    VarintOverlong,
    /// A `u32` field (`n`, `start_n`, `len`) carried a larger value.
    #[error("value exceeds u32")]
    U32Overflow,
    /// An `epoch` above [`MAX_EPOCH`] (`i64::MAX`).
    #[error("epoch exceeds i64::MAX")]
    EpochOverflow,
}

// ---------------------------------------------------------------------------
// LEB128
// ---------------------------------------------------------------------------

/// Append the minimal unsigned LEB128 encoding of `v` (§3).
pub fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let low = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(low);
            return;
        }
        out.push(low | 0x80);
    }
}

/// Encoded length of `v` as a minimal LEB128 varint (1–10 bytes).
pub fn varint_len(v: u64) -> usize {
    let bits = 64 - v.leading_zeros() as usize;
    bits.div_ceil(7).max(1)
}

/// Decode one minimal unsigned LEB128 varint from the front of `buf`.
/// Returns the value and the number of bytes consumed.
pub fn decode_varint(buf: &[u8]) -> Result<(u64, usize), ReadError> {
    let mut value: u64 = 0;
    for i in 0..MAX_VARINT_LEN {
        let b = *buf.get(i).ok_or(ReadError::Truncated)?;
        // The 10th byte may carry only bit 63 and must terminate.
        if i == MAX_VARINT_LEN - 1 && b > 0x01 {
            return Err(ReadError::VarintOverflow);
        }
        value |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            if i > 0 && b == 0 {
                return Err(ReadError::VarintOverlong);
            }
            return Ok((value, i + 1));
        }
    }
    Err(ReadError::VarintOverflow)
}

/// A bounds-checked cursor over untrusted bytes. Every read is checked
/// against the remaining length first; nothing here can panic.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// A reader over `buf`, positioned at its start.
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    /// Bytes consumed so far.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Bytes not yet consumed.
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// True once every byte is consumed.
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// The next `n` bytes.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], ReadError> {
        if n > self.remaining() {
            return Err(ReadError::Truncated);
        }
        let out = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    /// One byte.
    pub fn u8(&mut self) -> Result<u8, ReadError> {
        Ok(self.take(1)?[0])
    }

    /// A fixed-size array.
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], ReadError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    /// A little-endian `u16` (§6.1 `frag_bitmap`).
    pub fn u16_le(&mut self) -> Result<u16, ReadError> {
        Ok(u16::from_le_bytes(self.array::<2>()?))
    }

    /// A minimal LEB128 `u64`.
    pub fn varint(&mut self) -> Result<u64, ReadError> {
        let (v, used) = decode_varint(&self.buf[self.pos..])?;
        self.pos += used;
        Ok(v)
    }

    /// A minimal LEB128 varint that must fit a `u32`.
    pub fn varint_u32(&mut self) -> Result<u32, ReadError> {
        u32::try_from(self.varint()?).map_err(|_| ReadError::U32Overflow)
    }

    /// A minimal LEB128 `epoch`: at most [`MAX_EPOCH`].
    pub fn varint_epoch(&mut self) -> Result<u64, ReadError> {
        let v = self.varint()?;
        if v > MAX_EPOCH {
            return Err(ReadError::EpochOverflow);
        }
        Ok(v)
    }

    /// Everything left, consuming it.
    pub fn rest(&mut self) -> &'a [u8] {
        let out = &self.buf[self.pos..];
        self.pos = self.buf.len();
        out
    }
}

// ---------------------------------------------------------------------------
// msg_seq / msg_id
// ---------------------------------------------------------------------------

/// §4: `msg_seq = epoch ‖ n`. Ordered by `(epoch, n)` — the sender's issue
/// order, since epochs are strictly increasing announce-counter values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct MsgSeq {
    /// A freshly committed L2 announce-counter value (§4), ≥ 1 for
    /// `msg_seq` kinds; 0 only in the control form.
    pub epoch: u64,
    /// Strictly increasing per (peer, stream) within the epoch, from
    /// [`N_FIRST`]; 0 only in the control form.
    pub n: u32,
}

impl MsgSeq {
    /// §3.2: the `0 ‖ 0` carried by every control kind.
    pub const CONTROL: MsgSeq = MsgSeq { epoch: 0, n: 0 };

    /// `(epoch, n)`.
    pub fn new(epoch: u64, n: u32) -> Self {
        MsgSeq { epoch, n }
    }

    /// True for the control form `0 ‖ 0`.
    pub fn is_control(&self) -> bool {
        *self == Self::CONTROL
    }

    /// True when this is a valid `msg_seq` for a sequenced kind:
    /// `1 ≤ epoch ≤ MAX_EPOCH` and `n ≥ 1`.
    pub fn is_sequenced(&self) -> bool {
        (1..=MAX_EPOCH).contains(&self.epoch) && self.n >= N_FIRST
    }

    /// The next `n` in this epoch, or `None` when `n` would wrap 2^32 — §4:
    /// the caller then mints a new epoch.
    pub fn next(&self) -> Option<MsgSeq> {
        self.n.checked_add(1).map(|n| MsgSeq {
            epoch: self.epoch,
            n,
        })
    }

    /// Append `epoch(varint) ‖ n(varint)`.
    pub fn encode(&self, out: &mut Vec<u8>) {
        put_varint(out, self.epoch);
        put_varint(out, u64::from(self.n));
    }

    /// The two-varint wire form.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.encoded_len());
        self.encode(&mut out);
        out
    }

    /// Length of the two-varint wire form (2–15 bytes; 2–4 typical).
    pub fn encoded_len(&self) -> usize {
        varint_len(self.epoch) + varint_len(u64::from(self.n))
    }

    /// Read `epoch(varint) ‖ n(varint)`: `epoch ≤ MAX_EPOCH`
    /// ([`ReadError::EpochOverflow`]) and `n` must fit a `u32`
    /// ([`ReadError::U32Overflow`]). Zero checks are the kind's (§3.2).
    pub fn decode(r: &mut Reader<'_>) -> Result<MsgSeq, ReadError> {
        let epoch = r.varint_epoch()?;
        let n = r.varint_u32()?;
        Ok(MsgSeq { epoch, n })
    }
}

/// §4: the full message identity. Never derived from content hashes or
/// random values (both forget under pruning and then redeliver).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MsgId {
    /// The authenticated sender (resolved by `pair_mac`, never by `sig`).
    pub sender: NodeId,
    /// The recipient (`tx_state` / `rx_state` key with the sender).
    pub recipient: NodeId,
    /// [`STREAM_1TO1`] in v1.
    pub stream: u64,
    /// The sender's epoch.
    pub epoch: u64,
    /// The sender's per-(peer, stream) counter within `epoch`.
    pub n: u32,
}

impl MsgId {
    /// Compose an id from the authenticated parties and the wire `msg_seq`.
    pub fn new(sender: NodeId, recipient: NodeId, stream: u64, seq: MsgSeq) -> Self {
        MsgId {
            sender,
            recipient,
            stream,
            epoch: seq.epoch,
            n: seq.n,
        }
    }

    /// The wire `msg_seq` of this id.
    pub fn seq(&self) -> MsgSeq {
        MsgSeq::new(self.epoch, self.n)
    }
}

// ---------------------------------------------------------------------------
// §4 receiver epoch rule
// ---------------------------------------------------------------------------

/// What `rx_state` does with a `pair_mac`-verified packet's epoch (§4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochVerdict {
    /// The epoch has an entry (current or frozen): deliver against it.
    Existing,
    /// The epoch **exceeds** every kept one: open a new entry, which becomes
    /// current; the previous current is frozen and kept. `evict` is the
    /// oldest kept epoch to drop when [`RX_EPOCHS_KEPT`] are already held.
    OpenCurrent {
        /// Oldest epoch to drop to stay at [`RX_EPOCHS_KEPT`].
        evict: Option<u64>,
    },
    /// An older, never-seen epoch that still ranks among the newest
    /// [`RX_EPOCHS_KEPT`] (custody and multi-path legitimately deliver an
    /// earlier epoch after a later one): open it frozen, below the current.
    OpenFrozen {
        /// Oldest epoch to drop to stay at [`RX_EPOCHS_KEPT`].
        evict: Option<u64>,
    },
    /// Older than every kept epoch while [`RX_EPOCHS_KEPT`] are held: stale,
    /// dropped. Never redelivered, possibly never delivered (§4's stated rule).
    Stale,
}

/// §4: classify `incoming` against the epochs `rx_state` holds for one
/// (peer, stream). `kept` may be in any order; duplicates are ignored.
/// `incoming = 0` is never a sequenced epoch and is reported [`EpochVerdict::Stale`].
pub fn classify_epoch(kept: &[u64], incoming: u64) -> EpochVerdict {
    if incoming == 0 {
        return EpochVerdict::Stale;
    }
    if kept.contains(&incoming) {
        return EpochVerdict::Existing;
    }
    let mut distinct: Vec<u64> = kept.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    let full = distinct.len() >= RX_EPOCHS_KEPT;
    let oldest = distinct.first().copied();
    let newest = distinct.last().copied();
    let evict = if full { oldest } else { None };
    match newest {
        None => EpochVerdict::OpenCurrent { evict: None },
        Some(cur) if incoming > cur => EpochVerdict::OpenCurrent { evict },
        Some(_) => match oldest {
            Some(old) if full && incoming < old => EpochVerdict::Stale,
            _ => EpochVerdict::OpenFrozen { evict },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_round_trip_and_lengths() {
        for v in [
            0u64,
            1,
            127,
            128,
            300,
            16_383,
            16_384,
            u64::from(u32::MAX),
            u64::MAX - 1,
            u64::MAX,
        ] {
            let mut out = Vec::new();
            put_varint(&mut out, v);
            assert_eq!(out.len(), varint_len(v), "len of {v}");
            assert_eq!(decode_varint(&out), Ok((v, out.len())));
        }
        let mut out = Vec::new();
        put_varint(&mut out, 300);
        assert_eq!(out, [0xac, 0x02]);
        out.clear();
        put_varint(&mut out, u64::MAX);
        assert_eq!(out.len(), 10);
        assert_eq!(out[9], 0x01);
    }

    #[test]
    fn varint_rejects_overlong_overflow_truncation() {
        assert_eq!(decode_varint(&[]), Err(ReadError::Truncated));
        assert_eq!(decode_varint(&[0x80]), Err(ReadError::Truncated));
        assert_eq!(decode_varint(&[0x80, 0x00]), Err(ReadError::VarintOverlong));
        assert_eq!(
            decode_varint(&[0xff, 0x80, 0x00]),
            Err(ReadError::VarintOverlong)
        );
        let mut over = [0xffu8; 10];
        over[9] = 0x02;
        assert_eq!(decode_varint(&over), Err(ReadError::VarintOverflow));
        assert_eq!(decode_varint(&[0xff; 11]), Err(ReadError::VarintOverflow));
        // 0x00 alone is the canonical zero.
        assert_eq!(decode_varint(&[0x00]), Ok((0, 1)));
    }

    #[test]
    fn msg_seq_wire_form() {
        let s = MsgSeq::new(1, 1);
        assert_eq!(s.to_bytes(), [0x01, 0x01]);
        assert_eq!(s.encoded_len(), 2);
        let big = MsgSeq::new(200, 70_000);
        assert_eq!(big.encoded_len(), 2 + 3);
        let bytes = big.to_bytes();
        let mut r = Reader::new(&bytes);
        assert_eq!(MsgSeq::decode(&mut r), Ok(big));
        assert!(r.is_empty());
        // n above u32::MAX is refused.
        let mut wide = vec![0x01];
        put_varint(&mut wide, u64::from(u32::MAX) + 1);
        assert_eq!(
            MsgSeq::decode(&mut Reader::new(&wide)),
            Err(ReadError::U32Overflow)
        );
        // epoch i64::MAX is the ceiling; one above is refused before the
        // store can see it
        let top = MsgSeq::new(MAX_EPOCH, 1);
        assert!(top.is_sequenced());
        assert_eq!(MsgSeq::decode(&mut Reader::new(&top.to_bytes())), Ok(top));
        let mut over = Vec::new();
        put_varint(&mut over, MAX_EPOCH + 1);
        over.push(0x01);
        assert_eq!(
            MsgSeq::decode(&mut Reader::new(&over)),
            Err(ReadError::EpochOverflow)
        );
        assert!(!MsgSeq::new(MAX_EPOCH + 1, 1).is_sequenced());
        assert!(MsgSeq::CONTROL.is_control());
        assert!(!MsgSeq::CONTROL.is_sequenced());
        assert!(!MsgSeq::new(1, 0).is_sequenced());
        assert!(!MsgSeq::new(0, 1).is_sequenced());
        assert_eq!(MsgSeq::new(3, u32::MAX).next(), None);
        assert_eq!(MsgSeq::new(3, 7).next(), Some(MsgSeq::new(3, 8)));
        assert!(MsgSeq::new(2, 1) > MsgSeq::new(1, u32::MAX));
    }

    #[test]
    fn epoch_rule() {
        use EpochVerdict::*;
        assert_eq!(classify_epoch(&[], 5), OpenCurrent { evict: None });
        assert_eq!(classify_epoch(&[5], 5), Existing);
        assert_eq!(classify_epoch(&[5], 9), OpenCurrent { evict: None });
        // custody delivers an earlier, never-seen epoch after a later one
        assert_eq!(classify_epoch(&[9], 5), OpenFrozen { evict: None });
        assert_eq!(classify_epoch(&[2, 3, 4, 9], 3), Existing);
        assert_eq!(
            classify_epoch(&[2, 3, 4, 9], 12),
            OpenCurrent { evict: Some(2) }
        );
        assert_eq!(
            classify_epoch(&[9, 4, 3, 2], 5),
            OpenFrozen { evict: Some(2) }
        );
        assert_eq!(classify_epoch(&[2, 3, 4, 9], 1), Stale);
        assert_eq!(classify_epoch(&[2, 3, 4, 9], 0), Stale);
    }

    #[test]
    fn reader_bounds() {
        let mut r = Reader::new(&[1, 2, 3]);
        assert_eq!(r.take(4), Err(ReadError::Truncated));
        assert_eq!(r.u8(), Ok(1));
        assert_eq!(r.u16_le(), Ok(0x0302));
        assert!(r.is_empty());
        assert_eq!(r.u8(), Err(ReadError::Truncated));
        assert_eq!(r.array::<1>(), Err(ReadError::Truncated));
    }
}
