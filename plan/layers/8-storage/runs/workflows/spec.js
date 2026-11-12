export const meta = {
  name: 'layers-8-9-spec',
  description: 'Layers 8 (Storage & Shared State) and 9 (Self-Healing & Evolution): one opus drafter per layer writes <layer>/README.md v0.1 from the decision (D34 / D35), the brief, the research and the as-built code, Layer 9 after Layer 8 (one joint storage contract); three fable judges per layer attack each draft and write plan/layers/<layer>/review/<lens>.md',
  phases: [
    { title: 'Draft 8', detail: 'one opus spec author (xhigh): 8-storage/README.md v0.1', model: 'opus' },
    { title: 'Draft 9', detail: 'one opus spec author (xhigh): 9-evolution/README.md v0.1, reading the Layer 8 draft', model: 'opus' },
    { title: 'Panel 8', detail: 'three fable judges, read-only, one report file each', model: 'fable' },
    { title: 'Panel 9', detail: 'three fable judges, read-only, one report file each', model: 'fable' },
  ],
}

// args: { only: ['8'] | ['9'], panelOnly: true, lenses: { '8': [...] } } — the same script serves both layers (they share one storage contract).
const ROOT = '/home/user/os'
const a = args || {}
const ONLY = Array.isArray(a.only) ? a.only : ['8', '9']

