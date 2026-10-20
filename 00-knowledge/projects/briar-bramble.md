---
title: Briar Bramble
category: project
status: candidate
summary: Offline-first messaging protocol family with strong introduction patterns.
tags: [mesh, identity, offline]
platforms: [android]
upstream: https://code.briarproject.org/briar/briar
license: GPL-3.0-or-later
last_verified: 2026-10-19
sources:
  - https://briarproject.org
  - https://code.briarproject.org/briar/briar
---

Offline-first messaging protocol family with strong introduction patterns.

## What it is

Briar is an offline-first messenger whose Bramble protocol family
defines identity and transport for activist and journalist users.
Identity is a self-generated long-term Ed25519 keypair with no servers,
phone numbers, or accounts. Contacts are introduced in person via QR code
or a briar:// link encoding Base32 of a zero byte plus the public key in
53 characters, statically bound to the identity.

## Use when

- Studying authenticated introduction and contact-exchange patterns done
  right; the spec bookmarks Bramble's handshake documents for the later
  human-identity layer.
- A transport-agnostic sync design is needed: Bramble sync runs over
  Bluetooth, WiFi LAN, and Tor with one protocol.
- Offline-first threat-model precedents matter; Briar runs with no WAN
  at all.

## Do NOT use when

- Key rotation is expected; there is none without a new account because
  handshake links are statically bound to the long-term key.
- The WAN path must not depend on Tor; rendezvous uses Tor hidden
  services.
- The target is machine node identity; Briar's contact model is
  human-facing identity, out of the identity layer's scope.

## Details

The Bramble Handshake Protocol performs static Diffie-Hellman on the
long-term keys, hashing both public keys into the transport keys as a
reflection-attack mitigation, with duplicate-link and small-order checks.
The identity spec cites this family normatively for its 32-entry sliding
replay windows at mesh scale and records the 2023 Briar static-handshake
key-compromise-injection finding as the reason static-static ECDH alone
is banned. Assurance history: a Cure53 audit in 2017 covered a
development version in partial scope with a recommended follow-up, and an
ETH Zürich thesis in 2023 analyzed Bramble in Briar 1.4.20; close to the
rubric bar but not a release-version full audit. GPL-3.0-or-later (the upstream `LICENSE.txt` notice grants version 3
or any later version), Android-only client. `briar-headless` (REST on loopback, a Kotlin jar) registers only the Tor
plugin on Linux, so a hub can reach Briar only online; it is documented, not built.

## Evidence

### Alternatives considered

- [[nostr]] — self-authenticating identity too, but relay-centric and
  without Briar's introduction ceremony or recovery posture.
- libsignal — service-account-centric with prekey servers, the wrong
  shape for offline nodes.
- 2026-10-12 — desk research sweep; entered at candidate as a case
  study, audits noted as partial.
- 2026-10-19 — licence notice read upstream; Layer 6 research found no offline headless path.

## Links

- [Project site](https://briarproject.org)
- [Upstream repository](https://code.briarproject.org/briar/briar)
