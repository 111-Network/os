export const meta = {
  name: 'layers-8-9-spec-amend',
  description: 'Layers 8 and 9 spec v0.2 -> v0.3: one author applies the re-check (the partly-fixed ids and the new findings) under the lead\'s ruling, then one read-only judge reads the amend diff against the re-check and says lock or amend',
  phases: [
    { title: 'Amend', detail: 'one author applies review/recheck.md to the spec', model: 'opus' },
    { title: 'Verify', detail: 'one read-only judge reads the diff, writes review/recheck-2.md', model: 'fable' },
  ],
}

// args: { layer: '8' | '9', ruling: '<the lead\'s ruling on the re-check, appended to the default>', verifyOnly: true }
const ROOT = '/home/user/os'
const a = args || {}
const L = {
  8: { n: 8, slug: '8-storage', name: 'Storage & Shared State', dec: 'D34' },
  9: { n: 9, slug: '9-evolution', name: 'Self-Healing & Evolution', dec: 'D35' },
}
const c = L[String(a.layer || '8')]
const R = `${ROOT}/plan/layers/${c.slug}/review`
const SPEC = `${ROOT}/${c.slug}/README.md`

const common = `
Repo root ${ROOT}, branch claude/confident-ramanujan-y5ob49. Layer ${c.n} (${c.name}) of the 111network mesh OS. The spec draft is ${SPEC}
(v0.2, committed). Its history: a panel of three judges read v0.1 (${R}/*.md), the lead ruled (${R}/triage.md), three authors amended it to
v0.2 (${R}/fix-map.md says where each finding went), and one judge re-checked v0.2: ${R}/recheck.md. That re-check is your input: its "open"
rows (ids fixed only partly) and its "New in v0.2" table (R ids), each with a fix.
ORDER OF AUTHORITY: ${ROOT}/README.md (the owner's vision: quoted exactly, never reworded or contradicted; a v1 limit is stated as a limit:
plan/DECISIONS.md D36) > plan/DECISIONS.md ${c.dec} > ${ROOT}/plan/layers/${c.slug}/options.md > ${R}/triage.md > the lead's ruling on the
re-check (below) > ${R}/recheck.md. ${ROOT}/AGENTS.md: the Raspberry Pi floor (no resident process, periodic poll, timer, listener, runtime
dependency or build-time weight is added to the required configuration).
THE LEAD'S RULING ON THE RE-CHECK: every "partly" id and every R id of recheck.md is accepted with the fix the re-check gives, with these
additions: ${a.ruling || '(none)'}`

const FIX = {
  type: 'object',
  required: ['status', 'applied', 'not_applied', 'head', 'notes'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    applied: { type: 'array', items: { type: 'object', required: ['id', 'sections', 'note'], properties: {
      id: { type: 'string' }, sections: { type: 'string' }, note: { type: 'string' } } } },
    not_applied: { type: 'array', items: { type: 'object', required: ['id', 'why'], properties: { id: { type: 'string' }, why: { type: 'string' } } } },
    head: { type: 'string' },
    notes: { type: 'string' },
  },
}
const VERDICT = {
  type: 'object',
  required: ['verdict', 'ids', 'regressions', 'report_file', 'notes'],
  properties: {
    verdict: { type: 'string', enum: ['lock', 'amend'] },
    ids: { type: 'array', items: { type: 'object', required: ['id', 'state'], properties: {
      id: { type: 'string' }, state: { type: 'string', enum: ['fixed', 'fixed-differently-acceptable', 'partly', 'not-fixed'] },
      missing: { type: 'string' }, fix: { type: 'string' } } } },
    regressions: { type: 'array', items: { type: 'object', required: ['where', 'what', 'fix', 'severity'], properties: {
      where: { type: 'string' }, what: { type: 'string' }, fix: { type: 'string' }, severity: { type: 'string', enum: ['blocker', 'major', 'minor'] } } } },
    report_file: { type: 'string' },
    notes: { type: 'string' },
  },
}

