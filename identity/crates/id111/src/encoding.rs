//! §7 — encodings & display.
//!
//! Implements (owner: T7):
//!
//! - Wire/disk binary records: raw 32B key prefixed with the 2-byte multicodec
//!   varint `0xed 0x01` (ed25519-pub) = 34B (`0xec 0x01` x25519-pub where a
//!   record needs it) — no multiformats dependency. Parse is fail-closed on
//!   unknown prefix or wrong length (§12 rule 1, no silent downgrade).
//! - Machine string: lowercase hex of the raw 32B key (canonical in protocol
//!   messages, logs, APIs). Decode rejects uppercase, wrong length, non-hex.
//! - Human-transfer string: bech32 per BIP-173 — explicitly NOT bech32m —
//!   HRP `node`; display lowercase, QR transfer UPPERCASE, mixed case invalid;
//!   decoders MUST enforce the expected 32-byte data-part length (§7 normative,
//!   embedded#15: BIP-173's insertion/deletion weakness is only closed for
//!   fixed-size payloads by that length check). This decoder additionally
//!   enforces canonical zero-padding by re-encode comparison (malleability
//!   gate: a non-canonical string decodes to the right key but is rejected).
//! - Operator label: SHA-256(pubkey) → base32 (RFC 4648, lowercase, no
//!   padding) → first 10 chars, displayed grouped `xxxx-xxxx-xx` (pinned
//!   length — two UIs rendering different labels for the same node defeats the
//!   label's point). **Display-only, never an authenticator** (§2 B2 hard
//!   rule; Git short-hash collision attacks are the precedent).
//! - [`NodeIdDisplay`]: the crate's canonical presentation API for a NodeId —
//!   hex for APIs, bech32 for human transfer, label for dashboards (§7; the
//!   Layer-4 surface per §13).
//!
//! Metadata versioning uses monotonic counters, never timestamps. BANNED per
//! §7: X.509/PEM/PKCS#8, JOSE/JWK, bech32m, `nsec`/`npub` HRP reuse.
//!
//! This module handles PUBLIC material only — the NodeId IS the raw 32-byte
//! Ed25519 public key (§2 B2). No secret material passes through here, so no
//! zeroize handling is required in this module.

use core::fmt;

use bech32::primitives::decode::CheckedHrpstring;
use bech32::{Bech32, Hrp};
use sha2::{Digest, Sha256};

/// Raw key length — NodeId = 32B (§2 B2).
pub const KEY_LEN: usize = 32;
/// Wire/disk binary record length: 2B multicodec prefix ‖ 32B key = 34B (§7).
pub const RECORD_LEN: usize = 34;
/// Machine-string length: 64 lowercase hex chars for a 32B key (§7).
pub const HEX_LEN: usize = 64;
/// bech32 HRP registry (§7): v1 assigns `node` for the public NodeId.
pub const NODE_HRP: &str = "node";
/// Operator-label length in base32 chars (§7: pinned length).
pub const LABEL_CHARS: usize = 10;

/// Parsed-and-validated HRP for the NodeId bech32 form. `parse_unchecked` is
/// sound here: `node` is statically known to be a valid bech32 HRP.
const NODE_HRP_PARSED: Hrp = Hrp::parse_unchecked(NODE_HRP);

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
/// RFC 4648 base32 alphabet, lowercase form (§7 operator label).
const B32_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Errors from §7 decoding paths. All decoders fail closed: any malformed
/// input is an error, never a fallback interpretation (§12 rule 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodingError {
    /// Wrong length: records must be 34B, hex machine strings 64 chars, and
    /// the decoded bech32 data part must be exactly 32B (§7's normative
    /// data-part-length enforcement, embedded#15).
    InvalidLength { expected: usize, got: usize },
    /// Unknown 2-byte multicodec prefix on a binary record — only
    /// `0xed 0x01` (ed25519-pub) and `0xec 0x01` (x25519-pub) are §7-assigned.
    UnknownPrefix([u8; 2]),
    /// Hex decode failure: non-hex character or uppercase (§7's machine string
    /// is lowercase-only).
    InvalidHex,
    /// bech32 decode failure: bad checksum (including a bech32m checksum —
    /// bech32m is §7-BANNED), mixed case, invalid characters, or malformed
    /// structure.
    InvalidBech32,
    /// bech32 HRP mismatch — the string is well-formed bech32 but not HRP
    /// `node` (§7; e.g. an `npub` string must not be misread as a NodeId).
    WrongHrp,
    /// Well-formed, correctly-sized, valid-checksum bech32 whose data part is
    /// not the canonical zero-padded encoding of the 32B key (re-encode
    /// comparison failed) — representation malleability, rejected.
    NonCanonicalBech32,
}

