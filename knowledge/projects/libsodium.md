---
title: libsodium
category: project
status: candidate
summary: Portable C crypto library; no comprehensive audit, parity and reference role only.
tags: [identity, offline, networking]
platforms: [esp32, linux, android, macos, windows]
upstream: https://github.com/jedisct1/libsodium
license: ISC
last_verified: 2026-10-12
sources:
  - https://00f.net/2025/12/30/libsodium-vulnerability/
  - https://nvd.nist.gov/vuln/detail/CVE-2025-69277
  - https://github.com/jedisct1/libsodium
---

Portable C crypto library; no comprehensive audit, parity and reference role only.

## What it is

libsodium is the modern, portable, easy-to-use C cryptography library,
ISC licensed, with a very active upstream (1.0.21 shipped January 2026,
1.0.22 June 2026). It is operationally the safest boring choice in the C
world, but it has no comprehensive third-party audit, which caps it at
candidate under the rubric for identity use.

## Use when

- A C parity or reference implementation is needed alongside the Rust
  core, for example cross-checking vectors on Cat 2/4 hosts.
- Only the high-level APIs are exercised: crypto_sign family,
  crypto_aead_xchacha20poly1305_ietf family, and crypto_generichash.
- Version 1.0.21 or newer is pinned, closing CVE-2025-69277, in which
  crypto_core_ed25519_is_valid_point failed to reject small-order points.

## Do NOT use when

- The component anchors identity assurance; the audit gap forbids that
  under the rubric's proven gate.
- Sealed boxes are wanted for the packet construction: crypto_box_seal
  is XSalsa20-Poly1305 with a different nonce derivation and is not the
  spec's sealed-packet recipe; implementations compose from the AEAD and
  generichash APIs instead.
- Cat 1 nRF52 flash budgets are tight; [[monocypher]] is smaller there.

## Details

Assurance history: the closest public signal is a PIA-funded 2017
assessment covering versions 1.0.12 and 1.0.13, partial in scope, which
the identity spec records as dated. CVE-2025-69277 (disclosed December
2025) shows even boring libraries leak point-validation bugs; the fix
landed in 1.0.21, the fleet floor. Adoption is enormous, from DNSCrypt
and Wire to PyNaCl, and it is packaged in Debian, Alpine, nixpkgs, and
Fedora, with ESP-IDF ports in the field.

In our design, libsodium is the Cat 2/4 parity reference in C and stays
out of the agreement-vector scope unless a build actually links it; the
assurance anchor for identity is the audited primitive layer, not this
library.

## Evidence

### Alternatives considered

- [[monocypher]] — audited, smaller, and the Cat 1 pick; chosen where an
  audit is mandatory.
- [[ed25519-dalek]] — the Rust mainline for the 111d world; not a C
  reference.
- 2026-10-12 — desk research sweep; entered at candidate per the rubric,
  audit gap recorded.

## Links

- [Upstream repository](https://github.com/jedisct1/libsodium)
- [CVE-2025-69277 writeup](https://00f.net/2025/12/30/libsodium-vulnerability/)
- [NVD entry](https://nvd.nist.gov/vuln/detail/CVE-2025-69277)
