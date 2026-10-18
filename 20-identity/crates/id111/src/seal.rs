//! §6.4 — the sealed-packet construction (byte-exact, normative).
//!
//! Implements (owner: T3), per protected packet:
//!
//! 1. fresh ephemeral X25519 keypair `(epk, esk)` per §4 entropy discipline —
//!    `getrandom()` (the named Cat 2+ source, §4.2; a TRNG-seeded CSPRNG is the
//!    sanctioned operational path) drawn directly so entropy failure is a typed
//!    loud abort (§4 rule 2 — no silent weak fallback, and no panic path), the
//!    secret zeroized after use;
//! 2. `shared = X25519(esk, recipient_enc_pubkey)` — MUST abort on all-zero
//!    `shared` (RFC 7748 §6.1, via `was_contributory`; the zero secret is never
//!    returned), and the recipient key is re-checked against the §3 low-order
//!    rule (it MUST have passed at pin time — defense in depth);
//! 3. `key = BLAKE2b-256("111/seal/v1" ‖ shared ‖ epk ‖ rpk)`,
//!    `nonce = BLAKE2b-192("111/seal-n/v1" ‖ epk ‖ rpk)` — the nonce is
//!    derived, never transmitted (a fresh ephemeral per packet makes it unique
//!    per packet);
//! 4. `XChaCha20-Poly1305-Encrypt(key, nonce, plaintext, AD)` with AD = every
//!    cleartext header byte (tag 16B);
//! 5. wire packet = cleartext header ‖ `epk` (32B) ‖ ciphertext+tag (§6.4's
//!    56B overhead = 32B epk + 16B tag + 8B counter, stated honestly).
//!
//! **Wire-format pin (v1, this module owns it):** the cleartext header is
//! serialized as
//!
//! ```text
//! version(1) ‖ crypto_mode(1) ‖ counter(8, LE u64) ‖ extra_len(1) ‖ extra(extra_len)
//! ```
//!
//! so a receiver can locate `epk` unambiguously; `extra_len` is itself one of
//! the cleartext header bytes, so §6.4 step 4's "every cleartext header byte
//! is AD-covered" holds for the whole header. The AD passed to the AEAD is the
//! exact wire slice of the header (never a re-serialization — the §9
//! canonicalization-bug class). `version` is the L5-owned envelope-version
//! field (§6.4 registry note): this module AD-covers and passes it through;
//! gating unknown versions is L5's. `crypto_mode` is the L2-owned registry:
//! **`0x01` = this construction**; unknown or weaker modes are rejected, fail
//! closed, checked BEFORE decryption (§12 rule 1; Meshtastic CVE-2025-53627 —
//! a silent PKI→legacy downgrade displayed as "encrypted").
//!
//! **Inner plaintext pin (v1):** `sender_node_id(32) ‖ plaintext`. The
//! sender's NodeId MUST travel inside the ciphertext (§6.4) — a sealed box
//! authenticates the channel, never authorship (§12 rule 13: MAC auth is not
//! authorship; non-repudiation, if ever wanted, is a Layer-5 inner signature).
//!
//! **Caller-owned inner layout — Layer 5 amendment A2** (`os/messaging/
//! README.md` §3, §19): [`seal_payload`] / [`open_payload`] run the SAME
//! construction (same ephemeral discipline, KDFs, derived nonce, AD = the
//! exact header slice, `crypto_mode` `0x01` gate before decryption, the
//! [`MIN_PACKET_LEN`] size-limit-before-parse rule) but the AEAD plaintext is
//! the caller's bytes verbatim — NO 32-byte sender NodeId is prepended or
//! stripped. Layer 5 authenticates the sender itself (its inner `sender_ref`
//! plus a direction-bound `pair_mac` keyed by the static-static pair key, L5
//! §3), so the Noise-"N" sender gap that the 32 B field papered over is
//! closed above this layer, at 28 B less per packet. Because `pair_mac`
//! covers `header ‖ epk ‖ inner`, [`seal_payload`] takes the inner as a
//! closure that receives the packet's fresh `epk`, and [`open_payload`]
//! returns the wire `epk` beside the header and inner. An empty inner is
//! legal here (a bare-tag packet of exactly [`MIN_PACKET_LEN`] + `extra`
//! bytes).
//!
//! **One wire per version.** The two inner layouts are indistinguishable on
//! the wire — an [`open_payload`] of a [`seal`] packet simply returns
//! `sender_node_id ‖ plaintext` as opaque bytes, and an [`open`] of a
//! [`seal_payload`] packet misreads the caller's first 32 bytes as a NodeId.
//! Nothing in the header tells them apart, so **two inner layouts never share
//! a `version` value on a signed release**: the 32 B form ([`seal`] /
//! [`open`] and the frozen `sealed-packet.json` vectors) is a pre-release
//! gate only, kept for the Cat-1 Monocypher harness, and is removed before
//! the first signed release (L5 spec §3). A host speaks exactly one layout
//! per `version`.
//!
//! **Counter boundary (§6.5):** the counter VALUE is passed into [`seal`] and
//! parsed out of the authenticated header by [`open`]; counter STATE
//! (monotonicity, the 32-entry sliding acceptance window, batch-commit, the
//! J ≥ N forward jump) is the sibling `counters` module's — this module
//! deliberately implements none of it. Consequence, stated plainly: a replayed
//! packet decrypts identically; rejecting replays is the caller's counter
//! check. `open` does not panic on adversarial input: every shortfall is a
//! typed error, and sizes are checked before any parse (§12 rule 7).
//!
//! This is a project-pinned composition of audited primitives — NOT
//! wire-compatible with libsodium `crypto_box_seal` (that is XSalsa20-Poly1305
//! with a differently-derived nonce); nobody calls a sealed-box API verbatim.

use core::fmt;

use blake2::digest::consts::{U24, U32};
use blake2::{Blake2b, Digest};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305};
use zeroize::{Zeroize, Zeroizing};

use crate::keyset::{validate_x25519_public_key, EcdhError, Keyset, NodeId};

// ---------------------------------------------------------------------------
// §6.4 — wire-format constants (the v1 pin)
// ---------------------------------------------------------------------------

/// §6.2/§13 — the envelope `version` field value for v1. Owned by Layer 5;
/// this module AD-covers and passes it through (gating is L5's).
pub const VERSION_V1: u8 = 0x01;

/// §6.4 crypto-mode registry (owned by this layer, L2): `0x01` = the
/// construction in this module. Append-only; future modes may never weaken
/// below 0x01's bar. Receivers MUST reject unknown or weaker modes (§12
/// rule 1 — fail closed).
pub const CRYPTO_MODE_V1: u8 = 0x01;

/// Fixed header bytes: `version(1) ‖ crypto_mode(1) ‖ counter(8 LE) ‖
/// extra_len(1)` (the wire-format pin above).
pub const HEADER_FIXED_LEN: usize = 11;

/// Maximum `extra` header bytes (the u8 `extra_len` prefix bounds it).
pub const MAX_EXTRA_LEN: usize = 255;

/// Ephemeral X25519 public key length on the wire (§6.4 step 5).
pub const EPK_LEN: usize = 32;

/// XChaCha20-Poly1305 tag length (§6.4 step 4).
pub const TAG_LEN: usize = 16;

/// Sender NodeId length inside the ciphertext (§6.4: the sender's NodeId MUST
/// travel inside the ciphertext).
pub const SENDER_NODE_ID_LEN: usize = 32;

