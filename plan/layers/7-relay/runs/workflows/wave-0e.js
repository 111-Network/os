export const meta = {
  name: 'layer-7-wave-0e',
  description: 'Layer 7 Wave 0e: M1c (the store: no pass over the rows held on any per-item path: partial indexes in v5, custody_stats from the totals, a prune pass that builds nothing when nothing is over, the self-heal at open, the M1b review items) and M4f (the M4e bounce: an entry the hit cache drops is judged like a changed one), each followed by an independent read-only review; the lead merges, re-runs the gates and commits',
  phases: [
    { title: 'Fix', detail: 'M1c in the main tree (store/*), M4f in the M4e git worktree (fabric.rs, tests/forward.rs)' },
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
M1, M1b, M4, M4b, M4c, M4d, M4e carry the as-built facts you build on; read the ones your brief names whole). Decisions: ${ROOT}/plan/DECISIONS.md D30, D31,
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
  M1c: { model: 'opus', effort: 'xhigh', brief: (ROOT) => `TASK M1c: the store does no pass over the rows it holds on any per-item path (the follow-up of M1b, landed: read row "| M1b |" and the
M1b code first: M/store/schema.rs V5, M/store/prune.rs CustodyTotal / TotalKey / custody_total, the tests section "Layer 7 §5.8 (M1b)" in
M/store/tests.rs). Crate ${ROOT}/5-messaging/crates/mesh111 (M/ = its src/). M1b's own survey and its independent review found what is left;
the lead confirmed each in the code. Fix exactly these:

1. Store::custody_stats (M/store/rows.rs about :1326) is a full SCAN of custody, and it runs on EVERY accepted put on a hub: Fabric::status
   (M/fabric.rs about :5116) calls it, node111's publish_fabric calls status for every event batch that carries FabricEvent::Fabric, and
   custody_after raises that event on every accepted member or forward put. Answer it from custody_totals (the member and forward keys):
   the same numbers as today for the same rows, two primary-key reads. If today's numbers cannot be derived from the totals exactly (read
   what it counts: rows and which bytes), extend what the triggers keep rather than approximate, and say so.
2. The capacity pass. prune_inner (M/store/prune.rs about :755-930) builds EVERY candidate list inside a write transaction on every call,
   even when nothing is over: evictable (a SCAN of custody for committed rows, no index), superseded, foreign_inbound, the blob lists, the
   message lists. node111 calls it through capacity_pass every 60 s on every node that has a fabric (1-bootstrap/crates/node111/src/daemon/
   fabric/mod.rs PRUNE_EVERY x housekeeping), and admit_inner calls it on every put while a quota is hit, the store is over the high
   watermark, the disk is under its floor or store_full is set. Make a pass whose inputs show nothing to do (no quota exceeded counting the
   incoming item, live bytes plus the incoming item not over the high watermark, the disk not under its floor, store_full not set, no export
   offer owed: derive the exact condition from the function, do not trust this list) return the same PruneReport, flags and persisted state
   as today WITHOUT building a candidate list and without a write transaction; and when a pass must prune, build only the lists the steps it
   reaches need, in the same normative order (§10.4), so the outcome (what is pruned, in what order, the full / store_full flags, the export
   offer, the fits answer) is byte-identical to today's for every store. Every existing prune test stays green UNCHANGED: they are the oracle.
3. Partial indexes, added to V5 (V5 is unreleased: it landed on this branch today and no store in the field has it; amend its text, keep
   V1-V4 byte-identical) so the remaining per-item statements search and never scan. Candidates (add one ONLY if EXPLAIN QUERY PLAN shows it
   turns a SCAN or a filtered range walk into a bounded SEARCH for a statement on a per-item path; report each plan before and after, and the
   VM-step cost of one member put and one forward put before and after, since every index is a write on an SD card):
   - custody (dest, custody_seq) WHERE committed = 0: for custody_commit (rows.rs about :1285, a member's MAILBOX_COMMIT re-reads every
     already-committed row of that member today) and custody_floor (about :1313);
   - custody (custody_seq) WHERE fwd != 0 AND committed = 0: for the forward pages forward_pending and forward_pending_lens (about :1341,
     :1365: a rowid range filtered afterwards, which walks every member and committed row in between);
   - an index that serves evictable's read of committed rows in the order prune needs (read sort_by_weight first: if the order is computed
     in Rust over all candidates, bound the read instead: prune needs only as many candidates as bytes to free; say what you did).
   Rewrite a statement only as far as the planner needs to pick the index (the literal predicate must imply the index's WHERE); results
   identical.
4. The self-heal at open (M1b review finding 2): nothing re-creates the v5 objects if they are ever missing, and a drifted total wedges
   eviction on a CHECK. At open, after migrate, one cheap read of sqlite_master (a handful of rows, no scan of custody): if any v5 trigger or
   index is missing, recreate it, and if a trigger was missing rebuild custody_totals from the rows (the V5 backfill as a reusable
   crate-private function, one transaction). A store that is whole pays nothing beyond that read. Test: drop a trigger, write rows, reopen:
   the totals are exact again and the trigger is back; drop an index, reopen: it is back.
5. M1b review findings 1 and 3: make the module docs say exactly what is true after this task (which paths are constant, which are bounded
   by what; no blanket claim); in the_admission_path_never_scans_custody drop the vacuous EXPLAIN of the INSERT statements (their trigger
   programs do not show there) or say the counters cover them; add status-counter checks (FullscanStep == 0, VM steps equal at few and many
   rows and at few and many origins) for one forward_commit, one custody_commit, one prune delete and one capacity pass with nothing over.
6. A question to TRACE and answer with a test, fixing only if the specs are unambiguous: custody_commit commits every row of a dest below
   the member's cursor with no fwd filter. Can a protected forward row (fwd != 0) exist for a dest that is, or later becomes, a member of
   this hub (read the gate in M/fabric.rs that chooses Fwd for an incoming item, §5.1-§5.3 of the Layer 7 spec, and custody_fetch: does a
   member's mailbox page include forward rows of its dest)? If a member's MAILBOX_COMMIT can commit a forward row it was never given, that
   loses custody: add the fwd = 0 filter and a test. If the member IS given those rows (the home hub delivers them from its mailbox), the
   commit is right: write the test that pins it and say so. Report which it is under notes.
7. Leave and REPORT (notes, with the cost model): live_bytes's SUM over blobs on every admission (Layer 8 owns the object store), and
   whatever else on a per-item path still grows with the rows held after your changes.

Do NOT touch the END of the impl block in M/store/rows.rs or the END of M/store/tests.rs (another task appends one read,
Store::forward_waits_for_dest, and its test there, in another tree; the lead merges it after you). Do not restructure or reorder those files.

NEVER ADDS: a wire byte, a pub API beyond what exists (signatures unchanged), a setting, a timer, memory that grows with the store, a change
to what any quota, share, watermark or prune order admits, refuses or evicts.

FILES YOU OWN: M/store/schema.rs, M/store/prune.rs, M/store/rows.rs (the functions named), M/store/mod.rs, M/store/tests.rs. Nothing in
M/fabric.rs, the tests/ dir, the specs or node111 (if the 60 s capacity pass in node111 needs a change, describe it under deviations).

CHECK: from ${ROOT}/5-messaging: sh scripts/check.sh (fmt, clippy -D warnings, the whole suite; musl SKIP) green; from ${ROOT}/1-bootstrap:
cargo check -p node111 (CARGO_TARGET_DIR=${SP}/target/node111). Report: every plan before and after, the VM steps of a member put and a
forward put before and after, the cost of a capacity pass with nothing over at 10 and at 5,000 rows held, and the forward suite's wall time.` },

  M4f: { model: 'opus', effort: 'xhigh', brief: (ROOT) => `TASK M4f: the bounce on M4e. Your tree ALREADY HOLDS M4e's uncommitted change (git status shows M/fabric.rs, M/tests/forward.rs,
M/store/rows.rs, M/store/tests.rs modified against HEAD): build on it, never revert it. Read M4e's brief (${MAIN}/plan/layers/7-relay/runs/
workflows/wave-0d.js, T.M4e), then in M/fabric.rs: HitCache (put, forget_pair), Forwarder::cache_put and resident_above_floor,
Core::cache_changed and its call in on_forward_ack, forward_targets / first_target (how the cache ranks a pair), track_forward, swap_in.
Crate ${ROOT}/5-messaging/crates/mesh111 (M/ = its src/).

THE FINDING of M4e's independent re-review (medium, confirmed by the lead; the author raised it as an open question too): an entry the hit
cache DROPS also changes the turn of that dest's rows. HitCache::put evicts the oldest entry once the cache holds FORWARD_CACHE entries and
reports only whether the NEW dest's entry changed. A stored row whose turn was the dropped entry's pair P falls back to the probe order,
which can name an up pair C. Under M4d every changed put restarted the probe, so the drop was covered by accident; with M4e's exact rule the
dropped dest is never checked: its stored rows are not judged again (they wait for an unrelated restart, possibly for ever), a newer row of
that dest is admitted directly to C ahead of them (the documented rule "stored rows move before a newer row for the same target" breaks),
and the dest's RESIDENT idle rows keep the turn P as well, because nothing marks them dirty (that part is older than M4e).

THE DESIGN (the lead's; implement it, say under deviations where the code forced a different detail):
1. HitCache::put reports what it dropped: the dest of each entry it evicted for capacity (at most one per put while the cache is full;
   prove the bound or return a small fixed-size value). Forwarder::cache_put returns both facts (the new dest's entry changed; the dropped
   dest, if any) and marks the dropped dest's resident idle rows for recomputation exactly as it does for a changed dest (their cached turn
   is gone: the next pass recomputes it; a row P refused keeps its refusal).
2. Core::cache_changed runs for the dropped dest as for a changed one: the same guards (a hub, fwd.more), the same exact store read
   (Store::forward_waits_for_dest with that dest's resident count above the floor), the same restart when a stored row of it waits, the
   same conservative restart on a store error. At most two bounded reads per commit.
3. HitCache::forget_pair drops every entry naming a pair: list its callers and show each already restarts the probe and recomputes every
   idle row (all_changed); if one does not, fix it the same way.
4. Docs: the module-doc paragraph "The resident window" and restart_probe's doc name the dropped entry beside the changed one.
5. The integration test a_cache_entry_for_a_dest_with_no_stored_row_never_restarts_the_probe asserts spent < 3N + S, which one extra
   legitimate restart or a small change in computations per row would break: assert on what the test means instead (the cfg(test) restart
   counter through a test-only accessor if the harness can reach it, or a bound with real headroom derived in a comment), keeping its power
   to fail when the fix is turned off.

TESTS: (a) unit, in fabric.rs: a put that evicts X's entry returns X; X's resident idle rows are recomputed and move from P (down) to the
probe order's C (up); (b) integration or unit, whichever can build it deterministically: X's stored rows have turn P (down, through the
cache entry X -> P), the probe is done, the cache churns past FORWARD_CACHE through C with dests that have no stored row: X's entry is
dropped, the probe restarts once for X, X's stored rows reach C, and a newer X row is NOT admitted ahead of them; (c) the churn with no
stored row of any dropped dest still restarts nothing (M4e's F1 property holds with the extra read); (d) the forget_pair callers as item 3
decides. All M4c, M4d and M4e tests stay green; change none of their assertions except item 5's bound.

NEVER ADDS: a timer, a task, a listener, a pub API on Fabric or Store, a change to FORWARD_RESIDENT, FORWARD_LOAD_PAGE, FORWARD_CACHE or
FwdRow's size, memory that grows with the store, a store read on a non-hub or while fwd.more is false.

FILES YOU OWN: M/fabric.rs, M/tests/forward.rs. Leave M4e's two END-of-file additions in M/store/rows.rs and M/store/tests.rs as they are
(another task edits those files in the main tree; touch nothing else there).

CHECK: from ${ROOT}/5-messaging: sh scripts/check.sh (fmt, clippy -D warnings, the whole suite; musl SKIP) green, then cargo test -p mesh111
--test forward --test hub_flow once more. Before you return, turn your fix off once (ignore the dropped dest) and show test (b) fails, then
restore it and say the file is back.` },
  M4g: { model: 'opus', effort: 'xhigh', brief: (ROOT) => `TASK M4g: the bounce on M4f and the M1c review's lows (M4e + M4f LANDED as e733307, M1c LANDED: read rows "| M4e |", "| M4f |", "| M1c |" and
"| M4g |" of ${ROOT}/7-relay/TASKS.md whole, then M/fabric.rs (Core::cache_changed, Forwarder::cache_put, HitCache::put, on_forward_ack, retarget,
page_in, restart_probe, pair_link_down) and M/store/{rows,schema,prune}.rs as they now are). The independent re-review of M4e + M4f confirmed:

F1 (medium) Store::forward_waits_for_dest (the last item of impl Store in M/store/rows.rs) is bounded in the rows it RETURNS (LIMIT resident + 1),
not in the rows it VISITS: it runs through custody_by_dest (dest, custody_seq), which holds neither fwd nor committed, so SQLite steps over every
committed row and every member row of that dest above the floor and fetches the table row behind each to test fwd and committed. In the very state
the rule exists for (the window full of a down pair's rows, the floor standing still, probe done) each new up-turn row is admitted, committed and
left as a committed = 1 row above the floor until a capacity prune, so a dest's committed history above the floor grows with traffic, and M4f made
the read run up to twice per commit. The fabric.rs docs (the module doc and cache_changed) claim "bounded reads, never a sweep": false today.
  FIX: the read must be bounded in rows visited. M1c added partial indexes to schema v5 (read V5 and the heal at open in M/store/schema.rs for the
  exact names and predicates: custody_open_by_dest ON custody (dest, custody_seq) WHERE committed = 0 is the one that fits; custody_forward_open
  is (custody_seq) WHERE fwd != 0 AND committed = 0). M1c ALREADY moved this statement's plan to custody_open_by_dest and re-pinned its plan
  assertion (the planner picked the narrower index once it existed): confirm it, then do what is still owed. Shape the statement so the planner SEARCHes a partial index whose predicate leaves the
  committed rows out (the WHERE clause must imply the index predicate as SQLite's planner matches it: write committed = 0 literally), and decide
  with evidence what is left to step over: with custody_open_by_dest it is the uncommitted MEMBER rows (fwd = 0) of the same dest above the floor.
  Trace when a dest can hold both member rows and protected forward rows on one hub (a member that enrolled or left while rows were held; am. 11
  the former member's rows) and what bounds that set. If it is bounded by a constant or by the window, say so in the doc with the path; if it is
  bounded only by a quota (it grows with the rows held), make it exact instead: a partial index ON custody (dest, custody_seq) WHERE fwd != 0 AND
  committed = 0 in V5 (and in M1c's heal list, so a store created before it gains it at open), replacing custody_open_by_dest ONLY IF you show no
  other statement needs that one (grep every plan assertion and statement M1c wrote that names it), and state the write cost (one index entry per
  uncommitted forward row: an insert at the put, a delete at the commit). Do not bump the schema version: v5 is unreleased; M1c's heal covers stores
  made by an earlier v5.
  PROVE IT: (a) a plan assertion in M/store/tests.rs in the style M1b/M1c use (EXPLAIN QUERY PLAN: SEARCH ... USING INDEX <the partial index>, no
  SCAN); (b) a test with teeth on the rows visited: one dest with R resident-equivalent protected rows and K committed forward rows plus K member
  rows of that dest above the floor, for K = 10 and K = 2,000; the statement's SQLITE_STMTSTATUS_VM_STEP (rusqlite Statement::get_status with
  StatementStatus::VmStep; reset between runs; M1c's status-counter checks show the pattern) must not grow with K beyond a small constant, and
  FULLSCAN_STEP must be 0. Turn the fix off once (the old statement through custody_by_dest) and show the test fails; report the two step counts.
  Then correct the fabric.rs docs (module doc, cache_changed, forward_waits_for_dest's own doc) to the real bound, in the file's voice.

F2 (low) Forwarder::cache_put, the CHANGED branch (r.turn != Some(pair) && !r.refused.contains(&pair)): a resident Idle row of the dest whose turn
came from the OLD entry is not marked when the new pair is one it refused, so it keeps a turn the new list would not give it. The review's
sequence: pairs in probe order [a, q, p], all tcp; row R of dest X with no hint. (1) X → p is cached, R is offered to p, p answers
REJECT_UNKNOWN_DEST: R.refused = [p], its turn is a. (2) another X row commits at q: X → q (changed); R is marked and its turn becomes q. (3) p
accepts another X row (one hinted to p): X → p (changed); R is not marked, yet retarget would give [p, a, q] → first unrefused = a. If q then goes
down while a is up, R waits on q.
  FIX: in the changed branch mark every Idle row of the dest whose turn != Some(pair), refused or not (retarget keeps the refusal; the cost stays
  bounded by the dest's resident rows). The DROP branch is exact as it is (the review traced it): leave it. Add a unit test that drives exactly the
  sequence above through the real paths (the M4e/M4f unit tests in fabric.rs show the harness) and asserts R's turn is a after step 3 and that R's
  refusal of p is still recorded; turn the fix off once and show it fails.

GAPS the review listed (close each, or say precisely why it cannot be closed without a pub API or a production-only seam):
  G1 a store error inside Core::cache_changed restarts the probe and counts an error: no test injects one. Find the cheapest honest injection
     (a #[cfg(test)] hook on the store handle the fabric holds, or a store whose statement fails for a reason a test can cause: read how the existing
     fabric tests make a store call fail, if any do). No production-visible seam.
  G2 no test has BOTH the hit's dest and the dropped dest holding stored rows outside the window: assert exactly one probe restart for the commit
     and, with a #[cfg(test)] counter of forward_waits_for_dest calls on the core, at most two reads for the commit and exactly one once the first
     restarted the probe.
  G3 a row that is InFlight or Full at pair p when its dest's entry X → p is dropped or changed is not marked (only Idle rows are); it returns to
     Idle later (pair_link_down sets Idle; a REJECT_FULL row leaves Full when its pair has room; an ack timeout). TRACE every path that returns a
     row to Idle and say, with file:line, whether the row's turn is recomputed there (is it marked dirty, does retarget run for it before the next
     offer). The author of M4f held that a row keeps its pair like a changed-dest row and that C16 (a REJECT_FULL row stays with its pair) wants
     that. If every path recomputes, add one test that pins it for the dropped-entry case; if a path keeps a turn that only the vanished entry
     justified AND that can strand the row (its pair down, another target up, nothing judging it again), fix it at the point the row returns to
     Idle (mark it dirty: bounded, one row) and test it; C16's rule for a Full row whose pair is up stays.

ALSO, the independent review of M1c (landed; four lows, all in the store files you already own here):
  S1 the cfg(test) oracle prune_eager in M/store/prune.rs is described as the pass as it was before M1c, but it builds its lists with the NEW
     step_list, the new evictable SQL and the new Ranked/Head ordering, and the test helper sorted uses Ranked too: on list construction and
     ordering the random-store test compares the new pass with itself. FIX: the oracle carries a verbatim copy of the pre-M1c comparator and
     evictable-custody statement (git show 4b9e521:5-messaging/crates/mesh111/src/store/prune.rs: sort_by_weight and the step-1 custody read)
     so a later change to Ranked::cmp or EVICTABLE_CUSTODY_SQL is caught; show it by flipping the local_seq tie-break once (the test fails).
  S2 schema::heal runs CREATE INDEX for a missing index inside Store::open and any error fails the open: on a nearly full disk SQLITE_FULL
     then stops the fabric from starting, exactly when the store is under pressure. A missing index changes no fact. FIX: a failed index
     re-create is not fatal (roll that statement back, count it, carry on: the statements then run on the wider index or the table); a missing
     trigger or totals table stays fatal with its rebuild, since it guards correctness. Test both.
  S3 the forward_pending plan is asserted on a hand-written copy of the column list: build the asserted text from rows::FORWARD_COLS (pub(super))
     so the EXPLAIN always runs on the real statement.
  S4 V5_PRESENT_SQL repeats V5_OBJECTS' names by hand: derive one from the other, or assert they agree in a test.
  S5 custody_total returns zeros for a missing fixed key, so custody_stats would under-report silently: a missing fixed key is a heal case
     (re-create and rebuild) or an error, never a silent zero; pick the one that fits heal's design and test it.

NEVER ADDS: a timer, a task, a listener, a pub API (pub(crate) and #[cfg(test)] only), a constant or FwdRow change, a store read on a non-hub or
with fwd.more false, a pass over the rows held on any per-item path, a schema version bump.

FILES: M/fabric.rs, M/store/{rows,schema,prune,mod,tests}.rs, M/tests/forward.rs (only if an integration assertion needs it). Nothing else.

CHECK: cd ${ROOT}/5-messaging && sh scripts/check.sh (musl stages SKIP); then cargo test -p mesh111 --test forward --test hub_flow a second time
(the forward suite takes 3-6 minutes: other agents share the CPUs; a timing-only failure under load is re-run once alone before you call it a
defect); then cd ${ROOT}/1-bootstrap && cargo check -p node111 with its own target dir. Report: the plan line, the two step counts with the fix on
and off, the F2 test failing with its fix off, each gap closed or the precise reason it is not.` },
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
  M1c: `Verify: (1) custody_stats returns exactly today's numbers from the totals for stores with member rows, forward rows, committed rows
and none; (2) the capacity pass: construct the exact condition under which the old function pruned or changed any persisted state (flags,
the export offer, full_classes, store_full) and show the new early return is taken ONLY when the old function would have changed nothing and
returned the same report; a pass that must prune reaches the same items in the same order (compare the candidate construction and the
steps line by line with HEAD: git diff --histogram); no existing prune test changed; (3) each new index is used by the statement it was
added for (the plans in the tests are of the real statements' text, not of a copy that can drift) and no statement changed its result set
or order; the write cost per put is reported; V1-V4 are byte-identical to HEAD; (4) the self-heal: the open-time read is O(1), a missing
trigger leads to a rebuild inside one transaction, a whole store is untouched, and a store opened read-only or by an older build is
handled; (5) the custody_commit question is answered from the gate and the specs, with a test that pins the answer; (6) nothing on a
per-item path still scans custody (enumerate every statement on custody in the crate and its plan); (7) the tests would fail if a scan came
back (the counters are real: trigger subprograms count on the parent statement).`,
  M4f: `Verify (the diff in this tree holds M4e AND M4f: review the whole of it, with M4e's brief and review in mind): (1) the restart rule
now has no false negative: enumerate every way a cache entry can change or vanish (a new dest, another pair for a dest, the capacity
eviction, forget_pair, a restart of the daemon, hub role off and on) and show for each that the stored rows AND the resident idle rows of
the affected dest are judged again; (2) at most two bounded store reads per commit, none on a non-hub or with fwd.more false; a store error
restarts; (3) a row a pair refused keeps its refusal through the recomputation; an evicted-then-re-added entry does not loop (a restart
storm between two dests alternating in a full cache: bound it); (4) the tests exercise the claims and the reported negative check is
credible; the loosened bound of the M4e integration test still fails when M4e's fix is off; (5) every other restart site of M4d is
unchanged; (6) no Layer 5 no-pair behaviour changed, no pub API, no constant or FwdRow change, nothing resident unbounded.`,
  M4g: `Verify: (1) forward_waits_for_dest now runs as a SEARCH on a partial index that leaves committed rows out; derive from the schema and the
statement (not from the author's report) what it can still step over and whether that is bounded by the window or a constant; the plan assertion
and the step-counter test have teeth (they fail on the old statement; K really differs between the two runs; the counter is reset); if an index
was added or replaced in V5, the heal at open creates it on a store made by an earlier v5, every statement that used the replaced index still has
its plan asserted, and the write cost is stated; no schema version bump; (2) the fabric.rs docs now state the real bound; (3) the changed branch
of cache_put marks every Idle row of the dest whose turn is another target and the refusal survives; the unit test drives the review's three-step
sequence through real paths; no restart storm or recompute loop is opened by the wider marking (two dests or two pairs alternating: bound it);
(4) each gap G1-G3 is closed by a test that exercises the real path, or the reason given is true; for G3 re-trace every path that returns a row
to Idle yourself; (5) every other restart site and the M4d/M4e/M4f behaviour is unchanged; no Layer 5 no-pair behaviour changed, no pub API, no
constant or FwdRow change, nothing resident unbounded, no per-item pass over the rows held.`,
}

