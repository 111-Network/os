export const meta = {
  name: 'layer-9-research',
  description: 'Layer 9 (Self-Healing & Evolution) research sweep: as-built seams scout + 5 lens scouts, completeness critic, gap-fill',
  phases: [
    { title: 'Scout', detail: '1 opus seams scout + 5 sonnet lens scouts, web-verified where the box allows, reports on disk' },
    { title: 'Critique', detail: 'opus completeness critic over all reports' },
    { title: 'Gap-fill', detail: 'up to 2 scouts on blocking gaps' },
  ],
}

const ROOT = '/home/user/os'
const OUT = ROOT + '/plan/layers/9-evolution/research'

const CTX = `
You are a research scout for the 111network OS build (repo root: ${ROOT}). Today is 2026-11-02.

WHAT 111 IS: a minimal operating layer that turns any device (old phone, laptop, Pi, LoRa board) into a node of a resilient
offline mesh. Principle, non-negotiable: ADOPT, DON'T REINVENT. Existing projects are assets we run, bridge and show; custom
code is only glue. A local AI agent is the interface and the self-healer. Read ${ROOT}/README.md first: the section
"9. Self-Healing & Evolution" is the layer you research. Verbatim: "The local agent continuously observes the health of every
layer the device runs. On detecting a failure or degradation it formulates a diagnosis, generates and internally validates a
corrective change, and - if the change succeeds - packages it as a pull request. Humans remain the final gate for merging.
Nodes share proven fixes with each other through the gateways, so a repair validated on one node can heal others. Over time
the same process improves the agent itself, expands the Knowledge Layer, and refines bridging logic - always scoped to the
layers the device actually operates." Also read the Operating Principles ("The agent is both servant and guardian of the
node"; "Autonomy is progressive and always human-supervised at the final step"; no telemetry, no policy limits) and the
layer-category table: Layer 9 runs natively on Cat 2B, 4A and 4B, partially or delegated on 2A and 3, not on Cat 1. Layer 8
(storage) is the neighbour below (its research runs beside yours; do not research it), Layer 7 is decided (spec in draft at
${ROOT}/7-relay/README.md if that file exists today - read it if it does; it is a DRAFT, not locked).

WHAT IS ALREADY BUILT (Layers 0-7; do not re-research them, wire to them; cite file:line when you point at code):
- L0 Knowledge base ${ROOT}/0-knowledge/ (entries with YAML frontmatter; the rubric candidate -> testing -> proven and the
  adopt-vs-build gate in its README; the index built by _meta/build.py with a --check mode; the navigation contract
  ${ROOT}/0-knowledge/AGENTS.md; recipes/ hold step-by-step procedures). Entries you may cite: projects/{opencode,llama-cpp,
  meshchatx,meshtastic,reticulum,deltachat-rpc-server}, models/qwen3-4b, concepts/{store-and-forward-states,
  preferred-networks,secure-contexts}.
- L1 Bootstrap (${ROOT}/1-bootstrap/README.md): the Rust daemon "111d" (crate node111 at ${ROOT}/1-bootstrap/crates/node111)
  with a tokio supervision tree (restart-in-place per task with exponential back-off and a cap - called "the Layer 9 seam"
  in the README; supervise.rs), children with /health (200/503, tri-state degraded), systemd Type=notify on Linux, a Termux
  runit + cronie /health check on Android ("placeholder seam until Layer 9"), the updater with staging, a root helper that
  flips current/previous, probation and rollback (update_guard.rs, probation.rs, update_helper.rs; "APK refresh = Layer 9
  maintenance task"), a refusal philosophy (a node refuses what it cannot do honestly).
- L2 Identity (${ROOT}/2-identity/README.md): Ed25519 node key; "the agent signs its outbound artifacts (proposals, patch PRs)
  with the node key" (Layer 3's contract); attestations; the mesh epoch (Layer 7 defines it).
- L3 Intelligence (${ROOT}/3-intelligence/README.md and crates/agent111): "111-agent", a thin always-on Rust agent loop
  (D10 H1) with FIXED, shell-free, schema-validated tools, KB grounding, "monitoring", and the task API
  'POST /v1/agent/task {kind: diagnose | propose | patch, refs...}' -> a task id, progress on the events stream, the result a
  proposal document under /var/lib/111/agent/proposals/; 'patch' requires a hub (opencode adopted on hub tiers, opt-in);
  "The behavior of self-healing - monitor cadence, diagnosis playbooks, sandbox validation, PR packaging - is Layer 9, which
  consumes this layer through one task API"; the local model is qwen3-4b on llama.cpp where it fits, else offload to a hub.
- L4 Dashboard (${ROOT}/4-interface/README.md; contract ${ROOT}/4-interface/api/openapi.yaml): proposals are read by 111d
  from agent/proposals/*.json and decided through 'POST /api/v1/proposals/{id}/decision' into agent/decisions/{id}.json
  ("applying accepted proposals is Layer 9's job - the UI says so"); "every restart is a proposal" (D26); per-service
  restart with 3-state dots; the Repairs L9 page is a structural stub; the deferred list names "automatic proposal
  application (L9 consumes decisions/)".
- L5 Messaging & Data (${ROOT}/5-messaging/README.md, locked): sealed envelopes; the PATCH class, AGENT_OP and RECEIPT_STRONG
  are defined for Layer 9 (section 19, section 21: "the apply -> verify -> undo engine (P4)"); delivery states with evidence.
- L6 Transport & Bridge (${ROOT}/6-transport/README.md, locked D29): the DEVICE CORE's narrow applier cut - typed device
  actions applied, verified and reverted on failure (src/daemon/devices/**: the four-action applier, probation of a posture
  change, "a record-less applier entry"); components enabled only with recorded consent and a preflight; the integrated
  review made every restart a proposal and every apply run to completion in a spawned task; the verify gates G1-G7 under
  6-transport/verify/ run in network namespaces (root, no hardware) - the kind of "internal validation" a hub could run.
- L7 Relay & Presence (DECIDED, ${ROOT}/plan/DECISIONS.md D30; spec draft in progress): anything that travels between nodes
  travels as sealed frames through carriers and paired hubs; the hub is an update origin ('111 update --from'); the Layer 1
  trust fixes (channel check, keychain walk, min-upgrade-from, a freeze stance); git is REJECTED on the node (Layer 7 F6: no
  git, no bundles for the node); the agent proposes and never acts (D30, section 3 detail picks); the mesh epoch's later
  consumers (cert delegation) may be Layer 9's.
- Parked note P4 in ${ROOT}/plan/cross-layer/parked.md: "Apply -> verify -> undo-on-failure engine for device actions ('apply,
  verify, undo is smart!') - the owner wants it; picked up by Layer 9 (self-healing), or a narrow cut in Layer 6" (the narrow
  cut shipped). Decisions ${ROOT}/plan/DECISIONS.md D10, D17, D22, D26, D29, D30; the plan protocol ${ROOT}/plan/EXECUTION.md;
  the CI ${ROOT}/.github/workflows/ci.yml. The Layer 7 options brief ${ROOT}/plan/layers/7-relay/options.md is the SHAPE the
  lead will write for this layer (settled facts -> polled forks with picks -> plan corrections): write so it can be written.

RULES (non-negotiable):
1. Humans are the final gate: no code from a peer or from the agent is applied to a node without a human decision, and no
   node merges anything; a "fix" that arrives from another node is a PROPOSAL on the receiving node. Typed, reversible,
   consented actions may be the only things a node applies by itself, and only within a scope the user set.
2. The agent's tools stay fixed, shell-free and schema-validated (D10); the agent signs its artefacts with the node key;
   nothing leaves the node unsealed; no telemetry; no policy limits of our own.
3. THE RASPBERRY PI IS THE FLOOR (owner rule, AGENTS.md): "continuously observes" must not become a periodic poll in the
   required configuration without the owner's say. Prefer event sources (supervisor state changes, health flips, netlink,
   the events stream, log lines); quantify anything periodic (wakeups, RSS, CPU); the agent's own resident cost is measured
   (111-agent 5.3 MB idle release) and nothing may be added to it silently.
4. Reticulum stays a separate program (${ROOT}/LICENSING.md); D22: our code is GPL-3.0-or-later; every new dependency needs a
   sign-off row; incompatible licences stay out. Contributions packaged as pull requests fall under the project's licence.
5. No wall clock in crypto or ordering; offline-first (a node with no internet still heals itself and still receives fixes
   through gateways as sealed frames); no runtime downloads without consent.
6. Device floor: Linux x86_64/aarch64 (musl-static Rust binary) and Android via Termux (no root, no cargo, background
   limits, cron placeholder). A node cannot compile itself; "internally validates" must be designed for what a node can run.
7. License gate for adoption: verify every licence from the upstream LICENSE file, never from memory. Note release recency
   with dates. Mark anything you could not verify "(unverified)". Your training data is stale: fetched facts win.
8. Safety claims about autonomous agents are facts to cite, not opinions: cite published evaluations and incident reports
   for any "self-improving" pattern you recommend or reject.

THIS BOX'S NETWORK (read before you fetch; it decides your method): outbound HTTPS goes through a policy proxy. REACHABLE:
raw.githubusercontent.com (any public repo's files: LICENSE, README, docs/, source - use this for every GitHub-hosted
project), pypi.org and files.pythonhosted.org (package JSON at https://pypi.org/pypi/<name>/json), crates.io and
index.crates.io (https://crates.io/api/v1/crates/<name>), registry.npmjs.org, proxy.golang.org. DENIED (a fetch fails at
CONNECT with 403 or times out - do not retry): docs.rs, rfc-editor.org, github.com HTML pages, api.github.com, arxiv.org
(probably; try once), most project sites. WebSearch works (it runs outside the box) and returns snippets with URLs: cite the
URL and mark a claim "(search snippet)" when you could not fetch the page. For a GitHub project, derive the raw URL
(https://raw.githubusercontent.com/<owner>/<repo>/<default-branch>/<path>; try main then master) and read the LICENSE and
README from there. One failed fetch per host is enough.

METHOD (mandatory unless your lens says "no web"):
- First call ToolSearch with query "select:WebSearch,WebFetch" to load the web tools, then research on the live web within
  the network note above. Prefer upstream repos, package indexes and primary sources. Record the exact URL for every factual
  claim. Fetch the LICENSE file for every candidate you rate "adopt" or "bridge".
- Read only the repo sections named in your lens (sed -n / grep / rg); do not slurp whole trees. Cite file:line.
- Do NOT write code and do NOT modify any repo file except your own report path. Do NOT run cargo, pnpm, 111d, rnsd or any
  server. Nothing of yours may be left running.
- Cost discipline: aim for at most 40 web fetches; stop when the recommendation is grounded.

REPORT (write it with the Write tool, markdown, at most 3500 words, to the path given in your lens; then return the
structured summary). Sections, in order: 1. Summary (at most 6 bullets) - 2. Candidates table (name - upstream - license
(verified?) - language/runtime - maturity signals (last release with date, activity, adopters) - resident weight (RSS /
binary / listeners / timers, measured or published, or "unknown") - fit: adopt|bridge|reference|reject - why) -
3. Findings that change the plan (numbered facts with URLs or file:line) - 4. Recommendation: where a genuine fork exists,
give 2-4 options with one-line trade-offs and your pick - 5. Open questions for the owner - 6. Sources.
`

