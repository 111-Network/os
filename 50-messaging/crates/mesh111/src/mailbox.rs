//! §9 — the mailbox protocol's payloads and per-connection push state, and
//! §7.3 enrollment payloads. Owner: F1. The flows (member and hub) run in
//! [`crate::fabric`]; this module is the pure part.
//!
//! ```text
//! CUSTODY_PUT    (member → hub)  dest NodeId(32) ‖ sealed item bytes
//! CUSTODY_ACK    (hub → member)  §6.2 (custody.rs)
//! MAILBOX_FETCH  (member → hub)  cursor(varint)
//! MAILBOX_ITEM   (hub → member)  custody_seq(varint) ‖ sealed item bytes
//! MAILBOX_COMMIT (member → hub)  cursor(varint)
//! ENROLL         (member → hub)  code(8)            -- INTRO set; the card rides the §3 slot
//! ENROLL_OK      (hub → member)  status(1) ‖ [card_len(1) ‖ card ‖ custody_seq_floor(varint)]
//! ```
//!
//! Readings recorded for the lead (the spec names the fields, not the
//! bytes): every payload is exactly the fields above, nothing after them
//! (trailing bytes ⇒ reject). `ENROLL`'s `{code}` is the 8 canonical symbols.
//! `ENROLL_OK` carries a leading status byte so a refusal can be answered
//! on the same kind (the registry has no `ENROLL_REJECT`): `0` OK (card and
//! floor follow), `1` mismatch, `2` refused (no open offer: none, spent by
//! someone else, or dismissed), `3` locked out after 5 failures — the
//! contract surfaces 3 as `429` and 1–2 as `502`.
//!
//! Hub push window: per attributed connection the hub keeps the highest
//! custody sequence it has streamed and the sequences not yet committed;
//! it streams at most [`PUSH_WINDOW`] uncommitted items, then waits for the
//! member's `MAILBOX_COMMIT` (flow control on the member's acknowledgement,
//! never a timer). A `MAILBOX_FETCH {cursor}` restarts the stream at that
//! cursor.

use std::collections::VecDeque;

use id111::keyset::NodeId;

use crate::enroll::{parse_code, Code, EnrollError, CODE_ALPHABET, CODE_LEN, SECRET_LEN};
use crate::envelope::{MAX_PACKET_LEN, MIN_PACKET_LEN};
use crate::ids::{put_varint, ReadError, Reader};
use crate::sealer::{Sealer, SealerError};

/// Uncommitted items the hub streams per connection before it waits for a
/// `MAILBOX_COMMIT`.
pub const PUSH_WINDOW: usize = 64;

/// Mailbox payload failures (each drops the packet; nothing is stored).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MailboxError {
    /// Truncated or non-minimal fields.
    #[error("mailbox payload: {0}")]
    Read(#[from] ReadError),
    /// Bytes after the last field.
    #[error("trailing bytes")]
    Trailing,
    /// A custody item outside the sealed-packet size bounds.
    #[error("item of {0} B is not a sealed packet")]
    ItemSize(usize),
    /// An `ENROLL_OK` status byte outside 0–3.
    #[error("unknown ENROLL_OK status {0}")]
    Status(u8),
    /// A card length outside 137–169.
    #[error("card length {0}")]
    CardLength(usize),
}

fn check_item(item: &[u8]) -> Result<(), MailboxError> {
    if (MIN_PACKET_LEN..=MAX_PACKET_LEN).contains(&item.len()) {
        Ok(())
    } else {
        Err(MailboxError::ItemSize(item.len()))
    }
}

/// `CUSTODY_PUT` payload.
pub fn encode_put(dest: &NodeId, item: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + item.len());
    out.extend_from_slice(dest.as_bytes());
    out.extend_from_slice(item);
    out
}

