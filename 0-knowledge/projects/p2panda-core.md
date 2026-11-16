---
title: p2panda-core
category: project
status: candidate
summary: Rust signed per-author log library; pre-1.0 API, adoption target only.
tags: [sync, storage]
platforms: [linux, android]
upstream: https://github.com/p2panda/p2panda
license: MIT OR Apache-2.0
last_verified: 2026-11-02
sources:
  - https://github.com/p2panda/p2panda
  - https://raw.githubusercontent.com/p2panda/p2panda/main/p2panda-core/README.md
  - https://raw.githubusercontent.com/p2panda/p2panda/main/p2panda-core/Cargo.toml
---

Rust signed per-author log library; pre-1.0 API, adoption target only.

## What it is

p2panda-core is the operation and log crate of the p2panda project: each operation carries a header with
the author key, signature, payload size and hash, sequence number, backlink and extensions.
Layer 8 names it as the adoption target for a shared log, which is not built in v1.
A shared log enters only through the five-condition admission gate (a named consumer and writer, a never-list,
a measured Pi delta, an adopted engine).

## Use when

- A named consumer needs a per-author signed log that tolerates forks and replays over slow links.
- Old operations must be pruned at author-written points, with payloads deletable apart from headers.

## Do NOT use when

- A stable API is required: the README says APIs are not yet stable until 1.0.
- A second SQLite binding is the price: its store crate uses sqlx, so ops would stay in our own store.

## Details

As the Layer 8 spec states it: version 0.7.1 (2026-08-21), MIT OR Apache-2.0 (both licence files read upstream),
Rust edition 2024 with rust-version 1.96, Ed25519 and BLAKE3 and CBOR, no process and no socket.
Planned use: Layer 5's epoch as the log id so a restore from words never forks a log, ops kept in our store,
and a domain-separation tag so the node key does not sign foreign CBOR. The sync crate would need cursor glue
for sealed frames. Unverified from this box: the binary-size and RSS delta on a Pi, which the gate requires.

## Evidence

### Alternatives considered

- Our own hash chain — would reinvent this crate; the gate says adopt first.
- Automerge or Loro — CRDT documents; no spec asks for a shared document.
- Secure Scuttlebutt feeds — kuska-ssb is AGPL-3.0 and dormant since 2022.
- 2026-11-02 — desk research (Layer 8 reports 01, 03, 05): licences and versions read upstream by the scouts; the entry was written later and nothing was re-read, fetched, built or run on this box.

## Links

- [p2panda repository](https://github.com/p2panda/p2panda)
- [p2panda-core README](https://raw.githubusercontent.com/p2panda/p2panda/main/p2panda-core/README.md)
