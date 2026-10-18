---
title: Noise Protocol Framework
category: protocol
status: candidate
summary: Pattern-based handshake framework; patterns verified, implementations unaudited.
tags: [identity, networking, offline]
upstream: https://noiseprotocol.org
last_verified: 2026-10-12
sources:
  - https://noiseprotocol.org
  - https://noiseexplorer.com
  - https://github.com/mcginty/snow
  - https://eprint.iacr.org/2022/607
relations:
  alternative_to: ["[[edhoc]]"]
---

Pattern-based handshake framework; patterns verified, implementations unaudited.

## What it is

Noise is a public-domain framework for constructing interactive
handshake protocols from named patterns. The spec has been stable since
rev 34 (2018). Patterns are formally verified: Noise Explorer gives
Tamarin and ProVerif models, and the Noise-star project ships formally
verified C implementations of all 59 patterns. Production provenance runs
through WireGuard, WhatsApp's Noise Pipes, Lightning's Brontide, and I2P.

## Use when

- Both peers are online and a link should upgrade beyond per-packet
  encryption to a session with strong forward secrecy; the identity spec
  pins Noise IK (two messages, responder key already known from
  announces) for exactly this case.
- A pattern vocabulary is needed: XX for first contact, IK for known
  responders, with identity hiding where required.
- Cat 2/4 TCP-capable links want mutual authentication without
  certificate machinery.

## Do NOT use when

- Store-and-forward is the transport; Noise needs both peers online for
  the interactive exchange, so it is the online-link upgrade, not the
  base.
- The Rust implementation would anchor assurance: snow is explicitly
  unaudited (its RUSTSEC-2024-0011 state-machine DoS was fixed in 0.9.5),
  and clatter is unaudited too, so the rubric caps these at testing.
- A single-flight, zero-handshake model is required; per-packet
  ephemeral ECDH serves that instead.

## Details

The XX pattern costs three messages, about 192 bytes with X25519 and
ChaCha20-Poly1305, hiding both static keys; IK, WireGuard's pattern,
authenticates in two flights when the responder's key is known, which
maps directly onto announce-distributed keys. WireGuard-on-ESP32 ports
prove the primitive set and handshake state fit Cat 1 budgets at roughly
5KB of extra stack, though no standalone Noise-framework footprint study
exists. Under the rubric, the verified Noise-star C path is the clean
route upward; an implementation audit of a Rust stack is a recorded
procurement item.

## Evidence

### Alternatives considered

- [[edhoc]] — the IETF standards-track constrained alternative, leaner on
  the wire for tiny links but far younger in deployment.
- TLS 1.3 raw-public-key — standardized in RFC 7250 and supported by
  OpenSSL 3.2 onward, but with heavier machinery than a Noise IK session.
- 2026-10-12 — desk research sweep; entered at candidate under the
  protocol adaptation: stable spec plus multiple independent
  implementations, with the implementation-audit gap recorded.

## Links

- [Specification home](https://noiseprotocol.org)
- [Noise Explorer](https://noiseexplorer.com)
- [snow implementation](https://github.com/mcginty/snow)
- [Noise-star verified C](https://eprint.iacr.org/2022/607)
