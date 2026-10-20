//! §7.3 — the enrollment code keyed hash (`111/enroll/v1`) and the pure
//! single-use / lockout bookkeeping of one offer. Owner: C2 (hash, this
//! file) / F1 (the ENROLL flow around it: QR, storage, ENROLL_OK).
//!
//! The hub shows a one-time 8-char code from the L4 pair-code generator
//! (`node111::daemon::dashboard::api_v1::auth`, D15: alphabet
//! `0123456789BCDFGHJKMNPQRSTVWXYZ`, display `XXXX-XXXX`). The member sends
//! `ENROLL {code}` sealed with INTRO; the hub verifies
//! `BLAKE2b-128(key = code_secret, data = "111/enroll/v1" ‖ member NodeId ‖
//! code)` in constant time.
//!
//! Reading of `code_secret` (C2 deviation, recorded for the lead): the spec
//! names it once and never defines it. Here it is the offer's own random
//! 32 B key, drawn by the hub next to the code and never shown; the hub
//! keeps `(code, code_secret)` for the offer's lifetime and compares
//! `enroll_hash(code_secret, member, presented)` with
//! `enroll_hash(code_secret, member, offered)` in constant time
//! ([`Offer::present`]). Binding the member NodeId into the hash means a
//! code captured for one identity proves nothing for another. The keyed
//! hash is the frozen G4 vector (`test-vectors/enroll.json`); the storage
//! shape is F1's.
//!
//! Code rules (mirroring the pair code, D15): single-use, invalidated when
//! the owner dismisses the offer, attempt lockout after 5 failures. ENROLL
//! is a control kind and may be retransmitted, so a spent offer remembers
//! who consumed it: the same member re-presenting the same code gets
//! [`Presented::AlreadyEnrolled`] (the hub re-sends ENROLL_OK); anyone
//! else gets [`EnrollError::Spent`]. The
//! wire `code` is the canonical 8 symbols (upper-case, no separator);
//! normalizing what a person typed (dashes, case, `O`→`0`) is the UI's job
//! (L4 `normalize_code`), and this parser is strict — the frozen vectors
//! (`test-vectors/enroll.json`, `reject_code`) pin its rejections.

use id111::keyset::NodeId;
use zeroize::Zeroizing;

use crate::envelope::{keyed_blake2b_128, keyed_blake2b_128_verify, MacKeyLength};

/// §7.3 domain tag.
pub const ENROLL_TAG: &[u8] = b"111/enroll/v1";
/// Symbols in a code.
pub const CODE_LEN: usize = 8;
/// The L4 pair-code alphabet (30 symbols: digits + consonants, no vowels, no
/// read-alikes `I`/`L`/`O`).
pub const CODE_ALPHABET: &[u8; 30] = b"0123456789BCDFGHJKMNPQRSTVWXYZ";
/// Failed presentations before the offer locks (D15).
pub const MAX_FAILURES: u32 = 5;
/// Bytes of `code_secret`.
pub const SECRET_LEN: usize = 32;
/// Bytes of the keyed hash.
pub const ENROLL_HASH_LEN: usize = 16;

/// A canonical code: 8 alphabet symbols, upper-case, no separator.
pub type Code = [u8; CODE_LEN];

/// Strict parse of a wire code: exactly 8 bytes, every one in
/// [`CODE_ALPHABET`]. Length is checked first; the first offending byte is
/// named (`Symbol(b)`); nothing is normalized.
pub fn parse_code(code: &[u8]) -> Result<Code, EnrollError> {
    if code.len() != CODE_LEN {
        return Err(EnrollError::Length(code.len()));
    }
    if let Some(&b) = code.iter().find(|b| !CODE_ALPHABET.contains(b)) {
        return Err(EnrollError::Symbol(b));
    }
    let mut out = [0u8; CODE_LEN];
    out.copy_from_slice(code);
    Ok(out)
}

