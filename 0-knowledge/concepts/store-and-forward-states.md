---
title: Delivery States
category: concept
summary: Four delivery states and the evidence each one requires.
tags: [messaging, dtn, offline]
aliases: [store-and-forward states, queued sent relayed delivered]
sources:
  - https://raw.githubusercontent.com/markqvist/LXMF/master/LXMF/LXMessage.py
  - https://briarproject.org/news/2023-briar-mailbox-released/
  - https://meshtastic.org/docs/overview/mesh-algo/
  - https://www.ietf.org/archive/id/draft-ietf-dtn-bibect-03.txt
relations:
  relates_to: ["[[briar-mailbox]]", "[[lxmf]]", "[[bundle-protocol-v7]]"]
---

Four delivery states and the evidence each one requires.

## What it is

The Messages view shows each outgoing message in one of four states: `queued`, `sent`, `relayed`, `delivered`.
Each state is a claim, and each claim needs a specific piece of evidence; nothing is shown on a guess.
Retries happen on link and contact events, never on wall-clock timers, and the sender keeps its copy until
`delivered`. The semantics are spec §9; the custody receipt is §6.2.

## Use when

- Writing UI copy, API reason codes, or agent explanations for message status.
- Judging what another system's "sent" or "delivered" tick actually promises.

## Do NOT use when

- A hop-by-hop link acknowledgement is meant: that belongs to the link adapter, not these states.
- A cross-hub delivery guarantee is assumed: v1 custody works between members of one hub or over a direct link.

## Details

- `queued` — the message is in the outbox: no path, key missing, or the recipient's hub is not yours.
  Reasons shown: waiting for their key; their hub is not yours; restore pending.
- `sent` — a frame was written to a link (LoRa: the modem accepted TX), or a custody reject came back.
  Reasons shown: your hub does not know them; mailbox full.
- `relayed` — an authenticated `CUSTODY_ACK` with `ACCEPTED` from a mailbox-role node for a known member.
  Copy: "a mailbox node saved it and will hold it for them".
- `delivered` — the recipient's own `ACK`; only this releases the sender's copy.

Custody receipt statuses: 1 `ACCEPTED`, 2 `DUPLICATE`, 3 `REJECT_FULL`, 4 `REJECT_UNENROLLED`,
5 `REJECT_UNKNOWN_DEST`. The receipt carries `pkt_ref`, a 16-byte BLAKE2b hash of the exact sealed bytes,
which proves the custodian received them. `relayed` is the custodian's promise, not delivery.
An overheard repeater echo is detail on `sent`, never `relayed`.
A full store refuses new custody and never prunes undelivered outbound or uncommitted member custody.

Prior art: LXMF propagated sends end in `SENT`; `DELIVERED` is set only for opportunistic and direct sends.
The Briar mailbox reportedly shows one tick on upload and two on acknowledgement (secondary source).
In Meshtastic a broadcast is acknowledged implicitly by any rebroadcast and a direct message only by its
recipient; no intermediate acceptance signal is documented. BPv7 moved custody out to the BIBE draft.

## Evidence

- State meanings and receipt format are from the locked Layer 5 spec; prior-art claims from research
  reports 07-gap-2 and 01, with the Briar tick behavior flagged as secondary.
- No field data exists yet; the three-node harness (spec §20 G1) is the test that exercises all four states.

## Links

- [Briar mailbox announcement](https://briarproject.org/news/2023-briar-mailbox-released/)
- [Meshtastic mesh algorithm](https://meshtastic.org/docs/overview/mesh-algo/)
- Related: [[briar-mailbox]], [[lxmf]], [[bundle-protocol-v7]]
