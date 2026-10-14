//! §3 — the envelope codec. Owner: C2.
//!
//! The wire unit is one sealed packet (`id111::seal`, L2 §6.4, crypto-mode
//! `0x01`) carrying this layer's fields where L2 reserved them:
//!
//! ```text
//! L2 header (AD):  version(1)=0x01 ‖ crypto_mode(1)=0x01 ‖ counter(8 LE) ‖ extra_len(1)=5
//!                  ‖ extra = prio_flags(1) ‖ dest_hint(4)
//!                  ‖ epk(32) ‖ AEAD(inner) ‖ tag(16)
//! inner:           sender_ref(4) ‖ kind(1) ‖ flags(1) ‖ msg_seq = epoch(varint) ‖ n(varint)
//!                  ‖ [frag_idx, frag_count]            varint ×2, if HAS_FRAG
//!                  ‖ [n_ext(1) ‖ (type(1) len(1) value)*]       if HAS_EXT
//!                  ‖ [ack block]                       §6.1, if PIGGYBACK_ACK
//!                  ‖ [card_len(1) ‖ card]              137–169 B announce, if INTRO
//!                  ‖ payload
//!                  ‖ auth = pair_mac(16) ‖ [sig(64) if SIGNED]
//! ```
//!
//! Two framing bytes are this codec's reading of an under-specified layout
//! (recorded as C2 deviations): §3 gives no terminator for the TLV list and
//! no length for the variable-size card, so `HAS_EXT` is followed by a 1 B
//! TLV count and `INTRO` by a 1 B card length.
//!
//! Verification order (§3, §18 rule 1): outer gates (size, version,
//! crypto-mode, reserved `prio_flags` bits, `dest_hint`) before decrypt; then
//! **`pair_mac` before anything else** — only the flags byte at its fixed
//! offset is read first, to locate the auth tail. INTRO is the one stated
//! exception (§7.2 (3)): the inner is parsed (pure, no state) to reach the
//! card, the card is verified as an announce and offered as a provisional
//! candidate, and only then is `pair_mac` checked. `sig` is verified last and
//! never authenticates the sender by itself. Nothing here touches replay
//! state or a store: [`open_envelope`] returns the L2 counter for the caller
//! to offer to the window (`Sealer::observe_counter`, A11) **after** success.

use blake2::digest::consts::{U16, U32};
use blake2::digest::{KeyInit, Mac};
use blake2::{Blake2b, Blake2bMac, Digest};
use id111::announce::{self, Announce};
use id111::keyset::{verify_strict_tagged, NodeId, VerifyError};
use id111::seal::{SealHeader, CRYPTO_MODE_V1, EPK_LEN, HEADER_FIXED_LEN, TAG_LEN, VERSION_V1};
use zeroize::Zeroizing;

use crate::ack::{AckBlock, AckError};
use crate::ids::{put_varint, varint_len, MsgId, MsgSeq, ReadError, Reader, STREAM_1TO1};
use crate::sealer::{Direction, Key32, Sealer, SealerError};

// ---------------------------------------------------------------------------
// Constants (§3)
// ---------------------------------------------------------------------------

/// §3: the envelope `version` this layer emits and accepts.
pub const ENVELOPE_VERSION: u8 = VERSION_V1;
/// §3: `extra_len` is always 5 in v1.
pub const EXTRA_LEN: usize = 5;
/// L2 header bytes on the wire: 11 fixed + 5 `extra` = 16.
pub const HEADER_LEN: usize = HEADER_FIXED_LEN + EXTRA_LEN;
/// §3 size table: outer = 16 header + 32 epk + 16 tag = 64.
pub const OUTER_OVERHEAD: usize = HEADER_LEN + EPK_LEN + TAG_LEN;
/// §3: the L6 shim byte on broadcast bearers (never on `tcp-lan`).
pub const SHIM_LEN: usize = 1;
/// §3: `sender_ref` = sender NodeId[0..4].
pub const SENDER_REF_LEN: usize = 4;
/// §3: `pair_mac` = BLAKE2b-128.
pub const PAIR_MAC_LEN: usize = 16;
/// §3: Ed25519 `sig`.
pub const SIG_LEN: usize = 64;
/// §5 / §18 rule 2: `frag_count ≤ 16`.
pub const MAX_FRAGMENTS: u8 = 16;
/// §3/§7: the INTRO card is an L2 announce, 137–169 B.
pub const CARD_MIN_LEN: usize = announce::BASE_LEN;
/// §3/§7: the INTRO card is an L2 announce, 137–169 B.
pub const CARD_MAX_LEN: usize = announce::MAX_LEN;
/// §8.1: `tcp-lan` frames are ≤ 65,535 B; no sealed packet is larger.
pub const MAX_PACKET_LEN: usize = 65_535;
/// Largest inner plaintext: 65,535 − 64 (§3 table, TCP row: "65,535 − 90"
/// of payload once the 26 B inner header + `pair_mac` are taken).
pub const MAX_INNER_LEN: usize = MAX_PACKET_LEN - OUTER_OVERHEAD;
/// Smallest inner: sender_ref 4 + kind 1 + flags 1 + msg_seq 2 + pair_mac 16.
pub const MIN_INNER_LEN: usize = SENDER_REF_LEN + 1 + 1 + 2 + PAIR_MAC_LEN;
/// Smallest packet this codec opens.
pub const MIN_PACKET_LEN: usize = OUTER_OVERHEAD + MIN_INNER_LEN;

/// §3.1: pair-key KDF tag.
pub const PAIR_KEY_TAG: &[u8] = b"111/pair/v1";
/// §3.1: message signature tag.
pub const MSG_SIG_TAG: &[u8] = b"111/msg/v1";

/// `prio_flags` bit 5 — GROUP (v1 MUST be 0).
pub const PRIO_GROUP: u8 = 0x20;
/// `prio_flags` bit 4 — RATCHET (v1 MUST be 0; the only RATCHET bit).
pub const PRIO_RATCHET: u8 = 0x10;
/// `prio_flags` bits 3–0 — reserved, MUST be 0.
pub const PRIO_RESERVED: u8 = 0x0f;

/// Inner `flags` bits (§3).
pub mod flags {
    /// bit 0 — fragment fields present (§5 Tier 1).
    pub const HAS_FRAG: u8 = 0x01;
    /// bit 1 — ext TLVs present.
    pub const HAS_EXT: u8 = 0x02;
    /// bit 2 — a §6.1 ack block precedes the payload.
    pub const PIGGYBACK_ACK: u8 = 0x04;
    /// bit 3 — `sig` follows `pair_mac`.
    pub const SIGNED: u8 = 0x08;
    /// bit 4 — ask for the payload hash in the ACK (§6.1).
    pub const RECEIPT_STRONG: u8 = 0x10;
    /// bit 5 — the sender's announce rides the card slot (§7.2 (3)).
    pub const INTRO: u8 = 0x20;
    /// bits 6–7 — reserved, MUST be 0 (set ⇒ reject).
    pub const RESERVED: u8 = 0xc0;
}

