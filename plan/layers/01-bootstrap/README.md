# Layer 1 — Bootstrap: plan folder

**Status** (copied from [`INDEX.md`](../../INDEX.md) row 1, which stays the source of truth): ✅ built — spec v2.2 (D9 + D12 + D14d identity-seam amendment), all 17 tasks done: ~90 cargo tests + 21 shim cases + namespace e2e green; `verify/local-smoke.sh` ALL LOCAL GATES GREEN; external HW gates (V1–V3, V8, V11) in runbooks before first signed release

**Spec and build:** [`os/10-bootstrap/README.md`](../../../10-bootstrap/README.md) · [`os/10-bootstrap/TASKS.md`](../../../10-bootstrap/TASKS.md)

**Decisions:** D9, D12 in [`DECISIONS.md`](../../DECISIONS.md). Build protocol: [`EXECUTION.md`](../../EXECUTION.md).

## Files

| Path | What |
|---|---|
| [`options.md`](options.md) | Options brief from the 7-scout sweep plus completeness critic (24 gaps, all resolved). |
| [`review/triage.md`](review/triage.md) | Adversarial panel triage: 5 judges, 99 raw findings (7 blocker), disposition folded into spec v2. |

There is no research folder and no run log: the sweep (workflow `wf_bd54415f-7fa`) was condensed straight into `options.md`.
