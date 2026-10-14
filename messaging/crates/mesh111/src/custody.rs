//! §6.2 — the custody receipt (`kind = CUSTODY_ACK`) and `custody-sig`.
//! Owner: C2.
//!
//! ```text
//! CUSTODY_ACK payload = status(1) ‖ custody_seq(varint) ‖ pkt_ref(16) ‖ [custody_sig(64)]
//! pkt_ref     = BLAKE2b-128("111/custody/v1" ‖ exact sealed bytes of the item)
//! custody_sig = Ed25519("111/custody-sig/v1" ‖ hub(32) ‖ dest(32) ‖ status(1)
//!                       ‖ custody_seq(varint) ‖ pkt_ref(16))
//! ```
//!
//! Hashing the exact bytes proves the custodian **received** them. `ACCEPTED`
//! from a mailbox-role node for a known member is the only evidence for the
//! UI state `relayed` (§9) — that judgement is the caller's; this module only
//! encodes, decodes and verifies.
//!
//! Readings this codec pins (C2 deviations, fail closed):
//! - the transferable `custody_sig` rides as the last 64 B of the payload,
//!   like §6.1's `ack_sig` (the spec says "on request" without a placement);
//!   a tail that is neither empty nor 64 B is rejected;
//! - `custody_seq` is the hub's insertion sequence (≥ 1) for `ACCEPTED` and
//!   `DUPLICATE` (the held item's seq) and **0** for every `REJECT_*` (nothing
//!   was inserted);
//! - `custody_seq` is a varint inside the signed input too, as in the
//!   payload.

use id111::keyset::{verify_strict_tagged, NodeId, VerifyError};

use crate::envelope::blake2b_128;
use crate::ids::{put_varint, ReadError, Reader};
use crate::sealer::Sealer;

/// §3.1 `pkt_ref` domain tag.
pub const CUSTODY_TAG: &[u8] = b"111/custody/v1";
/// §3.1 transferable custody receipt domain tag.
pub const CUSTODY_SIG_TAG: &[u8] = b"111/custody-sig/v1";
/// `pkt_ref` length.
pub const PKT_REF_LEN: usize = 16;
/// `custody_sig` length.
pub const CUSTODY_SIG_LEN: usize = 64;

/// §6.2 statuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CustodyStatus {
    /// 1 — held for a member.
    Accepted = 1,
    /// 2 — already held (`pkt_ref` dedup).
    Duplicate = 2,
    /// 3 — the store refuses new custody (§10.4): "mailbox full".
    RejectFull = 3,
    /// 4 — the putting node is not enrolled (§7.3).
    RejectUnenrolled = 4,
    /// 5 — `dest` is not a member (§9): "your hub does not know them".
    RejectUnknownDest = 5,
}

impl CustodyStatus {
    /// Wire byte → status; anything outside 1–5 fails closed.
    pub fn from_byte(b: u8) -> Option<CustodyStatus> {
        Some(match b {
            1 => CustodyStatus::Accepted,
            2 => CustodyStatus::Duplicate,
            3 => CustodyStatus::RejectFull,
            4 => CustodyStatus::RejectUnenrolled,
            5 => CustodyStatus::RejectUnknownDest,
            _ => return None,
        })
    }

    /// The wire byte.
    pub fn byte(self) -> u8 {
        self as u8
    }

    /// `ACCEPTED` or `DUPLICATE`: the custodian holds the item.
    pub fn is_held(self) -> bool {
        matches!(self, CustodyStatus::Accepted | CustodyStatus::Duplicate)
    }
}

/// Custody codec failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CustodyError {
    /// Truncation or a non-minimal varint.
    #[error("custody receipt: {0}")]
    Read(#[from] ReadError),
    /// Status byte outside 1–5.
    #[error("unknown custody status {0}")]
    UnknownStatus(u8),
    /// `custody_seq` inconsistent with the status (module doc).
    #[error("custody_seq {seq} invalid for {status:?}")]
    BadSeq {
        /// The status.
        status: CustodyStatus,
        /// The seq.
        seq: u64,
    },
    /// Payload tail neither empty nor one 64 B signature.
    #[error("CUSTODY_ACK tail is {0} B (0 or 64 allowed)")]
    BadTail(usize),
    /// `custody_sig` failed the strict Ed25519 predicate.
    #[error("custody_sig: {0}")]
    BadSignature(VerifyError),
}