/// `BLAKE2b-128(key = code_secret, data = "111/enroll/v1" ‖ member ‖ code)`,
/// keyed BLAKE2b in MAC mode (RFC 7693 key block). The 32 B secret always
/// fits the key block; the `Err` arm is the MAC's typed refusal, never a
/// zero tag.
pub fn enroll_hash(
    code_secret: &[u8; SECRET_LEN],
    member: &NodeId,
    code: &Code,
) -> Result<[u8; ENROLL_HASH_LEN], EnrollError> {
    Ok(keyed_blake2b_128(
        code_secret,
        &[ENROLL_TAG, member.as_bytes(), code],
    )?)
}

/// Constant-time check of `presented` against `expected`
/// (`Mac::verify_slice`, the same primitive `pair_mac` uses).
pub fn verify_enroll_hash(
    code_secret: &[u8; SECRET_LEN],
    member: &NodeId,
    code: &Code,
    expected: &[u8; ENROLL_HASH_LEN],
) -> bool {
    keyed_blake2b_128_verify(
        code_secret,
        &[ENROLL_TAG, member.as_bytes(), code],
        expected,
    )
}

/// Why a presentation was refused. Every variant leaves the offer in the
/// state the variant names; none reveals the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EnrollError {
    /// Not 8 bytes (not counted as a failure: it never reached the hash).
    #[error("enrollment code is {0} bytes (8)")]
    Length(usize),
    /// A byte outside the alphabet (not counted as a failure).
    #[error("enrollment code byte 0x{0:02x} outside the alphabet")]
    Symbol(u8),
    /// Wrong code; `failures_left` more before lockout.
    #[error("enrollment code mismatch ({failures_left} attempts left)")]
    Mismatch {
        /// Attempts remaining before [`OfferState::LockedOut`].
        failures_left: u32,
    },
    /// Already used once.
    #[error("enrollment code already used")]
    Spent,
    /// The owner dismissed the offer.
    #[error("enrollment offer dismissed")]
    Dismissed,
    /// Five failures: the owner must mint a new offer.
    #[error("enrollment offer locked after {MAX_FAILURES} failures")]
    LockedOut,
    /// The keyed hash refused its key (unreachable for a 32 B secret).
    #[error("enrollment hash: {0}")]
    MacKey(#[from] MacKeyLength),
}

/// A successful presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presented {
    /// First success: the offer is now spent by this member.
    Enrolled,
    /// The consuming member presented the same code again (a retransmitted
    /// ENROLL): re-send ENROLL_OK, change nothing.
    AlreadyEnrolled,
}

/// Where an offer stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferState {
    /// Accepting presentations.
    Open,
    /// Consumed by a successful enrollment (single-use).
    Spent,
    /// Dismissed by the owner.
    Dismissed,
    /// Locked after [`MAX_FAILURES`] mismatches.
    LockedOut,
}

/// One enrollment offer: the code, its secret, and the single-use / lockout
/// state. Pure — no clock, no IO; F1 persists it and times it out.
#[derive(Debug)]
pub struct Offer {
    code: Code,
    secret: Zeroizing<[u8; SECRET_LEN]>,
    failures: u32,
    state: OfferState,
    /// The member that spent the offer.
    consumer: Option<NodeId>,
}

impl Offer {
    /// A fresh offer from a generated code and a random secret.
    pub fn new(code: Code, code_secret: [u8; SECRET_LEN]) -> Offer {
        Offer {
            code,
            secret: Zeroizing::new(code_secret),
            failures: 0,
            state: OfferState::Open,
            consumer: None,
        }
    }

    /// The code to show (canonical 8 symbols).
    pub fn code(&self) -> &Code {
        &self.code
    }

    /// Display form `XXXX-XXXX` (byte-wise: never slices a `str`).
    pub fn display(&self) -> String {
        format!(
            "{}-{}",
            String::from_utf8_lossy(&self.code[..4]),
            String::from_utf8_lossy(&self.code[4..])
        )
    }

