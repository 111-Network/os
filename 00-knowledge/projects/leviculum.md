---
title: Leviculum
category: project
status: candidate
summary: Independent Rust reimplementation of Reticulum; AGPL-3.0-or-later, not upstream, reference only.
tags: [mesh, networking, offline]
platforms: [linux]
upstream: https://codeberg.org/Lew_Palm/Leviculum
license: AGPL-3.0-or-later
last_verified: 2026-10-19
sources:
  - https://codeberg.org/Lew_Palm/Leviculum
relations:
  alternative_to: ["[[reticulum]]"]
---

Independent Rust reimplementation of Reticulum; AGPL-3.0-or-later, not upstream, reference only.

## What it is

Leviculum is a Rust implementation of the Reticulum stack whose daemon `lnsd` is wire-compatible with `rnsd`,
with musl-static builds for amd64 and arm64. It is not the upstream project, so it does not satisfy the rule that
Reticulum is run as released from upstream. Layer 6 records it as a reference and does not adopt it.
Licence as of 2026-10-19 (Layer 6 research, from `Cargo.toml` line 89 and the head of `LICENSE`): AGPL-3.0-or-later.
Codeberg is not reachable from the build box, so this entry did not re-read it.

## Use when

- Studying a Rust wire-compatible Reticulum design, or sizing what a lighter rnsd could weigh.
- The owner later decides that a reimplementation counts as "Reticulum as released".

## Do NOT use when

- Upstream Reticulum is required: this breaks the latest-upstream rule (D17 #12).
- Production readiness matters: it describes itself as "not yet production-ready" (0.10.0-dev, nightly 2026-10-18).
- Upstream's position matters: the Reticulum documentation calls independent copies a licence problem.

## Details

Reported as of 2026-10-19 (research reports 02 and 05): 39 stars; pre-1.0. AGPL-3.0 and GPL-3.0 are mutually compatible,
but running it would add a network-use source duty (AGPL section 13). It would stay a separate process, never linked.
Other reimplementations seen: Reticulum-rs (MIT) and go-reticulum (MIT); none is upstream.

## Evidence

### Alternatives considered

- Python `rnsd` from PyPI, unmodified — the chosen route ([[reticulum]]).
- Own Rust Reticulum — rejected; clean-room design reuse only, no code.
- 2026-10-19 — licence recorded from the Layer 6 research pass; no code run; not re-read upstream here.

## Links

- [Upstream repository](https://codeberg.org/Lew_Palm/Leviculum)
- Reference: [[reticulum]]
