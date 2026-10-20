export const meta = {
  name: 'layer-6-wave-1-bounce',
  description: 'Layer 6 Wave 1 bounce: fix the fable review findings on B1 (fabric foreign calls) and the S1-bounce minors — one opus agent on the fabric/store files only',
  phases: [{ title: 'Bounce', detail: 'B1 major (burst overlap) + S1-bounce/B1 minors; lead reviews + commits' }],
}

const ROOT = '/home/user/os'
const SP = '/tmp/claude-0/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/scratchpad'

const CTX = `
You are a BUILD subagent for the 111network OS, Layer 6 (Transport & Bridge). Today is 2026-10-19. Repo root: ${ROOT}
(a git repo on branch claude/layer-6-implementation-006mte; you NEVER run git commit/add/stash/checkout/reset — the lead reviews
and commits each task). You will not be messaged mid-task; everything you need is in this brief and on disk.

THE LAW: the locked spec ${ROOT}/6-transport/README.md (v1.0, D27) and your task row in ${ROOT}/6-transport/TASKS.md. Read your row
first, then the spec sections the row names (the spec is 888 lines; read it with sed -n ranges), then the as-built code you build on
(named in your brief; Wave 0 landed: store V3 in mesh111 (M/store/foreign.rs), the contract in 4-interface/api/openapi.yaml, the
A10 bytes + N/bridge/attest.rs, the supervisor's piped stdio + N/components/, the KB/licensing rows). The line-verified seam map is
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

const BRIEF = `TASK B1-bounce — close the fable review findings on B1 (commit fc802a7) and the S1-bounce review minors (commit a38fe19). Crates: ${ROOT}/5-messaging/crates/mesh111 (M/ = src/) and ${ROOT}/1-bootstrap/crates/node111 (N/ = src/daemon/). Other agents are building N/bridge, N/meshtastic, N/transport and agent111 right now: touch ONLY the files below; every API change is ADDITIVE (B2 is coding against M/fabric.rs as committed).
Owned: M/store/foreign.rs, M/store/tests.rs, M/fabric.rs, ${ROOT}/5-messaging/crates/mesh111/tests/foreign_burst.rs, N/fabric/mod.rs, N/fabric/json.rs.
Read first: the two commits (git show fc802a7 --stat; git show a38fe19 --stat), M/store/foreign.rs (queue_one, foreign_set_meta/meta_key, foreign_handoff_begin/foreign_accepted, foreign_cancel, drop_backlog), M/fabric.rs "Layer 6 foreign calls" block, N/fabric/mod.rs (the publish arm ~:1019-1031 and FabricHandle::fabric), N/fabric/json.rs (the contract validator ~:775-882), tests/foreign_burst.rs.
Findings to close (each with a test that fails without the fix; cite the finding id in the test name or a comment):
(F1, MAJOR, foreign_burst.rs:76-108) the test never proves the two 111 sends overlap the burst. Fix: record an Instant when the LAST foreign_ingest returns inside the burst task and one right after the two send calls are issued; assert the burst ended after the sends were issued (or gate the sends on the first ingest having started, e.g. a oneshot the burst task fires after its first call returns), so the bound is measured during the burst; keep the 10 s bound and the printed numbers.
(F2, minor, json.rs validator) check maxLength, minLength, maxItems, minItems, maximum (and minimum if present in the digest) in the subset validator; name the keywords still skipped in the module doc; the_validator_catches_drift gains a maxLength drift case (a 300-byte enabled_by must fail the validator).
(F3, minor, M/fabric.rs cancel event) a cancel currently emits ForeignChange::State with the id of a row that no longer exists. Fix: on cancel emit a Conversation change for the row's conversation (id None) instead, so the client refetches; update the fabric test.
(F4, minor, M/fabric.rs foreign_conversation_upsert) an uncached conversation re-emits a Conversation event even when nothing changed. Fix: when the id is not cached, compare the requested title/kind against the row read from the store and emit only on a real difference or a genuinely new row; test: two identical upserts after a cache clear emit one event.
(F5, minor, N/fabric/mod.rs ~:1019-1031) on a fabric error other than NotFound while reading a conversation for a batch, its network_message frames are dropped with an eprintln. Fix: push a conversation frame for that conversation (seq 0) so the client refetches; a unit test with a failing fabric if the existing test seam allows it, else a code comment and a note.
(F6, minor, N/fabric/mod.rs FabricHandle::fabric doc) state that foreign writes go through the handle's wrappers (display times are recorded there) and that a caller using the raw Fabric leaves channel rows stamped at first render.
(F7, minor, M/store/foreign.rs meta_key / foreign_set_meta) refuse the reserved key "handoff" with StoreError::Invalid so the at-most-once mark is representable only through foreign_handoff_begin / foreign_accepted / the new abort; one-line test.
(F8, minor, M/store/foreign.rs) ADD Store::foreign_handoff_abort(id) -> Result<()> (Durability::Full): only a queued own row with handoff = pending; clears the mark (the row is offered again by foreign_queued, cancellable, droppable); NotFound / Invalid otherwise. Doc: the caller uses it only when the carrier reported a pre-air refusal and no Accepted (§4 Later retries). Add Fabric::foreign_handoff_abort in M/fabric.rs (actor job; emit nothing or a State change — pick the reading consistent with foreign_handoff_begin and say so) and FabricHandle::foreign_handoff_abort in N/fabric/mod.rs. Tests at store and fabric level.
(F9, minor, M/store/foreign.rs:789-797) in one foreign_queue_sends batch, cross-check the further parts of one (edge, origin): part_n must equal the first part's and part_k must be the previous + 1 (else Invalid, nothing written); document that a batch queues origins in ascending local_seq order (a descending batch fails as a whole — fail closed). Test.
(F10, minor, M/store/foreign.rs foreign_cancel) restrict foreign_cancel to send-group rows (group_id set): a relay copy (edge_id set) answers Invalid — copies leave only by shed, disable or delete, which the edge's counters explain; update the module doc and the existing cancel tests (tests.rs:2795 cancels a copy today).
Check: export CARGO_TARGET_DIR=${SP}/target/mesh111 CARGO_BUILD_JOBS=2; cd ${ROOT}/5-messaging && sh scripts/check.sh (fmt, clippy, every test target incl. foreign_burst; musl SKIP expected); then export CARGO_TARGET_DIR=${SP}/target/node111; cd ${ROOT}/1-bootstrap && cargo fmt --all -- --check && cargo clippy -p node111 --lib --tests -- -D warnings && cargo test -p node111 --lib fabric. The node111 tree has other agents' half-edited files: if a build fails only in N/bridge, N/meshtastic or N/transport, wait 90 s and retry twice, then report blocked-by with the lib fabric result you could get; never touch those files.`

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

phase('Bounce')
log('Layer 6 Wave 1 bounce: B1 review major + S1-bounce/B1 minors')
const r = await agent(CTX + '\n\n' + BRIEF, { label: 'L6:B1-bounce', phase: 'Bounce', schema: RESULT, model: 'opus', effort: 'high' })
log('returned: ' + (r ? r.status : 'null'))
return r
