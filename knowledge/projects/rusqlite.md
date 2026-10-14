---
title: rusqlite
category: project
status: candidate
summary: Rust SQLite binding; SQLCipher build adds 5.4 to 6.4 MB per binary.
tags: [storage, messaging, offline]
platforms: [linux, android, macos, windows]
upstream: https://github.com/rusqlite/rusqlite
license: MIT
last_verified: 2026-10-14
sources:
  - https://github.com/rusqlite/rusqlite
  - https://raw.githubusercontent.com/rusqlite/rusqlite/master/libsqlite3-sys/build.rs
  - https://raw.githubusercontent.com/chatmail/core/main/Cargo.toml
relations:
  depends_on: ["[[sqlcipher]]"]
---

Rust SQLite binding; SQLCipher build adds 5.4 to 6.4 MB per binary.

## What it is

rusqlite is the Rust binding to SQLite and the mainstream Rust route to SQLCipher.
The Layer 5 fabric adopts it for the encrypted message store `fabric/store.db`,
through the `bundled-sqlcipher-vendored-openssl` feature.
SQL gives the store dedupe (`UNIQUE`), threads, full-text search, and migrations for free.

## Use when

- A Rust daemon needs an encrypted local database on Cat 2 to Cat 5 nodes and phones.
- Messages and metadata need relational queries, not a hand-built key-value layout.
- A static musl binary is required: the vendored-OpenSSL feature links no shared libraries.

## Do NOT use when

- The binary-size budget cannot absorb +5.4 MB (aarch64) or +6.4 MB (x86_64).
- A C and OpenSSL build step is unacceptable and no dependency sign-off exists.
- The target is a Cat 1 microcontroller; this stack never runs there.

## Details

Reported as of 2026-10-14: rusqlite 0.40.2 (2026-08-08), about 125M downloads, MIT.
Its `libsqlite3-sys` crate bundles SQLCipher 4.17.0; upstream SQLCipher is already at 4.19.0.
The feature that matters:

```toml
rusqlite = { version = "0.40", features = ["bundled-sqlcipher-vendored-openssl"] }
```

`bundled-sqlcipher` alone compiles SQLCipher but links `libcrypto` dynamically.
Only the vendored-OpenSSL variant builds OpenSSL 3.6.3 in, which a static musl target needs.

Measured size deltas (musl-static, workspace release profile, versus a hello-world base):
SQLCipher with vendored OpenSSL +6,427 KB x86_64 and +5,357 KB aarch64;
plain SQLite (`bundled`) +1,907 KB and +1,735 KB.
Against today's `111` binary of 7.0 MB that is about +92% on x86_64.
Cold build takes about 2m15s and needs perl, make, and the cross gcc already installed.
The bundled build sets `SQLITE_TEMP_STORE=3` on Android, keeping temp files in memory.

Layer 5 settings (spec §10.1): raw key, WAL, `synchronous = FULL` on sequence-allocating
transactions, `temp_store = MEMORY`, `auto_vacuum = INCREMENTAL`, `secure_delete = ON`.
Precedent: Delta Chat's Rust core ships this feature set as its default `vendored` build,
but its passphrase is optional, so its database is plaintext unless the user sets one.
Layer 1's dependency policy prefers pure-Rust backends; this C dependency needs a recorded sign-off.

## Evidence

### Alternatives considered

- Plain SQLite with per-record XChaCha20-Poly1305 — +1.9 MB and L2-native primitives, but we would
  invent and audit a record format, and table and index metadata would stay in the clear.
- redb and fjall with per-value AEAD — pure Rust at +0.8 MB and +1.0 MB, but indexes, search,
  and encryption are hand-built; a better fit for Cat 1 flash rings than for the Messages view.
- sled — rejected: last release 1.0.0-alpha.124 in 2024-10.
- A plaintext database on disk encryption — impossible under Termux.
- 2026-10-14 — license (MIT) and feature behavior read from upstream `LICENSE` and `build.rs` in the
  Layer 5 research (report 05); size figures are throwaway probe builds there, not field data.

## Links

- [Upstream repository](https://github.com/rusqlite/rusqlite)
- [libsqlite3-sys build script](https://raw.githubusercontent.com/rusqlite/rusqlite/master/libsqlite3-sys/build.rs)
- Engine underneath: [[sqlcipher]]
