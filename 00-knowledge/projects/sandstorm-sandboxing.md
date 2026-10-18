---
title: Sandstorm Sandboxing
category: project
status: candidate
summary: Case study of subdomain-per-app isolation; wildcard DNS is absent on offline LANs.
tags: [panel, networking]
platforms: [linux]
upstream: https://github.com/sandstorm-io/sandstorm
license: Apache-2.0
last_verified: 2026-10-14
sources:
  - https://github.com/sandstorm-io/sandstorm
  - https://raw.githubusercontent.com/sandstorm-io/sandstorm/master/docs/administering/wildcard.md
  - https://raw.githubusercontent.com/sandstorm-io/sandstorm/master/LICENSE
  - https://www.rfc-editor.org/rfc/rfc6761
---

Case study of subdomain-per-app isolation; wildcard DNS is absent on offline LANs.

## What it is

Role: case study, not a dependency. Sandstorm is a self-hostable web app package manager that
security-hardens each app. Its hosting rule is that every app gets its own hostname, because path-based URLs
are ruled out by the same-origin policy.
Layer 5 takes the isolation reasoning and rejects the mechanism for LAN use.

## Use when

- Reasoning about why panel isolation needs a separate origin per hosted app.
- Evaluating per-session random hostnames and a cookie-checked proxy as a hardening idea.

## Do NOT use when

- The network is an offline LAN: wildcard DNS is needed, and mDNS cannot supply wildcards.
- Android clients matter: `.local` names do not resolve there.
- Safari on older macOS matters: `*.localhost` failed through macOS 15.7.

## Details

Reported as of 2026-10-14: about 7,000 stars, JavaScript, repository pushed 2026-10-13. GitHub's license
detector returns no result, but the `LICENSE` file states Apache License 2.0 for Sandstorm's own files,
listing a few vendored files under other licenses (for example an MIT spinner image).
Origin facts behind the Layer 5 choice: a different port on the same host is a different origin yet the same
site, so the dashboard frame, ticket swap, and same-origin fetches work; cookies are shared across ports
(RFC 6265 §8.5) while origin storage is not.
Per-panel `.local` names are not viable: mDNS has no wildcards, and nss-mdns accepts two-label names by default.
`<id>.111.localhost` may be added later as an additive prettification on loopback only;
RFC 6761 §6.3 lets applications special-case `.localhost`, and Chromium and Firefox 84+ do.

## Evidence

### Alternatives considered

- [[home-assistant-ingress]] — sub-path proxying; shares origin with the dashboard.
- Port-per-panel (Umbrel, CasaOS pattern) — adopted: works for any app and any host form on a LAN.
- `<id>.111.localhost` on one listener — separate cookie jars, but unreliable on older Safari and useless over LAN.
- 2026-10-14 — license read from the upstream `LICENSE`; design notes from `docs/administering/wildcard.md`.

## Links

- [Upstream repository](https://github.com/sandstorm-io/sandstorm)
- [Wildcard DNS design notes](https://github.com/sandstorm-io/sandstorm/blob/master/docs/administering/wildcard.md)
- Related: [[home-assistant-ingress]], [[secure-contexts]]
