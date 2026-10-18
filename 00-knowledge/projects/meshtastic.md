---
title: Meshtastic
category: project
status: candidate
summary: LoRa mesh firmware using managed flooding; flooding scales poorly on busy channels.
tags: [mesh, lora]
platforms: [esp32, nrf52, rp2040, linux]
upstream: https://github.com/meshtastic/firmware
license: GPL-3.0-only
last_verified: 2026-10-14
sources:
  - https://meshtastic.org/docs
  - https://github.com/meshtastic/firmware
  - https://meshtastic.org/docs/development/device/client-api/
  - https://meshtastic.org/docs/overview/encryption/
  - https://raw.githubusercontent.com/meshtastic/protobufs/master/meshtastic/config.proto
relations:
  runs_on: ["[[heltec-wifi-lora-32-v3]]"]
  alternative_to: ["[[reticulum]]"]
  required_by: ["[[install-meshtastic-on-heltec-wifi-lora-32-v3]]", "[[meshmud]]", "[[meshtastic-web]]", "[[meshtastic-python]]"]
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
- Carrying opaque sealed payloads between 111 nodes on a shared channel through `PRIVATE_APP`.

## Do NOT use when

- Engineered multi-hop routes are required; flooding scales poorly with load.
- Low channel noise is needed; dense meshes add relay chatter.
- Deterministic latency matters; reported multi-hop latency varies by density.
- Opaque payloads must cross every router: `CORE_PORTNUMS_ONLY` routers drop `PRIVATE_APP`.
- A second local API client is needed on the same node; the node serves exactly one.

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

Layer 5 carriage facts (research reports 01, 02, 06; as of 2026-10-14):

- `PRIVATE_APP` (portnum 256) carries opaque payloads. `Data.payload` is at most 233 B, about 221 B on PKI
  direct messages or AEAD channels (each adds 12 B); the 16 B radio header is extra, LoRa max is 255 B.
  There is no fragmentation: an oversize packet gets a TOO_LARGE error. `STORE_FORWARD_APP` (65) is still
  marked work in progress.
- Rebroadcast modes decide whether foreign payloads travel. Default `ALL` passes channels a router cannot
  decrypt; `CORE_PORTNUMS_ONLY` forwards only NodeInfo, Text, Position, Telemetry, and Routing, so it drops
  `PRIVATE_APP`; `LOCAL_ONLY` and `KNOWN_ONLY` drop packets on channels the router cannot decrypt. The
  `REPEATER` role is deprecated since v2.7.11. Packet priority is never sent over the air.
- Firmware 2.8 adds XEdDSA-signed broadcasts (+66 B, sent unsigned when there is no room), optional AEAD
  channels (12 B tag), and direct-message ack proofs (`Routing.ack_proof`, reported but not enforced). Node
  numbers derive from a CRC-32 of the public key. Channel keys are AES-CTR with a public default key (`AQ==`)
  and no integrity; direct messages are X25519 with AES-256-CCM and no forward secrecy.
- One API client: a node, and meshtasticd, serves a single client. Layer 5 holds that slot with
  [[meshmud]]; the web client ([[meshtastic-web]]) and the presence reader ([[meshtastic-python]]) attach through it.
- Client API framing: serial and TCP port 4403 share one stream format, the bytes `0x94 0xC3`, a 2-byte
  length (max 512), then a ToRadio or FromRadio protobuf; `want_config_id` returns the node database.
  BLE uses MTU 512. meshtasticd drives SPI and CH341 radios only, with a self-signed HTTPS web server on 9443.
- The firmware normalizes a client-forged `from` to the local node, so a bridge always speaks as its own node,
  and the node decrypts before the client sees a packet. On amateur-licensed regions sealed traffic is not sent.

## Evidence

### Alternatives considered

- [[reticulum]] — engineered multi-hop routing with store-and-forward; stronger
  fit when routes must be deterministic, heavier to deploy than flooding.
- Desk research sweep of 2026-10-12; no first-party smoke test yet.
- 2026-10-14 — Layer 5 research pass added the carriage facts above from the client API, encryption, and
  `config.proto` documents; no hardware test.

## Links

- [Official documentation](https://meshtastic.org/docs)
- [Upstream firmware repository](https://github.com/meshtastic/firmware)
