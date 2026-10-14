---
title: MeshMonitor
category: project
status: candidate
summary: Mesh monitor web app with REST API; Node runtime, 189 MB image.
tags: [lora, mesh, panel]
platforms: [linux]
upstream: https://github.com/Yeraze/meshmonitor
license: BSD-3-Clause
last_verified: 2026-10-14
sources:
  - https://github.com/Yeraze/meshmonitor
  - https://github.com/Yeraze/meshmonitor/blob/main/LICENSE
  - https://github.com/Yeraze/meshmonitor/blob/v4.17.0/docs/configuration/virtual-node.md
  - https://github.com/Yeraze/meshmonitor/blob/v4.17.0/src/server/routes/v1/openapi.yaml
relations:
  relates_to: ["[[meshtastic]]", "[[meshcore]]"]
  alternative_to: ["[[meshtastic-python]]"]
---

Mesh monitor web app with REST API; Node runtime, 189 MB image.

## What it is

MeshMonitor is a web application that watches Meshtastic nodes (TCP, with serial and Bluetooth through
helper containers) and MeshCore nodes (USB or TCP). It offers a map, history, a REST API, and a Virtual Node.
Layer 5 records it as an optional later service: its own UI as a second panel and a richer presence source.
It is not the multiplexer for Meshtastic's own web client, and not the Meshtastic panel.

## Use when

- Richer presence is wanted: names, public key, SNR, RSSI, hops, position, battery, and history.
- MeshCore presence is in scope; among the options researched only MeshMonitor covers it.
- A second panel with a full monitoring UI is acceptable beside the official client.

## Do NOT use when

- Footprint matters: Node 22 or newer, native modules, and an image of about 189 MB.
- Read-only behavior is required: it sends config requests and LocalStats requests to the node.
- Loopback-only binding is required without a firewall: there is no bind-address option.

## Details

Reported as of 2026-10-14: v4.17.0 (2026-10-12), 689 stars, 102 forks, 4,333 commits.
License BSD-3-Clause, read from the LICENSE file ("meshmonitor contributors"); it covers MeshMonitor's own
code only. The build needs the GPL-3.0 `meshtastic/protobufs` submodule, and the image installs Python
`meshtastic` (GPL-3.0), `meshcore-cli`, and Apprise; run it as a separate process with a GPL source offer.
Native modules `better-sqlite3`, `bcrypt`, and `re2` must be compiled at build time; Termux is unverified.

Virtual Node: a TCP proxy (default port 4404) for the official phone apps and the Python CLI, no client
authentication, admin packets blocked unless allowed, about 240 cached config messages replayed.
It speaks raw Meshtastic TCP and has no `/api/v1/fromradio` route, so the official web client cannot join.
REST v1 uses `Authorization: Bearer mm_v1_<token>`; node rows hold `publicKey` in code but not in the OpenAPI schema.
Defaults to remember: first login `admin`/`changeme`; `TRUST_PROXY` off; `COOKIE_SECURE` off in code.
Offline: the UI has no CDN links, default basemaps are online (blank map offline), and
the `VERSION_CHECK_DISABLED` environment variable stops the GitHub version poll; self-upgrade was removed in 4.13.
Framing: `X-Frame-Options: DENY` unless `IFRAME_ALLOWED_ORIGINS` is set. Breaking changes land in minor releases.

## Evidence

### Alternatives considered

- [[meshmud]] plus the official web bundle plus a small presence reader — the chosen Layer 5 shape,
  with the least custom code for the panel and presence goals.
- [[meshtastic-python]] — the lighter read-only presence reader.
- MeshSense — GPL-3.0, last push 2025-10; weaker. PotatoMesh — Apache-2.0 Ruby map; tiles come from CDNs.
- 2026-10-14 — license read from upstream `LICENSE` and runtime facts from the v4.17.0 tarball, both in
  the Layer 5 research (report 07-gap-3).

## Links

- [Upstream repository](https://github.com/Yeraze/meshmonitor)
- [Virtual Node documentation](https://github.com/Yeraze/meshmonitor/blob/v4.17.0/docs/configuration/virtual-node.md)
- Networks covered: [[meshtastic]], [[meshcore]]
