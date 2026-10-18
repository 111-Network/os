export const meta = {
  name: 'layer-5-research',
  description: 'Layer 5 (Messaging & Data) research sweep: 5 lens scouts, completeness critic, gap-fill',
  phases: [
    { title: 'Scout', detail: '5 sonnet scouts, one lens each, web-verified, reports on disk' },
    { title: 'Critique', detail: 'opus completeness critic over all reports' },
    { title: 'Gap-fill', detail: 'up to 2 scouts on blocking gaps' },
  ],
}

const ROOT = '/home/kia/projects/111network/os'
const OUT = ROOT + '/plan/layers/50-messaging/research'

const CTX = `
You are a research scout for the 111network OS build (repo root: ${ROOT}). Today is 2026-10-14.

WHAT 111 IS: a minimal operating layer that turns any device (old phone, laptop, Pi, LoRa board) into a node of a resilient
offline mesh. Principle, non-negotiable: ADOPT, DON'T REINVENT. Existing mesh/offline projects are assets we run, bridge and
show; custom code is only glue. A local AI agent is the interface and the self-healer. Read ${ROOT}/README.md fully first
(174 lines): the "5. Messaging & Data" section is the layer you research, "6. Transport & Bridge" + "7. Relay & Presence"
are its neighbours, and the Device Categories (Cat 1 relay … Cat 4 hub, Cat 5 gateway overlay) are the fleet.

WHAT IS ALREADY BUILT (Layers 0–4, locked; do not re-research them, wire to them):
- L0 Knowledge base at ${ROOT}/00-knowledge/ (entries with YAML frontmatter; rubric candidate→testing→proven in
  00-knowledge/README.md; an entry needs: upstream URL, SPDX license, platforms, last_verified, sources). Existing entries you
  may cite: projects/{reticulum,meshtastic,meshcore,briar-bramble,automerge,iroh-nodeid,llama-cpp,opencode,...}.
- L1 Bootstrap: Rust daemon "111d" (crate node111) supervises children; runs Reticulum's "rnsd" as a child from a
  hash-locked Python venv with the config it renders (assets/rns/config.tmpl: AutoInterface, share_instance=no,
  enable_transport=no); netlink-triggered regeneration; "transport_state" = no transport|isolated|joined|degraded.
- L2 Identity (${ROOT}/20-identity/README.md): Ed25519 signing + X25519 ECDH from one seed; NodeId = raw 32-byte Ed25519
  pubkey; wire contract §6: signed announces (137–169 B) distribute keys, TOFU-pinned; routine traffic = per-packet
  ephemeral X25519 + XChaCha20-Poly1305 "sealed packet" (56 B overhead; nonce derived; AD covers cleartext header);
  per-peer 64-bit counters + 32-entry window, no wall clocks. §6.7 + §13 hand to Layer 5: the full envelope, signer_pubkey
  field, delegation trailer (v1 absent), ratchet-field space (size pinned by L5), message-id idempotency, authorship
  (inner signatures, domain-tagged), group/broadcast key distribution. READ §6 and §13 of that file.
- L3 Intelligence: "111-agent" (Rust, :11182) with fixed shell-free tools; llama-server :11181; opencode on hubs.
- L4 Dashboard (${ROOT}/40-interface/README.md §5–§7; contract 40-interface/api/openapi.yaml): React UI embedded in 111d,
  /api/v1 REST + one SSE stream, pairing auth, services registry (agent, model, transport, radio, kb, gateway, codeagent)
  with LIVE/NOT SET/ERROR pages, device registry, a Messages route that is a stub for Layer 5, Nearby stub for Layer 7.
- Layer 6 brief with 12 owner decisions: ${ROOT}/plan/cross-layer/devices.md (read §0 findings, §2 device classes, §7 decisions).
- Decisions log ${ROOT}/plan/DECISIONS.md (D13 = identity wire contract; D17 = checkpoint; D17 #12 = Reticulum rule).

HARD RULES THAT SHAPE EVERY RECOMMENDATION:
1. Reticulum is a CONNECTED SERVICE: run as-is, unmodified, separate process, latest upstream. NO Reticulum code may be
   imported, linked, copied or derived in our tree (its license is custom, non-OSI, with an AI-training clause). Our own
   light mesh core must not depend on it; the Reticulum *protocol design* (public domain, 2016) may inform a clean-room
   Rust implementation. Read ${ROOT}/LICENSING.md lines 20–35. Consequence: anything that needs "import RNS" in our code
   is off the table; talking to rnsd/LXMF through an adopted app's external API or its config is fine.
2. Our identity and sealed-packet construction (L2) are fixed; Layer 5 completes the envelope on top of them. A candidate
   that forces its own identity onto our messages is a bridge target, not the fabric.
3. Offline-first, no wall clock in crypto, no runtime downloads, no telemetry, no policy limits (no rate caps/expiry by
   policy; pruning is capacity-based), reliable delivery first, latency second.
4. Device floor for the fabric: Linux x86_64/aarch64 (musl-static Rust binary) and Android phones via Termux (aarch64,
   no root). Cat 1 ESP32 relays only forward.
5. License gate for adoption: OSI/FSF-free licenses (MIT, Apache-2.0, BSD, MPL-2.0, GPL/AGPL/LGPL acceptable for a separate
   process; GPL linking into our Rust binary needs flagging). CC-NC, non-OSI, source-available-only ⇒ reject as adoption
   (may still be a bridge target or reference). VERIFY the license from the upstream LICENSE file, never from memory.

OWNER DECISIONS FOR THIS CYCLE (2026-10-14), verbatim where quoted:
- Build "Layer 5 with layer 6 in mind!" — the messaging/data design must plug into the device/transport work (LoRa boards
  running RNode/Meshtastic/MeshCore, phone relays over TCP to a hub, BLE later) without redesign.
- Dashboard: "its important that we do ALL the heavy lifting and give users the unified dashboard the literally can host
  any mesh and offline project." Adopted projects are SHOWN inside our dashboard with their own panel; we maintain
  the integration, we never rebuild their UI.
- Meshtastic this cycle: identified (consented probe) + service row with its panel + read-only presence (nodes it hears);
  no message bridge yet.

METHOD (mandatory):
- First call ToolSearch with query "select:WebSearch,WebFetch" to load web tools, then research on the live web. Prefer
  upstream repos, docs, RFCs, release pages. Record the exact URL for every factual claim. Fetch the LICENSE file for every
  candidate you rate "adopt" or "bridge". Note release/commit recency (dates). Mark anything you could not verify as
  (unverified). Your training data may be stale: fetched facts win.
- Read only the repo sections named above (use sed -n / grep); do not slurp whole trees.
- Do NOT write code and do NOT modify any repo file except your own report path.
- Cost discipline: aim for ≤ 40 web fetches; stop when the recommendation is grounded.

REPORT (write it with the Write tool, markdown, ≤ 3500 words, to the path given in your lens; then return the structured
summary). Sections, in order: 1. Summary (≤ 6 bullets) · 2. Candidates table (name · upstream · license (verified?) ·
language/runtime · maturity signals (last release, activity, adopters, audits) · fit: adopt|bridge|reference|reject · why)
· 3. Findings that change the plan (numbered facts with URLs) · 4. Recommendation — where a genuine fork exists, give 2–4
options with one-line trade-offs and your pick · 5. Open questions for the owner · 6. Sources (URL list).
`

