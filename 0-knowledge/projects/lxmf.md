---
title: LXMF
category: project
status: candidate
summary: Reticulum message format and propagation daemon; custom license, helper-process only.
tags: [messaging, mesh, dtn]
platforms: [linux, android, macos, windows]
upstream: https://github.com/markqvist/LXMF
license: LicenseRef-Reticulum
last_verified: 2026-10-19
sources:
  - https://github.com/markqvist/LXMF
  - https://raw.githubusercontent.com/markqvist/LXMF/master/LXMF/LXMessage.py
  - https://raw.githubusercontent.com/markqvist/LXMF/master/LXMF/Utilities/lxmd.py
  - https://github.com/markqvist/Reticulum/blob/master/LICENSE
relations:
  depends_on: ["[[reticulum]]"]
  required_by: ["[[meshchatx]]"]
---

Reticulum message format and propagation daemon; custom license, helper-process only.

## What it is

LXMF is the message format and delivery layer of the Reticulum ecosystem, with `lxmd` as its
store-and-forward propagation daemon.
It is a bridge target: format facts are recorded here, and the code never enters our tree or the 111d binary.
Layer 6 carries it through `lxmf-sidecar`, a helper of ours that imports the user-installed packages in a separate
process and venv. Credit: Reticulum and LXMF are the work of the Reticulum project (reticulum.network); 111 uses
them as released and is not affiliated.
License is the custom Reticulum License (recorded as `LicenseRef-Reticulum`), non-OSI.

## Use when

- A 111 envelope must travel as an attachment to a Reticulum peer through an adopted program's API.
- Format facts are needed to size a bridge frame or to reason about what LXMF can promise.

## Do NOT use when

- It would become the fabric: it brings its own addressing, a wall-clock timestamp, and a 112-byte overhead.
- Any code would be imported into 111d (`import RNS` or `import LXMF`); only the separate helper process may do that.
- The Reticulum License is unacceptable for the use: it forbids systems able to purposefully harm humans
  and any use in AI or machine-learning training sets.

## Details

Reported as of 2026-10-19: LXMF 1.2.1 (PyPI, 2026-10-13, verified); the GitHub mirror is at 1.2.0 (2026-09-30).
1.2.1 is airtime-aware (slow-path cache, queued messages held for slow destinations, receipt timeouts scaled to the
round trip), so a sender's "sent" has to follow the library's state callbacks, not its own timers.
Format: overhead 112 bytes (the README says 111), made of 16 + 16 + 64-byte signature plus msgpack;
timestamp is a sender float64 in seconds; the message id is SHA-256 of destination, source, and payload and
is not transmitted. Messages are Ed25519 signed; link delivery is documented as forward-secret.
Reticulum MTU is 500, MDU 464, single-packet encrypted MDU 383; so opportunistic content is at most
287 bytes (383 - 112 + 16) and link-packet content 319 bytes.
RNode firmware splits any packet over 254 bytes into two LoRa frames.

`lxmd`: the `on_inbound` option of its `[lxmf]` section runs a program with the saved-message path (inbound only);
`enable_node` in `[propagation]` turns on a propagation node; `message_storage_limit` defaults to 500 MB
and prioritizes keeping new and small messages; per-message cap 256 KB, per-sync cap 10 MB;
default stamp cost 16 and peering cost 18; `control_allowed` and static peers exist.
A propagated send ends in `SENT`; `DELIVERED` is set only for opportunistic and direct sends.

Boundary: the LXMF address is a transport address only; our NodeId stays inside the sealed envelope.
The LXMF README warns that independent "fakes" violate its license and that downstream users are liable,
which reinforces the process boundary. Reach it through the helper or `lxmd`, never in-process; [[meshchatx]] is no
longer the carrier.

## Evidence

### Alternatives considered

- Our own envelope on `mesh111` — the native fabric; LXMF is only a carrier.
- Reimplementations — Leviculum (AGPL-3.0-or-later, Rust), Beechat Reticulum-rs (MIT), go-reticulum (MIT,
  README says it was built with an LLM reading the reference); none is upstream, and upstream disputes them.
- Sideband — CC BY-NC-SA 4.0, rejected. Columba — MPL-2.0 Android client with no external API seen.
- 2026-10-14 — licensing and format facts read from upstream files in the Layer 5 research; no code run.
- 2026-10-19 — PyPI confirms LXMF 1.2.1 under the Reticulum License; Layer 6 carriage decision recorded.

## Links

- [Upstream repository](https://github.com/markqvist/LXMF)
- [Reticulum License](https://github.com/markqvist/Reticulum/blob/master/LICENSE)
- Stack: [[reticulum]]; web client: [[meshchatx]]; state meanings: [[store-and-forward-states]]
