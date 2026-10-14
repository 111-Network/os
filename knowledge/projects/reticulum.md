---
title: Reticulum
category: project
status: candidate
summary: Sovereign end-to-end encrypted networking stack; custom license needs legal review
tags: [mesh, networking, offline]
platforms: [linux, android, macos, windows]
upstream: https://github.com/markqvist/Reticulum
license: LicenseRef-Reticulum
last_verified: 2026-10-14
sources:
  - https://reticulum.network
  - https://github.com/markqvist/Reticulum
  - https://raw.githubusercontent.com/markqvist/Reticulum/master/docs/source/interfaces.rst
  - https://github.com/liamcottle/reticulum-meshchat
relations:
  alternative_to: ["[[meshtastic]]"]
  required_by: ["[[lxmf]]", "[[meshchatx]]"]
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

No-import carriage (Layer 5; reports 01, 02, gap-1; as of 2026-10-14). 111 never imports RNS or LXMF; it
reaches Reticulum only through an adopted program's external interface:

- MeshChat HTTP and WebSocket API: `POST /api/v1/lxmf-messages/send` (content, image, audio, or file
  attachments only, no arbitrary field), `GET /api/v1/lxmf-messages/conversation/{hash}`,
  `/api/v1/lxmf/conversations`, `/api/v1/announces`, and WebSocket `/ws`. It has no authentication and
  binds `127.0.0.1:8000` by default, so 111d would front it with pairing auth. See [[meshchatx]].
- `lxmd` hook: the `on_inbound` option of its `[lxmf]` section receives the saved-message path, inbound only;
  there is no send hook. See [[lxmf]].
- `PipeInterface` runs a program and moves packets over its stdin and stdout, so rnsd rides on our links:
  the reverse of what carrying our envelope through Reticulum needs. For a radio 111 owns, the documented
  route is rnsd's `TCPClientInterface` with its `kiss_framing` option on and `fixed_mtu` set to 500, against a
  loopback KISS server. rnsd's own `KISSInterface` must not point at an RNode: its timing bytes read as radio settings.
- `rncp` moves files and `rnx` runs remote commands (rejected). The shared instance is on by default
  (ports 37428 and 37429, AF_UNIX on Linux), so clients attach to our rnsd when `share_instance` is yes.
- Independent reimplementations exist (Leviculum AGPL-3.0-or-later, Reticulum-rs MIT, go-reticulum MIT), but
  none is upstream, and D17 #12 requires the latest upstream.

## Evidence

### Alternatives considered

- [[meshtastic]] — larger community and app ecosystem, but firmware-level
  mesh radio rather than a full encrypted networking stack with
  cryptographic identities and transport-agnostic routing.
- 2026-10-12: desk research from the project site and the GitHub mirror;
  versions and activity figures reported as of 2026-10.
- 2026-10-14: Layer 5 research pass added the no-import carriage facts from interface documentation
  and the MeshChat source; nothing was run.

## Links

- https://reticulum.network
- https://github.com/markqvist/Reticulum
