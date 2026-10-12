---
title: Nostr
category: protocol
status: candidate
summary: Signed-event protocol with self-authenticating identity and offline-first relay sync.
tags: [sync, networking, identity]
upstream: https://github.com/nostr-protocol/nips
last_verified: 2026-10-12
sources:
  - https://nips.nostr.com
  - https://github.com/nostr-protocol/nips
---
Signed-event protocol with self-authenticating identity and offline-first relay sync.

## What it is

A simple signed-event protocol (NIP-01) over WebSocket relays. Clients
publish events to relays; relays store and forward them. Every event is
self-authenticating signed JSON, and identity is a secp256k1 Schnorr keypair
that comes free with the signature scheme — no certificate authority, no
registry, no account server. Around 89 relay implementations exist, reported
as of 2026-10.

## Use when

- A project needs portable, self-sovereign identity that signs offline and
  verifies anywhere, with no trust anchor to run.
- Catch-up-after-offline is the dominant traffic shape: two parties reconcile
  what changed while a link was down, instead of resending everything.
- A mesh application needs an internet bridge: relays gossip events globally,
  and bitchat already uses nostr this way in production.

## Do NOT use when

- The link is LoRa-class: the protocol is WebSocket-centric and needs
  adaptation before it fits low-bandwidth or store-and-forward radio links.
- Nobody can absorb relay operations: public-relay spam and DoS management is
  a real, ongoing burden on the relay operator.
- A mesh routing layer is what is needed: nostr is not a mesh protocol, it is
  a sync and identity layer, so it needs a transport such as [[reticulum]] or
  [[meshtastic]] underneath on radio paths.

## Details

- NIP-77 Negentropy set reconciliation is the key offline-first sync
  mechanism. NEG-OPEN / NEG-MSG / NEG-CLOSE frames reconcile event sets with
  a reported 10-100x bandwidth reduction, and the same reconciliation works
  relay-to-relay — designed exactly for catch-up-after-offline.
- Relay implementations: strfry (C++/LMDB, supports NIP-77, the standard for
  public relays), nostr-rs-relay (Rust/SQLite, friendly to a Raspberry Pi),
  and khatru (a Go relay framework).
- A local relay is trivially self-hosted on a Pi or Umbrel, so a node can
  hold its community's events and reconcile with the wider net whenever a
  link appears.
- Protocol gate view: NIP-01 is a stable spec with many independent
  implementations, which replaces the repo-activity gates for this category.

## Evidence

### Alternatives considered

- [[automerge]] — mergeable-document sync (CRDT); strong for application
  data, but it offers no global identity and no relay gossip layer, so it
  answers a different question than signed public events.
- [[reticulum]] — a mesh transport and routing stack; complementary rather
  than competing, since nostr needs a transport under it on radio links.
- [[meshtastic]] — LoRa mesh firmware with its own radio layer; not an
  event or identity protocol, and not a fit for internet-relay sync.
- 2026-10-12 — Desk-research sweep; entered at candidate tier per the
  three-gate rubric, protocol adaptation (spec stability plus multiple
  independent implementations).

## Links

- https://nips.nostr.com
- https://github.com/nostr-protocol/nips