impl fmt::Display for EncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, got } => {
                write!(f, "invalid length: expected {expected}, got {got}")
            }
            Self::UnknownPrefix(p) => {
                write!(f, "unknown multicodec prefix: 0x{:02x} 0x{:02x}", p[0], p[1])
            }
            Self::InvalidHex => write!(f, "invalid lowercase-hex machine string"),
            Self::InvalidBech32 => write!(f, "invalid bech32 (BIP-173) string"),
            Self::WrongHrp => write!(f, "bech32 HRP is not \"{NODE_HRP}\""),
            Self::NonCanonicalBech32 => {
                write!(f, "non-canonical bech32 data part (padding malleability)")
            }
        }
    }
}

impl std::error::Error for EncodingError {}

/// The key algorithm a §7 binary record describes (the multicodec prefix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// ed25519-pub — multicodec varint `0xed 0x01` (§7). The NodeId's kind.
    Ed25519Pub,
    /// x25519-pub — multicodec varint `0xec 0x01` (§7; records that carry the
    /// ECDH half of the keyset, e.g. the §6.2 announce's `enc_pubkey` field
    /// where a record form is needed).
    X25519Pub,
}

impl KeyKind {
    /// The §7 2-byte multicodec prefix for this key kind.
    pub fn prefix(self) -> [u8; 2] {
        match self {
            Self::Ed25519Pub => [0xed, 0x01],
            Self::X25519Pub => [0xec, 0x01],
        }
    }

    /// Inverse of [`KeyKind::prefix`]; `None` for any unassigned prefix
    /// (fail-closed, §7).
    pub fn from_prefix(prefix: [u8; 2]) -> Option<Self> {
        match prefix {
            [0xed, 0x01] => Some(Self::Ed25519Pub),
            [0xec, 0x01] => Some(Self::X25519Pub),
            _ => None,
        }
    }
}

/// §7 wire/disk binary record: `prefix ‖ key` = 34B.
pub fn encode_record(kind: KeyKind, key: &[u8; KEY_LEN]) -> [u8; RECORD_LEN] {
    let prefix = kind.prefix();
    let mut out = [0u8; RECORD_LEN];
    out[0] = prefix[0];
    out[1] = prefix[1];
    out[2..].copy_from_slice(key);
    out
}

/// Parse a §7 binary record. Fail-closed: wrong length or unknown prefix is an
/// error, never a guess.
pub fn parse_record(record: &[u8]) -> Result<(KeyKind, [u8; KEY_LEN]), EncodingError> {
    if record.len() != RECORD_LEN {
        return Err(EncodingError::InvalidLength {
            expected: RECORD_LEN,
            got: record.len(),
        });
    }
    let prefix = [record[0], record[1]];
    let kind =
        KeyKind::from_prefix(prefix).ok_or(EncodingError::UnknownPrefix(prefix))?;
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&record[2..]);
    Ok((kind, key))
}

/// §7 machine string: lowercase hex of the raw 32B key. Canonical in protocol
/// messages, logs, and APIs.
pub fn hex_encode(key: &[u8; KEY_LEN]) -> String {
    let mut out = String::with_capacity(HEX_LEN);
    for &b in key.iter() {
        out.push(HEX_DIGITS[(b >> 4) as usize] as char);
        out.push(HEX_DIGITS[(b & 0x0f) as usize] as char);
    }
    out
}

