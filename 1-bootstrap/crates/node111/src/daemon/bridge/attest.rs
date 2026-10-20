//! The bridge-identity attestation plug (Layer 6 `os/60-transport/README.md` §14; Layer 2
//! amendment A10, `os/20-identity/README.md` §13.1; D26 #3): the first implementor of
//! `id111`'s [`TransportVerifier`] seam.
//!
//! `id111` owns the 202 B §10 record, both signed inputs and the registry bytes
//! (`0x02` meshtastic, `0x03` reticulum); this module owns the transport leg for them:
//!
//! * **`0x03` reticulum — both legs.** `transport_key` = SHA-256 of the identity's 64 B
//!   public key (`X25519 ‖ Ed25519`). The 64 B key is **carried, never derived**: the
//!   LXMF helper reports it (`ready.pubkey`, §6) and the adapter hands it in with
//!   [`BridgeVerifier::add_reticulum_key`]. Verify = some known key hashes to
//!   `transport_key`, then the §3 strict Ed25519 predicate (`id111::keyset::verify_strict`)
//!   over the id111-assembled input `"111/attest-transport/v1" ‖ node_pubkey` under the
//!   key's second 32 B. No Reticulum arithmetic here (no address or destination-hash
//!   derivation; §14 "carried and compared, never derived").
//! * **`0x02` meshtastic — one-sided.** A 2.7.x board cannot sign (X25519 only), so its
//!   record carries 64 zero bytes and rests on `node_sig` alone. [`verify_attestation`]
//!   accepts exactly that as [`Leg::OneSided`] and says so. A one-sided record **never**
//!   yields `attested-111` — anyone can write a record naming a victim's board key — so
//!   this module exposes the fact, never the label (the label is the second release,
//!   §17). A `0x02` record with a non-zero `transport_sig` is **rejected**: no 2.8
//!   XEdDSA verifier exists in v1, and an unverifiable signature is not one-sided.
//!   Through the bare [`TransportVerifier`] trait `0x02` is always `false`: an all-zero
//!   field is not a signature, so id111's two-signature paths (`verify_full`,
//!   `AttestationSet::insert_with_transport`) can never count it as bidirectional.
//! * **every other `transport_id` ⇒ `false`** (fail closed): nostr `0x01` is Layer 7's.
//!
//! The table is small and in memory (one bridge identity per foreign network, §14), filled
//! only while an adapter runs; nothing here polls, persists or listens.
//!
//! [`verify_attestation`]: BridgeVerifier::verify_attestation

use std::fmt;

use id111::attest::{
    reticulum_transport_key, AttestError, Attestation, MeshEpoch, TransportId, TransportVerifier,
    MESHTASTIC_UNSIGNED,
};
use id111::keyset::{verify_strict, NodeId, VerifyError};

/// §14: one bridge identity per foreign network; a few helpers' identities at most. The
/// cap bounds a table fed from a child process's reports.
pub const MAX_RETICULUM_KEYS: usize = 16;

/// Which legs of a §10 record verified (§14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leg {
    /// `node_sig` and `transport_sig` both verified (`0x03` reticulum).
    Both,
    /// `node_sig` only; `transport_sig` is the 64 zero bytes of a 2.7.x Meshtastic board
    /// (`0x02`). Never `attested-111`, not in this release and not later (§14).
    OneSided,
}

/// An accepted record (§14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verified {
    pub leg: Leg,
}

/// Why a record was rejected (§14; every variant fails closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// The node leg failed the §3 strict predicate (checked first, for every id).
    NodeSignature(VerifyError),
    /// `0x03`: no known 64 B key hashes to `transport_key`.
    UnknownReticulumKey,
    /// `0x03`: `transport_sig` failed the strict Ed25519 predicate.
    TransportSignature,
    /// `0x02` with a non-zero `transport_sig`: no 2.8 XEdDSA verifier in v1.
    MeshtasticSigned,
    /// A `transport_id` this plug does not implement (nostr `0x01` is Layer 7's).
    Unsupported(TransportId),
    /// Dead at the caller-supplied mesh-epoch (`current >= expiry_epoch`, L2 §10).
    Expired {
        expiry_epoch: MeshEpoch,
        current: MeshEpoch,
    },
}

