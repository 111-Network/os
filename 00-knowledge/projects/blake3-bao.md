---
title: BLAKE3 bao-tree
category: project
status: candidate
summary: BLAKE3 addressing with verified streaming for bulk objects; bao-tree is pre-1.0.
tags: [storage, sync, messaging]
platforms: [linux, android, macos, windows]
upstream: https://github.com/n0-computer/bao-tree
license: (CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception) AND (MIT OR Apache-2.0)
last_verified: 2026-10-14
sources:
  - https://github.com/n0-computer/bao-tree
  - https://github.com/BLAKE3-team/BLAKE3
  - https://raw.githubusercontent.com/n0-computer/bao-tree/main/README.md
  - https://docs.rs/bao-tree/latest/bao_tree/struct.BlockSize.html
relations:
  relates_to: ["[[iroh-nodeid]]"]
---

BLAKE3 addressing with verified streaming for bulk objects; bao-tree is pre-1.0.

## What it is

The `blake3` crate (1.8.7) hashes content to a 32-byte root; `bao-tree` (0.16.1, 2026-08-26) adds
an outboard tree so any byte range can be verified against that root while it streams.
Layer 5 uses the pair as the single content address and verified-stream primitive for every link tier.
License is two dual licenses: `blake3` is CC0-1.0 OR Apache-2.0 (OR Apache-2.0 with LLVM exception),
`bao-tree` is MIT OR Apache-2.0; both read from their `Cargo.toml`.

## Use when

- Bulk objects (attachments, KB deltas, update payloads, patches) must be fetched in pieces
  from different carriers and verified chunk by chunk.
- One address must work over LoRa and TCP: the root is identical at every chunk-group size.
- Resume at any chunk boundary and range-set requests are wanted.

## Do NOT use when

- An unsealed chunk path on broadcast bearers is assumed settled; it needs a Layer 2 carve-out
  that is asked for in v1.1, not now.
- Production-grade API stability is required; bao-tree is pre-1.0.
- A whole QUIC stack is wanted for free; see the alternatives below.

## Details

Cost: `bao-tree` plus `blake3` add +94 KB (x86_64) and +29 KB (aarch64) to a musl-static binary.
Probe: a 1 MiB buffer outboarded at 16 KiB chunk groups gave a 4,032-byte outboard (0.38%) and a root
equal to `blake3::hash`. By construction 1 KiB groups cost 64 bytes per KiB (6.25%).
Layer 5 uses 1 KiB groups on LoRa and BLE, 16 KiB on TCP.
Private objects are encrypted once with a random per-object key and addressed by the ciphertext hash,
so relays and hubs verify what they carry without reading it. Public signed objects are stored plain.

Spec scope: v1 builds the blob store, addressing, outboard computation, and the reserved kinds;
v1.1 builds the wire transfer (`OBJ_ADVERT`, `OBJ_REQ`, `OBJ_CHUNK`).
Airtime reality: a 32 KB KB delta is minutes of LoRa; a 5.7 MB binary is hours (see [[lora-airtime-budget]]).

## Evidence

### Alternatives considered

- iroh with iroh-blobs — MIT OR Apache-2.0, but +13.3 MB, QUIC-only (1200-byte minimum datagram, so no LoRa),
  and a README stating it is not yet production quality. Its NodeId equals an iroh EndpointId ([[iroh-nodeid]]).
- Syncthing — MPL-2.0 separate process with its own device identities; a bridge, not the fabric.
- BPv7 bundles ([[bundle-protocol-v7]]) — no content addressing and a mandatory clock-based lifetime.
- 2026-10-14 — licenses and versions read from upstream `Cargo.toml` files; size probes from report 05.

## Links

- [bao-tree repository](https://github.com/n0-computer/bao-tree)
- [BLAKE3 repository](https://github.com/BLAKE3-team/BLAKE3)
- [BlockSize documentation](https://docs.rs/bao-tree/latest/bao_tree/struct.BlockSize.html)
