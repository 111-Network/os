export const meta = {
  name: 'test-system-build',
  description: 'Cross-layer test system (D36, D37): build rows of plan/cross-layer/test-system/TASKS.md — X2 the runner core first, then X1 (the seam map and check catalog), X3 (the adapters) and X4 (lint, matrix, changed, explain) in parallel; each row followed by an independent read-only review; the lead runs it for real and commits',
  phases: [
    { title: 'Core', detail: 'X2: verify/run.py and the t111 package core, the map schema, hermetic unit tests' },
    { title: 'Map, adapters, lint', detail: 'X1 · X3 · X4 on disjoint files' },
    { title: 'Later rows', detail: 'any other row named in args.only (X5-X9, N1-N11)' },
    { title: 'Review', detail: 'one read-only adversarial pass per row' },
  ],
}

const ROOT = '/home/user/os'
const SP = '/tmp/claude-0/-home-user-os/f8991dd8-132d-5f0d-807d-446e0e7eb394/scratchpad'
const TS = `${ROOT}/plan/cross-layer/test-system`
const a = args || {}
const ONLY = Array.isArray(a.only) ? a.only : null

const CTX = `
You are a BUILD subagent for the 111network OS (repo ${ROOT}, branch claude/confident-ramanujan-y5ob49), working on the cross-layer test
system the owner asked for (plan/DECISIONS.md D36: "a solid test system for this code base that helps AI to debug the connection between
layers"; D37: the design decisions). Today is 2026-11-11. You NEVER run git add/commit/stash/checkout/reset: the lead reviews and commits.
You will not be messaged mid-task; everything you need is in this brief and on disk.

THE LAW: the design brief ${TS}/options.md (read it whole first: sections 0-3 are the design, section 4 the new seam checks, section 6 the
rows; every recommendation marked "(pick)" is decided), your row in the board ${TS}/TASKS.md (the row "| <id> |": Task, Files (owned),
Check; the header's lead additions bind you), the seam x evidence matrix ${TS}/matrix.md and the survey reports ${TS}/research/01-05 (facts
with file:line about every existing gate and harness: read the ones your row touches). The repo rules in AGENTS.md apply: adopt and extend
what exists, never reinvent; the Raspberry Pi floor (the test system lives in the repo only and is never installed on a node); Linux
hygiene (nothing of yours stays running; scratch outside the repo).

HOUSE RULES: Python >= 3.11, the standard library ONLY (no pip install, no third-party import, no network), every entry point runs under
python3 -I; type hints and short docstrings; no global mutable state beyond what a module documents; subprocesses always with an explicit
argv list (never shell=True with interpolated text), an explicit cwd, an explicit timeout and their own session; every path the code writes
is under the caller's --scratch directory; no wall-clock value inside any pass/fail decision (durations are for the record only). Shell
edits: POSIX sh as the file already is, shellcheck clean (/usr/local/bin/shellcheck). Text a harness prints today stays byte-identical
unless your row says otherwise; you never weaken, skip or delete an existing assertion. No model names or tool vendor identifiers in any
file, no attribution lines. FILE OWNERSHIP IS STRICT: create or modify only the files your row's Files column lists (and new files inside
directories it names); if you need a change elsewhere, describe it under "deviations" and continue. Never touch plan/ boards, the specs or
the decision log.

BOX: 4 CPUs, 15 GB RAM. Other agents are compiling Rust in this tree right now (mesh111, node111) and editing the UI: do NOT run cargo
builds, the cargo test suites, pnpm, or any namespace harness (smoke.sh, panel-lint.sh, the */verify/*.sh gates) unless your row's Check
says so explicitly; your unit tests must be hermetic (fake checks made of tiny sh or python scripts written into a temp dir). You run as
root (euid 0) on this box: unprivileged user namespaces work, ip is installed. Scratch under ${SP}/<task-id>/ only; delete it when done.
Nothing of yours is running or listening when you return (ps and ss check).

RUN YOUR CHECK (the row's Check column, refined in your brief) until it is green; paste its tail.

RETURN (structured): task id, status (done | partial | blocked), files changed or created (exact paths), the check command and its tail
(at most 25 lines), deviations (each with why), notes for the reviewer (what to read first, open questions, and the exact names other rows
must use). Factual; the lead reads diffs.
`

