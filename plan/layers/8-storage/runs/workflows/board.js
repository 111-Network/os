export const meta = {
  name: 'layer-8-board',
  description: 'Layer 8 build board: one author cuts the locked spec into waves and rows (8-storage/TASKS.md, in the shape of the Layer 7 board), one read-only critic hunts spec rules and gate items with no row and rows that share a file in one wave, the author revises',
  phases: [
    { title: 'Draft', detail: 'one author writes 8-storage/TASKS.md from the locked spec', model: 'opus' },
    { title: 'Critic', detail: 'one read-only completeness and ownership check', model: 'opus' },
    { title: 'Revise', detail: 'the author applies the critic', model: 'opus' },
  ],
}
const ROOT = '/home/user/os'
const BOARD = `${ROOT}/8-storage/TASKS.md`
const DRAFT = {
  type: 'object', required: ['file', 'waves', 'rows', 'notes'],
  properties: {
    file: { type: 'string' },
    waves: { type: 'array', items: { type: 'object', required: ['name', 'rows', 'order'], properties: { name: { type: 'string' }, rows: { type: 'array', items: { type: 'string' } }, order: { type: 'string' } } } },
    rows: { type: 'number' },
    notes: { type: 'string' },
  },
}
const CRIT = {
  type: 'object', required: ['gaps', 'collisions', 'other', 'sound'],
  properties: {
    gaps: { type: 'array', items: { type: 'object', required: ['spec', 'what', 'row'], properties: { spec: { type: 'string' }, what: { type: 'string' }, row: { type: 'string' } } } },
    collisions: { type: 'array', items: { type: 'object', required: ['file', 'rows', 'fix'], properties: { file: { type: 'string' }, rows: { type: 'string' }, fix: { type: 'string' } } } },
    other: { type: 'array', items: { type: 'object', required: ['row', 'defect', 'fix'], properties: { row: { type: 'string' }, defect: { type: 'string' }, fix: { type: 'string' } } } },
    sound: { type: 'array', items: { type: 'string' } },
  },
}
const SRC = `
Repo root ${ROOT}, branch claude/confident-ramanujan-y5ob49. Layer 8 (Storage & Shared State) of the 111network mesh OS: its spec
${ROOT}/8-storage/README.md is LOCKED at v1.0 (plan/DECISIONS.md D34 and D38). Nothing of it is built. Read:
- the spec WHOLE (every rule J1-J9, every section, §16 routes and shapes, §18 the seam table and "Gate homes", §19 the Pi-floor figures, §21 what
  each other layer owes, §22 deviations and numbers, the gates G13-G18 and the extended G7 item by item);
- ${ROOT}/plan/layers/8-storage/review/triage.md (its last section gives the build order) and recheck.md / recheck-2.md (their notes for the build);
- ${ROOT}/plan/EXECUTION.md (the build protocol) and ${ROOT}/AGENTS.md (the Raspberry Pi floor; Linux hygiene; the box rules);
- THE SHAPE TO COPY: ${ROOT}/7-relay/TASKS.md: its header paragraph, box rules, legend, "T0 lead scaffold", the wave sections with their ordering
  sentence, the row table (columns: id | Task with spec sections and finding ids | Criteria | Check | Files | Model | state), the "never adds"
  clause in every daemon row, the Pi-floor record table, External gates, Summary. Read its header and three or four rows of each kind closely,
  not all 156 KB.
- the as-built tree the rows will touch, enough to name real files: ${ROOT}/1-bootstrap/crates/node111/src/daemon/ (fabric/, origin/, panels/,
  components/, dashboard/api_v1/, services/, updater/), ${ROOT}/5-messaging/crates/mesh111/src/ (store/, blobs.rs), ${ROOT}/3-intelligence/crates/agent111/src/,
  ${ROOT}/4-interface/api/openapi.yaml and ${ROOT}/4-interface/ui/src/, ${ROOT}/1-bootstrap/scripts/, ${ROOT}/verify/{seams,checks}.toml (the
  cross-layer test system: plan/cross-layer/test-system/TASKS.md rows N* describe how a new layer registers its gates and seams).`

