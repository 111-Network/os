# Layer 5 workflow journals (exported)

One JSON line per **finished** agent of each Layer 5 Workflow run: `{run, label, agentId, result}` where `result` is the
agent's structured return (status, files, check tail, deviations/findings, notes). Exported 2026-10-15 00:40 from the
lead machine's local transcript store so that the builders' recorded deviations and the reviewers' full findings survive
a device switch (the originals live only under `~/.claude/projects/…/subagents/workflows/<run>/journal.jsonl`).

| File | Run | Agents with a result | Script |
|---|---|---|---|
| `wave-0-build.*` | I1 C1–C7 build | 8 | [`wave-0.js`](../workflows/wave-0.js) |
| `wave-1-build.*` | A1 F1 P1 M1 U1 build (incl. the pre-crash A1) | see file | [`wave-1.js`](../workflows/wave-1.js) |
| `wave-1-review.*` | fable/opus review of F1 P1 M1 U1 | 4 | [`wave-1-review.js`](../workflows/wave-1-review.js) |
| `wave-1-bounce.*` | fix agents | 4 | [`wave-1-bounce.js`](../workflows/wave-1-bounce.js) |
| `wave-2-build.*` | B1 U2 F2 build | 3 | [`wave-2.js`](../workflows/wave-2.js) |
| `wave-2-review.*` | review of B1 U2 F2 | 3 | [`wave-2-review.js`](../workflows/wave-2-review.js) |
| `wave-2-bounce.*` | fix agents — **only U2 finished** before the crash | 1 | [`wave-2-bounce.js`](../workflows/wave-2-bounce.js) |
| `wave-3-verify.*` | V1 smoke, V2 panel lint — **only V2 (partial) finished** | 1 | [`wave-3.js`](../workflows/wave-3.js) |

The workflow scripts (`../workflows/*.js`; moved here from `os/messaging/` on 2026-10-15, see the move table in [`plan/README.md`](../../../../README.md)) point their `JOURNAL` constants at the local store; on another machine
point them at these files instead (same line format: the reviewer/fixer prompts only read `result.*`).
