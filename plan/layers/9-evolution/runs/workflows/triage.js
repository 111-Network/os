export const meta = {
  name: 'layer-9-triage',
  description: 'Layer 9 spec v0.1: draft the lead\'s triage of the three panel reports (merged findings, forks ruled, build order), then one read-only check of the draft against the reports, the vision and the Layer 8 storage contract',
  phases: [
    { title: 'Draft', detail: 'one author reads the three reports and the spec, writes review/triage.md', model: 'opus' },
    { title: 'Check', detail: 'one read-only judge hunts contradictions and unruled forks', model: 'fable' },
  ],
}

const ROOT = '/home/user/os'
const R = `${ROOT}/plan/layers/9-evolution/review`

const DRAFT = {
  type: 'object',
  required: ['file', 'merged', 'forks', 'lock_amendments', 'notes'],
  properties: {
    file: { type: 'string' },
    merged: { type: 'array', items: { type: 'object', required: ['ids', 'ruling'], properties: { ids: { type: 'string' }, ruling: { type: 'string' } } } },
    forks: { type: 'array', items: { type: 'object', required: ['ids', 'question', 'options', 'ruling', 'why', 'confidence'], properties: {
      ids: { type: 'string' }, question: { type: 'string' }, options: { type: 'array', items: { type: 'string' } },
      ruling: { type: 'string' }, why: { type: 'string' }, confidence: { type: 'string', enum: ['high', 'medium', 'low'] },
      owner_visible: { type: 'boolean' } } } },
    lock_amendments: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}
const CHECK = {
  type: 'object',
  required: ['problems', 'sound', 'not_covered'],
  properties: {
    problems: { type: 'array', items: { type: 'object', required: ['kind', 'ids', 'detail', 'proposed'], properties: {
      kind: { type: 'string', enum: ['contradicts-vision', 'contradicts-decision', 'contradicts-layer-8-contract', 'contradicts-locked-spec', 'pi-floor', 'unruled-fork', 'missed-merge', 'wrong-merge', 'default-unsafe', 'unclear-for-an-author'] },
      ids: { type: 'string' }, detail: { type: 'string' }, proposed: { type: 'string' } } } },
    sound: { type: 'array', items: { type: 'string' } },
    not_covered: { type: 'array', items: { type: 'string' } },
  },
}

const SOURCES = `
Repo root ${ROOT}. Layer 9 (Self-Healing & Evolution) of the 111network mesh OS. Read, in this order:
- ${ROOT}/README.md: the owner's vision. It is the source of truth and is never reworded or contradicted (plan/DECISIONS.md D36); where v1
  delivers less than it says, the spec states a v1 limit as a limit.
- ${ROOT}/plan/DECISIONS.md: D35 (the Layer 9 decision) and D34 (Layer 8), D36, D37.
- ${ROOT}/plan/layers/9-evolution/options.md: the brief the owner decided on (the decided option and its questions).
- ${ROOT}/9-evolution/README.md: the spec draft v0.1, whole.
- The three panel reports on it: ${R}/safety.md (S ids), ${R}/observe.md (O ids), ${R}/integration.md (I ids). Each says amend-then-lock.
- ${ROOT}/plan/layers/8-storage/review/triage.md: the lead's triage of the Layer 8 panel. It is the SHAPE to copy and it is BINDING for
  everything Layer 9 touches in Layer 8 (the store, the proposal spool and its hand-off rule, budgets, the housekeeping owner, the role
  bits, the kiwix component, the renamed tools): Layer 9 conforms to it. ${ROOT}/8-storage/README.md is Layer 8's spec, being amended to
  that triage right now by another agent: read it for context, trust the triage where the two differ, and never edit it.
- The locked specs Layer 9 leans on, only where a finding cites them: ${ROOT}/{1-bootstrap,3-intelligence,4-interface,5-messaging,7-relay}/README.md.
- ${ROOT}/AGENTS.md: the Raspberry Pi floor (no resident process, periodic poll, timer, listener, runtime dependency or build-time weight
  is added to the required configuration without the owner's say).`

const RULES = `
THE OWNER'S STANDING INSTRUCTION: go with the lead's recommended defaults and build A to Z; the owner fine-tunes on real tests. So every fork
is RULED in the triage (none is left "for the owner"); a ruling that departs from the brief or narrows what the root README promises is
marked "listed for the owner in the spec's deviations section".

HOW THE LEAD RULES (apply these; they are how the Layer 8 triage was ruled):
1. A finding not named in the triage is accepted with the fix its report gives. Name a finding only when: two or more findings are the same
   defect (merge them: one fix), two reports' fixes cannot both be applied, a fix offers alternatives, a fix would contradict a source above,
   a fix adds weight to the Pi floor, a fix needs a number or a name that the report leaves open, or a builder must not miss it (blockers).
2. Safety before reach. Between "cut it from v1" and "add a mechanism that makes it safe": cut when the mechanism adds a resident process,
   a timer, a poll, a listener or a dependency, or widens what an agent may change on a device without a human; keep and add the mechanism
   when the cut would remove something the root README names as the point of the layer (then narrow it and state the v1 limit as a limit).
3. Anything that changes a device is approved by a human on that device unless the root README and D35 say otherwise; a fix that travels
   between nodes is data until a human or a rule the spec already has makes it an action.
4. Where two reports disagree, the stricter rule wins unless it breaks a locked layer's contract; a needed change to a LOCKED spec
   (Layers 0-7) is never made silently: it is listed as a lock-amendment candidate with the layer and the section.
5. Numbers: when a report gives one, use it; when it leaves one open, choose a default, say what it bounds, and mark it "default, to be
   measured" for the build.
6. No ruling invents a feature no report asked for.

RULINGS THE LEAD HAS ALREADY MADE (write them as given; do not reopen them):
- S12: \`code\` and \`kb\` fixes are cut from the v1 wire: they travel as drafts and reach other nodes through the human's pull request.
- S18: \`paired_hubs\` means the home hub plus the Layer 7 paired hubs.
- S19: the three bridge actions and the two interface actions are "human or \`device.propose\` only" in v1.
- S27: \`update.rollback\` leaves the v1 action set (its usable window is about a minute); the \`human_step\` stays.
- Every other finding of safety.md: accepted with its fix, unless it collides with observe.md or integration.md (then rule the collision).`

phase('Draft')
const draft = await agent(`You draft the LEAD'S TRIAGE of the Layer 9 spec panel. The lead will read your draft, correct it and sign it; a fix pass of three
authors then amends the spec to v0.2 with your file as the law, and a judge re-checks v0.2 against it. Write for those readers: an author
who has only the triage and one report must know exactly what to write.
${SOURCES}
${RULES}

WRITE ${R}/triage.md (create it; edit no other file; run no build, no cargo, no pnpm, start no process). Copy the shape of the Layer 8 triage:
- Title "# Layer 9 spec v0.1 — the lead's triage of the panel (2026-11-13)" and an opening paragraph: the three reports with their id ranges
  and verdicts, the owner's instruction, the default ruling in bold, what the fix pass produces.
- "## Merged findings (one fix each)": a table | ids | ruling |. Every pair or group of findings across the three reports that is one defect.
- "## Forks the reports leave open, or where two reports disagree": a table | ids | ruling |, each ruling one decision with its reason in a
  clause, its number where it needs one, and "listed for the owner" where rule 4 or the departure rule applies. Include the lead's four
  rulings above, worded for the authors.
- "## Accepted with the report's fix (named here because a builder must not miss them)": every blocker not already above, and any major whose
  fix touches a seam with another layer; one line each, saying which section of the spec carries the fix.
- "## What Layer 9 takes from Layer 8's triage": the Layer 8 rulings Layer 9's text must conform to (name the Layer 8 ids), one line each.
- "## Lock-amendment candidates": changes a finding needs in a locked spec (layer, section, the change), or "none".
- "## Order for the build (from the reports' watch notes)": what to build and measure first.
Plain, exact prose; ids everywhere; no hedging; no model names; at most 120 characters is NOT required (long table cells are fine).
Count before you finish: every blocker id of the three reports appears in the file; every finding id you merged appears once in the merged
table. RETURN: the file path, the merged groups, every fork with the question, the options the reports give, your ruling, why, your
confidence, and whether the owner should see it; the lock-amendment candidates; notes (anything you could not rule and why).`,
  { label: 'L9:triage-draft', phase: 'Draft', schema: DRAFT, model: 'opus', effort: 'xhigh' })

if (!draft) return { error: 'the draft agent returned nothing' }
log(`draft: ${draft.merged.length} merged groups, ${draft.forks.length} forks, ${draft.lock_amendments.length} lock-amendment candidates`)

phase('Check')
const check = await agent(`You are a read-only judge. Do NOT edit any file, run any build or start any process. A drafter wrote the lead's triage of the Layer 9
spec panel: ${R}/triage.md. The lead signs it next, and three authors then amend the spec with it as the law. Find what would make that
go wrong.
${SOURCES}
${RULES}

The drafter's own summary of its forks (its reasons are claims, not facts):
${JSON.stringify({ forks: draft.forks, lock_amendments: draft.lock_amendments, notes: draft.notes }).slice(0, 14000)}

HUNT, reading the three reports yourself and the triage line by line:
1. A ruling that contradicts the root README, D35, D36, the brief's decided option, the Layer 8 triage (the storage contract), a locked
   spec, or the Pi floor. Quote both sides.
2. A fork left unruled: two findings (in one report or across reports) whose fixes cannot both be written into one spec, a fix that offers
   alternatives, or a fix that needs a number or a name, where the triage is silent and the default rule would therefore leave an author to
   guess. List each with the ids and the choice to make.
3. A merge that is wrong (the findings are different defects and one fix does not cover both) or missed (the same defect ruled twice or
   left to two authors to fix differently).
4. A finding where the default "accepted with the report's fix" is unsafe or adds weight (a resident task, a timer, a poll, a listener, a
   dependency, an agent action without a human) and the triage does not name it.
5. A ruling an author could read two ways.
Do not re-argue the four rulings the lead already made. Report only what you can show with ids and quotes; say what you checked and found
sound, and what you did not cover.`,
  { label: 'L9:triage-check', phase: 'Check', schema: CHECK, effort: 'high' })

return { draft, check }
