---
title: kiwix-tools
category: project
status: candidate
summary: GPL offline-encyclopedia server for ZIM files; RSS on a Pi unmeasured.
tags: [storage, offline, panel]
platforms: [linux, android]
upstream: https://github.com/kiwix/kiwix-tools
license: GPL-3.0-or-later
last_verified: 2026-11-02
sources:
  - https://github.com/kiwix/kiwix-tools
  - https://raw.githubusercontent.com/kiwix/kiwix-tools/main/docs/kiwix-serve.rst
  - https://raw.githubusercontent.com/kiwix/kiwix-tools/main/COPYING
  - https://github.com/openzim/libzim
---

GPL offline-encyclopedia server for ZIM files; RSS on a Pi unmeasured.

## What it is

kiwix-tools holds `kiwix-serve` (an HTTP server for ZIM archives with full-text search and an OPDS catalogue),
`kiwix-manage` (builds the library file) and `kiwix-search`.
Layer 8 adopts it as an on-demand component: a separate supervised program behind a hosted `child` panel,
started on the first panel open and stopped after an idle stretch. It is never linked into `111d`.

## Use when

- A hub or a roomy node should serve Wikipedia-class ZIM files to browsers on the LAN through a panel.
- The agent needs full-text search over a held ZIM (`encyclopedia.search`).

## Do NOT use when

- The device is a phone with little storage or memory: only small ZIMs, and the Termux closure is unmeasured.
- The Raspberry Pi floor must stay weightless while it runs: the process wakes once a second until stopped.
- The server would be bound beyond loopback: it has no authentication; reach it through the panel proxy only.

## Details

Versions as the Layer 8 spec states them: kiwix-tools 3.8.2, libkiwix 14.2.1, libzim 9.8.2.
Licences: kiwix-tools GPL-3.0-or-later (COPYING is GPLv3, README says "or later"), libkiwix GPL-3.0-or-later,
libzim GPL-2.0-or-later. A separate program, so no linking question; a hub that mirrors the binary owes the source.
Flags the spec pins: `--library`, `--threads`, `--nolibrarybutton`, `--blockexternal`, `--address 127.0.0.1`, `--port`;
never `--monitorLibrary` (adds a per-second stat). `SIGHUP` reloads the library.
Termux carries kiwix-tools 3.8.2 as a package with libzim, libkiwix, ICU, Xapian and libmicrohttpd.
Unverified from this box: an official 3.8.2 static aarch64 or musl tarball (only a 3.7.0 aarch64 name was seen),
the idle and loaded memory figures (forum anecdotes only), and the thread count.

## Evidence

### Alternatives considered

- kiwix-js and kiwix-js-pwa — browser side, GPL-3.0; needs a secure context, which `http://111.local` lacks.
- Serving ZIMs with our own reader — would reinvent libzim, Xapian search and OPDS.
- Xowa — Java, own database import, no gain over ZIM.
- 2026-11-02 — desk research (Layer 8 reports 01, 03, 05): licences and versions read upstream by the scouts; the entry was written later and nothing was re-read, fetched, built or run on this box.

## Links

- [kiwix-tools repository](https://github.com/kiwix/kiwix-tools)
- [kiwix-serve documentation](https://raw.githubusercontent.com/kiwix/kiwix-tools/main/docs/kiwix-serve.rst)
- [libzim repository](https://github.com/openzim/libzim)
