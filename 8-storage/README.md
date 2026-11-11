# Layer 8 - Storage & Shared State: spec v0.1 (draft for the adversarial panel)

The layer that keeps each node's own state encrypted and bounded by the disk the node has, and lets a hub hold the large public files a community shares.
**The node-state store** is a second SQLCipher file that only `111d` opens: the agent's conversations, proposals, tasks and decisions, and Layer 9's repair
records, live there; the agent holds no key and writes nothing over HTTP. **One node budget**, sized from the disk and shown on a Storage page, decides when
copies that can be fetched again give way; nothing is deleted because of its age, and a dataset is never deleted behind the user's back. **A hub** serves
encyclopedias (an adopted kiwix-serve, started on demand), regional maps (PMTiles files behind a `Range` route) and the pinned payloads of the components it
holds, all as files named by their sha256 in the one object tree Layer 7 built for releases, listed in a catalogue the publisher's node key signs, and checked
block by block. v1 adds **no shared log**, and no resident process, listener, timer or crate to the required configuration.

Status: **draft v0.1 (2026-11-11)** for the adversarial panel. Written from D34 (the owner took every pick, detail pick, plan correction and default of the
options brief), D36, the brief (`plan/layers/8-storage/options.md`), its judge's check (`review/options-check.md`: ready), research R00-R07g3 and the as-built
code. Nothing here is built. No Layer 8 code is written before the lock; the build board `8-storage/TASKS.md` follows the lock.

## 0. Reading guide

Reads with: `os/README.md` §8, the layer map row 8, Categories 2A, 2B, 3 and 4A · `os/5-messaging/README.md` §10, §11, §13, §19, §21 (amended six times, §21)
· `os/2-identity/README.md` §6.1, §6.4, §8.5, §13.1 A4, §13.2 A14 · `os/1-bootstrap/README.md` "On-disk layout", "Layer 5 amendments", "Layer 6: optional
components" · `os/6-transport/README.md` §2.1 · `os/7-relay/README.md` §9, §10.5, §12, §17, §18 · `os/3-intelligence/README.md` "State ← Layer 8" ·
`os/4-interface/api/README.md` · `os/plan/DECISIONS.md` **D34** (the law), **D36**, D37 (the seam map), D30-D32 and D29 (the lock precedents) · the options
brief (OB) and research R00 (seams), R01-R05 (lenses), R06 (critic), R07g1-R07g3 (gap-fills) under `os/plan/layers/8-storage/` · the Layer 9 brief F3
(`plan/layers/9-evolution/options.md:159-198`), whose joint contract is stated once, in §3 here.

**Citations.** `M/` = `5-messaging/crates/mesh111/src` · `N/` = `1-bootstrap/crates/node111/src/daemon` · `NI/` = `1-bootstrap/crates/node111/src` · `A/` =
`3-intelligence/crates/agent111` · `UI/` = `4-interface/ui` · `API` = `4-interface/api/openapi.yaml` · `BT/` = the bao-tree 0.16.1 registry source · `L0`…`L7`
= the layer READMEs. Every `file:line` was read on 2026-11-11 at head `f0b6806`. Four files under `M/store/` (`mod.rs`, `prune.rs`, `rows.rs`, `schema.rs`)
carry the uncommitted edits of Layer 7 row M1b (schema v5, `custody_totals`, `7-relay/TASKS.md:58`); every citation into them reads the committed tree (`git
show f0b6806:…`), and the fabric schema is cited as **v4, v5 in flight**. **Evidence:** *(src)* source read · *(doc)* docs read by a scout · *(design)* not
built · *(unverified from this box)* a claim no scout could fetch (R06 §7, OB §6).

**Normative:** numbered rules, byte and path tables, settings with defaults, gate assertions. **Informative:** "why" sentences, arithmetic, cost tables,
§19-§22. **v1 and later** (D34 F13):

