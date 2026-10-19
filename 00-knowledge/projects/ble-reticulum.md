---
title: BLE Reticulum
category: project
status: candidate
summary: Linux BLE interface plugin for rnsd; 0.x, BlueZ only, reference not adopted.
tags: [mesh, networking, hardware]
platforms: [linux]
upstream: https://github.com/torlando-tech/ble-reticulum
license: MIT
last_verified: 2026-10-19
sources:
  - https://github.com/torlando-tech/ble-reticulum
  - https://raw.githubusercontent.com/torlando-tech/ble-reticulum/main/LICENSE
  - https://raw.githubusercontent.com/torlando-tech/ble-reticulum/main/pyproject.toml
relations:
  depends_on: ["[[reticulum]]"]
---

Linux BLE interface plugin for rnsd; 0.x, BlueZ only, reference not adopted.

## What it is

ble-reticulum is a Python interface plugin that lets `rnsd` mesh over Bluetooth Low Energy on Linux using the
built-in radio, as central and peripheral at once. It settles whether a Linux BLE driver for Reticulum exists:
it does, as a 0.x plugin that needs BlueZ 5.x with experimental features. Layer 6 records it as a reference only.
The `LICENSE` file is MIT (Copyright 2025 torlando-tech); `pyproject.toml` declares MIT and version 0.2.3.

## Use when

- A later release wants a Pi-class hub to reach Reticulum peers over BLE without a dongle.
- A design needs the plugin's facts: up to 7 simultaneous connections, MTU 20 to 512 bytes, RSSI-based peer choice.

## Do NOT use when

- It would enter the base configuration: it adds a Python package and a BlueZ requirement, against the Raspberry Pi
  floor.
- Linux-only is a problem: Android, macOS, and Windows are unsupported.
- Production stability is required: it is 0.x, with 57 stars as of 2026-10-19.

## Details

Reported as of 2026-10-19 (research report 05): v0.2.3 (2026-10-03); tested on a Raspberry Pi Zero W by its README;
three power modes (aggressive, balanced, saver); automatic discovery; packet fragmentation. It installs into the
Reticulum environment and `rnsd` loads it as an interface, so it would be a user-installed extra, never bundled.
Not run on this box. Whether it coexists with the supervised `rnsd` posture is unverified.

## Evidence

### Alternatives considered

- RNode over USB serial ([[rnode-firmware]]) — the supported, documented radio path.
- TCP to the hub from a phone — the only reliable phone path today, with no BLE needed.
- 2026-10-19 — licence and version read upstream; behavior taken from the README; nothing was run.

## Links

- [Upstream repository](https://github.com/torlando-tech/ble-reticulum)
- Stack: [[reticulum]]; modem: [[rnode-firmware]]
