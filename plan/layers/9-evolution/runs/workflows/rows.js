export const meta = {
  name: 'board-rows',
  description: 'Build board rows of Layer 8 or 9 (args.layer): each row by its author, then an independent read-only review and one fix pass; cargo rows one at a time on one lane, the others beside them; the lead re-runs the checks and commits each row',
  phases: [
    { title: 'Rows', detail: 'author, review, fix per row; cargo rows serial, other rows parallel' },
  ],
}
// args: {
//   layer: '8' | '9',
//   rows: [{ id: 'D1', model: 'opus', cargo: true, note: '<lead note for this row>' }, ...],
//   facts: '<the as-built facts block for this run: what landed, what is stubbed, what other agents are doing>',
//   sp: '<session scratchpad>', root: '/home/user/os', review: true
// }
const a = args || {}
const ROOT = typeof a.root === 'string' ? a.root : '/home/user/os'
const SP = typeof a.sp === 'string' ? a.sp : '/tmp/scratch'
const LAYER = String(a.layer || '9')
const L = {
  '8': { slug: '8-storage', name: 'Storage & Shared State', lock: 'D34, D38', gates: 'G13-G18 and the extended G7' },
  '9': { slug: '9-evolution', name: 'Self-Healing & Evolution', lock: 'D35, D39', gates: 'G19-G25 and the extended G7' },
}[LAYER]
const ROWS = Array.isArray(a.rows) ? a.rows : []
const REVIEW = a.review !== false

const CTX = `You build ONE row of the Layer ${LAYER} (${L.name}) build board in ${ROOT}, the 111network mesh OS. The root README.md is the
owner's vision and the source of truth for what the project is; the layer spec is the binding form of it. Read first: ${ROOT}/AGENTS.md (the
Raspberry Pi floor, Linux hygiene, the box rules; never add attribution lines), ${ROOT}/plan/EXECUTION.md, then the board
${ROOT}/${L.slug}/TASKS.md: its header paragraph and box rules, the ordering sentence of your row's wave, and YOUR row whole (Task, Criteria, Check,
Files, the "never adds" clause, any "Additive amendment" or "Correction" mark). Spec: ${ROOT}/${L.slug}/README.md (locked v1.0, ${L.lock} in
plan/DECISIONS.md); read the sections your row names with sed -n ranges, and the locked spec of any layer your row amends. Contract:
${ROOT}/4-interface/api/openapi.yaml (binding for every route and shape; conventions 4-interface/api/README.md). Every file:line the row cites was
pinned at an earlier head: re-read it before you edit.

AS-BUILT FACTS FOR THIS RUN:
${a.facts || '(none given)'}

FILE OWNERSHIP IS STRICT: create or modify only the files your row's Files column lists (plus a new file inside a dir it names, the one claim of a
new test file in ${ROOT}/verify/checks.toml, and a tiny registration line a new module needs: list each). Other agents edit other files of this
tree right now. If you need a change elsewhere, do not make it: describe it under deviations. Never touch plan/, a TASKS.md board, INDEX.md or a
spec. A locked layer changes only as your row's mark says (additive, or a correction the spec names).

HOUSE RULES: Rust 2021, typed errors, never panic on input, sizes checked before parse, doc comments citing "§x.y" of the Layer ${LAYER} spec,
unit tests in-module and integration tests under tests/, rustfmt and clippy --all-targets -D warnings clean (node111's 4 manifest
unused_dependencies warnings, sysinfo, os_info, nusb, rtnetlink, are known: ignore, never remove); no new dependency unless your row is the one that
adds it; no wall clock in any ordering or cryptographic decision; foreign bytes untrusted; secrets never echoed, logged or framed; every route
problem+json and the agent's bearer 401 on every write; shell: POSIX sh or bash as the sibling scripts, shellcheck clean; no model name in any
file; never commit (the lead commits each row).

BOX: 4 CPUs; at most two cargo builds on the box, and another agent may hold one: run ONE cargo command at a time, with CARGO_INCREMENTAL=0
CARGO_BUILD_JOBS=2 and TMPDIR unset, from the workspace directory, and these warm target dirs: 1-bootstrap (node111) → ${SP}/target/node111,
5-messaging (mesh111) → ${SP}/target/mesh111, 3-intelligence (agent111) → ${SP}/target/agent111, 2-identity → ${SP}/target/id111. Iterate narrowly
(cargo test -p <crate> --lib <module>, cargo check), run your row's full Check once at the end. A build that fails inside another row's file is its
half-made edit: wait 90 s, retry twice, then report blocked. Scratch only under ${SP}/l${LAYER}-<row>/, deleted at the end. Hygiene: nothing of
yours running or listening when you return (ps, ss), no netns left.

Check that every new test fails without the code it tests (revert temporarily). RETURN a report: per criterion of your row MET / PARTIAL / NOT
MET with file:line; deviations (and the lock-amendment candidates your row lists); the commands you ran with their result lines; measured figures
(or "not measured" and why); the files you changed; open questions for the lead.`