const review = (r) => {
  const ROOT = rootOf(r.id)
  return agent(`You are a read-only adversarial reviewer of one Rust change in ${ROOT} (the 111network mesh OS, crate 5-messaging/crates/mesh111). Do NOT
run cargo, do NOT edit any file, do NOT start any process; read only (cat/sed/grep/git diff). The change is task ${r.id}: the uncommitted
working-tree diff in that tree (cd ${ROOT} && git status -s && git diff --histogram -- 5-messaging; a new file has no diff: read it whole). Its
brief (read it: the design and the claims to verify) is in ${MAIN}/plan/layers/7-relay/runs/workflows/wave-0e.js under T.${r.id}. Spec:
${ROOT}/7-relay/README.md §5.1-§5.9 (the forward table, the quotas and shares of §5.8); the Layer 5 spec ${ROOT}/5-messaging/README.md §10.3,
§10.4 (schema, capacity). The author's own report (its declared deviations are NOT findings; report what it did not declare):
${JSON.stringify(r).slice(0, 7000)}

${FOCUS[r.id]}

Report only what you can show with a concrete path (file:line in the CURRENT file); CONFIRMED when traced end to end, PLAUSIBLE otherwise;
list what you checked and found sound and what is not covered. Set task to "${r.id}".`,
  { label: `L7:${r.id}-review`, phase: 'Review', schema: REVIEW, model: 'opus', effort: 'high' })
}

