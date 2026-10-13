# Monocypher 4.0.3 — rubric smoke-test log (T9; spec §11 / D14)

The first-party smoke-test evidence for the `monocypher` entry in the knowledge base (`os/knowledge/projects/monocypher.md`, `### Smoke test`: who / device / date / procedure / result — format per `os/knowledge/README.md`). D14 designates the §14 Rust↔Monocypher-C agreement-vector run as this log. Produced by `os/identity/monocypher-harness/` (task T9); regenerate with `bash os/identity/monocypher-harness/run.sh --log <file>`.

**Gate result: PASS** — every vector file PASS, `run.sh` exit 0. **Three findings (F1–F3 below) do not fail the gate but must be decided before any Cat-1 port**; F1 and F2 contradict statements in spec §3 and in the KB entry, F3 is an open Rust↔C acceptance difference.

## Smoke test

* **Who:** identity build harness (Layer 2 task T9), executed by the T9 build subagent; the reviewer re-runs the gate before commit.
* **Device:** this host — the Rust dev host class (x86-64), **not a Cat-1 device**.
  `uname -a`: `Linux Alienware 6.6.87.2-microsoft-standard-WSL2 #1 SMP PREEMPT_DYNAMIC Thu Jun  5 18:30:46 UTC 2025 x86_64 x86_64 x86_64 GNU/Linux` (WSL2) · CPU: AMD Ryzen 7 5800X · `cc --version`: `cc (Ubuntu 11.4.0-1ubuntu1~22.04.3) 11.4.0` · flags `-std=c99 -O2 -Wall -Wextra -Werror`.
* **Date:** 2026-10-13.
* **Procedure:** `cd os/identity && bash monocypher-harness/run.sh --log <file>`. (1) `sha256sum --check` of the five vendored files against `monocypher-harness/SHA256SUMS`; (2) build `harness.c` + vendored `monocypher.c` + `optional/monocypher-ed25519.c`; (3) assert every `test-vectors/*.json` is consumed or listed n/a; (4) re-execute the frozen vectors (produced by the Rust `id111` crate) against Monocypher and demand byte-exact agreement — keygen-from-seed, Ed25519 sign (deterministic) / verify, X25519 ECDH, BIP-39 seed + SLIP-0010, §6.4 sealed-packet seal **and** open, announce, attestation node-leg, and the §3 edge/malleability suite wrapped by the strict predicate. No network access by the harness or the build. Details: `monocypher-harness/README.md`.
* **Result:** **PASS**, 0 failures. 411 checks over 214 vector entries (11 rows incl. the non-frozen supplement); runtime about 1.5 s. Full output below. The strict-predicate delta (before/after the §3 wrapper) is the section "Strict-predicate wrapper delta".

## Subject under test

