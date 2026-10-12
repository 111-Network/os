---
title: Install Meshtastic on Heltec V3
category: recipe
status: candidate
summary: Meshtastic firmware install path for the Heltec WiFi LoRa 32 V3.
tags: [mesh, lora, hardware]
last_verified: 2026-10-12
sources:
  - https://meshtastic.org/docs
  - https://flasher.meshtastic.org/
relations:
  depends_on: ["[[meshtastic]]"]
  runs_on: ["[[heltec-wifi-lora-32-v3]]"]
---
Meshtastic firmware install path for the Heltec WiFi LoRa 32 V3.

## What it is

The documented procedure for turning a [[heltec-wifi-lora-32-v3]] board into
a running [[meshtastic]] node. Firmware is flashed over USB-C using either
the upstream web flasher or the esptool command line tool; the node is then
configured over Bluetooth Low Energy or a USB serial web client.

## Use when

A fresh board needs firmware before its first field test, a node must be
recovered from a failed flash, or a bench unit should be pinned to a known
upstream build.

## Do NOT use when

The board already runs a working build and only needs setting changes; pair
the app with the node instead. Building firmware from source is a separate
development path, not this install recipe.

## Details

Desk research against upstream documentation as of 2026-10; not yet executed
on our bench, so status stays candidate. Two install paths exist.

Web flasher, the upstream-recommended path: open flasher.meshtastic.org in
Chrome or Edge (WebSerial support is required; browsers without it cannot
drive the flasher), connect the board over USB-C, select the firmware build
for the Heltec V3, and flash. If the board is not detected, put it in
bootloader mode and retry.

esptool CLI fallback for hosts without a compatible browser:

```sh
esptool.py write_flash 0x0 firmware-heltec-v3.bin
```

Use the Heltec V3 firmware build from upstream releases; the flasher path
and the CLI path flash the same per-board image.

After a successful flash, set the LoRa region (EU868, US915, or the
matching local plan) before the radio is used; the region is a legal
transmit parameter and a per-device setting. Pairing then happens over
Bluetooth Low Energy with the Meshtastic Android or iOS app, or through the
USB serial web client, for the remaining configuration.

Promotion past candidate needs a smoke test: one real bench flash with
firmware version, host OS, and result logged in this entry.

## Evidence

### Alternatives considered

2026-10-12: desk research against upstream documentation; neither path has
been run on our hardware. The web flasher stays the default for its
one-step, upstream-guided flow with per-board firmware selection built in.
esptool is kept as the fallback for hosts without a WebSerial-capable
browser. Building from the firmware repository source was recorded as the
development path and deliberately left out of scope for an install recipe.

## Links

Project: [[meshtastic]] · Device: [[heltec-wifi-lora-32-v3]]
