---
title: Nostr
category: protocol
status: candidate
summary: Simple open protocol for global messaging over relays, keys as identity.
tags: [networking, identity]
aliases: [Nostr protocol]
upstream: https://github.com/nostr-protocol/nips
last_verified: 2026-10-12
sources:
  - https://github.com/nostr-protocol/nips
  - https://github.com/nostr-protocol/nostr
  - https://nostr.com/
---
Simple open protocol for global messaging over relays, keys as identity.

## What it is

A minimal protocol for publishing signed events to relays and subscribing to
other users' events by public key. Identity is a secp256k1 keypair; events are
JSON with a Schnorr signature; relays are dumb replication servers chosen by
each client. Standards evolve as NIPs, Nostr Implementation Possibilities,
maintained in the protocol's specification repository.

## Use when

A key-based, relay-agnostic messaging or identity fabric is needed where
users (or agents) bring their own keys and tolerate relay plumbing. It is a
candidate for the identity-adjacent messaging layer on always-on nodes, and
its NIP process is a template for spec evolution the OS can mirror.

## Do NOT use when

Offline-first mesh delivery is the requirement: nostr expects reachable
relays and does not define store-and-forward radio transport. It is also the
wrong tool where human-readable names, contact lists, or trust graphs are
needed without extra protocol layers.

## Details

Desk research, verified 2026-10-12. The NIPs repository declares all NIPs
public domain via a README statement; there is no formal license file, which
is recorded here as provenance rather than an SPDX expression. Protocol gate
adaptation applies: spec stability plus independent implementations replace
repo-activity gates. Multiple independent relay implementations exist in
several languages, and client diversity is broad, so the implementation
diversity gate passes on paper. Relay selection, event storage cost, and spam
control remain the open operational problems the OS would need to solve for
its own deployments.

## Evidence

### Alternatives considered

2026-10-12: desk research sweep. ActivityPub was weighed as the federated
alternative; it has heavier server requirements and weaker identity
portability. Matrix was weighed as the rich-state alternative; its
server-side complexity is the wrong fit for ESP32-class or intermittent
links. Secure Scuttlebutt was noted as closer to offline-first but with a
smaller 2026 ecosystem. Nostr stays the leading candidate for key-based
identity messaging, pending a defined relay strategy.

## Links

Spec home: https://github.com/nostr-protocol/nips
