---
title: Delta Chat SecureJoin
category: project
status: candidate
summary: Case study of invite-based contact verification; not extractable as a crate.
tags: [identity, messaging, offline]
platforms: [linux, android, ios, macos, windows]
upstream: https://github.com/chatmail/core
license: MPL-2.0
last_verified: 2026-10-14
sources:
  - https://github.com/chatmail/core
  - https://github.com/chatmail/core/blob/main/src/securejoin.rs
  - https://securejoin.readthedocs.io/en/latest/summary.html
  - https://github.com/chatmail/core/blob/main/deltachat-rpc-server/README.md
relations:
  relates_to: ["[[briar-bramble]]"]
---

Case study of invite-based contact verification; not extractable as a crate.

## What it is

Role: case study. SecureJoin is the contact-verification protocol inside the Delta Chat core (chatmail core,
Rust, MPL-2.0). One out-of-band message, a QR or link, carries a key fingerprint and contact info;
the rest of the handshake then runs over a channel an attacker may control.
Layer 5 copies the shape for `node1…`-only contact adds, not the code.

## Use when

- Designing first contact where one trusted out-of-band fact bootstraps everything else.
- Designing how a second device of the same person is introduced (QR with a peer id and token).

## Do NOT use when

- A reusable crate is expected: its entry points take a `&Context` bound to SQL, SMTP, and MIME.
- It would be the fabric: Delta Chat's transport is IMAP and SMTP and its identity is OpenPGP.
- A browser panel is wanted: the desktop browser edition is described as highly experimental and for
  developers and automated testing only.

## Details

Reported as of 2026-10-14: chatmail core v2.63.0 (2026-10-09), MPL-2.0 per its `Cargo.toml`.
The QR payload has the form `fingerprint&v&i&s&a`: invite number, auth token, address.
Mapping to 111 (spec §7.2): the full `node1…` string plays the fingerprint; a one-time enrollment code
plays the invite number and auth token. A node holding only a `node1…` sends a cleartext card request to its
hub, which answers only for enrolled contacts from stored bytes; the reply is verified against the expected
NodeId and then pinned. Until then messages stay `queued`, reason "waiting for their key".

Other facts used: "Add Second Device" is a QR code (peer id plus auth token) that starts a one-shot iroh QUIC
transfer of the database and blobs, while Delta Chat core still pins iroh 0.35. The core uses rusqlite with
vendored SQLCipher by default but the passphrase is optional (see [[rusqlite]]). `deltachat-rpc-server` speaks
JSON-RPC 2.0 over stdio, so a hub could be a bot contact. The chatmail relay (MIT) has an experimental IP-only
mode that suits a LAN without DNS.

## Evidence

### Alternatives considered

- Briar's static-handshake introduction ([[briar-bramble]]) — links bound to the long-term key, no rotation.
- Signal safety numbers — a fingerprint-comparison UX only.
- The card (own genesis announce as QR) — also built, as `trust = oob`; SecureJoin informs the `node1…`-only path.
- 2026-10-14 — license read from upstream `Cargo.toml`; protocol facts from the SecureJoin documentation.

## Links

- [Upstream repository](https://github.com/chatmail/core)
- [SecureJoin summary](https://securejoin.readthedocs.io/en/latest/summary.html)
- Related: [[briar-bramble]]
