export const meta = {
  name: 'layer-6-wave-1a',
  description: 'Layer 6 build Wave 1 (part A1): B1 fabric foreign calls — started as soon as S1 (store V3) and S2 (contract) landed, beside the rest of Wave 0',
  phases: [{ title: 'Wave 1', detail: 'B1 alone; B2 and B3 follow in wave-1.js' }],
}

const ROOT = '/home/user/os'
const SP = '/tmp/claude-0/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/scratchpad'

const CTX = `
You are a BUILD subagent for the 111network OS, Layer 6 (Transport & Bridge). Today is 2026-10-19. Repo root: ${ROOT}
(a git repo on branch claude/layer-6-implementation-006mte; you NEVER run git commit/add/stash/checkout/reset — the lead reviews
and commits each task). You will not be messaged mid-task; everything you need is in this brief and on disk.

THE LAW: the locked spec ${ROOT}/6-transport/README.md (v1.0, D27) and your task row in ${ROOT}/6-transport/TASKS.md. Read your row
first, then the spec sections the row names (the spec is 888 lines; read it with sed -n ranges), then the as-built code you build on
(named in your brief; landed so far: store V3 in mesh111 (M/store/foreign.rs, S1) and the contract in 4-interface/api/openapi.yaml (S2); S3/S4/S6 are still running on other files — never touch N/supervise.rs, N/components/, src/install/, id111, N/bridge/attest.rs, 0-knowledge/, LICENSING.md). The line-verified seam map is
${ROOT}/plan/layers/6-transport/research/00-code-seams.md (line numbers from 2026-10-19; re-check before editing). Decisions:
${ROOT}/plan/DECISIONS.md D23, D25, D26, D27. Do not re-open decisions; implement them. If the spec is ambiguous, pick the reading that is
safest (fail closed) and record it under "deviations". Owner rule in AGENTS.md: the Raspberry Pi is the floor — add no resident
process, poll, timer or runtime dependency to the base configuration; everything third-party is an on-demand component with zero
runtime weight while off; the bridge core is inert with no adapter enabled.

FILE OWNERSHIP IS STRICT: create or modify ONLY the files/dirs your brief lists under "Owned" (plus new files inside those dirs). Other
tasks run in parallel on other files right now; if you need a change elsewhere, do NOT make it — describe it under "deviations" and
continue (exception: a tiny additive \`pub mod\` / registration line a new module needs — do it and list it). Never touch
plan/, TASKS.md boards or INDEX.md unless your brief names them.

HOUSE RULES: Rust — edition 2021, typed errors (never panic on input; sizes checked before parse), doc comments citing the spec section
("§x.y"), unit tests in-module and integration tests under tests/, rustfmt + clippy -D warnings clean; no new dependencies beyond the
crate's Cargo.toml unless your brief names one (node111 policy: ${ROOT}/1-bootstrap/TASKS.md "Dependency policy"; mesh111's and id111's sets
are final) — if you truly need one, do not add it, report it. No wall clock in any ordering or cryptographic decision (local_seq and the
monotonic tick only). Nothing downloads at runtime. No Reticulum code or imports anywhere in Rust. Foreign text is untrusted everywhere.
Never put a model name or identifier into any file. Never commit.

BOX: 4 CPUs, 15 GB RAM, a fixed disk allowance (~20 GB free; do NOT cargo clean, do NOT create another target dir). Shared WARM target
dirs, one per workspace, which cargo's own lock serializes between agents: 1-bootstrap (node111) → CARGO_TARGET_DIR=${SP}/target/node111 ·
5-messaging (mesh111) → ${SP}/target/mesh111 · 2-identity (id111) → ${SP}/target/id111 · 3-intelligence (agent111) → ${SP}/target/agent111.
Always export the right one before any cargo command, with CARGO_BUILD_JOBS=2. Iterate with narrow commands (cargo test -p <crate> --lib
<module>, cargo check) and run the full check once at the end. A build that fails only because of a file another task owns (another agent
is mid-edit; node111 compiles mesh111 and id111 from disk): wait 90 s, retry twice, then report "blocked-by: <task>" — never touch that
file. Toolchain: cargo 1.97 stable (rustfmt, clippy); NO musl cross toolchains here (musl / G2 stages SKIP — say so, never fake them);
node 22 + pnpm (UI deps installed); python 3.13 with venv + pip (PyPI reachable through the proxy); crates.io reachable; cc 13; jq, curl,
nc, strace, ip (iproute2), systemd-analyze present; unprivileged netns work (unshare -Urnm); no rnsd installed on the host. Scratch
files go under ${SP}/<task-id>/ only. Linux hygiene: nothing of yours stays running when you return (ps check), kill what you started,
delete your scratch.

RUN YOUR CHECK (the row's "Check" column, as refined in your brief) until it is green; paste its tail.

RETURN (structured): task id, status (done | partial | blocked), files changed/created (exact paths), the check command + its tail
(≤ 25 lines), deviations (each with why), notes for the reviewer (what to read first, open questions). Factual; the lead reads diffs.
`