const LENSES = [
  {
    key: 'seams', model: 'opus', effort: 'high',
    prompt: `LENS 0 - AS-BUILT CODE SEAMS (no web; read code; every claim line-verified with file:line).
Map exactly where Layer 9 plugs into the built tree, so the options brief and the build briefs can cite real seams. Read,
in this order, only what you need of: ${ROOT}/1-bootstrap/crates/node111/src/daemon/{supervise*.rs,mod.rs,health*.rs,
services/**} (the supervision tree: restart-in-place, back-off and cap, pdeathsig, piped stdio, state watch, what a
"crash loop" looks like and who sees it; /health tri-state; the services registry and per-service restart; systemd notify
and watchdog), ${ROOT}/1-bootstrap/crates/node111/src/{update_guard.rs,update_helper.rs,probation.rs,manifest.rs} and
src/install/** (staging, the root helper's request file update-request.json, probation, rollback, the torn-payload guard:
the transactional pattern a Layer 9 applier would reuse), ${ROOT}/1-bootstrap/crates/node111/src/daemon/intelligence/**
and ${ROOT}/3-intelligence/crates/agent111/src/** (the agent loop: any monitoring cadence or timer? the task API and its
kinds; the proposals directory and document schema; tools.rs: every tool and how one is added; KB grounding; opencode
integration on hubs: what exists, what is a stub; the 2 s intelligence mirror in the dashboard), ${ROOT}/1-bootstrap/
crates/node111/src/daemon/dashboard/api_v1/{proposals*.rs,services*.rs,status.rs,events*.rs} and the decisions directory
(schema; who reads decisions/ today: nobody?), ${ROOT}/1-bootstrap/crates/node111/src/daemon/devices/** (the applier:
apply -> verify -> revert; what is generic and what is device-specific; probation of a posture change; the proposal types
and the accept path), ${ROOT}/1-bootstrap/crates/node111/src/daemon/components/** (enable/disable with consent and
preflight: a typed, reversible action), the logs the agent can read (the daemon's log file or journal, the events stream,
any ring buffer), ${ROOT}/5-messaging/crates/mesh111/src/envelope.rs (PATCH, AGENT_OP, RECEIPT_STRONG: defined? produced?),
${ROOT}/1-bootstrap/crates/node111/src/identity/** (signing an artefact with the node key: what exists for proposals),
${ROOT}/0-knowledge/_meta/build.py (the --check mode a node could run; is Python on the node?), ${ROOT}/6-transport/verify/
*.sh and ${ROOT}/.github/workflows/ci.yml (what "internal validation" infrastructure exists and what it needs: root, netns,
cargo - none of which a phone has), ${ROOT}/4-interface/api/openapi.yaml and ${ROOT}/4-interface/ui/src (the proposals UI,
the Repairs stub, the services page), and ${ROOT}/7-relay/README.md if it exists (the carrier for sealed frames; the update
origin; the agent's proposal-only rule). Also re-verify the timer baseline of ${ROOT}/plan/layers/7-relay/research/
00-code-seams.md section 3.1 (do not redo it; note what a Layer 9 observer would add).
Deliver ${OUT}/00-code-seams.md with per-subsystem tables (file:line - what exists - the seam - what Layer 9 must add or
change) for six needs: (a) observation sources (supervisor state, health, netlink, the events stream, logs, update
probation outcomes, custody backlog, disk) and the timer baseline; (b) the diagnosis inputs the agent can read today
through its tools and what is missing; (c) the change vocabulary already typed (device actions, component enable/disable,
service restart, settings PATCH, an update request, a posture change) and the applier's apply -> verify -> revert shape;
(d) validation infrastructure a NODE can run (no cargo, no root on a phone; a hub with netns) versus what only CI runs;
(e) the proposal -> decision -> application flow (what exists end to end, what is missing: the applier of accepted
proposals, the decision schema, the audit trail); (f) packaging and sharing: signing, the PATCH class, the hub's opencode,
the KB check, what a "fix" artefact could contain. Then "traps" (things that look like seams but are not), the current test
counts per crate (read the board summaries; run nothing), and open questions. Return the structured summary (candidates
may be empty).`,
  },
  {
    key: 'observe-diagnose', model: 'sonnet', effort: 'high',
    prompt: `LENS 1 - OBSERVING EVERY LAYER WITHOUT A POLL, AND DIAGNOSIS.
README section 9 opens with "continuously observes the health of every layer the device runs" and "formulates a diagnosis".
The Pi-floor rule forbids a new periodic poll in the required configuration without the owner's say; the node already has
event sources (the seams scout lists them in ${OUT}/00-code-seams.md if it exists: supervisor state changes, /health flips,
netlink, the SSE events stream, update probation outcomes, log lines). Research: (1) EVENT-DRIVEN OBSERVATION: what
"failure or degradation" means per layer in observable terms (a child restarting in a loop, /health 503 or degraded, a
custody backlog growing, a carrier unreachable, disk near capacity, a radio board erroring, the UI unreachable, a stale
update, battery on a phone) and which existing event each maps to; where a periodic probe is unavoidable (a watchdog
heartbeat is already systemd's), give the numbers; (2) INCIDENT RECORDS: a structured, local, capacity-pruned incident
(what, when by monotonic time and mesh epoch, evidence, attempted actions, outcome) - prior art formats (OpenTelemetry
events, systemd journal fields, Kubernetes Events, SRE postmortem templates); (3) DIAGNOSIS PLAYBOOKS as KB recipes: a
format a small local model can follow deterministically (checklist, decision tree, "if X then try Y, verify Z"), prior art
(runbooks, Ansible playbooks, Kubernetes operator reconcile loops, systemd OnFailure, monit (AGPL - note), Netdata health
alarms (GPL-3), Prometheus alert rules (weight: reject), AIOps root-cause tools - reference only); (4) THE MODEL'S
CAPABILITY: what qwen3-4b on llama.cpp can diagnose locally (cite evaluations) versus what needs the hub's opencode; how
hallucinated diagnoses are contained (always a proposal; evidence required; confidence shown); (5) PRIVACY: everything
local, nothing leaves the node unsealed, no telemetry; (6) ANDROID: no cron of ours; what Termux allows for event-driven
observation (runit, inotify, Termux:API callbacks). Give 2-4 options (events only; events plus an opt-in low-rate probe;
a literal monitor loop) with numbers and your pick. Deliver ${OUT}/01-observe-diagnose.md.`,
  },
  {
    key: 'change-validate', model: 'sonnet', effort: 'high',
    prompt: `LENS 2 - THE CORRECTIVE CHANGE, ITS VALIDATION, AND APPLY -> VERIFY -> UNDO.
README section 9: "generates and internally validates a corrective change". Parked P4: "apply, verify, undo is smart!" - the
owner wants the engine; Layer 6 shipped a narrow cut for device actions (read the seams scout's (c) if ${OUT}/00-code-seams.md
exists, else ${ROOT}/6-transport/README.md section 12 and the devices applier). Research: (1) THE CHANGE VOCABULARY: typed
actions a node can apply itself (a settings PATCH, a component enable/disable, a service restart, a device action, an
update request, a KB or recipe edit, an agent prompt or tool configuration change) versus changes that can only be a pull
request (code, manifests, anything signed by the release key); for each: reversibility, blast radius, the verify step that
proves it worked; (2) VALIDATION ON A NODE: what a node can run (a phone: nothing but the action's own verify; a Pi: the
verify step; a hub: network namespaces and the Layer 6 verify scripts as root) - "internally validates" designed honestly
per category; dry-run and canary patterns; probation periods (the updater's probation is the precedent); (3) APPLY -> VERIFY
-> UNDO generalised: transactional patterns in proven systems - JunOS/IOS-XR "commit confirmed" (auto-rollback unless
confirmed: the exact pattern), NixOS generations, ostree and transactional-update, snapper, Android A/B and ESP-IDF OTA
rollback, Kubernetes rollouts with health gates, Erlang/OTP supervisors, Ansible/Salt idempotence; which pattern fits typed
actions on a single node with no second partition; (4) WHAT MAY AUTO-APPLY: tiers by consent (observe only; propose; apply
reversible changes within a user-set scope; never code), the audit trail, the human gate; (5) SAFETY: a change that
removes the user's access (the dashboard, the pairing) must be refused or guarded; loops of apply/undo; conflicting
proposals; (6) prior art for AI-generated fixes with validation (SWE-bench style agents, Google's and Meta's auto-fix
systems: cite their reported acceptance and regression rates). Give 2-4 options with picks. Deliver
${OUT}/02-change-validate.md.`,
  },
  {
    key: 'share-fixes-pr', model: 'sonnet', effort: 'high',
    prompt: `LENS 3 - PACKAGING A FIX AS A PULL REQUEST, AND SHARING PROVEN FIXES BETWEEN NODES.
README section 9: "packages it as a pull request. Humans remain the final gate for merging. Nodes share proven fixes with
each other through the gateways, so a repair validated on one node can heal others." Constraints: the node has no git
(Layer 7 F6 rejected git and git bundles on the node); a phone cannot compile; the project lives on GitHub (111-Network/os);
Layer 5 reserved the PATCH class, AGENT_OP and RECEIPT_STRONG; Layer 7 moves sealed frames through carriers and paired
hubs; the hub tier runs opencode (Layer 3, opt-in). Research: (1) WHAT A FIX ARTEFACT IS: a typed change plus its evidence
(the incident, the diagnosis, the verify log, the device category and layer set it was validated on), signed by the node
key; a unified diff only when the change is code; metadata (reproducibility, versions, hashes); prior art for signed patch
exchange (Debian's patch tracker and .dsc signing, git format-patch and signed commits, Nix/NixOS bot PRs, Dependabot and
Renovate, OpenWrt, F-Droid reproducible builds, Gentoo bugzilla patches); (2) FROM A NODE TO A PULL REQUEST: the hub with
internet opens the PR (through opencode with an owner-provided token? a GitHub App? a bundle the owner uploads by hand?),
what the PR body must carry (the evidence, the verify log, "generated on a node" provenance, the licence: contributions
under GPL-3.0-or-later; DCO or not), rate limits and spam control (a node never opens a PR by itself; the owner's hub does,
once per validated fix, with the owner's consent setting), the project's CI as the "internal validation" of code changes
(ci.yml; the repo's verify gates), (3) THE FIX EXCHANGE between nodes: a sealed PATCH frame through gateways carrying the
artefact, RECEIPT_STRONG for acknowledgement, the receiving node treats it as a proposal (rule 1), what "proven" means
(the KB rubric: testing needs a smoke log, proven needs the field program), how a node decides a fix applies to it (the
same category, layer set and versions), trust (the author's node key must be a known contact or the hub; the owner's
hub may vouch), supply-chain safety (typed actions and config only may auto-propose; code never executes from a frame),
abuse (a malicious "fix" that disables the gateway: refused by scope), (4) metered links and the Pi floor for the exchange
(fix artefacts are small; say the byte budget), (5) licensing of shared fixes. Give 2-4 options with picks and the v1 cut
(probably: a signed fix artefact as a proposal plus the owner's hub opening PRs; the exchange over the fabric v1.1 if the
carrier is not there yet). Deliver ${OUT}/03-share-fixes-pr.md.`,
  },
  {
    key: 'evolution-kb-agent', model: 'sonnet', effort: 'high',
    prompt: `LENS 4 - THE AGENT IMPROVING ITSELF, THE KNOWLEDGE LAYER AND THE BRIDGING LOGIC.
README section 9 closes: "Over time the same process improves the agent itself, expands the Knowledge Layer, and refines
bridging logic - always scoped to the layers the device actually operates." Research what each clause can honestly mean
under the rules (humans gate; fixed shell-free tools; the Pi floor; adopt, don't reinvent): (1) EXPANDING THE KNOWLEDGE
LAYER: the agent authoring KB entries and recipes (${ROOT}/0-knowledge/README.md rubric: candidate from desk research,
testing with a smoke log, proven in the field; _meta/schema.md and build.py --check; provenance and last_verified; the
staleness rule), how an incident and its fix become a recipe, review by the owner or by the KB's own lint, prior art
(wiki bots, Dependabot-style maintenance, runbook automation, LLM-authored docs with provenance); (2) IMPROVING THE AGENT
ITSELF: what is data (prompts, tool configuration, playbooks, the model pin) versus code (the agent crate: PR only);
prompt and tool-config evolution with guardrails (versioned, reversible, A/B against an evaluation set); fine-tuning on a
Pi (reject with numbers) or on a hub later; evaluation harnesses (an incident corpus, regression suites, scoring);
published evidence on self-improving agent loops and their failure modes (SWE-agent, OpenHands, AutoCodeRover, Voyager's
skill library, "self-refine" and recursive self-improvement claims; incident reports; cite them); (3) REFINING BRIDGING
LOGIC: what Layer 6's bridge exposes as typed settings (edges, filters, fan-out, carriers' modes) and recipes, so "refines"
means proposing typed changes and recipes, never code at runtime; (4) SCOPE: "always scoped to the layers the device
actually operates" - how the device category and the enabled components bound what the agent may propose; (5) the human
gate and the audit trail for every self-change; what the owner must be able to see and revert in one click; (6) the Pi
floor: an evaluation run is a hub task, never resident. Give 2-4 options with picks (likely: data-only self-change with
evaluation gates; code changes only as PRs; KB entries as candidates). Deliver ${OUT}/04-evolution-kb-agent.md.`,
  },
  {
    key: 'ui-floor', model: 'sonnet', effort: 'high',
    prompt: `LENS 5 - THE REPAIRS PAGE, CONSENT LEVELS, ANDROID LIMITS, AND THE PI-FLOOR WEIGHT OF EVERY CANDIDATE.
The dashboard has a Repairs L9 page stub and a working proposals UI with decisions (${ROOT}/4-interface/README.md: read the
proposals and services sections, the per-layer settings pattern and the Layer 6/7 settings rows for shape; "every restart
is a proposal"). Research: (1) THE REPAIRS PAGE: an incident timeline (what happened, evidence, what was tried, outcome),
proposals with their evidence and verify logs, "what changed and why" with one-click undo, the audit trail, honest states
(observing / diagnosing / proposed / applied-on-probation / confirmed / reverted); prior art UIs (systemd/cockpit's
services, Kubernetes dashboards' events, Home Assistant's repairs panel - a GPL reference, Fedora's ABRT, macOS's
diagnostics); (2) CONSENT LEVELS as settings: observe only; propose; auto-apply reversible changes within a scope (which
layers, which action kinds, a daily cap); never code; receive fixes from peers (yes/no; from whom); send validated fixes
through the hub (yes/no); open pull requests from the hub (with whose token); the defaults (owner's "100 % freedom" plus
"human-supervised at the final step"); (3) ANDROID/TERMUX: background limits, the cron placeholder, what an observer and
an applier can do on a phone (no root; Termux:API; notifications), and what the phone delegates to its hub; (4) THE PI FLOOR
for every program the other lenses are likely to propose: opencode on a hub (RSS; a separate component already), monit,
Netdata, Prometheus/alertmanager, Cockpit, git on a hub, llama.cpp runs for diagnosis (CPU seconds and RSS on a Pi 4/5 for
qwen3-4b; cite), an evaluation harness run: published or measurable RSS, binary size, listeners, timers - the one table the
owner needs to say yes or no to anything resident; the agent's own measured cost (111-agent 5.3 MB idle release;
read ${ROOT}/6-transport/TASKS.md Summary for the Pi-floor rows) and what any observer adds; (5) the metered-link rule for
the fix exchange and PRs. Candidates with licences verified from LICENSE files. Deliver ${OUT}/05-ui-consent-floor.md.`,
  },
]

