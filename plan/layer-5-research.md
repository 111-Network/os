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
