# Layer 7 — test vectors (`7-relay/README.md` v1.0)

Written **2026-11-02** (task M2). Spec: `../../../7-relay/README.md` §5.4 (the hint TLV `0x02 dest_hub`, Layer 7 §5.3
amendment 7 to the Layer 5 ext TLV registry). The five frozen Layer 5 files one level up are untouched and keep their own
drift gate (`test_double::frozen::frozen_files_match_generation`).

| File | Freezes |
|---|---|
| `dest-hub.json` | a member's `CUSTODY_PUT` without a TLV, with `dest_hub` alone (`n_ext ‖ 0x02 ‖ 8 ‖ hint`, 11 B, a 131 B wrap), with an unknown TLV alone and with `dest_hub` beside it (10 B); header, inner before auth, `pair_mac`, inner, packet length; rejections: `0x02` on every other built kind, a length other than 8, the type twice |

Gates: `crates/mesh111/src/envelope.rs` (`tests::layer7`): `layer7_files_match_generation` fails on a missing file or any
byte of drift, `layer7_vectors` re-asserts every field against the public `mesh111` API (and seals and opens each put);
`../crosscheck.py` (its `layer7/` section) re-derives the TLV bytes, layouts, `pair_mac`s, sizes and rejection reasons
from §5.4 alone. Regeneration only under a signed-off amendment: `cargo test -p mesh111 write_layer7_vectors -- --ignored`.
