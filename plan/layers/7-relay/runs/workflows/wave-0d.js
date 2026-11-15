export const meta = {
  name: 'layer-7-wave-0d',
  description: 'Layer 7 Wave 0d: M1b (the store counts custody bytes in O(1): schema v5 totals kept by triggers) and M4e (the M4d bounce: a hit-cache change restarts the swap probe only when a stored row of that dest waits), each followed by an independent read-only review; the lead re-runs the gates and commits',
  phases: [
    { title: 'Fix', detail: 'M1b in the main tree (store/*), M4e in a git worktree (fabric.rs, tests/forward.rs, one store read)' },
    { title: 'Review', detail: 'one read-only adversarial pass over each diff' },
  ],
}

const SP = '/tmp/claude-0/-home-user-os/f8991dd8-132d-5f0d-807d-446e0e7eb394/scratchpad'
const a = args || {}
const MAIN = '/home/user/os'
// args.roots = { M4e: '<git worktree dir>' } runs a task in its own tree; default: the main tree.
const ROOTS = a.roots || {}
const rootOf = id => ROOTS[id] || MAIN
const ONLY = Array.isArray(a.only) ? a.only : null

const ctx = (id) => {
  const ROOT = rootOf(id)
  return `
You are a BUILD subagent for the 111network OS, Layer 7 (Relay & Presence). Today is 2026-11-11. Your tree: ${ROOT}${ROOT === MAIN ? '' : ` (a git
worktree of ${MAIN} at the same commit; work ONLY inside it, never in ${MAIN}; another task edits ${MAIN} right now)`}. Branch
claude/confident-ramanujan-y5ob49; you NEVER run git commit/add/stash/checkout/reset/worktree: the lead reviews, merges and commits. You will not
be messaged mid-task; everything you need is in this brief and on disk.

THE LAW: the locked spec ${ROOT}/7-relay/README.md (v1.0, D32; §0 gives the path abbreviations M/ N/ NI/ ID/ A/ UI/ API RS/ LC/ L5:), the Layer 5 spec
${ROOT}/5-messaging/README.md (amendments 1-12 are in it, grep "Layer 7 amendment"), and the board ${ROOT}/7-relay/TASKS.md (the Wave 0 table: rows
M1, M4, M4b, M4c, M4d carry the as-built facts you build on; read the ones your brief names whole). Decisions: ${ROOT}/plan/DECISIONS.md D30, D31,
D32. Do not re-open decisions. If the spec is ambiguous, pick the reading that is safest (fail closed, lighter on the Pi floor) and record it
under "deviations". Owner rule in AGENTS.md: the Raspberry Pi is the floor: no resident process, poll, timer, listener or runtime dependency on a
non-hub node; on a hub, no work per item that grows with the rows held.

HOUSE RULES: Rust edition 2021, typed errors (never panic on input; sizes checked before parse), doc comments citing the spec section ("§x.y"),
unit tests in-module and integration tests under tests/, rustfmt + clippy -D warnings clean (CI runs clippy --all-targets -D warnings on both
workspaces); no new dependency (mesh111's set is final); no wall clock in any ordering or cryptographic decision; foreign bytes are untrusted
everywhere; never a model name or identifier in any file; no attribution lines; never commit; never touch plan/, the boards or the specs.

BOX: 4 CPUs, 15 GB RAM, a fixed disk allowance (about 20 GB free; do NOT cargo clean, do NOT create another target dir). Shared WARM target dir,
which cargo's own lock serializes between agents: mesh111 (run cargo from ${ROOT}/5-messaging) → CARGO_TARGET_DIR=${SP}/target/mesh111 · node111
(from ${ROOT}/1-bootstrap) → ${SP}/target/node111. Always export the right one with CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 and TMPDIR unset before
any cargo command. At most one cargo build of yours at a time; another agent builds the same crate from another tree at the same time, so a
build may wait on the lock: that is normal. Iterate with narrow commands (cargo test -p mesh111 --lib <module>, --test <name> <filter>) and run
the full check once at the end. Toolchain: cargo 1.97 stable; no musl cross toolchains (the musl stages of check.sh SKIP: say so, never fake
them). Scratch files under ${SP}/${id}/ only. Linux hygiene: nothing of yours stays running when you return (ps and ss check), delete your scratch.

RETURN (structured): task id, status (done | partial | blocked), files changed or created (exact paths), the check command and its tail (at most
25 lines), deviations (each with why), notes for the reviewer (what to read first, open questions, and the exact names other rows must use).
Factual; the lead reads diffs.
`
}

