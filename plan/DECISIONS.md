# DECISIONS — 111network os build log

ADR-lite. Every decision, its rejected alternatives, and why. Newest last; never delete, only supersede.

## D1 — Deliverable shape: proper README per layer, main README stays general

- **Date:** 2026-10-12
- **Decision:** Each layer gets its own proper README/spec. The main `os/README.md` remains (or becomes) user-facing, consumer-focused, and general. The deep architecture content currently living in `os/README.md` (it was placed there only because the repo was empty) will be moved into the per-layer specs as they are created.
- **Rejected:** (a) expand the main README itself into one giant buildable document — would get unwieldy and mixes audiences; (b) implement code immediately per layer before specs — decisions would evaporate between sessions.
- **Why:** per-layer files match the project's own Knowledge Layer conventions (one topic per file, small files, AI-readable) and make sessions disposable.

## D2 — Build harness and model roster

- **Date:** 2026-10-12
- **Decision:** Claude Code is the harness for building this project (not open code — open code is a *candidate component* decision inside the OS itself, e.g. Layer 3). Orchestration via Claude Code workflows (ultracode). Model assignments: k3[1m] main session; k3-256k research fleet; kimi-for-coding (K2.8) cheap bulk/background; fable/opus adversarial review; sonnet drafting; haiku tiny tasks.
- **Rejected:** running all agents on the main-session model — wastes the 1M window on scoped tasks and spends more than needed.
- **Why:** right model per job; the three Kimi tiers (K2.8, k3-1M, k3-256k) cover main/fleet/background roles.

## D3 — Session strategy: hybrid

- **Date:** 2026-10-12
- **Decision:** The research → options → decision loop stays in one continuous session (cross-layer consistency lives here). Builds of decided layers fork into fresh ultracode sessions when substantial, with the layer spec + this file as the handoff brief. Plan state lives on disk in `os/plan/`, never only in chat.
- **Rejected:** (a) everything in one session — context rot loses rejected-option rationale across 10 layers; (b) fully separate sessions per layer — re-reading everything each session drifts cross-layer decisions.
- **Why:** md files make session boundaries cheap; fork builds, not thinking.

## D4 — First layer: Layer 0 (Knowledge)

- **Date:** 2026-10-12
- **Decision:** Start with Layer 0, the Knowledge Layer.
- **Rejected:** Layer 3 first (highest leverage, but ungrounded before the KB exists); Layer 1 first (forces device decisions before research habits exist).
- **Why:** README designates it the foundation every other layer reads from; building it first establishes the format + research workflow every later layer reuses.

## D5 — Build execution protocol + commit discipline

- **Date:** 2026-10-12
- **Decision:** Every layer build follows `EXECUTION.md`: small independent tasks with acceptance criteria + verifiable checks, parallel subagents on disjoint file sets, self-review (run checks, read diffs) with bounce-back fixes, one commit per verified task, done = criteria met + full suite green + built/deferred summary. Commits carry **no** Claude co-author or attribution lines, ever.
- **Why:** save work at every step; review loop catches subagent drift; user wants no attribution noise in git history.

## D6 — Layer 0 (Knowledge) design decisions

- **Date:** 2026-10-12
- **Decision:**
  - **(A) Index contract:** hierarchical llms.txt v2 — root `os/knowledge/llms.txt` + one per category folder, CI-generated from entry frontmatter (never hand-written), proven-only entries in main sections, candidate/testing in the v2 `Optional` section, NO llms-full.txt. Plus an AGENTS.md pair (repo root + `os/knowledge/AGENTS.md`) teaching the retrieval ritual. No CLAUDE.md inside `knowledge/`.
  - **(D) Rubric:** three-gate candidate→testing→proven. External signals (activity, Scorecard, adopters, distro packaging) can at most justify `testing`; `proven` requires first-party field evidence logged in the entry (≥3 months on ≥2 device profiles, zero-WAN clean install, power-loss recovery). Quarterly CI re-checks; demotion + hold ring. Consequence accepted: seed entries enter as `candidate`; `proven` is earned in the field.
  - **(E) Publish:** DEFERRED — will most probably be part of the Next.js app in `website/`, not Quartz/Starlight. Binding constraint recorded instead: the KB format stays publisher-agnostic (vault-wide-unique slugs, wikilinks resolvable by name, CommonMark body, YAML frontmatter, regenerable indexes) so any pipeline can render it later.
  - **(F) v1 scope:** skeleton — format machinery (schema, validator/generator, AGENTS.md, README spec) + ~8 flagship entries (meshtastic, reticulum, nostr, llama-cpp, qwen3-4b, opencode, automerge, heltec-wifi-lora-32-v3). Full ~35-entry seed deferred.