/// `pkt_ref = BLAKE2b-128("111/custody/v1" ‖ item)` (unkeyed, 16 B digest).
pub fn pkt_ref(item: &[u8]) -> [u8; PKT_REF_LEN] {
    blake2b_128(&[CUSTODY_TAG, item])
}

/// The §6.2 receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustodyReceipt {
    /// Status.
    pub status: CustodyStatus,
    /// Hub insertion sequence (0 for rejects).
    pub custody_seq: u64,
    /// [`pkt_ref`] of the item.
    pub pkt_ref: [u8; PKT_REF_LEN],
}

impl CustodyReceipt {
    /// Check the status / `custody_seq` pairing.
    pub fn validate(&self) -> Result<(), CustodyError> {
        let ok = if self.status.is_held() {
            self.custody_seq >= 1
        } else {
            self.custody_seq == 0
        };
        if ok {
            Ok(())
        } else {
            Err(CustodyError::BadSeq {
                status: self.status,
                seq: self.custody_seq,
            })
        }
    }

    /// `status ‖ custody_seq ‖ pkt_ref`.
    pub fn encode(&self) -> Result<Vec<u8>, CustodyError> {
        self.validate()?;
        let mut out = Vec::with_capacity(1 + 10 + PKT_REF_LEN);
        out.push(self.status.byte());
        put_varint(&mut out, self.custody_seq);
        out.extend_from_slice(&self.pkt_ref);
        Ok(out)
    }

    fn decode_from(r: &mut Reader<'_>) -> Result<CustodyReceipt, CustodyError> {
        let b = r.u8()?;
        let status = CustodyStatus::from_byte(b).ok_or(CustodyError::UnknownStatus(b))?;
        let custody_seq = r.varint()?;
        let pkt_ref = r.array::<PKT_REF_LEN>()?;
        let rec = CustodyReceipt {
            status,
            custody_seq,
            pkt_ref,
        };
        rec.validate()?;
        Ok(rec)
    }
}

/// §6.2 `custody_sig` signed input **without** the domain tag:
/// `hub(32) ‖ dest(32) ‖ status(1) ‖ custody_seq(varint) ‖ pkt_ref(16)`.
pub fn custody_sig_input(hub: &NodeId, dest: &NodeId, rec: &CustodyReceipt) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + 32 + 1 + 10 + PKT_REF_LEN);
    out.extend_from_slice(hub.as_bytes());
    out.extend_from_slice(dest.as_bytes());
    out.push(rec.status.byte());
    put_varint(&mut out, rec.custody_seq);
    out.extend_from_slice(&rec.pkt_ref);
    out
}

/// A `CUSTODY_ACK` payload: receipt ‖ [`custody_sig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyAck {
    /// The receipt.
    pub receipt: CustodyReceipt,
    /// The transferable signature, when requested.
    pub custody_sig: Option<[u8; 64]>,
}

impl CustodyAck {
    /// Build (hub side; hub = this node), signing when `transferable`.
    pub fn build(
        sealer: &dyn Sealer,
        receipt: CustodyReceipt,
        dest: &NodeId,
        transferable: bool,
    ) -> Result<CustodyAck, CustodyError> {
        receipt.validate()?;
        let custody_sig = transferable.then(|| {
            sealer.sign(
                CUSTODY_SIG_TAG,
                &custody_sig_input(&sealer.node_id(), dest, &receipt),
            )
        });
        Ok(CustodyAck {
            receipt,
            custody_sig,
        })
    }

    /// The payload bytes.
    pub fn encode(&self) -> Result<Vec<u8>, CustodyError> {
        let mut out = self.receipt.encode()?;
        if let Some(s) = &self.custody_sig {
            out.extend_from_slice(s);
        }
        Ok(out)
    }