const SCOUT_SCHEMA = {
  type: 'object',
  required: ['lens', 'report_path', 'headline', 'candidates', 'findings', 'recommendation', 'open_questions', 'sources_count', 'unverified'],
  properties: {
    lens: { type: 'string' },
    report_path: { type: 'string' },
    headline: { type: 'string', description: 'one sentence: the recommendation' },
    candidates: {
      type: 'array',
      items: {
        type: 'object',
        required: ['name', 'upstream', 'license', 'license_verified', 'fit', 'why'],
        properties: {
          name: { type: 'string' },
          upstream: { type: 'string' },
          license: { type: 'string', description: 'SPDX id or "custom: <name>"' },
          license_verified: { type: 'boolean' },
          fit: { type: 'string', enum: ['adopt', 'bridge', 'reference', 'reject'] },
          why: { type: 'string' },
        },
      },
    },
    findings: { type: 'array', items: { type: 'string' }, description: 'facts that change the plan, each with a URL or file:line' },
    recommendation: { type: 'string', description: 'the fork options and your pick, at most 120 words' },
    open_questions: { type: 'array', items: { type: 'string' } },
    sources_count: { type: 'integer' },
    unverified: { type: 'array', items: { type: 'string' }, description: 'claims that matter which you could not verify from a reachable source: the lead verifies them elsewhere' },
  },
}

