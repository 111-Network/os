---
title: Iroh NodeId
category: project
status: candidate
summary: Raw-Ed25519-pubkey NodeId proven at 200M-plus endpoints on public relays.
tags: [networking, identity]
platforms: [linux, android, macos, windows]
upstream: https://github.com/n0-computer/iroh
license: MIT OR Apache-2.0
last_verified: 2026-10-12
sources:
  - https://docs.iroh.computer/about/faq
  - https://iroh.computer/blog/v1
  - https://github.com/n0-computer/iroh
---

Raw-Ed25519-pubkey NodeId proven at 200M-plus endpoints on public relays.

## What it is

Iroh is a Rust peer-to-peer networking stack from n0 whose NodeId is the
raw 32-byte Ed25519 public key, with no hashing indirection. Connections
authenticate with TLS 1.3 raw-public-key (RFC 7250) over QUIC, dialing
keys rather than IPs, with no X.509 and no certificate authority. The
identity spec cites it as the production-scale proof that a raw-pubkey
NodeId works: 200M-plus endpoints crossed the public relays in the first
30 days after the 1.0 launch in mid-2026.

## Use when

- A Cat 2/4 application wants key-authenticated QUIC connectivity with
  self-sovereign keys and pluggable discovery.
- The design question is whether raw-pubkey addressing scales; the
  relay telemetry says it does.
- Session authentication should follow the dial-keys-not-IPs pattern on
  IP-capable links.

## Do NOT use when

- The node is Cat 1; the QUIC-plus-TLS stack far exceeds 512KB-SRAM
  budgets.
- Air-gap operation is the default: out of the box, discovery uses
  Pkarr-signed DNS on n0 infrastructure with relay fallback, so
  offline-first requires self-hosted discovery and relay configuration.
- Key revocation is needed; a new key is simply a new identity, matching
  our deferred-rotation stance but worth stating.

## Details

Iroh 1.0 shipped in June 2026 after about four years of pre-releases,
followed by a v1.1 with security fixes in August 2026. Discovery is
pluggable: Pkarr-signed DNS by default, Mainline DHT, and mDNS on LAN.
Relays are blind layer-4 forwarders, and roughly 90 percent of
connections upgrade from relay to direct UDP. No third-party security
audit surfaced for the 1.0 line at research time, flagged as low
confidence and worth verifying directly with n0 before any deeper
adoption. Dual licensed MIT or Apache-2.0.

For our layers the takeaway is the identity pattern: the public key is
the address, and transport credentials are separate keys bound to the
node, which is exactly the spec's B2 NodeId rule and its transport
attestation design.

## Evidence

### Alternatives considered

- Reticulum-style truncated-hash addressing — a locator, not the
  identity; our spec pins the raw public key instead.
- libp2p PeerID — multihash-of-protobuf indirection that adds nothing in
  an offline-first fleet.
- 2026-10-12 — desk research sweep; entered at candidate as a case
  study; no audit surfaced, recorded as an open verification item.

## Links

- [Upstream repository](https://github.com/n0-computer/iroh)
- [Iroh 1.0 announcement](https://iroh.computer/blog/v1)
- [Project FAQ](https://docs.iroh.computer/about/faq)
