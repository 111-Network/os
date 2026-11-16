export const meta = {
  name: 'board-waves',
  description: 'Build rows of the Layer 8 and 9 boards as a dependency graph: a row starts when the rows it follows are done and its lane (a shared target dir or a shared file set) is free; each row by its author, then an independent read-only review and one fix pass; the lead lands each row as its chain completes',
  phases: [
    { title: 'Rows', detail: 'author, review, fix per row, ordered by the boards' },
  ],
}
// args: {
//   rows: [{ id: 'D1', layer: '8', model: 'opus', lane: 'n111', after: ['L8:M1'], note: '<lead note>' }, ...]  (after: ids as 'L<layer>:<id>')
//   facts: '<the as-built facts block>', sp: '<session scratchpad>', root: '/home/user/os', review: true
// }
const a = args || {}
const ROOT = typeof a.root === 'string' ? a.root : '/home/user/os'
const SP = typeof a.sp === 'string' ? a.sp : '/tmp/scratch'
const ROWS = Array.isArray(a.rows) ? a.rows : []
const REVIEW = a.review !== false
const LAYERS = {
  '8': { slug: '8-storage', name: 'Storage & Shared State', lock: 'D34, D38' },
  '9': { slug: '9-evolution', name: 'Self-Healing & Evolution', lock: 'D35, D39' },
}

const ctx = (L, layer) => `You build ONE row of the Layer ${layer} (${L.name}) build board in ${ROOT}, the 111network mesh OS. The root README.md is the
owner's vision and the source of truth for what the project is; the layer spec is the binding form of it. Read first: ${ROOT}/AGENTS.md (the
Raspberry Pi floor, Linux hygiene, the box rules; never add attribution lines), ${ROOT}/plan/EXECUTION.md, then the board ${ROOT}/${L.slug}/TASKS.md:
its header paragraph and box rules, the T0 paragraph (the scaffold your row fills: its stubs are committed), the ordering sentence of your row's wave,
and YOUR row whole (Task, Criteria, Check, Files, the "never adds" clause, any "Additive amendment", "Correction" or "Build watch" mark). Spec:
${ROOT}/${L.slug}/README.md (locked v1.0, ${L.lock} in plan/DECISIONS.md); read the sections your row names with sed -n ranges, and the locked spec of any
layer your row amends. Contract: ${ROOT}/4-interface/api/openapi.yaml (binding for every route and shape; conventions 4-interface/api/README.md). Every
file:line the row cites was pinned at an earlier head: re-read it before you edit. The status cells of the rows already built (✅) are as-built facts.

AS-BUILT FACTS FOR THIS RUN:
${a.facts || '(none given)'}

FILE OWNERSHIP IS STRICT: create or modify only the files your row's Files column lists (plus a new file inside a dir it names, the one claim of a
new test file in ${ROOT}/verify/checks.toml, and a tiny registration line a new module needs: list each). Other agents edit other files of this
tree right now (other rows of Layers 8 and 9, the Layer 7 gate scripts under 7-relay/verify/). If you need a change elsewhere, do not make it:
describe it under deviations. Never touch plan/, a TASKS.md board (unless your row's Files names one), INDEX.md or a spec (unless your row's Files
names it). A locked layer changes only as your row's mark says (additive, or a correction the spec names).

HOUSE RULES: Rust 2021, typed errors, never panic on input, sizes checked before parse, doc comments citing "§x.y" of your layer's spec, unit tests
in-module and integration tests under tests/, rustfmt and clippy --all-targets -D warnings clean (node111's 4 manifest unused_dependencies warnings,
sysinfo, os_info, nusb, rtnetlink, are known: ignore, never remove); no new dependency unless your row is the one that adds it; no wall clock in any
ordering or cryptographic decision; foreign bytes untrusted; secrets never echoed, logged or framed; every route problem+json and the agent's bearer
401 on every write; shell: POSIX sh or bash as the sibling scripts, shellcheck clean; TypeScript and the UI as 4-interface/ui's own conventions
(pnpm check); no model name in any file; never commit (the lead commits each row).

BOX: 4 CPUs; at most two cargo builds on the box and other agents hold them: run ONE cargo command at a time, with CARGO_INCREMENTAL=0
CARGO_BUILD_JOBS=2 and TMPDIR unset, from the workspace directory, and these warm target dirs: 1-bootstrap (node111) → ${SP}/target/node111,
5-messaging (mesh111) → ${SP}/target/mesh111, 3-intelligence (agent111) → ${SP}/target/agent111; 2-identity → ${SP}/target/id111. No other target dir:
the disk is tight (check df before a big build; under 3 GB free, stop and report). Iterate narrowly (cargo test -p <crate> --lib <module>, cargo
check), run your row's full Check once at the end. A build that fails inside another row's file is its half-made edit: wait 90 s, retry twice, then
report blocked. pnpm: from 4-interface/ui only. Scratch only under ${SP}/l${layer}-<row>/, deleted at the end. Hygiene: nothing of yours
running or listening when you return (ps, ss), no netns left.

Check that every new test fails without the code it tests (revert temporarily). RETURN a report: per criterion of your row MET / PARTIAL / NOT MET
with file:line; deviations (and the lock-amendment candidates your row lists); the commands you ran with their result lines; measured figures (or
"not measured" and why); the files you changed; open questions for the lead.`