    /// Parse a `CUSTODY_ACK` payload.
    pub fn decode(payload: &[u8]) -> Result<CustodyAck, CustodyError> {
        let mut r = Reader::new(payload);
        let receipt = CustodyReceipt::decode_from(&mut r)?;
        let tail = r.rest();
        let custody_sig = match tail.len() {
            0 => None,
            CUSTODY_SIG_LEN => {
                let mut s = [0u8; 64];
                s.copy_from_slice(tail);
                Some(s)
            }
            n => return Err(CustodyError::BadTail(n)),
        };
        Ok(CustodyAck {
            receipt,
            custody_sig,
        })
    }

    /// Verify the transferable signature (if any) as `hub` for `dest`.
    /// `Ok(false)` when unsigned.
    pub fn verify(&self, hub: &NodeId, dest: &NodeId) -> Result<bool, CustodyError> {
        match &self.custody_sig {
            None => Ok(false),
            Some(sig) => verify_strict_tagged(
                hub,
                CUSTODY_SIG_TAG,
                &custody_sig_input(hub, dest, &self.receipt),
                sig,
            )
            .map(|()| true)
            .map_err(CustodyError::BadSignature),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_double::{TestSealer, MNEMONIC_A, MNEMONIC_B};

    fn rec(status: CustodyStatus, custody_seq: u64) -> CustodyReceipt {
        CustodyReceipt {
            status,
            custody_seq,
            pkt_ref: pkt_ref(b"sealed item"),
        }
    }

    #[test]
    fn pkt_ref_is_domain_tagged_and_exact() {
        assert_ne!(pkt_ref(b"a"), pkt_ref(b"b"));
        assert_ne!(pkt_ref(b""), crate::envelope::blake2b_128(&[b""]));
        assert_eq!(
            pkt_ref(b"x"),
            crate::envelope::blake2b_128(&[CUSTODY_TAG, b"x"])
        );
    }

    #[test]
    fn status_seq_pairing() {
        for s in [CustodyStatus::Accepted, CustodyStatus::Duplicate] {
            assert!(rec(s, 1).validate().is_ok());
            assert_eq!(
                rec(s, 0).validate(),
                Err(CustodyError::BadSeq { status: s, seq: 0 })
            );
        }
        for s in [
            CustodyStatus::RejectFull,
            CustodyStatus::RejectUnenrolled,
            CustodyStatus::RejectUnknownDest,
        ] {
            assert!(rec(s, 0).validate().is_ok());
            assert!(rec(s, 7).validate().is_err());
        }
        assert_eq!(CustodyStatus::from_byte(0), None);
        assert_eq!(CustodyStatus::from_byte(6), None);
    }

    #[test]
    fn round_trip_and_transferable_sig() {
        let hub = TestSealer::from_words(MNEMONIC_A);
        let dest = TestSealer::from_words(MNEMONIC_B);
        let hub_id = hub.node_id_value();
        let dest_id = dest.node_id_value();
        let plain =
            CustodyAck::build(&hub, rec(CustodyStatus::Accepted, 300), &dest_id, false).unwrap();
        let bytes = plain.encode().unwrap();
        assert_eq!(bytes.len(), 1 + 2 + PKT_REF_LEN);
        assert_eq!(CustodyAck::decode(&bytes).unwrap(), plain);
        assert_eq!(plain.verify(&hub_id, &dest_id), Ok(false));

        let signed =
            CustodyAck::build(&hub, rec(CustodyStatus::Duplicate, 9), &dest_id, true).unwrap();
        let bytes = signed.encode().unwrap();
        let back = CustodyAck::decode(&bytes).unwrap();
        assert_eq!(back.verify(&hub_id, &dest_id), Ok(true));
        // bound to hub, dest and every receipt field
        assert!(back.verify(&dest_id, &hub_id).is_err());
        let mut moved = back.clone();
        moved.receipt.custody_seq = 10;
        assert!(moved.verify(&hub_id, &dest_id).is_err());
        let mut tail = bytes.clone();
        tail.pop();
        assert_eq!(CustodyAck::decode(&tail), Err(CustodyError::BadTail(63)));
    }
}