const T = {
  M1b: { model: 'opus', effort: 'xhigh', brief: (ROOT) => `TASK M1b: the store accounts custody bytes in O(1) per put (a fix on M1, row "| M1 |"; found while M4d was built).

THE DEFECT (confirmed by the lead in the code). Crate ${ROOT}/5-messaging/crates/mesh111 (M/ = its src/). Every forward-row put
(M/store/rows.rs Store::custody_put_fwd, about :1176) runs two full scans of the custody table before it inserts:
  (a) Store::forward_origin_bytes (rows.rs about :1477): SUM(length(bytes) + length(fwd_from) + overhead) WHERE fwd != 0 AND committed = 0 AND
      fwd_from = ?1, with no index that serves it;
  (b) Store::admit -> admit_inner (M/store/prune.rs about :867-890) -> class_bytes(Class::Transit) (prune.rs about :515): SUM over every forward
      row; Transit always has a quota (Store::quota, §5.8), so this runs on every forward put.
A member-row put runs class_bytes(Class::Custody) the same way whenever quota.custody is set. prune_inner (prune.rs about :721) calls
class_bytes inside its transaction too, and Store::forward_stats (rows.rs about :1460) is a third full scan. So a hub that holds n rows pays
O(n) per put and O(n^2) over a burst: seeding 24,000 forward rows through the gate did not finish in ten minutes on this box, and the forward
suite takes three minutes because of it. Work per item that grows with the rows held breaks the Pi-floor rule.

THE DESIGN (the lead's; implement it, do not redesign it; say under deviations where the code forced a different detail):
1. Schema v5, an APPEND-ONLY migration V5 in M/store/schema.rs (MIGRATIONS = [V1, V2, V3, V4, V5]; SCHEMA_VERSION = 5; the module doc's version
   list gains v5; V1-V4 are not edited by one byte):
   - a table custody_totals (key BLOB PRIMARY KEY, rows INTEGER NOT NULL, item_bytes INTEGER NOT NULL, from_bytes INTEGER NOT NULL) STRICT,
     WITHOUT ROWID, with CHECKs that the three counters are >= 0. Keys: x'00' = member rows (fwd = 0) · x'01' = forward rows (fwd != 0), every
     one · x'02' = protected forward rows (fwd != 0 AND committed = 0) · x'03' || fwd_from = the protected forward rows of that origin.
     rows counts rows, item_bytes sums length(bytes), from_bytes sums coalesce(length(fwd_from), 0). The per-row overhead constant (ROW_OVERHEAD)
     stays in Rust and is multiplied by rows at read time, so the stored totals never bake it in.
   - triggers on custody: AFTER INSERT, AFTER DELETE, and AFTER UPDATE OF fwd, fwd_from, committed, bytes; each takes OLD's contribution out of
     every key OLD counted under and adds NEW's to every key NEW counts under (UPSERT inside the trigger body; SQLite is the bundled SQLCipher of
     rusqlite 0.40). An origin key whose rows reach 0 is deleted by the trigger, so the table never grows with the origins ever seen. The triggers
     run inside the writing transaction, so a rollback takes the totals back with the rows.
   - V5 backfills custody_totals from the rows present in the same migration transaction (one grouped pass over custody), so a v4 store with
     member rows, forward rows, committed rows and several origins opens with exact totals.
2. Reads: class_bytes for Class::Custody and Class::Transit (both the free fn the prune transaction uses and Store::class_bytes),
   Store::forward_origin_bytes and Store::forward_stats read custody_totals by key (a primary-key lookup) and return EXACTLY what the old SUMs
   returned for the same rows (same byte rule: class bytes = item_bytes + from_bytes + rows * ROW_OVERHEAD; an origin's bytes the same over its
   protected rows; forward_stats = (rows, item_bytes) of key x'02'). Keep the old SUM queries only as a test oracle (cfg(test)), never on a
   production path. Nothing else about admission, quotas, shares, pruning order or eviction changes: same answers, same order, same errors.
3. The other classes are NOT yours to change (Layer 5 and 6 as built): but SURVEY them and report under notes, with file:line and when each
   runs in the default configuration: class_bytes(Class::Foreign), class_bytes for the bulk and message classes, live_bytes's SUM over blobs,
   and any other statement on a per-put or per-insert path of the store whose cost grows with the rows held (run EXPLAIN QUERY PLAN on the
   statements of custody_put_fwd, the mailbox fetch page, the forward head page and the prune candidate queries; list every SCAN).

TESTS (M/store/tests.rs, beside the existing v4 tests; use the existing helpers):
- an oracle helper (cfg(test)) that recomputes every total from the rows with the old SUM / GROUP BY queries and compares it with custody_totals
  (every key present, no stale key, no zero-row origin key); call it after every step of: member put, forward put for two origins, a duplicate,
  a share rejection, a quota rejection, forward_commit, forward_refuse, the member ack / advance path that deletes rows, a prune pass that evicts
  committed forward rows and one that evicts member rows, a failed transaction (totals unchanged), and a reopen of the store.
- the migration: build a v4 store with rows of every kind using migrate_with(&[V1..V4]) (the existing pattern for "a v4 store is refused by a
  v3 build"), then migrate to v5 and run the oracle; a v5 store is refused by a v4 build (the same pattern, one version up).
- the proof of O(1): EXPLAIN QUERY PLAN of every statement custody_put_fwd executes on the admission path (the dedup lookup, the origin total,
  the class total, the insert) contains no "SCAN custody" (assert on the plan text), and a put into a store already holding 5,000 forward rows
  executes no full-scan step on custody (rusqlite's statement status counters if this version exposes them; else the plan assertion is enough:
  say which). No timing assertion.
- update, do not delete, any existing test that pins user_version / SCHEMA_VERSION 4 or the migration list (grep the crate, the tests/ dir and
  ${ROOT}/1-bootstrap/crates/node111 for SCHEMA_VERSION, user_version and "v4").

NEVER ADDS: a wire byte, a pub API beyond what exists (Store::class_bytes, forward_origin_bytes and forward_stats keep their signatures), a
setting, a timer, memory that grows with the store, a change to what any quota or share admits or refuses.

FILES YOU OWN: M/store/schema.rs, M/store/prune.rs, M/store/rows.rs (forward_origin_bytes and forward_stats only, plus what the triggers make
dead), M/store/mod.rs, M/store/tests.rs; a pinned version number in an existing test elsewhere in mesh111 if one breaks. Another task (M4e)
adds ONE new read method at the very END of the impl block that holds forward_origin_bytes in M/store/rows.rs, and one test at the END of
M/store/tests.rs, in another tree: do not restructure or reorder those two files, keep your edits local to the functions named, so the lead's
merge is trivial. Do not touch M/fabric.rs, M/tests/forward.rs, the Layer 5 spec or any node111 file (if node111 pins the schema version, say
so under deviations with file:line: the lead changes it).

CHECK: from ${ROOT}/5-messaging: sh scripts/check.sh (fmt, clippy -D warnings, the whole suite; musl SKIP) green; then from ${ROOT}/1-bootstrap:
cargo check -p node111 (CARGO_TARGET_DIR=${SP}/target/node111). Report the forward suite's wall time before and after (it was about 150-190 s).` },

  M4e: { model: 'opus', effort: 'xhigh', brief: (ROOT) => `TASK M4e: the bounce on M4d (landed; read rows "| M4c |" and "| M4d |" whole, then M/fabric.rs's module doc paragraphs "The resident
window" and "Readings recorded for the lead", then Forwarder (struct, restart_probe, all_changed, cache_put, evictable), Core::track_forward,
up_targets, page_in, swap_in, run_forwarder, forward_pass). Crate ${ROOT}/5-messaging/crates/mesh111 (M/ = its src/).

THE FINDINGS of M4d's independent re-review (confirmed by the lead):
F1 (medium). Forwarder::cache_put restarts the swap probe from the paging floor on EVERY changed hit-cache entry. HitCache::put reports
   changed for any dest not in the 256-entry cache, so on a hub whose members talk to more than 256 dests every commit restarts the probe; and
   while probe_done is false Core::track_forward admits no new row directly (it leaves it to the probe). Scenario: 500,000 stored rows for a
   down pair D (about 2,000 pages), the window full of D's rows, pair C up, members sending to many dests through C. Each commit at C restarts
   the probe; the next row for C then waits for a sweep of all 2,000 pages (4 per pass, paced by the self-wake). Per tick the work is bounded;
   per commit it is O(rows stored), and the actor never goes idle.
F2 (low). The docs say a tick runs at most four passes (module doc; the FORWARD_PASSES doc). drain_out calls flush, hence run_forwarder, after
   each of up to DRAIN_ROUNDS rounds while the out-queue is non-empty, and Fabric::forward_take calls run_forwarder too: the true bound is
   FORWARD_PASSES per run_forwarder call, at most 1 + DRAIN_ROUNDS calls per tick plus the forward_take calls. A constant, but the stated one
   is wrong.
Coverage gaps the re-review listed: no test covers the probe restart on a hit-cache change, on a pair break or on a changed carrier bound; no
test shows the self-wake stops once the probe is done; and one path was not traced: can page_in read a FULL page in which every head is already
resident (rows above the floor can be resident: track_forward admits a new up-turn row directly), so that it inserts nothing, leaves run unset
and no self-wake follows, with rows still waiting in the store?

THE DESIGN (the lead's; implement it, do not redesign it; say under deviations where the code forced a different detail):
1. A changed hit-cache entry for dest restarts the probe ONLY when a stored row of that dest can have changed its turn: the window holds back
   stored rows (fwd.more) AND the store has a protected forward row of that dest above the paging floor that is not resident. Exact, by the
   store's index custody_by_dest (dest, custody_seq):
   - one new store read, pub(crate), added at the very END of the impl block that holds forward_origin_bytes in M/store/rows.rs (another task
     edits that file in another tree: add nothing else there and reorder nothing):
       forward_waits_for_dest(&self, dest: &[u8; 32], above: u64, resident: usize) -> Result<bool>
     true when MORE than resident protected forward rows (fwd != 0 AND committed = 0) of dest exist with custody_seq > above; it reads at most
     resident + 1 rows through the dest index (a LIMIT; show the plan in its unit test: SEARCH ... USING INDEX custody_by_dest, no SCAN). It
     must not depend on the schema version (the other task adds schema v5; your query touches only columns of v4).
   - resident = the count of the Forwarder's resident rows of that dest with seq > loaded (from by_dest; they are in the window, so the count is
     bounded by it). The decision needs the store, so it moves to the Core call site(s) of cache_put: Forwarder::cache_put keeps the cache
     update and the re-targeting of resident rows and returns whether the entry changed; Core then asks the store and calls restart_probe when
     the answer is true or when the store read fails (the conservative side). One bounded indexed read per CHANGED entry, none when fwd.more is
     false, none on a non-hub.
   - every other restart stays exactly as M4d built it (a target coming up, the pair set, a pair break, the carrier bound, the hub role).
   - the module doc and restart_probe's doc say it: "... or a hit-cache entry is added or changed while a stored row of that dest waits".
2. F2: state the true per-tick bound in the module doc and in the FORWARD_PASSES doc.
3. The page_in question: trace it. Either prove in a comment at the site that a full page of already-resident heads cannot leave rows waiting
   without a wake (name the invariant and where it is kept), or fix it (the read still moved the floor forward: a full page must set read_on /
   run so paging continues) and add a test that fails without the fix.

TESTS (M/tests/forward.rs with the existing harness; unit tests in fabric.rs where the harness cannot see the state; a cfg(test) counter of
probe restarts and of queued self-wakes on the Forwarder is allowed, never a production field):
- the F1 scenario scaled to the harness: the window full of a down pair D's rows with stored rows behind them and the probe done; pair C up;
  commits through C for more than FORWARD_CACHE distinct dests, none of which has a stored row: zero probe restarts, zero probe page reads
  after the first completion, and each new row is delivered without a sweep (assert on the counter and on the store reads you can observe);
- the positive case: a dest with stored rows gets its first hit at C: the probe restarts and those stored rows reach C before a newer row for C;
- a restart on a pair break and on a changed carrier bound (each moves a stored row whose turn it turned up);
- the self-wake stops: after the probe is done and nothing is left to read, no further self-wake is queued (the counter stays put across
  further unrelated inputs);
- the page_in case, as item 3 decides;
- forward_waits_for_dest in M/store/tests.rs (one test at the very END of that file): rows of two dests, member rows of the same dest
  (fwd = 0, not counted), committed rows (not counted), the above bound, resident = 0, 1, n, and the plan assertion.
The M4c and M4d tests stay green and unchanged (if one of M4d's tests relied on the restart-on-every-commit behaviour, keep its intent and say
what you changed).

NEVER ADDS: a timer, a task, a listener, a pub API on Fabric or Store (the new read is pub(crate)), a change to FORWARD_RESIDENT,
FORWARD_LOAD_PAGE, FORWARD_CACHE or FwdRow's size, memory that grows with the store, a store read on a non-hub.

FILES YOU OWN: M/fabric.rs, M/tests/forward.rs, and the two END-of-file additions in M/store/rows.rs and M/store/tests.rs named above. Nothing
else (not schema.rs, prune.rs, mod.rs, hub_flow.rs, the specs or node111).

CHECK: from ${ROOT}/5-messaging: sh scripts/check.sh (fmt, clippy -D warnings, the whole suite; musl SKIP) green, then cargo test -p mesh111
--test forward --test hub_flow once more (two green runs of those two). Before you return, turn your fix off once (restart on every changed
entry again) and show your F1 test fails, then restore it and say the file is back.` },
}

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

