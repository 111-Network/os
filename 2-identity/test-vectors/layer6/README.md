# Layer 6 amendment vectors — `layer6/` (frozen 2026-10-19, task S3)

Layer 6 (`../../../6-transport/README.md` §14, D26 #3) assigns the A10 `transport_id` values reserved in
`../../README.md` §13.1. They land as `id111` registry values (`src/attest.rs`) and these vectors, in a **subdirectory on
purpose**: the top-level set is the Cat-1 Monocypher harness contract and stays byte-identical (D20);
`monocypher-harness/run.sh` and `conformance.rs` inventory the top level only.
`../../crates/id111/tests/layer6_attest.rs` loads every `layer6/*.json` (its own inventory, frozen date `2026-10-19`) and
re-asserts the registry bytes, parsing, the node leg and a byte-exact re-issue; `node111`'s `BridgeVerifier`
(`1-bootstrap/crates/node111/src/daemon/bridge/attest.rs`) replays the same file through the transport-leg plug.

| File | Amendment | What it freezes |
|---|---|---|
| `attest-transport-ids.json` | **A10** — `transport_id` `0x02` meshtastic, `0x03` reticulum | the registry bytes and display strings; a Reticulum identity (RFC 8032 TEST 1 Ed25519 half, RFC 7748 Alice X25519 half), its `pubkey64`, `transport_key = SHA-256(pubkey64)` and the transport signature; a 202 B record with both legs valid, a one-sided `0x02` record (64 zero bytes), and the negatives: wrong hash (signing half only, halves swapped), signature over the wrong input (untagged, node tag, another node), an all-zero `0x03` signature, a `0x02` record with a non-zero signature, a flipped `node_sig`, a relabelled registry byte |

**Relation to the frozen L2 attestation vectors.** `attestation.json` still names a `0x02` record `err:tid:2` (the Cat-1
harness replays exactly that: a core without A10 refuses the id, the soft-upgrade rule). Since A10 `id111` parses `0x02`, so
that frozen record reaches the node leg and fails there (its `node_sig` was made over `0x01`); `conformance.rs` maps
`err:tid:2` / `err:tid:3` to `err:node-sig:InvalidSignature` and asserts the mapping applies exactly once. Still a rejection.

Frozen means frozen: regenerate only with an identity-layer sign-off
(`ID111_LAYER6_OUT=<path> cargo test -p id111 --test layer6_attest -- --ignored`). The keys here are public RFC test
material — never use them for a real node or bridge.
