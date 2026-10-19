---
title: SimpleX Chat
category: project
status: candidate
summary: Haskell chat CLI with a WebSocket bot API; AGPL-3.0, glibc only.
tags: [messaging, bridge, offline]
platforms: [linux]
upstream: https://github.com/simplex-chat/simplex-chat
license: AGPL-3.0-only
last_verified: 2026-10-19
sources:
  - https://github.com/simplex-chat/simplex-chat
  - https://raw.githubusercontent.com/simplex-chat/simplex-chat/stable/LICENSE
  - https://github.com/simplex-chat/simplex-chat/blob/stable/bots/README.md
  - https://github.com/simplex-chat/simplexmq
relations:
  alternative_to: ["[[deltachat-rpc-server]]"]
---

Haskell chat CLI with a WebSocket bot API; AGPL-3.0, glibc only.

## What it is

SimpleX Chat is a messenger without user identifiers. Its terminal client runs as a local WebSocket server with a
documented bot API, and `smp-server` (from `simplexmq`) is the relay it uses, which can run on a LAN with no domain.
Layer 6 treats it as the second app-peer adapter, built after Delta Chat, and only if the owner selects it.
The README says AGPLv3 and the `LICENSE` file is the GNU AGPL version 3 text; the SimpleX name, logo, and graphic
assets are excluded and fall under a separate trademark and assets licence.

## Use when

- A contact base already uses SimpleX and a hub should hold a bot profile for it.
- The hub is a Debian or Ubuntu class machine with a self-hosted `smp-server` on the LAN.

## Do NOT use when

- The host is Termux or musl-only: the CLI docs name no Android build and the binaries target Ubuntu glibc.
- The Raspberry Pi base configuration is at stake: it is an optional component and weighs nothing only while off.
- Branding is reused: the name and logo are not covered by the AGPL.

## Details

Reported as of 2026-10-19 (research report 03): CLI v7.0.3 (2026-09-26); simplexmq v7.1.0-beta.4 (2026-10-17); 19.6k
stars;
Ubuntu 22.04 and 24.04 binaries for x86_64 and aarch64; dynamic linking unverified; nothing was run.
Bot API: `simplex-chat -p <port>` serves JSON commands with correlation ids over WebSocket; bot profiles use
`peerType: "bot"`. Layer 6 draft port is loopback 11232.
Offline server: `smp-server init -y -l --ip <LAN address>`; the documented default port is 5223 (doc, not run).
Without a domain the invitation links use the non-browser `simplex:` scheme. Preset public servers have operator terms,
so they stay off by default. A bridge makes the hub an endpoint that holds the plaintext; per-contact opt-in applies.
AGPL: a separate process; the source offer names the pinned tags.

## Evidence

### Alternatives considered

- [[deltachat-rpc-server]] — lighter integration (stdio, no port) and an Android build; chosen first.
- Briar headless — Tor only on Linux, so not an offline bridge ([[briar-bramble]]).
- 2026-10-19 — licence read from the upstream `LICENSE` and README; versions from the research pass; no code run.

## Links

- [Upstream repository](https://github.com/simplex-chat/simplex-chat)
- [Bot API README](https://github.com/simplex-chat/simplex-chat/blob/stable/bots/README.md)
- Server code: [simplexmq](https://github.com/simplex-chat/simplexmq)
