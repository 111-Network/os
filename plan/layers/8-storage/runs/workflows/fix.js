export const meta = {
  name: 'layers-8-9-spec-fix',
  description: 'Layers 8 and 9 spec v0.1 -> v0.2: one author per panel report applies its findings under the lead\'s triage (in sequence, one file), a re-pin pass corrects every file:line citation at HEAD, one read-only judge re-checks v0.2 against the triage and the reports',
  phases: [
    { title: 'Fix', detail: 'one opus author per panel report, in sequence (they edit one file)', model: 'opus' },
    { title: 'Re-pin', detail: 'one sonnet pass over every file:line citation', model: 'sonnet' },
    { title: 'Re-check', detail: 'one fable judge, read-only, writes review/recheck.md', model: 'fable' },
  ],
}

// args: { only: ['8'] | ['9'], from: '<lens>' (skip the lenses before it: their fixes are already in the file), skipRepin: true,
//         recheckOnly: true, extra: { '<lens>' | 'repin' | 'recheck': '<lead note>' } }
const ROOT = '/home/user/os'
const a = args || {}
const ONLY = Array.isArray(a.only) ? a.only.map(String) : ['8']
const EXTRA = a.extra || {}

const L = {
  8: { n: 8, slug: '8-storage', name: 'Storage & Shared State', dec: 'D34', lenses: ['store', 'datasets', 'integration'] },
  9: { n: 9, slug: '9-evolution', name: 'Self-Healing & Evolution', dec: 'D35', lenses: ['safety', 'observe', 'integration'] },
}

const common = (c) => `
Repo root ${ROOT}, branch claude/confident-ramanujan-y5ob49. Layer ${c.n} (${c.name}) of the 111network OS. The spec draft is
${ROOT}/${c.slug}/README.md. A panel of three judges read v0.1; their reports are ${ROOT}/plan/layers/${c.slug}/review/{${c.lenses.join(',')}}.md.
The lead ruled on them in ${ROOT}/plan/layers/${c.slug}/review/triage.md: THE TRIAGE IS THE LAW for this pass. It overrides a report's fix
where the two differ, it merges findings that overlap, it decides the forks, and its default is "a finding not named is accepted with the fix
its report gives". Above the triage: ${ROOT}/README.md (the owner's vision; never reworded or contradicted; a v1 limit is stated as a limit:
DECISIONS D36), ${ROOT}/plan/DECISIONS.md ${c.dec}, the brief ${ROOT}/plan/layers/${c.slug}/options.md.

Other agents are building Layer 7 and the test system in this tree: ignore their uncommitted edits, run no git write command, no cargo, start
no process. When \`git status --short\` lists a code file as modified, read its committed content (\`git show HEAD:<path>\`) for any fact
you cite. No model names in any file.
`

const fixPrompt = (c, lens, first) => `
You are a spec author. You apply one panel report to the Layer ${c.n} spec draft, turning v0.1 into v0.2.
${common(c)}
YOUR REPORT: ${ROOT}/plan/layers/${c.slug}/review/${lens}.md. Apply EVERY finding in it (blockers, majors, minors) under the triage's
rulings. You edit exactly two files: ${ROOT}/${c.slug}/README.md and ${ROOT}/plan/layers/${c.slug}/review/fix-map.md.

HOW
1. Read the triage, then your report, then the whole spec once. ${first ? 'You are the first author: set the spec\'s version line to "draft v0.2, 2026-11-12" (keep the wording around it) and create fix-map.md with a title line and one table (id | sections changed | what the text now says, one line).' : 'Earlier authors already applied the other report(s): read fix-map.md first. For a finding the triage merges with one already applied, check that the text covers your report\'s angle too, add only what is missing, and add your id to that row.'}
2. For each finding: change the normative text where the finding points and everywhere else the same fact is stated (the tables in the
   accounting and scope sections, the gates, the amendments list, the API section, the deviations and numbers section). A rule without an
   assertion is not done: each changed or new rule gets its assertion in the gate the triage or the report names, with the seam it proves.
3. Keep the structure stable: other layers and the Layer 9 spec cite this file's section and contract numbers. Never renumber a section, a
   contract item or a gate; add sub-items at the end of a list or as lettered items; a new section goes at the end of its part.
4. Write in the file's own style: plain declarative sentences, the same citation key as its section 0, tables where it uses tables. No
   change log inside the spec: the map of what changed is fix-map.md, one row per finding id.
5. A code fact you write (a function, a route, a constant, a line) is verified by reading the code at HEAD first; mark what you could not
   verify "(unverified from this box)". A number the triage gives is used as given; a number nobody gave is your default, marked
   "(default; measured at build)" and listed where the spec lists its numbers.
6. Where the triage says "listed in section 22 for the owner" (or the spec's equivalent deviations section), add that entry there.
7. Do not fix what your report does not name, except a contradiction your own edit would create. If a finding cannot be applied as ruled
   (the code makes it impossible), do not improvise: leave the text, and return it in not_applied with the reason.
${EXTRA[lens] ? '\nLEAD NOTE: ' + EXTRA[lens] + '\n' : ''}
RETURN the structured result: status, applied (id, sections, one line each), not_applied (id, why), new_numbers (each default you set),
notes (at most 10 lines for the lead).
`

