---
title: bc-sskr
category: project
status: candidate
summary: Audited C Shamir key sharing that round-trips with BIP-39 mnemonics.
tags: [identity, offline]
platforms: [esp32, linux, android]
upstream: https://github.com/BlockchainCommons/bc-sskr
license: BSD-2-Clause
last_verified: 2026-10-12
sources:
  - https://github.com/BlockchainCommons/bc-sskr
  - https://github.com/BlockchainCommons/bc-shamir
relations:
  relates_to: ["[[bip-39]]"]
---

Audited C Shamir key sharing that round-trips with BIP-39 mnemonics.

## What it is

bc-sskr is Blockchain Commons' C implementation of Shamir sharded key
reconstruction, with its bc-shamir companion library, licensed
BSD-2-Clause. It splits a seed into threshold shares and recombines them,
and it round-trips with [[bip-39]]: seed to shares back to the identical
seed, which SLIP-39 cannot do. The identity layer reserves it for the
phase-2 guardian-split backup.

## Use when

- A threshold backup of the root secret is wanted: user-chosen guardians
  hold shares, and no single guardian or hub can reconstruct alone.
- The rubric's audit gate must be met: Radically Open Security reviewed
  the C libraries in summer 2021 (Bitmark funded), with all findings
  fixed.
- Shares need QR-friendly encodings such as Bytewords or UR, or plain-C
  embedding on ESP32-S3-class hardware.

## Do NOT use when

- Shares must be word-encoded for hand transcription; SLIP-39's word
  shares win on human UX, at the cost of no third-party audit.
- A non-C runtime is required; the Java, Swift, Rust, and TypeScript
  ports were not part of the 2021 review.
- Information-theoretic share security is demanded; the accepted
  residual is that the 4-byte share HMAC gives computational, not
  information-theoretic, security.

## Details

The 2021 review's accepted residual is recorded honestly: share HMACs
are computational. Adoption includes the Coldcard ecosystem, the Ledger
Seed Tool app, and AirGap plans; Blockchain Commons deprecated its own
bc-slip39 in favor of SSKR. In the identity design, guardians are
user-chosen peers, the split protects the backup rather than live
signing, and recombination is a temporary single point of failure that UX
copy must state plainly.

## Evidence

### Alternatives considered

- SLIP-39 — dominant ecosystem share encoding (Trezor default), but no
  third-party audit of spec or reference implementation, and it cannot
  round-trip with [[bip-39]].
- Codex32 (BIP-93) — hand-computable checksums ideal for paper, but no
  formal audit and no multi-level groups.
- 2026-10-12 — desk research sweep; entered at candidate per the rubric;
  the security review covers the C libraries only.

## Links

- [Upstream repository](https://github.com/BlockchainCommons/bc-sskr)
- [bc-shamir companion](https://github.com/BlockchainCommons/bc-shamir)
- Related: [[bip-39]]