/// Ext TLV 0x01 — `seen_peer_seq` (two-varint `msg_seq`, §4).
pub const TLV_SEEN_PEER_SEQ: u8 = 0x01;
/// Ext TLV 0x10 — ratchet pubkey, reserved (RATCHET is 0 in v1 ⇒ reject).
pub const TLV_RATCHET_PUBKEY: u8 = 0x10;
/// Ext TLV 0x20 — delegation trailer ⇒ REJECT in v1 (L2 §6.7).
pub const TLV_DELEGATION: u8 = 0x20;
/// Ext TLV 0x21 — `signer_pubkey`, reserved for L2 §6.7 (⇒ reject in v1).
pub const TLV_SIGNER_PUBKEY: u8 = 0x21;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Envelope codec failures. Every variant is a rejection that leaves no
/// state behind (§18 rule 1); none is a panic.
#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    /// Low-level read failure inside the inner layout.
    #[error("envelope: {0}")]
    Read(#[from] ReadError),
    /// The keyed MAC refused its key (unreachable for a 32 B pair key).
    #[error("pair_mac: {0}")]
    MacKey(#[from] MacKeyLength),
    /// Packet length outside `MIN_PACKET_LEN..=MAX_PACKET_LEN` (checked
    /// before any parse).
    #[error("packet length {0} out of bounds")]
    PacketSize(usize),
    /// Inner plaintext longer than [`MAX_INNER_LEN`] (encode side).
    #[error("inner {0} B exceeds the 65,535 B packet cap")]
    TooLarge(usize),
    /// `version ≠ 0x01` (§3: unknown ⇒ drop, fail closed).
    #[error("unknown envelope version 0x{0:02x}")]
    UnknownVersion(u8),
    /// `crypto_mode ≠ 0x01` (rejected before decrypt).
    #[error("unsupported crypto_mode 0x{0:02x}")]
    UnsupportedCryptoMode(u8),
    /// `extra_len ≠ 5`.
    #[error("extra_len {0} (v1 = 5)")]
    ExtraLength(usize),
    /// GROUP, RATCHET or a reserved `prio_flags` bit set.
    #[error("prio_flags 0x{0:02x} sets GROUP/RATCHET/reserved bits")]
    ReservedPrioBits(u8),
    /// `dest_hint` is not this node's NodeId[0..4].
    #[error("dest_hint does not name this node")]
    DestHintMismatch,
    /// Inner `flags` bits 6–7 set.
    #[error("inner flags 0x{0:02x} set reserved bits")]
    ReservedFlags(u8),
    /// Kind byte not in the §3.2 registry.
    #[error("unknown kind 0x{0:02x}")]
    UnknownKind(u8),
    /// A registered kind that v1 does not build (OBJ_* = v1.1, BROADCAST =
    /// reserved).
    #[error("kind {0:?} is not built in v1")]
    KindNotBuilt(Kind),
    /// A control kind with `msg_seq ≠ 0 ‖ 0`.
    #[error("control kind {0:?} must carry msg_seq 0‖0")]
    ControlSeq(Kind),
    /// A `msg_seq` kind with `epoch = 0` or `n = 0`.
    #[error("kind {0:?} needs epoch ≥ 1 and n ≥ 1")]
    UnsequencedSeq(Kind),
    /// `HAS_FRAG` on a kind without `msg_seq` (nothing to reassemble against).
    #[error("fragment fields on control kind {0:?}")]
    FragOnControl(Kind),
    /// `frag_count > 16`, `frag_count = 0` or `frag_idx ≥ frag_count`.
    #[error("fragment bounds idx {idx} / count {count}")]
    FragBounds {
        /// Decoded `frag_idx` (saturated to u64).
        idx: u64,
        /// Decoded `frag_count` (saturated to u64).
        count: u64,
    },
    /// `HAS_EXT` with an empty TLV list (non-canonical).
    #[error("HAS_EXT set with no TLVs")]
    ExtEmpty,
    /// The same TLV type twice.
    #[error("duplicate ext TLV 0x{0:02x}")]
    DuplicateTlv(u8),
    /// TLV 0x20 — delegation trailer, rejected in v1 (L2 §6.7).
    #[error("delegation trailer (TLV 0x20) rejected in v1")]
    DelegationTrailer,
    /// TLV 0x10 / 0x21 — reserved semantics v1 does not implement.
    #[error("reserved ext TLV 0x{0:02x}")]
    ReservedTlv(u8),
    /// A malformed `seen_peer_seq` value.
    #[error("malformed seen_peer_seq")]
    BadSeenPeerSeq,
    /// A TLV that cannot be encoded (value > 255 B, > 255 TLVs, or an
    /// "unknown" TLV using a registered type).
    #[error("unencodable ext TLV")]
    BadTlv,
    /// The piggybacked or standalone ack block is malformed.
    #[error("ack block: {0}")]
    Ack(#[from] AckError),
    /// `PIGGYBACK_ACK` on an `ACK` packet (its block is its payload).
    #[error("PIGGYBACK_ACK on an ACK packet")]
    PiggybackOnAck,
    /// A kind whose class mandates `sig` (§3) arrived without SIGNED.
    #[error("kind {0:?} must be SIGNED")]
    SignatureRequired(Kind),
    /// `RECEIPT_STRONG` on a kind without `msg_seq`.
    #[error("RECEIPT_STRONG on control kind {0:?}")]
    StrongOnControl(Kind),
    /// `INTRO` on a kind other than `MSG` / `ENROLL`.
    #[error("INTRO not allowed on {0:?}")]
    IntroNotAllowed(Kind),
    /// `INTRO` together with `HAS_FRAG` (card-in-fragment-0 is §21, not v1).
    #[error("INTRO on a fragment (deferred, §21)")]
    IntroFragmented,
    /// `ENROLL` without `INTRO` (§7.3: the card rides the INTRO slot).
    #[error("ENROLL without INTRO")]
    EnrollWithoutIntro,
    /// Card length outside 137–169 B.
    #[error("card length {0} (137–169)")]
    CardLength(usize),
    /// The card does not parse or verify as an L2 announce.
    #[error("INTRO card is not a valid announce")]
    BadCard,
    /// The card's NodeId does not start with `sender_ref`.
    #[error("INTRO card does not match sender_ref")]
    IntroMismatch,
    /// The card is this node's own announce.
    #[error("INTRO card carries this node's NodeId")]
    IntroFromSelf,
    /// The card's NodeId is pinned with different key material (§7.1).
    #[error("INTRO card conflicts with pinned keys")]
    IntroKeyConflict,
    /// The inner's `sender_ref` is not this sealer's NodeId[0..4].
    #[error("sender_ref does not match the sealing identity")]
    SenderRefMismatch,
    /// No candidate's pair key verifies `pair_mac`.
    #[error("pair_mac verification failed")]
    PairMacFailed,
    /// `sig` failed the strict Ed25519 predicate.
    #[error("sig: {0}")]
    BadSignature(VerifyError),
    /// The identity handle failed (seal/open/pair key/counter).
    #[error("sealer: {0}")]
    Sealer(#[from] SealerError),
}

// ---------------------------------------------------------------------------
// Hash helpers (BLAKE2b family only, §10.2 "one hash family")
// ---------------------------------------------------------------------------

/// Unkeyed BLAKE2b with a 32 B digest over the concatenation of `parts`.
pub fn blake2b_256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Blake2b::<U32>::new();
    for p in parts {
        Digest::update(&mut h, p);
    }
    h.finalize().into()
}

/// Unkeyed BLAKE2b with a 16 B digest (the BLAKE2b-128 parameter set, not
/// a truncation) over the concatenation of `parts`.
pub fn blake2b_128(parts: &[&[u8]]) -> [u8; 16] {
    let mut h = Blake2b::<U16>::new();
    for p in parts {
        Digest::update(&mut h, p);
    }
    h.finalize().into()
}

/// The keyed BLAKE2b primitive refused its key: RFC 7693 keyed mode takes
/// 1–64 B. Every caller passes a 32 B key, so this is unreachable in
/// practice; it is a typed error rather than a zero tag so that no path can
/// ever "verify" against an unkeyed digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("keyed BLAKE2b: key of {0} B (1-64 allowed)")]
pub struct MacKeyLength(pub usize);

fn keyed_mac(key: &[u8], parts: &[&[u8]]) -> Result<Blake2bMac<U16>, MacKeyLength> {
    if key.is_empty() || key.len() > 64 {
        return Err(MacKeyLength(key.len()));
    }
    let mut m =
        <Blake2bMac<U16> as KeyInit>::new_from_slice(key).map_err(|_| MacKeyLength(key.len()))?;
    for p in parts {
        Mac::update(&mut m, p);
    }
    Ok(m)
}

/// Keyed BLAKE2b-128 in MAC mode (RFC 7693 key block) over `parts`.
pub fn keyed_blake2b_128(key: &[u8; 32], parts: &[&[u8]]) -> Result<[u8; 16], MacKeyLength> {
    Ok(keyed_mac(key, parts)?.finalize().into_bytes().into())
}

/// Constant-time check of a keyed BLAKE2b-128 tag. A refused key verifies
/// nothing (`false`).
pub fn keyed_blake2b_128_verify(key: &[u8; 32], parts: &[&[u8]], tag: &[u8; 16]) -> bool {
    keyed_mac(key, parts).is_ok_and(|m| m.verify_slice(tag).is_ok())
}

// ---------------------------------------------------------------------------
// Outer: prio_flags, dest_hint, L2 header bytes
// ---------------------------------------------------------------------------

/// §3 relay priority (`prio_flags` bits 7–6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Priority {
    /// 0 — control.
    Control = 0,
    /// 1 — interactive.
    Interactive = 1,
    /// 2 — bulk-small (≤ 64 KiB, §11).
    BulkSmall = 2,
    /// 3 — bulk-large.
    BulkLarge = 3,
}

impl Priority {
    fn from_bits(b: u8) -> Priority {
        match b & 0x03 {
            0 => Priority::Control,
            1 => Priority::Interactive,
            2 => Priority::BulkSmall,
            _ => Priority::BulkLarge,
        }
    }
}

/// §3 outer `extra` = `prio_flags(1) ‖ dest_hint(4)`. GROUP and RATCHET are
/// always 0 in v1, so they are not representable here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OuterExtra {
    /// Relay priority.
    pub priority: Priority,
    /// Recipient NodeId[0..4] — a locator, never an authenticator (L2 B2).
    pub dest_hint: [u8; 4],
}

impl OuterExtra {
    /// `extra` for a packet to `recipient`.
    pub fn to(priority: Priority, recipient: &NodeId) -> Self {
        OuterExtra {
            priority,
            dest_hint: node_prefix(recipient),
        }
    }

    /// The 5 wire bytes.
    pub fn encode(&self) -> [u8; EXTRA_LEN] {
        let d = self.dest_hint;
        [(self.priority as u8) << 6, d[0], d[1], d[2], d[3]]
    }

    /// Parse `extra`: exactly 5 B; GROUP, RATCHET and bits 3–0 MUST be 0.
    pub fn decode(extra: &[u8]) -> Result<Self, EnvelopeError> {
        if extra.len() != EXTRA_LEN {
            return Err(EnvelopeError::ExtraLength(extra.len()));
        }
        let pf = extra[0];
        if pf & (PRIO_GROUP | PRIO_RATCHET | PRIO_RESERVED) != 0 {
            return Err(EnvelopeError::ReservedPrioBits(pf));
        }
        let mut dest_hint = [0u8; 4];
        dest_hint.copy_from_slice(&extra[1..]);
        Ok(OuterExtra {
            priority: Priority::from_bits(pf >> 6),
            dest_hint,
        })
    }

    /// The L2 header for this `extra` and the per-peer send counter.
    pub fn header(&self, counter: u64) -> SealHeader {
        SealHeader {
            version: ENVELOPE_VERSION,
            crypto_mode: CRYPTO_MODE_V1,
            counter,
            extra: self.encode().to_vec(),
        }
    }
}

/// NodeId[0..4] — `sender_ref` / `dest_hint`.
pub fn node_prefix(id: &NodeId) -> [u8; 4] {
    let b = id.as_bytes();
    [b[0], b[1], b[2], b[3]]
}

/// The L2 header's exact wire bytes (`version ‖ crypto_mode ‖ counter LE ‖
/// extra_len ‖ extra`) — the first `pair_mac` input.
pub fn header_bytes(h: &SealHeader) -> Result<Vec<u8>, EnvelopeError> {
    let extra_len =
        u8::try_from(h.extra.len()).map_err(|_| EnvelopeError::ExtraLength(h.extra.len()))?;
    let mut out = Vec::with_capacity(HEADER_FIXED_LEN + h.extra.len());
    out.push(h.version);
    out.push(h.crypto_mode);
    out.extend_from_slice(&h.counter.to_le_bytes());
    out.push(extra_len);
    out.extend_from_slice(&h.extra);
    Ok(out)
}

/// §3 outer gates on a header: version, crypto-mode, `extra`.
pub fn check_header(h: &SealHeader) -> Result<OuterExtra, EnvelopeError> {
    if h.version != ENVELOPE_VERSION {
        return Err(EnvelopeError::UnknownVersion(h.version));
    }
    if h.crypto_mode != CRYPTO_MODE_V1 {
        return Err(EnvelopeError::UnsupportedCryptoMode(h.crypto_mode));
    }
    OuterExtra::decode(&h.extra)
}

/// What a relay, mailbox or flood bearer may read without opening (§4, §9):
/// the header, `extra`, `epk` (dedup key) and the opaque ciphertext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OuterView<'a> {
    /// The cleartext L2 header.
    pub header: SealHeader,
    /// Parsed `extra`.
    pub extra: OuterExtra,
    /// The packet's ephemeral public key.
    pub epk: Key32,
    /// Ciphertext ‖ tag, opaque.
    pub ciphertext: &'a [u8],
}

impl<'a> OuterView<'a> {
    /// Gate and split a sealed packet without decrypting, in order: size
    /// bounds, `version`, `crypto_mode`, `extra_len = 5`, `prio_flags` bits.
    pub fn parse(packet: &'a [u8]) -> Result<Self, EnvelopeError> {
        if !(MIN_PACKET_LEN..=MAX_PACKET_LEN).contains(&packet.len()) {
            return Err(EnvelopeError::PacketSize(packet.len()));
        }
        let mut r = Reader::new(packet);
        let version = r.u8()?;
        if version != ENVELOPE_VERSION {
            return Err(EnvelopeError::UnknownVersion(version));
        }
        let crypto_mode = r.u8()?;
        if crypto_mode != CRYPTO_MODE_V1 {
            return Err(EnvelopeError::UnsupportedCryptoMode(crypto_mode));
        }
        let counter = u64::from_le_bytes(r.array::<8>()?);
        let extra_len = usize::from(r.u8()?);
        if extra_len != EXTRA_LEN {
            return Err(EnvelopeError::ExtraLength(extra_len));
        }
        let extra = r.take(extra_len)?.to_vec();
        let epk = r.array::<EPK_LEN>()?;
        let ciphertext = r.rest();
        let header = SealHeader {
            version,
            crypto_mode,
            counter,
            extra,
        };
        let extra = check_header(&header)?;
        Ok(OuterView {
            header,
            extra,
            epk,
            ciphertext,
        })
    }

