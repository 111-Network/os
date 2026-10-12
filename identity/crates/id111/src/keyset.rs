//! §2–§4 — the dual keyset and key generation discipline.
//!
//! Implements (owner: T2):
//! - §2: the keyset types — public keyset 64B (32B Ed25519 pub ‖ 32B X25519
//!   pub), private keyset 64B (32B Ed25519 seed ‖ 32B X25519 scalar);
//!   **NodeId = the raw 32-byte Ed25519 public key** (D13 B2). Derivation:
//!   BIP-39 mnemonic (§8.1) → 64B seed → SLIP-0010 `m/111'/0'/0'` (signing
//!   seed) and `m/111'/0'/1'` (X25519 scalar, raw — clamped per RFC 7748 at
//!   point of use).
//! - §3: the fleet-wide verification predicate — Ed25519 `verify_strict`
//!   semantics only (canonical R/A field encodings < 2²⁵⁵−19, S < ℓ,
//!   small-order A and R rejected); dalek's default `verify()` MUST NOT be
//!   used for wire artifacts and is not called anywhere in this crate.
//!   X25519 low-order public keys are rejected on ingress; an all-zero
//!   shared secret aborts (RFC 7748 §6.1, via `was_contributory`).
//! - §4: entropy-gated keygen — `getrandom()` is the named Cat 2+ source
//!   (§4.2), no silent weak fallback, and the §4.5 weak-key self-test
//!   (all-zero / uniform-byte / counter-pattern / X25519 low-order) runs
//!   before any keyset leaves this module.
//! - §4 rule 6: no clone/duplicate-keyset feature is ever offered here
//!   (`Keyset` is deliberately not `Clone`).
//! - §6.1/§6.2: every signed artifact prepends its ASCII domain tag inside
//!   the signed region — the only signing entry point is `sign_with_tag`
//!   (the caller passes the tag; the raw-signing helper is private).

use std::fmt;

use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::bip39::{Bip39Error, RootMnemonic, WordCount};
use crate::slip0010;

// ---------------------------------------------------------------------------
// §2/B2 — NodeId
// ---------------------------------------------------------------------------

/// §2 (D13 B2) — **NodeId = the raw 32-byte Ed25519 public key.** No hashing,
/// no truncation. Any derived or truncated label (§7) is a display aid, never
/// an authenticator.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId([u8; 32]);

impl NodeId {
    /// The 32 raw bytes (this IS the Ed25519 public key, B2).
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Byte copy.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0
    }
}

impl From<[u8; 32]> for NodeId {
    fn from(bytes: [u8; 32]) -> Self {
        NodeId(bytes)
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId(")?;
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        write!(f, ")")
    }
}

// ---------------------------------------------------------------------------
// §2 — PublicKeyset (64B: 32B Ed25519 pub ‖ 32B X25519 pub)
// ---------------------------------------------------------------------------

/// §2 — the public keyset: `sign_pubkey(32) ‖ enc_pubkey(32)` = 64B, the
/// §6.2 announce's key fields. `sign_pubkey` IS the NodeId (B2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicKeyset {
    sign: [u8; 32],
    enc: [u8; 32],
}

/// The exact encoded size of a public keyset (§2, normative sizes).
pub const PUBLIC_KEYSET_LEN: usize = 64;

/// The exact encoded size of a private keyset (§2, normative sizes).
pub const PRIVATE_KEYSET_LEN: usize = 64;

impl PublicKeyset {
    /// §2 — parse a 64B public keyset. Ingress validation per §3: the X25519
    /// half MUST pass the low-order check (RFC 7748 §6.1). (Ed25519-half
    /// strictness is enforced at verification time by the §3 predicate.)
    pub fn from_bytes(bytes: &[u8; 64]) -> Result<Self, EcdhError> {
        let mut sign = [0u8; 32];
        let mut enc = [0u8; 32];
        sign.copy_from_slice(&bytes[..32]);
        enc.copy_from_slice(&bytes[32..]);
        validate_x25519_public_key(&enc)?;
        Ok(PublicKeyset { sign, enc })
    }

    /// The 64B wire/disk form: `sign ‖ enc` (§2).
    pub fn to_bytes(&self) -> [u8; 64] {
        let mut out = [0u8; 64];
        out[..32].copy_from_slice(&self.sign);
        out[32..].copy_from_slice(&self.enc);
        out
    }

    /// §2/B2 — the NodeId (the Ed25519 public key).
    pub fn node_id(&self) -> NodeId {
        NodeId(self.sign)
    }

    /// The Ed25519 (signing) public key — the NodeId bytes.
    pub fn sign_pubkey(&self) -> &[u8; 32] {
        &self.sign
    }

    /// The X25519 (ECDH) public key — §6.2's `enc_pubkey`.
    pub fn enc_pubkey(&self) -> &[u8; 32] {
        &self.enc
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Keyset construction errors (§2/§4).
#[derive(Debug)]
pub enum KeysetError {
    /// §4.2 — the named entropy source (`getrandom()`) failed. No silent weak
    /// fallback (§4 rule 2): keygen aborts loudly.
    Entropy(getrandom::Error),
    /// Mnemonic layer rejected input (§8.1/§8.4 hygiene).
    Mnemonic(Bip39Error),
    /// The §4.5 weak-key self-test failed; nothing is persisted (§4 rule 5).
    WeakKey(WeakKeyError),
}

impl fmt::Display for KeysetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeysetError::Entropy(e) => write!(f, "§4.2: getrandom() failed: {e}"),
            KeysetError::Mnemonic(e) => write!(f, "{e}"),
            KeysetError::WeakKey(e) => write!(f, "§4.5 weak-key self-test failed: {e}"),
        }
    }
}

