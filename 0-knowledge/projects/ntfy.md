---
title: ntfy
category: project
status: candidate
summary: HTTP pub-sub notification server; the first Layer 7 hub-to-hub carrier candidate.
tags: [messaging, networking, dtn]
platforms: [linux]
upstream: https://github.com/binwiederhier/ntfy
license: Apache-2.0 OR GPL-2.0-only
last_verified: 2026-11-02
sources:
  - https://raw.githubusercontent.com/binwiederhier/ntfy/main/LICENSE
  - https://raw.githubusercontent.com/binwiederhier/ntfy/main/LICENSE.GPLv2
  - https://raw.githubusercontent.com/binwiederhier/ntfy/main/README.md
  - https://raw.githubusercontent.com/binwiederhier/ntfy/main/docs/releases.md
  - https://github.com/binwiederhier/ntfy/tags
relations:
  alternative_to: ["[[deltachat-rpc-server]]"]
  relates_to: ["[[store-and-forward-states]]"]
---

HTTP pub-sub notification server; the first Layer 7 hub-to-hub carrier candidate.

## What it is

ntfy is a Go server that takes an HTTP `POST` to a topic and hands the message to anyone
who reads that topic, by a poll or by a held stream.
Layer 7 adopts it as a mailbox between two paired gateway hubs (`7-relay/README.md` §6.2, D30 F3):
each hub posts sealed hub-to-hub frames to its outbound topic and drains the other's.
The server runs as the owner's own separate program; `111d` reaches it with the ureq and
rustls client it already links, so no process, crate or listener of ours is added.
ntfy.sh, the public instance, is for a manual check only (D30 Q15).

## Use when

- Two hubs with internet access must exchange custody frames and no direct link between them exists.
- The owner runs an ntfy server, or accepts one, with a cache file and an auth file.

## Do NOT use when

- Durable storage is assumed: the default cache is 12 hours, in memory, and is lost on a restart.
- A frame over 3,068 B must cross (one message body is at most 4,096 B of base64; §6.2).
- The node is not a hub, or the owner has not turned the carrier on with its consent (§6.3, §12).

## Details

Licence, read from upstream on 2026-11-02: `LICENSE` is the Apache License 2.0 text
("Copyright 2021 Philipp C. Heckel"), `LICENSE.GPLv2` is the GNU GPL version 2 text (June 1991),
and the README says the project "is dual licensed under the Apache License 2.0 and the GPLv2 License".
No "or later" wording was seen, so the field records `GPL-2.0-only` for that option.
Either way the server is a connected service, never linked or shipped (`LICENSING.md` §1, §3).

Releases, read on 2026-11-02 from the tags page and `docs/releases.md`: the latest is v2.36.0 (2026-10-31).
Tags came at least monthly from v2.22.0 (2026-04-21) to v2.28.0 (2026-08-27), none in September,
then ten in October (v2.29.0 on 2026-10-07 to v2.36.0); the research reports said "monthly".

How Layer 7 uses it (§6.2): one topic per direction, `"111"` plus base32 of a keyed BLAKE2b-160 (35 characters),
keyed by a pairing secret, never a NodeId; a batch format of sealed frames in custody order;
publish `POST <server>/<topic>`, drain `GET <server>/<topic>/json?poll=1&since=<id>`;
413, 401 and 403 are permanent, 429 and 5xx are transient.
The recommended self-host config ships as `7-relay/carriers/ntfy/server.yml`
(`cache-file`, `cache-duration: "168h"`, `auth-file` with `auth-default-access: "deny-all"`, attachments off).

Gates still unread from this box (D30 Q16 accepts them for a prototype behind the switch; §6.2, §6.4):
- bus factor and issue response time;
- the topic alphabet and maximum length (the 35-character topic assumes both);
- whether `since=all` re-reads the cache after a `since=<id>` miss;
- the access-token header name for an `auth-file` server;
- ntfy.sh's anonymous limits (reported 250 messages a day) and its retention;
- whether a server forwards messages to a third-party push service.

## Evidence

### Alternatives considered

- Chatmail through the Layer 6 Delta Chat component ([[deltachat-rpc-server]]): kept as the second carrier,
  built only after this entry reaches `testing` (§6.5, D30 Q4).
- Nostr relays, LXMF propagation and iroh: v1.1 (§6.7).
- A direct hub-to-hub link over TLS 1.3 with raw public keys: used where the hubs can reach each other (§4).
- 2026-11-02 — desk research: the two licence files, the README and the release list read upstream.
- 2026-11-02 — the prototype's stub smoke log (row P1): `7-relay/verify/logs/ntfy-smoke-2026.11.02-stub.md`,
  written by `7-relay/verify/carrier.sh --smoke` against `7-relay/verify/stub-ntfy.py` (the stdlib stand-in on
  loopback, not ntfy): the 5A flush order over the fake hand-off, the 4,096 / 4,097 B body boundary, a 429, `since`
  after a stub restart, the 5B held stream with its keepalive, RSS during a flush and bytes per frame; the eleven
  owner-server items (token header, `since` on a real server, retention, limits) are marked "not run from this box".
- Only the owner-run G10 smoke log (`7-relay/verify/logs/ntfy-smoke-<calver>.md`) moves this entry to `testing`.

## Links

- [Upstream repository](https://github.com/binwiederhier/ntfy)
- [Release notes](https://raw.githubusercontent.com/binwiederhier/ntfy/main/docs/releases.md)
- Spec: `7-relay/README.md` §6.1–§6.4; decision: `plan/DECISIONS.md` D30 (F3, Q15, Q16)
- Related: [[deltachat-rpc-server]], [[store-and-forward-states]]
