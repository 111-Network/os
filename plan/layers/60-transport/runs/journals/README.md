# Layer 6 workflow journals (exported)

One JSON line per **finished** agent of each Layer 6 Workflow run: `{run, label, agentId, result}` where `result` is the
agent's structured return. Exported from the lead machine's local transcript store (the originals live only under
`~/.claude/projects/…/subagents/workflows/<run>/journal.jsonl`) so the scouts' findings survive a device switch.

| File | Run | Agents with a result | Script |
|---|---|---|---|
| `research.*` | research sweep: 6 scouts (seams, meshtastic, lxmf, phones, bridge, transports), critic, gap 1 (MeshChatX run); exported 2026-10-19 | 8 | [`research.js`](../workflows/research.js) |

The reports those agents wrote are the files under [`../../research/`](../../research/); the journal keeps each agent's
structured return (status, files, evidence level, deviations) beside them.