impl std::error::Error for KeysetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            // getrandom 0.2's Error does not implement std::error::Error
            // without its "std" feature (dep set frozen — no feature change).
            KeysetError::Entropy(_) => None,
            KeysetError::Mnemonic(e) => Some(e),
            KeysetError::WeakKey(e) => Some(e),
        }
    }
}

impl From<Bip39Error> for KeysetError {
    fn from(e: Bip39Error) -> Self {
        KeysetError::Mnemonic(e)
    }
}

impl From<WeakKeyError> for KeysetError {
    fn from(e: WeakKeyError) -> Self {
        KeysetError::WeakKey(e)
    }
}

/// §4.5 weak-key self-test failures (concrete checks, normative).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeakKeyError {
    /// A 32-byte half was all-zero.
    AllZero,
    /// A 32-byte half was one repeated byte.
    UniformByte,
    /// A 32-byte half was a ±1 byte counter pattern.
    CounterPattern,
    /// The X25519 public key derived from the private scalar is low-order
    /// (RFC 7748 §6.1, §4.5).
    LowOrderX25519Public,
}

impl fmt::Display for WeakKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WeakKeyError::AllZero => write!(f, "all-zero key half"),
            WeakKeyError::UniformByte => write!(f, "uniform-byte key half"),
            WeakKeyError::CounterPattern => write!(f, "counter-pattern key half"),
            WeakKeyError::LowOrderX25519Public => {
                write!(f, "derived X25519 public key is low-order (RFC 7748 §6.1)")
            }
        }
    }
}

impl std::error::Error for WeakKeyError {}

/// §3 verification-predicate failures. Every variant is a rejection; the
/// strict predicate fails closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    /// The public key's encoding is non-canonical (field element ≥ 2²⁵⁵−19).
    NonCanonicalPublicKey,
    /// The public key bytes do not decompress to a curve point.
    MalformedPublicKey,
    /// The signature's R encoding is non-canonical (field element ≥ 2²⁵⁵−19).
    NonCanonicalR,
    /// The signature's S is non-canonical (S ≥ ℓ).
    NonCanonicalS,
    /// Everything parsed but the strict verification equation failed —
    /// including small-order A or R (§3).
    InvalidSignature,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerifyError::NonCanonicalPublicKey => {
                write!(f, "§3: non-canonical Ed25519 public-key encoding")
            }
            VerifyError::MalformedPublicKey => {
                write!(f, "§3: Ed25519 public key fails decompression")
            }
            VerifyError::NonCanonicalR => write!(f, "§3: non-canonical R encoding"),
            VerifyError::NonCanonicalS => write!(f, "§3: non-canonical S (S ≥ ℓ)"),
            VerifyError::InvalidSignature => {
                write!(f, "§3: strict verification failed (bad signature or small-order A/R)")
            }
        }
    }
}

impl std::error::Error for VerifyError {}

/// §3/§6.4 X25519 errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcdhError {
    /// §3 — the peer's X25519 public key is low-order (RFC 7748 §6.1);
    /// rejected on ingress (pin time) and again at exchange time.
    LowOrderPublicKey,
    /// §6.4 step 2 — the X25519 output was all-zero (non-contributory);
    /// the exchange MUST abort (RFC 7748 §6.1). The zero secret is never
    /// returned.
    NonContributory,
}

impl fmt::Display for EcdhError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EcdhError::LowOrderPublicKey => {
                write!(f, "§3: low-order X25519 public key rejected (RFC 7748 §6.1)")
            }
            EcdhError::NonContributory => {
                write!(f, "§6.4: all-zero X25519 shared secret — aborting (RFC 7748 §6.1)")
            }
        }
    }
}

impl std::error::Error for EcdhError {}

// ---------------------------------------------------------------------------
// §3 — X25519 ingress validation (low-order rejection, RFC 7748 §6.1)
// ---------------------------------------------------------------------------

