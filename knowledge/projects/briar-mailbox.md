---
title: Briar Mailbox
category: project
status: candidate
summary: Case study of the Briar home mailbox; QR pairing and Tor-only transport.
tags: [messaging, dtn, offline]
platforms: [android]
upstream: https://code.briarproject.org/briar/briar-mailbox
license: AGPL-3.0-or-later
last_verified: 2026-10-14
sources:
  - https://code.briarproject.org/briar/briar-mailbox
  - https://github.com/briar/briar-mailbox
  - https://briarproject.org/news/2023-briar-mailbox-released/
  - https://code.briarproject.org/briar/briar/-/tree/master/briar-headless
relations:
  relates_to: ["[[briar-bramble]]", "[[store-and-forward-states]]"]
---

Case study of the Briar home mailbox; QR pairing and Tor-only transport.

## What it is

Role: case study, not a dependency. Briar Mailbox is an always-on helper that holds a user's Briar
messages while their phone is offline, paired to the phone by QR code and reached over Tor.
It is the closest prior art for the Layer 5 home-hub mailbox: custody of sealed items for an absent member.
The code is an Android project in Kotlin; the GitHub copy is a mirror of the code.briarproject.org repository.

## Use when

- Designing hub custody, enrollment by QR, or the meaning of a "stored at the mailbox" state.
- Checking what a one-tick versus two-tick delivery display promises.

## Do NOT use when

- Code reuse is the goal: AGPL-3.0-or-later, a Briar identity, and a Tor transport do not fit a LAN hub.
- An offline path is needed from `briar-headless`: on Linux it registers only the Tor plugin, so no LAN and
  no Bluetooth; the offline transports exist only in the Android app, which has no API.
- Current maintenance is assumed: the GitHub mirror shows no releases and a last push of 2025-01-02,
  and the canonical host returned 403 to the research fetches.

## Details

Reported as of 2026-10-14: the GitHub mirror was created 2023-05-11, has 20 stars, and its license folder
holds `AGPL-3.0-or-later` (REUSE layout). The mailbox was announced in 2023.
Reportedly its test plan shows one tick on upload to the mailbox and two on the recipient's acknowledgement
(secondary source; the wiki page returned 403 to the research fetch).

What Layer 5 takes from it: the two-stage meaning. `relayed` is evidence that a mailbox-role node saved
the item for a known member (an authenticated custody receipt), and `delivered` waits for the recipient's
own ACK. Enrollment mirrors QR pairing, but over a LAN `tcp-lan` link with a one-time code, not Tor.
`briar-headless` (REST on 127.0.0.1:7000 with a bearer token, Linux jars) is gateway-class only.
See [[store-and-forward-states]] for the semantics adopted.

## Evidence

### Alternatives considered

- LXMF propagation node ([[lxmf]]) — its propagated sends end in `SENT`, so it offers no custody signal.
- Meshtastic Store and Forward — a ring buffer of about 11,000 records, 240-minute default window,
  25 per request; the port is marked work in progress and gives the sender no acknowledgement.
- The Layer 5 mailbox (`CUSTODY_PUT`, `CUSTODY_ACK`, `MAILBOX_FETCH`, `MAILBOX_COMMIT`) — the chosen design.
- 2026-10-14 — license folder and metadata read from the GitHub mirror; behavior from research report 07-gap-2.

## Links

- [Upstream repository](https://code.briarproject.org/briar/briar-mailbox)
- [GitHub mirror](https://github.com/briar/briar-mailbox)
- [Mailbox announcement](https://briarproject.org/news/2023-briar-mailbox-released/)
- Related: [[briar-bramble]], [[store-and-forward-states]]