/// Parse a §7 machine string. Rejects uppercase, wrong length, and non-hex
/// input — the canonical form is lowercase-only and decoders fail closed.
pub fn hex_decode(s: &str) -> Result<[u8; KEY_LEN], EncodingError> {
    if s.len() != HEX_LEN {
        return Err(EncodingError::InvalidLength {
            expected: HEX_LEN,
            got: s.len(),
        });
    }
    let mut out = [0u8; KEY_LEN];
    // Length pre-checked to HEX_LEN (even), so the remainder slice is empty.
    for (i, &[hi_c, lo_c]) in s.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let hi = hex_value(hi_c)?;
        let lo = hex_value(lo_c)?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_value(c: u8) -> Result<u8, EncodingError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        // Uppercase A–F (and everything else) rejected: §7's machine string is
        // lowercase-only.
        _ => Err(EncodingError::InvalidHex),
    }
}

/// §7 human-transfer string: bech32 per BIP-173 (the `Bech32` checksum
/// variant — bech32m is BANNED), HRP `node`, lowercase display form.
/// 63 chars for a 32B payload — inside bech32's ≤90-char proven-checksum zone.
pub fn bech32_encode(key: &[u8; KEY_LEN]) -> String {
    // Infallible: the HRP is a statically valid constant and a 32B payload
    // encodes to a fixed 63-char string, far under the length limit.
    bech32::encode_lower::<Bech32>(NODE_HRP_PARSED, key)
        .expect("encoding a 32-byte payload under a valid constant HRP cannot fail")
}

/// §7 QR-transfer form: the same BIP-173 string in UPPERCASE (alphanumeric-mode
/// density, BOLT11 convention).
pub fn bech32_encode_qr(key: &[u8; KEY_LEN]) -> String {
    bech32::encode_upper::<Bech32>(NODE_HRP_PARSED, key)
        .expect("encoding a 32-byte payload under a valid constant HRP cannot fail")
}

/// Decode a §7 human-transfer string. Acceptance rules (all normative):
///
/// - BIP-173 checksum only — a bech32m string fails here (bech32m is BANNED,
///   §7): `CheckedHrpstring::new::<Bech32>` pins the variant.
/// - Mixed case rejected (BIP-173 case rule); all-lowercase (display) and
///   all-uppercase (QR transfer) both accepted.
/// - HRP must be `node` (§7 HRP registry).
/// - The decoded data part MUST be exactly 32B (§7 normative, embedded#15 —
///   the BIP-173 insertion/deletion weakness is only closed for fixed-size
///   payloads by this length check).
/// - The data part must be the canonical zero-padded encoding of those 32B
///   (re-encode comparison) — padding-malleable aliases are rejected.
pub fn bech32_decode(s: &str) -> Result<[u8; KEY_LEN], EncodingError> {
    let checked =
        CheckedHrpstring::new::<Bech32>(s).map_err(|_| EncodingError::InvalidBech32)?;
    if checked.hrp() != NODE_HRP_PARSED {
        return Err(EncodingError::WrongHrp);
    }
    let data: Vec<u8> = checked.byte_iter().collect();
    if data.len() != KEY_LEN {
        return Err(EncodingError::InvalidLength {
            expected: KEY_LEN,
            got: data.len(),
        });
    }
    // Canonicality gate: `byte_iter` silently drops padding bits, so a string
    // whose final data char carries non-zero padding would decode to the same
    // key. Re-encode and compare (case-insensitively: the QR form is valid) to
    // reject those aliases.
    let canonical = bech32::encode_lower::<Bech32>(NODE_HRP_PARSED, &data)
        .map_err(|_| EncodingError::InvalidBech32)?;
    if !s.eq_ignore_ascii_case(&canonical) {
        return Err(EncodingError::NonCanonicalBech32);
    }
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&data);
    Ok(key)
}

