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
//!
//! **Layer 8 §2.7 S1** (`8-storage/README.md`; a code seam added to this
//! locked layer, no spec text, no wire byte): the Mode S KDF takes its tag as
//! an input, [`derive_tagged_key`], and [`KeyProvider::key_for`] answers a
//! tag, so the key of Layer 8's node-state store (`node/state.db`, Layer 8
//! §2.2 rule 2) comes from the same custody under [`NODE_STATE_KDF_TAG`]:
//! no new secret, no key on disk. `store_key()` equals
//! `key_for(STORE_KDF_TAG)` byte for byte; Mode T answers `Unsupported` for
//! every tag, so a store it would key stays closed.

use blake2::digest::consts::U32;
use blake2::digest::{FixedOutput, KeyInit, Update};
use blake2::Blake2bMac;
use zeroize::{Zeroize, Zeroizing};

/// §3.1 / §10.2 domain tag of the store-key KDF.
pub const STORE_KDF_TAG: &[u8] = b"111/store/v1";

/// Layer 8 §2.2 rule 2: the domain tag of the node-state store's key
/// (`node/state.db`), `node_state_key = BLAKE2b-256(key = store_secret,
/// data = "111/node-state/v1" ‖ NodeId)`, reached through
/// [`KeyProvider::key_for`] (Layer 8 §2.7 S1). A key-derivation tag, never a
/// signing tag (Layer 8 §3 J5): no signature is made under it. Known answers:
/// `5-messaging/test-vectors/layer8/node-state-kdf.json`.
pub const NODE_STATE_KDF_TAG: &[u8] = b"111/node-state/v1";

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
    /// The raw 32-byte store key. Equals `key_for(STORE_KDF_TAG)` byte for
    /// byte (Layer 8 §2.7 S1).
    fn store_key(&self) -> Result<StoreKey, KeyError>;
    /// Layer 8 §2.7 S1: the raw 32-byte key this custody derives under
    /// `tag`, a key-derivation tag such as [`NODE_STATE_KDF_TAG`] (Layer 8
    /// §2.2 rule 2) or [`STORE_KDF_TAG`]. Mode S answers
    /// [`derive_tagged_key`]; Mode T answers [`KeyError::Unsupported`], and
    /// the store it would key stays closed.
    fn key_for(&self, tag: &[u8]) -> Result<StoreKey, KeyError>;
}

/// Layer 8 §2.7 S1: the Mode S KDF under a given tag,
/// `BLAKE2b-256(key = secret, data = tag ‖ NodeId)` (RFC 7693 keyed mode).
/// [`derive_store_key`] is this call with [`STORE_KDF_TAG`]; the node-state
/// store's key is this call with [`NODE_STATE_KDF_TAG`] (Layer 8 §2.2 rule
/// 2). `tag` is a key-derivation tag of the `111/<name>/v1` family, a
/// constant of the calling code. The NodeId is a fixed 32 bytes at the end of
/// the data, so two different tags never hash the same data.
///
/// A 32-byte key is always within BLAKE2b's 64-byte key limit, so the keyed
/// constructor does not fail in practice; it is still a typed
/// [`KeyError::Kdf`], never a panic. Known answers:
/// `5-messaging/test-vectors/store-kdf.json` (the store tag) and
/// `5-messaging/test-vectors/layer8/node-state-kdf.json` (the node-state tag).
pub fn derive_tagged_key(
    tag: &[u8],
    secret: &[u8; 32],
    node_id: &[u8; 32],
) -> Result<StoreKey, KeyError> {
    let mut mac =
        <Blake2bMac<U32> as KeyInit>::new_from_slice(secret).map_err(|_| KeyError::Kdf)?;
    Update::update(&mut mac, tag);
    Update::update(&mut mac, node_id);
    let mut out = mac.finalize_fixed();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    out.zeroize();
    Ok(StoreKey::from_bytes(key))
}

