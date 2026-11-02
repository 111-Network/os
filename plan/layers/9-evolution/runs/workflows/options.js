export const meta = {
  name: 'layer-options-brief',
  description: 'Layer 8 or 9 (args.layer): close the critic\'s remaining gaps with scouts, draft the options brief (opus), check it against the reports (fable), apply one fix pass',
  phases: [
    { title: 'Gap-fill', detail: 'sonnet scouts on the gaps the critic left for the lead', model: 'sonnet' },
    { title: 'Draft', detail: 'one opus drafter writes plan/layers/<layer>/options.md in the Layer 7 brief\'s shape', model: 'opus' },
    { title: 'Check', detail: 'one fable judge verifies every fact, fork and pick against the reports and the locked rules', model: 'fable' },
    { title: 'Fix', detail: 'the drafter applies the judge\'s blockers and majors; the judge confirms', model: 'opus' },
  ],
}

const ROOT = '/home/user/os'
const a = args || {}
const LAYER = a.layer || '8-storage'
const L = {
  '8-storage': {
    n: 8, name: 'Storage & Shared State', readme: '"8. Storage & Shared State"',
    reports: '00-code-seams.md (as-built seams), 01-datasets-hub.md, 02-object-transfer.md, 03-shared-log.md, 04-local-store.md, 05-ui-consent-floor.md, 06-critic.md (nine conflicts, nine gaps, eighteen owner forks), 07-gap-1.md (G1: agent-state encryption)',
    neighbour: 'Layer 9 (Self-Healing & Evolution) is researched beside you: its reports are in ' + ROOT + '/plan/layers/9-evolution/research/ (read 06-critic.md conflict 9 and the files it cites when you settle the joint storage contract: incidents, the audit trail, the repair journal, domain tags).',
    gaps: [
      {
        key: 'g2-software-updates',
        title: 'G2: software updates beyond the 111 release (component payloads offline, Termux debs, APKs, mirroring duties)',
        prompt: `The critic (06-critic.md §4 G2) found this uncovered: README §8 says knowledge hubs host "software updates", but the sweep covered only the 111 release channel (Layer 7 F6: the hub serves channel-111-stable.toml, .minisig, blob/<sha256>). Research the rest, facts with citations:
(a) The optional component payloads (Layer 6: cache/packages/<id>/ hash-locked payloads, manifests under assets/components/ or wherever 1-bootstrap/6-transport pin them: read 1-bootstrap/README.md "components"/"payload" sections and lines 228-246, 6-transport/README.md §2 and the component manifest code under 1-bootstrap/crates/node111/src/daemon/components* or similar: grep). Today "never shipped as bytes in a release", fetched from upstream URLs by hash; offline, only a USB stick. Could a hub serve them in the same origin form (blob/<sha256> by the manifest's pinned hash, the manifest pins the URL list)? What does Layer 1's fetcher need (an origin list, like Layer 7's update origins)? Sizes: the Delta Chat + Madmail payloads (~104 MB), the rns/lxmf wheels (D26 (5): "always download, never bundle; the user installs" - read DECISIONS.md D26 (4)(5) and LICENSING.md), Meshtastic web, MeshChatX: take the sizes from 6-transport/README.md and TASKS.md.
(b) Termux: the 111 node on Android lives in Termux; packages are debs from packages.termux.dev (apt). Can a hub mirror a subset (a plain directory with a Packages index served by the existing listener; apt-cacher-ng; aptly; reprepro)? Which tool is zero-weight-off and fits "static files on the dashboard listener"? Termux's own APK and the Meshtastic/Delta Chat APKs: an F-Droid-style repo (fdroidserver: licence from its LICENSE file; what it needs at build time: Python, androguard, a signing key) versus a signed directory of APKs; what Android requires to install (user-enabled sideloading; no adb on the phone). 1-bootstrap/README.md:160 notes "APK refresh = Layer 9 maintenance task".
(c) Mirroring duties: when a hub conveys third-party binaries to other nodes, what each licence requires of the hub operator: GPL-3.0 §6 (Corresponding Source for object code: offer or same-place source), GPL-2.0 §3, MPL-2.0 §3.2, Apache-2.0 §4, the Reticulum License (read LICENSING.md for its terms as we recorded them: the rns/lxmf distribution clause), Mozilla-style notice duties. Facts, with the licence text lines, not advice. State what D26 (5) ("the user is the one who installs third-party software") changes when the hub is the source: is the hub operator the distributor?
(d) Candidates with licence from the upstream LICENSE file (raw.githubusercontent.com works; most project sites are denied): fdroidserver, apt-cacher-ng, aptly, reprepro, a pip "simple" index directory (PEP 503) for wheels, Kiwix's own mirror tooling if any, bandersnatch (pip mirror, weight), devpi. Fit: adopt / bridge / reference / reject with the Pi floor (zero weight off; static files preferred).
(e) Give the lead two or three fork options for "software updates on a hub" with a pick: for example (1) the hub's origin extends to component payloads and wheel files by pinned hash, served as static blobs, Layer 1's fetcher gains a hub origin for components; (2) plus a Termux apt directory and an APK directory signed by the hub owner's key; (3) nothing beyond the 111 release in v1, components by USB. Say what each costs (disk, a build-time tool, licence duties) and what Layer 1 or 6 text it amends.
Write the report to ${ROOT}/plan/layers/8-storage/research/07-gap-2.md (at most 250 lines; the format of the other reports: summary, candidates table, findings with file:line or URL, recommendation, open questions, unverified claims, sources).`,
      },
      {
        key: 'g3g4-chunks-and-pi-prior-art',
        title: 'G3+G4: bao-tree outboard sizes from the locked crate source; Internet-in-a-Box and offspot as Pi prior art',
        prompt: `Two gaps the critic (06-critic.md §4 G3, G4) left for the lead. Facts with citations, nothing run but reads and fetches.
(G3) 02-object-transfer.md proposes a custom sha256 chunk sidecar. The lock already has bao-tree 0.16.1 and blake3 1.8.7 (sources on this box: ~/.cargo/registry/src/*/bao-tree-0.16.1/ and blake3-1.8.7/; read them, cite file:line). Establish from the source: the BlockSize type and its range (is a 16 MiB block = BlockSize(14)? the chunk is 1 KiB, block = 2^n chunks), the outboard size formula and the figure for a 57 GB file at the chosen block size (the critic computed ≈217 KB at 16 MiB blocks: check), whether the crate can create and verify a PostOrderMemOutboard / PreOrderOutboard file beside the data (a sidecar outboard file), whether it supports verifying a byte range from a partial download (resumable Range pulls: encode_ranges / decode_ranges, the ChunkRanges type), and what Layer 5's spec already pins for Tier 2 (grep 5-messaging/README.md for bao, OBJ_CHUNK, chunk size, outboard). Compare with Metalink 4 pieces (RFC 5854 §4.1.4, if reachable) and aria2's chunk checksums as the HTTP-world equivalent. Conclude: the sidecar Layer 8 should use for dataset files (the outboard beside the blob; or Metalink; or both), and what verification a resumed Range pull gets.
(G4) Prior art for Pi knowledge hubs serving the same datasets: Internet-in-a-Box (github.com/iiab/iiab, iiab/iiab-factory; LICENSE: read; their kiwix role, the ZIM list they ship on a Pi 4/5 or Zero 2 W, the RAM they require, how they serve PMTiles/OSM vector tiles - the "iiab/maps" role or docs; their nginx config for Range), Kiwix Hotspot / offspot (github.com/offspot/*: overview README, their image builder's ZIM choices, kiwix-serve memory limits, the "dashboard" they ship), Kiwix's "kiwix-serve" docs for the --threads/--monitorLibrary flags and memory behaviour, Open Street Map prior art (protomaps on a Pi). Only raw.githubusercontent.com and search snippets may be reachable: cite what you fetched and mark the rest unverified. From these, extract the published numbers that settle the critic's conflict 7 (kiwix-serve RSS on a Pi; mini vs nopic ZIM sizes) and what they do about licences (attribution pages, the ODbL notice).
Write the report to ${ROOT}/plan/layers/8-storage/research/07-gap-3.md (at most 250 lines; the format of the other reports).`,
      },
    ],
    extraFacts: `The joint items with Layer 9 (the critic's conflict 9): incidents (plaintext JSONL vs an encrypted heal/journal), the audit trail (an own hash chain vs an adopted log), domain tags for the node key (one L2 rule), and where Layer 9's state lives (own files vs a Layer 8 namespace): the brief proposes ONE joint storage contract and marks it "joint with Layer 9" so the two decisions are taken together.`,
  },
  '9-evolution': {
    n: 9, name: 'Self-Healing & Evolution', readme: '"9. Self-Healing & Evolution"',
    reports: '00-code-seams.md (as-built seams), 01-observe-diagnose.md, 02-change-validate.md, 03-share-fixes-pr.md, 04-evolution-kb-agent.md, 05-ui-consent-floor.md, 06-critic.md (twelve conflicts, eight gaps, twelve owner forks), 07-gap-1.md (G1: the hub code-fix pipeline, containment and offline feasibility)',
    neighbour: 'Layer 8 (Storage & Shared State) is researched beside you: its reports are in ' + ROOT + '/plan/layers/8-storage/research/ (read 06-critic.md conflict 9 and 04-local-store.md / 03-shared-log.md where they place Layer 9 state: the joint storage contract is decided once, in both briefs with the same wording).',
    gaps: [
      {
        key: 'g2-agent-op-delegation',
        title: 'G2: AGENT_OP and delegation for Cat 2A/3 nodes over the fabric',
        prompt: `The critic (06-critic.md §3, §5 G2) found this uncovered: README §9 and the category table say Cat 2A (phones) and Cat 3 (laptops without a local model) "consume Layer 9 through a nearby hub", but the sweep covered only LAN offload (05 F6, Layer 3's v1 offload). Layer 5 reserved an AGENT_OP frame class as a Layer 9 hand-off (5-messaging/README.md:173 and :832; grep the mesh111 crate for AgentOp / AGENT_OP and the envelope kinds to see what exists: a kind number, a dispatch arm, nothing else?). Establish, with citations: (a) what Layer 3's offload does today (3-intelligence/README.md "offload", the agent's model routing: a phone's agent calls a hub's llama-server over the LAN with what auth? read agent111/src and the node111 intelligence module), and what breaks off-LAN; (b) what an AGENT_OP envelope would carry for Layer 9 (an incident summary + a request for diagnosis; a signed proposal back) and its size against the Tier 1 limit (the L5 frame budget: grep "Tier 1" and the 160 B / fragment limits in 5-messaging/README.md); (c) the trust rule: a phone delegates to its HOME HUB only (the attributed, enrolled relation of Layer 5/7), never to a contact; what the hub may read (the incident record: no message text by construction, see 01-observe-diagnose.md's incident fields) and what it returns (a proposal the phone's human accepts, per D26 (2)); (d) the Pi floor on both ends: nothing resident, event-driven on the incident; (e) alternatives: no fabric delegation in v1 (LAN offload only, a "needs a hub" badge), or AGENT_OP in v1 with the above rule, or a Layer 7 carrier ride (the hub is reachable through its mailbox carrier: read 7-relay/README.md §5-6, a DRAFT being edited live). Give the lead a fork with a pick and the L5 amendment it needs (additive, the AGENT_OP consumer only). Also cover the missed hand-off at 1-bootstrap/README.md:160 ("APK refresh = Layer 9 maintenance task"): what a proposal for a human-installed APK update looks like (no adb on a phone; a notice with the pinned hash; Layer 8's hub may hold the APK).
Write the report to ${ROOT}/plan/layers/9-evolution/research/07-gap-2.md (at most 250 lines; the format of the other reports: summary, candidates table if any, findings with file:line, recommendation, open questions, unverified claims, sources).`,
      },
      {
        key: 'g3-playbook-rules',
        title: 'G3: adopt vs build for playbook rules (cel, serde_json_path, a closed predicate enum; StackStorm, Salt reactor, Nagios handlers, MAPE-K)',
        prompt: `The critic (06-critic.md §5 G3) found that 01-observe-diagnose.md invents an "expect:" mini-language for playbook checks, where adoptable rule engines exist. Research, facts with licences read from upstream LICENSE files (raw.githubusercontent.com is reachable; crates.io API is; docs.rs is denied): (a) cel (cel-rust, 0.15.0, MIT): binary and dependency weight (its Cargo.toml dependencies; any proc-macro or regex weight), expression power (predicates over a JSON-like context, no side effects, bounded evaluation?), maturity and release date; (b) serde_json_path (RFC 9535 JSONPath, MIT): the same; (c) a closed predicate enum in our own code (a dozen predicates: service state equals, counter over threshold within N events, file exists, exit code equals, log line matches a pinned literal; evaluated by a 100-line matcher: no dependency), its limits; (d) jq-like (jaq, MIT/Apache) and rhai (MIT/Apache, a scripting language: too much power?) as references; (e) prior art for the SHAPE of a playbook: StackStorm rules (Apache-2.0: trigger, criteria, action), Salt reactor, Nagios/Icinga event handlers, Ansible handlers, the MAPE-K loop (monitor-analyse-plan-execute over knowledge), Kubernetes probes; what each got right about keeping checks declarative and the action set closed. Then: given the Pi floor (binary-size delta matters: estimate from the crates' sizes and their dependency trees via crates.io metadata), Layer 0's locked entry format (0-knowledge/README.md: a playbook as a KB entry category needs a Layer 0 amendment: say what fields) and rule 1 (data never executes: a playbook is declarative, the actions are the typed repair kinds of 02-change-validate.md), give the lead a fork with a pick: closed enum now (zero dependency) vs cel vs serde_json_path, and what grammar version pinning looks like (a schema_version in the entry, refusing unknown predicates: fail closed).
Write the report to ${ROOT}/plan/layers/9-evolution/research/07-gap-3.md (at most 250 lines; the format of the other reports).`,
      },
    ],
    extraFacts: `The joint items with Layer 8 (the critic's conflict 9 and G4): where Layer 9's state lives (own JSONL files vs a Layer 8 namespace in the encrypted store), the incident record's capacity pruning, the audit trail (an own hash chain vs an adopted log vs the store's own seq), domain tags for the node key (one L2 rule with Layer 7's 111/epoch/v1 and 111/broadcast/v1): the brief proposes ONE joint storage contract, marked "joint with Layer 8", with the same wording the Layer 8 brief carries. The Layer 7 spec (7-relay/README.md, a DRAFT edited live today) is the neighbour below: cite it by section, never by line.`,
  },
}[LAYER]
if (!L) throw new Error('unknown layer ' + LAYER)
const DIR = `${ROOT}/plan/layers/${LAYER}`
const OUT = `${DIR}/research`
const BRIEF = `${DIR}/options.md`
const CHECK = `${DIR}/review/options-check.md`

