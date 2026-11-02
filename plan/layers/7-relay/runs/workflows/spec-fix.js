export const meta = {
  name: 'layer-7-spec-fix',
  description: 'Layer 7: one opus spec editor applies the lead triage (plan/layers/7-relay/review/triage.md) to 7-relay/README.md -> v0.2; one fable judge re-checks every disposition and writes review/recheck.md',
  phases: [
    { title: 'Fix', detail: 'opus editor applies the accepted dispositions, bumps the changelog', model: 'opus' },
    { title: 'Recheck', detail: 'one fable judge verifies each disposition landed and nothing regressed', model: 'fable' },
  ],
}

const ROOT = '/home/user/os'
const a = args || {}
const TRIAGE = a.triage || `${ROOT}/plan/layers/7-relay/review/triage.md`
const FROM = a.from || 'v0.1'
const TO = a.to || 'v0.2'

const FIX_PROMPT = `
You are the spec editor for the 111network OS, Layer 7 (Relay & Presence). Today is 2026-11-02. Repo root ${ROOT}. You edit exactly one
file: ${ROOT}/7-relay/README.md (draft ${FROM}), producing ${TO}. You edit nothing else, run no git write commands, no cargo build or
test, start no process; read-only greps and file reads are fine.

THE INPUT: ${TRIAGE} is the lead's triage of the three-judge panel (the panel reports are ${ROOT}/plan/layers/7-relay/review/custody.md,
carriers.md, integration.md). Every finding there carries a disposition: accept as proposed, accept with the lead's variation (the
variation text wins), owner-answered (the answer text wins), or rejected (do nothing for it). Apply every accepted and owner-answered
disposition to the spec, exactly, in the section the finding names; where a finding's fix conflicts with another accepted fix, the
triage says which wins; if it does not, apply the stricter (fail-closed, lighter on the Pi floor) reading and record it in your
deviations. The law stays ${ROOT}/plan/DECISIONS.md D30 (and D31 if the triage says it exists): no change may widen v1 beyond D30, add
a Layer 5 spec amendment beyond the twelve (the byte-identical code seams the triage accepts under Q1 and Q2 are recorded in §19, not counted among the twelve), or add resident weight on a non-hub node.

RULES FOR THE TEXT: keep the section numbers and the voice; every new built-code claim cited file:line and verified by reading; no
model names or identifiers anywhere (write "the lead", "the panel", "a judge"); no attribution lines; every new setting named with
its default; every new wire byte accounted in a table. Append "Changelog ${FROM} -> ${TO}" at the end, one line per applied finding
id (e.g. "C3: ..."), in the style of 6-transport/README.md's changelogs. Bump the title to "spec ${TO}".

RETURN (structured): status (written | partial | blocked), applied (the finding ids you applied), skipped (ids you could not apply
and why, one line each), deviations (one line each), lines (the file's new line count), notes (at most 10 lines).
`

const FIX_SCHEMA = {
  type: 'object',
  required: ['status', 'applied', 'skipped', 'deviations', 'lines', 'notes'],
  properties: {
    status: { type: 'string', enum: ['written', 'partial', 'blocked'] },
    applied: { type: 'array', items: { type: 'string' } },
    skipped: { type: 'array', items: { type: 'string' } },
    deviations: { type: 'array', items: { type: 'string' } },
    lines: { type: 'number' },
    notes: { type: 'string' },
  },
}

const RECHECK_PROMPT = `
You are a READ-ONLY re-check judge for the 111network OS, Layer 7 (Relay & Presence) spec ${TO} at ${ROOT}/7-relay/README.md. Today is
2026-11-01. Repo root ${ROOT}. You edit nothing except your report file; no git write commands, no cargo build or test, no process.

THE INPUT: the three panel reports (${ROOT}/plan/layers/7-relay/review/custody.md, carriers.md, integration.md), the lead's triage
${TRIAGE} (dispositions per finding) and the editor's result the lead gives you below. The law: ${ROOT}/plan/DECISIONS.md D30 (and D31
if present), the brief ${ROOT}/plan/layers/7-relay/options.md, the locked specs the draft amends and the as-built code (verify
file:line claims by reading when a disposition depends on them).

METHOD: for every finding with an accepted or owner-answered disposition, find where ${TO} carries it and judge whether it does what
the disposition says (landed | partial | missing | landed-wrong). For every rejected finding, confirm the spec did not change for it.
Then read the whole spec once more for regressions the edits introduced: a contradiction between sections, a number that no longer
matches, a default that is no longer off, a v1.1 item pulled into v1, a Layer 5 spec amendment beyond the twelve (the §19 code seams under Q1/Q2 are allowed), a resident cost added on a
non-hub node, a model name. Severity as the panel used: blocker, major, minor.

WRITE ${ROOT}/plan/layers/7-relay/review/recheck.md: a verdict line (lock | amend-again), a table of every disposition with its state,
then Blockers / Majors / Minors tables for anything new (id R plus a number; where; what; fix), then at most 10 lines of notes. Then
RETURN the structured result.
`

const RECHECK_SCHEMA = {
  type: 'object',
  required: ['verdict', 'dispositions', 'findings', 'report_file', 'notes'],
  properties: {
    verdict: { type: 'string', enum: ['lock', 'amend-again'] },
    dispositions: {
      type: 'array',
      items: {
        type: 'object',
        required: ['id', 'state'],
        properties: {
          id: { type: 'string' },
          state: { type: 'string', enum: ['landed', 'partial', 'missing', 'landed-wrong', 'rejected-unchanged', 'rejected-but-changed'] },
          note: { type: 'string' },
        },
      },
    },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        required: ['id', 'severity', 'where', 'what', 'fix'],
        properties: {
          id: { type: 'string' },
          severity: { type: 'string', enum: ['blocker', 'major', 'minor'] },
          where: { type: 'string' },
          what: { type: 'string' },
          fix: { type: 'string' },
        },
      },
    },
    report_file: { type: 'string' },
    notes: { type: 'string' },
  },
}

let fix = null
if (!a.recheckOnly) {
  phase('Fix')
  log(`Layer 7 spec ${FROM} -> ${TO}: the editor applies ${TRIAGE}`)
  fix = await agent(FIX_PROMPT, { label: 'spec:fix', phase: 'Fix', model: 'opus', effort: 'xhigh', schema: FIX_SCHEMA })
  if (!fix || fix.status === 'blocked') return { fix, recheck: null }
  log(`fix: ${fix.status}, applied ${fix.applied.length}, skipped ${fix.skipped.length}, ${fix.lines} lines`)
}

phase('Recheck')
const summary = fix ? `EDITOR RESULT: applied ${fix.applied.join(', ')}; skipped: ${fix.skipped.join(' | ') || 'none'}; deviations: ${fix.deviations.join(' | ') || 'none'}.` : 'EDITOR RESULT: not available (recheck-only run); judge the file as it stands.'
const recheck = await agent(RECHECK_PROMPT + '\n\n' + summary, { label: 'spec:recheck', phase: 'Recheck', model: 'fable', effort: 'high', schema: RECHECK_SCHEMA })
if (recheck) {
  const n = s => recheck.findings.filter(f => f.severity === s).length
  log(`recheck: ${recheck.verdict} - ${n('blocker')} blockers, ${n('major')} majors, ${n('minor')} minors`)
}
return { fix, recheck }
