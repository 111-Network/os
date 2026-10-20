---
title: Madmail
category: project
status: candidate
summary: Rust single-binary chatmail relay; AGPL-3.0-or-later, young, glibc 2.38 floor.
tags: [messaging, bridge, offline]
platforms: [linux]
upstream: https://github.com/themadorg/madmail
license: AGPL-3.0-or-later
last_verified: 2026-10-19
sources:
  - https://github.com/themadorg/madmail
  - https://raw.githubusercontent.com/themadorg/madmail/main/LICENCE
  - https://raw.githubusercontent.com/themadorg/madmail/main/README.md
relations:
  required_by: ["[[deltachat-rpc-server]]"]
  relates_to: ["[[delta-chat-securejoin]]"]
---

Rust single-binary chatmail relay; AGPL-3.0-or-later, young, glibc 2.38 floor.

## What it is

Madmail (repository `themadorg/madmail`) is a Rust rewrite of a chatmail relay: SMTP, IMAP, and HTTPS in one binary,
with an IP-only mode and a self-signed certificate, so a LAN hub can relay Delta Chat mail with no domain and no
internet.
Layer 6 runs it as an optional, separate-process component beside [[deltachat-rpc-server]].
License: the `LICENCE` file is the GNU AGPL version 3 text; the README and `Cargo.toml` say `AGPL-3.0-or-later`.

## Use when

- A Delta Chat bridge account needs a relay on the same LAN with no public chatmail server.
- The hub can be a glibc 2.38 or newer host (Debian 13, Pi OS trixie, Ubuntu 24.04).

## Do NOT use when

- The host is Debian 12, Pi OS bookworm, Ubuntu 22.04, or Termux: the release binary does not run there.
- The relay must stay off the Raspberry Pi base configuration: it is about 35 MB RSS while on.
- A mature, audited relay is required: the README says "Use at your own risk" and notes AI-assisted Rust.

## Details

Spike on 2026-10-19 (release v2.30.5, commit `22db4602768364955670b0f08dd97f03db6e22cc`, linux amd64 binary
sha256 `ad89fb23de9580cd736c7caa8136422415cb8779954c16e366d94f9325b989d1`): started offline in IP-only self-signed mode,
two headless clients exchanged messages, zero `connect()` calls in about 10 minutes, 77 MB unstripped (61 MB stripped).
Pin by release, not tag: tags run past v2.35.2 but assets exist only on published releases.
Ports: the standard chatmail ports (25, 143, 465, 587, 993, 80, 443) need `CAP_NET_BIND_SERVICE`; every listener can
move
to high ports with no capability, and the `dclogin:` account URI carries the ports. Termux cannot bind the standard
ones.
`madmail install` writes under `/usr/share`; the component unpacks the binary and writes its own config instead.
Push notices go to `notifications.delta.chat` unless `madmail push off`. Known quirk: IMAP `UIDNEXT` decreases on fetch.
Network use of AGPL software carries the section 13 duty: offer the pinned source to its users.

## Evidence

### Alternatives considered

- The Debian chatmail relay (Postfix, Dovecot, Nginx, OpenDKIM; MIT) — a full mail stack on Debian 12, heavier than
  one binary.
- A public chatmail relay — needs internet and shows the operator's terms; off by default.
- 2026-10-19 — licence files and README read upstream; netns spike run on x86_64 (research report 08); arm64 not run.

## Links

- [Upstream repository](https://github.com/themadorg/madmail)
- [Licence file](https://raw.githubusercontent.com/themadorg/madmail/main/LICENCE)
- Client side: [[deltachat-rpc-server]]; contact verification: [[delta-chat-securejoin]]
