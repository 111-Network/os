---
title: Bundle Protocol Version 7
category: protocol
status: candidate
summary: IETF DTN bundle protocol; clock-based lifetime and heavy fragments rule out LoRa.
tags: [dtn, networking, messaging]
upstream: https://www.rfc-editor.org/rfc/rfc9171
last_verified: 2026-10-14
sources:
  - https://www.rfc-editor.org/rfc/rfc9171
  - https://datatracker.ietf.org/doc/html/draft-ietf-dtn-btpu-03
  - https://www.ietf.org/archive/id/draft-ietf-dtn-bibect-03.txt
  - https://github.com/dtn7/dtn7-rs
relations:
  implemented_by: ["[[dtn7-rs]]"]
  relates_to: ["[[store-and-forward-states]]"]
---

IETF DTN bundle protocol; clock-based lifetime and heavy fragments rule out LoRa.

## What it is

BPv7 (RFC 9171) is the IETF store-and-forward protocol for delay-tolerant networks.
A bundle has a primary block, optional extension blocks, and a payload, encoded in CBOR.
Layer 5 does not use it as the fabric core. It stays a reference and a possible later gateway format.
RFC 9171 is Standards Track; no code license applies to the specification itself.

## Use when

- Interop with a DTN gateway is wanted later: wrap a 111 envelope as a bundle payload.
- Designing identity or fragmentation: bundle identity (source, creation time, sequence number) and the
  no-clock rule are the precedent behind the envelope's `msg_seq`.

## Do NOT use when

- It would be the core on LoRa: lifetime is mandatory and counted from a creation time.
- Frames are small: each fragment carries its own 37 to 57 byte primary block (computed).
- Custody transfer is assumed: RFC 9171 left it to the separate BIBE draft.

## Details

Why not the core, in four facts:
1. Lifetime is mandatory, in milliseconds from creation time; there is no "never expires" value and
   expiry means the node MUST delete. Without a clock, creation time is zero and age comes from a Bundle Age block.
2. Overhead is 37 to 57 bytes before payload (computed from hand-encoded CBOR with ipn endpoint ids, with or
   without a CRC-16 and a Bundle Age block); the primary-block CRC is required unless an integrity block covers it.
   At 184 bytes minus the 56-byte seal, 71 to 91 bytes remain for content.
3. No maintained LoRa convergence layer was found (two searches, a negative result). The IETF draft BTPU
   (4-byte headers, segmentation, repetition instead of acks) has no implementation found; the Go project
   dtn7-go has a broadcast connector naming LoRa, last released in 2022.
4. No custody in 9171 (moved to BIBE), so reliable delivery would be our own end-to-end ack over any BPv7 core.

Implementations: [[dtn7-rs]] (Rust), uD3TN (C), ION-DTN (C), HDTN (C++), Hardy (Rust).
Fragments carry an offset and the total length (RFC 9171 §5.8); the research reports differ on whether
intermediate nodes may reassemble them.

## Evidence

### Alternatives considered

- The Layer 5 envelope with `msg_seq` — chosen: no clock, 1 to 3 byte message identity, LoRa-sized.
- LXMF propagation ([[lxmf]]) — carries its own clock timestamp and identity; a bridge only.
- Epidemic, spray-and-wait, PRoPHET, and MaxProp routing — specifications, not code; deferred until hubs replicate.
- 2026-10-14 — facts read from RFC 9171 in the Layer 5 research; byte sizes are computed, not measured.

## Links

- [RFC 9171](https://www.rfc-editor.org/rfc/rfc9171)
- [BTPU draft](https://datatracker.ietf.org/doc/html/draft-ietf-dtn-btpu-03)
- Implementation: [[dtn7-rs]]; semantics: [[store-and-forward-states]]
