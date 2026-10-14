---
title: Meshtastic Web
category: project
status: candidate
summary: Official Meshtastic web client under GPL-3.0-only; importing its SDK spreads the license.
tags: [panel, lora, mesh]
platforms: [linux, android, ios, macos, windows]
upstream: https://github.com/meshtastic/web
license: GPL-3.0-only
last_verified: 2026-10-14
sources:
  - https://github.com/meshtastic/web
  - https://meshtastic.org/docs/software/web-client/
  - https://github.com/meshtastic/web/blob/main/packages/sdk/package.json
  - https://meshtastic.org/docs/development/device/http-api/
relations:
  depends_on: ["[[meshtastic]]"]
  relates_to: ["[[meshmud]]", "[[secure-contexts]]"]
---

Official Meshtastic web client under GPL-3.0-only; importing its SDK spreads the license.

## What it is

The official Meshtastic browser client, a React and TypeScript monorepo (`apps/web`, `packages/sdk`,
`transport-http`, `transport-web-serial`, `transport-web-bluetooth`).
All packages declare GPL-3.0-only. Layer 5 hosts the built static bundle as the Meshtastic panel,
served unmodified from disk on its own origin, never embedded in the Rust binary and never imported as an SDK.

## Use when

- The dashboard must show Meshtastic's own UI for a bound board without building a UI of ours.
- The panel can reach the board by HTTP through the 111d shim on a plain-HTTP LAN.
- Serial or Bluetooth from the node's own browser is a loopback-only bonus.

## Do NOT use when

- Its SDK or components would be imported into the dashboard bundle; that makes the bundle GPL-derived.
- Web Serial or Web Bluetooth must work from a LAN visitor: they need a secure context.
- Offline guarantees must be absolute: optional hosts (map tiles, node API) are contacted by an online browser.

## Details

Reported as of 2026-10-14: v2.7.2 (2026-08-09) per report 03, while report 07-gap-3 reads v2.7.2 as latest;
pushed 2026-10-11. A release asset `build.tar` (2.1 MB) was reported but its presence is unverified,
so CI builds the bundle from the pinned tag when upstream ships no complete asset (v2.7.2's `build.tar` exists — 2,099,200 B, 320 gzipped files — but omits the SQLite `.wasm` and the font files the bundle references, so the source build is used).

Transports: HTTP (node HTTP API, `PUT /api/v1/toradio`, `GET /api/v1/fromradio`), Web Serial, Web Bluetooth.
The hosted copy at client.meshtastic.org needs HTTPS; our plain-HTTP hosting removes that friction.
Browser support: Web Serial is Chromium-only on desktop; Web Bluetooth is Chromium-only with none on iOS.
Serial and Bluetooth buttons work only on loopback origins; see [[secure-contexts]].
A cross-origin frame also needs the iframe `allow` delegation for `serial` and `bluetooth`.

Panel declaration (spec §13): `kind = static`, shim `meshtastic-http`, `needs_secure_context = false`,
optional hosts `tiles.openfreemap.org` and `api.meshtastic.org`, source offer naming the pinned tag.
A node serves one API client, so the bound board is reached through [[meshmud]], not directly.
What a user does in the panel (send, configure, reboot) goes to their radio; 111 does not see it.
"Meshtastic" is a registered mark and is used nominatively.

## Evidence

### Alternatives considered

- Link out to client.meshtastic.org — needs internet; breaks offline-first.
- Our own Meshtastic UI — rejected outright.
- [[meshmonitor]] UI — its Virtual Node speaks raw TCP only, so the official client cannot join it.
- Loopback Web Serial only — meets no presence goal and no LAN visitor.
- 2026-10-14 — GPL-3.0-only on every package, read from upstream manifests in the Layer 5 research
  (report 02); version facts conflict between research reports and are recorded as such.

## Links

- [Upstream repository](https://github.com/meshtastic/web)
- [Web client documentation](https://meshtastic.org/docs/software/web-client/)
- [Node HTTP API](https://meshtastic.org/docs/development/device/http-api/)
