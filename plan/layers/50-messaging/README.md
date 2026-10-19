# Layer 5 — Messaging & Data: plan folder

**Status** (copied from [`INDEX.md`](../../INDEX.md) row 5, which stays the source of truth): ✅ built — spec v1.2 locked (D20) + as-built lock **D24** (2026-10-19): `mesh111` store/envelope/fabric/`tcp-lan` (163 tests + musl G2 run), the fabric inside `111d` behind additive `/api/v1` + SSE, hosted panels with guards and offline lint, Meshtastic service (meshmud, shim, GPL presence reader in a hash-locked venv), agent tools with the loopback bearer, Messages/Contacts/Nearby/Panel/Settings UI (1201 vitest); V4 = one fable judge on the running system → fix-then-lock → fixed (Layer 2 A12 unclean-reboot verdict, honest hub reachability + §9 empty-fetch answer, children die with 111d, keep-alive timeout, manifest hashes); gates: `check.sh` ALL GREEN (834 cargo), smoke 118, three-node ×3 green, panel-lint 144; tag `checkpoint-layer-5`. Open: external hardware/CI gates (D24)

**Spec and build:** [`os/50-messaging/README.md`](../../../50-messaging/README.md) · [`os/50-messaging/TASKS.md`](../../../50-messaging/TASKS.md)

**Decisions:** D18, D19, D20, D21, **D24** (the lock) (and D17 for the device decisions it builds on) in [`DECISIONS.md`](../../DECISIONS.md). Build protocol: [`EXECUTION.md`](../../EXECUTION.md).

## Files

| Path | What |
|---|---|
| [`options.md`](options.md) | Options brief (lead-written from the 8-agent sweep): facts, detail picks, spec amendments, Layer 6 brief link. |
| [`research/coordinates.md`](research/coordinates.md) | Layer 5 resume log: research sweep, spec panel, build Waves 0 to 3 (run ids, script copies, gate milestones). |
| [`research/00-code-seams.md`](research/00-code-seams.md) | As-built extension seams: where Layer 5 plugs into built code (line-verified map). |
| [`research/01-fabric.md`](research/01-fabric.md) | Lens 1: the store-and-forward fabric between 111 nodes. |
| [`research/02-bridges-and-budgets.md`](research/02-bridges-and-budgets.md) | Lens 2: foreign meshes as bridge targets and physical budgets. |
| [`research/03-hosted-panels.md`](research/03-hosted-panels.md) | Lens 3: one dashboard hosting any project's own panel. |
| [`research/04-envelope.md`](research/04-envelope.md) | Lens 4: completing the message envelope on top of Layer 2. |
| [`research/05-data-store-and-bulk.md`](research/05-data-store-and-bulk.md) | Lens 5: encrypted store, capacity pruning, bulk payloads. |
| [`research/06-critic.md`](research/06-critic.md) | Completeness critique of reports 00 to 05. |
| [`research/07-gap-1.md`](research/07-gap-1.md) | Gap 1: who drives the LoRa radio, and what runs on a Cat-1 relay. |
| [`research/07-gap-2.md`](research/07-gap-2.md) | Gap 2: v1 delivery scope (keys, contacts, custody, "relayed"). |
| [`research/07-gap-3-meshmonitor.md`](research/07-gap-3-meshmonitor.md) | Gap 3: MeshMonitor vs alternatives for Meshtastic presence plus a hosted panel. |
| [`review/triage.md`](review/triage.md) | Lead triage of the 3-judge spec panel plus the re-check; every finding disposition. |
| [`review/crypto.md`](review/crypto.md) | Spec panel, lens: crypto / envelope / protocol. |
| [`review/panels.md`](review/panels.md) | Spec panel, lens: hosted-panel proxy security and Meshtastic integration. |
| [`review/integration.md`](review/integration.md) | Spec panel, lens: integration, scope honesty, buildability. |
| [`review/recheck.md`](review/recheck.md) | Fable re-check of spec v1.1 (crypto and panels lenses). |
| [`review/wave-1.md`](review/wave-1.md) | Wave 1 review triage with the lead's standing calls. |
| [`review/wave-2.md`](review/wave-2.md) | Wave 2 review triage (open bounce items). |
| [`review/integrated.md`](review/integrated.md) | V4: the fable integrated review of the running system (fix-then-lock → fixed, D24). |
| [`runs/workflows/`](runs/workflows/) | Workflow scripts, one per run: `research.js`, `spec-review.js`, `wave-0.js`, `wave-1.js`, `wave-1-review.js`, `wave-1-bounce.js`, `wave-2.js`, `wave-2-review.js`, `wave-2-bounce.js`, `wave-2-bounce-2.js`, `wave-3.js`, `wave-3-fix.js`, `wave-3-polish.js`, `wave-4-review.js`. |
| [`runs/journals/README.md`](runs/journals/README.md) | Index of the exported workflow journals (`*.jsonl`, one line per finished agent) and which script produced each. |

Workflow scripts are launched by their path under `runs/workflows/`. They hard-code `ROOT` and a local `JOURNAL` path; see `runs/journals/README.md` for re-pointing them on another machine.
