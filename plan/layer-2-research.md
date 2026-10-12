# Layer 2 (Identity) — research run state

**✅ COMPLETE 2026-10-12 — all 8 dimensions returned. Condensed into `layer-2-options.md`; this file kept as run history.**

Durable resume coordinates for the Layer 2 research sweep (D3: plan state lives on disk, never only in chat).

## Scope lock (user, 2026-10-12)

- **Node/machine identity only** — no human-facing identity (no contact model, trust/verification UX, profiles; that is a later layer's job).
- **Lifecycle in scope:** generate, store, use (sign traffic, authenticate peers), **backup + recovery** (wallet-style, seed-phrase-class restore designed in). Rotation + compromised-key revocation **deferred** — but the design must not paint us into a corner.
- **Crypto primitive open — picked on merit.** NOT Nostr-locked; Nostr itself is not final for this project (README's "keypair can anchor Nostr identity" is a soft nice-to-have). Must be scalable, global, registry-free.
- **Agentic-native:** the node's AI agent is a first-class key user — research delegation/forward-compat hooks.
- Carried from README + KB rubric: runs on all 6 device categories incl. ESP32-class relays; air-gapped core; third-party audit required for `proven` on identity/keys/crypto.

## Workflow run

- **Run ID:** `wf_73d7f290-f74` (8 × k3-256k agents, one phase "Research")
- **Dimensions:** identity-systems · embedded-crypto · backup-recovery · wire-auth · agentic-identity · nostr-axis · mesh-case-studies · encodings-standards
- **Journal (per-agent results):** `/home/kia/.claude/projects/-home-kia-projects-111network/094a2ab1-1cc4-4f3e-a1b6-62a8a238da64/subagents/workflows/wf_73d7f290-f74/journal.jsonl`
- **Script (original):** `/home/kia/.claude/projects/-home-kia-projects-111network-os/094a2ab1-1cc4-4f3e-a1b6-62a8a238da64/workflows/scripts/layer-2-identity-research-wf_73d7f290-f74.js`
- **Script (session-readable copy):** `/home/kia/projects/111network/.workflow-resume-layer-2.js` — byte-identical; keep until Layer 2 brief is written, then delete.

## History

- 2026-10-12 ~05:30 CEST: first launch. `wire-auth` completed (431k subagent tokens, 148 tool uses total); other 7 agents died on the k3 5-hour quota limit (403).
- 2026-10-12 12:55 CEST: quota refreshed; resumed via `Workflow({scriptPath, resumeFromRunId})`, unchanged script → wire-auth replayed from cache, 7 failed agents re-ran live with identical prompts.
- 2026-10-12 ~13:20 CEST: **run complete, 8/8** (496k tokens, 217 tool uses this run). Findings extracted from the journal, cross-checked (discrepancy resolutions in the brief §Cross-check), condensed into `layer-2-options.md`. Temp script copy `.workflow-resume-layer-2.js` deleted per cleanup note.

## If interrupted again (quota / session loss)

1. Check `journal.jsonl` — one `{"type":"result",...}` line per completed agent; count them.
2. Resume: `Workflow({ scriptPath: "/home/kia/projects/111network/.workflow-resume-layer-2.js", resumeFromRunId: "wf_73d7f290-f74" })` — completed agents replay from cache. **Do not edit the script** (edits invalidate the cache for changed calls).
3. If the script copy is missing, recreate it by copying the original script path above.

## When all 8 dimensions have findings

1. Read full results from `journal.jsonl` (the wire-auth result was truncated in the completion notice; also note its safety-classifier-unavailable caveat — verify key claims when writing the brief).
2. Write `os/plan/layer-2-options.md` following the shape of `os/plan/layer-0-options.md` (already-decided section → research convergence → 2–4-option decision tables with recommendation → deferred list).
3. Update `os/plan/INDEX.md`: Layer 2 → 🧭 deciding. Commit in `os/` (no attribution lines, ever).
4. Present decisions to the user — they decide. Then: fable/opus adversarial panel → spec at `os/identity/README.md` → TASKS.md → build per `EXECUTION.md`.
