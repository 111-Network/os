//! §10.2 — the `KeyProvider` seam: where the 32-byte raw SQLCipher key of
//! `fabric/store.db` comes from.
//!
//! - **Mode S** (default; Cat 2/3/4 without a TPM; phones):
//!   `store_key = BLAKE2b-256(key = L2 keyset-derived secret, data =
//!   "111/store/v1" ‖ NodeId)` — keyed BLAKE2b (RFC 7693 keyed mode) as the
//!   KDF, the same hash family as L2 §6.4 (D20 c, L2 amendment A9). The
//!   keyset-derived secret is computed by the identity holder inside `111d`
//!   and handed in; the keyset itself never enters `mesh111` (§18.3). The
//!   derived key is never written to disk; a DB copied elsewhere opens again
//!   after restore-from-words.
//! - **Mode T** (Cat 4 with TPM2 / SE050B): a random 32 B key sealed in
//!   hardware. Not built in v1 — [`ModeT`] answers [`KeyError::Unsupported`]
//!   (fail closed; Mode S is the documented recovery path).
//!
//! Never bind the key to an OS keyring without a recovery path (§10.2).

use blake2::digest::consts::U32;
use blake2::digest::{FixedOutput, KeyInit, Update};
use blake2::Blake2bMac;
use zeroize::{Zeroize, Zeroizing};

/// §3.1 / §10.2 domain tag of the store-key KDF.
pub const STORE_KDF_TAG: &[u8] = b"111/store/v1";

/// The raw 32-byte SQLCipher key (§10.1 `PRAGMA key = "x'…'"`).
///
/// Zeroized on drop; `Debug` never prints the bytes.
pub struct StoreKey(Zeroizing<[u8; 32]>);

impl StoreKey {
    /// Wrap raw key bytes (Mode T unseal, tests).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        let mut b = bytes;
        let k = StoreKey(Zeroizing::new(b));
        b.zeroize();
        k
    }

    /// The raw key bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The SQLCipher raw-key literal `x'<64 hex>'` (no PBKDF2 at open,
    /// §10.1), in zeroizing memory.
    pub(crate) fn sqlcipher_literal(&self) -> Zeroizing<String> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = Zeroizing::new(String::with_capacity(67));
        s.push_str("x'");
        for b in self.0.iter() {
            s.push(char::from(HEX[usize::from(b >> 4)]));
            s.push(char::from(HEX[usize::from(b & 0x0f)]));
        }
        s.push('\'');
        s
    }
}

impl std::fmt::Debug for StoreKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StoreKey(..)")
    }
}

/// Which custody mode a provider implements (§10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMode {
    /// Keyed BLAKE2b off a keyset-derived secret.
    S,
    /// Hardware-sealed random key (TPM2 / SE050B).
    T,
}

/// Typed key-custody failures (never a panic).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyError {
    /// The mode is not available on this build or device (Mode T in v1).
    #[error("key mode unsupported: {0}")]
    Unsupported(&'static str),
    /// The keyed-BLAKE2b KDF refused its key (cannot happen for a 32-byte
    /// key; typed instead of a panic path).
    #[error("store-key KDF failed")]
    Kdf,
}

/// §10.2 — the seam the store opens through. `111d` picks the provider.
pub trait KeyProvider: Send + Sync {
    /// The custody mode this provider implements.
    fn mode(&self) -> KeyMode;
    /// The raw 32-byte store key.
    fn store_key(&self) -> Result<StoreKey, KeyError>;
}

/// §10.2 Mode S: `BLAKE2b-256(key = secret, data = "111/store/v1" ‖ NodeId)`.
///
/// A 32-byte key is always within BLAKE2b's 64-byte key limit, so the keyed
/// constructor does not fail in practice; it is still a typed
/// [`KeyError::Kdf`], never a panic. Known answers:
/// `messaging/test-vectors/store-kdf.json`.
pub fn derive_store_key(
    keyset_secret: &[u8; 32],
    node_id: &[u8; 32],
) -> Result<StoreKey, KeyError> {
    let mut mac =
        <Blake2bMac<U32> as KeyInit>::new_from_slice(keyset_secret).map_err(|_| KeyError::Kdf)?;
    Update::update(&mut mac, STORE_KDF_TAG);
    Update::update(&mut mac, node_id);
    let mut out = mac.finalize_fixed();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    out.zeroize();
    Ok(StoreKey::from_bytes(key))
}

/// Mode S provider: holds the keyset-derived secret handed in by the identity
/// holder (zeroized on drop) and the node's NodeId.
pub struct ModeS {
    secret: Zeroizing<[u8; 32]>,
    node_id: [u8; 32],
}

impl ModeS {
    /// `keyset_secret` is the L2 keyset-derived secret computed behind the
    /// identity handle (A9); `node_id` is this node's NodeId (32 B).
    pub fn new(keyset_secret: Zeroizing<[u8; 32]>, node_id: [u8; 32]) -> Self {
        ModeS {
            secret: keyset_secret,
            node_id,
        }
    }
}