const FINDINGS = {
  type: 'object', required: ['findings', 'sound'],
  properties: {
    findings: { type: 'array', items: { type: 'object', required: ['severity', 'where', 'defect', 'scenario', 'fix'], properties: {
      severity: { type: 'string' }, verdict: { type: 'string' }, where: { type: 'string' }, defect: { type: 'string' }, scenario: { type: 'string' }, fix: { type: 'string' } } } },
    sound: { type: 'array', items: { type: 'string' } },
  },
}

const key = (r) => `L${r.layer}:${r.id}`
const brief = (r) => `TASK ${r.id} (row "| ${r.id} |" of ${ROOT}/${LAYERS[r.layer].slug}/TASKS.md).${r.note ? '\n\nLEAD NOTE (read first): ' + r.note : ''}`

const review = (r, report) => agent(`You are a read-only adversarial reviewer in ${ROOT} (Layer ${r.layer} build). Do NOT run cargo, pnpm or any
build, do NOT edit any file, do NOT start a process. Review row "| ${r.id} |" of ${ROOT}/${LAYERS[r.layer].slug}/TASKS.md (read it whole) against
the spec sections it names (${ROOT}/${LAYERS[r.layer].slug}/README.md, locked) and the contract (${ROOT}/4-interface/api/openapi.yaml), reading the
uncommitted diff of the files the author reports (cd ${ROOT} && git diff --histogram -- <paths>; a new file: read it whole). Other rows are built
beside it: review only this row's files. The author's report (its declared deviations are not findings; report what it did not declare):
<<<${String(report).slice(0, 9000)}>>>
Hunt for: correctness bugs reachable from a real path (cite file:line now); a criterion unmet or met only by a test helper; the "never adds" clause
broken (a resident task, timer, poll, listener, file write, dependency on an idle node: the Raspberry Pi floor); a locked layer changed beyond the
row's mark; security (the agent's bearer on a write, a secret logged, foreign bytes trusted, a size unchecked, a consent or human step that can be
skipped, a path that escapes its root); contract mismatches; tests that pass without their fix. CONFIRMED when traced end to end, PLAUSIBLE
otherwise. Report only what survives your own scrutiny; list what you checked and found sound.`,
  { label: `${key(r)}-review`, phase: 'Rows', schema: FINDINGS, model: 'fable' })

const refix = (r, report, rv) => agent(`${ctx(LAYERS[r.layer], r.layer)}\n\n${brief(r)}\n\nYou built row ${r.id}; your report:
<<<${String(report).slice(0, 9000)}>>>
An independent reviewer reports: ${JSON.stringify(rv.findings).slice(0, 20000)}
Verify each against the code; fix the real ones, each with a test that fails without its fix; rebut the wrong ones with evidence. Re-run your
row's Check and the gates. RETURN per finding FIXED / REBUTTED with evidence, the gates with their result lines, the files changed.`,
  { label: `${key(r)}-refix`, phase: 'Rows', model: r.model || 'opus' })

// a FIFO mutex per lane (a shared target dir, a shared file set)
const mutexes = {}
const lock = (name) => {
  const m = mutexes[name] || (mutexes[name] = { tail: Promise.resolve() })
  let release
  const mine = new Promise((res) => { release = res })
  const prev = m.tail
  m.tail = prev.then(() => mine)
  return prev.then(() => release)
}

const done = {}
const settle = {}
for (const r of ROWS) done[key(r)] = new Promise((res) => { settle[key(r)] = res })

const runRow = async (r) => {
  const deps = (r.after || []).filter((d) => done[d])
  const missing = (r.after || []).filter((d) => !done[d])
  if (missing.length) log(`${key(r)}: follows ${missing.join(', ')}, outside this run (assumed landed)`)
  const depResults = await Promise.all(deps.map((d) => done[d]))
  if (depResults.some((x) => !x || x.error)) {
    const out = { id: r.id, layer: r.layer, error: `not started: a row it follows failed (${deps.join(', ')})` }
    settle[key(r)](out)
    return out
  }
  const release = r.lane ? await lock(r.lane) : () => {}
  let out
  try {
    const report = await agent(`${ctx(LAYERS[r.layer], r.layer)}\n\n${brief(r)}`, { label: key(r), phase: 'Rows', model: r.model || 'opus' })
    if (!report) out = { id: r.id, layer: r.layer, error: 'the author died' }
    else if (!REVIEW || r.review === false) out = { id: r.id, layer: r.layer, report }
    else {
      const rv = await review(r, report)
      if (!rv || !rv.findings.length) out = { id: r.id, layer: r.layer, report, review: rv }
      else out = { id: r.id, layer: r.layer, report, review: rv, refix: await refix(r, report, rv) }
    }
  } finally {
    release()
  }
  log(`${key(r)}: ${out.error || (out.review ? out.review.findings.length + ' findings' + (out.refix ? ', fixed' : '') : 'done')}`)
  settle[key(r)](out)
  return out
}

phase('Rows')
const results = await parallel(ROWS.map((r) => () => runRow(r)))
return { rows: results.filter(Boolean) }
