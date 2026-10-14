# Layer 0 (Knowledge) — Options Brief

Status: ✅ decisions taken 2026-10-12 (recorded in `DECISIONS.md` D6): **A1** llms.txt v2 + AGENTS.md · **D** three-gate field-proven rubric · **E** publish deferred (likely Next.js in `website/`; format stays publisher-agnostic) · **F2** skeleton, ~8 entries · Source: 7-agent research sweep, 2026-10-12 (k3-256k fleet, 122 tool calls)
Full raw findings: workflow `wf_cd0bfb29-a38` journal. This file condenses them into decisions.

## Already decided by `os/README.md` — not relitigated

Markdown only · one topic per file · YAML frontmatter with tags/category/status (candidate→testing→proven) · wikilinks · llms.txt-style index · many small files · graphable later · source of truth at `os/knowledge/` · published to website later via CI.

## What research converged on (all 7 agents independently)

The 2026 ecosystem has consolidated: **llms.txt v2** (Aug 2026 revision, hierarchical subpath semantics) for indexes, **AGENTS.md** (60k+ repos, Linux Foundation, read natively by Claude Code / OpenCode / Codex / Copilot / Cursor / Gemini CLI) as the agent entry contract, **Obsidian-compatible wikilinks + typed `relations:` in frontmatter** for graphability, and **Quartz 5** as the only publisher that renders that exact format natively. MCP doc servers / Context7-style services are rejected on one ground: they need a server, and Layer 0 must work on an air-gapped LoRa relay.

---

## Decision A — Index & navigation contract

**Recommend: llms.txt v2 hierarchical indexes + AGENTS.md pair, all CI-generated from frontmatter.**

