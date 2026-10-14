# Layer 5 — research coordinates

Resume coordinates for the Layer 5 (Messaging & Data) research sweep. Reports land in `layer-5-research/`.
Sessions die (quota walls, restarts); this file is what the next session needs.

## Run 1 — 5-lens sweep + critic + gap-fill (2026-10-14)

- Workflow run id: `wf_a4b7e5d6-d10`
- Script copy (resume needs a path under the repo): `plan/layer-5-research/workflow-research.js`
- Harness script path: `~/.claude/projects/-home-kia-projects-111network-os/3a9939aa-635d-4885-a9ea-6584b070c677/workflows/scripts/layer-5-research-wf_a4b7e5d6-d10.js`
- Journal: `~/.claude/projects/-home-kia-projects-111network/3a9939aa-635d-4885-a9ea-6584b070c677/subagents/workflows/wf_a4b7e5d6-d10/journal.jsonl`
- Resume: `Workflow({scriptPath: "<script copy, absolute>", resumeFromRunId: "wf_a4b7e5d6-d10"})` — keep the script byte-identical.
- Models: scouts sonnet (effort high) · critic opus (xhigh) · gap-fill sonnet. Session on the Anthropic plan (roster map in `INDEX.md`).

| Lens | Report | Question |
|---|---|---|
| 1 fabric | `01-fabric.md` | what carries 111's sealed envelopes node-to-node with store-and-forward: thin Rust core vs BPv7 (dtn7-rs) vs LXMF-via-API vs adopted P2P cores |
| 2 bridges + budgets | `02-bridges-and-budgets.md` | Meshtastic / MeshCore / LXMF / Briar / Delta Chat as bridge targets; per-transport MTU + throughput table |
| 3 hosted panels | `03-hosted-panels.md` | how one dashboard hosts other projects' UIs (HA Ingress, Sandstorm, …) + the panel inventory |
| 4 envelope | `04-envelope.md` | message ids, fragmentation, ACK proofs, groups, inner signatures, ratchet space, encoding, time, classes |
| 5 data | `05-data-store-and-bulk.md` | encrypted store, capacity pruning, bulk payloads (KB sync, updates, patches), multi-device |
| critic | `06-critic.md` | completeness verdict, gaps, conflicts |
| gap-fill | `07-gap-N.md` | one per blocking gap (cap 2) |

Next step after the run: lead writes `layer-5-options.md` (options brief), owner decides → `DECISIONS.md` → spec draft
`os/messaging/README.md` → fable adversarial panel → `TASKS.md` → build.

## Run 2 — gap-3 scout (MeshMonitor vs web client + sidecar vs loopback Web Serial), 2026-10-14

- Agent tool (sonnet), name `scout-meshmonitor`; report `layer-5-research/07-gap-3-meshmonitor.md`.
- Critic gaps handled by the lead inside `layer-5-options.md`: G4 (online modality → not v1 fabric traffic; random link
  key + attestation rule), G5 (one tiered fragmentation design sized against the built `seal.rs`), G6–G9 noted.

## Status

All 7 forks decided (D19, 2026-10-14). Spec draft: `os/messaging/README.md`. Next: fable adversarial panel (`layer-5-review/`) → fixes → lock → `os/messaging/TASKS.md` → build.

## Run 3 — adversarial panel on the spec draft (2026-10-14)

- Workflow run id: `wf_c4054cfb-f80` · script copy: `plan/layer-5-review/workflow-review.js` · 3 fable judges (xhigh):
  `crypto` (§3–§12, §18, L2 amendments) · `panels` (§13–§15 proxy + Meshtastic) · `integration` (scope, seams, buildability).
- Reviews land in `plan/layer-5-review/{crypto,panels,integration}.md`; the lead's triage goes to `plan/layer-5-review.md`.
- Resume: `Workflow({scriptPath: "<script copy, absolute>", resumeFromRunId: "wf_c4054cfb-f80"})`.