const LENSES = [
  {
    key: 'fabric',
    file: '01-fabric.md',
    prompt: `LENS 1 — THE STORE-AND-FORWARD FABRIC BETWEEN 111 NODES.
Question: what carries 111's own sealed envelopes (L2 §6.4) between 111 nodes across heterogeneous transports with
store-and-forward, so the same fabric can carry human messages, agent operations, KB sync, updates and patches?
Evaluate as candidates for the fabric core and rank: (a) a thin custom Rust store-and-forward core (what it would need to
reimplement, honestly); (b) IETF Bundle Protocol v7 (RFC 9171) via existing implementations — dtn7-rs (Rust), µD3TN, ION,
HDTN: licenses, maturity, convergence layers available (TCP, UDP, serial/LoRa?), bundle fragmentation, custody/reliability,
embeddability as a library vs daemon, size on an aarch64 phone; (c) LXMF (Reticulum's messaging format + propagation
nodes, lxmd) — note rule 1: only usable via an external API or as a bridged network, document exactly which external
interfaces exist (lxmd, MeshChat HTTP API, NomadNet) and whether any clean-licensed Rust/Go Reticulum re-implementation
exists (name, license, completeness, activity); (d) adopted P2P messenger cores with headless daemons: Berty/wesh (Go),
qaul.net (Rust), Veilid (Rust), Secure Scuttlebutt stacks, Iroh (iroh, iroh-gossip, iroh-blobs; QUIC-based — does it
work with no internet/relay and over LAN only? over LoRa?), Willow/Earthstar, Delta Chat core (chatmail) — for each:
license, identity model (can our envelope ride as opaque payload without their identity taking over?), transports
(LAN, internet, BLE, serial/LoRa), offline store-and-forward semantics, headless API, maturity.
Also answer: how does each candidate deal with MTU as small as ~200 B (LoRa) — or is LoRa out of scope for it?
Deliver a decision-ready fork: fabric = custom-thin-Rust vs BPv7-via-dtn7-rs vs other, with transport-adapter seams
for Layer 6 (rnsd/RNode, Meshtastic client, TCP/LAN between 111 nodes, BLE later). Report path: ${OUT}/01-fabric.md`,
  },
  {
    key: 'bridges',
    file: '02-bridges-and-budgets.md',
    prompt: `LENS 2 — FOREIGN MESHES AS BRIDGE TARGETS + THE PHYSICAL BUDGETS (Layer 6 in mind).
For each foreign network the owner wants hosted/bridged, document the messaging API facts a thin bridge needs:
(1) Meshtastic: meshtastic-python / protobuf client API over serial/BLE/TCP (and meshtasticd's TCP :4403), text messages,
channels (PSK model), PKI direct messages, max text payload (bytes), node DB / "nodes heard" for READ-ONLY PRESENCE,
MQTT bridging, duty-cycle/airtime limits, what identity appears on the Meshtastic side when a bridge relays, license of
firmware + python lib, and what the hosted "Meshtastic web client" needs (which connection modes work from a plain-HTTP
page: HTTP-to-meshtasticd vs Web Serial/Bluetooth needing a secure context).
(2) MeshCore: companion protocol via meshcore_py / meshcore-cli — contacts, adverts, DMs, channel messages, payload caps,
license (verify: research conflicted GPL-3.0 vs MIT), web client facts.
(3) LXMF/Reticulum networks (Sideband, Columba, MeshChat, NomadNet users): LXMF message format limits, propagation-node
semantics, how an external program sends/receives LXMF WITHOUT importing RNS (MeshChat's HTTP/WebSocket API? lxmd?),
Reticulum MTU (500 B) and link/resource fragmentation behavior.
(4) Briar: is there a headless Linux peer (briar-headless REST API)? Bramble transports (LAN, Bluetooth, Tor), license,
feasibility of a hub acting as a Briar contact; note Android background limits.
(5) Delta Chat / chatmail: self-hosted chatmail relay on a hub, deltachat-rpc-server JSON-RPC, offline/LAN behavior.
(6) nostr: only note what a Layer 7 gateway would need (NIP-17/NIP-59 gift wrap, relay software) — one paragraph.
Then build THE BUDGET TABLE that shapes Layer 5's envelope: per transport (Meshtastic LoRa text, RNode/Reticulum LoRa,
BLE, Wi-Fi/TCP LAN, phone TCP uplink, internet) — payload MTU, typical throughput, latency class, duplex, airtime law
limits; and derive the fragmentation size Layer 5 must assume. Also list what a bridge must show honestly (where the
foreign network's crypto ends and ours begins), and legal/ToS limits on automated relaying per network.
Report path: ${OUT}/02-bridges-and-budgets.md`,
  },
  {
    key: 'panels',
    file: '03-hosted-panels.md',
    prompt: `LENS 3 — ONE UNIFIED DASHBOARD THAT HOSTS ANY PROJECT'S OWN PANEL.
Owner: "we do ALL the heavy lifting and give users the unified dashboard that literally can host any mesh and offline
project." Layer 4 (40-interface/README.md §6.6, §7) today links OUT to a project's UI and bans iframes (D15 reasons:
mixed content, framing headers, secure-context APIs). Research how proven projects host third-party web UIs inside one
dashboard and pin the mechanics: Home Assistant Ingress (supervisor reverse-proxy under HA's origin + iframe panel; auth
session; websocket proxying; path-rewrite rules add-ons must follow), Sandstorm (capability-token iframes, per-app
subdomains), Umbrel / CasaOS / Cosmos Cloud / YunoHost SSOwat / Portainer / Organizr-Homarr-Heimdall (iframe dashboards),
Grafana/Kibana embedding. Pin: sub-path vs sub-domain hosting (apps with absolute URLs break under sub-paths;
"*.111.localhost" resolves to loopback automatically in Chrome/Firefox per RFC 6761 — verify; what works for
111.local over mDNS on LAN where wildcard names do not exist; ports as the fallback), stripping/overriding
X-Frame-Options and CSP frame-ancestors at the proxy, cookie/session scoping, WebSocket/SSE proxying, what breaks in a
frame (Web Serial/Bluetooth/USB need a secure context — document the exact rule and whether http://localhost / *.localhost
count as secure contexts), postMessage bridges for theming/navigation, and a per-app manifest shape (entry path, port,
auth mode, health URL, "needs secure context" flag).
Then inventory the PANELS to host, with license + how they connect: Meshtastic web client (meshtastic/web: self-hostable
static bundle? connection modes: HTTP to meshtasticd / Web Serial / Web Bluetooth), MeshChat (liamcottle/reticulum-meshchat:
license, architecture — Python backend + Vue UI — how it attaches to an existing rnsd shared instance, its HTTP/WebSocket
API, headless mode), NomadNet (TUI only?), Sideband (CC-NC → reject), MeshCore web client, Syncthing GUI (as a data-sync
panel candidate), Delta Chat (any web UI?), Briar (none?), any Reticulum network visualizers (rnstatus web?).
Deliver: a recommended hosting architecture for 111d (Rust, axum/hyper) with the mechanics pinned, an amendment text
proposal for Layer 4 D15 (what changes, what stays), and the panel inventory table. Report path: ${OUT}/03-hosted-panels.md`,
  },
  {
    key: 'envelope',
    file: '04-envelope.md',
    prompt: `LENS 4 — COMPLETING THE MESSAGE ENVELOPE ON TOP OF LAYER 2.
Read 20-identity/README.md §6 (all subsections) and §13 "Layer 5 owns the full envelope" FIRST. Layer 5 must pin, on top of
the fixed sealed-packet construction: (1) message identity + idempotency (content hash vs random id; dedup window sizes
in Briar/Bramble, LXMF, Meshtastic, BPv7); (2) fragmentation/reassembly over ~200–500 B MTUs — compare how LXMF,
Reticulum Link/Resource, Meshtastic, Bundle Protocol v7 (RFC 9171 fragmentation), Bramble (BSP/BTP), MeshCore do it;
whether erasure coding (RaptorQ/fountain codes, e.g. the "raptorq" Rust crate) is worth it over lossy LoRa links;
(3) delivery receipts / ACK proofs — L2 requires authenticated ACKs with their own domain tag (Meshtastic's DM-ack
forgery CVE is the cautionary tale; Reticulum delivery proofs are signatures over the packet hash) — propose the construction;
(4) group/broadcast messaging without static-static ECDH: sender keys sealed per member vs MLS (RFC 9420; openmls Rust
crate: license, audit status, suitability for offline/partition-tolerant small groups without a delivery service) vs a
Meshtastic-style channel PSK (cautionary); (5) authorship / non-repudiation inner signature (when, cost: 64 B);
(6) the ratchet-field reservation — what size L5 should pin (Reticulum's per-destination ratchet announces a 32-B X25519
key; Signal-style needs more) and whether a simple per-conversation ratchet is viable offline (recall D13 rejected
Signal-class prekey servers); (7) wire encoding: hand-packed fixed-offset binary (as L2's announce) vs CBOR (RFC 8949)
with COSE (RFC 9052/9053: COSE_Encrypt0/Sign1 with EdDSA + ChaCha20/Poly1305 — could the envelope BE COSE, what does it
cost in bytes?) vs protobuf vs postcard; (8) time without trusted clocks — how Briar, LXMF, Meshtastic present message
times and order threads when nodes have no clock (L2 forbids wall clocks in crypto; display is different);
(9) traffic classes: how to mark human message vs agent op vs KB chunk vs update chunk vs patch so relays/pruning can
prioritize without reading content (header class field vs separate destinations); (10) what a Cat-1 ESP32 relay must
parse to forward (keep it tiny).
For each item: strongest prior art with URL, byte cost, and your pinned recommendation with 2–3 options where a true
fork exists. Report path: ${OUT}/04-envelope.md`,
  },
  {
    key: 'data',
    file: '05-data-store-and-bulk.md',
    prompt: `LENS 5 — THE DATA HALF: ENCRYPTED LOCAL STORE, CAPACITY PRUNING, BULK PAYLOADS.
(1) Encrypted-at-rest message/data store for a musl-static Rust daemon on Linux + Android/Termux: SQLCipher via rusqlite
("bundled-sqlcipher" / "bundled-sqlcipher-vendored-openssl": license, static-build viability, binary size, maintenance),
SQLite + per-record AEAD (key via HKDF from the node seed — check L2's reserved BIP-85 subtree note in 20-identity/README.md
§15) using redb or sled or fjall, "age"-style file encryption for blobs; how Briar, Delta Chat (its Rust core), Signal
desktop, Sideband/LXMF store messages at rest; key-at-rest custody (node keyset vs OS keystore vs TPM per L2 §5).
(2) Capacity-based pruning: patterns from LXMF propagation nodes (storage limits, priority), Briar, Bundle Protocol
storage, Syncthing; what metadata pruning needs (class, size, age-as-hint, delivered flag) with NO policy expiry.
(3) Bulk payload distribution over a delay-tolerant fabric: KB sync (git bundles over sneakernet/DTN as a proven pattern;
rsync-like deltas), software updates (signed manifest + chunked blobs — Layer 1 already has minisign + A/B), patches;
content-addressed verified streaming (BLAKE3 + bao; iroh-blobs — does it require iroh QUIC connections?; Reticulum
"rncp"/Resource transfers as reference; BPv7 bundle fragmentation; Syncthing block exchange). Which can run over ~200 B
MTU LoRa at all, and which are LAN/TCP-only? (4) Sync of shared state: Automerge (already in KB) / Loro / Yjs for the
"lightweight append-only shared log" the README reserves for Layer 8 — only note the seam, do not design Layer 8.
(5) Multi-device: a phone node (Termux) and a hub owned by one person — do messages sync between them (and how do
adopted projects do this: Delta Chat multi-device, Briar none, LXMF propagation)?
Deliver pinned recommendations with byte/size costs, licenses (verified), build-viability notes for musl-static and
Termux, and the fork options for the owner. Report path: ${OUT}/05-data-store-and-bulk.md`,
  },
]

