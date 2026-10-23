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
  cross-layer/         material that spans layers (devices.md, parked.md, ci-cd.md — CI/CD + GitHub hygiene, D28)
    devices.md           device-connection core brief (Layer 6 opener; read by Layers 4, 5, 6)
    parked.md            ideas parked by the owner for later layers
    ci-cd.md             CI/CD and GitHub hygiene (D28): what runs on GitHub, the owner's repository settings, the CD plan
  layers/
    N-<slug>/         one folder per layer that has plan material
      README.md          one-screen index: status line, links to every file, spec + TASKS.md in os/N-<slug>/
      options.md         options brief (research condensed into 2-4 options with a recommendation)
      research/          research reports, plus coordinates.md (run ids, resume info, build-wave log)
      review/            triage.md (lead's disposition of the adversarial panel) + per-judge and per-wave reports
      runs/              orchestration logs (only layers built with Workflow runs)
        workflows/         the Workflow scripts, one per run: <name>.js (no leading dot)
        journals/          exported run journals: <run-label>.<wf_id>.jsonl, plus a README mapping them to scripts
```

Layer folders exist only where there is plan material. A folder holds only the parts that layer produced
(Layer 0 has no research folder, for example). The spec and the task board are not here: they live in `os/N-<slug>/README.md`
and `os/N-<slug>/TASKS.md`, where the layer's code is.

### Layers and their folders

| Layer | Folder | Notes |
|---|---|---|
| 0 Knowledge | `layers/0-knowledge/` | options, triage |
| 1 Bootstrap | `layers/1-bootstrap/` | options, triage |
| 2 Identity | `layers/2-identity/` | options, research coordinates, triage |
| 3 Local Intelligence | none | no plan material: no research sweep or panel was run; see D10 and `os/3-intelligence/` |
| 4 Interface & Dashboard | `layers/4-interface/` | options, triage |
| 5 Messaging & Data | `layers/5-messaging/` | full set: research, options, review, runs |
| 6 Transport & Bridge | `layers/6-transport/` | research (R00–R08 + coordinates), options, review (the 3-judge panel, the recheck, V5 `review/v5-integrated.md`), runs (workflows `runs/workflows/*.js`, journals); opener brief [`cross-layer/devices.md`](cross-layer/devices.md); built and locked 2026-10-23 (D29) |
| 7 Relay & Presence | none yet | reserved name `7-relay`; pending |
| 8 Storage & Shared State | none yet | reserved name `8-storage`; pending |
| 9 Self-Healing & Evolution | none yet | reserved name `9-evolution`; pending |

## Naming conventions

- **Layer folder:** `layers/N-<slug>/`, where `N` is the layer number from `INDEX.md` and the name is identical to the layer's own folder under `os/`. Current names: `0-knowledge`, `1-bootstrap`, `2-identity`, `4-interface`, `5-messaging`, `6-transport` (Layer 3, `3-intelligence`, has no plan material, so no plan folder). Reserved for layers not started yet: `7-relay`, `8-storage`, `9-evolution`. `plan/` and `hooks/` are not layers and stay unnumbered.
- **Fixed names inside a layer folder:** `N-<slug>/README.md`, `N-<slug>/options.md`, `N-<slug>/research/coordinates.md`, `N-<slug>/review/triage.md`. A layer that lacks one of these simply has not produced it.
- **Research reports:** `research/NN-<topic>.md`, numbered in run order. Gap-fill reports reuse one number and add a suffix (07-gap-1, 07-gap-2, ...).
- **Review reports:** `review/<lens>.md` for each judge of the spec panel (crypto, panels, integration), `N-<slug>/review/recheck.md` for the re-check, `N-<slug>/review/wave-N.md` for the triage of build wave N.
- **Workflow scripts:** `runs/workflows/<name>.js`: `research.js`, `spec-review.js`, `wave-N.js`, `wave-N-review.js`, `wave-N-bounce.js`, `wave-N-fix.js`, `plan-check.js` (a letter after the wave number, `wave-2d.js`, `wave-3c-fix.js`, marks a round inside the wave).
- **Journals:** `runs/journals/<label>.<wf_id>.jsonl`, one JSON line per finished agent.
- **Handoffs:** `HANDOFF-YYYY-MM-DD.md`.

### How paths are written inside plan documents

- Markdown links are relative to the file that contains them.
- Backticked plan paths are written from the plan root (`layers/5-messaging/options.md`) or from the repo (`os/plan/layers/5-messaging/options.md`, `plan/layers/...`). Documents outside `plan/` always use the `os/plan/...` or `plan/...` form.
- A bare file name such as `DECISIONS.md` or `INDEX.md` means the top-level file of that name.

## Where does X go?

| You are producing | Put it in |
|---|---|
| A research scout report | `layers/N-<slug>/research/NN-<topic>.md` |
| Run ids, resume info, quota-wall notes, build-wave log | `layers/N-<slug>/research/coordinates.md` (append; sessions die, this file survives) |
| The options brief for the owner | `layers/N-<slug>/options.md` |
| An adversarial panel report or re-check | `layers/N-<slug>/review/` |
| The lead's triage of a panel or a build wave | `layers/N-<slug>/review/triage.md` or `review/wave-N.md` |
| A Workflow script you will launch or resume | `layers/N-<slug>/runs/workflows/<name>.js` (must stay under the repo: `scriptPath` has to be readable after a restart) |
| An exported workflow journal | `layers/N-<slug>/runs/journals/` and a row in its README |
| An owner decision | `DECISIONS.md` (new D-number, rejected alternatives included) |
| Layer status change | the layer's row in `INDEX.md`, then the status line in the layer README |
| A brief or note that spans layers | `cross-layer/` |
| A session handoff | `HANDOFF-YYYY-MM-DD.md` at the plan root |
| The layer spec, task board, code | `os/N-<slug>/` (never here) |

### Starting a new layer folder

1. `git mv` or create `layers/N-<slug>/` with only the parts you have. It takes the same name as the layer's folder under `os/`.
2. Write its `README.md` (status line copied from the `INDEX.md` row, links to every file, links to `os/N-<slug>/README.md` and `TASKS.md`).
3. Put the folder link in the Plan column of the layer's `INDEX.md` row.

### Handoffs

A handoff is the one document a fresh session needs to continue exactly where work stopped. It lives at the plan root as
`HANDOFF-YYYY-MM-DD.md` while it is current. Once its work is done it is marked closed at the top and stays as the record; the newest open one is current.
The handoffs so far: [`HANDOFF-2026-10-15.md`](HANDOFF-2026-10-15.md) (Layer 5, historical) and [`HANDOFF-2026-10-20.md`](HANDOFF-2026-10-20.md) (Layer 6, closed 2026-10-23); none is open now.
The 2026-10-15 handoff's paths were rewritten to the numbered layer folders on 2026-10-18 and again on 2026-10-20; the state it describes is unchanged.

> The paths in the section below are as they were on 2026-10-15. The layer folders at the repo root and under `layers/`
> were renumbered on 2026-10-18 and again on 2026-10-20 (see the next two sections), so read `bootstrap/` as `1-bootstrap/`,
> a plan folder named `05-messaging` as `5-messaging`, and so on. The handoff it mentions was rewritten to the new names both times.

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

> The names in the section below are as they were on 2026-10-18. The same folders were renumbered to one digit on
> 2026-10-20 (see the next section): read `10-bootstrap/` as `1-bootstrap/`, `layers/50-messaging/` as `layers/5-messaging/`, and so on.

## Moves on 2026-10-18

Why: the tree now reads top to bottom in layer order, one numbering scheme (layer number × 10) names the layer folder at
the repo root and under `layers/`, and the gaps leave room between layers.

Only directories were renamed. Every file inside moved with `git mv`, so `git log --follow` works on all of them.
Paths are relative to `os/`.

| Old path | New path |
|---|---|
| `knowledge/` | `00-knowledge/` |
| `bootstrap/` | `10-bootstrap/` |
| `identity/` | `20-identity/` |
| `intelligence/` | `30-intelligence/` |
| `interface/` | `40-interface/` |
| `messaging/` | `50-messaging/` |
| `plan/layers/01-bootstrap/` | `plan/layers/10-bootstrap/` |
| `plan/layers/02-identity/` | `plan/layers/20-identity/` |
| `plan/layers/04-interface/` | `plan/layers/40-interface/` |
| `plan/layers/05-messaging/` | `plan/layers/50-messaging/` |

`plan/layers/00-knowledge/` kept its name. `plan/` and `hooks/` are not layers and stay unnumbered. Folders for the layers
not started yet (`60-transport`, `70-relay`, `80-storage`, `90-evolution`) are created at each layer's kickoff, not now.

One script rewrote every path string in code, scripts, configs, docs, plan files and the ten Layer 5 workflow scripts. Its
dry run was reviewed line by line before it was applied. The forms it handled:

- `os/<slug>/` and `../<slug>/`
- `$OS/<slug>` and `${ROOT}/<slug>/`
- root-relative `<slug>/<child>`
- Python split segments such as `os.path.join(OS_DIR, "bootstrap", ...)`
- plan-folder paths under `layers/`, moved to the new `NN-<slug>` names
- GitHub URLs into the repo

Two generated artifacts were regenerated from their generators instead of edited:

- `THIRD-PARTY-NOTICES.md`, via `10-bootstrap/scripts/gen-notices.py`. Its lockfile-path table and the hashes of the two
  hand-maintained license lists changed.
- The seven `00-knowledge/**/llms.txt` indexes, via `00-knowledge/_meta/build.py`. Its do-not-hand-edit marker now names the
  new path.

Left as they were on purpose:

- The frozen Layer 5 test vectors: `50-messaging/test-vectors/envelope.json`, and the generator line in
  `50-messaging/crates/mesh111/src/test_double.rs` that emits its description, which still says `identity/test-vectors/...`.
- The exported workflow journals, `plan/layers/50-messaging/runs/journals/*.jsonl` (recorded agent output).
- The node's runtime state directory, which is also called `identity/`, and the node111 module `src/identity/`. They share a
  name with the old layer folder but are not it.
- The "Moves on 2026-10-15" table above, which is history.

The sibling website repo (`../website`) gets its own commit updating its `os/...` references (47 files, comments and docs,
plus the GitHub link to the install shim).

Gates on the renamed tree match the pre-move baseline exactly. Three reds were already there before the move and are
unchanged:

- `cargo fmt` on four Layer 5 files.
- The two `restore_pending` tests of the unfinished F2 bounce.
- The hermetic cross-build UI gate, `10-bootstrap/scripts/tests/cross-build-ui-gate.sh`, run by
  `10-bootstrap/verify/local-smoke.sh`. It already failed at HEAD: `cross-build.sh` sources `scripts/musl-env.sh`, which that
  test's synthetic tree never copies (the B1 bounce is in flight). Only the test's synthetic-tree paths were updated.

All three reds were closed on 2026-10-19 by the Wave 2 bounce, attempt 2 (`plan/layers/50-messaging/research/coordinates.md`,
"Run — Wave 2 bounce, attempt 2"); the full gate is ALL GREEN again, G2 on both targets.

Everything else is green: clippy, musl ×2, notices + 19 tests, build-scripts gate, systemd-analyze, mesh111 workspace,
G2 ×2, presence sidecar (69 tests), UI (1147 tests + budget), intelligence check, id111 (185 + 44 + 7), KB check (0 errors),
`hooks/pre-commit`, single-node smoke (118/118).

Editing a workflow script changes the cache keys of its recorded run, so a `resumeFromRunId` replay of a run launched from
the old text re-runs the changed calls live. This now applies to all ten scripts in `plan/layers/50-messaging/runs/workflows/`.

## Moves on 2026-10-20

Why: one digit is enough. There will be no sub-layers such as `11-`, so the layer number itself names the folder:
`N-<slug>` at the repo root and under `layers/`, the same name in both places.

Only directories were renamed, with `git mv` (`git log --follow` works on every file). Paths are relative to `os/`.

| Old path | New path |
|---|---|
| `00-knowledge/` | `0-knowledge/` |
| `10-bootstrap/` | `1-bootstrap/` |
| `20-identity/` | `2-identity/` |
| `30-intelligence/` | `3-intelligence/` |
| `40-interface/` | `4-interface/` |
| `50-messaging/` | `5-messaging/` |
| `60-transport/` | `6-transport/` |
| `plan/layers/00-knowledge/` | `plan/layers/0-knowledge/` |
| `plan/layers/10-bootstrap/` | `plan/layers/1-bootstrap/` |
| `plan/layers/20-identity/` | `plan/layers/2-identity/` |
| `plan/layers/40-interface/` | `plan/layers/4-interface/` |
| `plan/layers/50-messaging/` | `plan/layers/5-messaging/` |
| `plan/layers/60-transport/` | `plan/layers/6-transport/` |

Reserved for the layers not started: `7-relay`, `8-storage`, `9-evolution` (created at kickoff). The research-report
series inside a plan folder (`research/00-code-seams.md`, `07-gap-1.md`) is a different numbering and stays two-digit.

References were rewritten the way the 2026-10-18 section describes, by one script whose dry run was read line by line.
The ten old names are unique words, so one word-bounded replace covered code, scripts, configs, docs, plan files and the
workflow scripts; the convention sentences (`NN-<slug>`, "layer number × 10") became `N-<slug>`, "the layer number".
Regenerated from their generators instead of edited: `THIRD-PARTY-NOTICES.md` (lockfile-path table, hashes of the two
hand-maintained license lists) and the seven `0-knowledge/**/llms.txt` (marker). Left as they were: the exported journals
(`*.jsonl`), the frozen test vectors, and the two move sections above (history). The sibling website repo got its own commit.

Gates on the renamed tree: `1-bootstrap/scripts/check.sh` green in every stage but clippy, which stops on two lints in the
Layer 6 code as merged (`bridge/lxmf.rs:289` `chunks_exact_to_as_chunks`, `api_v1/networks.rs:107` `result_large_err`, clippy
1.99; the rename changed one doc-comment line in each file, so they are the owner's to settle, not this move's); every other
stage ran and passed (fmt, musl checks, 1195 Rust tests, shellcheck, the hermetic gate tests, notices, the mesh111 workspace,
G2 on both targets, the presence sidecar, the UI with its budget). `3-intelligence/scripts/check.sh`, id111 tests, KB check
0 errors, `hooks/pre-commit`, single-node smoke (118 checks) green; nothing left running afterwards.

Editing a workflow script changes the cache keys of its recorded run, so a `resumeFromRunId` replay of a run launched
from the old text re-runs the changed calls live. This applies again to every script under `layers/5-messaging/runs/workflows/`
and `layers/6-transport/runs/workflows/`.
