---
title: ed25519-dalek
category: project
status: candidate
summary: Rust Ed25519 library; Quarkslab audit was marginal and predates v2.
tags: [identity, offline]
platforms: [linux, android, esp32, nrf52]
upstream: https://github.com/dalek-cryptography/ed25519-dalek
license: BSD-3-Clause
last_verified: 2026-10-12
sources:
  - https://blog.quarkslab.com/security-audit-of-dalek-libraries.html
  - https://rustsec.org/advisories/RUSTSEC-2022-0093.html
  - https://github.com/dalek-cryptography/ed25519-dalek
relations:
  depends_on: ["[[curve25519-dalek]]"]
---

Rust Ed25519 library; Quarkslab audit was marginal and predates v2.

## What it is

ed25519-dalek is the mainstream Rust Ed25519 signature library, licensed
BSD-3-Clause. Version 2 (August 2023) restructured the API after
RUSTSEC-2022-0093 and is the pinned floor for our fleet. It builds on
[[curve25519-dalek]] group arithmetic and is RFC 8032 interoperable with
[[monocypher]] and [[libsodium]], so a mixed fleet cross-verifies.

## Use when

- The node is Cat 2/3/4 in the Rust static world, where this is the
  default Ed25519 implementation with massive ecosystem adoption.
- no_std operation on embedded Rust (esp-rs, embassy) is needed.
- Strict verification is required: the v2 API's verify-strict class rules
  (canonical R and A encodings, S below the group order, small-order
  rejection) match the identity spec's pinned wire predicate.

## Do NOT use when

- The job is Cat 1 C firmware; [[monocypher]] owns that role.
- Default verify() is acceptable for wire artifacts; it is not, per the
  identity spec's verification predicate — verify-strict is mandatory.
- Exact-version audit matching is demanded: the Quarkslab audit is
  pre-v2 and covered ed25519-dalek only marginally.

## Details

Quarkslab audited the dalek libraries in June 2019 (commissioned by Tari
Labs): a full review of curve25519-dalek, subtle, and bulletproofs, but
only a marginal review of ed25519-dalek, with no critical findings.
Version 1 carried RUSTSEC-2022-0093 (CVE-2022-50237), the double public
key signing function oracle attack, in which v1 Keypair assembly let an
attacker extract private keys; the v2.0.0 API restructure excludes that
class structurally. The companion crate curve25519-dalek must be at
4.1.3 or newer (RUSTSEC-2024-0344 timing variability, fixed at that
version).

Adoption is broad across the Rust ecosystem, including Signal-adjacent
tooling, Solana, and Iroh's authentication crates. The maintainer team is
small, about two to three people. The identity spec pins v2 plus
curve25519-dalek at 4.1.3 or newer, vendored with hash verification, and
bans the default verify() for anything crossing the wire.

## Evidence

### Alternatives considered

- [[monocypher]] — audited and Cat-1 sized, but C rather than Rust, so
  it serves the embedded category while this crate serves the Rust one.
- [[libsodium]] — the boring C option; lacks a comprehensive audit and
  pulls a larger API surface than signature and ECDH duties need.
- 2026-10-12 — desk research sweep; entered at candidate per the
  three-gate rubric, with the exact-version audit gap recorded.

## Links

- [Upstream repository](https://github.com/dalek-cryptography/ed25519-dalek)
- [Quarkslab audit announcement](https://blog.quarkslab.com/security-audit-of-dalek-libraries.html)
- [RUSTSEC-2022-0093 advisory](https://rustsec.org/advisories/RUSTSEC-2022-0093.html)
- Depends on: [[curve25519-dalek]]
