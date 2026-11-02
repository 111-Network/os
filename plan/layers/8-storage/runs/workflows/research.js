export const meta = {
  name: 'layer-8-research',
  description: 'Layer 8 (Storage & Shared State) research sweep: as-built seams scout + 5 lens scouts, completeness critic, gap-fill',
  phases: [
    { title: 'Scout', detail: '1 opus seams scout + 5 sonnet lens scouts, web-verified where the box allows, reports on disk' },
    { title: 'Critique', detail: 'opus completeness critic over all reports' },
    { title: 'Gap-fill', detail: 'up to 2 scouts on blocking gaps' },
  ],
}

const ROOT = '/home/user/os'
const OUT = ROOT + '/plan/layers/8-storage/research'

const CTX = `
You are a research scout for the 111network OS build (repo root: ${ROOT}). Today is 2026-11-02.

WHAT 111 IS: a minimal operating layer that turns any device (old phone, laptop, Pi, LoRa board) into a node of a resilient
offline mesh. Principle, non-negotiable: ADOPT, DON'T REINVENT. Existing projects are assets we run, bridge and show; custom
code is only glue. A local AI agent is the interface and the self-healer. Read ${ROOT}/README.md first: the section
"8. Storage & Shared State" is the layer you research. Verbatim: "Each node keeps an encrypted local store, pruned by
capacity rather than by imposed rules. Knowledge hubs host the large shared datasets: offline encyclopedias, regional maps,
software updates, and network troubleshooting patches. A lightweight append-only shared log may be used for the small
subset of state that benefits from collective consistency - adopted from a proven project, never a custom consensus
implementation." Also read "Category 4: Local Hubs" (4A knowledge hubs host datasets) and the layer-category table: Layer 8
runs natively on Cat 2B, 4A and 4B, partially or delegated on 2A and 3, not on Cat 1. Layer 7 is the neighbour below (decided,
spec in draft at ${ROOT}/7-relay/README.md if that file exists today - read it if it does; it is a DRAFT, not locked), Layer 9
(self-healing) the neighbour above (its research runs beside yours; do not research it).

WHAT IS ALREADY BUILT (Layers 0-7; do not re-research them, wire to them; cite file:line when you point at code):
- L0 Knowledge base ${ROOT}/0-knowledge/ (entries with YAML frontmatter; the rubric candidate -> testing -> proven and the
  adopt-vs-build gate in its README; navigation contract ${ROOT}/0-knowledge/AGENTS.md; the index is built by _meta/build.py).
  Entries you may cite and must not contradict without a fresher source: concepts/{store-and-forward-states,secure-contexts,
  preferred-networks}, protocols/{bundle-protocol-v7}, projects/{automerge,blake3-bao,iroh-nodeid,dtn7-rs,leviculum,age,
  ciborium,briar-mailbox,home-assistant-ingress,meshchatx,meshtastic-web,deltachat-rpc-server,madmail,opencode,llama-cpp}.
- L1 Bootstrap (${ROOT}/1-bootstrap/README.md): the Rust daemon "111d" (crate node111 at ${ROOT}/1-bootstrap/crates/node111)
  with an on-disk layout under /var/lib/111 whose ownership is the security boundary ("On-disk layout"); the release payload
  (binary, notices, the KB snapshot) staged and promoted by a root helper with probation and rollback (update_guard.rs,
  probation.rs, update_helper.rs); the installer categorizes the device; Android phones join via Termux (no root, no
  multicast receive, app-private storage).
- L2 Identity (${ROOT}/2-identity/README.md): Ed25519 + X25519 from one seed; backup/restore of the seed (SSKR) is specified
  there - the STORE's encryption key derivation is for you to find in the code (grep the mesh111 store for the key).
- L3 Intelligence (${ROOT}/3-intelligence/README.md): "111-agent" with fixed tools; its contract "State <- Layer 8":
  conversations, the task journal and proposals live under /var/lib/111/agent/ in the encrypted local store,
  capacity-pruned, never policy-expired; KB grounding reads the llms.txt index.
- L4 Dashboard (${ROOT}/4-interface/README.md; contract ${ROOT}/4-interface/api/openapi.yaml): React UI embedded in 111d,
  /api/v1 + one SSE stream; the Storage L8 settings page is a structural stub; hosted panels (Layer 6) are served through
  a panel proxy with its own listener rules (${ROOT}/1-bootstrap/crates/node111/src/daemon/panels/); static files with Range
  support exist in panels/static.rs (verify); UI bundle budget D21 (256 KB eager + lazy buckets; an exempt "map" bucket is
  recorded as a Layer 7 amendment for Layer 8's region files).
- L5 Messaging & Data (${ROOT}/5-messaging/README.md, locked, built): the fabric "mesh111" (${ROOT}/5-messaging/crates/mesh111)
  inside 111d with ONE encrypted SQLCipher store (schema V1-V3 built, V4 by Layer 7; section 10.3-10.4: prune classes,
  the capacity-pruning order, quotas - read store/prune.rs and store/rows.rs), sealed envelopes, custody at a home hub,
  Tier 2 object transfer SPECIFIED for v1.1 (OBJ_ADVERT/OBJ_REQ/OBJ_CHUNK, bao-tree verified chunks, a staging path
  fabric/blobs/<hash>, the tcp-lan chunk frame reserved - read section 19 and grep envelope.rs for the kinds), the KB/UPDATE/
  PATCH/SYNC frame classes defined with consumers in Layers 7-9; the SYNC class "for Layer 8's append-only log (opaque,
  idempotent ids, per-peer cursors - seam only)" (section 19, section 21). The fabric is connection-oriented, tcp-lan only
  (${ROOT}/plan/layers/7-relay/research/00-code-seams.md is the latest line-verified seam map; read its section on the store).
- L6 Transport & Bridge (${ROOT}/6-transport/README.md, locked D29): optional components (manifest-pinned payloads cached
  under cache/packages/<id>/, enable-time install, zero weight while off: the pattern any adopted dataset server follows),
  hosted panels (Meshtastic web, MeshChatX) behind the panel proxy, the Delta Chat core with its own database, the Pi floor
  measured (111d idle 44.7 MB debug / 19.7 MB release; nothing resident added).
- L7 Relay & Presence (DECIDED, ${ROOT}/plan/DECISIONS.md D30; spec draft in progress): the hub serves signed release files
  as an UPDATE ORIGIN on the existing dashboard listener while hub_role is on (an origin is a set of files: a channel
  manifest, its .minisig, blob/<sha256>; '111 update --from <dir|url>'); who signs Layer 8 datasets served from the same
  object store is YOUR question (Layer 7 noted "the publisher's key"); the map needs "a region file" from Layer 8 and a Range
  tile route (Leaflet + PMTiles, an exempt bundle bucket); a metered-link setting with a daily byte cap exists in Layer 7's
  v1; a carrier (ntfy) moves sealed frames only, never datasets; Tier 2 transfer is on Layer 7's v1.1 list - say who should
  own it. Read ${ROOT}/plan/layers/7-relay/options.md section 0 facts 12-15, 18, 21-22 and D30 F6, F9, F10.
- Parked notes ${ROOT}/plan/cross-layer/parked.md; decisions ${ROOT}/plan/DECISIONS.md D17, D20, D21, D22, D24, D29, D30; the
  plan protocol ${ROOT}/plan/EXECUTION.md; the Layer 7 options brief ${ROOT}/plan/layers/7-relay/options.md is the SHAPE the
  lead will write for this layer (settled facts -> polled forks with picks -> plan corrections): write so it can be written.

RULES (non-negotiable):
1. Reticulum (RNS/LXMF) is a connected service used as released, run as its own program; no Reticulum code in our Rust tree
   (${ROOT}/LICENSING.md). Any adopted dataset server, sync daemon or tile server is likewise a separate on-demand program or
   a vetted crate behind our seam, never a fork.
2. D22: our code is GPL-3.0-or-later; GPL-3.0 code may link into our binary; every new dependency needs a sign-off row in
   LICENSING.md; incompatible licenses (GPL-2.0-only, CC-BY-NC, SSPL, BUSL, PolyForm, the Reticulum License) stay out of the
   binary. DATASET licences are a separate matter (Wikipedia CC BY-SA 4.0, OpenStreetMap ODbL, ...): state the obligations
   a hub operator takes on by serving them (attribution, share-alike, the ODbL produced-work rule) - facts, not advice.
3. THE RASPBERRY PI IS THE FLOOR (owner rule, AGENTS.md): the required configuration stays super light; everything
   third-party is a separate on-demand component that weighs NOTHING while off (no process, listener, loaded code, timer or
   open file). No task may add a resident process, a periodic poll, a runtime dependency or build-time weight to the
   required configuration WITHOUT THE OWNER'S SAY. A hub hosting datasets is opt-in; quantify every resident cost (RSS,
   binary, listeners, timers, disk writes).
4. Our identity and the Layer 5 envelope are fixed. No wall clock in crypto or ordering (mesh epochs and monotonic counters
   only). Offline-first, no runtime downloads without the user's consent, no telemetry, no policy limits of our own (pruning
   by CAPACITY, never by an imposed expiry - README section 8 says so twice), delivery first.
5. Device floor: Linux x86_64/aarch64 (musl-static Rust binary) and Android via Termux (aarch64, no root, no FUSE, app-private
   storage, background limits). SD cards on a Pi wear: write amplification is a cost to state.
6. License gate for adoption: verify every license from the upstream LICENSE file, never from memory. Note release recency
   with dates. Mark anything you could not verify "(unverified)". Your training data is stale: fetched facts win.
7. Sizes are facts: give dataset sizes with the source URL and the date (a ZIM or PMTiles file size, a region extract).
8. No hardware is available; nothing radio-side matters here except that LoRa never carries a dataset (airtime arithmetic is
   in ${ROOT}/0-knowledge/concepts/lora-airtime-budget.md).

THIS BOX'S NETWORK (read before you fetch; it decides your method): outbound HTTPS goes through a policy proxy. REACHABLE:
raw.githubusercontent.com (any public repo's files: LICENSE, README, docs/, source - use this for every GitHub-hosted
project), pypi.org and files.pythonhosted.org (package JSON at https://pypi.org/pypi/<name>/json), crates.io and
index.crates.io (https://crates.io/api/v1/crates/<name>), registry.npmjs.org, proxy.golang.org. DENIED (a fetch fails at
CONNECT with 403 or times out - do not retry): docs.rs, rfc-editor.org, github.com HTML pages, api.github.com, most project
sites (wiki.openstreetmap.org, download.kiwix.org, protomaps.com are probably denied: try once each). WebSearch works (it
runs outside the box) and returns snippets with URLs: cite the URL and mark a claim "(search snippet)" when you could not
fetch the page. For a GitHub project, derive the raw URL (https://raw.githubusercontent.com/<owner>/<repo>/<default-branch>/
<path>; try main then master) and read the LICENSE and README from there. One failed fetch per host is enough.

METHOD (mandatory unless your lens says "no web"):
- First call ToolSearch with query "select:WebSearch,WebFetch" to load the web tools, then research on the live web within
  the network note above. Prefer upstream repos and package indexes. Record the exact URL for every factual claim. Fetch the
  LICENSE file for every candidate you rate "adopt" or "bridge".
- Read only the repo sections named in your lens (sed -n / grep / rg); do not slurp whole trees. Cite file:line.
- Do NOT write code and do NOT modify any repo file except your own report path. Do NOT run cargo, pnpm, 111d, rnsd or any
  server. Nothing of yours may be left running.
- Cost discipline: aim for at most 40 web fetches; stop when the recommendation is grounded.

REPORT (write it with the Write tool, markdown, at most 3500 words, to the path given in your lens; then return the
structured summary). Sections, in order: 1. Summary (at most 6 bullets) - 2. Candidates table (name - upstream - license
(verified?) - language/runtime - maturity signals (last release with date, activity, adopters) - resident weight (RSS /
binary / listeners / timers / disk, measured or published, or "unknown") - fit: adopt|bridge|reference|reject - why) -
3. Findings that change the plan (numbered facts with URLs or file:line) - 4. Recommendation: where a genuine fork exists,
give 2-4 options with one-line trade-offs and your pick - 5. Open questions for the owner - 6. Sources.
`

