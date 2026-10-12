---
title: Automerge
category: project
status: candidate
summary: Local-first CRDT library with a Rust core and language bindings.
tags: [sync, crdt, offline]
platforms: [linux, android, ios]
upstream: https://github.com/automerge/automerge
license: MIT
last_verified: 2026-10-12
sources:
  - https://github.com/automerge/automerge
  - https://automerge.org/
---
Local-first CRDT library with a Rust core and language bindings.

## What it is

A library for JSON-like documents that concurrent writers can edit offline
and merge automatically, with no server required for correctness. The
current implementation is a Rust core consumed through WebAssembly for
JavaScript and through C FFI for other languages, replacing the deprecated
pure-JavaScript automerge-classic line. Sync and storage protocols are
documented and interoperable across bindings.

## Use when

Shared mutable state must converge between intermittently connected nodes,
such as dashboards, notes, or presence data on the mesh. It is the leading
candidate for the OS's shared-state seam because local-first operation is
the design center, not an afterthought.

## Do NOT use when

A single-writer database with strict schema enforcement is enough; a CRDT
adds merge-semantics complexity that buys nothing there. It is also not a
messaging or query engine, so routing and indexing layers stay separate
concerns.

## Details

Desk research, verified 2026-10-12. MIT license, public source with visible
history, documented build path for the Rust core, and an active maintenance
organization. The Rust core builds network-isolated from locked crates,
which supports the testing-gate offline-build check. Footprint on
ESP32-class targets is untested; bindings exist for mobile and desktop
classes, and a smaller sync protocol exists in sibling projects if device
profiles demand it. Document size growth under high concurrency is the main
operational cost to measure in a smoke test.

## Evidence

### Alternatives considered

2026-10-12: desk research sweep. Yjs was weighed as the dominant alternative
with broader editor integrations; it is GPL-licensed in parts of its
ecosystem, which conflicts with adoption for core infrastructure.
y-crdt, its Rust port, was noted for a permissive path if Yjs compatibility
ever matters. Loro was recorded as a younger Rust CRDT to re-review
quarterly. Automerge stays the default for its MIT license, Rust core, and
explicit sync protocol.

## Links

Upstream: https://github.com/automerge/automerge · Docs: https://automerge.org/
