---
title: Delta Chat RPC Server
category: project
status: candidate
summary: Chatmail core as JSON-RPC over stdio; MPL-2.0, decrypts everything it receives.
tags: [messaging, bridge, offline]
platforms: [linux, android]
upstream: https://github.com/chatmail/core
license: MPL-2.0
last_verified: 2026-10-19
sources:
  - https://github.com/chatmail/core
  - https://raw.githubusercontent.com/chatmail/core/main/LICENSE
  - https://github.com/chatmail/core/blob/main/deltachat-rpc-server/README.md
relations:
  depends_on: ["[[madmail]]"]
  relates_to: ["[[delta-chat-securejoin]]"]
---

Chatmail core as JSON-RPC over stdio; MPL-2.0, decrypts everything it receives.

## What it is

`deltachat-rpc-server` is the chatmail core (Rust) packaged as a headless program that speaks JSON-RPC 2.0 over stdin
and stdout, with no network port of its own. Layer 6 runs it as an optional component that owns a bridge account, so a
Delta Chat contact becomes a conversation in Messages. The `LICENSE` file states the Mozilla Public License 2.0.

## Use when

- A hub needs a headless Delta Chat peer: a bot account joined to contacts through Secure-Join.
- The same core should later run in Termux: an Android arm64-v8a build exists (a runbook gate, not a v1 claim).

## Do NOT use when

- The hub must not hold plaintext: the core stores every contact's decrypted messages under its account store.
- It must stay off the Raspberry Pi base configuration: about 29 MB RSS per process while enabled.
- A per-chat retention policy is assumed: `delete_device_after` is account-wide.

## Details

Reported as of 2026-10-19: core v2.63.0 (2026-10-09), tag commit `de6f46971002c64b293d6095e66d8a1b7a11fd43`.
Linux x86_64 asset 26.8 MB static-pie (sha256 `57cab329...0d517`), aarch64 21.0 MB static, Android arm64-v8a 21.9 MB.
Spike run on 2026-10-19 (x86_64, one network namespace, no route off the box): two accounts exchanged messages through
[[madmail]] in both directions with read receipts; egress was four `connect()` calls to loopback per process.
Calls the adapter uses: `add_account`, `set_config` with `bot` = 1 (flags the account as a bot) and `mdns_enabled` = 1,
`add_transport_from_qr` with the relay's `dclogin:` URI, `start_io`, `get_chat_securejoin_qr_code`,
`misc_send_text_message`.
Secure-Join progress 1000 means the contact joined, which is also that contact's opt-in act ([[delta-chat-securejoin]]).
Unverified: whether the mobile apps accept an IP-only, self-signed relay on a LAN address (attended hardware gate).

## Evidence

### Alternatives considered

- SimpleX bot API ([[simplex-chat]]) — a second app peer, AGPL-3.0, glibc only.
- A native `deltachat` crate linked into the daemon — possible under MPL-2.0 (secondary-license clause unverified);
  a separate process was chosen to keep the base binary light.
- 2026-10-19 — licence read upstream; stdio JSON-RPC, bot flag, and Secure-Join run in the spike (research report 08).

## Links

- [Upstream repository](https://github.com/chatmail/core)
- [Server README](https://github.com/chatmail/core/blob/main/deltachat-rpc-server/README.md)
- Relay: [[madmail]]; verification: [[delta-chat-securejoin]]
