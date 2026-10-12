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
- `config::Config::load(env, args) -> Config` — `port` (default 11182) · `model_base` (default `http://127.0.0.1:11181`) · `model_id` (default `qwen3-4b`) · `kb_dir` (default `/var/lib/111/kb/current`) · `state_dir` (default `/var/lib/111/agent`) · `category` · `daemon_health_url` (default `http://127.0.0.1:11180/health`) · `opencode_path` (default `opencode`) · `lan bool` (default false).
- `model::Client { base, model_id }` · `Client::reachable() -> bool` · `Client::chat_stream(req, on_delta) -> Reply` where `Reply = { text, tool_calls: Vec<ToolCall> }` — OpenAI-compatible `/chat/completions`, streaming.
- `kb::Kb::open(dir) -> Kb` (absent dir ⇒ `Kb::absent()`, never panic) · `Kb::index_prompt() -> String` (root llms.txt digest for the system prompt) · `Kb::grep(name) -> Vec<String>` (filenames only, `_meta/` excluded) · `Kb::read_one(path) -> Option<String>`.
- `events::Journal::append(kind, detail)` — JSONL, capacity-trimmed (1 MiB FIFO); `Journal::subscribe() -> broadcast receiver` for SSE.
- `store::Store { state_dir }` — `conversation(id) -> append/load JSONL` · `save_proposal(Proposal) -> id` · `save_task(Task) -> id`; every state write goes through `store`.

**HTTP contract** (axum, bind `127.0.0.1:11182`; LAN bind only with `lan true` + startup warning; Host allowlist `127.0.0.1`/`localhost` any port; all GETs GET-only):
- `GET /health` → `{schema:1, status:"ok"|"degraded"|"error", components:{model:bool, kb:bool, state_dir:bool}}`; 200 for `ok`|`degraded` (process alive — supervision must not restart the agent over a dead hub or absent KB), 503 only for `error` (state_dir unusable).
- `GET /v1/agent/status` → `{schema:1, state:"local"|"offloaded"|"unavailable", model:{base, reachable:bool, id}, kb:{present:bool, entries:null|int}, conversations:int, tasks_open:int, uptime_sec, version}`.
- `POST /v1/chat` → body `{conversation_id:null|str, message:str, stream:bool(default true)}` → SSE `data:` deltas then `data: [DONE]` + `conversation_id` event; `stream:false` ⇒ JSON `{conversation_id, reply, tool_trace:[…]}`.
- `GET /v1/agent/events` → SSE stream of `{schema:1, seq, ts, kind, detail}` (replay last 50 + live).
- `POST /v1/agent/task` → body `{kind:"diagnose"|"propose"|"patch", refs:[str], note:null|str}` → 202 `{task_id}`; `patch` without a working opencode ⇒ 409 `{schema:1, error:"needs hub"}`.
- Errors: `{schema:1, error:str}` with honest codes (503 model unavailable, 400 bad body, 409 needs hub).

**Tools (fixed set — no shell, ever):** `kb.read{query}` · `health.read{}` (own components + daemon_health_url probe) · `logs.read{n}` (journal tail) · `proposal.write{title, body}` · `task.submit{kind, refs, note}`. Tool protocol = OpenAI tool-calling; loop bounded at 4 tool rounds per chat turn.

**Honesty rules (bind every task):** distinct states (`local|offloaded|unavailable`), never silent · KB absent ⇒ chat says knowledge is absent, never invents project facts · `patch` without opencode ⇒ `needs hub` · model base guard: loopback always allowed; RFC1918/ULA/link-local always allowed (that **is** the hub offload case — a 2A node offloads while still binding loopback); any public/WAN host refused at startup, no override · `lan` flag controls endpoint binding only, never the model guard.

**State layout** (`state_dir`, created 0700): `conversations/<id>.jsonl` · `proposals/<id>.json` `{schema:1, id, task_id:null|str, kind, title, body, created_at}` · `tasks/<id>.json` `{schema:1, id, kind, refs, note, state:"running"|"done"|"failed"|"refused", proposal_id:null|str, created_at, finished_at}` · `events.jsonl`.

## Tasks