/// Smallest possible wire packet: fixed header (empty `extra`) ‖ epk ‖ bare
/// tag (empty plaintext). Checked before any parse (§12 rule 7).
pub const MIN_PACKET_LEN: usize = HEADER_FIXED_LEN + EPK_LEN + TAG_LEN;

/// §6.4 step 3 — key-KDF domain tag: `BLAKE2b-256("111/seal/v1" ‖ shared ‖
/// epk ‖ rpk)`.
const KEY_KDF_DOMAIN: &[u8] = b"111/seal/v1";

/// §6.4 step 3 — nonce-derivation domain tag: `BLAKE2b-192("111/seal-n/v1" ‖
/// epk ‖ rpk)`. Derived, never transmitted.
const NONCE_KDF_DOMAIN: &[u8] = b"111/seal-n/v1";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The cleartext packet header (§6.4 step 4/5). Every byte of its serialized
/// form is AEAD associated data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealHeader {
    /// Envelope version (L5-owned field; `0x01` for v1). AD-covered, passed
    /// through — unknown-version gating is Layer 5's, not this module's.
    pub version: u8,
    /// §6.4 crypto-mode registry value. [`CRYPTO_MODE_V1`] (`0x01`) is the
    /// only value this module seals or opens; anything else fails closed.
    pub crypto_mode: u8,
    /// §6.5 per-peer traffic counter VALUE (LE u64 on the wire). State is the
    /// `counters` module's; this module carries the value, AD-covered.
    pub counter: u64,
    /// All other cleartext header bytes (opaque to this layer), at most
    /// [`MAX_EXTRA_LEN`].
    pub extra: Vec<u8>,
}

impl SealHeader {
    /// Convenience constructor: v1 header (version 0x01, crypto-mode 0x01,
    /// empty `extra`) carrying the given §6.5 counter value.
    pub fn v1(counter: u64) -> Self {
        SealHeader {
            version: VERSION_V1,
            crypto_mode: CRYPTO_MODE_V1,
            counter,
            extra: Vec::new(),
        }
    }

    /// Serialize per the v1 wire pin: `version ‖ crypto_mode ‖ counter(LE) ‖
    /// extra_len ‖ extra`.
    fn to_bytes(&self) -> Result<Vec<u8>, SealError> {
        let extra_len = u8::try_from(self.extra.len())
            .map_err(|_| SealError::HeaderTooLarge(self.extra.len()))?;
        let mut out = Vec::with_capacity(HEADER_FIXED_LEN + self.extra.len());
        out.push(self.version);
        out.push(self.crypto_mode);
        out.extend_from_slice(&self.counter.to_le_bytes());
        out.push(extra_len);
        out.extend_from_slice(&self.extra);
        Ok(out)
    }
}

/// The result of a successful [`open`]: the authenticated header, the sender
/// NodeId from inside the ciphertext, and the plaintext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedPacket {
    /// The authenticated cleartext header (parsed from the AD-covered wire
    /// bytes — the `counter` here has NOT been freshness-checked; that is the
    /// caller's §6.5 counters-module duty).
    pub header: SealHeader,
    /// The sender's NodeId (§6.4: travels inside the ciphertext). Channel
    /// authentication only — NOT proof of authorship (§12 rule 13).
    pub sender: NodeId,
    /// The decrypted application plaintext.
    pub plaintext: Vec<u8>,
}

/// [`seal`] failures. Every arm is a typed loud abort — never a panic, never
/// a silent fallback (§4 rule 2, §12 rule 1).
#[derive(Debug)]
pub enum SealError {
    /// §4.2 — the named entropy source (`getrandom()`) failed while drawing
    /// the per-packet ephemeral. Keygen aborts loudly; no weak fallback.
    Entropy(getrandom::Error),
    /// §3/§6.4 step 2 — the recipient's X25519 public key is low-order
    /// (RFC 7748 §6.1), or the shared secret came out all-zero
    /// (non-contributory; the zero secret is never returned).
    Ecdh(EcdhError),
    /// §6.4 registry — `crypto_mode != 0x01`. This module seals the 0x01
    /// construction only; unknown/weaker modes fail closed (§12 rule 1).
    UnsupportedCryptoMode(u8),
    /// `extra` exceeds [`MAX_EXTRA_LEN`] (the u8 `extra_len` wire prefix).
    HeaderTooLarge(usize),
    /// AEAD length bound exceeded (plaintext beyond XChaCha20-Poly1305's
    /// P_MAX — practically unreachable; typed so no path panics).
    PlaintextTooLarge,
}

impl fmt::Display for SealError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SealError::Entropy(e) => write!(f, "§4.2: getrandom() failed: {e}"),
            SealError::Ecdh(e) => write!(f, "{e}"),
            SealError::UnsupportedCryptoMode(m) => {
                write!(
                    f,
                    "§6.4: unsupported crypto_mode 0x{m:02x} (registry: 0x01 only)"
                )
            }
            SealError::HeaderTooLarge(n) => {
                write!(f, "§6.4: header extra {n}B exceeds {MAX_EXTRA_LEN}B")
            }
            SealError::PlaintextTooLarge => write!(f, "§6.4: plaintext exceeds AEAD length bound"),
        }
    }
}

impl std::error::Error for SealError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            // getrandom 0.2's Error does not implement std::error::Error
            // without its "std" feature (dep set frozen — no feature change).
            SealError::Entropy(_) => None,
            SealError::Ecdh(e) => Some(e),
            _ => None,
        }
    }
}

impl From<EcdhError> for SealError {
    fn from(e: EcdhError) -> Self {
        SealError::Ecdh(e)
    }
}

/// [`open`] failures. Decryption is all-or-nothing: a tampered header, epk, or
/// ciphertext, a wrong recipient key, or a forged packet all surface as
/// [`OpenError::AuthFailed`] — no partial plaintext ever escapes.
#[derive(Debug)]
pub enum OpenError {
    /// §12 rule 7 — the packet is shorter than the fixed minimum, or the
    /// header's `extra_len` overruns the packet. Size-limit before parse.
    Truncated,
    /// §6.4 registry — `crypto_mode != 0x01`; rejected BEFORE decryption
    /// (fail closed; Meshtastic CVE-2025-53627 downgrade lesson).
    UnsupportedCryptoMode(u8),
    /// §3/§6.4 step 2 — the wire `epk` is low-order (RFC 7748 §6.1) or the
    /// shared secret came out all-zero (non-contributory abort).
    Ecdh(EcdhError),
    /// AEAD tag verification failed — wrong recipient key, or tampered
    /// header/AD, epk, ciphertext, or tag.
    AuthFailed,
    /// The decrypted inner plaintext is shorter than the 32B sender NodeId —
    /// not a §6.4 plaintext. Produced by [`open`] only; [`open_payload`]
    /// (caller-owned inner, L5 amendment A2) has no inner minimum.
    PlaintextTooShort,
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Truncated => write!(f, "§6.4: packet truncated (size-limit before parse)"),
            OpenError::UnsupportedCryptoMode(m) => {
                write!(
                    f,
                    "§6.4: unsupported crypto_mode 0x{m:02x} (registry: 0x01 only)"
                )
            }
            OpenError::Ecdh(e) => write!(f, "{e}"),
            OpenError::AuthFailed => {
                write!(
                    f,
                    "§6.4: AEAD authentication failed (wrong key or tampered packet)"
                )
            }
            OpenError::PlaintextTooShort => {
                write!(
                    f,
                    "§6.4: inner plaintext shorter than the 32B sender NodeId"
                )
            }
        }
    }
}

impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            OpenError::Ecdh(e) => Some(e),
            _ => None,
        }
    }
}

