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
- **Panel → lock (2026-10-19, evening):** reviews `506cf65` (all amend-then-lock) → triage + six owner questions → **D26** `415aa93` → v0.2 `27487bb` →
  re-check `30af706` (amend-then-lock, 13 items) → v0.3 → **locked v1.0, D27**; board `os/60-transport/TASKS.md`. Build starts at `checkpoint-layer-5`.


## Build (2026-10-19, cloud session; branch `claude/layer-6-implementation-006mte`)

- Box: a cloud container, **4 CPUs, 15 GB RAM, a fixed disk allowance (~40 GB; a copy of the node111 target dir filled it once)**; no musl
  cross toolchains (G2 static-link stages SKIP; not a lock run); unprivileged netns work; `iproute2` installed by apt during the session;
  crates.io, PyPI and GitHub reachable through the proxy; no `rnsd` on the host. Shared warm `CARGO_TARGET_DIR`s under the session
  scratchpad, one per workspace (cargo's lock serializes agents); `CARGO_BUILD_JOBS=2`; at most two Workflow slots per run (the harness caps
  concurrency at CPUs − 2), so Rust and non-Rust tasks are interleaved in each wave's task list.
- Scripts hard-code `ROOT = /home/user/os` and the scratchpad path of this session (the Layer 5 scripts hard-code the owner's box).
- T0 scaffold: `ef04ac9`. AGENTS.md Raspberry Pi rule: `218fd81`.
- **Run — Wave 0** (`runs/workflows/wave-0.js`, S1 S2 S4 S6 S3): `wf_10d063a6-41e`. **Run — Wave 0 spike** (`wave-0-spike.js`, S5): `wf_d7fe17dd-f69`.
  Resume: `Workflow({scriptPath: "<repo>/plan/layers/60-transport/runs/workflows/<name>.js", resumeFromRunId: "<id>"})`.
- **Wave 0 landed:** S2 `12c1485` (contract) + review minors `8bccbd5`, S5 `711f5b1` (spike PASS), S1 `85cf84f` (store V3), S6 `e5073d5`,
  S3 `8a8f668`, S4 `5ae8030`. Wave 0 run complete (5 agents, ~1.27M subagent tokens, 49 min). **Run — Wave 1a** (`wave-1a.js`, B1 alone, started
  beside S4/S6/S3): `wf_55b4e5fc-8a9` (done, 29 min). **Run — S1 bounce** (`wave-0-bounce.js`, the S1 review's major + minors): `wf_8bec0a2d-af3`.
  **Reviews** (`wave-review.js`, fable judges, read-only): S1 + S2 `wf_4423192d-713`; S3 + S4 `wf_9f93907d-888`.
  GitHub access was granted mid-session (owner: the org has full access); the branch is pushed after every commit.
- **Wave 0 closed + B1:** S1 bounce `a38fe19`, B1 `fc802a7`. **Run — Wave 1** (`wave-1.js`, B2 + B3): `wf_0ea8a249-2d9`. **Run — Wave 1b** (`wave-1b.js`,
  B4 + B5): `wf_b2b6ec75-68f`. **Reviews:** S1-bounce + B1 `wf_182b25dc-d01`. Lead fixes from the S3/S4 reviews (version test, rnstatus re-probe
  back-off, `write_line` stdin poisoning) ride a lead commit after the Wave 1 launch (`19bc979`).
- **Wave 1 landing:** B5 `cfa065d` (review `wf_6dd8e7a4-3f0`). **Run — B1 bounce** (`wave-1-bounce.js`: the B1 review major + the S1-bounce/B1
  minors): `wf_f9c6bf9d-b98` (landed `da0c8a0`; node111 fabric verified on a HEAD overlay while B2/B4 held the tree). B5 review minors `edfd8bf`.
- **Wave 2 started early for the non-Rust tasks** (they depend on the landed contract and the spec only): `wave-2a.js` (C2 UI + C3py helper)
  `wf_3fd1e4f0-857`, launched beside B2/B3/B4. `wave-2.js` is now C1 alone (after B2/B4 land); `wave-2b.js` C4 + C3rs + C5 after that.
  Lesson: a backtick inside a brief closes the template literal — `node --check` accepts the result (tagged templates), the Workflow parser does not.
- **B4 landed** `39483b0` (+ fmt `b72067e`; review `wf_4e82a662-f37`). **Run — Wave 2c** (`wave-2c.js`, C4 device core alone, right after B4):
  `wf_4f4eadf8-b07`. `wave-2b.js` is now C3rs + C5 (after B2 and C3py). Ownership moves: the registry.rs binding reader/writer switch is C4's; the
  two `N/meshtastic/mod.rs` readers are C1's (B3 holds the file until it lands); C4 exposes `devices::applier::apply` and C1 calls it.