const CTX = `
You work on the 111network OS build (repo root ${ROOT}). Today is 2026-11-02. Layer ${L.n} (${L.name}) is in research; its plan folder is
${DIR}/ (read its README.md first: what the layer is, what it inherits, the rules that bound it). The research sweep is complete: the
reports are in ${OUT}/: ${L.reports}. ${L.neighbour}
Read-only: run no git write commands, no cargo, start no process; reads, greps and web fetches (ToolSearch select:WebSearch,WebFetch; this
box's network policy allows package indexes, crates.io's API and raw.githubusercontent.com and denies most documentation sites: mark what you
could not fetch "(unverified)") are fine. Your training data is stale: fetched facts and the repo win. No model names or identifiers in any
file you write (say "the lead", "a scout", "the critic", "the judge"); no attribution lines.
`

const SCOUT_SCHEMA = {
  type: 'object',
  required: ['report_path', 'headline', 'findings', 'recommendation', 'open_questions', 'unverified'],
  properties: {
    report_path: { type: 'string' },
    headline: { type: 'string' },
    findings: { type: 'array', items: { type: 'string' } },
    recommendation: { type: 'string' },
    open_questions: { type: 'array', items: { type: 'string' } },
    unverified: { type: 'array', items: { type: 'string' } },
  },
}

