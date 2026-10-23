# Layer 6 — Transport & Bridge: plan folder

**Status** (copied from [`INDEX.md`](../../INDEX.md) row 6, which stays the source of truth): ✅ built — spec v1.0 locked (D27) + as-built lock **D29** (2026-10-23): the bridge core with stub, Meshtastic (our own client on meshmud's downlink), LXMF (own helper on piped stdio) and Delta Chat/Madmail adapters as zero-weight on-demand components, the transport spine (rnsd config + secrets, posture, phone relay, QR join) and the device core (uevent hotplug, consented identify, the four-action applier) in `node111`; `mesh111` store V3 + foreign fabric calls; 7 agent tools; the UI (networks rail, foreign threads, fan-out, edges and consent, `/settings/l6`); gates G1 132 · G3 147 · G4 129 · G5 99 · G7 GREEN (after fix wave B) (the Pi floor measured: 111d idle 44.7 MB, nothing resident added), the Layer 1 aggregate stages, Layers 2–3, smoke 118 / three-node 159 / panel-lint 144; two defects found by the gates and fixed before the lock (a client rnsd after a restart, a helper forgetting its peers); V5 = one fable judge on the running system → fix-then-lock → fixed (the inline applier run, resend supersession, the Backbone port, empty bodies); CI/CD + GitHub hygiene D28. Open: the owner questions and follow-ups in the board's Summary, hardware/bench gates, G2 (musl) on the owner's machine

**Spec and build:** [`os/6-transport/README.md`](../../../6-transport/README.md) (v1.0, locked D27) · [`os/6-transport/TASKS.md`](../../../6-transport/TASKS.md)

**Opener brief:** [`cross-layer/devices.md`](../../cross-layer/devices.md) (device-connection core, D17 decisions, §9 Layer 5 hand-offs). Parked ideas picked up here: P1 Meshtastic bridge, P2 phone apps ([`cross-layer/parked.md`](../../cross-layer/parked.md)).

**Decisions:** D17 (device decisions), D19 c (LoRa ownership: 111 modem = MeshCore KISS, `owner = 111d | rnsd`), D20 (Layer 5 hand-offs), D22 (GPL-3.0-or-later: changes the linking boundary), **D23** (this kickoff), **D25** (design decisions), **D26** (panel amendments: own LXMF helper, component cache, restarts as proposals, licensing posture), **D27** (the spec lock), **D28** (CI/CD and GitHub hygiene, landed beside the build), **D29** (the as-built lock) in [`DECISIONS.md`](../../DECISIONS.md). Build protocol: [`EXECUTION.md`](../../EXECUTION.md).

## Files

| Path | What |
|---|---|
| [`research/coordinates.md`](research/coordinates.md) | Resume log: run ids, script paths, lens table, hardware note. |
| `research/00-code-seams.md` | As-built seam map: where Layer 6 plugs into mesh111, node111, the UI, the agent (written by the sweep). |
| `research/01-meshtastic-bridge.md` | Lens 1: sending and receiving Meshtastic text through meshmud; bridge identity; GPL boundary after D22. |
| `research/02-lxmf-bridge.md` | Lens 2: becoming an LXMF endpoint without importing RNS; MeshChatX / lxmd paths; rnsd posture. |
| `research/03-phone-apps.md` | Lens 3: what "bridge phone apps via Termux" can honestly mean (headless peers, ADB from Termux, no path). |
| `research/04-bridge-design.md` | Lens 4: bridge semantics: categories in Messages, fan-out, relay between networks, loop guard, consent, prior art. |
| `research/05-transports-devices.md` | Lens 5: the device-connection core scoped without hardware; MeshCore KISS modem status; interfaces.toml; sequencing. |
| `research/06-critic.md` | Completeness critique of 00 to 05. |
| [`research/07-gap-1.md`](research/07-gap-1.md) | Gap 1: MeshChatX actually run as the LXMF carrier; Python floor matrix; hybrid interpreter; slim closure. |
| [`options.md`](options.md) | Options brief for the owner: 18 settled facts, 8 polled forks with picks, detail picks, plan corrections, critic digest. |
| [`research/08-spike-deltachat.md`](research/08-spike-deltachat.md) | S5 spike: Delta Chat core + Madmail in one netns (PASS), the traps, the review-citation addendum. |
| [`runs/workflows/`](runs/workflows/) | Workflow scripts, one per run (`research.js`, `wave-0*.js` … `wave-3d-fix.js`, `wave-review.js`, `plan-check.js`); the run ids are in `research/coordinates.md`. |
| [`review/`](review/) | Fable panel on v0.1: `bridge.md` · `carriers.md` · `integration.md` · the lead's `triage.md` (dispositions + owner answers) · `recheck.md` (fable re-check of v0.2) · `v5-integrated.md` (the V5 integrated review of the built layer, fix-then-lock). |
| [`runs/journals/`](runs/journals/) | Exported run journals (`<label>.<wf_id>.jsonl`): `research.wf_7407f773-81c.jsonl` (6 scouts, critic, gap 1); see its README (the build-phase runs were not exported: their ids and results are in `research/coordinates.md`). |

Workflow scripts are launched by their path under `runs/workflows/`; `research.js` hard-codes `ROOT = /home/kia/projects/111network/os` (the owner's box), the build-phase scripts `/home/user/os` and the cloud session's scratchpad.