impl From<EcdhError> for OpenError {
    fn from(e: EcdhError) -> Self {
        OpenError::Ecdh(e)
    }
}

// ---------------------------------------------------------------------------
// §6.4 step 3 — the KDFs (byte-exact, normative)
// ---------------------------------------------------------------------------

/// §6.4 step 3 — `key = BLAKE2b-256("111/seal/v1" ‖ shared ‖ epk ‖ rpk)`.
fn derive_key(shared: &[u8; 32], epk: &[u8; 32], rpk: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let mut key = Zeroizing::new([0u8; 32]);
    let mut h = Blake2b::<U32>::new();
    h.update(KEY_KDF_DOMAIN);
    h.update(shared);
    h.update(epk);
    h.update(rpk);
    key.copy_from_slice(&h.finalize());
    key
}

/// §6.4 step 3 — `nonce = BLAKE2b-192("111/seal-n/v1" ‖ epk ‖ rpk)` (24B —
/// XChaCha20's nonce size). Derived, never transmitted; a fresh ephemeral per
/// packet makes it unique per packet (§12 rule 9's 24-byte XChaCha nonce
/// class, derived rather than random).
fn derive_nonce(epk: &[u8; 32], rpk: &[u8; 32]) -> [u8; 24] {
    let mut nonce = [0u8; 24];
    let mut h = Blake2b::<U24>::new();
    h.update(NONCE_KDF_DOMAIN);
    h.update(epk);
    h.update(rpk);
    nonce.copy_from_slice(&h.finalize());
    nonce
}

// ---------------------------------------------------------------------------
// §6.4 — seal / open
// ---------------------------------------------------------------------------

/// §6.4 — seal a packet to a recipient's announced X25519 encryption key.
///
/// Draws a fresh ephemeral per call (§4.2 getrandom path; per-packet key
/// independence — two seals of the same inputs produce different packets).
/// `header.crypto_mode` MUST be [`CRYPTO_MODE_V1`] — this module seals the
/// 0x01 construction only (fail closed, §12 rule 1). `sender` is placed
/// inside the ciphertext (§6.4's authorship rule); `header.counter` is the
/// §6.5 counter VALUE from the caller's counters module.
pub fn seal(
    recipient_enc_pubkey: &[u8; 32],
    header: &SealHeader,
    sender: &NodeId,
    plaintext: &[u8],
) -> Result<Vec<u8>, SealError> {
    // §4.2 — the ephemeral draw: getrandom() directly (x25519-dalek's
    // getrandom-enabled constructor path panics inside OsRng on failure;
    // the §4 rule 2 loud abort is typed here instead).
    let mut esk_bytes = Zeroizing::new([0u8; 32]);
    getrandom::getrandom(&mut esk_bytes[..]).map_err(SealError::Entropy)?;
    seal_with_ephemeral(recipient_enc_pubkey, header, sender, plaintext, &esk_bytes)
}

/// The §6.4 recipe with a caller-supplied ephemeral scalar. Split from
/// [`seal`] so the §14 known-answer vector can pin `esk`; production callers
/// use [`seal`].
fn seal_with_ephemeral(
    recipient_enc_pubkey: &[u8; 32],
    header: &SealHeader,
    sender: &NodeId,
    plaintext: &[u8],
    esk_bytes: &[u8; 32],
) -> Result<Vec<u8>, SealError> {
    // Inner pin (32 B form): sender ‖ plaintext — independent of `epk`.
    seal_core(recipient_enc_pubkey, header, esk_bytes, |_epk| {
        let mut inner = Vec::with_capacity(SENDER_NODE_ID_LEN + plaintext.len());
        inner.extend_from_slice(sender.as_bytes());
        inner.extend_from_slice(plaintext);
        inner
    })
}

/// L5 amendment A2 (messaging spec §3) — seal a packet whose inner plaintext
/// layout is caller-owned: the bytes returned by `inner` are encrypted
/// verbatim, with NO 32-byte sender NodeId prepended (Layer 5 authenticates
/// the sender with its own `pair_mac`).
///
/// `inner` is called exactly once, after the crypto-mode, header-size,
/// recipient low-order and contributory-ECDH checks have passed, with this
/// packet's fresh ephemeral public key `epk` — the 32 bytes that
/// sit on the wire between the header and the ciphertext. Layer 5 needs it
/// because `pair_mac` covers `L2 header bytes ‖ epk ‖ inner bytes before
/// auth` (L5 §3); the header bytes are the caller's own [`SealHeader`]
/// serialized per the wire pin above. The closure sees nothing secret. Its
/// returned buffer is zeroized once encrypted.
///
/// Otherwise identical to [`seal`]: a fresh ephemeral per call drawn by
/// `getrandom()` (§4.2; entropy failure is a typed loud abort), the
/// recipient key re-checked against the §3 low-order rule, the same
/// `"111/seal/v1"` / `"111/seal-n/v1"` KDFs, AD = the exact header bytes, and
/// `header.crypto_mode` MUST be [`CRYPTO_MODE_V1`] (fail closed, §12 rule
/// 1). `header.counter` is the §6.5 counter VALUE from the caller's
/// [`crate::counters::CounterStore`]; `header.version` is L5's passthrough
/// field. An empty inner is legal. See the module docs for the
/// one-layout-per-`version` rule.
pub fn seal_payload(
    recipient_enc_pubkey: &[u8; 32],
    header: &SealHeader,
    inner: impl FnOnce(&[u8; 32]) -> Vec<u8>,
) -> Result<Vec<u8>, SealError> {
    let mut esk_bytes = Zeroizing::new([0u8; 32]);
    getrandom::getrandom(&mut esk_bytes[..]).map_err(SealError::Entropy)?;
    seal_core(recipient_enc_pubkey, header, &esk_bytes, inner)
}

/// The shared §6.4 construction. `inner(epk)` yields the AEAD plaintext:
/// [`seal_with_ephemeral`] returns `sender ‖ plaintext` (ignoring `epk`),
/// [`seal_payload`] forwards the caller's closure. Byte-for-byte the recipe
/// the frozen `sealed-packet.json` KAT pins.
fn seal_core(
    recipient_enc_pubkey: &[u8; 32],
    header: &SealHeader,
    esk_bytes: &[u8; 32],
    inner: impl FnOnce(&[u8; 32]) -> Vec<u8>,
) -> Result<Vec<u8>, SealError> {
    if header.crypto_mode != CRYPTO_MODE_V1 {
        return Err(SealError::UnsupportedCryptoMode(header.crypto_mode));
    }
    let header_bytes = header.to_bytes()?;

    // §6.4 step 2 — the recipient's announced enc_pubkey MUST have passed the
    // §3 low-order check at pin time; re-checked here (defense in depth).
    validate_x25519_public_key(recipient_enc_pubkey)?;

    // Step 1 (key material): the ephemeral secret, zeroized after use.
    let mut esk = x25519_dalek::StaticSecret::from(*esk_bytes);
    let epk = x25519_dalek::PublicKey::from(&esk).to_bytes();

    // Step 2: shared = X25519(esk, rpk) — abort on all-zero (RFC 7748 §6.1).
    let shared = esk.diffie_hellman(&x25519_dalek::PublicKey::from(*recipient_enc_pubkey));
    esk.zeroize();
    if !shared.was_contributory() {
        return Err(EcdhError::NonContributory.into());
    }
    let shared = Zeroizing::new(shared.to_bytes());

    // Step 3: the KDFs.
    let key = derive_key(&shared, &epk, recipient_enc_pubkey);
    let nonce = derive_nonce(&epk, recipient_enc_pubkey);

    // Step 4: XChaCha20-Poly1305 over the AEAD plaintext with AD = the
    // exact header bytes (every cleartext header byte AD-covered). The
    // plaintext buffer is wiped on every exit path.
    let aead_pt = Zeroizing::new(inner(&epk));
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let ciphertext = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: &aead_pt,
                aad: &header_bytes,
            },
        )
        .map_err(|_| SealError::PlaintextTooLarge)?;

    // Step 5: packet = header ‖ epk ‖ ciphertext+tag.
    let mut packet = Vec::with_capacity(header_bytes.len() + EPK_LEN + ciphertext.len());
    packet.extend_from_slice(&header_bytes);
    packet.extend_from_slice(&epk);
    packet.extend_from_slice(&ciphertext);
    Ok(packet)
}