const SCOUT_SCHEMA = {
  type: 'object',
  required: ['lens', 'report_path', 'headline', 'candidates', 'findings', 'recommendation', 'open_questions', 'sources_count'],
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
    findings: { type: 'array', items: { type: 'string' }, description: 'facts that change the plan, each with a URL' },
    recommendation: { type: 'string', description: 'the fork options and your pick, ≤ 120 words' },
    open_questions: { type: 'array', items: { type: 'string' } },
    sources_count: { type: 'integer' },
  },
}

const CRITIC_SCHEMA = {
  type: 'object',
  required: ['verdict', 'gaps', 'conflicts'],
  properties: {
    verdict: { type: 'string', description: '≤ 150 words: is the sweep sufficient for an options brief?' },
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
  },
}

phase('Scout')
log('Layer 5 research: 5 scouts on disjoint lenses')
const scouts = await parallel(LENSES.map(l => () =>
  agent(CTX + '\n\n' + l.prompt, { label: `scout:${l.key}`, phase: 'Scout', schema: SCOUT_SCHEMA, model: 'sonnet', effort: 'high' })
    .then(r => r && { ...r, key: l.key })
))
const done = scouts.filter(Boolean)
log(`${done.length}/${LENSES.length} scouts returned`)
if (done.length < LENSES.length) log('missing: ' + LENSES.filter(l => !done.find(d => d.key === l.key)).map(l => l.key).join(', '))

