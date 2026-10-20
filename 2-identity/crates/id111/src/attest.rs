//! §10 — transport attestation (F1): binding a per-node transport credential
//! to the node by bidirectional cross-signed attestation (Keybase-proof /
//! Matrix cross-signing pattern; NIP-46's client-key/user-key split is the
//! in-nostr precedent).
//!
//! Implements (owner: T6): the 202B wire format
//! `version(0x01) ‖ node_pubkey(32) ‖ transport_id(1; registry owned by this
//! layer — 0x01 = nostr-secp256k1) ‖ transport_key(32) ‖ expiry_epoch(8B LE,
//! mesh-epoch) ‖ node_sig(64) ‖ transport_sig(64)` with domain-tagged signed
//! inputs (§6.1: the tag is part of the signed payload, reconstructed by the
//! verifier, never a wire field):
//!
//! - `node_sig` covers `"111/attest-node/v1" ‖ version ‖ node_pubkey ‖
//!   transport_key ‖ transport_id ‖ expiry_epoch` — **note the signed-input
//!   field order differs from the wire order** (`transport_key` before
//!   `transport_id` here); this module pins both layouts byte-exact.
//! - `transport_sig` covers `"111/attest-transport/v1" ‖ node_pubkey`.
//!
//! **IMPLEMENTATION BOUNDARY (normative).** id111 verifies `node_sig`
//! strictly under the §3 predicate (Ed25519, `verify_strict`-class — dalek's
//! default `verify()` is banned for wire artifacts). `transport_sig` is
//! secp256k1/BIP340 territory: k256 is **not** in the frozen dependency set
//! (§11: `candidate` when adopted — Layer 7's adoption decision). So
//! `transport_sig` is carried as **opaque bytes** and its verification lives
//! behind the [`TransportVerifier`] trait seam — the L7 gateway plugs its
//! secp256k1 verifier in. id111's built-in verification covers **structure +
//! `node_sig` only** ([`Attestation::verify_node_leg`]); the bidirectional
//! check is [`Attestation::verify_transport_leg`] / [`Attestation::verify_full`]
//! with a plugged verifier. Node-side verification alone already proves the
//! node authorized the binding (only the node key can produce `node_sig`); the
//! transport leg proves the reverse direction (the transport-key holder
//! vouches for `node_pubkey`) to third parties.
//!
//! **Supersedure (normative — panel crypto#6):** per `(node_pubkey,
//! transport_id)` the attestation with the greatest `expiry_epoch` wins;
//! verifiers retain only it and reject re-publication of a superseded one
//! (without this rule a replayed old attestation defeats every rotation).
//! Embodied by [`AttestationSet`], which verifies `node_sig` **before any
//! state change** (the §6.2 rule carried to attestations).
//!
//! **Expiry (v1 fail-mode, §6.7/§14):** `expiry_epoch` counts mesh-epochs,
//! never wall-clock (§12 rule 15). The epoch source is Layer 7's; until one
//! exists the field is **stored and compared, never evaluated** — this crate
//! has no clock API, and expiry checks take the current epoch as an explicit
//! caller-supplied argument ([`Attestation::is_expired`],
//! [`Attestation::verify_full`]). Semantics pinned for when a source lands:
//! `expiry_epoch` is the first mesh-epoch at which the attestation is dead.
//!
//! Hard rules carried from §10: a transport key MUST NOT share seed material
//! with the node key (no common seed, no shared derivation branch — compromise
//! cascade; enforced by the ceremony, not representable here) · a transport
//! credential is never the root identity · NIP-05/NIP-26 are never
//! load-bearing. This machinery is the designed future rotation path: under
//! B2 rotation is key succession, the successor a new NodeId pinned fresh
//! under §6.3, with the old key's attestation providing verifiable lineage.
//! This layer defines the **format only**; Layer 7 owns transports and
//! gateway publication policy.
//!
//! This module handles PUBLIC material only (pubkeys, signatures, epochs) —
//! no secret material passes through, so no zeroize handling is required.
//! Signing happens inside [`crate::keyset::Keyset::sign_with_tag`], which
//! holds the secret for its duration only.

use std::collections::HashMap;
use std::fmt;

use crate::keyset::{verify_strict_tagged, Keyset, NodeId, VerifyError};

// ---------------------------------------------------------------------------
// §10 — wire constants (normative)
// ---------------------------------------------------------------------------

/// §10 — the attestation wire length: `1 + 32 + 1 + 32 + 8 + 64 + 64 = 202`.
pub const WIRE_LEN: usize = 202;

/// §10 — the only attestation version in v1. Unknown version ⇒ fail closed.
pub const VERSION_V1: u8 = 0x01;

/// §10/§6.1 — domain tag inside the `node_sig` signed region.
pub const NODE_TAG: &[u8] = b"111/attest-node/v1";

/// §10/§6.1 — domain tag inside the `transport_sig` signed region.
pub const TRANSPORT_TAG: &[u8] = b"111/attest-transport/v1";

/// The `node_sig` signed-input body length:
/// `version(1) ‖ node_pubkey(32) ‖ transport_key(32) ‖ transport_id(1) ‖
/// expiry_epoch(8)` = 74 (the tag is prepended inside the signed region).
const NODE_BODY_LEN: usize = 74;

/// The `transport_sig` signed input length: tag(23) ‖ `node_pubkey(32)` = 55.
const TRANSPORT_INPUT_LEN: usize = 55;

// Wire offsets (§10, normative layout).
const OFF_VERSION: usize = 0;
const OFF_NODE: usize = 1;
const OFF_TID: usize = 33;
const OFF_TKEY: usize = 34;
const OFF_EXPIRY: usize = 66;
const OFF_NODE_SIG: usize = 74;
const OFF_TRANSPORT_SIG: usize = 138;

// ---------------------------------------------------------------------------
// §10 — mesh-epoch (a counter, never a clock; §12 rule 15)
// ---------------------------------------------------------------------------

/// §9/§10 — a mesh-epoch counter value. Counts epochs from Layer 7's epoch
/// source; **never wall-clock** (§12 rule 15 — clockless nodes). v1 has no
/// epoch source (§6.7 fail-mode): values are stored and ordered, and expiry
/// is evaluated only against an explicitly caller-supplied epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeshEpoch(u64);

impl MeshEpoch {
    /// Wrap a raw epoch counter value (from L7's epoch source).
    pub fn from_u64(v: u64) -> Self {
        MeshEpoch(v)
    }

    /// The raw epoch counter value.
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// §10 wire encoding: 8 bytes, little-endian.
    pub fn to_le_bytes(self) -> [u8; 8] {
        self.0.to_le_bytes()
    }

    /// Inverse of [`MeshEpoch::to_le_bytes`].
    pub fn from_le_bytes(b: [u8; 8]) -> Self {
        MeshEpoch(u64::from_le_bytes(b))
    }
}

impl fmt::Display for MeshEpoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "epoch {}", self.0)
    }
}

// ---------------------------------------------------------------------------
// §10 — the transport-id registry (owned by this layer, L2)
// ---------------------------------------------------------------------------

