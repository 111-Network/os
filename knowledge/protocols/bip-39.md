---
title: BIP-39
category: protocol
status: candidate
summary: Mnemonic seed-phrase standard; checksum is weak and format carries no version.
tags: [identity, offline]
upstream: https://github.com/bitcoin/bips
last_verified: 2026-10-12
sources:
  - https://github.com/bitcoin/bips
  - https://docs.electrum.org/en/latest/seedphrase.html
relations:
  required_by: ["[[slip-0010]]"]
---

Mnemonic seed-phrase standard; checksum is weak and format carries no version.

## What it is

BIP-39 is the Bitcoin standard for encoding entropy as a mnemonic phrase:
12 words for 128-bit entropy, 24 words for 256-bit, from a fixed
2048-word English list, with a checksum of the first entropy-over-32 bits
of SHA-256 and a 2048-round PBKDF2-HMAC-SHA512 seed derivation. It is
the universal wallet backup format and the identity layer's root-secret
encoding: 12 words by default, 24 for hub-class nodes.

## Use when

- A human must write down, verify, and later retype a node's root secret
  with zero infrastructure.
- Universal tooling interop matters; every serious wallet understands it.
- Air-gapped ceremony is the norm: paper default, metal for hubs.

## Do NOT use when

- The checksum strength is over-trusted: it is 4 bits at 12 words and 8
  at 24, so a meaningful fraction of single-word errors pass silently;
  restore flows need an independent confirmation step.
- The phrase is treated as a complete backup spec; it carries no version,
  curve, or path metadata, which is Electrum's standing critique.
- A passphrase, the so-called 25th word, is considered; it has no
  checksum and any typo silently derives a valid but different identity.

## Details

Known limits recorded by the identity spec: weak KDF by 2026 standards,
no version byte, and a passphrase feature rejected for v1 because any
normalization slip silently yields a different wallet. Our backup format
wraps the mnemonic in a versioned, self-describing descriptor that pins
curve, derivation path, and encoding, and restore must validate the
checksum before deriving. Embedded cost is trivial: the wordlist is about
16KB of flash and PBKDF2-HMAC-SHA512 runs comfortably on ESP32-S3-class
parts. The spec is frozen since 2013 with implementations everywhere,
which clears the protocol category's stability gate.

## Evidence

### Alternatives considered

- Electrum's versioned seed — self-describing and deliberately not
  BIP-39; rejected for zero wallet interop.
- SLIP-39 Shamir shares — a threshold upgrade path, not a plain mnemonic;
  see [[bc-sskr]] for the audited option.
- 2026-10-12 — desk research sweep; entered at candidate under the
  protocol adaptation: stable spec plus many independent implementations.

## Links

- [Specification home](https://github.com/bitcoin/bips)
- [Electrum seed documentation](https://docs.electrum.org/en/latest/seedphrase.html)
- Required by: [[slip-0010]]
