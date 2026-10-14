//! T0 — the identity-holder handle `mesh111` talks to (spec §1, §3, §18.3).
//!
//! The keyset never enters this crate. Everything that touches key material —
//! sealing, opening, the per-contact pair key (static-static X25519 behind
//! `111/pair/v1`), domain-tagged signatures, the L2 counters — is asked of the
//! daemon through this trait. `111d` implements it over `id111` in
//! `daemon/fabric/` (task F2); tests use in-memory doubles.

use id111::keyset::NodeId;
use id111::seal::{OpenError, SealError, SealHeader};

/// A 32-byte key or key-derived value.
pub type Key32 = [u8; 32];

/// The L2 window's verdict on a verified packet (spec §4, A11): bookkeeping,
/// never rejection, for `msg_seq` kinds. `Fresh` gates connection attribution,
/// card answers and proactive pushes; a replayed packet is never `Fresh`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Stale,
}

/// Which way a pair key runs (spec §3):
/// `key = BLAKE2b-256("111/pair/v1" ‖ ss ‖ sender_enc_pk ‖ recipient_enc_pk)`.
/// `Outbound` = this node is the sender; `Inbound` = the peer is the sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Outbound,
    Inbound,
}

/// Errors the handle can report. Typed, never panicking (L2 §12 rule 7).
#[derive(Debug, thiserror::Error)]
pub enum SealerError {
    #[error("seal failed: {0:?}")]
    Seal(SealError),
    #[error("open failed: {0:?}")]
    Open(OpenError),
    #[error("entropy unavailable")]
    Entropy,
    #[error("counter store: {0}")]
    Counters(String),
}

/// What `mesh111` may ask of the identity holder.
///
/// Rules carried from the spec: `pair_key` results are cached per peer inside
/// the implementation and zeroized on `forget_pair_key` (§18.3); `sign`
/// prepends the ASCII domain tag itself (L2 §6.1); `observe_counter` advances
/// the per-peer high-water mark **without** enforcing the 32-entry window
/// (L2 amendment A11 — replay authority for `msg_seq` kinds is the store);
/// `next_announce_counter` is the §4 epoch source and commits (fsync) before
/// returning.
pub trait Sealer: Send + Sync {
    /// This node's NodeId (= Ed25519 public key, L2 B2).
    fn node_id(&self) -> NodeId;

    /// This node's own announce, the stored bytes — never minted here (§7).
    fn own_announce(&self) -> Vec<u8>;

    /// §3: seal a caller-owned inner layout (no inner NodeId, amendment A2)
    /// to `recipient_enc_pubkey` under `header` (version, crypto-mode,
    /// counter, `extra`). The implementation draws the fresh ephemeral key,
    /// calls `inner` **once** with that `epk`, and AEADs whatever it returns —
    /// because `pair_mac` (inside the ciphertext) covers
    /// `header bytes ‖ epk ‖ inner-before-auth`, the caller must see the epk
    /// before it can finish the inner bytes. Returns the wire packet.
    fn seal_payload(
        &self,
        recipient_enc_pubkey: &Key32,
        header: &SealHeader,
        inner: &mut dyn FnMut(&Key32) -> Vec<u8>,
    ) -> Result<Vec<u8>, SealerError>;

    /// §3: open a packet sealed to this node. Returns the AD-verified header,
    /// the packet's `epk` (so the caller can recompute `pair_mac` over
    /// `header bytes ‖ epk ‖ inner-before-auth` without re-parsing) and the
    /// inner bytes. Replay and sender checks are the caller's (§18.1).
    fn open_payload(&self, packet: &[u8]) -> Result<(SealHeader, Key32, Vec<u8>), SealerError>;

    /// §3: the pair key for `direction` with the peer whose X25519 key is
    /// `peer_enc_pubkey`. Cached per peer by the implementation.
    fn pair_key(&self, peer_enc_pubkey: &Key32, direction: Direction)
        -> Result<Key32, SealerError>;

    /// Drop the cached pair key for a forgotten contact (zeroized).
    fn forget_pair_key(&self, peer_enc_pubkey: &Key32);

    /// L2 §6.1: Ed25519 over `tag ‖ msg` with this node's signing key.
    fn sign(&self, tag: &[u8], msg: &[u8]) -> [u8; 64];

    /// L2 §6.5: the next strictly increasing send counter for `peer`.
    fn next_send_counter(&self, peer: &NodeId) -> Result<u64, SealerError>;

    /// A11: offer `counter` from `peer` to the L2 window and record it
    /// (high-water advanced either way); returns the freshness verdict.
    fn observe_counter(&self, peer: &NodeId, counter: u64) -> Result<Freshness, SealerError>;

    /// §4 epoch source: a freshly committed announce-counter value. Errors
    /// while a restore floor is pending (`RestoreFloorRequired`): nothing is
    /// sealed until it clears.
    fn next_announce_counter(&self) -> Result<u64, SealerError>;

    /// §4: the identity's current `announce_last`, recorded at clean close and
    /// compared at open to decide whether a new epoch is minted.
    fn announce_last(&self) -> u64;

    /// §5 / §11: fresh random bytes (session tags, object keys) from the §4
    /// entropy discipline. Typed failure, never a silent weak fallback.
    fn random(&self, out: &mut [u8]) -> Result<(), SealerError>;
}