const CRITIC_SCHEMA = {
  type: 'object',
  required: ['verdict', 'gaps', 'conflicts', 'owner_forks'],
  properties: {
    verdict: { type: 'string', description: 'at most 150 words: is the sweep sufficient for an options brief?' },
    gaps: {
      type: 'array',
      items: {
        type: 'object',
        required: ['title', 'severity', 'why', 'scout_prompt'],
        properties: {
          title: { type: 'string' },
          severity: { type: 'string', enum: ['blocking', 'major', 'minor'] },
          why: { type: 'string' },
          scout_prompt: { type: 'string', description: 'a complete, self-contained research prompt to close the gap' },
        },
      },
    },
    conflicts: { type: 'array', items: { type: 'string' }, description: 'claims that disagree across reports, with which report says what' },
    owner_forks: { type: 'array', items: { type: 'string' }, description: 'the genuine forks the owner must decide, one line each' },
  },
}

phase('Scout')
log('Layer 9 research: seams scout + 5 lens scouts on disjoint questions')
const scouts = await parallel(LENSES.map(l => () =>
  agent(CTX + '\n\n' + l.prompt, { label: `scout:${l.key}`, phase: 'Scout', schema: SCOUT_SCHEMA, model: l.model, effort: l.effort })
    .then(r => r && { ...r, key: l.key })
))
const done = scouts.filter(Boolean)
log(`${done.length}/${LENSES.length} scouts returned`)
if (done.length < LENSES.length) log('missing: ' + LENSES.filter(l => !done.find(d => d.key === l.key)).map(l => l.key).join(', '))