const L = {
  8: {
    n: 8, slug: '8-storage', name: 'Storage & Shared State', dec: 'D34', forks: 'F1-F13',
    readme: 'section 8 ("Each node keeps an encrypted local store, pruned by capacity rather than by imposed rules. Knowledge hubs host the large shared datasets ... A lightweight append-only shared log may be used ... adopted from a proven project, never a custom consensus implementation.") and the device categories 4A (Knowledge Hubs) and 2A/2B/3',
    areas: `one numbered section per decided area, in the brief's order, each carrying every pick, detail pick and default answer that
  belongs to it: the encrypted local store and who writes the agent's state (F1) · THE JOINT STORAGE CONTRACT with Layer 9 (F2: state it
  once, completely, as a numbered contract Layer 9's spec cites: the items, the owner of each path, the single writer, the budget class of
  each kind of state, what is exported before a prune, what survives a reinstall) · one node budget and capacity pruning, datasets never
  pruned automatically (F3) · the object tree for datasets, verified block by block (F4) · who signs a dataset and how a node trusts a
  publisher (F5) · moving datasets in v1: the first copy, HTTP Range pulls, sneakernet, the metered cap, consent (F6) · Tier 2 object
  transfer and who owns it (F7; what is v1.1) · encyclopedias as an on-demand hub component and what a hub shows the LAN (F8) · maps and
  the exempt bucket (F9; closes Layer 7's recorded D21 amendment) · software updates beyond the 111 release: component pins on the hub
  origin (F10) · the shared log (F11: what v1 does and does not build, the seam kept) · backup, export, SD wear and migrations (F12) ·
  sequencing and what the first Layer 8 release holds (F13)`,
    code: `mesh111's store (${ROOT}/5-messaging/crates/mesh111/src/store/*: schema, prune classes and quotas, blobs; NOTE: a fix in
    flight adds schema v5, a custody_totals table kept by triggers, see row M1b of ${ROOT}/7-relay/TASKS.md: cite schema versions as "v4, v5
    in flight" where it matters), mesh111's fabric and Tier 2 reservations (src/fabric.rs, src/frag.rs, the OBJ_* kinds), node111
    (${ROOT}/1-bootstrap/crates/node111/src/daemon/{fabric,dashboard,components,origin,relay,panels,intelligence}/, the installer's on-disk
    layout in src/install/, the updater in src/{update_guard.rs,update_helper.rs,manifest.rs}), the agent crate's state under
    ${ROOT}/3-intelligence/crates/agent111/src (conversations, journal, proposals: where each file is written today), the UI
    ${ROOT}/4-interface/ui/src (the Storage settings stub, check-budget.mjs), the contract ${ROOT}/4-interface/api/openapi.yaml`,
    lenses: [
      { key: 'store', letter: 'S', prompt: 'LENS = the store (correctness and durability). Your sections: the local store and the agent state writer, the joint storage contract, the budget and pruning rules, the object tree and block verification, backup / export / migrations / SD wear. Verify every mesh111 store and agent111 file:line the draft cites. Hunt for: a second writer to an encrypted store, a path two layers both think they own, state that can be pruned before it was exported or that a reinstall loses against the contract, a prune that is policy and not capacity (the README forbids imposed rules), a pruning order that can evict what the node cannot get back, a key or secret that reaches disk or an API, a migration that is not append-only or cannot roll back, write amplification on an SD card, a byte count or schema fact that is wrong against the code, work per item that grows with the rows held.' },
      { key: 'datasets', letter: 'D', prompt: 'LENS = datasets and the floor (systems). Your sections: who signs a dataset and publisher trust, moving datasets (the first copy, Range pulls, sneakernet, the metered cap, consent copy), Tier 2 ownership and its v1.1 line, encyclopedias and what a hub shows the LAN, maps and the exempt bucket, software updates beyond the release, the shared log, settings and consent, the Pi-floor accounting. Verify every node111, updater and UI file:line the draft cites and every size or licence fact against the research reports (mark what no report supports). Hunt for: any resident weight on a non-hub node or on a hub with the feature off (a process, listener, timer, poll, loaded code, open file), a download without consent or on a metered link, a dataset served or trusted without its signature and block check, a custom consensus or a custom transfer protocol where an adopted project exists (the README: adopt, never a custom consensus), a component-seam misuse (Layer 6 section 2.1), a licensing row missing (dataset licences, the server program), an API or size claim the sources do not support, a setting without a default.' },
      { key: 'integration', letter: 'I', prompt: 'LENS = integration (seams, buildability, verification). Your sections: the reading guide, scope, the API and contract additions (against the conventions of 4-interface/api/README.md and the existing openapi.yaml: additive, pinned, problem+json; the agent tools against 3-intelligence conventions), the security and threat model, every verification gate (namespace-provable without hardware? concrete assertions? a pass line? the CI mapping real? does each gate name the layer seam it proves?), deferred / v1.1, the cross-layer seams and amendments (every amendment to a locked layer additive and complete; Layer 7 hand-offs closed: the hub origin, the map bucket, the Tier 2 reservations), plan corrections, and completeness against the decision: every fork F1-F13, every detail pick and every answered question of the brief carried somewhere. Hunt for: an item a builder could not start from (no file, no criterion, no check), a gate that cannot run in a namespace, a contract addition that breaks a pinned shape, a decision the draft dropped or widened, a v1.1 item that leaked into v1, a sentence that rewords or contradicts the root README (the owner\'s vision: never reworded, D36), a mismatch with what the Layer 9 brief expects of the joint storage contract (read plan/layers/9-evolution/options.md section 2 F3).' },
    ],
  },
  9: {
    n: 9, slug: '9-evolution', name: 'Self-Healing & Evolution', dec: 'D35', forks: 'F1-F14',
    readme: 'section 9 ("The local agent continuously observes the health of every layer the device runs. On detecting a failure or degradation it formulates a diagnosis, generates and internally validates a corrective change, and - if the change succeeds - packages it as a pull request. Humans remain the final gate for merging. Nodes share proven fixes with each other through the gateways ...") and the principles "Autonomy is progressive and always human-supervised at the final step" and "The agent is both servant and guardian of the node"',
    areas: `one numbered section per decided area, in the brief's order, each carrying every pick, detail pick and default answer that
  belongs to it (read the brief's section 2 headings F1-F14 and keep their order): what v1 does (propose plus automatic undo) and the
  autonomy ladder · the observer inside 111d (event-driven: what it watches per layer, with no timer or poll added) · THE JOINT STORAGE
  CONTRACT as Layer 9 consumes it (cite the Layer 8 draft's contract by its numbers; restate nothing that could drift; say exactly which
  Layer 9 state lives under which item) · diagnosis: deterministic playbooks as a Layer 0 category with a closed predicate enum, and where
  the model is allowed to help · the typed change engine behind an Action trait: the closed set of actions, validation in a sandbox, the
  undo record, what can never be changed by a fix · code fixes only as hub drafts, and the human sends the pull request · signing: the
  root key signs fixes under the new domain tags; how a node trusts a fix it receives · sharing proven fixes between nodes (what rides the
  fabric, the narrow home-hub-only AGENT_OP diagnose pair, what a gateway carries) · improving the agent itself, the Knowledge Layer and
  the bridging logic (what v1 allows) · settings, consent and the UI · sequencing and the first release · the additive amendments to
  locked layers (the brief counts sixteen: list each)`,
    code: `node111 (${ROOT}/1-bootstrap/crates/node111/src/daemon/: services and their state machine, the supervisor, components, the
    updater src/{update_guard.rs,update_helper.rs,manifest.rs}, dashboard/api_v1/proposals.rs and the proposal round trip, the intelligence
    module), the agent crate ${ROOT}/3-intelligence/crates/agent111/src (tools.rs, the journal, the proposals, the sandbox it has today, the
    loopback bearer), mesh111 (${ROOT}/5-messaging/crates/mesh111/src: the AGENT_OP class and kinds, envelope.rs, fabric.rs), id111
    (${ROOT}/2-identity/crates/id111/src: the domain tags, the root and agent keys, delegation), the Knowledge Layer
    (${ROOT}/0-knowledge/: categories, the generator _meta/build.py and its checks), the UI ${ROOT}/4-interface/ui/src (the agent view, the
    proposal cards), the contract ${ROOT}/4-interface/api/openapi.yaml`,
    lenses: [
      { key: 'safety', letter: 'S', prompt: 'LENS = safety (authority, validation, undo). Your sections: what v1 does and the autonomy ladder, the change engine and its closed action set, validation and the sandbox, the undo record and automatic undo, signing and trust in a received fix, code fixes as hub drafts and the human pull request, the security and threat model. Verify every node111, agent111 and id111 file:line the draft cites. Hunt for: any path where a change is applied without the human gate the README requires or outside the closed action set, an action whose undo is not total or not tested before it is applied, a fix that can touch identity keys, the updater trust chain, consent records or its own gate, the agent holding or requesting key material beyond Layer 2 and 3 rules, a prompt-injection path (untrusted text from a peer, a log or a KB entry reaching the model as instructions or reaching an action as parameters), a signed artefact without its own domain tag or with a replay, a rollback, a downgrade, a wall clock in an ordering or cryptographic decision, a sandbox that shares state with the live node.' },
      { key: 'observe', letter: 'O', prompt: 'LENS = observation, diagnosis and the floor. Your sections: the observer inside 111d, diagnosis and the playbooks (the Layer 0 category, the closed predicate enum, the generator checks), sharing fixes between nodes (the fabric class, the AGENT_OP diagnose pair, gateways), improving the agent, the KB and the bridges, settings and consent, the Pi-floor accounting. Verify every service-state, supervisor, mesh111 and 0-knowledge file:line the draft cites. Hunt for: any resident process, timer, poll, listener or loaded code added to the required configuration (the observer must be event-driven on signals the daemon already has: prove each one exists in the code), a health signal that does not exist or cannot distinguish the failure it claims to, a playbook predicate that needs a model or the network to evaluate, a playbook or fix format that is not Markdown-with-frontmatter under the Layer 0 rules, a diagnose request a non-home hub or a stranger could send, load a hub cannot bound, a phone-home, telemetry of any kind (the README: no telemetry), a claim about model capability the Layer 3 spec does not support, a device category handled wrongly (2A and 3 consume this layer through a hub; Cat 1 runs none of it).' },
      { key: 'integration', letter: 'I', prompt: 'LENS = integration (seams, buildability, verification). Your sections: the reading guide, scope, the joint storage contract as consumed (compare it line by line with the Layer 8 draft ${ROOT}/8-storage/README.md and with both briefs: one contract, no drift, every Layer 9 kind of state placed), the API and contract additions (against 4-interface/api/README.md conventions and the existing openapi.yaml; the agent tools against 3-intelligence conventions), every verification gate (namespace-provable without hardware? concrete assertions? a pass line? the CI mapping real? does each gate name the layer seam it proves?), deferred / v1.1, the cross-layer amendments (the brief counts sixteen additive amendments to locked layers: each present, additive, complete), plan corrections, and completeness against the decision: every fork F1-F14, every detail pick and every answered question of the brief carried somewhere. Hunt for: an item a builder could not start from, a gate that cannot run in a namespace, a contract addition that breaks a pinned shape, a decision the draft dropped or widened, a later-version item that leaked into v1, a sentence that rewords or contradicts the root README (the owner\'s vision: never reworded, D36).' },
    ],
  },
}