const T = {
  X2: { model: 'opus', effort: 'high', phase: 'Core', brief: `TASK X2 (row "| X2 |"): the runner core. Build ${ROOT}/verify/run.py (the entry point, a thin main) and the package
${ROOT}/verify/t111/ with the core of options.md sections 3.1-3.3 and 3.5's data model. Other rows build on your interfaces right after you
(X1 writes the real seam map and check catalog, X3 the adapters, X4 the lint / matrix / changed / explain commands), so the interfaces are
your first deliverable:
- t111/schema.py: the ONE definition of the map's data model (dataclasses for seam, check, fake, known, unmapped; the TOML field names
  and enums exactly as options.md 3.5 and 3.2 give them: seam id / status built|stub|planned / spec / a / b {layer, files} / read_first /
  symptom / gap; check id / kind gate|unit / cmd (argv list) / cwd / tier fast|slow|ns / layers / seams / phases (phase prefix -> seam
  ids) / adapter / cargo (bool) / lock / timeout_s / env / rerun (a template) / rerun_cost / covered_by / strength R|I|F|C|N / requires
  (ids of checks that must have passed first) / needs (tools or capabilities: e.g. "userns", "ip", "shellcheck", "network") / offline_skip),
  a loader for verify/seams.toml and verify/checks.toml (tomllib) that reports every schema error with file, table index and field, and a
  docstring per field: this file is the schema's documentation.
- t111/adapters.py holds ONLY the adapter registry and the one trivial adapter "exit" (exit code 0 -> one PASS record, non-zero -> one
  FAIL record carrying the last 40 lines of the log): the registry's contract (name -> a function taking the check, the path of the
  combined ordered log, the exit status and a context object, returning the list of result records) is what row X3 fills with the real
  adapters; leave that file small and say in its docstring that X3 owns the rest.
- t111/select.py (selection by --layer, --seam, --check, --tier, --gate; a unit is replaced by its covering gate in a full run so no
  suite runs twice; requires-ordering), t111/locks.py (the cargo slot locks and the exclusive verify lock via fcntl.flock in
  \${T111_LOCKS:-/tmp/111-verify-locks}; --cargo-slots 1|2, never more than two), t111/run.py (one check = one subprocess session, the
  combined ordered log, the timeout: TERM to the group then KILL after 10 s, TMPDIR and T111_SCRATCH pointed under the check's scratch
  dir, the environment a check declares), t111/hygiene.py (the listener / process snapshot before and after a check: anything whose
  cmdline or environ names the scratch dir, or whose binary is one of ours, becomes a hygiene/<check> FAIL naming pid and cmdline and is
  killed; a tree-dirtied-during-a-check FAIL naming the files, comparing git status --porcelain before and after; the port pre-check of
  3.2 from /proc/net/tcp and tcp6; the capability probes behind "needs": userns, ip, tools), t111/record.py (the record of 3.3: one run
  record then one check record per result, written as JSON lines to <scratch>/<run-id>/results.jsonl; the read_first rule; the "after"
  field for a failure that follows an earlier failure of the same check), t111/summary.py (summary.txt and stdout exactly in the form of
  3.3; the last line VERIFY GREEN | VERIFY GREEN (n SKIPPED — not a full run) | VERIFY RED (n FAIL); exit codes 0, 0, 1; usage error 2;
  --require turns every SKIPPED into FAIL).
- run.py: argparse for every flag of 3.1 (--scratch required for a run; --list prints the plan with tiers, locks and estimated time and
  runs nothing; --keep; --offline; --ci; --require; --cargo-slots; the commands --lint, --matrix, --changed and --explain are dispatched
  to t111/lint.py, matrix.py, changed.py, explain.py, which row X4 writes: until those modules exist print "not built yet (row X4)" and
  exit 2, by an import guard, not a stub file).
- The [[known]] rule of F7: a failing test that matches a known entry whose condition holds (e.g. euid0) is recorded SKIPPED with the
  reason "known: ..."; passing counts PASS; --require makes it FAIL.
- verify/tests/: hermetic unittest suites (python3 -I -m unittest discover -s verify/tests -t .) with a fixture map under
  verify/tests/fixtures/map/ (a handful of fake seams and checks whose cmd is a tiny script the test writes to a temp dir): a check that
  passes, one that fails, one that skips for a missing need, one that hangs (killed at its timeout, recorded FAIL with the reason), one
  that leaves a background process behind (caught by hygiene and killed), one that writes into the repo tree (use a temp git repo as the
  "tree" so the real repo is never dirtied; caught), two cargo=true checks under --cargo-slots 1 never overlapping and four under 2 never
  more than two at once (prove it with timestamps written by the fake checks under a lock-free counter file, not with sleeps alone), two
  lock="verify" checks strictly serial, requires-ordering (a unit is SKIPPED "requires <id> (FAIL)" when its prerequisite failed),
  nothing written outside --scratch, the exit codes and last lines of 3.3, --require, the [[known]] rule, --list running nothing.
No real gate of the repo is run by these tests. Check: cd ${ROOT} && python3 -I -m unittest discover -s verify/tests -t . (all green, in
well under a minute), python3 -I verify/run.py --help, and python3 -I -m py_compile on every file. Under notes give the exact interfaces
(function names and signatures) rows X1, X3 and X4 must use.` },

  X1: { model: 'sonnet', effort: 'high', phase: 'Map, adapters, lint', brief: `TASK X1 (row "| X1 |"): the seam map and the check catalog as data. Row X2 has LANDED the schema: read ${ROOT}/verify/t111/schema.py first (it is
the law for field names and enums) and the fixture map under ${ROOT}/verify/tests/fixtures/map/ as a worked example. Write
${ROOT}/verify/seams.toml (every seam of ${TS}/research/05-seams.md and ${TS}/matrix.md: the 89 that exist, with status built or stub
(a seam whose side is still a Layer 7 T0 stub) and the 14 planned Layer 8 / 9 rows with status planned; keep the ids EXACTLY as the matrix
has them, sorted by id) and ${ROOT}/verify/checks.toml (every existing gate and harness of options.md section 3.6 as a check entry: the
fast, slow and ns tiers listed there, each gate plus the units it covers: one unit per node111 integration test binary
(1-bootstrap/crates/node111/tests/*.rs), per mesh111, id111 and agent111 test file, per UI test file where the matrix ties it to a seam,
per verify harness; the [[fake]] table (every fake the matrix names, with what it mirrors and parity = "none" where nothing checks it);
the [[known]] table (the eight node111 tests that fail as root and the api_v1_devices root early return: find their names in
6-transport/TASKS.md:80, plan/HANDOFF-2026-11-02.md and ${TS}/research/04-suites.md, and verify each test fn exists by grep); the
[unmapped] table for test or harness files tied to no seam on purpose, each with its reason).
Rules: every fact comes from the matrix, the survey reports or the tree, verified by you: every file path you write must exist (ls it),
every spec reference must match a heading in that file (grep it; Layer 1 and Layer 3 sections have no numbers: use the heading text),
every cmd must be the exact command the gate documents (read the script's usage and the survey report; argv list, cwd, the env switches
such as REQUIRE_*), every rerun template must be a command that runs that one unit alone, tiers and cargo / lock flags as options.md 3.2
and 3.6 say (every ns check takes lock "verify"; every check that builds or tests with cargo is cargo = true), timeouts generous (three
times the runtime a board or report states, else 1800 s for slow and ns). For a built seam with no check of strength R write the gap in
one honest sentence from the matrix. Do not invent seams or checks; if the matrix and the tree disagree, follow the tree and say so under
deviations. Check: cd ${ROOT} && python3 -I -c "import sys; sys.path.insert(0,'verify'); from t111 import schema; m=schema.load('verify');
print(len(m.seams), len(m.checks))" loads both files with zero schema errors (use the loader's real name), every matrix seam id is present
(write a ten-line throwaway script that diffs the id sets and paste its output), python3 -I verify/run.py --list prints the plan for all
tiers without error, and every path named in either file exists (script it; paste the count checked and zero missing). Do NOT execute any
check. Under notes: the counts (seams by status, checks by tier and kind, fakes, known, unmapped) and every place where the tree differed
from the matrix.` },

  X3: { model: 'opus', effort: 'high', phase: 'Map, adapters, lint', brief: `LEAD ADDENDUM (binding, read with the task below): X2b has LANDED on top of X2 (read adapters.py's contract docstring as it now is: a non-zero
exit with no FAIL record always adds a FAIL unless the adapter set ctx.exit_declared; known applies only to adapter-parsed records). The catalog (X1,
X1b) names one adapter beyond options.md 3.4: "exit77" (exit 0 PASS; exit 77 SKIPPED with the last "SKIP:" line of the log as its reason and
exit_declared set; anything else FAIL with the tail): build it. A record a gate yields for a test that a unit covers carries check = that unit's
id (so the unit's seams, rerun and the [[known]] patterns "*/<test fn>" apply): build the attribution from the gate's stage headers and the
cargo "Running tests/<file>.rs" / "Running unittests src/lib.rs" lines, and say in your notes exactly which id format your libtest records use.
Every adapter is tested against a REAL log: capture one from the tree's own gate or unit where it is cheap (a cargo unit of a small crate, one
vitest file, the knowledge-base check), never only a hand-written sample.

TASK X3 (row "| X3 |"): the adapters of options.md section 3.4. Row X2 has LANDED the registry and its contract: read
${ROOT}/verify/t111/adapters.py, record.py and schema.py first. Fill ${ROOT}/verify/t111/adapters.py with: libtest (cargo test output:
the "Running <path>" lines name the binary, one record per test fn, the "---- <name> stdout ----" block gives the assertion (the
"panicked at" line and its message) and the log line; ignored tests are SKIPPED "ignored"; a binary that did not finish is one FAIL),
vitest (the JSON reporter file when present, else the text FAIL lines of pnpm check), pytest / unittest text, okfail (every sh harness:
ok: / FAIL: / note: lines, "== <phase>" headers mapped to seams through the check's phases table, the closing "<GATE> GREEN" / "<GATE>:
N FAIL" / "<GATE> SKIPPED: <reason>" line, "note: scratch kept at <dir>"; and the honesty rules: exit 0 with any FAIL: line is FAIL; a
missing closing line, or an inner run whose last phase never closed, is FAIL "harness did not finish (last phase: <name>)"; a SKIPPED
closing line after FAIL: lines is FAIL; a failure after an earlier failure of the same check carries "after"), stagewise (check.sh and
the mesh111 gate: "== name ==" stage headers, each stage handed to its inner adapter, a "... SKIPPED" line a SKIPPED record for that
stage, a non-zero exit a FAIL for the open stage with its last 40 lines and the stages after it SKIPPED "not reached (stopped at
<stage>)"), table (the Monocypher harness rows), buildpy (ERROR:<line> <msg> of 0-knowledge/_meta/build.py --check: file and line become
the evidence), and the emitter adapter (a T111_RESULTS file of JSON lines replaces the parse when it holds lines). READ THE REAL
SOURCES for the exact strings before you write a pattern: 1-bootstrap/scripts/check.sh (its stage headers and the three summary lines),
5-messaging/scripts/check.sh, 4-interface/verify/smoke.sh and panel-lint.sh (their ok / fail helpers, the inner.result file, the final
lines), 6-transport/verify/lib.sh (ok, fail, skip_or_fail and the REQUIRE_* rule), 7-relay/verify/lib.sh and the T0 stubs' SKIPPED
line, 2-identity/monocypher-harness/run.sh, 0-knowledge/_meta/build.py; survey reports 01-04 quote many of them with file:line. Golden
tests: ${ROOT}/verify/tests/fixtures/<adapter>/<case>.txt (a captured or faithfully constructed output; construct them from the scripts'
own printf / echo lines and from real cargo / vitest output formats; logs of this session's real runs are under ${SP}/logs/*.log:
m4d-check.log and m1b-check.log are real 5-messaging check.sh outputs you can copy from) with the expected records beside them as
<case>.jsonl; cover for each adapter a green run, a red run, a skip, and the lying shapes: smoke's silent abort (no closing line), a
SKIPPED after FAIL lines, exit 0 with a FAIL line, a libtest binary that crashed mid-run, a multi-line panic message, interleaved
stdout / stderr. Positive control: a planted FAIL in every adapter's fixture must surface as a FAIL record (a test that fails if an
adapter returns no FAIL for it). You own verify/t111/adapters.py, verify/tests/test_adapters*.py and verify/tests/fixtures/<adapter>/.
Check: cd ${ROOT} && python3 -I -m unittest discover -s verify/tests -t . (all green, X2's tests included).` },

  X4: { model: 'sonnet', effort: 'high', phase: 'Map, adapters, lint', brief: `TASK X4 (row "| X4 |" plus the lead's addition in the board header): the map's lint and the map commands. Row X2 has LANDED the schema
and the loader: read ${ROOT}/verify/t111/schema.py and verify/run.py first (run.py dispatches --lint, --matrix, --changed and --explain
to modules that do not exist yet: you write them). Build ${ROOT}/verify/t111/lint.py (the seven failing rules and the two warnings of
options.md 3.5, each rule a function with a stable rule id; the result is records in the runner's own form, so the lint is the check
x.seams.lint of the catalog; a "§N.M" spec reference must match a heading line of that file (^#+ .*N\\.M); unnumbered Layer 1 and
Layer 3 sections match by heading text; rule 4's globs are data at the top of the module; rule 7 greps 1-bootstrap/manifest/ and
cross-build.sh for any path under verify/), t111/matrix.py (--matrix: the matrix tables regenerated from the map, and from a
results.jsonl when --results FILE is given: per seam its strength, its checks and their last status), t111/changed.py (--changed [REV]:
git diff --name-only against REV or the working tree mapped through the seams' side files and the checks' files to the seam ids and the
unit checks that cover them; prints the run.py command to run them), t111/explain.py (--explain <seam-id | check-id>: one seam's whole
record in plain text an agent can read in ten seconds: both sides with their files, the spec sections, the symptom, read_first, every
check that proves it with tier, strength and the exact re-run command, every fake on it and what it mirrors, its gap; for a check id:
its command, tier, locks, needs, the seams and phases it covers). Tests: verify/tests/test_lint.py, test_matrix.py, test_changed.py,
test_explain.py on small fixture maps you add under verify/tests/fixtures/lint/ (one mutated map per rule, each failing exactly that
rule; a clean map passing; the warnings); changed.py against a temp git repo. The real verify/seams.toml and verify/checks.toml are
being written by another agent (X1) right now: do not edit them, and do not make your tests depend on them; if they exist when you
finish, run python3 -I verify/run.py --lint once and report its output under notes without fixing the map. You own exactly those four
modules, their tests and the lint fixtures. Check: cd ${ROOT} && python3 -I -m unittest discover -s verify/tests -t . (all green, X2's
tests included); python3 -I verify/run.py --explain and --changed and --matrix on the fixture map.` },
  X2b: { model: 'opus', effort: 'high', phase: 'Later rows', brief: `TASK X2b (row "| X2b |"): the fix wave on the runner core. X2 has LANDED (7674c6a: read ${ROOT}/verify/t111/*.py and
${ROOT}/verify/tests/ whole before you change a line; the module docstring of engine.py gives the order of operations per check). An independent
read-only review confirmed the defects below; the lead's ruling follows each. Two other tasks run beside you on OTHER files (X1 writes
verify/seams.toml and verify/checks.toml; X4 writes verify/t111/{lint,matrix,changed,explain}.py and its own test files): the interfaces they
use (schema.load_map, SeamMap, record.read_results, cli.estimates, select.select, the main(args, root, map_dir) contract, the field names of
the TOML tables) MUST NOT change. X3 (the adapters) starts after you land and builds on the adapter contract as you leave it.

F1 (high) A worker can reap another check's exit status. hygiene.kill_leftovers calls reap_orphans(runner_pid) with no keep list, which waitpid()s
   every zombie child of the runner, including the leader of a check that just exited while its own worker is between WNOHANG polls; Popen then
   reports status 0 and the exit adapter records PASS for a failed check. Helper subprocesses (the userns probe) are hit the same way.
   RULING: never reap a pid that subprocess owns. One registry of every pid spawned through Popen (check leaders and helper children), guarded by
   the lock that already guards the live checks; every reap_orphans call keeps those pids. Prove it: an e2e test with --jobs 2, one quick failing
   check beside one that leaks a process, looped enough times to hit the window (and a unit test that drives the race deterministically if you
   can: reap while a registered child is a zombie); the failing check is never PASS.
F2 (high) A killed runner orphans its checks. Only KeyboardInterrupt is handled; on SIGTERM or SIGHUP (a timeout wrapper, an agent tool's
   timeout, a closed terminal) the running check lives on in its own session, the flocks die with the runner, and no leftover scan runs. On
   Ctrl-C only the process groups are killed, so a setsid or double-forked daemon stays.
   RULING: (a) SIGTERM and SIGHUP do what Ctrl-C does, and all three then run the leftover scan and kill what is proven ours (the run marker, the
   runner's descendants, the scratch dir), write the records they can and end with VERIFY RED and exit 1 (an interrupted run is never GREEN).
   (b) The lock fds are handed to the check (pass_fds, not CLOEXEC), so a flock lives as long as any process of the check: the box rules (two
   cargo checks, one namespace harness) then hold even when a runner dies. Because a leftover can now hold a lock, waiting for a lock is bounded
   and names its holder: the lock file carries the holder's run id and pid (written after the lock is taken), a waiter prints one progress line
   "waiting for <lock> held by run <id> (pid <n>)" and after --lock-wait seconds (default 1800) the check FAILs "lock <name> not free after N s
   (held by run <id>, pid <n>)" instead of hanging. (c) No PR_SET_PDEATHSIG (it is tied to the parent THREAD and preexec_fn is unsafe with
   threads). Instead a start-of-run sweep: every process whose environment carries T111_RUN of another run AND whose T111_RUNNER_PID (add it to
   the environment each check gets) is no longer a live runner of that run is a leftover of a dead run: TERM, then KILL, and one summary line
   "note: killed N leftover process(es) of dead run <id>" (a note, not a FAIL of this run; never touch a process without the marker).
F3 (medium) The [[known]] rule can hide the engine's own failures. It runs after the safety rules and applies to every record, so a glob that
   matches a check id turns "timeout after N s", "could not start", "exit N but no failing result was parsed" or a hygiene/* record into SKIPPED;
   and mark_after runs first, so a FAIL can point at a record that became SKIPPED.
   RULING: engine-made records and hygiene records are never eligible (mark them at creation); known applies only to results an adapter parsed
   from a test's own output. mark_after runs after apply_known and counts only records that are still FAIL.
F4 (medium) One SKIPPED record switches off the non-zero-exit backstop (engine.py: not any(r.status in ("FAIL", "SKIPPED"))). X3's adapters emit
   SKIPPED for ignored tests as a matter of course, so a crash after some parsed results stays green.
   RULING: a non-zero exit with no FAIL record ALWAYS adds the FAIL "exit N but no failing result was parsed", unless the adapter declared that
   exit: add AdapterContext.exit_declared (bool, default False; an adapter sets it True only when its records explain this exact exit status,
   e.g. a harness whose documented skip code is 77 and whose log says why). Write the rule into the adapter contract docstring for X3 and test
   both sides (declared: SKIPPED stands; undeclared: FAIL added even with PASS and SKIPPED records present).
F5 (medium) The exit adapter keeps the last 40 lines and then record.clip cuts them to the FIRST 2 KB, so the real last error line is dropped and
   the headline shows a line from the middle. RULING: a tail is clipped from the END on a line boundary (keep the last 2 KB), log_line moves with
   it, and the summary headline is the last non-empty line that looks like an error if one is in the kept text, else the last non-empty line.
F6 (low) A file name that is not UTF-8 in a tree-hygiene reason raises UnicodeEncodeError in ResultsWriter.write and the run dies with a traceback,
   no summary and no VERIFY line. RULING: paths are decoded with backslashreplace for display (raw bytes for hashing); results.jsonl and
   summary.txt are written so that no text can raise (ensure_ascii or errors="backslashreplace"); and whatever happens inside the engine, the
   run still prints a last line: an internal error is caught at the top, written as a FAIL record "runner error: <type>: <text>", and the run
   ends VERIFY RED exit 1 (test it by making a writer raise).
F7 (low) test_e2e's "--scratch inside the tree exits 2" does not pass --scratch. RULING: pass it, assert exit 2 and that the dir was not made.
F8 (low) A check that cannot start yields two FAIL records with one id. RULING: one record (replace, as the timeout branch does).
F9 (low) The tree diff is charged to the check that ran while it happened; on a box where other agents edit the tree, or with --jobs > 1, that is
   a false RED an agent would chase. The same holds for the report-only "looks like ours" process rule (a vite or a 111 another agent started).
   RULING: the default stays strict (a quiet tree: CI, and a lead's run after its workers returned): FAIL, worded "changed during the check" and
   naming every check that overlapped the window. A new flag --shared-box (also T111_SHARED_BOX=1) is for a run beside other writers: a tree
   change during a check, and a look-alike process with no marker, are then written as SKIPPED "not attributed (shared box): <what>", so the
   run ends "VERIFY GREEN (n SKIPPED - not a full run)" and never RED on something it cannot prove; a process or change PROVEN ours (the marker,
   a descendant, the scratch dir in its command line; a path only this check could have written is not provable: do not guess) stays FAIL in
   both modes. --require turns those SKIPPED into FAIL like any other. --help says in one line when to use it.
F10 (low) requires ignores covered_by: a unit that requires a unit whose covering gate is in the plan pulls that unit in, so its suite runs
   twice. RULING: the requirement is then on the gate (its status decides; the plan shows it).
F11 (low) out.log has no size cap and is read whole, twice. RULING: cap it (--log-cap MB, default 64): past the cap the runner stops copying,
   keeps draining the pipe so the check is not blocked, and the record says "log cut at N MB"; tails are read by seeking from the end.
F12 (low) The default rerun leaves {scratch} {tmp} {run} unrendered, drops check.env and does not quote {root}. RULING: the printed rerun is a
   command a person can paste: tokens rendered ({scratch}/{tmp} as a fresh path under "$TMPDIR" the text names, or the kept path under --keep),
   the check's env as KEY=value prefixes, every substituted value shlex-quoted.
ALSO: say in the module docs what is by design and not checked (listeners inside another network namespace are not named, only their process is
found; cargo builds started outside the runner are not counted; the lock dir is per box, mode 1777 when the runner creates it). Keep --help
truthful for every flag you add (--shared-box, --lock-wait, --log-cap).

FILES: ${ROOT}/verify/run.py, ${ROOT}/verify/t111/{engine,hygiene,run,record,select,cli,summary,locks,adapters,schema}.py (adapters.py: only
AdapterContext.exit_declared, the exit adapter's tail and the contract docstring; schema.py: only if a ruling needs a field, and then additive
with a default so X1's files stay valid), ${ROOT}/verify/tests/** (your own tests and fixtures; test files named test_lint*, test_matrix*,
test_changed*, test_explain* are X4's: never touch them). Nothing else.

CHECK: cd ${ROOT} && TMPDIR=${SP}/X2b python3 -I -B -m unittest discover -s verify/tests -t . (five times in a row: the concurrency tests must
not flake; other agents load this box, so a test that needs a quiet CPU is a defect of the test), python3 -I -B verify/run.py --help, py_compile
of every file. If X4's or X1's files appear in verify/ while you work and one of THEIR tests fails, that is theirs: report it, do not fix it.
Report per finding: fixed + the test that pins it, or why not.` },
  X1b: { model: 'sonnet', effort: 'high', phase: 'Later rows', brief: `TASK X1b (row "| X1b |"): the review's findings on the seam map and the check catalog (X1 LANDED b847745: ${ROOT}/verify/seams.toml,
${ROOT}/verify/checks.toml; read their header comments and ${ROOT}/verify/t111/schema.py first). The full review with file:line and scenarios is
${SP}/results/X1-review.json if that file still exists; the list below is complete without it. You edit ONLY the two TOML files. Edit them in
place (the generator script is gone); keep every comment that is still true. The lead's ruling on each finding:
 1 l1.node111.lib-relay's libtest filter "relay::" also matches dashboard::api_v1::relay (the chat relay) and relay::netwatch. RULING: the filter
   is daemon::relay::settings:: in cmd and rerun; relay::netwatch gets its own unit (l1.node111.lib-netwatch, filter daemon::relay::netwatch::)
   on L6-L7:netlink-gateway (D4 has landed: 71af200; netwatch.rs is built, gateway.rs is still a T0 stub, so that seam stays stub with a gap
   that says so); say in the l1.node111.lib comment which unit owns which --lib records in a gate run.
 2 every multi-seam seams list and phases value is sorted alphabetically, but a record's sides, read_first and fake come from the FIRST seam.
   RULING: the primary seam (the one whose files the check's assertions exercise) goes first in every check's seams and in every phase list;
   never sort them; say so in the catalog's header comment. Decide each primary by reading the test or harness, not by its name.
 3 l7.g10-smoke (tier slow, a default tier) writes 7-relay/verify/logs/ntfy-smoke-<date>-stub.md into the tracked tree on every run, so a
   default run could never be GREEN. RULING: until row X5 moves that log under the scratch dir, the check carries needs = ["x5-log-to-scratch"]
   (no such tool exists: the runner reports it SKIPPED with that reason, which is the truth) and a comment naming X5; X5 removes the need.
 4 options.md 3.6: every Rust unit that reads contract.gen.ts requires l4.contract.fresh. RULING: add the requires to l1.node111.lib and to
   l1.gate (and to any other reader you find by grep for contract.gen.ts under the crates).
 5 every [[known]] names check = the unit id, so in a gate run (records carry the gate's id unless the adapter rewrites it) none matches.
   RULING: drop check from the [[known]] entries; the */<test fn> patterns are unique (verify by grep).
 6 l1.gates.musl-env, l1.gates.build-scripts and l1.linux-e2e use adapter okfail but do not speak its vocabulary. RULING: adapter = "exit" for the
   first two; l1.linux-e2e and every other harness whose documented skip is "SKIP: ..." + exit 77 (grep the verify/ and scripts/ dirs for
   "exit 77") gets adapter = "exit77" (row X3 builds it: exit 0 PASS, exit 77 SKIPPED with the last SKIP: line as its reason and the exit
   declared, anything else FAIL with the tail). Re-read each okfail check's script and keep okfail only where the script really ends with a
   "<GATE> GREEN" line and prints ok:/FAIL:/SKIPPED lines.
 7 the l4.ui.* vitest units lack the reporter flags of options.md 3.4. RULING: append "--reporter=default", "--reporter=json",
   "--outputFile={scratch}/vitest.json" to each.
 8 five checks rebuild 4-interface/ui/dist with no common lock. RULING: lock = "ui-dist" on every check that builds or reads dist (l1.gate,
   l4.ui, l4.budget, l4.smoke, l4.smoke3, l4.panel-lint and any other you find).
 9 two [[fake]] entries name the wrong side (L4-L6:transport-api, L1-L6:bridge-stubs). RULING: the fake names what it stands in for; fix as the
   review says (the rnsd / rnstatus fake belongs with L1-L6:rnsd-child; for bridge-stubs side a is the bridge core and side b the StubAdapter).
10 l1.gate timeout_s 3600 is below the file's own rule and CI's bound. RULING: 5400.
11 59 cargo units lack needs = ["cargo"]. RULING: add it (and "bash", "pnpm", "python3" where the cmd starts with them).
12 x.seams.lint runs without -B. RULING: "-I", "-B".
13 agent111 has no --lib unit. RULING: add l3.agent111.lib (covered_by l3.gate) with the seams its in-crate tests prove (read the seven files).
14 l1.gate nests the mesh111 check and the UI check, and lists layers [1,2,4,5,6,7], so --layer 5 pulls the 25-minute aggregate. RULING:
   l1.gate.layers = the layers it owns (1 plus those of its own static stages); its comment states which suites a full run runs twice and why
   (CI parity), so nobody "fixes" it by dropping a gate.
15 l3.gate ends with cargo check --target x86_64-unknown-linux-musl. The lead installed that rust-std target on this box today, so it passes
   here; add needs = ["cargo"] only, and say in the comment that a box without the target fails that last stage.
ALSO: the review's coverage gaps: audit read_first and the symptom of each of the 103 seams against the two sides' files (one pass; fix what is
wrong, list what you changed); check each [unmapped] reason against its file. The seam id L5/7-L9:fix-sharing-delegation is right (matrix.md was
corrected). RUN: python3 -I -B verify/run.py --lint (LINT GREEN; the stub-marker and fake-parity notes are expected until X4b and N-rows),
--list --tier fast,slow,ns, --matrix (totals R 42 / I 15 / F 13 / C 9 / N 10 unless you promote a seam, then say which and why), --explain on
five seams, and the unit suite (python3 -I -B -m unittest discover -s verify/tests -t .). Execute nothing from the catalog.` },
  X4b: { model: 'sonnet', effort: 'high', phase: 'Later rows', brief: `TASK X4b (row "| X4b |"): the review's findings on the map commands (X4 LANDED d990310: ${ROOT}/verify/t111/{lint,matrix,changed,explain}.py
and their tests; X2b has LANDED: read cli.py as it now is). The full review is ${SP}/results/X4-review.json if it still exists; the list below
is complete without it. Each fix gets a test that fails without it. The lead's rulings:
 1 rule "files" resolves a spec section only against '#' heading lines; the real specs mark many subsections with bold leads ("**9.4 The KB**",
   "- **Probation ...**", "**Binds:**"). RULING: a line that starts (optionally after "- ") with "**N.M" is a section anchor, and a section name
   also matches a bold lead-in up to its closing "**" or ":"; add these real shapes to the tests (copy three real lines from 7-relay/README.md
   and 1-bootstrap/README.md).
 2 the pi-floor detector misses the usual ways shell and Python name the root verify/ (a quoted variable before the slash, the bare directory,
   ../verify, os.path.join parts, a $(...) prefix), and passes silently when a PI_FLOOR_PATHS entry no longer exists. RULING: match "verify" as a
   whole path part in those forms, and FAIL when a PI_FLOOR_PATHS entry is missing from the tree.
 3 a cargo test with --lib / --doc / --bin(s) / --example(s) and no --test(s) claims every tests/*.rs of its package, so rule "unclaimed" goes
   blind for that package. RULING: such a command claims no tests/*.rs file. (The catalog has units for each test binary; X1b keeps them.)
 4 matrix.py reads args.results but the parser has no --results. RULING: add --results FILE to the commands group in cli.py (your one line
   there) with an end-to-end test through run.py.
 5 no rule checks [[known]]. RULING: fail when known.check (if present) names no check, when a test pattern's check prefix names no check, or
   when the file part of known.source does not exist.
 6 rule "unclaimed" (fnmatch: * crosses /) and rule "files" (Path.glob: it does not) read the same glob differently. RULING: one matcher for
   both, with * as one path segment and ** crossing segments; update the catalog's reading in the docstring.
 7 --changed misses the old path of a rename. RULING: git diff --no-renames.
 8 a check id with no l<N>. or x. prefix skips the layer half of rule "ids". RULING: such an id fails the rule.
 9 test_changed's git helper passes GIT_* through. RULING: strip GIT_* as changed._git does.
10 (from X1's author) stub-marker warns for every side file of a stub seam that has no T0 marker, so the built side of a mixed seam is told to
   promote the row. RULING: warn only when NO side file of the seam carries the marker.
11 (from X1's author) a cmd token with glob characters (test_*.py) is read as a missing file. RULING: a token with * ? [ is not a path.
12 gaps to close: rule "files" also checks script paths inside sh -c "..." style tokens when the first word is a path that exists or looks like
   one (ends in .sh/.py); UNCLAIMED_GLOBS also sees one level deeper under */verify/ (6-transport/verify/spine/node.sh and the like: X1b will
   map or [unmapped] them: run the lint on the real map and report what it now says, do not edit the TOML files); --matrix says "last run was
   a selection of N checks" when the newest run under --scratch did not run every check.
FILES: ${ROOT}/verify/t111/{lint,matrix,changed,explain}.py, the one parser line in cli.py, ${ROOT}/verify/tests/test_{lint,matrix,changed,explain}.py,
${ROOT}/verify/tests/fixtures/lint/**. CHECK: the unit suite (python3 -I -B -m unittest discover -s verify/tests -t .), then on the real map:
--lint, --matrix, --changed HEAD~5, --explain on two seams and one check; report the lint's output on the real map verbatim.` },
}