impl std::fmt::Debug for ModeS {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModeS").finish_non_exhaustive()
    }
}

impl KeyProvider for ModeS {
    fn mode(&self) -> KeyMode {
        KeyMode::S
    }

    fn store_key(&self) -> Result<StoreKey, KeyError> {
        derive_store_key(&self.secret, &self.node_id)
    }
}

/// Mode T stub: hardware-sealed keys are not built in v1 (§10.2).
#[derive(Debug, Default, Clone, Copy)]
pub struct ModeT;

impl KeyProvider for ModeT {
    fn mode(&self) -> KeyMode {
        KeyMode::T
    }

    fn store_key(&self) -> Result<StoreKey, KeyError> {
        Err(KeyError::Unsupported(
            "Mode T (hardware-sealed store key) is not built in v1; use Mode S",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, o) in out.iter_mut().enumerate() {
            *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    fn kdf(secret: &[u8; 32], node: &[u8; 32]) -> StoreKey {
        derive_store_key(secret, node).unwrap()
    }

    /// A9 known answers, computed independently with Python's
    /// `hashlib.blake2b(b"111/store/v1" + node_id, key=secret, digest_size=32)`.
    /// The same values are frozen in `test-vectors/store-kdf.json`; the
    /// loader below requires the file to carry them unchanged.
    const INLINE: [([u8; 32], [u8; 32], &str); 2] = [
        (
            {
                let mut s = [0u8; 32];
                let mut i = 0;
                while i < 32 {
                    s[i] = i as u8;
                    i += 1;
                }
                s
            },
            {
                let mut n = [0u8; 32];
                let mut i = 0;
                while i < 32 {
                    n[i] = 0xA0 + (i % 16) as u8;
                    i += 1;
                }
                n
            },
            "0bb982a752ba4c3c85d4b6d4a7023e8163ebd32dc1a13ad89f8f6ef7bad3858f",
        ),
        (
            [0x42; 32],
            [0x07; 32],
            "e2f9f08d74efe9a9e56b2b6c6beef807e86e1cfc57287478f96e0d95e810779e",
        ),
    ];

    #[test]
    fn mode_s_known_answers() {
        for (secret, node, want) in INLINE {
            assert_eq!(kdf(&secret, &node).as_bytes(), &unhex(want));
        }
    }

    /// G4: `test-vectors/store-kdf.json` — every case re-derived through the
    /// public KDF and the SQLCipher literal; the inline answers must appear
    /// in the file verbatim (the file cannot drift from them).
    #[test]
    fn store_kdf_vectors() {
        let doc = crate::test_double::load("store-kdf.json");
        assert_eq!(doc.get("frozen").s(), "2026-10-14");
        let cases = doc.get("cases").a();
        assert!(cases.len() >= INLINE.len());
        for c in cases {
            let name = c.get("name").s();
            let secret = c.get("store_secret").h32();
            let node = c.get("node_id").h32();
            let key = kdf(&secret, &node);
            assert_eq!(
                key.as_bytes(),
                &c.get("store_key").h32(),
                "{name}: store_key"
            );
            assert_eq!(
                &key.sqlcipher_literal()[..],
                c.get("sqlcipher_key").s(),
                "{name}: literal"
            );
        }
        for (secret, node, want) in INLINE {
            assert!(
                cases.iter().any(|c| c.get("store_secret").h32() == secret
                    && c.get("node_id").h32() == node
                    && c.get("store_key").s() == want),
                "inline answer {want} missing from store-kdf.json"
            );
        }
    }

    #[test]
    fn mode_s_provider_matches_kdf_and_binds_node_id() {
        let p = ModeS::new(Zeroizing::new([9; 32]), [1; 32]);
        assert_eq!(p.mode(), KeyMode::S);
        let k = p.store_key().unwrap();
        assert_eq!(k.as_bytes(), kdf(&[9; 32], &[1; 32]).as_bytes());
        assert_ne!(k.as_bytes(), kdf(&[9; 32], &[2; 32]).as_bytes());
        assert_ne!(k.as_bytes(), kdf(&[8; 32], &[1; 32]).as_bytes());
        assert_eq!(format!("{k:?}"), "StoreKey(..)");
        assert!(!format!("{p:?}").contains('9'));
    }

    #[test]
    fn mode_t_is_unsupported() {
        assert_eq!(ModeT.mode(), KeyMode::T);
        assert!(matches!(ModeT.store_key(), Err(KeyError::Unsupported(_))));
    }

    #[test]
    fn sqlcipher_literal_form() {
        let k = StoreKey::from_bytes([0xAB; 32]);
        let lit = k.sqlcipher_literal();
        assert_eq!(lit.len(), 67);
        assert!(lit.starts_with("x'abab") && lit.ends_with("ab'"));
    }
}