## Run 4 — build Wave 0 (2026-10-14)

- Workflow run id: `wf_3da6437a-9fa` · script copy: `messaging/.workflow-wave-0.js` · 8 agents: I1, C2, C3, C4 (opus) · C1, C5, C6, C7 (sonnet).
- Resume: `Workflow({scriptPath: "<script copy, absolute>", resumeFromRunId: "wf_3da6437a-9fa"})`. Agents never commit; the lead
  reviews each task (run its check, read the diff) and commits one per task — `messaging/TASKS.md` is the board.

## Run 5 — build Wave 1 (2026-10-14)

- Wave 0 closed at `a14736b` (C2 + C3 landed together after their bounces; identity I1, contract C1, node111 plumbing C4, docs C5,
  knowledge C6, sidecar/assets C7 committed earlier). Known interim red: `pnpm test` in `interface/ui` (12 adapter/mock tests)
  until U1 lands; bootstrap `check.sh` stops at its UI stage for the same reason.
- Workflow run id: `wf_f537132f-cdb` · script copy: `messaging/.workflow-wave-1.js` · 5 agents in parallel on disjoint files:
  F1 fabric runtime (opus, xhigh) · P1 hosted panels (opus) · M1 Meshtastic (opus) · U1 UI data layer + Messages + Contacts
  (sonnet) · A1 agent tools (opus). The F1 brief names the store API as landed after C3 (`receive_message` is the only
  production path that records a peer's `n`; `Store::offer` is test-only; `complete_partials` for the restart path) and the
  evicted-partial / SACK interaction to resolve in `frag.rs`/`outbox.rs`.
- Resume: `Workflow({scriptPath: "<script copy, absolute>", resumeFromRunId: "wf_f537132f-cdb"})`. Never message in-flight
  workflow agents (a SendMessage forks them). The lead reviews each task (check + diff), bounces via a fresh agent, commits one
  per task; then Wave 2 (F2, U2, B1), Wave 3 (V1–V4).
- **Crash + resume (19:03→19:30):** the session died ~62 min into the run; only A1 had returned (reviewed + committed
  `3c605fd`). Resumed with `resumeFromRunId` after adding a RESUMPTION NOTICE to the four unfinished briefs (partial work
  on disk; inventory, keep, finish). Lesson: the resume cache is a hash chain over the sequence of `agent()` calls — editing
  the first call's prompt re-ran every later one, including the finished A1 (started live twice, stopped both times,
  nothing on disk changed); moving A1 first changed its index, so it re-ran again. Fix that worked: delete the finished
  call from the script and take its result from `journal.jsonl`. Current run = 4 live agents F1 P1 M1 U1
  (task `wdhag90l2`). Put bounce-prone tasks LAST in a fan-out.
- **Wave 1 landed** (2026-10-14 evening): F1 `c8a093c` · P1 `79ca52d` · M1 `99f5e0a` · U1 `858077f` (board `f283c9a`). Every
  check green except the UI bundle budget (255.4 KB vs 200 KB; owner decision pending). Review run `wf_c857767c-bde`
  (script `messaging/.workflow-wave-1-review.js`): F1/P1/M1 fable xhigh, U1 opus; findings only → lead bounces via fresh agents.
- **Wave 1 review returned** (F1 fix-then-lock 2/13 · P1 ready 0/6 · M1 fix-then-lock 1/9 · U1 fix-then-lock 1/16; triage
  `plan/layer-5-review/wave-1.md` with the lead's calls: ignore = client-side fold, bearer never mints, contract text →
  F2). Bounce run `wf_62ea271c-e73` (script `messaging/.workflow-wave-1-bounce.js`): fresh agents P1 M1 U1 (opus/sonnet)
  + F1 (opus xhigh) on disjoint files; lead re-checks + commits per bounce. Wave 2 script drafted:
  `messaging/.workflow-wave-2.js` (B1, U2, F2 — F2 carries the contract additions).
