# Layer 9 — research coordinates

Resume coordinates for the Layer 9 (Self-Healing & Evolution) work. Reports land in `layers/9-evolution/research/`.
Sessions die; this file is what the next session needs.

## Kickoff (2026-11-02, cloud session)

- Owner's words (2026-11-02): "start layer 8 and 9 research in parallel now and document all and before starting tidy up docs if
  needed." Layers 8 and 9 kick off together while Layer 7 is in its spec phase (D30, run 2 of `layers/7-relay`). `INDEX.md`
  "Process (per layer)": research → options brief → the owner's decision → adversarial pass → build; **no Layer 9 code before the
  owner decides.**
- Tree at kickoff: the Layer 7 D30 state on `claude/layer-6-implementation-006mte` (PR #2 open, CI green). The docs tidy-up
  before the kickoff: INDEX rows 8 and 9, the plan README rows and folder rules, AGENTS.md's layer-folder line, these plan folders.
- Box: the cloud container (4 CPUs, 15 GB RAM, a fixed disk allowance; two Workflow slots per run; three workflows run at once
  today: the Layer 7 spec panel and the two sweeps). **Network policy:** the proxy allows package indexes and
  `raw.githubusercontent.com`; it denies most documentation sites (`docs.rs`, `rfc-editor.org`, GitHub HTML pages,
  `api.github.com`, most project sites). The scouts are told so; claims they could not verify are collected by the critic in
  `06-critic.md` for the lead to verify on the owner's machine.

## Run 1 — research sweep (2026-11-02)

- Script: `plan/layers/9-evolution/runs/workflows/research.js` (launched by `scriptPath`; keep it byte-identical to resume).
- Run id: `wf_d17d28de-ce6` (launched 2026-11-02 ~00:35 UTC from the tree `a65ac6d`, beside the Layer 7 spec run `wf_27c6c677-781` and the
  other sweep; harness transcript dir `~/.claude/projects/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/subagents/workflows/wf_d17d28de-ce6/`).
  Resume: `Workflow({scriptPath: "<repo>/plan/layers/9-evolution/runs/workflows/research.js", resumeFromRunId: "wf_d17d28de-ce6"})`.
- Models: seams scout opus (high) · lens scouts sonnet (high) · critic opus (xhigh) · gap-fill sonnet (high).

- Result (2026-11-02): complete — 8 agents (the seams scout, five lens scouts, the critic, one gap-fill (G1, the hub code-fix pipeline: `07-gap-1.md`); the critic rated G2 (`AGENT_OP` delegation for 2A/3) and G3 (playbook rule engine) major and left them to the lead); reports `00`–`07`
  committed; the critic's verdict: enough for the brief once the remaining gaps are closed.

## Run 2 — options brief (2026-11-02)

- Script: `plan/layers/9-evolution/runs/workflows/options.js` (shared with the other layer; `args.layer` = `9-evolution`): two sonnet gap-fill
  scouts on the gaps the critic left for the lead (`research/07-gap-2.md`, `07-gap-3.md`) → one opus drafter writes `options.md` in the
  Layer 7 brief's shape → one fable judge verifies every fact, fork and pick against the reports and the locked rules (`review/options-check.md`)
  → one fix pass by the drafter and a re-check when the judge says fix-first.
- Run id: `wf_e390b830-02a` (launched 2026-11-02 from the tree `dfd6828`, beside the other layer's run and the Layer 7 fix run). Resume:
  `Workflow({scriptPath: "<repo>/plan/layers/9-evolution/runs/workflows/options.js", args: {layer: "9-evolution"}, resumeFromRunId: "wf_e390b830-02a"})`.

Next after the run: the lead reads the judge's report and the brief, fixes what the judge left, presents the decision sheet to the owner →
the owner decides → `DECISIONS.md` → spec `os/9-evolution/README.md` → fable adversarial panel → `TASKS.md` → build.