/// §10.2 Mode S: `BLAKE2b-256(key = secret, data = "111/store/v1" ‖ NodeId)`,
/// [`derive_tagged_key`] under [`STORE_KDF_TAG`] (Layer 8 §2.7 S1), byte for
/// byte.
///
/// A typed [`KeyError::Kdf`], never a panic. Known answers:
/// `5-messaging/test-vectors/store-kdf.json`.
pub fn derive_store_key(
    keyset_secret: &[u8; 32],
    node_id: &[u8; 32],
) -> Result<StoreKey, KeyError> {
    derive_tagged_key(STORE_KDF_TAG, keyset_secret, node_id)
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

    fn key_for(&self, tag: &[u8]) -> Result<StoreKey, KeyError> {
        derive_tagged_key(tag, &self.secret, &self.node_id)
    }
}

/// Mode T stub: hardware-sealed keys are not built in v1 (§10.2).
#[derive(Debug, Default, Clone, Copy)]
pub struct ModeT;

/// Mode T's one refusal, the same for `store_key()` and every `key_for(tag)`.
const MODE_T_UNSUPPORTED: &str =
    "Mode T (hardware-sealed store key) is not built in v1; use Mode S";

impl KeyProvider for ModeT {
    fn mode(&self) -> KeyMode {
        KeyMode::T
    }

    fn store_key(&self) -> Result<StoreKey, KeyError> {
        Err(KeyError::Unsupported(MODE_T_UNSUPPORTED))
    }

