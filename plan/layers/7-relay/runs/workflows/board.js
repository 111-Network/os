export const meta = {
  name: 'layer-7-board',
  description: 'Layer 7: one opus planner writes the build board 7-relay/TASKS.md from the locked spec per EXECUTION.md; one fable judge checks coverage, file ownership and the gates; one fix pass',
  phases: [
    { title: 'Draft', detail: 'opus planner writes the board (waves, criteria, checks, owned files, models)', model: 'opus' },
    { title: 'Check', detail: 'fable judge: every normative section owned, files disjoint per wave, every gate has a task, D32 order kept', model: 'fable' },
    { title: 'Fix', detail: 'the planner applies blockers and majors; the judge confirms', model: 'opus' },
  ],
}

const ROOT = '/home/user/os'
const SPEC = `${ROOT}/7-relay/README.md`
const BOARD = `${ROOT}/7-relay/TASKS.md`
const CHECK = `${ROOT}/plan/layers/7-relay/review/board-check.md`
const MODEL_BOARD = `${ROOT}/6-transport/TASKS.md`

const CTX = `
You work on the 111network OS build (repo root ${ROOT}). Today is 2026-11-02. Layer 7 (Relay & Presence) is locked: the spec is ${SPEC} (v1.0,
D32; 1,484 lines; read it WHOLE before anything else, it is the law), decided by D30 and amended by D31 (${ROOT}/plan/DECISIONS.md: read D30, D31,
D32; D32's "Build" paragraph sets the wave order). The build protocol is ${ROOT}/plan/EXECUTION.md (read it). The MODEL for the board is the Layer 6
board ${MODEL_BOARD} (read it whole: its header paragraph, box rules, legend, T0, the wave tables with the columns #, Task, Criteria, Check, Files
(owned), Model, Status, the external gates, the Summary section left for the end). The panel's "build watch" items are in
${ROOT}/plan/layers/7-relay/review/recheck.md (Notes) and triage.md. Read-only on everything but the file you are told to write: no git write
commands, no cargo, no process; reads and greps are fine (verify that every path you cite exists, or mark it "new").
Rules that bind every task: the Raspberry Pi floor (${ROOT}/AGENTS.md: nothing resident, periodic, no runtime dependency or build-time weight in the
required configuration without the owner's say; the spec's §5.11 and §16 say what Layer 7 adds and where; every task that could add weight names
the measurement it records); adopt, don't reinvent; the twelve Layer 5 amendments are the only Layer 5 spec changes, the §19 code seams the only
other mesh111 edits; nothing in §17 (v1.1) is built; every consent in the pinned shape; no model names or identifiers in any file except the
board's "Model" column, which uses the roster words of the Layer 6 board (opus, sonnet, haiku, fable) as process vocabulary; no attribution lines.
`

const DRAFT_SCHEMA = {
  type: 'object',
  required: ['status', 'file', 'lines', 'waves', 'coverage_gaps', 'notes'],
  properties: {
    status: { type: 'string', enum: ['written', 'partial', 'blocked'] },
    file: { type: 'string' },
    lines: { type: 'integer' },
    waves: { type: 'array', items: { type: 'object', required: ['wave', 'tasks'], properties: { wave: { type: 'string' }, tasks: { type: 'array', items: { type: 'string' } } } }, description: 'each wave with its task ids and one-line titles' },
    coverage_gaps: { type: 'array', items: { type: 'string' }, description: 'spec sections no task owns, with why' },
    notes: { type: 'string' },
  },
}

const CHECK_SCHEMA = {
  type: 'object',
  required: ['verdict', 'findings', 'report_file', 'notes'],
  properties: {
    verdict: { type: 'string', enum: ['ready', 'fix-first'] },
    findings: { type: 'array', items: { type: 'object', required: ['id', 'severity', 'where', 'what', 'fix'], properties: { id: { type: 'string' }, severity: { type: 'string', enum: ['blocker', 'major', 'minor'] }, where: { type: 'string' }, what: { type: 'string' }, fix: { type: 'string' } } } },
    report_file: { type: 'string' },
    notes: { type: 'string' },
  },
}

