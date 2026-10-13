# Layer 3 — Build tasks

Execution per `os/plan/EXECUTION.md`. Spec: `os/intelligence/README.md` (spec v1, D10).
Subagents write files only within their row's "Files owned"; the reviewer (main session) runs checks, reads diffs, bounces failures with specific fixes, and commits each verified task. No attribution lines in commits.
If a task exposes a spec contradiction or a pinned-contract gap: STOP, report to reviewer — never improvise a contract change.

Legend: ⬜ todo · 🔧 in flight · 🔁 bounced · ✅ verified + committed

## Pinned contracts (bind every task; additive-only after T0)

**Crate:** workspace at `os/intelligence/`; single crate `crates/agent111` (package `agent111`, `[[bin]] name = "111-agent"`). Host target for dev; musl-static x86_64/aarch64 is the release shape (`cargo check --target x86_64-unknown-linux-musl` must stay green).

**Dependency policy:** only the deps scaffolded in T0's `Cargo.toml`; `Cargo.lock` committed; new deps require reviewer sign-off recorded in the task's status note. Approved set: tokio · axum · serde/serde_json · anyhow · time · ureq (plain HTTP only, `default-features = false` + `json` — no TLS: endpoints are loopback/private fabric, encryption is Layer 5's job) · tokio-stream (reviewer sign-off T0: axum SSE responses need a `Stream`; standard minimal companion) · tempfile (dev-only). Test servers use std `TcpListener` helpers in `tests/common/` — no HTTP-mock crates.

**Module stubs** (created in T0, tasks fill them): `config` · `health` · `model` · `kb` · `tools` · `chat` · `tasks` · `events` · `status` · `store` · `http`.

**Cross-module APIs** (frozen at T0; signatures may gain plumbing, not change semantics):
- `config::Config::load(env, args) -> Config` — `port` (default 11182) · `model_base` (default `http://127.0.0.1:11181`) · `model_id` (default `qwen3-4b`) · `kb_dir` (default `/var/lib/111/kb/current`) · `state_dir` (default `/var/lib/111/agent`) · `category` · `daemon_health_url` (default `http://127.0.0.1:11180/health`) · `opencode_path` (default `opencode`) · `node_id: Option<String>` (env `AGENT111_NODE_ID` / `--node-id`; Layer 2 NodeId hex handed down by 111d, stamps proposals) · `lan bool` (default false).
- `model::Client { base, model_id }` · `Client::reachable() -> bool` · `Client::chat_stream(req, on_delta) -> Reply` where `Reply = { text, tool_calls: Vec<ToolCall> }` — OpenAI-compatible `/chat/completions`, streaming.
- `kb::Kb::open(dir) -> Kb` (absent dir ⇒ `Kb::absent()`, never panic) · `Kb::index_prompt() -> String` (root llms.txt digest for the system prompt) · `Kb::grep(name) -> Vec<String>` (filenames only, `_meta/` excluded) · `Kb::read_one(path) -> Option<String>`.
- `events::Journal::append(kind, detail)` — JSONL, capacity-trimmed (1 MiB FIFO); `Journal::subscribe() -> broadcast receiver` for SSE.
- `store::Store { state_dir }` — `conversation(id) -> append/load JSONL` · `save_proposal(Proposal) -> id` · `save_task(Task) -> id`; every state write goes through `store`.

**HTTP contract** (axum, bind `127.0.0.1:11182`; LAN bind only with `lan true` + startup warning; Host allowlist `127.0.0.1`/`localhost` any port; all GETs GET-only):
- `GET /health` → `{schema:1, status:"ok"|"degraded"|"error", components:{model:bool, kb:bool, state_dir:bool}}`; 200 for `ok`|`degraded` (process alive — supervision must not restart the agent over a dead hub or absent KB), 503 only for `error` (state_dir unusable).
- `GET /v1/agent/status` → `{schema:1, state:"local"|"offloaded"|"unavailable", model:{base, reachable:bool, id}, kb:{present:bool, entries:null|int}, conversations:int, tasks_open:int, uptime_sec, version}`.
- `POST /v1/chat` → body `{conversation_id:null|str, message:str, stream:bool(default true)}`. `stream:true` ⇒ SSE `data:` events, each a JSON object: `{"type":"delta","text":"…"}` per text delta · `{"type":"tool_call","name":"…"}` when a tool runs · `{"type":"done","conversation_id":"…","reply":"…","tool_trace":[{"name","ok"}]}` final, then `data: [DONE]` · `{"type":"error","error":"…"}` on mid-stream failure; if the model is known-unreachable up front ⇒ HTTP 503 `{schema:1,error}` before any stream. `stream:false` ⇒ JSON `{schema:1, conversation_id, reply, tool_trace:[…]}`.
- `GET /v1/chat/history?conversation_id=…` → `{schema:1, conversation_id, messages:[OpenAI message objects in order]}`; unknown id ⇒ 200 with empty `messages`; missing param ⇒ 400. (111d's relay serves Layer 4 from this — the UI never talks to 11182 directly.)
- `GET /v1/agent/events` → SSE stream of `{schema:1, seq, ts, kind, detail}` (replay last 50 + live).
- `POST /v1/agent/task` → body `{kind:"diagnose"|"propose"|"patch", refs:[str], note:null|str}` → 202 `{task_id}`; `patch` without a working opencode ⇒ 409 `{schema:1, error:"needs hub"}`.
- Errors: `{schema:1, error:str}` with honest codes (503 model unavailable, 400 bad body, 409 needs hub).

**Tools (fixed set — no shell, ever):** `kb.read{query}` · `health.read{}` (own components + daemon_health_url probe) · `logs.read{n}` (journal tail) · `proposal.write{title, body}` · `task.submit{kind, refs, note}`. Tool protocol = OpenAI tool-calling; loop bounded at 4 tool rounds per chat turn.

**Honesty rules (bind every task):** distinct states (`local|offloaded|unavailable`), never silent · KB absent ⇒ chat says knowledge is absent, never invents project facts · `patch` without opencode ⇒ `needs hub` · model base guard: loopback always allowed; RFC1918/ULA/link-local always allowed (that **is** the hub offload case — a 2A node offloads while still binding loopback); any public/WAN host refused at startup, no override · `lan` flag controls endpoint binding only, never the model guard.

**State layout** (`state_dir`, created 0700): `conversations/<id>.jsonl` · `proposals/<id>.json` `{schema:1, id, node_id:null|str, task_id:null|str, kind, title, body, created_at}` · `tasks/<id>.json` `{schema:1, id, kind, refs, note, state:"running"|"done"|"failed"|"refused", proposal_id:null|str, created_at, finished_at}` · `events.jsonl`.

**Cross-layer wiring (built layers, minimal):** Layer 1 — `111d` spawns `111-agent` and `llama-server` as supervised children through its existing `spawn_child(ChildSpec)` (the rnsd pattern) when the binaries exist in `current/bin/`; child env carries category, KB dir (`<root>/current/share/kb`), state dir (`<root>/agent`), env file (`<etc>/agent.env`), node id; `/api/status` gains an additive `intelligence` object. Layer 2 — node id only (public NodeId hex) flows to the agent; **no private key material ever reaches the agent**; proposal *signing* is deferred to Layer 9's PR packaging (which can call id111 as root-of-trust holder). Layer 4 — UI talks only to 111d; 111d relays to the agent API (that relay is a Layer 4 build item, not this layer's). Layer 0 — KB dir is the release's `share/kb/` tree, already integrity-verified by 111d at boot.

## Tasks

| # | Task | Files owned | Acceptance criteria | Verifiable check | Status |
|---|---|---|---|---|---|
| T0 | Scaffolding + contracts | `intelligence/TASKS.md`, `Cargo.toml`, `crates/agent111/**` (stubs), `scripts/check.sh` | workspace compiles host+musl; all module stubs + approved deps; check.sh = fmt+clippy+test aggregate | `cargo check --target x86_64-unknown-linux-musl` green; `scripts/check.sh` exits 0 | ✅ |
| T1 | Config + state + health | `src/config.rs`, `src/health.rs`, `src/store.rs` (init only), `tests/config_*` | env-file (`/etc/111/agent.env`) + env-var + flag precedence; state dir 0700 + subdirs; model-base guard (loopback + private ranges allowed, public IP / hostname refused, `lan` never consulted); `/health` per contract | `cargo test`: precedence matrix, guard accept/refuse table, perms asserts | ✅ |
| T2 | Model client | `src/model.rs`, `tests/model_*`, `tests/common/mod.rs` | reachable() probe; non-stream + SSE-stream chat completions via ureq in spawn_blocking; tool-call parsing (tool_calls array); timeouts + size caps; honest errors (unreachable ≠ HTTP-error ≠ malformed) | `cargo test` vs std-TcpListener mock: happy stream, tool_calls reply, 500, timeout, malformed SSE | ✅ |
| T3 | KB module | `src/kb.rs`, `tests/kb_*`, `tests/fixtures/kb/**` | absent dir ⇒ `Kb::absent()` (no panic); index_prompt = root llms.txt body; grep = filenames only, `_meta/` excluded, case-insensitive; read_one resolves unique basename, never escapes kb_dir (no `..`) | `cargo test` on fixture KB (mini llms.txt + 3 entries + _meta decoy): ritual order, absent path, traversal refusal | ✅ |
| T4 | Tools + chat endpoint | `src/tools.rs`, `src/chat.rs`, `src/http.rs` (chat routes), `tests/chat_*` | 5 tools exactly, JSON schemas, no shell; bounded loop (≤4 rounds); conversation append via store; POST /v1/chat SSE + non-stream per contract; GET /v1/chat/history per contract; system prompt = preamble + category + kb.index_prompt(); KB-absent ⇒ honesty line injected | `cargo test` vs mock model: grounded answer flow (model calls kb.read → gets entry → answers), KB-absent honesty, round cap, bad-body 400 | ✅ |
| T5 | Events + status | `src/events.rs`, `src/status.rs`, `src/http.rs` (events/status routes), `tests/events_*` | journal JSONL + 1 MiB FIFO trim; SSE replay-50 + live; /v1/agent/status fields per contract (state derivation: base loopback ⇒ local, private ⇒ offloaded, unreachable ⇒ unavailable); logs.read backed by journal | `cargo test`: trim bound, SSE replay+live, status matrix | ✅ |
| T6 | Task engine + opencode adapter | `src/tasks.rs`, `tests/tasks_*`, `tests/fixtures/fake-opencode` | diagnose/propose: model-driven, result via proposal.write, task persisted state→done; patch: opencode present ⇒ spawn headless JSONL, parse events ⇒ proposal; absent ⇒ 409 `needs hub`, **no task persisted** (a refused submit is not a task; journal `task.refused`); task.submit tool routes here; restart marks leftover `running` tasks `failed` + journals `task.orphaned` (never pretends old work lives) | `cargo test` w/ fake-opencode script + mock model: diagnose→proposal, patch→delegated, patch-without-opencode 409, restart orphaning | ✅ |
| T8 | Layer 1 wiring (bootstrap crate) | `bootstrap/crates/node111/src/daemon/intelligence.rs` (new), `bootstrap/crates/node111/src/daemon/mod.rs` (one block + `pub mod`), `bootstrap/crates/node111/src/daemon/dashboard/mod.rs` (additive `intelligence` field + JSON), `bootstrap/crates/node111/tests/intelligence_*` | 111d conditionally spawns `llama-server` (binary in `current/bin/` AND a `.gguf` under `<root>/models/current/`) and `111-agent` (binary in `current/bin/`) via existing `spawn_child`; env per wiring contract; stdout logs under `<root>/agent/`; both absent ⇒ honest `absent` states, no error; `/api/status.intelligence = {agent:"absent"|"running"|"backoff"|"degraded", model_server:same, agent_url:null|"http://127.0.0.1:11182"}`; stop on shutdown; NO new deps; bootstrap pinned contracts untouched (additive-only) | `cargo test` in bootstrap w/ fake child binaries (sh scripts) on a temp root: both spawned w/ expected env, absent ⇒ absent, crash-loop ⇒ degraded; full bootstrap `scripts/check.sh` still green | ✅ |
| T7 | Docs + final gate | `intelligence/README.md` (build/contract appendix), `intelligence/TASKS.md` (summary) | README gains "Building / contract quick-reference" appendix matching implemented behavior exactly; full suite green on host + musl check | `scripts/check.sh` exits 0; reviewer reads doc vs code | ✅ |

## Waves (file-ownership disjoint within a wave)

- **Wave 0** (reviewer): T0 scaffolding.
- **Wave A**: T1 · T2 · T3 (self-contained modules against pinned contracts).
- **Wave B**: T4 · T5 (both need Wave A APIs only via pinned contracts; T4 owns chat routes, T5 owns events/status routes — `src/http.rs` touched by both ⇒ T5 first, T4 rebases, or sequential in one wave).
- **Wave C**: T6 (needs T4 tool loop + T5 journal via contracts) · T8 (different crate — bootstrap; disjoint by construction).
- **Wave D**: T7 → full `scripts/check.sh` + summary.

## Summary (filled at completion)

**Built (2026-10-13):** `agent111` crate / `111-agent` binary — 88 tests across 7 suites, `scripts/check.sh` green (fmt, clippy -D warnings, tests, musl x86_64 check). T0–T8 all verified and committed one per task. Layer 1 wiring landed in the bootstrap crate (`daemon/intelligence.rs`, additive `/api/status.intelligence`), bootstrap's own `scripts/check.sh` ALL GREEN (a whitespace-only rustfmt pass over Layer 1/2 files was needed first; identity's 161 + 44 tests unchanged).

