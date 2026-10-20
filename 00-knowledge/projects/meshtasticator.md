---
title: Meshtasticator
category: project
status: candidate
summary: Meshtastic network simulator; CC BY 4.0, test fixture only, never shipped.
tags: [lora, test-fixture, mesh]
platforms: [linux]
upstream: https://github.com/meshtastic/Meshtasticator
license: CC-BY-4.0
last_verified: 2026-10-19
sources:
  - https://github.com/meshtastic/Meshtasticator
  - https://raw.githubusercontent.com/meshtastic/Meshtasticator/master/README.md
relations:
  depends_on: ["[[meshtastic]]"]
---

Meshtastic network simulator; CC BY 4.0, test fixture only, never shipped.

## What it is

Meshtasticator is a Python simulator for Meshtastic networks. Its interactive mode runs several native firmware nodes on
TCP ports starting at 4403 and forwards packets between them by simulated radio range.
Layer 6 may use it as a hardware-less test fixture for the Meshtastic client. It is never part of a release.
The README states "Creative Commons Attribution 4.0 International License"; the repository has no `LICENSE` file on
its default branch.

## Use when

- A test needs several simulated Meshtastic nodes on a laptop with no radio.
- A client's handshake, reasons, and reconnects need a repeatable board stand-in.

## Do NOT use when

- It would ship in a payload or a manifest: it is test-only and its licence is a content licence.
- The test must prove a second client can attach beside the simulator's own client: nobody has verified that.
- Real radio timing is under test: it simulates range, not RF behavior.

## Details

Reported as of 2026-10-19 (research reports 05 and 06): the interactive simulator needs a built firmware; an official
`meshtasticd -s -p 4403` simulated radio is the other fixture, with no prebuilt binary found.
Part of the source derives from published LoRa simulation work (LoRaSim), cited in the README.
Unverified: attaching [[meshmud]] alongside the simulator's own TCP client.

## Evidence

### Alternatives considered

- `meshtasticd` (GPL-3.0) with the simulated radio — a fixture, never shipped.
- A stub speaking the client API in the test harness — the fallback when neither simulator attaches.
- 2026-10-19 — licence read from the upstream README; nothing run.

## Links

- [Upstream repository](https://github.com/meshtastic/Meshtasticator)
- Node: [[meshtastic]]; multiplexer: [[meshmud]]