const draftPrompt = (c) => `
You are the spec author for the 111network OS, Layer ${c.n} (${c.name}). Today is 2026-11-11. Repo root: ${ROOT}, branch
claude/confident-ramanujan-y5ob49. You WRITE exactly one file: ${ROOT}/${c.slug}/README.md (create the folder). You edit nothing else, run
no git write commands, start no process, run no cargo build or test (other agents are building Layer 7 in this tree right now: their
uncommitted edits under 5-messaging/ and 1-bootstrap/ are not yours to judge or touch); read-only greps, 'git log' and 'cargo metadata'
reads are fine.

THE LAW, in order:
(1) ${ROOT}/README.md, the owner's vision and the source of truth for what the project is: ${c.readme}. The spec realises it; it never
    rewords it. Where v1 builds less than a README sentence promises, say so in the reading guide as a v1 scope limit with the later
    version that closes it (owner rule, DECISIONS D36).
(2) ${ROOT}/plan/DECISIONS.md ${c.dec} (the owner's decision: every pick of the brief's section 2 (${c.forks}), every detail pick of its
    section 3, every plan correction of its section 4 and every default answer the lead gave in its section 5; never quote the owner's
    words as yours) and D36; D30-D32 and D29 as the lock precedents and their amendment style.
(3) ${ROOT}/plan/layers/${c.slug}/options.md, the brief: section 0 facts, section 2 forks with picks, section 3 detail picks, section 4
    plan corrections, section 5 answered questions, section 6; and ${ROOT}/plan/layers/${c.slug}/review/options-check.md (what its judge
    corrected).
(4) The research reports ${ROOT}/plan/layers/${c.slug}/research/00-code-seams.md ... 07-gap-3.md (evidence with file:line; 00 is the
    line-verified seam map; 06-critic lists the claims nobody could verify from this box: mark those "(unverified from this box)").
(5) The as-built code. Every claim about built code must be verified by you reading the file and cited file:line (the reports' lines are
    from 2026-11-02: re-check each): ${c.code}.
(6) The locked specs you amend or depend on: ${ROOT}/0-knowledge/README.md, 1-bootstrap/README.md, 2-identity/README.md,
    3-intelligence/README.md, 4-interface/README.md with 4-interface/api/README.md, 5-messaging/README.md, 6-transport/README.md and
    ${ROOT}/7-relay/README.md, which is THE STRUCTURE AND VOICE MODEL: read its sections 0, 1, 12-19 fully first (the reading guide, scope
    and the Pi floor, settings and consent, the contract, security, verification, the Pi-floor accounting, deferred, cross-layer seams and
    amendments, plan corrections) and write in that form.${c.n === 9 ? `