    /// §4 relay dedup key: `epk[0..8]` (flood suppression, not idempotency).
    pub fn dedup_key(&self) -> [u8; 8] {
        let mut k = [0u8; 8];
        k.copy_from_slice(&self.epk[..8]);
        k
    }
}

// ---------------------------------------------------------------------------
// Kinds (§3.2)
// ---------------------------------------------------------------------------

/// §3.2 kinds registry (1 B; unknown ⇒ drop and log).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// 0x01 peer ↔ peer message (per-stream `msg_seq`).
    Msg = 0x01,
    /// 0x02 delivery ACK (control).
    Ack = 0x02,
    /// 0x03 member → hub custody item (control `msg_seq`; `pkt_ref` dedup).
    CustodyPut = 0x03,
    /// 0x04 hub → member custody receipt (§6.2).
    CustodyAck = 0x04,
    /// 0x05 member → hub fetch.
    MailboxFetch = 0x05,
    /// 0x06 hub → member item.
    MailboxItem = 0x06,
    /// 0x07 member → hub commit (`max(cursor)`).
    MailboxCommit = 0x07,
    /// 0x08 member → hub enrollment (INTRO; §7.3).
    Enroll = 0x08,
    /// 0x09 hub → member enrollment answer.
    EnrollOk = 0x09,
    /// 0x10 agent operation (per-stream; SIGNED).
    AgentOp = 0x10,
    /// 0x11 knowledge (public class; SIGNED).
    Kb = 0x11,
    /// 0x12 update (public code class; SIGNED).
    Update = 0x12,
    /// 0x13 patch (public code class; SIGNED).
    Patch = 0x13,
    /// 0x14 sync (bulk, per stream).
    Sync = 0x14,
    /// 0x20 object advert (pinned; built v1.1).
    ObjAdvert = 0x20,
    /// 0x21 object request (pinned; built v1.1).
    ObjReq = 0x21,
    /// 0x22 object chunk (pinned; built v1.1).
    ObjChunk = 0x22,
    /// 0x30 broadcast (reserved; a Layer 7 format).
    Broadcast = 0x30,
}

impl Kind {
    /// Every registered kind, in byte order.
    pub const ALL: [Kind; 18] = [
        Kind::Msg,
        Kind::Ack,
        Kind::CustodyPut,
        Kind::CustodyAck,
        Kind::MailboxFetch,
        Kind::MailboxItem,
        Kind::MailboxCommit,
        Kind::Enroll,
        Kind::EnrollOk,
        Kind::AgentOp,
        Kind::Kb,
        Kind::Update,
        Kind::Patch,
        Kind::Sync,
        Kind::ObjAdvert,
        Kind::ObjReq,
        Kind::ObjChunk,
        Kind::Broadcast,
    ];

    /// Registry lookup; `None` for an unknown byte.
    pub fn from_byte(b: u8) -> Option<Kind> {
        Self::ALL.iter().copied().find(|k| *k as u8 == b)
    }

    /// The wire byte.
    pub fn byte(self) -> u8 {
        self as u8
    }

    /// §3.2: kinds with a per-stream `msg_seq` (the rest carry `0 ‖ 0` and
    /// are exempt from `rx_state` and ACKs).
    pub fn carries_msg_seq(self) -> bool {
        matches!(
            self,
            Kind::Msg | Kind::AgentOp | Kind::Kb | Kind::Update | Kind::Patch | Kind::Sync
        )
    }

    /// §3: kinds whose class mandates `sig` (AGENT_OP, KB, UPDATE, PATCH,
    /// BROADCAST; group posts are GROUP = 1, not v1).
    pub fn requires_sig(self) -> bool {
        matches!(
            self,
            Kind::AgentOp | Kind::Kb | Kind::Update | Kind::Patch | Kind::Broadcast
        )
    }

    /// §3 `sig` `dest` = 32 zero bytes for public classes: KB, UPDATE,
    /// PATCH (public signed objects, §10.3) and BROADCAST. Every other kind
    /// binds the recipient NodeId.
    pub fn is_public_class(self) -> bool {
        matches!(
            self,
            Kind::Kb | Kind::Update | Kind::Patch | Kind::Broadcast
        )
    }

    /// §3.2 priority column; `None` = "by inner item" / bulk by size.
    pub fn default_priority(self) -> Option<Priority> {
        match self {
            Kind::Msg | Kind::AgentOp | Kind::Broadcast => Some(Priority::Interactive),
            Kind::Ack
            | Kind::CustodyAck
            | Kind::MailboxFetch
            | Kind::MailboxCommit
            | Kind::Enroll
            | Kind::EnrollOk => Some(Priority::Control),
            _ => None,
        }
    }

    /// v1 builds everything except OBJ_* (v1.1) and BROADCAST (reserved).
    pub fn built_in_v1(self) -> bool {
        !matches!(
            self,
            Kind::ObjAdvert | Kind::ObjReq | Kind::ObjChunk | Kind::Broadcast
        )
    }

    /// INTRO is a first `MSG` to a never-contacted peer (§7.2 (3)) or an
    /// `ENROLL` (§7.3).
    pub fn intro_allowed(self) -> bool {
        matches!(self, Kind::Msg | Kind::Enroll)
    }
}

// ---------------------------------------------------------------------------
// Inner layout
// ---------------------------------------------------------------------------

/// §5 Tier-1 fragment position: `frag_idx < frag_count ≤ 16`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FragPos {
    /// 0-based index.
    pub idx: u8,
    /// Total fragments (1–16).
    pub count: u8,
}

impl FragPos {
    fn check(idx: u64, count: u64) -> Result<FragPos, EnvelopeError> {
        if count == 0 || count > u64::from(MAX_FRAGMENTS) || idx >= count {
            return Err(EnvelopeError::FragBounds { idx, count });
        }
        Ok(FragPos {
            idx: idx as u8,
            count: count as u8,
        })
    }
}

/// An ext TLV this codec does not interpret (skipped on receive, §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTlv {
    /// Type byte (never 0x01, 0x10, 0x20 or 0x21).
    pub tlv_type: u8,
    /// Value (≤ 255 B).
    pub value: Vec<u8>,
}

/// Ext TLVs (`HAS_EXT`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ext {
    /// TLV 0x01: the highest `msg_seq` the sender had seen from the peer (§4).
    pub seen_peer_seq: Option<MsgSeq>,
    /// Unknown TLVs, kept in wire order (skipped semantically).
    pub unknown: Vec<RawTlv>,
}

impl Ext {
    /// True when no TLV is present (`HAS_EXT` clear).
    pub fn is_empty(&self) -> bool {
        self.seen_peer_seq.is_none() && self.unknown.is_empty()
    }

    fn count(&self) -> usize {
        usize::from(self.seen_peer_seq.is_some()) + self.unknown.len()
    }

    fn validate(&self) -> Result<(), EnvelopeError> {
        if let Some(s) = self.seen_peer_seq {
            if !s.is_sequenced() {
                return Err(EnvelopeError::BadSeenPeerSeq);
            }
        }
        if self.count() > usize::from(u8::MAX) {
            return Err(EnvelopeError::BadTlv);
        }
        let mut seen = [false; 256];
        for t in &self.unknown {
            if matches!(
                t.tlv_type,
                TLV_SEEN_PEER_SEQ | TLV_RATCHET_PUBKEY | TLV_DELEGATION | TLV_SIGNER_PUBKEY
            ) || t.value.len() > usize::from(u8::MAX)
            {
                return Err(EnvelopeError::BadTlv);
            }
            if std::mem::replace(&mut seen[usize::from(t.tlv_type)], true) {
                return Err(EnvelopeError::DuplicateTlv(t.tlv_type));
            }
        }
        Ok(())
    }

    fn encode(&self, out: &mut Vec<u8>) {
        out.push(self.count() as u8); // ≤ 255, validated
        if let Some(s) = self.seen_peer_seq {
            out.push(TLV_SEEN_PEER_SEQ);
            out.push(s.encoded_len() as u8); // ≤ 15
            s.encode(out);
        }
        for t in &self.unknown {
            out.push(t.tlv_type);
            out.push(t.value.len() as u8); // ≤ 255, validated
            out.extend_from_slice(&t.value);
        }
    }

    fn decode(r: &mut Reader<'_>) -> Result<Ext, EnvelopeError> {
        let n = r.u8()?;
        if n == 0 {
            return Err(EnvelopeError::ExtEmpty);
        }
        let mut ext = Ext::default();
        let mut seen = [false; 256];
        for _ in 0..n {
            let t = r.u8()?;
            let len = usize::from(r.u8()?);
            let value = r.take(len)?;
            if std::mem::replace(&mut seen[usize::from(t)], true) {
                return Err(EnvelopeError::DuplicateTlv(t));
            }
            match t {
                TLV_SEEN_PEER_SEQ => {
                    let mut v = Reader::new(value);
                    let s = MsgSeq::decode(&mut v).map_err(|_| EnvelopeError::BadSeenPeerSeq)?;
                    if !v.is_empty() || !s.is_sequenced() {
                        return Err(EnvelopeError::BadSeenPeerSeq);
                    }
                    ext.seen_peer_seq = Some(s);
                }
                TLV_DELEGATION => return Err(EnvelopeError::DelegationTrailer),
                TLV_RATCHET_PUBKEY | TLV_SIGNER_PUBKEY => {
                    return Err(EnvelopeError::ReservedTlv(t))
                }
                _ => ext.unknown.push(RawTlv {
                    tlv_type: t,
                    value: value.to_vec(),
                }),
            }
        }
        Ok(ext)
    }
}

/// The inner plaintext **before** `auth` (§3). Flags are derived from the
/// fields, so an encoded inner can never set a reserved bit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inner {
    /// Sender NodeId[0..4].
    pub sender_ref: [u8; 4],
    /// §3.2 kind.
    pub kind: Kind,
    /// `msg_seq` ([`MsgSeq::CONTROL`] for control kinds).
    pub seq: MsgSeq,
    /// `HAS_FRAG`.
    pub frag: Option<FragPos>,
    /// `HAS_EXT` when non-empty.
    pub ext: Ext,
    /// `PIGGYBACK_ACK`.
    pub ack: Option<AckBlock>,
    /// `INTRO`: the sender's own announce bytes.
    pub card: Option<Vec<u8>>,
    /// The payload (opaque here; `MSG` bodies are §3.3 CBOR).
    pub payload: Vec<u8>,
    /// `SIGNED`: a 64 B `sig` follows `pair_mac`.
    pub signed: bool,
    /// `RECEIPT_STRONG`.
    pub receipt_strong: bool,
}