    /// Current state.
    pub fn state(&self) -> OfferState {
        self.state
    }

    /// Mismatches counted so far.
    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// The member that consumed the offer (`Some` once spent).
    pub fn consumer(&self) -> Option<&NodeId> {
        self.consumer.as_ref()
    }

    /// Does `code` (already parsed) hash to this offer's code for `member`?
    /// Constant time in the code bytes.
    fn matches(&self, member: &NodeId, presented: &Code) -> Result<bool, EnrollError> {
        let expected = enroll_hash(&self.secret, member, &self.code)?;
        Ok(verify_enroll_hash(
            &self.secret,
            member,
            presented,
            &expected,
        ))
    }

    /// The owner dismissed the offer (idempotent; a spent offer stays spent).
    pub fn dismiss(&mut self) {
        if self.state == OfferState::Open {
            self.state = OfferState::Dismissed;
        }
    }

    /// `member` presents the wire `code`. The first match consumes the
    /// offer ([`Presented::Enrolled`]); the same member matching again gets
    /// [`Presented::AlreadyEnrolled`]; any other presentation of a spent
    /// offer is [`EnrollError::Spent`]. While open, a mismatch counts toward
    /// lockout; a malformed code does not.
    pub fn present(&mut self, member: &NodeId, code: &[u8]) -> Result<Presented, EnrollError> {
        match self.state {
            OfferState::Open => {}
            OfferState::Spent => {
                if self.consumer.as_ref() == Some(member) {
                    let presented = parse_code(code)?;
                    if self.matches(member, &presented)? {
                        return Ok(Presented::AlreadyEnrolled);
                    }
                }
                return Err(EnrollError::Spent);
            }
            OfferState::Dismissed => return Err(EnrollError::Dismissed),
            OfferState::LockedOut => return Err(EnrollError::LockedOut),
        }
        let presented = parse_code(code)?;
        if self.matches(member, &presented)? {
            self.state = OfferState::Spent;
            self.consumer = Some(*member);
            return Ok(Presented::Enrolled);
        }
        self.failures += 1;
        if self.failures >= MAX_FAILURES {
            self.state = OfferState::LockedOut;
            return Err(EnrollError::LockedOut);
        }
        Err(EnrollError::Mismatch {
            failures_left: MAX_FAILURES - self.failures,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_double::{TestSealer, MNEMONIC_A, MNEMONIC_B};

    const CODE: Code = *b"7BCD9KMQ";

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn strict_parser_matches_frozen_rejections() {
        assert_eq!(parse_code(b"7BCD9KMQ"), Ok(CODE));
        assert_eq!(parse_code(b"00000000"), Ok(*b"00000000"));
        // `test-vectors/enroll.json` `reject_code` (Debug tokens).
        assert_eq!(
            format!("{:?}", parse_code(b"7BCD9KM").unwrap_err()),
            "Length(7)"
        );
        assert_eq!(
            format!("{:?}", parse_code(b"7BCD9KMQX").unwrap_err()),
            "Length(9)"
        );
        assert_eq!(
            format!("{:?}", parse_code(b"7BCD9KMA").unwrap_err()),
            "Symbol(65)"
        );
        assert_eq!(
            format!("{:?}", parse_code(b"7bcd9kmq").unwrap_err()),
            "Symbol(98)"
        );
        assert_eq!(
            format!("{:?}", parse_code(b"7BCD-KMQ").unwrap_err()),
            "Symbol(45)"
        );
    }

    /// `test-vectors/enroll.json` `tags[0..3]` (the `frozen` module in
    /// `test_double.rs` loads the file; this pins the formula directly).
    #[test]
    fn keyed_hash_known_answers() {
        let a = TestSealer::from_words(MNEMONIC_A).node_id_value();
        let b = TestSealer::from_words(MNEMONIC_B).node_id_value();
        assert_eq!(
            hex(a.as_bytes()),
            "6f1d5331e36e16d341fa06c9f622cc8569710f2a62d7491e9be8e953df844f01"
        );
        let secret: [u8; SECRET_LEN] =
            unhex("c365a807021a5208bb76f5f876ada3d191a1d9533ef1968a09b97aa75b3ae553")
                .try_into()
                .unwrap();
        assert_eq!(
            hex(&enroll_hash(&secret, &a, &CODE).unwrap()),
            "a4f1aa0bcbe311fee599df71beb39735"
        );
        assert_eq!(
            hex(&enroll_hash(&secret, &b, &CODE).unwrap()),
            "37b65a200ee7a8a563fd5c85155a8e9c"
        );
        assert_eq!(
            hex(&enroll_hash(&secret, &a, b"00000000").unwrap()),
            "7ead060a4633dea442cd7496c5fad5a1"
        );
    }

    #[test]
    fn hash_binds_secret_member_and_code() {
        let m1 = TestSealer::from_label("alice").node_id_value();
        let m2 = TestSealer::from_label("bob").node_id_value();
        let s1 = [1u8; 32];
        let s2 = [2u8; 32];
        let h = enroll_hash(&s1, &m1, &CODE).unwrap();
        assert!(verify_enroll_hash(&s1, &m1, &CODE, &h));
        assert_ne!(enroll_hash(&s2, &m1, &CODE).unwrap(), h);
        assert_ne!(enroll_hash(&s1, &m2, &CODE).unwrap(), h);
        assert_ne!(enroll_hash(&s1, &m1, b"7BCD9KMP").unwrap(), h);
        assert!(!verify_enroll_hash(&s1, &m2, &CODE, &h));
    }

    #[test]
    fn offer_single_use_and_lockout() {
        let m = TestSealer::from_label("alice").node_id_value();
        let mut o = Offer::new(CODE, [9u8; 32]);
        assert_eq!(o.display(), "7BCD-9KMQ");
        assert_eq!(o.present(&m, b"nope"), Err(EnrollError::Length(4)));
        assert_eq!(o.present(&m, b"7bcd9kmq"), Err(EnrollError::Symbol(b'b')));
        assert_eq!(o.failures(), 0);
        assert_eq!(
            o.present(&m, b"7BCD9KMP"),
            Err(EnrollError::Mismatch { failures_left: 4 })
        );
        assert_eq!(o.present(&m, b"7BCD9KMQ"), Ok(Presented::Enrolled));
        assert_eq!(o.state(), OfferState::Spent);
        assert_eq!(o.consumer(), Some(&m));
        // a retransmitted ENROLL from the consumer: ENROLL_OK again, no change
        assert_eq!(o.present(&m, b"7BCD9KMQ"), Ok(Presented::AlreadyEnrolled));
        assert_eq!(o.state(), OfferState::Spent);
        assert_eq!(o.failures(), 1);
        // the consumer with a different code, or anyone else: spent
        assert_eq!(o.present(&m, b"7BCD9KMP"), Err(EnrollError::Spent));
        let other = TestSealer::from_label("bob").node_id_value();
        assert_eq!(o.present(&other, b"7BCD9KMQ"), Err(EnrollError::Spent));
        assert_eq!(o.consumer(), Some(&m));
        o.dismiss();
        assert_eq!(o.state(), OfferState::Spent);

        let mut o = Offer::new(CODE, [9u8; 32]);
        for left in [4, 3, 2, 1] {
            assert_eq!(
                o.present(&m, b"00000000"),
                Err(EnrollError::Mismatch {
                    failures_left: left
                })
            );
        }
        assert_eq!(o.present(&m, b"00000000"), Err(EnrollError::LockedOut));
        assert_eq!(o.state(), OfferState::LockedOut);
        assert_eq!(o.present(&m, b"7BCD9KMQ"), Err(EnrollError::LockedOut));

        let mut o = Offer::new(CODE, [9u8; 32]);
        o.dismiss();
        assert_eq!(o.present(&m, b"7BCD9KMQ"), Err(EnrollError::Dismissed));
    }
}
