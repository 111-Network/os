# Layer 8 — research coordinates

Resume coordinates for the Layer 8 (Storage & Shared State) work. Reports land in `layers/8-storage/research/`.
Sessions die; this file is what the next session needs.

## Kickoff (2026-11-02, cloud session)

- Owner's words (2026-11-02): "start layer 8 and 9 research in parallel now and document all and before starting tidy up docs if
  needed." Layers 8 and 9 kick off together while Layer 7 is in its spec phase (D30, run 2 of `layers/7-relay`). `INDEX.md`
  "Process (per layer)": research → options brief → the owner's decision → adversarial pass → build; **no Layer 8 code before the
  owner decides.**
- Tree at kickoff: the Layer 7 D30 state on `claude/layer-6-implementation-006mte` (PR #2 open, CI green). The docs tidy-up
  before the kickoff: INDEX rows 8 and 9, the plan README rows and folder rules, AGENTS.md's layer-folder line, these plan folders.
- Box: the cloud container (4 CPUs, 15 GB RAM, a fixed disk allowance; two Workflow slots per run; three workflows run at once
  today: the Layer 7 spec panel and the two sweeps). **Network policy:** the proxy allows package indexes and
  `raw.githubusercontent.com`; it denies most documentation sites (`docs.rs`, `rfc-editor.org`, GitHub HTML pages,
  `api.github.com`, most project sites). The scouts are told so; claims they could not verify are collected by the critic in
  `06-critic.md` for the lead to verify on the owner's machine.

## Run 1 — research sweep (2026-11-02)

- Script: `plan/layers/8-storage/runs/workflows/research.js` (launched by `scriptPath`; keep it byte-identical to resume).
- Run id: `wf_68595f58-589` (launched 2026-11-02 ~00:35 UTC from the tree `a65ac6d`, beside the Layer 7 spec run `wf_27c6c677-781` and the
  other sweep; harness transcript dir `~/.claude/projects/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/subagents/workflows/wf_68595f58-589/`).
  Resume: `Workflow({scriptPath: "<repo>/plan/layers/8-storage/runs/workflows/research.js", resumeFromRunId: "wf_68595f58-589"})`.
- Models: seams scout opus (high) · lens scouts sonnet (high) · critic opus (xhigh) · gap-fill sonnet (high).

Next after the run: the lead writes `layers/8-storage/options.md` → the owner decides → `DECISIONS.md` → spec `os/8-storage/README.md`
→ fable adversarial panel → `TASKS.md` → build.
