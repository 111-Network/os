---
title: Meshtastic PKI
category: project
status: candidate
summary: Meshtastic identity layer; its 2025 CVE cascade is our cautionary test plan.
tags: [mesh, lora, identity]
platforms: [esp32, nrf52, rp2040, linux]
upstream: https://github.com/meshtastic/firmware
license: GPL-3.0-only
last_verified: 2026-10-12
sources:
  - https://meshtastic.org/docs
  - https://github.com/meshtastic/firmware
  - https://muzi.works
relations:
  relates_to: ["[[meshtastic]]"]
---

Meshtastic identity layer; its 2025 CVE cascade is our cautionary test plan.

## What it is

The identity and key-management layer of the [[meshtastic]] firmware,
studied as a cautionary case. Node identity is an on-device X25519
keypair stored in plain flash beside node config. Public-key crypto
arrived in firmware 2.5.0 (September 2024): X25519 ECDH plus AES-CCM for
direct messages and remote admin, layered over AES-256-CTR PSK channels
that are encrypted but not authenticated, with the default channel key
AQ== publicly known. No forward secrecy; trust on first use; the NodeID
was historically MAC-derived, making it spoofable.

## Use when

- Studying how identity fails in the field: the 2025 CVE cascade maps
  one-to-one onto the identity spec's hard rules and negative tests.
- A negative reference is needed for keygen discipline, downgrade
  handling, or default-key policy on the largest LoRa mesh fleet alive.

## Do NOT use when

- Designing our keyset: the X25519-only identity cannot sign, and its
  lifecycle is a list of field incidents.
- Any assumption that encrypted equals authenticated is tolerable;
  the PSK channel design proved otherwise in production.

## Details

The 2025 cascade, all fixed promptly but all shipped first: CVE-2025-52464
(CVSS 9.5) — vendor golden-image cloning duplicated X25519 keypairs
across production batches, and nRF52 keygen seeded a PRNG with the app
version, device ID, and weak randomness; the fix in 2.6.11 moved keygen
to first configuration, stirred real entropy, and wiped known-weak keys.
CVE-2025-53627 — silent downgrade to legacy symmetric crypto displayed
as encrypted. CVE-2025-55292 — identity spoof through MAC-based NodeIDs
via ham mode. CVE-2025-55293 — NodeInfo packets could overwrite an
established contact's public key. CVE-2025-24797 — a buffer-overflow RCE
found by fuzzing. DEF CON demonstrations spoofed and replayed NodeInfo
at scale. No formal third-party audit exists and there is no bug bounty.

The identity spec's rules trace directly here: entropy-gated keygen never
at factory time, pubkey-bound identity only, pinned keys immutable, no
default keys, and no silent downgrade.

## Evidence

### Alternatives considered

- [[meshcore]] — the cleaner signed-advert design worth copying, with
  its own composition mistakes cataloged in its entry.
- [[reticulum-protocol]] — the dual-keyset design reference, reused as a
  public-domain protocol only.
- 2026-10-12 — desk research sweep of disclosures and the June 2025
  field incident; entered at candidate as a case study.

## Links

- [Upstream firmware](https://github.com/meshtastic/firmware)
- [Meshtastic security documentation](https://meshtastic.org/docs)
- Related: [[meshtastic]]
