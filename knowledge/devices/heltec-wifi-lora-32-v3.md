---
title: Heltec WiFi LoRa 32 V3
category: device
status: candidate
summary: ESP32-S3 SX1262 LoRa dev board running all major mesh firmware stacks.
tags: [hardware, lora, mesh]
platforms: [esp32]
last_verified: 2026-10-12
sources:
  - https://heltec.org
  - https://meshtastic.org/docs
relations:
  hosted_by: ["[[meshtastic]]", "[[install-meshtastic-on-heltec-wifi-lora-32-v3]]"]
---
ESP32-S3 SX1262 LoRa dev board running all major mesh firmware stacks.

## What it is

Heltec WiFi LoRa 32 V3 is a development board pairing an Espressif ESP32-S3
microcontroller with an SX1262 LoRa radio.
Price reported as of 2026-10 is roughly $22-26 USD.
One hardware fleet serves all three major LoRa stacks — [[meshtastic]],
MeshCore, and RNode firmware; reflashing the board switches protocols.

## Use when

- A low-cost ESP32 LoRa mesh node is needed for field trials.
- One hardware fleet should stay switchable across Meshtastic, MeshCore, or RNode.
- The existing install recipe [[install-meshtastic-on-heltec-wifi-lora-32-v3]] is reused.

## Do NOT use when

- Best possible RF range out of the box is required; antenna quality dominates real range.
- The V4 revision's better power efficiency justifies its higher price.
- Counterfeit-prone marketplace listings cannot be vetted by the buyer.

## Details

- MCU: ESP32-S3; radio: SX1262.
- Price reported as of 2026-10: roughly $22-26 USD.
- V4 revision is newer at roughly $24-28 with better power efficiency.
- Three LoRa stacks share one fleet: [[meshtastic]], MeshCore, RNode firmware.
- Regional radio config (EU868 / US915 and similar) is a per-device install
  detail any bootstrapper must handle.
- Counterfeits are common on marketplaces; source from known-good suppliers.

## Evidence

### Alternatives considered

- 2026-10-12 — Desk research sweep (reported as of 2026-10): the newer V4
  revision was considered for its better power efficiency but costs more;
  V3 keeps the lower price and the existing install recipe.

## Links

- https://heltec.org
- https://meshtastic.org/docs
