---
title: curve25519-dalek
category: project
status: candidate
summary: Rust Curve25519 group arithmetic behind the dalek signing and ECDH crates.
tags: [identity, offline]
platforms: [linux, android, esp32, nrf52]
upstream: https://github.com/dalek-cryptography/curve25519-dalek
license: BSD-3-Clause
last_verified: 2026-10-12
sources:
  - https://rustsec.org/advisories/RUSTSEC-2024-0344.html
  - https://blog.quarkslab.com/security-audit-of-dalek-libraries.html
  - https://github.com/dalek-cryptography/curve25519-dalek
relations:
  required_by: ["[[ed25519-dalek]]"]
---

Rust Curve25519 group arithmetic behind the dalek signing and ECDH crates.

## What it is

curve25519-dalek is the Rust Curve25519 group-operations library,
licensed BSD-3-Clause. It underpins both [[ed25519-dalek]] signatures and
the x25519-dalek ECDH crate used by the identity layer's Rust core, and
it is no_std capable for embedded Rust targets.

## Use when

- Rust code needs constant-time Curve25519 arithmetic shared by signing
  and Diffie-Hellman crates.
- A single audited-lean group layer should back both the Ed25519 and
  X25519 halves of the dual-keyset design.
- no_std embedded Rust operation on ESP32-S3 or nRF52 is in scope.

## Do NOT use when

- The target is C firmware; [[monocypher]] provides the same curve work
  there in a fraction of the code size.
- The task needs a ready-made ECDH API; this crate is group arithmetic,
  and the x25519-dalek wrapper is the ECDH surface.

## Details

Quarkslab's June 2019 audit of the dalek libraries gave curve25519-dalek
a full review, with no critical findings. The crate later shipped
RUSTSEC-2024-0344: timing variability in the Scalar29 and Scalar52
subtraction routines from an LLVM-inserted branch, patched in version
4.1.3. The identity spec pins 4.1.3 as the floor and requires vendoring
with hash verification; the x25519-dalek contributory check on the
all-zero shared secret is opt-in and must be enabled, per the spec's
verification predicate.

## Evidence

### Alternatives considered

- [[monocypher]] — the C equivalent for Cat 1, audited and tiny; chosen
  there, while this crate backs the Rust world.
- [[libsodium]] — C Curve25519 with a high-level API; larger surface and
  no comprehensive audit.
- 2026-10-12 — desk research sweep; entered at candidate per the rubric.

## Links

- [Upstream repository](https://github.com/dalek-cryptography/curve25519-dalek)
- [RUSTSEC-2024-0344 advisory](https://rustsec.org/advisories/RUSTSEC-2024-0344.html)
- Required by: [[ed25519-dalek]]
