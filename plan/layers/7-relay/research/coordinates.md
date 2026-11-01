# Layer 7 — research coordinates

Resume coordinates for the Layer 7 (Relay & Presence) work. Reports land in `layers/7-relay/research/`.
Sessions die; this file is what the next session needs.

## Kickoff (2026-11-01, cloud session)

- Owner's words (2026-11-01): "continue what's left to build … send cheaper sub agents and check their work". Layer 6 is locked
  (D29); Layers 7–9 are pending; `EXECUTION.md` starts a layer with research → options brief → the owner's decision. So the kickoff
  is the research sweep and the brief; **no Layer 7 code before the owner decides.**
- Tree at kickoff: `01b4a72` (the CI fix on PR #2) on `claude/layer-6-implementation-006mte`, which is `origin/main` (`322a66d`)
  plus the Layer 6 build; PR #2 is open and unmerged on GitHub at kickoff (the branch is the one most ahead).
- Box: the cloud container (4 CPUs, 15 GB RAM, a fixed disk allowance; two Workflow slots per run). **Network policy:** the proxy
  allows package indexes and `raw.githubusercontent.com`; it denies most documentation sites (`meshtastic.org`,
  `reticulum.network`, `docs.rs`, `rfc-editor.org`, GitHub HTML pages, `api.github.com`). The scouts are told so; claims they
  could not verify are collected by the critic in `06-critic.md` for the lead to verify on the owner's machine.

## Run 1 — research sweep (2026-11-01)

- Script: `plan/layers/7-relay/runs/workflows/research.js` (launched by `scriptPath`; keep it byte-identical to resume).
- Workflow run id: `wf_60f165f1-80f` (launched 2026-11-01 ~21:55 UTC from the tree `bdea810`; harness transcript dir `~/.claude/projects/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/subagents/workflows/wf_60f165f1-80f/`). Resume: `Workflow({scriptPath: "<repo>/plan/layers/7-relay/runs/workflows/research.js", resumeFromRunId: "wf_60f165f1-80f"})`.
- Models: seams scout opus (high) · lens scouts sonnet (high) · critic opus (xhigh) · gap-fill sonnet (high).

| Lens | Report | Question |
|---|---|---|
| 0 seams | `00-code-seams.md` | line-verified map of where Layer 7 plugs into the as-built mesh111 / node111 / UI / agent; the timer baseline of the required configuration |
| 1 presence | `01-presence-discovery.md` | presence and discovery per transport; heartbeats vs event-driven (airtime arithmetic); the Service Discovery Registry; the mesh-epoch source |
| 2 gateways | `02-gateways-global.md` | what "the global network" honestly is; store-and-forward carriers for sealed 111 envelopes between islands; consent, privacy, legal |
| 3 relay | `03-fabric-relay.md` | multi-hop and cross-hub delivery inside the fabric: Reticulum as router, anti-entropy, IP mesh routing, hub-centric star; mesh111 amendments |
| 4 kb-updates | `04-kb-updates.md` | signed releases and the KB from a hub with no internet; manifest gossip vs hub-as-origin; trust model |
| 5 ui-floor | `05-ui-consent-floor.md` | Nearby radar/map offline, presence UI, consent; the Pi-floor weight table of every candidate |
| critic | `06-critic.md` | completeness verdict, gaps, conflicts, owner forks, the unverified-claims list |
| gap-fill | `07-gap-N.md` | one per blocking gap (cap 2) |

Next after the run: lead writes `layers/7-relay/options.md` → owner decides → `DECISIONS.md` → spec `os/7-relay/README.md`
→ fable adversarial panel → `TASKS.md` → build.
