# Layer 4 — Interface & Dashboard: plan folder

**Status** (copied from [`INDEX.md`](../../INDEX.md) row 4, which stays the source of truth): ✅ built — spec v2 locked (D11+D15), as-built lock D16: UI (588 vitest, 147.6 KB gz) + daemon side in `node111` (314 cargo tests) + install→dashboard on Linux and phone; I3 = 3 fable judges, all findings fixed; `check.sh` ALL GREEN · `interface/verify/smoke.sh` 118 checks · `local-smoke.sh` green. Open: on-device phone gate V1, real-Android pair-link check

**Spec and build:** [`os/interface/README.md`](../../../interface/README.md) · [`os/interface/TASKS.md`](../../../interface/TASKS.md)

**Decisions:** D11, D15, D16 in [`DECISIONS.md`](../../DECISIONS.md). Build protocol: [`EXECUTION.md`](../../EXECUTION.md).

## Files

| Path | What |
|---|---|
| [`options.md`](options.md) | Options brief from the 3-scout fast sweep (stack, design extraction, dual-mode UX). |
| [`review/triage.md`](review/triage.md) | Adversarial panel triage of the v1 draft (3 fable lenses, 41 findings), folded into spec v2. |

Device-connection material for the Layer 6 opener is cross-layer: see [`../../cross-layer/devices.md`](../../cross-layer/devices.md).
