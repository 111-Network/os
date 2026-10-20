---
title: SQLCipher
category: project
status: candidate
summary: Page-level encrypted SQLite; the 4.x and 5.0 file formats are incompatible.
tags: [storage, messaging, offline]
platforms: [linux, android, macos, windows]
upstream: https://github.com/sqlcipher/sqlcipher
license: BSD-3-Clause
last_verified: 2026-10-14
sources:
  - https://github.com/sqlcipher/sqlcipher
  - https://www.zetetic.net/sqlcipher/design/
  - https://www.zetetic.net/sqlcipher/sqlcipher-api/
  - https://www.zetetic.net/blog/2026/09/15/sqlcipher-5.0.0-beta/
relations:
  required_by: ["[[rusqlite]]"]
---

Page-level encrypted SQLite; the 4.x and 5.0 file formats are incompatible.

## What it is

SQLCipher Community Edition is SQLite with transparent page-level encryption
(AES-256-CBC with an HMAC-SHA512 per page).
The Layer 5 store opens it with a raw 32-byte key derived from the identity keyset.
License: the upstream `LICENSE.md` is the BSD 3-clause text with ZETETIC LLC as the named party.
Static builds also carry OpenSSL 3.x, which is Apache-2.0.

## Use when

- At-rest encryption of a relational store is required without writing a record format.
- The key comes from the identity layer, so a copied database reopens after restore-from-words.
- Backups and synced app directories must not expose message contents.

## Do NOT use when

- Protection against a stolen disk with a readable identity directory is expected; it is not provided.
- The key would be bound to an OS keyring with no recovery path.
- The 4.x file format cannot be pinned: 5.0 changes defaults and needs `cipher_migrate`.

## Details

Reported as of 2026-10-14: community 4.19.0 (2026-09-08); rusqlite bundles 4.17.0.
Raw key mode (`x'<64 hex>'`) skips PBKDF2, whose default of 256,000 iterations is too slow per open on phones.
Page overhead is a 16-byte IV plus a 64-byte HMAC per 4,096-byte page, about 1.95% (derived; exact reserve unverified).
Transient files are not encrypted, so temp storage must stay in memory.
Layer 5 pins the file format by setting the `cipher_compatibility` pragma to 4.

SQLCipher 5.0.0-beta (2026-09-15) moves to AES-256-GCM, VFS shims, 8 KiB pages, and 512K KDF iterations.
It is incompatible with 4.x by default, so a migration is planned behind the Layer 1 updater.

Lesson from Signal Desktop, which uses an AGPL SQLCipher fork: its key moved from a plaintext config
file to Electron `safeStorage`, and a keyring backend change can strand the database.
Briar instead keeps a random 32-byte key encrypted under a password-derived key.
Termux packages SQLCipher, but a Termux run of the Layer 5 build is an external gate, not yet done.

## Evidence

### Alternatives considered

- SQLite3 Multiple Ciphers — MIT, ChaCha20 option, no OpenSSL, but a thin Rust ecosystem (3k downloads).
- Plain SQLite with application-layer AEAD — smaller, but the record format and metadata leakage become ours.
- SQLCipher 5.0 now — beta only, incompatible defaults, and rusqlite lags upstream.
- 2026-10-14 — license read from upstream `LICENSE.md` (ZETETIC LLC, three-clause BSD text);
  format and cost claims from the Layer 5 research (report 05) and Zetetic design pages.

## Links

- [Upstream repository](https://github.com/sqlcipher/sqlcipher)
- [Design notes](https://www.zetetic.net/sqlcipher/design/)
- [5.0.0 beta announcement](https://www.zetetic.net/blog/2026/09/15/sqlcipher-5.0.0-beta/)
- Rust binding: [[rusqlite]]