/// §3 (normative) — X25519 low-order public keys MUST be rejected on ingress.
/// A public key is low-order iff its point has order dividing 8 (the
/// Curve25519 cofactor; the twist's cofactor 4 also divides 8), i.e. iff
/// 8·P is the identity. This single multiplication rejects the entire RFC
/// 7748 §6.1 / libsodium-blacklist class — including the all-zero and `01`
/// encodings and their non-canonical wraps (p, p+1) — without a hardcoded
/// table.
pub fn validate_x25519_public_key(bytes: &[u8; 32]) -> Result<(), EcdhError> {
    use curve25519_dalek::montgomery::MontgomeryPoint;
    use curve25519_dalek::scalar::Scalar;

    let point = MontgomeryPoint(*bytes);
    let cofactored = point * Scalar::from(8u64);
    // The Montgomery identity encodes as 32 zero bytes. Public input — a
    // plain byte compare is fine (no secret-dependent timing).
    if cofactored.to_bytes() == [0u8; 32] {
        return Err(EcdhError::LowOrderPublicKey);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// §4.5 — the weak-key self-test
// ---------------------------------------------------------------------------

fn is_all_zero(half: &[u8; 32]) -> bool {
    half.iter().all(|&b| b == 0)
}

fn is_uniform_byte(half: &[u8; 32]) -> bool {
    half.iter().all(|&b| b == half[0])
}

fn is_counter_pattern(half: &[u8; 32]) -> bool {
    let ascending = half
        .iter()
        .enumerate()
        .all(|(i, &b)| b == half[0].wrapping_add(i as u8));
    let descending = half
        .iter()
        .enumerate()
        .all(|(i, &b)| b == half[0].wrapping_sub(i as u8));
    ascending || descending
}

/// §4 rule 5 (normative) — the weak-key self-test over a 64-byte private
/// keyset (`sign seed ‖ X25519 scalar`): reject all-zero, uniform-byte, and
/// counter-pattern halves, and reject an X25519 half whose derived public key
/// is RFC 7748-low-order. A failed self-test aborts; nothing is persisted
/// (§4.5). Runs on every keyset this module produces or loads — keygen,
/// restore derivation, and identity-dir load alike.
///
/// Note: an honestly clamped X25519 scalar can never produce a low-order
/// public key (no multiple of the group order ℓ is a valid clamped scalar),
/// so the last leg is defense-in-depth that only crafted input reaches; the
/// low-order check itself is exercised directly against the blacklist class
/// by `validate_x25519_public_key`'s tests.
pub fn weak_key_self_test(private_keyset: &[u8; 64]) -> Result<(), WeakKeyError> {
    let mut sign = [0u8; 32];
    let mut ecdh = [0u8; 32];
    sign.copy_from_slice(&private_keyset[..32]);
    ecdh.copy_from_slice(&private_keyset[32..]);

    for half in [&sign, &ecdh] {
        if is_all_zero(half) {
            return Err(WeakKeyError::AllZero);
        }
        if is_uniform_byte(half) {
            return Err(WeakKeyError::UniformByte);
        }
        if is_counter_pattern(half) {
            return Err(WeakKeyError::CounterPattern);
        }
    }

    // §4.5: reject RFC 7748 low-order public keys for the X25519 output.
    let derived_pub = x25519_public_from_scalar(&ecdh);
    validate_x25519_public_key(&derived_pub).map_err(|_| WeakKeyError::LowOrderX25519Public)?;
    Ok(())
}

/// X25519 public key for a raw 32-byte private scalar (RFC 7748 basepoint
/// multiplication with clamping at point of use — §2).
fn x25519_public_from_scalar(scalar: &[u8; 32]) -> [u8; 32] {
    let secret = x25519_dalek::StaticSecret::from(*scalar);
    x25519_dalek::PublicKey::from(&secret).to_bytes()
}

// ---------------------------------------------------------------------------
// §2 — the private Keyset
// ---------------------------------------------------------------------------

/// §2 — the private keyset: 64B = 32B Ed25519 private seed ‖ 32B X25519
/// private scalar (stored raw; clamped per RFC 7748 at point of use).
/// Zeroized on drop (§12: this is THE asset). Deliberately not `Clone`
/// (§4 rule 6 — no identity cloning).
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Keyset {
    sign_seed: [u8; 32],
    ecdh_scalar: [u8; 32],
}

impl Keyset {
    /// §4 — entropy-gated keygen (Cat 2+ install path, §8.3 step 1): draws
    /// fresh entropy from `getrandom()` (the named §4.2 source; no silent
    /// weak fallback — failure aborts loudly), composes the BIP-39 root
    /// secret, derives the keyset per §2, and runs the §4.5 self-test.
    /// Returns the root mnemonic (the backup, §8.3) with the keyset.
    pub fn generate(words: WordCount) -> Result<(RootMnemonic, Keyset), KeysetError> {
        let mnemonic = generate_mnemonic(words)?;
        let keyset = Keyset::derive_from_root(&mnemonic)?;
        Ok((mnemonic, keyset))
    }

    /// §2/§8.4 — derive the keyset from mnemonic words (restore path).
    /// Restore input hygiene (NFKC + case/whitespace folding, checksum MUST
    /// validate) is applied by `RootMnemonic::parse`; the §4.5 self-test runs
    /// before the keyset is returned.
    pub fn derive_from_mnemonic(words: &str) -> Result<Keyset, KeysetError> {
        let mnemonic = RootMnemonic::parse(words)?;
        Keyset::derive_from_root(&mnemonic)
    }

    /// §2 — derive the keyset from an already-parsed root mnemonic: BIP-39
    /// seed → SLIP-0010 `m/111'/0'/0'` (signing seed) and `m/111'/0'/1'`
    /// (X25519 scalar). NOT BIP32-Ed25519/Cardano.
    pub fn derive_from_root(mnemonic: &RootMnemonic) -> Result<Keyset, KeysetError> {
        let seed = mnemonic.to_seed();
        let sign_seed = slip0010::derive_signing_seed(&seed[..]);
        let ecdh_scalar = slip0010::derive_ecdh_scalar(&seed[..]);
        Keyset::from_halves(*sign_seed, *ecdh_scalar)
    }

    /// §13 — load a persisted 64B private keyset (the identity dir's `keyset`
    /// file; a derived cache regenerable from the mnemonic). The §4.5
    /// self-test runs here too — a persisted blob failing it is corrupt, and
    /// the boot path quarantines rather than silently re-keys (§13).
    pub fn from_bytes(bytes: &[u8; 64]) -> Result<Keyset, KeysetError> {
        weak_key_self_test(bytes)?;
        let mut sign_seed = [0u8; 32];
        let mut ecdh_scalar = [0u8; 32];
        sign_seed.copy_from_slice(&bytes[..32]);
        ecdh_scalar.copy_from_slice(&bytes[32..]);
        Ok(Keyset {
            sign_seed,
            ecdh_scalar,
        })
    }

    fn from_halves(sign_seed: [u8; 32], ecdh_scalar: [u8; 32]) -> Result<Keyset, KeysetError> {
        let mut bytes = Zeroizing::new([0u8; 64]);
        bytes[..32].copy_from_slice(&sign_seed);
        bytes[32..].copy_from_slice(&ecdh_scalar);
        weak_key_self_test(&bytes)?;
        Ok(Keyset {
            sign_seed,
            ecdh_scalar,
        })
    }

    /// §2 — the 64B private encoding (`sign seed ‖ X25519 scalar`), the §13
    /// `keyset` file body. Zeroizing.
    pub fn to_bytes(&self) -> Zeroizing<[u8; 64]> {
        let mut out = Zeroizing::new([0u8; 64]);
        out[..32].copy_from_slice(&self.sign_seed);
        out[32..].copy_from_slice(&self.ecdh_scalar);
        out
    }

    /// §2 — the public keyset (Ed25519 pub ‖ X25519 pub).
    pub fn public(&self) -> PublicKeyset {
        let sign = ed25519_dalek::SigningKey::from_bytes(&self.sign_seed)
            .verifying_key()
            .to_bytes();
        let enc = x25519_public_from_scalar(&self.ecdh_scalar);
        PublicKeyset { sign, enc }
    }

    /// §2/B2 — the NodeId (the Ed25519 public key).
    pub fn node_id(&self) -> NodeId {
        self.public().node_id()
    }

    /// The X25519 public key (§6.2's `enc_pubkey`).
    pub fn x25519_public(&self) -> [u8; 32] {
        x25519_public_from_scalar(&self.ecdh_scalar)
    }

    /// §6.1/§6.2 — sign `tag ‖ bytes` with the Ed25519 key. Every signed
    /// artifact prepends its ASCII domain tag inside the signed region (the
    /// tag is part of the signed payload, reconstructed by the verifier,
    /// never a wire field); this is the ONLY signing entry point.
    pub fn sign_with_tag(&self, tag: &[u8], bytes: &[u8]) -> [u8; 64] {
        let mut msg = Vec::with_capacity(tag.len() + bytes.len());
        msg.extend_from_slice(tag);
        msg.extend_from_slice(bytes);
        self.sign_raw(&msg)
    }

    /// Raw RFC 8032 signing over `msg` — private: all crate signing goes
    /// through `sign_with_tag` (§6.1). Tests use it for the RFC 8032
    /// reference vectors (which are untagged by definition).
    fn sign_raw(&self, msg: &[u8]) -> [u8; 64] {
        use ed25519_dalek::Signer;
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&self.sign_seed);
        let sig = signing_key.sign(msg);
        // SigningKey zeroizes on drop (ed25519-dalek zeroize feature).
        sig.to_bytes()
    }

    /// §6.4 step 2 — X25519 ECDH against a peer public key. The peer key is
    /// re-checked against the §3 low-order rule (it MUST have passed at pin
    /// time; defense in depth), and an all-zero shared output aborts the
    /// exchange (RFC 7748 §6.1, via `was_contributory` — ungated in
    /// x25519-dalek 2.0.1). The zero secret is never returned.
    pub fn ecdh(&self, their_public: &[u8; 32]) -> Result<Zeroizing<[u8; 32]>, EcdhError> {
        validate_x25519_public_key(their_public)?;
        let secret = x25519_dalek::StaticSecret::from(self.ecdh_scalar);
        let shared = secret.diffie_hellman(&x25519_dalek::PublicKey::from(*their_public));
        if !shared.was_contributory() {
            return Err(EcdhError::NonContributory);
        }
        Ok(Zeroizing::new(shared.to_bytes()))
    }
}

impl fmt::Debug for Keyset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // §5/§12: the private keyset is never printed.
        write!(f, "Keyset(redacted)")
    }
}