phase('Critique')
const criticPrompt = `${CTX}

YOU ARE THE COMPLETENESS CRITIC for the Layer 9 research sweep. Six scouts wrote reports in ${OUT}/ (read every file there
in full: 00-code-seams.md, 01-observe-diagnose.md, 02-change-validate.md, 03-share-fixes-pr.md, 04-evolution-kb-agent.md,
05-ui-consent-floor.md). Their structured summaries: ${JSON.stringify(done.map(d => ({ key: d.key, headline: d.headline, recommendation: d.recommendation, open_questions: d.open_questions, findings: d.findings, unverified: d.unverified })), null, 1)}

Check the sweep against what an OPTIONS BRIEF for the owner needs: README section 9 verbatim (every clause: continuous
observation, diagnosis, a validated corrective change, a pull request with humans as the gate, proven fixes shared through
gateways, the agent / KB / bridging logic evolving, scoped to the layers a device runs), the Operating Principles (servant
and guardian; progressive autonomy, human-supervised at the final step; no telemetry; no policy limits), the recorded
hand-offs (Layer 1's supervision seam and probation, Layer 3's task API and proposals, Layer 4's decisions and the Repairs
stub, Layer 5's PATCH class, Layer 6's applier cut and consent, Layer 7's sealed carriage and the no-git rule, parked P4),
the Pi-floor rule (every resident cost quantified; no new poll without the owner's say), the device floor (a phone cannot
compile; Termux has no cron of ours), D22, "adopt, don't reinvent", and this box's network policy (claims the scouts marked
unverified because a host was denied: collect them into one list the lead verifies on the owner's machine - that is not a
gap to re-run here). Ask: which modality was not run, which claim is unverified where it decides a fork (a licence, an
evaluation result, a weight), which candidate or prior art was missed (name it), which cross-report conflict must be
resolved before the owner can choose (for example what may auto-apply, whether the exchange is v1, who opens PRs), where
did a scout recommend custom code although an adoptable project exists, where did a scout propose something resident or
periodic without a number, and where did a scout treat an unsafe autonomy pattern as settled? You may do targeted web
checks yourself (ToolSearch select:WebSearch,WebFetch; the network note applies) to settle a conflict cheaply, but do not
redo the sweep. Rate each gap blocking (the owner cannot choose without it) / major / minor and write a complete scout
prompt for each blocking or major gap. List the genuine owner forks you see across all reports. Write your critique to
${OUT}/06-critic.md (at most 1500 words; include the unverified-claims list as its own section) and return the structured
verdict.`
const critique = await agent(criticPrompt, { label: 'critic', phase: 'Critique', schema: CRITIC_SCHEMA, model: 'opus', effort: 'xhigh' })

