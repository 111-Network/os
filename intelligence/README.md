# Layer 3 — Local Intelligence

The capability layer that puts a local model behind the node's agent: one served model runtime, one thin always-on agent loop, and — on hub tiers — an adopted coding-agent harness for heavy work. It runs where device capacity allows and the user agrees: natively on AI-native terminals and hubs, while lighter devices use the same agent through a nearby hub over the local mesh.

Status of this document: **spec v1 — built** (`agent111` crate; build log `TASKS.md`; decisions: `os/plan/DECISIONS.md` D10). No new research was done for this layer: every adopted component is an existing Knowledge Layer entry. The adversarial review pass on the spec is still queued; the build followed the spec as written plus the amendments listed in the appendix.

## Boundary: capability, not behavior

Layer 3 provides the *ability* to run an agent. The *behavior* of self-healing — monitor cadence, diagnosis playbooks, sandbox validation, PR packaging — is Layer 9, which consumes this layer through one task API and stays harness-agnostic. The Interface layer (4) renders chat and notifications; this layer owns the backend endpoints it calls. Keeping Layer 3 to capability only is what makes it minimal.

## Components

Three. Nothing else.

| Component | What | Source | Runs on |
|---|---|---|---|
| `llama-server` | pinned llama.cpp build serving the node's model; OpenAI-compatible HTTP on `127.0.0.1:11181` | adopted: `knowledge/projects/llama-cpp.md` | nodes with a local model (2B, 4A, 4B) |
| `111-agent` | thin Rust agent loop: chat backend, KB grounding, task API, offload client — `127.0.0.1:11182` | custom, this repo | every node running Layer 3 (2A ◐, 2B, 3 ◐, 4A, 4B) |
| `opencode` | adopted coding-agent harness, headless JSONL, invoked **by** `111-agent` for patch-class tasks | adopted: `knowledge/projects/opencode.md` | hub tiers (4A, 4B; opt-in on 2B) |

Both daemons are supervised children of `111d` — exactly the `rnsd` pattern from the bootstrap spec: `111d` spawns them, restarts them in place, and health-gates them; systemd/runit stay thin wrappers. Layer 3 adds no new supervisor.

`111-agent` is deliberately thin: a single binary, no plugin system, a fixed enumerated tool set (`kb.read`, `health.read`, `logs.read`, `proposal.write`, `task.submit`), streaming chat via llama-server. It never gets a host shell. Multi-file patch generation is delegated to opencode where present; `111-agent` decides local-loop vs delegation, so callers never care which harness answered.

## Model profiles

The standard tier — the bootstrap's post-install offer — is **Qwen3 4B, Q4_K_M GGUF** (`knowledge/models/qwen3-4b.md`): one default everywhere a local model runs. Model blobs live in `/var/lib/111/models/` behind a `current` symlink (same pattern as `kb/<sha>/`), are sha256-pinned in the signed channel manifest, and arrive via Layer 1's prefetch / self-update path. There are no runtime downloads from model registries, ever.

| Category | Local model | Agent mode |
|---|---|---|
| 1 relay | — | layer not run |
| 2A light terminal | none | `111-agent` as offload client only |
| 2B AI-native terminal | qwen3-4b Q4_K_M | local; opencode opt-in |
| 3 bridge | none | offload client only |
| 4A knowledge hub | qwen3-4b Q4_K_M | local + serves offload to neighbors; opencode |
| 4B compute hub | hub ladder | local + serves offload to neighbors; opencode |

Hub ladder (4B), one config table with D10 defaults: <8 GB acceleratable RAM/VRAM → qwen3-4b; 8–24 GB → qwen3-8b Q4; >24 GB → qwen3-32b Q4 or the 30B-A3B MoE. The family spans 0.6B–32B dense plus MoE (KB entry), so the ladder is a config choice, not a new adoption. Category 5 is an overlay role and changes nothing here.

## Grounding — the Layer 0 connection

The agent's only curated knowledge source is the local KB snapshot at `/var/lib/111/kb/<sha>/`. System context = role preamble + node category + the root `llms.txt` index (~2 KB); retrieval follows the KB's own `AGENTS.md` ritual (index first, grep by name, one file at a time) through the `kb.read` tool. With no KB snapshot present the agent says so instead of improvising project facts. The components of this layer are themselves KB adoptions — the layer literally runs on Layer 0's output.

## Contracts

- **Chat → Layer 4.** `111-agent` owns the backend: `POST /v1/chat` (grounded multi-turn chat), `GET /v1/agent/events` (activity stream for the notifications surface), `GET /v1/agent/status`. Layer 4 renders; it never talks to llama-server directly. Localhost only until Layer 4 auth exists — LAN binding follows the bootstrap dashboard's explicit opt-in rule.
- **Tasks → Layer 9.** `POST /v1/agent/task` `{kind: diagnose | propose | patch, refs…}` → task id; progress rides the events stream; the result is a proposal document under `/var/lib/111/agent/proposals/`. `patch` requires a hub (opencode); elsewhere it refuses with `needs hub` — the bootstrap refusal philosophy: clear message, never a silent skip.
- **Offload ← Layers 5/7.** A model-less node's `111-agent` points its model base URL at a nearby hub discovered via the Service Discovery Registry (Layer 7 broadcasts hub role, model, load); traffic rides Layer 5's encrypted fabric. Layer 3 invents no mesh protocol, and a hub never proxies inference to the WAN.
- **Identity ← Layer 2.** The agent signs its outbound artifacts (proposals, patch PRs) with the node key; hub offload endpoints authenticate peers by node identity. Until Layer 2 lands, the provisional secp256k1 identity (D9) sits behind the same seam.
- **State ← Layer 8.** Conversations, task journal, and proposals live under `/var/lib/111/agent/` in the encrypted local store — capacity-pruned, never policy-expired.
- **Health ← Layer 1, → Layer 9.** Both children expose `/health` (200/503); `111d` restarts a dead child ≤10 s and owns the watchdog. Agent state is honest and distinct: `local | offloaded | unavailable`.

