---
title: MeshCore
category: project
status: candidate
summary: LoRa mesh whose Ed25519 pubkey-as-identity proves our Cat 1 shape.
tags: [mesh, lora, identity]
platforms: [esp32, nrf52]
upstream: https://github.com/meshcore-dev/MeshCore
license: MIT
last_verified: 2026-10-12
sources:
  - https://github.com/meshcore-dev/MeshCore
  - https://github.com/meshcore-dev/MeshCore/issues/1024
relations:
  alternative_to: ["[[meshtastic]]"]
---

LoRa mesh whose Ed25519 pubkey-as-identity proves our Cat 1 shape.

## What it is

MeshCore is a lightweight LoRa mesh protocol and firmware family
launched in early 2025, growing fast on ESP32-S3 and nRF52 hardware. Its
identity model is the shape our spec converged on: an Ed25519 keypair
generated at first boot whose 32-byte public key IS the node address.
Adverts carry the public key, a timestamp, and an Ed25519 signature over
the whole advert at about 100 bytes, verifiable by anyone, registry-free.

## Use when

- Proving Ed25519-pubkey-as-identity runs at scale on exactly our Cat 1
  chips; MeshCore's fleet is the live demonstration.
- Looking for the leanest proven signed-advert wire form on LoRa.
- Studying which composition mistakes to avoid next to the good identity
  model.

## Do NOT use when

- Copying its crypto composition: the message layer is AES-128-ECB with
  a 2-byte truncated HMAC, and there is no key separation.
- Operational security defaults matter: the firmware once shipped with
  the admin password literally password, and the private key was
  readable through a console command.
- An audited crypto stack is required; none exists, and the vendored
  legacy ed25519 C library uses a nonstandard key format.

## Details

Advert structure: 32-byte public key plus 4-byte timestamp plus 64-byte
signature plus role flags, with the signature covering the entire advert.
Companion devices do not relay; repeaters and room servers forward and
store; up to 64 hops. Its X25519 ECDH is derived by converting the
Ed25519 key, the birational coupling our spec rejects in favor of a
sibling derived key. Performance relies on a vendored TweetNaCl-class
Ed25519, roughly 1.5 to 2 seconds to verify on Cortex-M4 class parts.

License note: the research reports conflicted on GPL-3.0 versus MIT; the
GitHub API reports an MIT license file as of 2026-10-12, and the entry
records MIT with this conflict noted per the options brief.

## Evidence

### Alternatives considered

- [[meshtastic]] — far larger community and app ecosystem, but its 2025
  identity CVE cascade (see [[meshtastic-pki]]) versus this project's
  clean signed adverts.
- [[reticulum-protocol]] — the fuller mesh-native design reference,
  reused as protocol only.
- 2026-10-12 — desk research sweep; entered at candidate as a case
  study; license re-verified against the upstream repository.

## Links

- [Upstream repository](https://github.com/meshcore-dev/MeshCore)
- [Key format issue](https://github.com/meshcore-dev/MeshCore/issues/1024)
- Alternative to: [[meshtastic]]