phase('Gap-fill')
let fills = []
if (critique) {
  const blocking = critique.gaps.filter(g => g.severity === 'blocking').slice(0, 2)
  const dropped = critique.gaps.filter(g => g.severity === 'blocking').length - blocking.length
  if (dropped > 0) log(`cap: ${dropped} further blocking gap(s) left for the lead`)
  log(`${blocking.length} blocking gap(s) to fill: ${blocking.map(g => g.title).join(' | ') || 'none'}`)
  fills = (await parallel(blocking.map((g, i) => () =>
    agent(`${CTX}

GAP-FILL SCOUT. The completeness critic found this blocking gap in the Layer 9 sweep (existing reports are in ${OUT}/;
read the one(s) the gap concerns before you start, not all of them):
TITLE: ${g.title}
WHY IT BLOCKS: ${g.why}
TASK: ${g.scout_prompt}
Report path: ${OUT}/07-gap-${i + 1}.md`, { label: `gap:${i + 1}`, phase: 'Gap-fill', schema: SCOUT_SCHEMA, model: 'sonnet', effort: 'high' })
      .then(r => r && { ...r, key: `gap-${i + 1}`, gap: g.title })
  ))).filter(Boolean)
} else {
  log('critic returned nothing - lead reviews reports directly')
}

return {
  scouts: done.map(d => ({ key: d.key, report: d.report_path, headline: d.headline, recommendation: d.recommendation, open_questions: d.open_questions, unverified: d.unverified, candidates: d.candidates.map(c => `${c.name} [${c.license}${c.license_verified ? '' : ' (unverified)'}] ${c.fit}: ${c.why}`) })),
  critic: critique && { verdict: critique.verdict, gaps: critique.gaps.map(g => `${g.severity}: ${g.title}`), conflicts: critique.conflicts, owner_forks: critique.owner_forks },
  gap_fills: fills.map(f => ({ gap: f.gap, report: f.report_path, headline: f.headline, recommendation: f.recommendation })),
}
