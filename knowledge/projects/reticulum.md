---
title: Reticulum
category: project
status: candidate
summary: Sovereign end-to-end encrypted networking stack; custom license needs legal review
tags: [mesh, networking, offline]
platforms: [linux, android, macos, windows]
upstream: https://github.com/markqvist/Reticulum
license: LicenseRef-Reticulum
last_verified: 2026-10-12
sources:
  - https://reticulum.network
  - https://github.com/markqvist/Reticulum
relations:
  alternative_to: ["[[meshtastic]]"]
---

Sovereign end-to-end encrypted networking stack; custom license needs legal review

## What it is

Reticulum is a complete networking stack for sovereign and off-grid
communications.
It provides cryptographic identities and end-to-end encryption
(Ed25519 and X25519, with forward secrecy) as one universal layer,
independent of the transport underneath.

## Use when

- Building offline-first systems that must move data across whatever link
  exists: LoRa via RNode hardware, TCP/UDP, WiFi, serial links, or I2P.
- Designing a hub-plus-relay node where a dumb radio peripheral owns the
  physical link and the host does addressing, crypto, and routing.
- Delay-tolerant messaging is required: LXMF builds store-and-forward in
  by design, so links can drop without losing the message flow.

## Do NOT use when

- The deployment depends on a large install base or a broad app ecosystem
  around the protocol; the community is smaller than [[meshtastic]].
- Derivative work needs clean licensing: the custom Reticulum License needs
  a legal read first, even though the protocol itself was dedicated to the
  public domain in 2016.
- Single-maintainer risk is unacceptable: the GitHub repos are mirrors of
  private development, so bus factor is a structural concern.

## Details

- Component versions reported as of 2026-10: RNS 1.3.5, LXMF 1.0.1,
  NomadNet 1.4.4.
- The reference implementation is a Python 3 userland stack; it runs on
  anything with Python 3, including the Android build.
- Host-radio split: RNode is a dumb LoRa KISS modem, while the host owns
  addressing, encryption, and routing. This matches hub-plus-relay OS
  designs directly.
- Community implementations exist: microReticulum (C++) and Reticulum-Go.
- The project has been in continuous development since 2016, ten years as
  of this entry.
- The live world map at rmap.world shows a growing node population.

## Evidence

### Alternatives considered

- [[meshtastic]] — larger community and app ecosystem, but firmware-level
  mesh radio rather than a full encrypted networking stack with
  cryptographic identities and transport-agnostic routing.
- 2026-10-12: desk research from the project site and the GitHub mirror;
  versions and activity figures reported as of 2026-10.

## Links

- https://reticulum.network
- https://github.com/markqvist/Reticulum