/// §7 operator label: SHA-256(pubkey) → base32 (RFC 4648, lowercase, no
/// padding) → first 10 chars, displayed grouped `xxxx-xxxx-xx`.
///
/// **Display-only, never an authenticator** (§2 B2 hard rule, §7): a 50-bit
/// truncated label is trivially grindable — Git short-hash collision attacks
/// are the precedent. Never compare, pin, or authenticate by label.
pub fn operator_label(key: &[u8; KEY_LEN]) -> String {
    let digest = Sha256::digest(key);
    let chars = base32_encode(&digest);
    debug_assert!(chars.len() >= LABEL_CHARS);
    let c = &chars[..LABEL_CHARS];
    format!("{}-{}-{}", &c[0..4], &c[4..8], &c[8..LABEL_CHARS])
}

/// RFC 4648 base32, lowercase, no padding (§7 label construction).
fn base32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 8 / 5 + 1);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &b in data {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32_ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
        // Keep only the unconsumed low bits so `acc` stays ≤ 12 bits.
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(B32_ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// §7 — the crate's canonical presentation API for a NodeId: hex for APIs
/// (protocol messages, logs), bech32 for human transfer (lowercase display /
/// UPPERCASE QR), and the operator label for dashboards. The label is
/// display-only, never an authenticator (§2 B2).
///
/// `fmt::Display` emits the §7 canonical machine form (lowercase hex) — the
/// right default for programmatic formatting; human-facing surfaces call the
/// explicit [`bech32`](Self::bech32) / [`label`](Self::label) methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeIdDisplay([u8; KEY_LEN]);

impl NodeIdDisplay {
    pub fn new(key: &[u8; KEY_LEN]) -> Self {
        Self(*key)
    }

    /// The raw 32B NodeId (§2 B2: the Ed25519 public key itself).
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// §7 machine string — lowercase hex, canonical in APIs/logs.
    pub fn hex(&self) -> String {
        hex_encode(&self.0)
    }

    /// §7 human-transfer string — bech32 (BIP-173, HRP `node`), lowercase
    /// display form.
    pub fn bech32(&self) -> String {
        bech32_encode(&self.0)
    }

    /// §7 QR-transfer form — the same bech32 string in UPPERCASE.
    pub fn bech32_qr(&self) -> String {
        bech32_encode_qr(&self.0)
    }

    /// §7 operator label for dashboards (`xxxx-xxxx-xx`) — display-only,
    /// NEVER an authenticator.
    pub fn label(&self) -> String {
        operator_label(&self.0)
    }

    /// §7 wire/disk binary record with the ed25519-pub multicodec prefix
    /// (`nodeid` file content, §13).
    pub fn record(&self) -> [u8; RECORD_LEN] {
        encode_record(KeyKind::Ed25519Pub, &self.0)
    }
}

impl fmt::Display for NodeIdDisplay {
    /// The §7 canonical machine form: lowercase hex.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex_encode(&self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed test pubkey 0x00..=0x1f. The known answers below were computed
    /// with an independent reference (a from-scratch Python BIP-173 encoder
    /// plus hashlib/base32 for the label) and frozen here.
    const PK: [u8; KEY_LEN] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
        0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
        0x1c, 0x1d, 0x1e, 0x1f,
    ];
    const PK_HEX: &str =
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const PK_BECH32: &str =
        "node1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s2mc22u";
    const PK_BECH32_QR: &str =
        "NODE1QQQSYQCYQ5RQWZQFPG9SCRGWPUGPZYSNZS23V9CCRYDPK8QARC0S2MC22U";
    const PK_LABEL: &str = "mmg4-2klg-yq";

    #[test]
    fn kat_fixed_pubkey() {
        assert_eq!(hex_encode(&PK), PK_HEX);
        assert_eq!(bech32_encode(&PK), PK_BECH32);
        assert_eq!(bech32_encode_qr(&PK), PK_BECH32_QR);
        assert_eq!(operator_label(&PK), PK_LABEL);
        let mut expected_record = vec![0xed, 0x01];
        expected_record.extend_from_slice(&PK);
        assert_eq!(encode_record(KeyKind::Ed25519Pub, &PK)[..], expected_record[..]);
    }

    #[test]
    fn bech32_length_is_63() {
        // §7: ≈63 chars for a 32B payload — inside bech32's ≤90-char
        // proven-checksum zone.
        assert_eq!(bech32_encode(&PK).len(), 63);
    }

    #[test]
    fn record_roundtrip_both_kinds() {
        for kind in [KeyKind::Ed25519Pub, KeyKind::X25519Pub] {
            let rec = encode_record(kind, &PK);
            assert_eq!(rec.len(), RECORD_LEN);
            assert_eq!(parse_record(&rec), Ok((kind, PK)));
        }
        // x25519-pub record carries the §7 0xec 0x01 prefix.
        assert_eq!(&encode_record(KeyKind::X25519Pub, &PK)[..2], &[0xec, 0x01]);
    }

    #[test]
    fn record_rejects_unknown_prefix() {
        // Fail closed (§7/§12 rule 1): no prefix other than the two assigned.
        for bad in [[0x00, 0x00], [0xed, 0x00], [0xed, 0x02], [0xec, 0x00], [0x16, 0x01]] {
            let mut rec = [0u8; RECORD_LEN];
            rec[..2].copy_from_slice(&bad);
            rec[2..].copy_from_slice(&PK);
            assert_eq!(parse_record(&rec), Err(EncodingError::UnknownPrefix(bad)));
        }
    }

    #[test]
    fn record_rejects_wrong_length() {
        let rec = encode_record(KeyKind::Ed25519Pub, &PK);
        assert_eq!(
            parse_record(&rec[..33]),
            Err(EncodingError::InvalidLength { expected: 34, got: 33 })
        );
        let mut long = rec.to_vec();
        long.push(0x00);
        assert_eq!(
            parse_record(&long),
            Err(EncodingError::InvalidLength { expected: 34, got: 35 })
        );
        assert_eq!(
            parse_record(&[]),
            Err(EncodingError::InvalidLength { expected: 34, got: 0 })
        );
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(hex_decode(PK_HEX), Ok(PK));
        let other = [0xffu8; KEY_LEN];
        assert_eq!(hex_decode(&hex_encode(&other)), Ok(other));
    }

    #[test]
    fn hex_decode_rejects_uppercase() {
        assert_eq!(
            hex_decode(&PK_HEX.to_uppercase()),
            Err(EncodingError::InvalidHex)
        );
        // Even a single uppercase nibble fails.
        let mut s = PK_HEX.to_string();
        s.replace_range(10..11, "A");
        assert_eq!(hex_decode(&s), Err(EncodingError::InvalidHex));
    }

    #[test]
    fn hex_decode_rejects_non_hex() {
        let mut s = PK_HEX.to_string();
        s.replace_range(0..1, "g");
        assert_eq!(hex_decode(&s), Err(EncodingError::InvalidHex));
        s.replace_range(0..1, "z");
        assert_eq!(hex_decode(&s), Err(EncodingError::InvalidHex));
        // A "0x"-prefixed string is 66 chars: rejected by the length gate.
        assert!(matches!(
            hex_decode(&format!("0x{PK_HEX}")),
            Err(EncodingError::InvalidLength { .. })
        ));
    }

    #[test]
    fn hex_decode_rejects_wrong_length() {
        assert_eq!(
            hex_decode(&PK_HEX[..63]),
            Err(EncodingError::InvalidLength { expected: 64, got: 63 })
        );
        assert_eq!(
            hex_decode(&format!("{PK_HEX}00")),
            Err(EncodingError::InvalidLength { expected: 64, got: 66 })
        );
        assert_eq!(
            hex_decode(""),
            Err(EncodingError::InvalidLength { expected: 64, got: 0 })
        );
    }

    #[test]
    fn bech32_roundtrip_lower_and_upper() {
        assert_eq!(bech32_decode(PK_BECH32), Ok(PK));
        // The UPPERCASE QR form is valid (BIP-173 case rule, §7 QR transfer).
        assert_eq!(bech32_decode(PK_BECH32_QR), Ok(PK));
        assert_eq!(bech32_decode(&bech32_encode(&PK)), Ok(PK));
    }

    #[test]
    fn bech32_decode_rejects_bech32m() {
        // §7 BANNED: bech32m. A bech32m-checksummed encoding of the exact same
        // payload under the same HRP must fail.
        let m = bech32::encode_lower::<bech32::Bech32m>(NODE_HRP_PARSED, &PK)
            .expect("encode works");
        assert_ne!(m, PK_BECH32);
        assert_eq!(bech32_decode(&m), Err(EncodingError::InvalidBech32));
    }

    #[test]
    fn bech32_decode_enforces_data_part_length() {
        // §7 normative (embedded#15): the BIP-173 insertion/deletion weakness
        // is only closed by enforcing the fixed data-part length. These are
        // well-formed bech32 strings with valid checksums and the right HRP —
        // rejected purely on size.
        for bad_len in [0usize, 31, 33, 64] {
            let payload = vec![0x5au8; bad_len];
            let s = bech32::encode_lower::<Bech32>(NODE_HRP_PARSED, &payload)
                .expect("encode works");
            assert_eq!(
                bech32_decode(&s),
                Err(EncodingError::InvalidLength { expected: 32, got: bad_len })
            );
        }
    }

    #[test]
    fn bech32_decode_rejects_mixed_case() {
        // BIP-173 case rule: all-lower and all-upper are valid, mixed is not.
        let mut s = PK_BECH32.to_string();
        s.replace_range(0..1, "N"); // "Node1qqq..." — mixed
        assert_eq!(bech32_decode(&s), Err(EncodingError::InvalidBech32));
        // One lowercase char inside the uppercase QR form is also mixed case.
        let mut u = PK_BECH32_QR.to_string();
        u.replace_range(10..11, "q");
        assert_eq!(bech32_decode(&u), Err(EncodingError::InvalidBech32));
    }

    #[test]
    fn bech32_decode_rejects_wrong_hrp() {
        // §7: `npub` HRP reuse is BANNED — a nostr-shaped string must not be
        // misread as a NodeId.
        let npub = bech32::encode_lower::<Bech32>(Hrp::parse("npub").unwrap(), &PK)
            .expect("encode works");
        assert_eq!(bech32_decode(&npub), Err(EncodingError::WrongHrp));
        let other = bech32::encode_lower::<Bech32>(Hrp::parse("test").unwrap(), &PK)
            .expect("encode works");
        assert_eq!(bech32_decode(&other), Err(EncodingError::WrongHrp));
    }

    #[test]
    fn bech32_decode_rejects_tampered_checksum() {
        // Flip one data char: the BIP-173 checksum catches it.
        let mut s = PK_BECH32.to_string();
        s.replace_range(20..21, "x");
        assert_eq!(bech32_decode(&s), Err(EncodingError::InvalidBech32));
    }

    #[test]
    fn bech32_decode_rejects_garbage() {
        for bad in ["", "node", "node1", "1qqqsyqcyq5", "node1QQQSYQCYQ5!"] {
            assert_eq!(bech32_decode(bad), Err(EncodingError::InvalidBech32), "input: {bad:?}");
        }
        // 'b', 'i', 'o' are outside the bech32 charset.
        let mut s = PK_BECH32.to_string();
        s.replace_range(6..7, "b");
        assert_eq!(bech32_decode(&s), Err(EncodingError::InvalidBech32));
    }

    #[test]
    fn bech32_decode_rejects_noncanonical_padding() {
        // Construct a string whose final data char carries non-zero padding
        // bits but whose checksum is VALID: only the canonicality gate rejects
        // it (byte_iter drops pad bits, so it would otherwise decode fine).
        let data_part = &PK_BECH32[NODE_HRP.len() + 1..];
        let mut fes: Vec<u8> = data_part.bytes().map(fe_of_char).collect();
        fes.truncate(fes.len() - 6); // drop the 6 checksum fes
        let last = fes.last_mut().expect("non-empty data part");
        *last ^= 1; // flip one padding bit — decoded 32B unchanged
        let checksum = local_bech32_checksum(NODE_HRP, &fes);
        let mut mutated = String::from(PK_BECH32);
        mutated.truncate(NODE_HRP.len() + 1);
        for f in fes.iter().chain(checksum.iter()) {
            mutated.push(BECH32_CHARSET[*f as usize] as char);
        }
        // Sanity: the mutation is genuinely well-formed bech32 (valid BIP-173
        // checksum)...
        assert!(CheckedHrpstring::new::<Bech32>(&mutated).is_ok());
        assert_ne!(mutated, PK_BECH32);
        // ...and only the canonicality gate refuses it.
        assert_eq!(bech32_decode(&mutated), Err(EncodingError::NonCanonicalBech32));
    }

    #[test]
    fn base32_rfc4648_vectors() {
        // RFC 4648 test vectors, lowercase (the §7 label alphabet form).
        assert_eq!(base32_encode(b""), "");
        assert_eq!(base32_encode(b"f"), "my");
        assert_eq!(base32_encode(b"fo"), "mzxq");
        assert_eq!(base32_encode(b"foo"), "mzxw6");
        assert_eq!(base32_encode(b"foob"), "mzxw6yq");
        assert_eq!(base32_encode(b"fooba"), "mzxw6ytb");
        assert_eq!(base32_encode(b"foobar"), "mzxw6ytboi");
    }

    #[test]
    fn label_shape_and_determinism() {
        let label = operator_label(&PK);
        assert_eq!(label.len(), 12); // xxxx-xxxx-xx
        assert_eq!(label.as_bytes()[4], b'-');
        assert_eq!(label.as_bytes()[9], b'-');
        assert!(label
            .bytes()
            .filter(|&b| b != b'-')
            .all(|b| B32_ALPHABET.contains(&b)));
        assert_eq!(label.bytes().filter(|&b| b != b'-').count(), LABEL_CHARS);
        // Deterministic for the same key, distinct for a different key.
        assert_eq!(operator_label(&PK), label);
        assert_ne!(operator_label(&[0xffu8; KEY_LEN]), label);
    }

    #[test]
    fn node_id_display_groups_the_presentation_api() {
        let d = NodeIdDisplay::new(&PK);
        assert_eq!(d.as_bytes(), &PK);
        assert_eq!(d.hex(), PK_HEX);
        assert_eq!(d.bech32(), PK_BECH32);
        assert_eq!(d.bech32_qr(), PK_BECH32_QR);
        assert_eq!(d.label(), PK_LABEL);
        assert_eq!(&d.record()[..2], &[0xed, 0x01]);
        // Display = the §7 canonical machine form (hex).
        assert_eq!(format!("{d}"), PK_HEX);
    }

    // --- test-local helpers for the padding-malleability negative ----------

    const BECH32_CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

    fn fe_of_char(c: u8) -> u8 {
        BECH32_CHARSET
            .iter()
            .position(|&x| x == c)
            .expect("test input is valid bech32") as u8
    }

    /// BIP-173 reference checksum (constant 1 — bech32, never bech32m), used
    /// only to build the non-canonical-padding negative vector.
    fn local_bech32_checksum(hrp: &str, data: &[u8]) -> [u8; 6] {
        const GEN: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
        let mut values: Vec<u8> = hrp.bytes().map(|b| b >> 5).collect();
        values.push(0);
        values.extend(hrp.bytes().map(|b| b & 31));
        values.extend_from_slice(data);
        values.extend_from_slice(&[0; 6]);
        let mut chk: u32 = 1;
        for &v in &values {
            let top = chk >> 25;
            chk = ((chk & 0x1ff_ffff) << 5) ^ u32::from(v);
            for (i, g) in GEN.iter().enumerate() {
                if (top >> i) & 1 == 1 {
                    chk ^= g;
                }
            }
        }
        let pm = chk ^ 1;
        let mut out = [0u8; 6];
        for (i, o) in out.iter_mut().enumerate() {
            *o = ((pm >> (5 * (5 - i))) & 31) as u8;
        }
        out
    }
}
