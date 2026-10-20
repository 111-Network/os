export const meta = {
  name: 'layer-6-wave-review',
  description: 'Layer 6: one read-only fable judge per committed build task — reads the commit against the locked spec and the task row, returns findings (blocker / major / minor) for the lead to bounce',
  phases: [{ title: 'Review', detail: 'one judge per task (args: [{id, commit, row, sections}]), parallel' }],
}

const ROOT = '/home/user/os'

const CTX = `
You are a READ-ONLY review judge for the 111network OS, Layer 6 (Transport & Bridge). Today is 2026-10-19. Repo root: ${ROOT}.
You never edit files, never run git write commands, never run cargo builds or tests that take more than a minute (the lead ran the
checks; you read). The law: the locked spec ${ROOT}/6-transport/README.md (v1.0, D27), the task row in ${ROOT}/6-transport/TASKS.md,
${ROOT}/plan/DECISIONS.md D25–D27, and AGENTS.md (the Raspberry Pi is the floor: no resident weight added to the base configuration).

METHOD: read the task row and the spec sections named for it; then \`git show <commit> --stat\` and the full diff (\`git show <commit>\`);
then open the touched files in the working tree to read the surrounding code where the diff alone misleads. Attack the change: does
it do what the row's criteria say, exactly, with the spec's fail-closed readings? What did it skip, fake, or widen? Is any test a
tautology (asserts what it set up), any check skipped (musl SKIP is expected on this box; anything else is not), any wall clock used
for ordering or crypto, any untrusted string rendered or parsed as instructions, any new resident process/timer/poll in the base
configuration, any new dependency without a sign-off, any file outside the row's ownership touched without being listed, any
model name or identifier in a file? Verify a claim by reading, not by trusting the commit message. Cite file:line.

RETURN (structured): task id, verdict (accept | amend | reject), findings: a list of {severity: blocker|major|minor, where (file:line),
what (one or two sentences), fix (one sentence)}, and notes (≤ 10 lines: what is good, what to watch in later waves). Be specific and
short; the lead bounces blockers and majors to a fix agent and records minors.
`

const RESULT = {
  type: 'object',
  required: ['task', 'verdict', 'findings', 'notes'],
  properties: {
    task: { type: 'string' },
    verdict: { type: 'string', enum: ['accept', 'amend', 'reject'] },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        required: ['severity', 'where', 'what', 'fix'],
        properties: {
          severity: { type: 'string', enum: ['blocker', 'major', 'minor'] },
          where: { type: 'string' },
          what: { type: 'string' },
          fix: { type: 'string' },
        },
      },
    },
    notes: { type: 'string' },
  },
}

const tasks = Array.isArray(args) ? args : []
if (!tasks.length) throw new Error('args must be [{id, commit, row, sections}]')

phase('Review')
log('Layer 6 review: ' + tasks.map(t => `${t.id}@${t.commit.slice(0, 7)}`).join(' · '))
const out = (await parallel(tasks.map(t => () =>
  agent(CTX + `\n\nTASK ${t.id} — commit ${t.commit}. Row: ${t.row}. Spec sections: ${t.sections}.${t.extra ? ' Extra context from the lead: ' + t.extra : ''}`,
    { label: `review:${t.id}`, phase: 'Review', schema: RESULT, model: 'fable', effort: 'high' })
    .then(r => r && { ...r, id: t.id })
))).filter(Boolean)
log('verdicts: ' + out.map(r => `${r.id}=${r.verdict} (${r.findings.length})`).join(' · '))
return out
