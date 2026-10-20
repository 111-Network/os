# Monocypher 4.0.3 — rubric smoke-test log (T9; spec §11 / D14)

The first-party smoke-test evidence for the `monocypher` entry in the knowledge base (`os/0-knowledge/projects/monocypher.md`, `### Smoke test`: who / device / date / procedure / result — format per `os/0-knowledge/README.md`). D14 designates the §14 Rust↔Monocypher-C agreement-vector run as this log. Produced by `os/2-identity/monocypher-harness/` (task T9); regenerate with `bash os/2-identity/monocypher-harness/run.sh --log <file>`.

**Gate result: PASS** — every vector file PASS, `run.sh` exit 0. This is the **2026-10-14 re-run under spec v2.1** (§3 amendment: A and R MUST be torsion-free, `[ℓ]P = O`), which **closed F3**. The first run (2026-10-13, spec v2) is kept verbatim under "History". Of the five findings, F1 and F2 (wrong §3/KB claims about 4.x) were corrected in the spec and the KB at T9 review, F3 is closed, and F4/F5 are port notes.

## Smoke test

* **Who:** identity build harness (Layer 2 task T9), first executed by the T9 build subagent; this re-run by the F3-closure subagent after the v2.1 amendment; the reviewer re-runs the gate before commit.
* **Device:** this host — the Rust dev host class (x86-64), **not a Cat-1 device**.
  `uname -a`: `Linux Alienware 6.6.87.2-microsoft-standard-WSL2 #1 SMP PREEMPT_DYNAMIC Thu Jun  5 18:30:46 UTC 2025 x86_64 x86_64 x86_64 GNU/Linux` (WSL2) · CPU: AMD Ryzen 7 5800X · `cc --version`: `cc (Ubuntu 11.4.0-1ubuntu1~22.04.3) 11.4.0` · flags `-std=c99 -O2 -Wall -Wextra -Werror`.