/// §6.4 — open a sealed packet with the recipient's private keyset.
///
/// Parses the header from the wire (size-limited before parse, §12 rule 7),
/// rejects unknown crypto-modes BEFORE decryption (fail closed), and returns
/// the authenticated header, sender NodeId, and plaintext. The header's
/// `counter` is the sender-asserted §6.5 VALUE: replay/monotonicity checking
/// against the 32-entry acceptance window is the CALLER's counters-module
/// duty — a replayed packet decrypts identically, by design of this layer.
pub fn open(recipient: &Keyset, packet: &[u8]) -> Result<OpenedPacket, OpenError> {
    let (header, _epk, inner) = open_raw(recipient, packet)?;
    // The decrypted inner is wiped once split (the plaintext copy is the
    // caller's to keep).
    let inner = Zeroizing::new(inner);

    // Inner pin: sender_node_id(32) ‖ plaintext.
    if inner.len() < SENDER_NODE_ID_LEN {
        return Err(OpenError::PlaintextTooShort);
    }
    let mut sender = [0u8; 32];
    sender.copy_from_slice(&inner[..SENDER_NODE_ID_LEN]);
    let plaintext = inner[SENDER_NODE_ID_LEN..].to_vec();

    Ok(OpenedPacket {
        header,
        sender: NodeId::from(sender),
        plaintext,
    })
}

/// L5 amendment A2 (messaging spec §3) — open a packet whose inner layout is
/// caller-owned: returns the AD-authenticated header, the wire `epk` (32 B,
/// the input Layer 5's `pair_mac` covers between the header and the inner
/// bytes, L5 §3), and the decrypted inner bytes verbatim (NO 32-byte sender
/// NodeId is split off; an empty inner is legal).
///
/// Same gates as [`open`], in the same order: size-limit before parse
/// ([`MIN_PACKET_LEN`], then `extra_len` against the packet length — §12 rule
/// 7), `crypto_mode != 0x01` rejected BEFORE any ECDH or decryption (fail
/// closed), the §3 low-order / RFC 7748 §6.1 all-zero checks on the wire
/// `epk`, AD = the exact wire header slice. The header's `counter` is NOT
/// freshness-checked here — offer it to
/// [`crate::counters::CounterStore::observe_counter`] (L5 amendment A11) or
/// [`crate::counters::CounterStore::on_receive`]; sender authentication is the
/// caller's `pair_mac` (L5 §3), which MUST be verified before any replay or
/// store state is touched.
pub fn open_payload(
    recipient: &Keyset,
    packet: &[u8],
) -> Result<(SealHeader, [u8; 32], Vec<u8>), OpenError> {
    open_raw(recipient, packet)
}

