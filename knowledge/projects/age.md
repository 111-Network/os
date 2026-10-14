---
title: age
category: project
status: candidate
summary: File-encryption format with X25519 recipients; planned for history export only.
tags: [storage, identity, offline]
platforms: [linux, android, macos, windows]
upstream: https://github.com/str4d/rage
license: MIT OR Apache-2.0
last_verified: 2026-10-14
sources:
  - https://github.com/str4d/rage
  - https://raw.githubusercontent.com/C2SP/C2SP/main/age.md
---

File-encryption format with X25519 recipients; planned for history export only.

## What it is

age is a simple file-encryption format (specified in C2SP) and `rage` is its Rust implementation;
the `age` crate is at 0.12.1 (rage release 2026-07-14).
Layer 5 reserves it for exports: a phone can encrypt a backup of message history to a hub's `enc_pubkey`
with no new key type, matching Layer 2's deferred recipient-mode export.
It is deferred, not built in v1; the CLI instruction stands in until then.

## Use when

- A history or backup file must be encrypted to a recipient key that is already an X25519 public key.
- The "export before prune" offer needs an export format (spec §10.4, §21).

## Do NOT use when

- Live traffic is being protected; sealing belongs to the Layer 2 packet, not to age.
- Forward secrecy or sender authentication is needed; age files give neither by default.
- Streaming small messages: the header costs bytes that LoRa cannot spare.

## Details

Format facts (spec text, C2SP `age.md`): a one-recipient header is 168 bytes plus a 16-byte nonce;
the payload is ChaCha20-Poly1305 in 64 KiB STREAM chunks; an X25519 recipient is a raw public key.
rage is dual licensed MIT OR Apache-2.0; the MIT file was read in the Layer 5 research.
The workspace `Cargo.toml` of the `age` crate inherits its license from the workspace root,
which this pass did not re-read.
Pruning rule that depends on it: own history is pruned oldest-first only at hard capacity and only after
an export offer.

## Evidence

### Alternatives considered

- A per-record format of our own (XChaCha20-Poly1305) — we would invent and audit it.
- Exporting the SQLCipher file as-is — keeps the key coupling to the identity keyset.
- Sealing the export with Layer 2 packets — needs a live peer and per-packet sizing, wrong for files.
- 2026-10-14 — crate version read from upstream; license inherited from workspace, MIT file verified in report 05.

## Links

- [rage repository](https://github.com/str4d/rage)
- [age specification](https://raw.githubusercontent.com/C2SP/C2SP/main/age.md)
