# Layer 8 — test vectors (`8-storage/README.md` v1.0)

Written **2026-11-16** (task M1). Spec: `../../../8-storage/README.md` §2.2 rule 2 (the node-state store's key) and §2.7 S1
(`derive_tagged_key` and `KeyProvider::key_for`, a code seam added to locked Layer 5: no Layer 5 spec text, no wire byte); the
`111/node-state/v1` tag is a key-derivation tag, never a signing tag (Layer 8 §3 J5). The five frozen Layer 5 files and
`store-kdf.json` one level up are untouched and keep their own gates (`test_double::frozen::frozen_files_match_generation`,
`keys::tests::store_kdf_vectors`).

| File | Freezes |
|---|---|
| `node-state-kdf.json` | `node_state_key = BLAKE2b-256(key = store_secret, data = "111/node-state/v1" ‖ NodeId)` (RFC 7693 keyed mode) over the four inputs of `../store-kdf.json`, in its order: the keyed hash's input (`tag ‖ NodeId`, 49 B), the key and its SQLCipher raw-key literal; each key differs from the `store_key` of the same inputs |

Gates: `crates/mesh111/src/keys.rs` (`tests::layer8`): `layer8_files_match_generation` fails on a missing file or any byte
of drift, `node_state_kdf_vectors` re-derives every case through `derive_tagged_key` and through `KeyProvider::key_for`
(Mode S) and requires the answers computed independently with Python's `hashlib` verbatim; `../crosscheck.py` (its
`layer8/` section) re-derives every field from Layer 8 §2.2 rule 2 alone. Regeneration only under a signed-off amendment:
`cargo test -p mesh111 write_layer8_vectors -- --ignored`.
