export const meta = {
  name: 'layer-t0-scaffold',
  description: 'A Layer 8 or 9 T0 scaffold (args.layer): optionally authored by one agent, then three read-only review lenses (the board and spec, the test map, the running daemon and the Pi floor), each finding adversarially verified, one fix pass on the confirmed ones; the lead re-runs the checks and commits "Layer N scaffold"',
  phases: [
    { title: 'Author', detail: 'one opus agent writes the T0 paragraph exactly (skipped when args.author is false)' },
    { title: 'Review', detail: 'three lenses, read-only' },
    { title: 'Verify', detail: 'each finding: one skeptic tries to refute it' },
    { title: 'Fix', detail: 'one agent fixes the confirmed findings and re-runs the checks' },
  ],
}
// args: { layer: '8' | '9', author: true|false, report: '<the author report when author is false>', brief: '<the author brief>', sp, root }
const a = args || {}
const ROOT = typeof a.root === 'string' ? a.root : '/home/user/os'
const SP = typeof a.sp === 'string' ? a.sp : '/tmp/scratch'
const LAYER = String(a.layer || '9')
const L = {
  '8': { slug: '8-storage', name: 'Storage & Shared State', seams: '§18 ("Seam ids", the table)', gates: 'G13-G18 and G7 extended' },
  '9': { slug: '9-evolution', name: 'Self-Healing & Evolution', seams: '§20 (the seam table)', gates: 'G19-G25 and G7 extended' },
}[LAYER]

const FINDINGS = {
  type: 'object', required: ['findings', 'sound'],
  properties: {
    findings: { type: 'array', items: { type: 'object', required: ['severity', 'where', 'defect', 'evidence', 'fix'], properties: {
      severity: { type: 'string' }, where: { type: 'string' }, defect: { type: 'string' }, evidence: { type: 'string' }, fix: { type: 'string' } } } },
    sound: { type: 'array', items: { type: 'string' } },
  },
}
const VERDICT = {
  type: 'object', required: ['real', 'why'],
  properties: { real: { type: 'boolean' }, why: { type: 'string' }, corrected_fix: { type: 'string' } },
}

const SCOPE = `the Layer ${LAYER} (${L.name}) T0 scaffold in ${ROOT}, the 111network mesh OS: the paragraph "## T0 — lead scaffold" of
${ROOT}/${L.slug}/TASKS.md is the task, exactly (read it whole, and the rows it names as the fillers of each stub). Spec: ${ROOT}/${L.slug}/README.md
(locked v1.0). The scaffold is UNCOMMITTED in the working tree: \`cd ${ROOT} && git status --short\` and \`git diff\` show it (new files: read them
whole). Other agents are editing other files of this tree right now (Layer 7 gate scripts under 7-relay/verify/, other rows): judge only the T0
paragraph's files and the registration lines it names.`

const LENSES = [
  { key: 'board', prompt: `LENS: the board and the spec. For every item of the T0 paragraph: is it there, named as the paragraph names it, and nothing
beyond it? Each stub file: its first lines carry "T0" (the test-map lint's marker), its doc cites the right § of the spec and the row(s) that fill it
(check each against the board's Files columns), it declares names only (types, fns that return at once), constructs, spawns, opens, reads and writes
nothing. Named behaviours: every start call, exit status and message, constant, usage text and registration line the paragraph names, exactly
(compare strings byte for byte with the spec sentence they come from). Constants or lists copied from another file (LICENSING.md, the spec): copied,
not invented.` },
  { key: 'map', prompt: `LENS: the test map. verify/seams.toml and verify/checks.toml: the seams the paragraph adds (their ids, status, spec refs that
resolve to real headings or bold leads, read_first, symptom in the code's and contract's words, per the spec's ${L.seams}); which existing planned seams
now have T0 side files, and whether each side lists files of the right layer (a = the id's first layer, b the second); a planned seam with an
existing side file draws the lint's stub-marker warning (it should be stub then, unless the paragraph says planned and lists no files); the gate
stubs of ${L.gates}: one check each, the shape of the existing l7.* T0 stubs (kind, tier ns, adapter okfail, lock verify, strength N, needs, the
three-line comment), the seams of each gate per the spec's table; [unmapped] for the sourced lib.sh. Run \`cd ${ROOT} && python3 -I -B verify/run.py
--lint\` (read-only: allowed) and \`--explain <id>\` for a new check; read its findings and warnings, and separate those of this scaffold from other
rows' (another row's unclaimed file is not this scaffold's).` },
  { key: 'runtime', prompt: `LENS: the running daemon and the Raspberry Pi floor (AGENTS.md: nothing new may run, poll, listen, open or weigh in the
required configuration). Trace every line the scaffold adds to a code path that runs (N/mod.rs daemon_main, api_v1/mod.rs's router, lib.rs, any
module's start fn): at start and per request, does anything now touch the disk, allocate per event, log, take a lock, spawn, or change an exit status
or a route's answer (a merged empty router must leave the shared problem+json 404 for its paths unchanged, and must not shadow an existing route)?
Is the start-call order the paragraph pins (e.g. a lock taken before any store opens, a start call before the fabric boots) true in daemon_main
as it is now? An Err arm: the exit status and the exact message; no panic path. Clippy-relevant hazards: a dead_code allow scoped to the stub, no
unused import or variable in a non-stub file, nothing rustfmt would change. The gate stub scripts: SKIPPED and exit 0 plain, FAIL and non-zero
under their REQUIRE switch; you may run each stub script (they start nothing) and shellcheck; do NOT run cargo or any build.` },
]

