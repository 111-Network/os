//! §6.2 — the announce format and verification rules (normative).
//!
//! Implements (owner: T4): the 137–169B announce —
//! `version(0x01) ‖ sign_pubkey(32=NodeId) ‖ enc_pubkey(32) ‖ counter(LE u64)
//! ‖ appdata(0–32) ‖ signature(64)` — with the ASCII domain tag
//! `"111/announce/v1"` prepended inside the signed region (never a wire field;
//! the verifier reconstructs it).
//!
//! Rules pinned here, in [`process_announce`]'s order: parse fail-closed
//! (length bound §12 rule 7; unknown version ⇒ reject, §12 rule 1; §3
//! low-order `enc_pubkey` ingress check); the signature MUST verify under the
//! §3 strict predicate BEFORE any state change; an announce carrying the
//! receiver's OWN NodeId ⇒ the distinct [`AnnounceError::IdentityCollision`]
//! state, never pinned (the §8.4 duplicate-instance case); then the pinning
//! store applies key-material immutability (§6.3) and the stale-counter rule
//! (a same-key stale announce is never applied; since L5 amendment A6 it is
//! reported as `PinOutcome::Unchanged`, not an error).
//!
//! The announce counter is the ANNOUNCE sequence — distinct from §6.5's
//! per-peer traffic counters — and is committed synchronously (fsync) BEFORE
//! the announce it authorizes is transmitted: callers take the counter from
//! [`crate::counters::CounterStore::next_announce_counter`] (which persists
//! before returning) and only then build/sign/transmit with
//! [`Announce::create`]. Restore-time resumption (floor + 1024) is §8.4,
//! implemented in `counters.rs`.
//!
//! This module handles PUBLIC material plus signatures only — no secret or
//! zeroize handling here.

use crate::keyset::{
    validate_x25519_public_key, verify_strict_tagged, Keyset, NodeId, VerifyError,
};
use crate::store::{PinOutcome, PinStore, StoreError};

/// §6.2 — the ASCII domain tag prepended inside the signed region (part of
/// the signed payload, reconstructed by the verifier; never a wire field).
pub const ANNOUNCE_TAG: &[u8] = b"111/announce/v1";

/// §6.2 — the only version byte v1 emits; unknown versions fail closed.
pub const VERSION_V1: u8 = 0x01;

/// §6.2 — base size with empty appdata (1 + 32 + 32 + 8 + 0 + 64).
pub const BASE_LEN: usize = 137;

/// §6.2 — max size with full 32B appdata.
pub const MAX_LEN: usize = 169;

/// §6.2 — appdata is 0–32B, opaque to this layer (reserved for L6/7 role
/// flags / transport hints; transport-MUTABLE data never rides inside this
/// signed blob, §13).
pub const APPDATA_MAX: usize = 32;

/// Signed-region ("head") length without appdata: version‖sign‖enc‖counter.
const HEAD_LEN: usize = 1 + 32 + 32 + 8;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Announce processing errors. Every variant is a rejection: the pinning
/// store is never mutated on the error paths (§6.2: the signature verifies
/// BEFORE any state change, and stale/conflict/collision checks only read).
#[derive(Debug)]
pub enum AnnounceError {
    /// §12 rule 7 — wire length outside 137–169B.
    MalformedLength { got: usize },
    /// §6.2/§12 rule 1 — unknown version ⇒ reject (fail closed).
    UnsupportedVersion(u8),
    /// §6.2/§3 — `enc_pubkey` failed the low-order ingress check (RFC 7748
    /// §6.1).
    LowOrderEncKey,
    /// §6.2 — the strict-predicate signature check failed; nothing was
    /// applied.
    InvalidSignature(VerifyError),
    /// §6.2 — the announce carries the receiver's OWN NodeId: never pinned;
    /// the distinct identity-collision error state (surfaced to the
    /// dashboard/`/api/status`, §13; the §8.4 duplicate-instance case).
    IdentityCollision,
    /// §6.2 — the counter did not exceed the last-seen value (stale ⇒ drop).
    /// Not produced by [`process_announce`] since L5 amendment A6 (see
    /// [`crate::store::StoreError::StaleCounter`]); retained for the L2
    /// vocabulary.
    StaleCounter { last_seen: u64, got: u64 },
    /// §6.3/§12 rule 3 — conflicting key material for a pinned NodeId.
    KeyConflict,
    /// §6.3 store failure (persist/parse).
    Store(StoreError),
    /// Construction error: appdata over the 32B cap.
    AppdataTooLong { got: usize },
}