/// §4.2 — the named Cat 2+ entropy source: `getrandom()`. Draws
/// `words.entropy_bytes()` fresh bytes and composes the BIP-39 root secret.
/// No silent weak fallback (§4 rule 2) — any failure aborts keygen loudly.
/// Identity keys draw from the stirred pool directly (§4 rule 2).
pub fn generate_mnemonic(words: WordCount) -> Result<RootMnemonic, KeysetError> {
    let mut entropy = Zeroizing::new([0u8; 32]);
    let n = words.entropy_bytes();
    getrandom::getrandom(&mut entropy[..n]).map_err(KeysetError::Entropy)?;
    Ok(RootMnemonic::from_entropy(&entropy[..n])?)
}

// ---------------------------------------------------------------------------
// §3 — the strict verification predicate (the ONLY verification path)
// ---------------------------------------------------------------------------

/// 2²⁵⁵−19 (the field prime p), little-endian.
const FIELD_P_LE: [u8; 32] = [
    0xed, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f,
];

/// The Ed25519 group order ℓ = 2²⁵² + 27742317777372353535851937790883648493,
/// little-endian.
const GROUP_ORDER_L_LE: [u8; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10,
];

/// Little-endian 32-byte integer comparison: a < b. (Signature material is
/// public; no constant-time requirement.)
fn le_lt(a: &[u8; 32], b: &[u8; 32]) -> bool {
    for i in (0..32).rev() {
        if a[i] != b[i] {
            return a[i] < b[i];
        }
    }
    false
}

/// §3 — a canonical Ed25519 point encoding: the 255-bit y-coordinate (top
/// bit masked — it is the x sign bit) is a field element < 2²⁵⁵−19.
fn is_canonical_point_encoding(enc: &[u8; 32]) -> bool {
    let mut y = *enc;
    y[31] &= 0x7f;
    le_lt(&y, &FIELD_P_LE)
}

