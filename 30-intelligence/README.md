# Layer 3 — Local Intelligence

The capability layer that puts a local model behind the node's agent: one served model runtime, one thin always-on agent loop, and — on hub tiers — an adopted coding-agent harness for heavy work. It runs where device capacity allows and the user agrees: natively on AI-native terminals and hubs, while lighter devices use the same agent through a nearby hub over the local mesh.

Status of this document: **spec v1 — built** (`agent111` crate; build log `TASKS.md`; decisions: `os/plan/DECISIONS.md` D10). No new research was done for this layer: every adopted component is an existing Knowledge Layer entry. The adversarial review pass on the spec is still queued; the build followed the spec as written plus the amendments listed in the appendix.

## Boundary: capability, not behavior

Layer 3 provides the *ability* to run an agent. The *behavior* of self-healing — monitor cadence, diagnosis playbooks, sandbox validation, PR packaging — is Layer 9, which consumes this layer through one task API and stays harness-agnostic. The Interface layer (4) renders chat and notifications; this layer owns the backend endpoints it calls. Keeping Layer 3 to capability only is what makes it minimal.

## Components

Three. Nothing else.

| Component | What | Source | Runs on |
|---|---|---|---|
| `llama-server` | pinned llama.cpp build serving the node's model; OpenAI-compatible HTTP on `127.0.0.1:11181` | adopted: `00-knowledge/projects/llama-cpp.md` | nodes with a local model (2B, 4A, 4B) |
| `111-agent` | thin Rust agent loop: chat backend, KB grounding, task API, offload client — `127.0.0.1:11182` | custom, this repo | every node running Layer 3 (2A ◐, 2B, 3 ◐, 4A, 4B) |
| `opencode` | adopted coding-agent harness, headless JSONL, invoked **by** `111-agent` for patch-class tasks | adopted: `00-knowledge/projects/opencode.md` | hub tiers (4A, 4B; opt-in on 2B) |

Both daemons are supervised children of `111d` — exactly the `rnsd` pattern from the bootstrap spec: `111d` spawns them, restarts them in place, and health-gates them; systemd/runit stay thin wrappers. Layer 3 adds no new supervisor.

`111-agent` is deliberately thin: a single binary, no plugin system, a fixed enumerated tool set (`kb.read`, `health.read`, `logs.read`, `proposal.write`, `task.submit`), streaming chat via llama-server. It never gets a host shell. Multi-file patch generation is delegated to opencode where present; `111-agent` decides local-loop vs delegation, so callers never care which harness answered. *[The tool set grows in Layer 5 (D20): see "Layer 5 amendments" below.]*

## Model profiles

The standard tier — the bootstrap's post-install offer — is **Qwen3 4B, Q4_K_M GGUF** (`00-knowledge/models/qwen3-4b.md`): one default everywhere a local model runs. Model blobs live in `/var/lib/111/models/` behind a `current` symlink (same pattern as `kb/<sha>/`), are sha256-pinned in the signed channel manifest, and arrive via Layer 1's prefetch / self-update path. There are no runtime downloads from model registries, ever.

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
- **Offload ← Layers 5/7.** A model-less node's `111-agent` points its model base URL at a nearby hub discovered via the Service Discovery Registry (Layer 7 broadcasts hub role, model, load); traffic rides Layer 5's encrypted fabric. Layer 3 invents no mesh protocol, and a hub never proxies inference to the WAN. *[In v1 (Layer 5, D20) offload stays on the LAN binding + pairing auth — not v1 fabric traffic.]*
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

Crate `os/30-intelligence/crates/agent111`, binary `111-agent`, 93 tests, gate `scripts/check.sh` (fmt + clippy + tests + musl check). Full contracts: `TASKS.md` → Pinned contracts.