* **Date:** 2026-10-13 (first run, spec v2); **2026-10-14** (this run, spec v2.1).
* **Procedure:** `cd os/2-identity && bash monocypher-harness/run.sh --log <file>`. (1) `sha256sum --check` of the five vendored files against `monocypher-harness/SHA256SUMS`; (2) build `harness.c` + vendored `monocypher.c` + `optional/monocypher-ed25519.c`; (3) assert every `test-vectors/*.json` is consumed or listed n/a; (4) re-execute the frozen vectors (produced by the Rust `id111` crate) against Monocypher and demand byte-exact agreement — keygen-from-seed, Ed25519 sign (deterministic) / verify, X25519 ECDH, BIP-39 seed + SLIP-0010, §6.4 sealed-packet seal **and** open, announce, attestation node-leg, and the §3 edge/malleability suite wrapped by the strict predicate (v2.1: including the torsion-free A/R leg, whose `[ℓ]P` the harness computes itself — Monocypher's public API cannot). No network access by the harness or the build. Details: `monocypher-harness/README.md`.
* **Result:** **PASS**, 0 failures. 442 checks over 219 vector entries (11 rows incl. the non-frozen supplement); the harness run takes about 0.3 s, build + run about 4.4 s. Full output below. The strict-predicate delta (before/after the §3 wrapper) is the section "Strict-predicate wrapper delta".

## Subject under test

* Monocypher **4.0.3** from `https://monocypher.org/download/monocypher-4.0.3.tar.gz`, downloaded 2026-10-13; tarball SHA-256 `8cc9bc341a66249016db9bd70e9142d8d0aef9945973744b1ac05dbc55d8ee66` (SHA-512 and BLAKE2b match upstream's published checksum files; sources cross-checked against the GitHub tag `4.0.3`, differing only in the line-1 version stamp). Vendored: `monocypher.{c,h}` + `optional/monocypher-ed25519.{c,h}` + `LICENCE.md`, hash-pinned in `monocypher-harness/SHA256SUMS`. Full provenance: `monocypher-harness/VENDOR.md`.
* Ed25519 under test is `crypto_ed25519_*` (RFC 8032, SHA-512, from `optional/`), **not** Monocypher's core `crypto_eddsa_*` (EdDSA-BLAKE2b, which does not interoperate with dalek).
* Rust side (the oracle's producer): `cd os/2-identity && cargo test` — 167 unit + 44 conformance tests pass, 0 failed, clippy clean. Since v2.1, `id111::keyset::verify_strict` checks `is_torsion_free()` on A and R before dalek `verify_strict`. `python3 -I test-vectors/crosscheck.py` (independent re-derivation, torsion-free leg included): 424 checks, 0 failures.

## Result — full `run.sh` output (verbatim, 2026-10-14)

```text
vendor: SHA256SUMS verified (Monocypher 4.0.3, see VENDOR.md)
cc: cc (Ubuntu 11.4.0-1ubuntu1~22.04.3) 11.4.0  flags: -std=c99 -O2 -Wall -Wextra -Werror
inventory: all 14 vector files accounted for
  [edge-vectors.json] ed25519_strict: 40 vectors, 39 oracle-rejects; raw crypto_ed25519_check ACCEPTED 13 of them; wrapped accepted 0; exact variant match 40/40
  [edge-vectors.json] rejecting leg histogram: NonCanonicalPublicKey=3 NonCanonicalR=4 NonCanonicalS=4 MalformedPublicKey=1 InvalidSignature(small-order/torsion/undecodable R/equation)=27
  [attestation.json] node-leg executed for 11 entries whose expected outcome is decided at a later stage (transport leg = Layer 7 seam, expiry, supersedure); expiry[] x5 skipped (epoch policy, no crypto)
  KNOWN-DELTA mixed-order R = [r]B + order-8 torsion, S = r + h*a: oracle=reject raw Monocypher=ACCEPT wrapped=reject  [closed]
  KNOWN-DELTA mixed-order R = [r]B + order-2 torsion, S = r + h*a: oracle=reject raw Monocypher=ACCEPT wrapped=reject  [closed]
  KNOWN-DELTA mixed-order A' = A + order-8 torsion (attacker-chosen NodeId), honest signing key a: oracle=reject raw Monocypher=ACCEPT wrapped=reject  [closed]

FILE                                       VECTORS  CHECKS   PASS  FAIL  SKIP  RESULT
(built-in primitive KATs)                        8      19     19     0     0  PASS
ed25519-rfc8032.json                             4      32     32     0     0  PASS
x25519-rfc7748.json                              4      11     11     0     0  PASS
slip0010-published.json                         12      36     36     0     0  PASS
slip0010-project-paths.json                      4       8      8     0     0  PASS
keyset-project.json                              2      12     12     0     0  PASS
edge-vectors.json                               64     141    141     0     0  PASS
announce.json                                   45      82     82     0     0  PASS
sealed-packet.json                              27      41     41     0     0  PASS
attestation.json                                41      46     46     0     5  PASS
supplement/cofactor-edges.json (non-frozen)       8      14     14     0     0  PASS
TOTAL                                          219     442    442     0     5  PASS
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
  36   mixed-order R = [r]B + order-8 torsion, S = r + h*a            reject ACCEPT reject   InvalidSignature
  37   mixed-order R = [r]B + order-2 torsion, S = r + h*a            reject ACCEPT reject   InvalidSignature
  38   mixed-order A' = A + order-8 torsion, h mod 8 != 0             reject ACCEPT reject   InvalidSignature
  39   mixed-order A' = A + order-8 torsion, h ground to 0 mod 8      reject ACCEPT reject   InvalidSignature
  -> 13 of 40 vectors: raw Monocypher verdict differs from the oracle (every one is repaired by the wrapper)
X25519 DELTA (edge-vectors.json x25519_low_order): 11 low-order inputs; raw crypto_x25519 returned all-zero for 11 and a non-zero value for 0 (Monocypher 4.x has no native rejection; ingress blacklist + all-zero abort are mandatory)
SUPPLEMENT (cofactor-edges.json, non-frozen): 8 rows = 1 control + 3 wrapper-bites + 1 wrapper-redundant + 3 residual-mixed-order;
  residual rows: raw Monocypher accepted 3/3, wrapper accepted 0/3 (strict oracle rejects all) -> CLOSED (spec v2.1 torsion-free A/R leg)
run.sh: OK -- every file PASS
```

Reading guide: *VECTORS* = frozen entries executed (must equal the file's array lengths — a mismatch is itself a FAIL); *CHECKS* = individual byte/verdict assertions; *SKIP* = entries deliberately not executed (the 5 `attestation.json` `expiry[]` entries: a mesh-epoch comparison, no crypto). `(built-in primitive KATs)` = BLAKE2b-512/192, SHA-512, HMAC-SHA-512 (RFC 4231) and BIP-39 PBKDF2 known answers, the decode probe, and (v2.1) the `[ℓ]P` reference self-test: base-point round-trip, `[ℓ]B = O`, `[s]B` equal to Monocypher's `crypto_eddsa_scalarbase(s)` for six scalars, `B + T8` mixed-order and `[8](B + T8)` torsion-free. The `supplement/` row is harness-local and not frozen. The `KNOWN-DELTA` lines keep the supplement's historical kind name; `[closed]` means the v2.1 wrapper rejects the row, and since v2.1 a row that is not closed fails the run.

## Strict-predicate wrapper delta (before / after)

Monocypher's `crypto_ed25519_check` (cofactored, accepts non-canonical A/R) is wrapped by `strict_verify()` in the §3 order used by `id111::keyset::verify_strict`: canonical A → canonical R → `S < ℓ` → A decodes → R decodes → small-order R/A rejected → torsion-free A, then R (`[ℓ]P = O`, v2.1) → Monocypher equation. `edge-vectors.json` is the oracle.

| | raw `crypto_ed25519_check` | §3 wrapper (v2.1) |
|---|---|---|
| Ed25519 vectors that the oracle rejects (39 of 40) | **accepts 13** | accepts **0** |
| accept/reject agreement with the oracle | 27 / 40 | **40 / 40** |
| exact `VerifyError` variant agreement | not available | **40 / 40** |

The 13 raw acceptances are the 9 validly-formed *small-order A* cases (`R = identity, S = 0`; rows 21–29) and the 4 *mixed-order* rows frozen on 2026-10-14 (rows 36–39). For a small-order `A`, `[8h]A = O` for every `h`, so the cofactored equation always holds. For a mixed-order point, the `[8]` multiplication erases the torsion component, so the cofactored equation cannot see it. Every other frozen edge vector (non-canonical S/R/A, small-order R, off-curve A, wrong message/key, bit flips) is *already* rejected by Monocypher's own equation, because those vectors are built by swapping one field of RFC 8032 TEST 2 and so are false under any verifier. Rejecting leg across the 39 oracle-rejects: NonCanonicalPublicKey 3 · NonCanonicalR 4 · NonCanonicalS 4 · MalformedPublicKey 1 · InvalidSignature 27.

Because most frozen vectors cannot show that each wrapper leg is *load-bearing*, the harness adds `supplement/cofactor-edges.json` (generated by an independent cofactorless Python oracle, v2.1 predicate, not frozen, not in this directory so the Rust inventory test is unaffected): validly-signed inputs on which raw Monocypher **accepts** and the wrapper must reject — small-order R (identity), non-canonical R alias of the identity (`y = p+1`), order-8 R, and the three mixed-order rows — all six pass (raw ACCEPT, wrapper rejects with the oracle's exact variant). `id111::keyset::verify_strict` (v2.1) returns the identical verdict and variant on all 8 supplement rows (re-checked 2026-10-14 with a scratch test, not committed). The frozen mixed-order rows make the torsion legs load-bearing without the supplement: `mutation-check.sh` (which runs without it) kills both.

X25519: all **11** frozen low-order inputs make raw `crypto_x25519` return the all-zero shared secret and **none** is rejected by Monocypher itself; the `ecdh()` wrapper (7-entry low-order blacklist with bit 255 masked, plus the RFC 7748 §6.1 all-zero abort) rejects 11/11 as `LowOrderPublicKey` and the 4 controls (u = 9, Alice, Bob, project key) pass with non-zero output.

## Findings (for the reviewer / spec owner)

**F1 — `crypto_x25519` does NOT reject an all-zero result in Monocypher 4.x; spec §3 and the KB entry say it does.** Upstream `CHANGELOG.md` 4.0.0: "`crypto_x25519()` and `crypto_key_exchange()` now return `void`"; the 4.x documentation tells callers to compare against an all-zero buffer themselves. Observed: 11/11 frozen low-order inputs → raw all-zero output, no error. Spec §3 ("Monocypher's `crypto_x25519` rejects all-zero natively") and `0-knowledge/projects/monocypher.md` ("rejects all-zero X25519 outputs natively") are true for 3.x only. The Cat-1 core must carry the ingress blacklist and the all-zero abort itself (the harness's `ecdh()` is the shape). Suggest correcting both texts. *Status: corrected in spec §3 and the KB at T9 review (2026-10-13).*

**F2 — Monocypher 4.x Ed25519 verification is not "strict by default"; spec §3 and the KB entry say it is.** `crypto_eddsa_check_equation` (used by `crypto_ed25519_check`): source comment "*Allow* non-cannonical encoding for A and R"; the last step multiplies by 8 ("ensures consistency with batched verification"). It does enforce `S < ℓ` and on-curve A/R. It does **not** enforce canonical A/R, small-order A/R rejection, or cofactorless equality — observed: 9/36 oracle-rejects accepted raw. The §3 wrapper in `harness.c` repairs the written predicate; the spec's phrase "Monocypher and libsodium ≥1.0.21 are strict by default" should be narrowed to libsodium, and the KB "strict-by-default on Ed25519 encodings" claim corrected. *Status: corrected in spec §3 and the KB at T9 review (2026-10-13).*

**F3 — CLOSED 2026-10-14 (spec v2.1, owner decision option (a)).** Original finding, kept as written on 2026-10-13: *the §3 predicate as written does not make Monocypher+wrapper agree with `id111` on mixed-order points.* `id111` calls dalek `verify_strict`, which is *cofactorless* (`[S]B = R + [h]A` as point equality) and rejects only small-order A/R. Monocypher's equation is cofactored. For R or A that is *not* small-order but carries a torsion component, the two disagree:

```text
  KNOWN-DELTA mixed-order R = [r]B + order-8 torsion      oracle=reject  raw Monocypher=ACCEPT  wrapped=ACCEPT
  KNOWN-DELTA mixed-order R = [r]B + order-2 torsion      oracle=reject  raw Monocypher=ACCEPT  wrapped=ACCEPT
  KNOWN-DELTA mixed-order A' = A + order-8 torsion        oracle=reject  raw Monocypher=ACCEPT  wrapped=ACCEPT
  (id111::keyset::verify_strict: Err(InvalidSignature) on all three)
```

Impact is bounded: only the key holder can mint such a signature (no forgery); but (a) a signer can produce a second, byte-different valid signature for the same message on cofactored nodes (malleability the strict predicate was meant to remove), and (b) a NodeId `A' = A + T` verifies on a Cat-1 (Monocypher) node and never on a Rust node — a partitioned identity. No frozen vector exercises it, and Monocypher's public API cannot express the missing check (it needs `[ℓ]P = O` or a cofactorless equation). Options: amend §3 to require A and R in the prime-order subgroup (dalek `is_torsion_free()`, mirrored in `id111`, own `[ℓ]P` check in the C port), or require a cofactorless C verify, or accept and document the residual. The harness reports these rows as `KNOWN-DELTA` and flips them to `[closed]` automatically if a later wrapper rejects them.

*Closure (2026-10-14).* The owner chose option (a): spec §3 now requires A and R torsion-free (`[ℓ]P = O`), so mixed-order points are rejected everywhere.

* **Rust:** `id111::keyset::verify_strict` decompresses R and rejects with `InvalidSignature` unless `VerifyingKey::to_edwards().is_torsion_free()` and R's `is_torsion_free()` both hold, then calls dalek `verify_strict`. Every wire verify path (announce, attestation node leg, commissioning challenge, tagged artifacts) goes through this one function.
* **C:** `strict_verify()` gains two legs after the small-order leg, `point_torsion_free(pk)` then `point_torsion_free(sig)`. Monocypher 4.0.3's public API cannot express `[ℓ]P`. `crypto_eddsa_check_equation` multiplies by 8, `crypto_eddsa_scalarbase` is fixed-base, `crypto_eddsa_mul_add` is scalar-only, and the X25519 functions clamp to multiples of 8. So `harness.c` carries a small self-contained Edwards implementation, a host reference validated against `crypto_eddsa_scalarbase` at start-up. The vendored sources are untouched (`SHA256SUMS` unchanged). The production Cat-1 core owns its own `[ℓ]P`.
* **Correction to impact (b) above:** "never on a Rust node" was wrong for ground signatures. With `A' = A + T8` and `h ≡ 0 (mod 8)`, `[h]T8 = O` and the cofactorless equation holds. dalek `verify_strict` alone therefore accepted the ghost NodeId, one nonce in eight. Frozen row 39 is that case. Before the Rust change `cargo test` failed on it (`left: "ok"`). After it, both implementations reject.
* **Load-bearing legs:** in Rust the A leg is load-bearing, and a mutant without it fails two unit tests. The R leg is implied by the cofactorless equation once A is torsion-free, so a mutant without it survives (documented in the code). In C both legs are load-bearing, and both mutants are killed.
* **Vectors:** the three `KNOWN-DELTA` constructions are frozen byte-for-byte as `edge-vectors.json` rows 36–38, and the ground row is row 39, all `err:InvalidSignature`. The supplement rows now print `[closed]` and gate the run.
* **Cost (host ratios, not Cat-1 figures):** one `[ℓ]P` on Monocypher's own point arithmetic costs about 0.7–0.9× one `crypto_ed25519_check`. The measurement used a scratch benchmark, not committed: plain double-and-add 0.82–0.94×, width-4 sliding window 0.67–0.81×, three runs under WSL2 noise. A verify with both legs therefore costs about 2.4–2.6×. If the A leg is memoised per pinned NodeId, it costs about 1.7–1.8×. The harness's own reference `[ℓ]P` costs about 6× a verify (TweetNaCl-class arithmetic); that is fine for the host gate and says nothing about Cat 1.

**F4 — Monocypher < 4.0.3 has a published timing leak in Ed25519 signing** (CHANGELOG 4.0.3, 2026/06/15: "Fixed timing leak vulnerability in EdDSA/Ed25519 signatures"). The pin is 4.0.3; the Cure53 audit (2020-06) covers v3.1.1, not 4.x (already stated in spec §3/§11).

**F5 — API gotcha for the port:** `crypto_ed25519_key_pair(secret_key, public_key, seed)` takes a *non-const* `seed` and wipes it. The harness copies the seed first.

## What this run does NOT show

* No Cat-1 hardware run: no ESP32-S3 / nRF52 execution, no latency, stack or flash figures (host `size` numbers would mislead; on-device benchmark firmware is a §15 build task). The KB rubric's footprint, Scorecard, release-recency and other repository gates are outside this log. The torsion-free leg's cost is a host ratio only.
* The harness's `[ℓ]P` is a variable-time host reference, not the Cat-1 core's implementation (spec §3: the C core computes `[ℓ]P` itself).
* Direction of agreement: the vectors are Rust-produced; C reproduces the deterministic outputs byte-for-byte (Ed25519 signatures, announce, attestation node_sig, the sealed packet from a pinned `esk`) and accepts/rejects every verify/open input identically. Fresh-ephemeral (random `esk`) sealing is covered only by a C-only round-trip sweep, not cross-implementation.
* The all-zero-DH abort (`NonContributory`) is unreachable by design (the low-order ingress check fires first); it is kept as defence in depth and survives mutation (documented equivalent mutant).
* `attestation.json`: only the `node_sig` leg is C's. `transport_sig` (Layer 7 seam), expiry ordering and supersedure are Rust/Layer-7 policy — the C harness checks that the node leg accepts exactly where the later stage is the expected failure.
* Constant-time behaviour, fault injection and RNG quality are not tested. This is a vector-agreement smoke test, not an audit.

## Harness self-validation (so a green run means something)

* `bash monocypher-harness/mutation-check.sh` (2026-10-14): 17 one-line mutants of `harness.c` are all **killed** (non-zero exit). They drop each §3 wrapper leg (including the v2.1 torsion-free A and R legs), multiply by 8 instead of ℓ in `[ℓ]P`, drop the √−1 branch of the reference decoder, drop the X25519 blacklist, and break the seal nonce length, the AEAD AD, the mode check, the announce tag, the attestation field order, SLIP-0010 hardening and the PBKDF2 round count. 1 equivalent mutant (drop the unreachable all-zero abort) survives as documented.
* `bash monocypher-harness/run.sh --sanitize` (ASan + UBSan, `-fno-sanitize-recover=all`): PASS; `-O0` and `-O3` builds: PASS; `harness.c` alone also compiles clean under `-std=c99 -pedantic -Wshadow -Wmissing-prototypes -Werror` (all re-checked 2026-10-14).
* `run.sh` guards verified with scratch copies on 2026-10-13 (`run.sh` unchanged since): an extra `test-vectors/*.json` makes it fail (inventory), a one-byte edit of a vendored file makes it fail (hash pin), a corrupted vector byte makes the harness exit non-zero.

## Reproducibility pins (bytes this log describes)

```text
dca78513428b29051a6b1e88db716f35203b7dafdbe01941a8d8f9b8e3cab634  test-vectors/ed25519-rfc8032.json
bf2b91a71ae9f75c8703f0153a9a076162739653f950c3971064f4dad65e454d  test-vectors/x25519-rfc7748.json
c3cf6c52ab5072c0adb8e87d23a64b6b45e28c8af290266f00286304a8c19bac  test-vectors/slip0010-published.json
8f654c8946b8fa01b3114308da68028cfe7bbc5d63a8767806575a2d42f293a2  test-vectors/slip0010-project-paths.json
7f6bda3d2353548b759d364de0a9989101d32d26ebc6e562a2f0d04e1b88e1c7  test-vectors/keyset-project.json
f6f0e46fa5b87805b5e1b82217d6d04c8478d5bf473fa1374eaf38a9c874ebab  test-vectors/edge-vectors.json
4ede7c6df33df19fe0ddf9cadc21918b2d9af084d5cdde4a7b9e9a399ca3c045  test-vectors/announce.json
66b4e15e27a25a17a6ffe1585ffc92b28d0abc416a97bb4024eac3fa7b9006b2  test-vectors/sealed-packet.json
76c558dc6d118c45af1ea46a6d9f866a3056305557f5de2b27f2cd9bc65545f6  test-vectors/attestation.json
9fb42259c6e14b8b944353209987029efba89bbf5a3e59de3512197a34e164d9  monocypher-harness/harness.c
13b0264490e86c8cb7bf9b905893b15c30c28202fc94e0e54bfcf2ed99415805  monocypher-harness/run.sh
6dc9df02b8aad9fdd8218c38d676d9508f8e6512d381b96da9bd1a07a5ebc9e6  monocypher-harness/mutation-check.sh
310671168af9a8f0f368fefb3b97d89cd292a092cbdb1262b08b1a8530ca83a7  monocypher-harness/supplement/cofactor-edges.json
8c693d6eb45511413e15e591f594790c055f3608c4896aced8bf852e9fba0753  monocypher-harness/supplement/gen_supplement.py
81d3ddc1f3416e078041bd60ffa55f00904a53ca9f46a41119b50a977a5d4c08  monocypher-harness/SHA256SUMS
```

A different set of bytes needs a new log. Changing the vendored Monocypher version, any frozen vector, or `harness.c` invalidates this one.

## History — first run, 2026-10-13 (spec v2; superseded by the run above)

Gate PASS: 411 checks over 214 entries. `edge-vectors.json` then had 36 Ed25519 rows, of which raw Monocypher accepted 9 oracle-rejects and the wrapper 0. The three supplement mixed-order rows were accepted by both raw and wrapped (F3 OPEN). `mutation-check.sh` killed 13 of 13 mutants with 1 equivalent survivor. Rust side: 161 unit + 44 conformance tests. Verbatim output:

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

Pins of that run:

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