const B1 = `TASK B1 — Fabric foreign calls (spec §3 "Typed writes, batched", "Ids", §10 SSE coalescing; integration M3). Crates: ${ROOT}/5-messaging/crates/mesh111 (M/ = src/) and ${ROOT}/1-bootstrap/crates/node111 (N/ = src/daemon/).
Owned: M/fabric.rs, ${ROOT}/5-messaging/crates/mesh111/tests/foreign_burst.rs (NEW), N/fabric/mod.rs, N/fabric/json.rs, N/fabric/times.rs (only to use S1's foreign instance), tests in those modules.
Read first: M/store/foreign.rs AS LANDED by S1 (its pub surface is final for this wave: foreign_conversation_upsert/…/mark_read, foreign_insert_batch (inbound only; a per-row id_globally_unique flag), foreign_queue_send/foreign_queue_sends (relay copies: advance relayed_seq in the same transaction), send_group_create(text, targets, parts) (atomic), send_group/send_group_rows, foreign_set_state (monotone; StateChange::Unchanged), foreign_set_meta, foreign_handoff_begin (FULL) / foreign_accepted (native id + clears the handoff mark), foreign_message/foreign_messages/foreign_thread_page, foreign_queued (user rows first; skips pending handoffs and copies on disabled/paused edges), edge_create (needs an EdgeConsent {hash, enabled_by}; re-enable moves cursor_seq), edge/edge_list/edge_set_enabled/edge_set_paused/edge_advance_relayed_seq/edge_count/edge_delete/edge_inputs, foreign_seen_insert/contains/count; S1's as-built rules you must respect: every own row is user-composed (group_id set) XOR a relay copy (edge_id + origin_msg set) — a direct reply in a conversation is therefore a ONE-TARGET send group; inbound rows are stored delivered; there is no cancel call: cancel = foreign_set_state(Failed, reason cancelled) only while queued with nothing handed off; the shaping quota key is quota.foreign via Class::has_quota() — N/fabric/json.rs quota_classes() still filters on is_refusable(): switch it to has_quota() so the contract's quotas map shows foreign (you own json.rs)), M/fabric.rs (FabricEvent :363, the Fabric handle :628, call/with_store :934, the actor run loop :1021 with TICK_BATCH, Core::emit — how Message events are coalesced per tick), N/fabric/mod.rs (publish :927, pump, housekeeping, the FabricHandle and how routes reach the Fabric), N/fabric/json.rs (msg_id/parse_msg_id, the Message JSON shape), the contract ${ROOT}/4-interface/api/openapi.yaml AS LANDED by S2 (schemas ForeignConversation, ForeignMessage, SendGroup, BridgeEdge, the SSE network_message payload) — your JSON shapes must match it exactly.
Build: (1) mesh111: typed Fabric methods that run inside the actor (one synchronous SQLCipher transaction between frame pumps, via the same call mechanism as send/threads): foreign_ingest(batch: Vec<ForeignInsert>) -> Result<Vec<Option<i64>>> (≤ 256 rows per call, error above; the bridge core awaits each call — back-pressure on the actor's input channel), foreign_set_state(id, state, evidence, reason) -> Result<ForeignStateOutcome>, foreign_set_meta, foreign_queue_send(...) -> Result<i64>, foreign_cancel(id) (= foreign_set_state(Failed, cancelled) only while queued with nothing handed off — S1 has no cancel call; refuse otherwise), foreign_group_create/foreign_group(id), foreign_edge_create/list/get/set_enabled/set_paused/patch/advance_relayed/count/delete, foreign_edge_inputs(edge, limit), foreign_seen_insert/contains, foreign_conversation_upsert/pin/mark_read, foreign_conversations(network: Option<&str>), foreign_conversation(id), foreign_thread(conv, since_seq, limit) -> page, foreign_queued(limit) (the adapter tick's queue: user rows first, then local_seq — S1's query), foreign_message(id). (2) FabricEvent::Foreign { conv: i64, id: Option<i64>, change: ForeignChange::{New, State, Conversation} }: emitted beside Message/Contact/Fabric/Settings and COALESCED in Core::emit: per message per tick for DMs and own rows (id set), per CONVERSATION per tick (id = None, highest seq carried in the frame by the daemon) for inbound channel traffic — a flag on the conversation kind decides (channel ⇒ coalesce per conversation). Nothing of this touches 111 message sequencing or the outbox. (3) node111 N/fabric/json.rs: fc_id/fm_id/be_id/sg_id + parse_*_id (digits only, like parse_msg_id; msg- stays 111-only), foreign_conversation_json, foreign_message_json (dir, sender {native, name}, auth, text, state, evidence, reason_code, group_id, part {k, n}, relayed_from {msg, edge}, via_internet from meta.via_mqtt, seq, at from the foreign display-times file (S1's second instance), claim_ts, truncated from meta), send_group_json, edge_json — all matching S2's contract. (4) N/fabric/mod.rs: the publish arm for FabricEvent::Foreign → SSE event "network_message" {network, conv: fc-id, id: fm-id|null, change, seq} (coalesce again per key in the batch as the message arm does), and a FabricHandle accessor the bridge core (B2) and the routes (C1) will use: pub fn fabric(&self) -> Option<Fabric> exists? (check; expose what B2 needs: the Fabric clone + the foreign display-times instance). The foreign display-times instance is opened beside the 111 one at start (fabric/display-times.foreign).
Tests: mesh111 — unit tests for every call (in fabric.rs tests or tests/), the coalescing rule (channel inbound: one Foreign event per conversation per tick even with 50 rows; DM/own: per message), the 256 cap; tests/foreign_burst.rs — two in-process fabrics over the lossy test link (read tests/ for the existing two-member harness) exchanging a 111 message while 1,000 foreign rows are ingested in batches of 256 on one side: assert the 111 message reaches delivered within a bound (10 s wall on this box is fine for the assertion; print the measured latency) and every foreign row is present with contiguous local_seq order. node111 — json round-trips (ids, shapes vs. the contract digest ${ROOT}/4-interface/ui/test/contract.gen.ts: assert the keys you emit are the schema's required keys), the SSE arm with a fake EventBus (read how tests publish events).
Check: export CARGO_TARGET_DIR=${SP}/target/mesh111 CARGO_BUILD_JOBS=2; sh ${ROOT}/5-messaging/scripts/check.sh (musl SKIPS — say so); then export CARGO_TARGET_DIR=${SP}/target/node111; cd ${ROOT}/1-bootstrap && cargo fmt --all -- --check && cargo clippy -p node111 --all-targets -- -D warnings && cargo test -p node111 --lib fabric.`

const RESULT = {
  type: 'object',
  required: ['task', 'status', 'files', 'check', 'deviations', 'notes'],
  properties: {
    task: { type: 'string' },
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files: { type: 'array', items: { type: 'string' } },
    check: { type: 'string' },
    deviations: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

const run = (id, brief, model, effort) =>
  agent(CTX + '\n\n' + brief, { label: `L6:${id}`, phase: 'Wave 1', schema: RESULT, model, effort }).then(r => r && { ...r, id })


phase('Wave 1')
log('Layer 6 Wave 1 A1: B1 fabric foreign calls')
const r = await run('B1', B1, 'opus', 'xhigh')
log('B1 returned: ' + (r ? r.status : 'null'))
return r
