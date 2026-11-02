# Layer 9 — Self-Healing & Evolution: plan folder

**Status** (summarised from [`INDEX.md`](../../INDEX.md) row 9, which stays the source of truth): 🔬 researching — kickoff 2026-11-02
(cloud session, branch `claude/layer-6-implementation-006mte`; the owner: "start layer 8 and 9 research in parallel now and document
all"): the research sweep (`runs/workflows/research.js`: an opus seams scout, five sonnet lens scouts, an opus completeness critic, up to
two gap-fill scouts; reports `research/00`–`07`) runs beside Layer 7's spec work and Layer 8's sweep; then the lead writes
`options.md`; **the owner decides before any Layer 9 code is written** (`INDEX.md` "Process (per layer)").

**What the layer is:** `os/README.md` §9, verbatim: "The local agent continuously observes the health of every layer the device runs.
On detecting a failure or degradation it formulates a diagnosis, generates and internally validates a corrective change, and — if
the change succeeds — packages it as a pull request. Humans remain the final gate for merging. Nodes share proven fixes with each
other through the gateways, so a repair validated on one node can heal others. Over time the same process improves the agent
itself, expands the Knowledge Layer, and refines bridging logic — always scoped to the layers the device actually operates." Runs
natively on Cat 2B, 4A, 4B; partial or delegated on 2A and 3; not on Cat 1. Operating principle: "Autonomy is progressive and always
human-supervised at the final step."

**What it inherits (recorded hand-offs):** Layer 1 — the tokio supervision tree with restart-in-place and capped back-off ("the
Layer 9 seam"), the Android cronie `/health` check ("placeholder seam until Layer 9"), APK refresh as a Layer 9 maintenance task, the
update staging / probation / rollback machinery (`update_guard.rs`, `probation.rs`) · Layer 3 — the task API `POST /v1/agent/task
{kind: diagnose | propose | patch}` → a proposal document under `/var/lib/111/agent/proposals/`; `patch` needs a hub (opencode);
"the behaviour of self-healing — monitor cadence, diagnosis playbooks, sandbox validation, PR packaging — is Layer 9"; health
tri-state and the ≤ 10 s child restart · Layer 4 — proposals and `decisions/` (`POST /api/v1/proposals/{id}/decision`; "applying
accepted proposals is Layer 9's job"), the Repairs L9 page stub, "every restart is a proposal" (D26) · Layer 5 — the `PATCH` class,
`AGENT_OP`, `RECEIPT_STRONG`, "the apply→verify→undo engine (P4)" (§19, §21) · Layer 6 — the narrow applier cut for device actions
(apply → verify → revert; `devices/`), components enable/disable with consent, every restart a proposal · Layer 7 (D30) — fixes travel
only as sealed frames through gateways; the hub as update origin and `111 update --from`; the epoch's later consumers (cert
delegation) · parked P4 (`plan/cross-layer/parked.md`): "apply, verify, undo is smart!" — the owner wants it, generalised here.

**Rules that bound it:** the Raspberry Pi floor (`AGENTS.md`: "continuously observes" must not become a poll in the required
configuration without the owner's say — event sources first, numbers for anything periodic) · humans are the final gate (no code
from a peer is applied without a human; no fix is merged by a node) · the agent is "servant and guardian" with fixed, shell-free,
schema-validated tools (D10) · no telemetry, nothing leaves the node unsealed · adopt, don't reinvent (runbooks, probes, transactional
apply patterns from proven projects) · D22 licensing · Reticulum stays a separate program · no hardware yet.

**Decisions:** none yet. The kickoff is recorded in `DECISIONS.md` once the owner answers the options brief.

## Files

| File | What |
|---|---|
| [`research/coordinates.md`](research/coordinates.md) | Resume log: run ids, script paths, lens table, box and network notes. |
| `research/00-code-seams.md` | As-built seam map: supervision and health, the agent task API and proposals, decisions, the devices applier, components, the update probation and rollback, logs and events, the agent's tools, the timer baseline (written by the sweep). |
| `research/01-observe-diagnose.md` | Lens 1: observing every layer without a poll (event sources), incident records, diagnosis playbooks as KB recipes, local model vs hub offload for diagnosis, prior art, privacy. |
| `research/02-change-validate.md` | Lens 2: the corrective-change vocabulary (config, pins, device actions, recipes, code), sandboxed validation, the apply→verify→undo engine generalised, rollback semantics, what may auto-apply. |
| `research/03-share-fixes-pr.md` | Lens 3: packaging a validated change as a pull request with humans as the gate (no git on the node), signed `PATCH` frames through gateways, the trust model for shared fixes, supply-chain safety. |
| `research/04-evolution-kb-agent.md` | Lens 4: the agent improving itself, the KB and the bridging logic: KB authoring with validation and provenance, prompt/tool evolution with guardrails, evaluation harnesses, scoping to the layers a device runs. |
| `research/05-ui-consent-floor.md` | Lens 5: the Repairs page, incident timeline, consent levels (observe / propose / auto-apply reversible / never code), Android limits, the Pi-floor weight table of every candidate program. |
| `research/06-critic.md` | Completeness critique of 00 to 05, with the unverified-claims list. |
| `research/07-gap-N.md` | Gap-fill reports (cap 2), if the critic found blocking gaps. |
| `options.md` | The options brief for the owner (written by the lead after the sweep). |
| [`runs/workflows/`](runs/workflows/) | Workflow scripts, one per run (`research.js` first); run ids in `research/coordinates.md`. |