(7) The Layer 8 spec draft ${ROOT}/8-storage/README.md, written just before yours: its joint storage contract is the one contract. Cite it
    by its section and item numbers; where the Layer 9 brief's F3 words differ from it, follow the Layer 8 draft and record the
    difference under deviations.` : `
(7) The Layer 9 brief ${ROOT}/plan/layers/9-evolution/options.md section 2 F3: the joint storage contract is decided there in the same
    words as your F2; your spec is where the contract is stated, and the Layer 9 spec, written right after yours, will cite it.`}
(8) ${ROOT}/AGENTS.md rules: the Raspberry Pi floor, adopt before you build, Linux hygiene, no model names in files.

WHAT TO WRITE: ${ROOT}/${c.slug}/README.md, titled "Layer ${c.n} - ${c.name}: spec v0.1 (draft for the adversarial panel)", 800-1200
lines, normative where it says so, in the voice of 7-relay/README.md: short declarative sentences, numbered rules, byte and path tables,
"MUST" and "never" only where a rule is hard, every built-code fact cited file:line, every number sourced to a report section or a file.
Sections, in this order (number them from 0; you choose the exact numbers and say them in the reading guide):
- Reading guide: what is normative and what is informative; the v1 / later split in one table; how this spec relates to ${c.dec}, the brief
  and the README (the v1 scope limits against the README's wording, each with the version that closes it).
- Scope and non-goals: what Layer ${c.n} is on each device category (the README's layer-to-category table row for layer ${c.n}); the Pi
  floor as a hard rule (nothing resident, no timer, no poll, no listener, no dependency added to the required configuration; every
  hub-only or opt-in cost enumerated); non-goals.
- Then ${c.areas}.
- Settings and consent: every setting with its name, default (every default off or inert, so a node with nothing configured behaves as
  today), where it is stored, the consent copy verbatim, and what the agent may propose versus what only a human may confirm.
- The contract: every HTTP route, shape, event and problem code as additive rows for ${ROOT}/4-interface/api/openapi.yaml (conventions:
  4-interface/api/README.md), which routes the agent's bearer may call, the agent tools (name, arguments, what each reads or proposes).
- Security and threat model: the hard rules as a numbered list, an exposure table (who learns what), the attack cases with the rule that
  stops each.
- Verification (lock gates; network namespaces, no hardware): the gates under ${ROOT}/${c.slug}/verify/, each with its assertion list and
  its pass line, in the repo's harness conventions (read ${ROOT}/6-transport/verify/lib.sh and ${ROOT}/7-relay/verify/lib.sh: PASS / FAIL /
  SKIPPED lines, the REQUIRE_* switches); for EACH gate name the layer seam it proves (which layers it joins and through which interface),
  because a cross-layer seam-test runner (DECISIONS D36) aggregates every layer's gates; the CI mapping (which job in
  .github/workflows/ci.yml); the unit and integration test homes per crate.
- Pi-floor accounting: a table of every resident cost by role (a node with the feature off: 0 for every item), measured versus estimated,
  and the rule that the build records measurements in ${c.slug}/TASKS.md.
- Deferred / later versions: each item with what it needs.
- Cross-layer seams and amendments: per layer (L0 ... L${c.n - 1}${c.n === 8 ? ', and the hand-off to L9' : ''}) the exact additive
  amendments this spec requires, in the style of 7-relay/README.md section 18; the hand-offs of earlier layers this layer closes.
- Plan corrections recorded: the brief's section 4, verbatim where they touch locked text (never a rewording of the root README: record
  those as v1 scope limits in the reading guide instead).
- Changelog v0.1: one line.

HARD RULES FOR THE TEXT: no model names or identifiers anywhere in the file (write "the lead", "the panel", "a scout"); no attribution
lines; a file:line citation for every claim about built code, each verified by reading; where the brief or a report is wrong against the
code, follow the code and record the correction; never invent a number: mark an unknown "(measured at build)"; every default off or
inert; every new setting named with its default; every new wire byte and every new on-disk path accounted in a table; every amendment to
a locked layer additive; v1 adds no process, listener, timer or poll to the required configuration, and every opt-in cost is enumerated;
adopt a proven project wherever one exists and say which, with its licence, and never specify a custom consensus or a custom transfer
protocol where the brief adopted one.

RETURN (structured): status (written | partial | blocked), file, lines, sections (the list you wrote), verified_claims (the count of
file:line citations you verified by reading), deviations (where you departed from ${c.dec} or the brief and why, one line each),
open_questions (what only the owner can answer, one line each; expect few: the owner asked for the build to proceed on the lead's
defaults), notes (at most 12 lines for the lead).
`

