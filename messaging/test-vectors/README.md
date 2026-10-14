# Layer 5 — frozen test vectors (spec §20 G4)

Frozen **2026-10-14** (task C2). Spec: `../README.md` v1.2 (locked, D20). Written by C2/C3/F1; **never edited by hand
after freeze** — a vector changes only with a spec amendment and reviewer sign-off. If `cargo test` fails against a file
here, the implementation drifted: fix the code, do not regenerate.

The files are live gates: `crates/mesh111/src/test_double.rs` (`frozen` test module, one loader per file) reads every
file with a strict in-tree JSON reader and re-asserts every field against the public `mesh111` API — layouts, hashes,
pair keys, `pair_mac`s (and their direction binding), signed inputs, the Ed25519 signatures (re-signed from the frozen
identities and compared byte for byte, then verified under id111's strict predicate) and every rejection token — and
`frozen_files_match_generation` fails on any byte of drift between the files and what the code generates.
`crosscheck.py` independently re-derives every hash, layout and signed input from the spec text with the Python standard
library only (no `mesh111` code): `python3 -I crosscheck.py`.

| File | Freezes |
|---|---|
| `ids.json` | §3 minimal LEB128 varints, §4 two-varint `msg_seq` (epoch ≤ `i64::MAX`, the store's INTEGER); over-long / overflow / `n > u32` / `epoch > i64::MAX` rejections |
| `envelope.json` | §3 L2 header bytes (`extra` = `prio_flags` ‖ `dest_hint`), inner layout before auth, pair key (`111/pair/v1`), `pair_mac`, `sig` input with `dest` (recipient / zeros for public classes) and the fragment position (`frag_idx ‖ frag_count`, `0 ‖ 0` unfragmented — a signed Tier-1 fragment case is included), full inner; Tier-0 and Tier-1 fragment headers incl. cases that fill a 160 B / 233 B bearer exactly; piggybacked two-epoch ACK; INTRO and ENROLL cards; standalone ACK and CUSTODY_ACK packets; reserved-bit, kind, `msg_seq` (incl. `epoch = 0` and `epoch > i64::MAX`), fragment-bound, TLV, flag-combination and card rejections |
| `ack.json` | §6.1 blocks (prefix, ranges ≤ 8, frozen-epoch ranges, partial bitmaps, strong receipts), `ack_sig` input and signature, malformed-block rejections |
| `custody.json` | §6.2 receipts for statuses 1–5, `pkt_ref`, `custody-sig` input and signature, rejections |
| `enroll.json` | §7.3 keyed hash (`111/enroll/v1`), member binding, strict code-parser rejections |
| `store-kdf.json` | §10.2 Mode S `store_key = BLAKE2b-256(key = store_secret, "111/store/v1" ‖ NodeId)` and the SQLCipher raw-key literal (task C3; asserted by `keys::tests::store_kdf_vectors`, which also requires the two inline doc answers verbatim) |

Format: plain JSON, hex bytes, u64 values as decimal strings. Test identities: `a` = canonical-12 and `b` = canonical-24
mnemonics (the same as `identity/test-vectors`). `epk` values are fixed stand-ins for the sealer's ephemeral key. The AEAD
around the inner bytes is id111's (`identity/test-vectors/layer5/sealed-payload.json`). Rejection `expect` tokens are the
Rust `Debug` form of the module's error enum.

Regeneration (only to re-freeze under a signed-off amendment): `cargo test -p mesh111 write_vectors -- --ignored`, then
re-run `crosscheck.py` and the full `scripts/check.sh`.
