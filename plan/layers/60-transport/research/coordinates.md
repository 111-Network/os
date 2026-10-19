# Layer 6 — research coordinates

Resume coordinates for the Layer 6 (Transport & Bridge) work. Reports land in `layers/60-transport/research/`.
Sessions die; this file is what the next session needs.

## Kickoff (2026-10-19)

- Owner answers at kickoff (recorded as D23): research Layer 6 now and close Layer 5 alongside, updating the plan folder as
  work lands · the bridge carries readable text both ways; foreign networks are shown as their own categories in the
  Messages view; the other services' own UIs stay fully available as hosted panels · v1 networks: Meshtastic, Reticulum/LXMF,
  phone apps via Termux (P2); the MeshCore network was not selected as a bridge target (the 111 KISS modem of D19 c stays a
  transport item) · **no LoRa hardware available yet — the owner will set up a full test setup later to test it fully**;
  until then every radio path is built against stubs in netns and the hardware gates stay in the runbooks.
- Tree at kickoff: `7a1d142` (relicense D22). Layer 5 is 🔨 with the Wave 2 bounce cut off (WIP commit `6b4f0e5`), V1/V2
  partial, V3/V4 not run; `10-bootstrap/scripts/check.sh` is red at its first stage (cargo fmt on four F2-bounce files).
- Box: 12 GB RAM, 16 CPUs (WSL2). At most two cargo-building agents at a time; research scouts do not compile.

## Run 1 — research sweep (2026-10-19)

- Script: `plan/layers/60-transport/runs/workflows/research.js` (launched by `scriptPath`; keep it byte-identical to resume).
- Workflow run id: `wf_7407f773-81c` (launched 2026-10-19 01:20 CEST; harness transcript dir `~/.claude/projects/-home-kia-projects-111network-os/af41ab70-058e-4e3c-bc5e-eec7688f5c7a/subagents/workflows/wf_7407f773-81c/`). Resume: `Workflow({scriptPath: "<repo>/plan/layers/60-transport/runs/workflows/research.js", resumeFromRunId: "wf_7407f773-81c"})`.
- Models: seams scout opus (high) · lens scouts sonnet (high) · critic opus (xhigh) · gap-fill sonnet (high).

| Lens | Report | Question |
|---|---|---|
| 0 seams | `00-code-seams.md` | line-verified map of where Layer 6 plugs into the as-built mesh111 / node111 / UI / agent |
| 1 meshtastic | `01-meshtastic-bridge.md` | send + receive Meshtastic text and DMs through meshmud; bridge identity; what D22 (GPL) changes |
| 2 lxmf | `02-lxmf-bridge.md` | an LXMF endpoint for our node without importing RNS: MeshChatX API, lxmd, sidecar options; rnsd posture |
| 3 phone apps | `03-phone-apps.md` | what bridging phone apps via Termux can honestly mean: headless peers, ADB from Termux, no path |
| 4 bridge design | `04-bridge-design.md` | categories in Messages, fan-out, inter-network relay, loop guard, consent, prior art, contract |
| 5 transports | `05-transports-devices.md` | device core scoped without hardware; MeshCore KISS modem status; interfaces.toml; sequencing |
| critic | `06-critic.md` | completeness verdict, gaps, conflicts |
| gap-fill | `07-gap-N.md` | one per blocking gap (cap 2) |

Next after the run: lead writes `layers/60-transport/options.md` → owner decides → `DECISIONS.md` → spec `os/60-transport/README.md`
→ fable adversarial panel → `TASKS.md` → build (after the Layer 5 lock).
- **Run 1 complete (2026-10-19 ~03:40 CEST):** 8 agents (seams opus · 5 lens scouts sonnet · critic opus · gap-1 sonnet), ~2.0M subagent
  tokens, 895 tool calls, 95 min. Reports `00`–`07-gap-1` committed. Critic verdict: sufficient after G1; G1 filled by running MeshChatX in two
  netns. Options brief written and finalized (`options.md`); INDEX row 6 → 🧭 deciding. Next: owner decisions → `DECISIONS.md` D25 (D24 is
  the Layer 5 lock) → spec `os/60-transport/README.md` → fable panel → `TASKS.md` → build after the Layer 5 lock.
