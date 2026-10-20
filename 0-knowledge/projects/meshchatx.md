---
title: MeshChatX
category: project
status: candidate
summary: Reticulum LXMF web client fork; unaudited and parked behind the generic proxy.
tags: [panel, messaging, mesh]
platforms: [linux, android]
upstream: https://github.com/Quad4-Software/MeshChatX
license: 0BSD AND MIT
last_verified: 2026-10-19
sources:
  - https://github.com/Quad4-Software/MeshChatX
  - https://raw.githubusercontent.com/Quad4-Software/MeshChatX/master/LICENSE
  - https://github.com/liamcottle/reticulum-meshchat
  - https://reticulum.network/manual/software.html
relations:
  depends_on: ["[[reticulum]]", "[[lxmf]]"]
---

Reticulum LXMF web client fork; unaudited and parked behind the generic proxy.

## What it is

MeshChatX is a fork of Reticulum MeshChat: a Python web client for LXMF messaging that embeds Reticulum
in-process. It is the intended first consumer of the reserved generic panel reverse proxy (parked item P7).
It is a bridge target with its own LXMF identity, not part of the 111 fabric. Since Layer 6 it is no longer the LXMF
carrier (that is the helper `lxmf-sidecar`); it may return later as an optional hosted panel.
The `LICENSE` file reads: 0BSD for the Quad4 modifications, MIT for the upstream MeshChat portions.

## Use when

- A hosted Reticulum client panel is wanted once the generic proxy with WebSocket support exists.
- The client must run headless behind a proxy: `--no-https`, `--auth`, and `--reticulum-config-dir` exist.

## Do NOT use when

- An audited client is required: no audit is known for this fork.
- The WebSocket proxy is not built: the client depends on `/ws`, and v1 ships static panels only.
- A shared-instance posture change is unacceptable: it needs `share_instance = yes` in rnsd.

## Details

Reported as of 2026-10-19: v4.10.7 (2026-10-11); listed in the Reticulum manual's software chapter. Its README announces
a sunset on 2027-01-01 with v5.0.0 as the last release, and its floors rise with `rns` (4.10.7 needs `rns>=1.5.7`).
Delivery: PyPI wheel with the UI bundled, Docker, and a Termux guide.
Proxy behavior: a WebSocket Origin check on `/ws` and `/ws/telephone/audio` that needs
`MESHCHAT_TRUSTED_PROXIES` plus `X-Forwarded-Host`, `-Proto`, and `-Port` behind a proxy.
It drops `X-Frame-Options` and sends `frame-ancestors` only in demo mode, has Landlock and seccomp sandboxes,
and an offline-ready map. Audio calls need `localhost` or HTTPS (AudioWorklet), so they fall under
[[secure-contexts]] limits.
Whether the fork keeps MeshChat's `/api/v1` send and receive API is unverified.
Each client carries its own LXMF identity; the second identity is a bridge, never the 111 NodeId.

## Evidence

### Alternatives considered

- MeshChat (original) — MIT, v2.4.0 (2026-07-06), about 1.3k stars. Headless flags exist, but `meshchat.py` has
  no authentication or Origin handling, and it binds `127.0.0.1:8000`. The frontend calls root-absolute
  `/api/v1/...`, so it needs an origin root. Its `POST /api/v1/lxmf-messages/send` takes content, image, audio,
  or file attachments only, with no arbitrary field. Accepts 50 MB uploads. The kept fallback.
- Sideband — CC BY-NC-SA 4.0, rejected. Columba — MPL-2.0 native Android app, no external API seen.
- NomadNet — GPL-3.0 text UI with no web UI; MeshChat already browses its pages.
- 2026-10-14 — license read from the upstream `LICENSE`; the fork's provenance is for the review panel to judge.
- 2026-10-19 — Layer 6 research: sunset notice read; the carrier role moved to the helper.

## Links

- [MeshChatX repository](https://github.com/Quad4-Software/MeshChatX)
- [MeshChat original](https://github.com/liamcottle/reticulum-meshchat)
- Stack: [[reticulum]]; format: [[lxmf]]