/// §10 — the transport registry, owned by this layer. Fail-closed on parse:
/// an unknown `transport_id` is rejected (§12 rule 1), so adding a transport
/// is a soft upgrade — un-upgraded peers refuse attestations naming it.
/// Append-only: assigned ids are never reused or weakened.
///
/// A10 (§13.1) assigned `0x02` and `0x03` for **bridge identities** (Layer 6,
/// `os/60-transport/README.md` §14; D26 #3, 2026-10-19). This crate carries
/// the bytes and the binding rules below; it grows no verifier for them —
/// the [`TransportVerifier`] plug lives in `node111` (the trait is the seam).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportId {
    /// `0x01` — a nostr secp256k1 public key (the npub's raw 32B x-only key).
    NostrSecp256k1,
    /// `0x02` — a Meshtastic board (A10; Layer 6 §14). `transport_key` = the
    /// board's X25519 public key (32 B; the node number is learned from
    /// NodeInfo, never carried). A 2.7.x board cannot sign (X25519 only), so
    /// `transport_sig` is **64 zero bytes** and the record is **one-sided**:
    /// `node_sig` only. A one-sided record never yields `attested-111` —
    /// anyone can write a record naming a victim's board key. A 2.8
    /// XEdDSA-signed broadcast over `"111/attest-transport/v1" ‖ node_pubkey`
    /// may fill the field later; no verifier for it exists in v1, so a
    /// non-zero `transport_sig` under `0x02` is rejected by the plug.
    Meshtastic,
    /// `0x03` — a Reticulum identity (A10; Layer 6 §14). The 64 B Reticulum
    /// public key (`X25519 ‖ Ed25519`) does not fit the 32 B field, so
    /// `transport_key` = SHA-256 of the whole 64 B key
    /// ([`reticulum_transport_key`]; the hash covers both halves).
    /// `transport_sig` = Ed25519 by the identity's signing key (the second
    /// 32 B) over the same input `"111/attest-transport/v1" ‖ node_pubkey`:
    /// **both signatures**. The 64 B key is carried (the helper's
    /// `ready.pubkey`), never derived.
    Reticulum,
}

/// §10 + A10 (§13.1; Layer 6 §14) — the `transport_key` of a Reticulum
/// bridge identity (`transport_id` `0x03`): SHA-256 of the full 64 B public
/// key `X25519(32) ‖ Ed25519(32)` as carried from the helper. A pure binding
/// helper — not a verifier, and no Reticulum arithmetic (no address or
/// destination-hash derivation).
pub fn reticulum_transport_key(pubkey64: &[u8; 64]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(pubkey64).into()
}

/// A10 (Layer 6 §14) — the `transport_sig` a 2.7.x Meshtastic board leaves
/// in a `0x02` record: 64 zero bytes (it cannot sign). Marks the record
/// one-sided; it is not a signature.
pub const MESHTASTIC_UNSIGNED: [u8; 64] = [0u8; 64];

impl TransportId {
    /// The registry byte (§10).
    pub fn to_byte(self) -> u8 {
        match self {
            TransportId::NostrSecp256k1 => 0x01,
            TransportId::Meshtastic => 0x02,
            TransportId::Reticulum => 0x03,
        }
    }

    /// Inverse of [`TransportId::to_byte`]; fail-closed on unassigned bytes.
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(TransportId::NostrSecp256k1),
            0x02 => Some(TransportId::Meshtastic),
            0x03 => Some(TransportId::Reticulum),
            _ => None,
        }
    }
}

impl fmt::Display for TransportId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportId::NostrSecp256k1 => write!(f, "nostr-secp256k1 (0x01)"),
            TransportId::Meshtastic => write!(f, "meshtastic (0x02)"),
            TransportId::Reticulum => write!(f, "reticulum (0x03)"),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// §10 attestation errors. Every variant is a rejection — parsing,
/// verification, and resolution all fail closed (§12 rule 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttestError {
    /// §12 rule 7 — the wire blob is not exactly 202B (fixed-field binary;
    /// size-limited before parse).
    InvalidLength { expected: usize, got: usize },
    /// §10 — unknown `version`; fail closed (no silent downgrade).
    UnknownVersion(u8),
    /// §10 — unknown `transport_id`; the L2-owned registry assigns `0x01`
    /// (and, since A10, `0x02` / `0x03`); unknown entries fail closed (§12
    /// rule 1).
    UnknownTransportId(u8),
    /// §3 — `node_sig` failed the strict Ed25519 predicate.
    InvalidNodeSignature(VerifyError),
    /// §10 — the plugged [`TransportVerifier`] rejected `transport_sig`.
    InvalidTransportSignature,
    /// §10 — the attestation is dead at the supplied epoch (`current >=
    /// expiry_epoch`); expired attestations are dead. Only reachable with a
    /// caller-supplied mesh-epoch (§6.7: never evaluated without a source).
    Expired {
        expiry_epoch: MeshEpoch,
        current: MeshEpoch,
    },
    /// §10 supersedure — re-publication of a superseded attestation: the
    /// resolver already holds a greater `expiry_epoch` for this
    /// `(node_pubkey, transport_id)`.
    Superseded {
        retained: MeshEpoch,
        offered: MeshEpoch,
    },
    /// §10 — same `(node_pubkey, transport_id, expiry_epoch)` but different
    /// bytes: an unresolvable conflict, rejected fail-closed (the spec's
    /// ordering is by `expiry_epoch` only; equal epochs admit no tie-break).
    ConflictingAttestation { epoch: MeshEpoch },
}

impl fmt::Display for AttestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttestError::InvalidLength { expected, got } => {
                write!(f, "§10: attestation must be {expected}B, got {got}B")
            }
            AttestError::UnknownVersion(v) => {
                write!(
                    f,
                    "§10: unknown attestation version 0x{v:02x} (fail closed)"
                )
            }
            AttestError::UnknownTransportId(t) => {
                write!(
                    f,
                    "§10: unknown transport_id 0x{t:02x} (L2 registry, fail closed)"
                )
            }
            AttestError::InvalidNodeSignature(e) => write!(f, "§10 node_sig: {e}"),
            AttestError::InvalidTransportSignature => {
                write!(
                    f,
                    "§10: transport_sig rejected by the plugged transport verifier"
                )
            }
            AttestError::Expired {
                expiry_epoch,
                current,
            } => {
                write!(
                    f,
                    "§10: attestation dead at {current} (expiry {expiry_epoch})"
                )
            }
            AttestError::Superseded { retained, offered } => write!(
                f,
                "§10 supersedure: retained expiry {retained} supersedes offered {offered}"
            ),
            AttestError::ConflictingAttestation { epoch } => write!(
                f,
                "§10: conflicting attestation bytes at equal expiry epoch {epoch}"
            ),
        }
    }
}

impl std::error::Error for AttestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AttestError::InvalidNodeSignature(e) => Some(e),
            _ => None,
        }
    }
}

impl From<VerifyError> for AttestError {
    fn from(e: VerifyError) -> Self {
        AttestError::InvalidNodeSignature(e)
    }
}

// ---------------------------------------------------------------------------
// The transport-verifier seam (implementation boundary — module header)
// ---------------------------------------------------------------------------

