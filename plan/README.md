# os/plan — how the plan folder works

The plan folder is the working record of how the 111network OS is specced and built, layer by layer. The system is
unchanged: a status board, a decision log, a build protocol, and per-layer research, options, review and build logs.
This file explains where everything lives and how files are named.

## The three top-level documents

| File | Role |
|---|---|
| [`INDEX.md`](INDEX.md) | Status board: process, model roster, one row per layer (status, spec, plan folder). Read first. |
| [`DECISIONS.md`](DECISIONS.md) | Decision log: every owner decision with rejected alternatives (D1, D2, ...). |
| [`EXECUTION.md`](EXECUTION.md) | Build protocol: tasks with criteria and checks, parallel subagents, self-review, commit per verified task. |

## Layout

```
os/plan/
  README.md            this file
  INDEX.md             status board
  DECISIONS.md         decision log
  EXECUTION.md         build protocol
  HANDOFF-<date>.md    session handoff (newest = current; see "Handoffs")
  cross-layer/         material that spans layers
    devices.md           device-connection core brief (Layer 6 opener; read by Layers 4, 5, 6)
    parked.md            ideas parked by the owner for later layers
  layers/
    0N-<slug>/         one folder per layer that has plan material
      README.md          one-screen index: status line, links to every file, spec + TASKS.md in os/<slug>/
      options.md         options brief (research condensed into 2-4 options with a recommendation)
      research/          research reports, plus coordinates.md (run ids, resume info, build-wave log)
      review/            triage.md (lead's disposition of the adversarial panel) + per-judge and per-wave reports
      runs/              orchestration logs (only layers built with Workflow runs)
        workflows/         the Workflow scripts, one per run: <name>.js (no leading dot)
        journals/          exported run journals: <run-label>.<wf_id>.jsonl, plus a README mapping them to scripts
```

Layer folders exist only where there is plan material. A folder holds only the parts that layer produced
(Layer 0 has no research folder, for example). The spec and the task board are not here: they live in `os/<slug>/README.md`
and `os/<slug>/TASKS.md`, where the layer's code is.

### Layers and their folders

| Layer | Folder | Notes |
|---|---|---|
| 0 Knowledge | `layers/00-knowledge/` | options, triage |
| 1 Bootstrap | `layers/10-bootstrap/` | options, triage |
| 2 Identity | `layers/20-identity/` | options, research coordinates, triage |
| 3 Local Intelligence | none | no plan material: no research sweep or panel was run; see D10 and `os/30-intelligence/` |
| 4 Interface & Dashboard | `layers/40-interface/` | options, triage |
| 5 Messaging & Data | `layers/50-messaging/` | full set: research, options, review, runs |
| 6 Transport & Bridge | none yet | opener brief is [`cross-layer/devices.md`](cross-layer/devices.md); folder is created at Layer 6 kickoff |
| 7 to 9 | none yet | pending |

## Naming conventions

- **Layer folder:** `layers/0N-<slug>/`, where `N` is the layer number from `INDEX.md` and `<slug>` is the layer's directory name under `os/` (`knowledge`, `bootstrap`, `identity`, `interface`, `messaging`).
- **Fixed names inside a layer folder:** `0N-<slug>/README.md`, `0N-<slug>/options.md`, `0N-<slug>/research/coordinates.md`, `0N-<slug>/review/triage.md`. A layer that lacks one of these simply has not produced it.
- **Research reports:** `research/NN-<topic>.md`, numbered in run order. Gap-fill reports reuse one number and add a suffix (07-gap-1, 07-gap-2, ...).
- **Review reports:** `review/<lens>.md` for each judge of the spec panel (crypto, panels, integration), `0N-<slug>/review/recheck.md` for the re-check, `0N-<slug>/review/wave-N.md` for the triage of build wave N.
- **Workflow scripts:** `runs/workflows/<name>.js`: `research.js`, `spec-review.js`, `wave-N.js`, `wave-N-review.js`, `wave-N-bounce.js`.
- **Journals:** `runs/journals/<label>.<wf_id>.jsonl`, one JSON line per finished agent.
- **Handoffs:** `HANDOFF-YYYY-MM-DD.md`.

