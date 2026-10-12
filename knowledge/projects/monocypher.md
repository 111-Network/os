---
title: Monocypher
category: project
status: testing
summary: Audited compact C crypto for Cat 1; single maintainer, CC0 forkability recorded.
tags: [identity, hardware, offline]
platforms: [esp32, nrf52, linux, android]
upstream: https://github.com/LoupVaillant/Monocypher
license: CC0-1.0 OR BSD-2-Clause
last_verified: 2026-10-12
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
- Strict wire verification matters: the library is strict-by-default on
  Ed25519 encodings and rejects all-zero X25519 outputs natively.

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

### Smoke test

Planned per the identity spec's section 11 and D14: the section 14
Rust-to-C agreement-vector harness (build task T9, a host runner crossing
the Rust core and the vendored Monocypher 4.x) doubles as this entry's
smoke-test log. Procedure: RFC 8032 and RFC 7748 vectors plus the
edge and malleability suite run against the vendored C library, results
recorded in identity/test-vectors/monocypher-smoke.md. Who: identity
build owner, on the Cat 1-class host plus the Rust dev host. Result:
pending the T9 harness run, which the spec designates as this entry's
first-party testing evidence.

## Links

- [Upstream repository](https://github.com/LoupVaillant/Monocypher)
- [Cure53 audit report](https://cure53.de/pentest-report_monocypher.pdf)
- Alternatives: [[ed25519-dalek]], [[libsodium]]