const FINDINGS = {
  type: 'object', required: ['findings', 'sound'],
  properties: {
    findings: { type: 'array', items: { type: 'object', required: ['severity', 'where', 'defect', 'scenario', 'fix'], properties: {
      severity: { type: 'string' }, verdict: { type: 'string' }, where: { type: 'string' }, defect: { type: 'string' }, scenario: { type: 'string' }, fix: { type: 'string' } } } },
    sound: { type: 'array', items: { type: 'string' } },
  },
}

const brief = (r) => `TASK ${r.id} (row "| ${r.id} |" of ${ROOT}/${L.slug}/TASKS.md).${r.note ? '\n\nLEAD NOTE (read first): ' + r.note : ''}`

const review = (r, report) => agent(`You are a read-only adversarial reviewer in ${ROOT} (Layer ${LAYER} build). Do NOT run cargo, pnpm or any build, do
NOT edit any file, do NOT start a process. Review row "| ${r.id} |" of ${ROOT}/${L.slug}/TASKS.md (read it whole) against the spec sections it names
(${ROOT}/${L.slug}/README.md, locked) and the contract (${ROOT}/4-interface/api/openapi.yaml), reading the uncommitted diff of the files the author
reports (cd ${ROOT} && git diff --histogram -- <paths>; a new file: read it whole). Other rows are built beside it: review only this row's files.
The author's report (its declared deviations are not findings; report what it did not declare): <<<${String(report).slice(0, 9000)}>>>
Hunt for: correctness bugs reachable from a real path (cite file:line now); a criterion unmet or met only by a test helper; the "never adds" clause
broken (a resident task, timer, poll, listener, file write, dependency on an idle node: the Raspberry Pi floor); a locked layer changed beyond the
row's mark; security (the agent's bearer on a write, a secret logged, foreign bytes trusted, a size unchecked, a consent or human step that can be
skipped); contract mismatches; tests that pass without their fix. CONFIRMED when traced end to end, PLAUSIBLE otherwise. Report only what survives
your own scrutiny; list what you checked and found sound.`, { label: `L${LAYER}:${r.id}-review`, phase: 'Rows', schema: FINDINGS, model: 'fable' })

const refix = (r, report, rv) => agent(`${CTX}\n\n${brief(r)}\n\nYou built row ${r.id}; your report: <<<${String(report).slice(0, 9000)}>>>
An independent reviewer reports: ${JSON.stringify(rv.findings).slice(0, 20000)}
Verify each against the code; fix the real ones, each with a test that fails without its fix; rebut the wrong ones with evidence. Re-run your
row's Check and the gates. RETURN per finding FIXED / REBUTTED with evidence, the gates with their result lines, the files changed.`,
  { label: `L${LAYER}:${r.id}-refix`, phase: 'Rows', model: r.model || 'opus' })

const one = async (r) => {
  const report = await agent(`${CTX}\n\n${brief(r)}`, { label: `L${LAYER}:${r.id}`, phase: 'Rows', model: r.model || 'opus' })
  if (!report) return { id: r.id, error: 'the author died' }
  if (!REVIEW || r.review === false) return { id: r.id, report }
  const rv = await review(r, report)
  if (!rv || !rv.findings.length) return { id: r.id, report, review: rv }
  const fixed = await refix(r, report, rv)
  return { id: r.id, report, review: rv, refix: fixed }
}

phase('Rows')
const cargoRows = ROWS.filter(r => r.cargo)
const otherRows = ROWS.filter(r => !r.cargo)
const out = await parallel([
  async () => { const res = []; for (const r of cargoRows) { const x = await one(r); res.push(x); log(`${r.id}: ${x.error || (x.review ? x.review.findings.length + ' findings' : 'done')}`) } return res },
  ...otherRows.map(r => async () => { const x = await one(r); log(`${r.id}: ${x.error || (x.review ? x.review.findings.length + ' findings' : 'done')}`); return [x] }),
])
return { layer: LAYER, rows: out.filter(Boolean).flat() }
