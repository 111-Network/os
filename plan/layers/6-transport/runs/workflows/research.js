export const meta = {
  name: 'layer-6-research',
  description: 'Layer 6 (Transport & Bridge) research sweep: as-built seams scout + 5 lens scouts, completeness critic, gap-fill',
  phases: [
    { title: 'Scout', detail: '1 opus seams scout + 5 sonnet lens scouts, web-verified, reports on disk' },
    { title: 'Critique', detail: 'opus completeness critic over all reports' },
    { title: 'Gap-fill', detail: 'up to 2 scouts on blocking gaps' },
  ],
}

const ROOT = '/home/kia/projects/111network/os'
const OUT = ROOT + '/plan/layers/6-transport/research'

const CTX = `
You are a research scout for the 111network OS build (repo root: ${ROOT}). Today is 2026-10-19.

WHAT 111 IS: a minimal operating layer that turns any device (old phone, laptop, Pi, LoRa board) into a node of a resilient
offline mesh. Principle, non-negotiable: ADOPT, DON'T REINVENT. Existing mesh/offline projects are assets we run, bridge and
show; custom code is only glue. A local AI agent is the interface and the self-healer. Read ${ROOT}/README.md first: the
section "6. Transport & Bridge" is the layer you research ("Existing mesh networks are treated as pure transport and
knowledge sources … Bridges join physically incompatible channels … remain thin: they translate only the minimal common
format and never reimplement the underlying radio or routing protocols"), "5. Messaging & Data" is what is already built
underneath, "7. Relay & Presence" is the neighbour above, and "Category 3: Transport Bridges" is the hardware tier this
layer brings to life.

WHAT IS ALREADY BUILT (Layers 0–5; do not re-research them, wire to them; cite file:line when you point at code):
- L0 Knowledge base ${ROOT}/0-knowledge/ (entries with YAML frontmatter; rubric candidate→testing→proven in its README).
  Existing entries you may cite and must not contradict without a fresher source: projects/{meshtastic,meshtastic-python,
  meshtastic-web,meshtastic-pki,meshmud,meshmonitor,meshcore,meshchatx,lxmf,reticulum,briar-bramble,briar-mailbox,
  delta-chat-securejoin,home-assistant-ingress}, protocols/{reticulum-protocol,nostr,bundle-protocol-v7},
  concepts/{lora-airtime-budget,secure-contexts,store-and-forward-states}, devices/heltec-wifi-lora-32-v3.
- L1 Bootstrap: Rust daemon "111d" (crate node111 at ${ROOT}/1-bootstrap/crates/node111) supervises children; runs
  Reticulum's "rnsd" as a child from a hash-locked Python venv with the config it renders from
  ${ROOT}/1-bootstrap/crates/node111/assets/rns/config.tmpl (AutoInterface, share_instance = no, enable_transport = no);
  "transport_state" = no transport | isolated | joined | degraded; Android phones join via Termux (no root, no USB serial,
  no BLE, no multicast receive). Devices registry at src/daemon/devices/ (DeviceRecord, devices.json, lora-table.toml).
- L2 Identity (${ROOT}/2-identity/README.md): Ed25519 + X25519 from one seed; NodeId = raw 32-byte Ed25519 pubkey;
  sealed packets (XChaCha20-Poly1305, per-packet ephemeral X25519); §10 attestation format for transport identities
  (A10: one bridge identity per foreign network, bound by bidirectional cross-signed attestation; link keys never
  seed-derived).
- L3 Intelligence: "111-agent" (crate agent111, ${ROOT}/3-intelligence/crates/agent111) with fixed shell-free tools incl.
  messages.read / messages.send (proposal-gated) / contacts.read / fabric.read / nearby.read; a per-spawn bearer token.
- L4 Dashboard (${ROOT}/4-interface/README.md; contract ${ROOT}/4-interface/api/openapi.yaml): React UI embedded in 111d,
  /api/v1 REST + one SSE stream, services registry, device registry, Messages / Contacts / Nearby / Settings views,
  hosted panels (one origin per panel by port 11190–11229, ticket cookie, Meshtastic web client as the first panel).
- L5 Messaging & Data (${ROOT}/5-messaging/README.md, spec v1.2 locked, built; closure in flight): the fabric "mesh111"
  (${ROOT}/5-messaging/crates/mesh111) wired into 111d as daemon/fabric/: envelope codec, msg ids, fragmentation (F = 160 B),
  authenticated ACKs + custody receipts, contacts/cards/node1… addresses, home-hub mailbox, SQLCipher store, the Link trait
  (§8) with ONE link built (tcp-lan, port 11183) and the seams kiss-modem / meshtastic-client / rns-bridge / ble pinned
  with their frame budgets (233/221 B Meshtastic PRIVATE_APP; 287 B LXMF; 160 B MeshCore GRP_DATA). Meshtastic service (§14):
  consented identify, the "meshmud" multiplexer (Go, MIT, vendored, CI-built; uplink = board serial/TCP; TCP downlink
  127.0.0.1:11230 only, NO HTTP downlink), a GPL Python presence reader in its own hash-locked venv
  (${ROOT}/5-messaging/sidecars/meshtastic-presence/) that writes a presence file 111d reads into Nearby, the built-in
  HTTP shim that serves the hosted Meshtastic web client. Layer 5 promised "no message bridge" and "111 never transmits
  on the Meshtastic mesh" — Layer 6 supersedes both (see the owner decisions below).
- Layer 6 opener brief with 12 owner decisions (D17): ${ROOT}/plan/cross-layer/devices.md — read §0 findings, §2 device
  classes, §3 what the core must do, §7 decisions, §8 sequencing, §9 Layer 5 hand-offs. Decisions log
  ${ROOT}/plan/DECISIONS.md: D17 (devices), D19 c (LoRa ownership: a 111-owned board runs the MeshCore KISS modem; stock
  MeshCore repeaters flood our GRP_DATA frames; rnsd-owned boards run RNode), D20 (Layer 5 lock + hand-offs), D22
  (relicensed to GPL-3.0-or-later on 2026-10-19), D23 (this kickoff — read it in full).

OWNER DECISIONS FOR THIS LAYER (D23, 2026-10-19), verbatim where quoted:
- Goal: "we can send the same message in several networks + we receive from other networks and BRIDGE; the bridge
  function is super important."
- Bridge semantics: "the chances are that people will get used to our network fully, even the dashboard. The UI of other
  services must be available fully, but still text appears in ours, in the Messages view, from different networks. Keep it
  categorized for other networks." So: readable text both ways; foreign networks' own UIs stay hosted panels; their
  messages appear in OUR Messages view under per-network categories (never merged into 111 threads); our users send into
  those networks in each one's native format; a node on several networks relays between them (opt-in, loop guard).
  Sealed 111 traffic keeps riding foreign radios opaquely between 111 nodes through the Layer 5 §8 link seams.
- v1 networks: Meshtastic, Reticulum/LXMF, and phone apps reached through Termux (Briar and the like; the owner's
  parked note P2 in ${ROOT}/plan/cross-layer/parked.md). The MeshCore network is NOT a v1 bridge target; the 111 KISS modem
  on MeshCore firmware (D19 c) stays a transport item.
- Hardware: NONE on the lead box yet (no LoRa boards, no phones wired in). The owner sets up a full test setup later. Every
  radio path is built and verified against stubs in network namespaces first; hardware gates stay in the runbooks.
- Agentic principle (D17): "the agent looks for what is connected or paired and resolves errors … we don't need specific
  hard-coded instructions for each device type; everything agentic and based on what it is."

HARD RULES THAT SHAPE EVERY RECOMMENDATION:
1. Reticulum is a CONNECTED SERVICE: run as-is, unmodified, separate process, latest upstream. NO Reticulum/LXMF code may
   be imported, linked, copied or derived in our tree (its license is custom, non-OSI, with an AI-training clause; D17 #12;
   ${ROOT}/LICENSING.md section 2 and section 7). Talking to rnsd/LXMF through an adopted app's external API, lxmd, or
   rnsd config is fine. If a path needs "import RNS/LXMF" in code WE write, say so explicitly and present it as a rule
   change the owner would have to make, never as the default.
2. D22 changed the GPL boundary: our own code is now GPL-3.0-or-later, so GPL-3.0 code MAY link into our Rust binary
   (LICENSING.md section 7 now checks GPL-3.0 compatibility instead of excluding copyleft). Every new dependency still
   needs a sign-off row, and incompatible licenses (the Reticulum License, GPL-2.0-only, CC-BY-NC, SSPL, BUSL, PolyForm, …)
   stay out. Weigh "in-binary" options that D22 unlocked against the separate-process pattern Layer 5 used.
3. Our identity (L2) and the Layer 5 envelope are fixed. A foreign network's messages are NOT 111-sealed: they are shown
   as that network's content, with that network's sender identity, labelled honestly. One bridge identity per foreign
   network (A10 attestation).
4. Offline-first, no wall clock in crypto, no runtime downloads, no telemetry, no policy limits of our own (no rate caps
   or expiry by policy; airtime/duty-cycle are physics the adapters advertise), delivery first, latency second.
5. Device floor: Linux x86_64/aarch64 (musl-static Rust binary) and Android via Termux (aarch64, no root). Sidecars run as
   supervised children from hash-locked venvs (the presence-reader pattern) or CI-built static binaries (the meshmud pattern).
6. License gate for adoption: verify every license from the upstream LICENSE file, never from memory. Note release
   recency with dates. Mark anything unverified as (unverified). Your training data is stale: fetched facts win.
7. Legal facts the agent must never guess: LoRa ISM region plans; amateur bands forbid obscured content (so sealed 111
   frames never ride Meshtastic HAM mode); Meshtastic's own community rules for automated senders.

METHOD (mandatory unless your lens says "no web"):
- First call ToolSearch with query "select:WebSearch,WebFetch" to load web tools, then research on the live web. Prefer
  upstream repos, docs, release pages. Record the exact URL for every factual claim. Fetch the LICENSE file for every
  candidate you rate "adopt" or "bridge".
- Read only the repo sections named in your lens (use sed -n / grep / rg); do not slurp whole trees.
- Do NOT write code and do NOT modify any repo file except your own report path.
- Cost discipline: aim for ≤ 40 web fetches; stop when the recommendation is grounded.

REPORT (write it with the Write tool, markdown, ≤ 3500 words, to the path given in your lens; then return the structured
summary). Sections, in order: 1. Summary (≤ 6 bullets) · 2. Candidates table (name · upstream · license (verified?) ·
language/runtime · maturity signals (last release, activity, adopters) · fit: adopt|bridge|reference|reject · why) ·
3. Findings that change the plan (numbered facts with URLs or file:line) · 4. Recommendation — where a genuine fork
exists, give 2–4 options with one-line trade-offs and your pick · 5. Open questions for the owner · 6. Sources.
`

