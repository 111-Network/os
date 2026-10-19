# Layer 6 — Transport & Bridge: plan folder

**Status** (copied from [`INDEX.md`](../../INDEX.md) row 6, which stays the source of truth): ✍️ speccing — spec draft v0.1 `os/60-transport/README.md` (2026-10-19, 19 sections, sonnet-drafted from D25 + the options brief + research 00–07, every `file:line` re-read) under a 3-judge fable panel (bridge semantics and relay safety · carriers, components and licensing · integration, buildability and verification honesty); triage → amendments → lock (D26); build after the Layer 5 lock. Kickoff D23, decisions D25; no hardware yet

**Spec and build:** [`os/60-transport/README.md`](../../../60-transport/README.md) (draft v0.1, under review) · `os/60-transport/TASKS.md` is written after the lock.

**Opener brief:** [`cross-layer/devices.md`](../../cross-layer/devices.md) (device-connection core, D17 decisions, §9 Layer 5 hand-offs). Parked ideas picked up here: P1 Meshtastic bridge, P2 phone apps ([`cross-layer/parked.md`](../../cross-layer/parked.md)).

**Decisions:** D17 (device decisions), D19 c (LoRa ownership: 111 modem = MeshCore KISS, `owner = 111d | rnsd`), D20 (Layer 5 hand-offs), D22 (GPL-3.0-or-later: changes the linking boundary), **D23** (this kickoff), **D25** (design decisions from the options brief: store, crate, on-demand carriers, bridge-first R-B, relay defaults) in [`DECISIONS.md`](../../DECISIONS.md). Build protocol: [`EXECUTION.md`](../../EXECUTION.md).

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
| [`runs/workflows/`](runs/workflows/) | Workflow scripts, one per run: `research.js` first. |
| `review/` | Fable panel on draft v0.1: `bridge.md` · `carriers.md` · `integration.md`, then the lead's `triage.md` (written as the panel returns). |
| [`runs/journals/`](runs/journals/) | Exported run journals (`<label>.<wf_id>.jsonl`): `research.wf_7407f773-81c.jsonl` (6 scouts, critic, gap 1); see its README. |

Workflow scripts are launched by their path under `runs/workflows/`; they hard-code `ROOT = /home/kia/projects/111network/os`.