impl std::fmt::Display for AnnounceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnnounceError::MalformedLength { got } => {
                write!(
                    f,
                    "§6.2: announce length {got} outside {BASE_LEN}–{MAX_LEN}"
                )
            }
            AnnounceError::UnsupportedVersion(v) => {
                write!(f, "§6.2: unknown announce version 0x{v:02x} (fail closed)")
            }
            AnnounceError::LowOrderEncKey => {
                write!(f, "§3: low-order X25519 enc_pubkey rejected on ingress")
            }
            AnnounceError::InvalidSignature(e) => write!(f, "{e}"),
            AnnounceError::IdentityCollision => write!(
                f,
                "§6.2: announce carries this node's OWN NodeId — identity collision (§8.4)"
            ),
            AnnounceError::StaleCounter { last_seen, got } => write!(
                f,
                "§6.2: stale announce counter {got} (last-seen {last_seen}) — dropped"
            ),
            AnnounceError::KeyConflict => {
                write!(f, "§6.3: conflicting key material for a pinned NodeId")
            }
            AnnounceError::Store(e) => write!(f, "{e}"),
            AnnounceError::AppdataTooLong { got } => {
                write!(f, "§6.2: appdata {got}B exceeds {APPDATA_MAX}B")
            }
        }
    }
}

impl std::error::Error for AnnounceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AnnounceError::InvalidSignature(e) => Some(e),
            AnnounceError::Store(e) => Some(e),
            _ => None,
        }
    }
}

impl From<StoreError> for AnnounceError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::StaleCounter { last_seen, got } => {
                AnnounceError::StaleCounter { last_seen, got }
            }
            StoreError::KeyConflict => AnnounceError::KeyConflict,
            other => AnnounceError::Store(other),
        }
    }
}

// ---------------------------------------------------------------------------
// The announce
// ---------------------------------------------------------------------------

/// §6.2 — a parsed or freshly built announce. Fields are the wire fields;
/// the domain tag is implicit (reconstructed at verification, §6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announce {
    node_id: NodeId,
    enc_pubkey: [u8; 32],
    counter: u64,
    appdata: Vec<u8>,
    signature: [u8; 64],
}

/// The signed wire bytes (everything preceding the signature field):
/// `version ‖ sign_pubkey ‖ enc_pubkey ‖ counter(LE) ‖ appdata`.
fn head_bytes(sign: &[u8; 32], enc: &[u8; 32], counter: u64, appdata: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEAD_LEN + appdata.len());
    out.push(VERSION_V1);
    out.extend_from_slice(sign);
    out.extend_from_slice(enc);
    out.extend_from_slice(&counter.to_le_bytes());
    out.extend_from_slice(appdata);
    out
}

impl Announce {
    /// §6.2 — build and sign an announce with this node's keyset.
    ///
    /// The `counter` MUST come from
    /// [`crate::counters::CounterStore::next_announce_counter`], which
    /// commits it (fsync) before returning — the §6.2 sync-commit rule: the
    /// counter is durable BEFORE the announce it authorizes is transmitted.
    /// This constructor never invents a counter value.
    pub fn create(
        keyset: &Keyset,
        counter: u64,
        appdata: &[u8],
    ) -> Result<Announce, AnnounceError> {
        if appdata.len() > APPDATA_MAX {
            return Err(AnnounceError::AppdataTooLong { got: appdata.len() });
        }
        let pubks = keyset.public();
        let head = head_bytes(pubks.sign_pubkey(), pubks.enc_pubkey(), counter, appdata);
        let signature = keyset.sign_with_tag(ANNOUNCE_TAG, &head);
        Ok(Announce {
            node_id: pubks.node_id(),
            enc_pubkey: *pubks.enc_pubkey(),
            counter,
            appdata: appdata.to_vec(),
            signature,
        })
    }

