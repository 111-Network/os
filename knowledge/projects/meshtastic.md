---
title: Meshtastic
category: project
status: candidate
summary: LoRa mesh firmware with mature phone apps and a large deployed network.
tags: [mesh, lora, offline]
aliases: [Meshtastic firmware]
platforms: [esp32, nrf52, android, ios]
upstream: https://github.com/meshtastic/firmware
license: GPL-3.0-only
last_verified: 2026-10-12
sources:
  - https://github.com/meshtastic/firmware
  - https://meshtastic.org/docs
  - https://github.com/meshtastic/Meshtastic-Android
relations:
  runs_on: ["[[heltec-wifi-lora-32-v3]]"]
  alternative_to: ["[[reticulum]]"]
---
LoRa mesh firmware with mature phone apps and a large deployed network.

## What it is

An open-source firmware project that turns cheap ESP32 and nRF52 boards with a
LoRa radio into mesh nodes. Nodes forward text messages, position reports, and
telemetry for each other over LoRa; phones connect over Bluetooth or WiFi to a
nearby node. The project ships companion Android and iOS apps, a device
firmware repository with a documented flashing path, and a protobuf-based
firmware API that downstream tools build on.

## Use when

A low-bandwidth, long-range, infrastructure-free text-mesh is needed on
commodity hardware. It is the default candidate for the messaging backbone on
ESP32-class nodes, and the most common way hobbyists run LoRa mesh today.

## Do NOT use when

Throughput matters. LoRa duty cycles and data rates put user payloads in the
tens of bytes per second class. It is also a poor fit where store-and-forward
beyond a few hops or rich metadata is required; the mesh is optimized for
small, ephemeral messages between nearby nodes.

## Details

Desk research, verified 2026-10-12. The firmware is GPL-3.0-only, source is
public on GitHub with a large contributor base and frequent tagged releases.
Supported hardware families include ESP32, ESP32-S3, and nRF52 boards with
SX1262-family radios; [[heltec-wifi-lora-32-v3]] is the budget reference
board. The project maintains official apps, a web-based flasher, and a
protobuf API surface, which makes it scriptable from an agent harness. The
GPL-3.0-only license is copyleft: fine for the OS to adopt and wrap behind a
seam, with linking boundaries kept clean. Candidate gate status: OSI license,
public source, active history, documented install path all pass; testing and
proven gates require first-party smoke tests and field log.

## Evidence

### Alternatives considered

2026-10-12: desk research sweep. [[reticulum]] is the main alternative: more
generic and transport-agnostic, but the Python reference implementation does
not run on ESP32-class hardware directly, and its custom license needs review
against the candidate gate. MeshCore and Meshtastic-fork-derived firmware were
noted in the sweep as younger contenders with smaller deployed networks.
qaul and Serval were recorded as historical or unverified. Meshtastic wins on
deployed-node count, tooling maturity, and phone-app ecosystem at this stage.

## Links

Device profile: [[heltec-wifi-lora-32-v3]]. Mesh alternative: [[reticulum]].
Upstream: https://github.com/meshtastic/firmware