| Area | v1 (this release) | Later (recorded, §20) |
|---|---|---|
| Node-state store (§2) | `node/state.db`, 111d the only writer; chat through the relay; the proposal and task spool; the legacy import | Mode T hardware key; a sealed spool |
| Joint contract (§3) | the seven items, the state service API, the paths | the fix-attestation feed (only through §12's gate) |
| Budget (§4) | one `keep` figure, four budgeted classes, the Storage page, the floor | datasets pruned under 1 % free as an opt-in |
| Object tree (§5) | datasets and sidecars in `state/origin/blob/`; the bao-tree outboard | Tier 2 adopts the same roots |
| Signing (§6) | the publisher's node key under `111/catalogue/v1`; home hub plus followed publishers | endorsements (`111/catalogue-vouch/v1`) in the UI |
| Moving datasets (§7) | the Range arm, `get_to_file`, sneakernet, the first copy, consent, the metered window | hub-to-hub bulk over TLS-RPK |
| Tier 2 (§8) | owner named (Layer 8); nothing built | `OBJ_*`, `bao-tree` 0.17, bulk on its own connection |
| Encyclopedias (§9) | the HTTP-only `child` panel source, the kiwix component, `kb.search` | WebSocket/SSE children (P7) |
| Maps (§10) | vector PMTiles in the exempt 90 KB `map` bucket; the Nearby map tab | raster, rebuilt OSM extracts |
| Component pins (§11) | `origin.components`, the installer's origin list, the files table, the release source tarball | the Termux apt directory and the APK index |
| Shared log (§12) | none; the admission gate; the consumer registry | p2panda-core at a first consumer |
| Backup and wear (§13) | `111 store export`, the export-before-prune offer, writes on change, the migration copy | live snapshots, snapshots to the home hub |

**Relation to D34, the brief and the README.** D34 is the law: every pick of OB §2 (F1-F13), every detail pick of §3, every plan correction of §4 and every
default of §5 (Q1-Q26). The §3 detail picks are the lead's and open to the panel. Where the brief or a report disagrees with the code, the code wins and §22
records it; where the code forces a departure from D34, §22 "Deviations" names it for the owner. The root README is the vision (D36): this spec never rewords
it. Where v1 builds less than a README sentence promises, the table below says so and names the version that closes it.

**v1 scope limits against the root README (D36).**

| README sentence | What v1 builds | Closed by |
|---|---|---|
| §8 "Each node keeps an encrypted local store" | encrypted: the fabric store and the node-state store (§2.1). Plaintext, stated: the Delta Chat core's own database, `rns/`, `state/*.json`, `identity/`, the agent's event FIFO, work files and logs, and datasets (public and signed by design). Encryption protects copied files, backups and synced directories, not a stolen disk with a readable `identity/` | v2: Mode T (a hardware-sealed key, `M/keys.rs:12-14`) and the foreign programs' own encryption when their upstream offers one |
| §8 "Knowledge hubs host the large shared datasets: offline encyclopedias, regional maps, software updates, and network troubleshooting patches" | encyclopedias (§9), regional maps (§10), the 111 release and the components' pinned payloads (§11); no dataset ships and no default catalogue entry exists (Q9); patches travel as Layer 9's sealed `PATCH` messages, not from a hub shelf | v1.1: the hub fix shelf (catalogue class `patch`), the Termux apt directory and APK index, hub-to-hub bulk (Tier 2) |
| §5 "The same fabric carries every kind of traffic the layers need: … knowledge-base synchronization, software updates" | datasets and component payloads move over HTTP on the LAN and to reachable hubs (§7), never on the fabric; the KB rides the release payload (L7 §9.4) | v1.1: Tier 2 on the fabric (§8) |
| Category 4A "Nodes equipped with expanded storage that host local offline databases" | any node with `hub_role` may host; the 4A probe (≥ 512 GB free, ≥ 8 GB RAM, `NI/install/categorize.rs:102-105`) names the class, it gates nothing | not a limit; stated so a Pi hub on an SD card is not called 4A (OB fact 24) |

## 1. Scope & non-goals

**What Layer 8 is per device category** (README layer map, row 8: — · ◐ · ✓ · ◐ · ✓ · ✓):

| Category | Layer 8 there |
|---|---|
| Cat 1 | nothing: no 111 code runs on the stock repeater. |
| Cat 2A, 3 (◐) | the node-state store and the budget run natively; datasets are read from a hub over the LAN (its dashboard, its kiwix panel), kept locally only when the user fetches a small one, never served (serving needs `hub_role`). |
| Cat 2B (✓) | as 2A, plus local datasets the user fetches (a phone holds small ZIMs and a city map; app-private on Termux). |
| Cat 4A, 4B (✓) and any `hub_role` node | the above plus publishing a catalogue, serving datasets and component payloads (each behind its own switch, off by default), the kiwix component, the first copy from upstream. |

**The Raspberry Pi floor (hard rule, AGENTS.md).**
1. A node with nothing configured is byte-identical on the wire to a Layer 7 node. It keeps no file of Layer 8 open while idle: the node-state store opens on
   use and closes on the first capacity pass with no use (§2.2 rule 7).
2. Nothing in Layer 8 adds a resident process, a listener, a timer, a periodic poll or a runtime dependency to the required configuration. The work of the
   budget rides the existing 60 s capacity pass (`N/fabric/mod.rs:163-164`, `:1991-2010`). The new direct crate edges (`rusqlite`, `bao-tree`, `blake3` from
   node111) are already in the lock at the same pins: build-time weight only, +0 crates (§21 L1).
3. Layer 8 removes one recurring cost: the idle fabric store's once-a-minute `settings` commit (5.9 MB a day of WAL, derived, R04 finding 7; §13.4).
4. Hub-only and opt-in costs are enumerated, each behind its own switch and measured at build into `8-storage/TASKS.md` (§19): the kiwix-serve process, one
   loopback listener and its own 1 Hz wake while the panel is open; one fetch thread and a bounded buffer while a dataset job runs; the served files on disk;
   a 16 MiB buffer per active block check.

**Hard rules carried in:** adopt before you build (SQLCipher, `bao-tree`, kiwix-serve, Leaflet and protomaps-leaflet, PMTiles, the L2 export format) · no wall
clock in an ordering, freshness or pruning decision · no telemetry; requests are never logged · the agent proposes and a human decides; the agent's bearer is
refused on every non-GET (D26 (2), `N/dashboard/api_v1/auth.rs:931`) · `111d` imports nothing of Reticulum (`LICENSING.md` §1) · no policy limits of ours:
capacity, never age · every new setting is off or inert by default.

**Non-goals:** a shared log or any consensus of ours (§12) · a sync daemon (Syncthing, IPFS, Hypercore) · a resident caching proxy (apt-cacher-ng, devpi,
pypiserver) · a BitTorrent or iroh swarm in v1 (Q12) · a ZIM reader or libzim linked into `111d` (OB F8 D) · a tile server (Martin, tileserver-gl) · delta
transfer of datasets (`zimdiff` waits for a measured gain) · re-signing a third party's APK or binary · a default dataset or catalogue · automatic deletion of
anything the user fetched.

## 2. The encrypted local store (D34 F1)

**2.1 What is encrypted (normative).**

| Path | At rest | Why |
|---|---|---|
| `fabric/store.db` (+ `-wal`, `-shm`) | SQLCipher 4, raw key from `111/store/v1` (`M/keys.rs:24`, `:101-115`) | Layer 5, unchanged |
| `node/state.db` (+ `-wal`, `-shm`) | SQLCipher 4, raw key from `111/node-state/v1` (§2.2) | conversations, proposals, tasks, decisions, Layer 9's records |
| `agent/proposals/`, `agent/tasks/` | plaintext spool for the time §2.4 states | the agent holds no key (`L2:234`) |
| `agent/events.jsonl`, `agent/work/`, `agent/*.log` | plaintext, stated (Q6) | kinds, ids and titles (`A/src/events.rs:19-21`); task work files; child output |
| `bridge/<adapter>/` | plaintext, stated (Q7) | the Delta Chat core's own database; upstream marked its encryption deprecated (R04 §2); fscrypt is the documented hardening for Linux operators |
| `identity/`, `rns/`, `state/*.json`, `fabric/display-times*` | plaintext behind 0600/0700 ownership (`L2:309`; `N/fabric/times.rs:7`) | unchanged |
| `state/origin/blob/` (datasets, sidecars, payloads) | plaintext on purpose | public files; their protection is the signature (§6) and the block check (§5.2) |

**What encryption buys, stated in the UI and the docs.** The key derives from the keyset, which the words recover (`M/keys.rs:4-11`), and the words sit in
`identity/mnemonic` 0600 (`L2:309`). So encryption protects copied files, backups and synced directories. It never protects a stolen disk whose `identity/` is
readable, and never a compromised process of the daemon's uid: the agent runs under the daemon's uid (`N/supervise.rs:529-532` sets only the parent-death
signal). "The agent holds zero key material" (`L2:234`) is kept by what code holds the key, not by the OS (R07g1 finding 10).

**2.2 The node-state store (normative).**
1. **Path.** `<root>/node/state.db`, directory `node/` 0700 and files 0600, owner `node111` (the fabric store's modes, `M/store/mod.rs:229-243`). A separate
   file, never V5+ tables in `store.db`: a failed fabric migration or a rollback must not take agent and repair state down with messaging
   (`M/store/schema.rs:345-352` refuses a newer `user_version`; R04 finding 12).
2. **Key.** `node_state_key = BLAKE2b-256(key = store_secret, data = "111/node-state/v1" ‖ NodeId)`, keyed mode, where `store_secret` is the existing Mode S
   secret (`N/fabric/sealer.rs:44-47`, `:78`, `:134-145`). It goes through the `KeyProvider` seam (`M/keys.rs:88-93`); Mode T answers `Unsupported`
   (`M/keys.rs:12-14`) and the store stays closed. No new secret, no key on disk; a copied file reopens after a restore from words.
3. **Engine.** The fabric store's open, exposed as a code seam (§2.7 S2): raw key, `cipher_compatibility = 4`, a plaintext SQLite refused, a wrong key
   detected on page 1, WAL, `temp_store = MEMORY`, `secure_delete = ON`, `auto_vacuum = INCREMENTAL`, `synchronous = NORMAL` (`M/store/mod.rs:402-460`), plus
   `journal_size_limit` (§13.5). `synchronous = FULL` for decisions (they gate an applied action) and migrations.
4. **Schema v1** (its own `user_version`; migrations append-only through the fabric store's `migrate_with`, `M/store/schema.rs:345-372`):

| Table | Columns | Notes |
|---|---|---|
| `meta` | `key TEXT PRIMARY KEY, value` | `next_seq`, the clean record, the export offers per class (§4.5), the prune markers |
| `records` | `ns TEXT, id TEXT, seq INTEGER UNIQUE, boot_id BLOB, mono_ms INTEGER, wall TEXT NULL, state TEXT, body BLOB`, `PRIMARY KEY (ns, id)` | documents: a conversation's head, a proposal, a task, a decision, an incident |
| `entries` | `stream TEXT, seq INTEGER UNIQUE, body BLOB`, `PRIMARY KEY (stream, seq)` | append streams: a conversation's messages, Layer 9's audit trail |
| `ns_totals` | `ns TEXT PRIMARY KEY, rows INTEGER, bytes INTEGER` | kept by `AFTER INSERT/DELETE/UPDATE` triggers inside the writing transaction (the M1b pattern) |

   Tables are STRICT. `seq` comes from one allocator (`meta.next_seq`) in the writing transaction: the store's monotonic sequence, the only order. `boot_id`
   (`/proc/sys/kernel/random/boot_id`) and `mono_ms` (`CLOCK_BOOTTIME`) sit beside it; `wall` is a display claim and orders nothing.
5. **Namespaces (v1).** `agent/conversations`, `agent/proposals`, `agent/tasks`, `agent/decisions` (Layer 3's data), `heal/` (Layer 9's, §3), `storage/`
   (Layer 8's own markers). A conversation is one `records` row (its head; `seq` bumped on every turn) and one stream `agent/conversations/<id>` of `entries`.
6. **One writer.** `111d` is the only process that opens the file. The agent reads through `GET` tools only; no Layer 8 route accepts the bearer on a non-GET.
7. **Open on use.** `111d` opens the store on its first use (a chat turn, an ingest, a history read, a reclaim, a Layer 9 write) and closes it on the first
   capacity pass that finds no use since the previous pass: no timer of its own, and no open file on an idle node. `111d` keeps `ns_totals` in memory from its
   own writes, so the pass reads the store only to reclaim. A close checkpoints the WAL (its cost is in §2.8).
8. **When it does not open** (no key, a newer `user_version`, a damaged file, §13.7): the store stays closed, the `node_state` field of `GET /api/v1/storage`
   names the reason, chat still relays (the turn is answered and not saved; the chat view says "History is not being saved: <reason>"), the history route
   answers `503 store_unavailable`, and proposals stay in the spool.

**2.3 Chat through 111d (M1, normative).**
1. `111d`'s chat relay (`POST /api/v1/chat`, `N/dashboard/api_v1/relay.rs:614`) loads the conversation's stream and adds `history` (the OpenAI message objects
   in order; `[]` for a new conversation) to the body it forwards to the agent. Additive to the agent contract (`3-intelligence/TASKS.md:27`).
2. The agent uses `history` instead of reading its file (`A/src/chat.rs:197-201`) and does not append (`A/src/chat.rs:278`). Its `done` frame gains
   `messages`: the turn's new messages in order (the user message, tool calls, tool results, the reply) and the `conversation_id`. A body without `history`
   (an older `111d`) keeps today's behaviour.
3. On `done`, `111d` appends the messages and bumps the head's `seq` in one `NORMAL` transaction, then strips `messages` before the frame reaches the browser:
   the browser's `done` frame is unchanged (`API:121`).
4. **Persist with no browser.** Today the relay stops reading the agent when the browser's body stream is dropped (`relay.rs:585-612`: the upstream lives
   inside the browser's stream). The relay task owns the upstream and drains it to `done` or its existing deadline whether or not a browser reads; the
   browser's stream is a channel fed by that task. An acceptance test (G13).
5. `GET /api/v1/chat/history` (`relay.rs:679`) is answered by `111d` from the store, shape unchanged (`ChatHistory`, `API:803`; unknown id ⇒ 200 with empty
   `messages`). It works while the agent is down; today it answers 503 (`relay.rs:10`, `:26`; `API:131`).
6. The relayed `GET /api/v1/agent/status` replaces the agent's `conversations` count (it counts files, `A/src/store.rs:89`) with the store's.

**2.4 Proposals, tasks and decisions (M2, normative).**
1. The agent writes `proposals/<id>.json` and `tasks/<id>.json` atomically: a temp file in the same directory, `fsync`, `rename`. Today both use
   `std::fs::write` (`A/src/store.rs:139`, `:145`) while the module promises atomic writes (`:1-3`).
2. `111d` ingests a proposal or a finished task on its `proposal.written` or `task.done` event (`relay.rs:826-827`) and, at daemon start, in a sweep of every
   file present before the relay subscribes. It parses with the existing untrusted-file reader (1 MiB cap, `N/dashboard/api_v1/proposals.rs:124`), inserts by
   id (`INSERT OR IGNORE`), commits, then unlinks. A crash between commit and unlink repeats harmlessly.
3. A task whose state is `running` stays in `tasks/`: the agent marks orphans at start (`A/src/tasks.rs:251`). `task.done` ingests and unlinks it.
4. A file that does not parse is never ingested and never deleted by `111d`; the Storage page counts it ("unreadable agent files: N").
5. Decisions are written by `111d` into `agent/decisions` (`FULL`), not `agent/decisions/<id>.json` (`proposals.rs:13`, `:139-141`, `:455-464`). `GET
   /api/v1/proposals` and the decision route read the store. A `kind: message` proposal's outbound text (`N/fabric/proposal.rs:1-19`) is encrypted from its
   ingest on.
6. **The plaintext window, stated.** A proposal or task is plaintext on flash from the agent's rename until `111d` ingests it: sub-second while the relay is
   subscribed; until the next daemon start after a `111d` crash. A deleted file may leave remnants on flash (no RAM-backed writable path). This is the reading
   of Q3's default: "encrypted" covers the durable copy.

**2.5 The legacy import (once, normative).** At the first start of a release that carries the node-state store, `111d` ingests every
`agent/conversations/*.jsonl` (one conversation per file, its lines in file order), `agent/proposals/*.json`, `agent/decisions/*.json` and every finished
`agent/tasks/*.json`, each idempotent by id, then unlinks the files. Conversations take their `seq` in the lexical order of their ids: the one time the legacy
ids, which embed wall-clock nanoseconds (`A/src/store.rs:164-170`), order anything. Afterwards `agent/conversations/` holds nothing and the agent never writes
there again.

**2.6 What stays plaintext, stated (Q6, Q7).** `events.jsonl` stays a plaintext 1 MiB FIFO trimmed to half (`A/src/events.rs:19-21`; 111d resyncs a restarted
journal, `relay.rs:750-773`). `agent/work/<task>/` holds a task's gathered context; it is deleted with its task's record (§4.4). The child logs get a size
ring (§13.8); whether `llama-server` logs prompts is *(unverified from this box)*. The Delta Chat core's database stays plaintext (fscrypt is named for Linux
operators); Android's file-based encryption covers Termux's app-private data *(unverified from this box)*.

**2.7 Code seams in Layer 5 (no Layer 5 spec text, no wire byte).**
- **S1** `M/keys.rs` gains `derive_tagged_key(tag, secret, node_id)`; `derive_store_key` (`:101-115`) becomes that call with `STORE_KDF_TAG`, byte-identical
  (its vector `5-messaging/test-vectors/store-kdf.json` unchanged). A known-answer vector for `111/node-state/v1` joins the file.
- **S2** `M/store/mod.rs` exposes `open_encrypted(path, &StoreKey) -> Connection`: the directory and file modes of `:229-243` and `apply_key_and_pragmas`
  (`:402-460`, private today), and `migrate_with` (`M/store/schema.rs:345`) for a caller's own migration list. The fabric store's open is unchanged.

**2.8 Costs.** No new crate, process, listener or timer; the agent's dependency list is unchanged and its state code shrinks. About 12.4 KB of WAL per chat
turn, about 25 KB with the checkpoint, about 2.5 MB on a 100-turn day (derived, R07g1 §2 and finding 13), against the 5.9 MB a day §13.4 removes. The store's
page cache while open, the open and close cost on an SD card and the binary delta are measured at build (§19).

## 3. The joint storage contract with Layer 9 (D34 F2; D35 F3)

Stated once, here. Layer 9's spec cites these items by number (J1-J9) and adds nothing to them. The seven decided items are J1-J7; J8 and J9 are the paths and
the API that make them buildable.

- **J1 Home.** Layer 9's durable records (incidents, apply attempts, the loop ledger, kept-change snapshots, received fixes, the audit trail) live in Layer
  8's node111 state service under the namespace `heal/`, inside `node/state.db` (§2.2). `111d` is their only writer; the agent reads them only through GET
  tools.
- **J2 Before the store opens.** Three small files stay plaintext (0600, atomic writes, the probation-journal pattern), because a shell hook writes them or
  they must be read before the store opens: the run marker `state/heal/run`, the witness inbox `state/heal/inbox` (one line per daemon exit: code, signal,
  boot id; no free text) and the armed undo record `state/heal/undo` of an apply in progress. `111d` reads them at start before serving, folds them into
  `heal/` and clears them; if the store does not open, they stay and the Repairs page says so.
- **J3 Pruning.** By capacity only, as the `heal` class of the node budget (§4): oldest closed incident first by the store's `seq`; never an open incident, an
  armed undo, an undecided proposal or a running task. A prune leaves one marker record in `heal/` (`pruned through seq N, K records`). No age, no expiry, no
  wall clock.
- **J4 Order and integrity.** The store's monotonic `seq` orders the records, with the boot id and monotonic time beside it; wall time is a display claim. No
  own hash chain and no shared log in v1: SQLCipher checks every page with an HMAC (`cipher_integrity_check`, R04 finding 10). A future shared fix-attestation
  feed must pass §12.2's admission gate, with p2panda-core as the adoption target.
- **J5 Signatures and tags.** Identity keys sign "announces, proofs, and control artifacts only — never routine traffic" (`L2:91`). Layer 9 signs nothing that
  stays on the node (the node key signs only its two outbound artefacts, `fix/v1` and `fix-vouch/v1`); no incident, proposal or audit record is signed. Every
  new node-key signature carries its own `111/<name>/v1` tag, registered in the one Layer 2 registry (A4, continued by A14, `L2:353-364`, `:449-456`) in one
  amendment block for Layers 8 and 9 (§21 L2): Layer 9's `111/fix/v1` and `111/fix-vouch/v1`, Layer 8's `111/catalogue/v1` and `111/export/v1`. Layer 7's
  `111/epoch/v1` and `111/broadcast/v1` are already registered (A14, `L2:455-456`). Key-derivation inputs (Layer 7's `111/carrier-topic/v1`; Layer 8's
  `111/node-state/v1`, the key of J1's file) are key-derivation tags, never signing tags.
- **J6 Nothing leaves.** Incidents, evidence and the audit never leave the node, except inside the user's own encrypted store export (§13.1), which only this
  node's key opens. A shared fix carries only the `fix/v1` fields `111d` builds; a delegation carries only the `AGENT_OP` request fields.
- **J7 Fallback.** If `node/state.db` is not built when Layer 9 builds, `heal/` is a `111d`-owned JSONL ring `state/heal/journal.jsonl` (0600, 1 MiB, the
  `events.jsonl` pattern) under J2-J6, moved into the store by the §2.5 import once it exists.
- **J8 Paths, owners, writers, classes.**

| Path or namespace | Owner layer | Single writer | Budget class (§4) | Offered for export before its first prune | After an update or rollback · a re-run of `111 install` · `111-uninstall` · a words-only restore |
|---|---|---|---|---|---|
| `node/state.db` | L8 | `111d` | `agent` + `heal` | (per class, below) | kept (a migration keeps a pre-migration copy, §13.6) · kept (`L1:209`) · removed (`L1:208`) · key yes, contents only from an export |
| `agent/conversations`, `agent/proposals`, `agent/tasks`, `agent/decisions` | L3 data, L8 storage | `111d` | `agent` | yes, one offer for the class (Q5) | as the file |
| `heal/*` | L9 data, L8 storage | `111d` | `heal` | yes, one offer for the class (Q5) | as the file |
| `state/heal/{run,inbox,undo}` | L9 | the shell hook (`inbox`), `111d` | none (a line or a record each) | no | kept · kept · removed · lost |
| `state/heal/journal.jsonl` (J7 only) | L9 | `111d` | `heal` (1 MiB ring) | no | kept until imported |
| `agent/proposals/`, `agent/tasks/` spool | L3 | the agent | counted in `agent` | no | kept · kept · removed · lost |
| `fabric/store.db` | L5 | `111d` | `fabric` | yes, L5 §10.4 step 3 (`set_export_offered`) | as `node/state.db` |

- **J9 The state service API (node111, in-process; Layer 9's modules call it, the agent never does).** `put(ns, id, state, body)` · `get(ns, id)` · `list(ns,
  state?)` · `append(stream, body) -> seq` · `read_stream(stream, since_seq)` · `set_state(ns, id, state)` · `delete(ns, id)` · `usage() -> {ns: (rows,
  bytes)}` · `reclaim(class, bytes) -> freed`. Each call is one transaction; each write returns the record's `seq`. The states a reclaim never touches are
  declared per namespace (`open`, `armed`, `undecided`, `running`); a reclaim writes the J3 marker.

## 4. One node budget; datasets never pruned automatically (D34 F3)

**4.1 The classes (normative; the closed set `StorageClass.id`).**

| `id` | Label | Layer | Counted from | Budgeted | Rule |
|---|---|---|---|---|---|
| `fabric` | Messages and mail | 5 | live bytes: DB pages minus freelist, plus present blobs (`M/store/prune.rs:498-510`) | yes | L5 §10.4 as built (`prune.rs:1-46`), under the watermark §4.2 sets |
| `agent` | Agent state | 3 | `ns_totals` of `agent/*`, plus the spool and `agent/work/` | yes | §4.4 |
| `heal` | Repair records | 9 | `ns_totals` of `heal/*` | yes | J3 |
| `logs` | Logs | 1 | one `stat` per capped file: `agent/111-agent.log`, `agent/llama-server.log` (`N/intelligence.rs:274-281`), their `.1`, `agent/events.jsonl` | yes | §13.8 |
| `datasets` | Datasets | 8 | the holdings index (§5.5): held sizes plus sidecars, no walk | no | only the user (§4.6) |
| `updates` | Updates | 1, 7 | `releases/`, `state/staging/`, the origin's release blobs | no | as built: two releases kept (`L1:136`) |
| `components` | Components cache | 6 | the registry's `cached_bytes`, `installed_bytes` (`N/components/registry.rs:103-107`) | no | the user's "remove files" (`L1:265-266`) |
| `bridge` | Bridge accounts | 6 | `bridge/<adapter>/` | no | shown only |
| `kb` | Knowledge base | 0 | `current/share/kb/` (`N/kb.rs:1-3`) | no | rides the release (L7 §9.4) |

The `updates`, `bridge` and `kb` figures come from one `tree_bytes` walk each (`N/components/registry.rs:112`), on request only; their time on an SD card is
measured at build. Nothing measures them on a timer (R00 §3: nothing measures `agent/`, `bridge/`, `rns/`, `releases/` or `cache/` today).

**4.2 The `keep` figure, the shares and the floor (normative).**
1. **`keep_bytes`** is one figure per node: the budget of the four budgeted classes. Its default is computed once, at the first daemon start of a Layer 8
   release, from `statvfs` of the root, and stored as `keep_default` in `state/storage.json` (the one write that start makes): a Termux node: min(2 GiB, 10 %
   of free space) · `hub_role` on, or category 4A or 4B: 25 % of the volume's size · every other Linux node: min(2 GiB, 15 % of free space). Derived, not
   measured (R04 fork 2), each measured on a Pi before the lock (Q24). The probe has no storage-medium fact (`NI/install/probe.rs:70-75`, `:372`), so the
   brief's "a Pi on an SD card" figure applies to every non-hub Linux node (§22 D5).
2. **Who sets it** (Q2): the install-time figure; the user on the Storage page; the agent may propose it (`quota.set {class: "keep"}`); nothing else. Turning
   `hub_role` on later does not recompute it: the page shows the 25 % figure as a suggestion.
3. **Shares** (the drafter's defaults; the owner may change). The `logs` reserve is 3 × `logs.max_file_bytes` (§13.8). Of the rest, `fabric` takes 3/4 and the
   node-state store 1/4; inside it `heal` takes at most 1/4 and `agent` the remainder. `caps.agent` and `caps.heal` override a share (null = the share). The
   fabric's share is its high watermark (`watermark.high`); its low mark stays as built, `high − high/8` (`M/store/prune.rs:559-565`). Until a human sets
   `keep_bytes`, the budget never sets the watermark below the built 512 MiB (`:96`): it only raises it (§15.1 "Inert by default"). Arithmetic at keep 2 GiB:
   logs 12 MiB, fabric ≈ 1.49 GiB, node-state ≈ 509 MiB, heal ≤ 127 MiB.
4. **The Transit default follows.** Layer 7's Transit quota, when unset, is a quarter of the high watermark (`M/store/prune.rs:52-63`, `:585-595`): 128 MiB
   only at today's fixed 512 MiB (`:96`). Once the budget sets the watermark, an unset `quota.transit` scales with `keep` as L7 §5.8 defines it; an owner who
   wants the old figure sets `quota.transit` (§21 L7).
5. **The floor** is the fabric's built 1 % (`DiskSpace::floor`, `M/store/prune.rs:129-136`). Below it `disk_low` is true: the fabric prunes as built (its
   floor trigger, `prune.rs:3-6`); `agent` and `heal` reclaim to their low marks; dataset jobs pause (`paused (disk low)`); new draft and import jobs and
   component caching answer `409 insufficient_space`. Messages, chat turns and decisions are never refused (`prune.rs:40-42`).

**4.3 The pass (normative).** On the existing capacity pass (`capacity_pass`, `N/fabric/mod.rs:1853-1873`), every `PRUNE_EVERY` = 12 housekeeping ticks of 5 s
(`:163-164`, `:1991-2010`; the first runs on the first tick, `n = 0`, `:2004`):
1. The fabric's `prune` runs as built, with the `statvfs` figure it takes (`:1855`).
2. The budget reuses that figure and reads the fabric's live bytes, the in-memory `ns_totals` and one `stat` per capped log file. No directory walk; no store
   open.
3. If `keep` or a share changed since the last pass, it calls `set_watermarks` (`M/store/prune.rs:569-577`; no caller today, R00 finding 6): one settings
   write per change, never per pass.
4. A class over its cap reclaims (§4.4) down to its low mark, 90 % of the cap for `agent` and `heal` (the lead's number, Q24).
5. A log over its cap rotates (§13.8).
6. A dataset job paused at the metered cap or at a busy source is checked against its condition (one comparison; no I/O) and resumed when it clears.
7. The node-state store closes if no operation used it since the previous pass (§2.2 rule 7).
8. A change of `disk_low` or `store_full`, or a reclaim, publishes one `storage` frame (§16.3). Nothing is published otherwise. The pass's added time and
   reads, with the node-state store open, are measured at build (OB §3).

**4.4 Rules per class (normative).**
- `fabric`: L5 §10.4 unchanged (`M/store/prune.rs:8-42`): re-fetchable and committed items first, own history only at hard capacity and after the export offer
  (§4.5).
- `agent`: after the class's export offer (§4.5), first decided proposals with their task record, their decision and the task's `agent/work/<task>/`, oldest
  `seq` first; then whole conversations by their head's `seq`, oldest first. Never an undecided proposal, a running task, or the conversation of a turn in
  flight.
- `heal`: J3.
- `logs`: the ring (§13.8); `events.jsonl` keeps its own FIFO (`A/src/events.rs:19-21`).
- No rule reads a clock: "oldest" is `seq` distance, as the fabric's age is `local_seq` distance (`M/store/prune.rs:12-13`).

**4.5 Export before prune (normative; Q5).** One offer per class before its first prune, then capacity order. The fabric's offer is L5 §10.4 step 3's gate,
now called: `set_export_offered` (`M/store/prune.rs:640-642`; no caller today, so step 3 never runs in production, R00 finding 6). For `agent` and `heal` the
offer is a flag in `meta`. Before a class's first prune of its own records (fabric step 3; any `agent` or `heal` record), the Storage page says: "Your oldest
<messages | conversations | repair records> will be removed to make room. You can export them first with `111 store export` (Settings → Storage → Export shows
how), or let them go." The answer (`POST /api/v1/storage/export-offer`) records the offer; until then the class reports `export_offer_needed` and only that
step waits.

**4.6 Datasets are never pruned automatically (normative; Q1).** Only the user, or a `dataset.remove` proposal the user accepts, removes a dataset. A fetch
checks free space first: size, sidecar and the floor must fit, else `409 insufficient_space` before any byte moves (§7.7). Datasets never count against
`keep`. Pruning a dataset below 1 % free, as an opt-in, is later (OB F3 C).

**4.7 Page copy (verbatim; R05 finding 3).** "Nothing here is deleted because of its age. When the space set aside is used up, copies that can be fetched
again go first, oldest first; your own messages, contacts and anything waiting to be sent stay." **4.8 Costs.** Code only; no timer of its own. One settings
write per change of `keep`. Every Storage-page figure is computed on request; §13.4's idle write is never copied.

## 5. One object tree for datasets, checked block by block (D34 F4)

**5.1 The tree (normative).** (1) Every public file the daemon keeps (release payloads, datasets and their sidecars, the release's source tarball) lives in
Layer 7's sha256-addressed tree `state/origin/blob/<sha256>` (L7 §9.1, `7-relay/README.md:857-872`), whether or not the node serves it: one Range handler, one
admission rule. Component payloads stay in the root-owned cache and are served in place (§11.1, §22 C9). · (2) Datasets never enter `fabric/blobs/`: its
writer emits both outboards for every blob (`ChunkGroup::ALL`, `M/blobs.rs:97`, `:456-461`), 3.56 GB of `ob1k` plus 222.7 MB of `ob16k` for a 57 GB file
(R07g3 finding 8), and its bytes count toward the message watermark (`M/store/prune.rs:498-510`). · (3) A file is renamed into `blob/` only after its last
check passed; a half-written file never replaces a served one.

**Paths** (owner `node111`; directories 0755, files 0644 unless noted; every path on the root's filesystem):

| Path | What | Writer |
|---|---|---|
| `state/origin/blob/<sha256>` | a verified file: payload, dataset or sidecar | `111d`, the import verbs |
| `state/origin/catalogue/<publisher hex>.toml` + `.sig` | the highest verified catalogue per publisher (§6.3) | `111d`, the import verb |
| `state/origin/incoming/<sha256>.part` + `.blocks` | a pull in progress and its block bitmap (a hint; the hashes are the proof) | the job |
| `state/origin/drafts/<id>@<version>.toml` | an entry not yet published (§6.4) | `111d`, `111 dataset add` |
| `state/datasets.json` (0600) | the holdings index (§5.5) | `111d`, the import verbs |
| `state/storage.json` (0600) | Layer 8's settings and consent hashes (§15.1) | `111d` |
| `state/kiwix/library.xml`, `state/kiwix/zim/<id>-<version>.zim` → `../../origin/blob/<sha256>` | the kiwix library and one symlink per ZIM (§9.2) | `111d` (through `kiwix-manage`) |
| `node/state.db` (0600, dir 0700) | the node-state store (§2.2) | `111d` |
| `state/heal/` (0700) | J2 and J7 (§3) | `111d`, Layer 9's hook |

**5.2 The sidecar (normative; G3).** (1) A dataset's sidecar is the bao-tree post-order outboard (`outboard_post_order`, `BT/io/sync.rs:706`) at
`BlockSize::from_chunk_log(ob_log)` (`BT/tree.rs:144`): `ob_log = 14` (16 MiB blocks) for files of 1 GiB and more, 12 (4 MiB) below (§22 D9); a Pi hub may
choose 12 for any file after measurement (Q18). It is stored as one more blob, named by its own sha256: an address, not a trust anchor. · (2) Its size is
`(blocks − 1) × 64` B (`BT/lib.rs:511`): 28,224 B for 7.4e9 B, 49,536 B for 13e9 B, 217,408 B for 57e9 B (3,398 blocks), 423,424 B for 111e9 B (R07g3 §3
finding 2). · (3) The 32 B BLAKE3 root in the signed entry authenticates the sidecar alone (`valid_outboard_ranges`, `BT/io/sync.rs:979`) and each fetched
block (`valid_ranges`, `:813`), both in 0.16.1, which v1's checks use (Q22; an inference from the call sites, not a test, R07g3 finding 10). The root is the
same at every block size, so Tier 2 adopts the file later with no re-signing (R07g3 finding 6). · (4) Each active check holds one block-sized buffer: 16 MiB
at 14, 4 MiB at 12 (R07g3 finding 9). · (5) Metalink and upstream `.sha256` files stay operator aids: `--sha256` checks an upstream hash when the first copy
comes from a mirror (§7.5, §7.6).

**5.3 The catalogue entry (normative).** TOML in the release manifest's style, `deny_unknown_fields` (the manifest precedent, `NI/manifest.rs:270`):

| Field | Type | Meaning |
|---|---|---|
| `id` | `^[a-z0-9][a-z0-9_.-]{0,63}$` | stable across versions; never a path |
| `kind` | `zim` · `pmtiles` | the reader (`patch` is reserved for v1.1) |
| `name` · `version` | strings ≤ 120 · ≤ 40 | shown; `version` is never ordered |
| `seq` | u64 | the catalogue `seq` at which this (`id`, `version`) first appeared: orders the versions of an `id` |
| `size` | u64 | bytes |
| `sha256` · `blake3` | 64 hex each | the address in the tree · the root every block is checked against |
| `ob_log` · `outboard_sha256` | 10..=16 · 64 hex | the sidecar's block size and its address |
| `licence` | SPDX expression or `LicenseRef-…` | an `-NC-` id is tagged "non-commercial" wherever it shows (Q11) |
| `attribution` | string ≤ 1024 | the exact notice the licence asks for |
| `source_url` | https URL | where it came from |
| `built` | string ≤ 40 | a display string, never used for ordering |
| `requires` | a component id, optional | `kiwix` for a `zim` |
| `fits` | `2A` … `4B` | the smallest category it is meant for: shown in the consent, never a refusal |
| `used_by` | a subset of `map`, `encyclopedia`, `agent`, `updates` | the chips of §15.4 |

**5.4 The catalogue file (normative).** `schema = 1`, `publisher = "<64 hex NodeId>"`, `seq` (u64, strictly increasing per publisher), then `[[entry]]`
tables. At most 1,000 entries and 1 MiB, checked before parsing; every (`id`, `version`) unique; the detached `.sig` beside it (§6.1).

**5.5 The holdings index (normative).** `state/datasets.json`: per held (publisher, `id`, `version`) its `sha256`, its location (`origin`, or `in_place` with
an absolute path), its state (`partial`, `present`, or `unverified` for an in-place file after an unclean shutdown), the hash of the consent that fetched it,
and its job. Written atomically and only on change, with the `state/relay.json` file rules (`N/relay/settings.rs:16-36`: absent ⇒ defaults; not a small
regular file or not JSON ⇒ moved aside and defaults), except that an unknown key is kept and ignored, never "invalid", so a rollback to an older release never
drops a record (§22 C1).

**5.6 Costs.** No new crate: `bao-tree`, `blake3`, `sha2` and `toml` are linked (R00 §2; `bao-tree 0.16.1` and `blake3 1.8.7` in
`1-bootstrap/Cargo.lock:101-102`, `:179-180`); node111 gains direct edges at those pins (§21 L1). No listener, no timer. One import pass computes sha256,
BLAKE3 and the sidecar together; its throughput on Pi 4 and Pi 5 is *(unverified from this box)*, measured at build.

## 6. Who signs a dataset (D34 F5)

**6.1 The signature (normative).** Ed25519 by the publisher's node identity key over `"111/catalogue/v1" ‖ publisher NodeId (32 B) ‖ the catalogue file's
bytes`; a detached 64 B `.sig` beside the file. The tag leads the signed input (`L2:91`). The release key never signs datasets (OB F5 C).

**6.2 The trust rule (normative; G6).** A node verifies and lists the catalogues of (a) its pinned home hub and (b) the publishers in `follow` (§15.1, default
`[]`): the rule L7 §11.1 uses for broadcasts. Any other catalogue is not listed; the API answers `409 untrusted_publisher`; the CLI import refuses without
`--follow` (§7.6). A mirror hub serves another publisher's catalogue and blobs with the original signature and never re-signs (R02 finding 16).

**6.3 Freeze stance (normative; L7 §9.5 T4).** A node keeps the highest verified `seq` per publisher (`state/origin/catalogue/<publisher hex>.toml`) and
refuses a lower one. A stale mirror can hold a node on an old catalogue until the node reaches another source; the Datasets view shows "newest catalogue seen:
seq N from <source>". No clock expires a catalogue.

**6.4 Publishing (normative).**
1. A draft is written by `111 dataset add` (§7.6) or a first-copy job (§7.5), only after its bytes and its sidecar passed.
2. The hub owner publishes on a human action: `POST /api/v1/datasets/publish` (loopback or a paired session; the bearer is refused; `409 needs_hub` without
   the hub role). `111d` builds catalogue `seq + 1` from its previous catalogue, the drafts named and the removals named, and signs it through the identity
   handle: one signature per publish.
3. Only an entry whose file and sidecar the hub holds and verified can be published.

**6.5 Endorsements (later).** A further signature over one entry under its own tag `111/catalogue-vouch/v1` (reserved in §21 L2, the pattern of Layer 9's
`111/fix-vouch/v1`), shown as "vouched for by <alias>", never as automatic trust.

**6.6 Costs.** No new key (v1 signs with the root node key, which the words recover), no crate, process, listener or timer: the Ed25519 verifier and the TOML
parser are linked. 64 B per catalogue; one verify per catalogue fetched.

## 7. Moving datasets in v1 (D34 F6)

**7.1 The Range arm (normative).** One function beside `file_body` (`N/panels/static.rs:181`), used by the panel host, the origin's blob route (Layer 7 row
D7; `N/origin/mod.rs:1-11` is a stub at `f0b6806`) and the tiles route (§10): (1) `Accept-Ranges: bytes` on every 200 and 206. · (2) One range (`bytes=a-b`,
`bytes=a-`, `bytes=-n`) ⇒ 206 with `Content-Range: bytes a-b/size` and `Content-Length`. A range past the end ⇒ 416 with `Content-Range: bytes */size`. Two or
more ranges ⇒ 416: never a multipart body (§22 D10). · (3) `If-Range` against the existing ETag (`etag_of`: inode, mtime, length,
`N/panels/static.rs:163-170`): a mismatch serves 200 and the whole body. · (4) Always `Content-Length`; never compressed, never chunked (IIAB's PMTiles
workaround, R07g3 §4 finding 9). · (5) The body streams from disk in the existing 64 KiB chunks (`N/panels/static.rs:42`). The 64 MiB cap (`:39`, `:227`)
stays for panel bundles; the origin and tiles routes pass none. · (6) GET and HEAD only. Estimated 60 to 120 lines, no dependency (R05 finding 17).

**7.2 The stream cap (normative).** At most `origin.max_streams` (default 4, the lead's number, Q24) bodies on `/origin/v1/blob/*` at once per hub; the next
request answers 503 with no body. kiwix-serve's `--ipConnectionLimit` is the precedent (R07g3 §4 finding 6). The session-gated tiles route has no cap.

**7.3 `get_to_file` (normative).** (1) New in `NI/util/http.rs`: `Fetcher::get_to_file(url, range, file, offset)` streams a body into a file at an offset,
with a per-read idle timeout and no global timeout (the ureq 3 knob that bounds one read is chosen at build). `Fetcher::get` (the whole body in memory under
one global timeout, `NI/util/http.rs:134-151`, `:172-200`) is unchanged. · (2) The scheme rule is T6's (`NI/util/http.rs:1-17`): https; http only to the home
hub's `origin_base` (`HttpException::HomeHubOrigin`, `:25-32`) or to a URL the user typed for a dataset, a new typed exception `UserTypedDataset` beside
`UserTypedFrom`. · (3) **A pull from a node origin:** (a) fetch the sidecar `blob/<outboard_sha256>` (≤ 1 MiB) and check it with `valid_outboard_ranges`
against the entry's root, so a swapped sidecar fails before any data byte; (b) request block-aligned ranges, one block per request, into
`state/origin/incoming/<sha256>.part`; (c) check each block with `valid_ranges`, refetch a failed block once from the same source and then from the next
source; (d) set its bit in `.blocks`; (e) after the last block, `fsync`, rename the file and the sidecar into `blob/`, update the holdings index. The root is
then proven with no second whole-file pass (R07g3 finding 5); nodes do not recompute the sha256 the publisher signed. · (4) Hash-named URLs make a resume
against any source safe; two hubs can serve different blocks of one file with no extra protocol (R02 finding 3). · (5) After a crash a job re-checks the
blocks its bitmap marked in flight; it re-checks the whole `.part` only after an unclean shutdown.

**7.4 Jobs (normative).** At most one dataset job runs per node; accepted fetches queue in order. A job runs on one named thread, `111d-dataset`, that exists
only while a job runs, with one block buffer. · States: `queued` · `running` · `paused (offline)` · `paused (metered cap)` · `paused (disk low)` · `paused
(source busy)` · `done` · `failed (<reason>)` · `cancelled`. A pause is a state, never an error (OB §3). · What resumes a paused job, with no timer of its
own: offline — the source hub's link attribution (L7 §9.3's event), the user's Resume, a daemon start; metered cap, source busy, disk low — the capacity pass
(§4.3 step 6). An offline job never retries on a timer. · Progress rides SSE `dataset` frames (one per verified block) only while a job runs. Cancel deletes
the `.part` and its bitmap.

**7.5 The first copy (normative; Q8).** (1) **From upstream, on a hub with internet:** `POST /api/v1/datasets/drafts` with the URL, the upstream sha256 and
the entry's fields. The job pulls the whole file with `get_to_file` (a resume sends `If-Range` with the upstream's validator), checks the sha256 the user or
the mirror supplied, computes BLAKE3 and the sidecar in the same pass, places both in the tree and writes a draft. Only then may the owner publish (§6.4). The
publisher copies a ZIM's own `License` and `Source` metadata into `licence` and `attribution`, read off-node with zim-tools' `zimdump` (R05 finding 15). · (2)
**From a disk:** §7.6.

**7.6 Sneakernet (normative).** `111 dataset import --from <dir|url> [--in-place] [--follow]` runs once as the node user (the `111 update --from` pattern, L7
§9.2): it reads `catalogue/<publisher hex>.toml` + `.sig` and `blob/<sha256>` from a directory or a URL in the origin layout, verifies the signature (an
unfollowed publisher: it prints the NodeId and refuses unless `--follow`, which adds it to `follow`), checks each sidecar and every block, and copies into the
tree. · `--in-place` (hubs only) registers a file on an attached disk with one hash pass and no write under `state/origin/`; after an unclean shutdown the
file is `unverified` and is checked again before it is served (Q19). · `111 dataset add <file> --id --kind --name --version --licence --attribution
--source-url [--sha256 HEX] [--ob-log N] [--in-place]` (hubs only) computes sha256, BLAKE3 and the sidecar in one pass, places the file and writes a draft. ·
FAT32 caps a file at 4 GiB: the docs name exFAT or ext4; no `zimsplit` (Q19). On Termux datasets stay app-private, and the copy says that uninstalling Termux
removes them (R05 finding 16).

**7.7 Consent and space (normative).** Every fetch, draft and dashboard import is the pinned round-trip (L7 §12; `ConsentRequired`, `API:1492-1501`): `409
consent_required` with `consent = {action: "dataset_fetch", subject: "<publisher short>:<id>@<version>", warning_text, hash, addons}`; the client shows
`warning_text` verbatim and repeats with `accept_warning` = the hash; a changed text has a new hash and asks again. The text (§15.2) names the size, the free
space before and after, the licence and attribution, the signer, `used_by`, `fits` and the connection. A fetch that does not fit says so before any byte moves
(`409 insufficient_space`). Accepting a `dataset.fetch` proposal opens the same dialog and never skips it.

**7.8 Metered links (amends L7 §12, additive).** Dataset fetch bytes (bodies and headers, both ways) count in Layer 7's rolling 24 h window on
`CLOCK_BOOTTIME` (`7-relay/README.md:1002-1004`; a stub at `f0b6806`, `N/relay/metered.rs:1-9`, row D11). The link is the interface of the job's connection
(its local address after connect) matched against `metered`. With `metered = []` nothing is metered and the dialog says "No metered link is set." On a marked
link the job pauses at the cap and resumes in the next window, and the dialog shows the days: at 1 GB a day, about 8 days for 7.4 GB and 57 days for 57 GB
(R05 finding 8). The override (`dataset_override`) is a second explicit tap with its own text, per download, never the agent's (OB F6 (c), default (b)). The
window's persistence cadence is Layer 7's, measured under its board; a pull adds bytes to it, not writes (options-check K1).

**7.9 Refresh and removal (normative).** A new version of a held `id` is a new fetch: staged, checked, renamed. The old version is removed only when the user
accepts the refresh, whose consent names the bytes it frees; a refresh that does not fit beside the old version says so before any byte moves (R01 finding 7).
Removal (`DELETE /api/v1/datasets/{id}?version=`, a human action or an accepted `dataset.remove`) deletes the file and its sidecar unless another held entry
or a pin list (§11.1) still names them.

**7.10 Reach (normative).** v1 moves datasets over the LAN and to hubs with a reachable address (a public address or Layer 7's reach layer, L7 §6.6). A
dataset never rides a carrier or LoRa: Berlin's 84 MB is about 9 days of LongFast airtime (R02 finding 12).

**7.11 Costs.** Nothing resident. During a job, one thread and one block buffer; one sequential write per file plus its sidecar, one `fsync` and a rename (57
GB is about 47 min at an assumed 20 MB/s, R02 finding 13). A bad block costs one block, where a whole-file check would cost the file again (57 GB is 12.7 h at
10 Mbit/s, R02 finding 4). No listener.

## 8. Tier 2 object transfer (D34 F7; later)

1. **Owner.** Layer 8 owns the Tier 2 build (`OBJ_ADVERT`, `OBJ_REQ`, `OBJ_CHUNK`, L5 §11), in v1.1, as an additive Layer 5 amendment list Layer 8 writes and
   the owner sees. Layer 7's broadcast pointer (an `obj_hash` "fetched by Tier 2", `7-relay/README.md:966`) and Layer 9's patches use it when it lands. L7
   §17's Tier 2 row gains this owner (§21 L7).
2. **v1 builds none of it.** The kinds stay registered and unbuilt (`M/envelope.rs:543-548`; `built_in_v1` false, `:629-634`); an inbound one falls through
   the dispatch (`M/fabric.rs:3936`); the chunk frame is discarded (`M/fabric.rs:3694`).
3. **Prerequisites.** `bao-tree` 0.16 → 0.17 at Tier 2's start (Q22): a lock bump, MSRV 1.85, the `LICENSING.md:144` row, regenerated notices; 0.17 fixes
   `encode_ranges` and decode-after-error on the range paths Tier 2 uses (R02 finding 6). Bulk on its own connection or Layer 7's v1.1 non-blocking
   `Link::send`, because 57 GB on the chat connection would stall chat (R02 finding 9). Hub to hub over the internet it rides L7's TLS-RPK link; public
   adverts are signed under the registered `111/obj/v1` (`L2:364`).
4. **Layer 8's storage half**, built in v1 where §5 and §7 need it: the size check against free space, the superseded mark of a replaced version, resume by
   block.
5. **Costs.** None in v1. In v1.1: the two prerequisites, no listener of its own, one block buffer per active check; binary and RSS deltas measured then.

## 9. Encyclopedias, and what a hub shows the LAN (D34 F8)

**9.1 The `child` panel source (amends L5 §13, additive; normative).** The panel host registers `kind = "static"` only; `child` and `detected` are refused at
registration (`N/panels/manifest.rs:12-13`, `:400-403`; the reverse proxy L5 §13 specified and reserved, `5-messaging/README.md:33`). Layer 8 builds `child`
for children that speak plain HTTP: (1) A `child` manifest names `[source] component = "<id>"` and `upstream = "127.0.0.1:<port>"`, the port one of that
component's `[run] ports` (loopback, claimed by the components runner, `N/components/manifest.rs:312-314`, `:888-898`). `detected` stays reserved. · (2)
`[proxy] websocket` and `sse` stay `false`; `true` still fails adoption (`N/panels/manifest.rs:14-16`, `:510`; D20 (a)). · (3) The panel listener forwards
each request it admitted (the ticket → `p111_<id>` cookie and every guard of L5 §13, unchanged) to `upstream` with the `hyper` client node111 already links
(`1-bootstrap/crates/node111/Cargo.toml:61`), the request body capped at `max_body`. It strips the panel cookie and the dashboard's session cookie before
forwarding, and the panel host's header policy filters the answer. · (4) The panel's origin is its own port in the panel block (`N/panels/mod.rs:56`), so a
ZIM's JavaScript runs there, never in the dashboard's origin (R01 finding 5).

**9.2 The kiwix component (amends L6 §2.1, additive; normative).** `assets/components/kiwix.toml` (embedded in the release binary like every declaration,
`N/components/manifest.rs:1-5`, `:88`): **No `adapter`.** A server component names no network; today `adapter` must be a network id
(`N/components/manifest.rs:685-686`). It becomes optional (§21 L6). · **Payload.** `kind = "binary"` with per-target tables
(`N/components/manifest.rs:21-31`); a `.tar.gz` unpacks whole into `components/kiwix/` (`N/components/registry.rs:599-607`), so `kiwix-serve` and
`kiwix-manage` arrive together. Linux: the upstream static kiwix-tools tarball (a 3.8.2 aarch64 or musl build is *(unverified from this box)*, R01 finding 3;
the manifest pins whichever bytes pass the smoke). Termux: the deb closure (kiwix-tools 3.8.2, libzim 9.8.2, libkiwix 14.2.1 with ICU, Xapian, libmicrohttpd,
libcurl, pugixml, zstd, lzma, docopt; R01 finding 2), which waits for its measurement before any phone path (Q17). · **`[install] prefetch = false`**
(`N/components/manifest.rs:53`): fetched on a hub by `sudo 111 install --components kiwix`, never on a phone by default (Q17). · **`[run]`:** `program =
"{payload}/kiwix-serve"`, `args = ["--library", "{state}/kiwix/library.xml", "--threads", "2", "--nolibrarybutton", "--address", "127.0.0.1", "--port",
"11250"]`, `ports = [11250]`, and the new key `start = "on_demand"` (§21 L6). Never `--monitorLibrary`, which adds a per-second stat (R07g3 §4 finding 6). The
thread count is unmeasured; `--address` comes from `kiwix-serve.rst` (R01 finding 1) and is re-read at build (R07g3 §4 finding 6 did not re-check it). 11250
is outside the base ports (`N/components/manifest.rs:962-964`) and every declared one. · **`[license]`:** GPL-3.0-or-later (kiwix-tools, libkiwix),
GPL-2.0-or-later (libzim), read upstream (R01 candidates). A separate program: D22's linking boundary is untouched. `[weight]` is measured at build (§19). ·
**Panel** `assets/panels/kiwix.toml`: `[source] kind = "child"`, `component = "kiwix"`, `upstream = "127.0.0.1:11250"`; `[auth] mode = "111-session"`;
`[license] spdx = "GPL-3.0-or-later"`, `served = "unmodified"`. · **The library.** `111d` keeps one symlink per held ZIM (`state/kiwix/zim/<id>-<version>.zim`
→ its blob, so the file name carries the extension) and rebuilds `library.xml` with `kiwix-manage` (adopted, from the same tarball) when a ZIM is added or
removed, then sends `SIGHUP` to a running kiwix-serve, which reloads its library without a watcher (R07g3 §4 finding 6).

**9.3 Lifecycle (normative).** (1) Enable is Layer 6's: the licence notice and its `component_enable` consent, the cache check, the install
(`N/components/mod.rs:12-20`). With `start = "on_demand"`, enable starts nothing. · (2) The first panel session mint (`POST /api/v1/panels/kiwix/session`)
starts kiwix-serve as a supervised child, with the runner's back-off, and answers once its port answers. · (3) It stops when the user stops it (Stop on its
panel page, or disable) and at daemon stop. There is no idle stop: kiwix-serve wakes once a second whatever it does (`kiwix-serve.cpp:423,451`, R07g3 §1), so
an idle detector would need a timer of ours. · (4) While off: no process, listener, timer or open file (G7's zero-weight gate,
`6-transport/verify/zero-weight.sh`).

**9.4 `kb.search` (normative).** `GET /api/v1/datasets/search?q=<text>&book=<id>&n=<1..20>` (session; the agent's bearer reads). While kiwix-serve runs,
`111d` asks `http://127.0.0.1:11250/search?pattern=…&books.name=…&format=xml&pageLength=n` (a documented public endpoint, R05 finding 12) and answers
`{schema, format: "opensearch-xml", body}`, `body` the upstream text capped at 8 KiB. While it is off: `409 component_unavailable` with "The encyclopedia is
off." `111d` parses nothing (no XML crate is in the lock); the agent tool labels the body untrusted. The agent may say a person can open the encyclopedia from
Settings → Storage; it never starts it (the session mint is a POST, refused to the bearer).

**9.5 Raw files on the origin (normative; Q14, Q15).** ZIMs, PMTiles files and their sidecars, and the catalogue files (§16.7), are served read-only and
unauthenticated on `/origin/v1/` only while `hub_role`, LAN sharing and `origin.datasets` (default off) are all on, after the `datasets_serve` consent
(§15.2), under the stream cap (§7.2). kiwix-serve is never LAN-bound: the encyclopedia is read through the panel and the hub's session only. Requests are
never logged.

**9.6 What a hub shows the LAN (normative; §17's exposure table).** With `origin.datasets` on, anyone on the LAN learns the holdings, their versions and
contents, and a map's area; with `origin.components` on, which programs the hub cached. The kiwix panel and the Map tab show nothing without the hub's
session.

**9.7 Credits (normative).** A Credits section on the Storage page, linked from each panel: per held dataset its `licence` and `attribution` verbatim; for
every map the ODbL notice "Contains information from OpenStreetMap, which is made available here under the Open Database License (ODbL)." (ODbL s.4.3, R01
finding 12); a "non-commercial" tag on any `-NC-` licence; no single licence claimed for a ZIM that mixes image licences (R07g3 fork D D1). IIAB names
OpenStreetMap without the ODbL sentence (R07g3 §4 finding 9): the baseline a hub does better than.

**9.8 Costs.** While off: nothing. While the panel is open: one process, one loopback listener (11250), kiwix's own 1 Hz wake. Its RSS is published nowhere;
it is measured at build, anonymous against file-backed (OB §3). The only primary figure is IIAB's sizing of a Zero 2 W at ≥ 1,024 MB of RAM plus swap for the
whole box (R07g3 §4 finding 3). The tarball's size on disk is measured at build.

## 10. Maps (D34 F9)

(1) **Libraries.** Leaflet 1.9.4 (BSD-2-Clause) and protomaps-leaflet 5.1.0 (BSD-3-Clause; it embeds pmtiles 3.2.1 and the basemap themes): 80.4 KB gzip, 83.9
KB with CSS, measured by a scout with esbuild (R05 finding 13). One lazy chunk, `map`. · (2) **The exempt `map` bucket** in `UI/scripts/check-budget.mjs` (the
fonts precedent, `:1-7`): 90 KB gzip on its own line, outside the 256 KB lazy budget and the 64 KB per-chunk cap (`:13-14`). L7 §10.5's "≈ 69 KB"
(`7-relay/README.md:951`) reads 83.9 KB (§21 L7); D21 gains the bucket's figure (§22, OB §4 item 9). · (3) **Loaded only where needed.** The chunk loads only
where `GET /api/v1/tiles` lists a set: a node with none never downloads it. · (4) **Data.** PMTiles files as Protomaps publishes them, and regional subsets
made off-node with `pmtiles extract` (go-pmtiles, BSD-3-Clause). Nothing rebuilt from OSM data until the legal read of "a re-extracted vector tileset is a
Produced Work" (Q10). · (5) **Serving.** `GET /api/v1/tiles` lists the held `pmtiles` sets; `GET /api/v1/tiles/{id}` serves the file through the Range arm
(§7.1) as `application/octet-stream` (§22 D7). Both are session-gated: the browser reads tiles from the dashboard it is paired with, under the unchanged
`connect-src 'self'` (`N/dashboard/routes.rs:78`). Vector tiles draw to a canvas, so `blob:` stays out of the CSP (R05 finding 13). · (6) **Where.** A Map tab
in the Nearby view, beside Layer 7's radar: the tile set plus the radar's Meshtastic position cells (L7 §10.1 rule 4, the coarsest cell in v1); 111 nodes
share no position (L7 §10.3). The ODbL sentence is drawn on the map itself, each set's attribution beside it. · (7) **Privacy.** A regional extract tells
every LAN visitor where the community is; the serving consent says so (§15.2). · (8) **Labels offline** are *(unverified from this box)*: the build checks
that protomaps-leaflet draws labels with no network; if it cannot, v1 draws no labels. · (9) This is Layer 7's v1.1 map item ("a Layer 8 region file, the D21
amendment, a Range route", `7-relay/README.md:1265`); it lands in Layer 8 v1. · (10) **Costs.** Browser-side only; on the node the Range arm and two routes.
Build-time weight: about 84 KB gzip of UI assets in every payload, fetched by a browser only on the Map tab.

## 11. Software updates beyond the 111 release (D34 F10)

**11.1 One admission rule (amends L7 §9.1, additive; normative).** A hub answers `GET /origin/v1/blob/{sha256}` only if the hash is in a pin list it holds and
the bytes it holds match it. The pin sources, each behind its own switch (every one also needs `hub_role` and LAN sharing):

| Pin source | Switch | Where the bytes are |
|---|---|---|
| the verified release manifest (`parse_verified`; L7 §9.1) and its `[source]` pin (§11.6) | `origin.serve` (Layer 7) | `state/origin/blob/` |
| an embedded component declaration of an installed release (the pins ride inside the signed `111` binary, `N/components/manifest.rs:1-5`, `:88`) | `origin.components` | `cache/packages/<id>/`, read in place |
| a catalogue the hub verified (§6) | `origin.datasets` | `state/origin/blob/`, or in place (§7.6) |

Everything else answers 404. The components cache is root-owned (`L1:142`), so the daemon cannot copy into it or hard-link out of it; it serves those files
where they are (§22 C9), and serving ends with the file (Q26).

**11.2 `origin.components` (normative; Q13, Q14).** Default off, needs `hub_role` and LAN sharing, and the `components_serve` consent (§15.2), which names the
operator's duty. The `lxmf` closure is excluded until the owner's upstream contact answers (Q25; `LICENSING.md` §9 item 16). Each served blob has a notice
beside it (§11.7).

**11.3 The installer's origin list (amends L1 "Layer 6: optional components" and L6 §2.1 rule 3, additive).** `PayloadFetcher`
(`NI/install/packages.rs:255-262`) gains `fetch_pin(sha256, size, upstream_url)`, which tries `[the home hub's origin_base, the pinned upstream URL]` in that
order; a miss (404, a wrong size, a wrong hash) falls through. The notice is printed and recorded before any download, as today
(`NI/install/packages.rs:265-289`), now naming the source; the receipt records it. Enable stays offline and "There is no runtime fetch"
(`6-transport/README.md:146`) holds: the verb runs at install time. A node with no home hub is unchanged. `origin_base` is the one Layer 7 learns at enrolment
(L7 §9.1 S4).

**11.4 Pip closures: the files table (normative).** A pip lock holds hashes, not file names (R07g2 finding 6). `gen_lock.py`
(`6-transport/sidecars/lxmf-sidecar/gen_lock.py`) also writes a files table (name, sha256, size, per target) beside the lock, embedded the same way. The
installer pulls each wheel from the hub by hash, writes it under its name into `cache/packages/<id>/`, and enable's offline `--require-hashes` install is
unchanged (`L1:262-264`). The hub path needs no throwaway venv.

**11.5 Termux debs and APKs (normative).** The release manifest pins each deb and APK (R07g2 finding 9); a provisioning host fetches them by hash from a hub
as from any origin. A hub never re-signs an APK (R07g2 finding 11). The Termux apt directory (`termux-apt-repo`, Apache-2.0) and a signed APK index
(`fdroidserver`, AGPL-3.0, on the publisher's machine) are later (OB F10 B).

**11.6 The 111 release's source tarball (normative).** GPL-3.0 §6(d) binds whoever conveys a binary (R07g2 item 15), and Layer 7's origin already conveys the
111 payload, which §9 never mentions (R07g2 item 22). The release manifest gains a `[source]` pin (url, sha256, size of the tagged source tarball), added
before the first signed release as Layer 5's sections were (`manifest-version` stays 1, `NI/manifest.rs:140-143`). A hub with `origin.serve` also serves that
blob, and the payload's notice names it. Its size is measured at release; the Rust sources and manifests alone are about 1.7 MB gzipped (L9 R07g1 §1).

**11.7 The licence duty (normative; amends D26 (5) by one sentence, OB §4 item 9).** D26 (5) gains: "When a hub is the source, the hub's operator is the one
who conveys; a member's acknowledgement records the member's own install." The hub code gives the operator the means: `GET /origin/v1/notice/{sha256}` answers
a text per served component blob (program, version, licence, where its source is, the licence text when the release carries it in `THIRD-PARTY-NOTICES.md`).
Whether serving one's own devices is conveying stays the open legal read (`LICENSING.md` §9 item 14).

**11.8 Costs.** Disk only for what the operator cached for itself: up to 104.1 MB (x86_64) or 94.5 MB (aarch64) for the Delta Chat core and Madmail, and about
6 MB of wheels per target (R07g2 finding 4). No process, listener, tool, key or timer. Rejected as resident: apt-cacher-ng, devpi, pypiserver; bandersnatch
needs a runner and Python 3.12 (R07g2 §2).

## 12. The shared log (D34 F11)

**12.1 No shared log in v1 (normative).** README §8 says a log "may" be used. Nothing in v1 needs one: every state that crosses nodes has one writer and is
ordered by a signed counter (the announce, the release and its KB, a dataset catalogue, the epoch beacon), and local state must not leave the node (R03
findings 1-2). `SYNC` (`Kind::Sync = 0x14`, `M/envelope.rs:541-542`) stays reserved. No catalogue-head frame (conflict 4): §12.4.

**12.2 The admission gate (normative).** A shared log enters only with all five: (1) a named consumer; (2) one named writer per key; (3) the never-list: keys
and seeds, message bodies, alias and contact graph, location and presence, device ids, conversation or agent text, relay edges, anything its subject has not
agreed to publish (R03 finding 5); (4) a measured Pi delta; (5) an adopted engine, never our own consensus. The adoption target is p2panda-core (0.7.1, MIT OR
Apache-2.0, pre-1.0, rust-version 1.96; Ed25519, BLAKE3 and CBOR; prunes at author-written points; R03 §2), with Layer 5's epoch as the log id so a restore
from words never forks a log (R03 finding 7). CRDT documents (Loro first) only for a user-facing shared document, which no spec asks for (Q23).

**12.3 The consumer registry (amends L5's dispatch, additive; the textless-row fix, Q20; normative).** Today an inbound `AGENT_OP`, `KB`, `UPDATE`, `PATCH` or
`SYNC` (0x10-0x14, `M/envelope.rs:534-542`) from a contact falls into `k if k.carries_msg_seq() => self.on_data` (`M/fabric.rs:3912`) and is stored as a
message row of its class (`on_data`, `:3951`; `Class::from_kind` and `receive_message`, `:4018-4024`), which the UI draws as a textless chat row
(`message_view` decodes text for `MSG` only, `:7593-7600`).
1. The fabric gains `Fabric::set_consumer(kind, consumer)` for 0x10-0x14. Layer 9 registers 0x10 and 0x13: its F13 item 13 asks for the same hook, built once
   by whichever layer lands first.
2. An inbound frame of those kinds still passes L5 §4's sequencing and acknowledgement (it was received): the sequencing commit stays. Its body then goes to
   the registered consumer, or, with none registered, is dropped and counted (`unconsumed`, per kind, a session counter on the `fabric` status). It never
   becomes a message row.
3. `MSG` and the custody, mailbox and enrolment kinds are unchanged.
4. Cost: none resident; the row, its bytes and its place in a chat thread go.

**12.4 Catalogue updates without gossip (normative; §22 D8).** A member asks its home hub for the hub's catalogue with one GET on the link-attribution event
L7 §9.3 uses for releases, only while its last answer was a catalogue, and whenever the user opens the Datasets view or taps Check now (`POST
/api/v1/datasets/check`). A hub that never served one costs its members one 404 after a Layer 8 install, then nothing. No head frame, no timer, no gossip. A
followed publisher's new entry will travel as a signed `BROADCAST` pointer (L7 §11.1), later.

## 13. Backup, SD wear and migrations (D34 F12)

**13.1 The export format (amends L2 §8.5(i): built, with its tag; normative).** L2 §8.5(i)'s layout (`L2:218`), byte for byte:

| Field | Bytes | Value |
|---|---|---|
| `version` | 1 | `0x01` |
| `ephemeral_pk` | 32 | fresh per export; `shared = X25519(esk, own_enc_pk)` (abort on all-zero), `key = BLAKE2b-256("111/seal/v1" ‖ shared ‖ epk ‖ rpk)` (L2 §6.4, `L2:122-130`) |
| `ciphertext` | per 64 KiB plaintext chunk: the chunk + 16 | XChaCha20-Poly1305, nonce = chunk index (u64 LE) ‖ 16 zero bytes, AD = chunk index ‖ `last_chunk` (Layer 5's private-object framing, `M/blobs.rs:12-21`) |
| `node_sig` | 64 | Ed25519 by the node key over `"111/export/v1" ‖ NodeId (32) ‖ u64 LE length ‖ BLAKE3(every byte before node_sig)` |

The plaintext is a tar (`tar` is linked, `1-bootstrap/crates/node111/Cargo.toml:45`) of `manifest.toml` (schema 1, NodeId, each store's `user_version` and
size, the release that wrote it) and the two store files as they are (still SQLCipher, keyed from the words). A fresh ephemeral makes the key unique, so
nonces never repeat; a cut at a chunk boundary fails decryption; one AEAD call over a store would hold it in memory and the file can be gigabytes, hence the
chunks and the prehash (§22 D11). Restore verifies the signature before decrypting (one BLAKE3 pass), runs `quick_check` on each store, then places them.
Hub-storable, never published (L2 §8.5(i)). No new crate. `age` stays a possible format for hub-side tooling (OB §4 item 10): it pulls `hpke`, `ml-kem`,
`p256`, `scrypt` and `i18n` and carries no sender signature (R04 finding 14).

**13.2 The verbs (normative).** `111 store export --out <file>` and `111 store import --from <file>` run as the node user on a local terminal, never over HTTP
(the `identity backup` rule, `L2:311`), with the daemon stopped: both refuse while `111d` runs ("Stop the node first: sudo systemctl stop 111node"). A clean
stop leaves each store one checkpointed file (`M/store/mod.rs:354`). A live snapshot of an encrypted WAL store needs a route this box could not verify
(SQLCipher's backup API, `VACUUM INTO`, R04 finding 15, *(unverified from this box)*), and mesh111's `rusqlite` has no `backup` feature
(`5-messaging/crates/mesh111/Cargo.toml:17`): a live export is later (§20; §22 D6). Import onto a node with another NodeId refuses. **13.3 The
export-before-prune offer** is §4.5: Layer 8 calls `set_export_offered`, so L5 §10.4 step 3 can run at all.

**13.4 Writes only on change (amends L5 code; normative).** Every pass commits `capacity.store_full`, `capacity.full_classes`, `capacity.disk_free` and
`capacity.disk_total` (`save_capacity`, `M/store/prune.rs:928-947`, called in the pass's transaction at `:836`): about 5.9 MB a day, 2.2 GB a year of logical
WAL writes on an idle node (derived, not measured, R04 finding 7). Writing "only when a value changes" is not enough, because free space moves with any write
on the disk: (1) `save_capacity` writes `store_full` and the class mask only when they differ from the stored values. · (2) The disk figures are no longer
persisted; the two keys are deleted once. `load_capacity` restores them into `last_disk` at open (`:896-910`), and the first pass runs on the housekeeping
task's first tick (`N/fabric/mod.rs:2004`, `n = 0`), which replaces them at once (§22 C2). · (3) An idle node then commits nothing to `fabric/store.db` on the
pass (G14 asserts it). `PRAGMA incremental_vacuum` after the pass (`:845`) frees pages only when the freelist holds some.

**13.5 `journal_size_limit` (amends L5 §10.1, additive).** Both stores set `journal_size_limit` to 4 MiB (the lead's number, about the 1,000-page
auto-checkpoint, R04 fork 4), so the WAL file is truncated after a checkpoint instead of keeping its high-water size; today none of `journal_size_limit`,
`wal_autocheckpoint`, `cache_size` or `mmap_size` is set (R04 finding 5; `M/store/mod.rs:402-460`). `synchronous = FULL` stays where custody receipts need it
(`M/store/rows.rs:1153`). `synchronous = OFF` and a RAM write buffer are rejected: the receipt promises the hub holds the item.

**13.6 The migration copy and the rollback (amends L5 §10.3 and L1's update guard, additive; normative).** (1) Before any migration of `fabric/store.db` or
`node/state.db` (a `user_version` above 0 and below the build's): `wal_checkpoint(TRUNCATE)`, copy the file to `<file>.pre-v<N>` (N the version the build
writes; same key, same owner, 0600), `fsync`, then migrate. · (2) The daemon deletes `*.pre-v<N>` once the probation journal (`state/update.json`, `L1:141`)
shows the release confirmed. · (3) A probation revert runs release N's own `111 update-guard` before the flip (`ExecStartPre=+/usr/local/bin/111
update-guard`, `1-bootstrap/systemd/111node.service:22`; `revert_to`, `NI/update_guard.rs:20-33`). It gains one step: for each store with a `.pre-v<N>` copy,
it moves the store and its `-wal` and `-shm` aside to `<file>.rolled-back-v<N>` and renames the copy into place (owner `node111`, 0600). N−1 then opens a
store it reads, instead of refusing it (`SchemaTooNew`, `M/store/schema.rs:345-350`; R04 finding 12). What arrived during probation stays in the rolled-back
file, which the Storage page lists and only the user deletes. · (4) No signed release has shipped (`NI/manifest.rs:140-143`), so no field store has migrated:
Layer 7's V4 and in-flight V5 (`M/store/schema.rs:66`; `7-relay/TASKS.md:58`) run on development nodes only. The copy exists before the first migration after
the first signed release; if Layer 7's migration is that one, Layer 7's build takes this step (Q21, a cross-layer request, §21 L7).

**13.7 Integrity checks after an unclean shutdown (normative).** `PRAGMA quick_check` and `PRAGMA cipher_integrity_check` (page HMACs, R04 finding 10) run
once at open when the clean record is absent (`unclean`, `M/store/mod.rs:134-136`; the node-state store keeps the same record in `meta`), never on a clean
boot. On failure the store is moved aside to `<file>.damaged-<n>` and stays closed with that reason; the Storage page offers "Start an empty store (the
damaged file is kept)" and "Import an export" (§13.2). Never a silent re-creation (Layer 1's "never silent re-key", `L1:204`). Their time on an SD card is
measured at build.

**13.8 Child log rings (normative; Q6).** `open_log` opens a child's log create-and-append with no cap (`N/supervise.rs:752-755`). On the pass, a capped log
above `logs.max_file_bytes` (default 4 MiB, the drafter's) has its last half copied to `<name>.1`, replacing it, and is truncated to zero; the child keeps
writing at the end, as its descriptor appends (`O_APPEND`) (§22 D15). Capped: `agent/111-agent.log`, `agent/llama-server.log` (`N/intelligence.rs:274-281`)
and every child log the supervisor opens the same way. The journald namespace keeps its own cap (`SystemMaxUse=100M`,
`1-bootstrap/systemd/journald.111node.conf:6`).

**13.9 SD guidance (informative).** Datasets belong on an attached SSD on a hub, never on the boot SD (R02 finding 13); a register-in-place import writes
nothing. An endurance-class card is the docs' advice for a Pi node (no TBW figure is published, R04 fork 4). F2FS only where the user already runs it. **13.10
Costs.** No timer or process. One transient copy of a store per migration; one full read of a store after an unclean shutdown.

## 14. Sequencing and the first Layer 8 release (D34 F13)

**v1** = F1 (§2) · F2 (§3, built with Layer 9's first state) · F3 (§4) · F12 (§13) · F4 and F5 (§5, §6) · F6 (§7) · F8 (§9) · F9 (§10) · F10 (§11) · F11
(§12). **Build order**, the one the dependencies force (the board splits it into rows):
1. The consumer registry (§12.3) and the fabric-store fixes (§13.4, §13.5): Layer 5 code, no dependency on Layer 7's open rows.
2. The node-state store, the chat relay, the spool, the legacy import and the J9 API (§2, §3): before Layer 9's state (D35: "Layer 9 builds after Layer 8's
   state service").
3. The budget, the Storage page, the export offer, the log rings (§4, §13.8).
4. The migration copy, the integrity checks, the export verbs (§13.2, §13.6, §13.7).
5. The Range arm, the object tree, the catalogue and publishing (§5, §6, §7.1-§7.2): after Layer 7 row D7 (the origin's routes).
6. `get_to_file`, jobs, consent, the metered window, sneakernet (§7.3-§7.11): after Layer 7 rows D10 (consents) and D11 (the metered window).
7. The `child` panel source, the kiwix component, `kb.search` (§9).
8. The map bucket and the tiles routes (§10).
9. `origin.components`, the installer's origin list, the files table, the source tarball (§11).

**Later (v1.1)**: Tier 2 (§8) · automatic encrypted snapshots to the home hub (Tier 2 and a hub that opted in) · the Termux apt directory and the APK index ·
endorsements in the UI · a catalogue pointer by `BROADCAST` · Layer 9's hub fix shelf on the same tree · `zimdiff` bundles only after a measured gain. Every
check stays a netns check on this box; the Pi measurements wait for hardware.

## 15. Settings and consent

**15.1 Every Layer 8 setting (normative).** `state/storage.json`: JSON `{"schema": 1, …}`, 0600, written atomically and only on a change. Absent ⇒ every
default. Not a small regular file, or not JSON ⇒ moved aside to `storage.json.invalid-<n>` and every default (the `state/relay.json` rule,
`N/relay/settings.rs:16-36`). Unlike `relay.json`, an unknown key is kept and ignored, so a rollback to an older release never drops a consent (§22 C1).

| Key | Default | Where | Changed by | Consent |
|---|---|---|---|---|
| `keep_bytes` | `null` (= `keep_default`) | `state/storage.json` | a human, or an accepted `quota.set {class: "keep"}` | — |
| `keep_default` | computed once (§4.2) | same | nobody: an install-time fact | — |
| `caps.agent`, `caps.heal` | `null` (the share, §4.2) | same | a human, or an accepted `quota.set` | — |
| `logs.max_file_bytes` | 4194304 | same | a human | — |
| `follow` | `[]` (the home hub is always trusted) | same | a human only (Settings, or `111 dataset import --follow`) | — |
| `origin.datasets` | `false` | same | a human; `409 needs_hub` without the hub role, `409 lan_off` without LAN sharing | `datasets_serve` |
| `origin.components` | `false` | same | a human; as above | `components_serve` |
| `origin.max_streams` | 4 (1..=16) | same | a human | — |
| the L5 class quotas | unset, as built | `fabric/store.db` (`PATCH /api/v1/fabric/settings`, `API:1132-1145`) | a human, or an accepted `quota.set` | — |
| `metered` | `[]` | `state/relay.json` (Layer 7, L7 §12) | a human | Layer 7's |

**Inert by default.** With nothing configured, nothing a user wrote is removed before the user answered that class's export offer (§4.5), no file is served,
nothing is fetched, and the fabric's high watermark never drops below today's 512 MiB (`M/store/prune.rs:96`) unless a human set `keep_bytes`: until then the
budget only raises it. What does change by design (D34): the agent's state moves into `node/state.db` (§2) and the idle write stops (§13.4).

**15.2 Consent copy (verbatim; the pinned `ConsentRequired` round-trip, L7 §12).** **`dataset_fetch`** (subject `<publisher short>:<id>@<version>`; `addons`:
`metered`, `non_commercial` when they apply): "Download <name> <version>, <size>, from <source>. It is signed by <publisher label> (<publisher short>). Free
space now: <free>; after this: <free after>. Licence: <licence><, non-commercial>. <attribution> Used by: <used by>. Meant for: <fits> and up. Connection:
<connection> You can pause or cancel it at any time." `<connection>` is "No metered link is set." or "'<match>' is marked metered with a daily cap of <cap>:
this download pauses at the cap and takes about <days> days." · **`dataset_override`** (same subject): "Continue downloading <name> past today's cap on
'<match>'. This uses up to <remaining> more than the <cap> you set for this link, which may cost you money. It applies to this download only." ·
**`datasets_serve`** (subject the hub's NodeId, short): "Any device on this network can download this hub's dataset files without signing in: which
encyclopedias and maps it holds, their versions and their contents. A map file shows the area it covers, which tells visitors where this community is.
Requests are not logged. When you share these files, their licences ask you to keep their notices; this hub shows them on its Credits page." ·
**`components_serve`** (same subject): "Any device on this network can download the program files this hub cached for itself (<list>) without signing in. When
you share a program file, you are the one who passes it on under its licence: this hub serves each file with its licence notice and where its source is, which
is what those licences ask of you. 111 and its author do not distribute these files."

Each consent records its text hash; a changed text asks again. "You won't be warned in future" is the pattern refused (R05 finding 6). "Paused (offline)" and
"paused (metered cap)" are states, never errors.

**15.3 What the agent may propose, what only a human confirms (normative).**

| Action | The agent | A human |
|---|---|---|
| `keep`, a class cap, an L5 quota | proposes `quota.set {class, bytes \| null}` | accepts it, or edits the page |
| fetch a dataset | proposes `dataset.fetch {id, version}` | accepts, then accepts the consent dialog it opens |
| remove a dataset | proposes `dataset.remove {id, version}` | accepts |
| the metered override | never | a second explicit tap |
| a serving switch · following a publisher · publishing a catalogue | never | only |
| export · import | never (a CLI verb with the daemon stopped) | only |
| prune | never: the budget prunes by capacity | answers the export offer |
| start the encyclopedia | never; it may say it is off | opens the panel |

The bearer is refused on every non-GET route Layer 8 adds (D26 (2)); the agent never accepts a consent and never sees a consent hash.

**15.4 The Storage page (normative).** `/settings/l8`, "Storage", its own lazy chunk ≤ 64 KB (the `l6` precedent, `UI/src/views/settings/index.tsx:12-13`,
`:18`), replacing the generic page whose rows say "arrives with Layer 8" (`UI/src/views/settings/model.ts:222`). Sections: **Disk:** the gauge ("12.4 GB free
of 29.8 GB"), the floor, `disk_low`; the §4.7 copy. · **Space set aside:** `keep` with its default and rule; the class table of §4.1 (layer, bytes, cap);
editable caps; the export offers (§4.5); damaged and rolled-back store files (§13.6, §13.7) and unreadable agent files (§2.4). · **Datasets:** the home hub's
and the followed publishers' catalogues (newest `seq` and its source, §6.3); held entries with a `used_by` chip each (`map`, `encyclopedia`, `agent`,
`updates`), so "what do I lose" is answered before a delete (R05 finding 10); Get, Refresh, Remove; jobs with their progress and pauses. · **Publishing** and
**Serving** (hubs only): drafts and Publish; `origin.datasets`, `origin.components`, the stream cap. · **Export:** the §13.2 command lines. **Credits:** §9.7.

Every figure is computed on request (`GET /api/v1/storage`); the page refetches on a `storage` frame. Two lazy chunks of about 15 KB fit the lazy bucket
(219.9 of 256 KB, R05 finding 5); the chunk's size is measured at build.

## 16. The contract (additive; the binding form is `API` at build time)

**16.1 Routes.**

| Route | Method | Shape | Who |
|---|---|---|---|
| `/api/v1/storage` | GET | `StorageStatus` (§16.2) | session or loopback; the agent's bearer reads |
| `/api/v1/storage/settings` | PATCH | a partial `StorageSettings` (§15.1); `origin.*` answers 409 `consent_required` (`datasets_serve`, `components_serve`), `needs_hub`, `lan_off` | a human; bearer 401 |
| `/api/v1/storage/export-offer` | POST | `{class: "fabric" \| "agent" \| "heal"}` → 204 (§4.5) | a human; bearer 401 |
| `/api/v1/datasets` | GET | `{schema, publishers: [Publisher], holdings: [Holding], jobs: [Job]}` | session; bearer reads |
| `/api/v1/datasets/check` | POST | 202; one GET of the home hub's catalogue (§12.4) | a human |
| `/api/v1/datasets/{id}/fetch` | POST | `{publisher, version, accept_warning?, accept_override?}` → 202 `Job`; 409 `consent_required` (`dataset_fetch`; `dataset_override` at a cap), `insufficient_space`, `untrusted_publisher`, `job_running` | a human; bearer 401 |
| `/api/v1/datasets/{id}` | DELETE | `?publisher=&version=` → 204 (§7.9) | a human |
| `/api/v1/datasets/jobs/{job}` · `/resume` | DELETE · POST | cancel · resume a paused job | a human |
| `/api/v1/datasets/drafts` | GET · POST | the drafts · a first-copy job (§7.5; consent `dataset_fetch`) → 202 `Job`; 409 `needs_hub` | session · a human |
| `/api/v1/datasets/publish` | POST | `{drafts: [string], remove: [{id, version}]}` → 200 `{seq}` (§6.4); 409 `needs_hub` | loopback or a paired session, a human |
| `/api/v1/datasets/search` | GET | §9.4; 409 `component_unavailable` | session; bearer reads |
| `/api/v1/tiles` | GET | `{schema, sets: [{id, publisher, version, name, bytes, licence, attribution, url}]}` | session |
| `/api/v1/tiles/{id}` | GET, HEAD | the file through the Range arm (206, 416), `application/octet-stream` | session |
| `/api/v1/chat/history` | GET | unchanged shape, answered by `111d` (§2.3); 503 `store_unavailable` | as today |

**16.2 Shapes.**

```json
{"schema": 1,
 "disk": {"free": 12400000000, "total": 29800000000, "floor": 298000000, "disk_low": false},
 "keep": {"bytes": 2147483648, "default": 2147483648, "rule": "linux"},
 "node_state": {"state": "closed", "reason": null},
 "classes": [{"id": "fabric", "layer": 5, "bytes": 81234567, "cap": 1600000000, "budgeted": true, "export_offer_needed": false},
             {"id": "datasets", "layer": 8, "bytes": 7400028224, "cap": null, "budgeted": false, "export_offer_needed": false}],
 "notices": [{"kind": "unreadable_agent_files", "count": 0}]}
```

`node_state.state` ∈ `open`, `closed` (idle), `unavailable` (with `reason`); `keep.rule` ∈ `termux`, `hub`, `linux`, `user`. · `Publisher {node_id, label,
home, seq, source, entries: [DatasetEntry]}`; `DatasetEntry` = the §5.3 fields plus `state` (`absent`, `partial`, `present`, `in_place`, `unverified`) and
`non_commercial`. `Holding {publisher, id, version, sha256, bytes, location, state, used_by}`. `Job {id, publisher, entry, version, state, done_bytes,
total_bytes, source, metered: {match, cap, days} | null}` (states §7.4). · `FabricStatus` gains an optional `unconsumed {agent_op, kb, update, patch, sync}`
(§12.3). `Proposal.kind` gains `storage` (§16.4). `ConsentRequired.action` (`API:1497`) gains `dataset_fetch`, `dataset_override`, `datasets_serve`,
`components_serve`, appended. `Problem.code` (`API:692`) gains `insufficient_space`, `untrusted_publisher`, `job_running`, `store_unavailable`, appended.

**16.3 Events.** `storage` → `SseStorage` (a full `StorageStatus`, on a change of `disk_low`, `store_full` or `node_state`, or after a reclaim) and `dataset`
→ `SseDataset` (a `Job`, one per verified block while a job runs, and its end). Neither is sent on a timer. After `reset` the Layer 8 views refetch `/storage`
and `/datasets`.

**16.4 Proposal kind `storage`.** The body is one JSON object `{"why": string, "actions": [Action]}` from the closed vocabulary `quota.set {class, bytes or
null}` (class: `keep`, `agent`, `heal`, or an L5 refusable class) · `dataset.remove {publisher?, id, version}` · `dataset.fetch {publisher?, id, version}`
(`publisher` defaults to the home hub). Accepting it runs the routes' own validation, consent and refusal paths; a consent gate answers 409 `consent_required`
and the human repeats the decision with `accept_warning`. Any other action is `unsupported_action`; a body of another shape is `502 upstream_error`. It
renders as a card from the parsed JSON, never as Markdown.

**16.5 Agent tools (the Layer 3 set grows; fixed, shell-free, schema-validated).**

| Tool | Kind | Shape | Gate |
|---|---|---|---|
| `storage.read` | read | → `StorageStatus` and a summary of `/datasets` (no secret, no consent hash) | none |
| `kb.search` | read | `{query, book?, n?}` → §9.4, inside a labelled untrusted block | none |
| `storage.propose` | write intent | `{actions, why}` → a proposal `kind: storage` | a human accepts |

The count grows by three from the count the build finds: 17 at `f0b6806` (`A/tests/daemon_tools.rs:318`, `A/tests/chat_loop.rs:308`), 20 after Layer 7's
§13.3, 23 after this. Schemas beside `fabric.read` (`A/src/tools.rs:184`), dispatch beside `:330`. `proposal.write` takes only a title and a body
(`A/src/tools.rs:100-108`), so a typed proposal needs its own tool, the `device.propose` pattern (`:268`; §22 D1). The agent reads freely and writes nothing
(`3-intelligence/README.md:119-122`).

**16.6 The pin list (the conventions' form, `4-interface/api/README.md:127`; the change is not done until all are green).** (1) `Proposal.kind` +1
(`storage`): `KINDS` in `N/dashboard/api_v1/proposals.rs:104-114` (nine at `f0b6806`), the UI's proposal card, the agent's writer. · (2) The agent tool count
+3: both tests, the `DAEMON_TOOLS` list, `A/src/tools.rs` schemas and dispatch. · (3) `pnpm gen:types`, then `node scripts/gen-sse.mjs --check`; the new enum
tuples and aliases (`StorageClass.id`, `Problem.code`, `ConsentRequired.action`, `Proposal.kind`, `Job.state`, the two events) are listed in
`UI/scripts/gen-sse.mjs`. · (4) `x-sse-events.events` (`API:2044-2059`) gains `storage` and `dataset`; the UI store's `switch (frame.event)` handles them. ·
(5) The contract tests: `UI/test/gen.test.ts`, `UI/test/http.test.ts`, a new `UI/test/layer8-contract.test.ts` (the `layer7-contract.test.ts` precedent), and
the mock adapters, whose frames validate against `x-sse-events`. · (6) node111's mirrors in `dashboard/api_v1/`: the `Problem` codes, the `ConsentRequired`
actions, the class ids and the job states.

**16.7 Origin routes (outside `API`; Layer 7's §9.1 posture).** Read-only, unauthenticated, GET and HEAD only (405 otherwise), no CSRF, the same CSP, on the
existing listener, outside the session gate; listed with `/origin/v1/*` in the conventions' "Not gated" list (`4-interface/api/README.md:210`) and Layer 1's
machine contracts.

| Route | Served while |
|---|---|
| `/origin/v1/blob/{sha256}` (Layer 7's) | its pin source's switch (§11.1); through the Range arm and the stream cap |
| `/origin/v1/catalogue/{publisher hex}.toml` · `.toml.sig` | `hub_role`, LAN sharing, `origin.datasets` |
| `/origin/v1/notice/{sha256}` (`text/plain`) | `hub_role`, LAN sharing, and `origin.components` (or `origin.serve` for the release's source) |

## 17. Security, privacy and legal (threat model; hard rules)

**Hard rules.** (1) **The agent holds no key and writes nothing.** No route Layer 8 adds accepts the bearer on a non-GET (`N/dashboard/api_v1/auth.rs:931`);
the node-state store's key never leaves `111d`. · (2) **Never delete what the user fetched or wrote by policy.** No dataset is removed automatically; no
undecided proposal, running task, open incident or armed undo is ever pruned; no class prunes own history before its export offer. · (3) **Never serve an
unverified byte.** A blob is served only when a pin list the hub holds names it (§11.1) and its bytes passed: sha256 at import or publish, BLAKE3 per block on
a pull. · (4) **The release key never signs a dataset;** the node key signs a catalogue only on a human action, under its own tag. · (5) **Never fetch without
the consent** that names size, free space, licence, signer, `used_by` and connection; never past a metered cap without the override; never on a carrier or
LoRa. · (6) **kiwix-serve is never LAN-bound;** it is reached only through its panel, the hub's session and its own origin. · (7) **No wall clock** orders,
freshens or prunes anything: `seq`, the boot id and `CLOCK_BOOTTIME` do. · (8) **No telemetry:** requests are never logged; nothing about a node leaves it
except what a hub serves under a recorded consent. · (9) **Export never over HTTP;** restore verifies the signature before decrypting. · (10) **Encryption is
described honestly** (§2.1): copies and backups, not a stolen disk with a readable `identity/`, not a same-uid process.

**What is exposed, to whom:**

| Path | On the path | At the far end |
|---|---|---|
| a member's catalogue check or dataset pull (LAN, plain HTTP) | IPs, timing, sizes, the hashes asked for | the hub: which member pulled which file, when (never logged) |
| `origin.datasets` | — | anyone on the LAN: the holdings, versions, contents, a map's area |
| `origin.components` | — | anyone on the LAN: which programs the hub cached |
| the kiwix panel, the map tab | as the dashboard | the hub's session holder only |
| a first copy from upstream | the hub's IP and the file to the network | the mirror's operator: the hub's IP, the file, the time |
| an export file | — | whoever holds it: its size; a holder who guesses the NodeId can check the signature |

**Attacks and defences.** **A hostile mirror or hub serves wrong bytes:** the block fails `valid_ranges` and is fetched from the next source; a whole file is
never accepted on its name. · **A swapped sidecar:** `valid_outboard_ranges` fails against the signed root before any data byte moves. · **A forged or foreign
catalogue:** the signature fails under the pinned key, or the publisher is not followed and nothing is listed. · **A replayed old catalogue (freeze):** a
lower `seq` is refused; a stale mirror can only withhold, until the node reaches another source (§6.3). · **A publisher that signs a sha256 that does not
match its BLAKE3:** nodes check BLAKE3 and serve the bytes under the signed name; only that publisher's followers are affected; an importer that compares
upstream sha256 sees the mismatch. · **A ZIM's JavaScript:** it runs in the panel's own origin (L5 §13), never the dashboard's; ZIMs come only from catalogues
the user follows. · **Bulk abuse on the LAN** (a 57 GB route hammered): four streams at most, single ranges only, 64 KiB streaming, no multipart body. ·
**Path tricks in a catalogue:** an `id` matches a fixed pattern and is never a path; files are named by hash. · **Filling the disk through fetches:** the
free-space check before any byte and the floor (§4.2). · **A compromised agent:** it reads what GETs return; it cannot write, fetch, serve, prune or accept.
The same-uid residual is stated (§2.1). · **A tampered export:** the signature over the BLAKE3 prehash fails before any decryption. · **A store swapped at
rollback:** the guard renames only its own `.pre-v<N>` copy, same owner, same key; the moved-aside file is kept.

**Legal (facts from the texts; the readings are the owner's).** Serving a ZIM is "Sharing" under CC BY-SA 4.0 s.3(a); a repack or subset is arguably Adapted
Material (unverified legal read). Serving OSM-derived tiles to a neighbourhood is publicly conveying under ODbL s.1 and s.4.2, and a publicly used Produced
Work needs the s.4.3 notice (R01 finding 12). A hub that mirrors a third-party binary is the one who conveys it under GPL-3.0 §6(d), GPL-2.0 §3, MPL-2.0
§3.2(a), AGPL-3.0 §6, Apache-2.0 §4 and the Reticulum License's notice clause (R07g2 §3.4). Non-commercial content may be listed and served, tagged, never
preselected; 111 refuses nothing (Q11).

## 18. Verification (lock gates; netns, no hardware)

**Gate homes.** New `8-storage/verify/{state,budget,datasets,kiwix,maps,components}.sh`, a stdlib-only stub mirror `8-storage/verify/stub-mirror.py` (serves a
file and, on request, one corrupted block), and `8-storage/verify/lib.sh`, which sources `6-transport/verify/lib.sh` and `6-transport/verify/root.sh` as
`7-relay/verify/lib.sh:14-19` does and names the netns, veth, address and port names (names only). The conventions are `6-transport/verify/lib.sh:1-29`: each
check prints `ok:` or `FAIL:` (`note:` informs); a gate whose prerequisites are absent (unprivileged netns through `unshare -Urnm`, a built binary,
`KIWIX_BIN`, a fixture) prints `<GATE> SKIPPED: <reason>` and exits 0 unless its `REQUIRE_<GATE>=1` (a lock run) makes it a FAIL; the last line is `<GATE>
GREEN` or `<GATE>: N FAIL`. Every harness stops what it started, leaves nothing listening, and runs alone (AGENTS.md, Linux hygiene).

**Seam ids (D36 (2), D37).** The ids are the cross-layer map's: Layer 8's planned rows (`plan/cross-layer/test-system/matrix.md:177-178`, entering
`verify/seams.toml` under D37 F5) plus one new row, `L6-L8:server-component`. Layer 8's board carries the verify row that registers these gates in
`verify/checks.toml` and promotes the seams to built, or names a gap (D37 F5). Each `ok:` and `FAIL:` line starts with its seam id in brackets: the phase
prefix `checks.toml` maps to a seam (D37 §3.5), so the runner needs no adapter of its own for these gates.

| Seam id | Layers | What Layer 8's gates prove there | Gates |
|---|---|---|---|
| `L3-L8:agent-state` | 3 ↔ 8, through `111d`'s relay | the chat body's `history` and the `done` frame's `messages`, the history route, the spool and its events, the legacy import | G13 |
| `L1-L8:state-service` | 1 ↔ 8, on 5's store code | the node-state store's key, open and modes; the budget on the housekeeping pass, `set_watermarks`, no idle write, the log ring | G13, G14 |
| `L8-L9:heal-store` | 8 ↔ 9 | the J2 files, the J3 marker, the J9 API (a fake Layer 9 writer until Layer 9 builds) | G13 |
| `L5-L8:tier2-objects` | 5 ↔ 8 | the consumer registry (no message row for 0x10-0x14); the sidecar on Layer 5's BLAKE3 roots; Tier 2 later | G14, G15 |
| `L2-L8:dataset-signing` | 2 ↔ 8 | `111/catalogue/v1`, `111/export/v1`, the `111/node-state/v1` vector | G13, G15 |
| `L7-L8:move-datasets` | 7 ↔ 8, and 1's fetcher | `/origin/v1/*`, the admission rule and the switches, the Range arm, the stream cap, `get_to_file`, the consent round-trip, the metered window | G15 |
| `L4-L8:hub-content` | 4 ↔ 8, and 5's panel host | `/api/v1/storage*`, `/datasets*`, `/tiles*`, the two frames, `/settings/l8`, the `map` bucket, the `child` panel, `kb.search` | G14, G16, G17 |
| `L1-L8:updates-backup` | 1 ↔ 8, and 6's cache | the migration copy and the guard's restore, `111 store export` and `import`, the installer's origin list, the files table, `origin.components` | G13, G18 |
| `L6-L8:server-component` (new) | 6 ↔ 8 | a declaration with no `adapter`, `start = "on_demand"`, zero weight while off | G16, G7 |
| `L8-L8:shared-log` | 8 | nothing in v1: the gap text "no log in v1 (§12)" | none |

**G13, the node-state store and the joint contract** (`state.sh`: one netns node with the real `111d`, the real `111-agent` and a stub model, the Layer 4
smoke pattern; plus node111, mesh111 and agent111 tests). Asserts: (1) `[L1-L8:state-service]` `node/` is 0700 and `state.db` 0600; the file opens only with
the `111/node-state/v1` key (a known-answer vector beside `store-kdf.json`); a plaintext SQLite is refused; a wrong key fails on page 1. · (2)
`[L3-L8:agent-state]` A turn through `POST /api/v1/chat` writes no `agent/conversations/*.jsonl`; the store holds its messages; the browser's `done` frame
carries no `messages`; `GET /api/v1/chat/history` returns them with the agent stopped; a sentence of the turn is found in no file under the root after the
store closes (a byte search), logs excepted. · (3) `[L3-L8:agent-state]` A browser that leaves mid-turn: the turn is saved when the agent's `done` arrives. ·
(4) `[L3-L8:agent-state]` A proposal is ingested on its event (the file is gone, the record is there); with `111d` killed between the agent's write and the
ingest, the start-up sweep ingests it; a temp file is never ingested; a file that does not parse stays and is counted; a running task's file stays. · (5)
`[L3-L8:agent-state]` The legacy import: a root seeded with v0 agent files ends with the files gone and the records in id order. · (6) `[L3-L8:agent-state]`
Every non-GET route Layer 8 adds answers 401 to the agent's bearer. · (7) `[L8-L9:heal-store]` With a fake Layer 9 writer (node111 tests): the J2 files are
folded and cleared at start; with the store unopenable they stay and the status names the reason; a reclaim never touches `open` or `armed` records and leaves
one marker. · (8) `[L1-L8:updates-backup]` (node111 tests on a scratch root, the L7 I14 precedent) Opening a store one version behind writes `<file>.pre-v<N>`
before the migration; the guard's revert restores it and keeps `.rolled-back-v<N>`; a confirmed probation deletes the copy. · (9) `[L1-L8:updates-backup]`
`111 store export` refuses while the daemon runs; with it stopped it writes the blob; one flipped byte makes `111 store import` refuse before decrypting; a
round trip restores both stores and they pass `quick_check`. · (10) `[L1-L8:state-service]` After `kill -9` the next open runs `quick_check` and
`cipher_integrity_check` once; after a clean stop it runs neither. **Pass:** `G13 GREEN`.

**G14, the budget** (`budget.sh`: one node whose root is a small tmpfs mounted inside the user namespace; node111 tests). Asserts: (1) `[L1-L8:state-service]`
The default `keep` follows §4.2 for this category, and `state/storage.json` is written once. · (2) `[L1-L8:state-service]` `GET /api/v1/fabric`'s `watermark`
equals the fabric's share, and never below 512 MiB with `keep_bytes` unset. · (3) `[L1-L8:state-service]` Over a test-sized `agent` cap: nothing goes before
the export offer; after it, decided proposals go first, then conversations by `seq`; an undecided proposal, a running task and the conversation of a turn in
flight stay. · (4) `[L1-L8:state-service]` Below the floor: `disk_low` is set, a dataset job pauses, a draft answers `insufficient_space`, a message is still
accepted. · (5) `[L1-L8:state-service]` **No idle write:** with no traffic over three capacity passes, `fabric/store.db`, its WAL and `node/state.db*` are
unchanged (size and mtime), and the node-state store is closed. · (6) `[L1-L8:state-service]` A child log past the cap holds at most the cap after the next
pass, `.1` holds its previous tail, and the child keeps writing. · (7) `[L4-L8:hub-content]` `GET /api/v1/storage` lists the closed class set with figures
that add up, and writes nothing; a `storage` frame is sent only on a change. · (8) `[L5-L8:tier2-objects]` (mesh111 `tests/consumers.rs`) An inbound `SYNC`,
`KB`, `UPDATE`, `PATCH` or `AGENT_OP` from a contact never becomes a message row; it is acknowledged and counted; with a consumer registered it reaches the
consumer once. **Pass:** `G14 GREEN`.

**G15, datasets** (`datasets.sh`: hubs H and H2 and a member M on one bridge in 10.111.8x.0/24, the stub mirror; node111 tests). Asserts: (1)
`[L2-L8:dataset-signing]` `111 dataset add` on H computes sha256, BLAKE3 and the sidecar in one pass; the sidecar's size is `(blocks − 1) × 64`; publishing
signs under `111/catalogue/v1`; one flipped byte fails the check. · (2) `[L7-L8:move-datasets]` With `hub_role`, LAN sharing or `origin.datasets` off, the
catalogue and the blobs answer 404; with all on they are served with no session; a non-GET answers 405. · (3) `[L7-L8:move-datasets]` The Range arm: 206 with
`Content-Range`; `bytes=-n`; 416 past the end and for two ranges; an `If-Range` mismatch serves 200; `Content-Length` always; never gzip or chunked; a fifth
concurrent blob body answers 503. · (4) `[L7-L8:move-datasets]` M's fetch: the first call answers 409 `consent_required` whose text names size, free space,
licence, signer, `used_by` and the connection; the repeat with the hash answers 202; `dataset` frames flow while the job runs and stop after it; the file
lands by rename and no `.part` remains. · (5) `[L7-L8:move-datasets]` A job stopped mid-file resumes at the next missing block (bytes fetched ≤ size plus one
block); a corrupted block from the stub is fetched again from H2 (one block of extra bytes); a swapped sidecar fails before any data byte moves. · (6)
`[L7-L8:move-datasets]` On an interface marked metered with a test cap the job pauses at the cap, its bytes count in Layer 7's window, and the
`dataset_override` consent continues it; the bearer cannot accept either consent. · (7) `[L2-L8:dataset-signing]` A catalogue from an unfollowed publisher is
not listed; the CLI import refuses without `--follow`; a lower `seq` is refused after a higher one. · (8) `[L7-L8:move-datasets]` `111 dataset import --from
<dir>` verifies and copies; `--in-place` writes nothing under `state/origin/blob/`; after an unclean shutdown the in-place file is `unverified` and is checked
before it is served. · (9) `[L1-L8:state-service]` Filling the budgeted classes past `keep` removes no dataset; a fetch that does not fit answers
`insufficient_space` before any byte; `DELETE` removes the file and its sidecar. **Pass:** `G15 GREEN`.

**G16, the encyclopedia** (`kiwix.sh`; `KIWIX_BIN` a pinned kiwix-tools tarball and `KIWIX_ZIM` one small ZIM (Q9), else `G16 SKIPPED` unless
`REQUIRE_G16=1`). Asserts: `[L6-L8:server-component]` declared and disabled, and enabled but never opened, kiwix-serve has no process, listener or open file
(G7's `zero-weight.sh`) · `[L4-L8:hub-content]` the first panel session starts it on 127.0.0.1:11250 only (`ss`), it answers through the panel listener with
the cookie and 403 without it, a WebSocket upgrade is refused · a held ZIM added or removed rebuilds the library and sends `SIGHUP`, with no restart (the same
pid) · Stop leaves no process · `[L4-L8:hub-content]` `kb.search` returns at most 8 KiB in an untrusted block while it runs and 409 `component_unavailable`
while it is off; the bearer's session mint answers 401. **Pass:** `G16 GREEN`.

**G17, maps** (`maps.sh` with a small PMTiles fixture, `PMTILES_FIXTURE`, made off-node with go-pmtiles; UI tests). Asserts: `[L4-L8:hub-content]` `GET
/api/v1/tiles` lists the held set; `/api/v1/tiles/{id}` serves ranges and needs a session · `check-budget.mjs` counts the `map` chunk in the exempt bucket, ≤
90 KB, and the lazy total without it is unchanged · the map chunk is never fetched when no set exists · the CSP is unchanged (`connect-src 'self'`, no
`blob:`) · the ODbL sentence is in the rendered map. **Pass:** `G17 GREEN`.

**G18, component pins from a hub** (`components.sh`; node111 tests with a fixture declaration; the real `deltachat` pin when `cache/packages/deltachat/` holds
it, else that item SKIPPED unless `REQUIRE_G18=1`). Asserts: `[L1-L8:updates-backup]` with `origin.components` off a component blob answers 404; on (after the
consent) only a pinned hash is served, from `cache/packages/`, with its notice; a member with no internet route installs the component from its hub (egress
log), and a 404, a wrong size or a wrong hash falls through to the upstream URL; a pip closure arrives through the files table; the `lxmf` closure answers 404
(Q25); removing `cache/packages/<id>/` ends the serving. **Pass:** `G18 GREEN`.

**G7, extended:** `zero-weight.sh` with kiwix declared and disabled; Layer 7's `idle.sh` no-timer line unchanged with Layer 8 present (L7 §3.7). **Pass:**
both.

**Unit and integration test homes.** mesh111: `src/store/tests.rs` (save on change, `journal_size_limit`, the migration copy, `open_encrypted`),
`tests/consumers.rs` (new); its vectors in `5-messaging/test-vectors/`. node111 (`1-bootstrap/crates/node111/tests/`, 35 files today): new `node_state.rs`,
`storage_budget.rs`, `range.rs`, `datasets_origin.rs`, `update_guard_store.rs`, `export.rs`, `panels_child.rs`, `install_origin.rs`. agent111:
`tests/chat_loop.rs` (`history` in, `messages` out; the tool count), `tests/daemon_tools.rs` (the three tools), new `tests/store_atomic.rs`. UI: new
`test/layer8-contract.test.ts`, `test/layer8-views.test.ts`; `test/budget.test.ts` (the `map` bucket).

**CI mapping** (`.github/workflows/ci.yml`; parity with the local gates, `plan/cross-layer/ci-cd.md`): the mesh111 and node111 tests and the UI checks behind
G13-G18 run in `bootstrap-gate` (`1-bootstrap/scripts/check.sh`, which nests `5-messaging/scripts/check.sh` and `pnpm check`); the agent's tests in
`intelligence`; contract drift (`pnpm gen:types` leaves the tree unchanged) and the KB entries in `knowledge-contract-hygiene`. The netns harnesses and G7
stay the lead's: the runners restrict `unshare -Urnm` (`plan/cross-layer/ci-cd.md:18-19`). Root-only `check.sh` failures on the build box are recorded, never
counted as tree faults (D29).

## 19. Pi-floor accounting

| Item | Every Layer 8 switch off | Idle node, against Layer 7 | A chat turn | Hub, `origin.datasets` on, idle | A dataset job | The kiwix panel open | Basis |
|---|---|---|---|---|---|---|---|
| processes | 0 | 0 | 0 | 0 | 0 | +1 (kiwix-serve) | design |
| listeners | 0 | 0 | 0 | 0 (routes on :11180) | 0 | +1 loopback (11250) | design |
| threads | 0 | 0 | 0 | 0 | +1 (`111d-dataset`) | the runner's, as any child | design |
| timers | 0 | 0 (the pass is the fabric's) | 0 | 0 | 0 | kiwix's own 1 Hz wake | R07g3 §1 |
| open files | 0 | 0 (the node-state store is closed) | +3 (`state.db`, `-wal`, `-shm`) until the next idle pass | each file while it streams | the `.part`, the sidecar, a socket | the ZIMs | design |
| writes | 0 | −5.9 MB/day (the idle commit removed) | ≈ 12.4 KB WAL, ≈ 25 KB with the checkpoint | 0 | the file and its sidecar once, one `fsync` | measured at build | R04 finding 7; R07g1 §2 (derived) |
| memory | 0 | 0 | the store's page cache, measured at build | 64 KiB per stream | one block buffer (16 MiB or 4 MiB) | RSS measured at build | R07g3 finding 9 |
| disk | 0 | `state/storage.json` | the node-state store | the served files | the dataset and its sidecar | the tarball, measured at build | design |

**Build-time weight, not resident:** the `111` binary delta and the `111-agent` delta (measured at build); the Storage chunk (≤ 64 KB); the `map` bucket,
about 84 KB gzip (a scout's esbuild figure, R05 finding 13, measured again at build); +0 crates.

**Rule.** The build records each figure, measured on the release binaries, in `8-storage/TASKS.md` beside Layer 6's (`111` 19,404,064 B at 19.7 MB idle RSS,
`111-agent` 4,392,280 B at 5.3 MB, listeners :11180 (+ the best-effort :80) and :11182, `6-transport/TASKS.md:82`), with the protocol of OB §3: `VmRSS` and
`Pss` from `/proc/<pid>/smaps_rollup` (anonymous against file-backed, which settles kiwix-serve's figure), `ss -ltnup` before and after, `strace -c -f -e
trace=write,fsync` over ten idle minutes, `/proc/<pid>/io` `write_bytes` over an idle hour and `/sys/block/mmcblk0/stat` (§13.4's 5.9 MB a day), binary and
cache sizes, and the time and reads the budget adds to the pass with the node-state store open. A change to the first two columns fails the gate until the
owner says otherwise.

## 20. Deferred / later versions

| Item | What it needs |
|---|---|
| Tier 2 transfer; hub-to-hub bulk over the internet | `bao-tree` 0.17, bulk on its own connection or a non-blocking `Link::send`, the Layer 5 amendment list Layer 8 writes (§8) |
| automatic encrypted snapshots to the home hub | Tier 2 and a hub that opted in; the §13.1 format |
| a live export | a verified live-snapshot route on SQLCipher (the backup API or `VACUUM INTO`) and `rusqlite`'s `backup` feature |
| Mode T for both stores | a hardware-sealed key (L5 §10.2) |
| a sealed spool (no plaintext proposal on flash) | a sealing key the agent may hold: an L2 amendment (OB F1 M6; Q3) |
| endorsements in the UI · a catalogue pointer by `BROADCAST` | `111/catalogue-vouch/v1` built · L7 §11.1's consumer |
| the hub fix shelf | a catalogue class `patch`, Layer 9's later version |
| the Termux apt directory and the APK index | `termux-apt-repo` and `fdroidserver` on a publisher, a hub-owner key (OB F10 B) |
| WebSocket or SSE panel children, `detected` panels | P7 |
| datasets pruned below 1 % free, as an opt-in | the owner's opt-in (OB F3 C) |
| raster maps; extracts rebuilt from OSM data | the CSP `blob:` question; the Produced Work legal read (Q10) |
| `zimdiff` bundles | a measured Wikipedia-to-Wikipedia gain |
| a shared log | §12.2's five conditions; p2panda-core |
| rqbit or iroh-blobs as hub components | Range pulls and Tier 2 falling short (Q12); Syncthing never |
| the `lxmf` wheels on `origin.components` | the owner's upstream contact (Q25) |
| a phone kiwix path | the Termux closure measured (Q17) |

## 21. Cross-layer seams & amendments

- **Layer 5 (locked v1.2), six additive spec amendments:** (1) §10.1: `journal_size_limit` (4 MiB) joins the pragma list, and "One DB: `fabric/store.db`"
  (`5-messaging/README.md:425`) gains "Layer 8 keeps a second file, `node/state.db`, for the node's own state" · (2) §10.3: the migration copy before any
  migration (§13.6) · (3) §10.4 step 3 (`:460`) and §21 (`:865`): "`age`-based export" → "the L2 §8.5(i) export (Layer 8 §13.1)" · (4) §11 (`:477-479`): the
  updater intake at `fabric/blobs/<hash>` → superseded by `state/origin/blob/<sha256>` (L7 §9.1); "`KB` deltas and git bundles (Layer 7)" → the KB rides the
  release payload (L7 §9.4; git bundles were rejected, D30 F6); `SYNC` stays reserved with no consumer in v1 (§12; no cursor table and no prune step for
  `Class::Sync`, R03 finding 10) · (5) §13: `child` panels are built for plain-HTTP children (§9.1); WebSocket and SSE children and `detected` stay reserved ·
  (6) the dispatch: the consumer registry for 0x10-0x14 (§12.3), shared with Layer 9's F13 item 13. **Code seams** (no spec text, no wire byte): S1
  `derive_tagged_key`, S2 `open_encrypted` and `migrate_with` (§2.7), `save_capacity` on change with the disk figures unpersisted (§13.4; the module note
  `M/store/prune.rs:81-83` changes with it). No wire byte changes in v1. L5 §19's Layer 8 row ("`SYNC` class + per-peer cursors; the append-only log is an
  opaque consumer", `:830`) is closed by §12.
- **Layer 7 (locked v1.0), additive:** (1) §9.1: the tree holds datasets, sidecars and the catalogue files; `/origin/v1/catalogue/*` and `/origin/v1/notice/*`
  join the routes; one admission rule over three pin sources (§11.1); `origin.datasets` and `origin.components` gate their own files and live in
  `state/storage.json` (§22 C1); the Range arm and the stream cap; the release's source tarball served beside its payload (§11.6) · (2) §10.5: "≈ 69 KB"
  (`7-relay/README.md:951`) → 83.9 KB, bucket 90 KB · (3) §12: dataset fetch bytes count in the same window as carrier bytes; `ConsentRequired.action` gains
  four values after Layer 7's four · (4) §17: the Tier 2 row gains its owner, Layer 8 (later); the map bucket row (`:1265`) moves to Layer 8 v1 · (5) §9.5 T6:
  `UserTypedDataset` joins the typed exceptions. **Effect, no text:** an unset `quota.transit` scales with the watermark the budget sets (§4.2 item 4).
  **Cross-layer request:** the migration copy in Layer 7's build if its migration is the first after a signed release (§13.6 item 4). §9.6's "signed by their
  publisher" (`:923`) is made concrete by §6.
- **Layer 6 (locked v1.0):** §2.1 rule 1: `adapter` optional, `[run] start = "on_demand"` (§9.2) · §2.1 rule 3 (`6-transport/README.md:145-146`): the
  install-time fetch tries the home hub's origin first (§11.3); "There is no runtime fetch" stands · G7 covers a declared, disabled kiwix.
- **Layer 4 (locked):** the §16 contract and its pin list · `/settings/l8` · the `map` bucket in `check-budget.mjs` · `4-interface/README.md:717` "storage
  (L8)" leaves the deferred list · `UI/src/views/settings/model.ts:222` · the conventions' "Not gated" list (`4-interface/api/README.md:210`) gains
  `/origin/v1/catalogue/*` and `/origin/v1/notice/*`.
- **Layer 3 (locked):** an additive note under `3-intelligence/README.md:50`: "As built by Layer 8: conversations, proposals, tasks and decisions live in
  `111d`'s encrypted `node/state.db`; the agent holds no key, sends nothing but GETs, and spools proposals and tasks under `agent/` until `111d` ingests them
  (Layer 8 §2)." The agent contract (`3-intelligence/TASKS.md:22`, `:27-28`, `:37`): the chat body's `history`, the `done` frame's `messages`, proposals and
  tasks as an atomic spool. Tools +3 (§16.5). The `conversations` count comes from `111d`.
- **Layer 2 (locked v2):** one block §13.3 "Layers 8 and 9 amendments": **A19** domain tags: `111/catalogue/v1` (Ed25519 by the publisher over `tag ‖
  publisher ‖ catalogue bytes`, §6.1) · `111/export/v1` (Ed25519 by the node over `tag ‖ NodeId ‖ length ‖ BLAKE3(the blob before the signature)`, §13.1) ·
  `111/node-state/v1` (a key-derivation input, the `111/store/v1` pattern of A9) · reserved `111/catalogue-vouch/v1` · Layer 9's `111/fix/v1` and
  `111/fix-vouch/v1` (J5; their constructions are Layer 9's). **A20:** §8.5(i) is built (§13.1): its tag, the chunked framing, the prehash. The root node key
  signs catalogues and exports (`L2:91`, control artifacts).
- **Layer 1 (locked v2.2):** layout rows: `node/` (0700; `state.db` 0600) · `state/storage.json`, `state/datasets.json` (0600) · `state/origin/` widened
  (`blob/` holds datasets, sidecars and the source tarball; `catalogue/`, `incoming/`, `drafts/`) · `state/kiwix/` · `state/heal/` (0700, Layer 9, J2 and J7)
  · `*.pre-v<N>`, `*.rolled-back-v<N>`, `*.damaged-<n>` beside a store · verbs `111 store export|import` and `111 dataset add|import` (the subcommand set,
  `NI/cli.rs:153-192`) · the fetcher's `get_to_file` and `UserTypedDataset`; `PayloadFetcher::fetch_pin` with the origin list (§11.3); the manifest's
  `[source]` pin (§11.6) · the update guard's store restore (§13.6) · the child log ring (§13.8) · text: "a USB drop into the same directory is the offline
  alternative" (`L1:252-266`) gains "or the home hub's origin (Layer 8 §11)"; "Updater intake (seam)" (`L1:247-248`) is superseded by `state/origin/blob/`;
  the machine contracts (`L1:162`) gain `/origin/v1/catalogue/*` and `/origin/v1/notice/*` · dependencies, signed off under "Dependency policy" in
  `1-bootstrap/TASKS.md` with the `blake2 = "=0.11.0"` precedent (`1-bootstrap/crates/node111/Cargo.toml:69-71`): direct `rusqlite` (=0.40.2,
  `bundled-sqlcipher-vendored-openssl`), `bao-tree` (=0.16.1), `blake3` (=1.8.7); +0 crates.
- **Layer 0:** `candidate` entries `projects/kiwix-tools.md`, `projects/pmtiles.md` (PMTiles, go-pmtiles, protomaps-leaflet, Leaflet) and
  `projects/p2panda-core.md` (the named log target), at the desk-research cap (`0-knowledge/README.md:93`); `projects/age.md`'s summary "planned for history
  export only" (`:5`) → "a possible format for hub-side tooling; 111's export uses the L2 §8.5(i) format (Layer 8 §13.1)".
- **`LICENSING.md` and `DECISIONS.md`:** §5 rows for the direct edges (no new crate); §8 rows for the hub's conveyance duty (§11.7), kiwix-tools as a
  component and the three map packages (Leaflet BSD-2-Clause, protomaps-leaflet and pmtiles BSD-3-Clause), with `THIRD-PARTY-NOTICES.md` regenerated from the
  UI lockfile; `:144`'s 0.17 note stands until Tier 2. D26 (5) gains the §11.7 sentence; D21 gains a note: the exempt `map` bucket, 90 KB gzip (D30 F9
  recorded the bucket with no figure and stands).
- **Hand-off to Layer 9:** §3 (J1-J9), cited by number; the consumer registry for 0x10 and 0x13 (§12.3); the hub fix shelf, later: a catalogue class `patch`
  on the same tree, signed by its author and vouched for with `111/fix-vouch/v1`; Layer 8 adds storage, a class and the Range path, never a format (R01
  finding 9). In v1 Layer 9's fix artefacts (under 4 KiB, a 64 KiB attachment cap) travel as sealed `PATCH` messages (OB §3).
- **Hand-offs this layer closes:** Layer 3 "State ← Layer 8" (§2, §3) · Layer 4's Storage stub and its deferred "storage (L8)" (§15.4) · Layer 5's prune
  classes, `set_watermarks` and the export offer (§4), Tier 2's owner (§8) and `SYNC` (§12) · Layer 7's origin store, the dataset signer
  (`7-relay/README.md:923`), the map's region file (`:1265`) and the unverified sizes (this spec states none: a size is a catalogue entry's signed `size`) ·
  Layer 1's KB in the release payload (unchanged, D30 K1) and its on-disk layout.

## 22. Plan corrections recorded

**From the brief §4 (items 1-14), with what this spec does.** (1) README §8 ("an encrypted local store", "software updates"): **not reworded** (D36). The
honest scope is the v1 scope-limit table of §0. · (2) `3-intelligence/README.md:50` is false as built: an additive note (§21 L3). `A/src/store.rs:1-3`
promises atomic writes while `save_proposal` and `save_task` use `std::fs::write` (`:139`, `:145`): fixed under every option (§2.4 rule 1). · (3)
`5-messaging/README.md` §10.4 step 3 and §21, §11's intake path, §11's "git bundles", §11's `SYNC` cursors, §10.1's "One DB": §21 L5 (1)-(4). · (4)
`7-relay/README.md`: §10.5 "≈ 69 KB", §12, §9.1 and the GPL source offer, §17: §21 L7 (1)-(4). · (5) `1-bootstrap/README.md`: the layout rows; the components
text, now at `:252-266` (the brief cited `:227-246`); `:160` "APK refresh = Layer 9 maintenance task" is Layer 9's (text unchanged here). · (6)
`6-transport/README.md`: the origin list, now at `:145-146` (the brief cited `:113-117`); `adapter` optional. · (7) `2-identity/README.md` §13.1 A4: the tags
join the registry (§21 L2, as §13.3, C4); §8.5(i) names its tag. · (8) `LICENSING.md:56`: **already done** ("Text verified 2026-11-01", read at `f0b6806`);
`:144` already carries the 0.17 note; the §8 rows and the notices: §21. · (9) `plan/DECISIONS.md`: the D26 (5) sentence and the D21 note (§21). · (10)
`0-knowledge/projects/age.md:5` and the new candidates (§21 L0). · (11) `4-interface/README.md` "storage (L8)", now at `:717` (the brief cited `:697`);
`UI/src/views/settings/model.ts:222` (cited `:221-222`). · (12) This folder's research: R01 §4's "Cat 4A on Pi" rows contradict
`NI/install/categorize.rs:102-105` (Cat 4A needs ≥ 512 GB free and ≥ 8 GB RAM); R06 §4's "3,396 × 64 B" is 3,397 pairs = 217,408 B; R01 and R02 quote the
March 2024 mini (13 GB) without the November 2024 figure (7.4 GB); the plan README's "cap 2" gap-fill scouts reads three. For the lead, with the status
update. · (13) Layer 9's research (incidents at `state/incidents.jsonl`, a per-record node-key signature, two tag spellings) gives way to §3. · (14)
`/api/status`: Layer 1 says it needs no session (`L1:162`) while the code gates it (`N/dashboard/routes.rs:16-17`, `:473-479`). The brief also cited
`routes.rs:28` as calling it exempt; that line exempts it from the `localhost` redirect, not from the session (C13). A Layer 1/4 text fix; no dataset route
copies that posture (§16.7).

**Where the code overrules the brief or a report.** **C1** `state/relay.json` treats an unknown key as an invalid file: it is moved aside and every Layer 7
setting and consent falls back to off (`N/relay/settings.rs:21-26`). A Layer 8 key there would be read as invalid by an older release after a rollback. Layer
8's settings live in `state/storage.json`, which keeps and ignores an unknown key (§15.1). · **C2** Free space changes with any write, so "write the disk
figures only on change" would still write about every pass: the figures are not persisted at all (`M/store/prune.rs:928-947`, `:896-910`;
`N/fabric/mod.rs:2004`). · **C3** The brief's lines moved since 2026-11-02 and are re-pinned here: `DEFAULT_HIGH_WATERMARK` `prune.rs:96` (was `:79`),
`save_capacity` `:928-947` (was `:869-888`), `PRUNE_EVERY` `N/fabric/mod.rs:164` (was `:115`), the capacity pass `:1853` (was `:1131`),
`apply_key_and_pragmas` `M/store/mod.rs:402-460` (was `:396-456`), `A/src/events.rs:19-21` (was `:3`, `:17-21`), `L2:91`, `:216-218`, `:234`, `:311` (were
`:85`, `:206-208`, `:224`, `:296`). · **C4** Layer 7's tags are already registered as A14 (`L2:449-456`, recorded 2026-11-11), so J5's "one amendment" carries
the tags of Layers 8 and 9 (§21 L2). · **C5** `Proposal.kind` holds nine kinds (`N/dashboard/api_v1/proposals.rs:104-114`, `relay` built) while the agent's
tool count is still 17 (`A/tests/daemon_tools.rs:318`): every bump is stated relative (§16.5, §16.6). · **C6** The fabric schema is v4 at `f0b6806`
(`M/store/schema.rs:66`), v5 in flight; no signed release has shipped (`NI/manifest.rs:140-143`), so the migration copy's first field use is the first
migration after one (§13.6 item 4). · **C7** The probe has no storage-medium fact (`NI/install/probe.rs:70-75`, `:372`): §4.2's `keep` rule (D5). · **C8**
`proposal.write` takes only a title and a body (`A/src/tools.rs:100-108`): a typed proposal needs its own tool (D1). · **C9** The components cache is
root-owned (`L1:142`): component blobs are served in place, never copied into `state/origin/blob/` (§11.1; D18). · **C10** The chat relay drops the agent's
stream when the browser leaves (`N/dashboard/api_v1/relay.rs:585-612`): the relay task owns it (§2.3 rule 4). · **C11** The origin, the consents and the
metered window are stubs at `f0b6806` (`N/origin/mod.rs:1-11`, `N/relay/consent.rs`, `N/relay/metered.rs:1-9`): Layer 8's origin work waits for Layer 7 rows
D7, D10 and D11 (§14). · **C12** mesh111's `rusqlite` has no `backup` feature (`5-messaging/crates/mesh111/Cargo.toml:17`): the export runs with the daemon
stopped (D6). · **C13** `N/dashboard/routes.rs:28` concerns the `localhost` redirect, not the session gate (brief §4 item 14). · **C14** The Transit default
is a quarter of the high watermark (`M/store/prune.rs:52-63`, `:585-595`): it scales once the budget sets the watermark (§4.2 item 4). · **C15** The agent
counts conversation files for its status (`A/src/store.rs:88-95`): `111d` fills the count from the store (§2.3 rule 6). · **C16** kiwix-serve wakes once a
second whatever it does (R07g3 §1): an idle stop would need a timer of ours, so v1 has none (§9.3). · **C17** D37 (decided on 2026-11-11, after the brief)
fixed the cross-layer seam ids and the runner's map (`verify/seams.toml`, `verify/checks.toml`): §18 uses its Layer 8 ids and adds one row,
`L6-L8:server-component`.

**Deviations from D34 and the brief (each needs the owner's acknowledgement).** **D1** three agent tools (`storage.propose` beside the brief's two read tools;
C8) · **D2** the node-state store opens on use and closes on an idle pass (F1's cost line named a resident second connection) · **D3** Layer 8's switches live
in `state/storage.json`, not Layer 7's file (C1) · **D4** the disk figures are not persisted at all (C2) · **D5** the min(2 GiB, 15 %) rule applies to every
non-hub Linux node (C7) · **D6** export and import with the daemon stopped in v1 (C12) · **D7** tiles are served as `application/octet-stream` (no PMTiles
media type could be verified from this box), where the brief named "a `.pmtiles` content type" · **D8** a member checks its hub's catalogue on attribution
only while its last answer was a catalogue (F11 A: "on the same link-attribution event"), so a hub that serves none costs nothing after one 404 · **D9**
`ob_log = 12` below 1 GiB (the brief fixes 14 at 1 GiB and above) · **D10** two or more ranges answer 416 · **D11** the export's ciphertext uses Layer 5's
chunked object framing and its signature a BLAKE3 prehash (L2 §8.5(i) reads as one AEAD over the whole state) · **D12** with the node-state store closed, chat
is answered and not saved, and history answers `503 store_unavailable` (a new code) · **D13** the J2 file names and the J7 ring path are pinned here
(`state/heal/…`); the joint contract named none · **D14** a catalogue is published from a dashboard route, where `111d` holds the identity handle · **D15**
the log ring copies the tail and truncates, because the child holds an appending descriptor · **D16** J5's "one amendment" is the §13.3 block for Layers 8 and
9 (C4) · **D17** the watermark never drops below 512 MiB until a human sets `keep` (the inert default) · **D18** component payloads are served in place from
`cache/packages/`, where F4 says every public file lives in the tree (C9).

**Numbers.** Kept as the lead's defaults (Q24; each measured on a Pi before the lock): `journal_size_limit` 4 MiB · four bulk streams · the `keep` percentages
· the 90 % low mark · `ob_log = 14` at 1 GiB and above. The drafter's own, for the panel: the shares (fabric 3/4, the node-state store 1/4, `heal` at most a
quarter of it) · `logs.max_file_bytes` 4 MiB and a 3× reserve · `ob_log = 12` below 1 GiB · kiwix's port 11250 · 1,000 entries and 1 MiB per catalogue ·
`kb.search` at most 20 results and 8 KiB (R05's 8 KB) · `origin.max_streams` 1..=16.

## Changelog v0.1

- 2026-11-11: first draft from D34, D36, the options brief, research R00-R07g3 and the code at `f0b6806`, for the adversarial panel.
