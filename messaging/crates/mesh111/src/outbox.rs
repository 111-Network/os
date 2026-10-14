//! The outbox side of delivery (§4, §6.1, §9): reason codes for the UI
//! states, the `MSG` body (§3.3), and the §6.1 ACK block built from the
//! store's receive state. The retry engine that drives these lives in
//! [`crate::fabric`]; the per-fragment window is [`crate::frag::TxTracker`].
//! Owner: F1.
//!
//! **States (§9, normative evidence):** `queued` (no path, key missing,
//! their hub is not yours, restore pending) · `sent` (a frame was written to
//! a link, or a custody reject kept it here) · `relayed` (an authenticated
//! `CUSTODY_ACK ACCEPTED` from the home hub for a known member) ·
//! `delivered` (the recipient's own ACK — the only event that releases the
//! sender's copy). Retries happen on link and contact events, never on
//! wall-clock timers.
//!
//! **Never-abandoned `n` (§4):** numbers are allocated only by
//! `Store::queue_own_message`, together with their content; a message the
//! user cancels before it leaves is marked [`Reason::Cancelled`] and goes
//! out as an empty tombstone (a Tier-0 packet with an empty payload), so no
//! permanent hole can wedge the receiver's bounded range list.
//!
//! **ACK ordering (§6.1):** at most 8 ranges across all kept epochs are
//! *selected* in this priority — the range containing the acknowledged
//! packet first, then the current epoch's ranges newest-first, then frozen
//! epochs newest-first (each frozen epoch's delivered prefix travels as the
//! absolute range `[1, prefix]`) — so a retransmitted packet is always
//! acknowledged and a sender can never be made to retransmit forever. The
//! selected set is then written in the codec's canonical ascending order
//! (`ack.rs` rejects any other order; selection, not wire order, carries the
//! priority — recorded as an F1 reading). Partials follow the same rule (the
//! acknowledged packet's message first) and are rebuilt from `rx_snapshot`
//! every time, so a partial the store evicted is never claimed again
//! (`frag.rs` module doc).

use ciborium::value::{Integer, Value};

use crate::ack::{AckBlock, AckRange, PartialAck, StrongReceipt, MAX_RANGES};
use crate::ids::MsgSeq;
use crate::store::RxEpoch;

/// Why an own message sits in its state (§9, §15 `reason_code`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// `queued`: the peer's card has not arrived (§7.2 (2)).
    WaitingForKey,
    /// `queued` (their hub is not yours) or `sent` after a
    /// `REJECT_UNKNOWN_DEST` (§9: "your hub does not know them").
    HubNotYours,
    /// `queued`: the identity holder reports a restore floor; nothing is
    /// sealed (§4).
    RestorePending,
    /// `queued`: the local store refuses new work (§10.4).
    StoreFull,
    /// `sent`: the hub answered `REJECT_FULL` (§9).
    MailboxFull,
    /// `sent`: the hub answered `REJECT_UNENROLLED` (it no longer lists
    /// this node). Not in the C1 contract's emitted set: renders `unknown`
    /// until the contract grows it.
    NotEnrolled,
    /// The user cancelled before it left: an empty tombstone goes instead
    /// (§4 never-abandoned `n`).
    Cancelled,
}

impl Reason {
    /// Every reason.
    pub const ALL: [Reason; 7] = [
        Reason::WaitingForKey,
        Reason::HubNotYours,
        Reason::RestorePending,
        Reason::StoreFull,
        Reason::MailboxFull,
        Reason::NotEnrolled,
        Reason::Cancelled,
    ];