**Process shape.** `111d` (Layer 1) spawns `111-agent` and `llama-server` as supervised children when their binaries exist in `current/bin/` (`llama-server` additionally needs a `.gguf` under `/var/lib/111/models/current/`); otherwise each is honestly `absent`. `/api/status.intelligence = {agent, model_server, agent_url}`. Verified end-to-end with the real binaries on a staged node root: agent spawned, KB grounded (the release's integrity-checked `share/kb/`), honest 503 on chat without a model, task failures journaled, clean SIGTERM shutdown of the child.

**Configuration** (flags > `AGENT111_*` env > `/etc/111/agent.env` > defaults): `--port 11182`, `--model-base http://127.0.0.1:11181`, `--model-id qwen3-4b`, `--kb-dir`, `--state-dir /var/lib/111/agent`, `--category`, `--daemon-health-url`, `--opencode-path`, `--node-id`, `--lan`, `--env-file`. The model base is guarded at startup: loopback or private/mesh address only; any public IP is refused with no override (no WAN inference). `--lan` controls endpoint binding only.

**API** (loopback, Host-allowlisted, GET routes GET-only): `GET /health` (`ok|degraded|error`; 503 only when the agent itself is broken, so a dead hub never restart-loops it) · `GET /v1/agent/status` (`local|offloaded|unavailable`, model, kb, node_id, category) · `POST /v1/chat` (SSE `delta|tool_call|done|error` or JSON) · `GET /v1/chat/history` · `GET /v1/agent/events` (SSE, replay 50) · `POST /v1/agent/task` (`diagnose|propose|patch` → 202; `patch` without opencode → 409 `needs hub`).

**Tools** (fixed, shell-free): `kb.read` · `health.read` · `logs.read` · `proposal.write` · `task.submit`. Chat loop ≤4 tool rounds; turns persist atomically; KB absent ⇒ the system prompt states it and the agent says so. *[Layer 5 adds `messages.read`, `messages.send`, `contacts.read`, `fabric.read`, `nearby.read` — see "Layer 5 amendments" below.]*

**Tasks.** `diagnose`/`propose` = one tool-less model call over gathered context (daemon health, journal tail, refs resolved through the KB) → proposal stamped with the node id. `patch` = opencode headless (`run --format json --pure -m local/<model>`) in a per-task work dir with a generated `opencode.json` declaring exactly one provider (the local endpoint) and a scrubbed environment; 10-min timeout; JSONL collected into a proposal. Leftover `running` tasks are marked `failed` on restart.

**Amendments to the spec made during the build** (all recorded in `TASKS.md`): health tri-state (`degraded` is 200); env prefix `AGENT111_` (a leading digit is not a valid shell identifier); refused `patch` writes no task file; node id (not key material) flows from Layer 2 and proposal *signing* is deferred to Layer 9's PR packaging; `/v1/chat/history` added because Layer 4's UI talks only to `111d`, which relays.

**Pre-hub verification gates (not yet passed — needed before a 4A/4B profile ships):** G1 real `llama-server` + qwen3-4b GGUF under `111d` on one machine, chat round-trip offline · G2 opencode offline: on first use opencode 1.18.18 fetches its provider SDK (`@ai-sdk/openai-compatible`) from the network — the hub profile must prefetch/pin it or the `patch` path hangs on an air-gapped hub (observed: it never contacted the local endpoint without network) · G3 2A → hub offload over a real LAN with the hub's `--lan` binding · G4 model swap via the signed channel manifest (Layer 1 updater owns the mechanics; a model section in the manifest is not yet defined).

## Layer 5 amendments (D20, 2026-10-14)

Additive (`os/50-messaging/README.md` §17, §19; built by Layer 5 task A1). The tool set stays fixed, shell-free and
schema-validated (D10); `111-agent` still never gets a host shell.

**Tools added** (the Layer 3 set grows by five):

| Tool | Kind | Shape | Gate |
|---|---|---|---|
| `messages.read` | read | `{peer?, since_seq?, limit}` → threads/messages with states | none |
| `messages.send` | write intent | `{to, text}` → **proposal** `kind: message`; the human accepts in the UI, `111d` sends | human accept; never autonomous |
| `contacts.read` | read | → contacts + requests | none |
| `fabric.read` | read | → links, hub, custody, store (same as `GET /api/v1/fabric`) | none |
| `nearby.read` | read | → Meshtastic presence (messaging §14); names inside a labelled **untrusted block** | none |

- **Daemon access.** The tools run inside `111-agent` and reach `111d`'s loopback `/api/v1` with a bearer token
  (`Authorization: Bearer <AGENT111_DAEMON_TOKEN>`) — a per-spawn token `111d` mints into the child's environment beside the
  existing `AGENT111_*`; `111d` accepts it from a loopback peer only (`os/40-interface/README.md` §11.1). Config gains
  `daemon_api_url` (the loopback `/api/v1` base) and reads the token from the environment, with the existing precedence (flags >
  `AGENT111_*` env > `/etc/111/agent.env` > defaults); the exact flag and key spellings are fixed at build in task A1.
- **Writes** only via the proposal path: this layer's "only write path" rule is preserved, and `messages.send` never sends on its own.
- **Untrusted text.** Names heard over Meshtastic are RF-origin strings, handed to the model inside a labelled untrusted block.
- **Offload** is unchanged in v1: it stays on the LAN binding + pairing auth and is not v1 fabric traffic.

## Appendix — Layer 6 tools (D27, 2026-10-19)

Additive (`os/60-transport/README.md` §13, §15.6, §8, §12; built by Layer 6 task B5). The tool set stays fixed,
shell-free and schema-validated (D10) and grows from ten to **seventeen**: seven new tools, and `messages.send` gains
`targets`. Schemas follow the §13 order after the Layer 5 five: `bridge.read` · `bridge.propose_edge` · `devices.read` ·
`transport.read` · `device.identify` · `device.propose` · `service.restart`.

**The agent writes nothing.** Every daemon call the tools make is a GET with the per-spawn bearer (one call, no retry, no
redirect, size-capped before parsing); every write is a **proposal** the human accepts in the UI. The agent's bearer is
refused on every non-GET (the global write gate, D26 #2 — transport §13), so even a bug here could not reach a send,
an edge, a restart or a probe. A test fails if any request other than a GET ever leaves the agent for the daemon URL.

| Tool | Kind | Daemon reads (GET) | Result |
|---|---|---|---|
| `bridge.read {network?, conv?, since_seq?, limit 1..50}` | read | `/networks` always; `/networks/{net}/conversations` with `network`; `/networks/{net}/conversations/{conv}?limit&since_seq` with `conv`; `/bridge/edges` | networks, conversations, messages, edges (see the untrusted-block rule) |
| `messages.send {to, text}` | write intent | `/contacts/{to}` | proposal `message` (Layer 5, unchanged) |
| `messages.send {targets: [{network, conv}] 1..16, text}` | write intent | `/networks/{net}/conversations/{conv}?limit=1` per target | proposal `bridge_message` |
| `bridge.propose_edge {from, to, why}` | write intent | `/networks`, then `/networks/{net}/conversations` of each enabled network until both ends are found | proposal `bridge_edge` |
| `devices.read {id?}` | read | `/devices` or `/devices/{id}` | records with raw facts ≤ 4 KB each |
| `transport.read {}` | read | `/transport` | the cached `rnstatus` interface facts, posture, region, order |
| `device.identify {id}` | consent | `/devices/{id}` | the remembered probe result, or proposal `device` |
| `device.propose {id?, actions 1..8, why}` | write intent | `/devices/{id}` for each device named | proposal `device` |
| `service.restart {service_id, why}` | write intent | none | proposal `device` |

**Ids.** `network` is from the closed set `meshtastic`, `lxmf`, `app:deltachat`, `app:simplex` (`111` is not one: its
threads are the `messages.*` tools; the colon is percent-encoded in a path). A conversation is `^fc-[0-9]{1,19}$`, a
device id `^[A-Za-z0-9_-]{4,64}$`, a service id `^[a-z0-9-]{1,32}$`. Bad ids are refused before any request.

**Untrusted-block rule (bridge.read).** Everything another person or network wrote reaches the model **only inside one
block**: `{"untrusted": {"label": "UNTRUSTED: received from other networks", "origin", "warning", "networks",
"conversations", "conversation", "messages", "edges"}}`. Names, ids, native ids, titles, accounts, sender names, message
bodies, adapter wording (`label`, `copy`, `reason`, `evidence`, recipient syntax) and edge records are all in it, scrubbed like
`messages.read` — control and bidi characters dropped, bodies via `clean_text` at 4096 characters, names and ids via
`clean_line` at 64 (adapter sentences at 512), nesting bounded — and **not** with `nearby.read`'s 256. Every body is
`untrusted_text`, own rows included (an own row can be a relayed copy of someone else's words). Outside the block sit
only adapter *facts*, filtered to the contract's closed vocabularies, numbers and booleans: per network `id`, `state`,
`reason_code`, `caps` (kinds, max text bytes, evidence, auth levels, splittable, airtime numbers), `gate`, `counts`,
`dropped`; plus the echoed `network`/`conv`, `more` and the totals. `devices.read` and `transport.read` clean names and
bound raw facts (each device's `identity` and `detail` ≤ 4 KB of JSON, whole entries dropped, never cut) and carry an
`untrusted_notice`. Foreign text never reaches a send or an edge without the human reading it first: the agent can put it
into a send only through a proposal whose text part the UI renders preformatted.

**Proposal bodies — verbatim shapes (111d's C1 parses exactly these on accept; `40-interface/api/openapi.yaml`
`Proposal.kind`).** Every proposal is stamped with this node's id and journaled as `proposal.written` with its kind.

| `kind` | Body, byte for byte |
|---|---|
| `bridge_message` | one line `to: <fc-id>` per target, in the order given · a blank line · the text **verbatim** (`check_send_text`: non-blank, ≤ 4096 bytes, no control characters but newline and tab, refused never truncated). Each target is checked with `GET /networks/{net}/conversations/{conv}`; a 404 writes nothing and says why. |
| `bridge_edge` | `from: <fc-id>` · `to: <fc-id>` · a blank line · `why` (≤ 512 characters, cleaned). Both conversations must exist and be `channel` or `group` kinds and differ; a DM endpoint is refused with the §8 reason and no proposal is written. |
| `device` | **one compact JSON object** `{"why": <why>, "actions": [<Action>, …]}`; an Action is `{"action": name, …args}`. |

`device` actions are the **closed** vocabulary (devices brief §3, transport §12): `rns.interface.upsert {device_id, type,
params}` (params: a flat object of strings, numbers and booleans) · `rns.interface.remove {name}` · `rns.posture.set
{share_instance?, enable_transport?}` (at least one) · `radio.region.set {region}` · `identify {device_id}` ·
`service.restart {service_id}`. The rest of the vocabulary (`meshtastic.set`, `ble.pair`, `ble.forget`, `firmware.flash`)
and anything unknown is refused here as `unsupported_action` — the agent never files what 111d would refuse. Each action's
keys are written in a fixed order (`action` first, then the fields in the order above) and nested objects with sorted keys,
so a body is byte-stable. `device.identify` writes the single action `{"action":"identify","device_id":"<id>"}` with
`why` = `identify this radio once (a consented probe)`: accepting it **is** the per-device consent (opening a serial port
can reset a board); a device whose record already carries `firmware.source == "probe"` returns the remembered result and
writes nothing. `service.restart` writes the single action `service.restart` and makes **no** daemon call at all: a restart
route is never requested, and 111d answers `not_supervised` on accept for an unsupervised child.

Count pins: `tests/daemon_tools.rs` (schema count 17, new names in order) and `tests/chat_loop.rs` (17 tools offered).
New tests: `tests/layer6_tools.rs` (a stub 111d serving the new GET routes). No dependency, resident process or timer is
added: the tools are plain functions that run only inside a chat turn.
