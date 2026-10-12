---
title: EDHOC
category: protocol
status: candidate
summary: Lightweight authenticated key exchange for constrained links; young ecosystem.
tags: [identity, networking, offline]
upstream: https://datatracker.ietf.org/doc/html/rfc9528
last_verified: 2026-10-12
sources:
  - https://datatracker.ietf.org/doc/html/rfc9528
  - https://www.rfc-editor.org/info/rfc9528
  - https://github.com/openwsn-berkeley/libedhoc
---

Lightweight authenticated key exchange for constrained links; young ecosystem.

## What it is

EDHOC, standardized as RFC 9528 in March 2024 by the IETF LAKE working
group, is an authenticated key exchange designed for constrained devices.
A complete handshake runs 101 to 242 bytes, CBOR and COSE encoded, and it
bootstraps OSCORE security contexts. It is the most formally analyzed
lightweight AKE available, with more than five independent formal
verification studies behind it.

## Use when

- A standardized online mutual-authentication handshake is needed on
  constrained links; measurements show 6 to 14 times fewer over-air bytes
  than DTLS 1.3, about 7.75 times in the raw-public-key configuration,
  with roughly a quarter of the flash and RAM.
- Credentials arrive by reference, which maps onto announce-distributed
  public keys.
- Post-quantum-ready variants matter; they are beginning to appear.

## Do NOT use when

- Deployment maturity is required; adoption is academic and early
  industrial pilots, and implementation audits are scarce in the young
  ecosystem.
- v1 needs it today; the identity spec keeps EDHOC on the watch list for
  constrained online handshakes and pins Noise IK for the online-link
  upgrade in the meantime.
- COSE machinery is unwanted; the encoding pulls in CBOR.

## Details

The identity spec's section 6.6 records EDHOC as the constrained-handshake
standards track on watch, deferred with its formal-verification pedigree
noted; activation would ride a future transport decision rather than the
v1 identity build. Confidence on deployment maturity is low, stated
plainly by the research.

## Evidence

### Alternatives considered

- [[noise-framework]] — production-proven patterns and implementations
  with broader deployment, but no standards body and unaudited Rust
  stacks.
- DTLS 1.3 with raw public keys — standardized but roughly an order of
  magnitude heavier on the wire.
- 2026-10-12 — desk research sweep; entered at candidate on watch per
  the spec, under the protocol adaptation: stable RFC with early
  independent implementations.

## Links

- [RFC 9528](https://datatracker.ietf.org/doc/html/rfc9528)
- [RFC editor info](https://www.rfc-editor.org/info/rfc9528)
- [libedhoc implementation](https://github.com/openwsn-berkeley/libedhoc)