| # | Task | Files owned | Acceptance criteria | Verifiable check | Status |
|---|---|---|---|---|---|
| T0 | Scaffolding + contracts | `intelligence/TASKS.md`, `Cargo.toml`, `crates/agent111/**` (stubs), `scripts/check.sh` | workspace compiles host+musl; all module stubs + approved deps; check.sh = fmt+clippy+test aggregate | `cargo check --target x86_64-unknown-linux-musl` green; `scripts/check.sh` exits 0 | ✅ |
| T1 | Config + state + health | `src/config.rs`, `src/health.rs`, `src/store.rs` (init only), `tests/config_*` | env-file (`/etc/111/agent.env`) + env-var + flag precedence; state dir 0700 + subdirs; model-base guard (loopback, or private-range iff `lan true`, else startup refusal); `/health` per contract | `cargo test`: precedence matrix, guard accept/refuse table, perms asserts | ⬜ |
| T2 | Model client | `src/model.rs`, `tests/model_*`, `tests/common/mod.rs` | reachable() probe; non-stream + SSE-stream chat completions via ureq in spawn_blocking; tool-call parsing (tool_calls array); timeouts + size caps; honest errors (unreachable ≠ HTTP-error ≠ malformed) | `cargo test` vs std-TcpListener mock: happy stream, tool_calls reply, 500, timeout, malformed SSE | ⬜ |
| T3 | KB module | `src/kb.rs`, `tests/kb_*`, `tests/fixtures/kb/**` | absent dir ⇒ `Kb::absent()` (no panic); index_prompt = root llms.txt body; grep = filenames only, `_meta/` excluded, case-insensitive; read_one resolves unique basename, never escapes kb_dir (no `..`) | `cargo test` on fixture KB (mini llms.txt + 3 entries + _meta decoy): ritual order, absent path, traversal refusal | ⬜ |
| T4 | Tools + chat endpoint | `src/tools.rs`, `src/chat.rs`, `src/http.rs` (chat routes), `tests/chat_*` | 5 tools exactly, JSON schemas, no shell; bounded loop (≤4 rounds); conversation append via store; POST /v1/chat SSE + non-stream per contract; system prompt = preamble + category + kb.index_prompt(); KB-absent ⇒ honesty line injected | `cargo test` vs mock model: grounded answer flow (model calls kb.read → gets entry → answers), KB-absent honesty, round cap, bad-body 400 | ⬜ |
| T5 | Events + status | `src/events.rs`, `src/status.rs`, `src/http.rs` (events/status routes), `tests/events_*` | journal JSONL + 1 MiB FIFO trim; SSE replay-50 + live; /v1/agent/status fields per contract (state derivation: base loopback ⇒ local, private ⇒ offloaded, unreachable ⇒ unavailable); logs.read backed by journal | `cargo test`: trim bound, SSE replay+live, status matrix | ⬜ |
| T6 | Task engine + opencode adapter | `src/tasks.rs`, `tests/tasks_*`, `tests/fixtures/fake-opencode` | diagnose/propose: model-driven, result via proposal.write, task persisted state→done; patch: opencode present ⇒ spawn headless JSONL, parse events ⇒ proposal; absent ⇒ 409 `needs hub` + task `refused`; task.submit tool routes here; tasks survive restart (reload from tasks/*.json) | `cargo test` w/ fake-opencode script + mock model: diagnose→proposal, patch→delegated, patch-without-opencode 409, restart reload | ⬜ |
| T7 | Docs + final gate | `intelligence/README.md` (build/contract appendix), `intelligence/TASKS.md` (summary) | README gains "Building / contract quick-reference" appendix matching implemented behavior exactly; full suite green on host + musl check | `scripts/check.sh` exits 0; reviewer reads doc vs code | ⬜ |

## Waves (file-ownership disjoint within a wave)

- **Wave 0** (reviewer): T0 scaffolding.
- **Wave A**: T1 · T2 · T3 (self-contained modules against pinned contracts).
- **Wave B**: T4 · T5 (both need Wave A APIs only via pinned contracts; T4 owns chat routes, T5 owns events/status routes — `src/http.rs` touched by both ⇒ T5 first, T4 rebases, or sequential in one wave).
- **Wave C**: T6 (needs T4 tool loop + T5 journal via contracts).
- **Wave D**: T7 → full `scripts/check.sh` + summary.

## Summary (filled at completion)

_pending._