impl Inner {
    /// A Tier-0 inner from `sender` with no options set.
    pub fn new(sender: &NodeId, kind: Kind, seq: MsgSeq, payload: Vec<u8>) -> Self {
        Inner {
            sender_ref: node_prefix(sender),
            kind,
            seq,
            frag: None,
            ext: Ext::default(),
            ack: None,
            card: None,
            payload,
            signed: kind.requires_sig(),
            receipt_strong: false,
        }
    }

    /// The flags byte implied by the fields.
    pub fn flags(&self) -> u8 {
        let mut f = 0;
        if self.frag.is_some() {
            f |= flags::HAS_FRAG;
        }
        if !self.ext.is_empty() {
            f |= flags::HAS_EXT;
        }
        if self.ack.is_some() {
            f |= flags::PIGGYBACK_ACK;
        }
        if self.signed {
            f |= flags::SIGNED;
        }
        if self.receipt_strong {
            f |= flags::RECEIPT_STRONG;
        }
        if self.card.is_some() {
            f |= flags::INTRO;
        }
        f
    }

    /// `auth` length: 16, or 80 when SIGNED.
    pub fn auth_len(&self) -> usize {
        auth_len(self.signed)
    }

    /// Every §3 / §3.2 / §18 rule-2 semantic check, shared by encode and
    /// decode.
    pub fn validate(&self) -> Result<(), EnvelopeError> {
        let k = self.kind;
        if !k.built_in_v1() {
            return Err(EnvelopeError::KindNotBuilt(k));
        }
        if k.carries_msg_seq() {
            if !self.seq.is_sequenced() {
                return Err(EnvelopeError::UnsequencedSeq(k));
            }
        } else {
            if !self.seq.is_control() {
                return Err(EnvelopeError::ControlSeq(k));
            }
            if self.frag.is_some() {
                return Err(EnvelopeError::FragOnControl(k));
            }
            if self.receipt_strong {
                return Err(EnvelopeError::StrongOnControl(k));
            }
        }
        if let Some(f) = self.frag {
            FragPos::check(u64::from(f.idx), u64::from(f.count))?;
        }
        self.ext.validate()?;
        if let Some(a) = &self.ack {
            if k == Kind::Ack {
                return Err(EnvelopeError::PiggybackOnAck);
            }
            a.validate()?;
        }
        if k.requires_sig() && !self.signed {
            return Err(EnvelopeError::SignatureRequired(k));
        }
        match &self.card {
            Some(card) => {
                if !k.intro_allowed() {
                    return Err(EnvelopeError::IntroNotAllowed(k));
                }
                if self.frag.is_some() {
                    return Err(EnvelopeError::IntroFragmented);
                }
                if !(CARD_MIN_LEN..=CARD_MAX_LEN).contains(&card.len()) {
                    return Err(EnvelopeError::CardLength(card.len()));
                }
            }
            None if k == Kind::Enroll => return Err(EnvelopeError::EnrollWithoutIntro),
            None => {}
        }
        Ok(())
    }

    /// Encode the inner bytes before `auth` (the `pair_mac` input tail).
    pub fn encode_before_auth(&self) -> Result<Vec<u8>, EnvelopeError> {
        self.validate()?;
        let mut out = Vec::with_capacity(16 + self.payload.len());
        out.extend_from_slice(&self.sender_ref);
        out.push(self.kind.byte());
        out.push(self.flags());
        self.seq.encode(&mut out);
        if let Some(f) = self.frag {
            put_varint(&mut out, u64::from(f.idx));
            put_varint(&mut out, u64::from(f.count));
        }
        if !self.ext.is_empty() {
            self.ext.encode(&mut out);
        }
        if let Some(a) = &self.ack {
            out.extend_from_slice(&a.encode()?);
        }
        if let Some(card) = &self.card {
            out.push(card.len() as u8); // 137–169, validated
            out.extend_from_slice(card);
        }
        out.extend_from_slice(&self.payload);
        let total = out.len() + self.auth_len();
        if total > MAX_INNER_LEN {
            return Err(EnvelopeError::TooLarge(total));
        }
        Ok(out)
    }

    /// Decode the inner bytes before `auth`. `bytes` is exactly the region
    /// [`split_auth`] returned; every rule of [`Inner::validate`] applies.
    pub fn decode_before_auth(bytes: &[u8]) -> Result<Inner, EnvelopeError> {
        let mut r = Reader::new(bytes);
        let sender_ref = r.array::<SENDER_REF_LEN>()?;
        let kind_byte = r.u8()?;
        let f = r.u8()?;
        if f & flags::RESERVED != 0 {
            return Err(EnvelopeError::ReservedFlags(f));
        }
        let kind = Kind::from_byte(kind_byte).ok_or(EnvelopeError::UnknownKind(kind_byte))?;
        if !kind.built_in_v1() {
            return Err(EnvelopeError::KindNotBuilt(kind));
        }
        let seq = MsgSeq::decode(&mut r)?;
        let frag = if f & flags::HAS_FRAG != 0 {
            let idx = r.varint()?;
            let count = r.varint()?;
            Some(FragPos::check(idx, count)?)
        } else {
            None
        };
        let ext = if f & flags::HAS_EXT != 0 {
            Ext::decode(&mut r)?
        } else {
            Ext::default()
        };
        let ack = if f & flags::PIGGYBACK_ACK != 0 {
            if kind == Kind::Ack {
                return Err(EnvelopeError::PiggybackOnAck);
            }
            Some(AckBlock::decode(&mut r)?.0)
        } else {
            None
        };
        let card = if f & flags::INTRO != 0 {
            let len = usize::from(r.u8()?);
            if !(CARD_MIN_LEN..=CARD_MAX_LEN).contains(&len) {
                return Err(EnvelopeError::CardLength(len));
            }
            Some(r.take(len)?.to_vec())
        } else {
            None
        };
        let payload = r.rest().to_vec();
        let inner = Inner {
            sender_ref,
            kind,
            seq,
            frag,
            ext,
            ack,
            card,
            payload,
            signed: f & flags::SIGNED != 0,
            receipt_strong: f & flags::RECEIPT_STRONG != 0,
        };
        inner.validate()?;
        Ok(inner)
    }

    /// Assemble the full inner: before-auth ‖ `pair_mac` ‖ [`sig`] — the
    /// bytes the sealer AEADs once it has drawn `epk`.
    pub fn finish(before_auth: &[u8], pair_mac: &[u8; 16], sig: Option<&[u8; 64]>) -> Vec<u8> {
        let mut out = Vec::with_capacity(before_auth.len() + PAIR_MAC_LEN + SIG_LEN);
        out.extend_from_slice(before_auth);
        out.extend_from_slice(pair_mac);
        if let Some(s) = sig {
            out.extend_from_slice(s);
        }
        out
    }
}

fn auth_len(signed: bool) -> usize {
    PAIR_MAC_LEN + if signed { SIG_LEN } else { 0 }
}

/// The inner's `auth` tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Auth {
    /// `pair_mac`.
    pub pair_mac: [u8; 16],
    /// `sig` when SIGNED.
    pub sig: Option<[u8; 64]>,
}

/// Locate `auth` by reading **only** the flags byte at its fixed offset (5):
/// reserved bits ⇒ reject; SIGNED ⇒ 80 B tail, else 16 B. Returns the
/// before-auth region, the auth, and the raw flags. Sizes checked first.
pub fn split_auth(inner: &[u8]) -> Result<(&[u8], Auth, u8), EnvelopeError> {
    if inner.len() < MIN_INNER_LEN {
        return Err(EnvelopeError::Read(ReadError::Truncated));
    }
    let f = inner[SENDER_REF_LEN + 1];
    if f & flags::RESERVED != 0 {
        return Err(EnvelopeError::ReservedFlags(f));
    }
    let signed = f & flags::SIGNED != 0;
    let tail = auth_len(signed);
    if inner.len() < MIN_INNER_LEN - PAIR_MAC_LEN + tail {
        return Err(EnvelopeError::Read(ReadError::Truncated));
    }
    let (before, auth) = inner.split_at(inner.len() - tail);
    let mut pair_mac = [0u8; 16];
    pair_mac.copy_from_slice(&auth[..PAIR_MAC_LEN]);
    let sig = signed.then(|| {
        let mut s = [0u8; 64];
        s.copy_from_slice(&auth[PAIR_MAC_LEN..]);
        s
    });
    Ok((before, Auth { pair_mac, sig }, f))
}

// ---------------------------------------------------------------------------
// pair_mac (§3, L2 amendment A2)
// ---------------------------------------------------------------------------

/// §3 pair key: `BLAKE2b-256("111/pair/v1" ‖ ss ‖ sender_enc_pk ‖
/// recipient_enc_pk)`. For `Sealer` implementations (F2, behind the
/// identity handle, where `ss = X25519(sender_enc_sk, recipient_enc_pk)` is
/// computed) and for the frozen vectors; `mesh111` itself only ever holds
/// the derived key.
pub fn derive_pair_key(
    shared: &Key32,
    sender_enc_pk: &Key32,
    recipient_enc_pk: &Key32,
) -> Zeroizing<Key32> {
    Zeroizing::new(blake2b_256(&[
        PAIR_KEY_TAG,
        shared,
        sender_enc_pk,
        recipient_enc_pk,
    ]))
}

/// §3 `pair_mac = BLAKE2b-128(key, header bytes ‖ epk ‖ inner before auth)`.
pub fn pair_mac(
    pair_key: &Key32,
    header_bytes: &[u8],
    epk: &Key32,
    before_auth: &[u8],
) -> Result<[u8; 16], EnvelopeError> {
    Ok(keyed_blake2b_128(
        pair_key,
        &[header_bytes, epk, before_auth],
    )?)
}

/// Constant-time `pair_mac` check.
pub fn verify_pair_mac(
    pair_key: &Key32,
    header_bytes: &[u8],
    epk: &Key32,
    before_auth: &[u8],
    tag: &[u8; 16],
) -> bool {
    keyed_blake2b_128_verify(pair_key, &[header_bytes, epk, before_auth], tag)
}

// ---------------------------------------------------------------------------
// sig (§3)
// ---------------------------------------------------------------------------

/// The 32 B `dest` bound into `sig` for `kind` sent to `recipient` (§3):
/// zeros for public classes, the recipient NodeId otherwise (group posts
/// would bind `group_id`; GROUP is 0 in v1).
pub fn sig_dest(kind: Kind, recipient: &NodeId) -> [u8; 32] {
    if kind.is_public_class() {
        [0u8; 32]
    } else {
        recipient.to_bytes()
    }
}