/// Parse a `CUSTODY_PUT` payload: `dest` and the item (size-checked).
pub fn decode_put(payload: &[u8]) -> Result<(NodeId, &[u8]), MailboxError> {
    let mut r = Reader::new(payload);
    let dest = NodeId::from(r.array::<32>()?);
    let item = r.rest();
    check_item(item)?;
    Ok((dest, item))
}

/// A one-varint payload (`MAILBOX_FETCH` / `MAILBOX_COMMIT`).
pub fn encode_cursor(cursor: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(10);
    put_varint(&mut out, cursor);
    out
}

/// Parse a one-varint payload exactly.
pub fn decode_cursor(payload: &[u8]) -> Result<u64, MailboxError> {
    let mut r = Reader::new(payload);
    let c = r.varint()?;
    if !r.is_empty() {
        return Err(MailboxError::Trailing);
    }
    Ok(c)
}

/// `MAILBOX_ITEM` payload.
pub fn encode_item(custody_seq: u64, item: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(10 + item.len());
    put_varint(&mut out, custody_seq);
    out.extend_from_slice(item);
    out
}

/// Parse a `MAILBOX_ITEM` payload.
pub fn decode_item(payload: &[u8]) -> Result<(u64, &[u8]), MailboxError> {
    let mut r = Reader::new(payload);
    let seq = r.varint()?;
    let item = r.rest();
    check_item(item)?;
    Ok((seq, item))
}

/// `ENROLL` payload: the canonical code.
pub fn encode_enroll(code: &Code) -> Vec<u8> {
    code.to_vec()
}

/// `ENROLL` payload bytes (strict parsing is `Offer::present`'s).
pub fn decode_enroll(payload: &[u8]) -> &[u8] {
    payload
}

/// `ENROLL_OK` status (module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollStatus {
    /// Enrolled (or re-confirmed).
    Ok = 0,
    /// Wrong code (counted toward lockout).
    Mismatch = 1,
    /// No open offer (none, spent by someone else, dismissed).
    Refused = 2,
    /// Locked after 5 failures.
    LockedOut = 3,
}

impl EnrollStatus {
    fn from_byte(b: u8) -> Option<EnrollStatus> {
        Some(match b {
            0 => EnrollStatus::Ok,
            1 => EnrollStatus::Mismatch,
            2 => EnrollStatus::Refused,
            3 => EnrollStatus::LockedOut,
            _ => return None,
        })
    }

    /// The hub's answer for an `Offer::present` failure.
    pub fn from_error(e: &EnrollError) -> EnrollStatus {
        match e {
            EnrollError::Length(_) | EnrollError::Symbol(_) | EnrollError::Mismatch { .. } => {
                EnrollStatus::Mismatch
            }
            EnrollError::LockedOut => EnrollStatus::LockedOut,
            EnrollError::Spent | EnrollError::Dismissed | EnrollError::MacKey(_) => {
                EnrollStatus::Refused
            }
        }
    }
}

/// `ENROLL_OK` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollOk {
    /// Status.
    pub status: EnrollStatus,
    /// The hub's card (status OK only).
    pub card: Option<Vec<u8>>,
    /// The hub's custody sequence at enrollment: the member's cursor starts
    /// there (status OK only; 0 otherwise).
    pub custody_seq_floor: u64,
}

impl EnrollOk {
    /// A refusal.
    pub fn refusal(status: EnrollStatus) -> EnrollOk {
        EnrollOk {
            status,
            card: None,
            custody_seq_floor: 0,
        }
    }

    /// Encode.
    pub fn encode(&self) -> Result<Vec<u8>, MailboxError> {
        let mut out = vec![self.status as u8];
        if self.status == EnrollStatus::Ok {
            let card = self.card.as_deref().unwrap_or(&[]);
            if !(id111::announce::BASE_LEN..=id111::announce::MAX_LEN).contains(&card.len()) {
                return Err(MailboxError::CardLength(card.len()));
            }
            out.push(card.len() as u8);
            out.extend_from_slice(card);
            put_varint(&mut out, self.custody_seq_floor);
        }
        Ok(out)
    }