* Monocypher **4.0.3** from `https://monocypher.org/download/monocypher-4.0.3.tar.gz`, downloaded 2026-10-13; tarball SHA-256 `8cc9bc341a66249016db9bd70e9142d8d0aef9945973744b1ac05dbc55d8ee66` (SHA-512 and BLAKE2b match upstream's published checksum files; sources cross-checked against the GitHub tag `4.0.3`, differing only in the line-1 version stamp). Vendored: `monocypher.{c,h}` + `optional/monocypher-ed25519.{c,h}` + `LICENCE.md`, hash-pinned in `monocypher-harness/SHA256SUMS`. Full provenance: `monocypher-harness/VENDOR.md`.
* Ed25519 under test is `crypto_ed25519_*` (RFC 8032, SHA-512, from `optional/`), **not** Monocypher's core `crypto_eddsa_*` (EdDSA-BLAKE2b, which does not interoperate with dalek).
* Rust side (the oracle's producer): `cd os/identity && cargo test` — 161 unit + 44 conformance tests pass, 0 failed (this task touched no Rust).

## Result — full `run.sh` output (verbatim)

```text
vendor: SHA256SUMS verified (Monocypher 4.0.3, see VENDOR.md)
cc: cc (Ubuntu 11.4.0-1ubuntu1~22.04.3) 11.4.0  flags: -std=c99 -O2 -Wall -Wextra -Werror
inventory: all 14 vector files accounted for
  [edge-vectors.json] ed25519_strict: 36 vectors, 35 oracle-rejects; raw crypto_ed25519_check ACCEPTED 9 of them; wrapped accepted 0; exact variant match 36/36
  [edge-vectors.json] rejecting leg histogram: NonCanonicalPublicKey=3 NonCanonicalR=4 NonCanonicalS=4 MalformedPublicKey=1 InvalidSignature(small-order/undecodable R/equation)=23
  [attestation.json] node-leg executed for 11 entries whose expected outcome is decided at a later stage (transport leg = Layer 7 seam, expiry, supersedure); expiry[] x5 skipped (epoch policy, no crypto)
  KNOWN-DELTA mixed-order R = [r]B + order-8 torsion, S = r + h*a: oracle=reject raw Monocypher=ACCEPT wrapped=ACCEPT  [OPEN: wrapper does not close it]
  KNOWN-DELTA mixed-order R = [r]B + order-2 torsion, S = r + h*a: oracle=reject raw Monocypher=ACCEPT wrapped=ACCEPT  [OPEN: wrapper does not close it]
  KNOWN-DELTA mixed-order A' = A + order-8 torsion (attacker-chosen NodeId), honest signing key a: oracle=reject raw Monocypher=ACCEPT wrapped=ACCEPT  [OPEN: wrapper does not close it]

FILE                                       VECTORS  CHECKS   PASS  FAIL  SKIP  RESULT
(built-in primitive KATs)                        7       7      7     0     0  PASS
ed25519-rfc8032.json                             4      32     32     0     0  PASS
x25519-rfc7748.json                              4      11     11     0     0  PASS
slip0010-published.json                         12      36     36     0     0  PASS
slip0010-project-paths.json                      4       8      8     0     0  PASS
keyset-project.json                              2      12     12     0     0  PASS
edge-vectors.json                               60     125    125     0     0  PASS
announce.json                                   45      82     82     0     0  PASS
sealed-packet.json                              27      41     41     0     0  PASS
attestation.json                                41      46     46     0     5  PASS
supplement/cofactor-edges.json (non-frozen)       8      11     11     0     0  PASS
TOTAL                                          214     411    411     0     5  PASS
encoding.json                              n/a -- hex/bech32/label presentation encodings; no Monocypher primitive involved
weak-key.json                              n/a -- section 4.5 keygen-time pattern self-test (policy on key bytes); no Monocypher primitive
backup-format.json                         n/a -- section 8/13 backup framing, NFKC hygiene, descriptor policy; no Monocypher primitive
restore-counters.json                      n/a -- section 6.2/6.5 counter state-machine arithmetic; no Monocypher primitive
d9-migration.json                          n/a -- Layer-1 directory migration/quarantine; no Monocypher primitive

STRICT-PREDICATE DELTA (edge-vectors.json ed25519_strict): raw crypto_ed25519_check vs section-3 wrapper vs oracle
  #    vector                                                         oracle raw    wrapped  rejecting leg
  21   small-order A: identity (y=1), order 1 (R = identity, S = 0)   reject ACCEPT reject   InvalidSignature
  22   small-order A: y=-1 (p-1), order 2 (R = identity, S = 0)       reject ACCEPT reject   InvalidSignature
  23   small-order A: y=0, order 4 (R = identity, S = 0)              reject ACCEPT reject   InvalidSignature
  24   small-order A: y=0 with x-sign bit, order 4 (R = identity, S = reject ACCEPT reject   InvalidSignature
  25   small-order A: order 8 (a) (R = identity, S = 0)               reject ACCEPT reject   InvalidSignature
  26   small-order A: order 8 (b) (R = identity, S = 0)               reject ACCEPT reject   InvalidSignature
  27   small-order A: order 8 (c) (R = identity, S = 0)               reject ACCEPT reject   InvalidSignature
  28   small-order A: order 8 (d) (R = identity, S = 0)               reject ACCEPT reject   InvalidSignature
  29   small-order A = R = identity, S = 0 (empty message)            reject ACCEPT reject   InvalidSignature
  -> 9 of 36 vectors: raw Monocypher verdict differs from the oracle (every one is repaired by the wrapper)
X25519 DELTA (edge-vectors.json x25519_low_order): 11 low-order inputs; raw crypto_x25519 returned all-zero for 11 and a non-zero value for 0 (Monocypher 4.x has no native rejection; ingress blacklist + all-zero abort are mandatory)
SUPPLEMENT (cofactor-edges.json, non-frozen): 8 rows = 1 control + 3 wrapper-bites + 1 wrapper-redundant + 3 residual-mixed-order;
  residual rows: raw Monocypher accepted 3/3, wrapper accepted 3/3 (strict cofactorless oracle rejects all) -> OPEN residual
run.sh: OK -- every file PASS
```

Reading guide: *VECTORS* = frozen entries executed (must equal the file's array lengths — a mismatch is itself a FAIL); *CHECKS* = individual byte/verdict assertions; *SKIP* = entries deliberately not executed (the 5 `attestation.json` `expiry[]` entries: a mesh-epoch comparison, no crypto). `(built-in primitive KATs)` = BLAKE2b-512/192, SHA-512, HMAC-SHA-512 (RFC 4231) and BIP-39 PBKDF2 known answers, a build/miscompile guard. The `supplement/` row is harness-local and not frozen (see F3).

## Strict-predicate wrapper delta (before / after)

Monocypher's `crypto_ed25519_check` (cofactored, accepts non-canonical A/R) is wrapped by `strict_verify()` in the §3 order used by `id111::keyset::verify_strict`: canonical A → canonical R → `S < ℓ` → A decodes → R decodes → small-order R/A rejected → Monocypher equation. `edge-vectors.json` is the oracle.

| | raw `crypto_ed25519_check` | §3 wrapper |
|---|---|---|
| Ed25519 vectors that the oracle rejects (35 of 36) | **accepts 9** | accepts **0** |
| accept/reject agreement with the oracle | 27 / 36 | **36 / 36** |
| exact `VerifyError` variant agreement | not available | **36 / 36** |

The 9 raw acceptances are exactly the validly-formed *small-order A* cases (`R = identity, S = 0`; rows 21–29 in the listing above) — for a small-order `A`, `[8h]A = O` for every `h`, so the cofactored equation always holds. Every other frozen edge vector (non-canonical S/R/A, small-order R, off-curve A, wrong message/key, bit flips) is *already* rejected by Monocypher's own equation, because the frozen vectors are built by swapping one field of RFC 8032 TEST 2 and so are false under any verifier. Rejecting leg across the 35 oracle-rejects: NonCanonicalPublicKey 3 · NonCanonicalR 4 · NonCanonicalS 4 · MalformedPublicKey 1 · InvalidSignature 23.

Because the frozen vectors cannot show that each wrapper leg is *load-bearing*, the harness adds `supplement/cofactor-edges.json` (generated by an independent cofactorless Python oracle, not frozen, not in this directory so the Rust inventory test is unaffected): validly-signed inputs on which raw Monocypher **accepts** and the wrapper must reject — small-order R (identity), non-canonical R alias of the identity (`y = p+1`), order-8 R — all three pass (raw ACCEPT, wrapper rejects with the oracle's exact variant). `id111::keyset::verify_strict` returns the identical verdict and variant on all 8 supplement rows (checked with a scratch crate, not committed).

X25519: all **11** frozen low-order inputs make raw `crypto_x25519` return the all-zero shared secret and **none** is rejected by Monocypher itself; the `ecdh()` wrapper (7-entry low-order blacklist with bit 255 masked, plus the RFC 7748 §6.1 all-zero abort) rejects 11/11 as `LowOrderPublicKey` and the 4 controls (u = 9, Alice, Bob, project key) pass with non-zero output.

## Findings (for the reviewer / spec owner — nothing outside T9's files was edited)

**F1 — `crypto_x25519` does NOT reject an all-zero result in Monocypher 4.x; spec §3 and the KB entry say it does.** Upstream `CHANGELOG.md` 4.0.0: "`crypto_x25519()` and `crypto_key_exchange()` now return `void`"; the 4.x documentation tells callers to compare against an all-zero buffer themselves. Observed: 11/11 frozen low-order inputs → raw all-zero output, no error. Spec §3 ("Monocypher's `crypto_x25519` rejects all-zero natively") and `knowledge/projects/monocypher.md` ("rejects all-zero X25519 outputs natively") are true for 3.x only. The Cat-1 core must carry the ingress blacklist and the all-zero abort itself (the harness's `ecdh()` is the shape). Suggest correcting both texts.

**F2 — Monocypher 4.x Ed25519 verification is not "strict by default"; spec §3 and the KB entry say it is.** `crypto_eddsa_check_equation` (used by `crypto_ed25519_check`): source comment "*Allow* non-cannonical encoding for A and R"; the last step multiplies by 8 ("ensures consistency with batched verification"). It does enforce `S < ℓ` and on-curve A/R. It does **not** enforce canonical A/R, small-order A/R rejection, or cofactorless equality — observed: 9/36 oracle-rejects accepted raw. The §3 wrapper in `harness.c` repairs the written predicate; the spec's phrase "Monocypher and libsodium ≥1.0.21 are strict by default" should be narrowed to libsodium, and the KB "strict-by-default on Ed25519 encodings" claim corrected.

**F3 — OPEN: the §3 predicate as written does not make Monocypher+wrapper agree with `id111` on mixed-order points.** `id111` calls dalek `verify_strict`, which is *cofactorless* (`[S]B = R + [h]A` as point equality) and rejects only small-order A/R. Monocypher's equation is cofactored. For R or A that is *not* small-order but carries a torsion component, the two disagree:

```text
  KNOWN-DELTA mixed-order R = [r]B + order-8 torsion      oracle=reject  raw Monocypher=ACCEPT  wrapped=ACCEPT
  KNOWN-DELTA mixed-order R = [r]B + order-2 torsion      oracle=reject  raw Monocypher=ACCEPT  wrapped=ACCEPT
  KNOWN-DELTA mixed-order A' = A + order-8 torsion        oracle=reject  raw Monocypher=ACCEPT  wrapped=ACCEPT
  (id111::keyset::verify_strict: Err(InvalidSignature) on all three)
```

Impact is bounded: only the key holder can mint such a signature (no forgery); but (a) a signer can produce a second, byte-different valid signature for the same message on cofactored nodes (malleability the strict predicate was meant to remove), and (b) a NodeId `A' = A + T` verifies on a Cat-1 (Monocypher) node and never on a Rust node — a partitioned identity. No frozen vector exercises it, and Monocypher's public API cannot express the missing check (it needs `[ℓ]P = O` or a cofactorless equation). Options: amend §3 to require A and R in the prime-order subgroup (dalek `is_torsion_free()`, mirrored in `id111`, own `[ℓ]P` check in the C port), or require a cofactorless C verify, or accept and document the residual. The harness reports these rows as `KNOWN-DELTA` and flips them to `[closed]` automatically if a later wrapper rejects them.

**F4 — Monocypher < 4.0.3 has a published timing leak in Ed25519 signing** (CHANGELOG 4.0.3, 2026/06/15: "Fixed timing leak vulnerability in EdDSA/Ed25519 signatures"). The pin is 4.0.3; the Cure53 audit (2020-06) covers v3.1.1, not 4.x (already stated in spec §3/§11).

**F5 — API gotcha for the port:** `crypto_ed25519_key_pair(secret_key, public_key, seed)` takes a *non-const* `seed` and wipes it. The harness copies the seed first.

## What this run does NOT show

* No Cat-1 hardware run: no ESP32-S3 / nRF52 execution, no latency, stack or flash figures (host `size` numbers would mislead; on-device benchmark firmware is a §15 build task). The KB rubric's footprint, Scorecard, release-recency and other repository gates are outside this log.
* Direction of agreement: the vectors are Rust-produced; C reproduces the deterministic outputs byte-for-byte (Ed25519 signatures, announce, attestation node_sig, the sealed packet from a pinned `esk`) and accepts/rejects every verify/open input identically. Fresh-ephemeral (random `esk`) sealing is covered only by a C-only round-trip sweep, not cross-implementation.
* The all-zero-DH abort (`NonContributory`) is unreachable by design (the low-order ingress check fires first); it is kept as defence in depth and survives mutation (documented equivalent mutant).
* `attestation.json`: only the `node_sig` leg is C's. `transport_sig` (Layer 7 seam), expiry ordering and supersedure are Rust/Layer-7 policy — the C harness checks that the node leg accepts exactly where the later stage is the expected failure.
* Constant-time behaviour, fault injection and RNG quality are not tested. This is a vector-agreement smoke test, not an audit.

## Harness self-validation (so a green run means something)

* `bash monocypher-harness/mutation-check.sh`: 13 one-line mutants of `harness.c` (drop each §3 wrapper leg, X25519 blacklist, wrong BLAKE2b nonce length, empty AEAD AD, mode-check removal, wrong announce tag, wrong attestation field order, non-hardened SLIP-0010, 2047-round PBKDF2) are all **killed** (non-zero exit); 1 equivalent mutant (drop the unreachable all-zero abort) survives as documented.
* `bash monocypher-harness/run.sh --sanitize` (ASan + UBSan, `-fno-sanitize-recover=all`): PASS; `-O0` and `-O3` builds: PASS; `harness.c` alone also compiles clean under `-std=c99 -pedantic -Wshadow -Wmissing-prototypes -Werror`.
* `run.sh` guards verified with scratch copies: an extra `test-vectors/*.json` makes it fail (inventory), a one-byte edit of a vendored file makes it fail (hash pin), a corrupted vector byte makes the harness exit non-zero.

## Reproducibility pins (bytes this log describes)

```text
dca78513428b29051a6b1e88db716f35203b7dafdbe01941a8d8f9b8e3cab634  test-vectors/ed25519-rfc8032.json
bf2b91a71ae9f75c8703f0153a9a076162739653f950c3971064f4dad65e454d  test-vectors/x25519-rfc7748.json
c3cf6c52ab5072c0adb8e87d23a64b6b45e28c8af290266f00286304a8c19bac  test-vectors/slip0010-published.json
8f654c8946b8fa01b3114308da68028cfe7bbc5d63a8767806575a2d42f293a2  test-vectors/slip0010-project-paths.json
7f6bda3d2353548b759d364de0a9989101d32d26ebc6e562a2f0d04e1b88e1c7  test-vectors/keyset-project.json
be7afeabd39a5ac4221b44f0a5be4bd2abf3d3df4b5682bec244444ac2953062  test-vectors/edge-vectors.json
4ede7c6df33df19fe0ddf9cadc21918b2d9af084d5cdde4a7b9e9a399ca3c045  test-vectors/announce.json
66b4e15e27a25a17a6ffe1585ffc92b28d0abc416a97bb4024eac3fa7b9006b2  test-vectors/sealed-packet.json
76c558dc6d118c45af1ea46a6d9f866a3056305557f5de2b27f2cd9bc65545f6  test-vectors/attestation.json
60facbbeab20cb2189b4fc2a94ad7213c762dfbb5604806b4c024859fe740ac3  monocypher-harness/harness.c
13b0264490e86c8cb7bf9b905893b15c30c28202fc94e0e54bfcf2ed99415805  monocypher-harness/run.sh
3a448a5deb3c83401617128e3aaf1456f9c28956b2268f414b067a5be64c5a67  monocypher-harness/supplement/cofactor-edges.json
25260c1a935dc9f4743f85bfd5f7381210d00aa6f432f6004e1bdea5cb8315d7  monocypher-harness/supplement/gen_supplement.py
81d3ddc1f3416e078041bd60ffa55f00904a53ca9f46a41119b50a977a5d4c08  monocypher-harness/SHA256SUMS
```

A different set of bytes needs a new log. Changing the vendored Monocypher version, any frozen vector, or `harness.c` invalidates this one.
