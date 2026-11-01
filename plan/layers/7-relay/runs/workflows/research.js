export const meta = {
  name: 'layer-7-research',
  description: 'Layer 7 (Relay & Presence) research sweep: as-built seams scout + 5 lens scouts, completeness critic, gap-fill',
  phases: [
    { title: 'Scout', detail: '1 opus seams scout + 5 sonnet lens scouts, web-verified where the box allows, reports on disk' },
    { title: 'Critique', detail: 'opus completeness critic over all reports' },
    { title: 'Gap-fill', detail: 'up to 2 scouts on blocking gaps' },
  ],
}

const ROOT = '/home/user/os'
const OUT = ROOT + '/plan/layers/7-relay/research'

const CTX = `
You are a research scout for the 111network OS build (repo root: ${ROOT}). Today is 2026-11-01.

WHAT 111 IS: a minimal operating layer that turns any device (old phone, laptop, Pi, LoRa board) into a node of a resilient
offline mesh. Principle, non-negotiable: ADOPT, DON'T REINVENT. Existing mesh/offline projects are assets we run, bridge and
show; custom code is only glue. A local AI agent is the interface and the self-healer. Read ${ROOT}/README.md first: the
section "7. Relay & Presence" is the layer you research — verbatim: "Nodes continuously announce reachability through
heartbeats. Nodes that possess internet connectivity advertise that fact and become gateways. Gateways synchronize the
offline local mesh state with the global network through an asynchronous store-and-forward transport adopted via the
Knowledge Layer: intermittent gateways flush pending queues whenever connectivity appears, while permanent gateways stream
global broadcasts and encrypted messages into the local mesh in real time. When a node loses connectivity it continues to
serve as a pure mesh participant. Delivery paths form dynamically from the combination of local mesh links and
internet-capable gateways." Also read "Category 5: Global Gateways" (an overlay role, not a hardware tier; 5A intermittent,
5B permanent), "Category 1: Edge Relays", the layer ↔ category table (Layer 7 runs natively on EVERY category, Cat 1
included) and "Intent-Based Routing" (the Service Discovery Registry nearby nodes broadcast). "6. Transport & Bridge" is
the neighbour below (built), "8. Storage & Shared State" the neighbour above (not started).

WHAT IS ALREADY BUILT (Layers 0–6; do not re-research them, wire to them; cite file:line when you point at code):
- L0 Knowledge base ${ROOT}/0-knowledge/ (entries with YAML frontmatter; rubric candidate→testing→proven in its README;
  navigation contract ${ROOT}/0-knowledge/AGENTS.md). Entries you may cite and must not contradict without a fresher source:
  concepts/{preferred-networks,store-and-forward-states,lora-airtime-budget,secure-contexts}, protocols/{reticulum-protocol,
  nostr,bundle-protocol-v7,noise-framework}, projects/{reticulum,lxmf,meshtastic,meshcore,meshmud,meshmonitor,meshchatx,
  dtn7-rs,briar-bramble,briar-mailbox,iroh-nodeid,automerge,deltachat-rpc-server,madmail,simplex-chat,blake3-bao,
  delta-chat-securejoin,home-assistant-ingress}.
- L1 Bootstrap (${ROOT}/1-bootstrap/README.md): the Rust daemon "111d" (crate node111 at ${ROOT}/1-bootstrap/crates/node111)
  supervises children; the installer categorizes the device and RECORDS gateway capability ("overlay: internet up at detect
  time; re-evaluated by later layers" — that re-evaluation is yours); the updater fetches a minisign-signed manifest over the
  internet only ("mesh gossip arrives with Layer 7"; "KB git-sync is Layer 7, v1: KB inside the release payload"; "manifest
  gossip" handed to Layer 7); runs Reticulum's "rnsd" as a child from a hash-locked venv; Android phones join via Termux (no
  root, no multicast receive, no BLE).
- L2 Identity (${ROOT}/2-identity/README.md): Ed25519 + X25519 from one seed; NodeId = raw 32-byte Ed25519 pubkey; the
  announce format (§7; v1 emits announces ON DEMAND only — in a card, on link-up, inside an INTRO — never periodic); §10
  transport attestations (built in Layer 6 as bridge/attest.rs; transport_id 0x01 = nostr-secp256k1 is RESERVED, which
  tells you the identity layer already anticipated nostr as a global transport); §13: the MESH-EPOCH SOURCE that cert and
  attestation expiries count in is Layer 7's to define (v1: undefined ⇒ expiry never evaluated); the open questions handed
  to L6/7 (announce-as-routing vs identity-only; Cat-1 announce-verification policy; announce rate-limit policy).
- L3 Intelligence (${ROOT}/3-intelligence/README.md): "111-agent" with fixed tools (nearby.read, fabric.read, messages.*);
  offload to a nearby hub "discovered via the Service Discovery Registry (Layer 7 broadcasts hub role, model, load)" — v1
  offload stays on the LAN binding; a hub never proxies inference to the WAN.
- L4 Dashboard (${ROOT}/4-interface/README.md; contract ${ROOT}/4-interface/api/openapi.yaml): React UI embedded in 111d,
  /api/v1 + one SSE stream; §6.4 Nearby is "a structural stub, Layer 7": the real per-source list landed with L5/L6
  ("heard via Meshtastic on <board>", Reticulum heards), but "radar and map views arrive with Layer 7"; peers/transports/
  gateway detail panels are L6/L7; UI bundle budget D21 (256 KB eager + lazy buckets; \`pnpm budget\`).
- L5 Messaging & Data (${ROOT}/5-messaging/README.md, locked, built): the fabric "mesh111" (${ROOT}/5-messaging/crates/mesh111)
  inside 111d: sealed envelopes, msg ids, fragmentation F = 160 B, authenticated ACKs + custody receipts, contacts/cards,
  the HOME-HUB MAILBOX (§7.3 enrol → ENROLL_OK {hub card, custody_seq_floor}; CUSTODY_PUT/ACK, MAILBOX_FETCH/COMMIT; custody
  only between members of ONE hub or over a direct link), delivery states queued/sent/relayed/delivered with evidence, the
  Link trait §8 with ONE link built (tcp-lan, port 11183); §4 anti-entropy SUMMARY VECTORS specified for hub-to-hub
  replication "A+" (§21: deferred — cross-hub custody needs it); kind 0x30 BROADCAST reserved ("public signed frames are a
  Layer 7 format"); KB/UPDATE/PATCH classes defined, consumers are Layers 7–9; "multi-hop discovery and announce-as-routing
  (L7)"; "LAN-multicast epidemic and announce-as-routing rejected in v1 (pull Layer 7 forward)". The Layer 6 seams scout
  found the fabric as built is CONNECTION-ORIENTED: sends only to an attributed (link, PeerAddr), never Broadcast; no transit
  forwarding (Dir::Transit is a schema value only); the fragment plan is fixed at queue time; no remove_link
  (${ROOT}/plan/layers/6-transport/research/00-code-seams.md §3 findings 1–7). Read those seven findings before you propose
  anything that routes 111 frames.
- L6 Transport & Bridge (${ROOT}/6-transport/README.md, locked as built D29, 2026-10-23): the bridge (readable text both ways
  with Meshtastic, Reticulum/LXMF via our own helper in a venv, Delta Chat + Madmail) as zero-weight on-demand components;
  the transport spine (interfaces.toml with the [order] table of preferred networks, rnsd posture, phone relay over TCP with
  a QR join, the PortTable); the device core (hotplug, consented identify, the applier); relay EDGES between foreign
  networks (bridge/edges.rs — explicit, directed, non-transitive, loop-guarded; that is "relay" in Layer 6's sense, NOT this
  layer's); the Pi floor measured (111d idle 44.7 MB; nothing resident added). Explicit hand-offs to you (§18 "Layer 7
  inherits"): runtime join-in-sequence and fall-back through the preferred list (README §6; concepts/preferred-networks.md:
  "until Layer 7 lands the node is on every enabled network at once"), presence (NodeInfo and Reticulum announces already
  feed Nearby), hub-to-hub replication, attestation gossip and labels, the hub-learned custody_seq_floor wherever a join
  order needs it; v1.1 hub-member routing of foreign DMs. D19 c: the Cat-1 relay IS the stock MeshCore repeater (it floods
  our GRP_DATA frames blind; 111 writes no relay firmware); 111-owned boards run the MeshCore KISS modem; rnsd-owned boards
  run RNode.
- Parked notes ${ROOT}/plan/cross-layer/parked.md (P4 apply→verify→undo is Layer 9; P5 "own light mesh core in Rust" if our
  transport ever depends on Reticulum code; P7 MeshChatX panel). Decisions ${ROOT}/plan/DECISIONS.md: D17 (#5 rnsd posture,
  #12 Reticulum stays a connected service), D19 c, D20, D22 (GPL-3.0-or-later), D23–D29 (Layer 6). Plan protocol
  ${ROOT}/plan/EXECUTION.md; the Layer 6 options brief ${ROOT}/plan/layers/6-transport/options.md is the SHAPE the lead will
  write for this layer (settled facts → polled forks with picks → plan corrections): write so that brief can be written.

RULES (non-negotiable):
1. Reticulum (RNS/LXMF) is a connected service used as released, run as its own program (rnsd child; our LXMF helper is a
   separate Python process in its own venv that imports the user-installed packages, D26 #4). NO Reticulum/LXMF code may be
   imported, linked, copied or derived in our Rust tree (${ROOT}/LICENSING.md sections 1, 2, 7). Talking to rnsd through its
   config, rnstatus/rnpath, or a helper of ours in a venv is fine. Say so explicitly when a path needs "import RNS" in a NEW
   program of ours, and present it as a rule question, never the default.
2. D22: our code is GPL-3.0-or-later; GPL-3.0 code may link into our binary; every new dependency needs a sign-off row in
   LICENSING.md; incompatible licenses (the Reticulum License, GPL-2.0-only, CC-BY-NC, SSPL, BUSL, PolyForm) stay out.
   Weigh in-binary options against the separate-process pattern (Layers 5/6) every time.
3. THE RASPBERRY PI IS THE FLOOR (owner rule, AGENTS.md): the required configuration stays super light; everything
   third-party is a separate on-demand component that weighs NOTHING while off (no process, listener, loaded code, timer or
   open file). No task may add a resident process, a periodic poll, a runtime dependency or build-time weight to the
   required configuration WITHOUT THE OWNER'S SAY. So: README §7's "continuously announce through heartbeats" is a direct
   tension with this rule and with L5 A6 (announces on demand only). Do not resolve it by fiat — quantify it (bytes, airtime,
   wakeups, RSS) so the owner can decide, and offer an event-driven design beside the heartbeat one.
4. Our identity and the Layer 5 envelope are fixed. No wall clock in crypto (mesh-epochs and monotonic counters only).
   Offline-first, no runtime downloads, no telemetry, no policy limits of our own (airtime/duty cycle are physics the
   adapters advertise), delivery first, latency second. Nothing unsealed leaves the node except what the user sends or
   bridges; a gateway never proxies inference to the WAN.
5. Device floor: Linux x86_64/aarch64 (musl-static Rust binary) and Android via Termux (aarch64, no root, no multicast
   receive, background limits). Sidecars run as supervised children from hash-locked venvs or CI-built static binaries.
6. License gate for adoption: verify every license from the upstream LICENSE file, never from memory. Note release recency
   with dates. Mark anything you could not verify "(unverified)". Your training data is stale: fetched facts win.
7. Legal facts never guessed: amateur bands forbid obscured content (sealed frames never into HAM mode); LoRa duty cycles
   per region; a gateway relaying into a licensed radio needs a human.
8. No hardware is available (no LoRa boards; the owner sets up a full bench later). Everything radio-side is verified
   against stubs in network namespaces; say what stays hardware-gated.

THIS BOX'S NETWORK (read before you fetch; it decides your method): outbound HTTPS goes through a policy proxy. REACHABLE:
raw.githubusercontent.com (any public repo's files: LICENSE, README, docs/, source — use this for every GitHub-hosted
project), pypi.org and files.pythonhosted.org (package JSON at https://pypi.org/pypi/<name>/json), crates.io and
index.crates.io (https://crates.io/api/v1/crates/<name>), registry.npmjs.org, proxy.golang.org. DENIED (a fetch fails at
CONNECT with 403 or times out — do not retry): meshtastic.org, reticulum.network, docs.rs, rfc-editor.org, github.com HTML
pages, api.github.com, nostr.com, yggdrasil-network.github.io and most other sites. WebSearch works (it runs outside the
box) and returns snippets with URLs: cite the URL and mark a claim "(search snippet)" when you could not fetch the page.
For a GitHub project, derive the raw URL (https://raw.githubusercontent.com/<owner>/<repo>/<default-branch>/<path>; try
main then master) and read the LICENSE and README from there. One failed fetch per host is enough to know it is denied.

METHOD (mandatory unless your lens says "no web"):
- First call ToolSearch with query "select:WebSearch,WebFetch" to load the web tools, then research on the live web within
  the network note above. Prefer upstream repos and package indexes. Record the exact URL for every factual claim. Fetch the
  LICENSE file for every candidate you rate "adopt" or "bridge".
- Read only the repo sections named in your lens (sed -n / grep / rg); do not slurp whole trees. Cite file:line.
- Do NOT write code and do NOT modify any repo file except your own report path. Do NOT run cargo, pnpm, 111d, rnsd or any
  server. Nothing of yours may be left running.
- Cost discipline: aim for ≤ 40 web fetches; stop when the recommendation is grounded.

REPORT (write it with the Write tool, markdown, ≤ 3500 words, to the path given in your lens; then return the structured
summary). Sections, in order: 1. Summary (≤ 6 bullets) · 2. Candidates table (name · upstream · license (verified?) ·
language/runtime · maturity signals (last release with date, activity, adopters) · resident weight (RSS / binary / listeners
/ timers, measured or published, or "unknown") · fit: adopt|bridge|reference|reject · why) · 3. Findings that change the
plan (numbered facts with URLs or file:line) · 4. Recommendation — where a genuine fork exists, give 2–4 options with
one-line trade-offs and your pick · 5. Open questions for the owner · 6. Sources.
`