const DRAFT_SCHEMA = {
  type: 'object',
  required: ['status', 'file', 'lines', 'forks', 'questions', 'corrections', 'joint', 'notes'],
  properties: {
    status: { type: 'string', enum: ['written', 'partial', 'blocked'] },
    file: { type: 'string' },
    lines: { type: 'integer' },
    forks: { type: 'array', items: { type: 'object', required: ['id', 'title', 'pick'], properties: { id: { type: 'string' }, title: { type: 'string' }, pick: { type: 'string' } } } },
    questions: { type: 'array', items: { type: 'string' }, description: 'the owner questions beyond the forks, each with the default you wrote' },
    corrections: { type: 'array', items: { type: 'string' }, description: 'plan corrections the brief records (which file, what)' },
    joint: { type: 'string', description: 'the joint storage contract wording you used, verbatim' },
    notes: { type: 'string' },
  },
}

const CHECK_SCHEMA = {
  type: 'object',
  required: ['verdict', 'findings', 'report_file', 'notes'],
  properties: {
    verdict: { type: 'string', enum: ['ready', 'fix-first'] },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        required: ['id', 'severity', 'where', 'what', 'fix'],
        properties: {
          id: { type: 'string' },
          severity: { type: 'string', enum: ['blocker', 'major', 'minor'] },
          where: { type: 'string', description: 'section and line of the brief' },
          what: { type: 'string' },
          fix: { type: 'string' },
        },
      },
    },
    report_file: { type: 'string' },
    notes: { type: 'string' },
  },
}