    /// Decode (exactly; trailing bytes refused).
    pub fn decode(payload: &[u8]) -> Result<EnrollOk, MailboxError> {
        let mut r = Reader::new(payload);
        let b = r.u8()?;
        let status = EnrollStatus::from_byte(b).ok_or(MailboxError::Status(b))?;
        let out = if status == EnrollStatus::Ok {
            let len = usize::from(r.u8()?);
            if !(id111::announce::BASE_LEN..=id111::announce::MAX_LEN).contains(&len) {
                return Err(MailboxError::CardLength(len));
            }
            let card = r.take(len)?.to_vec();
            let floor = r.varint()?;
            EnrollOk {
                status,
                card: Some(card),
                custody_seq_floor: floor,
            }
        } else {
            EnrollOk::refusal(status)
        };
        if !r.is_empty() {
            return Err(MailboxError::Trailing);
        }
        Ok(out)
    }
}

/// Normalize a code as a person typed it (the L4 pair-code rule: dashes,
/// spaces and case ignored; `O`→`0`, `I`/`L`→`1`), then parse strictly.
pub fn normalize_code(input: &str) -> Result<Code, EnrollError> {
    if input.len() > 64 {
        return Err(EnrollError::Length(input.len()));
    }
    let mut out = Vec::with_capacity(CODE_LEN);
    for c in input.chars() {
        let c = match c.to_ascii_uppercase() {
            '-' | ' ' | '\t' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !c.is_ascii() {
            return Err(EnrollError::Symbol(0xff));
        }
        out.push(c as u8);
    }
    parse_code(&out)
}

/// Mint a one-time code (8 symbols, unbiased by rejection sampling) and its
/// 32 B `code_secret` from the handle's entropy (§7.3).
pub fn mint_code(sealer: &dyn Sealer) -> Result<(Code, [u8; SECRET_LEN]), SealerError> {
    let mut code = [0u8; CODE_LEN];
    let mut n = 0;
    let limit = 256 - 256 % CODE_ALPHABET.len(); // 240: unbiased
    while n < CODE_LEN {
        let mut buf = [0u8; 32];
        sealer.random(&mut buf)?;
        for b in buf {
            if usize::from(b) < limit && n < CODE_LEN {
                code[n] = CODE_ALPHABET[usize::from(b) % CODE_ALPHABET.len()];
                n += 1;
            }
        }
    }
    let mut secret = [0u8; SECRET_LEN];
    sealer.random(&mut secret)?;
    Ok((code, secret))
}

/// The hub's per-connection stream state (module doc).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushWindow {
    /// Highest custody sequence streamed (or the fetch cursor).
    pub high: u64,
    /// Streamed, not yet committed.
    pub inflight: VecDeque<u64>,
    /// The member has fetched on this connection.
    pub active: bool,
}

impl PushWindow {
    /// A `MAILBOX_FETCH {cursor}`: restart the stream at `cursor`.
    pub fn fetch(&mut self, cursor: u64) {
        self.high = cursor;
        self.inflight.clear();
        self.active = true;
    }

    /// Room for this many more items.
    pub fn room(&self) -> usize {
        PUSH_WINDOW.saturating_sub(self.inflight.len())
    }

    /// Record a streamed item.
    pub fn pushed(&mut self, custody_seq: u64) {
        self.high = self.high.max(custody_seq);
        self.inflight.push_back(custody_seq);
    }

    /// A streamed item's frame was not written (a send error): it and
    /// everything after it leave the window, and the stream restarts just
    /// below it (items go out in order, with no gaps — the member's cursor
    /// is a high-water mark).
    pub fn unpush(&mut self, custody_seq: u64) {
        self.inflight.retain(|s| *s < custody_seq);
        self.high = self.high.min(custody_seq.saturating_sub(1));
    }