phase('Draft')
const draftPrompt = `${CTX}
You are the PLANNER. Write ${BOARD}: the Layer 7 build board, in the Layer 6 board's exact shape and density (its header paragraph adapted: spec,
reviews, contract, the crate ownership rule - Layer 7 owns new modules under ${ROOT}/1-bootstrap/crates/node111/src/daemon/ (say which: e.g.
relay/, carrier/, origin/, registry/ - pick names the spec uses or the lightest that fit), the mesh111 changes are the twelve amendments plus the §19
code seams, the model per task, the wave order from D32; its box rules paragraph verbatim-adapted; the legend; T0; the waves; the external gates; an
empty "Summary (append when done)" section).

HOW TO BREAK IT DOWN (EXECUTION.md step 1): small, independent tasks; each with a goal, acceptance criteria that cite the spec sections they
implement (§n.m; the amendment numbers of §5.3; the gate items of §15), a verifiable check (a test name, a gate script in 7-relay/verify/ with the
REQUIRE_* convention of 6-transport/verify/, a lint, a contract test, 'cargo test -p <crate> <filter>'), and the files it owns (existing paths
verified by reading the tree; new files marked new). Tasks that touch the same file never share a wave: partition mesh111 edits by file
(store/schema.rs + store/rows.rs + store/prune.rs; fabric.rs; envelope.rs / the hint TLV; link/tcp_lan.rs; settings) and node111 edits by module;
openapi.yaml, the UI, the KB, LICENSING.md and the docs are their own tasks. Within a wave every Files cell is disjoint; across waves, later waves
may touch earlier files.

THE ORDER (D32 Build; D30 F10): T0 the lead's scaffold (verify skeleton, module stubs, the KB candidate entry and LICENSING rows that need no code,
the board). Wave 0: the ntfy carrier PROTOTYPE and its KB smoke log FIRST (§6.2, §6.4; it may use a stub ntfy server in a netns for the log, the
real server only where the box's network allows: say so); the Layer 1 trust amendments early (§9: the staging lock, the origin list and the hub/--from
fetch rule, T1-T6 of §9.5; §18 L1); the twelve Layer 5 amendments as ONE wave of parallel tasks partitioned by file (§5.3 table with its file
cites; §4.6 am. 12; §5.9 am. 11); the contract (§13 with the six-item pin list of §13.2 as ONE task's criteria, done only when all six are green).
Wave 1 (after Wave 0): presence (§2: the attributed_on seam, the 111 Nearby kind, evidence, the inert heartbeat switch and its validator), the
gateway test (§3: netlink groups, the probe removal and the gateway row, the mdns re-arm, the Termux fallback, the timer trim, the services-registry
Notify), TLS raw public keys (§4: the stream seam, tokio-rustls, the link key, pairing routes and the bundle, the warning consent), the origin
serving routes (§9.1-9.2), the registry (§7: the DNS-SD record, role bits in node111's sealer, the badge; the sealed query pinned not built), the
epoch carriage (§8: format and unit only), the radar and the relay/settings/consent UI and the fabric view additions (§10, §12, §13.1 events), the
agent tools (§13.3). Wave 2 (after Wave 1): the carrier wired to the forwarder (§5.9 hand-off API + §6.1-6.2 scheduling: 5A on events in
spawn_blocking, 5B one named thread), the hub origin in Layer 1's updater and '111 update --from' (§9.2-9.5), the no-timer gate script (§3.7 and
§15 G7 extended), the KB entries and LICENSING/THIRD-PARTY rows that depend on code. Wave 3 (integration): one V task per gate G8-G12 and the
extended G7 (§15: name each gate's script, netns shape, the tape for G9 with the old hub from the tag checkpoint-layer-6 which EXISTS at 4689dbf,
REQUIRE_G9=1; the CI mapping of §15), then V-lead: every gate run by the lead, the Pi-floor measurements of §16 recorded (binary delta, idle RSS,
listeners, timers, wakeups, disk writes; release build), the board Summary, the as-built lock entry (D33), INDEX row 7; then V-review: one fable
integrated review and its fix wave. External gates (hardware, the real ntfy server, the owner's machine for the unverified claims) listed as in the
Layer 6 board, not blockers.

EVERY ROW: # (T0; P1..; M1..; L1..; C1..; U1..; V1.. - pick a scheme and keep it), Task (bold title + what, citing §), Criteria (bullet-dense, the
spec's normative sentences as assertions, numbers included), Check (exact), Files (owned), Model (Rust → opus; UI/docs/KB/Python/shell → sonnet;
tiny → haiku; review → fable), Status ⬜. Nothing resident on a non-hub node: say in the criteria of each daemon task what it must NOT add (§5.11).
Mark the owner-fork dependencies: a task that would change if the owner flips a D31 fork says "(D31 Qn applied as recommended)".

Length: 110-200 lines (the Layer 6 board is 92 lines of dense tables; this layer has more surface). Return: status, file, lines, waves (ids and
titles), coverage_gaps (every spec section with no owner, and why), notes (at most 10 lines).`
const draft = await agent(draftPrompt, { label: 'board:draft', phase: 'Draft', schema: DRAFT_SCHEMA, model: 'opus', effort: 'xhigh' })
if (!draft || draft.status === 'blocked') return { draft, check: null, fix: null }
log(`board: ${draft.status}, ${draft.lines} lines, ${draft.waves.length} waves, ${draft.coverage_gaps.length} coverage gaps`)