phase('Gap-fill')
const fills = (await parallel(L.gaps.map(g => () =>
  agent(`${CTX}
GAP-FILL SCOUT (${g.title}). Read the critic's report ${OUT}/06-critic.md and the report(s) the gap concerns before you start, not all of
them. ${g.prompt}
Return: report_path, headline, findings (each with a citation), recommendation (the fork options and your pick, at most 120 words),
open_questions, unverified.`, { label: `gap:${g.key}`, phase: 'Gap-fill', schema: SCOUT_SCHEMA, model: 'sonnet', effort: 'high' })
    .then(r => r && { ...r, key: g.key, title: g.title })
))).filter(Boolean)
log(`gap-fills: ${fills.map(f => f.key).join(', ') || 'none'}`)

phase('Draft')
const MODEL_BRIEF = `${ROOT}/plan/layers/7-relay/options.md`
const draftPrompt = `${CTX}
You are the lead's drafter for the OPTIONS BRIEF of Layer ${L.n}: ${BRIEF}. The brief is what the owner reads to decide the layer; after the
owner's decision it becomes a DECISIONS.md entry, then the spec. Its SHAPE is the Layer 7 brief ${MODEL_BRIEF} (read it whole: the status
header; §0 "What the research settled" as numbered facts, each citing its report and section; §1 the layer in one paragraph; §2 polled forks
F1..Fn, each with the options A/B/C, the costs in numbers, the pick first and why, and what locked text it amends; §3 detail picks the panel
reviews but the owner is not polled on; §4 plan corrections this brief records (file by file); §5 open questions for the owner with the
lead's default for each; §6 the critic's digest, the gap-fill digest and the unverified-claims list for the owner's machine). Match that
voice and density; 350 to 600 lines; the status line reads "🧭 deciding — awaiting the owner (sweep complete, run ids in
research/coordinates.md)" (do not invent run ids: write "run ids in research/coordinates.md").

INPUTS, all of them read in full before you write: every report in ${OUT}/ including the gap-fills 07-gap-*.md (the gap-fill scouts'
headlines: ${fills.map(f => `[${f.key}] ${f.headline}`).join(' | ') || 'none ran'}); ${DIR}/README.md; ${ROOT}/README.md (the layer's
section ${L.readme}, the category table, the principles); ${ROOT}/AGENTS.md (the Pi floor rule verbatim); ${ROOT}/plan/DECISIONS.md D17,
D20, D21, D22, D26, D29, D30, D31 (a fork may not silently reverse a decision: if a pick needs one reversed, say so as a fork option and
mark it); ${ROOT}/plan/INDEX.md "Process (per layer)"; the locked specs the reports cite where a fact decides a pick (verify by reading the
cited lines; cite file:line for code and spec, section numbers for 7-relay/README.md which is being edited live).

WHAT THE BRIEF MUST DO:
1. Settle every conflict the critic lists (06-critic.md "Conflicts") explicitly: name the conflict, the resolution, the reports on each side.
2. Turn every owner fork the critic lists into either a polled fork in §2 (with the pick) or an open question in §5 (with the default);
   none dropped. Group related ones; number the forks F1..Fn in the order the owner should decide them (the ones that bound the others first).
3. Every pick obeys: the Pi floor (nothing resident, no timer, no poll, no runtime dependency, no build-time weight in the required
   configuration without the owner's say; every hub-only cost quantified: RSS, binary delta, listeners, timers, disk writes; "unmeasured"
   where no number exists, never a guess presented as a number); adopt, don't reinvent (a custom mechanism only where the brief shows no
   adoptable project fits, and says which were considered); offline-first, consent before any download, no telemetry, pruning by capacity
   never by expiry, no wall clock in crypto or ordering; D22 licensing (every new crate or program with its licence as read from upstream;
   dataset and content licences as obligations stated as facts); the locked specs amended only additively and only where a fork says so;
   the agent holds zero key material and its bearer is refused on every write (D26 (2)); Reticulum stays a separate program.
4. ${L.extraFacts}
5. §4 plan corrections: what the reports found wrong or stale in the plan and specs (numbers, citations, missed hand-offs), file by file,
   so the lead applies them after the decision; never edit those files yourself.
6. §6: the critic's digest (verdict, gaps and how each was closed or left), the gap-fill digest, and the unverified-claims list for the
   owner's machine, merged from the critic and the gap-fills, grouped (sizes, binaries, measurements, behaviour, legal/licences).
7. Honesty: every number carries its source (report §, URL date, file:line); "(unverified)" where the scouts said so; no claim about code
   that you did not read; the brief is read by the owner, so plain words, short sentences, no jargon left unexplained on first use.

Write ${BRIEF}. Return: status, file, lines, forks (id, title, pick: one line each), questions (each with its default), corrections,
joint (the joint contract wording verbatim), notes (at most 10 lines: what you could not settle and why).`
const draft = await agent(draftPrompt, { label: 'draft', phase: 'Draft', schema: DRAFT_SCHEMA, model: 'opus', effort: 'xhigh' })
if (!draft || draft.status === 'blocked') return { layer: LAYER, fills, draft, check: null, fix: null }
log(`draft: ${draft.status}, ${draft.lines} lines, ${draft.forks.length} forks, ${draft.questions.length} questions`)