**End-to-end check with real binaries** (staged node root, real KB snapshot with payload manifest, no model): `111d` spawned `111-agent`; `/api/status.intelligence` = running/absent/url; agent `/health` degraded-200, status `unavailable`, KB present (23 entries), category from 111d; chat → 503 `model unavailable`; diagnose → 202 then `task.failed` journaled; SIGTERM → child gone. This dev box has opencode 1.18.18 on PATH, so `patch` was correctly *not* refused — which exposed that opencode needs a project `opencode.json` to accept a non-catalog model id (fixed) and that it fetches its provider SDK from the network on first use (recorded as pre-hub gate G2 in the README).

**Found and fixed during review:** `ToolCall` wire shape (T2 serialized `{id,name,arguments}`; OpenAI/llama-server require `{id,type,function:{name,arguments:"json string"}}` — a mock cannot catch this); journal replay/live gap (one lock); `111_AGENT_*` → `AGENT111_*`.

**Deferred (honest list):** proposal signing (Layer 9 PR packaging, via id111 held by 111d — the agent never gets key material) · real llama-server/GGUF packaging in the channel manifest (Layer 1 release machinery; the daemon-side spawn is ready) · opencode offline provider prefetch (hub profile) · 2A→hub offload over a real LAN · conversation trimming for long histories · serializing concurrent turns on one conversation · the queued adversarial pass on the spec · Layer 4's `111d` relay to this API (Layer 4 build item).