phase('Critique')
const criticPrompt = `${CTX}

YOU ARE THE COMPLETENESS CRITIC for the Layer 5 research sweep. Five scouts wrote reports in ${OUT}/ (read every file
there in full: 01-fabric.md, 02-bridges-and-budgets.md, 03-hosted-panels.md, 04-envelope.md, 05-data-store-and-bulk.md).
Their structured summaries: ${JSON.stringify(done.map(d => ({ key: d.key, headline: d.headline, recommendation: d.recommendation, open_questions: d.open_questions, findings: d.findings })), null, 1)}

Check the sweep against what an OPTIONS BRIEF for the owner needs: every requirement in README.md §5 (E2E signed payloads,
reliable delivery first, store-and-forward across heterogeneous transports, encrypted at-rest store, no runtime policies,
capacity pruning, one fabric for human messages + agent ops + KB sync + updates + patches), the Layer 2 §13 hand-offs,
the Layer 3 offload seam ("traffic rides Layer 5's encrypted fabric"), the Layer 4 contract (Messages route, services
registry, hosted panels), the Layer 6 brief (cross-layer/devices.md: RNode/Meshtastic/MeshCore boards, phone TCP relay, BLE later),
the owner's three decisions for this cycle, and the Reticulum rule. Ask: which modality was not run, which claim is
unverified where it matters (a license, an API that may not exist, a size that decides a fork), which candidate was
missed (name it), which cross-report conflict must be resolved before the owner can choose? Also check the sweep
against the "adopt, don't reinvent" rule: did any scout recommend custom code where a candidate exists that they did not
evaluate? You may do targeted web checks yourself (ToolSearch select:WebSearch,WebFetch) to settle a conflict cheaply,
but do not redo the sweep. Rate each gap blocking (the owner cannot choose without it) / major / minor and write a
complete scout prompt for each blocking or major gap. Write your critique to ${OUT}/06-critic.md (≤ 1500 words) and
return the structured verdict.`
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