- Root `os/knowledge/llms.txt` + one per category folder (v2's "most specific path wins" blesses this). Fixed line grammar: `- [Name](path.md): ≤12-word summary | status | keywords`.
- Main H2 sections list **proven only**; candidate/testing live in the v2 `Optional` section.
- **No llms-full.txt** (v2 dropped it; Next.js's is ~676k tokens — poison for 1B–8B local models).
- Token budgets driven by small-model evidence (Lost-in-the-Middle): root index ≤200 lines (~1.5k tok), category index ≤2k tok, entry 300–1500 tok.
- `AGENTS.md` at repo root + `os/knowledge/AGENTS.md` teach the retrieval ritual: *read llms.txt → pick section → grep keywords → open one file*. No CLAUDE.md inside knowledge/ (it would suppress Claude's AGENTS.md reading).

| Option | Verdict |
|---|---|
| **A1: llms.txt v2 + AGENTS.md (recommended)** | Purpose-built, tooling tailwind, works offline with grep |
| A2: AGENTS.md only, hand-written index | No per-entry metadata, drifts from content |
| A3: MCP doc server | Needs a server — impossible on relays/old phones |
| A4: `.context/` or per-harness rule dirs | Nothing reads `.context/`; legacy dirs being absorbed into AGENTS.md |

## Decision B — Frontmatter schema (detail — reviewed in spec, not polled)

Merged from kb-conventions + agent-context research. 12 fields, each earns its lint cost:

```yaml
---
title: Meshtastic                  # human name
category: project                  # project|device|recipe|protocol|model|concept — drives folder
status: testing                    # candidate|testing|proven (+hold on demotion)
summary: LoRa mesh firmware, huge community, Android app.  # one line; feeds llms.txt; trigger-phrased (SKILL.md-style discovery surface)
tags: [mesh, lora]                 # filtering only — NEVER relations
aliases: [Meshtastic firmware]     # Obsidian-reserved, list
platforms: [esp32, nrf52, android] # projects/devices
repo: https://github.com/meshtastic/firmware
license: GPL-3.0                   # adopt-vs-build needs it
last_verified: 2026-10-01          # ISO; drives quarterly staleness lint
sources: [<urls>]                  # provenance, verbatim
relations:                         # typed edges — the machine graph
  implements: ["[[meshtastic-protocol]]"]
  runs_on: ["[[esp32-s3]]"]
  alternative_to: ["[[reticulum]]"]
---
```

Filenames: vault-wide-unique kebab-case; recipes deterministic (`install-<project>-on-<device>.md`); body contract = one-sentence summary first line, then fixed sections (What it is / Use when / Do NOT use when / Details / Links). Folders: `projects/ devices/ recipes/ protocols/ models/ concepts/ _meta/`.

## Decision C — Typed relations vocabulary (detail — reviewed in spec, not polled)

`relations:` as predicate → wikilink-list map (clickable in Obsidian graph view; OKF-style triples rejected — invisible in Obsidian). Starter vocabulary with declared inverses: `implements`, `runs_on`/`hosted_by`, `depends_on`/`required_by`, `alternative_to`, `supersedes`/`superseded_by`, `used_in`; `relates_to` last resort. CI lints like code: dangling typed edge = error, unknown predicate/self-edge/supersede-cycle rejected, inverse symmetry checked, tags are not edges.

## Decision D — Maturity rubric (the adopt-vs-build heart)

**Recommend: three-gate rubric. External signals can only reach `testing`; `proven` requires first-party field evidence recorded in the entry itself** — because no external signal can verify "works without internet on our hardware." All gates binary + CI-checkable, date-stamped, quarterly re-check with automatic demotion proposals.

- **candidate**: OSI license (SSPL/BUSL flagged, never core), public source, sign of life ≤24mo, documented install path, alternatives considered recorded.
- **testing**: commit ≤6mo + ≥10 commits/12mo, tagged release ≤12mo, bus factor ≥2 (CHAOSS contributor-absence), issue response ≤30d, SECURITY.md, no unpatched critical CVE >90d, **offline: core works with networking disabled (telemetry-by-default = auto-fail)**, builds from locked deps network-isolated, footprint measured per device class, OpenSSF Scorecard ≥4 (or documented exception), **test log in entry** (who/device/date/procedure/result).
- **proven**: ≥2 independent external adopters + ≥1 internal deployment, **we ran it ≥3 months on ≥2 device profiles** incl. zero-WAN clean install + power-loss recovery, maintainers ≥2 orgs (or permissive license + succession note), Scorecard ≥7/badge, **third-party audit mandatory for identity/keys/crypto**, in-place upgrade without data loss, offline-readable docs, distro-packaged (nixpkgs/Debian/Alpine — near-gate, exception allowed).
- **hold/demote**: no commit 12mo, critical CVE >90d, license flip, telemetry introduced, failed field report → demote + replacement plan. (ThoughtWorks Hold ring: "the most valuable ring.")
- **Adopt-vs-build rule**: proven entry covers ≥80% → adopt. Only testing entries exist → prototype on risk-tolerant device first. Build only when nothing passes testing gates. Spend innovation tokens on the OS + agent, never on plumbing.

⚠️ **The commitment**: this makes "proven" expensive *for us* — real field testing on real hardware. Lighter alternative: external signals suffice for proven (faster, but "proven" becomes trust-me).

## Decision E — Publish toolchain (decide now, build later)

**Recommend: Quartz 5.** Only candidate rendering the decided format natively (wikilinks, frontmatter, backlinks, graph — zero conversion). Vault stays pure at `os/knowledge/`; Quartz lives in sibling `site/` and builds with `quartz build -d ../os/knowledge`. Emits `contentIndex.json` (machine-readable, agent-consumable) + sitemap; llms.txt via ~50-line post-build script. Official GH Actions workflow (Node 24); monochrome terminal aesthetic fully achievable per `website/docs/DESIGN.md`; no tracking; static HTML servable by any mesh node over local HTTP.

| Option | Verdict |
|---|---|
| **Quartz 5 (recommended)** | Zero conversion loss; one gap (no llms.txt emitter) = ~50 lines |
| Starlight + starlight-llms-txt | Best llms.txt plugin, but wikilinks = one-way conversion, vault becomes bolt-on |
| mdBook | Fastest builds; no wikilinks/frontmatter semantics; wrong shape (book ≠ graph) |
| Docusaurus | Heavy React, ~200KB JS, no wikilinks — eliminated |
| pandoc DIY | Max control, but hand-build nav/backlinks/search/index forever |

## Decision F — Seed scope for the v1 build

The landscape sweeps produced a concrete seed inventory (~35 entries): mesh stacks (Meshtastic, MeshCore, Reticulum+RNode, bitchat, Briar-as-warning, goTenna-as-antipattern, qaul-unverified, Serval/Meshenger-historical, Yggdrasil), Nostr relays (strfry, nostr-rs-relay, khatru) + NIP-77, runtimes (llama.cpp, Ollama, ExecuTorch, MLC, whisper.cpp, Moonshine), models (Qwen3 4B/8B/30B-A3B, Phi-4-mini, Gemma 4-license-flagged, Devstral 24B, gpt-oss-20b), harnesses (OpenCode, Goose, Aider-pattern, pi-mono-class loops), storage (SQLite+Litestream, Automerge, Yjs), distribution (Hypercore, IPFS-caveated, Iroh-candidate, Kiwix), hardware profiles (Heltec V3/V4, LilyGO T-Beam/T-Deck/T-Echo, RAK WisBlock), candidates to track (Willow, p2panda, Loro).

| Option | Verdict |
|---|---|
| **F1: Full seed (recommended)** | Schema + linters + index generator + AGENTS.md + all ~35 entries via EXECUTION.md task fan-out. Layer 0's point is grounding later layers; a skeleton is theater. |
| F2: Skeleton first | Format + ~8 flagship entries; cheaper now, but Layer 1-3 research re-does the sweep |
| F3: Full seed + publish pipeline now | Also builds site/ + Quartz CI this cycle; bigger scope, publish can wait per README |

## Deferred (recorded, not forgotten)

`os/knowledge/skills/` (real SKILL.md packaging of agent procedures like *evaluate-candidate*, *promote-status*) · hub-side MCP wrapper over the same markdown · Foam-style link-reference definitions for GitHub-web browsing · per-category llms-full bundles if cloud agents ask · monthly headless LLM audit of KB freshness · Yjs only if dashboard needs live multi-user text · Willow/p2panda/Iroh/Loro quarterly re-review · markdown content negotiation (Cloudflare) at publish time.