### How paths are written inside plan documents

- Markdown links are relative to the file that contains them.
- Backticked plan paths are written from the plan root (`layers/50-messaging/options.md`) or from the repo (`os/plan/layers/50-messaging/options.md`, `plan/layers/...`). Documents outside `plan/` always use the `os/plan/...` or `plan/...` form.
- A bare file name such as `DECISIONS.md` or `INDEX.md` means the top-level file of that name.

## Where does X go?

| You are producing | Put it in |
|---|---|
| A research scout report | `layers/0N-<slug>/research/NN-<topic>.md` |
| Run ids, resume info, quota-wall notes, build-wave log | `layers/0N-<slug>/research/coordinates.md` (append; sessions die, this file survives) |
| The options brief for the owner | `layers/0N-<slug>/options.md` |
| An adversarial panel report or re-check | `layers/0N-<slug>/review/` |
| The lead's triage of a panel or a build wave | `layers/0N-<slug>/review/triage.md` or `review/wave-N.md` |
| A Workflow script you will launch or resume | `layers/0N-<slug>/runs/workflows/<name>.js` (must stay under the repo: `scriptPath` has to be readable after a restart) |
| An exported workflow journal | `layers/0N-<slug>/runs/journals/` and a row in its README |
| An owner decision | `DECISIONS.md` (new D-number, rejected alternatives included) |
| Layer status change | the layer's row in `INDEX.md`, then the status line in the layer README |
| A brief or note that spans layers | `cross-layer/` |
| A session handoff | `HANDOFF-YYYY-MM-DD.md` at the plan root |
| The layer spec, task board, code | `os/<slug>/` (never here) |

### Starting a new layer folder

1. `git mv` or create `layers/0N-<slug>/` with only the parts you have.
2. Write its `README.md` (status line copied from the `INDEX.md` row, links to every file, links to `os/<slug>/README.md` and `TASKS.md`).
3. Put the folder link in the Plan column of the layer's `INDEX.md` row.

### Handoffs

A handoff is the one document a fresh session needs to continue exactly where work stopped. It lives at the plan root as
`HANDOFF-YYYY-MM-DD.md` while it is current. The 2026-10-15 handoff is [`HANDOFF-2026-10-15.md`](HANDOFF-2026-10-15.md).
It still cites the pre-restructure paths; use the move table below to translate them.

## Moves on 2026-10-15

Every file below was moved with `git mv`, except the journals, which were untracked and moved with `mv`.
Paths are relative to `os/`.