/// §3 (normative, fleet-wide) — the strict Ed25519 verification predicate:
/// canonical encodings of R and A (field elements < 2²⁵⁵−19), S < ℓ,
/// small-order A and R rejected — ed25519-dalek `verify_strict`-class rules,
/// with the canonicality legs checked explicitly so acceptance does not
/// depend on library decompression leniency. Dalek's default `verify()` is
/// BANNED for wire artifacts (§3) and is never called here.
///
/// `node` is the claimed signer's NodeId (B2: it IS the Ed25519 public key);
/// `message` is the exact signed input (tagged artifacts: `tag ‖ bytes` —
/// use `verify_strict_tagged`).
pub fn verify_strict(node: &NodeId, message: &[u8], signature: &[u8; 64]) -> Result<(), VerifyError> {
    // A (public key): canonical encoding.
    if !is_canonical_point_encoding(node.as_bytes()) {
        return Err(VerifyError::NonCanonicalPublicKey);
    }
    // R: canonical encoding.
    let mut r_bytes = [0u8; 32];
    r_bytes.copy_from_slice(&signature[..32]);
    if !is_canonical_point_encoding(&r_bytes) {
        return Err(VerifyError::NonCanonicalR);
    }
    // S < ℓ.
    let mut s_bytes = [0u8; 32];
    s_bytes.copy_from_slice(&signature[32..]);
    if !le_lt(&s_bytes, &GROUP_ORDER_L_LE) {
        return Err(VerifyError::NonCanonicalS);
    }

    let vk = ed25519_dalek::VerifyingKey::from_bytes(node.as_bytes())
        .map_err(|_| VerifyError::MalformedPublicKey)?;
    let sig = ed25519_dalek::Signature::from_bytes(signature);
    // verify_strict rejects small-order A and R and applies the strict
    // (cofactorless) verification equation (§3).
    vk.verify_strict(message, &sig)
        .map_err(|_| VerifyError::InvalidSignature)
}

