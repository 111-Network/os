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
- **Owner decided (2026-10-19, D25):** F1 mesh111 tables · F2 official crate in-binary · F3/F4 delegated under the "separate, on demand,
  zero weight when off, users free" rule (lead picks: MeshChatX slim on-demand child; Delta Chat + Madmail first) · F5 bridge-first, R-B ·
  F6 second release · F7 three defaults accepted, `ok_to_mqtt` filter per-edge off · optional components: thin own layer, hand-selected natives
  after a full review. Next: spec `os/60-transport/README.md` (sonnet draft, lead review) → fable panel → `TASKS.md`; build after the L5 lock.
- **Spec draft v0.1 (2026-10-19):** `os/60-transport/README.md`, 867 lines, sonnet agent `spec-l6` from the lead's 19-section brief (D25 as law).
  Lead read: consistent with D25; items handed to the panel — wave order vs bridge-first (§1), the `service.restart` bearer exception (§13),
  `enable_transport = yes` on Cat 3/4 (§11, D17 #5 vs R02), the compiled-in crate vs "zero weight" (§2.1), the MeshChatX auth window (§6 rule 2),
  the one-sided Meshtastic A10 (§14), frame budgets by arithmetic (§4/§16 G2). **Panel:** three fable judges as Agent-tool subagents (read-only; no
  builds — the Layer 5 V4 judge holds the box): lenses bridge · carriers · integration → `review/{bridge,carriers,integration}.md` → lead triage.

