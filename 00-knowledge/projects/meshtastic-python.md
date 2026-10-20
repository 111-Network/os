---
title: Meshtastic Python
category: project
status: candidate
summary: Meshtastic Python library and CLI; GPL-3.0-only with no consistent maintainer.
tags: [lora, mesh, messaging]
platforms: [linux, macos, windows]
upstream: https://github.com/meshtastic/python
license: GPL-3.0-only
last_verified: 2026-10-19
sources:
  - https://github.com/meshtastic/python
  - https://python.meshtastic.org/mesh_interface.html
  - https://raw.githubusercontent.com/meshtastic/python/master/pyproject.toml
relations:
  depends_on: ["[[meshtastic]]"]
---

Meshtastic Python library and CLI; GPL-3.0-only with no consistent maintainer.

## What it is

The official Python API and command-line tool for a Meshtastic node, over serial, TCP, or Bluetooth.
Layer 5 uses it only as the dependency of `meshtastic-presence`, a tiny read-only sidecar
that feeds the Nearby list, run in its own source directory and hash-locked virtualenv.
`pyproject.toml` declares `GPL-3.0-only`.

## Use when

- Node, position, and telemetry events from a board are needed with the least custom code.
- The reader can connect to a TCP downlink (meshmud) instead of the board itself.
- A separate GPL process boundary is acceptable and its source offer is carried.

## Do NOT use when

- A client inside the Rust daemon is wanted: after D22 in-binary protobufs are no longer a blocker, and the
  in-process client is [[meshtastic-rust]].
- The reader must send anything: the presence reader has no send path by design.
- A guaranteed maintainer is required: the README says it lacks a consistent one.

## Details

Reported as of 2026-10-14: 2.7.11 (2026-07-17).
Interfaces: `SerialInterface`, `TCPInterface`, `BLEInterface`; `sendText`, `sendData` with
`portNum=PRIVATE_APP`, `nodesByNum`, and pubsub receive topics.
One-client fact: a Meshtastic node, and meshtasticd, serves one API client, so the reader attaches to
[[meshmud]]'s TCP downlink on `127.0.0.1:11230` rather than opening the board.

Reader behavior (spec §14): sends `want_config` once, subscribes to NodeInfo, position, and telemetry,
and writes `fabric/presence/meshtastic.json` atomically. Node rows carry `user` (id, names, `publicKey`),
`snr`, `hopsAway`, `lastHeard` (node clock, a hint only), `deviceMetrics`, and `position`.
Names are untrusted RF-origin strings: truncated to 40 bytes, control characters stripped, rendered as text.
GPL boundary: separate process, own source directory, source offer naming the pinned tag and pip closure.

## Evidence

### Alternatives considered

- [[meshmonitor]] REST API — richer fields and MeshCore coverage, but a 189 MB Node image that sends
  extra requests to the node by default.
- A Rust adapter on prost-generated protobufs — after D22 no longer a blocker; the binary then ships as GPL-3.0-only,
  and the reader retires after a parity gate ([[meshtastic-rust]]).
- No presence at all — rejected: the Nearby list is a stated goal.
- 2026-10-14 — license read from upstream `pyproject.toml`; no field run of the reader yet.
- 2026-10-19 — Layer 6 retires the reader after the parity gate; the pin stays at 2.7.11 until a 2.8-capable release
  exists.

## Links

- [Upstream repository](https://github.com/meshtastic/python)
- [API documentation](https://python.meshtastic.org/mesh_interface.html)
- Node: [[meshtastic]]; multiplexer: [[meshmud]]