    /// §6.2 — parse wire bytes. Fail-closed: length within 137–169B (§12
    /// rule 7), version `0x01` only (§12 rule 1), and the `enc_pubkey` §3
    /// low-order ingress check. Does NOT verify the signature — that is
    /// [`Self::verify`], and [`process_announce`] orders verify before any
    /// state change.
    pub fn parse(wire: &[u8]) -> Result<Announce, AnnounceError> {
        if !(BASE_LEN..=MAX_LEN).contains(&wire.len()) {
            return Err(AnnounceError::MalformedLength { got: wire.len() });
        }
        if wire[0] != VERSION_V1 {
            return Err(AnnounceError::UnsupportedVersion(wire[0]));
        }
        let node_id = NodeId::from(<[u8; 32]>::try_from(&wire[1..33]).expect("32-byte slice"));
        let enc_pubkey = <[u8; 32]>::try_from(&wire[33..65]).expect("32-byte slice");
        // §3 (normative): X25519 low-order public keys MUST be rejected on
        // ingress — here, before the bytes go anywhere near a pin or ECDH.
        validate_x25519_public_key(&enc_pubkey).map_err(|_| AnnounceError::LowOrderEncKey)?;
        let counter = u64::from_le_bytes(wire[65..73].try_into().expect("8-byte slice"));
        let sig_off = wire.len() - 64;
        let appdata = wire[HEAD_LEN..sig_off].to_vec();
        let signature = <[u8; 64]>::try_from(&wire[sig_off..]).expect("64-byte slice");
        Ok(Announce {
            node_id,
            enc_pubkey,
            counter,
            appdata,
            signature,
        })
    }

    /// The wire encoding: head ‖ signature (137–169B).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.head();
        out.extend_from_slice(&self.signature);
        out
    }

    /// The signed wire bytes (the head: all fields preceding the signature).
    fn head(&self) -> Vec<u8> {
        head_bytes(
            self.node_id.as_bytes(),
            &self.enc_pubkey,
            self.counter,
            &self.appdata,
        )
    }

    /// §6.2 — verify the signature under the §3 strict predicate over
    /// `"111/announce/v1" ‖ head`. This MUST pass before any state change;
    /// [`process_announce`] enforces the ordering.
    pub fn verify(&self) -> Result<(), VerifyError> {
        verify_strict_tagged(&self.node_id, ANNOUNCE_TAG, &self.head(), &self.signature)
    }

    /// The announcing node's NodeId (B2: it IS the `sign_pubkey` field).
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// The announced X25519 public key (§3 low-order-checked at parse).
    pub fn enc_pubkey(&self) -> &[u8; 32] {
        &self.enc_pubkey
    }

    /// The announce-sequence counter (§6.2; NOT §6.5's traffic counters).
    pub fn counter(&self) -> u64 {
        self.counter
    }

    /// The opaque 0–32B appdata (reserved for L6/7, §6.2).
    pub fn appdata(&self) -> &[u8] {
        &self.appdata
    }

    /// The 64B Ed25519 signature over the tag ‖ head.
    pub fn signature(&self) -> &[u8; 64] {
        &self.signature
    }
}