impl fmt::Display for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rejected::NodeSignature(e) => write!(f, "attestation node_sig: {e}"),
            Rejected::UnknownReticulumKey => {
                write!(f, "reticulum transport_key matches no known identity key")
            }
            Rejected::TransportSignature => write!(f, "reticulum transport_sig does not verify"),
            Rejected::MeshtasticSigned => write!(
                f,
                "meshtastic record carries a signature; no verifier for it exists in v1"
            ),
            Rejected::Unsupported(t) => write!(f, "transport {t} is not verified by the bridge"),
            Rejected::Expired {
                expiry_epoch,
                current,
            } => write!(f, "attestation dead at {current} (expiry {expiry_epoch})"),
        }
    }
}

impl std::error::Error for Rejected {}

/// The 64 B key table is full ([`MAX_RETICULUM_KEYS`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyTableFull;

impl fmt::Display for KeyTableFull {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "reticulum key table full ({MAX_RETICULUM_KEYS})")
    }
}

impl std::error::Error for KeyTableFull {}

/// The §14 plug: id111's [`TransportVerifier`] for `0x02` / `0x03`.
#[derive(Debug, Default, Clone)]
pub struct BridgeVerifier {
    /// Known Reticulum public keys, `X25519(32) ‖ Ed25519(32)`, as carried.
    reticulum: Vec<[u8; 64]>,
}

impl BridgeVerifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remember a Reticulum identity's 64 B public key as the helper reported it
    /// (`ready.pubkey`, §6). Idempotent; bounded by [`MAX_RETICULUM_KEYS`].
    pub fn add_reticulum_key(&mut self, pubkey64: [u8; 64]) -> Result<(), KeyTableFull> {
        if self.reticulum.contains(&pubkey64) {
            return Ok(());
        }
        if self.reticulum.len() >= MAX_RETICULUM_KEYS {
            return Err(KeyTableFull);
        }
        self.reticulum.push(pubkey64);
        Ok(())
    }

    /// The known 64 B key whose SHA-256 is `transport_key` (A10 hash binding), if any.
    pub fn reticulum_key_for(&self, transport_key: &[u8; 32]) -> Option<&[u8; 64]> {
        self.reticulum
            .iter()
            .find(|k| reticulum_transport_key(k) == *transport_key)
    }

    /// §14: verify a parsed §10 record and say which legs hold. The node leg first (id111's
    /// strict predicate), then the transport leg per `transport_id`, then expiry.
    ///
    /// `now` is a mesh-epoch from Layer 7's source; v1 has none, so callers pass `None` and
    /// expiry is stored, never evaluated (L2 §6.7) — never a wall clock.
    pub fn verify_attestation(
        &self,
        att: &Attestation,
        now: Option<MeshEpoch>,
    ) -> Result<Verified, Rejected> {
        att.verify_node_leg().map_err(|e| match e {
            AttestError::InvalidNodeSignature(v) => Rejected::NodeSignature(v),
            // verify_node_leg yields no other variant; refuse anyway.
            _ => Rejected::NodeSignature(VerifyError::InvalidSignature),
        })?;
        let leg = match att.transport_id() {
            TransportId::Reticulum => {
                if self.reticulum_key_for(att.transport_key()).is_none() {
                    return Err(Rejected::UnknownReticulumKey);
                }
                att.verify_transport_leg(self)
                    .map_err(|_| Rejected::TransportSignature)?;
                Leg::Both
            }
            TransportId::Meshtastic => {
                if *att.transport_sig() != MESHTASTIC_UNSIGNED {
                    return Err(Rejected::MeshtasticSigned);
                }
                Leg::OneSided
            }
            other => return Err(Rejected::Unsupported(other)),
        };
        if let Some(current) = now {
            if att.is_expired(current) {
                return Err(Rejected::Expired {
                    expiry_epoch: att.expiry_epoch(),
                    current,
                });
            }
        }
        Ok(Verified { leg })
    }
}