- **Rejected:** AGENTS.md-only/flat index; external-signals-suffice proven; locking Quartz/Starlight now; full seed; building the site pipeline this cycle.
- **Why:** build correctly and compatible; the publish tool must not constrain the format.

## D7 — Layer 0 spec locked; build amendments

- **Date:** 2026-10-12
- **Decision:** Layer 0 spec v2 (`os/knowledge/README.md`) locked after the adversarial panel (4 judges, 62 findings → 42 accepted fixes; triage in `layer-0-review.md`). Amendments adopted with the lock: (a) seed set 8 → 9, adding `recipes/install-meshtastic-on-heltec-wifi-lora-32-v3.md` so the recipe machinery ships exercised; (b) validator uses pinned PyYAML instead of stdlib-only parsing (hand-rolled YAML would get the 1.1 traps wrong; PyYAML is the boring, system-packaged choice), with a strict linted YAML authoring subset on top; (c) seeds are real researched content whose purpose is exercising the machinery — all enter as `candidate`.
- **Rejected:** hand-rolled stdlib YAML parser; entry_type frontmatter field (derivable); reopening skeleton scope.
- **Why:** the rubric's "proven = field-proven" promise is only real if the linter can check its evidence structurally; the skeleton's job is proving the machinery end-to-end on real data.

## D8 — The vision doc names no tools

- **Date:** 2026-10-12
- **Decision:** `os/README.md` states goals, intentions, and ideas only — no specific tool or project names. Removed: Nostr (5 mentions), llms.txt, GitHub Actions, Wikipedia. Generic technology *classes* (LoRa, Wi-Fi, BLE, Markdown, YAML) stay. Tool/project adoption lives in Knowledge Layer entries and this decisions log, earned through the research→decision cycle.
- **Why:** the founding document stays stable while implementation choices evolve; nothing gets prematurely fixed by being named in it.

## D9 — Layer 1 (Bootstrap) design decisions

- **Date:** 2026-10-12
- **Decision:** Scope locked with user: targets = Linux laptops/servers/RPi (x86_64 + aarch64, systemd-only) + Android phones provisioned over USB-C from a Linux computer running the same command; end state = running minimal node; offline prefetch = minimal with post-install offer of a standard tier (small model). From the options brief (`layer-1-options.md`; 7-scout sweep + completeness critic, 24 gaps folded):
  - **A1** two-stage installer: version-pinned POSIX shim on own domain (function-wrapped, artifact sha256 pinned inside, downloads+verifies as user then re-execs via sudo) → ONE musl-static Rust binary from GitHub Releases + own-domain mirror; `receipt.json` + on-disk `111-uninstall.sh`; zero telemetry.
  - **B** implementation: Rust static binary (spec detail, not polled).
  - **C2** minimal node = `111d` kernel (secp256k1 identity, tokio supervision tree, embedded minimal dashboard + `/health`, minisign self-updater with A/B + health-gated auto-rollback, KB fetcher) + KB tarball snapshot + conditional `rnsd` child when python3 exists (meshtasticd/radios detection-only). User amendment: v1 dashboard stays truly minimal — "the very minimums live is enough"; real dashboard work is Layer 4's.
  - **D1** identity: provisional secp256k1 (k256, NCC-audited), BIP-39 mnemonic born silently at install (NIP-06 path), shown only via post-install `111 identity backup`. Provisional: Layer 2 owns final crypto on merit (its scope is explicitly NOT Nostr-locked; see also D8 — README no longer names Nostr); a one-time re-key on Layer 2 landing is accepted.
  - **E1** phone path: same command drives adb — pinned GitHub debuggable Termux APKs (single signature source) + `run-as com.termux` headless channel + Termux:Boot/runit supervision + adb hardening; aarch64 phones only; debuggable-build trust model accepted with post-install revocation of USB-debugging authorizations; postmarketOS stays an experimental opt-in tier.
  - Detail-level (spec-reviewed, not polled — full text in the brief): FHS layout `/opt/111` + `/var/lib/111` + `/etc/111`; rustup-v2 TOML channel manifest + minisign with pre-designed key-rotation chain; systemd system unit (user `node111`, ambient caps, `dialout`, watchdog, journald cap); category rule set with proposed default thresholds; pre-build verification matrix V1–V8.
