# 111network — OS build (repo-local copy of the project rules)

- Plan lives in `plan/`: `README.md` (folder layout + conventions) · `INDEX.md` (status) · `DECISIONS.md` (log) ·
  `EXECUTION.md` (build protocol) · `layers/NN-<slug>/` (per-layer research, options, review, runs). Read at session
  start, update as work lands. A handoff file (`plan/HANDOFF-<date>.md`) is the canonical state when one exists.
- Per layer: research (parallel scouts) → options brief → user decides → fable/opus adversarial review → build per
  `EXECUTION.md` (small tasks + checks, parallel subagents on disjoint files, lead runs checks / reads diffs / bounces,
  one commit per verified task).
- Models (Anthropic plan): sonnet build/docs/UI · opus Rust · fable review · haiku tiny. (Kimi roster: k3[1m] main ·
  k3-256k research/parallel · kimi-for-coding bulk.)
- `README.md` stays user-facing and general; deep specs live per layer in `<layer>/README.md`, tasks in
  `<layer>/TASKS.md`.
- Layer folders are `NN-<slug>` with `NN` = layer number × 10: `00-knowledge` … `50-messaging` (`60-transport`, `70-relay`,
  `80-storage`, `90-evolution` reserved). `plan/layers/` uses the same names. Specs in `<NN-slug>/README.md`, boards in
  `<NN-slug>/TASKS.md`.
- Commit in this repo after every verified task. NEVER add Claude co-author or attribution lines.
- Ask the owner on interpretive forks; do not assume. Never reinvent the wheel: adopt existing projects, host their
  panels, do not recreate them.