// Later rows carry no long brief here: the board row plus options.md are the brief (the lead passes args.only and, when a row needs
// more, args.extra = { <id>: '<text>' }).
const later = (id) => ({ model: (a.models && a.models[id]) || 'sonnet', effort: 'high', phase: 'Later rows', brief:
  `TASK ${id} (row "| ${id} |" of the board; its design is in options.md section 4 for an N row, section 6 and the section the row names for an X row). X1-X4 have LANDED: the runner (${ROOT}/verify/run.py, verify/t111/), the seam map (verify/seams.toml) and the check catalog (verify/checks.toml) exist; read verify/README.md if present, else verify/t111/schema.py. Do what the row says, in its files only; when your row adds or changes a check, register it in verify/checks.toml (and promote the seam it proves in verify/seams.toml: strength, gap) only if the row's Files column lists those files, else describe the catalog edit under deviations for the lead. Run the row's Check.${a.extra && a.extra[id] ? ' ' + a.extra[id] : ''}` })

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

const review = (r) => agent(`You are a read-only adversarial reviewer in ${ROOT} (the 111network mesh OS; the cross-layer test system, plan/DECISIONS.md D36 and D37).
Do NOT edit any file, do NOT start any long-running process, do NOT run cargo, pnpm or any namespace harness; you MAY run python3 -I on the
test system's own hermetic unit tests (cd ${ROOT} && python3 -I -m unittest discover -s verify/tests -t .) and tiny read-only python
one-liners to check a parse. Other agents are editing other files right now: review ONLY the files the task reports (below), as they are
on disk now (git status -s; git diff for tracked files; new files have no diff: read them whole).

The task: row "| ${r.id} |" of ${TS}/TASKS.md and the design it implements in ${TS}/options.md (sections 3.1-3.5 and the row's own
section). The author's own report (its declared deviations are NOT findings; report what it did not declare):
${JSON.stringify(r).slice(0, 7000)}

This system exists so that an agent can trust a GREEN and act on a RED. Hunt for: (1) any way a real failure is reported PASS or SKIPPED,
or silently dropped (a parse that misses a FAIL line, an exception swallowed, an exit code ignored, a timeout treated as a pass, a record
never written, a summary whose last line says GREEN while a record says FAIL); (2) any way a pass is reported FAIL (a false alarm an agent
would chase); (3) the box rules: more than two cargo checks at once, two namespace harnesses at once, a process or listener left behind, a
byte written outside the scratch directory, the repo tree dirtied; (4) robustness: a path with spaces, a log that is not UTF-8, a check
that prints megabytes, a missing tool, a lock file left by a killed run, two runner instances at once; (5) the data: a field the schema
accepts but the runner ignores, an id that can collide, a path or spec reference that does not exist, a command that is not what the gate
documents (read the gate's script), a seam of the matrix that is missing; (6) tests that do not exercise their claim (no positive control,
a sleep standing in for a proof); (7) the house rules: a third-party import, shell=True with interpolated text, a model name, an edit
outside the row's files. Report only what you can show with a concrete path (file:line) and, where you can, a three-line reproduction;
CONFIRMED when shown, PLAUSIBLE otherwise; list what you checked and found sound and what is not covered. Set task to "${r.id}".`,
  { label: `TS:${r.id}-review`, phase: 'Review', schema: REVIEW, model: 'opus', effort: 'high' })