const LENSES = [
  {
    key: 'seams', model: 'opus', effort: 'high',
    prompt: `LENS 0 — AS-BUILT CODE SEAMS (no web; read code; every claim line-verified with file:line).
Map exactly where Layer 6 plugs into the built tree, so the options brief and the build briefs can cite real seams.
Read, in this order, only what you need of: ${ROOT}/5-messaging/crates/mesh111/src/link/mod.rs (the Link trait as built vs
spec §8: LinkKind, PathProfile, PeerAddr, Frame; how a link is registered), src/fabric.rs (public API node111 calls: send,
events, link registration, path/profile selection, outbox behaviour per link), src/contacts.rs + src/store/** (contact
and thread/message schema: can a message row carry a foreign origin today? what is keyed by NodeId?), src/envelope.rs
(kinds; the L6 shim byte), src/link/tcp_lan.rs (the one driver: the pattern a second driver copies);
${ROOT}/1-bootstrap/crates/node111/src/daemon/fabric/{mod.rs,json.rs,settings.rs} (how TcpLan is constructed, settings,
events → SSE, the Message/Thread JSON), src/daemon/dashboard/api_v1/{messages.rs,contacts.rs,nearby.rs,fabric.rs,
devices.rs,proposals.rs}, src/daemon/meshtastic/{mod.rs,meshmud.rs,shim.rs,presence.rs,identify.rs} (the TCP-downlink
client pattern; the presence-file ingest pattern; the identify flow + port-owner table in src/daemon/ports.rs),
src/daemon/devices/{registry.rs,rns.rs,detect.rs,store.rs}, src/daemon/transport/** (rnsd child + config rendering:
schema-version seam), src/daemon/services/{mod.rs,probe.rs} (ServiceId list, Facts, rules), src/daemon/supervise.rs
(child pattern), assets/rns/config.tmpl, assets/lora-table.toml; ${ROOT}/4-interface/api/openapi.yaml (Thread, Message,
Contact, NearbySource, DeviceRecord, ServiceId, Proposal.kind, x-sse-events); ${ROOT}/4-interface/ui/src/views/messages/**
(how threads are keyed and listed; where a per-network category would live), src/lib/api/{messages.ts,store.ts,
nearby.ts}, src/views/{nearby,devices,settings}/** (file names + the models only), src/lib/{layers.ts,prompts.ts};
${ROOT}/3-intelligence/crates/agent111/src/tools.rs (messages.send proposal path; how a tool is added);
${ROOT}/5-messaging/sidecars/meshtastic-presence/meshtastic_presence.py + README (the sidecar pattern: venv lock, file
output, hardening) — and the test/check commands per crate (1-bootstrap/scripts/check.sh stages, 5-messaging/scripts/
check.sh, 3-intelligence/scripts/check.sh, 4-interface/ui package.json scripts).
Deliver ${OUT}/00-code-seams.md with: per subsystem a table (file:line · what exists · the seam · what Layer 6 must add or
change) for six needs: (a) foreign-network messages + per-network categories in store, API, SSE and UI; (b) a second Link
driver; (c) a daemon ↔ sidecar bridge protocol (what the presence reader does today, what a two-way sidecar needs);
(d) state/interfaces.toml + rnsd renderer v2; (e) the full identify sequence (RNode, Meshtastic, MeshCore) over the
port-owner table; (f) new agent tools. Then "traps" (things that look like seams but are not), the exact current test
counts per crate, and open questions. Return the structured summary (candidates may be empty).`,
  },
  {
    key: 'meshtastic', model: 'sonnet', effort: 'high',
    prompt: `LENS 1 — THE MESHTASTIC BRIDGE: send and receive readable text through the board 111 already drives.
Today 111d runs meshmud (uplink = the board; TCP downlink 127.0.0.1:11230 for the presence reader and the hosted web client
shim) and never transmits. Layer 6 must send text and DMs as a Meshtastic node and receive channel text and DMs into our
Messages view (category "Meshtastic"). Research and pin, with URLs: (1) the client API facts for text: ToRadio/FromRadio
and MeshPacket fields (to, channel index, decoded.portnum TEXT_MESSAGE_APP, payload, want_ack, hop_limit, id), the firmware
rewriting "from" to the attached node (so the bridge speaks as its own Meshtastic node — naming, long_name/short_name),
max text payload per packet (DATA_PAYLOAD_LEN; the app's 200-char rule), channels and PSKs (index 0 LongFast public key;
private channels), PKI direct messages (2.5+: public_key in NodeInfo, pki_encrypted; what a DM to the bridge node looks
like and who should receive it on our side), ack semantics (ROUTING_APP acks, implicit acks, error codes → our
sent/delivered/failed mapping), duplicate suppression (packet id), firmware-side throttling and airtime enforcement
(what a sender gets when it sends too fast), MQTT uplink/downlink flags (what leaks where), HAM mode (no encryption),
rebroadcast_mode. (2) meshmud as of today: upstream, latest release and date, whether several TCP-downlink clients may
SEND concurrently and how it serialises them, whether it rewrites anything, Termux build status. (3) Community and
legal rules for automated senders on Meshtastic (official docs, the public MQTT server policy, default-channel etiquette),
and what the honest service-page copy must now say. (4) The implementation fork, now that D22 allows GPL in our binary:
A. in-binary Rust (prost + the GPL-3.0 meshtastic/protobufs — generation at build vs vendored; the "meshtastic" Rust crate:
license, maturity, last release; binary-size and musl/aarch64 build facts); B. extend the GPL Python presence sidecar into a
two-way bridge sidecar (meshtastic-python send_text / onReceive over the TCP downlink; a local JSON-lines or Unix-socket
protocol to 111d; the venv already exists on Termux); C. a feature in meshmud itself (Go). Trade-offs: size, latency,
restart semantics, the single-client slot, Termux, maintenance. (5) How the hosted web client panel coexists with the
bridge sending through the same meshmud. Report path: ${OUT}/01-meshtastic-bridge.md`,
  },
  {
    key: 'lxmf', model: 'sonnet', effort: 'high',
    prompt: `LENS 2 — THE RETICULUM/LXMF BRIDGE: our node as an LXMF endpoint WITHOUT importing RNS.
Users of Sideband, Columba, NomadNet and MeshChat talk LXMF. Layer 6 must receive LXMF messages into our Messages view
(category "Reticulum") and send LXMF messages from it, through the rnsd child 111d already runs. First read
${ROOT}/LICENSING.md sections 2 and 7 and D17 #12 for the exact boundary, and the KB entries projects/lxmf.md,
projects/meshchatx.md, projects/reticulum.md. Then research and pin with URLs: (1) current upstream state (Reticulum and
LXMF versions and release dates as of today; anything new in 2026 such as a Rust rnsd, protocol changes, LXMF stamps /
tickets and what they cost a sender, propagation-node changes). (2) The external paths that do not import RNS in our code:
A. MeshChatX (or MeshChat) run headless as a child beside rnsd — its HTTP + WebSocket API for conversations, send, receive,
announces, attachments; auth flags; attaching to the shared rnsd instance; what it needs on Termux; license (verify 0BSD /
MIT), provenance and activity in 2026, and whether it is also the hosted panel P7 wants; B. lxmd — what it offers a
program that is not an LXMF client (delivery hooks, propagation node role, config); C. a tiny Python sidecar that imports
LXMF in the rns venv — present this ONLY as a rule change the owner would have to make (D17 #12), with the license
consequences spelled out; D. anything else (NomadNet, rnsh, LXMF over HTTP gateways). (3) rnsd posture changes needed in
assets/rns/config.tmpl (share_instance, shared_instance_port, enable_transport, a TCP server interface for phones) and
what each exposes on loopback/LAN. (4) LXMF facts a bridge needs: destination/identity model (a separate identity for the
bridge; display name announce; A10 attestation binding), message fields (title, content, fields such as attachments),
size limits over LoRa vs TCP, delivery methods (direct link, opportunistic, propagated) and proofs (what "delivered"
means), how other users discover and address us, group/channel semantics (LXMF has no channels: say so). Deliver the
fork with a pick. Report path: ${OUT}/02-lxmf-bridge.md`,
  },
  {
    key: 'phones', model: 'sonnet', effort: 'high',
    prompt: `LENS 3 — PHONE APPS VIA TERMUX (parked idea P2): what is HONESTLY feasible.
Owner's words: "once connected to a phone, we can access it and download existing apps like Briar etc and test connection
with those as well and become a node there as well and bridge within ours with others" — fully agentic, no per-app
hard-coding, the agent reads the app's docs from the KB. Our phone path is Termux (no root): 111d runs there; the agent
can run commands in Termux. Research and pin with URLs: (1) What an agent in Termux can actually do to a phone: termux-api
capabilities (and its 2026 state), installing an APK (termux-open → the user must tap), driving another app's UI (is
there any path without an accessibility service? ADB over wireless debugging from Termux to the SAME phone: pairing flow,
Android 11–16 limits, what adb shell input/am/pm then allow; is this stable enough to automate?), reading another app's
data (no, unless the app exports it), background execution limits. (2) Candidate networks with a programmable peer the
hub or phone could join: Briar (briar-headless REST API: platforms, transports it supports headless — LAN? Bluetooth? Tor
only?; license; 2026 activity; Briar Mailbox; whether a Linux hub can be a Briar contact of the phone user), Delta Chat
(deltachat-rpc-server JSON-RPC, chatmail relay self-hosted on a hub, offline-LAN viability), SimpleX (simplex-chat CLI
WebSocket API; self-hosted SMP/XFTP servers on a hub; offline LAN?), Jami (jami-daemon API; DHT on a LAN without
internet), Berty/wesh (daemon gRPC; BLE/mDNS), Matrix (a hub homeserver such as Conduit/Tuwunel plus matterbridge-style
bridges — note only), Bitchat (BLE mesh; any API?), Session (no), the Meshtastic and MeshCore Android apps (no API — the
board is the path; say so), Columba (LXMF client — covered by lens 2, note only). For each: license (verified), headless
peer availability, offline/LAN behaviour, how messages could be read and sent programmatically, ToS/legal limits on
automation. (3) Deliver three tiers: (i) bridgeable in v1 via a headless peer or API on a Linux hub; (ii) reachable only by
driving the phone (ADB/UI automation) — experimental; (iii) no path. Recommend what Layer 6 v1 should promise under the
"phone apps" category and what stays a KB-guided agent experiment. Report path: ${OUT}/03-phone-apps.md`,
  },
  {
    key: 'bridge', model: 'sonnet', effort: 'high',
    prompt: `LENS 4 — BRIDGE DESIGN: categories, fan-out, relay between networks, loop guard, consent, prior art.
Design question: how does one node present N networks in one Messages view, send one message into several of them, and
relay between them, thinly and honestly? Research prior art with URLs and verified licenses: matterbridge (multi-protocol
chat bridge — its gateway/channel mapping, prefix format, loop prevention, 2026 status, whether any Meshtastic or LXMF
backend exists), meshtastic-matrix-relay and other Meshtastic ↔ Matrix/Discord/Telegram/MQTT bridges, LXMF ↔ Telegram or
Matrix bridges, any Meshtastic ↔ Reticulum message bridge (not RNS_Over_Meshtastic, which is a transport), mautrix bridges
(pattern only), and the IRC/XMPP gateway lessons. Then pin, with 2–4 options each and a pick: (1) the foreign message
model (origin network, channel/conversation id, sender id + untrusted display name, received-time hint, state) and where it
lives (the SQLCipher store with an origin column vs per-network tables vs memory-only) — read ${ROOT}/5-messaging/README.md
§10.3 (schema) and §15 (routes) and ${ROOT}/4-interface/api/openapi.yaml Thread/Message first; (2) the Messages view
categories (per network, per channel/DM threads, badges, "unverified" labels; a Meshtastic node or LXMF address is never a
111 contact — how to show "message them" honestly); (3) compose fan-out: one text → several targets with per-target
state, and the size problem (Meshtastic ≈ 200 B per packet, LXMF larger): split / refuse / truncate per target, shown
honestly before sending; (4) relay between networks: opt-in per channel pair, who may enable it, prefix on the far side
("name via Meshtastic:" costs bytes), loop guard (bridge tag + id map + never re-relay a relayed message + bridge-own echo
suppression), fan-out of a 111 message into a foreign network only by the user's explicit act; (5) consent and privacy
(relaying a public Meshtastic channel into LXMF; private DMs never auto-relayed); (6) airtime honesty (no policy limits
of ours; show duty-cycle facts; what the firmware itself refuses); (7) where the bridge core lives: in 111d (Rust) with
sidecars as pure transports vs in each sidecar vs adopting matterbridge as the core with custom Meshtastic/LXMF backends —
trade-offs for size, Termux, testability in netns without hardware; (8) agent tools (bridge.read, messages.send with a
target network — proposal-gated) and the additive API/SSE contract shape; (9) delivery states for foreign targets (what
"delivered" can honestly mean per network). Report path: ${OUT}/04-bridge-design.md`,
  },
  {
    key: 'transports', model: 'sonnet', effort: 'high',
    prompt: `LENS 5 — TRANSPORTS AND THE DEVICE CORE, SCOPED WITHOUT HARDWARE.
The opener brief ${ROOT}/plan/cross-layer/devices.md (read §2, §3, §7, §8, §9) lists the device-connection core: uevent
hotplug + port ownership + consented identify (RNode via rnodeconf --info, Meshtastic via meshmud — built by L5 —,
MeshCore companion), state/interfaces.toml + an rnsd renderer v2 (RNodeInterface stanzas, posture, LAN TCP server),
agent tools + a narrow applier, phone relay v1 (TCP to hub, QR join), and the Layer 5 link seams (kiss-modem = our 111
modem on MeshCore KISS firmware per D19 c; meshtastic-client; rns-bridge; ble later). The owner has NO hardware yet.
Research and pin with URLs: (1) MeshCore as of today: does the KISS/raw-modem firmware profile D19 c relies on exist in the
upstream repo (name it: env, docs, frame format, SetRadio, airtime report), release and activity in 2026, the data-type
allocation file for our GRP_DATA range, companion protocol changes; flag anything that undermines D19 c. (2) Reticulum
side as of today: RNode firmware + rnodeconf offline install (--use-extracted, image pinning), TCPServerInterface /
TCPClientInterface / AutoInterface facts for the phone-relay pattern, any 2026 changes (Rust rnsd? new interfaces),
Columba status (releases, license, BLE). (3) Meshtastic firmware and client changes in 2026 that touch us (TCP/BLE
client API, Linux-native meshtasticd, new rate limits). (4) Android reality for the phone relay: Termux 2026 status on
Android 14–16 (background limits, phantom process killer), the MulticastLock gap, helper-APK options (usb-serial-for-android,
LocalOnlyHotspot) — adopt vs build, one paragraph each. (5) What is buildable AND verifiable on a hardware-less box: for
each core item say how it is tested in netns with stubs (a software KISS modem on a pty, a fake rnodeconf, a stub meshmud,
rnsd TCP server/client between namespaces, rendering interfaces.toml → rnsd config and parsing it with rnsd --exampleconfig
or a dry run) and which items are blocked until the owner's test setup exists. (6) Deliver a sequencing recommendation
for the Layer 6 build: transport items first vs bridge items first, with the dependency reasons, and 2–3 options for
what the first signed Layer 6 release must contain. Report path: ${OUT}/05-transports-devices.md`,
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
    findings: { type: 'array', items: { type: 'string' }, description: 'facts that change the plan, each with a URL or file:line' },
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
log('Layer 6 research: seams scout + 5 lens scouts on disjoint questions')
const scouts = await parallel(LENSES.map(l => () =>
  agent(CTX + '\n\n' + l.prompt, { label: `scout:${l.key}`, phase: 'Scout', schema: SCOUT_SCHEMA, model: l.model, effort: l.effort })
    .then(r => r && { ...r, key: l.key })
))
const done = scouts.filter(Boolean)
log(`${done.length}/${LENSES.length} scouts returned`)
if (done.length < LENSES.length) log('missing: ' + LENSES.filter(l => !done.find(d => d.key === l.key)).map(l => l.key).join(', '))

phase('Critique')
const criticPrompt = `${CTX}

YOU ARE THE COMPLETENESS CRITIC for the Layer 6 research sweep. Six scouts wrote reports in ${OUT}/ (read every file there
in full: 00-code-seams.md, 01-meshtastic-bridge.md, 02-lxmf-bridge.md, 03-phone-apps.md, 04-bridge-design.md,
05-transports-devices.md). Their structured summaries: ${JSON.stringify(done.map(d => ({ key: d.key, headline: d.headline, recommendation: d.recommendation, open_questions: d.open_questions, findings: d.findings })), null, 1)}

Check the sweep against what an OPTIONS BRIEF for the owner needs: the owner's D23 goal (send one message into several
networks; receive from other networks; bridge between them; per-network categories in Messages; foreign UIs stay panels),
README §6 (thin bridges, minimal common format, never reimplement radio or routing), the Layer 5 §8 link seams and §19
hand-offs, the opener brief §3/§8/§9, D17 (12 device decisions), D19 c (LoRa ownership), D22 (GPL boundary), the
Reticulum rule, the no-hardware constraint, and "adopt, don't reinvent". Ask: which modality was not run, which claim is
unverified where it matters (a license, an API that may not exist, a size or a firmware limit that decides a fork), which
candidate was missed (name it), which cross-report conflict must be resolved before the owner can choose, where did a
scout recommend custom code although an adoptable project exists? Also check the sweep for honesty about the phone-apps
tier and about what cannot be verified without hardware. You may do targeted web checks yourself (ToolSearch
select:WebSearch,WebFetch) to settle a conflict cheaply, but do not redo the sweep. Rate each gap blocking (the owner
cannot choose without it) / major / minor and write a complete scout prompt for each blocking or major gap. Write your
critique to ${OUT}/06-critic.md (≤ 1500 words) and return the structured verdict.`
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

GAP-FILL SCOUT. The completeness critic found this blocking gap in the Layer 6 sweep (existing reports are in ${OUT}/;
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