| Old path | New path |
|---|---|
| `plan/devices.md` | `plan/cross-layer/devices.md` |
| `plan/parked.md` | `plan/cross-layer/parked.md` |
| `plan/layer-0-options.md` | `plan/layers/00-knowledge/options.md` |
| `plan/layer-0-review.md` | `plan/layers/00-knowledge/review/triage.md` |
| `plan/layer-1-options.md` | `plan/layers/01-bootstrap/options.md` |
| `plan/layer-1-review.md` | `plan/layers/01-bootstrap/review/triage.md` |
| `plan/layer-2-options.md` | `plan/layers/02-identity/options.md` |
| `plan/layer-2-research.md` | `plan/layers/02-identity/research/coordinates.md` |
| `plan/layer-2-review.md` | `plan/layers/02-identity/review/triage.md` |
| `plan/layer-4-options.md` | `plan/layers/04-interface/options.md` |
| `plan/layer-4-review.md` | `plan/layers/04-interface/review/triage.md` |
| `plan/layer-5-options.md` | `plan/layers/05-messaging/options.md` |
| `plan/layer-5-research.md` | `plan/layers/05-messaging/research/coordinates.md` |
| `plan/layer-5-research/00-code-seams.md` | `plan/layers/05-messaging/research/00-code-seams.md` |
| `plan/layer-5-research/01-fabric.md` | `plan/layers/05-messaging/research/01-fabric.md` |
| `plan/layer-5-research/02-bridges-and-budgets.md` | `plan/layers/05-messaging/research/02-bridges-and-budgets.md` |
| `plan/layer-5-research/03-hosted-panels.md` | `plan/layers/05-messaging/research/03-hosted-panels.md` |
| `plan/layer-5-research/04-envelope.md` | `plan/layers/05-messaging/research/04-envelope.md` |
| `plan/layer-5-research/05-data-store-and-bulk.md` | `plan/layers/05-messaging/research/05-data-store-and-bulk.md` |
| `plan/layer-5-research/06-critic.md` | `plan/layers/05-messaging/research/06-critic.md` |
| `plan/layer-5-research/07-gap-1.md` | `plan/layers/05-messaging/research/07-gap-1.md` |
| `plan/layer-5-research/07-gap-2.md` | `plan/layers/05-messaging/research/07-gap-2.md` |
| `plan/layer-5-research/07-gap-3-meshmonitor.md` | `plan/layers/05-messaging/research/07-gap-3-meshmonitor.md` |
| `plan/layer-5-research/workflow-research.js` | `plan/layers/05-messaging/runs/workflows/research.js` |
| `plan/layer-5-review.md` | `plan/layers/05-messaging/review/triage.md` |
| `plan/layer-5-review/crypto.md` | `plan/layers/05-messaging/review/crypto.md` |
| `plan/layer-5-review/panels.md` | `plan/layers/05-messaging/review/panels.md` |
| `plan/layer-5-review/integration.md` | `plan/layers/05-messaging/review/integration.md` |
| `plan/layer-5-review/recheck.md` | `plan/layers/05-messaging/review/recheck.md` |
| `plan/layer-5-review/wave-1.md` | `plan/layers/05-messaging/review/wave-1.md` |
| `plan/layer-5-review/wave-2.md` | `plan/layers/05-messaging/review/wave-2.md` |
| `plan/layer-5-review/workflow-review.js` | `plan/layers/05-messaging/runs/workflows/spec-review.js` |
| `plan/layer-5-review/journals/README.md` | `plan/layers/05-messaging/runs/journals/README.md` |
| `plan/layer-5-review/journals/*.jsonl` (8 files) | `plan/layers/05-messaging/runs/journals/*.jsonl` |
| `messaging/.workflow-wave-0.js` | `plan/layers/05-messaging/runs/workflows/wave-0.js` |
| `messaging/.workflow-wave-1.js` | `plan/layers/05-messaging/runs/workflows/wave-1.js` |
| `messaging/.workflow-wave-1-review.js` | `plan/layers/05-messaging/runs/workflows/wave-1-review.js` |
| `messaging/.workflow-wave-1-bounce.js` | `plan/layers/05-messaging/runs/workflows/wave-1-bounce.js` |
| `messaging/.workflow-wave-2.js` | `plan/layers/05-messaging/runs/workflows/wave-2.js` |
| `messaging/.workflow-wave-2-review.js` | `plan/layers/05-messaging/runs/workflows/wave-2-review.js` |
| `messaging/.workflow-wave-2-bounce.js` | `plan/layers/05-messaging/runs/workflows/wave-2-bounce.js` |
| `messaging/.workflow-wave-3.js` | `plan/layers/05-messaging/runs/workflows/wave-3.js` |

Unchanged: `plan/INDEX.md`, `plan/DECISIONS.md`, `plan/EXECUTION.md`, `plan/HANDOFF-2026-10-15.md`.
Every reference to a moved path in `os/` and the repo `CLAUDE.md` was rewritten to the new path, except
`HANDOFF-2026-10-15.md`, which still uses the old ones.

Left as they were on purpose: the `meta.name` of `research.js` and `spec-review.js` (run names, not paths), the exported `*.jsonl`
journals (recorded agent output), and mentions of files outside the repo (the harness script under `~/.claude/` and a deleted
temp copy in `layers/02-identity/research/coordinates.md`).

Two edits inside the moved Workflow scripts beyond prompt path strings: the `OUT` constants of `research.js` and
`spec-review.js` now point at the new report folders. Editing a script changes the cache keys of its recorded run, so a
`resumeFromRunId` replay of a run launched from the old text re-runs the changed calls live.