const FIX_SCHEMA = {
  type: 'object',
  required: ['status', 'applied', 'not_applied', 'new_numbers', 'notes'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    applied: { type: 'array', items: { type: 'object', required: ['id', 'sections'], properties: { id: { type: 'string' }, sections: { type: 'string' }, note: { type: 'string' } } } },
    not_applied: { type: 'array', items: { type: 'object', required: ['id', 'why'], properties: { id: { type: 'string' }, why: { type: 'string' } } } },
    new_numbers: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

const repinPrompt = (c) => `
You re-pin the citations of a spec. One file is yours to edit: ${ROOT}/${c.slug}/README.md.
${common(c)}
TASK: every \`file:line\` (and \`file:line-line\`) citation in the spec must be true at HEAD (\`git rev-parse --short HEAD\`; use the
committed content, \`git show HEAD:<path>\`, for every file). The spec's section 0 gives the citation key (the short prefixes for the crates,
the contract and the other layers' READMEs). Work through the file top to bottom in chunks: for each citation, open the file, find the
symbol or sentence the spec's text names, and correct the line number(s). Then update the sentence in section 0 that names the head the
citations were read at. Change NOTHING else: no wording, no numbers, no rules. A citation whose fact you cannot find at HEAD is left as it
is with "(unverified from this box)" after it, and listed in your result. Also correct "v4, v5 in flight" style statements only where the
triage says so.
${EXTRA.repin ? '\nLEAD NOTE: ' + EXTRA.repin + '\n' : ''}
RETURN: head, checked (count), corrected (count), unverified (the citations you could not find, with the section), notes.
`

const REPIN_SCHEMA = {
  type: 'object',
  required: ['head', 'checked', 'corrected', 'unverified', 'notes'],
  properties: {
    head: { type: 'string' }, checked: { type: 'number' }, corrected: { type: 'number' },
    unverified: { type: 'array', items: { type: 'string' } }, notes: { type: 'string' },
  },
}

const recheckPrompt = (c) => `
You are a READ-ONLY re-check judge. One file is yours to write: ${ROOT}/plan/layers/${c.slug}/review/recheck.md.
${common(c)}
The spec is now v0.2. ${ROOT}/plan/layers/${c.slug}/review/fix-map.md says what each author changed for each finding id.

TASK: decide whether v0.2 can lock.
1. For EVERY finding id in the three reports: is it fixed as the triage rules (or, where the triage is silent, as the report's fix says)?
   Read the spec's text, not the map. Verdict per id: fixed | partly | not fixed | fixed differently (say whether that is acceptable).
2. Hunt for what the fixes broke: two edits that contradict each other, a section or contract number that moved, a gate assertion whose rule
   was removed, a rule added without an assertion, a number stated in two places with two values, an amendment to a locked layer that is no
   longer additive, a sentence that rewords or contradicts the root README, a new resident process, timer, poll, listener or dependency.
3. Spot-check thirty file:line citations spread over the file against HEAD (committed content).
Severity as the panel used it (blocker = cannot lock). One fix per finding; no finding without a fix.

WRITE the report: a verdict line (lock | amend-then-lock), a table "Not fixed or partly fixed" (id, what is missing, fix), a table "New in
v0.2" (id starting with R, where, what, fix, severity), a one-line count of the ids fixed, at most 10 lines of notes. RETURN the same.
${EXTRA.recheck ? '\nLEAD NOTE: ' + EXTRA.recheck + '\n' : ''}`

const RECHECK_SCHEMA = {
  type: 'object',
  required: ['verdict', 'fixed', 'open', 'new_findings', 'report_file', 'notes'],
  properties: {
    verdict: { type: 'string', enum: ['lock', 'amend-then-lock'] },
    fixed: { type: 'number' },
    open: { type: 'array', items: { type: 'object', required: ['id', 'state', 'fix'], properties: { id: { type: 'string' }, state: { type: 'string' }, missing: { type: 'string' }, fix: { type: 'string' } } } },
    new_findings: { type: 'array', items: { type: 'object', required: ['id', 'severity', 'where', 'what', 'fix'], properties: { id: { type: 'string' }, severity: { type: 'string', enum: ['blocker', 'major', 'minor'] }, where: { type: 'string' }, what: { type: 'string' }, fix: { type: 'string' } } } },
    report_file: { type: 'string' },
    notes: { type: 'string' },
  },
}

const out = {}
for (const k of ONLY) {
  const c = L[k]
  if (!c) continue
  const r = { fixes: [] }
  if (!a.recheckOnly) {
    const start = a.from ? Math.max(0, c.lenses.indexOf(a.from)) : 0
    for (let i = start; i < c.lenses.length; i++) {
      const lens = c.lenses[i]
      const f = await agent(fixPrompt(c, lens, i === 0), { label: `fix${c.n}:${lens}`, phase: 'Fix', model: 'opus', effort: 'high', schema: FIX_SCHEMA })
      if (!f) { log(`Layer ${c.n} fix ${lens}: the author died; stop (re-run with from: '${lens}' and an extra note)`); r.stopped = lens; break }
      log(`Layer ${c.n} fix ${lens}: ${f.status}, ${f.applied.length} applied, ${f.not_applied.length} not applied`)
      r.fixes.push({ lens, ...f })
      if (f.status === 'blocked') { r.stopped = lens; break }
    }
    if (!r.stopped && !a.skipRepin) {
      r.repin = await agent(repinPrompt(c), { label: `repin${c.n}`, phase: 'Re-pin', model: 'sonnet', effort: 'medium', schema: REPIN_SCHEMA })
      if (r.repin) log(`Layer ${c.n} re-pin: ${r.repin.checked} checked, ${r.repin.corrected} corrected, ${r.repin.unverified.length} unverified`)
    }
  }
  if (!r.stopped) {
    r.recheck = await agent(recheckPrompt(c), { label: `recheck${c.n}`, phase: 'Re-check', model: 'fable', effort: 'high', schema: RECHECK_SCHEMA })
    if (r.recheck) log(`Layer ${c.n} re-check: ${r.recheck.verdict}, ${r.recheck.fixed} fixed, ${r.recheck.open.length} open, ${r.recheck.new_findings.length} new`)
  }
  out[k] = r
}
return out
