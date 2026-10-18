---
title: Home Assistant Ingress
category: project
status: candidate
summary: Case study of sub-path panel proxying; it shares the dashboard origin.
tags: [panel, networking]
platforms: [linux]
upstream: https://github.com/home-assistant/supervisor
license: Apache-2.0
last_verified: 2026-10-14
sources:
  - https://github.com/home-assistant/supervisor
  - https://developers.home-assistant.io/docs/add-ons/presentation/
  - https://github.com/home-assistant/supervisor/blob/main/supervisor/api/ingress.py
relations:
  alternative_to: ["[[sandstorm-sandboxing]]"]
  relates_to: ["[[secure-contexts]]"]
---

Case study of sub-path panel proxying; it shares the dashboard origin.

## What it is

Role: case study, not a dependency. Ingress is how Home Assistant shows an add-on's web UI inside its own
dashboard: the Supervisor proxies the add-on under a path of the Home Assistant origin.
It is the design Layer 5 considered and rejected for hosting adopted projects' panels.

## Use when

- Studying how a dashboard authenticates and proxies HTTP and WebSocket to a hosted UI.
- Borrowing the session-ticket idea: a dashboard call mints access, and the proxy checks a session cookie.

## Do NOT use when

- Hosted UIs are third-party code: a shared origin lets their JavaScript call the dashboard's own API.
- Apps cannot run under a path prefix: root-absolute URLs (MeshChat) and Flutter apps break.

## Details

Reported as of 2026-10-14: supervisor repository Apache-2.0, about 2,200 stars, pushed 2026-10-14.
Mechanics: route `/api/hassio_ingress/{token}/{path}`; the session cookie `ingress_session` is checked;
the add-on receives `X-Ingress-Path`; only the proxy's address may connect; HTTP and WebSocket (16 MiB frames)
are bridged; response headers pass through except length, encoding, and type, so an add-on's own
`X-Frame-Options` survives; add-ons must work under a prefix.
Home Assistant accepts the shared origin because add-ons are trusted; adopted projects are not.

Layer 5's answer (spec §13, D19 d): one origin per panel by port, ticket to host-only cookie `p111_<id>`,
framing headers rewritten, no body rewriting, sub-path hosting rejected. The ticket flow echoes Ingress's
session endpoint. Related port-per-app designs: Umbrel and CasaOS open each app on its own port;
YunoHost SSOwat is an auth gateway with one signed cookie, not an embedder.

## Evidence

### Alternatives considered

- [[sandstorm-sandboxing]] — a wildcard subdomain per app; strong isolation, but no wildcard DNS offline.
- Umbrel and CasaOS — one port per app; the adopted pattern for the panel origin.
- Link-out only — zero risk but not "hosted".
- 2026-10-14 — license from the GitHub license endpoint, not read from the `LICENSE` file; mechanics from report 03.

## Links

- [Supervisor repository](https://github.com/home-assistant/supervisor)
- [Add-on presentation docs](https://developers.home-assistant.io/docs/add-ons/presentation/)
- Related: [[sandstorm-sandboxing]], [[secure-contexts]]
