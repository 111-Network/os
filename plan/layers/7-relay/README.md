# Layer 7 — Relay & Presence: plan folder

**Status** (copied from [`INDEX.md`](../../INDEX.md) row 7, which stays the source of truth): 🔬 researching — kickoff 2026-11-01
(cloud session, branch `claude/layer-6-implementation-006mte`): the research sweep (`runs/workflows/research.js`: opus seams scout,
five sonnet lens scouts, opus completeness critic, up to two gap-fill scouts) writes `research/00`–`07`; the lead then writes
`options.md`; **the owner decides before any Layer 7 code is written** (`EXECUTION.md`: research → options brief → decision →
adversarial pass → build).

**What the layer is:** `os/README.md` §7 (heartbeats, gateways, the adopted asynchronous store-and-forward transport, dynamic
delivery paths) and "Category 5: Global Gateways" (an overlay role: 5A intermittent, 5B permanent). Layer 7 runs on every
device category, Cat 1 included.

**What it inherits (recorded hand-offs):** Layer 1 — gateway re-evaluation, KB git-sync, manifest gossip (`1-bootstrap/README.md`
"Updates", §cross-layer) · Layer 2 — the mesh-epoch source (§13), attestation gossip and gateway publication (§10), announce-as-routing
vs identity-only, the Cat-1 announce-verification and rate-limit policies · Layer 3 — the Service Discovery Registry (hub role,
model, load) for offload · Layer 4 — Nearby radar and map views, gateway/transport detail panels (§6.4) · Layer 5 — presence from
announces and `nearby` sources, hub-to-hub replication A+ (§4 summary vectors, §21), `BROADCAST` 0x30, `KB`/`UPDATE` consumers,
`appdata` TLVs, multi-hop discovery (§19, §21) · Layer 6 — runtime join-in-sequence and fall-back through the preferred list,
presence (NodeInfo and Reticulum announces already feed Nearby), the hub-learned `custody_seq_floor`, v1.1 hub-member routing of
foreign DMs (§18); D19 c: the Cat-1 relay is the stock MeshCore repeater.

**Rules that bound it:** the Raspberry Pi floor (`AGENTS.md`: nothing resident, periodic or heavier in the required configuration
without the owner's say — README §7's "continuous heartbeats" is put to the owner with numbers, not assumed) · Reticulum stays a
separate program (`LICENSING.md`) · D22 GPL-3.0-or-later linking boundary · offline-first, no wall clock in crypto, no policy
limits of our own · no hardware yet (D23 #4).

**Decisions:** none yet. The kickoff itself is recorded in `DECISIONS.md` once the owner answers the options brief (the next free
number after D29).

## Files

| Path | What |
|---|---|
| [`research/coordinates.md`](research/coordinates.md) | Resume log: run ids, script paths, lens table, box and network notes. |
| `research/00-code-seams.md` | As-built seam map: where Layer 7 plugs into mesh111, node111, the UI, the agent (written by the sweep). |
| `research/01-presence-discovery.md` | Lens 1: presence, heartbeats vs event-driven announces (with the airtime arithmetic), discovery, the Service Discovery Registry, the mesh-epoch source. |
| `research/02-gateways-global.md` | Lens 2: the gateway role and the global store-and-forward carrier candidates (Nostr relays, Reticulum over IP, iroh, chatmail, Tor/arti, dtn7, MQTT, a 111-run rendezvous). |
| `research/03-fabric-relay.md` | Lens 3: relay inside the 111 fabric: Reticulum as router vs anti-entropy vs IP mesh routing vs a hub-centric star; the mesh111 amendments each needs. |
| `research/04-kb-updates.md` | Lens 4: signed releases and the KB from a hub with no internet; git bundles; manifest gossip vs hub-as-origin; trust model. |
| `research/05-ui-consent-floor.md` | Lens 5: Nearby radar/map (offline maps), presence UI, consent and privacy, the Pi-floor weight table of every candidate program. |
| `research/06-critic.md` | Completeness critique of 00 to 05, with the unverified-claims list (this box's network policy denies most doc sites). |
| `research/07-gap-N.md` | Gap-fill reports (cap 2), if the critic found blocking gaps. |
| `options.md` | The options brief for the owner (written by the lead after the sweep). |
| [`runs/workflows/`](runs/workflows/) | Workflow scripts, one per run (`research.js` first); run ids in `research/coordinates.md`. |

Workflow scripts are launched by their path under `runs/workflows/`; `research.js` hard-codes `ROOT = /home/user/os` (the cloud box).
