---
title: ciborium
category: project
status: candidate
summary: Serde CBOR crate for Cat 2+ bodies; never in the relay path.
tags: [messaging, networking]
platforms: [linux, android, macos, windows]
upstream: https://github.com/enarx/ciborium
license: Apache-2.0
last_verified: 2026-10-14
sources:
  - https://github.com/enarx/ciborium
  - https://www.rfc-editor.org/rfc/rfc8949
---

Serde CBOR crate for Cat 2+ bodies; never in the relay path.

## What it is

ciborium is a Serde-based CBOR encoder and decoder in Rust.
Layer 5 uses it for the `MSG` body, a deterministic CBOR map (RFC 8949 §4.2):
text, attachment references, an unverified sender-claimed `sent_at`, and `reply_to`.
Relays never parse bodies, and Cat 1 nodes never see them.

## Use when

- A self-describing, evolvable body format is wanted on IP-class nodes (Cat 2 and above).
- Bodies must tolerate unknown keys across versions without a codegen step.

## Do NOT use when

- The code runs in the relay path or on a Cat 1 microcontroller; the outer header is hand-packed instead.
- Packets themselves would be wrapped in COSE; that costs about 31 bytes more per sealed packet.
- Canonical map ordering is assumed from the crate alone; the codec must enforce and test it.

## Details

Reported as of 2026-10-14: v0.2.2 (2024-01-24); repository pushed in 2026-09; license read from
`ciborium/LICENSE` (Apache-2.0).
Spec §3.3 body shape: `{1: text, 2: [attachment refs], 3: sent_at?, 4: reply_to?}`, attachments from v1.1.
An attachment reference is `{hash (32-byte BLAKE3), size, name?, object_key (32)}` and the object moves
by the bulk path (see [[blake3-bao]]).
Whether the crate emits RFC 8949 deterministic encoding by default was not verified here.
The dependency needs a recorded sign-off in the Layer 1 dependency policy before it lands.

## Evidence

### Alternatives considered

- postcard — MIT and Apache-2.0, fine for local state, but not self-describing, so brittle as an evolving wire.
- coset (COSE types, Apache-2.0) — usable for large signed manifests on Cat 2 and Cat 4 only.
- COSE for the packet — rejected: no XChaCha20 in the IANA registry, and EdDSA (-8) is now deprecated for Ed25519 (-19).
- Protobuf — rejected for the wire: code generation weight.
- 2026-10-14 — license read from upstream `ciborium/LICENSE` in the Layer 5 research (report 04);
  the maintenance signal is a repository push, not a release.

## Links

- [Upstream repository](https://github.com/enarx/ciborium)
- [RFC 8949, CBOR](https://www.rfc-editor.org/rfc/rfc8949)