/// The shared §6.4 open: parse + gate + decrypt, returning the header, the
/// wire `epk` and the raw AEAD plaintext. [`open`] splits the 32 B sender off
/// it; [`open_payload`] returns it whole.
fn open_raw(
    recipient: &Keyset,
    packet: &[u8],
) -> Result<(SealHeader, [u8; 32], Vec<u8>), OpenError> {
    // §12 rule 7 — size-limit before any parse.
    if packet.len() < MIN_PACKET_LEN {
        return Err(OpenError::Truncated);
    }
    let version = packet[0];
    let crypto_mode = packet[1];
    // §6.4 registry: unknown or weaker mode ⇒ reject BEFORE decryption.
    if crypto_mode != CRYPTO_MODE_V1 {
        return Err(OpenError::UnsupportedCryptoMode(crypto_mode));
    }
    let mut counter_le = [0u8; 8];
    counter_le.copy_from_slice(&packet[2..10]);
    let counter = u64::from_le_bytes(counter_le);
    let extra_len = packet[10] as usize;
    let header_len = HEADER_FIXED_LEN + extra_len;
    if packet.len() < header_len + EPK_LEN + TAG_LEN {
        return Err(OpenError::Truncated);
    }
    let extra = &packet[HEADER_FIXED_LEN..header_len];
    let mut epk = [0u8; 32];
    epk.copy_from_slice(&packet[header_len..header_len + EPK_LEN]);
    let ciphertext = &packet[header_len + EPK_LEN..];

    // Steps 2–3: shared = X25519(recipient_esk, epk) with the §3 low-order
    // ingress check and the RFC 7748 §6.1 all-zero abort (Keyset::ecdh does
    // both; the zero secret is never returned). Then the KDFs.
    let shared = recipient.ecdh(&epk)?;
    let rpk = recipient.x25519_public();
    let key = derive_key(&shared, &epk, &rpk);
    let nonce = derive_nonce(&epk, &rpk);

    // Step 4: decrypt with AD = the exact wire header bytes (never a
    // re-serialization — the §9 canonicalization-bug class).
    let ad = &packet[..header_len];
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let inner = cipher
        .decrypt(
            (&nonce).into(),
            Payload {
                msg: ciphertext,
                aad: ad,
            },
        )
        .map_err(|_| OpenError::AuthFailed)?;

    Ok((
        SealHeader {
            version,
            crypto_mode,
            counter,
            extra: extra.to_vec(),
        },
        epk,
        inner,
    ))
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

    fn to_hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    // ------------------------------------------------------------------
    // Fixed test key material (RFC 7748 / RFC 8032 anchors, as keyset.rs).
    // ------------------------------------------------------------------

    /// RFC 7748 Alice's scalar (used as the recipient's ECDH half).
    const ALICE_SCALAR: [u8; 32] = [
        0x77, 0x07, 0x6d, 0x0a, 0x73, 0x18, 0xa5, 0x7d, 0x3c, 0x16, 0xc1, 0x72, 0x51, 0xb2, 0x66,
        0x45, 0xdf, 0x4c, 0x2f, 0x87, 0xeb, 0xc0, 0x99, 0x2a, 0xb1, 0x77, 0xfb, 0xa5, 0x1d, 0xb9,
        0x2c, 0x2a,
    ];
    /// RFC 7748 Bob's scalar.
    const BOB_SCALAR: [u8; 32] = [
        0x5d, 0xab, 0x08, 0x7e, 0x62, 0x4a, 0x8a, 0x4b, 0x79, 0xe1, 0x7f, 0x8b, 0x83, 0x80, 0x0e,
        0xe6, 0x6f, 0x3b, 0xb1, 0x29, 0x26, 0x18, 0xb6, 0xfd, 0x1c, 0x2f, 0x8b, 0x27, 0xff, 0x88,
        0xe0, 0xeb,
    ];
    /// RFC 8032 TEST 1 private seed (the sign half is unused by seal; any
    /// non-weak value passes the §4.5 self-test).
    const SIGN_SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    const RFC7748_ALICE_PUB: &str =
        "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";
    const RFC7748_BOB_PUB: &str =
        "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f";
    const RFC7748_SHARED: &str = "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742";

    /// The frozen §14 project keyset's NodeId (keyset.rs
    /// `project_keyset_vector`) — the KAT sender.
    const PROJECT_NODE_ID: &str =
        "6f1d5331e36e16d341fa06c9f622cc8569710f2a62d7491e9be8e953df844f01";

    fn keyset_with_scalar(scalar: [u8; 32]) -> Keyset {
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&SIGN_SEED);
        bytes[32..].copy_from_slice(&scalar);
        Keyset::from_bytes(&bytes).unwrap()
    }

    fn alice_keyset() -> Keyset {
        keyset_with_scalar(ALICE_SCALAR)
    }

    fn bob_keyset() -> Keyset {
        keyset_with_scalar(BOB_SCALAR)
    }

    // ------------------------------------------------------------------
    // Round-trips.
    // ------------------------------------------------------------------

    #[test]
    fn round_trip_basic() {
        let bob = bob_keyset();
        let sender = NodeId::from(hex32(PROJECT_NODE_ID));
        let header = SealHeader {
            version: VERSION_V1,
            crypto_mode: CRYPTO_MODE_V1,
            counter: 0x0102_0304_0506_0708,
            extra: b"L5-hints".to_vec(),
        };
        let plaintext = b"payload bytes";
        let packet = seal(&bob.x25519_public(), &header, &sender, plaintext).unwrap();

        // Wire shape: header(11+8 extra) ‖ epk(32) ‖ ct(32 sender + 13 pt) ‖ tag(16).
        assert_eq!(
            packet.len(),
            11 + 8 + EPK_LEN + SENDER_NODE_ID_LEN + plaintext.len() + TAG_LEN
        );

        let opened = open(&bob, &packet).unwrap();
        assert_eq!(opened.header, header);
        assert_eq!(opened.sender, sender);
        assert_eq!(opened.plaintext, plaintext);
    }

    #[test]
    fn round_trip_empty_plaintext_and_extra() {
        let bob = bob_keyset();
        let sender = NodeId::from([0x5au8; 32]);
        let packet = seal(&bob.x25519_public(), &SealHeader::v1(0), &sender, b"").unwrap();
        // Minimum real packet: 11 header ‖ 32 epk ‖ 32 sender ‖ 16 tag = 91B.
        assert_eq!(packet.len(), MIN_PACKET_LEN + SENDER_NODE_ID_LEN);
        let opened = open(&bob, &packet).unwrap();
        assert_eq!(opened.header, SealHeader::v1(0));
        assert_eq!(opened.sender, sender);
        assert!(opened.plaintext.is_empty());
    }

    #[test]
    fn round_trip_max_extra() {
        let bob = bob_keyset();
        let sender = NodeId::from([0x11u8; 32]);
        let header = SealHeader {
            version: 0x07, // version is L5's passthrough field — AD-covered, not gated here
            crypto_mode: CRYPTO_MODE_V1,
            counter: u64::MAX,
            extra: vec![0xabu8; MAX_EXTRA_LEN],
        };
        let packet = seal(&bob.x25519_public(), &header, &sender, b"x").unwrap();
        let opened = open(&bob, &packet).unwrap();
        assert_eq!(opened.header, header);
    }

    #[test]
    fn fresh_ephemeral_per_packet() {
        // §6.4 step 1: a fresh ephemeral per packet — same inputs seal to
        // different packets, and both open (per-packet key independence).
        let bob = bob_keyset();
        let sender = NodeId::from([0x77u8; 32]);
        let header = SealHeader::v1(7);
        let p1 = seal(&bob.x25519_public(), &header, &sender, b"same").unwrap();
        let p2 = seal(&bob.x25519_public(), &header, &sender, b"same").unwrap();
        assert_ne!(p1, p2);
        assert_eq!(open(&bob, &p1).unwrap().plaintext, b"same");
        assert_eq!(open(&bob, &p2).unwrap().plaintext, b"same");
    }

    // ------------------------------------------------------------------
    // Negatives — wrong key, tampering, truncation, downgrade, low-order.
    // ------------------------------------------------------------------

    #[test]
    fn wrong_recipient_key_fails_auth() {
        let bob = bob_keyset();
        let alice = alice_keyset();
        let sender = NodeId::from([0x01u8; 32]);
        let packet = seal(&bob.x25519_public(), &SealHeader::v1(1), &sender, b"hi").unwrap();
        // Alice's keyset derives a different shared secret → tag mismatch.
        assert!(matches!(open(&alice, &packet), Err(OpenError::AuthFailed)));
    }

    #[test]
    fn tampered_ad_fields_fail_auth() {
        let bob = bob_keyset();
        let sender = NodeId::from([0x01u8; 32]);
        let header = SealHeader {
            version: VERSION_V1,
            crypto_mode: CRYPTO_MODE_V1,
            counter: 42,
            extra: b"hdr".to_vec(),
        };
        let packet = seal(&bob.x25519_public(), &header, &sender, b"body").unwrap();

        // Every cleartext header byte is AD-covered (§6.4 step 4): version…
        let mut tampered = packet.clone();
        tampered[0] ^= 0x01;
        assert!(matches!(open(&bob, &tampered), Err(OpenError::AuthFailed)));
        // …counter (a replayed/rolled counter value breaks the tag)…
        let mut tampered = packet.clone();
        tampered[2] ^= 0x01;
        assert!(matches!(open(&bob, &tampered), Err(OpenError::AuthFailed)));
        let mut tampered = packet.clone();
        tampered[9] ^= 0x80;
        assert!(matches!(open(&bob, &tampered), Err(OpenError::AuthFailed)));
        // …extra_len (shifting the epk boundary fails somewhere — never panics)…
        let mut tampered = packet.clone();
        tampered[10] ^= 0x01;
        assert!(open(&bob, &tampered).is_err());
        // …and extra bytes.
        let mut tampered = packet.clone();
        tampered[11] ^= 0x01;
        assert!(matches!(open(&bob, &tampered), Err(OpenError::AuthFailed)));
    }

    #[test]
    fn tampered_epk_and_ciphertext_fail() {
        let bob = bob_keyset();
        let sender = NodeId::from([0x01u8; 32]);
        let packet = seal(&bob.x25519_public(), &SealHeader::v1(1), &sender, b"m").unwrap();
        let header_len = HEADER_FIXED_LEN; // extra empty
        let epk_off = header_len;
        let ct_off = header_len + EPK_LEN;

        // Flip a bit in epk: a different shared secret (auth fails) — or, in
        // the low-order corner, the RFC 7748 §6.1 abort; either way, an error.
        let mut tampered = packet.clone();
        tampered[epk_off] ^= 0x01;
        assert!(open(&bob, &tampered).is_err());

        // Flip a ciphertext byte and a tag byte.
        let mut tampered = packet.clone();
        tampered[ct_off] ^= 0x01;
        assert!(matches!(open(&bob, &tampered), Err(OpenError::AuthFailed)));
        let last = packet.len() - 1;
        let mut tampered = packet;
        tampered[last] ^= 0x01;
        assert!(matches!(open(&bob, &tampered), Err(OpenError::AuthFailed)));
    }

    #[test]
    fn truncated_packet_never_panics() {
        let bob = bob_keyset();
        let sender = NodeId::from([0x01u8; 32]);
        let header = SealHeader {
            version: VERSION_V1,
            crypto_mode: CRYPTO_MODE_V1,
            counter: 5,
            extra: b"xx".to_vec(),
        };
        let packet = seal(&bob.x25519_public(), &header, &sender, b"data").unwrap();
        // Every proper prefix is a typed error (Truncated / AuthFailed), never
        // a panic — adversarial short packets included (§12 rule 7).
        for len in 0..packet.len() {
            assert!(open(&bob, &packet[..len]).is_err(), "prefix len {len}");
        }
        // The full packet still opens.
        assert!(open(&bob, &packet).is_ok());
    }

    #[test]
    fn unknown_crypto_mode_rejected_fail_closed() {
        let bob = bob_keyset();
        let sender = NodeId::from([0x01u8; 32]);

        // Seal side: this module seals 0x01 only (§6.4 registry).
        let bad_header = SealHeader {
            version: VERSION_V1,
            crypto_mode: 0x02,
            counter: 0,
            extra: Vec::new(),
        };
        assert!(matches!(
            seal(&bob.x25519_public(), &bad_header, &sender, b"x"),
            Err(SealError::UnsupportedCryptoMode(0x02))
        ));

        // Open side: a weaker/unknown mode byte is rejected BEFORE decryption
        // (Meshtastic CVE-2025-53627 — no silent downgrade displayed as
        // "encrypted"). 0x00 = "weaker-than-expected", 0x02 = unknown future.
        let packet = seal(&bob.x25519_public(), &SealHeader::v1(1), &sender, b"x").unwrap();
        for mode in [0x00u8, 0x02, 0xff] {
            let mut forged = packet.clone();
            forged[1] = mode;
            assert!(matches!(
                open(&bob, &forged),
                Err(OpenError::UnsupportedCryptoMode(m)) if m == mode
            ));
        }
    }

    #[test]
    fn low_order_keys_rejected_both_directions() {
        let bob = bob_keyset();
        let sender = NodeId::from([0x01u8; 32]);

        // Seal: a low-order recipient enc_pubkey (all-zero is the RFC 7748
        // §6.1 blacklist head) is rejected on ingress.
        assert!(matches!(
            seal(&[0u8; 32], &SealHeader::v1(1), &sender, b"x"),
            Err(SealError::Ecdh(EcdhError::LowOrderPublicKey))
        ));
        let mut one = [0u8; 32];
        one[0] = 1;
        assert!(matches!(
            seal(&one, &SealHeader::v1(1), &sender, b"x"),
            Err(SealError::Ecdh(EcdhError::LowOrderPublicKey))
        ));

        // Open: a forged packet whose epk is low-order aborts the exchange
        // (Keyset::ecdh's ingress check + was_contributory pattern) before any
        // AEAD work.
        let packet = seal(&bob.x25519_public(), &SealHeader::v1(1), &sender, b"x").unwrap();
        let mut forged = packet;
        forged[HEADER_FIXED_LEN..HEADER_FIXED_LEN + EPK_LEN].copy_from_slice(&[0u8; 32]);
        assert!(matches!(
            open(&bob, &forged),
            Err(OpenError::Ecdh(EcdhError::LowOrderPublicKey))
        ));
    }

    #[test]
    fn header_too_large_rejected() {
        let bob = bob_keyset();
        let sender = NodeId::from([0x01u8; 32]);
        let header = SealHeader {
            version: VERSION_V1,
            crypto_mode: CRYPTO_MODE_V1,
            counter: 0,
            extra: vec![0u8; MAX_EXTRA_LEN + 1],
        };
        assert!(matches!(
            seal(&bob.x25519_public(), &header, &sender, b"x"),
            Err(SealError::HeaderTooLarge(256))
        ));
    }

    #[test]
    fn short_inner_plaintext_rejected() {
        // A packet whose decrypted inner is shorter than the 32B sender NodeId
        // is not a §6.4 plaintext (reachable only via a crafted-but-authentic
        // inner — built here with the module's own helpers).
        let bob = bob_keyset();
        let rpk = bob.x25519_public();
        let esk = [0x42u8; 32];
        let header = SealHeader::v1(9);
        let header_bytes = header.to_bytes().unwrap();
        let esk_static = x25519_dalek::StaticSecret::from(esk);
        let epk = x25519_dalek::PublicKey::from(&esk_static).to_bytes();
        let shared = esk_static.diffie_hellman(&x25519_dalek::PublicKey::from(rpk));
        assert!(shared.was_contributory());
        let shared = Zeroizing::new(shared.to_bytes());
        let key = derive_key(&shared, &epk, &rpk);
        let nonce = derive_nonce(&epk, &rpk);
        let cipher = XChaCha20Poly1305::new((&*key).into());
        let ct = cipher
            .encrypt(
                (&nonce).into(),
                Payload {
                    msg: b"too short",
                    aad: &header_bytes,
                },
            )
            .unwrap();
        let mut packet = header_bytes;
        packet.extend_from_slice(&epk);
        packet.extend_from_slice(&ct);
        assert!(matches!(
            open(&bob, &packet),
            Err(OpenError::PlaintextTooShort)
        ));
    }

    #[test]
    fn replay_decrypts_identically_counter_check_is_callers() {
        // §6.5 boundary, stated plainly: a replayed packet decrypts
        // identically at THIS layer — replay rejection is the caller's
        // counters-module freshness check against the 32-entry acceptance
        // window. Documented on `open`; tested here as the honest property.
        let bob = bob_keyset();
        let sender = NodeId::from([0x01u8; 32]);
        let packet = seal(&bob.x25519_public(), &SealHeader::v1(100), &sender, b"msg").unwrap();
        let first = open(&bob, &packet).unwrap();
        let replayed = open(&bob, &packet).unwrap();
        assert_eq!(first, replayed);
        assert_eq!(first.header.counter, 100);
    }

    // ------------------------------------------------------------------
    // L5 amendment A2 — seal_payload / open_payload (caller-owned inner).
    // ------------------------------------------------------------------

    /// RFC 7748 §5.2 first scalar-multiplication input — the A2 KAT's esk
    /// (distinct from the 32 B-form KAT's esk, so no two frozen vectors share
    /// a key/nonce pair).
    const RFC7748_S52_SCALAR1: &str =
        "a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4";

    /// The L5-shaped `extra` (messaging §3): `prio_flags(1) ‖ dest_hint(4)`,
    /// prio = interactive (bits 7–6 = 01), dest_hint = Bob's NodeId[0..4].
    fn l5_extra(bob: &Keyset) -> Vec<u8> {
        let mut extra = vec![0x40u8];
        extra.extend_from_slice(&bob.node_id().as_bytes()[..4]);
        extra
    }

    /// A closure that ignores `epk` and yields fixed inner bytes.
    fn fixed(inner: &[u8]) -> impl FnOnce(&[u8; 32]) -> Vec<u8> + '_ {
        move |_epk| inner.to_vec()
    }

    #[test]
    fn payload_round_trip_has_no_inner_node_id() {
        let bob = bob_keyset();
        let header = SealHeader {
            version: VERSION_V1,
            crypto_mode: CRYPTO_MODE_V1,
            counter: 0x0102_0304_0506_0708,
            extra: l5_extra(&bob),
        };
        let inner = b"caller-owned inner bytes";
        let packet = seal_payload(&bob.x25519_public(), &header, fixed(inner)).unwrap();
        // Wire shape: header(11 + 5) ‖ epk(32) ‖ ct(inner, verbatim) ‖ tag(16)
        // — 32 B smaller than the seal() form for the same plaintext.
        assert_eq!(packet.len(), 11 + 5 + EPK_LEN + inner.len() + TAG_LEN);
        let (h, epk, got) = open_payload(&bob, &packet).unwrap();
        assert_eq!(h, header);
        assert_eq!(&epk[..], &packet[16..16 + EPK_LEN]);
        assert_eq!(got, inner);
    }

    #[test]
    fn payload_closure_sees_the_wire_epk_exactly_once() {
        // L5's pair_mac covers `header ‖ epk ‖ inner-before-auth` (messaging
        // §3): the closure is handed the packet's own epk, the receiver gets
        // the same 32 bytes back from open_payload, and an inner computed
        // from it round-trips.
        let bob = bob_keyset();
        let header = SealHeader::v1(9);
        let mut calls = 0u32;
        let mut seen = [0u8; 32];
        let packet = seal_payload(&bob.x25519_public(), &header, |epk| {
            calls += 1;
            seen = *epk;
            let mut inner = b"mac-over:".to_vec();
            inner.extend_from_slice(epk);
            inner
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(&packet[HEADER_FIXED_LEN..HEADER_FIXED_LEN + EPK_LEN], &seen);
        let (_, epk, inner) = open_payload(&bob, &packet).unwrap();
        assert_eq!(epk, seen);
        assert_eq!(&inner[9..], &seen);
        // The closure is never invoked for a packet that is refused up front.
        let mut invoked = false;
        let bad = SealHeader {
            crypto_mode: 0x02,
            ..SealHeader::v1(0)
        };
        assert!(seal_payload(&bob.x25519_public(), &bad, |_| {
            invoked = true;
            Vec::new()
        })
        .is_err());
        assert!(seal_payload(&[0u8; 32], &header, |_| {
            invoked = true;
            Vec::new()
        })
        .is_err());
        assert!(!invoked);
    }

    #[test]
    fn payload_empty_inner_is_a_bare_tag_packet() {
        let bob = bob_keyset();
        let packet = seal_payload(&bob.x25519_public(), &SealHeader::v1(0), fixed(b"")).unwrap();
        // The MIN_PACKET_LEN rule: header(11) ‖ epk(32) ‖ tag(16) = 59 B is
        // the smallest well-formed packet, and for open_payload it is valid.
        assert_eq!(packet.len(), MIN_PACKET_LEN);
        let (h, _, got) = open_payload(&bob, &packet).unwrap();
        assert_eq!(h, SealHeader::v1(0));
        assert!(got.is_empty());
    }

    #[test]
    fn payload_max_extra_and_version_passthrough() {
        let bob = bob_keyset();
        let header = SealHeader {
            version: 0x07, // L5's field: AD-covered, passed through, not gated here
            crypto_mode: CRYPTO_MODE_V1,
            counter: u64::MAX,
            extra: vec![0xabu8; MAX_EXTRA_LEN],
        };
        let packet = seal_payload(&bob.x25519_public(), &header, fixed(b"x")).unwrap();
        assert_eq!(
            packet.len(),
            HEADER_FIXED_LEN + MAX_EXTRA_LEN + EPK_LEN + 1 + TAG_LEN
        );
        let (h, _, got) = open_payload(&bob, &packet).unwrap();
        assert_eq!(h, header);
        assert_eq!(got, b"x");
        // One byte over the u8 prefix is refused at seal.
        let too_big = SealHeader {
            extra: vec![0u8; MAX_EXTRA_LEN + 1],
            ..header
        };
        assert!(matches!(
            seal_payload(&bob.x25519_public(), &too_big, fixed(b"x")),
            Err(SealError::HeaderTooLarge(256))
        ));
    }

    #[test]
    fn payload_fresh_ephemeral_and_wrong_recipient() {
        let bob = bob_keyset();
        let alice = alice_keyset();
        let header = SealHeader::v1(3);
        let p1 = seal_payload(&bob.x25519_public(), &header, fixed(b"same")).unwrap();
        let p2 = seal_payload(&bob.x25519_public(), &header, fixed(b"same")).unwrap();
        assert_ne!(p1, p2, "§6.4 step 1: a fresh ephemeral per packet");
        assert_eq!(open_payload(&bob, &p1).unwrap().2, b"same");
        assert_eq!(open_payload(&bob, &p2).unwrap().2, b"same");
        assert!(matches!(
            open_payload(&alice, &p1),
            Err(OpenError::AuthFailed)
        ));
    }

    #[test]
    fn payload_wrong_mode_rejected_before_decryption() {
        let bob = bob_keyset();
        for mode in [0x00u8, 0x02, 0xff] {
            let bad = SealHeader {
                crypto_mode: mode,
                ..SealHeader::v1(0)
            };
            assert!(matches!(
                seal_payload(&bob.x25519_public(), &bad, fixed(b"x")),
                Err(SealError::UnsupportedCryptoMode(m)) if m == mode
            ));
        }
        let packet = seal_payload(&bob.x25519_public(), &SealHeader::v1(1), fixed(b"x")).unwrap();
        for mode in [0x00u8, 0x02, 0xff] {
            let mut forged = packet.clone();
            forged[1] = mode;
            // Even with a low-order epk the mode gate fires first: no ECDH,
            // no AEAD work on a downgraded packet.
            forged[HEADER_FIXED_LEN..HEADER_FIXED_LEN + EPK_LEN].copy_from_slice(&[0u8; 32]);
            assert!(matches!(
                open_payload(&bob, &forged),
                Err(OpenError::UnsupportedCryptoMode(m)) if m == mode
            ));
        }
    }

    #[test]
    fn payload_truncation_tamper_and_low_order_fail_closed() {
        let bob = bob_keyset();
        let header = SealHeader {
            version: VERSION_V1,
            crypto_mode: CRYPTO_MODE_V1,
            counter: 77,
            extra: l5_extra(&bob),
        };
        let packet = seal_payload(&bob.x25519_public(), &header, fixed(b"inner")).unwrap();
        // Every strict prefix is a typed error — below MIN_PACKET_LEN it is
        // Truncated before any parse (§12 rule 7) — never a panic.
        for len in 0..packet.len() {
            let r = open_payload(&bob, &packet[..len]);
            assert!(r.is_err(), "prefix {len}");
            if len < MIN_PACKET_LEN {
                assert!(matches!(r, Err(OpenError::Truncated)), "prefix {len}");
            }
        }
        // Every single-bit flip fails (AD, epk, ciphertext, tag).
        for i in 0..packet.len() {
            let mut p = packet.clone();
            p[i] ^= 0x01;
            assert!(open_payload(&bob, &p).is_err(), "byte {i}");
        }
        // A trailing byte breaks the tag; an overrunning extra_len truncates.
        let mut longer = packet.clone();
        longer.push(0);
        assert!(matches!(
            open_payload(&bob, &longer),
            Err(OpenError::AuthFailed)
        ));
        let mut overrun = packet[..MIN_PACKET_LEN].to_vec();
        overrun[10] = 0xff;
        assert!(matches!(
            open_payload(&bob, &overrun),
            Err(OpenError::Truncated)
        ));
        // Low-order recipient (seal) and low-order wire epk (open).
        assert!(matches!(
            seal_payload(&[0u8; 32], &header, fixed(b"x")),
            Err(SealError::Ecdh(EcdhError::LowOrderPublicKey))
        ));
        let mut forged = packet;
        let hl = HEADER_FIXED_LEN + header.extra.len();
        forged[hl..hl + EPK_LEN].copy_from_slice(&[0u8; 32]);
        assert!(matches!(
            open_payload(&bob, &forged),
            Err(OpenError::Ecdh(EcdhError::LowOrderPublicKey))
        ));
    }

    #[test]
    fn the_two_inner_layouts_are_indistinguishable_on_the_wire() {
        // Why "two inner layouts never share a version" (module docs; L5 §3):
        // nothing on the wire tells them apart.
        let bob = bob_keyset();
        let sender = NodeId::from(hex32(PROJECT_NODE_ID));
        let p32 = seal(&bob.x25519_public(), &SealHeader::v1(5), &sender, b"pt").unwrap();
        let (_, _, inner) = open_payload(&bob, &p32).unwrap();
        assert_eq!(&inner[..32], sender.as_bytes());
        assert_eq!(&inner[32..], b"pt");
        // …and the reverse misreads caller bytes as a NodeId, or (short
        // inner) fails the 32 B form's minimum.
        let caller = [0x5au8; 40];
        let pp = seal_payload(&bob.x25519_public(), &SealHeader::v1(6), fixed(&caller)).unwrap();
        let misread = open(&bob, &pp).unwrap();
        assert_eq!(misread.sender.as_bytes(), &[0x5au8; 32]);
        let short = seal_payload(&bob.x25519_public(), &SealHeader::v1(7), fixed(b"")).unwrap();
        assert!(matches!(
            open(&bob, &short),
            Err(OpenError::PlaintextTooShort)
        ));
    }

    /// The A2 known answer (frozen in `test-vectors/layer5/sealed-payload.json`
    /// as `kats[0]`, re-derived there by `crosscheck.py` from the spec text
    /// and by `tests/layer5_amendments.rs` from the primitive crates).
    #[test]
    fn payload_known_answer_vector() {
        let bob = bob_keyset();
        let header = SealHeader {
            version: VERSION_V1,
            crypto_mode: CRYPTO_MODE_V1,
            counter: 1025,
            extra: l5_extra(&bob),
        };
        let inner = b"111network: sealed-payload KAT (L5 A2)";
        let packet = seal_core(
            &hex32(RFC7748_BOB_PUB),
            &header,
            &hex32(RFC7748_S52_SCALAR1),
            fixed(inner),
        )
        .unwrap();
        let (h, epk, got) = open_payload(&bob, &packet).unwrap();
        assert_eq!(h, header);
        assert_eq!(got, inner);
        assert_eq!(
            to_hex(&epk),
            "1c9fd88f45606d932a80c71824ae151d15d73e77de38e8e000852e614fae7019"
        );
        // Frozen wire bytes — byte-equal to the from-scratch Python
        // derivation that generated the vector file.
        let want = hex(concat!(
            "010101040000000000000540d75a9801", // header: v1, mode 1, counter 1025 LE, extra_len 5, prio 0x40 ‖ dest_hint
            "1c9fd88f45606d932a80c71824ae151d15d73e77de38e8e000852e614fae7019", // epk = X25519(RFC 7748 §5.2 scalar 1)
            "d8cf83ea5129cfdd601646f970784ddaf5f456ddefc8fc2a2d2d2a0907127e259e8bb70bebce", // ct (38 B inner, verbatim)
            "f0616d814520981323cc56a11afd0673", // tag(16B)
        ));
        assert_eq!(packet, want, "A2 KAT regression — recipe drift");
        println!("A2 sealed-payload KAT packet = {}", to_hex(&packet));
    }

    // ------------------------------------------------------------------
    // §14 — the sealed-packet known-answer vector (printed for T8's freeze).
    //
    // All inputs fixed: esk = RFC 7748 Alice scalar (so epk = Alice's public
    // key and shared = the RFC 7748 shared value — the vector is anchored to
    // published reference material), recipient = Bob's keyset, sender = the
    // frozen §14 project NodeId. Cross-verified against an independent
    // from-scratch Python reference (X25519 via OpenSSL, BLAKE2b via hashlib,
    // XChaCha20-Poly1305 from scratch — itself validated against the
    // draft-irtf-cfrg-xchacha AEAD vector) before the hex was frozen below.
    // ------------------------------------------------------------------

    #[test]
    fn known_answer_vector() {
        let bob = bob_keyset();
        let sender = NodeId::from(hex32(PROJECT_NODE_ID));
        let header = SealHeader {
            version: VERSION_V1,
            crypto_mode: CRYPTO_MODE_V1,
            counter: 0x0102_0304_0506_0708,
            extra: b"L5".to_vec(),
        };
        let plaintext = b"111network: sealed-packet KAT";
        let packet = seal_with_ephemeral(
            &hex32(RFC7748_BOB_PUB),
            &header,
            &sender,
            plaintext,
            &ALICE_SCALAR,
        )
        .unwrap();

        // Anchors: epk = RFC 7748 Alice public key; the recipe opens under
        // Bob's keyset.
        let header_len = HEADER_FIXED_LEN + header.extra.len();
        assert_eq!(
            &packet[header_len..header_len + EPK_LEN],
            &hex(RFC7748_ALICE_PUB)[..]
        );
        let opened = open(&bob, &packet).unwrap();
        assert_eq!(opened.header, header);
        assert_eq!(opened.sender, sender);
        assert_eq!(opened.plaintext, plaintext);

        // Frozen wire bytes (see the test's doc block for provenance).
        let want = hex(concat!(
            "01010807060504030201024c35", // header: v1, mode 1, counter LE, extra_len 2, "L5"
            "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a", // epk (RFC 7748 Alice)
            "40103faf6dcc15be827549e8e97013842bddf38f3d387abd5be52a306466e3616b", // ct
            "0d6bbbe27ca4456f8551b58630c6baddbd6d80b6e2bc1e5450de58a0e248771491",
            "61f02b41fda0db2c09dcd1", // ct end ‖ tag(16B)
        ));
        assert_eq!(packet, want, "§6.4 KAT regression — recipe drift");

        // Printed for T8's §14 vector freeze (run with --nocapture).
        let esk_static = x25519_dalek::StaticSecret::from(ALICE_SCALAR);
        let shared =
            esk_static.diffie_hellman(&x25519_dalek::PublicKey::from(hex32(RFC7748_BOB_PUB)));
        let shared = Zeroizing::new(shared.to_bytes());
        let epk = hex32(RFC7748_ALICE_PUB);
        let rpk = hex32(RFC7748_BOB_PUB);
        println!("§6.4 sealed-packet KAT (frozen by the §14 vector task):");
        println!("  esk (RFC 7748 Alice scalar) = {}", to_hex(&ALICE_SCALAR));
        println!("  epk                         = {RFC7748_ALICE_PUB}");
        println!("  rpk (RFC 7748 Bob pub)      = {RFC7748_BOB_PUB}");
        println!("  shared (RFC 7748 value)     = {RFC7748_SHARED}");
        println!("  sender NodeId (§14 project) = {PROJECT_NODE_ID}");
        println!(
            "  header bytes                = {}",
            to_hex(&packet[..header_len])
        );
        println!(
            "  key   = BLAKE2b-256(\"111/seal/v1\"‖shared‖epk‖rpk)   = {}",
            to_hex(&derive_key(&shared, &epk, &rpk)[..])
        );
        println!(
            "  nonce = BLAKE2b-192(\"111/seal-n/v1\"‖epk‖rpk)        = {}",
            to_hex(&derive_nonce(&epk, &rpk))
        );
        println!("  plaintext                   = {}", to_hex(plaintext));
        println!("  packet hex                  = {}", to_hex(&packet));
        assert_eq!(&shared[..], &hex(RFC7748_SHARED)[..]);
    }
}
