---
title: Heltec WiFi LoRa 32 V3
category: device
status: candidate
summary: ESP32-S3 LoRa development board with OLED, the current budget Meshtastic node.
tags: [hardware, lora]
aliases: [Heltec V3, WiFi LoRa 32 V3]
platforms: [esp32]
upstream: https://heltec.org/project/wifi-lora-32-v3/
last_verified: 2026-10-12
sources:
  - https://heltec.org/project/wifi-lora-32-v3/
  - https://github.com/Heltec-Aaron-Lee/WiFi_Kit_series
relations:
  hosted_by: ["[[meshtastic]]"]
---
ESP32-S3 LoRa development board with OLED, the current budget Meshtastic node.

## What it is

A development board pairing an ESP32-S3 microcontroller with an SX1262 LoRa
radio, WiFi and Bluetooth, a small onboard OLED, USB-C, and a battery
connector. It is one of the cheapest supported ways to run
[[meshtastic]] and is widely stocked by hobbyist suppliers, which makes it
the natural first hardware profile for mesh experiments.

## Use when

A low-cost LoRa mesh node is needed for bench tests, relay prototypes, or
field units where range and price beat polish. The onboard display and USB-C
flashing path keep bring-up simple without extra tooling.

## Do NOT use when

Ruggedized or weatherproof deployment is required out of the box; this is a
bare development board needing an enclosure and antenna care. It is also not
a Linux-class host, so heavier runtimes do not fit its memory budget.

## Details

Desk research, verified 2026-10-12. Device gate adaptation applies:
availability plus our own install test replace repo-activity gates. The board
is broadly available from multiple distributors, and [[meshtastic]]
maintains a documented flashing path for it, including a web flasher and
USB-serial recovery. Exact flash and RAM headroom for OS-side services beyond
the firmware remains to be measured on the bench; that measurement belongs
in the testing-gate smoke test. Power behavior on battery with the radio at
high duty cycle is another open bench item.

## Evidence

### Alternatives considered

2026-10-12: desk research sweep. LilyGO T-Beam and T-Echo boards were
weighed; they add GPS or nRF52 efficiency at higher cost and lower stock.
RAK WisBlock was noted for modular builds. Heltec V3 stays the budget
reference profile, with the T-Echo recorded as the efficiency step-up if
battery life becomes the binding constraint.

## Links

Firmware: [[meshtastic]] · Install path: [[install-meshtastic-on-heltec-wifi-lora-32-v3]]
