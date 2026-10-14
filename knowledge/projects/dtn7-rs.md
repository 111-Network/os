---
title: dtn7-rs
category: project
status: candidate
summary: Rust BPv7 daemon and library; stale release tag, no documented API authentication.
tags: [dtn, networking, messaging]
platforms: [linux, macos]
upstream: https://github.com/dtn7/dtn7-rs
license: MIT OR Apache-2.0
last_verified: 2026-10-14
sources:
  - https://github.com/dtn7/dtn7-rs
  - https://raw.githubusercontent.com/dtn7/dtn7-rs/master/core/dtn7/Cargo.toml
  - https://raw.githubusercontent.com/dtn7/dtn7-rs/master/doc/ecla.md
  - https://raw.githubusercontent.com/dtn7/dtn7-rs/master/doc/http-client-api.md
relations:
  implements: ["[[bundle-protocol-v7]]"]
---

Rust BPv7 daemon and library; stale release tag, no documented API authentication.

## What it is

dtn7-rs is a Rust implementation of the Bundle Protocol version 7: a daemon (`dtnd`) plus library,
with the `bp7` crate as codec. It is the best "do not write a store" option the research found.
Layer 5 keeps it as a later optional gateway bridge, not the fabric core.

## Use when

- Interop with a DTN network is wanted: translate a 111 envelope into a bundle payload at a gateway.
- The `bp7` crate is wanted as a CBOR codec in a separate gateway process.

## Do NOT use when

- It would be the core: lifetimes are clock-based and every fragment repeats a 37 to 57 byte primary block.
- It must run embedded or on a phone: it is a daemon with global state and a loopback HTTP control port.
- Active release tagging is required: the last tag is v0.21.0 (2024-03-27).

## Details

Reported as of 2026-10-14: v0.21.0 (2024-03-27); master pushed 2026-09-09; 112 stars; `bp7` 0.10.7 (2025-06-01).
License MIT OR Apache-2.0, read from `LICENSE-MIT`, `LICENSE-APACHE`, and `Cargo.toml`.
It speaks TCPCLv4, MTCP, UDP, and HTTP, has mDNS discovery, and routers named Epidemic, Flooding, Sink,
External, SprayAndWait, and Static. Routing is a tokio actor over `bp7` types, not a library seam.

External convergence layers use ECLA: a JSON API over WebSocket or TCP (`Register`, `ForwardData` with
base64 bundle bytes, `Beacon`). Any language can implement a CLA, which is the gateway seam.
The HTTP control API (`/send`, `/register`) listens on localhost TCP port 3000 with no authentication
documented; on Android any app on the phone could reach it (inference).
Release assets are 9 to 10 MB tarballs for armv7-musl, x86_64 (gnu and musl), and macOS; no aarch64 build.
The manifest pulls sled 0.34.7, axum 0.5, and hyper 0.14; the sled store is one of the objections.
A bridge would set a maximum lifetime and a clockless Bundle Age block (see [[bundle-protocol-v7]]).

## Evidence

### Alternatives considered

- uD3TN — C, SQLite, POSIX only, AGPL-3.0-or-later since early 2024 (v0.15.1, 2026-09-18); safe as a separate process,
  no custody, no phone or LoRa path.
- ION-DTN — BSD-style Caltech license, heavy multi-process design.
- HDTN — NASA Open Source Agreement 1.3 (the LICENSE file wins over GitHub's Apache-2.0 label); C++ with Boost.
- Hardy — Rust, Apache-2.0 per crates.io, 0.2.0 (2026-07); young.
- 2026-10-14 — license read from upstream files in the Layer 5 research; nothing was built or run.

## Links

- [Upstream repository](https://github.com/dtn7/dtn7-rs)
- [ECLA documentation](https://raw.githubusercontent.com/dtn7/dtn7-rs/master/doc/ecla.md)
- Protocol: [[bundle-protocol-v7]]