/// §10 — the transport-side verification seam. `transport_sig` is
/// secp256k1/BIP340 territory and k256 is not in the frozen dep set (§11:
/// `candidate` when adopted — Layer 7's decision), so id111 carries
/// `transport_sig` as opaque bytes and the **L7 gateway plugs its verifier
/// in** here.
///
/// `signed_input` is the canonical input assembled by this crate —
/// `"111/attest-transport/v1" ‖ node_pubkey` ([`TRANSPORT_TAG`]); the domain
/// tag and field order are owned here, never re-serialized by the plug
/// (§9's byte-exact rule applied to the attestation seam). `transport_id`
/// names the scheme the plug must apply; a plug MUST fail closed on a
/// `transport_id` it does not implement. Returns `true` iff the signature
/// verifies under `transport_key`.
pub trait TransportVerifier {
    fn verify_transport_signature(
        &self,
        transport_id: TransportId,
        transport_key: &[u8; 32],
        signed_input: &[u8],
        signature: &[u8; 64],
    ) -> bool;
}

// ---------------------------------------------------------------------------
// §10 — the signed inputs (byte-exact, pinned)
// ---------------------------------------------------------------------------

/// §10 — the `node_sig` signed-input body (tag prepended inside the signed
/// region by `sign_with_tag` / `verify_strict_tagged`):
/// `version ‖ node_pubkey ‖ transport_key ‖ transport_id ‖ expiry_epoch(LE)`.
/// **Field order differs from the wire order** (`transport_key` before
/// `transport_id`) — pinned by test.
fn node_sig_body(
    version: u8,
    node_pubkey: &NodeId,
    transport_key: &[u8; 32],
    transport_id: TransportId,
    expiry_epoch: MeshEpoch,
) -> [u8; NODE_BODY_LEN] {
    let mut body = [0u8; NODE_BODY_LEN];
    body[0] = version;
    body[1..33].copy_from_slice(node_pubkey.as_bytes());
    body[33..65].copy_from_slice(transport_key);
    body[65] = transport_id.to_byte();
    body[66..74].copy_from_slice(&expiry_epoch.to_le_bytes());
    body
}

/// §10 — the `transport_sig` signed input: `"111/attest-transport/v1" ‖
/// node_pubkey` (tag included — the full input is handed to the
/// [`TransportVerifier`] plug so the tag stays crate-owned).
fn transport_sig_input(node_pubkey: &NodeId) -> [u8; TRANSPORT_INPUT_LEN] {
    let mut input = [0u8; TRANSPORT_INPUT_LEN];
    input[..TRANSPORT_TAG.len()].copy_from_slice(TRANSPORT_TAG);
    input[TRANSPORT_TAG.len()..].copy_from_slice(node_pubkey.as_bytes());
    input
}

// ---------------------------------------------------------------------------
// §10 — the node-signed leg (creation helper; transport side is L7's)
// ---------------------------------------------------------------------------

/// §10 — the node-signed half of an attestation: all fields plus `node_sig`,
/// awaiting the transport leg. The cross-signing ceremony can start on either
/// side; this is the node side (signing via
/// [`Keyset::sign_with_tag`]; transport-side signing is out of scope — L7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeLeg {
    node_pubkey: NodeId,
    transport_id: TransportId,
    transport_key: [u8; 32],
    expiry_epoch: MeshEpoch,
    node_sig: [u8; 64],
}

impl NodeLeg {
    /// §10 — issue the node leg: sign `"111/attest-node/v1" ‖ version ‖
    /// node_pubkey ‖ transport_key ‖ transport_id ‖ expiry_epoch` with the
    /// node's Ed25519 key (strict domain-tagged signing, §6.1).
    ///
    /// Ceremony obligation (§10 hard rule): `transport_key` MUST NOT share
    /// seed material with the node key — no common seed, no shared derivation
    /// branch. That is a property of how the transport key was generated;
    /// this function cannot check it.
    pub fn issue(
        keyset: &Keyset,
        transport_id: TransportId,
        transport_key: [u8; 32],
        expiry_epoch: MeshEpoch,
    ) -> Self {
        let node_pubkey = keyset.node_id();
        let body = node_sig_body(
            VERSION_V1,
            &node_pubkey,
            &transport_key,
            transport_id,
            expiry_epoch,
        );
        let node_sig = keyset.sign_with_tag(NODE_TAG, &body);
        NodeLeg {
            node_pubkey,
            transport_id,
            transport_key,
            expiry_epoch,
            node_sig,
        }
    }

    /// The attesting node's NodeId (§2 B2: the Ed25519 public key).
    pub fn node_id(&self) -> NodeId {
        self.node_pubkey
    }

    pub fn transport_id(&self) -> TransportId {
        self.transport_id
    }

    /// The bound transport public key (raw 32B; scheme per `transport_id`).
    pub fn transport_key(&self) -> &[u8; 32] {
        &self.transport_key
    }

    /// The expiry mesh-epoch (stored/compared; never wall-clock, §6.7).
    pub fn expiry_epoch(&self) -> MeshEpoch {
        self.expiry_epoch
    }

    /// The node's signature over the §10 node signed input.
    pub fn node_sig(&self) -> &[u8; 64] {
        &self.node_sig
    }

    /// §10/§3 — verify the node leg under the strict predicate.
    pub fn verify_node_leg(&self) -> Result<(), AttestError> {
        let body = node_sig_body(
            VERSION_V1,
            &self.node_pubkey,
            &self.transport_key,
            self.transport_id,
            self.expiry_epoch,
        );
        Ok(verify_strict_tagged(
            &self.node_pubkey,
            NODE_TAG,
            &body,
            &self.node_sig,
        )?)
    }

    /// §10 — complete the bidirectional attestation by attaching the
    /// transport side's signature over `"111/attest-transport/v1" ‖
    /// node_pubkey` (produced by the transport-key holder — L7's ceremony).
    pub fn complete(self, transport_sig: [u8; 64]) -> Attestation {
        Attestation {
            node_pubkey: self.node_pubkey,
            transport_id: self.transport_id,
            transport_key: self.transport_key,
            expiry_epoch: self.expiry_epoch,
            node_sig: self.node_sig,
            transport_sig,
        }
    }
}

// ---------------------------------------------------------------------------
// §10 — the 202B attestation
// ---------------------------------------------------------------------------

/// §10 — a complete bidirectional transport attestation (202B wire).
/// Structural invariants (version = 0x01, `transport_id` in the L2 registry,
/// exact 202B length) hold by construction and are enforced fail-closed at
/// parse. `transport_sig` is **opaque bytes** to this crate — see the module
/// header's implementation boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attestation {
    node_pubkey: NodeId,
    transport_id: TransportId,
    transport_key: [u8; 32],
    expiry_epoch: MeshEpoch,
    node_sig: [u8; 64],
    transport_sig: [u8; 64],
}

