export const meta = {
  name: 'layer-6-wave-0-bounce',
  description: 'Layer 6 Wave 0 bounce: fix the fable review findings on S1 (store V3) — one opus agent on the S1 files only',
  phases: [{ title: 'Bounce', detail: 'S1 findings → fix + tests' }],
}

const ROOT = '/home/user/os'
const SP = '/tmp/claude-0/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/scratchpad'

const BRIEF = `
You are a FIX subagent for the 111network OS, Layer 6. Today is 2026-10-19. Repo root: ${ROOT} (git; NEVER commit/add/stash/checkout/reset —
the lead commits). The law: ${ROOT}/6-transport/README.md §3, §4, §8, §9 and the S1 row of ${ROOT}/6-transport/TASKS.md. Read the S1 commit
first: git show 85cf84f --stat, then M/store/foreign.rs (M/ = ${ROOT}/5-messaging/crates/mesh111/src), schema.rs V3, prune.rs, tests.rs.

OWNED (strict): M/store/{foreign.rs, schema.rs, prune.rs, tests.rs, mod.rs}, ${ROOT}/1-bootstrap/crates/node111/src/daemon/fabric/times.rs.
Another agent (B1) is editing M/fabric.rs and N/fabric/{mod,json}.rs RIGHT NOW: never touch those; keep every existing pub signature in
foreign.rs working (B1 codes against it); additions are fine. The V3 migration has not shipped, so editing the V3 SQL in place is allowed.

FINDINGS to fix (a fable judge's review of 85cf84f; each with a test):
1. MAJOR foreign.rs queue_one (~:712-750): a relay copy whose origin is at or below the edge's relayed_seq is accepted again, so a second copy of
   the same origin on the same edge is possible (the store is the ID map, §8 guard 6, and must fail closed). Fix: refuse origin.local_seq <=
   edge.relayed_seq unless a copy row with the same (edge_id, origin_msg) already exists in this transaction (the split-parts case queued by
   foreign_queue_sends), and add a partial UNIQUE index ON foreign_messages (edge_id, origin_msg, part_k) WHERE edge_id IS NOT NULL so a
   duplicate part is unrepresentable. Test: queueing the same origin twice fails; the split parts of one copy still succeed.
2. MINOR ForeignState::rank: refuse failed once the current state is relayed (keep sent → failed: a queue-accepted NAK can produce it; keep
   delivered over failed; keep failed → delivered). Flip the monotone test row accordingly and document the §4 reason.
3. MINOR is_duplicate: the 32-row dedupe window must count inbound rows only (AND dir = 'in'), so our own copies cannot push inbound rows out
   of the window. Test it with own rows interleaved.
4. MINOR prune step 2b: a queued copy with meta.handoff = 'pending' (may have been transmitted) must not be shed as backlog; treat it as handed
   (a 2a candidate) — edge_delete already keeps such rows; make the two agree and say so in the prune module note. Test.
5. MINOR add Store::foreign_cancel(id) -> Result<()>: deletes an own queued row that has no pending handoff (NotFound / Invalid otherwise);
   §9 "cancel drops unsent parts only": a group's unsent parts can be cancelled one by one. Test.
6. MINOR times.rs: DisplayTimes::open takes a two-variant enum (Ours | Foreign) instead of a free &str file name; keep load() working;
   update the one test.
Also (judge's watch item): decide and document in foreign.rs whether edge_set_enabled(false) drops never-handed queued copies — pick: it
DROPS them (a disabled edge has no backlog; re-enable moves cursor_seq anyway) and test it; and make foreign_accepted Durability::Full
(the recorded native id is own-echo evidence that must survive a crash).

BOX: export CARGO_TARGET_DIR=${SP}/target/mesh111 CARGO_BUILD_JOBS=2 (shared with B1 — cargo's lock serializes; a transient failure caused
by B1's half-edited fabric.rs: wait 90 s and retry twice; B1 does not touch your files). For times.rs: CARGO_TARGET_DIR=${SP}/target/node111.
No new dependencies. Never put a model name in a file. Nothing of yours stays running.
CHECK: sh ${ROOT}/5-messaging/scripts/check.sh (musl SKIPS: say so) and cd ${ROOT}/1-bootstrap && cargo test -p node111 --lib fabric::times
&& cargo clippy -p node111 --lib -- -D warnings. Paste the tails.
RETURN (structured): task "S1-bounce", status, files, check, deviations, notes (one line per finding: what changed, the test name).
`

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
log('Layer 6 Wave 0 bounce: S1 review findings')
const r = await agent(BRIEF, { label: 'L6:S1-bounce', phase: 'Bounce', schema: RESULT, model: 'opus', effort: 'high' })
log('S1-bounce returned: ' + (r ? r.status : 'null'))
return r