let fix = null
if (!a.verifyOnly) {
  phase('Amend')
  fix = await agent(`You amend the Layer ${c.n} spec from v0.2 to v0.3. You edit ONLY ${SPEC} and ${R}/fix-map.md. No build, no cargo, no pnpm, no
process, no git write command (no commit, stash, checkout, reset). Other agents are editing other files of this repo right now: never touch them.
${common}

DO, in this order:
1. Read recheck.md whole, then the spec whole once, then fix-map.md.
2. Apply every "open" (partly) id and every R id, each with the fix the re-check gives (or the lead's addition above where it names the id).
   NEVER renumber a section, a gate, a deviation or a table row: add at the end of a list, or as a lettered item (the spec already does
   this). Every changed or new rule gets its assertion in the gate the fix names (a rule with no assertion is not done) and, where it
   touches another layer, its line in the section that lists what that layer owes. A new route goes in the routes table with who may call
   it; a new number goes in the Numbers list marked "default, measured at build".
3. Finish the citation pin: read \`git rev-parse --short HEAD\` once at the start; every file:line the spec cites is correct at that head
   (re-derive a moved one by the symbol or sentence it names, with \`git show HEAD:<path>\`), the spec names that ONE head in its pin
   sentence, and no statement says "at <an older head>". Where a fact changed since the older head (the re-check names them: the agent's
   tool count is 20 since Layer 7 row A1), correct the fact and everything derived from it.
4. The version line reads "draft v0.3, 2026-11-13" (keep the wording around it); the changelog gains one entry for v0.2 (the panel's
   findings applied under the triage) and one for v0.3 (the re-check's), two or three lines each.
5. Append one row per id to fix-map.md (id, sections, one-line note), under a heading "## v0.3 (the re-check)"; correct fix-map.md's own
   head reference.
6. Read your whole diff once (git diff -- ${c.slug}/README.md): no sentence of the root README is reworded, no contradiction between two
   sections you touched, no orphan reference to a name you changed.
Plain, exact prose in the spec's own voice; no hedging; no model names.
RETURN: status; one entry per id (the sections changed, a one-line note); any id you could not apply and why; the head you pinned; notes
(every name, number or route you had to choose yourself, so the lead can rule on it).`,
    { label: `L${c.n}:amend`, phase: 'Amend', schema: FIX, model: 'opus', effort: 'high' })
  if (!fix) return { error: 'the amend author returned nothing' }
  log(`amend: ${fix.status}, ${fix.applied.length} ids applied, ${fix.not_applied.length} not applied`)
}

phase('Verify')
const verdict = await agent(`You are a read-only judge: edit no file except the one report you write (${R}/recheck-2.md), run no build and no process.
${common}

An author has just amended the spec to v0.3 (uncommitted): read the amend with \`cd ${ROOT} && git diff -- ${c.slug}/README.md\`, and the
spec sections around each hunk as they now stand. The author's own report (claims, not facts):
${fix ? JSON.stringify(fix).slice(0, 12000) : '(not available: judge from the diff and fix-map.md)'}

JUDGE:
1. Every "open" (partly) id and every R id of recheck.md: fixed as the re-check's fix (or the lead's addition) says / fixed differently
   and acceptable / partly / not fixed. For anything short of fixed: what is missing and the exact fix.
2. Regressions the amend itself introduced, and only those (do not re-open what recheck.md already accepted): a contradiction between two
   sections, a rule with no assertion, a route with no caller rule, a name used two ways, a renumbered section or gate, a root README
   sentence reworded, a locked layer's contract changed other than additively, weight added to the Pi floor, a number with no source.
3. Sample the citation pin: at least thirty file:line citations across the files the re-check named as moved, read with
   \`git show HEAD:<path>\`; list each wrong one. One pinned head only.
VERDICT: "lock" when every id is fixed (or fixed differently and acceptable) and no regression is a blocker or a major; minors may ride
with a lock if each has a one-line fix you give (the lead applies them at the lock). Otherwise "amend".
Write ${R}/recheck-2.md: the verdict in its first line, the per-id table, the regressions table (id-less: where, what, fix, severity),
the citations you found wrong, and notes for the lock commit. Exact, no hedging, no model names. Then RETURN the same as structured data.`,
  { label: `L${c.n}:verify`, phase: 'Verify', schema: VERDICT, effort: 'high' })

return { fix, verdict }