    /// Layer 8 §2.2 rule 2: no tag is answered in v1; the node-state store
    /// stays closed.
    fn key_for(&self, _tag: &[u8]) -> Result<StoreKey, KeyError> {
        Err(KeyError::Unsupported(MODE_T_UNSUPPORTED))
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

    /// Layer 8 §2.7 S1: `derive_store_key` is `derive_tagged_key` under
    /// `STORE_KDF_TAG`, byte for byte (the A9 answers above and
    /// `store-kdf.json` unchanged); the tagged KDF binds its tag, its secret
    /// and the NodeId.
    #[test]
    fn store_key_is_the_tagged_kdf_under_the_store_tag() {
        for (secret, node, want) in INLINE {
            let tagged = derive_tagged_key(STORE_KDF_TAG, &secret, &node).unwrap();
            assert_eq!(tagged.as_bytes(), &unhex(want));
            assert_eq!(tagged.as_bytes(), kdf(&secret, &node).as_bytes());
            let node_state = derive_tagged_key(NODE_STATE_KDF_TAG, &secret, &node).unwrap();
            assert_ne!(node_state.as_bytes(), tagged.as_bytes());
        }
        let k = |tag: &[u8], s: u8, n: u8| {
            *derive_tagged_key(tag, &[s; 32], &[n; 32])
                .unwrap()
                .as_bytes()
        };
        let base = k(NODE_STATE_KDF_TAG, 9, 1);
        assert_ne!(base, k(NODE_STATE_KDF_TAG, 8, 1), "secret");
        assert_ne!(base, k(NODE_STATE_KDF_TAG, 9, 2), "NodeId");
        assert_ne!(base, k(b"111/node-state/v2", 9, 1), "tag");
    }

    /// Layer 8 §2.2 rule 2, §3 J5: the node-state tag is
    /// `111/node-state/v1`, a key-derivation tag of its own: no other tag
    /// this crate defines, signing or keyed, has its bytes.
    #[test]
    fn node_state_tag_is_its_own_kdf_tag() {
        assert_eq!(NODE_STATE_KDF_TAG, b"111/node-state/v1");
        for other in [
            STORE_KDF_TAG,
            crate::envelope::PAIR_KEY_TAG,
            crate::envelope::MSG_SIG_TAG,
            crate::ack::ACK_SIG_TAG,
            crate::custody::CUSTODY_TAG,
            crate::custody::CUSTODY_SIG_TAG,
            crate::enroll::ENROLL_TAG,
        ] {
            assert_ne!(NODE_STATE_KDF_TAG, other);
        }
    }

    /// Layer 8 vectors (`test-vectors/layer8/`, Layer 8 §2.2 rule 2 and §2.7
    /// S1): generator, drift gate and loader. `store-kdf.json`, its gate
    /// above and the frozen Layer 5 files' drift gate (`test_double::frozen`)
    /// are untouched.
    mod layer8 {
        use super::*;
        use crate::test_double::{hex, load, vectors_dir};

        const FILE: &str = "layer8/node-state-kdf.json";
        const WRITTEN: &str = "2026-11-16";

        /// Known answers computed independently with Python's
        /// `hashlib.blake2b(b"111/node-state/v1" + node_id, key=secret,
        /// digest_size=32)` over the two inline A9 inputs above; the loader
        /// requires them in the file verbatim.
        const INLINE_NODE_STATE: [&str; 2] = [
            "f7baa8403a789e49454168ac665980ade729156bea6c819310c509f8d240fb7e",
            "1475a4f8d11ce8e045ca1b4cd323dc7fca02309f71b9ab76650b53d665b76075",
        ];

        fn s(v: &str) -> String {
            format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
        }

        fn obj(fields: &[(&str, String)]) -> String {
            let body: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{}: {v}", s(k)))
                .collect();
            format!("{{{}}}", body.join(", "))
        }

        fn arr(items: &[String]) -> String {
            format!("[\n    {}\n  ]", items.join(",\n    "))
        }

        fn doc(fields: &[(&str, String)]) -> String {
            let body: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("  {}: {v}", s(k)))
                .collect();
            format!("{{\n{}\n}}\n", body.join(",\n"))
        }

        /// The data the keyed hash takes: `tag ‖ NodeId` (49 B).
        fn kdf_input(node: &[u8; 32]) -> Vec<u8> {
            [NODE_STATE_KDF_TAG, &node[..]].concat()
        }

        /// The file's text: the public KDF over the inputs of
        /// `store-kdf.json`, in its order (the vector sits beside it).
        fn generate() -> String {
            let store = load("store-kdf.json");
            let cases: Vec<String> = store
                .get("cases")
                .a()
                .iter()
                .map(|c| {
                    let secret = c.get("store_secret").h32();
                    let node = c.get("node_id").h32();
                    let key = derive_tagged_key(NODE_STATE_KDF_TAG, &secret, &node).unwrap();
                    obj(&[
                        ("name", s(c.get("name").s())),
                        ("store_secret", s(&hex(&secret))),
                        ("node_id", s(&hex(&node))),
                        ("kdf_input", s(&hex(&kdf_input(&node)))),
                        ("node_state_key", s(&hex(key.as_bytes()))),
                        ("sqlcipher_key", s(&key.sqlcipher_literal())),
                    ])
                })
                .collect();
            let tag = std::str::from_utf8(NODE_STATE_KDF_TAG).unwrap();
            doc(&[
                ("spec", s("Layer 8 §2.2 rule 2 the node-state store key (§2.7 S1 KeyProvider::key_for; §3 J5: a key-derivation tag, never a signing tag)")),
                ("written", s(WRITTEN)),
                ("description", s("node_state_key = BLAKE2b-256(key = store_secret (32 B), data = '111/node-state/v1' ‖ NodeId (32 B)) — RFC 7693 keyed mode, 32-byte digest: the KDF of ../store-kdf.json under the node-state tag (mesh111 keys::derive_tagged_key, reached through KeyProvider::key_for); handed to SQLCipher as the raw key literal x'<64 hex>' for node/state.db (no PBKDF2 at open). 'kdf_input' is the keyed hash's data, tag ‖ NodeId (49 B). The inputs are ../store-kdf.json's cases in its order, so each node_state_key stands beside the store_key of the same inputs and differs from it. store_secret is the identity holder's Mode S secret (Sealer::store_secret); it is an input here, never a key on disk.")),
                ("provenance", s("Generated by mesh111 keys::tests::layer8::write_layer8_vectors (ignored) over the inputs of ../store-kdf.json. layer8_files_match_generation fails on a missing file or any byte of drift; node_state_kdf_vectors re-derives every case through derive_tagged_key and KeyProvider::key_for (Mode S) and requires the two answers computed independently with Python hashlib.blake2b(b'111/node-state/v1' + node_id, key=store_secret, digest_size=32) verbatim; crosscheck.py's layer8/ section re-derives every field from Layer 8 §2.2 rule 2 alone.")),
                ("tag", s(tag)),
                ("cases", arr(&cases)),
            ])
        }

        /// Regenerate `layer8/node-state-kdf.json`. Run ONLY under a
        /// signed-off amendment: `cargo test -p mesh111 write_layer8_vectors
        /// -- --ignored`.
        #[test]
        #[ignore]
        fn write_layer8_vectors() {
            std::fs::create_dir_all(vectors_dir().join("layer8")).unwrap();
            std::fs::write(vectors_dir().join(FILE), generate()).unwrap();
        }

        /// The file on disk is exactly what the code generates (drift gate);
        /// a missing file fails here.
        #[test]
        fn layer8_files_match_generation() {
            let p = vectors_dir().join(FILE);
            let disk =
                std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
            assert!(
                disk == generate(),
                "{FILE}: implementation drifted from the vector"
            );
        }

        /// G13 (1): every case re-derived through `derive_tagged_key` and
        /// through `KeyProvider::key_for` (Mode S), with its keyed-hash input
        /// and its SQLCipher literal; the inputs are `store-kdf.json`'s, in
        /// its order, and no node-state key equals the store key of the same
        /// inputs; the independent inline answers appear verbatim.
        #[test]
        fn node_state_kdf_vectors() {
            let v = load(FILE);
            assert_eq!(v.get("written").s(), WRITTEN);
            assert_eq!(v.get("tag").s().as_bytes(), NODE_STATE_KDF_TAG);
            let cases = v.get("cases").a();
            let store = load("store-kdf.json");
            let store_cases = store.get("cases").a();
            assert_eq!(cases.len(), store_cases.len());
            for (c, sc) in cases.iter().zip(store_cases) {
                let name = c.get("name").s();
                assert_eq!(name, sc.get("name").s());
                let secret = c.get("store_secret").h32();
                let node = c.get("node_id").h32();
                assert_eq!(secret, sc.get("store_secret").h32(), "{name}: store_secret");
                assert_eq!(node, sc.get("node_id").h32(), "{name}: node_id");
                assert_eq!(
                    c.get("kdf_input").hex(),
                    kdf_input(&node),
                    "{name}: kdf_input"
                );
                let want = c.get("node_state_key").h32();
                let key = derive_tagged_key(NODE_STATE_KDF_TAG, &secret, &node).unwrap();
                assert_eq!(key.as_bytes(), &want, "{name}: node_state_key");
                let mode_s = ModeS::new(Zeroizing::new(secret), node);
                let provider: &dyn KeyProvider = &mode_s;
                let via = provider.key_for(NODE_STATE_KDF_TAG).unwrap();
                assert_eq!(via.as_bytes(), &want, "{name}: key_for");
                assert_eq!(
                    &via.sqlcipher_literal()[..],
                    c.get("sqlcipher_key").s(),
                    "{name}: literal"
                );
                assert_ne!(want, sc.get("store_key").h32(), "{name}: the store key");
            }
            for ((secret, node, _), want) in INLINE.iter().zip(INLINE_NODE_STATE) {
                assert!(
                    cases.iter().any(|c| c.get("store_secret").h32() == *secret
                        && c.get("node_id").h32() == *node
                        && c.get("node_state_key").s() == want),
                    "inline answer {want} missing from {FILE}"
                );
            }
        }
    }
}
