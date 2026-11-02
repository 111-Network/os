# Layer 7 — research coordinates

Resume coordinates for the Layer 7 (Relay & Presence) work. Reports land in `layers/7-relay/research/`.
Sessions die; this file is what the next session needs.

## Kickoff (2026-11-01, cloud session)

- Owner's words (2026-11-01): "continue what's left to build … send cheaper sub agents and check their work". Layer 6 is locked
  (D29); Layers 7–9 are pending; `INDEX.md` "Process (per layer)" starts a layer with research → options brief → the owner's decision. So the kickoff
  is the research sweep and the brief; **no Layer 7 code before the owner decides.**
- Tree at kickoff: `01b4a72` (the CI fix on PR #2) on `claude/layer-6-implementation-006mte`, which is `origin/main` (`322a66d`)
  plus the Layer 6 build; PR #2 is open and unmerged on GitHub at kickoff (the branch is the one most ahead).
- Box: the cloud container (4 CPUs, 15 GB RAM, a fixed disk allowance; two Workflow slots per run). **Network policy:** the proxy
  allows package indexes and `raw.githubusercontent.com`; it denies most documentation sites (`meshtastic.org`,
  `reticulum.network`, `docs.rs`, `rfc-editor.org`, GitHub HTML pages, `api.github.com`). The scouts are told so; claims they
  could not verify are collected by the critic in `06-critic.md` for the lead to verify on the owner's machine.

## Run 1 — research sweep (2026-11-01)

- Script: `plan/layers/7-relay/runs/workflows/research.js` (launched by `scriptPath`; keep it byte-identical to resume).
- Workflow run id: `wf_60f165f1-80f` (launched 2026-11-01 ~21:55 UTC from the tree `bdea810`; the scouts read `3bf545b`, the commit after launch; harness transcript dir `~/.claude/projects/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/subagents/workflows/wf_60f165f1-80f/`). Resume: `Workflow({scriptPath: "<repo>/plan/layers/7-relay/runs/workflows/research.js", resumeFromRunId: "wf_60f165f1-80f"})`.
- Models: seams scout opus (high) · lens scouts sonnet (high) · critic opus (xhigh) · gap-fill sonnet (high).

| Lens | Report | Question |
|---|---|---|
| 0 seams | `00-code-seams.md` | line-verified map of where Layer 7 plugs into the as-built mesh111 / node111 / UI / agent; the timer baseline of the required configuration |
| 1 presence | `01-presence-discovery.md` | presence and discovery per transport; heartbeats vs event-driven (airtime arithmetic); the Service Discovery Registry; the mesh-epoch source |
| 2 gateways | `02-gateways-global.md` | what "the global network" honestly is; store-and-forward carriers for sealed 111 envelopes between islands; consent, privacy, legal |
| 3 relay | `03-fabric-relay.md` | multi-hop and cross-hub delivery inside the fabric: Reticulum as router, anti-entropy, IP mesh routing, hub-centric star; mesh111 amendments |
| 4 kb-updates | `04-kb-updates.md` | signed releases and the KB from a hub with no internet; manifest gossip vs hub-as-origin; trust model |
| 5 ui-floor | `05-ui-consent-floor.md` | Nearby radar/map offline, presence UI, consent; the Pi-floor weight table of every candidate |
| critic | `06-critic.md` | completeness verdict, gaps, conflicts, owner forks, the unverified-claims list |
| gap-fill | `07-gap-1.md`, `07-gap-2.md` | one per blocking gap (cap 2): G1 the adopted-carrier sweep and the internet link's security; G2 the one cross-hub custody shape |

**Run 1 complete** (2026-11-01, ~21:55–22:44 UTC): 9 agents (the opus seams scout, five sonnet lens scouts, the opus critic, two
sonnet gap-fill scouts), about 1.88 M subagent tokens, 519 tool calls, 49 min wall clock. The critic's verdict: "close to ready", two
blocking gaps (G1 the carrier sweep had skipped the KB gate and three candidates; G2 three cross-hub shapes in conflict), both filled
(`07-gap-1.md`, `07-gap-2.md`) and folded by the lead into `options.md` (F3, F4, F5, F10, facts 8 and 23–24, §3, §5 items 15–21, §6).
The brief is complete; it waits on the owner's decisions F1–F11 and the §5 questions. Nothing of the run is left running. The run's
journal is not exported (unlike Layer 6's `runs/journals/`): this box refuses to copy from the session-local transcript store; it lives
at `~/.claude/projects/…/subagents/workflows/wf_60f165f1-80f/journal.jsonl` on the lead machine and the run resumes by its id.

`options.md` is written (above); **the owner decided on 2026-11-01 (D30: every pick and default)**. Next: spec `os/7-relay/README.md` v0.1
(run 2, `runs/workflows/spec.js`: an opus drafter, then three fable judges) → lead triage → fix and one fable re-check → lock → `TASKS.md` → build.

## Run 2 — spec draft and fable panel (2026-11-01)

- Script: `runs/workflows/spec.js` (Draft: one opus author, xhigh, writes `os/7-relay/README.md` v0.1 from D30, the brief, the reports
  and the as-built code, modelled on `6-transport/README.md`; Panel: three fable judges, read-only, one report each in `review/`:
  `custody.md` (C), `carriers.md` (S), `integration.md` (I)). `args.panelOnly` reruns the panel on the file as it stands.
- Run id: `wf_27c6c677-781` (launched 2026-11-01 ~23:40 UTC from the tree `2c144e9`). Resume:
  `Workflow({scriptPath: "<repo>/plan/layers/7-relay/runs/workflows/spec.js", resumeFromRunId: "wf_27c6c677-781"})`.
- Result (2026-11-02): v0.1 drafted (1,100 lines, 262 verified citations, eleven recorded deviations, three questions); the panel's three
  verdicts all amend-then-lock: 4 blockers (C1, C2, S1, I1), 17 majors, 40 minors. The lead's triage `review/triage.md` accepts every
  finding (six with a variation) and lists six owner forks; D31 records the amendments and applies each fork as the lead recommends.

## Run 3 — fix and re-check (2026-11-02)

- Script: `runs/workflows/spec-fix.js` (Fix: one opus editor applies `review/triage.md` to `os/7-relay/README.md` → v0.2, appends the
  changelog; Recheck: one fable judge verifies every disposition landed and nothing regressed, writes `review/recheck.md`;
  `args.triage`, `args.from`, `args.to`, `args.recheckOnly`).
- Run id: `wf_b307f6ca-f43` (launched 2026-11-02 from the tree `e0ebd71`). Resume:
  `Workflow({scriptPath: "<repo>/plan/layers/7-relay/runs/workflows/spec-fix.js", resumeFromRunId: "wf_b307f6ca-f43"})`.
- Result (2026-11-02): v0.2 (`00d17b1`; all 61 findings, the eleven deviations and the numbers applied, sixteen editor deviations within the
  triage); the re-check's verdict **lock** with three minors (R1–R3) folded into the lock edit by the lead → **v1.0 locked, D32** (`4689dbf`).
- The tag `checkpoint-layer-6` (annotated, at `4689dbf`: the last tree before Layer 7 code; G9's old hub) exists locally; this session's git
  proxy disconnects on every tag push (the branch pushes fine), so the owner pushes it from their machine (`git push origin checkpoint-layer-6`
  after fetching the branch and tagging `4689dbf`); until then G9 takes the commit hash.

## Run 4 — build board (2026-11-02)

- Script: `runs/workflows/board.js` (Draft: one opus planner writes `os/7-relay/TASKS.md` from the locked spec per `EXECUTION.md`, modelled on
  the Layer 6 board; Check: one fable judge builds the section-to-task coverage matrix, checks file ownership per wave, the checks, D32's order,
  the floor rows and the form → `review/board-check.md`; Fix: one planner pass and a re-check when the judge says fix-first).
- Run id: `wf_125acb61-573` (launched 2026-11-02 from the tree `78b0d08`). Resume:
  `Workflow({scriptPath: "<repo>/plan/layers/7-relay/runs/workflows/board.js", resumeFromRunId: "wf_125acb61-573"})`.
- Result (2026-11-02): the board (141 lines; T0, Wave 0 P1–P2/L1–L3/M1–M7/D1/C1, Wave 1 D2–D10/U1–U3/A1/K1–K2, Wave 2 D11/L4/V1/K3–K4, a gated
  D12, Wave 3 V2–V10, the Pi-floor record, the external gates); the judge: fix-first (one blocker, the `lan` mark no row set; two majors; eleven
  minors) → fix pass → **ready** with five minors (n1–n5) applied by the lead; `review/board-check.md`.

## Run 5 — T0 scaffold (2026-11-02)

- Script: `runs/workflows/t0.js` (one opus agent lays the T0 row: the verify skeleton, the module and router stubs, the ntfy KB candidate entry,
  the LICENSING fix; the lead runs fmt, clippy, `build.py --check`, shellcheck, reads the diff and commits "Layer 7 scaffold").
- Run id: `wf_0cc2beb9-430` (launched 2026-11-02 from the tree `561cb5e`). Resume:
  `Workflow({scriptPath: "<repo>/plan/layers/7-relay/runs/workflows/t0.js", resumeFromRunId: "wf_0cc2beb9-430"})`.
- Result (2026-11-02): done in one pass; the lead re-ran fmt, clippy, the test build, `build.py --check`, shellcheck, a stub's SKIPPED and
  REQUIRE paths and the notices check, read the diff and committed "Layer 7 scaffold" (`005a510`). Agent notes kept for the rows: `relay::tap` is
  wired by M5 in `FabricHandle::publish`; `carrier/mod.rs` carries a local `HandOff` trait and mirrors of `CarrierFrame`, `ForwardDone`,
  `ForwardPending` until M4 lands (D11 adapts them); `Pair {own, peer, topic_key}`; ntfy's licence read as `Apache-2.0 OR GPL-2.0-only` (no "or
  later" wording seen); upstream's tag cadence is not monthly (none in September 2026, ten in October), recorded in the entry.

## Run 6 — Wave 0 (2026-11-02)

- Script: `runs/workflows/wave-0.js` (three stages for the landing order, two slots: A = P1 C1 M1 M2 M6 M3 L1; B = M4 L2 L3 D1 M7 P2; C = M5;
  `args.only` = a subset of ids, `args.stopAfter` = "A" | "B"). The lead reviews each returned task against its row (the check re-run, the diff
  read) and commits it; a failed task is bounced with the same script and `args.only`.
- Run id: `wf_5c3ab45e-2a7` (launched 2026-11-02 from the tree `005a510`). Resume:
  `Workflow({scriptPath: "<repo>/plan/layers/7-relay/runs/workflows/wave-0.js", resumeFromRunId: "wf_5c3ab45e-2a7"})`.
- Progress (2026-11-02, ~04:10 UTC): stage A returned 7/7 (six done, L1 partial on one unowned test line the lead applied:
  `tests/manifest_parse.rs` floor assertion moved to the T2 reading). Landed with the lead's re-run green: M1 `57af4bc`, M2 `ef84094`, M3 `7746eef`,
  M6 `22de995` (board rows ✅, `758675f`). Reviewed and waiting only for the node111 re-run: P1, C1, L1, L2 (stage B's M4 edits `M/fabric.rs`, which
  node111 compiles from disk, so the re-run waits for M4's return; the UI checks of C1 and the mesh111 checks re-ran green). In flight: M4, L3; then
  D1, M7, P2; stage C M5. Wave 1 script written (`runs/workflows/wave-1.js`, `ef69cd5`, facts block to finish from M4/M5/D1/L3 before the launch).