const DRAFT_SCHEMA = {
  type: 'object',
  required: ['status', 'file', 'lines', 'sections', 'verified_claims', 'deviations', 'open_questions', 'notes'],
  properties: {
    status: { type: 'string', enum: ['written', 'partial', 'blocked'] },
    file: { type: 'string' },
    lines: { type: 'number' },
    sections: { type: 'array', items: { type: 'string' } },
    verified_claims: { type: 'number' },
    deviations: { type: 'array', items: { type: 'string' } },
    open_questions: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

const judgeCtx = (c, key) => `
You are a READ-ONLY review judge for the 111network OS, Layer ${c.n} (${c.name}) spec draft v0.1 at ${ROOT}/${c.slug}/README.md. Today is
2026-11-11. Repo root ${ROOT}. You edit nothing except the one report file named below; you run no git write commands, no cargo build or
test, and start no process (other agents are building Layer 7 in this tree: ignore their uncommitted edits). The law you judge against:
${ROOT}/README.md (the owner's vision, the source of truth: ${c.readme}; a spec that contradicts or rewords it is wrong; a v1 scope limit
must be stated as one), ${ROOT}/plan/DECISIONS.md ${c.dec} and D36 (the owner took every pick and default of the brief: a spec that
contradicts the brief is wrong unless the code makes the pick impossible, which you then say), the brief
${ROOT}/plan/layers/${c.slug}/options.md, the research reports ${ROOT}/plan/layers/${c.slug}/research/*.md (00 is the line-verified seam
map), the locked specs the draft amends (Layers 0-7 READMEs, ${ROOT}/4-interface/api/openapi.yaml) and the as-built code: verify the
draft's file:line claims by reading; a wrong citation is a finding. ${ROOT}/AGENTS.md rules apply (the Raspberry Pi floor; adopt before you
build; no model names in files).

METHOD: read the whole draft once, then your lens's sections closely, then verify. Attack it: what is unbuildable, contradictory,
underspecified (a builder would have to guess), unsafe, heavier than the Pi floor allows (any new resident process, timer, poll, listener
or dependency in the required configuration; an unmeasured cost stated as a fact), wider than the decision (anything the brief deferred
that the draft puts in v1), or missing (a decided pick the draft does not carry; a seam the code has that the draft contradicts).
Severity: blocker = the spec cannot lock without it (wrong against the code, the decision or the README; unsafe; unbuildable); major = a
builder would build the wrong thing or an owner promise is broken; minor = wording, a missing citation, a nit. Be specific: the draft's
line and the code's file:line; one fix per finding; no finding without a fix.

WRITE your report as Markdown to ${ROOT}/plan/layers/${c.slug}/review/${key}.md (the folder exists): a verdict line (lock |
amend-then-lock | redesign), then tables Blockers / Majors / Minors with columns id (the lens letter plus a number), where, what, fix,
then at most 15 lines of notes (what is good; what to watch in the build). Then RETURN the structured result with the same findings.
`

const JUDGE_SCHEMA = {
  type: 'object',
  required: ['lens', 'verdict', 'findings', 'report_file', 'notes'],
  properties: {
    lens: { type: 'string' },
    verdict: { type: 'string', enum: ['lock', 'amend-then-lock', 'redesign'] },
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

const draft = async (c) => {
  if (a.panelOnly) return { status: 'written', file: `${ROOT}/${c.slug}/README.md` }
  const d = await agent(draftPrompt(c), { label: `spec${c.n}:draft`, phase: `Draft ${c.n}`, model: 'opus', effort: 'xhigh', schema: DRAFT_SCHEMA })
  if (d) log(`Layer ${c.n} draft: ${d.status}, ${d.lines} lines, ${d.verified_claims} verified citations`)
  return d
}

const panel = async (c) => {
  // args.lenses = { '8': ['datasets', 'integration'] }: only those lenses of that layer (the others' reports are already on disk).
  const want = (a.lenses && a.lenses[String(c.n)]) || null
  const ps = (await parallel(c.lenses.filter(l => !want || want.includes(l.key)).map(l => () =>
    agent(judgeCtx(c, l.key) + '\n\n' + l.prompt + ' Your ids start with ' + l.letter + '.',
      { label: `judge${c.n}:${l.key}`, phase: `Panel ${c.n}`, model: 'fable', effort: 'high', schema: JUDGE_SCHEMA })
      .then(r => r && { ...r, lens: l.key })
  ))).filter(Boolean)
  for (const p of ps) {
    const n = s => p.findings.filter(f => f.severity === s).length
    log(`Layer ${c.n} ${p.lens}: ${p.verdict} - ${n('blocker')} blockers, ${n('major')} majors, ${n('minor')} minors`)
  }
  return ps
}

const out = {}
let p8 = null
if (ONLY.includes('8')) {
  const d8 = await draft(L[8])
  out.draft8 = d8
  if (d8 && d8.status !== 'blocked') p8 = panel(L[8])   // judged while Layer 9 is drafted
}
if (ONLY.includes('9')) {
  const d9 = await draft(L[9])
  out.draft9 = d9
  if (d9 && d9.status !== 'blocked') out.panel9 = await panel(L[9])
}
if (p8) out.panel8 = await p8
return out
