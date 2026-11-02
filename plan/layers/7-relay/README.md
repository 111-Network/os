# Layer 7 — Relay & Presence: plan folder

**Status** (summarised from [`INDEX.md`](../../INDEX.md) row 7, which stays the source of truth): 🔨 building — kickoff 2026-11-01
(cloud session, branch `claude/layer-6-implementation-006mte`): the research sweep ran (`runs/workflows/research.js`: opus seams scout,
five sonnet lens scouts, opus completeness critic, two sonnet gap-fill scouts; `research/00`–`07`), the lead wrote [`options.md`](options.md),
the owner decided on 2026-11-01 (D30: every pick and default of the brief); the spec `os/7-relay/README.md` was drafted (`runs/workflows/spec.js`),
panelled by three fable judges (`review/custody.md`, `carriers.md`, `integration.md`), triaged by the lead (`review/triage.md`, D31), amended to
v0.2 (`runs/workflows/spec-fix.js`) and re-checked (`review/recheck.md`: lock); **locked v1.0 on 2026-11-02 (D32)**; the build runs per
`os/7-relay/TASKS.md` (`EXECUTION.md`). The six D31 owner forks are applied as the lead recommends until the owner answers (DECISIONS Pending).

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
| `research/07-gap-1.md` | Gap-fill G1: the adopted-carrier sweep re-scored with the KB gates (ntfy, Yggdrasil/yggstack, WireGuard, Headscale/Tailscale) and the internet link's security costed (TLS-RPK via rustls, `snow`, a tunnel). |
| `research/07-gap-2.md` | Gap-fill G2: the one cross-hub custody shape (single-copy push hand-over, the `fwd` column, the home-hub hint TLV), its bytes and resident cost, the twelve additive Layer 5 amendments. |
| `options.md` | The options brief for the owner (written by the lead after the sweep). |
| [`runs/workflows/`](runs/workflows/) | Workflow scripts, one per run (`research.js` first); run ids in `research/coordinates.md`. |

Workflow scripts are launched by their path under `runs/workflows/`; `research.js` hard-codes `ROOT = /home/user/os` (the cloud box).