impl Attestation {
    /// §10 — parse a 202B wire attestation. Fail closed: exact length (§12
    /// rule 7), version 0x01 (unknown ⇒ reject), `transport_id` in the L2
    /// registry (unknown ⇒ reject). Structural only — signatures are verified
    /// by [`verify_node_leg`](Self::verify_node_leg) /
    /// [`verify_transport_leg`](Self::verify_transport_leg), and MUST be
    /// verified before any state change ([`AttestationSet`] enforces this).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, AttestError> {
        if bytes.len() != WIRE_LEN {
            return Err(AttestError::InvalidLength {
                expected: WIRE_LEN,
                got: bytes.len(),
            });
        }
        let version = bytes[OFF_VERSION];
        if version != VERSION_V1 {
            return Err(AttestError::UnknownVersion(version));
        }
        let transport_id = TransportId::from_byte(bytes[OFF_TID])
            .ok_or(AttestError::UnknownTransportId(bytes[OFF_TID]))?;

        let mut node_pubkey = [0u8; 32];
        node_pubkey.copy_from_slice(&bytes[OFF_NODE..OFF_TID]);
        let mut transport_key = [0u8; 32];
        transport_key.copy_from_slice(&bytes[OFF_TKEY..OFF_EXPIRY]);
        let mut expiry = [0u8; 8];
        expiry.copy_from_slice(&bytes[OFF_EXPIRY..OFF_NODE_SIG]);
        let mut node_sig = [0u8; 64];
        node_sig.copy_from_slice(&bytes[OFF_NODE_SIG..OFF_TRANSPORT_SIG]);
        let mut transport_sig = [0u8; 64];
        transport_sig.copy_from_slice(&bytes[OFF_TRANSPORT_SIG..WIRE_LEN]);