phase('Check')
const checkPrompt = (round) => `${CTX}
You are the fable JUDGE of the Layer ${L.n} options brief ${BRIEF}${round > 1 ? ` (round ${round}: after a fix pass; your previous report is ${CHECK}: confirm each previous blocker and major is closed and look for regressions)` : ''}.
The brief is what the owner decides on; a wrong fact or a dropped fork costs a layer. Read the brief whole, then the critic's report
${OUT}/06-critic.md, then every report the brief cites where a fact or a pick rests on it. Verify, adversarially:
(1) FACTS: spot-check at least 20 of the brief's numbered facts and numbers against the cited report, URL or file:line (read the code
lines where cited); a fact with no source, a changed number, a report's "(unverified)" dropped, or a claim the report does not make is a
finding (blocker when a pick rests on it).
(2) COMPLETENESS: every conflict in the critic's list is settled by name; every owner fork of the critic's list appears as a §2 fork or a
§5 question; every gap is closed or honestly left; the hand-offs in the plan README are all addressed.
(3) RULES: every pick against the Pi floor (nothing resident/periodic/new dependency in the required configuration without the owner's
say; hub-only costs quantified or marked unmeasured), adopt-don't-reinvent (a custom mechanism justified against named alternatives),
offline-first/consent/no telemetry/capacity-not-expiry/no wall clock, D22 licensing from upstream, D26 (2) and the L2 "zero key
material" rule, additive amendments only; a pick that silently reverses a DECISIONS.md entry or contradicts a locked spec is a blocker.
(4) THE JOINT CONTRACT with the neighbour layer is stated and marked joint.
(5) FORM: the Layer 7 brief's shape and status line; 350-600 lines; no model names; no attribution lines; every §2 fork has the options,
the costs, the pick first and what locked text it amends; every §5 question has a default.
Write your report to ${CHECK} (findings as a table: id, severity, where, what, fix; then "What I verified" with the citations you
checked). Return: verdict (ready: no blocker and no major; fix-first otherwise), findings, report_file, notes.`
let check = await agent(checkPrompt(1), { label: 'check', phase: 'Check', schema: CHECK_SCHEMA, model: 'fable', effort: 'high' })
log(`check: ${check ? check.verdict : 'none'} (${check ? check.findings.length : 0} findings)`)