    /// The stable snake_case `reason_code` (§15).
    pub fn code(self) -> &'static str {
        match self {
            Reason::WaitingForKey => "waiting_for_key",
            Reason::HubNotYours => "hub_not_yours",
            Reason::RestorePending => "restore_pending",
            Reason::StoreFull => "store_full",
            Reason::MailboxFull => "mailbox_full",
            Reason::NotEnrolled => "not_enrolled",
            Reason::Cancelled => "cancelled",
        }
    }

    /// Parse a stored code.
    pub fn from_code(code: &str) -> Option<Reason> {
        Reason::ALL.into_iter().find(|r| r.code() == code)
    }

    /// One plain sentence (§9 wording).
    pub fn sentence(self) -> &'static str {
        match self {
            Reason::WaitingForKey => "Waiting for their key.",
            Reason::HubNotYours => {
                "Their hub is not yours; it is delivered when you meet directly."
            }
            Reason::RestorePending => "Nothing is sealed until the identity restore completes.",
            Reason::StoreFull => "The local store is full.",
            Reason::MailboxFull => "Your hub's mailbox is full.",
            Reason::NotEnrolled => "Your hub no longer lists this node.",
            Reason::Cancelled => "Cancelled before it left; an empty placeholder is sent.",
        }
    }
}

/// §3.3 body failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BodyError {
    /// Not one well-formed CBOR item.
    #[error("body is not CBOR")]
    NotCbor,
    /// Not a map with an integer-keyed text at key 1.
    #[error("body shape: {0}")]
    Shape(&'static str),
}

/// §3.3 `MSG` body: deterministic CBOR `{1: text, 2: [refs] (v1.1),
/// 3: sent_at?, 4: reply_to?}`. `sent_at` is the sender's **claim**
/// (shown as "unverified sender time", never used for ordering).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageBody {
    /// The text (peer-authored and untrusted on receipt).
    pub text: String,
    /// The sender's claimed send time (Unix seconds), display only.
    pub sent_at: Option<u64>,
}

impl MessageBody {
    /// A text body.
    pub fn text(text: impl Into<String>) -> Self {
        MessageBody {
            text: text.into(),
            sent_at: None,
        }
    }

    /// Deterministic CBOR (RFC 8949 §4.2: preferred heads, keys ascending).
    pub fn encode(&self) -> Vec<u8> {
        let mut map = vec![(
            Value::Integer(Integer::from(1u8)),
            Value::Text(self.text.clone()),
        )];
        if let Some(t) = self.sent_at {
            map.push((Value::Integer(Integer::from(3u8)), Value::Integer(t.into())));
        }
        let mut out = Vec::with_capacity(self.text.len() + 16);
        // Writing to a Vec cannot fail.
        let _ = ciborium::ser::into_writer(&Value::Map(map), &mut out);
        out
    }

    /// Decode a received body. Key 1 (text) is required; key 3 is read when
    /// it is an unsigned integer; keys 2 and 4 (and unknown keys) are
    /// skipped; a duplicate key is refused.
    pub fn decode(bytes: &[u8]) -> Result<MessageBody, BodyError> {
        let v: Value = ciborium::de::from_reader(bytes).map_err(|_| BodyError::NotCbor)?;
        let Value::Map(entries) = v else {
            return Err(BodyError::Shape("not a map"));
        };
        let mut seen: Vec<u64> = Vec::new();
        let mut text = None;
        let mut sent_at = None;
        for (k, v) in entries {
            let Value::Integer(k) = k else {
                return Err(BodyError::Shape("non-integer key"));
            };
            let k = u64::try_from(k).map_err(|_| BodyError::Shape("negative key"))?;
            if seen.contains(&k) {
                return Err(BodyError::Shape("duplicate key"));
            }
            seen.push(k);
            match (k, v) {
                (1, Value::Text(t)) => text = Some(t),
                (1, _) => return Err(BodyError::Shape("key 1 is not text")),
                (3, Value::Integer(i)) => sent_at = u64::try_from(i).ok(),
                _ => {}
            }
        }
        Ok(MessageBody {
            text: text.ok_or(BodyError::Shape("no text"))?,
            sent_at,
        })
    }
}