/// §6.2 — the announce ingress path, in the normative order:
///
/// 1. parse — length bound, version fail-closed, §3 low-order enc ingress;
/// 2. the signature MUST verify under the §3 strict predicate BEFORE any
///    state change (real nostr clients were caught skipping verification —
///    EuroS&P 2025);
/// 3. self-NodeId ⇒ [`AnnounceError::IdentityCollision`] — NEVER pinned;
///    checked after verification so a forged garbage-signature "collision"
///    cannot surface a false collision state;
/// 4. the pinning store applies the remaining rules atomically with the
///    mutation: conflicting key material ⇒ reject at any counter (§6.3);
///    identical key material at a counter ≤ last-seen ⇒
///    `Ok(PinOutcome::Unchanged)`, no state change (L5 amendment A6 — the
///    stale announce is still never applied, it is just not an error); else
///    first-seen pin / counter update.
pub fn process_announce(
    store: &mut PinStore,
    own_node_id: &NodeId,
    wire: &[u8],
) -> Result<PinOutcome, AnnounceError> {
    let announce = Announce::parse(wire)?;
    announce.verify().map_err(AnnounceError::InvalidSignature)?;
    if &announce.node_id == own_node_id {
        return Err(AnnounceError::IdentityCollision);
    }
    Ok(store.apply_announce(announce.node_id, announce.enc_pubkey, announce.counter)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Minimal unique tempdir (deps are frozen — no tempfile crate).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "id111-t4-announce-{tag}-{}-{seq}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

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

    /// The Ed25519 group order ℓ, little-endian (test-local mirror of the
    /// keyset.rs constant, used to build the S + ℓ malleability negative).
    const GROUP_ORDER_L_LE: [u8; 32] = [
        0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde,
        0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x10,
    ];

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

    // Two fixed keysets from public test material: RFC 8032 signing seeds +
    // RFC 7748 X25519 scalars (Alice/Bob).
    const SIGN_SEED_A: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    const SIGN_SEED_B: &str = "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb";
    const ALICE_SCALAR: [u8; 32] = [
        0x77, 0x07, 0x6d, 0x0a, 0x73, 0x18, 0xa5, 0x7d, 0x3c, 0x16, 0xc1, 0x72, 0x51, 0xb2, 0x66,
        0x45, 0xdf, 0x4c, 0x2f, 0x87, 0xeb, 0xc0, 0x99, 0x2a, 0xb1, 0x77, 0xfb, 0xa5, 0x1d, 0xb9,
        0x2c, 0x2a,
    ];
    const BOB_SCALAR: [u8; 32] = [
        0x5d, 0xab, 0x08, 0x7e, 0x62, 0x4a, 0x8a, 0x4b, 0x79, 0xe1, 0x7f, 0x8b, 0x83, 0x80, 0x0e,
        0xe6, 0x6f, 0x3b, 0xb1, 0x29, 0x26, 0x18, 0xb6, 0xfd, 0x1c, 0x2f, 0x8b, 0x27, 0xff, 0x88,
        0xe0, 0xeb,
    ];

    fn keyset(sign_seed_hex: &str, scalar: [u8; 32]) -> Keyset {
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&hex32(sign_seed_hex));
        bytes[32..].copy_from_slice(&scalar);
        Keyset::from_bytes(&bytes).unwrap()
    }

    fn keyset_a() -> Keyset {
        keyset(SIGN_SEED_A, ALICE_SCALAR)
    }

    fn keyset_b() -> Keyset {
        keyset(SIGN_SEED_B, BOB_SCALAR)
    }

    /// Hand-assemble a wire announce from explicit fields, signed under an
    /// arbitrary tag — the crafting tool for the negative tests.
    fn craft_wire(
        signer: &Keyset,
        sign_pub: &[u8; 32],
        enc: &[u8; 32],
        counter: u64,
        appdata: &[u8],
        tag: &[u8],
    ) -> Vec<u8> {
        let mut head = Vec::new();
        head.push(VERSION_V1);
        head.extend_from_slice(sign_pub);
        head.extend_from_slice(enc);
        head.extend_from_slice(&counter.to_le_bytes());
        head.extend_from_slice(appdata);
        let sig = signer.sign_with_tag(tag, &head);
        head.extend_from_slice(&sig);
        head
    }

    // ------------------------------------------------------------------
    // Round-trip + the KAT printed for T8's vector freeze (§14).
    // ------------------------------------------------------------------

    #[test]
    fn announce_round_trip_and_kat() {
        // The project vector keyset (the same mnemonic keyset.rs freezes).
        let ks = Keyset::derive_from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let ann = Announce::create(&ks, 1, b"111net").unwrap();
        let wire = ann.encode();
        assert_eq!(wire.len(), BASE_LEN + 6);
        assert_eq!(Announce::parse(&wire).unwrap(), ann);
        assert!(ann.verify().is_ok());
        // Fixed point: the NodeId matches keyset.rs's frozen project vector.
        assert_eq!(
            ks.node_id(),
            NodeId::from(hex32(
                "6f1d5331e36e16d341fa06c9f622cc8569710f2a62d7491e9be8e953df844f01"
            ))
        );

        // §14 vector — PRINTED for the T8 freeze (run with --nocapture).
        println!("Announce KAT (frozen by T8):");
        println!("  mnemonic   = abandon×11 about (BIP-39, empty passphrase)");
        println!("  NodeId     = {}", to_hex(ann.node_id().as_bytes()));
        println!("  enc_pubkey = {}", to_hex(ann.enc_pubkey()));
        println!("  counter    = 1 (genesis announce, LE u64 on the wire)");
        println!("  appdata    = {}", to_hex(b"111net"));
        println!("  signed input = \"111/announce/v1\" ‖ wire[..len-64] (tag not on the wire)");
        println!("  signature  = {}", to_hex(ann.signature()));
        println!("  wire(143B) = {}", to_hex(&wire));
    }

    #[test]
    fn process_round_trip_pin_then_update() {
        let tmp = TempDir::new("process");
        let mut store = PinStore::open(tmp.path()).unwrap();
        let a = keyset_a();
        let own = keyset_b().node_id();

        let wire1 = Announce::create(&a, 1, b"").unwrap().encode();
        assert_eq!(wire1.len(), BASE_LEN);
        assert_eq!(
            process_announce(&mut store, &own, &wire1).unwrap(),
            PinOutcome::Pinned { evicted: None }
        );
        let e = *store.get(&a.node_id()).unwrap();
        assert_eq!(e.enc_pubkey(), a.public().enc_pubkey());
        assert_eq!(e.highest_announce_counter(), 1);

        let wire2 = Announce::create(&a, 2, b"role=relay;hint=lora")
            .unwrap()
            .encode();
        assert_eq!(
            process_announce(&mut store, &own, &wire2).unwrap(),
            PinOutcome::CounterUpdated { previous: 1 }
        );
        assert_eq!(
            store.get(&a.node_id()).unwrap().highest_announce_counter(),
            2
        );
    }

    #[test]
    fn appdata_bounds() {
        let a = keyset_a();
        let full = Announce::create(&a, 1, &[0x5au8; 32]).unwrap();
        assert_eq!(full.encode().len(), MAX_LEN);
        assert!(matches!(
            Announce::create(&a, 1, &[0x5au8; 33]),
            Err(AnnounceError::AppdataTooLong { got: 33 })
        ));
    }

    // ------------------------------------------------------------------
    // §14 negatives: stale counter, conflicting key, self-NodeId, domain-tag
    // confusion, strict-predicate edges, version/length fail-closed.
    // ------------------------------------------------------------------

    #[test]
    fn stale_counter_dropped() {
        let tmp = TempDir::new("stale");
        let mut store = PinStore::open(tmp.path()).unwrap();
        let a = keyset_a();
        let own = keyset_b().node_id();
        let wire1 = Announce::create(&a, 5, b"").unwrap().encode();
        process_announce(&mut store, &own, &wire1).unwrap();
        // Replay the exact same announce: counter 5 ≤ last-seen 5 ⇒ never
        // applied. Since L5 amendment A6 (identical key material) this is
        // Ok(Unchanged), not an error.
        assert_eq!(
            process_announce(&mut store, &own, &wire1).unwrap(),
            PinOutcome::Unchanged { last_seen: 5 }
        );
        // An older counter too.
        let wire0 = Announce::create(&a, 2, b"").unwrap().encode();
        assert_eq!(
            process_announce(&mut store, &own, &wire0).unwrap(),
            PinOutcome::Unchanged { last_seen: 5 }
        );
        // Nothing changed: Unchanged performs no state change — the counter
        // and the pinned key material are exactly as before.
        let e = store.get(&a.node_id()).unwrap();
        assert_eq!(e.highest_announce_counter(), 5);
        assert_eq!(e.enc_pubkey(), a.public().enc_pubkey());
    }

    #[test]
    fn conflicting_key_material_rejected() {
        let tmp = TempDir::new("conflict");
        let mut store = PinStore::open(tmp.path()).unwrap();
        let a = keyset_a();
        let b = keyset_b();
        let own = b.node_id();
        let wire1 = Announce::create(&a, 1, b"").unwrap().encode();
        process_announce(&mut store, &own, &wire1).unwrap();

        // A's NodeId but B's enc_pubkey — validly SIGNED by A (a compromised
        // or confused holder); §6.3/§12 rule 3 rejects it regardless.
        let evil = craft_wire(
            &a,
            a.public().sign_pubkey(),
            b.public().enc_pubkey(),
            2,
            b"",
            ANNOUNCE_TAG,
        );
        assert!(matches!(
            process_announce(&mut store, &own, &evil),
            Err(AnnounceError::KeyConflict)
        ));
        // The held pin is untouched.
        let e = store.get(&a.node_id()).unwrap();
        assert_eq!(e.enc_pubkey(), a.public().enc_pubkey());
        assert_eq!(e.highest_announce_counter(), 1);
    }

    #[test]
    fn self_node_id_is_identity_collision_never_pinned() {
        let tmp = TempDir::new("collision");
        let mut store = PinStore::open(tmp.path()).unwrap();
        let a = keyset_a();
        // The receiver's OWN NodeId: two live instances on one mnemonic (the
        // §8.4 duplicate case). The signature is VALID — the duplicate holds
        // the same key — yet the announce is never pinned and the distinct
        // collision error surfaces.
        let own = a.node_id();
        let wire = Announce::create(&a, 1, b"").unwrap().encode();
        assert!(matches!(
            process_announce(&mut store, &own, &wire),
            Err(AnnounceError::IdentityCollision)
        ));
        assert!(store.is_empty());

        // A FORGED self-NodeId announce (invalid signature) surfaces as a
        // signature failure, not a false collision: verification precedes the
        // collision check.
        let mut forged = wire.clone();
        forged[10] ^= 1; // corrupt the signed region
        assert!(matches!(
            process_announce(&mut store, &own, &forged),
            Err(AnnounceError::InvalidSignature(_))
        ));
        assert!(store.is_empty());
    }

    #[test]
    fn domain_tag_confusion_fails() {
        let tmp = TempDir::new("tagconf");
        let mut store = PinStore::open(tmp.path()).unwrap();
        let a = keyset_a();
        let own = keyset_b().node_id();
        // §14 negative: an artifact signed under a DIFFERENT tag MUST fail —
        // same bytes, `"111/attest-node/v1"` in place of `"111/announce/v1"`.
        let confused = craft_wire(
            &a,
            a.public().sign_pubkey(),
            a.public().enc_pubkey(),
            1,
            b"",
            b"111/attest-node/v1",
        );
        assert!(matches!(
            process_announce(&mut store, &own, &confused),
            Err(AnnounceError::InvalidSignature(_))
        ));
        assert!(store.is_empty());
        // And the tag is genuinely inside the signed region: a correctly
        // tagged announce verifies, the same signature checked without the
        // tag does not.
        let good = Announce::create(&a, 1, b"").unwrap();
        assert!(good.verify().is_ok());
        let head = good.encode();
        let head = &head[..head.len() - 64];
        assert!(crate::keyset::verify_strict(&good.node_id(), head, good.signature()).is_err());
    }

    #[test]
    fn non_strict_edge_signature_rejected() {
        let tmp = TempDir::new("malleable");
        let mut store = PinStore::open(tmp.path()).unwrap();
        let a = keyset_a();
        let own = keyset_b().node_id();
        let mut wire = Announce::create(&a, 1, b"").unwrap().encode();
        // Malleate S → S + ℓ: the same integer mod ℓ, a non-canonical
        // encoding — accepted by non-strict verifiers, rejected by §3.
        let sig_off = wire.len() - 64;
        let s: [u8; 32] = wire[sig_off + 32..].try_into().unwrap();
        wire[sig_off + 32..].copy_from_slice(&le_add(&s, &GROUP_ORDER_L_LE));
        assert!(matches!(
            process_announce(&mut store, &own, &wire),
            Err(AnnounceError::InvalidSignature(VerifyError::NonCanonicalS))
        ));
        assert!(store.is_empty());
    }

    #[test]
    fn unknown_version_fails_closed_even_when_signed() {
        let tmp = TempDir::new("version");
        let mut store = PinStore::open(tmp.path()).unwrap();
        let a = keyset_a();
        let own = keyset_b().node_id();
        // A "version 2" announce, validly signed under the announce tag —
        // parse rejects it before the signature is even considered (§12 rule
        // 1: no silent downgrade).
        let mut v2 = craft_wire(
            &a,
            a.public().sign_pubkey(),
            a.public().enc_pubkey(),
            1,
            b"",
            ANNOUNCE_TAG,
        );
        v2[0] = 0x02;
        // Re-sign so the signature IS valid for the v2 bytes.
        let head = &v2[..v2.len() - 64];
        let sig = a.sign_with_tag(ANNOUNCE_TAG, head);
        v2.truncate(head.len());
        v2.extend_from_slice(&sig);
        assert!(matches!(
            process_announce(&mut store, &own, &v2),
            Err(AnnounceError::UnsupportedVersion(0x02))
        ));
        assert!(store.is_empty());
    }

    #[test]
    fn length_bounds_fail_closed() {
        let a = keyset_a();
        let wire = Announce::create(&a, 1, b"").unwrap().encode();
        // 136 < 137 and 170 > 169 are both rejected (§12 rule 7).
        assert!(matches!(
            Announce::parse(&wire[..BASE_LEN - 1]),
            Err(AnnounceError::MalformedLength { got: 136 })
        ));
        let mut long = Announce::create(&a, 1, &[0u8; 32]).unwrap().encode();
        long.push(0x00);
        assert!(matches!(
            Announce::parse(&long),
            Err(AnnounceError::MalformedLength { got: 170 })
        ));
    }

    #[test]
    fn low_order_enc_pubkey_rejected_at_ingress() {
        let a = keyset_a();
        // enc_pubkey = all-zero (RFC 7748 §6.1 low-order), the rest valid and
        // validly signed — parse rejects it BEFORE the signature check.
        let wire = craft_wire(
            &a,
            a.public().sign_pubkey(),
            &[0u8; 32],
            1,
            b"",
            ANNOUNCE_TAG,
        );
        assert!(matches!(
            Announce::parse(&wire),
            Err(AnnounceError::LowOrderEncKey)
        ));
    }

    #[test]
    fn bad_signature_changes_no_state() {
        let tmp = TempDir::new("nostate");
        let mut store = PinStore::open(tmp.path()).unwrap();
        let a = keyset_a();
        let own = keyset_b().node_id();
        let wire1 = Announce::create(&a, 1, b"").unwrap().encode();
        process_announce(&mut store, &own, &wire1).unwrap();
        // A higher-counter announce with a corrupted signature MUST NOT move
        // the stored counter (§6.2: verification BEFORE any state change).
        let mut bad = Announce::create(&a, 99, b"").unwrap().encode();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(matches!(
            process_announce(&mut store, &own, &bad),
            Err(AnnounceError::InvalidSignature(_))
        ));
        assert_eq!(
            store.get(&a.node_id()).unwrap().highest_announce_counter(),
            1
        );
    }
}