const LENSES = [
  {
    key: 'seams', model: 'opus', effort: 'high',
    prompt: `LENS 0 — AS-BUILT CODE SEAMS (no web; read code; every claim line-verified with file:line).
Map exactly where Layer 7 plugs into the built tree, so the options brief and the build briefs can cite real seams.
Read, in this order, only what you need of:
${ROOT}/5-messaging/crates/mesh111/src/{fabric.rs,contacts.rs,envelope.rs,link/mod.rs,link/tcp_lan.rs,store/**} — announces
as built (when emitted, what they carry, appdata TLV room), the hub enrolment and custody calls (§7.3: ENROLL_OK,
custody_seq_floor, CUSTODY_PUT/ACK, MAILBOX_FETCH/COMMIT), the §4 summary vectors (specified vs built), Dir::Transit,
Broadcast, kind 0x30, remove_link, the fragment plan, what a second Link driver registers (the Layer 6 seams report
${ROOT}/plan/layers/6-transport/research/00-code-seams.md §3 is your starting map — re-verify its seven findings against
today's code and note what Layer 6 changed: fabric foreign calls B1, store V3);
${ROOT}/1-bootstrap/crates/node111/src/daemon/{fabric/**,transport/**,devices/registry.rs,ports.rs,services/**,
supervise.rs,components/**,update*/**,detect*/**,bridge/{mod.rs,core.rs,attest.rs,edges.rs,ingest.rs},meshtastic/presence.rs,
dashboard/api_v1/{nearby.rs,fabric.rs,networks.rs,transport.rs,status.rs,mod.rs},dashboard/events*.rs} — the Nearby
sources and their JSON (meshtastic presence file, the LXMF push_heard/nearby_source, the bridge's), transport_state and
its producers, the [order] table and posture (interfaces.rs, posture.rs: enable_transport, share_instance, TCP server/
client stanzas), the PortTable, EVERY timer/tick/poll that exists in the required configuration today (the registry 5 s
tick, the bridge's demand-only facts tick, SSE keep-alive, the update check cadence: name each with file:line — the Pi-floor
rule makes this list the baseline), the gateway capability the installer records and where it is stored/exposed, the
updater's inputs (manifest URL, channel file, minisign key, payload layout, where the KB lands on disk), the services
registry/probe pattern (how a "gateway" or "presence" service row would be added), the components pattern (manifest,
cache, enable/disable, zero weight) for any adopted gateway program, the phone relay (TCP to hub + QR join) as the
pattern a gateway rendezvous would reuse, the attestation bytes (attest.rs) and where they could be gossiped;
${ROOT}/3-intelligence/crates/agent111/src/tools.rs (nearby.read / fabric.read; how a tool is added);
${ROOT}/4-interface/api/openapi.yaml (NearbySource, Nearby, Thread, Status.transport, x-sse-events, the bridge snapshot)
and ${ROOT}/4-interface/ui/src/views/nearby/** + src/lib/api/{nearby.ts,store.ts} (how sources are rendered, where a radar
canvas and a lazy map bucket would live; ${ROOT}/4-interface/ui/scripts/check-budget.mjs for the budget rule);
${ROOT}/6-transport/verify/zero-weight.sh (how the floor is measured) and the test/check commands per crate
(1-bootstrap/scripts/check.sh stages, 5-messaging/scripts/check.sh, 3-intelligence/scripts/check.sh, 4-interface/ui
package.json scripts).
Deliver ${OUT}/00-code-seams.md with: per subsystem a table (file:line · what exists · the seam · what Layer 7 must add
or change) for six needs: (a) presence — sources, states, "last heard", what a heartbeat or an event-driven announce
would touch, the timer baseline; (b) transit forwarding and multi-link delivery in mesh111 (what is missing, the cost of
each amendment, what Layer 6 already loosened); (c) hub-to-hub replication A+ (what §4/§7.3 already provide, what a
second hub needs); (d) a gateway role — detection (internet up: netlink trigger? the updater's last successful fetch?),
advertisement, and the seam a store-and-forward adapter would implement (is Layer 6's Adapter trait or Layer 5's Link
trait the right shape? say which and why); (e) KB/UPDATE distribution — what the updater and KB loader expect on disk,
the KB/UPDATE classes in envelope.rs, the hub serving them; (f) Nearby UI radar/map + the contract (additive fields only).
Then "traps" (things that look like seams but are not), the exact current test counts per crate (run nothing: read the
board summaries and the last CI-run notes in ${ROOT}/6-transport/TASKS.md), and open questions. Return the structured
summary (candidates may be empty).`,
  },
  {
    key: 'presence', model: 'sonnet', effort: 'high',
    prompt: `LENS 1 — PRESENCE, HEARTBEATS, DISCOVERY AND THE MESH-EPOCH SOURCE.
README §7 opens with "nodes continuously announce reachability through heartbeats". The built rules say: announces on
demand only (L5 A6), no periodic timer in the required configuration without the owner's say (Pi floor), Reticulum caps
announces near 2 % of interface bandwidth, LoRa airtime is physics (concepts/lora-airtime-budget.md), Termux phones have
no multicast receive and background limits. Research, with URLs, how the adopted and neighbouring projects do presence
and discovery and what each costs: Reticulum announces (default interval, path requests, the announce cap, what rnstatus/
rnpath expose; propagation-node announces), Meshtastic NodeInfo/position broadcast intervals and the NodeDB (already feeding
Nearby), MeshCore adverts (interval, flood), LXMF, mDNS/DNS-SD on a LAN (111d already runs mdns-sd for 111.local — can a
_111._tcp TXT record carry role/hub/gateway at zero resident cost? multicast on Android), Yggdrasil/cjdns peering, libp2p
mDNS and rendezvous, Briar LAN/BLE discovery, IPv6 NDP/link-local, Bluetooth LE presence (later layer; note only). Then
design options for 111 the owner can choose between, each with numbers: (A) event-driven presence only — link-up, cards,
INTRO, custody traffic, and the foreign networks' own presence feeds — plus an honest "last heard" (no heartbeat at all);
(B) adaptive heartbeats as an on-demand component or per-transport opt-in, with the airtime arithmetic (a 60–100 B frame
at SF7/BW125 and SF10, per node, per interval; the LAN cost; wakeups on a phone); (C) presence piggybacked on traffic that
exists (receipts, summaries, the foreign presence). Say what "reachability" can honestly mean per transport (LAN: a TCP
dial; LoRa: heard a frame; via a hub: the hub's custody) — the Nearby list already distinguishes sources. The SERVICE
DISCOVERY REGISTRY (README "Intent-Based Routing"; Layer 3 offload needs hub role, model, load): which adopted mechanism —
DNS-SD TXT on the LAN, a signed role TLV in the L2 announce's appdata (0–32 B, versioned), the hub card at enrolment — and
what must never be in it (load figures change; a signed blob cannot). THE MESH-EPOCH SOURCE (L2 §13, §9, §10: cert and
attestation expiries count in mesh-epochs; suppressing advancement extends a compromised cert, forcing it is a DoS; the
source is Layer 7's; v1 evaluates nothing): survey clockless epoch designs (Lamport/vector clocks, a hub-signed epoch
beacon, "advance when you see a greater one" gossip, Reticulum has none, Nostr uses wall-clock created_at, SSB sequence
numbers, Matrix DAG depth, CT signed tree heads, Roughtime) and propose 2–3 options with their trust fail modes and what
each asks of a Cat-1 relay. Also: the runtime "join in sequence, fall back through the list" (README §6; preferred-networks)
— what joining means per network today (everything enabled is on at once) and whether Layer 7 should change that at all
(say when it should not). Deliver ${OUT}/01-presence-discovery.md.`,
  },
  {
    key: 'gateways', model: 'sonnet', effort: 'high',
    prompt: `LENS 2 — GATEWAYS AND THE GLOBAL STORE-AND-FORWARD TRANSPORT.
README §7 + Category 5: a node with internet advertises it and becomes a gateway; gateways synchronize the offline local
mesh with "the global network" through an asynchronous store-and-forward transport ADOPTED VIA THE KNOWLEDGE LAYER;
intermittent gateways flush queues when connectivity appears; permanent gateways stream global broadcasts and encrypted
messages into the local mesh in real time. Settle what "the global network" can honestly be for 111 in v1 and what carries
sealed 111 envelopes (the L5 envelope, F = 160 B fragments, ids, receipts) between two 111 islands over the internet.
Survey candidates with license (verified from the LICENSE file), maturity with dates, resident weight (must be an on-demand
zero-weight component on the Pi; a Cat 5 role is opt-in), NAT traversal, offline-first fit, abuse and legal exposure:
Nostr relays (KB protocols/nostr.md; signed events; L2 reserved transport_id 0x01 = nostr-secp256k1 — the identity layer
anticipated it; could relays be dumb mailboxes for sealed 111 frames: which NIPs (ephemeral/parameterized events, NIP-13
PoW), public relay size limits and retention, paid relays, the rust-nostr/nostr-sdk crates and their weight), Reticulum
over the internet (TCP/I2P interfaces, Backbone, LXMF propagation nodes — within rule 1: rnsd as the carrier, our helper
in a venv; what a "111 over a Reticulum destination" carrier would need from the helper), iroh (projects/iroh-nodeid.md;
relays + QUIC hole punching; MIT/Apache; the +13 MB L5 rejected — re-weigh as an optional component), the chatmail / Delta
Chat relay already shipped as a Layer 6 component (email is the most deployed store-and-forward; could a sealed 111
envelope ride a chatmail relay between gateways: size limits, retention, rate limits, Madmail self-hosted), Matrix (weight),
SimpleX SMP servers (AGPL), Tor onion services via arti (Rust; the Briar Mailbox pattern: projects/briar-mailbox.md),
Veilid, Syncthing relays (MPL; strelaysrv), WireGuard/Headscale-style owner-run rendezvous, dtn7/BPv7 for gateway-to-
gateway only (KB: clock-based lifetime and heavy fragments ruled it out for LoRa — on IP between gateways that cost
vanishes: re-evaluate honestly), MQTT (Meshtastic's own global layer: the Layer 6 Meshtastic bridge could ride an MQTT
broker for that category), and the plainest option: a 111-RUN RENDEZVOUS — a Cat 4/5 hub with a public address running
the same tcp-lan link (no new software; what it needs: a stable address, Noise/TLS, DoS posture, a dial-out from behind
NAT) — plus hub-to-hub replication A+ (L5 §4 summary vectors) over that stream. For each: what a gateway must never do
(proxy inference to the WAN, forward anything unsealed, relay into licensed radio without a human), consent and privacy
(advertising "I have internet" to the mesh; metered connections; data budgets on phones; Android doze/background limits
for Termux), and the "global broadcasts" clause (what is a global broadcast in 111, who signs it, how it is rate-shaped
into a LoRa mesh — physics). Give 2–4 options with picks and a v1 shortlist of at most two carriers. Deliver
${OUT}/02-gateways-global.md.`,
  },
  {
    key: 'relay', model: 'sonnet', effort: 'high',
    prompt: `LENS 3 — RELAY INSIDE THE 111 FABRIC: MULTI-HOP, STORE-CARRY-FORWARD, CROSS-HUB DELIVERY.
README §7 closes with "delivery paths form dynamically from the combination of local mesh links and internet-capable
gateways". As built (read ${ROOT}/5-messaging/README.md §4, §7.3, §8, §9, §12, §21 and
${ROOT}/plan/layers/6-transport/research/00-code-seams.md §3 findings 1–7 first): the fabric is connection-oriented, no
transit forwarding, fixed fragment plan, custody only within one hub or over a direct link; A+ hub-to-hub replication is
specified as summary vectors but not built; the Cat-1 relay is the stock MeshCore repeater flooding our GRP_DATA frames
blind (D19 c); rnsd can be a Reticulum transport node (enable_transport) but Reticulum code cannot enter our tree (rule 1).
Research adoptable forwarding substrates against "adopt, don't reinvent" and "never reimplement routing": (1) Reticulum as
the multi-hop router for sealed 111 frames — what an external process can use without importing RNS in our Rust: the shared
instance, TCP/local interfaces, our LXMF helper (already imports RNS in its own venv, D26 #4) opening a raw Reticulum
Destination/Link/Channel for 111 frames (the "rns-bridge" seam Layer 6 pinned), MTU (~500 B), link establishment cost over
LoRa, the 2 % announce cap, path requests, what rnpath/rnstatus expose; (2) epidemic / anti-entropy store-carry-forward
between hubs and devices — the §4 summary vectors; prior art: Scuttlebutt replication, Briar's sync protocol (BSP),
Delta Chat (none), DTN epidemic/PRoPHET, Automerge sync (KB; Layer 8), Syncthing's index exchange; which is a pattern to
copy and which a program to run; (3) IP-level mesh routing where IP links exist (Babel/babeld MIT, BATMAN-adv GPL-2
kernel module, Yggdrasil LGPL-3, cjdns, OLSRd) — whether a hub-managed Wi-Fi mesh belongs here at all (say if it is Layer 6
or out of scope); (4) the minimal own forwarder: hub-centric star + gateways, transit only at hubs (device→hub→hub-to-hub
over LAN/LoRa/internet→hub→device), which keeps "no routing protocol of our own" honest — pin exactly which of the seven
mesh111 amendments each option needs, what the airtime arithmetic says for hub-to-hub LoRa frames, how the built dedup +
summaries give loop/duplicate suppression, and what the Cat-1 relay does in each (MeshCore flood vs Reticulum transport
node vs nothing). Also answer the L2 §13 hand-offs: announce-as-routing vs identity-only, the Cat-1 announce-verification
policy (verify-before-relay vs relay-blind; the stock repeater cannot verify), and the announce rate-limit policy. Give
2–4 options with one-line trade-offs and your pick; name what is v1 and what is v1.1. Deliver ${OUT}/03-fabric-relay.md.`,
  },
  {
    key: 'kb-updates', model: 'sonnet', effort: 'high',
    prompt: `LENS 4 — KB, UPDATES AND MANIFEST DISTRIBUTION OVER THE MESH; THE KNOWLEDGE HUB.
Layer 1 handed over "gateway re-evaluation, KB git-sync, manifest gossip" ("mesh gossip arrives with Layer 7"; "KB git-sync
is Layer 7 — v1: KB inside the release payload"); Layer 5 defined the KB and UPDATE frame classes with consumers in Layer 7;
Category 4A knowledge hubs host offline datasets and software updates; the updater is rooted in a minisign-signed manifest
(read ${ROOT}/1-bootstrap/README.md "Updates" and the updater module named by the seams scout's lens; do not wait for it —
grep ${ROOT}/1-bootstrap/crates/node111/src for minisign, manifest, channel). Research: (1) distributing a signed release
(binary + manifest + notices) from a hub to nodes with no internet — the hub as the update origin on the LAN (the updater's
origin URL becomes the hub's; the hub's own HTTP; what the updater must verify so a hub can never forge: the manifest
signature, the channel file, downgrade protection), over the fabric's Tier 2 (bao-tree verified chunks — specified in L5
§5/§11, built?), over LoRa (never for binaries — show the airtime arithmetic), by sneakernet (a signed bundle on a USB
stick: \`111 update --from <dir>\`); prior art: apt mirrors/apt-offline, F-Droid nearby swap, Kiwix hotspot, TUF (compare
its roles with our one minisign key: what we lack, what we deliberately skip), Syncthing; (2) KB git-sync — the KB is a git
repository of markdown + generated llms.txt (${ROOT}/0-knowledge/, built by _meta/build.py): options are signed git
bundles carried any way (git bundle create/verify; a hub serving a dumb-HTTP git export so \`git fetch\` works with no new
software; a \`111 kb sync\` verb), a KB snapshot only in the release payload (status quo), gateways fetching the 111 repo
when online (signed tags, minisign over the bundle), and how llms.txt regeneration is kept deterministic on the node (is
build.py on the node? Python on a Pi is fine, on Termux?); (3) "manifest gossip" — a small signed "a newer release exists"
announcement riding the L2 announce appdata or an UPDATE frame vs the hub-as-origin approach (no gossip needed; the
installer already records hub pairing) — recommend the simplest honest one; (4) the trust model: never trust a hub's bytes
without the release signature; key rotation implications; the clock-fault class (minisign is time-free); what a Cat-1
relay (a stock MeshCore repeater) can carry of any of this (nothing: say so). (5) Note only what Layer 8 later needs
(Kiwix ZIM, maps, large datasets) so Layer 7's transfer mechanism is not redone. Candidates with licenses verified from
LICENSE files where you rate adopt. Deliver ${OUT}/04-kb-updates.md.`,
  },
  {
    key: 'ui-floor', model: 'sonnet', effort: 'high',
    prompt: `LENS 5 — NEARBY RADAR/MAP, GATEWAY AND PRESENCE UI, CONSENT, AND THE PI-FLOOR WEIGHT OF EVERY CANDIDATE.
The dashboard promised: "radar and map views arrive with Layer 7" (${ROOT}/4-interface/README.md §6.4; read it and
${ROOT}/4-interface/ui/src/views/nearby/** for what exists: one panel per source, untrusted names rendered as text, "Ask
the agent who is nearby"); peers/transports/gateway detail panels are L6/L7; the UI budget D21 (256 KB eager + lazy
buckets; ${ROOT}/4-interface/ui/scripts/check-budget.mjs). Research: (1) an OFFLINE map inside a GPL web UI with no network
at all: MapLibre GL JS (BSD-3; size gz; vector tiles from PMTiles (BSD) — a hub-served regional extract is Layer 8's
"regional maps"), Leaflet (BSD-2; ~40 KB) + raster tiles from a hub (MBTiles/PMTiles via a tiny tile route in 111d, or
none), OpenLayers; what Meshtastic web, MeshMonitor and MeshChatX do for maps (internet tiles?) and what offline-first apps
do (Organic Maps, Kiwix); the honest v1: a RADAR with no basemap (relative positions from SNR/hops/GPS deltas; Meshtastic
position precision bits; our own nodes share no position by default) and a lazy map bucket that works only when a tile
source exists — size the bundle cost of each; (2) presence UI semantics: "last heard" per source and transport, the gateway
badge, the hub/role registry in Nearby and Settings, what a Cat-1 relay shows (nothing but "heard via"), the agent's
nearby.read answer; (3) consent and privacy: advertising gateway status to the mesh, position sharing, the licensed-radio
rule (built in Layer 6 as consent facts — a gateway must not relay sealed frames into HAM channels), metered connections
on phones (Android NetworkCapabilities NOT_METERED; what Termux:API exposes: termux-wifi-connectioninfo, termux-telephony),
data budgets, Android doze; (4) THE PI FLOOR for every program the other lenses are likely to propose — rnsd as a transport
node (vs client; RSS and CPU published by Reticulum users on a Pi Zero/3/4), a Nostr relay client crate (rust-nostr size),
iroh (binary delta), a chatmail/Delta Chat core (~64 MB RSS measured in Layer 6 for core + Madmail), babeld, a tile server,
git on the node, a BLE scanner: find published or measurable resident cost (RSS, binary size, listeners, timers) and whether
each can be an on-demand zero-weight component; produce the one table the owner needs to say yes or no to anything resident.
State plainly: a "continuous heartbeat" of our own is an owner question under the Pi-floor rule, and give the numbers
(bytes/day, wakeups/hour, airtime/hour per transport) from lens 1's arithmetic if that report is already on disk, else your
own. Candidates with licenses verified from LICENSE files. Deliver ${OUT}/05-ui-consent-floor.md.`,
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
    recommendation: { type: 'string', description: 'the fork options and your pick, ≤ 120 words' },
    open_questions: { type: 'array', items: { type: 'string' } },
    sources_count: { type: 'integer' },
    unverified: { type: 'array', items: { type: 'string' }, description: 'claims that matter which you could not verify from a reachable source (host denied, page missing): the lead verifies them elsewhere' },
  },
}

