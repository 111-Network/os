---
title: Meshtastic Rust Crate
category: project
status: candidate
summary: Official Meshtastic Rust client crate; GPL-3.0 makes the linked binary GPL-3.0-only.
tags: [lora, mesh, bridge]
platforms: [linux]
upstream: https://github.com/meshtastic/rust
license: GPL-3.0-only
last_verified: 2026-10-19
sources:
  - https://github.com/meshtastic/rust
  - https://crates.io/crates/meshtastic
  - https://github.com/meshtastic/protobufs
relations:
  depends_on: ["[[meshtastic]]"]
  alternative_to: ["[[meshtastic-python]]"]
  relates_to: ["[[meshmud]]"]
---

Official Meshtastic Rust client crate; GPL-3.0 makes the linked binary GPL-3.0-only.

## What it is

The official `meshtastic` crate (0.1.9, tokio, prost types) speaks the Meshtastic client API over serial, TCP, or BLE.
Layer 6 links it into the daemon as the Meshtastic bridge client, attached to the [[meshmud]] downlink on loopback,
so the in-process client replaces the Python presence reader after a parity gate.
The crate declares `GPL-3.0` (crates.io) and ships a GPL-3.0 `LICENSE`.

## Use when

- A Rust program needs the Meshtastic framing, handshake, and keep-alive without writing them.
- The combined binary may be conveyed as GPL-3.0-only (decision D25 F2 accepts that for the 111 daemon).
- The stream is any `AsyncRead + AsyncWrite`, such as a loopback TCP socket to a multiplexer.

## Do NOT use when

- A permissive or "or later" binary is required: linking it makes the shipped binary effectively GPL-3.0-only.
- Binary size on the Raspberry Pi floor is tight and a thin prost client (about 300 lines) would do.
- The send path must be provably narrow without a wrapper: the crate exposes more `ToRadio` kinds than the allowlist.

## Details

Reported as of 2026-10-19: 0.1.9 released 2026-05-25 (crates.io); last commit 2026-10-07; one listed author; pre-1.0.
Measured with the release profile (thin LTO, strip, abort): +372 KiB on x86_64 musl and +332 KiB on aarch64 musl
over a tokio baseline. Normal closure 49 crates: 31 `MIT OR Apache-2.0`, the GPL-3.0 crate itself, and `serialport`
4.10.1 (MPL-2.0).
Its serial and BLE features pull `serialport` and `btleplug`; dropping them is unverified until the build proves it.
Fit: a nonce-0 heartbeat every 5 minutes, a config id the caller chooses, and `connect` over any stream.
`disconnect()` sends no `ToRadio.disconnect`.

Licence effect (D25 F2): the shipped 111 daemon binary is effectively GPL-3.0-only while 111's own files stay
"or later". The protobuf snapshot's `LICENSE` holds the GPL-3.0 text without a version notice, so a legal read decides
whether "or later" survives (GPLv3 section 14). Unverified: whether the crates.io package ships `.proto` files or
generated Rust.
Wrap the stream with a send allowlist (text, one `want_config_id`, heartbeat nonce 0) and never expose admin or
config writes.

## Evidence

### Alternatives considered

- An own thin client on prost (about 300 lines): +152 KiB on x86_64 and +132 KiB on aarch64; the kept fallback.
- [[meshtastic-python]] — a separate process; the presence reader that this crate retires after parity.
- The `meshtastic_protobufs` crate — rejected: a year behind the schema repository.
- 2026-10-19 — version and licence read from crates.io and upstream `LICENSE`; sizes taken from the Layer 6 research
  measurement; nothing run in this pass.

## Links

- [Upstream repository](https://github.com/meshtastic/rust)
- [Crate page](https://crates.io/crates/meshtastic)
- Node: [[meshtastic]]; multiplexer: [[meshmud]]