/// §3 `sig` signed input **without** the domain tag (the signer and the
/// strict verifier prepend `"111/msg/v1"`): `sender(32) ‖ dest(32) ‖
/// stream(varint) ‖ msg_seq(epoch varint ‖ n varint) ‖ kind(1) ‖ flags(1) ‖
/// frag_idx(varint) ‖ frag_count(varint) ‖ BLAKE2b-256(payload)`. `flags`
/// is the transmitted inner flags byte, the frag fields this packet's
/// position (`0 ‖ 0` when unfragmented) and `payload` this packet's
/// payload, so a pair-key holder can neither move a signed fragment to
/// another slot nor recombine signed fragments into a different message
/// (spec §3 amendment).
pub fn sig_input(sender: &NodeId, dest: &[u8; 32], stream: u64, inner: &Inner) -> Vec<u8> {
    let (idx, count) = inner
        .frag
        .map_or((0, 0), |f| (u64::from(f.idx), u64::from(f.count)));
    let mut out = Vec::with_capacity(
        32 + 32
            + varint_len(stream)
            + inner.seq.encoded_len()
            + 2
            + varint_len(idx)
            + varint_len(count)
            + 32,
    );
    out.extend_from_slice(sender.as_bytes());
    out.extend_from_slice(dest);
    put_varint(&mut out, stream);
    inner.seq.encode(&mut out);
    out.push(inner.kind.byte());
    out.push(inner.flags());
    put_varint(&mut out, idx);
    put_varint(&mut out, count);
    out.extend_from_slice(&blake2b_256(&[&inner.payload]));
    out
}

/// Verify `sig` under id111's strict Ed25519 predicate (never dalek
/// `verify()`).
pub fn verify_sig(
    sender: &NodeId,
    recipient: &NodeId,
    stream: u64,
    inner: &Inner,
    sig: &[u8; 64],
) -> Result<(), EnvelopeError> {
    let input = sig_input(sender, &sig_dest(inner.kind, recipient), stream, inner);
    verify_strict_tagged(sender, MSG_SIG_TAG, &input, sig).map_err(EnvelopeError::BadSignature)
}

// ---------------------------------------------------------------------------
// Seal / open through the identity handle
// ---------------------------------------------------------------------------

/// A peer's public keys as pinned in contacts (§7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerKeys {
    /// NodeId (Ed25519 public key).
    pub node_id: NodeId,
    /// Announced X25519 key.
    pub enc_pubkey: Key32,
}

/// §3: seal `inner` to `to` through the identity handle. Validates, signs
/// when SIGNED (`dest` per [`sig_dest`]), takes the next L2 send counter,
/// and computes `pair_mac` inside the sealer's epk callback. The caller has
/// already committed `n` (§4: before the first fragment is sealed).
pub fn seal_envelope(
    sealer: &dyn Sealer,
    to: &PeerKeys,
    priority: Priority,
    inner: &Inner,
) -> Result<Vec<u8>, EnvelopeError> {
    let own = sealer.node_id();
    if inner.sender_ref != node_prefix(&own) {
        return Err(EnvelopeError::SenderRefMismatch);
    }
    let before_auth = inner.encode_before_auth()?;
    let sig = inner.signed.then(|| {
        sealer.sign(
            MSG_SIG_TAG,
            &sig_input(&own, &sig_dest(inner.kind, &to.node_id), STREAM_1TO1, inner),
        )
    });
    let key = Zeroizing::new(sealer.pair_key(&to.enc_pubkey, Direction::Outbound)?);
    let counter = sealer.next_send_counter(&to.node_id)?;
    let header = OuterExtra::to(priority, &to.node_id).header(counter);
    let hb = header_bytes(&header)?;
    // The epk callback cannot return an error; a refused MAC key (unreachable
    // for a 32 B pair key) is captured here and the sealed bytes dropped.
    let mut mac_err: Option<EnvelopeError> = None;
    let packet = sealer.seal_payload(
        &to.enc_pubkey,
        &header,
        &mut |epk: &Key32| match pair_mac(&key, &hb, epk, &before_auth) {
            Ok(mac) => Inner::finish(&before_auth, &mac, sig.as_ref()),
            Err(e) => {
                mac_err = Some(e);
                Vec::new()
            }
        },
    )?;
    if let Some(e) = mac_err {
        return Err(e);
    }
    Ok(packet)
}

/// A verified envelope. Nothing has touched replay state yet: the caller
/// offers [`Opened::counter`] to the L2 window (`observe_counter`, A11),
/// then applies `msg_seq` dedup, then stores (§18 rule 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    /// The AD-verified L2 header.
    pub header: SealHeader,
    /// Parsed `extra`.
    pub outer: OuterExtra,
    /// The packet's ephemeral key.
    pub epk: Key32,
    /// The sender, authenticated by `pair_mac`.
    pub sender: PeerKeys,
    /// True when the sender was authenticated with the INTRO card's key (not
    /// yet a contact: the caller pins provisionally / files a request).
    pub via_intro: bool,
    /// The decoded inner.
    pub inner: Inner,
    /// The verified `sig`, when SIGNED.
    pub sig: Option<[u8; 64]>,
    /// The stream ([`STREAM_1TO1`] in v1).
    pub stream: u64,
}

impl Opened {
    /// The L2 per-peer counter, for `Sealer::observe_counter`.
    pub fn counter(&self) -> u64 {
        self.header.counter
    }

    /// §4 `msg_id` of this packet as received by `recipient`.
    pub fn msg_id(&self, recipient: NodeId) -> MsgId {
        MsgId::new(self.sender.node_id, recipient, self.stream, self.inner.seq)
    }
}

/// §3 / §18 rule 1: open and verify one packet addressed to this node.
///
/// `lookup` returns the pinned contacts whose NodeId starts with a
/// `sender_ref` (collisions ⇒ each candidate's pair key is tried). Order:
/// outer gates → open → `pair_mac` (INTRO: card verified first) → full
/// inner parse → `sig`. Any failure returns an error and leaves no state.
pub fn open_envelope(
    sealer: &dyn Sealer,
    packet: &[u8],
    lookup: &dyn Fn(&[u8; 4]) -> Vec<PeerKeys>,
) -> Result<Opened, EnvelopeError> {
    let view = OuterView::parse(packet)?;
    let own = sealer.node_id();
    if view.extra.dest_hint != node_prefix(&own) {
        return Err(EnvelopeError::DestHintMismatch);
    }
    let (header, epk, inner_bytes) = sealer.open_payload(packet)?;
    let outer = check_header(&header)?;
    if inner_bytes.len() > MAX_INNER_LEN {
        return Err(EnvelopeError::PacketSize(packet.len()));
    }
    let hb = header_bytes(&header)?;
    let (before, auth, raw_flags) = split_auth(&inner_bytes)?;
    let mut sender_ref = [0u8; 4];
    sender_ref.copy_from_slice(&before[..SENDER_REF_LEN]);

    let mut candidates: Vec<(PeerKeys, bool)> = lookup(&sender_ref)
        .into_iter()
        .filter(|c| node_prefix(&c.node_id) == sender_ref)
        .map(|c| (c, false))
        .collect();
    let mut parsed: Option<Inner> = None;
    if raw_flags & flags::INTRO != 0 {
        // §7.2 (3): parse (pure) to reach the card, verify it as an
        // announce, offer it as a provisional candidate — then pair_mac.
        let inner = Inner::decode_before_auth(before)?;
        let card = inner.card.as_deref().ok_or(EnvelopeError::BadCard)?;
        let ann = Announce::parse(card).map_err(|_| EnvelopeError::BadCard)?;
        ann.verify().map_err(|_| EnvelopeError::BadCard)?;
        let card_id = ann.node_id();
        if node_prefix(&card_id) != sender_ref {
            return Err(EnvelopeError::IntroMismatch);
        }
        if card_id == own {
            return Err(EnvelopeError::IntroFromSelf);
        }
        match candidates.iter().find(|(c, _)| c.node_id == card_id) {
            Some((pinned, _)) if pinned.enc_pubkey != *ann.enc_pubkey() => {
                return Err(EnvelopeError::IntroKeyConflict);
            }
            Some(_) => {}
            None => candidates.push((
                PeerKeys {
                    node_id: card_id,
                    enc_pubkey: *ann.enc_pubkey(),
                },
                true,
            )),
        }
        parsed = Some(inner);
    }

    // Every candidate's inbound pair key is derived through the handle, which
    // caches it. A provisional INTRO candidate that does not authenticate
    // must not stay in that cache (it is not a contact): forget it. The one
    // that wins stays — the caller pins it next. Pinned contacts' keys are
    // theirs to keep either way.
    let mut sender: Option<(PeerKeys, bool)> = None;
    for (c, via_intro) in candidates {
        if sender.is_none() {
            let verified = match sealer.pair_key(&c.enc_pubkey, Direction::Inbound) {
                Ok(k) => {
                    let key = Zeroizing::new(k);
                    verify_pair_mac(&key, &hb, &epk, before, &auth.pair_mac)
                }
                Err(e) => {
                    if via_intro {
                        sealer.forget_pair_key(&c.enc_pubkey);
                    }
                    return Err(e.into());
                }
            };
            if verified {
                sender = Some((c, via_intro));
                continue;
            }
        }
        if via_intro {
            sealer.forget_pair_key(&c.enc_pubkey);
        }
    }
    let (sender, via_intro) = sender.ok_or(EnvelopeError::PairMacFailed)?;

    let inner = match parsed {
        Some(i) => i,
        None => Inner::decode_before_auth(before)?,
    };
    if let Some(sig) = &auth.sig {
        verify_sig(&sender.node_id, &own, STREAM_1TO1, &inner, sig)?;
    }
    Ok(Opened {
        header,
        outer,
        epk,
        sender,
        via_intro,
        inner,
        sig: auth.sig,
        stream: STREAM_1TO1,
    })
}

// ---------------------------------------------------------------------------
// §3 size table
// ---------------------------------------------------------------------------