    /// A `MAILBOX_COMMIT {cursor}`: everything ≤ cursor leaves the window.
    pub fn commit(&mut self, cursor: u64) {
        self.inflight.retain(|s| *s > cursor);
        self.high = self.high.max(cursor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_item_cursor_round_trip() {
        let dest = NodeId::from([3; 32]);
        let item = vec![1u8; MIN_PACKET_LEN];
        let p = encode_put(&dest, &item);
        let (d, i) = decode_put(&p).unwrap();
        assert_eq!((d, i), (dest, &item[..]));
        assert!(matches!(
            decode_put(&p[..40]),
            Err(MailboxError::ItemSize(8))
        ));
        assert!(decode_put(&[0; 10]).is_err());
        let it = encode_item(300, &item);
        assert_eq!(decode_item(&it).unwrap(), (300, &item[..]));
        assert_eq!(decode_cursor(&encode_cursor(0)).unwrap(), 0);
        assert_eq!(decode_cursor(&encode_cursor(u64::MAX)).unwrap(), u64::MAX);
        assert_eq!(decode_cursor(&[0x01, 0x00]), Err(MailboxError::Trailing));
        assert!(decode_cursor(&[0x80, 0x00]).is_err()); // non-minimal
        assert!(decode_cursor(&[]).is_err());
    }

    #[test]
    fn enroll_ok_round_trip_and_refusals() {
        let ok = EnrollOk {
            status: EnrollStatus::Ok,
            card: Some(vec![7; 137]),
            custody_seq_floor: 42,
        };
        let b = ok.encode().unwrap();
        assert_eq!(EnrollOk::decode(&b).unwrap(), ok);
        let mut t = b.clone();
        t.push(0);
        assert_eq!(EnrollOk::decode(&t), Err(MailboxError::Trailing));
        for s in [
            EnrollStatus::Mismatch,
            EnrollStatus::Refused,
            EnrollStatus::LockedOut,
        ] {
            let r = EnrollOk::refusal(s);
            assert_eq!(EnrollOk::decode(&r.encode().unwrap()).unwrap(), r);
        }
        assert_eq!(EnrollOk::decode(&[9]), Err(MailboxError::Status(9)));
        assert!(EnrollOk {
            card: Some(vec![0; 10]),
            ..ok
        }
        .encode()
        .is_err());
        assert_eq!(
            EnrollStatus::from_error(&EnrollError::LockedOut),
            EnrollStatus::LockedOut
        );
        assert_eq!(
            EnrollStatus::from_error(&EnrollError::Mismatch { failures_left: 2 }),
            EnrollStatus::Mismatch
        );
    }

    #[test]
    fn codes_normalize_like_the_pair_code() {
        assert_eq!(&normalize_code("0123-bcdf").unwrap(), b"0123BCDF");
        assert_eq!(&normalize_code("o1l2 i3BC").unwrap(), b"011213BC");
        assert!(normalize_code("0123BCD").is_err());
        assert!(normalize_code("0123BCDA").is_err()); // vowel
        assert!(normalize_code("0123BCDÉ").is_err());
    }

    #[test]
    fn push_window_flow_control() {
        let mut w = PushWindow::default();
        w.fetch(10);
        assert_eq!(w.room(), PUSH_WINDOW);
        for s in 11..11 + PUSH_WINDOW as u64 {
            w.pushed(s);
        }
        assert_eq!(w.room(), 0);
        w.commit(20);
        assert_eq!(w.room(), 10);
        assert_eq!(w.high, 10 + PUSH_WINDOW as u64);
        w.fetch(5);
        assert_eq!((w.high, w.room()), (5, PUSH_WINDOW));
        // A send error on item 8: 8 and later leave, the stream restarts at 7.
        for s in 6..=9 {
            w.pushed(s);
        }
        w.unpush(8);
        assert_eq!(w.high, 7);
        assert_eq!(w.inflight.iter().copied().collect::<Vec<_>>(), vec![6, 7]);
        w.unpush(1);
        assert_eq!((w.high, w.inflight.len()), (0, 0));
    }
}
