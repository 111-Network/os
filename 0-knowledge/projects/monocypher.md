---
title: Monocypher
category: project
status: testing
summary: Audited compact C crypto for Cat 1; single maintainer, CC0 forkability recorded.
tags: [identity, hardware, offline]
platforms: [esp32, nrf52, linux, android]
upstream: https://github.com/LoupVaillant/Monocypher
license: CC0-1.0 OR BSD-2-Clause
last_verified: 2026-10-14
sources:
  - https://cure53.de/pentest-report_monocypher.pdf
  - https://www.opentech.fund/security-safety-audits/monocypher/
  - https://monocypher.org
  - https://loup-vaillant.fr/articles/monocypher-4
  - https://github.com/LoupVaillant/Monocypher
relations:
  alternative_to: ["[[ed25519-dalek]]", "[[libsodium]]"]
---

Audited compact C crypto for Cat 1; single maintainer, CC0 forkability recorded.

## What it is

Monocypher is a compact, dependency-free C99 cryptography library covering
Ed25519 signatures, X25519 ECDH, XChaCha20-Poly1305, and BLAKE2b in a
single-file pair, dual-licensed CC0-1.0 or BSD-2-Clause. The 4.x line is
current (4.0.3 reported as of 2026-10). It is the only candidate that is
both third-party-audited and feasible on our Cat 1 chips.

## Use when

- Cat 1 targets (ESP32-S3, nRF52) need Ed25519 and X25519 in about
  10-13KB of flash with roughly 3-5KB of stack and no heap allocation.
- A third-party audit of the crypto is mandatory and vendoring a pinned,
  hash-verified copy is acceptable practice.
- The caller owns the strict wire predicate and the X25519 all-zero
  abort (the 4.x API is permissive, see Details); the primitives
  themselves agree byte-for-byte with ed25519-dalek on every vector.

## Do NOT use when

- Bus factor above one is required with no vendoring or succession plan:
  the project is essentially a single maintainer.
- The audit must cover the exact running version: Cure53 reviewed v3.1.1,
  while the 4.x line is a deliberate 2024 clean-break API redesign.
- A full TLS-style or high-level protocol API is wanted; this is
  primitives only, with composition left to the caller.

## Details

Cure53 audited v3.1.1 in June 2020 (Open Tech Fund funded, six
person-days): four low/medium findings plus two miscellaneous ones, none
high or critical, with the code called exceptionally clean. Version 4
reworked the API to close audit-era weaknesses, including an unsafe
signature API that could leak private keys; TrustInSoft and Frama-C
formal-methods CI on the 4.x line bridges the audit-version gap.
Independent measurements put Ed25519 verify near 45 ms on Cortex-M4
class hardware with about 5KB of stack, roughly twenty times faster than
TweetNaCl; benchmarks on our exact boards remain a build task.

Two 4.x behaviours the smoke test established (both differ from 3.x and
from what earlier drafts of this entry claimed): `crypto_x25519` returns
void and does not reject an all-zero shared secret, and
`crypto_ed25519_check` is cofactored and accepts non-canonical or
small-order A and R. The identity spec's section 3 wrapper (canonical
A/R, S below the group order, small-order rejection, torsion-free A and
R, low-order X25519 blacklist plus all-zero abort) is therefore
mandatory in any port. The cofactored check also accepts mixed-order
points (a torsion component, not small-order), so since spec v2.1 the
wrapper requires A and R in the prime-order subgroup. The public API
cannot compute that check, so a port carries its own scalar
multiplication by the group order. With the full wrapper the library
matches the Rust identity core on 40 of 40 edge vectors, including four
mixed-order rows that raw Monocypher accepts. Releases
before 4.0.3 carry a published Ed25519 signing timing leak (fixed
2026-06-15); pin 4.0.3 or later.

Adopted as the Cat 1 primitive source by the identity layer, vendored and
hash-pinned. The identity spec's D14 vendored-primitive exception relaxes
the bus-factor gate for this entry: single maintainer, CC0 licensed, and
forkability recorded.

## Evidence

### Alternatives considered

- [[libsodium]] — operationally strong and boring, but no comprehensive
  third-party audit, and heavier than needed on nRF52-class parts.
- [[ed25519-dalek]] — the Rust mainline for Cat 2/4; wrong language for
  the C Cat-1 core, and its Quarkslab audit was marginal.
- mbedTLS — ships inside ESP-IDF but still has no Ed25519 software
  support as of TF-PSA-Crypto 1.0, a decisive gap for this role.
- Nordic ocrypto — pragmatic vendor path on nRF52, but a proprietary
  blob with no public third-party audit.
- 2026-10-12 — desk research sweep against the Layer 2 research
  findings; entered at testing per the spec's D14 vendored-primitive
  exception, with the smoke log planned as described below.
- 2026-10-13 — T9 harness run landed: 4.0.3 vendored and hash-pinned,
  411 of 411 cross-vector checks pass on the Rust dev host; the two
  wrong claims above were corrected and the mixed-order residual
  recorded.
- 2026-10-14 — mixed-order residual closed by the identity spec v2.1
  torsion-free rule: the harness wrapper rejects all four frozen
  mixed-order rows; 442 of 442 checks pass. On the host, each
  group-order check costs about 0.7 to 0.9 of a Monocypher verify.

### Smoke test

- Who: identity build task T9 (subagent), re-run by the Layer 2
  reviewer before commit.
- Device: Rust dev host only (x86-64, WSL2, gcc 11.4); no Cat 1 board
  yet, so no on-device footprint or latency figures.
- Date: 2026-10-13; re-run 2026-10-14 after the spec v2.1 amendment.
- Procedure: `bash os/2-identity/monocypher-harness/run.sh` verifies the
  SHA-256 pins of the vendored 4.0.3 sources, builds the harness, and
  re-executes every frozen vector file produced by the Rust core (RFC
  8032, RFC 7748, SLIP-0010, keyset, edge and malleability suite,
  announce, sealed packet, attestation node leg) against Monocypher,
  demanding byte-exact agreement. Full log:
  2-identity/test-vectors/monocypher-smoke.md.
- Result: PASS, 442 checks over 219 vector entries, 0 failures; ASan and
  UBSan clean; 17 of 17 harness mutants killed. Findings F1 to F5 in the
  log (API deltas, and the mixed-order residual F3, closed 2026-10-14)
  are reflected in Details above.

## Links

- [Upstream repository](https://github.com/LoupVaillant/Monocypher)
- [Cure53 audit report](https://cure53.de/pentest-report_monocypher.pdf)
- Alternatives: [[ed25519-dalek]], [[libsodium]]
