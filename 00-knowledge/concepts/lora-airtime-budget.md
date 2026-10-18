---
title: LoRa Airtime Budget
category: concept
summary: Duty-cycle law and time-on-air numbers that cap LoRa traffic volume.
tags: [lora, networking, mesh]
aliases: [LoRa duty cycle, time on air]
sources:
  - https://www.thethingsnetwork.org/docs/lorawan/duty-cycle/
  - https://meshtastic.org/docs/overview/radio-settings/
  - https://meshtastic.org/docs/configuration/radio/lora/
  - https://github.com/meshcore-dev/MeshCore/blob/main/src/MeshCore.h
relations:
  relates_to: ["[[meshtastic]]", "[[meshcore]]", "[[blake3-bao]]"]
---

Duty-cycle law and time-on-air numbers that cap LoRa traffic volume.

## What it is

LoRa carries kilobytes, not megabytes. Two limits apply: time on air (physics) and duty cycle (law).
The Layer 5 fabric never rate-limits; each link adapter advertises its airtime facts (`duty_cycle_permille`,
`bits_per_s`) and the agent can read them. Capacity classes and fragment-level retry follow from this page.

## Use when

- Sizing a fragment, a retry policy, or deciding whether a payload may cross a LoRa link at all.
- Explaining to a user why a large object waits for a LAN link.

## Do NOT use when

- Exact planning for one region is needed: sub-band rules vary and the figures below are single-hop ideals.
- Wall-clock delivery times are wanted: ACKs, retransmits, flooding, and shared channels all add airtime.

## Details

EU duty cycle (ETSI EN 300 220-2, via The Things Network docs):

| Sub-band | Duty cycle |
|---|---|
| 863–865 MHz | 0.1% |
| 865–868 MHz, 868–868.6 MHz | 1% |
| 868.7–869.2 MHz | 0.1% |
| 869.4–869.65 MHz | 10% |
| 869.7–870 MHz | 1% |

Meshtastic's region table lists EU_868 at 10% and 27 dBm, EU_866 at 2.5 to 10%, and the US without a limit.
A 1% band stretches wall time by 100, a 10% band by 10.

Payload ceilings per bearer: Meshtastic `PRIVATE_APP` 233 B (about 221 B with PKI or AEAD);
MeshCore raw packet 184 B, group data 163 B (companion) or 165 B (firmware); RNode single frame 254 B,
and a 500 B Reticulum packet splits into two frames. The fabric's fragment floor is 160 B.

Time on air for a 255 B frame (Semtech formula, CRC on):

| Link | Rate | One frame |
|---|---|---|
| Meshtastic LongFast | 1.07 kbps | about 2.1 s |
| Meshtastic MediumFast | 3.52 kbps | 0.63 s |
| Meshtastic ShortFast | 10.94 kbps | 0.20 s |
| MeshCore US narrow (SF7, 62.5 kHz) | about 2.7 kbps | 0.82 s |
| RNode SF8, 125 kHz | 3.1 kbps | 0.73 s |

An 8-fragment note of 160 B frames costs about 12.3 s on LongFast, which is 20.6 minutes of a 1% budget.
Bulk objects, single-hop ideal airtime on LongFast (unsealed / every frame sealed):
a 4 KB manifest 40 s / 69 s; a 32 KB KB delta 5.0 / 8.9 min; a 5.7 MB binary 15.0 / 26.7 h;
a 1 GB model thousands of hours. Sealing every bulk frame costs about 1.8 times the airtime.

Rules: bulk classes default to `Bulk` links and cross a `LowBandwidth` link only on an explicit ask;
the firmware enforces airtime locks only if the host sets them, and the host sets only what law requires.
On amateur-licensed regions no sealed traffic is sent (obscuring rules, not re-fetched this pass).

## Evidence

- Frame limits come from firmware sources (Meshtastic `mesh.options`, MeshCore `MeshCore.h`, RNode `Config.h`).
- Airtime figures were derived from the Semtech time-on-air formula; the two research models differ by 3%
  on LongFast (2.09 s versus 2.16 s). All figures are on-air time; wall time scales by the duty cycle.

## Links

- [Duty-cycle table source](https://www.thethingsnetwork.org/docs/lorawan/duty-cycle/)
- [Meshtastic radio settings](https://meshtastic.org/docs/overview/radio-settings/)
- Networks: [[meshtastic]], [[meshcore]]; bulk addressing: [[blake3-bao]]