const REVIEW = {
  type: 'object',
  required: ['task', 'findings', 'sound', 'coverage_gaps'],
  properties: {
    task: { type: 'string' },
    findings: { type: 'array', items: { type: 'object', required: ['severity', 'verdict', 'where', 'defect', 'scenario', 'fix'], properties: {
      severity: { type: 'string', enum: ['high', 'medium', 'low'] },
      verdict: { type: 'string', enum: ['CONFIRMED', 'PLAUSIBLE'] },
      where: { type: 'string' }, defect: { type: 'string' }, scenario: { type: 'string' }, fix: { type: 'string' } } } },
    sound: { type: 'array', items: { type: 'string' } },
    coverage_gaps: { type: 'array', items: { type: 'string' } },
  },
}

const FOCUS = {
  M1b: `Verify: (1) the triggers are exact on every path that writes custody: enumerate every INSERT, UPDATE and DELETE on custody in the crate
(grep them all, including multi-row statements and the prune transaction) and show for each which trigger covers it and that OLD and NEW are
counted under the right keys (a committed flip moves a row out of x'02' and its origin key but not out of x'01'; a member row never touches
x'01'-x'03'; an UPDATE that changes none of the four columns changes no total); (2) the reads return exactly what the old SUMs returned (the
byte rule, the overhead, NULL fwd_from, an origin with no rows); (3) the V5 backfill is exact for a v4 store with rows of every kind, runs
inside the migration transaction, and V1-V4 are byte-identical to HEAD; (4) no production path still scans custody for a total; the prune
transaction reads totals that already reflect its own deletes; (5) a rollback (a failed statement in with_tx, a constraint failure inside a
trigger) cannot leave the totals and the rows apart; an origin key at zero rows is removed and a negative counter cannot be stored; (6) what a
store whose totals were ever wrong would do (is there any repair, and does anything depend on one); (7) the tests exercise the claims (the
oracle really recomputes from rows; the plan assertions would fail on a scan).`,
  M4e: `Verify: (1) the restart rule has no false negative: construct every way a protected forward row of a dest can be in the store and
not resident (left by track_forward with the window full, evicted, never paged in after a restart, left by the probe) and show a changed cache
entry for its dest restarts the probe; and no restart happens when no such row exists; the resident count passed to the store read is right
when resident rows of that dest sit both below and above the floor; (2) the store read is bounded (LIMIT, the dest index, fwd and committed
filtered) and is never reached on a non-hub or with fwd.more false; a store error restarts; (3) every other restart of M4d is unchanged (list
the call sites); (4) the page_in question is really closed (trace it yourself: a full page of already-resident heads); (5) the per-tick bound
now stated is the true one (count the run_forwarder call sites per tick); (6) no Layer 5 no-pair behaviour changed, no pub API, no constant or
FwdRow change, nothing resident unbounded; (7) the tests exercise the claims and the negative check the author reports is credible.`,
}