/// Payload bytes that fit one bearer frame of `bearer_payload` bytes (§3
/// size table): bearer − shim − 64 outer − inner header (sender_ref, kind,
/// flags, `msg_seq`, frag fields) − `pair_mac` − [`sig`]. Options that add
/// bytes (TLVs, piggyback, card) are the caller's to subtract.
pub fn payload_budget(
    bearer_payload: usize,
    shim: bool,
    seq: MsgSeq,
    frag: Option<FragPos>,
    signed: bool,
) -> Option<usize> {
    let frag_len = frag.map_or(0, |f| {
        varint_len(u64::from(f.idx)) + varint_len(u64::from(f.count))
    });
    let fixed = if shim { SHIM_LEN } else { 0 }
        + OUTER_OVERHEAD
        + SENDER_REF_LEN
        + 2
        + seq.encoded_len()
        + frag_len
        + auth_len(signed);
    bearer_payload.checked_sub(fixed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ack::{AckRange, PartialAck};
    use crate::sealer::Freshness;
    use crate::test_double::{TestSealer, MNEMONIC_A, MNEMONIC_B};

    fn pair() -> (TestSealer, TestSealer) {
        (
            TestSealer::from_words(MNEMONIC_A),
            TestSealer::from_words(MNEMONIC_B),
        )
    }

    /// `open_envelope` leaves the L2 replay window to the caller: the double
    /// saw no `observe_counter` call, the counter comes back in `Opened`,
    /// and offering it is what makes the window move.
    fn counter_left_to_caller(b: &TestSealer, got: &Opened, counter: u64) {
        assert!(b.observed().is_empty(), "open_envelope touched the window");
        assert_eq!(got.counter(), counter);
        assert_eq!(
            b.observe_counter(&got.sender.node_id, got.counter())
                .unwrap(),
            Freshness::Fresh
        );
        assert_eq!(b.observed(), vec![(got.sender.node_id, counter)]);
    }

    /// A rejected packet leaves no trace: no window call, no forgotten key
    /// beyond what the test expects.
    fn nothing_observed(b: &TestSealer) {
        assert!(
            b.observed().is_empty(),
            "a rejected packet touched the window"
        );
    }

    fn lookup_of(contacts: Vec<PeerKeys>) -> impl Fn(&[u8; 4]) -> Vec<PeerKeys> {
        move |r: &[u8; 4]| {
            contacts
                .iter()
                .copied()
                .filter(|c| node_prefix(&c.node_id) == *r)
                .collect()
        }
    }

    #[test]
    fn outer_extra_bits() {
        let id = NodeId::from([0xab; 32]);
        let e = OuterExtra::to(Priority::BulkSmall, &id);
        assert_eq!(e.encode(), [0x80, 0xab, 0xab, 0xab, 0xab]);
        assert_eq!(OuterExtra::decode(&e.encode()).unwrap(), e);
        for bad in [PRIO_GROUP, PRIO_RATCHET, 0x01, 0x08] {
            let mut x = e.encode();
            x[0] |= bad;
            assert!(matches!(
                OuterExtra::decode(&x),
                Err(EnvelopeError::ReservedPrioBits(_))
            ));
        }
        assert!(matches!(
            OuterExtra::decode(&[0u8; 4]),
            Err(EnvelopeError::ExtraLength(4))
        ));
        let mut h = e.header(7);
        assert_eq!(header_bytes(&h).unwrap().len(), HEADER_LEN);
        h.version = 2;
        assert!(matches!(
            check_header(&h),
            Err(EnvelopeError::UnknownVersion(2))
        ));
        h.version = 1;
        h.crypto_mode = 0;
        assert!(matches!(
            check_header(&h),
            Err(EnvelopeError::UnsupportedCryptoMode(0))
        ));
    }

    #[test]
    fn seal_open_round_trip_tier0() {
        let (a, b) = pair();
        let inner = Inner::new(&a.node_id(), Kind::Msg, MsgSeq::new(1, 1), b"hi".to_vec());
        let pkt = seal_envelope(&a, &b.keys(), Priority::Interactive, &inner).unwrap();
        let view = OuterView::parse(&pkt).unwrap();
        assert_eq!(view.extra.dest_hint, node_prefix(&b.node_id()));
        let lookup = lookup_of(vec![a.keys()]);
        let got = open_envelope(&b, &pkt, &lookup).unwrap();
        assert_eq!(got.inner, inner);
        assert_eq!(got.sender, a.keys());
        assert!(!got.via_intro);
        assert_eq!(got.msg_id(b.node_id()).seq(), MsgSeq::new(1, 1));
        counter_left_to_caller(&b, &got, 1);
        // a second packet carries the next counter; still the caller's call
        let pkt2 = seal_envelope(&a, &b.keys(), Priority::Interactive, &inner).unwrap();
        let got2 = open_envelope(&b, &pkt2, &lookup).unwrap();
        assert_eq!(got2.counter(), 2);
        assert_eq!(b.observed(), vec![(a.node_id(), 1)]);
    }

    #[test]
    fn signed_kinds_and_dest_binding() {
        let (a, b) = pair();
        let mut inner = Inner::new(
            &a.node_id(),
            Kind::AgentOp,
            MsgSeq::new(4, 9),
            b"op".to_vec(),
        );
        assert!(inner.signed);
        inner.receipt_strong = true;
        let pkt = seal_envelope(&a, &b.keys(), Priority::Interactive, &inner).unwrap();
        let got = open_envelope(&b, &pkt, &lookup_of(vec![a.keys()])).unwrap();
        let sig = got.sig.unwrap();
        // the sig binds b as dest: it does not verify for a third party
        let c = TestSealer::from_seed(7);
        assert!(verify_sig(&a.node_id(), &b.node_id(), 0, &got.inner, &sig).is_ok());
        assert!(matches!(
            verify_sig(&a.node_id(), &c.node_id(), 0, &got.inner, &sig),
            Err(EnvelopeError::BadSignature(_))
        ));
        // an unsigned AGENT_OP is refused on both sides
        inner.signed = false;
        assert!(matches!(
            seal_envelope(&a, &b.keys(), Priority::Interactive, &inner),
            Err(EnvelopeError::SignatureRequired(Kind::AgentOp))
        ));
        // KB: public class, dest = zeros
        let kb = Inner::new(&a.node_id(), Kind::Kb, MsgSeq::new(4, 1), b"entry".to_vec());
        let pkt = seal_envelope(&a, &b.keys(), Priority::BulkSmall, &kb).unwrap();
        let got = open_envelope(&b, &pkt, &lookup_of(vec![a.keys()])).unwrap();
        let input = sig_input(&a.node_id(), &[0u8; 32], 0, &kb);
        assert!(verify_strict_tagged(&a.node_id(), MSG_SIG_TAG, &input, &got.sig.unwrap()).is_ok());
    }

    #[test]
    fn resealed_signature_never_authenticates() {
        // c holds a's signed plaintext and re-seals it to b under a's
        // sender_ref: pair_mac fails, nothing is returned, no window touched.
        let (a, b) = pair();
        let c = TestSealer::from_seed(9);
        let inner = Inner::new(
            &a.node_id(),
            Kind::AgentOp,
            MsgSeq::new(4, 1),
            b"op".to_vec(),
        );
        let honest = seal_envelope(&a, &b.keys(), Priority::Interactive, &inner).unwrap();
        let opened = open_envelope(&b, &honest, &lookup_of(vec![a.keys()])).unwrap();
        let sig = opened.sig.unwrap();
        let before = inner.encode_before_auth().unwrap();
        let header = OuterExtra::to(Priority::Interactive, &b.node_id()).header(99);
        let hb = header_bytes(&header).unwrap();
        let ckey = c.pair_key(&b.enc_pubkey(), Direction::Outbound).unwrap();
        let forged = c
            .seal_payload(&b.enc_pubkey(), &header, &mut |epk: &Key32| {
                Inner::finish(
                    &before,
                    &pair_mac(&ckey, &hb, epk, &before).unwrap(),
                    Some(&sig),
                )
            })
            .unwrap();
        let both = lookup_of(vec![a.keys(), c.keys()]);
        assert!(matches!(
            open_envelope(&b, &forged, &both),
            Err(EnvelopeError::PairMacFailed)
        ));
        nothing_observed(&b);
        // and pinned contacts' keys are never forgotten on a miss
        assert!(b.forgotten().is_empty());
    }

    #[test]
    fn forged_sender_ref_and_collisions() {
        let (a, b) = pair();
        let inner = Inner::new(&a.node_id(), Kind::Msg, MsgSeq::new(1, 2), b"x".to_vec());
        let pkt = seal_envelope(&a, &b.keys(), Priority::Interactive, &inner).unwrap();
        // no contact for the ref
        assert!(matches!(
            open_envelope(&b, &pkt, &|_: &[u8; 4]| Vec::new()),
            Err(EnvelopeError::PairMacFailed)
        ));
        // a colliding contact (same prefix, different keys) tried first
        let mut decoy_id = [0u8; 32];
        decoy_id[..4].copy_from_slice(&node_prefix(&a.node_id()));
        let decoy = PeerKeys {
            node_id: NodeId::from(decoy_id),
            enc_pubkey: TestSealer::from_seed(3).enc_pubkey(),
        };
        let got = open_envelope(&b, &pkt, &lookup_of(vec![decoy, a.keys()])).unwrap();
        assert_eq!(got.sender, a.keys());
        // a packet to someone else is refused on dest_hint before opening
        assert!(matches!(
            open_envelope(&a, &pkt, &lookup_of(vec![b.keys()])),
            Err(EnvelopeError::DestHintMismatch)
        ));
        nothing_observed(&b);
        nothing_observed(&a);
        counter_left_to_caller(&b, &got, 1);
    }

    #[test]
    fn pair_mac_binds_header_epk_and_inner() {
        let (a, b) = pair();
        let inner = Inner::new(
            &a.node_id(),
            Kind::Msg,
            MsgSeq::new(1, 3),
            b"bound".to_vec(),
        );
        let before = inner.encode_before_auth().unwrap();
        let key = a.pair_key(&b.enc_pubkey(), Direction::Outbound).unwrap();
        let header = OuterExtra::to(Priority::Interactive, &b.node_id()).header(5);
        let hb = header_bytes(&header).unwrap();
        let lookup = lookup_of(vec![a.keys()]);
        // a MAC computed over a different epk than the one sealed under
        let stale_epk = [9u8; 32];
        let pkt = a
            .seal_payload(&b.enc_pubkey(), &header, &mut |_epk: &Key32| {
                Inner::finish(
                    &before,
                    &pair_mac(&key, &hb, &stale_epk, &before).unwrap(),
                    None,
                )
            })
            .unwrap();
        assert!(matches!(
            open_envelope(&b, &pkt, &lookup),
            Err(EnvelopeError::PairMacFailed)
        ));
        // a MAC over another counter (header bytes differ)
        let other =
            header_bytes(&OuterExtra::to(Priority::Interactive, &b.node_id()).header(6)).unwrap();
        let pkt = a
            .seal_payload(&b.enc_pubkey(), &header, &mut |epk: &Key32| {
                Inner::finish(
                    &before,
                    &pair_mac(&key, &other, epk, &before).unwrap(),
                    None,
                )
            })
            .unwrap();
        assert!(matches!(
            open_envelope(&b, &pkt, &lookup),
            Err(EnvelopeError::PairMacFailed)
        ));
        // one flipped payload bit under a correct-looking MAC
        let pkt = a
            .seal_payload(&b.enc_pubkey(), &header, &mut |epk: &Key32| {
                let mac = pair_mac(&key, &hb, epk, &before).unwrap();
                let mut tampered = before.clone();
                if let Some(last) = tampered.last_mut() {
                    *last ^= 1;
                }
                Inner::finish(&tampered, &mac, None)
            })
            .unwrap();
        assert!(matches!(
            open_envelope(&b, &pkt, &lookup),
            Err(EnvelopeError::PairMacFailed)
        ));
        // a reflected packet (b's own outbound key) never verifies as a's
        let rkey = b.pair_key(&a.enc_pubkey(), Direction::Outbound).unwrap();
        let pkt = a
            .seal_payload(&b.enc_pubkey(), &header, &mut |epk: &Key32| {
                Inner::finish(&before, &pair_mac(&rkey, &hb, epk, &before).unwrap(), None)
            })
            .unwrap();
        assert!(matches!(
            open_envelope(&b, &pkt, &lookup),
            Err(EnvelopeError::PairMacFailed)
        ));
        nothing_observed(&b);
    }

    /// §18.1 order: `pair_mac` is checked before the inner is parsed, so a
    /// packet that is both unauthenticated and malformed reports
    /// `PairMacFailed`, never a parse error that would leak what the
    /// receiver understood. The same bytes under a valid `pair_mac` reach
    /// the parser and fail there.
    #[test]
    fn pair_mac_is_checked_before_inner_parse() {
        let (a, b) = pair();
        let header = OuterExtra::to(Priority::Interactive, &b.node_id()).header(3);
        let hb = header_bytes(&header).unwrap();
        let lookup = lookup_of(vec![a.keys()]);
        // unknown kind 0x0a, flags 0, msg_seq 1‖1: parses as far as the
        // auth split, fails in decode_before_auth
        let mut malformed = node_prefix(&a.node_id()).to_vec();
        malformed.extend_from_slice(&[0x0a, 0x00, 1, 1]);
        assert!(matches!(
            Inner::decode_before_auth(&malformed),
            Err(EnvelopeError::UnknownKind(0x0a))
        ));
        let pkt = a
            .seal_payload(&b.enc_pubkey(), &header, &mut |_epk: &Key32| {
                Inner::finish(&malformed, &[0x5a; 16], None)
            })
            .unwrap();
        assert!(matches!(
            open_envelope(&b, &pkt, &lookup),
            Err(EnvelopeError::PairMacFailed)
        ));
        let key = a.pair_key(&b.enc_pubkey(), Direction::Outbound).unwrap();
        let pkt = a
            .seal_payload(&b.enc_pubkey(), &header, &mut |epk: &Key32| {
                Inner::finish(
                    &malformed,
                    &pair_mac(&key, &hb, epk, &malformed).unwrap(),
                    None,
                )
            })
            .unwrap();
        assert!(matches!(
            open_envelope(&b, &pkt, &lookup),
            Err(EnvelopeError::UnknownKind(0x0a))
        ));
        nothing_observed(&b);
    }

    /// A valid `pair_mac` does not rescue reserved bits: the outer
    /// `prio_flags` reserved bit is refused before decrypt, the inner
    /// `flags` reserved bit before the MAC is even checked.
    #[test]
    fn reserved_bits_rejected_under_valid_pair_mac() {
        let (a, b) = pair();
        let lookup = lookup_of(vec![a.keys()]);
        let key = a.pair_key(&b.enc_pubkey(), Direction::Outbound).unwrap();
        let inner = Inner::new(&a.node_id(), Kind::Msg, MsgSeq::new(1, 1), b"x".to_vec());
        let before = inner.encode_before_auth().unwrap();
        // outer: reserved bit 3 set in prio_flags, MAC computed over it
        let mut header = OuterExtra::to(Priority::Interactive, &b.node_id()).header(4);
        header.extra[0] |= 0x08;
        let hb = header_bytes(&header).unwrap();
        let pkt = a
            .seal_payload(&b.enc_pubkey(), &header, &mut |epk: &Key32| {
                Inner::finish(&before, &pair_mac(&key, &hb, epk, &before).unwrap(), None)
            })
            .unwrap();
        assert!(matches!(
            open_envelope(&b, &pkt, &lookup),
            Err(EnvelopeError::ReservedPrioBits(0x48))
        ));
        // inner: flags bit 6 set, MAC computed over the modified bytes
        let header = OuterExtra::to(Priority::Interactive, &b.node_id()).header(5);
        let hb = header_bytes(&header).unwrap();
        let mut bad = before.clone();
        bad[SENDER_REF_LEN + 1] |= 0x40;
        let pkt = a
            .seal_payload(&b.enc_pubkey(), &header, &mut |epk: &Key32| {
                Inner::finish(&bad, &pair_mac(&key, &hb, epk, &bad).unwrap(), None)
            })
            .unwrap();
        assert!(matches!(
            open_envelope(&b, &pkt, &lookup),
            Err(EnvelopeError::ReservedFlags(0x40))
        ));
        nothing_observed(&b);
    }

    /// A provisional INTRO candidate that fails `pair_mac` is forgotten from
    /// the handle's pair-key cache; the one that authenticates stays for the
    /// caller to pin.
    #[test]
    fn losing_intro_candidate_is_forgotten() {
        let (a, b) = pair();
        let mut inner = Inner::new(&a.node_id(), Kind::Msg, MsgSeq::new(1, 1), b"hi".to_vec());
        inner.card = Some(a.own_announce());
        let before = inner.encode_before_auth().unwrap();
        // c re-seals a's INTRO plaintext under its own pair key: the card
        // names a, the MAC is c's — a's provisional key must not linger
        let c = TestSealer::from_seed(11);
        let header = OuterExtra::to(Priority::Interactive, &b.node_id()).header(1);
        let hb = header_bytes(&header).unwrap();
        let ckey = c.pair_key(&b.enc_pubkey(), Direction::Outbound).unwrap();
        let forged = c
            .seal_payload(&b.enc_pubkey(), &header, &mut |epk: &Key32| {
                Inner::finish(&before, &pair_mac(&ckey, &hb, epk, &before).unwrap(), None)
            })
            .unwrap();
        assert!(matches!(
            open_envelope(&b, &forged, &|_: &[u8; 4]| Vec::new()),
            Err(EnvelopeError::PairMacFailed)
        ));
        assert_eq!(b.forgotten(), vec![a.enc_pubkey()]);
        nothing_observed(&b);
        // the honest INTRO: authenticated via the card, key kept
        let pkt = seal_envelope(&a, &b.keys(), Priority::Interactive, &inner).unwrap();
        let got = open_envelope(&b, &pkt, &|_: &[u8; 4]| Vec::new()).unwrap();
        assert!(got.via_intro);
        assert_eq!(b.forgotten(), vec![a.enc_pubkey()]);
        counter_left_to_caller(&b, &got, 1);
    }

    /// The signature binds the fragment slot: a signed fragment moved to
    /// another `(frag_idx, frag_count)` by a pair-key holder fails `sig`.
    #[test]
    fn sig_binds_fragment_position() {
        let (a, b) = pair();
        let mut inner = Inner::new(
            &a.node_id(),
            Kind::AgentOp,
            MsgSeq::new(2, 2),
            b"part".to_vec(),
        );
        inner.frag = Some(FragPos { idx: 1, count: 3 });
        let pkt = seal_envelope(&a, &b.keys(), Priority::Interactive, &inner).unwrap();
        let got = open_envelope(&b, &pkt, &lookup_of(vec![a.keys()])).unwrap();
        let sig = got.sig.unwrap();
        // b (a pair-key holder) re-seals the same payload and sig at slot 0/3
        let mut moved = inner.clone();
        moved.frag = Some(FragPos { idx: 0, count: 3 });
        let before = moved.encode_before_auth().unwrap();
        let header = OuterExtra::to(Priority::Interactive, &b.node_id()).header(2);
        let hb = header_bytes(&header).unwrap();
        let key = a.pair_key(&b.enc_pubkey(), Direction::Outbound).unwrap();
        let forged = a
            .seal_payload(&b.enc_pubkey(), &header, &mut |epk: &Key32| {
                Inner::finish(
                    &before,
                    &pair_mac(&key, &hb, epk, &before).unwrap(),
                    Some(&sig),
                )
            })
            .unwrap();
        assert!(matches!(
            open_envelope(&b, &forged, &lookup_of(vec![a.keys()])),
            Err(EnvelopeError::BadSignature(_))
        ));
        // and an unfragmented message signs 0 ‖ 0, distinct from slot 0/1
        let whole = Inner::new(
            &a.node_id(),
            Kind::AgentOp,
            MsgSeq::new(2, 3),
            b"w".to_vec(),
        );
        let mut slot0 = whole.clone();
        slot0.frag = Some(FragPos { idx: 0, count: 1 });
        let dest = b.node_id().to_bytes();
        assert_ne!(
            sig_input(&a.node_id(), &dest, 0, &whole),
            sig_input(&a.node_id(), &dest, 0, &slot0)
        );
    }

    #[test]
    fn intro_first_contact_and_conflicts() {
        let (a, b) = pair();
        let mut inner = Inner::new(
            &a.node_id(),
            Kind::Msg,
            MsgSeq::new(1, 1),
            b"hello".to_vec(),
        );
        inner.card = Some(a.own_announce());
        let pkt = seal_envelope(&a, &b.keys(), Priority::Interactive, &inner).unwrap();
        let got = open_envelope(&b, &pkt, &|_: &[u8; 4]| Vec::new()).unwrap();
        assert!(got.via_intro);
        assert_eq!(got.sender, a.keys());
        assert!(b.forgotten().is_empty());
        counter_left_to_caller(&b, &got, 1);
        // pinned with the same keys ⇒ authenticated as the contact
        let got = open_envelope(&b, &pkt, &lookup_of(vec![a.keys()])).unwrap();
        assert!(!got.via_intro);
        // pinned with different keys ⇒ conflict surfaced
        let wrong = PeerKeys {
            node_id: a.node_id(),
            enc_pubkey: TestSealer::from_seed(4).enc_pubkey(),
        };
        assert!(matches!(
            open_envelope(&b, &pkt, &lookup_of(vec![wrong])),
            Err(EnvelopeError::IntroKeyConflict)
        ));
        // a card that is not the sender's
        let mut bad = inner.clone();
        bad.card = Some(TestSealer::from_seed(5).own_announce());
        let pkt = seal_envelope(&a, &b.keys(), Priority::Interactive, &bad).unwrap();
        assert!(matches!(
            open_envelope(&b, &pkt, &|_: &[u8; 4]| Vec::new()),
            Err(EnvelopeError::IntroMismatch)
        ));
        // a tampered card fails its signature
        let mut bad = inner.clone();
        let mut card = a.own_announce();
        card[100] ^= 1;
        bad.card = Some(card);
        let pkt = seal_envelope(&a, &b.keys(), Priority::Interactive, &bad).unwrap();
        assert!(matches!(
            open_envelope(&b, &pkt, &|_: &[u8; 4]| Vec::new()),
            Err(EnvelopeError::BadCard)
        ));
        // INTRO only on MSG / ENROLL; ENROLL needs INTRO
        let mut x = Inner::new(&a.node_id(), Kind::AgentOp, MsgSeq::new(1, 1), vec![]);
        x.card = Some(a.own_announce());
        assert!(matches!(
            x.encode_before_auth(),
            Err(EnvelopeError::IntroNotAllowed(Kind::AgentOp))
        ));
        let e = Inner::new(
            &a.node_id(),
            Kind::Enroll,
            MsgSeq::CONTROL,
            b"CODE".to_vec(),
        );
        assert!(matches!(
            e.encode_before_auth(),
            Err(EnvelopeError::EnrollWithoutIntro)
        ));
    }

    #[test]
    fn piggyback_and_ext_round_trip() {
        let (a, b) = pair();
        let mut inner = Inner::new(
            &a.node_id(),
            Kind::Msg,
            MsgSeq::new(300, 200),
            b"re".to_vec(),
        );
        inner.ext.seen_peer_seq = Some(MsgSeq::new(7, 3));
        inner.ext.unknown.push(RawTlv {
            tlv_type: 0x7f,
            value: vec![1, 2, 3],
        });
        inner.ack = Some(AckBlock {
            epoch: 7,
            prefix: 3,
            ranges: vec![AckRange {
                epoch: 7,
                start: 6,
                len: 2,
            }],
            partials: vec![PartialAck {
                seq: MsgSeq::new(7, 5),
                frag_bitmap: 0x0003,
            }],
            strong: vec![],
        });
        let pkt = seal_envelope(&a, &b.keys(), Priority::Interactive, &inner).unwrap();
        let got = open_envelope(&b, &pkt, &lookup_of(vec![a.keys()])).unwrap();
        assert_eq!(got.inner, inner);
        assert_eq!(got.inner.flags(), flags::HAS_EXT | flags::PIGGYBACK_ACK);
    }

    #[test]
    fn decode_rejections() {
        let a = TestSealer::from_words(MNEMONIC_A);
        let base = Inner::new(&a.node_id(), Kind::Msg, MsgSeq::new(1, 1), b"p".to_vec());
        let ok = base.encode_before_auth().unwrap();
        assert_eq!(Inner::decode_before_auth(&ok).unwrap(), base);
        let with = |f: &dyn Fn(&mut Vec<u8>)| {
            let mut v = ok.clone();
            f(&mut v);
            Inner::decode_before_auth(&v)
        };
        assert!(matches!(
            with(&|v| v[5] |= 0x40),
            Err(EnvelopeError::ReservedFlags(_))
        ));
        assert!(matches!(
            with(&|v| v[5] |= 0x80),
            Err(EnvelopeError::ReservedFlags(_))
        ));
        assert!(matches!(
            with(&|v| v[4] = 0x0a),
            Err(EnvelopeError::UnknownKind(0x0a))
        ));
        assert!(matches!(
            with(&|v| v[4] = 0x30),
            Err(EnvelopeError::KindNotBuilt(Kind::Broadcast))
        ));
        assert!(matches!(
            with(&|v| v[4] = 0x21),
            Err(EnvelopeError::KindNotBuilt(Kind::ObjReq))
        ));
        assert!(matches!(
            with(&|v| v[4] = Kind::Ack.byte()),
            Err(EnvelopeError::ControlSeq(Kind::Ack))
        ));
        assert!(matches!(
            with(&|v| v[7] = 0),
            Err(EnvelopeError::UnsequencedSeq(Kind::Msg))
        ));
        // HAS_FRAG with count 17 / idx ≥ count
        assert!(matches!(
            with(&|v| {
                v[5] |= flags::HAS_FRAG;
                v.splice(8..8, [0, 17]);
            }),
            Err(EnvelopeError::FragBounds { idx: 0, count: 17 })
        ));
        assert!(matches!(
            with(&|v| {
                v[5] |= flags::HAS_FRAG;
                v.splice(8..8, [3, 3]);
            }),
            Err(EnvelopeError::FragBounds { idx: 3, count: 3 })
        ));
        // TLVs: empty list, delegation, reserved, duplicate
        assert!(matches!(
            with(&|v| {
                v[5] |= flags::HAS_EXT;
                v.insert(8, 0);
            }),
            Err(EnvelopeError::ExtEmpty)
        ));
        assert!(matches!(
            with(&|v| {
                v[5] |= flags::HAS_EXT;
                v.splice(8..8, [1, 0x20, 0]);
            }),
            Err(EnvelopeError::DelegationTrailer)
        ));
        for t in [0x10u8, 0x21] {
            assert!(matches!(
                with(&|v| {
                    v[5] |= flags::HAS_EXT;
                    v.splice(8..8, [1, t, 0]);
                }),
                Err(EnvelopeError::ReservedTlv(x)) if x == t
            ));
        }
        assert!(matches!(
            with(&|v| {
                v[5] |= flags::HAS_EXT;
                v.splice(8..8, [2, 0x40, 0, 0x40, 0]);
            }),
            Err(EnvelopeError::DuplicateTlv(0x40))
        ));
        // an unknown TLV is skipped (kept as raw)
        let skipped = with(&|v| {
            v[5] |= flags::HAS_EXT;
            v.splice(8..8, [1, 0x40, 1, 0xee]);
        })
        .unwrap();
        assert_eq!(skipped.ext.unknown[0].tlv_type, 0x40);
        // over-long msg_seq varint
        assert!(matches!(
            with(&|v| v.splice(6..7, [0x81, 0x00]).for_each(drop)),
            Err(EnvelopeError::Read(ReadError::VarintOverlong))
        ));
        // PIGGYBACK on ACK
        let mut ack = vec![0u8; 4];
        ack.copy_from_slice(&ok[..4]);
        ack.extend_from_slice(&[Kind::Ack.byte(), flags::PIGGYBACK_ACK, 0, 0]);
        assert!(matches!(
            Inner::decode_before_auth(&ack),
            Err(EnvelopeError::PiggybackOnAck)
        ));
        // RECEIPT_STRONG / HAS_FRAG on a control kind
        let mut c = Inner::new(&a.node_id(), Kind::MailboxFetch, MsgSeq::CONTROL, vec![0]);
        c.receipt_strong = true;
        assert!(matches!(
            c.encode_before_auth(),
            Err(EnvelopeError::StrongOnControl(Kind::MailboxFetch))
        ));
        c.receipt_strong = false;
        c.frag = Some(FragPos { idx: 0, count: 2 });
        assert!(matches!(
            c.encode_before_auth(),
            Err(EnvelopeError::FragOnControl(Kind::MailboxFetch))
        ));
        // INTRO + HAS_FRAG
        let mut m = base.clone();
        m.card = Some(a.own_announce());
        m.frag = Some(FragPos { idx: 0, count: 2 });
        assert!(matches!(
            m.encode_before_auth(),
            Err(EnvelopeError::IntroFragmented)
        ));
    }

    #[test]
    fn split_auth_reads_only_flags() {
        assert!(matches!(
            split_auth(&[0u8; MIN_INNER_LEN - 1]),
            Err(EnvelopeError::Read(ReadError::Truncated))
        ));
        let mut v = vec![0u8; MIN_INNER_LEN];
        v[5] = flags::SIGNED;
        assert!(matches!(
            split_auth(&v),
            Err(EnvelopeError::Read(ReadError::Truncated))
        ));
        v[5] = 0x80;
        assert!(matches!(
            split_auth(&v),
            Err(EnvelopeError::ReservedFlags(0x80))
        ));
    }

    #[test]
    fn packet_size_gates() {
        assert!(matches!(
            OuterView::parse(&[0u8; MIN_PACKET_LEN - 1]),
            Err(EnvelopeError::PacketSize(_))
        ));
        assert!(matches!(
            OuterView::parse(&vec![0u8; MAX_PACKET_LEN + 1]),
            Err(EnvelopeError::PacketSize(_))
        ));
        let mut p = vec![0u8; MIN_PACKET_LEN];
        p[0] = 1;
        p[1] = 1;
        p[10] = 4;
        assert!(matches!(
            OuterView::parse(&p),
            Err(EnvelopeError::ExtraLength(4))
        ));
        p[10] = 5;
        p[0] = 9;
        assert!(matches!(
            OuterView::parse(&p),
            Err(EnvelopeError::UnknownVersion(9))
        ));
        p[0] = 1;
        p[1] = 2;
        assert!(matches!(
            OuterView::parse(&p),
            Err(EnvelopeError::UnsupportedCryptoMode(2))
        ));
        p[1] = 1;
        p[11] = 0x04;
        assert!(matches!(
            OuterView::parse(&p),
            Err(EnvelopeError::ReservedPrioBits(0x04))
        ));
        p[11] = 0x40;
        let view = OuterView::parse(&p).unwrap();
        assert_eq!(view.extra.priority, Priority::Interactive);
        assert_eq!(view.dedup_key(), [0u8; 8]);
        let mut big = Inner::new(
            &NodeId::from([1; 32]),
            Kind::Msg,
            MsgSeq::new(1, 1),
            vec![0; MAX_INNER_LEN],
        );
        big.signed = false;
        assert!(matches!(
            big.encode_before_auth(),
            Err(EnvelopeError::TooLarge(_))
        ));
    }

    /// §3 size table, asserted by sealing real envelopes at the budget.
    #[test]
    fn size_table() {
        let (a, b) = pair();
        assert_eq!(OUTER_OVERHEAD, 64);
        let small = MsgSeq::new(1, 1); // 2 B msg_seq ⇒ inner hdr 8
        let large = MsgSeq::new(200, 200); // 4 B msg_seq ⇒ inner hdr 10
        let tier1 = Some(FragPos { idx: 0, count: 16 });
        let cases: [(usize, Option<FragPos>, bool, usize, usize); 5] = [
            (160, None, false, 71, 69),
            (160, tier1, false, 69, 67),
            (160, None, true, 7, 5),
            (233, None, false, 144, 142),
            (233, None, true, 80, 78),
        ];
        let mut sealed = 0;
        for (bearer, frag, signed, hi, lo) in cases {
            for (seq, want) in [(small, hi), (large, lo)] {
                let budget = payload_budget(bearer, true, seq, frag, signed).unwrap();
                assert_eq!(
                    budget, want,
                    "bearer {bearer} frag {frag:?} signed {signed}"
                );
                let kind = if signed { Kind::AgentOp } else { Kind::Msg };
                let mut inner = Inner::new(&a.node_id(), kind, seq, vec![0x55; budget]);
                inner.frag = frag;
                let pkt = seal_envelope(&a, &b.keys(), Priority::Interactive, &inner).unwrap();
                assert_eq!(SHIM_LEN + pkt.len(), bearer);
                let got = open_envelope(&b, &pkt, &lookup_of(vec![a.keys()])).unwrap();
                assert_eq!(got.inner, inner);
                sealed += 1;
            }
        }
        assert_eq!(sealed, 10);
        // TCP row: up to 65,535 − 90 with a 26 B inner header + pair_mac
        assert_eq!(
            payload_budget(MAX_PACKET_LEN, false, large, None, false),
            Some(65_535 - 90)
        );
        assert_eq!(payload_budget(80, true, small, None, false), None);
    }
}
