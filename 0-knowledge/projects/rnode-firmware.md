---
title: RNode Firmware
category: project
status: candidate
summary: GPL-3.0 LoRa modem firmware for Reticulum; flasher ships inside the rns wheel.
tags: [lora, firmware, hardware]
platforms: [esp32, nrf52]
upstream: https://github.com/markqvist/RNode_Firmware
license: GPL-3.0-only
last_verified: 2026-10-19
sources:
  - https://github.com/markqvist/RNode_Firmware
  - https://raw.githubusercontent.com/markqvist/RNode_Firmware/master/LICENSE
  - https://github.com/markqvist/RNode_Firmware/releases/latest/download/release.json
relations:
  relates_to: ["[[reticulum]]"]
---

GPL-3.0 LoRa modem firmware for Reticulum; flasher ships inside the rns wheel.

## What it is

RNode is a dumb LoRa KISS modem: the board owns the radio and the host runs Reticulum's addressing and routing.
The firmware is Copyright Mark Qvist; the README says GNU General Public License v3.0, with an MIT-licensed SX1276
driver inside.
`rnodeconf`, the tool that identifies, provisions, and flashes boards, ships inside the `rns` wheel, so the Reticulum
License covers it, not the firmware's GPL. Layer 6 adopts `rnodeconf --info` as the identify probe.

## Use when

- A board must become a Reticulum interface on a hub, driven by `rnsd`.
- A board needs identifying: the adopted probe is `rnodeconf --info` (rather than own byte probes).

## Do NOT use when

- Own probe bytes are tempting: the DETECT bytes come from Reticulum-licensed source, so re-deriving them is avoided.
- A board is owned by the Meshtastic client: one owner per port, never both.
- Silent flashing is expected: flashing is human-confirmed, and `--autoinstall` is an interactive terminal flow.

## Details

Reported as of 2026-10-19 (research report 05): firmware 1.86 (2026-04-24); repository pushed 2026-09-27.
Upstream `release.json` lists a SHA-256 per zip, which can be mirrored into a signed manifest.

Cache-seeding note for offline installs (read from `rnodeconf.py`, not run on hardware): the update cache lives at
`~/.config/rnodeconf/update/<version>/<firmware>.zip` beside `<firmware>.zip.version`, whose content is
`"<version> <sha256>"`. With `--nocheck --fw-version <version>` the tool takes the cache and verifies the SHA-256;
otherwise it downloads from GitHub or from `--fw-url`. `--use-extracted` needs five files taken from a connected RNode
of the same model, so it does not pin an image. `--flash` with `--platform`, `--product`, `--model`, and `--hwrev`
is the scriptable path (unverified without hardware).
Redistributing images needs a legal read; fetching at flash time avoids it but breaks offline nodes.

## Evidence

### Alternatives considered

- MeshCore KISS modem firmware (MIT) — a stock-repeater-friendly modem for boards 111 owns ([[meshcore]]).
- [[meshtastic]] firmware on a board 111 drives through the multiplexer — a different owner, not a Reticulum modem.
- 2026-10-19 — licence read upstream (GPL-3.0 text, README wording v3.0, no "or later"); offline mechanics read from
  source.

## Links

- [Upstream repository](https://github.com/markqvist/RNode_Firmware)
- [Release manifest](https://github.com/markqvist/RNode_Firmware/releases/latest/download/release.json)
- Stack: [[reticulum]]