const review = (r) => {
  const ROOT = rootOf(r.id)
  return agent(`You are a read-only adversarial reviewer of one Rust change in ${ROOT} (the 111network mesh OS, crate 5-messaging/crates/mesh111). Do NOT
run cargo, do NOT edit any file, do NOT start any process; read only (cat/sed/grep/git diff). The change is task ${r.id}: the uncommitted
working-tree diff in that tree (cd ${ROOT} && git status -s && git diff --histogram -- 5-messaging; a new file has no diff: read it whole). Its
brief (read it: the design and the claims to verify) is in ${MAIN}/plan/layers/7-relay/runs/workflows/wave-0d.js under T.${r.id}. Spec:
${ROOT}/7-relay/README.md §5.1-§5.9 (the forward table, the quotas and shares of §5.8); the Layer 5 spec ${ROOT}/5-messaging/README.md §10.3,
§10.4 (schema, capacity). The author's own report (its declared deviations are NOT findings; report what it did not declare):
${JSON.stringify(r).slice(0, 7000)}

${FOCUS[r.id]}

Report only what you can show with a concrete path (file:line in the CURRENT file); CONFIRMED when traced end to end, PLAUSIBLE otherwise;
list what you checked and found sound and what is not covered. Set task to "${r.id}".`,
  { label: `L7:${r.id}-review`, phase: 'Review', schema: REVIEW, model: 'opus', effort: 'high' })
}

const ids = ['M1b', 'M4e'].filter(id => !ONLY || ONLY.includes(id))
const out = await pipeline(ids,
  id => agent(ctx(id) + '\n\n' + T[id].brief(rootOf(id)), { label: `L7:${id}`, phase: 'Fix', schema: RESULT, model: T[id].model, effort: T[id].effort })
    .then(r => r && { ...r, id }),
  r => (r && r.status !== 'blocked') ? review(r).then(v => ({ ...r, review: v })) : r,
)
log('wave 0d: ' + out.filter(Boolean).map(r => `${r.id}=${r.status}${r.review ? ' (' + r.review.findings.length + ' findings)' : ''}`).join(' · '))
return out.filter(Boolean)