// args.extra = { <id>: '<text>' } also reaches the rows with a long brief above, as a lead note (later rows already read it).
const note = (id) => (a.extra && a.extra[id]) ? `\n\nLEAD NOTE (read first): ${a.extra[id]}` : ''
const build = (id) => {
  const t = T[id] || later(id)
  return agent(CTX + '\n\n' + t.brief + (T[id] ? note(id) : ''), { label: `TS:${id}`, phase: t.phase, schema: RESULT, model: t.model, effort: t.effort })
    .then(r => r && { ...r, id })
}
const withReview = (r) => (r && r.status !== 'blocked') ? review(r).then(v => ({ ...r, review: v })) : r
const want = (id) => !ONLY || ONLY.includes(id)
const line = (rs) => rs.filter(Boolean).map(r => `${r.id}=${r.status}${r.review ? ' (' + r.review.findings.length + ' findings)' : ''}`).join(' · ')

const out = {}
if (want('X2')) {
  out.core = [await build('X2').then(withReview)].filter(Boolean)
  log('core: ' + line(out.core))
  if (!out.core.length || out.core[0].status !== 'done' || a.stopAfter === 'core') return out
}
const second = ['X1', 'X3', 'X4'].filter(want)
if (second.length) {
  out.second = (await pipeline(second, build, withReview)).filter(Boolean)
  log('map, adapters, lint: ' + line(out.second))
}
const rest = (ONLY || []).filter(id => !['X1', 'X2', 'X3', 'X4'].includes(id))
if (rest.length) {
  out.later = (await pipeline(rest, build, withReview)).filter(Boolean)
  log('later rows: ' + line(out.later))
}
return out