const authorOut = a.author === false ? null : await (async () => {
  phase('Author')
  return agent(a.brief || `Write ${SCOPE}`, { label: `L${LAYER}:T0`, phase: 'Author', model: 'opus' })
})()
const report = a.author === false ? (a.report || '(no report given)') : authorOut
if (!report) return { error: 'the author died' }

phase('Review')
const reviews = await pipeline(
  LENSES,
  (lens) => agent(`You are a read-only reviewer of ${SCOPE}\n\n${lens.prompt}\n\nDo NOT edit any file, do NOT run cargo or any build, do NOT start
a daemon. The author's report (declared deviations are not findings unless they break the paragraph): <<<${String(report).slice(0, 12000)}>>>
Report only defects you traced in the files (cite file:line), with the evidence (the paragraph's or spec's words against the file's), and the fix;
list what you checked and found sound.`, { label: `L${LAYER}:T0-${lens.key}`, phase: 'Review', schema: FINDINGS, model: 'fable' }),
  (rv, lens) => parallel((rv && rv.findings ? rv.findings : []).map((f, i) => () => agent(`You are a skeptic. A reviewer of ${SCOPE} claims a
defect. Try to REFUTE it: read the files and the paragraph yourself (read-only; no edit, no build). Default to real=false when the claim is not
traced in the files or the paragraph does not ask for what it demands. Claim (${lens.key} lens): ${JSON.stringify(f)}
Return real (true only if you traced it), why (your evidence), and corrected_fix when the proposed fix is wrong or too wide.`,
    { label: `L${LAYER}:T0-verify-${lens.key}-${i + 1}`, phase: 'Verify', schema: VERDICT, model: 'opus' })
    .then(v => ({ lens: lens.key, ...f, verdict: v })))),
)
const all = reviews.filter(Boolean).flat().filter(Boolean)
const confirmed = all.filter(f => f.verdict && f.verdict.real)
log(`review: ${all.length} findings, ${confirmed.length} confirmed`)

let fixed = null
if (confirmed.length) {
  phase('Fix')
  fixed = await agent(`You fix ${SCOPE}\n\nIndependent reviewers found these defects, each confirmed by a skeptic (use corrected_fix where given):
${JSON.stringify(confirmed).slice(0, 30000)}
Fix each in the scaffold's own files and registration lines only (never another row's file, never plan/ or a board). Then run the checks the
paragraph names: cargo fmt --check (rustfmt --edition 2021 --check on the files), cargo clippy -p node111 --all-targets -- -D warnings (the 4
manifest unused_dependencies warnings sysinfo, os_info, nusb, rtnetlink are known), cargo clippy -p mesh111 --all-targets -- -D warnings when a
mesh111 file changed, cargo test -p node111 --no-run, the stubs plain and under REQUIRE, shellcheck, python3 -I -B verify/run.py --lint. Cargo: ONE
command at a time, CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2, TMPDIR unset, CARGO_TARGET_DIR=${SP}/target/node111 (1-bootstrap) or
${SP}/target/mesh111 (5-messaging); a build failing inside another row's file (7-relay/verify is shell; any other) is its half-made edit: wait 90 s,
retry twice, then report blocked. Never commit. RETURN per finding FIXED / NOT FIXED (why), each check's result line, the files changed.`,
    { label: `L${LAYER}:T0-fix`, phase: 'Fix', model: 'opus' })
}
return { layer: LAYER, report: a.author === false ? '(given)' : report, findings: all, confirmed: confirmed.length, fix: fixed }
