export const meta = {
  name: 'layer-7-spec',
  description: 'Layer 7 (Relay & Presence): one opus drafter writes 7-relay/README.md v0.1 from D30, the brief, the research and the as-built code; three fable judges attack it (custody, carriers, integration) and write plan/layers/7-relay/review/<lens>.md',
  phases: [
    { title: 'Draft', detail: 'one opus spec author (xhigh): 7-relay/README.md v0.1', model: 'opus' },
    { title: 'Panel', detail: 'three fable judges in parallel, read-only, one report file each', model: 'fable' },
  ],
}

const ROOT = '/home/user/os'
const a = args || {}

const DRAFT_PROMPT = `
You are the spec author for the 111network OS, Layer 7 (Relay & Presence). Today is 2026-11-01. Repo root: ${ROOT}, branch
claude/layer-6-implementation-006mte. You WRITE exactly one file: ${ROOT}/7-relay/README.md (create the folder). You edit nothing
else, run no git write commands, start no process, run no cargo build or test; read-only greps, 'git log' and 'cargo metadata' reads
are fine.

THE LAW, in order:
(1) ${ROOT}/plan/DECISIONS.md D30 (the owner's decisions: every pick and every default answer; never quote the owner's words as
    yours), D29 and D24-D27 (the Layer 5 and 6 lock precedents and their amendment style).
(2) ${ROOT}/plan/layers/7-relay/options.md (the brief: section 0 facts, section 2 forks with picks, section 3 detail picks, section 4
    plan corrections, section 5 answered questions, section 6).
(3) The research reports ${ROOT}/plan/layers/7-relay/research/00-code-seams.md ... 07-gap-2.md (evidence with file:line; 00 is the
    line-verified seam map; 07-gap-2 sections 2-3 are the custody design and the twelve Layer 5 amendments; 07-gap-1 the carrier and
    link-security facts; 06-critic section 6 lists claims nobody could verify from this box: mark those "(unverified from this box)").
(4) The as-built code. Every claim about built code must be verified by you reading the file and cited file:line: mesh111
    (${ROOT}/5-messaging/crates/mesh111/src/{fabric.rs,mailbox.rs,envelope.rs,frag.rs,outbox.rs,store/*,link/*}), node111
    (${ROOT}/1-bootstrap/crates/node111/src/daemon/{fabric,dashboard,components,transport,devices,bridge,panels,intelligence}/,
    the updater in src/{update_guard.rs,update_helper.rs,manifest.rs,minisign.rs,floor.rs,cli.rs,install/}), the agent crate under
    ${ROOT}/3-intelligence/, the UI ${ROOT}/4-interface/ui/src, the contract ${ROOT}/4-interface/api/openapi.yaml.
(5) The locked specs you amend or depend on: ${ROOT}/5-messaging/README.md (section 3 frames and TLVs, 6.2 evidence, 7.3 enrol and
    pairing, 9 custody, 10.3-10.4 schema and prune, 12-13 listeners, 21 A+), ${ROOT}/2-identity/README.md (section 6 link keys, 10
    attestation record, 13 mesh epoch), ${ROOT}/1-bootstrap/README.md (Updates & rollback, Security & trust model, Seams for later
    layers), ${ROOT}/3-intelligence/README.md (registry mentions, agent tools), ${ROOT}/4-interface/README.md and
    ${ROOT}/4-interface/api/README.md (contract conventions), ${ROOT}/README.md section 7 and "Category 5", and
    ${ROOT}/6-transport/README.md, which is THE STRUCTURE AND VOICE MODEL: read it fully first; its 2.1 component seam, 15 security,
    16 verification, 17 deferred, 18 seams and amendments and 19 plan corrections are your templates.
(6) ${ROOT}/AGENTS.md rules: the Raspberry Pi floor, adopt before you build, Linux hygiene, no model names in files.

WHAT TO WRITE: ${ROOT}/7-relay/README.md, titled "Layer 7 - Relay & Presence: spec v0.1 (draft for the adversarial panel)",
800-1100 lines, normative where it says so, in the voice of 6-transport/README.md: short declarative sentences, numbered rules,
byte tables, "MUST" and "never" only where a rule is hard, every built-code fact cited file:line, every number sourced to a report
section or a file. Sections, in this order, with these numbers:

0 Reading guide: what is normative and what is informative; the v1 / v1.1 split in one table; how this spec relates to D30 and the brief.
1 Scope and non-goals: what Layer 7 is on each device category (Cat 1 = the stock repeater's own floods, nothing of ours runs there);
  the Pi floor as a hard rule (nothing resident, no timer, no poll, no listener, no dependency added to the required configuration;
  every hub-only cost enumerated); non-goals (a global 111 swarm, a routing protocol, a relay firmware, any contact with 111's own
  servers for a liveness fact).
2 Presence (D30 F1): the evidence model (a local monotone sequence plus an in-session age per source and transport; never a peer's
  clock); what bumps last-seen; the '111' Nearby kind and its evidence strings; the per-transport heartbeat component switch
  (interval, 0 = events only, defaults per transport, the bans: duty-cycle-limited and amateur bands, phones never under 9 min);
  announce rules unchanged (L5 A6); the airtime facts as the stated reason.
3 The gateway role and the carrier test (D30 F2): 5A and 5B defined (hub-only in v1); "internet" = the chosen carrier endpoint is
  reachable; the netlink address and route groups (the Layer 6 mask change, file:line); the Termux fallback; the removal of the 60 s
  HEAD and what the 'gateway' service row and 'wan_ok' become; the timer-baseline trim (which ticks, under what condition); the
  no-timer gate added to G7 (what it measures and how, a method the lead can implement: for example voluntary context switches from
  /proc/<pid>/task/*/stat over 60 s idle plus the count of outbound connections; state your method and its pass line); the four
  gateway badge states.
4 Hub-to-hub over the internet: TLS 1.3 raw public keys (D30 F3): which links MUST be wrapped (non-LAN hub-to-hub); the link key
  (a fresh Ed25519 key per node, random, never seed-derived, never the identity key; where it is stored; how the hub card carries its
  SPKI; the TOFU pin; the later attestation reserved as a named L2 registry entry); the rustls facts (cite the registry source
  file:line as 07-gap-1 does, with versions); tokio-rustls as the one new lockfile entry and its LICENSING.md row; the pairing flow
  amendment (L5 7.3: two enrolments both ways; the 'paired_hubs' setting shape; the warning shown before TLS-RPK ships); what the path
  and the far hub see (a bytes table).
5 Cross-hub custody: single-copy push hand-over (D30 F4, F5): the 'fwd' table (four rows); the widened gate in words and the twelve
  Layer 5 amendments reproduced from 07-gap-2 section 3 as a numbered table (where / change / additive?) with the exact replacement
  text for each L5 sentence that changes; the hint TLV 0x02 (bytes, placement, old-hub behaviour); discovery without a hint (probe
  order, cache size); evidence ('relayed' with reason 'forwarding' at the first hub; nothing later; 'delivered' only by the
  recipient's ACK); loop and duplicate freedom (pkt_ref kept after hand-over, the fwd rules); quotas ('Class::Transit' at one quarter
  of the hub custody quota: state the byte figure from L5's quota and cite it); eviction (protected while committed = 0, prune step 1
  after); the forwarder (event-driven; its triggers; back-off = the existing redial); the hand-off API (a Rust trait with
  'forward_take', 'forward_done(Taken | Rejected | Retry)', 'carrier_inbound' and the forward-pending / target-up event; in-process in
  v1; the JSON-lines pipe form for external carrier programs is v1.1); byte costs per message (the 07-gap-2 table); the resident cost
  per paired hub; "nothing on a non-hub node" and how that is proven (the holds_custody() gate, file:line).
6 Carriers: the carrier contract and the ntfy carrier prototype (D30 F3 P): the contract (opaque sealed batches, never re-fragmented,
  the batching rule, hand-off instead of a Link); the ntfy carrier (POST and GET shapes, the 'since=<id>' drain, the body limit and the
  batch size rule, the topic derived from a pairing secret under a new L2 domain tag that you define, the retention facts and the
  recommended self-host config shipped as a documented file, the rate-limit facts, 5A flush on events, 5B one held long-poll
  connection, what the operator sees and the consent copy verbatim); the component switch and settings ('carrier.kind = "ntfy"', the
  server URL, the topic secret, the 5A/5B mode; every default off; byte-identical when off); the KB entry and the smoke-log rule (what
  the prototype must log so the KB entry can move from candidate to testing); chatmail as the alternative carrier (what Layer 6 already
  has; what glue, if any); the reach layer (a public address; yggstack in v1.1 once tagged; a tunnel documented); what Nostr, LXMF
  propagation and iroh would need (v1.1, one paragraph).
7 Service Discovery Registry and gateway advertisement (D30 F7): the three layers with exact shapes (the DNS-SD record: instance, SRV,
  TXT keys and sizes; the appdata role bits and TLVs: which bits, consent gates, the counter cost; the sealed query: a new L5 kind or an
  existing request, decide and justify, additive); what is never advertised; members-only gateway advertisement (how it rides the hub
  offer, INTRO and cards); registry rows say which layer answered (the UI labels); lost-hub recovery (retry the last address and the
  mDNS name, then a QR rescan; no sweep); Termux limits.
8 The mesh epoch (D30 F8): the beacon bytes ("111/epoch/v1" || issuer || epoch, a u64, the unit = day, the floor = the manifest's
  epoch_floor); what advances a hub's counter (persisted powered-on monotonic time divided by 86400, monotone, never a wall clock);
  carriage (ENROLL_OK, the mailbox handshake, cards; the TLV); adoption (the maximum from the pinned home hub); v1 "not evaluated" with
  the first consumer named (freeze protection for updates); threat notes (a frozen hub, a compromised hub, the manifest floor); the L2
  section 13 hand-off closed; the distinction from L5 section 4's epoch.
9 Updates and the KB from a hub (D30 F6): the origin file set and its URL shape on the dashboard listener while hub_role is on
  (read-only, unauthenticated, the /health posture, no third port); '111 update --from <dir|url>'; the origin order; the check on
  link-up (no timer); the KB in the payload; the Layer 1 trust amendments as exact rules (the channel check; the keychain walk from the
  compiled key; min-upgrade-from in the root helper; the freeze stance; the new verb's staging write and update-request.json);
  the transient RSS note; LoRa carries only "a release exists"; the Layer 8 note.
10 Nearby radar (D30 F9): the SVG radar rules (rings by hop count, SNR order within a ring, the angle rule, uncertainty cells, the
  unplaced ring, the legend text); evidence labels; positions never shared by default; the hosted Meshtastic panel's map link; the map
  bucket as a recorded D21 amendment for Layer 8 (exempt bucket, a Range tile route), v1.1.
11 Public signed frames (D30 F11): the BROADCAST 0x30 frame bytes (publisher, signature, the pointer form on the 160 B floor; the
  object by Tier 2); the attestation gossip rule (the L2 section 10 record rides cards, INTRO and the hub handshake, never a LoRa
  announce); the UI label for an attested transport identity; consumers v1.1.
12 UI, settings, consent, metered and licensed links: the Layer 7 settings page (named by the Layer 6 precedent); the gateway badge;
  the per-transport presence switch; the pairing flow; the carrier consent copy verbatim per carrier; the metered-link setting (a
  daily byte cap; the SSID as a user-marked hint); the licensed-board hard block; what the agent may propose.
13 API contract additions and agent tools (additive; the binding form is 4-interface/api/openapi.yaml at build time): every new route,
  field, enum value and SSE event with its schema shape as JSON examples; the reason code 'forwarding'; the Nearby '111' kind; the
  gateway row redefinition; agent tools ('presence.read', 'gateway.read', 'nearby.read' extended; proposals 'gateway.enable',
  'hub.pair', 'presence.interval', 'carrier.flush'; pins bumped; read-only versus proposal rules; the private-IP-literal rule for
  offload).
14 Security, privacy and legal (threat model; hard rules): per carrier and per link what is exposed to whom (tables); the hard rules
  (never proxy inference or a WAN request for a peer; never forward anything unsealed except public signed frames the user follows;
  never into a licensed or amateur link; never on a metered link past the cap; no contact with 111's servers for liveness; topic
  secrecy; the link-key rules); the attack cases (a forged hint, custody starvation, topic squatting, a malicious paired hub, replayed
  announces, a frozen or compromised hub for the epoch, a stale-hub freeze for updates) each with its defence.
15 Verification (lock gates; netns, no hardware): the Layer 7 gates in 7-relay/verify/: G8 presence and registry; G9 cross-hub forward
  (three hubs; the sequences of 07-gap-2 section 2 as assertions; loop and quota cases); G10 carrier (a stub of the ntfy API in the
  harness by default, a pinned real ntfy release when an env var asks; the smoke-log output); G11 TLS-RPK (the handshake, a wrong-key
  refusal, an optional OpenSSL interop); G12 hub-as-origin updates and the Layer 1 trust fixes; G7 extended with the no-timer gate;
  each with its assertion list and its pass line; the CI mapping (which job in .github/workflows/ci.yml); the known root-only tests rule.
16 Pi-floor accounting: a table of every resident cost by role (a non-hub node: 0 for every item; a hub: per paired hub, per carrier
  mode; a LAN-sharing hub: the mdns record), measured versus estimated, and the rule that the build records measurements in TASKS.md.
17 Deferred / v1.1: the list from D30 F10, each with what it needs.
18 Cross-layer seams and amendments: per layer (L1 ... L6) the exact additive amendments this spec requires, in the style of
  6-transport/README.md section 18 (L5 = the twelve; L6 = the netlink mask, the probe removal, the mdns check, the '[order]' reading,
  receipt.gateway; L2 = the link-key transport_id reservation, the epoch unit, the attestation carriage; L1 = the trust fixes, the
  update verb and the origin list; L3 = the registry answers, the README line-48 correction, the agent tools; L4 = the contract
  additions and the D21 map amendment).
19 Plan corrections recorded: the ten of the brief section 4, verbatim where they touch locked text.
Changelog v0.1: one line.

HARD RULES FOR THE TEXT: no model names or identifiers anywhere in the file (write "the lead", "the panel", "a scout"); no attribution
lines; a file:line citation for every claim about built code, each verified by reading; where the brief or a report is wrong against
the code, follow the code and record the correction in section 19; never invent a number: mark an unknown "(measured at build)";
every default off, so a node with nothing configured is byte-identical to today; every new setting named with its default; every new
wire byte accounted in a table; the twelve Layer 5 amendments are the only Layer 5 changes; v1 adds no process, listener, timer or
poll on a non-hub node and only the enumerated held connections on a hub.

RETURN (structured): status (written | partial | blocked), file, lines, sections (the list you wrote), verified_claims (the count of
file:line citations you verified by reading), deviations (where you departed from D30 or the brief and why, one line each),
open_questions (what only the owner can answer, one line each; expect few), notes (at most 12 lines for the lead).
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

const JUDGE_CTX = `
You are a READ-ONLY review judge for the 111network OS, Layer 7 (Relay & Presence) spec draft v0.1 at ${ROOT}/7-relay/README.md.
Today is 2026-11-01. Repo root ${ROOT}. You edit nothing except the one report file named below; you run no git write commands, no
cargo build or test, and start no process. The law you judge against: ${ROOT}/plan/DECISIONS.md D30 (the owner's decisions; a spec
that contradicts D30 is wrong unless the code makes D30 impossible, which you then say), the brief ${ROOT}/plan/layers/7-relay/options.md,
the research reports ${ROOT}/plan/layers/7-relay/research/*.md (00 is the line-verified seam map; 07-gap-2 the custody design and the
twelve amendments; 07-gap-1 the carrier and link-security facts), the locked specs the draft amends (${ROOT}/5-messaging/README.md,
${ROOT}/2-identity/README.md, ${ROOT}/1-bootstrap/README.md, ${ROOT}/6-transport/README.md, ${ROOT}/4-interface/api/openapi.yaml) and the
as-built code: verify the draft's file:line claims by reading; a wrong citation is a finding. ${ROOT}/AGENTS.md rules apply (the
Raspberry Pi floor; adopt before you build; no model names in files).

METHOD: read the whole draft once, then your lens's sections closely, then verify. Attack it: what is unbuildable, contradictory,
underspecified (a builder would have to guess), unsafe (a loop, a leak, a starvation, a wall clock used for ordering or crypto, an
untrusted string rendered as instructions, a phone-home), heavier than the Pi floor allows (any new resident process, timer, poll,
listener or dependency on a non-hub node; an unmeasured hub cost stated as a fact), wider than D30 (anything D30 deferred to v1.1
that the draft puts in v1; a Layer 5 change beyond the twelve), or missing (a D30 decision the draft does not carry; a seam the code
has that the draft contradicts). Severity: blocker = the spec cannot lock without it (wrong against the code or D30, unsafe,
unbuildable); major = a builder would build the wrong thing or an owner promise is broken; minor = wording, a missing citation, a
nit. Be specific: the draft's line and the code's file:line; one fix per finding; no finding without a fix.

WRITE your report as Markdown to ${ROOT}/plan/layers/7-relay/review/LENS.md (the folder exists): a verdict line (lock |
amend-then-lock | redesign), then tables Blockers / Majors / Minors with columns id (the lens letter plus a number), where, what, fix,
then at most 15 lines of notes (what is good; what to watch in the build). Then RETURN the structured result with the same findings.
`

const LENSES = [
  {
    key: 'custody', letter: 'C',
    prompt: 'LENS = custody (protocol and custody). Your sections: 5 (cross-hub custody and the twelve Layer 5 amendments: check each against the mesh111 code and the locked L5 text, bytes, loop freedom, quotas, eviction, the hint TLV, evidence honesty), 4 as it touches L5 7.3 pairing, 8 (the epoch beacon carriage and bytes), 11 (the frame formats), the reason code in 13, and the custody and forwarding attack cases of 14. Verify every mesh111 file:line the draft cites. Hunt for: a loop the fwd rules allow, a way a stranger starves member custody, a privacy leak through the hint or the probe, a sentence that contradicts locked Layer 5 text outside the twelve, a byte count that is wrong, an evidence state that is not honest.',
  },
  {
    key: 'carriers', letter: 'S',
    prompt: 'LENS = carriers (systems and the floor). Your sections: 3 (the gateway test, the netlink groups, the timer trim, the no-timer gate method), 6 (the carrier contract and the ntfy carrier: limits, retention, the topic derivation, consent copy, the KB smoke-log rule, the component switch), 4 (the rustls and tokio-rustls facts, the link-key rule, the LICENSING row), 9 (the hub as update origin and the Layer 1 trust amendments against the updater code), 12 (consent copy), 16 (the Pi-floor accounting). Verify every node111 and updater file:line the draft cites and the rustls facts in the local registry source. Hunt for: any resident weight on a non-hub node, any phone-home, an API claim about ntfy or rustls that the sources do not support, a component-seam misuse (Layer 6 section 2.1), a licensing row missing, a setting without a default, a 5B cost stated as measured when it is not.',
  },
  {
    key: 'integration', letter: 'I',
    prompt: 'LENS = integration (seams, buildability, verification). Your sections: 0-2 (the reading guide, scope, presence and the 111 Nearby kind), 7 (the registry shapes and the sealed query), 10 (the radar), 13 (every API addition against the conventions of 4-interface/api/README.md and the existing openapi.yaml: additive, pinned, problem+json; the agent tools against 3-intelligence conventions), 15 (every gate: netns-provable without hardware? assertions concrete? a pass line? the CI mapping real?), 17-19 (completeness against D30 F10 and the brief section 4; every cross-layer amendment list complete and additive; every D30 decision carried somewhere). Hunt for: an item a builder could not start from (no file, no criterion, no check), a gate that cannot run in a namespace, a contract addition that breaks a pinned shape, a D30 decision the draft dropped or widened, a v1.1 item that leaked into v1.',
  },
]

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

let draft = null
if (!a.panelOnly) {
  phase('Draft')
  log('Layer 7 spec v0.1: one opus drafter writes 7-relay/README.md')
  draft = await agent(DRAFT_PROMPT, { label: 'spec:draft', phase: 'Draft', model: 'opus', effort: 'xhigh', schema: DRAFT_SCHEMA })
  if (!draft || draft.status === 'blocked') return { draft, panel: [] }
  log(`draft: ${draft.status}, ${draft.lines} lines, ${draft.verified_claims} verified citations`)
}

phase('Panel')
log('Layer 7 panel: three fable judges (custody, carriers, integration)')
const panel = (await parallel(LENSES.map(l => () =>
  agent(JUDGE_CTX.replace('LENS.md', l.key + '.md') + '\n\n' + l.prompt + ' Your ids start with ' + l.letter + '.',
    { label: `judge:${l.key}`, phase: 'Panel', model: 'fable', effort: 'high', schema: JUDGE_SCHEMA })
    .then(r => r && { ...r, lens: l.key })
))).filter(Boolean)
for (const p of panel) {
  const n = s => p.findings.filter(f => f.severity === s).length
  log(`${p.lens}: ${p.verdict} - ${n('blocker')} blockers, ${n('major')} majors, ${n('minor')} minors`)
}
return { draft, panel }