fn push_unique<T: PartialEq>(v: &mut Vec<T>, x: T) {
    if !v.contains(&x) {
        v.push(x);
    }
}

fn range_of(e: &RxEpoch, n: u32) -> Option<AckRange> {
    if n == 0 {
        return None;
    }
    if n <= e.prefix {
        return Some(AckRange {
            epoch: e.epoch,
            start: 1,
            len: e.prefix,
        });
    }
    e.ranges
        .iter()
        .find(|(s, l)| n >= *s && u64::from(n) < u64::from(*s) + u64::from(*l))
        .map(|(s, l)| AckRange {
            epoch: e.epoch,
            start: *s,
            len: *l,
        })
}

fn sorted(mut b: AckBlock) -> AckBlock {
    b.ranges.sort_by_key(|r| (r.epoch, r.start));
    b.partials.sort_by_key(|p| p.seq);
    b.strong.sort_by_key(|s| s.seq);
    b
}

fn fits(b: &AckBlock, budget: usize) -> bool {
    sorted(b.clone())
        .encode()
        .is_ok_and(|bytes| bytes.len() <= budget)
}

/// §6.1: the ACK block for one (sender, stream) from the store's receive
/// state (`Store::rx_snapshot`, newest epoch first), within `budget` bytes
/// (the whole block, `ack_len` included). `acked` is the packet being
/// acknowledged (its range or partial is selected first). `strong` lists
/// `RECEIPT_STRONG` hashes the caller holds; only those the block
/// acknowledges are included. `None` when there is no receive state or not
/// even the bare block fits.
pub fn build_ack_block(
    snapshot: &[RxEpoch],
    acked: Option<MsgSeq>,
    strong: &[StrongReceipt],
    budget: usize,
) -> Option<AckBlock> {
    let current = snapshot.first()?;
    let mut block = AckBlock {
        epoch: current.epoch,
        prefix: current.prefix,
        ..AckBlock::default()
    };
    if !fits(&block, budget) {
        return None;
    }
    let acked_epoch = acked.and_then(|a| snapshot.iter().find(|e| e.epoch == a.epoch));

    // Ranges, in §6.1 priority order.
    let mut ranges: Vec<AckRange> = Vec::new();
    if let (Some(a), Some(e)) = (acked, acked_epoch) {
        if let Some(r) = range_of(e, a.n) {
            // The current epoch's prefix is the block header itself.
            if !(e.epoch == current.epoch && a.n <= current.prefix) {
                push_unique(&mut ranges, r);
            }
        }
    }
    for (s, l) in current.ranges.iter().rev() {
        push_unique(
            &mut ranges,
            AckRange {
                epoch: current.epoch,
                start: *s,
                len: *l,
            },
        );
    }
    for e in snapshot.iter().skip(1) {
        if e.prefix > 0 {
            push_unique(
                &mut ranges,
                AckRange {
                    epoch: e.epoch,
                    start: 1,
                    len: e.prefix,
                },
            );
        }
        for (s, l) in e.ranges.iter().rev() {
            push_unique(
                &mut ranges,
                AckRange {
                    epoch: e.epoch,
                    start: *s,
                    len: *l,
                },
            );
        }
    }
    for r in ranges {
        if block.ranges.len() >= MAX_RANGES {
            break;
        }
        block.ranges.push(r);
        if !fits(&block, budget) {
            block.ranges.pop();
        }
    }

    // Partials: the acknowledged packet's message first, then newest-first.
    let mut partials: Vec<PartialAck> = Vec::new();
    if let (Some(a), Some(e)) = (acked, acked_epoch) {
        if let Some(p) = e.partials.iter().find(|p| p.n == a.n) {
            push_unique(
                &mut partials,
                PartialAck {
                    seq: MsgSeq::new(e.epoch, p.n),
                    frag_bitmap: p.bitmap,
                },
            );
        }
    }
    for e in snapshot {
        for p in e.partials.iter().rev() {
            push_unique(
                &mut partials,
                PartialAck {
                    seq: MsgSeq::new(e.epoch, p.n),
                    frag_bitmap: p.bitmap,
                },
            );
        }
    }
    for p in partials {
        if p.frag_bitmap == 0 || block.partials.len() >= usize::from(u8::MAX) {
            continue;
        }
        block.partials.push(p);
        if !fits(&block, budget) {
            block.partials.pop();
        }
    }

    for s in strong {
        if !block.acks(s.seq)
            || block.strong.iter().any(|x| x.seq == s.seq)
            || block.strong.len() >= usize::from(u8::MAX)
        {
            continue;
        }
        block.strong.push(*s);
        if !fits(&block, budget) {
            block.strong.pop();
        }
    }
    let block = sorted(block);
    block.validate().ok()?;
    Some(block)
}