// args.reviewOnly = { <id>: <the author's returned result> }: only the read-only review runs (a review that died, the author's work on disk).
if (a.reviewOnly) {
  const rs = Object.entries(a.reviewOnly).map(([id, r]) => ({ ...r, id }))
  const vs = await parallel(rs.map(r => () => review(r).then(v => ({ ...r, review: v }))))
  return vs.filter(Boolean)
}
// args.extra = { <id>: '<text>' }: a lead note appended to that row's brief (e.g. an interrupted attempt whose edits are on disk).
const note = (id) => (a.extra && a.extra[id]) ? `\n\nLEAD NOTE (read first): ${a.extra[id]}` : ''
const ids = ['M1c', 'M4f', 'M4g'].filter(id => !ONLY || ONLY.includes(id))
const out = await pipeline(ids,
  id => agent(ctx(id) + '\n\n' + T[id].brief(rootOf(id)) + note(id), { label: `L7:${id}`, phase: 'Fix', schema: RESULT, model: T[id].model, effort: T[id].effort })
    .then(r => r && { ...r, id }),
  r => (r && r.status !== 'blocked') ? review(r).then(v => ({ ...r, review: v })) : r,
)
log('wave 0e: ' + out.filter(Boolean).map(r => `${r.id}=${r.status}${r.review ? ' (' + r.review.findings.length + ' findings)' : ''}`).join(' · '))
return out.filter(Boolean)