impl TransportVerifier for BridgeVerifier {
    /// `true` iff `0x03` and a known key hashes to `transport_key` and `signature` passes
    /// the §3 strict predicate over `signed_input` (assembled by id111, never re-serialized
    /// here) under that key's Ed25519 half. `0x02` and every other id ⇒ `false`.
    fn verify_transport_signature(
        &self,
        transport_id: TransportId,
        transport_key: &[u8; 32],
        signed_input: &[u8],
        signature: &[u8; 64],
    ) -> bool {
        if transport_id != TransportId::Reticulum {
            return false;
        }
        let Some(key) = self.reticulum_key_for(transport_key) else {
            return false;
        };
        let mut ed = [0u8; 32];
        ed.copy_from_slice(&key[32..]);
        verify_strict(&NodeId::from(ed), signed_input, signature).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use id111::attest::{AttestationSet, NodeLeg};
    use serde_json::Value;

    const LAYER6: &str =
        include_str!("../../../../../../20-identity/test-vectors/layer6/attest-transport-ids.json");
    const L2_ATTEST: &str =
        include_str!("../../../../../../20-identity/test-vectors/attestation.json");

    fn unhex(s: &str) -> Vec<u8> {
        assert!(s.len().is_multiple_of(2));
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    fn vectors() -> Value {
        serde_json::from_str(LAYER6).unwrap()
    }

    fn pubkey64(v: &Value) -> [u8; 64] {
        unhex(v["reticulum"]["pubkey64"].as_str().unwrap())
            .try_into()
            .unwrap()
    }

    fn plug() -> BridgeVerifier {
        let mut p = BridgeVerifier::new();
        p.add_reticulum_key(pubkey64(&vectors())).unwrap();
        p
    }

    fn tok(r: Result<Verified, Rejected>) -> String {
        match r {
            Ok(Verified { leg: Leg::Both }) => "ok:both".into(),
            Ok(Verified { leg: Leg::OneSided }) => "ok:one-sided".into(),
            Err(Rejected::NodeSignature(e)) => format!("err:node-sig:{e:?}"),
            Err(Rejected::UnknownReticulumKey) => "err:unknown-key".into(),
            Err(Rejected::TransportSignature) => "err:transport-sig".into(),
            Err(Rejected::MeshtasticSigned) => "err:meshtastic-signed".into(),
            Err(Rejected::Unsupported(t)) => format!("err:unsupported:{}", t.to_byte()),
            Err(Rejected::Expired { .. }) => "err:expired".into(),
        }
    }

    fn case(name: &str) -> Attestation {
        let v = vectors();
        let c = v["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("no case {name}"));
        Attestation::from_bytes(&unhex(c["wire"].as_str().unwrap())).unwrap()
    }

    #[test]
    fn frozen_vectors_round_trip_through_the_plug() {
        let v = vectors();
        let p = plug();
        let cases = v["cases"].as_array().unwrap();
        assert!(cases.len() >= 10);
        for c in cases {
            let name = c["name"].as_str().unwrap();
            let att = Attestation::from_bytes(&unhex(c["wire"].as_str().unwrap())).unwrap();
            assert_eq!(
                tok(p.verify_attestation(&att, None)),
                c["plug"].as_str().unwrap(),
                "{name}"
            );
        }
    }

    #[test]
    fn trait_seam_agrees_and_never_counts_one_sided_as_both() {
        let p = plug();
        let both = case("reticulum both legs");
        assert!(both.verify_transport_leg(&p).is_ok());
        let mut set = AttestationSet::new();
        assert!(set.insert_with_transport(both, &p).is_ok());
        // One-sided passes the node leg but never id111's two-signature paths.
        let one = case("meshtastic one-sided");
        assert!(one.verify_node_leg().is_ok());
        assert_eq!(
            one.verify_transport_leg(&p),
            Err(AttestError::InvalidTransportSignature)
        );
        assert!(set.insert_with_transport(one, &p).is_err());
        for n in [
            "reticulum signature over the wrong input (no tag)",
            "reticulum cross-node transplant",
            "reticulum all-zero signature is never one-sided",
            "reticulum wrong hash (halves swapped)",
        ] {
            assert!(case(n).verify_transport_leg(&p).is_err(), "{n}");
        }
    }

    #[test]
    fn unknown_key_is_rejected() {
        let both = case("reticulum both legs");
        let empty = BridgeVerifier::new();
        assert_eq!(
            tok(empty.verify_attestation(&both, None)),
            "err:unknown-key"
        );
        assert!(both.verify_transport_leg(&empty).is_err());
        // A different known key (one Ed25519 byte off) does not stand in.
        let mut other = pubkey64(&vectors());
        other[63] ^= 0x01;
        let mut p = BridgeVerifier::new();
        p.add_reticulum_key(other).unwrap();
        assert_eq!(tok(p.verify_attestation(&both, None)), "err:unknown-key");
    }

    #[test]
    fn nostr_is_false_from_this_plug() {
        let l2: Value = serde_json::from_str(L2_ATTEST).unwrap();
        let att = Attestation::from_bytes(&unhex(l2["kat"]["wire"].as_str().unwrap())).unwrap();
        assert_eq!(att.transport_id(), TransportId::NostrSecp256k1);
        assert!(att.verify_node_leg().is_ok());
        let p = plug();
        assert!(!p.verify_transport_signature(
            TransportId::NostrSecp256k1,
            att.transport_key(),
            b"anything",
            att.transport_sig()
        ));
        assert_eq!(tok(p.verify_attestation(&att, None)), "err:unsupported:1");
        assert!(att.verify_transport_leg(&p).is_err());
    }

    #[test]
    fn expiry_only_against_a_supplied_epoch() {
        let p = plug();
        let v = vectors();
        let expiry: u64 = v["expiry_epoch"].as_str().unwrap().parse().unwrap();
        let one = case("meshtastic one-sided");
        assert_eq!(
            tok(p.verify_attestation(&one, Some(MeshEpoch::from_u64(expiry - 1)))),
            "ok:one-sided"
        );
        assert_eq!(
            tok(p.verify_attestation(&one, Some(MeshEpoch::from_u64(expiry)))),
            "err:expired"
        );
    }

    #[test]
    fn key_table_is_bounded_and_idempotent() {
        let mut p = BridgeVerifier::new();
        let k = pubkey64(&vectors());
        p.add_reticulum_key(k).unwrap();
        p.add_reticulum_key(k).unwrap();
        for i in 1..MAX_RETICULUM_KEYS {
            let mut x = [0u8; 64];
            x[0] = i as u8;
            p.add_reticulum_key(x).unwrap();
        }
        assert_eq!(p.add_reticulum_key([0xffu8; 64]), Err(KeyTableFull));
        assert!(p.add_reticulum_key(k).is_ok(), "known key stays idempotent");
        let tkey = unhex(v_tkey().as_str()).try_into().unwrap();
        assert_eq!(p.reticulum_key_for(&tkey), Some(&k));
    }

    fn v_tkey() -> String {
        vectors()["reticulum"]["transport_key"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn a_fresh_record_from_the_api_verifies_both_legs() {
        // The plug against a record issued now (not only the frozen bytes): node leg via
        // id111, transport leg = the frozen identity signature over this node's input.
        let v = vectors();
        let ks =
            id111::keyset::Keyset::derive_from_mnemonic(v["node"]["mnemonic"].as_str().unwrap())
                .unwrap();
        let sig: [u8; 64] = unhex(v["reticulum"]["transport_sig"].as_str().unwrap())
            .try_into()
            .unwrap();
        let tkey: [u8; 32] = unhex(&v_tkey()).try_into().unwrap();
        let att =
            NodeLeg::issue(&ks, TransportId::Reticulum, tkey, MeshEpoch::from_u64(7)).complete(sig);
        assert_eq!(tok(plug().verify_attestation(&att, None)), "ok:both");
    }
}