let fix = null
if (check && check.verdict === 'fix-first') {
  phase('Fix')
  const must = check.findings.filter(f => f.severity !== 'minor')
  fix = await agent(`${CTX}
You are the drafter of ${BRIEF} again (you wrote it; read it whole). The judge's report ${CHECK} lists findings; apply every blocker and
major exactly as its fix says (${must.map(f => f.id).join(', ')}), and the minors where the fix is a line; keep the shape, the line budget
(350-600) and every rule of the first draft (no model names, no attribution lines, facts sourced, "(unverified)" kept). Where a fix would
contradict a report or a locked rule, do not apply it and say why in notes. Return: status, file, lines, forks, questions, corrections,
joint, notes (what you applied by id and what you declined).`, { label: 'fix', phase: 'Fix', schema: DRAFT_SCHEMA, model: 'opus', effort: 'xhigh' })
  check = await agent(checkPrompt(2), { label: 'recheck', phase: 'Fix', schema: CHECK_SCHEMA, model: 'fable', effort: 'high' })
  log(`recheck: ${check ? check.verdict : 'none'} (${check ? check.findings.length : 0} findings)`)
}

return { layer: LAYER, fills: fills.map(f => ({ key: f.key, report: f.report_path, headline: f.headline, recommendation: f.recommendation })), draft, check, fix }
