# Layer 8 — Storage & Shared State: plan folder

**Status** (summarised from [`INDEX.md`](../../INDEX.md) row 8, which stays the source of truth): 🔬 researching — kickoff 2026-11-02
(cloud session, branch `claude/layer-6-implementation-006mte`; the owner: "start layer 8 and 9 research in parallel now and document
all"): the research sweep (`runs/workflows/research.js`: an opus seams scout, five sonnet lens scouts, an opus completeness critic, up to
two gap-fill scouts; reports `research/00`–`07`) runs beside Layer 7's spec work; then the lead writes `options.md`; **the owner decides
before any Layer 8 code is written** (`INDEX.md` "Process (per layer)": research → options brief → decision → adversarial pass → build).

**What the layer is:** `os/README.md` §8, verbatim: "Each node keeps an encrypted local store, pruned by capacity rather than by
imposed rules. Knowledge hubs host the large shared datasets: offline encyclopedias, regional maps, software updates, and network
troubleshooting patches. A lightweight append-only shared log may be used for the small subset of state that benefits from
collective consistency — adopted from a proven project, never a custom consensus implementation." Runs natively on Cat 2B, 4A, 4B;
partial or delegated on 2A and 3; not on Cat 1.

**What it inherits (recorded hand-offs):** Layer 3 — "State ← Layer 8": conversations, the task journal and proposals live under
`/var/lib/111/agent/` in the encrypted local store, capacity-pruned, never policy-expired (`3-intelligence/README.md` Contracts) ·
Layer 4 — the Storage L8 settings page is a structural stub; "storage (L8)" in the deferred list (`4-interface/README.md`) · Layer 5 —
the SQLCipher store with capacity pruning classes (§10.3–10.4), Tier 2 object transfer (`OBJ_ADVERT`/`OBJ_REQ`/`OBJ_CHUNK`, bao-tree
verified chunks, `fabric/blobs/<hash>`, specified for v1.1), the `SYNC` class with per-peer cursors for an append-only log ("opaque,
idempotent ids — seam only", §19, §21) · Layer 7 (D30) — the hub serves signed release files as an update origin (F6; the same object
store would serve datasets; who signs Layer 8 datasets is Layer 8's question, noted as the publisher's key), the map bucket waits for
"a region file" from Layer 8 (F9; the D21 exempt-bucket amendment is recorded), Kiwix/ZIM and PMTiles sizes were noted but unverified
(`plan/layers/7-relay/research/05-ui-consent-floor.md`, `06-critic.md` §6) · Layer 1 — the KB ships inside the release payload (v1),
the installer's on-disk layout and ownership rules (`1-bootstrap/README.md` "On-disk layout").

**Rules that bound it:** the Raspberry Pi floor (`AGENTS.md`: nothing resident, periodic or heavier in the required configuration
without the owner's say; a dataset server is an on-demand component that weighs nothing while off) · adopt, don't reinvent (the KB's
adopt-vs-build gate, `0-knowledge/README.md`) · D22 GPL-3.0-or-later linking boundary; dataset licences (ODbL, CC BY-SA, …) are
their own matter · offline-first, no runtime downloads without consent, no telemetry, no policy limits of our own (pruning by
capacity, never by rule) · Reticulum stays a separate program · no hardware yet.

**Decisions:** none yet. The kickoff is recorded in `DECISIONS.md` once the owner answers the options brief.

## Files

| File | What |
|---|---|
| [`research/coordinates.md`](research/coordinates.md) | Resume log: run ids, script paths, lens table, box and network notes. |
| `research/00-code-seams.md` | As-built seam map: the SQLCipher store and its prune classes, `fabric/blobs`, the agent state dir, the release payload and KB on disk, the hub serving surface, the UI budget (written by the sweep). |
| `research/01-datasets-hub.md` | Lens 1: the knowledge hub's datasets: offline encyclopedias (Kiwix/ZIM), regional maps (PMTiles, OSM extracts), updates and patches as datasets, manifests and signing, catalogues, licences, sizes, the Pi floor. |
| `research/02-object-transfer.md` | Lens 2: moving large objects between nodes: Tier 2 over the fabric (bao/blake3), iroh-blobs, rsync/zsync-style deltas, resumability, verification; LAN/IP vs LoRa. |
| `research/03-shared-log.md` | Lens 3: the "lightweight append-only shared log": CRDT and feed candidates (Automerge, Loro, Yjs, Hypercore, SSB), what state qualifies, the `SYNC` seam, what must never be in it. |
| `research/04-local-store.md` | Lens 4: the encrypted local store: SQLCipher as built, capacity pruning, disk budgets per category, backup/restore, Android/Termux encryption at rest, the store API other layers use. |
| `research/05-ui-consent-floor.md` | Lens 5: the Storage page, dataset catalogue and downloads with consent (metered links), hosted panels (Kiwix), the Pi-floor weight table of every candidate program. |
| `research/06-critic.md` | Completeness critique of 00 to 05, with the unverified-claims list (this box's network policy denies most doc sites). |
| `research/07-gap-N.md` | Gap-fill reports (cap 2), if the critic found blocking gaps. |
| `options.md` | The options brief for the owner (written by the lead after the sweep). |
| [`runs/workflows/`](runs/workflows/) | Workflow scripts, one per run (`research.js` first); run ids in `research/coordinates.md`. |
