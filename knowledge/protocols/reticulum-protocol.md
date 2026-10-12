---
title: Reticulum protocol
category: protocol
status: candidate
summary: Public-domain mesh protocol design; code license non-OSI with AI-training clause.
tags: [mesh, networking, identity]
upstream: https://reticulum.network
license: LicenseRef-Reticulum
last_verified: 2026-10-12
sources:
  - https://reticulum.network
  - https://github.com/markqvist/Reticulum
  - https://github.com/markqvist/Reticulum/discussions/790
relations:
  relates_to: ["[[reticulum]]"]
---

Public-domain mesh protocol design; code license non-OSI with AI-training clause.

## What it is

The Reticulum network protocol, dedicated to the public domain by its
author in 2016, is the closest prior art to our identity architecture.
Its design: a self-generated 512-bit keyset per node, Ed25519 for
signing and X25519 for encryption, 128-bit truncated-hash destinations
over the identity hash, signed announces of about 167 bytes carrying the
full public key, delivery proofs, and per-packet ephemeral X25519 for
forward secrecy. The reference implementation is the [[reticulum]]
project.

## Use when

- Designing our own clean-room identity layer over audited primitives;
  the dual-keyset model, signed announces, and rate-capped announce
  flooding are the patterns to copy.
- A delay-tolerant, transport-agnostic mesh design is the reference
  point.

## Do NOT use when

- The code would be adopted: the April 2025 relicensing to a custom
  Reticulum License failed the OSI bar, adds a no-harm clause and a ban
  on use directly or indirectly in AI training datasets, and killed the
  Debian packaging effort.
- An audited stack is needed; no third-party security audit of Reticulum
  has ever existed.
- A governed specification is required; the Python reference
  implementation is the sole authoritative spec, and its maintainer
  announced stepping back in 2026.

## Details

The identity spec is Reticulum-shaped by explicit decision: the
dual-keyset model and announce mechanics inform sections 2 and 6, while
the code is never adopted, making this entry the design reference only.
Announce rate is capped around two percent of interface bandwidth, the
pattern behind our per-interface rate-limit handoff to the transport
layers. The protocol's public-domain dedication is what keeps clean-room
reuse safe despite the license on the implementation.

## Evidence

### Alternatives considered

- Adopting the RNS implementation — eliminated: non-OSI license with an
  AI-training clause, zero audits, Python-only, single-maintainer
  governance.
- Designing without a mesh-native reference — rejected; Reticulum's
  decade of field use is the cheapest source of hard-won design lessons.
- 2026-10-12 — desk research sweep; entered at candidate as a case-study
  protocol under the protocol adaptation, license flag recorded
  machine-visible as LicenseRef-Reticulum.

## Links

- [Protocol home](https://reticulum.network)
- [Reference implementation](https://github.com/markqvist/Reticulum)
- [Relicensing discussion](https://github.com/markqvist/Reticulum/discussions/790)