const LENSES = [
  {
    key: 'seams', model: 'opus', effort: 'high',
    prompt: `LENS 0 - AS-BUILT CODE SEAMS (no web; read code; every claim line-verified with file:line).
Map exactly where Layer 8 plugs into the built tree, so the options brief and the build briefs can cite real seams. Read,
in this order, only what you need of: ${ROOT}/5-messaging/crates/mesh111/src/store/** (the SQLCipher store: how the key is
derived and held, the schema versions and migrations, every table and its prune class, store/prune.rs: the capacity order,
quotas, what "capacity" is measured against, the evict-first tiers; rows.rs admit/quota; the custody and foreign tables
Layer 6 added) and ${ROOT}/5-messaging/crates/mesh111/src/{envelope.rs,fabric.rs,frag.rs} for the OBJ_*, KB, UPDATE, PATCH
and SYNC kinds (defined? produced? consumed?), the reserved tcp-lan chunk frame and the Tier 2 plan of
${ROOT}/5-messaging/README.md section 19; ${ROOT}/1-bootstrap/crates/node111/src/{install/**,update_guard.rs,
update_helper.rs,probation.rs,manifest.rs,config.rs} and src/daemon/{mod.rs,layout*,paths*} for the on-disk layout
(/var/lib/111/{state,current,previous,staging,cache,agent,rns,fabric,...}: who owns what, modes, what is encrypted at rest and
what is plaintext, where the KB snapshot lands, how big the payload is), ${ROOT}/1-bootstrap/crates/node111/src/daemon/
{dashboard/**,panels/**} for static and Range serving (panels/static.rs: Range, size caps, auth posture), the hosted-panel
proxy and the components cache (components/**: cache/packages/<id>, manifests, enable-time install - the pattern a dataset
server component would reuse), ${ROOT}/1-bootstrap/crates/node111/src/daemon/intelligence/** and ${ROOT}/3-intelligence/
crates/agent111/src/** for the agent's state directory (conversations, task journal, proposals: format, pruning, encryption
or not), ${ROOT}/0-knowledge/_meta/build.py (what the node has of the KB on disk; is Python on the node?), ${ROOT}/4-interface/
api/openapi.yaml and ${ROOT}/4-interface/ui/src (the Storage settings stub, status fields about disk, the budget rule in
scripts/check-budget.mjs), ${ROOT}/1-bootstrap/Cargo.lock (which of blake3, bao-tree, iroh*, automerge, rusqlite/sqlcipher,
zstd, tar, flate2 are in the lock already - exact versions), and ${ROOT}/7-relay/README.md sections on the update origin and
the map bucket if the file exists. Also list EVERY timer/tick/poll in the required configuration today (the Layer 7 seams
report ${ROOT}/plan/layers/7-relay/research/00-code-seams.md section 3.1 lists them: re-verify, do not redo) and every
place disk usage is measured or reported today.
Deliver ${OUT}/00-code-seams.md with per-subsystem tables (file:line - what exists - the seam - what Layer 8 must add or
change) for six needs: (a) the encrypted local store: what is encrypted, how the key is derived, capacity pruning as built,
quotas, write patterns (WAL, fsync, SD wear), what other layers store outside it in plaintext; (b) dataset hosting on a hub:
the serving surface, Range, auth posture, disk layout, what a "dataset object" and a catalogue would need; (c) object
transfer: Tier 2 as specified versus built, the chunk frame, bao/blake3 in the lock, the update origin's blob/<sha256>
shape as the simplest transfer; (d) the SYNC seam: the classes, per-peer cursors, what the store can hold for a log;
(e) agent and task state and proposals storage; (f) the UI: the Storage stub, status fields, where a catalogue and a
download consent dialog would live, the budget. Then "traps" (things that look like seams but are not), the current test
counts per crate (read the board summaries; run nothing), and open questions. Return the structured summary (candidates
may be empty).`,
  },
  {
    key: 'datasets-hub', model: 'sonnet', effort: 'high',
    prompt: `LENS 1 - THE KNOWLEDGE HUB'S DATASETS: ENCYCLOPEDIAS, MAPS, UPDATES, PATCHES.
README section 8: "Knowledge hubs host the large shared datasets: offline encyclopedias, regional maps, software updates,
and network troubleshooting patches." Research each dataset class with sizes, licences (verified from LICENSE files where
you rate adopt), formats, the program that serves it and its resident weight on a Pi, and whether it can be an on-demand
zero-weight component: (1) OFFLINE ENCYCLOPEDIAS: the ZIM format (openZIM), libzim and libkiwix (licences), kiwix-serve and
kiwix-tools static binaries (aarch64? musl? size, RSS while serving, listeners), ZIM sizes with dates (Wikipedia English
nopic/maxi, Simple English, a medical selection, Wiktionary; what fits a 32 GB SD card), zimit for custom bundles, the
Kiwix library catalogue format (library.xml / OPDS), alternatives (Xowa; a plain static mirror) and what a Termux phone can
host; (2) REGIONAL MAPS: Protomaps PMTiles basemaps (format licence BSD; data ODbL; per-region build sizes with dates;
pmtiles extract), Geofabrik and other OSM extracts, tile servers (tileserver-gl weight; martin (Rust, licence?); a Range
route in 111d serving a PMTiles file needs no server - Layer 7 recorded that), Organic Maps data as a reference, offline
routing (reject or reference), what the dashboard map (Leaflet + PMTiles + protomaps-leaflet) needs from the hub; (3)
SOFTWARE UPDATES as datasets: Layer 7's update origin already serves signed release files from the hub - say what Layer 8
adds (a catalogue, mirrors, hub-to-hub replication of the payload) and what it must not duplicate; (4) NETWORK
TROUBLESHOOTING PATCHES: what the dataset is (recipes and KB entries, Layer 9's PATCH artefacts) and how it is served and
versioned; (5) DATASET MANIFESTS AND SIGNING: who signs a dataset the hub serves (the release key - no; the hub owner's
node key; the publisher's key; a checksum only) - compare with TUF roles, apt Release files, F-Droid index signing, Kiwix's
own checksums; a catalogue format the UI and the agent can read (sizes, licences, versions, hashes, attribution text);
(6) LICENCE OBLIGATIONS for a hub operator serving Wikipedia (CC BY-SA 4.0) and OSM (ODbL) extracts: attribution and
share-alike facts, the ODbL produced-work notice - cite the licence texts; (7) STORAGE BUDGETS per device category (a
phone's app-private storage, a Pi's SD card, a hub's disk) and which dataset classes run where (the layer-category table
says 2B native, 2A/3 delegated). Give 2-4 options per fork (serve ZIM through kiwix-serve as a hosted panel vs a static
export; PMTiles via a Range route vs a tile server; publisher-signed catalogues vs checksums) with picks. Deliver
${OUT}/01-datasets-hub.md.`,
  },
  {
    key: 'object-transfer', model: 'sonnet', effort: 'high',
    prompt: `LENS 2 - MOVING LARGE OBJECTS BETWEEN NODES AND BETWEEN HUBS.
Datasets are tens of MB to tens of GB; the fabric's sealed frames fragment at 160 B and LoRa never carries a dataset
(concepts/lora-airtime-budget.md). Layer 5 specified Tier 2 object transfer for v1.1 (OBJ_ADVERT/OBJ_REQ/OBJ_CHUNK, bao-tree
verified streaming, fabric/blobs/<hash>; read ${ROOT}/5-messaging/README.md section 19 and the seams scout's finding on the
chunk frame if ${OUT}/00-code-seams.md exists); Layer 7 (D30 F6) serves signed release files over plain HTTP with sha256
pins from the hub (the simplest transfer: a GET by hash); Layer 7's v1.1 list carries "Tier 2 transfer" - say who should
own it (Layer 7 or 8) and why. Research adoptable transfer mechanisms with licences (verified), weight and offline fit:
(1) HTTP Range + content hash (what exists; resumable; verification only at the end unless chunked hashes), the bao-tree /
BLAKE3 verified streaming (bao-tree 0.16 pinned, 0.17 shipped: check crates.io; blake3 in the lock?), iroh-blobs (BLAKE3,
QUIC, licence, binary delta, as an optional component; Layer 5 rejected +13 MB for the core: re-weigh for a hub-only
component), (2) delta and content-defined chunking: rsync and zsync, casync/desync, FastCDC crates (fastcdc, cdc), what a
ZIM or PMTiles update looks like as a delta (do these formats delta well? facts), (3) BitTorrent for hub-to-hub and
LAN swarming (rqbit Rust, libtorrent; licences; weight; private trackers vs DHT off), (4) Syncthing (Go, MPL-2.0; block
exchange; RSS on a Pi; as a hosted on-demand component), (5) IPFS/kubo (reject on weight; say why), Dat/Hypercore, (6)
sneakernet: a signed bundle on a USB stick imported by '111 update --from' extended to datasets, (7) hub-to-hub replication
of a dataset catalogue over the fabric's sealed frames (small) plus the bytes over IP (large) - the split. For each: the
verification model (hash at the end vs chunked hashes vs signatures), resumability, NAT (hubs behind NAT reach each other
only through Layer 7's reach layer), LAN vs internet, metered links (Layer 7's byte cap applies to datasets too?), the Pi
floor. Give 2-4 options with picks and name what is v1 (probably: HTTP Range + sha256 catalogue from a hub on the LAN,
sneakernet) and v1.1. Deliver ${OUT}/02-object-transfer.md.`,
  },
  {
    key: 'shared-log', model: 'sonnet', effort: 'high',
    prompt: `LENS 3 - THE LIGHTWEIGHT APPEND-ONLY SHARED LOG.
README section 8: "A lightweight append-only shared log may be used for the small subset of state that benefits from
collective consistency - adopted from a proven project, never a custom consensus implementation." Note the modal verb: MAY.
Layer 5 reserved a SYNC class "opaque, idempotent ids, per-peer cursors - seam only". Research honestly: (1) WHICH STATE
qualifies - go through what the node holds (contact cards and home-hub hints, the registry and presence facts, proposals
and decisions, KB entries and recipes, the dataset catalogue, device journals, relay edges, the agent's task journal) and
say for each whether collective consistency buys anything, what the conflict semantics would be without a wall clock
(Lamport or vector clocks, the mesh epoch, last-writer-wins by counter), and what must NEVER be in a shared log (keys,
messages - custody and ACKs already exist -, anything naming a person without consent); the honest answer may be "nothing
in v1"; (2) CANDIDATES, with licences verified, Rust availability, weight, sync protocol over an opaque transport (our
sealed frames or a tcp-lan stream): Automerge (KB entry; automerge crate; sync messages; document size growth and compaction),
Loro, Yjs/yrs, Hypercore/Holepunch (JS; Rust ports?), Secure Scuttlebutt feeds (append-only signed logs; Rust crates?),
iroh-docs/willow (status), the crdts crate, Syncthing's index exchange as a pattern, a plain signed append-only log
(a hash chain per author, like SSB) that needs no CRDT; (3) STORAGE GROWTH versus capacity pruning (README's rule): an
append-only log and "pruned by capacity" conflict unless compaction or snapshotting is designed in - say how each candidate
handles it; (4) the SYNC seam: per-peer cursors, idempotent ids, sealed over the fabric (small deltas) or over LAN streams;
(5) prior art in offline-first apps (Briar's group sync, Delta Chat has none, Scuttlebutt, Earthstar, Matrix DAG - reference
only); (6) the Pi floor: any candidate as an in-binary crate (binary delta) or a separate program. Give 2-4 options including
"no shared log in v1; the seam stays reserved" with picks. Deliver ${OUT}/03-shared-log.md.`,
  },
  {
    key: 'local-store', model: 'sonnet', effort: 'high',
    prompt: `LENS 4 - THE ENCRYPTED LOCAL STORE, CAPACITY PRUNING, BUDGETS, BACKUP, SD WEAR.
README section 8: "Each node keeps an encrypted local store, pruned by capacity rather than by imposed rules." The built
store is mesh111's SQLCipher database with prune classes and a capacity order (the seams scout maps it; read
${OUT}/00-code-seams.md section (a) if it exists, else ${ROOT}/5-messaging/README.md section 10.3-10.4). Research: (1) what
"the encrypted local store" should cover across layers: the fabric store (encrypted), the agent's conversations, task
journal and proposals (plaintext today? verify from the seams report), foreign-network databases (the Delta Chat core keeps
its own DB), the KB snapshot, receipts and state files, datasets (large, public, plaintext is fine - say so), and whether
each should move into the encrypted store, be file-encrypted (age, rage crate; fscrypt on Linux; Android file-based
encryption already covers app-private storage on Termux - facts), or stay plaintext with the ownership boundary as the
protection; (2) key handling: where the SQLCipher key comes from today, backup and restore of the store (Layer 2 backs up
the seed; is the store recoverable from the seed alone? what is lost), the owner's "100% freedom" and the no-policy rule;
(3) CAPACITY PRUNING as a policy of the whole node: per-class quotas (messages, custody, foreign, datasets, agent state,
logs), a disk budget per device category (a phone's app-private storage, a Pi's SD card, a hub), what the user sees
(usage per class; "this node keeps X of Y"), what the agent may propose, and the rule that nothing is expired by time
(README says twice: capacity, never imposed rules); (4) SD-CARD WEAR on a Pi: SQLite WAL and synchronous settings, checkpoint
cadence, write amplification facts, measured or published numbers (the owner's floor is a Pi); log and journal writes; (5)
integrity: checksums, torn-write recovery (the update guard has a torn-payload guard: reuse the pattern), migrations (V1-V3
built, V4 by Layer 7: how a schema change rolls forward on a node), (6) prior art: Signal and Briar local stores, Delta
Chat's DB, Syncthing's index DB, SQLCipher vs SQLite SEE vs app-level encryption; (7) the store API other layers use (a
mesh111 Store call vs a node111-level store service): what Layer 8 should offer Layer 9 and the agent. Give 2-4 options with
picks. Deliver ${OUT}/04-local-store.md.`,
  },
  {
    key: 'ui-floor', model: 'sonnet', effort: 'high',
    prompt: `LENS 5 - THE STORAGE PAGE, CATALOGUE AND DOWNLOAD CONSENT, HOSTED PANELS, AND THE PI-FLOOR WEIGHT OF EVERY CANDIDATE.
The dashboard has a Storage L8 settings stub (${ROOT}/4-interface/README.md: layer pages, the deferred list "storage (L8)";
read the per-layer settings page pattern and the Layer 6 and Layer 7 settings rows for shape), a hosted-panel pattern
(Layer 6: Meshtastic web, MeshChatX behind the panel proxy), the UI budget D21 with an exempt "map" bucket recorded for
Layer 8's region file (Layer 7 F9), and a metered-link setting (Layer 7). Research: (1) the Storage page: usage per class and
per layer, quotas the user may set, what the agent may propose, honest copy ("pruned by capacity"); (2) a dataset CATALOGUE:
browse what the hub offers (name, size, licence and attribution, version, hash, publisher), download with explicit consent
(size and the metered rule shown first), progress and resumability, where a dataset is used (the map, the encyclopedia
panel, the agent's grounding); prior art UIs (Kiwix's library, F-Droid, Organic Maps' downloader); (3) HOSTED PANELS for
datasets: kiwix-serve's web UI behind the panel proxy (licence, bundle size, auth posture, does it run on a Pi with the RSS
numbers), a map page using Leaflet + PMTiles in the exempt bucket (size the bundle: Leaflet, pmtiles, protomaps-leaflet
versions and gz sizes from npm), the agent's "ask the knowledge" grounding over a ZIM (libzim bindings? a search endpoint?)
as a note; (4) consent and privacy: sharing datasets to LAN peers vs the internet (a hub serving to its members only; the
attribution notice shown), what a hub reveals by hosting (nothing personal), the metered rule; (5) Android/Termux: storage
limits, app-private vs shared storage, Termux:API; (6) THE PI FLOOR for every program the other lenses are likely to
propose: kiwix-serve, kiwix-tools, tileserver-gl, martin, iroh, syncthing, rqbit, an automerge in-binary delta, SQLCipher
(already in), zstd/tar crates: published or measurable RSS, binary size, listeners, timers, disk writes - the one table the
owner needs to say yes or no to anything resident; and which can be an on-demand zero-weight component (Layer 6's seam).
Candidates with licences verified from LICENSE files. Deliver ${OUT}/05-ui-consent-floor.md.`,
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
log('Layer 8 research: seams scout + 5 lens scouts on disjoint questions')
const scouts = await parallel(LENSES.map(l => () =>
  agent(CTX + '\n\n' + l.prompt, { label: `scout:${l.key}`, phase: 'Scout', schema: SCOUT_SCHEMA, model: l.model, effort: l.effort })
    .then(r => r && { ...r, key: l.key })
))
const done = scouts.filter(Boolean)
log(`${done.length}/${LENSES.length} scouts returned`)
if (done.length < LENSES.length) log('missing: ' + LENSES.filter(l => !done.find(d => d.key === l.key)).map(l => l.key).join(', '))

phase('Critique')
const criticPrompt = `${CTX}

YOU ARE THE COMPLETENESS CRITIC for the Layer 8 research sweep. Six scouts wrote reports in ${OUT}/ (read every file there
in full: 00-code-seams.md, 01-datasets-hub.md, 02-object-transfer.md, 03-shared-log.md, 04-local-store.md,
05-ui-consent-floor.md). Their structured summaries: ${JSON.stringify(done.map(d => ({ key: d.key, headline: d.headline, recommendation: d.recommendation, open_questions: d.open_questions, findings: d.findings, unverified: d.unverified })), null, 1)}

Check the sweep against what an OPTIONS BRIEF for the owner needs: README section 8 verbatim (every clause: encrypted local
store, pruned by capacity not rules, knowledge hubs hosting encyclopedias / maps / updates / patches, the optional
append-only shared log adopted never custom), the recorded hand-offs (Layer 3's agent state, Layer 4's Storage stub, Layer
5's Tier 2 transfer and the SYNC seam and the prune classes, Layer 7's update origin and map bucket and the dataset-signing
question), the Pi-floor rule (every resident cost quantified; dataset hosting opt-in), the no-policy rule (capacity, never
expiry), D22 and the dataset licences, "adopt, don't reinvent", the device floor (Termux, SD wear), and this box's network
policy (claims the scouts marked unverified because a host was denied: collect them into one list the lead verifies on the
owner's machine - that is not a gap to re-run here). Ask: which modality was not run, which claim is unverified where it
decides a fork (a licence, a size, an API, a weight), which candidate was missed (name it), which cross-report conflict must
be resolved before the owner can choose (for example who owns Tier 2 transfer, whether the agent's state is encrypted,
whether a shared log exists in v1), where did a scout recommend custom code although an adoptable project exists, and
where did a scout propose something resident without a number? You may do targeted web checks yourself (ToolSearch
select:WebSearch,WebFetch; the network note applies) to settle a conflict cheaply, but do not redo the sweep. Rate each gap
blocking (the owner cannot choose without it) / major / minor and write a complete scout prompt for each blocking or major
gap. List the genuine owner forks you see across all reports. Write your critique to ${OUT}/06-critic.md (at most 1500
words; include the unverified-claims list as its own section) and return the structured verdict.`
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

GAP-FILL SCOUT. The completeness critic found this blocking gap in the Layer 8 sweep (existing reports are in ${OUT}/;
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
