---
title: Meshtastic
category: project
status: candidate
summary: LoRa mesh firmware using managed flooding; flooding scales poorly on busy channels.
tags: [mesh, lora]
platforms: [esp32, nrf52, rp2040, linux]
upstream: https://github.com/meshtastic/firmware
license: GPL-3.0-only
last_verified: 2026-10-12
sources:
  - https://meshtastic.org/docs
  - https://github.com/meshtastic/firmware
relations:
  runs_on: ["[[heltec-wifi-lora-32-v3]]"]
  alternative_to: ["[[reticulum]]"]
  required_by: ["[[install-meshtastic-on-heltec-wifi-lora-32-v3]]"]
---

LoRa mesh firmware using managed flooding; flooding scales poorly on busy channels.

## What it is

Open-source LoRa mesh networking firmware for ESP32, nRF52, RP2040, and Linux
targets. Nodes form a peer mesh and relay packets by managed flooding, with a
roughly seven-hop cap. Licensed GPL-3.0.

## Use when

- Off-grid text messaging and position sharing over LoRa radio.
- Community deployments where nodes join and relay without engineered routes.
- Integration with MQTT, Home Assistant, or ATAK plugins is wanted.
- Running on cheap ESP32 hardware such as [[heltec-wifi-lora-32-v3]].

## Do NOT use when

- Engineered multi-hop routes are required; flooding scales poorly with load.
- Low channel noise is needed; dense meshes add relay chatter.
- Deterministic latency matters; reported multi-hop latency varies by density.

## Details

Reported as of 2026-10: about 40k GitHub stars. The firmware 2.8.x line is
current (2.8.1; 2.8.0 was revoked over a packet-signing bug). Recent releases
added XEdDSA packet signing, a redesigned NodeDB, and traffic management.
Multi-hop latency of 10-20s is reported on dense networks. US nodes now default
to the LongTurbo modem preset.

Two 2026 CVEs were patched promptly: a mesh DoS via malformed UTF-8 node names,
fixed in 2.7.23, and a GitHub Actions RCE, fixed in 2.7.21. The response speed
is evidence of active security maintenance.

The community is large, with regional deployments such as Austin Mesh and
NoDak Mesh.

## Evidence

### Alternatives considered

- [[reticulum]] — engineered multi-hop routing with store-and-forward; stronger
  fit when routes must be deterministic, heavier to deploy than flooding.
- Desk research sweep of 2026-10-12; no first-party smoke test yet.

## Links

- [Official documentation](https://meshtastic.org/docs)
- [Upstream firmware repository](https://github.com/meshtastic/firmware)
