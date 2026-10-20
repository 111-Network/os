---
title: Automerge
category: project
status: candidate
summary: Local-first CRDT library for JSON documents with Git-like merge history.
tags: [crdt, sync, storage, offline]
platforms: [linux, macos, windows]
upstream: https://github.com/automerge/automerge
license: MIT
last_verified: 2026-10-12
sources:
  - https://github.com/automerge/automerge
  - https://automerge.org
---
Local-first CRDT library for JSON documents with Git-like merge history.

## What it is

Automerge is a local-first CRDT library for JSON-shaped documents.
Each edit produces a mergeable change; concurrent edits converge
without coordination, and history supports branch and merge like Git.
Version 3 is stable, with a Rust/WASM core, columnar compression, and
Peritext rich text; memory use is down roughly 25x since v1.

## Use when

- Many small Markdown or JSON files must merge offline across nodes.
- Two mesh nodes sync edits conflict-free over store-and-forward links.
- Transport is a mesh link, not a server: the sync protocol is
  transport-agnostic and tolerates intermittent connectivity.

## Do NOT use when

- Large single documents dominate: Automerge runs 2-5x slower than
  Yjs on large docs, reported as of 2026-10.
- Ecosystem breadth matters: about 85k weekly npm downloads versus
  about 920k for Yjs, reported as of 2026-10.
- Bus factor is a blocker: roughly two core maintainers.

## Details

License MIT. Repository activity is public with a push on 2026-10-09;
6,656 stars, reported as of 2026-10. Platforms: linux, macos, windows.
A CRDT gives convergence, not a product: persistence, transport, and
authentication still need to be built around it.

## Evidence

Desk research sweep on 2026-10-12; no first-party smoke test yet.

### Alternatives considered

- Yjs: faster on large documents and a far larger user base; weaker
  fit for Git-like document history out of the box.
- Building merge by hand: rejected — innovation tokens go to the OS
  and the agent, not sync plumbing.

## Links

- https://github.com/automerge/automerge
- https://automerge.org