const RULES = `
BOARD RULES:
1. A row is one task one agent can finish and one reviewer can check in a sitting: one concern, a closed file list, a check command that
   proves its criteria. Split anything larger. Rows that share a file never sit in the same parallel group: the Files column is the
   ownership record, and a wave's ordering sentence states every "X before Y".
2. Every rule of the spec and EVERY numbered item of gates G13-G18 and the extended G7 is carried by a named row (the gate rows cite the
   item numbers they implement); every route and shape of §16 by a contract row and a route row; every item other layers owe (§21) by a row
   in the wave that needs it, marked as an additive amendment of that locked layer with the lock-amendment candidates listed for the lead.
3. Wave order: the triage's last section, then dependencies. A contract row (openapi.yaml, generated types, the pin lists) comes before the
   rows that implement its routes; the store's schema and migration before its consumers; the housekeeping owner moves first; gates last
   (authored in parallel, run one harness at a time); a Pi-floor record table with one line per figure of §19, each owned by a row.
4. Every daemon row states what it NEVER ADDS (a resident task, timer, poll, listener, dependency, file write on an idle node) and, where it
   could add weight, the §19 figure it records.
5. Model column: Rust rows "opus"; UI, docs, KB, Python and shell rows "sonnet"; tiny transforms "haiku"; the integrated review "fable"
   (these are the board's own column values, as on the Layer 7 board).
6. Measurements the triage orders (the export, the migration copy, the integrity checks, the sha256 pass on the largest store the defaults
   allow; every number marked "default, measured at build") each belong to a named row.
7. Row ids follow the Layer 7 board's letters where the kind matches (M mesh111, D daemon, L Layer 1 trust and installer, A agent, U UI,
   C contract, K docs and KB, V gates and verification, P prototypes) and never reuse a Layer 7 id inside this board's own text ambiguously:
   this board's ids are its own namespace (cite another board as "Layer 7 row D6").
8. Paths use the spec's §0 abbreviations. No model names beyond the column values. Plain, exact prose; no hedging.`

phase('Draft')
const draft = await agent(`You write the Layer 8 build board: ${BOARD} (create it; edit no other file; run no build, no cargo, no pnpm, start no process; no git write
command). Other agents are editing other files of this repo right now: never touch them.
${SRC}
${RULES}
WRITE the whole board: header paragraph (what the spec is, where the reviews are, the contract, the ownership rule, the wave order in one
sentence, the forks applied on the lead's default with a pointer to §22), box rules, legend, T0 lead scaffold, the waves with their rows
(every row state ⬜), the Pi-floor record, External gates (owner, network, hardware), an empty Summary heading.
RETURN: the file path; per wave its name, row ids and ordering sentence; the row count; notes (every place the spec left you a choice about
ownership or order, and any spec rule you could not place).`, { label: 'L8:board-draft', phase: 'Draft', schema: DRAFT, model: 'opus', effort: 'xhigh' })
if (!draft) return { error: 'no draft' }
log(`draft: ${draft.rows} rows in ${draft.waves.length} waves`)

phase('Critic')
const crit = await agent(`You are a read-only critic: edit no file, run no build, start no process. A board author has just cut the locked Layer 8 spec into build rows:
${BOARD}. Agents will build from it row by row, so a rule with no row is a rule that never gets built, and two rows of one parallel group
that share a file will overwrite each other.
${SRC}
${RULES}
The author's own summary (claims): ${JSON.stringify(draft).slice(0, 9000)}

HUNT, walking the SPEC (not the board) section by section and gate item by gate item:
1. GAPS: a rule (J1-J9 and each numbered rule of each section), a route or shape of §16, a seam of §18, a figure of §19, an item another
   layer owes (§21), a deviation or number of §22 that needs a measurement, or a numbered item of G13-G18 and G7 that no row carries (or a
   gate row that cites an item number that does not exist). For each: the spec place, what is missing, and the row that should carry it
   (an existing id, or "new row in wave N").
2. COLLISIONS: two rows that can run in parallel by the wave's ordering sentence and name the same file (or one names a directory that holds
   the other's file). For each: the file, the rows, and the ordering fix.
3. OTHER: a row too large for one sitting, a row with no check that proves its criteria, a daemon row with no "never adds" clause, a row whose
   dependency lands in a later wave, a file path that does not exist where the row says "edit" (new files are fine when marked new), a
   contract change with no pin-list update, a locked-layer amendment not marked.
Report only what you can show with a spec section and a row id; list what you walked and found sound.`, { label: 'L8:board-critic', phase: 'Critic', schema: CRIT, model: 'opus', effort: 'high' })
if (!crit) return { draft, error: 'no critic result' }
log(`critic: ${crit.gaps.length} gaps, ${crit.collisions.length} collisions, ${crit.other.length} other`)

phase('Revise')
const rev = await agent(`You wrote the Layer 8 build board ${BOARD} (edit that file only; no build, no process, no git write command). A critic walked the locked spec
against it and returned the items below. Apply EVERY item: add the missing rows or criteria, fix each collision with the ordering fix or by
moving a file to one owner, split or repair the rows named under "other". Where you judge an item wrong, do not apply it and say why. Keep
every row id stable (add new ids; never renumber). Then re-read the board once for the rules below.
${SRC}
${RULES}
THE CRITIC'S ITEMS:
${JSON.stringify({ gaps: crit.gaps, collisions: crit.collisions, other: crit.other }).slice(0, 30000)}
RETURN: the file path; per wave its name, row ids and ordering sentence (as they now stand); the row count; notes: each critic item you did
NOT apply and why, and every open choice left for the lead.`, { label: 'L8:board-revise', phase: 'Revise', schema: DRAFT, model: 'opus', effort: 'high' })
return { draft, crit, rev }