        Ok(Attestation {
            node_pubkey: NodeId::from(node_pubkey),
            transport_id,
            transport_key,
            expiry_epoch: MeshEpoch::from_le_bytes(expiry),
            node_sig,
            transport_sig,
        })
    }

    /// §10 — the 202B wire encoding: `version ‖ node_pubkey ‖ transport_id ‖
    /// transport_key ‖ expiry_epoch(LE) ‖ node_sig ‖ transport_sig`.
    pub fn to_bytes(&self) -> [u8; WIRE_LEN] {
        let mut out = [0u8; WIRE_LEN];
        out[OFF_VERSION] = VERSION_V1;
        out[OFF_NODE..OFF_TID].copy_from_slice(self.node_pubkey.as_bytes());
        out[OFF_TID] = self.transport_id.to_byte();
        out[OFF_TKEY..OFF_EXPIRY].copy_from_slice(&self.transport_key);
        out[OFF_EXPIRY..OFF_NODE_SIG].copy_from_slice(&self.expiry_epoch.to_le_bytes());
        out[OFF_NODE_SIG..OFF_TRANSPORT_SIG].copy_from_slice(&self.node_sig);
        out[OFF_TRANSPORT_SIG..WIRE_LEN].copy_from_slice(&self.transport_sig);
        out
    }

    /// The attested node's NodeId (§2 B2).
    pub fn node_id(&self) -> NodeId {
        self.node_pubkey
    }

    pub fn transport_id(&self) -> TransportId {
        self.transport_id
    }

    /// The bound transport public key (raw 32B; scheme per `transport_id`).
    pub fn transport_key(&self) -> &[u8; 32] {
        &self.transport_key
    }

    /// The expiry mesh-epoch (stored/compared; never wall-clock, §6.7).
    pub fn expiry_epoch(&self) -> MeshEpoch {
        self.expiry_epoch
    }

    /// The node's Ed25519 signature over the §10 node signed input.
    pub fn node_sig(&self) -> &[u8; 64] {
        &self.node_sig
    }

    /// The transport side's signature — **opaque bytes** to id111 (module
    /// header's implementation boundary); verify via a [`TransportVerifier`].
    pub fn transport_sig(&self) -> &[u8; 64] {
        &self.transport_sig
    }

    /// §10/§3 — the built-in leg: verify `node_sig` over `"111/attest-node/v1"
    /// ‖ version ‖ node_pubkey ‖ transport_key ‖ transport_id ‖ expiry_epoch`
    /// under the strict Ed25519 predicate. Proves the node authorized this
    /// binding; does NOT cover `transport_sig` (see the module header).
    pub fn verify_node_leg(&self) -> Result<(), AttestError> {
        let body = node_sig_body(
            VERSION_V1,
            &self.node_pubkey,
            &self.transport_key,
            self.transport_id,
            self.expiry_epoch,
        );
        Ok(verify_strict_tagged(
            &self.node_pubkey,
            NODE_TAG,
            &body,
            &self.node_sig,
        )?)
    }

    /// §10 — the transport leg via the plugged [`TransportVerifier`] (the
    /// implementation-boundary seam; L7's secp256k1/BIP340 verifier). The
    /// signed input is assembled here: `"111/attest-transport/v1" ‖
    /// node_pubkey`.
    pub fn verify_transport_leg(
        &self,
        verifier: &dyn TransportVerifier,
    ) -> Result<(), AttestError> {
        let input = transport_sig_input(&self.node_pubkey);
        if verifier.verify_transport_signature(
            self.transport_id,
            &self.transport_key,
            &input,
            &self.transport_sig,
        ) {
            Ok(())
        } else {
            Err(AttestError::InvalidTransportSignature)
        }
    }

    /// §10 — the expiry predicate, evaluated only against a caller-supplied
    /// mesh-epoch (§6.7 fail-mode: never evaluated without an L7 epoch
    /// source; v1 stores/compares the field only). Semantics pinned for when
    /// a source lands: `expiry_epoch` is the **first** epoch at which the
    /// attestation is dead — expired attestations are dead (§10).
    pub fn is_expired(&self, current: MeshEpoch) -> bool {
        current >= self.expiry_epoch
    }

    /// §10 — the full check: both signatures (node leg built-in, transport
    /// leg via the plug) and the expiry epoch. `current_epoch` must come from
    /// Layer 7's mesh-epoch source — never a wall clock (§12 rule 15); v1
    /// callers have no such source and must not call this (§6.7).
    pub fn verify_full(
        &self,
        verifier: &dyn TransportVerifier,
        current_epoch: MeshEpoch,
    ) -> Result<(), AttestError> {
        self.verify_node_leg()?;
        self.verify_transport_leg(verifier)?;
        if self.is_expired(current_epoch) {
            return Err(AttestError::Expired {
                expiry_epoch: self.expiry_epoch,
                current: current_epoch,
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// §10 — the supersedure resolver (normative, panel crypto#6)
// ---------------------------------------------------------------------------

/// The outcome of an accepted [`AttestationSet::insert`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// First attestation seen for its `(node_pubkey, transport_id)`.
    Pinned,
    /// Accepted: its `expiry_epoch` is strictly greater than the retained
    /// one, which it supersedes (the superseded one is now dead — §10).
    Supersedes { previous: MeshEpoch },
    /// Byte-identical to the retained attestation (harmless gossip
    /// re-receipt); state unchanged.
    Duplicate,
}

/// §10 — the attestation resolver embodying the normative supersedure rule:
/// per `(node_pubkey, transport_id)` the greatest `expiry_epoch` wins; only
/// it is retained, and re-publication of a superseded attestation is
/// rejected. Equal `expiry_epoch` admits no ordering (the spec orders by
/// epoch only): byte-identical re-receipts are duplicates; differing bytes
/// are an unresolvable conflict, rejected fail-closed.
///
/// `node_sig` is verified under the §3 strict predicate **before any state
/// change** (the §6.2 verification rule carried to attestations). The
/// transport leg is verified only through
/// [`insert_with_transport`](Self::insert_with_transport) with a plugged
/// [`TransportVerifier`] — `insert` covers structure + `node_sig` only (the
/// module header's implementation boundary). Expiry is never evaluated here:
/// the resolver orders by `expiry_epoch` but has no epoch source (§6.7).
#[derive(Debug, Default)]
pub struct AttestationSet {
    by_binding: HashMap<(NodeId, TransportId), Attestation>,
}

impl AttestationSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// §10 — insert with the built-in coverage (structure + `node_sig`,
    /// strict predicate, before any state change). Use
    /// [`insert_with_transport`](Self::insert_with_transport) when a
    /// transport verifier is plugged (full bidirectional check).
    pub fn insert(&mut self, attestation: Attestation) -> Result<Resolution, AttestError> {
        attestation.verify_node_leg()?;
        self.resolve(attestation)
    }

    /// §10 — insert with the full two-signature check (node leg + transport
    /// leg via the plug) before any state change. Expiry is not evaluated —
    /// no mesh-epoch source exists in v1 (§6.7); dead-attestation pruning
    /// belongs to the layer that owns the epoch source.
    pub fn insert_with_transport(
        &mut self,
        attestation: Attestation,
        verifier: &dyn TransportVerifier,
    ) -> Result<Resolution, AttestError> {
        attestation.verify_node_leg()?;
        attestation.verify_transport_leg(verifier)?;
        self.resolve(attestation)
    }

    /// The supersedure rule itself: greatest `expiry_epoch` wins per
    /// `(node_pubkey, transport_id)`; superseded re-publication rejected;
    /// equal-epoch duplicates idempotent, equal-epoch conflicts refused.
    fn resolve(&mut self, attestation: Attestation) -> Result<Resolution, AttestError> {
        let key = (attestation.node_pubkey, attestation.transport_id);
        match self.by_binding.get(&key) {
            None => {
                self.by_binding.insert(key, attestation);
                Ok(Resolution::Pinned)
            }
            Some(retained) => {
                if attestation.expiry_epoch > retained.expiry_epoch {
                    let previous = retained.expiry_epoch;
                    self.by_binding.insert(key, attestation);
                    Ok(Resolution::Supersedes { previous })
                } else if attestation.expiry_epoch < retained.expiry_epoch {
                    Err(AttestError::Superseded {
                        retained: retained.expiry_epoch,
                        offered: attestation.expiry_epoch,
                    })
                } else if attestation == *retained {
                    Ok(Resolution::Duplicate)
                } else {
                    Err(AttestError::ConflictingAttestation {
                        epoch: attestation.expiry_epoch,
                    })
                }
            }
        }
    }

    /// The retained (greatest-`expiry_epoch`) attestation for a binding.
    pub fn get(&self, node: &NodeId, transport_id: TransportId) -> Option<&Attestation> {
        self.by_binding.get(&(*node, transport_id))
    }

    pub fn len(&self) -> usize {
        self.by_binding.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_binding.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyset::verify_strict;

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

    /// The canonical project test node (mnemonic "abandon×11 about" — the
    /// frozen §14 project keyset vector in keyset.rs).
    fn test_keyset() -> Keyset {
        Keyset::derive_from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap()
    }

    /// A second, distinct node (RFC 8032 TEST 1 seed; X25519 half = Alice's
    /// scalar, as in keyset.rs tests).
    fn second_keyset() -> Keyset {
        let sign_seed = hex32("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let alice_scalar =
            hex32("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&sign_seed);
        bytes[32..].copy_from_slice(&alice_scalar);
        Keyset::from_bytes(&bytes).unwrap()
    }

    /// Fixed transport-key bytes for tests/KAT: 0xe0..=0xff.
    fn fixed_transport_key() -> [u8; 32] {
        let mut k = [0u8; 32];
        for (i, b) in k.iter_mut().enumerate() {
            *b = 0xe0u8 + i as u8;
        }
        k
    }

    /// Fixed opaque transport_sig bytes for tests/KAT: 0xa0..=0xdf.
    fn fixed_transport_sig() -> [u8; 64] {
        let mut s = [0u8; 64];
        for (i, b) in s.iter_mut().enumerate() {
            *b = 0xa0u8 + i as u8;
        }
        s
    }

    const TID: TransportId = TransportId::NostrSecp256k1;

    fn epoch(v: u64) -> MeshEpoch {
        MeshEpoch::from_u64(v)
    }

    // ------------------------------------------------------------------
    // Round-trip and layout.
    // ------------------------------------------------------------------

    #[test]
    fn wire_layout_and_round_trip() {
        let ks = test_keyset();
        let leg = NodeLeg::issue(
            &ks,
            TID,
            fixed_transport_key(),
            epoch(0x1122_3344_5566_7788),
        );
        assert!(leg.verify_node_leg().is_ok());
        let att = leg.complete(fixed_transport_sig());

        let wire = att.to_bytes();
        assert_eq!(wire.len(), WIRE_LEN);
        // Normative offsets (§10).
        assert_eq!(wire[0], 0x01);
        assert_eq!(&wire[1..33], ks.node_id().as_bytes());
        assert_eq!(wire[33], 0x01); // transport_id = nostr-secp256k1
        assert_eq!(&wire[34..66], &fixed_transport_key());
        assert_eq!(&wire[66..74], &0x1122_3344_5566_7788u64.to_le_bytes());
        assert_eq!(&wire[74..138], att.node_sig());
        assert_eq!(&wire[138..202], &fixed_transport_sig());

        let parsed = Attestation::from_bytes(&wire).unwrap();
        assert_eq!(parsed, att);
        assert!(parsed.verify_node_leg().is_ok());
        assert_eq!(parsed.node_id(), ks.node_id());
        assert_eq!(parsed.transport_id(), TID);
        assert_eq!(parsed.transport_key(), &fixed_transport_key());
        assert_eq!(parsed.expiry_epoch(), epoch(0x1122_3344_5566_7788));
        // Re-encoding is byte-exact (fixed-field binary, §12 rule 7).
        assert_eq!(parsed.to_bytes(), wire);
    }

    #[test]
    fn signed_input_layouts_are_pinned() {
        // §10 pins the signed-input field order, which DIFFERS from the wire
        // order (transport_key before transport_id in the node input). Build
        // the expected inputs by hand and compare byte-exact.
        let node = NodeId::from([0x11u8; 32]);
        let tkey = [0x22u8; 32];
        let expiry = epoch(0x0102_0304_0506_0708);

        let body = node_sig_body(0x01, &node, &tkey, TID, expiry);
        let mut expected = vec![0x01u8];
        expected.extend_from_slice(&[0x11u8; 32]);
        expected.extend_from_slice(&[0x22u8; 32]);
        expected.push(0x01); // transport_id AFTER transport_key
        expected.extend_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
        assert_eq!(&body[..], &expected[..]);
        assert_eq!(body.len(), 74);

        let tinput = transport_sig_input(&node);
        let mut expected_t = b"111/attest-transport/v1".to_vec();
        expected_t.extend_from_slice(&[0x11u8; 32]);
        assert_eq!(&tinput[..], &expected_t[..]);
        assert_eq!(tinput.len(), 55);
        assert_eq!(NODE_TAG, b"111/attest-node/v1");
    }

    // ------------------------------------------------------------------
    // Fail-closed parsing (§10 version, §12 rule 1, §12 rule 7).
    // ------------------------------------------------------------------

    fn valid_wire() -> [u8; WIRE_LEN] {
        let ks = test_keyset();
        NodeLeg::issue(&ks, TID, fixed_transport_key(), epoch(1000))
            .complete(fixed_transport_sig())
            .to_bytes()
    }

    #[test]
    fn unknown_version_fails_closed() {
        for bad in [0x00u8, 0x02, 0x03, 0x04, 0xff] {
            let mut wire = valid_wire();
            wire[0] = bad;
            assert_eq!(
                Attestation::from_bytes(&wire),
                Err(AttestError::UnknownVersion(bad))
            );
        }
    }

    #[test]
    fn unknown_transport_id_fails_closed() {
        // 0x02 / 0x03 are assigned since A10 (Layer 6 §14); 0x04.. stay closed.
        for bad in [0x00u8, 0x04, 0x05, 0x80, 0xff] {
            let mut wire = valid_wire();
            wire[33] = bad;
            assert_eq!(
                Attestation::from_bytes(&wire),
                Err(AttestError::UnknownTransportId(bad))
            );
        }
        assert_eq!(TransportId::from_byte(0x01), Some(TID));
    }

    #[test]
    fn a10_registry_values_round_trip() {
        for (b, t, d) in [
            (
                0x01u8,
                TransportId::NostrSecp256k1,
                "nostr-secp256k1 (0x01)",
            ),
            (0x02, TransportId::Meshtastic, "meshtastic (0x02)"),
            (0x03, TransportId::Reticulum, "reticulum (0x03)"),
        ] {
            assert_eq!(TransportId::from_byte(b), Some(t));
            assert_eq!(t.to_byte(), b);
            assert_eq!(t.to_string(), d);
        }
        for b in 0x04..=0xffu8 {
            assert_eq!(TransportId::from_byte(b), None, "0x{b:02x}");
        }
        assert_eq!(TransportId::from_byte(0x00), None);
    }

    #[test]
    fn reticulum_transport_key_hashes_both_halves() {
        let mut k = [0u8; 64];
        for (i, b) in k.iter_mut().enumerate() {
            *b = i as u8;
        }
        let h = reticulum_transport_key(&k);
        // FIPS 180-4 SHA-256 of 0x00..0x3f.
        assert_eq!(
            to_hex(&h),
            "fdeab9acf3710362bd2658cdc9a29e8f9c757fcf9811603a8c447cd1d9151108"
        );
        let mut x = k;
        x[0] ^= 1; // X25519 half
        assert_ne!(reticulum_transport_key(&x), h);
        let mut e = k;
        e[63] ^= 1; // Ed25519 half
        assert_ne!(reticulum_transport_key(&e), h);
    }

    #[test]
    fn wrong_length_rejected() {
        let wire = valid_wire();
        assert!(matches!(
            Attestation::from_bytes(&wire[..201]),
            Err(AttestError::InvalidLength {
                expected: 202,
                got: 201
            })
        ));
        let mut long = wire.to_vec();
        long.push(0x00);
        assert!(matches!(
            Attestation::from_bytes(&long),
            Err(AttestError::InvalidLength {
                expected: 202,
                got: 203
            })
        ));
        assert!(matches!(
            Attestation::from_bytes(&[]),
            Err(AttestError::InvalidLength {
                expected: 202,
                got: 0
            })
        ));
    }

    // ------------------------------------------------------------------
    // Negatives: field-swap, transplant, tag confusion (§14).
    // ------------------------------------------------------------------

    #[test]
    fn field_swap_negatives_rejected() {
        // Tamper with the transport_key on the wire: node_sig must fail.
        let mut wire = valid_wire();
        wire[40] ^= 0x01;
        let att = Attestation::from_bytes(&wire).unwrap();
        assert!(matches!(
            att.verify_node_leg(),
            Err(AttestError::InvalidNodeSignature(_))
        ));

        // Tamper with the expiry epoch.
        let mut wire = valid_wire();
        wire[70] ^= 0x01;
        let att = Attestation::from_bytes(&wire).unwrap();
        assert!(att.verify_node_leg().is_err());

        // Swap the wire positions of transport_id and transport_key[0] with
        // transport_key[0] != 0x01: the registry gate fails closed at parse.
        // (With v1's single-entry registry a sig-level swap cannot exist:
        // staying parseable requires transport_key[0] == 0x01, which makes
        // the swap a byte-level no-op. The signed-input field-order subtlety
        // — key before id — is pinned byte-exact by
        // signed_input_layouts_are_pinned and negatively below.)
        let mut wire = valid_wire();
        wire.swap(33, 34);
        assert_eq!(
            Attestation::from_bytes(&wire),
            Err(AttestError::UnknownTransportId(0xe0))
        );
    }

    #[test]
    fn wire_order_signed_input_is_invalid() {
        // §10 pins the node signed input as version ‖ node_pubkey ‖
        // transport_key ‖ transport_id ‖ expiry_epoch — NOT the wire order
        // (id before key). A body assembled in wire order MUST NOT verify
        // against a genuine node_sig.
        let ks = test_keyset();
        let tkey = fixed_transport_key();
        let leg = NodeLeg::issue(&ks, TID, tkey, epoch(1000));

        let mut wrong_order = vec![0x01u8];
        wrong_order.extend_from_slice(ks.node_id().as_bytes());
        wrong_order.push(0x01); // transport_id BEFORE transport_key (wire order)
        wrong_order.extend_from_slice(&tkey);
        wrong_order.extend_from_slice(&epoch(1000).to_le_bytes());
        assert!(
            verify_strict_tagged(&ks.node_id(), NODE_TAG, &wrong_order, leg.node_sig()).is_err()
        );
        // Sanity: the spec order verifies.
        assert!(leg.verify_node_leg().is_ok());
    }

    #[test]
    fn cross_node_transplant_rejected() {
        // node_sig from node A's attestation placed into node B's attestation
        // (same other fields) MUST fail — the signature binds node_pubkey.
        let a = test_keyset();
        let b = second_keyset();
        assert_ne!(a.node_id(), b.node_id());

        let leg_a = NodeLeg::issue(&a, TID, fixed_transport_key(), epoch(1000));
        let leg_b = NodeLeg::issue(&b, TID, fixed_transport_key(), epoch(1000));

        // Transplant A's node_sig into B's leg.
        let transplanted = NodeLeg {
            node_sig: *leg_a.node_sig(),
            ..leg_b
        };
        assert!(transplanted.verify_node_leg().is_err());

        // And the reverse direction.
        let transplanted = NodeLeg {
            node_sig: *leg_b.node_sig(),
            ..leg_a
        };
        assert!(transplanted.verify_node_leg().is_err());

        // A full attestation carrying A's node_sig but B's node_pubkey field.
        let att_b = leg_b.complete(fixed_transport_sig());
        let mut wire = att_b.to_bytes();
        wire[74..138].copy_from_slice(leg_a.node_sig());
        let att = Attestation::from_bytes(&wire).unwrap();
        assert!(att.verify_node_leg().is_err());
    }

    #[test]
    fn wrong_tag_rejection() {
        // §6.1/§14: domain-tag confusion MUST fail. A signature over the exact
        // node-leg body made under another tag is not a valid node_sig.
        let ks = test_keyset();
        let body = node_sig_body(
            0x01,
            &ks.node_id(),
            &fixed_transport_key(),
            TID,
            epoch(1000),
        );

        let wrong = ks.sign_with_tag(b"111/announce/v1", &body);
        assert!(verify_strict_tagged(&ks.node_id(), NODE_TAG, &body, &wrong).is_err());

        // The module's own other tag is equally wrong for the node leg.
        let wrong = ks.sign_with_tag(TRANSPORT_TAG, &body);
        assert!(verify_strict_tagged(&ks.node_id(), NODE_TAG, &body, &wrong).is_err());

        // And a real node_sig does not verify under the transport tag.
        let leg = NodeLeg::issue(&ks, TID, fixed_transport_key(), epoch(1000));
        assert!(verify_strict_tagged(&ks.node_id(), TRANSPORT_TAG, &body, leg.node_sig()).is_err());
        // The bare body (tag stripped) is not the signed input either.
        assert!(verify_strict(&ks.node_id(), &body, leg.node_sig()).is_err());
    }

    // ------------------------------------------------------------------
    // §10 supersedure resolver + expiry ordering.
    // ------------------------------------------------------------------

    #[test]
    fn supersedure_greatest_expiry_wins() {
        let ks = test_keyset();
        let mk = |e: u64| {
            NodeLeg::issue(&ks, TID, fixed_transport_key(), epoch(e))
                .complete(fixed_transport_sig())
        };

        let mut set = AttestationSet::new();
        assert!(set.is_empty());

        // First-seen pins.
        assert_eq!(set.insert(mk(10)), Ok(Resolution::Pinned));
        assert_eq!(set.len(), 1);
        assert_eq!(
            set.get(&ks.node_id(), TID).unwrap().expiry_epoch(),
            epoch(10)
        );

        // Greater expiry supersedes; the retained one is replaced.
        assert_eq!(
            set.insert(mk(20)),
            Ok(Resolution::Supersedes {
                previous: epoch(10)
            })
        );
        assert_eq!(set.len(), 1);
        assert_eq!(
            set.get(&ks.node_id(), TID).unwrap().expiry_epoch(),
            epoch(20)
        );

        // Re-publication of the superseded one is rejected.
        assert_eq!(
            set.insert(mk(10)),
            Err(AttestError::Superseded {
                retained: epoch(20),
                offered: epoch(10),
            })
        );
        // State untouched by the rejection.
        assert_eq!(
            set.get(&ks.node_id(), TID).unwrap().expiry_epoch(),
            epoch(20)
        );

        // Byte-identical re-receipt at the retained epoch is a duplicate.
        assert_eq!(set.insert(mk(20)), Ok(Resolution::Duplicate));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn supersedure_equal_epoch_conflict_rejected() {
        let ks = test_keyset();
        let base = NodeLeg::issue(&ks, TID, fixed_transport_key(), epoch(10))
            .complete(fixed_transport_sig());
        let mut set = AttestationSet::new();
        assert_eq!(set.insert(base), Ok(Resolution::Pinned));

        // Same (node, transport_id, expiry_epoch) but different bytes — here
        // a different transport_key. Unresolvable by the spec's ordering;
        // rejected fail-closed, retained state unchanged.
        let conflict =
            NodeLeg::issue(&ks, TID, [0x99u8; 32], epoch(10)).complete(fixed_transport_sig());
        assert_eq!(
            set.insert(conflict),
            Err(AttestError::ConflictingAttestation { epoch: epoch(10) })
        );
        assert_eq!(set.get(&ks.node_id(), TID), Some(&base));
    }

    #[test]
    fn resolver_verifies_before_any_state_change() {
        let ks = test_keyset();
        let good = NodeLeg::issue(&ks, TID, fixed_transport_key(), epoch(10))
            .complete(fixed_transport_sig());
        let mut set = AttestationSet::new();
        assert_eq!(set.insert(good), Ok(Resolution::Pinned));

        // A forged "superseding" attestation (greater expiry, bad node_sig)
        // is rejected and changes nothing.
        let mut forged = NodeLeg::issue(&ks, TID, fixed_transport_key(), epoch(50))
            .complete(fixed_transport_sig())
            .to_bytes();
        forged[80] ^= 0x01; // corrupt node_sig
        let forged = Attestation::from_bytes(&forged).unwrap();
        assert!(matches!(
            set.insert(forged),
            Err(AttestError::InvalidNodeSignature(_))
        ));
        assert_eq!(set.get(&ks.node_id(), TID), Some(&good));

        // A forged first-seen attestation pins nothing.
        assert!(set.insert(forged).is_err());
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn expiry_semantics_ordering() {
        // §10 + §6.7: expiry_epoch is the first dead epoch; evaluated only
        // against an explicit epoch argument, never a clock. Attestation with
        // expiry 10: alive at 9, dead at 10 and after.
        let ks = test_keyset();
        let att = NodeLeg::issue(&ks, TID, fixed_transport_key(), epoch(10))
            .complete(fixed_transport_sig());
        assert!(!att.is_expired(epoch(0)));
        assert!(!att.is_expired(epoch(9)));
        assert!(att.is_expired(epoch(10)));
        assert!(att.is_expired(epoch(11)));
        assert!(att.is_expired(epoch(u64::MAX)));

        // LE encoding round-trip (the wire form is 8B LE mesh-epoch).
        assert_eq!(
            MeshEpoch::from_le_bytes(epoch(0x0102_0304_0506_0708).to_le_bytes()),
            epoch(0x0102_0304_0506_0708)
        );
        assert_eq!(
            epoch(0x0102_0304_0506_0708).to_le_bytes(),
            [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]
        );
    }

    // ------------------------------------------------------------------
    // The TransportVerifier seam (implementation boundary).
    // ------------------------------------------------------------------

    /// Test-only stand-in for L7's secp256k1 verifier: treats `transport_key`
    /// as an Ed25519 public key and strict-verifies. Exercises the seam
    /// end-to-end without k256 (not in the frozen dep set, §11).
    struct Ed25519MockVerifier;

    impl TransportVerifier for Ed25519MockVerifier {
        fn verify_transport_signature(
            &self,
            transport_id: TransportId,
            transport_key: &[u8; 32],
            signed_input: &[u8],
            signature: &[u8; 64],
        ) -> bool {
            if transport_id != TID {
                return false; // plugs fail closed on unimplemented schemes
            }
            verify_strict(&NodeId::from(*transport_key), signed_input, signature).is_ok()
        }
    }

    struct RejectAllVerifier;

    impl TransportVerifier for RejectAllVerifier {
        fn verify_transport_signature(
            &self,
            _t: TransportId,
            _k: &[u8; 32],
            _i: &[u8],
            _s: &[u8; 64],
        ) -> bool {
            false
        }
    }

    /// A "transport-side" signer for the mock: a second keyset acting as the
    /// transport credential holder (its pubkey plays `transport_key`).
    fn mock_transport_sig(node: &NodeId) -> ([u8; 32], [u8; 64]) {
        let transport_ks = second_keyset();
        let tkey = *transport_ks.public().sign_pubkey();
        let sig = transport_ks.sign_with_tag(TRANSPORT_TAG, node.as_bytes());
        (tkey, sig)
    }

    #[test]
    fn transport_verifier_seam_round_trip() {
        let ks = test_keyset();
        let (tkey, tsig) = mock_transport_sig(&ks.node_id());
        let att = NodeLeg::issue(&ks, TID, tkey, epoch(100)).complete(tsig);

        // Both legs verify; the transport input handed to the plug is the
        // canonical "111/attest-transport/v1" ‖ node_pubkey.
        assert!(att.verify_node_leg().is_ok());
        assert!(att.verify_transport_leg(&Ed25519MockVerifier).is_ok());
        assert!(att.verify_full(&Ed25519MockVerifier, epoch(99)).is_ok());

        // A tampered transport_sig fails the plug.
        let mut bad = att.to_bytes();
        bad[150] ^= 0x01;
        let bad = Attestation::from_bytes(&bad).unwrap();
        assert_eq!(
            bad.verify_transport_leg(&Ed25519MockVerifier),
            Err(AttestError::InvalidTransportSignature)
        );

        // A rejecting plug fails even a genuine signature (fail-closed seam).
        assert_eq!(
            att.verify_transport_leg(&RejectAllVerifier),
            Err(AttestError::InvalidTransportSignature)
        );

        // Full check with an expired epoch: both sigs valid, still dead.
        assert_eq!(
            att.verify_full(&Ed25519MockVerifier, epoch(100)),
            Err(AttestError::Expired {
                expiry_epoch: epoch(100),
                current: epoch(100),
            })
        );

        // The node_sig is NOT a valid transport_sig (domain separation across
        // legs): exchange the two 64-byte signature fields and both checks
        // must fail.
        let mut swapped = att.to_bytes();
        let (front, back) = swapped.split_at_mut(138);
        front[74..138].swap_with_slice(&mut back[..64]);
        let swapped = Attestation::from_bytes(&swapped).unwrap();
        assert!(swapped.verify_node_leg().is_err());
        assert!(swapped.verify_transport_leg(&Ed25519MockVerifier).is_err());
    }

    #[test]
    fn insert_with_transport_gates_on_both_sigs() {
        let ks = test_keyset();
        let (tkey, tsig) = mock_transport_sig(&ks.node_id());
        let att = NodeLeg::issue(&ks, TID, tkey, epoch(10)).complete(tsig);

        let mut set = AttestationSet::new();
        assert_eq!(
            set.insert_with_transport(att, &Ed25519MockVerifier),
            Ok(Resolution::Pinned)
        );

        // A node-valid supersedure whose transport_sig is garbage is refused
        // without a state change.
        let forged = NodeLeg::issue(&ks, TID, tkey, epoch(20)).complete([0x42u8; 64]);
        assert_eq!(
            set.insert_with_transport(forged, &Ed25519MockVerifier),
            Err(AttestError::InvalidTransportSignature)
        );
        assert_eq!(set.get(&ks.node_id(), TID), Some(&att));

        // The node-side-only insert (built-in coverage) accepts it: node_sig
        // alone proves the node authorized the binding (module header).
        assert_eq!(
            set.insert(forged),
            Ok(Resolution::Supersedes {
                previous: epoch(10)
            })
        );
    }

    #[test]
    fn supersedure_is_per_node_and_per_transport() {
        // Different nodes and different transport_ids are independent slots.
        let a = test_keyset();
        let b = second_keyset();
        let mut set = AttestationSet::new();
        let att_a = NodeLeg::issue(&a, TID, fixed_transport_key(), epoch(10))
            .complete(fixed_transport_sig());
        let att_b = NodeLeg::issue(&b, TID, fixed_transport_key(), epoch(5))
            .complete(fixed_transport_sig());
        assert_eq!(set.insert(att_a), Ok(Resolution::Pinned));
        assert_eq!(set.insert(att_b), Ok(Resolution::Pinned));
        assert_eq!(set.len(), 2);
        // B's smaller epoch is not superseded by A's greater one.
        assert_eq!(set.get(&b.node_id(), TID).unwrap().expiry_epoch(), epoch(5));
    }

    // ------------------------------------------------------------------
    // §14 KAT — printed for the T8 vector freeze.
    // ------------------------------------------------------------------

    #[test]
    fn attestation_kat_for_t8() {
        // Fixed inputs, fully deterministic (Ed25519 signing is deterministic,
        // RFC 8032): the frozen project node key + fixed transport bytes.
        let ks = test_keyset();
        let tkey = fixed_transport_key();
        let tsig = fixed_transport_sig();
        let expiry = epoch(0x1122_3344_5566_7788);
        let att = NodeLeg::issue(&ks, TID, tkey, expiry).complete(tsig);
        let wire = att.to_bytes();

        println!("Attestation KAT (frozen by T8 into os/20-identity/test-vectors/):");
        println!("  node mnemonic   = abandon×11 about (BIP-39, empty passphrase)");
        println!("  node_pubkey     = {}", to_hex(ks.node_id().as_bytes()));
        println!("  transport_id    = 0x01 (nostr-secp256k1)");
        println!("  transport_key   = {}", to_hex(&tkey));
        println!("  expiry_epoch    = 0x1122334455667788 (LE on wire)");
        println!("  transport_sig   = {} (opaque)", to_hex(&tsig));
        println!("  node_sig        = {}", to_hex(att.node_sig()));
        println!("  attestation hex = {}", to_hex(&wire));

        // Frozen known answers (computed by this test, pinned for T8).
        assert_eq!(
            ks.node_id().as_bytes(),
            &hex32("6f1d5331e36e16d341fa06c9f622cc8569710f2a62d7491e9be8e953df844f01")[..]
        );
        assert_eq!(to_hex(att.node_sig()), "b7ed757d585966ebe924715493daee606b6e69865784e49fd3de4c0d1cf55a36055b88ffb7c2a909f0aecebb5d3a4af4af58fec400eae78a5583564378b16303");
        assert_eq!(to_hex(&wire), "016f1d5331e36e16d341fa06c9f622cc8569710f2a62d7491e9be8e953df844f0101e0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff8877665544332211b7ed757d585966ebe924715493daee606b6e69865784e49fd3de4c0d1cf55a36055b88ffb7c2a909f0aecebb5d3a4af4af58fec400eae78a5583564378b16303a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf");
        assert!(att.verify_node_leg().is_ok());
        assert_eq!(Attestation::from_bytes(&wire).unwrap(), att);
    }
}
