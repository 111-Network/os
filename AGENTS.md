# AGENTS.md

This repo (`os/`) is the operating layer of the 111network mesh OS: an offline-first, self-healing layer that turns any device into a mesh node — adopt proven projects, write custom glue only where nothing fits.

## Read first

- Layer folders are `N-<slug>` with `N` = the layer number (`0-knowledge`, `1-bootstrap`, `2-identity`, `3-intelligence`, `4-interface`, `5-messaging`, `6-transport`); `plan/layers/` uses the same names.
- Knowledge-base navigation contract: `0-knowledge/AGENTS.md` — read it before querying `0-knowledge/`.
- Layer 0 (Knowledge Layer) spec: `0-knowledge/README.md` — locked; defines entry format, relations, status gates, indexes.
- Layer 1 (Bootstrap) spec: `1-bootstrap/README.md` — locked v2.2; the `111` binary, installer, daemon, updates.
- Layer 2 (Identity) spec: `2-identity/README.md` — locked v2; keyset, announces, backup/restore, agentic + transport hooks. Core crate: `2-identity/crates/id111`.
- Build process and task plan: `plan/INDEX.md` (layer status) · `plan/DECISIONS.md` (every decision + rejected alternatives) · `plan/EXECUTION.md` (build protocol). Per-layer build boards: `<layer>/TASKS.md`.
- Layer 3 (Local Intelligence) spec + as-built appendix: `3-intelligence/README.md`; build log/contracts: `3-intelligence/TASKS.md`.
- Layer 4 (Interface & Dashboard) spec: `4-interface/README.md` — locked v2; HTTP contract `4-interface/api/openapi.yaml` (conventions `4-interface/api/README.md`); build board `4-interface/TASKS.md`. The UI is `4-interface/ui`; the Rust side lives in `1-bootstrap/crates/node111/src/daemon/dashboard/`.
- Layer 5 (Messaging & Data) spec: `5-messaging/README.md` — locked v1.2 (D20); build board `5-messaging/TASKS.md`. Core crate: `5-messaging/crates/mesh111`. The daemon side (fabric, hosted panels, Meshtastic) lives in `1-bootstrap/crates/node111/src/daemon/{fabric,panels,meshtastic}/`; the presence sidecar is `5-messaging/sidecars/meshtastic-presence/`.
- Licensing: our own license is `LICENSE.md` (GPL-3.0-or-later; contributions of any kind are appreciated). Third-party licensing and dependency boundaries (Reticulum is a separate service, never incorporated): `LICENSING.md` — update it when a dependency, vendored file, or pinned external program changes. Third-party notices and license texts: `THIRD-PARTY-NOTICES.md` — regenerate it when a lockfile changes.

## Working rules (merged from the former `CLAUDE.md`, 2026-10-19; `CLAUDE.md` now only includes this file)

- Plan lives in `plan/`: `README.md` (folder layout + conventions) · `INDEX.md` (status) · `DECISIONS.md` (log) ·
  `EXECUTION.md` (build protocol) · `layers/N-<slug>/` (per-layer research, options, review, runs). Read at session
  start, update as work lands. A handoff file (`plan/HANDOFF-<date>.md`) is the canonical state when one exists.
- Per layer: research (parallel scouts) → options brief → user decides → fable/opus adversarial review → build per
  `EXECUTION.md` (small tasks + checks, parallel subagents on disjoint files, lead runs checks / reads diffs / bounces,
  one commit per verified task).
- Models (Anthropic plan): sonnet build/docs/UI · opus Rust · fable review · haiku tiny. (Kimi roster: k3[1m] main ·
  k3-256k research/parallel · kimi-for-coding bulk.)
- `README.md` stays user-facing and general; deep specs live per layer in `<layer>/README.md`, tasks in
  `<layer>/TASKS.md`.
- Layer folders are `N-<slug>` with `N` = the layer number: `0-knowledge` … `6-transport` (`7-relay`, `8-storage`,
  `9-evolution` reserved). `plan/layers/` uses the same names. Specs in `<N-slug>/README.md`, boards in
  `<N-slug>/TASKS.md`.
- Commit in this repo after every verified task. NEVER add Claude co-author or attribution lines.
- **Raspberry Pi is the floor (owner rule, 2026-10-19):** the OS without the local models was confirmed to run on a Raspberry Pi, and the
  required configuration stays that way: super light and super efficient. The base install is the thin 111 layer only; everything third-party is
  a separate, on-demand component that weighs nothing (no process, listener, loaded code, timer or open file) while it is off. No task may add a
  resident process, a periodic poll, a runtime dependency or a build-time weight to the required configuration without the owner's say; measure
  (RSS, binary size, listeners) and record it in the layer's `TASKS.md` when it changes.
- Ask the owner on interpretive forks; do not assume. Never reinvent the wheel: adopt existing projects, host their
  panels, do not recreate them.
- **Linux hygiene while building and testing:** nothing of ours stays running that the next step does not need. Dev servers
  (`pnpm dev`, Vite preview), `111 daemon`, headless Chrome, stub sidecars and relays are started for a check and stopped
  when it ends; the harnesses tear down their own namespaces. Before handing work to a subagent and after it returns,
  nothing of ours is listening or running (`ps` / `ss` check); a subagent starts a server only for its own check and stops
  it. Kept scratch roots (`SMOKE_KEEP=1` → `/tmp/111-smoke.*`) are deleted once inspected; temporary files and private
  `CARGO_TARGET_DIR`s live under the session scratchpad, never in the repo. At most two cargo builds at once on the 12 GB
  box; `smoke.sh` and `panel-lint.sh` never together.
