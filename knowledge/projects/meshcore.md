---
title: MeshCore
category: project
status: candidate
summary: LoRa mesh whose Ed25519 pubkey-as-identity proves our Cat 1 shape.
tags: [mesh, lora, identity]
platforms: [esp32, nrf52]
upstream: https://github.com/meshcore-dev/MeshCore
license: MIT
last_verified: 2026-10-14
sources:
  - https://github.com/meshcore-dev/MeshCore
  - https://github.com/meshcore-dev/MeshCore/issues/1024
  - https://github.com/meshcore-dev/MeshCore/blob/main/docs/kiss_modem_protocol.md
  - https://github.com/meshcore-dev/MeshCore/blob/main/src/Mesh.cpp
  - https://github.com/meshcore-dev/MeshCore/blob/main/docs/number_allocations.md
relations:
  alternative_to: ["[[meshtastic]]"]
---

LoRa mesh whose Ed25519 pubkey-as-identity proves our Cat 1 shape.

## What it is

MeshCore is a lightweight LoRa mesh protocol and firmware family
launched in early 2025, growing fast on ESP32-S3 and nRF52 hardware. Its
identity model is the shape our spec converged on: an Ed25519 keypair
generated at first boot whose 32-byte public key IS the node address.
Adverts carry the public key, a timestamp, and an Ed25519 signature over
the whole advert at about 100 bytes, verifiable by anyone, registry-free.

## Use when

- Proving Ed25519-pubkey-as-identity runs at scale on exactly our Cat 1
  chips; MeshCore's fleet is the live demonstration.
- Looking for the leanest proven signed-advert wire form on LoRa.
- Studying which composition mistakes to avoid next to the good identity
  model.
- Carrying opaque sealed payloads through stock repeaters as `GRP_DATA` floods.

## Do NOT use when

- Copying its crypto composition: the message layer is AES-128-ECB with
  a 2-byte truncated HMAC, and there is no key separation.
- Operational security defaults matter: the firmware once shipped with
  the admin password literally password, and the private key was
  readable through a console command.
- An audited crypto stack is required; none exists, and the vendored
  legacy ed25519 C library uses a nonstandard key format.
- `RAW_CUSTOM` is assumed to flood; repeaters forward it only along a known direct path.

## Details

Advert structure: 32-byte public key plus 4-byte timestamp plus 64-byte
signature plus role flags, with the signature covering the entire advert.
Companion devices do not relay; repeaters and room servers forward and
store; up to 64 hops. Its X25519 ECDH is derived by converting the
Ed25519 key, the birational coupling our spec rejects in favor of a
sibling derived key. Performance relies on a vendored TweetNaCl-class
Ed25519, roughly 1.5 to 2 seconds to verify on Cortex-M4 class parts.

License note: the research reports conflicted on GPL-3.0 versus MIT; the
GitHub API reports an MIT license file as of 2026-10-12, and the entry
records MIT with this conflict noted per the options brief. The 2026-10-14
pass closed it: the firmware `license.txt` is MIT, and `meshcore_py` and
`meshcore-cli` are MIT too.

Layer 5 carriage facts (reports 01, 02, 06, gap-1; as of 2026-10-14):

- KISS modem mode: MeshCore documents a standard KISS TNC firmware (115200 8N1, raw 255 B data frames, one
  pending TX, built-in p-persistent CSMA) with `SetRadio`, `GetAirtime`, `RxMeta`, `TxDone`, and
  `EncryptData`. PlatformIO environments exist for Heltec V3, Heltec V4, and RAK4631, but no kiss-modem
  release tag was among the latest 60 releases, so images are CI-built and pinned. It has no forwarding code.
- `GRP_DATA` flood: a stock repeater floods any group-data packet without holding the channel key, using a
  seen-table, path append, and `flood_max` 64. Payload is 163 B through the companion protocol (165 B
  firmware cap), up to 173 B when host-built (computed, unverified on air). `number_allocations.md`
  registers third-party data types. A wildcard region set to DENY_FLOOD drops unscoped floods.
- Repeater as Cat-1 relay: the stock MIT repeater (repeater-v1.17.1, 2026-08-14) is relay-blind carriage, so
  a Cat-1 relay needs no firmware of ours. Companions repeat only in client-repeat mode, and by default only
  on 433.000, 869.495, or 918.000 MHz.
- `RAW_CUSTOM` (0x0F) is not flooded: it is forwarded only along a known direct path ("don't flood route these
  (yet)"), so 184 B applies to learned direct paths and flood carriage means group data.
- Companion link: serial, BLE, or TCP (one per build); `send_raw_packet` and `RAW_DATA` exist; BLE default MTU 23,
  requesting 512. Group data is AES-128-ECB with a 2-byte MAC and 1-byte hashes: integrity is only 16 bits.

## Evidence

### Alternatives considered

- [[meshtastic]] — far larger community and app ecosystem, but its 2025
  identity CVE cascade (see [[meshtastic-pki]]) versus this project's
  clean signed adverts.
- [[reticulum-protocol]] — the fuller mesh-native design reference,
  reused as protocol only.
- 2026-10-12 — desk research sweep; entered at candidate as a case
  study; license re-verified against the upstream repository.
- 2026-10-14 — Layer 5 research pass added the carriage facts from `Mesh.cpp` and the KISS modem
  documentation; no two-board bench has run yet.

## Links

- [Upstream repository](https://github.com/meshcore-dev/MeshCore)
- [Key format issue](https://github.com/meshcore-dev/MeshCore/issues/1024)
- Alternative to: [[meshtastic]]