/// §4 `seen_peer_seq`: the highest `msg_seq` delivered from the peer in its
/// newest epoch (prefix or the end of the last range).
pub fn highest_seen(snapshot: &[RxEpoch]) -> Option<MsgSeq> {
    let e = snapshot.first()?;
    let top = e
        .ranges
        .last()
        .map(|(s, l)| s.saturating_add(l.saturating_sub(1)))
        .unwrap_or(0)
        .max(e.prefix);
    (top > 0).then(|| MsgSeq::new(e.epoch, top))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Partial;

    fn ep(epoch: u64, prefix: u32, ranges: &[(u32, u32)], partials: &[(u32, u8, u16)]) -> RxEpoch {
        RxEpoch {
            epoch,
            prefix,
            ranges: ranges.to_vec(),
            partials: partials
                .iter()
                .map(|(n, c, b)| Partial {
                    n: *n,
                    frag_count: *c,
                    bitmap: *b,
                })
                .collect(),
        }
    }

    #[test]
    fn reasons_round_trip() {
        for r in Reason::ALL {
            assert_eq!(Reason::from_code(r.code()), Some(r));
            assert!(!r.sentence().is_empty());
        }
        assert_eq!(Reason::from_code("nope"), None);
    }

    #[test]
    fn body_is_deterministic_cbor() {
        let b = MessageBody::text("hi");
        // {1: "hi"} = a1 01 62 68 69
        assert_eq!(b.encode(), vec![0xa1, 0x01, 0x62, b'h', b'i']);
        assert_eq!(MessageBody::decode(&b.encode()).unwrap(), b);
        let t = MessageBody {
            text: "x".into(),
            sent_at: Some(1_700_000_000),
        };
        assert_eq!(MessageBody::decode(&t.encode()).unwrap(), t);
        assert!(MessageBody::decode(&[]).is_err());
        assert!(MessageBody::decode(&[0x01]).is_err()); // not a map
        assert!(MessageBody::decode(&[0xa1, 0x01, 0x01]).is_err()); // key 1 not text
        assert!(MessageBody::decode(&[0xa2, 0x01, 0x60, 0x01, 0x60]).is_err()); // dup
                                                                                // unknown keys skipped
        assert_eq!(
            MessageBody::decode(&[0xa2, 0x01, 0x61, b'a', 0x09, 0x00])
                .unwrap()
                .text,
            "a"
        );
    }

    #[test]
    fn ack_basic_prefix_and_ranges() {
        let snap = vec![ep(5, 3, &[(5, 2), (9, 1)], &[(12, 4, 0b0101)])];
        let b = build_ack_block(&snap, Some(MsgSeq::new(5, 9)), &[], 1000).unwrap();
        assert_eq!((b.epoch, b.prefix), (5, 3));
        assert_eq!(b.ranges.len(), 2);
        assert!(b.acks(MsgSeq::new(5, 6)));
        assert!(b.acks(MsgSeq::new(5, 9)));
        assert!(!b.acks(MsgSeq::new(5, 4)));
        assert_eq!(b.partials.len(), 1);
        assert_eq!(b.partials[0].frag_bitmap, 0b0101);
        assert!(build_ack_block(&[], None, &[], 1000).is_none());
        assert!(build_ack_block(&snap, None, &[], 2).is_none());
    }

    /// §6.1: a retransmitted packet in an old range is always acknowledged,
    /// even when more than 8 ranges are kept across epochs.
    #[test]
    fn ack_ordering_rule_selects_the_acknowledged_range_first() {
        let cur: Vec<(u32, u32)> = (0..8).map(|k| (10 + 2 * k, 1)).collect();
        let snap = vec![
            ep(9, 1, &cur, &[]),
            ep(7, 4, &[(6, 1), (8, 1)], &[]),
            ep(5, 0, &[(3, 1)], &[]),
        ];
        // Acknowledging the oldest frozen epoch's isolated range.
        let b = build_ack_block(&snap, Some(MsgSeq::new(5, 3)), &[], 4096).unwrap();
        assert_eq!(b.ranges.len(), 8);
        assert!(b.acks(MsgSeq::new(5, 3)), "the acknowledged packet first");
        // Then the current epoch's ranges newest-first: the oldest current
        // range is the one squeezed out.
        assert!(b.acks(MsgSeq::new(9, 24)));
        assert!(!b.acks(MsgSeq::new(9, 10)));
        // A frozen epoch's prefix travels as [1, prefix].
        let c = build_ack_block(&snap, Some(MsgSeq::new(7, 2)), &[], 4096).unwrap();
        assert!(c.acks(MsgSeq::new(7, 2)));
        // Without an acknowledged packet: current newest-first fills all 8.
        let d = build_ack_block(&snap, None, &[], 4096).unwrap();
        assert!((0..8).all(|k| d.acks(MsgSeq::new(9, 10 + 2 * k))));
        assert!(!d.acks(MsgSeq::new(7, 2)));
        // Wire order is canonical ascending (the codec accepts it).
        let bytes = b.encode().unwrap();
        assert_eq!(AckBlock::decode_exact(&bytes).unwrap(), b);
    }

    #[test]
    fn ack_respects_the_byte_budget_and_keeps_the_acked_partial() {
        let partials: Vec<(u32, u8, u16)> = (2..=17).map(|n| (n, 2, 0b01)).collect();
        let snap = vec![ep(1_000_000, 0, &[], &partials)];
        // A 160 B ACK payload budget is about 71 B: not all 16 partials fit.
        let b = build_ack_block(&snap, Some(MsgSeq::new(1_000_000, 2)), &[], 71).unwrap();
        assert!(b.encode().unwrap().len() <= 71);
        assert!(b.partials.len() < 16);
        assert!(
            b.partials.iter().any(|p| p.seq.n == 2),
            "the acknowledged packet's partial is listed"
        );
    }

    #[test]
    fn strong_receipts_only_for_acknowledged() {
        let snap = vec![ep(2, 5, &[], &[])];
        let s = [
            StrongReceipt {
                seq: MsgSeq::new(2, 4),
                payload_hash: [1; 16],
            },
            StrongReceipt {
                seq: MsgSeq::new(2, 9),
                payload_hash: [2; 16],
            },
        ];
        let b = build_ack_block(&snap, None, &s, 1000).unwrap();
        assert_eq!(b.strong.len(), 1);
        assert_eq!(b.strong[0].seq, MsgSeq::new(2, 4));
    }

    #[test]
    fn highest_seen_reads_the_newest_epoch() {
        assert_eq!(highest_seen(&[]), None);
        assert_eq!(highest_seen(&[ep(3, 0, &[], &[])]), None);
        assert_eq!(
            highest_seen(&[ep(3, 2, &[(5, 3)], &[])]),
            Some(MsgSeq::new(3, 7))
        );
    }
}