## Refusals & boundaries (v1)

No WAN/cloud model calls and no API keys anywhere — opencode is configured with exactly one provider: the local OpenAI-compatible endpoint · no on-node fine-tuning — model changes arrive only as signed channel-manifest swaps through the bootstrap updater (A/B + health gate) · no runtime model downloads · no host shell for the agent · no plugin system · nothing from this layer installs on Category 1 relays · no LAN exposure of any Layer 3 port without the explicit opt-in.

## Acceptance criteria (v1 build done = all true)

1. 2B node, WAN pulled: llama-server child up with qwen3-4b loaded; dashboard chat round-trips fully offline; answers about adopted projects trace to KB entries and cite them.
2. 2A node + hub on LAN: chat answered via offload; hub killed → status flips to `unavailable`, no hang, no silent error.
3. `kill -9` either child: `111d` restarts it ≤10 s; `/health` honest throughout.
4. No KB snapshot: the agent states its knowledge is absent rather than inventing project facts.
5. A `patch` task on a hub produces an opencode JSONL-traced run and a proposal document; the same task on a non-hub refuses with `needs hub`.
6. Model update via the channel manifest: A/B swap + health gate; a forced-bad model auto-reverts together with the binary.

## Deferred

Hub-ladder tuning beyond the default table · opencode as default on 2B · agent-to-agent mesh protocols (Layers 7/9) · GPU-specific llama.cpp builds beyond the CPU default · fine-grained tool ACLs · multimodal/voice interfaces.

## Appendix — as built (v1)

Crate `os/intelligence/crates/agent111`, binary `111-agent`, 93 tests, gate `scripts/check.sh` (fmt + clippy + tests + musl check). Full contracts: `TASKS.md` → Pinned contracts.

**Process shape.** `111d` (Layer 1) spawns `111-agent` and `llama-server` as supervised children when their binaries exist in `current/bin/` (`llama-server` additionally needs a `.gguf` under `/var/lib/111/models/current/`); otherwise each is honestly `absent`. `/api/status.intelligence = {agent, model_server, agent_url}`. Verified end-to-end with the real binaries on a staged node root: agent spawned, KB grounded (the release's integrity-checked `share/kb/`), honest 503 on chat without a model, task failures journaled, clean SIGTERM shutdown of the child.

**Configuration** (flags > `AGENT111_*` env > `/etc/111/agent.env` > defaults): `--port 11182`, `--model-base http://127.0.0.1:11181`, `--model-id qwen3-4b`, `--kb-dir`, `--state-dir /var/lib/111/agent`, `--category`, `--daemon-health-url`, `--opencode-path`, `--node-id`, `--lan`, `--env-file`. The model base is guarded at startup: loopback or private/mesh address only; any public IP is refused with no override (no WAN inference). `--lan` controls endpoint binding only.

**API** (loopback, Host-allowlisted, GET routes GET-only): `GET /health` (`ok|degraded|error`; 503 only when the agent itself is broken, so a dead hub never restart-loops it) · `GET /v1/agent/status` (`local|offloaded|unavailable`, model, kb, node_id, category) · `POST /v1/chat` (SSE `delta|tool_call|done|error` or JSON) · `GET /v1/chat/history` · `GET /v1/agent/events` (SSE, replay 50) · `POST /v1/agent/task` (`diagnose|propose|patch` → 202; `patch` without opencode → 409 `needs hub`).

**Tools** (fixed, shell-free): `kb.read` · `health.read` · `logs.read` · `proposal.write` · `task.submit`. Chat loop ≤4 tool rounds; turns persist atomically; KB absent ⇒ the system prompt states it and the agent says so.

**Tasks.** `diagnose`/`propose` = one tool-less model call over gathered context (daemon health, journal tail, refs resolved through the KB) → proposal stamped with the node id. `patch` = opencode headless (`run --format json --pure -m local/<model>`) in a per-task work dir with a generated `opencode.json` declaring exactly one provider (the local endpoint) and a scrubbed environment; 10-min timeout; JSONL collected into a proposal. Leftover `running` tasks are marked `failed` on restart.

**Amendments to the spec made during the build** (all recorded in `TASKS.md`): health tri-state (`degraded` is 200); env prefix `AGENT111_` (a leading digit is not a valid shell identifier); refused `patch` writes no task file; node id (not key material) flows from Layer 2 and proposal *signing* is deferred to Layer 9's PR packaging; `/v1/chat/history` added because Layer 4's UI talks only to `111d`, which relays.

**Pre-hub verification gates (not yet passed — needed before a 4A/4B profile ships):** G1 real `llama-server` + qwen3-4b GGUF under `111d` on one machine, chat round-trip offline · G2 opencode offline: on first use opencode 1.18.18 fetches its provider SDK (`@ai-sdk/openai-compatible`) from the network — the hub profile must prefetch/pin it or the `patch` path hangs on an air-gapped hub (observed: it never contacted the local endpoint without network) · G3 2A → hub offload over a real LAN with the hub's `--lan` binding · G4 model swap via the signed channel manifest (Layer 1 updater owns the mechanics; a model section in the manifest is not yet defined).