/// §3 + §6.1 — verify a domain-tagged artifact: the signed input is
/// reconstructed as `tag ‖ bytes` (the tag is never a wire field). This is
/// the announce/proof/control-artifact verification helper; there is no
/// untagged verification of wire artifacts anywhere in the crate.
pub fn verify_strict_tagged(
    node: &NodeId,
    tag: &[u8],
    bytes: &[u8],
    signature: &[u8; 64],
) -> Result<(), VerifyError> {
    let mut msg = Vec::with_capacity(tag.len() + bytes.len());
    msg.extend_from_slice(tag);
    msg.extend_from_slice(bytes);
    verify_strict(node, &msg, signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    fn hex32(s: &str) -> [u8; 32] {
        hex(s).try_into().unwrap()
    }

    fn hex64(s: &str) -> [u8; 64] {
        hex(s).try_into().unwrap()
    }

    fn to_hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// LE 32-byte addition (test construction only).
    fn le_add(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
        let mut out = [0u8; 32];
        let mut carry = 0u16;
        for i in 0..32 {
            let s = a[i] as u16 + b[i] as u16 + carry;
            out[i] = s as u8;
            carry = s >> 8;
        }
        out
    }

    /// A non-weak X25519 half for tests that only exercise the Ed25519 leg:
    /// RFC 7748 Alice's scalar.
    const ALICE_SCALAR: [u8; 32] = [
        0x77, 0x07, 0x6d, 0x0a, 0x73, 0x18, 0xa5, 0x7d, 0x3c, 0x16, 0xc1, 0x72, 0x51, 0xb2, 0x66,
        0x45, 0xdf, 0x4c, 0x2f, 0x87, 0xeb, 0xc0, 0x99, 0x2a, 0xb1, 0x77, 0xfb, 0xa5, 0x1d, 0xb9,
        0x2c, 0x2a,
    ];

    fn keyset_from_sign_seed(sign_seed: [u8; 32]) -> Keyset {
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&sign_seed);
        bytes[32..].copy_from_slice(&ALICE_SCALAR);
        Keyset::from_bytes(&bytes).unwrap()
    }

    // ------------------------------------------------------------------
    // RFC 8032 (Ed25519) reference vectors — §14 gate.
    // ------------------------------------------------------------------

    #[test]
    fn rfc8032_vectors() {
        // TEST 1 (empty message)
        let sk1 = hex32("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let pk1 = hex32("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        let sig1 = hex64(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
             5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
                .replace(' ', "")
                .as_str(),
        );
        let ks1 = keyset_from_sign_seed(sk1);
        assert_eq!(ks1.public().sign_pubkey(), &pk1);
        assert_eq!(ks1.sign_raw(&[]), sig1);
        assert!(verify_strict(&NodeId::from(pk1), &[], &sig1).is_ok());

        // TEST 2 (1-byte message 0x72)
        let sk2 = hex32("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb");
        let pk2 = hex32("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c");
        let sig2 = hex64(
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da\
             085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00"
                .replace(' ', "")
                .as_str(),
        );
        let ks2 = keyset_from_sign_seed(sk2);
        assert_eq!(ks2.public().sign_pubkey(), &pk2);
        assert_eq!(ks2.sign_raw(&[0x72]), sig2);
        assert!(verify_strict(&NodeId::from(pk2), &[0x72], &sig2).is_ok());

        // TEST 3 (2-byte message 0xaf82)
        let sk3 = hex32("c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7");
        let pk3 = hex32("fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025");
        let sig3 = hex64(
            "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac\
             18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a"
                .replace(' ', "")
                .as_str(),
        );
        let ks3 = keyset_from_sign_seed(sk3);
        assert_eq!(ks3.public().sign_pubkey(), &pk3);
        assert_eq!(ks3.sign_raw(&[0xaf, 0x82]), sig3);
        assert!(verify_strict(&NodeId::from(pk3), &[0xaf, 0x82], &sig3).is_ok());
    }

    // ------------------------------------------------------------------
    // RFC 7748 (X25519) reference vector — §14 gate.
    // ------------------------------------------------------------------

    #[test]
    fn rfc7748_vector_and_agreement() {
        let bob_scalar = hex32("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");
        let alice_pub = hex32("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a");
        let bob_pub = hex32("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f");
        let want_shared = hex32("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");

        // Public-key derivation from the raw scalars.
        assert_eq!(x25519_public_from_scalar(&ALICE_SCALAR), alice_pub);
        assert_eq!(x25519_public_from_scalar(&bob_scalar), bob_pub);

        // ECDH through the Keyset API, both directions.
        let sign_seed =
            hex32("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let mut b = [0u8; 64];
        b[..32].copy_from_slice(&sign_seed);
        b[32..].copy_from_slice(&ALICE_SCALAR);
        let alice = Keyset::from_bytes(&b).unwrap();
        b[32..].copy_from_slice(&bob_scalar);
        let bob = Keyset::from_bytes(&b).unwrap();

        let s1 = alice.ecdh(&bob_pub).unwrap();
        let s2 = bob.ecdh(&alice_pub).unwrap();
        assert_eq!(&s1[..], &want_shared[..]);
        assert_eq!(&s2[..], &want_shared[..]);
    }

    // ------------------------------------------------------------------
    // §3 strict predicate — negatives (Wycheproof-class edges, §14).
    // ------------------------------------------------------------------

    /// Base material: RFC 8032 TEST 2 (valid key, message, signature).
    fn valid_triple() -> (NodeId, Vec<u8>, [u8; 64]) {
        let node = NodeId::from(hex32(
            "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
        ));
        let sig = hex64(
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da\
             085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00"
                .replace(' ', "")
                .as_str(),
        );
        (node, vec![0x72], sig)
    }

    /// Find a small y ∈ [2, 18] whose point decompresses (prefer torsion-free,
    /// i.e. a legitimate prime-subgroup point). Such a y has a second,
    /// non-canonical encoding y+p — the §3 non-canonical-encoding edge.
    fn dual_encoding_point() -> ([u8; 32], [u8; 32]) {
        use curve25519_dalek::edwards::CompressedEdwardsY;
        let mut fallback = None;
        for y in 2u64..=18 {
            let mut enc = [0u8; 32];
            enc[..8].copy_from_slice(&y.to_le_bytes());
            if let Some(pt) = CompressedEdwardsY(enc).decompress() {
                if !pt.is_small_order() {
                    let nc = le_add(&enc, &FIELD_P_LE); // y + p: same point, non-canonical
                    assert!(nc[31] & 0x80 == 0, "construction stays below 2^255");
                    if pt.is_torsion_free() {
                        return (enc, nc);
                    }
                    fallback = Some((enc, nc));
                }
            }
        }
        fallback.expect("some y in 2..=18 decompresses to a non-small-order point")
    }

    #[test]
    fn strict_predicate_rejects_noncanonical_s() {
        let (node, msg, sig) = valid_triple();
        // S' = S + ℓ (still < 2^256): same integer mod ℓ, non-canonical.
        let mut s = [0u8; 32];
        s.copy_from_slice(&sig[32..]);
        let s_nc = le_add(&s, &GROUP_ORDER_L_LE);
        let mut sig_bad = sig;
        sig_bad[32..].copy_from_slice(&s_nc);
        assert!(matches!(
            verify_strict(&node, &msg, &sig_bad),
            Err(VerifyError::NonCanonicalS)
        ));
        // S = ℓ exactly.
        let mut sig_bad = sig;
        sig_bad[32..].copy_from_slice(&GROUP_ORDER_L_LE);
        assert!(matches!(
            verify_strict(&node, &msg, &sig_bad),
            Err(VerifyError::NonCanonicalS)
        ));
    }

    #[test]
    fn strict_predicate_rejects_noncanonical_r() {
        let (node, msg, sig) = valid_triple();
        let (_canonical, noncanonical) = dual_encoding_point();
        let mut sig_bad = sig;
        sig_bad[..32].copy_from_slice(&noncanonical);
        assert!(matches!(
            verify_strict(&node, &msg, &sig_bad),
            Err(VerifyError::NonCanonicalR)
        ));
    }

    #[test]
    fn strict_predicate_rejects_noncanonical_a() {
        let (_canonical, noncanonical) = dual_encoding_point();
        let (_node, msg, sig) = valid_triple();
        let bad_node = NodeId::from(noncanonical);
        assert!(matches!(
            verify_strict(&bad_node, &msg, &sig),
            Err(VerifyError::NonCanonicalPublicKey)
        ));
    }

    #[test]
    fn strict_predicate_rejects_small_order_r_and_a() {
        // Identity-point encoding (y = 1): canonical, decompresses, order 1.
        let identity = {
            let mut b = [0u8; 32];
            b[0] = 0x01;
            b
        };
        // Small-order R with an otherwise-valid key/message.
        let (node, msg, sig) = valid_triple();
        let mut sig_bad = sig;
        sig_bad[..32].copy_from_slice(&identity);
        assert!(matches!(
            verify_strict(&node, &msg, &sig_bad),
            Err(VerifyError::InvalidSignature)
        ));

        // The classic cofactored-verification acceptance: A = identity,
        // R = identity, S = 0 passes ZIP-215/cofactored verifiers — including
        // dalek's banned default verify() — and MUST be rejected by the §3
        // strict predicate (small-order A and R).
        let weak_node = NodeId::from(identity);
        let mut sig_weak = [0u8; 64];
        sig_weak[..32].copy_from_slice(&identity);
        assert!(matches!(
            verify_strict(&weak_node, b"any message", &sig_weak),
            Err(VerifyError::InvalidSignature)
        ));
    }

    #[test]
    fn strict_predicate_rejects_malformed_a() {
        use curve25519_dalek::edwards::CompressedEdwardsY;
        // Find a small canonical y that fails decompression entirely.
        let mut target = None;
        for y in 2u64..=64 {
            let mut enc = [0u8; 32];
            enc[..8].copy_from_slice(&y.to_le_bytes());
            if CompressedEdwardsY(enc).decompress().is_none() {
                target = Some(enc);
                break;
            }
        }
        let bad = target.expect("some small y is non-decompressible");
        let (_node, msg, sig) = valid_triple();
        assert!(matches!(
            verify_strict(&NodeId::from(bad), &msg, &sig),
            Err(VerifyError::MalformedPublicKey)
        ));
    }

    #[test]
    fn strict_predicate_rejects_wrong_message_and_key() {
        let (node, msg, sig) = valid_triple();
        assert!(verify_strict(&node, &[0x73], &sig).is_err());
        let other = NodeId::from(hex32(
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
        ));
        assert!(verify_strict(&other, &msg, &sig).is_err());
    }

    // ------------------------------------------------------------------
    // §3 — X25519 low-order ingress + §6.4 all-zero abort.
    // ------------------------------------------------------------------

    #[test]
    fn x25519_low_order_rejected_on_ingress() {
        // The RFC 7748 §6.1 / libsodium-blacklist class: u = 0, 1, the two
        // order-8 points, and the non-canonical wraps p−1, p, p+1.
        let blacklist: [[u8; 32]; 7] = [
            [0u8; 32],
            {
                let mut b = [0u8; 32];
                b[0] = 1;
                b
            },
            hex32("e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800"),
            hex32("5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157"),
            {
                // p − 1
                let mut b = [0xffu8; 32];
                b[0] = 0xec;
                b[31] = 0x7f;
                b
            },
            {
                // p (non-canonical zero)
                let mut b = [0xffu8; 32];
                b[0] = 0xed;
                b[31] = 0x7f;
                b
            },
            {
                // p + 1 (non-canonical one)
                let mut b = [0xffu8; 32];
                b[0] = 0xee;
                b[31] = 0x7f;
                b
            },
        ];
        for (i, bad) in blacklist.iter().enumerate() {
            assert!(
                matches!(
                    validate_x25519_public_key(bad),
                    Err(EcdhError::LowOrderPublicKey)
                ),
                "blacklist entry {i} must be rejected"
            );
        }
        // Legitimate keys pass: the basepoint and both RFC 7748 parties.
        let mut basepoint = [0u8; 32];
        basepoint[0] = 9;
        assert!(validate_x25519_public_key(&basepoint).is_ok());
        assert!(
            validate_x25519_public_key(&hex32(
                "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
            ))
            .is_ok()
        );
        assert!(
            validate_x25519_public_key(&hex32(
                "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"
            ))
            .is_ok()
        );
    }

    #[test]
    fn ecdh_aborts_on_all_zero_output() {
        // §6.4 step 2 / RFC 7748 §6.1: a low-order peer key forces an
        // all-zero shared secret; the exchange MUST abort. (The ingress
        // low-order check fires first — the was_contributory arm is kept as
        // defense-in-depth; every input that would reach it is already
        // rejected here. The observable behavior is the mandated abort.)
        let sign_seed =
            hex32("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let mut b = [0u8; 64];
        b[..32].copy_from_slice(&sign_seed);
        b[32..].copy_from_slice(&ALICE_SCALAR);
        let ks = Keyset::from_bytes(&b).unwrap();

        assert!(ks.ecdh(&[0u8; 32]).is_err());
        let mut one = [0u8; 32];
        one[0] = 1;
        assert!(ks.ecdh(&one).is_err());
    }

    // ------------------------------------------------------------------
    // §4.5 weak-key self-test vectors (§14).
    // ------------------------------------------------------------------

    #[test]
    fn weak_key_self_test_vectors() {
        let good_seed =
            hex32("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let mut base = [0u8; 64];
        base[..32].copy_from_slice(&good_seed);
        base[32..].copy_from_slice(&ALICE_SCALAR);
        assert!(weak_key_self_test(&base).is_ok());

        // All-zero, either half.
        let mut v = base;
        v[..32].copy_from_slice(&[0u8; 32]);
        assert!(matches!(weak_key_self_test(&v), Err(WeakKeyError::AllZero)));
        let mut v = base;
        v[32..].copy_from_slice(&[0u8; 32]);
        assert!(matches!(weak_key_self_test(&v), Err(WeakKeyError::AllZero)));

        // Uniform byte (0x11 × 32).
        let mut v = base;
        v[..32].copy_from_slice(&[0x11u8; 32]);
        assert!(matches!(
            weak_key_self_test(&v),
            Err(WeakKeyError::UniformByte)
        ));

        // Counter patterns: ascending from 0, ascending from 0x40, descending
        // from 0xff.
        let mut asc = [0u8; 32];
        for (i, b) in asc.iter_mut().enumerate() {
            *b = i as u8;
        }
        let mut v = base;
        v[32..].copy_from_slice(&asc);
        assert!(matches!(
            weak_key_self_test(&v),
            Err(WeakKeyError::CounterPattern)
        ));

        let mut asc2 = [0u8; 32];
        for (i, b) in asc2.iter_mut().enumerate() {
            *b = 0x40u8.wrapping_add(i as u8);
        }
        let mut v = base;
        v[..32].copy_from_slice(&asc2);
        assert!(matches!(
            weak_key_self_test(&v),
            Err(WeakKeyError::CounterPattern)
        ));

        let mut desc = [0u8; 32];
        for (i, b) in desc.iter_mut().enumerate() {
            *b = 0xffu8.wrapping_sub(i as u8);
        }
        let mut v = base;
        v[..32].copy_from_slice(&desc);
        assert!(matches!(
            weak_key_self_test(&v),
            Err(WeakKeyError::CounterPattern)
        ));

        // from_bytes applies the same gate (§13 load path).
        let mut v = [0u8; 64];
        v[..32].copy_from_slice(&asc);
        assert!(matches!(
            Keyset::from_bytes(&v),
            Err(KeysetError::WeakKey(WeakKeyError::CounterPattern))
        ));
    }

    // ------------------------------------------------------------------
    // §2/§4 — keygen, restore derivation, domain-tagged signing.
    // ------------------------------------------------------------------

    #[test]
    fn keygen_round_trip_via_mnemonic() {
        for words in [WordCount::Twelve, WordCount::TwentyFour] {
            let (mnemonic, ks) = Keyset::generate(words).unwrap();
            assert_eq!(mnemonic.word_count(), words.word_count());
            // §8.4: restore from the phrase reproduces the identical keyset.
            let restored = Keyset::derive_from_mnemonic(&mnemonic.phrase()).unwrap();
            assert_eq!(ks.public(), restored.public());
            assert_eq!(ks.node_id(), restored.node_id());
            assert_eq!(&ks.to_bytes()[..], &restored.to_bytes()[..]);
        }
    }

    /// §14 — the project keyset vector for the canonical mnemonic: PRINTED
    /// for freezing by the vector task (run with --nocapture).
    #[test]
    fn project_keyset_vector() {
        let ks = Keyset::derive_from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let pubks = ks.public();
        println!("Project keyset vector (frozen by the §14 vector task):");
        println!("  mnemonic = abandon×11 about (BIP-39, empty passphrase)");
        println!("  NodeId (Ed25519 pubkey) = {}", to_hex(pubks.sign_pubkey()));
        println!("  X25519 pubkey           = {}", to_hex(pubks.enc_pubkey()));
        println!("  public keyset (64B)     = {}", to_hex(&pubks.to_bytes()));

        // Frozen values (the derivation chain is pinned byte-exact by the
        // BIP-39 seed vector in bip39.rs and the SLIP-0010 vectors in
        // slip0010.rs).
        assert_eq!(
            pubks.sign_pubkey(),
            &hex32("6f1d5331e36e16d341fa06c9f622cc8569710f2a62d7491e9be8e953df844f01")[..]
        );
        assert_eq!(
            pubks.enc_pubkey(),
            &hex32("712678456644b8cba1389072ee5dec93d044875ca13c73e217157814bbb99218")[..]
        );
    }

    #[test]
    fn tagged_signing_round_trip_and_domain_separation() {
        let (mnemonic, ks) = Keyset::generate(WordCount::Twelve).unwrap();
        let _ = mnemonic;
        let node = ks.node_id();
        let body = b"\x01announce-body-bytes";

        let sig = ks.sign_with_tag(b"111/announce/v1", body);
        assert!(verify_strict_tagged(&node, b"111/announce/v1", body, &sig).is_ok());

        // Wrong tag: domain confusion MUST fail (§6.1/§14 negative).
        assert!(verify_strict_tagged(&node, b"111/attest-node/v1", body, &sig).is_err());
        // The tag is inside the signed region: verifying the bare body against
        // a tagged signature MUST fail.
        assert!(verify_strict(&node, body, &sig).is_err());
        // Tampered body MUST fail.
        let mut tampered = body.to_vec();
        tampered[1] ^= 1;
        assert!(verify_strict_tagged(&node, b"111/announce/v1", &tampered, &sig).is_err());
    }

    #[test]
    fn public_keyset_bytes_layout_and_ingress() {
        let (_m, ks) = Keyset::generate(WordCount::Twelve).unwrap();
        let pubks = ks.public();
        let bytes = pubks.to_bytes();
        assert_eq!(&bytes[..32], pubks.sign_pubkey());
        assert_eq!(&bytes[32..], pubks.enc_pubkey());
        let parsed = PublicKeyset::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, pubks);
        assert_eq!(parsed.node_id(), ks.node_id());

        // Ingress: a low-order enc_pubkey is rejected (§3, §6.2 rule).
        let mut bad = bytes;
        bad[32..].copy_from_slice(&[0u8; 32]);
        assert!(matches!(
            PublicKeyset::from_bytes(&bad),
            Err(EcdhError::LowOrderPublicKey)
        ));
    }

    #[test]
    fn derive_from_mnemonic_rejects_bad_input() {
        // Bad checksum.
        assert!(Keyset::derive_from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon"
        )
        .is_err());
        // Out-of-profile word count (15 words, checksum-valid).
        let fifteen = ::bip39::Mnemonic::from_entropy(&[0u8; 20]).unwrap();
        assert!(Keyset::derive_from_mnemonic(&fifteen.to_string()).is_err());
    }
}