const CRITIC_SCHEMA = {
  type: 'object',
  required: ['verdict', 'gaps', 'conflicts', 'owner_forks'],
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
    owner_forks: { type: 'array', items: { type: 'string' }, description: 'the genuine forks the owner must decide, one line each, as the critic sees them across all reports' },
  },
}

phase('Scout')
log('Layer 7 research: seams scout + 5 lens scouts on disjoint questions')
const scouts = await parallel(LENSES.map(l => () =>
  agent(CTX + '\n\n' + l.prompt, { label: `scout:${l.key}`, phase: 'Scout', schema: SCOUT_SCHEMA, model: l.model, effort: l.effort })
    .then(r => r && { ...r, key: l.key })
))
const done = scouts.filter(Boolean)
log(`${done.length}/${LENSES.length} scouts returned`)
if (done.length < LENSES.length) log('missing: ' + LENSES.filter(l => !done.find(d => d.key === l.key)).map(l => l.key).join(', '))

phase('Critique')
const criticPrompt = `${CTX}

YOU ARE THE COMPLETENESS CRITIC for the Layer 7 research sweep. Six scouts wrote reports in ${OUT}/ (read every file there
in full: 00-code-seams.md, 01-presence-discovery.md, 02-gateways-global.md, 03-fabric-relay.md, 04-kb-updates.md,
05-ui-consent-floor.md). Their structured summaries: ${JSON.stringify(done.map(d => ({ key: d.key, headline: d.headline, recommendation: d.recommendation, open_questions: d.open_questions, findings: d.findings, unverified: d.unverified })), null, 1)}

Check the sweep against what an OPTIONS BRIEF for the owner needs: README §7 verbatim and Category 5, the Layer 1/2/3/4/5/6
hand-offs to Layer 7 (listed in the context: gateway re-evaluation, KB git-sync, manifest gossip; the mesh-epoch source,
attestation gossip, announce-as-routing; the Service Discovery Registry; radar/map; A+ replication, BROADCAST, KB/UPDATE
consumers; join-in-sequence, presence), the Pi-floor rule (every resident cost quantified; the heartbeat tension put to the
owner with numbers, not resolved by fiat), the Reticulum rule, D22, "adopt, don't reinvent", the no-hardware constraint,
and this box's network policy (claims the scouts marked unverified because a host was denied: collect them into one list
the lead verifies on the owner's machine — that is not a gap to re-run here). Ask: which modality was not run, which claim
is unverified where it decides a fork (a license, an API that may not exist, a size or a limit), which candidate was missed
(name it), which cross-report conflict must be resolved before the owner can choose, where did a scout recommend custom
code although an adoptable project exists, and where did a scout propose something resident without a number? You may do
targeted web checks yourself (ToolSearch select:WebSearch,WebFetch; the network note applies) to settle a conflict cheaply,
but do not redo the sweep. Rate each gap blocking (the owner cannot choose without it) / major / minor and write a
complete scout prompt for each blocking or major gap. List the genuine owner forks you see across all reports. Write your
critique to ${OUT}/06-critic.md (≤ 1500 words; include the unverified-claims list as its own section) and return the
structured verdict.`
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

GAP-FILL SCOUT. The completeness critic found this blocking gap in the Layer 7 sweep (existing reports are in ${OUT}/;
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
  scouts: done.map(d => ({ key: d.key, report: d.report_path, headline: d.headline, recommendation: d.recommendation, open_questions: d.open_questions, unverified: d.unverified, candidates: d.candidates.map(c => `${c.name} [${c.license}${c.license_verified ? '' : '?'}] ${c.fit}`), sources: d.sources_count })),
  critic: critique && { verdict: critique.verdict, gaps: critique.gaps.map(g => `${g.severity}: ${g.title}`), conflicts: critique.conflicts, owner_forks: critique.owner_forks },
  gap_fills: fills.map(f => ({ gap: f.gap, report: f.report_path, headline: f.headline, recommendation: f.recommendation })),
}