GAP-FILL SCOUT. The completeness critic found this blocking gap in the Layer 5 sweep (existing reports are in ${OUT}/;
read the one(s) the gap concerns before you start, not all of them):
TITLE: ${g.title}
WHY IT BLOCKS: ${g.why}
TASK: ${g.scout_prompt}
Report path: ${OUT}/07-gap-${i + 1}.md`, { label: `gap:${i + 1}`, phase: 'Gap-fill', schema: SCOUT_SCHEMA, model: 'sonnet', effort: 'high' })
      .then(r => r && { ...r, key: `gap-${i + 1}`, gap: g.title })
  ))).filter(Boolean)
} else {
  log('critic returned nothing — lead reviews reports directly')
}

return {
  scouts: done.map(d => ({ key: d.key, report: d.report_path, headline: d.headline, recommendation: d.recommendation, open_questions: d.open_questions, candidates: d.candidates.map(c => `${c.name} [${c.license}${c.license_verified ? '' : '?'}] ${c.fit}`), sources: d.sources_count })),
  critic: critique && { verdict: critique.verdict, gaps: critique.gaps.map(g => `${g.severity}: ${g.title}`), conflicts: critique.conflicts },
  gap_fills: fills.map(f => ({ gap: f.gap, report: f.report_path, headline: f.headline, recommendation: f.recommendation })),
}