phase('Check')
const checkPrompt = (round) => `${CTX}
You are the fable JUDGE of the build board ${BOARD}${round > 1 ? ` (round ${round}: after a fix pass; your previous report is ${CHECK}; confirm each previous blocker and major is closed, look for regressions)` : ''}.
Read the board whole, then verify against the spec, adversarially: (1) COVERAGE: build a section-to-task matrix for every normative section of the
spec (§2-§13, §15, §16, §18, §19 code seams; each of the twelve amendments of §5.3; each pin-list item of §13.2; each gate G7-ext, G8-G12 of §15;
each Layer 1/4/5/6 seam of §18): a normative item no task owns is a blocker; a v1.1 item (§17) a task builds is a blocker. (2) OWNERSHIP: within
each wave the Files cells are disjoint (a shared file is a blocker); every existing path exists in the tree (read it) and every new path is marked
new; mesh111 edits are only the twelve amendments and the §19 seams. (3) CHECKS: every Check is runnable as written (a named test, a script with
a path, a gate with its REQUIRE_* switch) and asserts the row's criteria, not less. (4) ORDER: D32's order (the ntfy prototype and its smoke log
first; the Layer 1 trust amendments early; the twelve as one wave; the contract's six items as one task) and the wave dependencies hold. (5) FLOOR:
every daemon task states what it must not add on a non-hub node; the measurement rows of §16 have an owner. (6) FORM: the Layer 6 board's shape,
the legend, the Summary stub, 110-200 lines, no model names outside the Model column, no attribution lines.
Write ${CHECK} (findings table: id, severity, where, what, fix; then the coverage matrix you built, compact). Return: verdict (ready: no blocker, no
major), findings, report_file, notes.`
let check = await agent(checkPrompt(1), { label: 'board:check', phase: 'Check', schema: CHECK_SCHEMA, model: 'fable', effort: 'high' })
log(`check: ${check ? check.verdict : 'none'} (${check ? check.findings.length : 0} findings)`)

let fix = null
if (check && check.verdict === 'fix-first') {
  phase('Fix')
  const must = check.findings.filter(f => f.severity !== 'minor')
  fix = await agent(`${CTX}
You are the PLANNER again (you wrote ${BOARD}; read it whole). The judge's report ${CHECK} lists findings; apply every blocker and major exactly
as its fix says (${must.map(f => f.id).join(', ')}) and the minors where the fix is a line; keep the shape and every rule of the first draft. Where a
fix would contradict the spec or D32, do not apply it and say why in notes. Return: status, file, lines, waves, coverage_gaps, notes (what you applied
by id and what you declined).`, { label: 'board:fix', phase: 'Fix', schema: DRAFT_SCHEMA, model: 'opus', effort: 'xhigh' })
  check = await agent(checkPrompt(2), { label: 'board:recheck', phase: 'Fix', schema: CHECK_SCHEMA, model: 'fable', effort: 'high' })
  log(`recheck: ${check ? check.verdict : 'none'} (${check ? check.findings.length : 0} findings)`)
}
return { draft, check, fix }
