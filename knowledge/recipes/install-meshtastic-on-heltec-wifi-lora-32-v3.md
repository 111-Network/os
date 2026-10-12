---
title: Install Meshtastic on Heltec V3
category: recipe
status: candidate
summary: Installs Meshtastic firmware on a Heltec WiFi LoRa 32 V3 over USB.
tags: [mesh, lora, hardware]
last_verified: 2026-10-12
sources:
  - https://meshtastic.org/docs/hardware/devices/heltec-automation/wifi-lora-32-v3/
  - https://flasher.meshtastic.org/
relations:
  depends_on: ["[[meshtastic]]"]
  runs_on: ["[[heltec-wifi-lora-32-v3]]"]
---
Installs Meshtastic firmware on a Heltec WiFi LoRa 32 V3 over USB.

## What it is

The documented bring-up path for turning a [[heltec-wifi-lora-32-v3]] board
into a [[meshtastic]] node. It flashes the current stable device firmware
over USB-C, after which the node is configured through the Android or iOS
app over Bluetooth, or through the device's WiFi-hosted web UI.

## Use when

A fresh board arrives, a node needs recovery from a bad flash, or a bench
unit must be pinned to a known firmware version before field testing. This
recipe is also the template other device bring-ups are copied from.

## Do NOT use when

The board already runs a working Meshtastic build and only needs settings
changes; use the app or web UI instead. It is also not the path for
firmware development builds, which come from the firmware repository's
own build tooling.

## Details

Verified against upstream documentation on 2026-10-12; not yet executed on
our bench. The web flasher at flasher.meshtastic.org is the upstream-recommended
path: connect the board over USB-C, put it in download mode, and flash the
firmware build matched to the board. Command-line equivalent with esptool:

```sh
esptool.py write_flash 0x0 firmware-heltec-v3.bin
```

Download mode is entered by holding the PRG button while connecting USB, or
by holding PRG and tapping RST on an already-powered board. After flashing,
the OLED shows the node status screen and the device advertises over
Bluetooth for app pairing, or serves its web UI when WiFi is enabled.
Promotion of this recipe past candidate happens when the smoke test logs a
real bench run with firmware version, host OS, and result.

## Evidence

### Alternatives considered

2026-10-12: desk research against upstream docs. esptool.py is the fallback
when a browser flasher is unavailable; it needs the correct firmware binary
per board revision. PlatformIO and the firmware repo's own build flow were
recorded as the development path, deliberately out of scope for an install
recipe. The web flasher stays the default for its one-step operation.

## Links

Project: [[meshtastic]] · Device: [[heltec-wifi-lora-32-v3]]