- **Rejected:** package-repo installer; TLS-only shim; bundle C1 (fakes "mesh joined") and C3 (dead weight, non-portable); raw-key-only identity; blocking Layer 1 on Layer 2's crypto; release-APK `input text` injection (kept as fallback); own provisioner APK (revisit v1.1); postmarketOS as default; armv7 phones; non-systemd distros (clean refusal); proot-distro; GPG signing; installer telemetry.
- **Why:** strongest verified installer anatomy (uv + Determinate + k3s patterns); smallest node that honestly satisfies "running minimal node"; identity seam cheap to swap; run-as is the only verified fully-headless phone channel.

## D10 — Layer 3 (Local Intelligence) design decisions

- **Date:** 2026-10-12
- **Decision:** Scope locked with user: **no new research** — every adopted component comes from existing KB entries (`llama-cpp`, `qwen3-4b`, `opencode`), whose own use-when/do-not-use-when sections dictated the split. Layer boundary: **Layer 3 = capability** (models, serving, harness); **Layer 9 = behavior** (monitor cadence, diagnosis playbooks, sandbox, PR gate) — Layer 9 consumes Layer 3 through one task API and stays harness-agnostic.
  - **H1** hybrid harness: thin always-on Rust agent loop (`111-agent`, supervised child of `111d`, same pattern as the `rnsd` child) for chat, KB grounding, monitoring, and the task API; opencode adopted on hub tiers (4A/4B, opt-in on 2B) for patch-class tasks via headless JSONL — invoked **by** `111-agent`, never directly by Layer 9.
  - **S1** serving: pinned `llama-server` binary as supervised child of `111d`; OpenAI-compatible endpoint `127.0.0.1:11181`; GGUF blobs sha256-pinned in the signed channel manifest, fetched via Layer 1's standard-tier prefetch; no runtime model downloads.
  - **M1** model default: qwen3-4b Q4_K_M is the standard tier wherever a local model runs (2B, 4A, 4B); the 4B hub ladder (8B → 32B / 30B-A3B MoE) is one config table, not a new adoption. Offload-only nodes (2A, 3) run `111-agent` as a pure client of a nearby hub.
  - Ports: dashboard 11180 (Layer 1), llama-server 11181, `111-agent` 11182 — localhost; LAN binding follows the bootstrap dashboard's explicit opt-in rule.
- **Rejected:** opencode everywhere (a 24/7 Node/TypeScript runtime on every intelligence-capable node; the KB entry itself warns it "belongs on hubs, not relays"); custom-only thin loop (reinvents an agent harness — against adopt-don't-reinvent — and caps patch-generation quality); Ollama (a second heavy always-on daemon whose supervision/update duties overlap `111d`'s; would require new adoption research); llama.cpp embedded as a library in `111d` (couples the kernel's release cycle to the inference engine and adds custom serving plumbing); on-node fine-tuning (weights change only via signed channel-manifest swaps); WAN/cloud model calls (breaks offline-first).
- **Why:** the smallest layer that honestly satisfies the README's "continuous agent" without a resident Node daemon; the harness-agnostic task seam keeps Layer 9 unblocked; every component traces to a KB entry, so the adopt-vs-build rule is exercised, not bypassed.

## Pending

- Move deep architecture content out of `os/README.md` into per-layer specs (blocked on layer homes existing).
