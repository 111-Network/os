# Layer 8 - Storage & Shared State: spec v1.0 (locked 2026-11-13, D38)

The layer that keeps each node's own state encrypted and bounded by the disk the node has, and lets a hub hold the large public files a community shares.
**The node-state store** is a second SQLCipher file that only `111d` opens: the agent's conversations, proposals, tasks and decisions, and Layer 9's repair
records, live there; the agent holds no key and writes nothing over HTTP. **One node budget**, sized from the disk and shown on a Storage page, decides when
copies that can be fetched again give way; nothing is deleted because of its age, and a dataset is never deleted behind the user's back. **A hub** serves
encyclopedias (an adopted kiwix-serve, started on demand), regional maps (PMTiles files behind a `Range` route) and the pinned payloads of the components it
holds, all as files named by their sha256 in the one object tree Layer 7 built for releases, listed in a catalogue the publisher's node key signs, and checked
block by block. v1 adds **no shared log**, and no resident process, listener, timer or crate to the required configuration.

Status: 🔒 **locked v1.0 (2026-11-13, D38)** = draft v0.3 as re-checked. v0.1 (written from D34: the owner took every pick, detail pick, plan
correction and default of the options brief, and from D36, the brief (`plan/layers/8-storage/options.md`), its judge's check (`review/options-check.md`:
ready), research R00-R07g3 and the as-built code) → a three-judge panel (`plan/layers/8-storage/review/{store,datasets,integration}.md`: 92 findings) → the
lead's triage (`review/triage.md`) → v0.2 → one re-check (`review/recheck.md`: amend-then-lock) → v0.3 → a second re-check (`review/recheck-2.md`: lock,
eight minors folded into this lock edit) → lock; the map from each finding to the text is `review/fix-map.md`.
Nothing here is built; the build board is `8-storage/TASKS.md`.

## 0. Reading guide

Reads with: `os/README.md` §8, the layer map row 8, Categories 2A, 2B, 3 and 4A · `os/5-messaging/README.md` §10, §11, §13, §19, §21 (amended six times, §21)
· `os/2-identity/README.md` §6.1, §6.4, §8.5, §13.1 A4, §13.2 A14 · `os/1-bootstrap/README.md` "On-disk layout", "Layer 5 amendments", "Layer 6: optional
components" · `os/6-transport/README.md` §2.1 · `os/7-relay/README.md` §9, §10.5, §12, §17, §18 · `os/3-intelligence/README.md` "State ← Layer 8" ·
`os/4-interface/api/README.md` · `os/plan/DECISIONS.md` **D34** (the law), **D36**, D37 (the seam map), D30-D32 and D29 (the lock precedents) · the options
brief (OB) and research R00 (seams), R01-R05 (lenses), R06 (critic), R07g1-R07g3 (gap-fills) under `os/plan/layers/8-storage/` · the Layer 9 brief F3
(`plan/layers/9-evolution/options.md:159-198`), whose joint contract is stated once, in §3 here.

**Citations.** `M/` = `5-messaging/crates/mesh111/src` · `N/` = `1-bootstrap/crates/node111/src/daemon` · `NI/` = `1-bootstrap/crates/node111/src` · `A/` =
`3-intelligence/crates/agent111` · `UI/` = `4-interface/ui` · `API` = `4-interface/api/openapi.yaml` · `BT/` = the bao-tree 0.16.1 registry source · `L0`…`L7`
= the layer READMEs. Every `file:line` is pinned at head `8936a8e` (2026-11-13): read each at that commit (`git show 8936a8e:<path>`), since later commits move lines. Every fact this spec states about the code is a fact at that head
("the pinned head" below). Layer 7 rows M1b (`4b9e521`, schema v5 and `custody_totals`), M1c (`4209e32`, a capacity pass at rest writes nothing while its
outcome is unchanged) and A1 (`443cf66`, the agent's three Layer 7 tools) are committed: the fabric schema is **v5** (`M/store/schema.rs:76`) and the agent
has 20 tools. Files that carry uncommitted edits of other rows in the working tree are cited as committed (`git show HEAD:…`). The pin is not moved at the lock. **Evidence:** *(src)* source read · *(doc)* docs read by a scout · *(design)* not
built · *(unverified from this box)* a claim no scout could fetch (R06 §7, OB §6).

**Normative:** numbered rules, byte and path tables, settings with defaults, gate assertions. **Informative:** "why" sentences, arithmetic, cost tables,
§19-§22. **v1 and later** (D34 F13):

| Area | v1 (this release) | Later (recorded, §20) |
|---|---|---|
| Node-state store (§2) | `node/state.db`, 111d the only writer; chat through the relay; the proposal and task spool; the legacy import | Mode T hardware key; a sealed spool |
| Joint contract (§3) | the seven items, the state service API, the paths | the fix-attestation feed (only through §12's gate) |
| Budget (§4) | one `keep` figure, four budgeted classes, the Storage page, the floor; the fabric does not borrow room the node-state store leaves unused (§4.2 item 3) | datasets pruned under 1 % free as an opt-in; the fabric borrowing that room |
| Object tree (§5) | datasets and sidecars in `state/origin/blob/`; the bao-tree outboard | Tier 2 adopts the same roots |
| Signing (§6) | the publisher's node key under `111/catalogue/v1`; home hub plus followed publishers | endorsements (`111/catalogue-vouch/v1`) in the UI |
| Moving datasets (§7) | the Range arm, `get_to_file`, sneakernet, the first copy, consent, the metered window | hub-to-hub bulk over TLS-RPK |
| Tier 2 (§8) | owner named (Layer 8); nothing built | `OBJ_*`, `bao-tree` 0.17, bulk on its own connection |
| Encyclopedias (§9) | the HTTP-only `child` panel source, the kiwix component, `encyclopedia.search` | WebSocket/SSE children (P7) |
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
| §8 "Knowledge hubs host the large shared datasets: offline encyclopedias, regional maps, software updates, and network troubleshooting patches" | encyclopedias (§9), regional maps (§10), the 111 release and the components' pinned payloads (§11); no dataset ships and no default catalogue entry exists (Q9); patches travel as Layer 9's sealed `PATCH` messages, not from a hub shelf. The release manifest's Termux deb and APK pins are served (§11.1) only once `LICENSING.md` carries each one's verified licence; both rows are unverified today (`LICENSING.md:64-65`), so v1 ships that pin source off | v1.1: the hub fix shelf (catalogue class `patch`), the Termux apt directory and APK index, hub-to-hub bulk (Tier 2); the Termux pins: a verified licence read per pin |
| §5 "The same fabric carries every kind of traffic the layers need: … knowledge-base synchronization, software updates" | datasets and component payloads move over HTTP on the LAN and to reachable hubs (§7), never on the fabric; the KB rides the release payload (L7 §9.4) | v1.1: Tier 2 on the fabric (§8) |
| Category 4A "Nodes equipped with expanded storage that host local offline databases" | any node with `hub_role` may host; the 4A probe (≥ 512 GB free, ≥ 8 GB RAM, `NI/install/categorize.rs:102-105`) names the class, it gates nothing | not a limit; stated so a Pi hub on an SD card is not called 4A (OB fact 24) |
| §8 "Knowledge hubs host the large shared datasets…" and Category 4A "Nodes equipped with expanded storage that host local offline databases" | the encyclopedia and the map on a hub are read by devices paired with that hub: a LAN session in v1 is the paired, full-control one (`lan_read_only` is "unused in v1 but reserved", `4-interface/README.md:591`); any other member downloads the raw files (`origin.datasets`, §9.5) to its own node | v1.1: a read-only LAN session |
| §8 "offline encyclopedias" on a Pi hub | the kiwix component's `aarch64-linux` pin rests on a static kiwix-tools 3.8.2 build no scout could fetch *(unverified from this box)*; the lock run sets `REQUIRE_G16=1` with that pin (§9.2, G16). If no such build exists, this row reads "a Pi hub has no encyclopedia in v1" | a static `aarch64-linux` build that passes G16 |

## 1. Scope & non-goals

**What Layer 8 is per device category** (README layer map, row 8: — · ◐ · ✓ · ◐ · ✓ · ✓):

| Category | Layer 8 there |
|---|---|
| Cat 1 | nothing: no 111 code runs on the stock repeater. |
| Cat 2A, 3 (◐) | the node-state store and the budget run natively; datasets are read from a hub over the LAN (its dashboard, its kiwix panel), kept locally only when the user fetches a small one, never served (serving needs `hub_role`). |
| Cat 2B (✓) | as 2A, plus local datasets the user fetches (a phone holds small ZIMs and a city map; app-private on Termux). |
| Cat 4A, 4B (✓) and any `hub_role` node | the above plus publishing a catalogue, serving datasets and component payloads (each behind its own switch, off by default), the kiwix component, the first copy from upstream. |

v1: the encyclopedia and the map on a hub are read by devices paired with that hub (the hub's session; a LAN session in v1 is the paired one); any other
member downloads the raw files (`origin.datasets`) to its own node. A read-only LAN session closes this in v1.1 (§0).

**The Raspberry Pi floor (hard rule, AGENTS.md).**
1. A node with nothing configured is byte-identical on the wire to a Layer 7 node. It keeps one descriptor of Layer 8 open for the daemon's lifetime, the
   exclusive `flock` on `<root>/node/lock` (§13.2), and no other file of Layer 8 while idle: the node-state store opens on use and closes on the first
   capacity pass with no use (§2.2 rule 7).
2. Nothing in Layer 8 adds a resident process, a listener, a timer, a periodic poll or a runtime dependency to the required configuration. The work of the
   budget rides the existing 5 s housekeeping interval and its 60 s capacity pass (`N/fabric/mod.rs:182-183`, `:2372-2394`), whose owner moves from the
   fabric's handle to a daemon-level task that runs with or without a live fabric (§4.3): the same one interval, no timer added. The new direct crate edges
   (`rusqlite`, `bao-tree`, `blake3` from node111) are already in the lock at the same pins: build-time weight only, +0 crates (§21 L1).
3. Layer 8 removes what is left of one recurring cost. Layer 7 row M1c (`4209e32`) already stops the idle fabric store's commit while the pass's persisted
   outcome is unchanged (`rest_pass`, `M/store/prune.rs:1216-1224`); the two disk figures in that outcome still change with any write on the disk, so the
   pass still commits whenever anything else wrote. Layer 8 stops persisting them (§13.4). The saving is measured at build (§19), not derived.
4. Hub-only and opt-in costs are enumerated, each behind its own switch and measured at build into `8-storage/TASKS.md` (§19): the kiwix panel's listener
   while the kiwix component is enabled; the kiwix-serve process, one loopback listener and its own 1 Hz wake from the first panel open until Stop, the
   idle stop (§9.3) or daemon stop; one fetch thread and a bounded buffer while a dataset job runs; the served files on disk; a 16 MiB buffer per active
   block check.

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
| `fabric/store.db` (+ `-wal`, `-shm`) | SQLCipher 4, raw key from `111/store/v1` (`M/keys.rs:24`, `:101-115`) | Layer 5, unchanged; with `fabric/blobs/` and `fabric/display-times*` it is the fabric store wherever Layer 8 exports, imports or sets one aside (§13.1, §13.6) |
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
1. **Path.** `<root>/node/state.db`, directory `node/` 0700 and files 0600, owner `node111` (the fabric store's modes, `M/store/mod.rs:259-273`). A separate
   file, never V5+ tables in `store.db`: a failed fabric migration or a rollback must not take agent and repair state down with messaging
   (`M/store/schema.rs:724-731` refuses a newer `user_version`; R04 finding 12).
2. **Key.** `node_state_key = BLAKE2b-256(key = store_secret, data = "111/node-state/v1" ‖ NodeId)`, keyed mode, where `store_secret` is the existing Mode S
   secret (`N/fabric/sealer.rs:44-47`, `:78`, `:134-145`). It goes through the `KeyProvider` seam, which gains `key_for(tag)` (§2.7 S1; the trait has one
   method today, `store_key()`, `M/keys.rs:88-93`); Mode T answers `Unsupported` (`M/keys.rs:12-14`) and the store stays closed. The daemon-level owner of
   the housekeeping interval (§4.3) derives it through the identity handle (`store_secret`, `N/fabric/sealer.rs:508`), outside `fabric::boot`: today the Mode S
   secret is taken only inside the fabric's `boot_with` (`N/fabric/mod.rs:726-745`), so a fabric that is down would leave the store with no key owner. No new
   secret, no key on disk; a copied file reopens after a restore from words.
3. **Engine.** The fabric store's open, exposed as a code seam (§2.7 S2): raw key, `cipher_compatibility = 4`, a plaintext SQLite refused, a wrong key
   detected on page 1, WAL, `temp_store = MEMORY`, `secure_delete = ON`, `auto_vacuum = INCREMENTAL`, `synchronous = NORMAL` (`M/store/mod.rs:433-493`), plus
   `journal_size_limit` (§13.5). `synchronous = FULL` for decisions (they gate an applied action, J9), every spool ingest and the legacy import (they precede
   an unlink, §2.4, §2.5), and migrations.
4. **Schema v1** (its own `user_version`; migrations append-only through the fabric store's `migrate_with`, `M/store/schema.rs:724-758`):

| Table | Columns | Notes |
|---|---|---|
| `meta` | `key TEXT PRIMARY KEY, value` | `next_seq`, the export offers per class (§4.5), the prune markers; no clean record (rule 9) |
| `records` | `ns TEXT, id TEXT, seq INTEGER UNIQUE, boot_id BLOB, mono_ms INTEGER, wall TEXT NULL, state TEXT, body BLOB`, `PRIMARY KEY (ns, id)`; an index on (`ns`, `state`, `seq`) | documents: a conversation's head, a proposal, a task, a decision, an incident; the index serves the reclaim and the J9 `list` cursor |
| `entries` | `ns TEXT, stream TEXT, seq INTEGER UNIQUE, body BLOB`, `PRIMARY KEY (stream, seq)` | append streams: a conversation's messages, Layer 9's audit trail; `ns` is the stream's namespace, so a trigger keys `ns_totals` without parsing `stream` |
| `ns_totals` | `ns TEXT PRIMARY KEY, rows INTEGER, bytes INTEGER` | kept by `AFTER INSERT/DELETE/UPDATE` triggers on `records` and `entries` inside the writing transaction (the M1b pattern); `bytes` is the sum of `length(body)` plus a fixed 64 B per row, the fabric's `ROW_OVERHEAD` (`M/store/prune.rs:135`) |

   Tables are STRICT. `seq` comes from one allocator (`meta.next_seq`) in the writing transaction: the store's monotonic sequence, the only order. Every write
   of a record takes a new `seq` (a `put` or `set_state` on an existing record included), so "oldest" means least recently written. `boot_id`
   (`/proc/sys/kernel/random/boot_id`) and `mono_ms` (`CLOCK_BOOTTIME`) sit beside it; `wall` is a display claim and orders nothing.
5. **Namespaces (v1).** `agent/conversations`, `agent/proposals`, `agent/tasks`, `agent/decisions` (Layer 3's data), `heal/` (Layer 9's, §3), `storage/`
   (Layer 8's own markers). A conversation is one `records` row (its head; `seq` bumped on every turn) and one stream `agent/conversations/<id>` of `entries`.
6. **One writer.** `111d` is the only process that opens the file while it runs; `111 store export | import` open it only with the daemon stopped, holding
   `node/lock` (§13.2). The agent reads through `GET` tools only; no Layer 8 route accepts the bearer on a non-GET.
7. **Open on use.** `111d` opens the store on its first use (a chat turn, an ingest, a history read, a reclaim, a Layer 9 write, a status read before the
   totals are known, and the first start of a build that writes a newer node-state version than the file holds: that start opens the store before the
   daemon reports health, takes the migration copy and migrates, §13.6 (1); the next idle pass closes it) and closes it on the first capacity pass that finds no use since the previous pass: no timer of its own, and no open file on an idle
   node. A turn in flight or an operation still running is a use, so a slow turn spanning two passes keeps the store open. `111d` reads `ns_totals` at the
   first open after a daemon start and then keeps them in memory from its own writes, so the pass reads the store only to reclaim. A close runs
   `wal_checkpoint(TRUNCATE)`, which writes nothing when only reads ran (its cost is in §2.8).
8. **When it does not open** (no key, a newer `user_version`, a damaged file, §13.7): the store stays closed, the `node_state` field of `GET /api/v1/storage`
   names the reason, chat still relays (the turn is answered and not saved; the chat view says "History is not being saved: <reason>"), the history route
   answers `503 store_unavailable`, the decision route answers `503 store_unavailable` and applies nothing (J9), and proposals stay in the spool.
9. **No clean record.** Unlike the fabric store (`M/store/mod.rs:306-329`, `:376-389`), the node-state store keeps no clean-shutdown record: a close
   checkpoints with `TRUNCATE`, so a clean close leaves `state.db-wal` absent or empty, and a non-empty `state.db-wal` found by a `stat` before the open is
   the unclean signal (§13.7). An open that only reads commits nothing; `GET /api/v1/storage` and a history read write nothing.

**2.3 Chat through 111d (M1, normative).**
1. `111d`'s chat relay (`POST /api/v1/chat`, `N/dashboard/api_v1/relay.rs:614`) reads the conversation's stream by `seq` descending and adds `history`: the
   newest whole messages up to 512 KiB, cut at a message boundary, as OpenAI message objects in order (`[]` for a new conversation), to the body it forwards
   to the agent (the lead's number, a default measured at build). A turn costs the same whatever the conversation's length. Additive to the agent contract
   (`3-intelligence/TASKS.md:27`).
2. The agent uses `history` instead of reading its file (`A/src/chat.rs:198-202`) and does not append (`A/src/chat.rs:279`). It sets its chat body limit
   explicitly at 2 MiB (axum's `DefaultBodyLimit`; today the `Json` extractor's framework default applies, `A/src/chat.rs:100`). Before `done` it sends the
   turn's new messages in order (the user message, tool calls, tool results, the reply) as one `message` frame each, every frame at most 512 KiB: a larger
   tool result (one may be 1 MiB today, `A/src/tools.rs:554`) is cut with a marker, so no frame meets the relay's 1 MiB event cap (`MAX_SSE`,
   `N/dashboard/api_v1/relay.rs:125`, `:430`). `done` carries the `conversation_id`. The non-streamed answer (`stream: false`, `relay.rs:621-631`) carries the
   same messages in a `messages` array, cut the same way. A body without `history` (an older `111d`) keeps today's behaviour.
3. On `done`, `111d` appends the messages its `message` frames carried and bumps the head's `seq` in one `NORMAL` transaction. It forwards no `message` frame
   to the browser and strips `messages` from a non-streamed answer: the browser's frames and `done` are unchanged (`API:121`).
4. **Persist with no browser.** Today the relay stops reading the agent when the browser's body stream is dropped (`relay.rs:585-612`: the upstream lives
   inside the browser's stream). The relay task owns the upstream and drains it to `done` or its existing deadline whether or not a browser reads; the
   browser's stream is a channel fed by that task. An acceptance test (G13).
5. `GET /api/v1/chat/history` (`relay.rs:679`) is answered by `111d` from the store, shape unchanged (`ChatHistory`, `API:803`; unknown id ⇒ 200 with empty
   `messages`). It works while the agent is down; today it answers 503 (`relay.rs:10`, `:26`; `API:131`).
6. The relayed `GET /api/v1/agent/status` replaces the agent's `conversations` count (it counts files, `A/src/store.rs:89`) with the store's.

**2.4 Proposals, tasks and decisions (M2, normative).**
1. The agent writes `proposals/<id>.json` and `tasks/<id>.json` atomically: a temp file named `.tmp-<anything>` in the same directory, `fsync`, `rename`.
   Today both use `std::fs::write` (`A/src/store.rs:139`, `:145`) while the module promises atomic writes (`:1-3`). At its start the agent removes every
   `.tmp-*` in its two spool directories (a temp file a crash left).
2. **One hand-off rule.** `111d` ingests a proposal on `proposal.written` and a task on every terminal task event (`task.done`, `task.failed`,
   `task.orphaned`, `task.refused`; `A/src/tasks.rs:146`, `:262`, `:296-300`; `relay.rs:826-827`), never gated on the relay's wall-clock freshness test
   (`is_fresh`, `relay.rs:789`, `:827`). It also sweeps the spool (two `readdir`, no timer) at daemon start, at every (re)subscription to the agent's events
   (the relay replays only the last 50, `relay.rs:127`) and before `GET /api/v1/proposals`, `GET /api/v1/proposals/{id}` and the decision route answer, so
   a file whose event was missed, or one a harness seeded with no agent running (`6-transport/verify/bridge.sh:696-700`), is still ingested and listed. A sweep takes only `<valid id>.json`, never a `.tmp-` name. Each ingest parses with the existing untrusted-file reader (1 MiB cap,
   `N/dashboard/api_v1/proposals.rs:124`), inserts by id (`INSERT OR IGNORE`), commits `FULL`, then unlinks. A crash between commit and unlink repeats
   harmlessly.
3. A task whose state is `running` stays in `tasks/`: the agent marks orphans at start (`A/src/tasks.rs:251`). Its terminal event, or the next sweep that
   finds it finished, ingests and unlinks it.
4. A file that does not parse is never ingested and never deleted by `111d`; the Storage page counts it ("unreadable agent files: N").
5. Decisions live in `agent/decisions`, not `agent/decisions/<id>.json` (`proposals.rs:13`, `:139-141`, `:455-464`). They keep today's protocol, "the
   record on disk is the guard" (`proposals.rs:26-33`): a pending record before the action, the result after, a refused apply removes it (`reopen`,
   `proposals.rs:754`). Both writers, the proposals route and Layer 6's applier `DecisionFile` (`N/devices/applier.rs:581-660`), go through J9's
   `decision_begin` (`FULL`), `decision_complete` (`FULL`) and `decision_reopen` (§3). A `pending` decision is never reclaimed. With the store unavailable
   the decision route answers `503 store_unavailable` and applies nothing. `GET /api/v1/proposals` and the decision route read the store. A `kind: message`
   proposal's outbound text (`N/fabric/proposal.rs:1-19`) is encrypted from its ingest on.
6. **The plaintext window, stated.** A proposal or task is plaintext on flash from the agent's rename until `111d` ingests it: sub-second while the relay is
   subscribed; otherwise until the next sweep (a resubscription, a proposals list, a daemon start). A deleted file may leave remnants on flash (no RAM-backed
   writable path). This is the reading of Q3's default: "encrypted" covers the durable copy.

**2.5 The legacy import (once, normative).** At the first start of a release that carries the node-state store, `111d` ingests every
`agent/conversations/*.jsonl` (one conversation per file, its lines in file order), `agent/proposals/*.json`, `agent/decisions/*.json` and every finished
`agent/tasks/*.json`, one transaction per file, each committed `FULL`, then moves the file to `agent/legacy-v0/` (same name and subdirectory). A proposal,
task or decision is idempotent by id. A conversation is merged by line count: the conversation's head records the number of lines imported from its file
(`legacy_lines`), and when its id is already in the store a later import appends only the file's lines beyond that count and raises it, so turns an older
release appended after a rollback are kept even when this release added turns to the stream before the revert. Conversations take their `seq` in the order
the import reads them; their ids order nothing (a client may choose one, `A/src/chat.rs:105-112`). `agent/legacy-v0/` is deleted when the probation journal
shows the release confirmed (the `.pre-v<N>` rule, §13.6 (2)); the update guard's restore moves the files back under its own trigger, on both revert paths
(§13.6 (3)), so a release N−1 without the node-state store keeps its conversations, proposals and decisions (J8: "kept"). A crash mid-file repeats that file. Afterwards `agent/conversations/` holds nothing and the agent never
writes there again. **The J7 ring.** When Layer 9 built before the store (J7), the same start ingests `state/heal/journal.jsonl` into `heal/`: its lines in
file order, each record idempotent by its namespace and id, one `FULL` transaction, then the file moves to `state/heal/legacy-v0.jsonl` under the same rule
(deleted once the release is confirmed, moved back by the guard's restore), so a release N−1 that writes the ring keeps it.

**2.6 What stays plaintext, stated (Q6, Q7).** `events.jsonl` stays a plaintext 1 MiB FIFO trimmed to half (`A/src/events.rs:19-21`; 111d resyncs a restarted
journal, `relay.rs:750-773`). `agent/work/<task>/` holds a task's gathered context; it is deleted with its task's record (§4.4). The child logs get a size
ring (§13.8); whether `llama-server` logs prompts is *(unverified from this box)*. The Delta Chat core's database stays plaintext (fscrypt is named for Linux
operators); Android's file-based encryption covers Termux's app-private data *(unverified from this box)*.

**2.7 Code seams in Layer 5 (no Layer 5 spec text, no wire byte).**
- **S1** `M/keys.rs` gains `derive_tagged_key(tag, secret, node_id)`; `derive_store_key` (`:101-115`) becomes that call with `STORE_KDF_TAG`, byte-identical
  (its vector `5-messaging/test-vectors/store-kdf.json` unchanged). `KeyProvider` (`:88-93`) gains `key_for(tag)`: Mode S answers `derive_tagged_key`, Mode T
  `Unsupported`, and `store_key()` equals `key_for(STORE_KDF_TAG)`. A known-answer vector for `111/node-state/v1` joins the file.
- **S2** `M/store/mod.rs` exposes `open_encrypted(path, &StoreKey) -> Connection`: the directory and file modes of `:259-273` and `apply_key_and_pragmas`
  (`:433-493`, private today), and `migrate_with` (`M/store/schema.rs:724`) for a caller's own migration list. It takes no epoch decision and leaves the
  clean record alone (the export verb, §13.2, and the migration copy, §13.6, use it). The fabric store's open is unchanged.
- **S3** `sweep_blob_orphans` (`M/store/blobs.rs:345-365`, called from every open, `M/store/mod.rs:301`) removes temp entries only, never a blob file, while a
  set-aside fabric store (`store.db.rolled-back-v<N>`, `.damaged-<n>`, `.replaced-<n>`) exists in `fabric/` (§13.6, §13.7, §13.2): its rows may still list
  those blobs. The sweep resumes when the user deletes the set-aside file (`DELETE /api/v1/storage/set-aside/{file}`, loopback only, §16.1).

**2.8 Costs.** No new crate, process, listener or timer; the agent's dependency list is unchanged and its state code shrinks. The WAL written per chat turn
is measured at build for the schema as written: a turn touches `records` and its three indexes, `entries` and its two, `ns_totals` and `meta`, where R07g1
§2's derived 12.4 KB (about 25 KB with the checkpoint) counted three pages. An open that only reads, a history read and a status read write nothing (§2.2
rule 9). The saving it is set against, the disk-figure commits §13.4 stops, is measured too. The store's page cache while open, the open and close cost on an
SD card and the binary delta are measured at build (§19).

## 3. The joint storage contract with Layer 9 (D34 F2; D35 F3)

Stated once, here. Layer 9's spec cites these items by number (J1-J9) and adds nothing to them. The seven decided items are J1-J7; J8 and J9 are the paths and
the API that make them buildable.

- **J1 Home.** Layer 9's durable records (incidents, apply attempts, the loop ledger, kept-change snapshots, received fixes, the audit trail) live in Layer
  8's node111 state service under the namespace `heal/`, inside `node/state.db` (§2.2). `111d` is their only writer; the agent reads them only through GET
  tools.
- **J2 Before the store opens.** Three small files stay plaintext (0600, atomic writes, the probation-journal pattern), because a shell hook writes them or
  they must be read before the store opens: the run marker `state/heal/run`, the witness inbox `state/heal/inbox` (one line per daemon exit: code, signal,
  boot id; no free text; the hook keeps the newest 256 lines, through a temp file and a rename, so a crash loop with the store unopenable cannot grow it;
  an exit with status 75, the daemon's answer to a store verb holding `node/lock` (§13.2), is recorded with its code and never counted as a crash) and
  the armed undo record `state/heal/undo` of an apply in progress. `111d` reads them at start before serving, folds them into `heal/` and clears them; if the
  store does not open, they stay and the Repairs page says so.
- **J3 Pruning.** By capacity only, as the `heal` class of the node budget (§4): oldest closed incident first by the store's `seq`; then a `heal` stream (the
  audit trail) from its oldest `seq`, after the closed incidents; never an open incident, an armed undo, an undecided proposal, a running task or a `pending`
  decision. A prune leaves one marker record in `heal/` (`pruned through seq N, K records`). No age, no expiry, no wall clock. With its export offer
  unanswered the class does not prune and is `full` (§4.5).
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
| `state/heal/{run,inbox,undo}` | L9 | the shell hook (`inbox`), `111d` | none (`inbox` at most 256 lines, J2; one record each for `run` and `undo`) | no | kept · kept · removed · lost |
| `state/heal/journal.jsonl` (J7 only) | L9 | `111d` | `heal` (1 MiB ring) | no | kept until imported (§2.5), then `state/heal/legacy-v0.jsonl` until the release is confirmed; moved back by the guard's restore |
| `agent/proposals/`, `agent/tasks/` spool | L3 | the agent | `agent`, measured on request only (§4.1) | no | kept · kept · removed · lost |
| `agent/legacy-v0/` (§2.5) | L3 data, L8 | `111d` | `agent`, measured on request only | no | kept until the release is confirmed, then deleted; moved back by the guard's restore · kept · removed · lost |
| `fabric/store.db` | L5 | `111d` | `fabric` | yes, L5 §10.4 step 3 (`set_export_offered`) | as `node/state.db` |

- **J9 The state service API (node111, in-process; Layer 9's modules call it, the agent never does).** `put(ns, id, state, body)` · `get(ns, id)` · `list(ns,
  state?, after_seq?, limit)` · `append(stream, body) -> seq` · `read_stream(stream, after_seq?, limit)` · `set_state(ns, id, state)` · `delete(ns, id)` ·
  `delete_stream(stream)` · `trim_stream(stream, through_seq)` · `usage() -> {ns: (rows, bytes)}` · `reclaim(class, bytes) -> freed` · `decision_begin(id,
  pending)` (`FULL`) · `decision_complete(id, applied)` (`FULL`) · `decision_reopen(id)`. Each call is one transaction; every write takes a new `seq` and
  returns it (§2.2 rule 4). `list` and `read_stream` return at most `limit` rows (1,000 at most; default, measured at build) after the `seq` cursor, so a
  call's work never grows with the rows held. `reclaim` calls the reclaimer the owning module registers for each namespace of the class (its order and the
  states it never touches); Layer 8 supplies the default (oldest `seq` first, protected states skipped, a stream deleted with its head) and writes the J3
  marker. The states a reclaim never touches are declared per namespace (`open`, `armed`, `undecided`, `running`, `pending`). A reclaim ends with `PRAGMA
  incremental_vacuum` (the store is `auto_vacuum = INCREMENTAL`, rule 3 of §2.2) and returns the bytes freed from the file. A write to a class that is `full`
  (§4.5) returns `Full`, which Layer 9's spec handles; the three decision calls and a spool ingest (§2.4 rule 2) are written whatever the class's state. The decision calls are the only
  writers of `agent/decisions`: `decision_begin` writes `applied: {pending: true}` and refuses an id already decided (`already_decided`),
  `decision_complete` replaces it with the result, `decision_reopen` removes a pending record whose apply was refused before anything ran.

## 4. One node budget; datasets never pruned automatically (D34 F3)

**4.1 The classes (normative; the closed set `StorageClass.id`).**

| `id` | Label | Layer | Counted from | Budgeted | Rule |
|---|---|---|---|---|---|
| `fabric` | Messages and mail | 5 | live bytes: DB pages minus freelist, plus present blobs (`M/store/prune.rs:745-757`) | yes | L5 §10.4 as built (`prune.rs:1-50`), under the watermark §4.2 sets |
| `agent` | Agent state | 3 | `ns_totals` of `agent/*` (the spool, `agent/work/` and `agent/legacy-v0/` are measured on request only, one walk each, and shown beside it) | yes | §4.4 |
| `heal` | Repair records | 9 | `ns_totals` of `heal/*` | yes | J3 |
| `logs` | Logs | 1 | one `stat` per capped file: every file `open_log` opens (`agent/111-agent.log`, `agent/llama-server.log`, `N/intelligence.rs:274-281`, and each component's and child's log, §13.8), their `.1`, `agent/events.jsonl` | yes | §13.8 |
| `datasets` | Datasets | 8 | the holdings index (§5.5): held sizes plus sidecars, no walk; free space from `statvfs` of `state/origin/` (§4.6) | no | only the user (§4.6) |
| `updates` | Updates | 1, 7 | `releases/`, `state/staging/`, the origin's release blobs | no | as built: two releases kept (`L1:136`) |
| `components` | Components cache | 6 | the registry's `cached_bytes`, `installed_bytes` (`N/components/registry.rs:103-107`) | no | the user's "remove files" (`L1:265-266`) |
| `bridge` | Bridge accounts | 6 | `bridge/<adapter>/` | no | shown only; its one control is Layer 6's "remove files", which deletes `bridge/<adapter>/` with the account store (`6-transport/README.md:153-154`): the brief's "reset account" (§22 C18) |
| `kb` | Knowledge base | 0 | `current/share/kb/` (`N/kb.rs:1-3`) | no | rides the release (L7 §9.4) |

The `updates`, `bridge` and `kb` figures come from one `tree_bytes` walk each (`N/components/registry.rs:112`), on request only; their time on an SD card is
measured at build. Nothing measures them on a timer (R00 §3: nothing measures `agent/`, `bridge/`, `rns/`, `releases/` or `cache/` today). A walk's result
is kept in memory until a release flip, a component change (enable, disable, remove files, a cache write) or 60 s, whichever comes first (default; measured at build), so a page
refresh or the agent's `storage.read` in a loop walks at most once a minute. Each walk stops at 100,000 entries (default; measured at build) and then reports its
figure as a lower bound ("at least").

**4.2 The `keep` figure, the shares and the floor (normative).**
1. **`keep_bytes`** is one figure per node: the budget of the four budgeted classes. Its default is computed once, at the first daemon start of a Layer 8
   release, from `statvfs` of the root, and stored as `keep_default` in `state/storage.json` (the one write that start makes): a Termux node: min(2 GiB, 10 %
   of free space) · `hub_role` on, or category 4A or 4B: min(25 % of the volume's size, the free space above the floor) · every other Linux node: min(2 GiB,
   15 % of free space). Derived, not measured (R04 fork 2): they stand as defaults, are measured on the owner's real-device run (D36 (3)) and are
   recorded in `8-storage/TASKS.md` (§22 D23). The probe has no storage-medium fact (`NI/install/probe.rs:70-75`, `:372`), so the
   brief's "a Pi on an SD card" figure applies to every non-hub Linux node (§22 D5).
2. **Who sets it** (Q2): the install-time figure; the user on the Storage page; the agent may propose it (`quota.set {class: "keep"}`); nothing else. Turning
   `hub_role` on later does not recompute it: the page shows the 25 % figure as a suggestion.
3. **Shares** (the drafter's defaults; the owner may change). The `logs` reserve is computed from the capped files that exist (§13.8): each cap × 1.5 (the
   file and its `.1` half), plus 1 MiB (`events.jsonl`). Of the rest, `fabric` takes 3/4 and the node-state store 1/4; inside it `heal` takes at most 1/4 and
   `agent` the remainder. `caps.agent` and `caps.heal` override a share (null = the share). **The shares order the reclaim, they do not trigger it:** `agent`
   and `heal` reclaim only when the budgeted classes together exceed `keep` or `disk_low` is set, and then the class furthest over its share goes first
   (§4.3 step 4), so a class never prunes while the budget has room. The fabric's share is its high watermark (`watermark.high`), which the fabric's own pass
   enforces as built; its low mark stays as built, `high − high/8` (`M/store/prune.rs:183-190`, `:903-911`). A v1 limit, stated: the fabric does not borrow
   room the node-state store leaves unused (§20). Until a human sets `keep_bytes`, the budget never sets the watermark below the built 512 MiB (`:131`): it
   only raises it (§15.1 "Inert by default"). When that 512 MiB minimum is above the fabric's share (a `keep` under about 695 MiB), the caps in force add up
   to more than `keep`: `keep.effective` (§16.2) is the sum of the caps in force, and the Storage page shows it whenever it differs from `keep`. Arithmetic at
   keep 2 GiB with two capped logs: logs 13 MiB, fabric ≈ 1.49 GiB, node-state ≈ 509 MiB, heal ≤ 127 MiB.
4. **The Transit quota keeps its figure.** Layer 7's Transit quota, when unset, is a quarter of the custody quota when one is set, else a quarter of the high
   watermark (`M/store/prune.rs:52-66`, `:925-944`): 128 MiB at today's fixed 512 MiB (`:131`). When the budget first calls `set_watermarks`, it writes
   `quota.transit` at its effective value of that moment if it is unset (one write, once), so L7 §5.8's figure stays until a human changes it. The Storage page
   offers the scaled figure (a quarter of the new watermark) as a suggestion.
5. **The floor** is the fabric's built 1 % (`DiskSpace::floor`, `M/store/prune.rs:164-172`). Below it `disk_low` is true: the fabric prunes as built (its
   floor trigger, `prune.rs:3-6`); `agent` and `heal` reclaim to their low marks once their export offer is answered (§4.5); dataset jobs pause (`paused
   (disk low)`); new draft and import jobs and component caching answer `409 insufficient_space`. Messages and decisions are never refused (`prune.rs:38-42`);
   a chat turn is always answered, and saved unless its class is `full` (§4.5).

**4.3 The pass (normative).** **The owner.** Today the 5 s housekeeping interval (`FabricConfig.housekeeping`, `N/fabric/mod.rs:229-230`, `:249`) is spawned
only while the fabric is live (`if let Some(l) = &handle.live`, `:911-928`), and `capacity_pass` returns at once without a fabric (`:2235-2236`). Layer 8
moves the interval's owner from `FabricHandle` to a daemon-level task that runs with or without a live fabric: the same one interval, so no timer is added.
The fabric's capacity pass (`:2235-2256`) and `refresh_facts` (`:1301`) become two of its callees, called while the fabric is live; the node-state key is
derived outside `fabric::boot` (§2.2 rule 2). Every `PRUNE_EVERY` = 12 ticks (`:182-183`, `:2372-2394`; the first runs on the first tick, `n = 0`, `:2386`)
the pass runs:
1. The fabric's `prune` runs as built, with the `statvfs` figure it takes (`:2237`). With the fabric down this step is skipped and the pass takes its own
   `statvfs` of the root.
2. The budget reuses that figure and reads the fabric's live bytes (none with the fabric down), the in-memory `ns_totals` and one `stat` per capped log file.
   No directory walk; no store open unless step 4 reclaims.
3. If `keep` or a share changed since the last pass, it calls `set_watermarks` (`M/store/prune.rs:913-921`; no caller today, R00 finding 6): one settings
   write per change, never per pass. The first call also writes `quota.transit` once if unset (§4.2 item 4).
4. If the budgeted classes together exceed `keep`, or `disk_low` is set, `agent` and `heal` reclaim (§4.4): the class furthest over its share first, each
   down to its low mark, 90 % of its share (the lead's number, Q24), until the total is back under `keep` or the floor is clear. A class whose export offer
   is unanswered does not reclaim and is `full` (§4.5). A reclaim ends with `PRAGMA incremental_vacuum` (J9).
5. A log over its cap rotates (§13.8).
6. A dataset job paused at the metered cap or at low disk is checked against its condition (one comparison; no I/O) and resumed when it clears. A job
   paused at a busy source sends one request to that source on a pass, after a gap counted in passes that doubles from 1 to 32 (1, 2, 4 … 32, then every
   32nd pass), and resumes when the source answers; the gap resets when the job resumes (§7.4).
7. The node-state store closes if no operation used it since the previous pass (§2.2 rule 7).
8. A change §16.3 lists publishes one `storage` frame (the one list of what sends it; the store's close in step 7 sends none). Nothing is published
   otherwise. The pass's added time and reads, with the node-state store open, are measured at build (OB §3).
9. A running kiwix-serve that no request reached through its panel's proxy and no search reached through `111d` (§9.4) for 30 passes is stopped (§9.3): one
   comparison of a counter the proxy and the search route reset.

Steps 2 and 4 to 9 run whether the fabric is live or not; step 3 waits for the fabric (it writes the fabric store's settings). With the fabric down (a newer
`user_version`, a damaged store, §13.7) the node-state store still closes, `agent` and `heal` still reclaim, logs still rotate, a paused job still resumes
and an idle kiwix-serve still stops.

**4.4 Rules per class (normative).**
- `fabric`: L5 §10.4 unchanged (`M/store/prune.rs:8-42`): re-fetchable and committed items first, own history only at hard capacity and after the export offer
  (§4.5).
- `agent`: after the class's export offer (§4.5), first finished tasks with no undecided proposal, each with its record and its `agent/work/<task>/` (an
  opencode working tree, `A/src/tasks.rs:397`), and its decided proposal and decision when it has one, oldest `seq` first; then decided proposals with no
  task, with their decision, oldest `seq` first; then whole conversations (the head and its stream) by their head's `seq`, oldest first. Never an undecided
  proposal, a running task, a `pending` decision, or the conversation of a turn in flight.
- `heal`: J3.
- `logs`: the ring (§13.8); `events.jsonl` keeps its own FIFO (`A/src/events.rs:19-21`).
- No rule reads a clock: "oldest" is `seq` distance, as the fabric's age is `local_seq` distance (`M/store/prune.rs:12-13`).

**4.5 Export before prune (normative; Q5).** One offer per class before its first prune, then capacity order. The fabric's offer is L5 §10.4 step 3's gate,
now called: `set_export_offered` (`M/store/prune.rs:984-986`; no caller today, so step 3 never runs in production, R00 finding 6). For `agent` and `heal` the
offer is a flag in `meta`. Before a class's first prune of its own records (fabric step 3; any `agent` or `heal` record), the Storage page says: "Your oldest
<messages | conversations | repair records> will be removed to make room. You can export them first with `111 store export` (Settings → Storage → Export shows
how), or let them go." The answer (`POST /api/v1/storage/export-offer`) records the offer; until then the class reports `export_offer_needed` and only that
step waits.

**`full` (`agent` and `heal`).** When the pass must reclaim a class (§4.3 step 4) and its offer is unanswered, the class stops growing and says so:
`StorageStatus.classes[].full` is true; a chat turn is answered and not saved, with the §2.2 rule 8 notice ("History is not being saved: the space set aside
for the agent is full. Answer the export offer on the Storage page."); a J9 write to the class returns `Full`, which Layer 9's spec handles; decisions are
still written. A spool ingest is written whatever the class's state, as a decision is, so a new proposal is never left plaintext and unlisted. The class leaves `full` once the offer is answered and the reclaim made room, or once the budget has room again.

**4.6 Datasets are never pruned automatically (normative; Q1).** Only the user, or a `dataset.remove` proposal the user accepts, removes a dataset. A fetch,
a draft, an import and a component cache check free space first, else `409 insufficient_space` before any byte moves (§7.7). On the root's filesystem the
check reserves the unused part of the budget: it passes only if `free − size − sidecar ≥ floor + max(0, keep − the bytes the budgeted classes hold)`, so a file
the node can fetch again never takes the room `keep` sets aside and never drives the fabric's floor trigger into own history. When `state/origin/` is its
own filesystem (§5.1), every dataset check and the `datasets` figure read `statvfs` of `state/origin/` and reserve that volume's 1 % floor only, while the
budget reads the root's; a component cache check always reads the root's (`cache/packages/`). The consent's "free after" (in its `facts`, §7.7) is the figure the check computed.
Datasets never count against `keep`. Pruning a dataset below 1 % free, as an opt-in, is later (OB F3 C).

**4.7 Page copy (verbatim; R05 finding 3).** "Nothing here is deleted because of its age. When the space set aside is used up, copies that can be fetched
again go first, oldest first; your own messages, contacts and anything waiting to be sent stay." **4.8 Costs.** Code only; no timer of its own. One settings
write per change of `keep`. Every Storage-page figure is computed on request; §13.4's idle write is never copied. The housekeeping interval changes owner,
not count (§4.3).

## 5. One object tree for datasets, checked block by block (D34 F4)

**5.1 The tree (normative).** (1) Every public file the daemon keeps (release payloads, datasets and their sidecars, the release's source tarball) lives in
Layer 7's sha256-addressed tree `state/origin/blob/<sha256>` (L7 §9.1, `7-relay/README.md:857-872`), whether or not the node serves it: one Range handler, one
admission rule. Component payloads stay in the root-owned cache and are served in place (§11.1, §22 C9). · (2) Datasets never enter `fabric/blobs/`: its
writer emits both outboards for every blob (`ChunkGroup::ALL`, `M/blobs.rs:97`, `:456-461`), 3.56 GB of `ob1k` plus 222.7 MB of `ob16k` for a 57 GB file
(R07g3 finding 8), and its bytes count toward the message watermark (`M/store/prune.rs:745-757`). · (3) A file is renamed into `blob/` only after its last
check passed, the sha256 of its name included (§7.3 (3)(e)); a half-written file never replaces a served one, and an existing `blob/<sha256>` is never
replaced by a pull, an import or a draft.

**Paths** (owner `node111`; directories 0755, files 0644 unless noted). Every path is on the root's filesystem, except that `state/origin/` may be its own: an
operator who keeps datasets on an attached disk mounts it at `<root>/state/origin/` (it lies under the unit's `StateDirectory`, so it stays writable under
`ProtectSystem=strict`, `1-bootstrap/systemd/111node.service:35-36`). `incoming/` and `blob/` are always on one filesystem, checked by `st_dev` at job start
(a rename never crosses filesystems). §13.9 advises that layout for a Pi hub.

| Path | What | Writer |
|---|---|---|
| `state/origin/blob/<sha256>` | a verified file: payload, dataset or sidecar | `111d`; the import verbs only with the daemon stopped |
| `state/origin/catalogue/<publisher hex>.toml`, `<publisher hex>.toml.sig` | the highest verified catalogue per publisher (§6.3) and its detached signature (one name on disk and on the route, §16.7) | `111d`; the import verb only with the daemon stopped |
| `state/origin/incoming/<sha256>.part` + `.blocks` | a pull or a verb's copy in progress and its block bitmap (a hint; the hashes are the proof); the `.blocks` header records the boot id (§7.3 (5)) | the job, the import verbs |
| `state/origin/incoming/<publisher hex>.toml` + `.toml.sig`, `state/origin/incoming/adopt-<n>.json` | a dataset verb's staged catalogue copies and its one request file, until the daemon adopts them (§7.6) | the dataset verbs; `111d` deletes them once adopted |
| `state/origin/import.lock` | the dataset verbs' `flock` (§7.6) | the verbs, `111d` while it adopts |
| `state/origin/drafts/<id>@<version>.toml` | an entry not yet published (§6.4) | `111d`; `111 dataset add` only with the daemon stopped |
| `state/datasets.json` (0600) | the holdings index (§5.5) | `111d`; the import verbs only with the daemon stopped |
| `state/storage.json` (0600) | Layer 8's settings and consent hashes (§15.1) | `111d`; the import verb's `--follow` only with the daemon stopped |
| `state/kiwix/library.xml`, `state/kiwix/zim/<publisher short>-<id>-<version>.zim` → `../../origin/blob/<sha256>`, `state/kiwix/kiwix.log` | the kiwix component's state and log directory (an adapterless component's is `state/<id>/`, §9.2): the library, one symlink per held ZIM, keyed by publisher as a holding is (§5.5), where `<publisher short>` is the first 16 hex characters of the publisher's NodeId (the same short form names a publisher in a consent's subject, §7.7, §15.2), the child's log (in the §13.8 ring); "remove files" removes it | `111d` (through `kiwix-manage`); the child writes its log |
| `node/state.db` (0600, dir 0700) | the node-state store (§2.2) | `111d` |
| `node/lock` (0600) | the daemon's lifetime `flock`; the store verbs take it (§13.2) | `111d`, `111 store export` and `import` |
| `agent/legacy-v0/` | the v0 agent files until the release is confirmed (§2.5) | `111d`, the update guard's restore |
| `state/heal/` (0700) | J2 and J7 (§3) | `111d`, Layer 9's hook |

**5.2 The sidecar (normative; G3).** (1) A dataset's sidecar is the bao-tree post-order outboard (`outboard_post_order`, `BT/io/sync.rs:706`) at
`BlockSize::from_chunk_log(ob_log)` (`BT/tree.rs:144`): `ob_log = 14` (16 MiB blocks) for files of 1 GiB and more, 12 (4 MiB) below (§22 D9); a Pi hub may
choose 12 for any file after measurement (Q18). It is stored as one more blob, named by its own sha256: an address, not a trust anchor, and proven against
its bytes before its rename into `blob/` exactly as a dataset file's name is (§5.1 (3), §7.3 (3)(a), §7.6). · (2) Its size is
`(blocks − 1) × 64` B (`BT/lib.rs:511`): 28,224 B for 7.4e9 B, 49,536 B for 13e9 B, 217,408 B for 57e9 B (3,398 blocks), 423,424 B for 111e9 B (R07g3 §3
finding 2), all at `ob_log = 14`. The exact length a node expects is `(ceil(size / 2^(10 + ob_log)) − 1) × 64`, computed from the signed `size` and
`ob_log`: any other length is refused at fetch and at publish, and an entry whose figure exceeds 8 MiB is refused (at `ob_log = 12` a 111e9 B file's
sidecar is 1,693,696 B). · (3) The 32 B BLAKE3 root in the signed entry authenticates the sidecar alone (`valid_outboard_ranges`, `BT/io/sync.rs:979`) and each fetched
block (`valid_ranges`, `:813`), both in 0.16.1, which v1's checks use (Q22; an inference from the call sites, not a test, R07g3 finding 10). The root is the
same at every block size, so Tier 2 adopts the file later with no re-signing (R07g3 finding 6). · (4) Each active check holds one block-sized buffer: 16 MiB
at 14, 4 MiB at 12 (R07g3 finding 9). · (5) Metalink and upstream `.sha256` files stay operator aids: `--sha256` checks an upstream hash when the first copy
comes from a mirror (§7.5, §7.6).

**5.3 The catalogue entry (normative).** TOML in the release manifest's style, `deny_unknown_fields` (the manifest precedent, `NI/manifest.rs:270`):

| Field | Type | Meaning |
|---|---|---|
| `id` | `^[a-z0-9][a-z0-9_.-]{0,63}$` | stable across versions; never a path |
| `kind` | `zim` · `pmtiles` | the reader (`patch` is reserved for v1.1); a reader skips an entry whose `kind` it does not know (§5.4) |
| `name` · `version` | string ≤ 120 · `^[A-Za-z0-9][A-Za-z0-9_.+-]{0,39}$` | shown; `version` is never ordered and is never a path, though it is a URL path segment (§16.1) and part of file names (§5.1); a catalogue holding an entry whose `version` does not match is refused whole (§5.4) |
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
| `book` | string ≤ 120, optional, `kind = "zim"` only | the ZIM's own `Name` metadata, copied by the publisher with `License` and `Source` (§7.5): the name kiwix-serve's search knows the book by (§9.4) |

**5.4 The catalogue file (normative).** `schema = 1`, `publisher = "<64 hex NodeId>"`, `seq` (u64, strictly increasing per publisher, minted from the
publisher's L2 announce counter at each publish, §6.4), then `[[entry]]` tables. At most 1,000 entries and 1 MiB, checked before parsing; every (`id`,
`version`) unique, and an `id` or a `version` that does not match its §5.3 pattern refuses the file; the detached signature `<publisher hex>.toml.sig` beside it (§6.1). **Unknown kinds.** An entry whose `kind` the reader does not know is
skipped whole (its other keys are not read) and counted: the Datasets view says "N entries need a newer 111" (`Publisher.skipped`, §16.2). Every other
unknown key, in the file or in an entry of a known `kind`, still refuses the file. `schema` changes only for a change a skip cannot carry, so a v1 node keeps
reading the catalogue of a hub that updated first and lists one `patch` entry.

**5.5 The holdings index (normative).** `state/datasets.json`: per held (publisher, `id`, `version`) its `sha256`, its location (`origin`, or `in_place` with
an absolute path), its state (`partial`, `present`, `unverified`, or `disk_absent`: the one enum of §16.2, where `not_held` is the state of a listed entry
the index does not hold), for a file served from outside the tree its tuple (device, inode, mtime ns, size), the hash of the consent that fetched it, and
its job. **Files outside the tree.** A file served from outside `state/origin/blob/` (an in-place dataset; a component blob in the root-owned cache, §11.1)
is hashed once and the result kept against its tuple; the Range arm's existing metadata read (`N/panels/static.rs:220-227`) compares the tuple on every
request, and `111d` makes one `stat` per in-place ZIM against its tuple before it starts kiwix-serve and before it rebuilds the library (§9.2), since
kiwix-serve reads an in-place ZIM itself through its symlink and no request of ours touches it while `origin.datasets` is off. A changed tuple, a missing
file or an unclean shutdown makes it `unverified` (or `disk_absent` when the path is gone): it answers 404 and leaves the kiwix library (before the start
or the rebuild) and the tiles list, and it is re-hashed as a queued job on the `111d-dataset` thread (§7.4); a match makes it `present` again. Nothing is
hashed per request. **The unclean signal** is the fabric store's own for this start: its clean record absent at open (`unclean`,
`M/store/mod.rs:156-158`). When the fabric store did not open at this start (a newer `user_version`, a damaged file, §13.7), every out-of-tree file is
`unverified` at that start. **Crash order.** The `partial` record is
written before the first byte of a pull; at daemon start a `partial` record whose `blob/<sha256>` exists becomes `present` (a crash fell between the rename
and the index write), so the index never misses a blob the tree holds. The index is written on a state change only; a job's progress lives in its
`.blocks` file, never here. Written atomically and only on change, with the `state/relay.json` file rules (`N/relay/settings.rs:16-36`: absent ⇒ defaults; not a small
regular file or not JSON ⇒ moved aside and defaults), except that an unknown key is kept and ignored, never "invalid", so a rollback to an older release never
drops a record (§22 C1).

**5.6 Costs.** No new crate: `bao-tree`, `blake3`, `sha2` and `toml` are linked (R00 §2; `bao-tree 0.16.1` and `blake3 1.8.7` in
`1-bootstrap/Cargo.lock:101-102`, `:179-180`); node111 gains direct edges at those pins (§21 L1). No listener, no timer. One import pass computes sha256,
BLAKE3 and the sidecar together; its throughput on Pi 4 and Pi 5 is *(unverified from this box)*, measured at build.

## 6. Who signs a dataset (D34 F5)

**6.1 The signature (normative).** Ed25519 by the publisher's node identity key over `"111/catalogue/v1" ‖ publisher NodeId (32 B) ‖ the catalogue file's
bytes`; a detached 64 B `<publisher hex>.toml.sig` beside the file. The tag leads the signed input (`L2:91`). The release key never signs datasets (OB F5 C).

**6.2 The trust rule (normative; G6).** A node verifies and lists the catalogues of (a) its pinned home hub (the pinned-home-hub pattern of L7 §8.4,
`7-relay/README.md:839`) and (b) the publishers in `follow` (§15.1, default `[]`; the rule L7 §11.1 uses for broadcasts, `:971`). Any other catalogue is
not listed; the API answers `409 untrusted_publisher`; the CLI import refuses without `--follow` (§7.6). A mirror hub serves another publisher's catalogue
and blobs with the original signature and never re-signs (R02 finding 16). **How a followed publisher's catalogue reaches a node in v1.** A check (§12.4:
Check now, opening the Datasets view, and for the home hub the link-attribution event) asks each source of the node's source list (§7.4: the home hub's
`origin_base`, then the URLs in `sources` for that publisher) for the home hub's and every followed publisher's `catalogue/<hex>.toml` and its
`.toml.sig`; a home hub that mirrors a followed publisher answers for it, and a publisher with neither a mirroring home hub nor a `sources` entry is
reached only by `111 dataset import --from` (§7.6). No timer refreshes a catalogue.

**6.3 Freeze stance (normative; L7 §9.5 T4).** A node keeps the highest verified `seq` per publisher (`state/origin/catalogue/<publisher hex>.toml`) and
refuses a lower one. A stale mirror can hold a node on an old catalogue until the node reaches another source; the Datasets view shows "newest catalogue seen:
seq N from <source>". No clock expires a catalogue.

**6.4 Publishing (normative).**
1. A draft is written by `111 dataset add` (§7.6) or a first-copy job (§7.5), only after its bytes and its sidecar passed.
2. The hub owner publishes on a human action: `POST /api/v1/datasets/publish` (loopback or a paired session; the bearer is refused; `409 needs_hub` without
   the hub role). `111d` builds the new catalogue from its previous one, the drafts named and the removals named, mints its `seq` from the L2 announce
   counter (`next_announce_counter()`, the counter Layer 5's epochs use, `5-messaging/README.md:196-201`), gives each new entry that same `seq`, and signs
   it through the identity handle: one counter value and one signature per publish. A words-only restore floors the counter above every earlier value
   (`2-identity/README.md:206`), so a publisher restored onto a new disk publishes a `seq` above its last and its followers accept it (§6.3); no state of
   Layer 8's is added. The catalogue is a second caller of the counter; Layer 5's epoch rule is unaffected, because a mint only ever moves the counter up.
3. Only an entry whose file and sidecar the hub holds and verified can be published.

**6.5 Endorsements (later).** A further signature over one entry under its own tag `111/catalogue-vouch/v1` (reserved in §21 L2, the pattern of Layer 9's
`111/fix-vouch/v1`), shown as "vouched for by <alias>", never as automatic trust.

**6.6 Costs.** No new key (v1 signs with the root node key, which the words recover), no crate, process, listener or timer: the Ed25519 verifier and the TOML
parser are linked. 64 B per catalogue; one verify per catalogue fetched.

## 7. Moving datasets in v1 (D34 F6)

**7.1 The Range arm (normative).** One function beside `file_body` (`N/panels/static.rs:181`), used by the panel host, the origin's blob route (Layer 7 row
D7, built: `N/origin/mod.rs:1-37`) and the tiles route (§10): (1) `Accept-Ranges: bytes` on every 200 and 206. · (2) One range (`bytes=a-b`,
`bytes=a-`, `bytes=-n`) ⇒ 206 with `Content-Range: bytes a-b/size` and `Content-Length`. A range past the end ⇒ 416 with `Content-Range: bytes */size`. Two or
more ranges ⇒ 416: never a multipart body (§22 D10). A 416 under `/api/v1` (the tiles route) and on a panel listener carries a `problem+json` body,
code `range_not_satisfiable` (§16.2); on `/origin/v1/` it has no body. · (3) `If-Range` against the existing ETag (`etag_of`: inode, mtime, length,
`N/panels/static.rs:163-170`): a mismatch serves 200 and the whole body. · (4) Always `Content-Length`; never compressed, never chunked (IIAB's PMTiles
workaround, R07g3 §4 finding 9). · (5) The body streams from disk in the existing 64 KiB chunks (`N/panels/static.rs:42`). The 64 MiB cap (`:39`, `:227`)
stays for panel bundles; the origin and tiles routes pass none. · (6) GET and HEAD only. Estimated 60 to 120 lines, no dependency (R05 finding 17).

**7.2 The stream cap (normative).** At most `origin.max_streams` (default 4, the lead's number, Q24) bodies on `/origin/v1/blob/*` at once per hub; the next
request answers 503 with no body. kiwix-serve's `--ipConnectionLimit` is the precedent (R07g3 §4 finding 6). A blob body whose reader accepts no byte for
60 s (default; measured at build) is closed and its stream slot freed, so four slow readers cannot hold every stream. The session-gated tiles route has no cap.

**7.3 `get_to_file` (normative).** (1) New in `NI/util/http.rs`: `Fetcher::get_to_file(url, range, file, offset)` streams one body into a file at an offset.
Every call is one bounded range: one block from a node origin, one window of 16 MiB from an upstream (§7.5; default, measured at build). The locked ureq 3.4.2 has no per-read
timeout (its knobs are `timeout_global`, `_per_call`, `_resolve`, `_connect`, `_send_request`, `_await_100`, `_send_body`, `_recv_response` and
`_recv_body`, `ureq-3.4.2/src/config.rs:726-821`); `timeout_recv_body` bounds the whole body of one call, so on a one-range call it is the stall bound: the
range's length at a floor rate of 64 KiB/s (default; measured at build): 256 s for a 16 MiB block, 64 s for 4 MiB. A call that passes the bound counts as a failed block ((3)(c)).
A 200 answer to a range at an offset above 0 ends the job `failed (source cannot resume)`; nothing of that answer is written. `Fetcher::get` (the whole body
in memory under one global timeout, `NI/util/http.rs:134-151`, `:172-200`) is unchanged. · (2) The scheme rule is T6's (`NI/util/http.rs:1-17`): https; http only to the home
hub's `origin_base` (`HttpException::HomeHubOrigin`, `:25-32`) or to a URL the user typed for a dataset, a new typed exception `UserTypedDataset` beside
`UserTypedFrom`. · (3) **A pull from a node origin:** (a) fetch the sidecar `blob/<outboard_sha256>`, asking for exactly its length, `(ceil(size / 2^(10 +
ob_log)) − 1) × 64` B from the signed `size` and `ob_log` (§5.2 (2); any other length is refused, and an entry whose figure exceeds 8 MiB is refused before
any request), and check it with `valid_outboard_ranges` against the entry's root, so a swapped sidecar fails before any data byte; its sha256, computed as
it arrives (or as it is read from a directory, §7.6), must equal the entry's `outboard_sha256` before the sidecar is renamed into `blob/`: a mismatch ends
the job `failed (name mismatch)` before any data byte is requested and deletes the sidecar's copy, so a signed `outboard_sha256` that names a release pin
or another publisher's file never places bytes under that name; (b) request block-aligned
ranges, one block per request, into `state/origin/incoming/<sha256>.part`; (c) check each block with `valid_ranges`, refetch a failed block once from the
same source and then from the next source of the job's source list (§7.4); (d) set its bit in `.blocks`; (e) after the last block, `fsync`, then **prove the name**: the sha256 over the
file's bytes, hashed in arrival order when the pull ran front to back from its first block in this process, else by one sequential read of the `.part`.
A mismatch ends the job `failed (name mismatch)` and deletes the `.part` and its bitmap; a match renames the file and the sidecar (its name proven in (a))
into `blob/` and updates the holdings index. An existing `blob/<sha256>` is never replaced, the sidecar's name included: a pull whose name the tree already
holds keeps the held file and discards its own. The
BLAKE3 root is proven block by block (R07g3 finding 5); the sha256 proves that the name in the shared tree (release payloads, other publishers' files, §11.1)
belongs to these bytes, so one publisher's entry can never place bytes under another pin's name. · (4) Hash-named URLs make a resume against any source
safe; two hubs can serve different blocks of one file with no extra protocol (R02 finding 3). · (5) The `.blocks` header records the boot id
(`/proc/sys/kernel/random/boot_id`) of the run that last wrote it. A job that resumes under the same boot id (the daemon restarted, the machine did not)
trusts the set bits and re-checks only the block that was being written; under another boot id (the machine restarted, the page cache may have lost
writes) it re-checks every marked block against the sidecar.

**7.4 Jobs (normative).** At most one dataset job runs per node; accepted fetches queue in order. A job runs on one named thread, `111d-dataset`, that exists
only while a job runs, with one block buffer. · States: `queued` · `running` · `paused (offline)` · `paused (metered cap)` · `paused (metered link)` ·
`paused (disk low)` · `paused (source busy)` · `done` · `failed (<reason>)` (`failed (name mismatch)`, §7.3 (3)(a) and (e), and `failed (source cannot resume)`,
§7.3 (1), among them) · `cancelled`. A pause is a state, never an error (OB §3). · What resumes a paused job, with no timer of its own: offline — the source
hub's link attribution (L7 §9.3's event), the user's Resume, a daemon start; metered cap, disk low — the capacity pass (§4.3 step 6); source busy — a retry
on the capacity pass after a gap counted in passes that doubles from 1 to 32 (§4.3 step 6); metered link — the human accepting the new text (§7.8). An
offline job never retries on a timer. · **Sources.** A job's source list is the home hub's `origin_base` (L7 §9.1 S4), then the origin URLs that `sources`
(§15.1) lists for the entry's publisher, in their order; a first copy (§7.5) has the one upstream URL the user gave. A block that fails twice at one source
moves the job to the next; `Job.source` shows the one in use. · A fetch for an entry that already has a job (queued, running or paused) answers `409
job_running`; any other accepted fetch queues. · Progress rides SSE `dataset` frames only while a job runs, coalesced (the conventions' rule,
`4-interface/api/README.md:27-29`): at a verified block the job sends a frame only when 5 s, one housekeeping tick, passed on `CLOCK_BOOTTIME` since its
last one (default; measured at build), plus one on every state change and at its end; no timer. · A job's id is `job-…`, minted by `111d` (§16.2). ·
The user's Resume on a job `paused (metered cap)` is the override (§7.8): it answers 409 `consent_required` (`dataset_override`) until the repeat carries
`accept_override`. Cancel deletes the `.part` and its bitmap.

**7.5 The first copy (normative; Q8).** (1) **From upstream, on a hub with internet:** `POST /api/v1/datasets/drafts` with the URL, the upstream sha256 and
the entry's fields. The job pulls the file in consecutive 16 MiB windows, one `get_to_file` call each (§7.3 (1)), each sending `If-Range` with the
upstream's validator, then checks the sha256 the user or the mirror supplied and computes BLAKE3 and the sidecar in the same hash pass: in arrival order
when the pull ran front to back in this process, else one sequential read of the file (§7.11). It places both in the tree and writes a draft. Only then may the owner publish (§6.4). The publisher copies a ZIM's own `License`, `Source` and
`Name` metadata into `licence`, `attribution` and `book`, read off-node with zim-tools' `zimdump` *(unverified from this box: R01's candidates row names
`zimdump`, `research/01-datasets-hub.md:42`; no scout ran it on a ZIM)*. · (2)
**From a disk:** §7.6.

**7.6 Sneakernet (normative).** `111 dataset import --from <dir|url> [--in-place] [--follow [<publisher hex>]]` runs once as the node user (the `111 update --from` pattern, L7
§9.2): it reads `catalogue/<publisher hex>.toml`, `catalogue/<publisher hex>.toml.sig` and `blob/<sha256>` from a directory or a URL in the origin layout,
verifies the signature (an unfollowed publisher: it prints the NodeId and refuses unless `--follow`, which adds it to `follow`), checks each sidecar (its
sha256 against the entry's `outboard_sha256` and its root, as §7.3 (3)(a); a mismatch refuses that entry with "name mismatch") and every block, and copies
into the tree. A directory is listed; a URL cannot be, so from a URL the verb asks for the names a check asks for (§6.2: the home
hub's and every followed publisher's `catalogue/<hex>.toml`) plus the publisher `--follow <publisher hex>` names, and adds the URL to `sources` for each
publisher it found there (§15.1). · **One writer.** The dataset verbs take an `flock` on `state/origin/import.lock`, verify and copy into `incoming/`, then hand the result to the
running daemon on loopback (`POST /api/v1/datasets/adopt`, loopback-only, §16.1). Beside its verified copies the verb stages in `incoming/` the catalogue
files it verified (`<publisher hex>.toml` and `.toml.sig`) and one request file, `adopt-<n>.json`, which lists what it staged and, for each in-place file,
the tuple it hashed (§5.5). The adopt body names that file (`request`) and carries `incoming` (the copies), `catalogues` (the publishers whose staged
catalogue to install), `drafts` (`111 dataset add`'s entries, as `DatasetEntry`), `in_place` (`{sha256, path}` per registered file), `follow` and
`sources`. `111d` verifies each staged catalogue's signature again (§6.1) and its `seq` against the one it holds (§6.3), and `stat`s each in-place path
again and compares it with the tuple the request file recorded; any item that fails refuses the whole request (400 `bad_request` naming the item, or 409
`untrusted_publisher`, `needs_hub` for a draft on a node without the hub role) and nothing is written; the verb then deletes what it staged, and a dataset verb that takes `import.lock` first deletes any `adopt-*.json` and staged catalogue copy left there. Otherwise it renames the copies into `blob/`,
writes `state/origin/catalogue/`, `state/origin/drafts/<id>@<version>.toml`, `state/datasets.json` (the in-place records with their path and tuple) and
(for `--follow` and a URL's `sources`) `state/storage.json`, rebuilds the kiwix library and deletes the request file and the staged catalogue copies:
`111d` is the only writer of the catalogue, the drafts and the two JSON files while it runs. With the daemon stopped (the `node/lock` of §13.2 is free) the
verb writes them itself under both locks. · `--in-place` (hubs only) registers a file on an attached disk with one hash
pass and copies no data: the sidecar (a small blob in `blob/`) and the index are written, the file stays where it is; it refuses a path the daemon's user cannot read (the unit runs `ProtectSystem=strict` and `ProtectHome=true`,
`1-bootstrap/systemd/111node.service:35-37`). The file is then checked by its tuple (§5.5): a changed or missing file, or an unclean shutdown, makes it
`unverified` or `disk_absent`, and it is hashed again before it is served (Q19). · `111 dataset add <file> --id --kind --name --version
--licence --attribution --source-url [--sha256 HEX] [--ob-log N] [--in-place]` (hubs only) computes sha256, BLAKE3 and the sidecar in one pass, places the
file and writes a draft, through the same lock and hand-off. ·
FAT32 caps a file at 4 GiB: the docs name exFAT or ext4; no `zimsplit` (Q19). On Termux datasets stay app-private, and the copy says that uninstalling Termux
removes them (R05 finding 16).

**7.7 Consent and space (normative).** Every fetch, draft and dashboard import is the pinned round-trip (L7 §12; `ConsentRequired`, `API:1492-1501`): `409
consent_required` with `consent = {action: "dataset_fetch", subject: "<publisher short>:<id>@<version>", warning_text, hash, addons}`; the client shows
`warning_text` verbatim and repeats with `accept_warning` = the hash; a changed text has a new hash and asks again. The text (§15.2) and its hash hold only
the stable facts: the name, version, size, source, signer, licence and attribution, `used_by`, `fits`, and the connection (the metered rule that matches
and its cap). The live figures travel beside it in `ConsentRequired.facts` (§16.2), shown under the text and never hashed: the free space now and after
(after = the §4.6 figure, the unused budget reserved), the days a metered pull takes, and for the override the bytes left. So another job writing between the
409 and the repeat does not change the hash and the dialog does not ask again. `insufficient_space` is checked again when the repeat arrives: a fetch that
no longer fits says so before any byte moves (`409 insufficient_space`). Accepting a `dataset.fetch` proposal opens the same dialog and never skips it.

**7.8 Metered links (amends L7 §12, additive).** Dataset fetch bytes (bodies and headers, both ways) count in Layer 7's rolling 24 h window on
`CLOCK_BOOTTIME` (`7-relay/README.md:1002-1004`; a stub at the pinned head, `N/relay/metered.rs:1-9`, row D11). **Before every block** the job asks Layer 7's
metered module (row D11) which `metered` rule matches the job's connection (its interface, and the SSID a rule may name, L7 §12). A match the accepted
consent text did not name (a phone that left one Wi-Fi for another, or for mobile data) pauses the job `paused (metered link)` until the human accepts the
new text, which names the new rule. With `metered = []` nothing is metered and the dialog says "No metered link is set." **The carrier's reserve.** Past a
cap no carrier flush runs (`7-relay/README.md:1003`), so a dataset job must never use up the window the carrier needs: on a marked link the job pauses
`paused (metered cap)` when the window has less than the carrier reserve left (10 % of the rule's daily cap and at least 1 MiB, the lead's number) and resumes in the next
window; the dialog's facts show the days: at 1 GB a day, about 8 days for 7.4 GB and 57 days for 57 GB (R05 finding 8). The override (`dataset_override`)
is a second explicit tap with its own text, per download, never the agent's (OB F6 (c), default (b)): `accept_override` on the fetch, or on the Resume of
a job already paused at the cap (`POST /api/v1/datasets/jobs/{job}/resume`, §16.1); the bytes it moves are counted and shown on the job
and are not added to the window, so an override never closes the carrier's gate. The window's persistence cadence is Layer 7's, measured under its board; a
pull adds bytes to it, not writes (options-check K1).

**7.9 Refresh and removal (normative).** A new version of a held `id` is a new fetch: staged, checked, renamed. The old version is removed only when the user
accepts the refresh, whose consent names the bytes it frees; a refresh that does not fit beside the old version says so before any byte moves (R01 finding 7).
Removal (`DELETE /api/v1/datasets/entries/{publisher}/{id}/{version}`, a human action or an accepted `dataset.remove`) deletes the file and its sidecar unless another held entry
or a pin list (§11.1) still names them.

**7.10 Reach (normative).** v1 moves datasets over the LAN and to hubs with a reachable address (a public address or Layer 7's reach layer, L7 §6.6). A
dataset never rides a carrier or LoRa: Berlin's 84 MB is about 9 days of LongFast airtime (R02 finding 12).

**7.11 Costs.** Nothing resident. During a job, one thread and one block buffer; one sequential write per file plus its sidecar, one `fsync` and a rename (57
GB is about 47 min at an assumed 20 MB/s, R02 finding 13). The name's sha256 (§7.3 (3)(e)) costs CPU only when the pull ran front to back, hashed as the
blocks arrive; after a resume, a pull from several sources out of order or a daemon restart it costs one more sequential read of the file (57 GB is about
47 min more at the same assumed rate). A bad block costs one block, where a whole-file check would cost the file again (57 GB is 12.7 h at 10 Mbit/s, R02
finding 4). An out-of-tree file's re-hash (§5.5) is one read of that file, queued, never per request. **Reads, listed.** Each block check reads its block
back through `valid_ranges` (`BT/io/sync.rs:813`: "it recomputes the hashes for all chunks"), usually from the page cache just written; a resume under
another boot id reads every marked block once (§7.3 (5)). A first copy's hash pass (sha256, BLAKE3 and the sidecar together) costs one sequential read of
the file when it did not run front to back (about 12 min for 57 GB at an assumed 80 MB/s, R02 finding 13). An import or `add` reads its file once (sha256,
BLAKE3, the sidecar and every block in one pass) and, in copy mode, writes it once. After an unclean shutdown every in-place and cache file is read once
more (§5.5), queued one at a time. No listener.

## 8. Tier 2 object transfer (D34 F7; later)

1. **Owner.** Layer 8 owns the Tier 2 build (`OBJ_ADVERT`, `OBJ_REQ`, `OBJ_CHUNK`, L5 §11), in v1.1, as an additive Layer 5 amendment list Layer 8 writes and
   the owner sees. Layer 7's broadcast pointer (an `obj_hash` "fetched by Tier 2", `7-relay/README.md:966`) and Layer 9's patches use it when it lands. L7
   §17's Tier 2 row gains this owner (§21 L7).
2. **v1 builds none of it.** The kinds stay registered and unbuilt (`M/envelope.rs:543-548`; `built_in_v1` false, `:629-634`); an inbound one falls through
   the dispatch (`M/fabric.rs:4063`); the chunk frame is discarded (`M/fabric.rs:3821`).
3. **Prerequisites.** `bao-tree` 0.16 → 0.17 at Tier 2's start (Q22): a lock bump, MSRV 1.85, the `LICENSING.md:151` row, regenerated notices; 0.17 fixes
   `encode_ranges` and decode-after-error on the range paths Tier 2 uses (R02 finding 6). Bulk on its own connection or Layer 7's v1.1 non-blocking
   `Link::send`, because 57 GB on the chat connection would stall chat (R02 finding 9). Hub to hub over the internet it rides L7's TLS-RPK link; public
   adverts are signed under the registered `111/obj/v1` (`L2:364`).
4. **Layer 8's storage half**, built in v1 where §5 and §7 need it: the size check against free space, a refresh that keeps the old version until the user
   accepts its removal (§7.9), resume by block.
5. **Costs.** None in v1. In v1.1: the two prerequisites, no listener of its own, one block buffer per active check; binary and RSS deltas measured then.

## 9. Encyclopedias, and what a hub shows the LAN (D34 F8)

**9.1 The `child` panel source (amends L5 §13, additive; normative).** The panel host registers `kind = "static"` only; `child` and `detected` are refused at
registration (`N/panels/manifest.rs:12-13`, `:400-403`; the reverse proxy L5 §13 specified and reserved, `5-messaging/README.md:33`). Layer 8 builds `child`
for children that speak plain HTTP: (1) A `child` manifest names `[source] component = "<id>"` and `upstream = "127.0.0.1:<port>"`, the port one of that
component's `[run] ports` (loopback, claimed by the components runner, `N/components/manifest.rs:312-314`, `:888-898`). `detected` stays reserved. · (2)
`[proxy] websocket` and `sse` stay `false`; `true` still fails adoption (`N/panels/manifest.rs:14-16`, `:510`; D20 (a)). · (3) The panel listener forwards
each request it admitted (the ticket → `p111_<id>` cookie and every guard of L5 §13, unchanged) to `upstream` with the `hyper` client node111 already links
(`1-bootstrap/crates/node111/Cargo.toml:61`), the request body capped at `max_body`. It strips the panel cookie and the dashboard's session cookie before
forwarding, and the panel host's header policy filters the answer. That policy applies no CSP of ours to a panel today (`N/panels/headers.rs:13`, "never
apply the dashboard's own CSP") and the static panel's lint cannot read a child's pages, so a `child` panel's answers get one more appended header,
`Content-Security-Policy: connect-src 'self'`: a page's scripts reach only the panel's own origin, never the internet from the reader's browser (§17
states the residual). · (4) The panel's origin is its own port in the panel block (`N/panels/mod.rs:56`), so a ZIM's JavaScript runs there, never in the
dashboard's origin (R01 finding 5). · (5) **Id, enablement, health and place.** A panel's id is a `ServiceId` today: the manifest parser answers "not a
service id" otherwise (`N/panels/manifest.rs:387`), the enum is closed at nine (`N/services/mod.rs:87-99`) and the contract's `panelId` is that enum
(`API:674`, `:696`), so `POST /api/v1/panels/kiwix/session` would answer 404. A panel is enabled only when it is registered, its bundle directory exists and
its service is set up (`N/panels/mod.rs:22-27`, `:536`), and a `child` panel has neither a service nor a bundle. Layer 8 amends: a `child` panel's id is its
component's id, so `panelId` is a `ServiceId` or a `ComponentId` (`API:1278`) and the parser accepts a declared component id for `kind = "child"` only; it
has no `bundle`; it is enabled while its component is enabled, so it holds its panel listener from enable on (§19); its health is its component's port
(the component's `health = "listen"`, §9.2); and it is framed from Storage → Datasets ("Open encyclopedia", §15.4), not from a service's LIVE page
(`N/panels/mod.rs:2`). §21 L5 (5) and §16.6 (7) carry it.

**9.2 The kiwix component (amends L6 §2.1, additive; normative).** `assets/components/kiwix.toml` (embedded in the release binary like every declaration,
`N/components/manifest.rs:1-5`, `:88`). Today's parser refuses such a declaration; this is the whole additive set it needs (§21 L6, pinned in §16.6
(7)). · **No `adapter`.** A server component names no network; today `adapter` is required and must be a network id
(`N/components/manifest.rs:685-686`), `Component.required` holds it (`API:1304`, a `NetworkId`, `:1308`) and the UI row labels the component by it
(`UI/src/views/settings/l6/components.tsx:52`, `:63`). It becomes optional in the parser, nullable in `Component` (it stays in `Component.required`, typed
`NetworkId` or null, so no key vanishes from an answer, `4-interface/api/README.md:17`) and in the UI row (null = a server component, labelled by its
`title`). An adapterless component's state and log directory is `state/<id>/` (today a component's log is
`bridge/<adapter dir>/<id>.log`, `N/components/mod.rs:830`, `registry.rs:58`), inside the §13.8 ring; "remove files" removes it as it removes
`bridge/<adapter>/` today (`N/components/mod.rs:20`). The routes stay `/api/v1/bridge/components/*`. · **Payload.** `kind = "binary"` with per-target tables
(`N/components/manifest.rs:21-31`); a `.tar.gz` unpacks whole into `components/kiwix/` (`N/components/registry.rs:599-607`), so `kiwix-serve` and
`kiwix-manage` arrive together. Upstream's tarball unpacks under a top directory that carries the target and the version, so each
`[payload.targets.<t>]` table gains an optional `program` key, the inner path of the program in that target's tree (a `rel_path`,
`N/components/manifest.rs:458-476`; `RawTarget` holds `url`, `sha256` and `size` today, `:179-185`); `[run] program = "{payload}"` (`:570`, `:847-858`)
then names the target's `program` when its table has one, else the URL's file name as built. `kiwix-manage` is run from the same directory. Linux: the
upstream static kiwix-tools tarball (a 3.8.2 aarch64 or musl build is *(unverified from this box)*, R01 finding 3; the manifest pins whichever bytes pass
the smoke). The lock run sets `REQUIRE_G16=1` with the `aarch64-linux` pin in `kiwix.toml` (its sha256, its size and a measured `[weight]`), so the Pi hub's
encyclopedia rests on a pin G16 ran; if no such build exists, §0's row says so. Termux: the deb closure (kiwix-tools 3.8.2, libzim 9.8.2, libkiwix 14.2.1
with ICU, Xapian, libmicrohttpd, libcurl, pugixml, zstd, lzma, docopt; R01 finding 2), which waits for its measurement before any phone path (Q17). ·
**`[install] prefetch = false`** (`N/components/manifest.rs:53`): fetched on a hub by `sudo 111 install --components kiwix`, never on a phone by default
(Q17). · **`[run]`:** `program = "{payload}"`, `args = ["--library", "{root}/state/kiwix/library.xml", "--threads", "2", "--nolibrarybutton",
"--blockexternal", "--address", "127.0.0.1", "--port", "11250"]` (`{root}` is a known placeholder, `N/components/manifest.rs:531`, `:540-545`; no new one
is added), `ports = [11250]`, `health = "listen"` and the new key `start = "on_demand"` (§9.3). `health = "listen"` is a third health kind beside `ready`
and `none` (`N/components/manifest.rs:300-305`, `:910-927`): healthy once the first declared port accepts a loopback connection within
`health_timeout_secs`. `--blockexternal` makes kiwix-serve refuse a ZIM page's links to the internet (`kiwix-serve.cpp:73`, R07g3 §4 finding 6). Never
`--monitorLibrary`, which adds a per-second stat (R07g3 §4 finding 6). The thread count is unmeasured; `--address` comes from `kiwix-serve.rst` (R01
finding 1) and is re-read at build (R07g3 §4 finding 6 did not re-check it). 11250 is outside the base ports (`N/components/manifest.rs:962-964`) and every
declared one. A node111 unit test parses the embedded `kiwix.toml` and `panels/kiwix.toml` (G16). · **`[license]`:** GPL-3.0-or-later (kiwix-tools, libkiwix),
GPL-2.0-or-later (libzim), read upstream (R01 candidates). A separate program: D22's linking boundary is untouched. `[weight]` is measured at build (§19). ·
**Panel** `assets/panels/kiwix.toml`: `id = "kiwix"` (the component's id, §9.1 (5)), no `bundle`; `[source] kind = "child"`, `component = "kiwix"`,
`upstream = "127.0.0.1:11250"`; `[auth] mode = "111-session"`;
`[license] spdx = "GPL-3.0-or-later"`, `served = "unmodified"`. · **The library.** `111d` keeps one symlink per held ZIM
(`state/kiwix/zim/<publisher short>-<id>-<version>.zim` → its blob, §5.1, so the file name carries the extension and two publishers holding one `id` and
version never collide) and rebuilds `library.xml` with `kiwix-manage` (adopted, from the same tarball) when a ZIM is added or removed, then sends `SIGHUP`
to a running kiwix-serve, which reloads its library without a watcher (R07g3 §4 finding 6). Before it starts kiwix-serve and before each rebuild it makes
one `stat` per in-place ZIM against its tuple, and a ZIM that no longer matches leaves the library first (§5.5).

**9.3 Lifecycle (normative).** (1) Enable is Layer 6's: the licence notice and its `component_enable` consent, the cache check, the install
(`N/components/mod.rs:12-20`). Enable today runs install → claim and pre-bind the ports → start → health (`N/components/mod.rs:12-19`), and every enabled
component is enabled again at daemon start (`resume_enabled`, `:948-960`). With `start = "on_demand"`, enable stops after the install, and
`resume_enabled` skips the component: nothing is claimed, bound or started. · (2) The first panel session mint (`POST /api/v1/panels/kiwix/session`)
claims and pre-binds the port, starts kiwix-serve as a supervised child, with the runner's back-off, and answers once its port answers (`health =
"listen"`). · (3) It stops when the user stops it (Stop on Storage → Datasets: `POST /api/v1/bridge/components/{id}/stop`, a new route valid for an
`on_demand` component only, §16.1; or disable), at daemon stop, and on the existing capacity pass when no request reached the panel's proxy and no
`encyclopedia.search` reached the search route (§9.4) for 30 passes (the lead's number; §4.3 step 9: the proxy and the route see every request, and each
resets the same counter, so the idle stop needs no timer of ours and never stops kiwix-serve under an agent that is searching). The next panel open starts it again. kiwix-serve wakes once a second
while it runs (`kiwix-serve.cpp:423,451`, R07g3 §1); the idle stop keeps it from being left resident (R05 finding 16). · (4) While off: no process, listener, timer or open
file of kiwix-serve (G7's zero-weight gate, `6-transport/verify/zero-weight.sh`); while the component is enabled, the panel's own listener (§9.1 (5), §19).

**9.4 `encyclopedia.search` (normative).** The agent tool is named after what it searches: `kb.` is the Layer 0 knowledge base's prefix (`kb.read`,
`A/src/tools.rs:78`; the budget class `kb`), which a model would pair with it (§22 D25). `GET /api/v1/datasets/search?q=<text>&book=<id>&publisher=<hex>&n=<1..20>` (session; the agent's bearer reads). While kiwix-serve runs,
`111d` asks `http://127.0.0.1:11250/search?pattern=…&books.name=…&format=xml&pageLength=n` (a documented public endpoint, R05 finding 12) and answers
`{schema, format: "opensearch-xml", body}`, `body` the upstream text capped at 8 KiB; each search resets the idle-stop counter (§9.3 (3)). `books.name` is
the ZIM's own `Name` metadata (R05 finding 12), not the catalogue `id` and not the symlink's name, so `book=<id>` is mapped to the `book` field (§5.3) of
the entry that `publisher` (optional, default the home hub, as `dataset.fetch` takes it, §16.4) holds under that `id`; an entry without one answers `400
bad_request` ("This encyclopedia cannot be searched by name"), and without `book` the search covers every held ZIM. While it is off: `409 component_unavailable` with "The encyclopedia is
off." `111d` parses nothing (no XML crate is in the lock); the agent tool labels the body untrusted. The agent may say a person can open the encyclopedia from
Settings → Storage; it never starts it (the session mint is a POST, refused to the bearer). It does not propose turning it on: no proposal kind enables a
component (the `device` vocabulary, `4-interface/api/README.md:119-122`, and the `relay` one, `7-relay/README.md:1057-1058`, name none), so v1 keeps "it
says it is off" (§22 D24).

**9.5 Raw files on the origin (normative; Q14, Q15).** ZIMs, PMTiles files and their sidecars, and the catalogue files (§16.7), are served read-only and
unauthenticated on `/origin/v1/` only while `hub_role`, LAN sharing and `origin.datasets` (default off) are all on, after the `datasets_serve` consent
(§15.2), under the stream cap (§7.2). kiwix-serve is never LAN-bound: the encyclopedia is read through the panel and the hub's session only. Requests are
never logged.

**9.6 What a hub shows the LAN (normative; §17's exposure table).** With `origin.datasets` on, anyone on the LAN learns the holdings, their versions and
contents, and a map's area; with `origin.components` on, which programs the hub cached. Every holder of the hub's card (its signed announce) learns that
the hub serves a catalogue: `origin.datasets` sets the role bit `ROLE_DATASETS` (§12.4), and the `datasets_serve` consent says so. The kiwix panel and the
Map tab show nothing without the hub's session.

**9.7 Credits (normative).** A Credits section on the Storage page, linked from each panel: per held dataset its `licence` and `attribution` verbatim; for
every map the ODbL notice "Contains information from OpenStreetMap, which is made available here under the Open Database License (ODbL)." (ODbL s.4.3, R01
finding 12); a "non-commercial" tag on any `-NC-` licence; no single licence claimed for a ZIM that mixes image licences (R07g3 fork D D1). IIAB names
OpenStreetMap without the ODbL sentence (R07g3 §4 finding 9): the baseline a hub does better than.

**9.8 Costs.** Disabled: nothing. Enabled and never opened: the panel's listener in the panel block (11190-11229, `N/panels/mod.rs:56`; LAN-bound when LAN
sharing is on), no process. From the first panel open until Stop, 30 idle passes or daemon stop: one process, one loopback listener (11250), kiwix's own
1 Hz wake. Its RSS is published nowhere;
it is measured at build, anonymous against file-backed (OB §3). The only primary figure is IIAB's sizing of a Zero 2 W at ≥ 1,024 MB of RAM plus swap for the
whole box (R07g3 §4 finding 3). The tarball's size on disk is measured at build.

## 10. Maps (D34 F9)

(1) **Libraries.** Leaflet 1.9.4 (BSD-2-Clause) and protomaps-leaflet 5.1.0 (BSD-3-Clause; it embeds pmtiles 3.2.1 and the basemap themes): 80.4 KB gzip, 83.9
KB with CSS, measured by a scout with esbuild (R05 finding 13). One lazy chunk, `map`. · (2) **The exempt `map` bucket** in `UI/scripts/check-budget.mjs` (the
fonts precedent, `:1-7`): 90 KB gzip on its own line, outside the 256 KB lazy budget and the 64 KB per-chunk cap (`:13-14`). L7 §10.5's "≈ 69 KB"
(`7-relay/README.md:951`) reads 83.9 KB (§21 L7); D21 gains the bucket's figure (§22, OB §4 item 9). · (3) **Loaded only where needed.** The chunk loads only
where `GET /api/v1/tiles` lists a set: a node with none never downloads it. · (4) **Data.** PMTiles files as Protomaps publishes them, and regional subsets
made off-node with `pmtiles extract` (go-pmtiles, BSD-3-Clause). Nothing rebuilt from OSM data until the legal read of "a re-extracted vector tileset is a
Produced Work" (Q10). · (5) **Serving.** `GET /api/v1/tiles` lists the held `pmtiles` sets; `GET /api/v1/tiles/{publisher}/{id}/{version}` serves the file through the Range
arm (§7.1) as `application/octet-stream` (§22 D7), keyed as a held entry is (§5.5), so two versions or two publishers of one `id` never collide. Both
follow the usual rule (a paired session or loopback; the agent's bearer reads, as it reads every GET): the browser reads tiles from the dashboard it is paired with, under the unchanged
`connect-src 'self'` (`N/dashboard/routes.rs:84`). Vector tiles draw to a canvas, so `blob:` stays out of the CSP (R05 finding 13). · (6) **Where.** A Map tab
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
| the verified release manifest's `[[deb]]` and `[apk.*]` pins (`NI/manifest.rs:239-240`, `:481-495`; §11.5) | `origin.components`, and the operator's "Termux install files" tick under it | `state/origin/blob/`, fetched from their pinned URLs by a job (§7.4) when the operator ticks it; a pin is served only once `LICENSING.md` carries its verified licence (both rows are unverified today, `LICENSING.md:64-65`, so v1 ships this row off, §0) |

Everything else answers 404. No pin source changes a role bit except `origin.datasets`, which sets `ROLE_DATASETS` (§12.4); `origin.components` changes
none, and `origin.serve` keeps Layer 7's bit 5. "The bytes it holds match it": a file in `blob/` was proven against its name before its rename (§5.1 (3), §7.3 (3)(e)); a
file served from outside the tree is checked by its tuple (§5.5). The components cache is root-owned (`L1:142`), so the daemon cannot copy into it or
hard-link out of it; it serves those files where they are (§22 C9), hashing each against its pin when `origin.components` turns on and whenever its tuple
changes, never per request, and serving ends with the file (Q26).

**11.2 `origin.components` (normative; Q13, Q14).** Default off, needs `hub_role` and LAN sharing, and the `components_serve` consent (§15.2), which names the
operator's duty. The `lxmf` closure is excluded until the owner's upstream contact answers (Q25; `LICENSING.md` §9 item 16). Each served blob has a notice
beside it (§11.7).

**11.3 The installer's origin list (amends L1 "Layer 6: optional components" and L6 §2.1 rule 3, additive).** `PayloadFetcher`
(`NI/install/packages.rs:255-262`) gains `fetch_pin(sha256, size, upstream_url)`, which tries `[the home hub's origin_base, the pinned upstream URL]` in that
order; a miss (404, a wrong size, a wrong hash) falls through. The notice is printed and recorded before any download, as today
(`NI/install/packages.rs:265-289`), now naming the source; the receipt records it. Enable stays offline and "There is no runtime fetch"
(`6-transport/README.md:146`) holds: the verb runs at install time. A node with no home hub is unchanged. `origin_base` is the one Layer 7 learns at enrolment
(L7 §9.1 S4). The root installer runs with no daemon, so it reads `home_hub.address` from `fabric/settings` (`N/fabric/settings.rs:8`) and forms the same
`http://<hub host>:11180/origin/v1/` (`NI/util/http.rs:27`); no `home_hub` there means no hub in the list.

**11.4 Pip closures: the files table (normative).** A pip lock holds hashes, not file names (R07g2 finding 6). `gen_lock.py`
(`6-transport/sidecars/lxmf-sidecar/gen_lock.py`) also writes a files table (name, sha256, size, per target) beside the lock, embedded the same way. The
installer pulls each wheel from the hub by hash, writes it under its name into `cache/packages/<id>/`, and enable's offline `--require-hashes` install is
unchanged (`L1:262-264`). The hub path needs no throwaway venv.

**11.5 Termux debs and APKs (normative).** The release manifest pins each deb and APK (R07g2 finding 9; `NI/manifest.rs:239-240`). They are their own pin
source behind `origin.components`, as OB F10 A decides, not under `origin.serve`: Layer 7's tree holds "its own target's payload" only (L7 §9.1), so no
step placed their bytes on a hub, and the notice route could not name their licences. Under `origin.components` the operator ticks "Termux install files";
a job (§7.4: free-space check, the metered window, resume by block, the name checked against the pin's sha256 and size) then fetches each pin from its
pinned URL into `state/origin/blob/`, and the `components_serve` consent's text names them (addon `termux`, §15.2). A pin is served only once
`LICENSING.md` carries its verified licence; both rows are unverified today (`LICENSING.md:64-65`), so v1 ships the pin source off (§0). A provisioning
host fetches them by hash from a hub as from any origin. A hub never re-signs an APK (R07g2 finding 11). The Termux apt directory (`termux-apt-repo`, Apache-2.0) and a signed APK index
(`fdroidserver`, AGPL-3.0, on the publisher's machine) are later (OB F10 B).

**11.6 The 111 release's source tarball (normative).** GPL-3.0 §6(d) binds whoever conveys a binary (R07g2 item 15), and Layer 7's origin already conveys the
111 payload, which §9 never mentions (R07g2 item 22). The release manifest gains a `[source]` pin (url, sha256, size of the tagged source tarball), added
before the first signed release as Layer 5's sections were (`manifest-version` stays 1, `NI/manifest.rs:140-143`). A hub with `origin.serve` also serves that
blob, and the payload's notice names it. **Who fetches it.** The updater of a node with `origin.serve` on fetches the `[source]` pin after it staged its
payload: the same origin order as the payload (its home hub's origin, then the pinned URL), sha256 and size checked, its bytes counted in the metered window
(§7.8), the file placed in `state/origin/blob/` like the payload. If the fetch fails, the hub still serves the payload and its notice names the upstream
URL of the source tarball instead. Its size is measured at release; the Rust sources and manifests alone are about 1.7 MB gzipped (L9 R07g1 §1).

**11.7 The licence duty (normative; amends D26 (5) by one sentence, OB §4 item 9).** D26 (5) gains: "When a hub is the source, the hub's operator is the one
who conveys; a member's acknowledgement records the member's own install." The hub code gives the operator the means: `GET /origin/v1/notice/{sha256}` answers
a text per served component blob: the declaration's `[license]` `spdx`, `read_at` and `notice` verbatim (`ComponentLicense`, `API:1295-1301`; a
declaration has no other licence field), and the licence text when the release carries it in `THIRD-PARTY-NOTICES.md`. "Where its source is" is therefore
the notice's own text: a node111 unit test checks that each embedded declaration's `notice` names a source URL (G18).
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
`SYNC` (0x10-0x14, `M/envelope.rs:534-542`) from a contact falls into `k if k.carries_msg_seq() => self.on_data` (`M/fabric.rs:4039`) and is stored as a
message row of its class (`on_data`, `:4078`; `Class::from_kind` and `receive_message`, `:4145-4151`), which the UI draws as a textless chat row
(`message_view` decodes text for `MSG` only, `:7811-7818`).
1. The fabric gains `Fabric::set_consumer(kind, consumer)` for 0x10-0x14. Layer 9 registers 0x10 and 0x13: its F13 item 13 asks for the same hook, built once
   by whichever layer lands first.
2. An inbound frame of those kinds still passes L5 §4's sequencing and acknowledgement (it was received). With no consumer registered its body is dropped
   and counted (`unconsumed`, per kind, a session counter on the `fabric` status), and the sequence is committed and acknowledged. With a consumer the
   delivery crosses two stores, so the order is fixed: the fabric decides the sequence (new, duplicate or stale) without committing; a new frame's body
   goes to the consumer, which returns only after its own durable write (in `node/state.db` or wherever it keeps state) and is idempotent by the frame's
   `msg_id` (`5-messaging/README.md:191`); then the fabric commits the sequence and acknowledges. A consumer error leaves the frame uncommitted and
   unacknowledged, so the sender retransmits it; a crash between the consumer's write and the fabric's commit redelivers the frame, and the consumer's
   idempotence keeps one record. A duplicate is re-acknowledged without reaching the consumer. Layer 9's `PATCH` and `AGENT_OP` consumers rely on this
   order (`9-evolution/README.md:477`). It never becomes a message row.
3. `MSG` and the custody, mailbox and enrolment kinds are unchanged.
4. Cost: none resident; the row, its bytes and its place in a chat thread go.

**12.4 Catalogue updates without gossip (normative; §22 D8).** **The role bit.** `origin.datasets`, while its `datasets_serve` consent is recorded (with
`hub_role` and LAN sharing on), sets a new `appdata` role bit, `ROLE_DATASETS` = bit 7 (`1 << 7`): the next free bit after Layer 9's bit 6 `ROLE_HEAL`
(`9-evolution/README.md`, §23 L7) in the u16 roles field (L2 A8 and A17, `2-identity/README.md:375`, `:478`; bits 3-5 are node111's, `N/fabric/sealer.rs:175-185`).
It is set by node111's `roles()` beside bits 3-5 (`N/fabric/mod.rs:592-607`) and named `datasets` in `role_names`. Bit 5 keeps its Layer 7 meaning,
`origin.serve` consented, so a Layer 7 member's behaviour is unchanged; `origin.components` changes no bit. **The check.** A member asks its home hub for
the hub's catalogue with one GET on the link-attribution event L7 §9.3 uses for releases, only while the hub's signed announce carries `ROLE_DATASETS`
(no "last answer" state is kept), and whenever the user opens the Datasets view or taps Check now (`POST /api/v1/datasets/check`). The GET sends
`If-None-Match` with the ETag of the catalogue it holds (the hub answers 304 when it is unchanged: the `If-None-Match` handling of `file_response`, the serving function beside `file_body`, `N/panels/static.rs:231-237`, which the Range arm keeps),
its bytes count in the metered window (§7.8), and it is skipped while that window's cap is reached. A check also asks the node's other sources for the
followed publishers' catalogues (§6.2). A hub without the bit gets no request on attribution; one that turns `origin.datasets` on later is noticed at its
next announce. The Datasets view's empty state offers Check now. No head frame, no timer, no gossip. A followed publisher's new entry will travel as a
signed `BROADCAST` pointer (L7 §11.1), later.

## 13. Backup, SD wear and migrations (D34 F12)

**13.1 The export format (amends L2 §8.5(i): built, with its tag; normative).** L2 §8.5(i)'s layout (`L2:218`), byte for byte:

| Field | Bytes | Value |
|---|---|---|
| `version` | 1 | `0x01` |
| `ephemeral_pk` | 32 | fresh per export; `shared = X25519(esk, own_enc_pk)` (abort on all-zero), `key = BLAKE2b-256("111/seal/v1" ‖ shared ‖ epk ‖ rpk)` (L2 §6.4, `L2:122-130`) |
| `ciphertext` | per 64 KiB plaintext chunk: the chunk + 16 | XChaCha20-Poly1305, nonce = chunk index (u64 LE) ‖ 16 zero bytes, AD = chunk index ‖ `last_chunk` (Layer 5's private-object framing, `M/blobs.rs:12-21`) |
| `node_sig` | 64 | Ed25519 by the node key over `"111/export/v1" ‖ NodeId (32) ‖ u64 LE length ‖ BLAKE3(every byte before node_sig)` |

The plaintext is a tar (`tar` is linked, `1-bootstrap/crates/node111/Cargo.toml:45`) of `manifest.toml` (schema 1, NodeId, each store's `user_version` and
size, the release that wrote it) and the two stores, each checkpointed into one file first (§13.2) and still SQLCipher, keyed from the words. The fabric
store is `store.db` plus its blob files and `display-times*` (every open sweeps the blob files its DB does not list, `M/store/blobs.rs:345-365`): the tar
carries `fabric/display-times*` and the `fabric/blobs/` files of the rows that cannot be fetched again (own attachments; re-fetchable public objects are
left out), so an export taken before own history is pruned holds its attachments. A fresh ephemeral makes the key unique, so
nonces never repeat; a cut at a chunk boundary fails decryption; one AEAD call over a store would hold it in memory and the file can be gigabytes, hence the
chunks and the prehash (§22 D11). Restore verifies the signature before decrypting (one BLAKE3 pass), runs `quick_check` on each store, then places them
(§13.2): the fabric store's blob files and `display-times*` before its `store.db`.
Hub-storable, never published (L2 §8.5(i)). No new crate. `age` stays a possible format for hub-side tooling (OB §4 item 10): it pulls `hpke`, `ml-kem`,
`p256`, `scrypt` and `i18n` and carries no sender signature (R04 finding 14).

**13.2 The verbs (normative).** `111 store export --out <file>` and `111 store import --from <file>` run as the node user on a local terminal, never over HTTP
(the `identity backup` rule, `L2:311`), with the daemon stopped: both refuse while `111d` runs ("Stop the node first: sudo systemctl stop 111node"). **The
lock.** `111d` holds an exclusive `flock` on `<root>/node/lock` (0600) for its lifetime: one descriptor, opened `O_CLOEXEC` at start (the
`state/staging.lock` pattern, `N/updater/stage.rs:20`, `:39-79`); the verbs take the same lock without waiting and refuse when it is held. A `111d` that
starts while a verb holds the lock does not wait: it exits with status 75 and the message "a store verb is running", the unit restarts it after its
`RestartSec=5` (`Restart=always`, `1-bootstrap/systemd/111node.service:42-43`), and Layer 9's J2 hook records each such exit with its code and never as a
crash (§3 J2). **Export** opens
each store with its key through the §2.7 S2 open (no epoch decision, the clean record untouched), runs `PRAGMA wal_checkpoint(TRUNCATE)` (after a crash the
last commits sit in `-wal`, which a tar of the main file alone would drop) and reads `user_version`, then tars; a store that does not open is refused, never
skipped. **Import** refuses an export from another NodeId and a store whose `user_version` is newer than the build writes. It sets each existing store aside
as `<file>.replaced-<n>` (the fabric store's blob files stay; the sweep keeps them while the set-aside file exists, §2.7 S3), places the export's files, and
erases `settings.clean` in the fabric store it placed (`FULL`), so the first open reads `unclean` and mints a new epoch (§13.6 (3) gives the reason). A
live snapshot of an encrypted WAL store needs a route this box could not verify (SQLCipher's backup API, `VACUUM INTO`, R04 finding 15, *(unverified from
this box)*), and mesh111's `rusqlite` has no `backup` feature (`5-messaging/crates/mesh111/Cargo.toml:17`): a live export is later (§20; §22 D6). **13.3
The export-before-prune offer** is §4.5: Layer 8 calls `set_export_offered`, so L5 §10.4 step 3 can run at all.

**13.4 Writes only on change (amends L5 code; normative).** The pass persists `capacity.store_full`, `capacity.full_classes`, `capacity.disk_free` and
`capacity.disk_total` (`save_capacity`, `M/store/prune.rs:1337-1350`). Since Layer 7 row M1c (`4209e32`) a pass at rest writes them only when they differ
from the stored values (`rest_pass` and `capacity_saved`, `:1216-1224`, `:1355`), and a pruning pass writes them in its own transaction (`:1178`). That is
not enough on an idle node, because free space moves with any write on the disk (a log line, another program), so the disk figures differ at almost every
pass and the record is committed again. (1) `save_capacity` writes `store_full` and the class mask only when they differ from the stored values: built by
M1c, kept. · (2) Layer 8's remaining change: the disk figures are no longer persisted; the two keys are deleted once, and the compared record holds
`store_full` and the class mask only. `load_capacity` restores the figures into `last_disk` at open today (`:1279-1295`); the first pass runs on the
housekeeping interval's first tick (`N/fabric/mod.rs:2386`, `n = 0`), which replaces them at once (§22 C2). · (3) An idle node then commits nothing to `fabric/store.db` on the pass even while other files on the same filesystem change (G14 (5) asserts it). `PRAGMA incremental_vacuum`
after a pass frees pages only when the freelist holds some (`:1187`, `:1221-1224`). The saving against Layer 7's tree is measured at build (§19); R04
finding 7's 5.9 MB a day described the pre-M1c pass.

**13.5 `journal_size_limit` (amends L5 §10.1, additive).** Both stores set `journal_size_limit` to 4 MiB (the lead's number, about the 1,000-page
auto-checkpoint, R04 fork 4), so the WAL file is truncated after a checkpoint instead of keeping its high-water size; today none of `journal_size_limit`,
`wal_autocheckpoint`, `cache_size` or `mmap_size` is set (R04 finding 5; `M/store/mod.rs:433-493`). `synchronous = FULL` stays where custody receipts need it
(`M/store/rows.rs:1166-1167`). `synchronous = OFF` and a RAM write buffer are rejected: the receipt promises the hub holds the item.

**13.6 The migration copy and the rollback (amends L5 §10.3 and L1's update guard, additive; normative).** (1) Before any migration of `fabric/store.db` or
`node/state.db` (a `user_version` above 0 and below the build's): `wal_checkpoint(TRUNCATE)`, copy the file to `<file>.pre-v<N>.tmp` (N the version the
build writes; same key, same owner, 0600), `fsync`; for the fabric store, open the `.tmp` through the §2.7 S2 open and delete `settings.clean` (`FULL`),
checkpoint and close it, so the copy carries no clean record; then rename it to `<file>.pre-v<N>`, `fsync` the directory, and migrate. A copy cut short (a
start timeout, a power cut) is only ever a `.tmp`, never renamed into place. Copy and migration finish before the daemon reports health: the unit is
`Type=notify` (`1-bootstrap/systemd/111node.service:21`), and the daemon sends `EXTEND_TIMEOUT_USEC` while they run. · (2) The daemon deletes `*.pre-v<N>`,
`agent/legacy-v0/` and `state/heal/legacy-v0.jsonl` (§2.5) once the probation journal (`state/update.json`, `L1:141`) shows the release confirmed. · (3) A probation revert by the guard
runs release N's own `111 update-guard` before the flip (`ExecStartPre=+/usr/local/bin/111 update-guard`, `1-bootstrap/systemd/111node.service:22`;
`revert_to`, `NI/update_guard.rs:22-33`). It gains one step: for each store with a `.pre-v<N>` copy, it moves the store and its `-wal` and `-shm` aside to
`<file>.rolled-back-v<N>` and renames the copy into place (owner `node111`, 0600); and it moves the legacy files back whenever they exist (they exist only until the release that imported them is confirmed, item (2), so the release it flips to writes no node-state store). A
READY-but-unhealthy release is reverted by the root helper instead (`handle_revert_request`, `NI/update_helper.rs:282-308`): it flips, clears the journal
and restarts, so the next `ExecStartPre` runs release N−1's guard (`/usr/local/bin/111` is `current/bin/111`, `L1:146`) with no journal to act on. So the
restore is also a content rule the guard applies at every `ExecStartPre`, whatever flipped `current`: for each store with a `.pre-v<K>` whose K is above
the schema version the running guard's build writes, the same set-aside and rename; a `.pre-v<K>.tmp` is deleted, never renamed in. **The legacy files'
own trigger.** At every other `ExecStartPre`, when `agent/legacy-v0/` or `state/heal/legacy-v0.jsonl` exists and the running guard's build writes no node-state store, the guard moves
the files of `agent/legacy-v0/` back to their places and `state/heal/legacy-v0.jsonl` back to `state/heal/journal.jsonl`, whether or not any store has a
copy: the first release with the node-state store creates `state.db` new (no migration from 0, so no copy) and may migrate nothing else. Both rules run
on both revert paths, the guard's `revert_to` and the root helper's revert followed by N−1's guard. They ship in the release before the first migrating
one and in the release before the first one with the node-state store (item 4). N−1 then opens a store it reads, instead of refusing it (`SchemaTooNew`,
`M/store/schema.rs:727-728`; R04 finding 12). The restored fabric store has no clean record, so its first open mints a new epoch (`decide_epoch`,
`M/store/mod.rs:306-329`): the copy was taken before release N allocated sequence numbers under the old epoch, its `tx_state` is behind them, and keeping
the epoch would reuse an `(epoch, n)` for new content that receivers drop as delivered (L5 §4, `5-messaging/README.md:196-203`). What arrived during
probation stays in the rolled-back file, its blob files kept (§2.7 S3), which the Storage page lists and only the user deletes. · (4) No signed release has
shipped (`NI/manifest.rs:140-143`), so no field store has migrated: Layer 7's V4 and V5 (`M/store/schema.rs:76`; committed, rows M1b and M1c) run on
development nodes only. The copy exists before the first migration after the first signed release; if Layer 7's migration is that one, Layer 7's build
takes this step (Q21, a cross-layer request, §21 L7). Likewise the guard's two rules of (3), the legacy files' trigger included, ship in the release
before the first one that carries the node-state store; if that release is Layer 7's, its build carries them (the same cross-layer request). · (5) **The bound, before the flip.** A probation lasts 600 s from the flip (`NI/probation.rs:57`) and
a health confirmation after the deadline is ignored (`NI/update_helper.rs:326-330`), so a copy that outlasts it leaves a revert at some later start. The
release manifest names the store schema versions each release writes (`[stores]`: `fabric`, `node_state`; additive, as `[source]`, §11.6). Release N−1's
updater compares them with the versions it writes; an update that migrates a store whose copy does not fit above the floor (free − the store's size <
floor), or cannot be copied inside the probation window at the rate measured on an SD card at build, is refused when its manifest is read, before anything is staged: no request waits on
the updater (`POST /api/v1/update/check` answers 202, `API:190-195`), so the refusal is an updater state, `/status.update.last_result` =
`refused_space` (`API:749`, one value appended), with the notice `update_refused_space` naming the release, the store and the bytes needed (§16.2), shown
on the Storage page and Layer 7's update page. A human may accept "update without a rollback copy" (consent `update_without_copy`, §15.2), whose text says
that if this release fails, that store stays closed until the next update. The consent is taken by one route, `POST
/api/v1/storage/update-without-copy` `{release, store, accept_warning?}` (§16.1): a human's, in a dashboard session or on loopback, never the agent's
bearer; it answers 409 `consent_required` with the text first and 204 once the repeat carries the hash. `111d` records it in `state/storage.json`
(`update_without_copy`, one entry per release and store, §15.1); the updater reads it at its next check, and only for that release and that store does
the migration run without a copy; a revert then leaves the store closed (`SchemaTooNew`) until the next update or `111 store import` (§13.2); Start an empty store is for a damaged store only (§13.7). The entry is forgotten once that release
is confirmed (the moment of (2)) or reverted (`111d`'s first start under another release). The seconds per GiB on an SD card are a build measurement that
sets the bound (`8-storage/TASKS.md`).

**13.7 Integrity checks after an unclean shutdown (normative).** `PRAGMA quick_check` and `PRAGMA cipher_integrity_check` (page HMACs, R04 finding 10) run
once when a store's unclean signal is present at open (the fabric store: its clean record absent, `unclean`, `M/store/mod.rs:156-158`; the node-state
store: a non-empty `state.db-wal`, §2.2 rule 9), never on a clean boot. A power cut is a Pi's usual stop, so this is the common boot, and the fabric store
may hold 1.49 GiB or more: the store opens and serves at once, and the checks run on a second, read-only connection on a blocking thread, while `GET
/api/v1/storage` shows that store as `checking` (§16.2). On failure the store is closed and moved aside to `<file>.damaged-<n>` (the fabric store's blob
files kept, §2.7 S3) and stays closed with that reason; the Storage page offers "Start an empty store (the damaged file is kept)" and "Import an export"
(§13.2). Start an empty store is `POST /api/v1/storage/start-empty` `{store, accept_warning?}` (§16.1), loopback only: it names the store, answers 409
`consent_required` (`store_start_empty`, §15.2) until the repeat carries the hash, then lifts the closed state; the node-state store is created at its next use, the fabric store at the next daemon start (its boot runs once per start), and the page says so after the 204. Deleting a set-aside file is `DELETE /api/v1/storage/set-aside/{file}`, loopback only. Never a silent re-creation (Layer 1's "never silent re-key", `L1:204`). The checks' time on an SD card is measured at build.

**13.8 Child log rings (normative; Q6).** `open_log` opens a child's log create-and-append with no cap (`N/supervise.rs:752-755`). On the pass, a capped log
above `logs.max_file_bytes` (default 4 MiB, the drafter's) has its last half copied to `<name>.1`, replacing it, and is truncated to zero; the child keeps
writing at the end, as its descriptor appends (`O_APPEND`) (§22 D15). Capped: every file `open_log` opens, that is `agent/111-agent.log`,
`agent/llama-server.log` (`N/intelligence.rs:274-281`) and each component's and child's log; the `logs` class (§4.1) and its reserve (§4.2 item 3) count
exactly the capped files that exist. The journald namespace keeps its own cap (`SystemMaxUse=100M`, `1-bootstrap/systemd/journald.111node.conf:6`).

**13.9 SD guidance (informative).** Datasets belong on an attached SSD on a hub, never on the boot SD (R02 finding 13): mount it at `<root>/state/origin/`
(§5.1); a register-in-place import copies no data (it writes the sidecar and the index only). An endurance-class card is the docs' advice for a Pi node (no TBW figure is published, R04 fork 4). F2FS
only where the user already runs it. **13.10 Costs.** No timer or process. One descriptor for the daemon's lifetime (`node/lock`). One transient copy of a
store per migration, bounded before the flip (§13.6 (5)); one full read of a store after an unclean shutdown, on a second read-only connection while the
store serves (§13.7). The export's checkpoint, the copy and the two checks are measured at build on the largest store the defaults allow, in one sitting.

## 14. Sequencing and the first Layer 8 release (D34 F13)

**v1** = F1 (§2) · F2 (§3, built with Layer 9's first state) · F3 (§4) · F12 (§13) · F4 and F5 (§5, §6) · F6 (§7) · F8 (§9) · F9 (§10) · F10 (§11) · F11
(§12). **Build order**, the one the dependencies force (the board splits it into rows):
1. The consumer registry (§12.3) and the fabric-store fixes (§13.4, §13.5): Layer 5 code, no dependency on Layer 7's open rows. The move of the
   housekeeping interval's owner (§4.3) lands first of all, after Layer 7's rows on the same function (M1c, M4g, both in), re-pinned when it lands.
2. The node-state store, the chat relay, the spool, the legacy import and the J9 API (§2, §3): before Layer 9's state (D35: "Layer 9 builds after Layer 8's
   state service").
3. The budget, the Storage page, the export offer, the log rings (§4, §13.8).
4. The migration copy, the integrity checks, the export verbs (§13.2, §13.6, §13.7).
5. The Range arm, the object tree, the catalogue and publishing (§5, §6, §7.1-§7.2): after Layer 7 row D7 (the origin's routes).
6. `get_to_file`, jobs, consent, the metered window, sneakernet (§7.3-§7.11): after Layer 7 rows D10 (consents) and D11 (the metered window).
7. The `child` panel source, the kiwix component, `encyclopedia.search` (§9).
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
| `keep_bytes` | `null` (= `keep_default`) | `state/storage.json` | a human on loopback, or an accepted `quota.set {class: "keep"}` decided on loopback | — |
| `keep_default` | computed once (§4.2) | same | nobody: an install-time fact | — |
| `caps.agent`, `caps.heal` | `null` (the share, §4.2) | same | a human on loopback, or an accepted `quota.set` decided on loopback | — |
| `logs.max_file_bytes` | 4194304 | same | a human on loopback | — |
| `follow` | `[]` (the home hub is always trusted) | same | a human only (Settings on loopback, or `111 dataset import --follow`, handed to `111d` through the adopt route, §7.6) | — |
| `sources` | `{}`: `{publisher hex: [origin URL]}` (§7.4) | same | a human only (Settings on loopback, or `111 dataset import --from <url>`, handed to `111d` through the adopt route, §7.6) | — |
| `origin.datasets` | `false` | same | a human on loopback; `409 needs_hub` without the hub role, `409 lan_off` without LAN sharing; while on and consented it sets `ROLE_DATASETS` (§12.4) | `datasets_serve` |
| `origin.components` | `false` | same | a human on loopback; as above | `components_serve` |
| `origin.termux` | `false`: the "Termux install files" tick under `origin.components` (§11.5); not offered by a release whose `LICENSING.md` carries no verified licence for those pins, so off in v1 | same | a human on loopback | `components_serve` with addon `termux` |
| `origin.max_streams` | 4 (1..=16) | same | a human on loopback | — |
| the L5 class quotas | unset, as built; `quota.transit` written once at its effective value when the budget first sets the watermark (§4.2 item 4) | `fabric/store.db` (`PATCH /api/v1/fabric/settings`, `API:1132-1145`) | a human on loopback (that route is loopback-only, `4-interface/api/README.md:49`), or an accepted `quota.set` decided on loopback | — |
| `metered` | `[]` | `state/relay.json` (Layer 7, L7 §12) | a human | Layer 7's |
| `update_without_copy` | `[]`: `[{release, store, hash}]` | `state/storage.json` | a human in a dashboard session or on loopback, through its own route only (§13.6 (5)); `111d` forgets an entry once its release is confirmed or reverted | `update_without_copy` |

**Who writes them.** Every key `state/storage.json` holds is written through `PATCH /api/v1/storage/settings`, which is loopback-only (403
`loopback_only` otherwise), the posture of its Layer 5 sibling `PATCH /api/v1/fabric/settings` (`4-interface/api/README.md:49`), except
`update_without_copy`, which only `POST /api/v1/storage/update-without-copy` writes (§16.1); the adopt route of the dataset verbs (§7.6) is loopback-only
too. A paired session reads every setting (`GET /api/v1/storage`, `GET /api/v1/datasets`) and changes none but `update_without_copy`, through its own route. An
accepted `quota.set` (§16.4) applies through the route it names with that route's own `loopback_only` check: decided from a session that is not on
loopback it answers 403 `loopback_only`, nothing is applied and the proposal stays open.

**Inert by default.** With nothing configured, nothing a user wrote is removed before the user answered that class's export offer (§4.5; a class that
would need to is `full` instead), no file is served, nothing is fetched, the fabric's high watermark never drops below today's 512 MiB
(`M/store/prune.rs:131`) unless a human set `keep_bytes` (until then the budget only raises it), and Layer 7's Transit quota keeps the figure it had (§4.2
item 4). What does change by design (D34): the agent's state moves into `node/state.db` (§2) and the idle disk-figure commits stop (§13.4).

**15.2 Consent copy (verbatim; the pinned `ConsentRequired` round-trip, L7 §12).** **`dataset_fetch`** (subject `<publisher short>:<id>@<version>`; `addons`:
`metered`, `non_commercial` when they apply): "Download <name> <version>, <size>, from <source>. It is signed by <publisher label> (<publisher short>).
Licence: <licence><, non-commercial>. <attribution> Used by: <used by>. Meant for: <fits> and up. Connection: <connection> You can pause or cancel it at any
time." `<connection>` is "No metered link is set." or "'<match>' is marked metered with a daily cap of <cap>: this download pauses before the cap, leaving
room for messages, and goes on in the next day's allowance." The live figures are not in the text (§7.7): `facts` = `{free, free_after, days}` (`days`
when metered), shown under it as "Free space now: <free>; after this: <free after>. About <days> days on this link." · **`dataset_override`** (same
subject): "Continue downloading <name> past today's allowance on '<match>'. This uses more than the <cap> you set for this link, which may cost you money.
It applies to this download only; messages keep their own room." `facts` = `{remaining}`, shown as "Up to <remaining> more." · **`datasets_serve`**
(subject the hub's NodeId, short): "Any device on this network can download this hub's dataset files without signing in: which encyclopedias and maps it
holds, their versions and their contents. A map file shows the area it covers, which tells visitors where this community is. Every device that holds this
hub's card also learns that it serves datasets. Requests are not logged. When you share these files, their licences ask you to keep their notices; this
hub shows them on its Credits page." · **`components_serve`** (same subject): "Any device on this network can download the program files this hub cached
for itself (<list>)<, and the Termux install files (<size>), which this hub downloads first,> without signing in. When you share a program file, you are
the one who passes it on under its licence: this hub serves each file with its licence notice and where its source is, which is what those licences ask
of you. 111 and its author do not distribute these files." The bracketed part appears with addon `termux` (§11.5). · **`update_without_copy`** (subject `<release>:<store>`, §13.6 (5)):
"Install <release> without a rollback copy of <messages | the agent's history and repair records>. This node cannot keep a copy while it updates: there
is not enough free space, or the copy would not finish in time. If this release fails, <messages stay | the agent's history and repair records stay>
closed until the next update." `facts` = `{needed, free}`, shown as "Needed: <needed>; free now: <free>."
· **`store_start_empty`** (subject the store, `fabric` or `node_state`, §13.7): "Start an empty store for <messages | the agent's history and repair
records>. The damaged file stays on this node as <file> and is not read again; only you can delete it. What it held comes back only from an export."

Each consent records its text hash; a changed text asks again. The hashed text holds stable facts only; live figures ride `facts` (§16.2), unhashed. "You
won't be warned in future" is the pattern refused (R05 finding 6). "Paused (offline)", "paused (metered cap)" and "paused (metered link)" are states,
never errors.

**15.3 What the agent may propose, what only a human confirms (normative).**

| Action | The agent | A human |
|---|---|---|
| `keep`, a class cap, an L5 quota | proposes `quota.set {class, bytes \| null}` | accepts it, or edits the page, on loopback (§15.1 "Who writes them") |
| fetch a dataset | proposes `dataset.fetch {id, version}` | accepts, then accepts the consent dialog it opens |
| remove a dataset | proposes `dataset.remove {id, version}` | accepts |
| the metered override | never | a second explicit tap |
| a serving switch · following a publisher · publishing a catalogue | never | only |
| export · import | never (a CLI verb with the daemon stopped) | only |
| prune | never: the budget prunes by capacity | answers the export offer |
| start the encyclopedia | never; it says it is off and proposes nothing (no proposal kind enables a component, §9.4, §22 D24) | opens the panel |
| update without a rollback copy · start an empty store · delete a set-aside store file | never | only (§13.6 (5), §13.7; the last two on loopback) |

The bearer is refused on every non-GET route Layer 8 adds (D26 (2)); the agent never accepts a consent and never sees a consent hash.

**15.4 The Storage page (normative).** `/settings/l8`, "Storage", in two lazy chunks (the `l6` and `l7` precedent, `UI/src/views/settings/index.tsx:13-15`,
`:20-21`): `l8` (Disk, Space set aside, Datasets, Export, Credits) and `l8-hub` (Publishing and Serving), which the page imports only on a node with
`hub_role`, so a member never downloads the hub-only sections. It replaces the generic page whose rows say "arrives with Layer 8" (`UI/src/views/settings/model.ts:222`). Sections: **Disk:** the gauge ("12.4 GB free
of 29.8 GB"), the floor, `disk_low`, and a second gauge for `state/origin/` when it is its own filesystem (§5.1); the §4.7 copy. · **Space set aside:** `keep`
with its default and rule, and `keep.effective` when it differs (§4.2 item 3); the class table of §4.1 (layer, bytes, cap, `full`), with the node-state
store's file size beside the class figures and the spool, `agent/work/` and `agent/legacy-v0/` measured on request; caps editable on loopback and
read-only in a paired session (§15.1 "Who writes them"); the export offers (§4.5);
the scaled Transit figure as a suggestion (§4.2 item 4); a store being checked (§13.7); damaged, rolled-back and replaced store files (§13.2, §13.6, §13.7),
each with Delete, and Start an empty store for a damaged one (both on loopback, read-only in a paired session); an update refused for space with "Update
without a rollback copy" (§13.6 (5)); and unreadable agent files (§2.4). · **Datasets:** the home hub's
and the followed publishers' catalogues (newest `seq` and its source, §6.3); held entries with a `used_by` chip each (`map`, `encyclopedia`, `agent`,
`updates`), so "what do I lose" is answered before a delete (R05 finding 10); Get, Refresh, Remove; jobs with their progress, their source and their
pauses; "N entries need a newer 111" when a catalogue held entries of an unknown `kind` (§5.4); the followed publishers and each one's `sources` (§7.4),
editable on loopback; with no catalogue, an empty state that offers Check now (§12.4); on a node that holds a ZIM and has the kiwix component enabled, "Open
encyclopedia" (the kiwix panel, §9.1 (5)) and, while kiwix-serve runs, Stop (§9.3). · **Publishing** and **Serving** (hubs only): drafts and Publish;
`origin.datasets`, `origin.components` and its "Termux install files" tick (§11.5), the stream cap. · **Export:** the §13.2 command lines. **Credits:** §9.7.

Every figure is computed on request (`GET /api/v1/storage`); the page refetches on a `storage` frame. **The lazy budget.** After Layer 7's row U2 the
lazy bucket holds 238.9 of 256 KB (`7-relay/TASKS.md:89`; the `l7` chunk is 11.9 KB gzip), not the 219.9 KB R05 finding 5 read, so 17.1 KB remain for
everything later layers add: the `l8` and `l8-hub` chunks, the storage adapter (§16.6 (8)), the `storage` proposal card and Layer 9's page. Layer 8's
allowance is 12 KB gzip for its chunks, its adapter and its card together (default; measured at build), restated against the bucket at the lock head;
the gate asserts it and the lazy total (G17). If Layer 8's part cannot fit, raising D21's lazy budget is the owner's (§22 D22): this spec assumes no raise.

## 16. The contract (additive; the binding form is `API` at build time)

**16.1 Routes.**

| Route | Method | Shape | Who |
|---|---|---|---|
| `/api/v1/storage` | GET | `StorageStatus` (§16.2) | session or loopback; the agent's bearer reads |
| `/api/v1/storage/settings` | PATCH | a partial `StorageSettings` (§15.1) plus `accept_warning?` (the consent round-trip); `origin.*` answers 409 `consent_required` (`datasets_serve`, `components_serve`) until the repeat carries the hash, `needs_hub`, `lan_off` | a human on loopback only (403 `loopback_only` otherwise, the `PATCH /fabric/settings` posture, §15.1); bearer 401 |
| `/api/v1/storage/export-offer` | POST | `{class: "fabric" \| "agent" \| "heal"}` → 204 (§4.5) | a human; bearer 401 |
| `/api/v1/datasets` | GET | `{schema, publishers: [Publisher], holdings: [Holding], jobs: [Job]}` | session or loopback; bearer reads |
| `/api/v1/datasets/check` | POST | 202; one GET per source and catalogue name: the home hub's and every followed publisher's catalogue (§6.2, §12.4), `If-None-Match`, counted in the metered window | a human |
| `/api/v1/datasets/entries/{publisher}/{id}/{version}/fetch` | POST | `{accept_warning?, accept_override?}` → 202 `Job` (queued behind a running job); 409 `consent_required` (`dataset_fetch`; `dataset_override` at a cap), `insufficient_space`, `untrusted_publisher`, `job_running` (only for an entry that already has a job, §7.4) | a human; bearer 401 |
| `/api/v1/datasets/entries/{publisher}/{id}/{version}` | DELETE | → 204 (§7.9); entries live under `entries/`, keyed as a holding is (§5.5), so no `id` meets a literal sibling (`check`, `drafts`, `publish`, `search`, `jobs`, `adopt`) | a human; bearer 401 |
| `/api/v1/datasets/jobs/{job}` · `/resume` | DELETE · POST | cancel · resume a paused job; `{accept_warning?, accept_override?}` on resume: a job `paused (metered link)` answers 409 `consent_required` with the text naming the new rule until the repeat carries its hash, and a job `paused (metered cap)` answers 409 `consent_required` (`dataset_override`) until the repeat carries `accept_override` (§7.4, §7.8) | a human; bearer 401 |
| `/api/v1/datasets/drafts` | GET · POST | `{schema, drafts: [{draft, entry}]}` (`draft` the `<id>@<version>` name `publish` takes, `entry` a `DatasetEntry`) · a first-copy job (§7.5; consent `dataset_fetch`) → 202 `Job`; 409 `needs_hub` | session or loopback, bearer reads · a human, bearer 401 |
| `/api/v1/datasets/publish` | POST | `{drafts: [string], remove: [{id, version}]}` → 200 `{seq}` (§6.4); 409 `needs_hub` | loopback or a paired session, a human |
| `/api/v1/datasets/search` | GET | §9.4 (`q`, `book?`, `publisher?` defaulting to the home hub, `n?`); 409 `component_unavailable` | session or loopback; bearer reads |
| `/api/v1/tiles` | GET | `{schema, sets: [{id, publisher, version, name, bytes, licence, attribution, url}]}` (`url` the path below) | session or loopback; bearer reads (every GET, `4-interface/api/README.md:87`) |
| `/api/v1/tiles/{publisher}/{id}/{version}` | GET, HEAD | the file through the Range arm (206; 416 with a `problem+json` body, code `range_not_satisfiable`, §7.1), `application/octet-stream` | session or loopback; bearer reads |
| `/api/v1/chat/history` | GET | unchanged shape, answered by `111d` (§2.3); 503 `store_unavailable` | as today |
| `/api/v1/proposals` · `/api/v1/proposals/{id}` · `/api/v1/proposals/{id}/decision` | GET · GET · POST | unchanged shapes, answered from the store after a spool sweep (§2.4 rule 2); the decision route answers 503 `store_unavailable` and applies nothing while the store is unavailable | as today |
| `/api/v1/datasets/adopt` | POST | `{request: "adopt-<n>.json", incoming: [sha256], catalogues: [publisher hex], drafts: [DatasetEntry], in_place: [{sha256, path}], follow?: [publisher hex], sources?: {publisher hex: [origin URL]}}` → 204: each staged catalogue's signature and `seq` checked again and installed, the drafts written, each in-place path `stat`ed again against the tuple the request file recorded and indexed with its path and tuple, the verified copies in `incoming/` renamed into the tree and indexed, `follow` and `sources` written, the library rebuilt, the request file deleted (§7.6); any item that fails: 400 `bad_request` naming it, 409 `untrusted_publisher` or `needs_hub`, and nothing written | loopback only; never a session, never the bearer |
| `/api/v1/bridge/components/{id}/stop` | POST | 204: stops a running `on_demand` component (kiwix-serve, §9.3); its files and its enable switch stay; 400 `bad_request` for a component that is not `on_demand` | a human; bearer 401 |
| `/api/v1/panels/{id}/session` | POST | unchanged shape; `{id}` is a `ServiceId` or, for a `child` panel, its `ComponentId` (§9.1 (5)); for kiwix the first mint starts kiwix-serve (§9.3 (2)) | as today |
| `/api/v1/storage/update-without-copy` | POST | `{release, store: "fabric" \| "node_state", accept_warning?}` → 409 `consent_required` (`update_without_copy`, subject `<release>:<store>`) until the repeat carries the hash, then 204: the consent recorded in `state/storage.json` for that release and store, read by the updater at its next check (§13.6 (5)); 400 `bad_request` for a release and store that no `update_refused_space` notice names | a human in a dashboard session or on loopback; bearer 401 |
| `/api/v1/storage/set-aside/{file}` | DELETE | → 204: deletes one set-aside store file a `set_aside_store` notice names (`.rolled-back-v<N>`, `.damaged-<n>`, `.replaced-<n>`, with its `-wal` and `-shm`); 404 for any other name; the blob sweep resumes once no set-aside fabric store remains (§2.7 S3) | a human on loopback only (403 `loopback_only` otherwise); never the bearer |
| `/api/v1/storage/start-empty` | POST | `{store: "fabric" \| "node_state", accept_warning?}` → 409 `consent_required` (`store_start_empty`, subject the store) until the repeat carries the hash, then 204: the store's closed state lifted, the next open creates it empty, the damaged file kept (§13.7); 400 `bad_request` for a store that is not closed after a set-aside | a human on loopback only (403 `loopback_only` otherwise); never the bearer |

**16.2 Shapes.**

```json
{"schema": 1,
 "disk": {"free": 12400000000, "total": 29800000000, "floor": 298000000, "disk_low": false, "origin": null},
 "keep": {"bytes": 2147483648, "default": 2147483648, "effective": 2147483648, "rule": "linux"},
 "node_state": {"state": "closed", "reason": null, "file_bytes": 4194304},
 "integrity": {"fabric": null, "node_state": null},
 "classes": [{"id": "fabric", "layer": 5, "bytes": 81234567, "cap": 1600000000, "budgeted": true, "export_offer_needed": false, "full": false},
             {"id": "datasets", "layer": 8, "bytes": 7400028224, "cap": null, "budgeted": false, "export_offer_needed": false, "full": false}],
 "notices": [{"kind": "unreadable_agent_files", "count": 0}]}
```

`node_state.state` ∈ `open`, `closed` (idle), `unavailable` (with `reason`); `node_state.file_bytes` is `state.db`'s size from one `stat`.
`disk.origin` is `null`, or `{free, total, floor, disk_low}` of `state/origin/` when it is another filesystem than the root's (§5.1). `keep.effective` is the
sum of the caps in force (§4.2 item 3); `keep.rule` ∈ `termux`, `hub`, `linux`, `user`. `integrity.fabric` and `integrity.node_state` ∈ `null` (no check
due), `checking`, `ok`, `failed` (§13.7). `classes[].full` (§4.5) is false for every class but `agent` and `heal`. `notices[].kind` gains
`update_refused_space` (`{kind, release, store, needed_bytes}`, §13.6 (5)) and `set_aside_store` (`{kind, file, bytes}`: a `.rolled-back-v<N>`,
`.damaged-<n>` or `.replaced-<n>` file). · `Publisher {node_id, label,
home, seq, source, sources, skipped, entries: [DatasetEntry]}` (`sources` the origin URLs `sources` lists for it, §7.4; `skipped` the entries of an
unknown `kind`, §5.4); `DatasetEntry` = the §5.3 fields plus `state`, `location` and `non_commercial`. **One state enum** for `DatasetEntry.state` and
`Holding.state`: `not_held` (listed, not held; an entry only), `partial`, `present`, `unverified`, `disk_absent` (an in-place file whose disk is not
attached) (§5.5); `location` ∈ `origin`, `in_place` on both, `null` on an entry that is `not_held`. `Holding {publisher, id, version, sha256, bytes, location, state, used_by}`. `Job {id, publisher, entry, version, state, done_bytes,
total_bytes, source, metered: {match, cap, days} | null, override_bytes}` (`id` a new id family, `job-…`, minted by `111d`, which the conventions' IDs line gains, `4-interface/api/README.md:221`; states §7.4, `paused (metered link)` and `failed (source cannot resume)` among
them; `override_bytes` the bytes moved past a cap under `dataset_override`, §7.8). · `FabricStatus` gains an optional `unconsumed {agent_op, kb, update,
patch, sync}` (§12.3). `Proposal.kind` gains `storage` (§16.4). `Proposal.decision` (`API:852`, `{decision, at}`) gains an optional `applied` member
(additive): the record's `applied` as the store holds it, `{pending: true}` while an apply runs, then the result (§2.4 rule 5), so a harness reads a
half-applied decision through `GET /api/v1/proposals/{id}` (§18). `Status.update.last_result` (`API:749`) gains `refused_space` (§13.6 (5)), appended.
`ConsentRequired.action` (`API:1497`) gains `dataset_fetch`, `dataset_override`, `datasets_serve`, `components_serve`, `update_without_copy` (§13.6
(5)), `store_start_empty` (§13.7), appended. `ConsentRequired` gains an optional `facts` object (additive; absent for
every earlier action): the live figures shown under `warning_text` and never part of `hash` (§7.7): `{free, free_after, days?}` for `dataset_fetch`,
`{remaining}` for `dataset_override`, `{needed, free}` for `update_without_copy`. `StorageSettings` (§15.1) carries `sources` and `origin.termux`. `Component.adapter` becomes nullable and stays in
`Component.required`, typed `NetworkId` or null (null = a server component, §9.2), so the key never vanishes from an answer
(`4-interface/api/README.md:17`); `Component` gains `start` (`always` | `on_demand`). `Problem.code` (`API:692`) gains `insufficient_space`, `untrusted_publisher`, `job_running`, `store_unavailable`, `range_not_satisfiable` (416, §7.1), appended. ·
**Publisher strings.** `Publisher.label` and an entry's `name`, `version`, `licence`, `attribution`, `source_url`, `built` and `book` are the publisher's: untrusted,
rendered as text only (never Markdown or HTML), the conventions' rule for names and labels (`4-interface/api/README.md:160-161`), and so are the
same strings wherever they appear inside a consent's `warning_text` (§15.2).

**16.3 Events.** `storage` → `SseStorage` on a change of `disk_low`, `store_full`, a class's `full`, `node_state.state` to or from `unavailable` (or its
`reason`), or `integrity`, or after a reclaim; this is the one list of what sends it (§4.3 step 8 cites it), and an open or a close of the node-state store
on use (`open` ↔ `closed`) sends none, so opening the store for a chat turn sends no frame. The frame carries the
fields the pass already holds, `{schema, disk, keep, node_state, integrity, classes, notices}` with `classes` the four budgeted ones (§4.1), never a figure
that needs a walk (`updates`, `bridge`, `kb`) or a read of the index or the registry (`datasets`, `components`); the page refetches `GET /api/v1/storage`
on the frame for the rest (§4.3 step 2: the pass walks nothing). `dataset` → `SseDataset` (a `Job`), coalesced per job (§7.4): at most one per 5 s
housekeeping tick while it runs, plus one on each state change and at its end, so a LAN job at tens of blocks a second cannot flood the 256-entry ring
(`4-interface/api/README.md:20`, `:27-29`). Neither is sent on a timer. After `reset` the Layer 8 views refetch `/storage`
and `/datasets`.

**16.4 Proposal kind `storage`.** The body is one JSON object `{"why": string, "actions": [Action]}` from the closed vocabulary `quota.set {class, bytes or
null}` (class: `keep`, `agent`, `heal`, or an L5 refusable class) · `dataset.remove {publisher?, id, version}` · `dataset.fetch {publisher?, id, version}`
(`publisher` defaults to the home hub). Accepting it runs the routes' own validation, consent and refusal paths; a consent gate answers 409 `consent_required`
and the human repeats the decision with `accept_warning`. A `quota.set` applies through `PATCH /api/v1/storage/settings` or `PATCH /api/v1/fabric/settings`
with the route's own `loopback_only` check: decided from a session that is not on loopback it answers 403 `loopback_only`, nothing is applied and the
proposal stays open (§15.1). Any other action is `unsupported_action`; a body of another shape is `502 upstream_error`. It
renders as a card from the parsed JSON, never as Markdown.

**16.5 Agent tools (the Layer 3 set grows; fixed, shell-free, schema-validated).**

| Tool | Kind | Shape | Gate |
|---|---|---|---|
| `storage.read` | read | `{datasets?}` → one GET: `StorageStatus` (`GET /api/v1/storage`), or with `datasets: true` the `/api/v1/datasets` listing (no secret, no consent hash in either); one call per use, the agent's rule (`3-intelligence/README.md:119-122`) | none |
| `encyclopedia.search` | read | `{query, book?, publisher?, n?}` → §9.4 (`publisher` defaults to the home hub), inside a labelled untrusted block | none |
| `storage.propose` | write intent | `{actions, why}` → a proposal `kind: storage` | a human accepts |

The count grows by three from the count the build finds: 20 at the pinned head, Layer 7's three tools of its §13.3 included (row A1;
`A/tests/daemon_tools.rs:322`, `A/tests/chat_loop.rs:308`, `A/tests/layer6_tools.rs:395`, `A/tests/layer7_tools.rs:315`), 23 after this. The three
schemas are appended after Layer 7's, the A1 precedent (the list ends with `relay.propose`, `A/src/tools.rs:352`; dispatch beside `:403`), so
`names[17..20]` stays Layer 7's and Layer 8's are `names[20..]`. `proposal.write` takes only a title and a body
(`A/src/tools.rs:113-121`), so a typed proposal needs its own tool, the `device.propose` pattern (`:281`; §22 D1). The agent reads freely and writes nothing
(`3-intelligence/README.md:119-122`).

**16.6 The pin list (the conventions' form, `4-interface/api/README.md:127`; the change is not done until all are green).** (1) `Proposal.kind` +1
(`storage`): `KINDS` in `N/dashboard/api_v1/proposals.rs:104-114` (nine at the pinned head), the UI's proposal card, the agent's writer. · (2) The agent
tool count +3, 20 → 23: the four count pins of §16.5, a `LAYER8_TOOLS` constant beside `LAYER7_TOOLS` (`A/tests/daemon_tools.rs:301`,
`A/tests/layer7_tools.rs:45`; `DAEMON_TOOLS` unchanged, as A1 left it), the two `&names[17..]` pins (`A/tests/daemon_tools.rs:325`, `A/tests/layer7_tools.rs:316`) become `[17..20]`, `A/src/tools.rs` schemas and dispatch. · (3) `pnpm gen:types`, then `node scripts/gen-sse.mjs --check`; the new enum
tuples and aliases (`StorageClass.id`, `Problem.code`, `ConsentRequired.action`, `Proposal.kind`, `Job.state`, the two events) are listed in
`UI/scripts/gen-sse.mjs`. · (4) `x-sse-events.events` (`API:2044-2059`) gains `storage` and `dataset`; the UI store's `switch (frame.event)` handles them. ·
(5) The contract tests: `UI/test/gen.test.ts`, `UI/test/http.test.ts`, a new `UI/test/layer8-contract.test.ts` (the `layer7-contract.test.ts` precedent), and
the mock adapters, whose frames validate against `x-sse-events`. · (6) node111's mirrors in `dashboard/api_v1/`: the `Problem` codes, the `ConsentRequired`
actions, the class ids and the job states. · (7) The panel and component contract (§9.1 (5), §9.2, §9.3): `panelId` (`API:674`) becomes `oneOf`
`ServiceId` | `ComponentId`, with node111's panel manifest parser (`N/panels/manifest.rs:387`) and the panel host's enabled rule (`N/panels/mod.rs:22-27`)
following it; `Component.adapter` typed `NetworkId` or null and kept in `Component.required` (`API:1304`, `:1308`), the parser's `adapter` optional
(`N/components/manifest.rs:685-686`) and the UI row's label falling back to `title` (`UI/src/views/settings/l6/components.tsx:52`, `:63`); `[run] start`,
`health = "listen"` and the per-target `program` in the parser and its tests; the new route `POST /api/v1/bridge/components/{id}/stop` in `API`, its
client method in the UI adapter and its mock; `ConsentRequired.facts`; a node111 unit test parsing the embedded `kiwix.toml` and `panels/kiwix.toml`. ·
(8) The UI adapter seam: `getStorage()` beside `getRelay()` in `UI/src/lib/api/index.ts` (`:45-54`, the seam that keeps Layer 7's code out of the eager
chunk), importing `./storage` dynamically, only when a Layer 8 view or the store (after a `storage` or `dataset` frame) asks; the mock's storage is reached
through the mock adapter inside the same `import.meta.env.DEV` guard; `resetAdapterForTests` forgets it. Its size counts in Layer 8's lazy allowance
(§15.4). · (9) The conventions (`4-interface/api/README.md`): the IDs line (`:221`) gains `job-…` minted by `111d`; the loopback-only writes gain
`PATCH /storage/settings`, `POST /datasets/adopt`, `DELETE /storage/set-aside/{file}` and `POST /storage/start-empty` (§15.1, §16.1); the Layer 8
additions name the `storage` frame's fields and the `dataset` coalescing (§16.3). · (10) The v0.3 additions: `Proposal.decision.applied` (optional, with
node111's proposals mirror and the UI's decision view), `Status.update.last_result` `refused_space`, `ConsentRequired.action` `store_start_empty`, the one
`DatasetEntry.state` / `Holding.state` enum and `DatasetEntry.location` (listed in `UI/scripts/gen-sse.mjs` with the tuples of (3)), the three routes
`POST /storage/update-without-copy`, `DELETE /storage/set-aside/{file}` and `POST /storage/start-empty` in `API` with their client methods in the storage
adapter and its mock, and the search route's and the tool's `publisher`.

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
unverified byte.** A blob is served only when a pin list the hub holds names it (§11.1) and its bytes passed: sha256 at import, publish and the end of
every pull (a sidecar's before its rename), BLAKE3 per block on a pull; an out-of-tree file only while its tuple matches (§5.5); an existing
`blob/<sha256>` is never replaced. · (4) **The release key never signs a dataset;** the node key signs a catalogue only on a human action, under its own tag. · (5) **Never fetch without
the consent** that names size, licence, signer, `used_by` and connection, with the free space shown beside it (§7.7); never past a metered cap without the override; never on a carrier or
LoRa. · (6) **kiwix-serve is never LAN-bound;** it is reached only through its panel, the hub's session and its own origin. · (7) **No wall clock** orders,
freshens or prunes anything: `seq`, the boot id and `CLOCK_BOOTTIME` do. · (8) **No telemetry:** requests are never logged; nothing about a node leaves it
except what a hub serves under a recorded consent. · (9) **Export never over HTTP;** restore verifies the signature before decrypting. · (10) **Encryption is
described honestly** (§2.1): copies and backups, not a stolen disk with a readable `identity/`, not a same-uid process.

**What is exposed, to whom:**

| Path | On the path | At the far end |
|---|---|---|
| a member's catalogue check or dataset pull (LAN, plain HTTP) | IPs, timing, sizes, the hashes asked for | the hub: which member pulled which file, when (never logged) |
| a catalogue check (§6.2, §12.4) | IPs, timing, the catalogue names asked for | each source a check asks: the home hub's id and every publisher this node follows |
| `origin.datasets` | — | anyone on the LAN: the holdings, versions, contents, a map's area; every holder of the hub's card: that it serves datasets (`ROLE_DATASETS`, §12.4) |
| `origin.components` | — | anyone on the LAN: which programs the hub cached |
| the kiwix panel, the map tab | as the dashboard | the hub's session holder only |
| a first copy from upstream | the hub's IP and the file to the network | the mirror's operator: the hub's IP, the file, the time |
| an export file | — | whoever holds it: its size; a holder who guesses the NodeId can check the signature |

**Attacks and defences.** **A hostile mirror or hub serves wrong bytes:** the block fails `valid_ranges` and is fetched from the next source; a whole file is
never accepted on its name. · **A swapped sidecar:** `valid_outboard_ranges` fails against the signed root before any data byte moves. · **A forged or foreign
catalogue:** the signature fails under the pinned key, or the publisher is not followed and nothing is listed. · **A replayed old catalogue (freeze):** a
lower `seq` is refused; a stale mirror can only withhold, until the node reaches another source (§6.3). · **A publisher that signs a sha256 that does not
match its BLAKE3** (to place its bytes under a release pin's name or another publisher's file in the shared tree): the pull's sha256 check after the last
block fails, the job ends `failed (name mismatch)` and the `.part` is deleted; no byte enters `blob/` under a name it does not hash to, and a file already
held under that name is never replaced, so no other pin source and no other publisher's followers are affected (§7.3 (3)(e)). The same holds for a signed
`outboard_sha256` that does not hash the sidecar: its sha256 is proven before its rename and the job ends `failed (name mismatch)` before any data byte
(§7.3 (3)(a)). · **A ZIM's JavaScript:** it runs in the panel's own origin (L5 §13), never the dashboard's; ZIMs come only from catalogues
the user follows. kiwix-serve runs with `--blockexternal` (its links to the internet are refused) and a `child` panel's answers carry `connect-src 'self'`
(§9.1 (3)), so a page's scripts cannot send to the internet from the reader's browser. The residual, stated: a ZIM page's own markup may still name an
image, a stylesheet or a frame on the internet, which an online reader's browser loads (no `img-src`, `style-src` or `frame-src` of ours is added, since a
ZIM's pages use their own); the hub does not see or log it. · **Bulk abuse on the LAN** (a 57 GB route hammered): four streams at most, single ranges only, 64 KiB streaming, no multipart body, a body that accepts no byte for 60 s closed (§7.2). ·
**Path tricks in a catalogue:** an `id` and a `version` each match a fixed pattern (§5.3; no `/`, no control character, no leading dot) and are never
paths, though both name files and URL path segments; a catalogue with another is refused whole; blobs are named by hash. · **Filling the disk through fetches:** the
free-space check before any byte and the floor (§4.2). · **A compromised agent:** it reads what GETs return; it cannot write, fetch, serve, prune or accept.
The same-uid residual is stated (§2.1). · **A tampered export:** the signature over the BLAKE3 prehash fails before any decryption. · **A store swapped at
rollback:** the guard renames only a `.pre-v<N>` copy that release N completed and renamed (never a `.tmp`), same owner, same key; the moved-aside file is
kept; the restored store carries no clean record, so the node mints a new epoch and never reuses a sequence number (§13.6 (3)). · **A second writer:** the
daemon holds `node/lock` for its lifetime and the store verbs refuse while it is held; the dataset verbs hand their results to the daemon (§7.6, §13.2).

**Legal (facts from the texts; the readings are the owner's).** Serving a ZIM is "Sharing" under CC BY-SA 4.0 s.3(a); a repack or subset is arguably Adapted
Material (unverified legal read). Serving OSM-derived tiles to a neighbourhood is publicly conveying under ODbL s.1 and s.4.2, and a publicly used Produced
Work needs the s.4.3 notice (R01 finding 12). A hub that mirrors a third-party binary is the one who conveys it under GPL-3.0 §6(d), GPL-2.0 §3, MPL-2.0
§3.2(a), AGPL-3.0 §6, Apache-2.0 §4 and the Reticulum License's notice clause (R07g2 §3.4). Non-commercial content may be listed and served, tagged, never
preselected; 111 refuses nothing (Q11).

## 18. Verification (lock gates; netns, no hardware)

**Gate homes.** New `8-storage/verify/{state,budget,datasets,kiwix,maps,components}.sh`, a stdlib-only stub mirror `8-storage/verify/stub-mirror.py` (serves a
file and, on request, one corrupted block, a 200 to a range, a body that stalls mid-block, or 503 to every request; it logs each request's range), a
stdlib-only stub child `8-storage/verify/stub-child.py` (a plain-HTTP server on the declared loopback port that echoes the headers it received, answers a
WebSocket upgrade with 101 if one ever reaches it, and logs each request; G16), a stdlib-only `8-storage/verify/gen-pmtiles.py` (writes a small PMTiles v3
archive of a few vector tiles the script draws, no third-party data, into the harness's scratch root, never into the tree or a payload; G17), and
`8-storage/verify/lib.sh`, which sources `6-transport/verify/lib.sh` and `6-transport/verify/root.sh` as
`7-relay/verify/lib.sh:14-19` does and names the netns, veth, address and port names (names only). The conventions are `6-transport/verify/lib.sh:1-29`: each
check prints `ok:` or `FAIL:` (`note:` informs); a gate whose prerequisites are absent (unprivileged netns through `unshare -Urnm`, a built binary,
`KIWIX_BIN`, a fixture) prints `<GATE> SKIPPED: <reason>` and exits 0 unless its `REQUIRE_<GATE>=1` (a lock run) makes it a FAIL; the last line is `<GATE>
GREEN` or `<GATE>: N FAIL`. Every harness stops what it started, leaves nothing listening, and runs alone (AGENTS.md, Linux hygiene). **The pass knob.**
Every gate item that waits on capacity passes (among them G13 (8), G14 (5), (9) and (11), G15 (13) and G16's idle stop) runs a debug `111 daemon` whose housekeeping interval a
build-only test setting shortens, `NODE111_TEST_HOUSEKEEPING_MS`, read by a debug build only (the `NODE111_BRIDGE_STUBS` and `NODE111_STUB_CHILD`
pattern; a release build has no such setting, so it adds no runtime weight); the pass still runs every `PRUNE_EVERY` = 12 ticks.

**Seam ids (D36 (2), D37).** The ids are the cross-layer map's: Layer 8's planned rows (`plan/cross-layer/test-system/matrix.md:177-178`, entering
`verify/seams.toml` under D37 F5) plus three new rows, `L6-L8:server-component`, `L5-L8:consumer-registry` and `L5-L8:child-panel`, so a FAIL in the consumer registry or the panel proxy
points at Layer 5's dispatch and panel host, not at Tier 2 or the dashboard. Layer 8's board carries the verify row that registers these gates in
`verify/checks.toml` and promotes the seams to built, or names a gap (D37 F5). Each `ok:` and `FAIL:` line starts with its seam id in brackets: the phase
prefix `checks.toml` maps to a seam (D37 §3.5), so the runner needs no adapter of its own for these gates.

| Seam id | Layers | What Layer 8's gates prove there | Gates |
|---|---|---|---|
| `L3-L8:agent-state` | 3 ↔ 8, through `111d`'s relay | the chat body's `history` and the `done` frame's `messages`, the history route, the spool and its events, the legacy import | G13 |
| `L1-L8:state-service` | 1 ↔ 8, on 5's store code | the node-state store's key, open and modes; the budget on the housekeeping pass, `set_watermarks`, no idle write, the log ring; datasets never pruned by the budget, the index's crash order | G13, G14, G15, G7 |
| `L8-L9:heal-store` | 8 ↔ 9 | the J2 files, the J3 marker, the J9 API (a fake Layer 9 writer until Layer 9 builds) | G13 |
| `L5-L8:tier2-objects` | 5 ↔ 8 | nothing in v1: the gap text "Tier 2 later (§8)"; the sidecar's checks are G15's under `L2-L8:dataset-signing` and `L7-L8:move-datasets` | none |
| `L2-L8:dataset-signing` | 2 ↔ 8 | `111/catalogue/v1`, `111/export/v1`, the `111/node-state/v1` vector | G13, G15 |
| `L7-L8:move-datasets` | 7 ↔ 8, and 1's fetcher | `/origin/v1/*`, the admission rule and the switches, the Range arm, the stream cap, `get_to_file`, the consent round-trip, the metered window | G15 |
| `L4-L8:hub-content` | 4 ↔ 8 | `/api/v1/storage*`, `/datasets*`, `/tiles*`, the two frames, `/settings/l8` and its lazy allowance, the `map` bucket, `encyclopedia.search` | G14, G15, G16, G17 |
| `L1-L8:updates-backup` | 1 ↔ 8, and 6's cache | the migration copy and the guard's restore, `111 store export` and `import`, the installer's origin list, the files table, `origin.components` | G13, G18 |
| `L6-L8:server-component` (new) | 6 ↔ 8 | a declaration with no `adapter`, `start = "on_demand"`, `health = "listen"`, a per-target `program`, the stop route, zero weight while off | G16, G7 |
| `L8-L8:shared-log` | 8 | nothing in v1: the gap text "no log in v1 (§12)" | none |
| `L5-L8:consumer-registry` (new) | 5 ↔ 8, and 9's consumers | the dispatch hook for 0x10-0x14 (§12.3): no message row, the consumer's durable write before the fabric's commit and acknowledgement, redelivery after a crash between them; `mesh111` `tests/consumers.rs` | G14 |
| `L5-L8:child-panel` (new) | 5 ↔ 8 | L5 §13's panel host with a `child` source (§9.1): the ticket and cookie, the stripped cookies, the appended `connect-src 'self'`, the refused WebSocket, the panel keyed by a component id, start on mint | G16 |

**G13, the node-state store and the joint contract** (`state.sh`: one netns node with the real `111d`, the real `111-agent` and a stub model, the Layer 4
smoke pattern; plus node111, mesh111 and agent111 tests). Asserts: (1) `[L1-L8:state-service]` `node/` is 0700 and `state.db` 0600;
`[L2-L8:dataset-signing]` the file opens only with the `111/node-state/v1` key (a known-answer vector beside `store-kdf.json`, reached through `KeyProvider::key_for`; `store_key()` equals
`key_for(STORE_KDF_TAG)` byte for byte); a plaintext SQLite is refused; a wrong key fails on page 1. · (2)
`[L3-L8:agent-state]` A turn through `POST /api/v1/chat` writes no `agent/conversations/*.jsonl`; the store holds its messages; the agent receives at most
512 KiB of `history`; no `message` frame reaches the browser and its `done` frame carries no `messages`; a `stream: false` answer reaches the browser
without `messages` and is saved the same way; `GET /api/v1/chat/history` returns them with the agent stopped; a sentence of the turn is found in no file under the root after the
store closes (a byte search), logs excepted. · (3) `[L3-L8:agent-state]` A browser that leaves mid-turn: the turn is saved when the agent's `done` arrives. ·
(4) `[L3-L8:agent-state]` A proposal is ingested on its event (the file is gone, the record is there); with `111d` killed between the agent's write and the
ingest, the start-up sweep ingests it; a temp file is never ingested; a file that does not parse stays and is counted; a running task's file stays; a
failed and an orphaned task are ingested on `task.failed` and `task.orphaned` and their files are gone; a proposal written while the relay's subscription
was down, its event missed across a reconnect, is ingested at the resubscription and listed by `GET /api/v1/proposals`, whatever its timestamp; a
proposal file seeded with no agent running is ingested by the sweep before `GET /api/v1/proposals/{id}` answers and is returned by it; a
`.tmp-` file is never ingested and the agent removes it at its next start. · (5) `[L3-L8:agent-state]` The legacy import: a root seeded with v0 agent files
ends with the files moved to `agent/legacy-v0/` and the records in the store; release N adds a turn to an imported conversation; then, on each revert
path in turn (the guard's `revert_to` after an expired probation, and the root helper's revert followed by N−1's guard at the next `ExecStartPre`), with
no `.pre-v<N>` copy of any store, the files are back in place (the legacy files' own trigger, §13.6 (3)) and release N−1 lists the same conversations
and the undecided proposal; N−1 appends turns to that conversation's file; a second update merges it: the head's `legacy_lines` count is raised and
exactly N−1's new lines are appended after N's turn, none lost and none taken twice; a confirmed release deletes `agent/legacy-v0/`. · (6) `[L3-L8:agent-state]`
Every non-GET route Layer 8 adds answers 401 to the agent's bearer. · (7) `[L8-L9:heal-store]` With a fake Layer 9 writer (node111 tests): the J2 files are
folded and cleared at start; with the store unopenable they stay and the status names the reason, and after 300 daemon exits the inbox holds 256 lines; a
reclaim never touches `open`, `armed` or `pending` records and leaves one marker; `list` and `read_stream` return at most `limit` rows after the cursor; a
`put` on an existing record takes a new `seq`; `trim_stream` and `delete_stream` remove what they name; a reclaim of `heal` takes a stream from its oldest
`seq` only after the closed incidents; a root seeded with a J7 ring `state/heal/journal.jsonl` ends its first start with every line's record in `heal/`
(a second start adds none) and the file at `state/heal/legacy-v0.jsonl` (§2.5). · (8) `[L1-L8:updates-backup]` (node111 tests on a scratch root, the L7 I14 precedent, through the real guard)
Opening a store one version behind writes `<file>.pre-v<N>.tmp`, then `<file>.pre-v<N>`, before the migration; a copy cut short (the process killed
mid-copy) leaves only a `.tmp`, which the guard deletes and never renames in; both revert paths restore the copy and keep `.rolled-back-v<N>`: the
guard's `revert_to` (an expired probation) and the root helper's revert followed by N−1's guard at the next `ExecStartPre` (a READY-but-unhealthy release);
the restored fabric store's first open returns `EpochDecision::Mint`, and a message sent after it reaches its receiver (never dropped as already
delivered); the blob files that arrived during probation are still on disk; a confirmed probation deletes the copy; a daemon whose build writes a newer
node-state version than `state.db` holds takes the node-state copy and migrates at its first start, before it reports ready (`READY=1` follows the
migration), with no chat turn or other use, and closes the store on the next idle pass. · (9) `[L1-L8:updates-backup]` `111
store export` and `import` refuse while the daemon holds `node/lock`; a `111d` started while `111 store export` holds the lock exits with status 75 and
the message "a store verb is running" and is started again by the unit once the verb ends; with it stopped, an export taken after a `kill -9` (commits still in `-wal`) holds
those commits and the fabric's attachment files; one flipped byte makes `111 store import` refuse before decrypting; import refuses a store newer than the
build and sets an existing store aside as `<file>.replaced-<n>`; a round trip restores both stores, they pass `quick_check`, and the imported fabric store's
first open returns `EpochDecision::Mint`, through the real import. · (10) `[L1-L8:state-service]` After `kill -9` the next open runs `quick_check` and
`cipher_integrity_check` once, on a second read-only connection while the store already serves (`integrity` reads `checking`, then `ok`); after a clean
stop it runs neither; a corrupted page makes them fail, and the store is set aside as `.damaged-<n>` and closed; for the node-state store a non-empty
`state.db-wal` is the signal and a clean close leaves none; `POST /api/v1/storage/start-empty {store}` answers 409 `consent_required`
(`store_start_empty`) until the repeat carries the hash, then the next open creates an empty store and the `.damaged-<n>` file is still there; while a
set-aside fabric store exists the open's orphan sweep removes no blob file, and after `DELETE /api/v1/storage/set-aside/{file}` the next open's sweep
removes the orphans; both routes answer 403 `loopback_only` from a paired session off loopback; `DELETE /api/v1/storage/set-aside/{file}` naming a live store file, or a name no `set_aside_store` notice lists, answers 404 and deletes nothing; `start-empty` for a store that is open answers 400. · (11) `[L3-L8:agent-state]` A conversation whose stored messages exceed 2 MiB still answers a
new turn (no 413); a tool result over 512 KiB is saved cut, with its marker, and the turn is not lost. · (12) `[L3-L8:agent-state]` The decision protocol:
an accept writes the pending record through `decision_begin` before the action and the result through `decision_complete`; a refused apply removes it
through `decision_reopen`; Layer 6's applier writes through the same calls; no `agent/decisions/*.json` file appears; a `pending` decision survives a
reclaim; with the store unavailable the decision route answers 503 `store_unavailable` and nothing is applied; `GET /api/v1/proposals/{id}` carries
`decision.applied` as stored (`{pending: true}` mid-apply, then the result); a node111 test that kills the writer after `decision_begin` committed
(the `kill -9` of Layer 6's G5, `6-transport/verify/devices.sh:358-359`) reopens the store with its key and reads the record `applied: {pending: true}`,
and a retried accept answers 409 `already_decided` with the record unchanged. · (13) `[L1-L8:state-service]` After a daemon
start, `GET /api/v1/storage` and a history read open the node-state store and leave `state.db` and its WAL unchanged (size and mtime). · (14)
`[L1-L8:updates-backup]` (node111 tests) An update whose manifest names a newer store version is refused before the flip when the store's copy does not
fit above the floor: nothing is staged, `/status.update.last_result` reads `refused_space` and the `update_refused_space` notice names the release, the
store and the bytes needed; with a test copy rate set in the test so slow that a copy that fits cannot finish inside the 600 s probation, it is refused
the same way; `POST /api/v1/storage/update-without-copy` answers 409 `consent_required` (`update_without_copy`) until the repeat carries the hash, from a
paired session as from loopback, and 401 to the agent's bearer; the consent is recorded in `state/storage.json` for that release and store only, the
updater's next check installs that release with no `.pre-v<N>` of that store, and a revert leaves that store closed with the reason shown; the entry is
gone after the release is confirmed, and after a revert. · (15) `[L1-L8:state-service]` Over a test-sized budget with `agent`'s offer unanswered the class
reads `full`: a chat turn is answered and not saved, with the notice; a J9 write to a `full` `heal` returns `Full`; a decision is still written; a
proposal the agent writes while `agent` is `full` is still ingested (its spool file is gone) and listed by `GET /api/v1/proposals`. · (16)
`[L3-L8:agent-state]` With the node-state store unavailable (a `state.db` whose `user_version` is newer than the build): a chat turn is answered through the
relay and not saved, the chat view's notice names the reason, `GET /api/v1/chat/history` answers 503 `store_unavailable`, `GET /api/v1/storage` reports
`node_state.state` `unavailable` with the reason, and a proposal stays in the spool (§2.2 rule 8). · (17) `[L3-L8:agent-state]` The three agent tools
against the real `111d` (the real `111-agent` with a stub model that calls them): `storage.read` sends exactly one GET (the daemon's request log), to
`/api/v1/storage`, or to `/api/v1/datasets` with `datasets: true`, and returns no consent hash; `encyclopedia.search` with kiwix not enabled returns the
409 `component_unavailable` text "The encyclopedia is off."; `storage.propose` writes a `kind: storage` proposal that `GET /api/v1/proposals` lists as a
card, whose acceptance on loopback applies `quota.set {class: "keep"}` (`keep.bytes` changes) and from a session off loopback answers 403
`loopback_only` with nothing applied and the proposal open; no tool sends a non-GET (§16.4, §16.5).
**Pass:** `G13 GREEN`.

**G14, the budget** (`budget.sh`: one node whose root is a small tmpfs mounted inside the user namespace; node111 tests). Asserts: (1) `[L1-L8:state-service]`
The default `keep` follows §4.2 for this category, and `state/storage.json` is written once. · (2) `[L1-L8:state-service]` `GET /api/v1/fabric`'s `watermark`
equals the fabric's share, and never below 512 MiB with `keep_bytes` unset; after the first `set_watermarks`, `quota.transit` holds the value it had before
(written once) and the Storage page shows the scaled figure as a suggestion. · (3) `[L1-L8:state-service]` Over a test-sized budget: `agent` over its
share while the budgeted classes stay under `keep` loses nothing; with the total over `keep`, nothing goes before the export offer; after it, finished tasks
with no undecided proposal go first (record and `agent/work/<task>/`), then decided proposals, then conversations by `seq`, the class furthest over its
share first; an undecided proposal, a running task, a pending decision and the conversation of a turn in flight stay; after the reclaim `state.db`'s size
on disk shrinks (`incremental_vacuum`) and `disk_low` can clear. · (4) `[L1-L8:state-service]` Below the floor: `disk_low` is set, a dataset job pauses, a draft answers `insufficient_space`, a message is still
accepted. · (5) `[L1-L8:state-service]` **No idle write:** with no traffic over three capacity passes, while a file on the same filesystem is written
between passes (so free space moves), `fabric/store.db`, its WAL and `node/state.db*` are unchanged (size and mtime), and the node-state store is closed. ·
(6) `[L1-L8:state-service]` A child log past the cap holds at most the cap after the next pass, `.1` holds its previous tail, and the child keeps writing;
the `logs` figure and reserve count every capped file that exists (each cap × 1.5, plus 1 MiB). · (7) `[L4-L8:hub-content]` `GET /api/v1/storage` lists
the closed class set with figures that add up, and writes nothing; on a disk small enough that the fabric's 512 MiB minimum exceeds its share, the figures
add up to `keep.effective` and the page shows it; a `storage` frame is sent only on a change §16.3 lists (a chat turn that opens the node-state store
and the idle pass that closes it send none) and carries no `updates`, `bridge`, `kb`, `datasets` or
`components` figure, and publishing it walks no tree (node111 test: no `tree_bytes` call on the pass). · (8) `[L5-L8:consumer-registry]` (mesh111
`tests/consumers.rs`) An inbound `SYNC`, `KB`, `UPDATE`, `PATCH` or `AGENT_OP` from a contact never becomes a message row; with no consumer it is
acknowledged and counted; with a consumer registered it reaches the consumer once, and the fabric commits and acknowledges only after the consumer
returned; a consumer that fails leaves the frame unacknowledged and the retransmission reaches it again; a process killed after the consumer's write and
before the fabric's commit (a test fault point) redelivers the frame at the next connection, and the consumer, idempotent by `msg_id`, keeps one record; a
duplicate is re-acknowledged and never reaches the consumer. · (9) `[L1-L8:state-service]` **Fabric down:** with `fabric/store.db` unopenable (a newer `user_version`), the housekeeping interval still
runs: the node-state store closes after an idle pass, a log over its cap rotates, a job paused at low disk resumes when space returns, and step 3 writes
nothing (a changed `keep` reaches `set_watermarks` once the fabric is live again); the daemon holds the same one interval as with the fabric live (no
second timer). · (10) `[L4-L8:hub-content]` (node111 tests) Two `GET /api/v1/storage` within 60 s walk
`updates`, `bridge` and `kb` once; a release flip or a component change between them walks again; a tree over 100,000 entries stops the walk there and
reports its figure as a lower bound. · (11) `[L1-L8:state-service]` The fabric's export offer: over a test-sized fabric watermark whose excess is the
node's own history, the fabric's step 3 prunes nothing and `classes[fabric].export_offer_needed` is true until `POST /api/v1/storage/export-offer
{class: "fabric"}` (which calls `set_export_offered`); on the next pass step 3 runs and the class's bytes fall to the fabric's low mark (§4.2 item 3, §4.5, §13.3). **Pass:** `G14 GREEN`.

**G15, datasets** (`datasets.sh`: hubs H and H2 and a member M on one bridge in 10.111.8x.0/24, the stub mirror; node111 tests). Asserts: (1)
`[L2-L8:dataset-signing]` `111 dataset add` on H computes sha256, BLAKE3 and the sidecar in one pass; the sidecar's size is exactly `(ceil(size / 2^(10 +
ob_log)) − 1) × 64`, asserted at `ob_log` 12 and 14; a sidecar of another length is refused at fetch and at publish, and an entry whose figure exceeds
8 MiB is refused; publishing signs under `111/catalogue/v1` and writes `<hex>.toml.sig`, the name the route serves; the catalogue's `seq` and each new
entry's `seq` equal the announce-counter value the publish minted; one flipped byte fails the check. · (2) `[L7-L8:move-datasets]` With `hub_role`, LAN sharing or `origin.datasets` off, the
catalogue and the blobs answer 404; with all on they are served with no session; a non-GET answers 405. · (3) `[L7-L8:move-datasets]` The Range arm: 206 with
`Content-Range`; `bytes=-n`; 416 past the end and for two ranges; an `If-Range` mismatch serves 200; `Content-Length` always; never gzip or chunked; a fifth
concurrent blob body answers 503. · (4) `[L7-L8:move-datasets]` M's fetch: the first call answers 409 `consent_required` whose text names size,
licence, signer, `used_by` and the connection, and whose `facts` carry the free space, its "free after" reserving the unused budget; a file written on
M between the 409 and the repeat leaves the hash unchanged and the repeat with the hash answers 202, while a repeat that no longer fits answers
`insufficient_space`; `dataset` frames flow while the job runs, at most one per 5 s per job plus one per state change, and stop after it; the file lands by rename and no `.part` remains; a second fetch of another
entry while the job runs answers 202 and queues, a second fetch of the same entry answers `job_running`. · (5) `[L7-L8:move-datasets]` A job stopped mid-file resumes at the next missing block (bytes fetched ≤ size plus one
block); with H2 listed in M's `sources` for H's publisher, a corrupted block from the stub is fetched again from H2 (one block of extra bytes) and
`Job.source` names H2; a swapped sidecar fails before any data byte moves; an entry
whose `sha256` does not hash its bytes ends `failed (name mismatch)` and leaves no `.part`; an entry whose `sha256` names a blob H already holds (the
release payload) leaves that blob byte-identical; an entry whose `outboard_sha256` names the release payload, with the stub serving the entry's true sidecar
under that name (it passes its length and its root), ends `failed (name mismatch)` before any data byte is requested (the stub's log), and the payload stays byte-identical; the
same entry handed to `111 dataset import --from <dir>` is refused with "name mismatch" and nothing of it is adopted; a job resumed under the same boot id re-checks one block, under another boot id every marked block. ·
(6)
`[L7-L8:move-datasets]` On an interface marked metered with a test cap the job pauses `paused (metered cap)` with the carrier reserve (10 % of the cap,
at least 1 MiB) still left in the window, its bytes count in Layer 7's window, its Resume answers 409 `consent_required` (`dataset_override`) until the
repeat carries `accept_override`, and that override continues it, its bytes shown in
`Job.override_bytes` and not added to the window; a carrier flush still runs after the job paused and after the override; a rule that starts to match
mid-job (the test marks the job's interface metered between two blocks) pauses it `paused (metered link)` before the next block, and Resume answers 409
`consent_required` naming that rule until the repeat carries its hash; the bearer cannot accept either consent. · (7) `[L2-L8:dataset-signing]` A catalogue from an unfollowed publisher is
not listed; the CLI import refuses without `--follow`; a lower `seq` is refused after a higher one; H, restored from its words onto a new root with an
operator-supplied counter floor, publishes a catalogue whose `seq` is above its last and M accepts it; a catalogue that lists one entry of an unknown
`kind` (`patch`) is listed with its other entries and `skipped` = 1, while an unknown key in an entry of a known `kind` refuses the file, and so does an
entry whose `version` holds a `/`, a control character or a leading `.`. · (8) `[L7-L8:move-datasets]` `111 dataset import --from
<dir>` verifies and copies under `state/origin/import.lock` and hands its copies to the running daemon (`POST /api/v1/datasets/adopt`, refused off
loopback), which alone writes `state/datasets.json`; with the daemon stopped the verb writes it; `--in-place` copies no data (only the sidecar lands in
`state/origin/blob/` and the index is written) and refuses a path the daemon's user cannot read; `--from http://<H2>/origin/v1/` reads the catalogue names a
check asks for, adds that URL to `sources` through the adopt route, and with `--follow <hex>` imports a publisher it could not list; after an unclean shutdown, or once its mtime changes, the in-place file is `unverified`, answers 404 and leaves
the kiwix library until it is hashed again; an unplugged one is `disk_absent`, in `Holding.state` and `DatasetEntry.state` alike (`location`
`in_place`), and an entry listed but not held reads `not_held`; with `fabric/store.db` unopenable at a start (a newer `user_version`), every in-place
file reads `unverified` at that start; with `origin.datasets` off, an in-place ZIM replaced on its disk while kiwix-serve is stopped is out of the
library before the next start (no request of ours touched it), and one replaced while it runs is out at the next rebuild. **The adopt route carries what
the verbs hand off:** the import's catalogue reaches `state/origin/catalogue/` only through `111d` (its staged `.toml.sig` tampered with between the
verb and the hand-off: 400 `bad_request` and nothing written), `111 dataset add` on H with the daemon running leaves a draft that `GET
/api/v1/datasets/drafts` lists, and `--in-place` leaves an index record with its path and tuple, while a file touched between the verb's hash and the
hand-off is refused with nothing written; the request file and the staged catalogue copies are gone after each adopt. · (9) `[L1-L8:state-service]` Filling the budgeted classes
past `keep` removes no dataset; a fetch that does not fit answers `insufficient_space` before any byte, and so does one that fits the disk but not beside the
unused budget; `DELETE` removes the file and its sidecar. · (10) `[L1-L8:state-service]` A job killed between the rename and the index write: at the next
start the `partial` record whose blob exists reads `present`; `state/datasets.json` is not rewritten while a job runs (its mtime unchanged between the
first and the last block). · (11) `[L7-L8:move-datasets]` With `state/origin/` on its own filesystem (a second tmpfs mounted there), a job's space check
and the `datasets` figure read that filesystem and `GET /api/v1/storage` shows `disk.origin`; a job whose `incoming/` and `blob/` differ in `st_dev`
refuses at start. · (12) `[L7-L8:move-datasets]` The stub's request log shows every `get_to_file` call of a pull asking one block, and of a first copy
one 16 MiB window; a stub that answers 200 to a range at an offset above 0 ends the job `failed (source cannot resume)` with nothing of that body written;
a stub that stalls mid-block fails that call within the bound (the range at 64 KiB/s) and the block is fetched from the next source. · (13)
`[L7-L8:move-datasets]` A stub that answers 503 pauses the job `paused (source busy)`; over 40 capacity passes it receives requests only on the passes the
doubling gap names (1, 2, 4 … 32), and the job resumes on the first pass after it answers again. · (14) `[L7-L8:move-datasets]` With `origin.datasets` off
on H, M sends no catalogue request to H on a link attribution (the stub log and H's request count); with it on and consented, H's announce carries
`ROLE_DATASETS` (bit 7) and bit 5 is unchanged, and M's GET on the next attribution sends `If-None-Match` and receives 304 for an unchanged catalogue, its
bytes counted in the metered window; with that window's cap reached no GET is sent; `origin.components` changes no role bit. · (15)
`[L7-L8:move-datasets]` A Check now on M asks the home hub and each `sources` URL for the home hub's and every followed publisher's catalogue name, and a
followed publisher served only by H2 is listed with `source` H2. · (16) `[L7-L8:move-datasets]` A fifth reader that accepts no byte for 60 s is closed
and its stream slot serves the next request. · (17) `[L7-L8:move-datasets]` A refresh: with version 1 of an `id` held, a fetch of version 2 lands
beside it while version 1 stays listed and served; accepting the refresh, whose consent names the bytes it frees, removes version 1 and its sidecar; a
refresh that does not fit beside the old version answers `insufficient_space` before any byte moves (§7.9). · (18) `[L4-L8:hub-content]` Two publishers
holding one `id`, and two versions of one `id`, each answer on their own `/api/v1/datasets/entries/{publisher}/{id}/{version}` and
`/api/v1/tiles/{publisher}/{id}/{version}` paths; an entry whose `id` is `check`, `drafts`, `publish`, `search`, `jobs` or `adopt` is fetched and
deleted under `entries/` with no collision; a 416 on the tiles route carries a `problem+json` body, code `range_not_satisfiable`; a `Job.id` starts
`job-`; `PATCH /api/v1/storage/settings` from a paired session off loopback answers 403 `loopback_only` and changes nothing; two publishers holding one
`id` and version of a ZIM each have their own symlink, `state/kiwix/zim/<publisher short>-<id>-<version>.zim` (16 hex characters), and `library.xml`
lists both. **Pass:** `G15 GREEN`.

**G16, the encyclopedia** (`kiwix.sh`, in two parts). **Part A, our own code, always run** (its prerequisites are the netns and a debug build only):
the stub child `stub-child.py` stands in for kiwix-serve, run as the kiwix component's program through a debug-build-only override (the
`NODE111_BRIDGE_STUBS` precedent, `N/bridge/stub.rs:351`, `:402`: a new `NODE111_STUB_CHILD`, read by a debug `111 daemon` only; a release build has no
such override). **Part B, the real binary**, behind `KIWIX_BIN` (a pinned kiwix-tools tarball) and `KIWIX_ZIM` (one small ZIM, Q9), both fetched by their
pinned sha256 and never vendored, else `G16 SKIPPED` for part B unless `REQUIRE_G16=1`; the lock run sets `REQUIRE_G16=1` with the `aarch64-linux` pin of
`kiwix.toml` (§9.2). Asserts, part A: `[L6-L8:server-component]` (node111 unit tests) the embedded `kiwix.toml` and `panels/kiwix.toml` parse: no
`adapter`, `start = "on_demand"`, `health = "listen"`, the per-target `program` resolving `{payload}`, `{root}` in the library argument; a declaration
naming an unknown placeholder or a `program` with braces is still refused; a component without `adapter` reads `adapter: null` in `GET
/api/v1/bridge/components` · `[L6-L8:server-component]` declared and disabled, the child has no process, listener or open file (G7's `zero-weight.sh`);
enabled but never opened, it has none either (enable stopped after the install; a daemon restart's `resume_enabled` starts nothing), and the panel holds
exactly one listener in 11190-11229 · `[L5-L8:child-panel]` `POST /api/v1/panels/kiwix/session` (a component id) mints; the first mint starts the child
on 127.0.0.1:11250 only (`ss`) and answers once its port answers; a request through the panel listener with the cookie reaches the child (its log) and
one without it answers 403; the child's echo shows neither the panel cookie nor the dashboard's session cookie; a WebSocket upgrade is refused at the
panel listener and never reaches the child (no 101); every answer carries the appended `connect-src 'self'` · `[L6-L8:server-component]` `POST
/api/v1/bridge/components/kiwix/stop` leaves no process and keeps the files; the same route for a component that is not `on_demand` answers 400; with no
request through the proxy for 30 passes (the pass knob of "Gate homes") the child stops and the next mint starts it, while a `GET
/api/v1/datasets/search` sent every 20 passes with no panel request keeps it running past 30 passes (each search reaches the child, its log); after a stop and after the idle stop the child has no process, listener or open file; "remove files" removes `state/kiwix/` ·
`[L4-L8:hub-content]` the bearer's session mint answers 401; `encyclopedia.search` while the component is off answers 409 `component_unavailable`.
Asserts, part B: `[L6-L8:server-component]` the real kiwix-serve runs with `--blockexternal` (`/proc/<pid>/cmdline`) and answers once its port answers ·
`[L6-L8:server-component]` a held ZIM added or removed rebuilds the library with `kiwix-manage` and sends `SIGHUP`, with no restart (the same pid) ·
`[L4-L8:hub-content]` `encyclopedia.search` returns at most 8 KiB in an untrusted block while it runs; `book=<id>` finds the held ZIM by its `book` name,
and an entry without `book` answers 400 `bad_request`; with the same `id` held from two publishers under different `book` names, `publisher=<hex>`
selects that publisher's book and no `publisher` selects the home hub's. **Pass:** `G16 GREEN` (part B counted only when it ran).

**G17, maps** (`maps.sh` with a PMTiles fixture `gen-pmtiles.py` writes into the scratch root, so G17 needs no download and never SKIPs for a fixture;
UI tests). Asserts, in `maps.sh`: `[L4-L8:hub-content]` `GET /api/v1/tiles` lists the held set; `/api/v1/tiles/{publisher}/{id}/{version}` serves ranges
and refuses a request with no session off loopback · `[L4-L8:hub-content]` the CSP is unchanged (`connect-src 'self'`, no `blob:`). In `pnpm check`
(`check-budget.mjs` on the scratch build, `test/budget.test.ts`): `[L4-L8:hub-content]` the `map` chunk counts in the exempt bucket, ≤ 90 KB, and the lazy
total without it is unchanged · `[L4-L8:hub-content]` the lazy total with the `l8` and `l8-hub` chunks is ≤ 256 KB, and Layer 8's chunks, its adapter
and its card together are ≤ 12 KB gzip (§15.4). In vitest (`test/layer8-views.test.ts`, the mock adapter): `[L4-L8:hub-content]` the map chunk is never
fetched when no set exists · `[L4-L8:hub-content]` the `l8-hub` chunk is never fetched on a node without `hub_role` · `[L4-L8:hub-content]` the ODbL
sentence is in the rendered map. **Pass:** `G17 GREEN`.

**G18, component pins from a hub** (`components.sh`; node111 tests with a fixture declaration; the real `deltachat` pin when `cache/packages/deltachat/` holds
it, else that item SKIPPED unless `REQUIRE_G18=1`). Asserts: `[L1-L8:updates-backup]` with `origin.components` off a component blob answers 404; on (after the
consent) only a pinned hash is served, from `cache/packages/`, with its notice; a member with no internet route installs the component from its hub (egress
log), and a 404, a wrong size or a wrong hash falls through to the upstream URL; a pip closure arrives through the files table; the `lxmf` closure answers 404
(Q25); removing `cache/packages/<id>/` ends the serving; a cache file is hashed when `origin.components` turns on, and one whose tuple changes (a new
mtime) answers 404 until it is hashed again, with no hash per request; `GET /origin/v1/notice/{sha256}` answers the declaration's `spdx`, `read_at` and
`notice` verbatim, and a node111 unit test checks that every embedded declaration's `notice` names a source URL · `[L1-L8:updates-backup]` (with a
fixture manifest) with `origin.components` on and the Termux tick off, a `[[deb]]` or `[apk.*]` pin answers 404 and nothing is fetched; a build whose
licence list marks the pins verified offers the tick, its job fetches each pin from its pinned URL into `state/origin/blob/` (sha256 and size checked,
counted in the metered window), and they are served · `[L1-L8:updates-backup]` a hub with `origin.serve` on fetches the `[source]` pin after it staged its payload, from its home hub first, sha256 and
size checked and counted in the window, and serves it; when that fetch fails the payload's notice names the upstream URL. **Pass:** `G18 GREEN`.

**G7, extended:** `[L6-L8:server-component]` `zero-weight.sh` with kiwix declared and disabled · `[L1-L8:state-service]` Layer 7's `idle.sh` no-timer
line unchanged with Layer 8 present (L7 §3.7). **Pass:** both.

**Unit and integration test homes.** mesh111: `src/store/tests.rs` (save on change, `journal_size_limit`, the migration copy, `open_encrypted`, the
orphan sweep beside a set-aside store, `KeyProvider::key_for`),
`tests/consumers.rs` (new); its vectors in `5-messaging/test-vectors/`. node111 (`1-bootstrap/crates/node111/tests/`, 37 test files at the pinned head): new `node_state.rs`,
`storage_budget.rs`, `range.rs`, `datasets_origin.rs`, `update_guard_store.rs`, `export.rs`, `panels_child.rs`, `install_origin.rs`; the embedded
`kiwix.toml` and `panels/kiwix.toml` parse tests beside the manifest parsers' own (`N/components/manifest.rs`, `N/panels/manifest.rs`). agent111:
`tests/chat_loop.rs` (`history` in, `messages` out; the tool count), `tests/daemon_tools.rs` (the three tools), `tests/layer6_tools.rs` and
`tests/layer7_tools.rs` (their count pins), new `tests/store_atomic.rs`. UI: new
`test/layer8-contract.test.ts`, `test/layer8-views.test.ts`; `test/budget.test.ts` (the `map` bucket). The harnesses that seed or read decision files
today move to the store with §2.4 rule 5 (§21 L4, L6): `4-interface/verify/smoke.sh:515-529` (phase 5f) and `6-transport/verify/devices.sh`, which reads
decisions through `decision_file` (`6-transport/verify/spine/node.sh:239`; `devices.sh:151`, `:329`, `:356-368`, `:402`, `:417`). The store is
SQLCipher and only `111d` opens it, so both read the decision through `GET /api/v1/proposals/{id}`, whose `decision` carries `applied` as stored
(§16.2): smoke's phase 5f checks `decision.decision` there in place of the file (`smoke.sh:529`); `devices.sh` reads `decision.applied.pending` there
while the daemon runs (`:351-352`) and after the restart (`:363-364`), and compares the `decision` object before and after the retry in place of the
file's sha256 (`:356`, `:367-368`). Its one assertion made while the daemon is dead after `kill -9` (`:358-359`, the pending record surviving the kill)
moves to G13 (12), a node111 test that reopens the store.
`6-transport/verify/bridge.sh:696-700` seeds a proposal file with no agent running and reads it by id: it keeps working through the sweep before `GET
/api/v1/proposals/{id}` (§2.4 rule 2).

**CI mapping** (`.github/workflows/ci.yml`; parity with the local gates, `plan/cross-layer/ci-cd.md`): the mesh111 and node111 tests and the UI checks behind
G13-G18 run in `bootstrap-gate` (`1-bootstrap/scripts/check.sh`, which nests `5-messaging/scripts/check.sh` and `pnpm check`); the agent's tests in
`intelligence`; contract drift (`pnpm gen:types` leaves the tree unchanged) and the KB entries in `knowledge-contract-hygiene`. The netns harnesses and G7
stay the lead's: the runners restrict `unshare -Urnm` (`plan/cross-layer/ci-cd.md:18-19`). Root-only `check.sh` failures on the build box are recorded, never
counted as tree faults (D29).

## 19. Pi-floor accounting

| Item | Every Layer 8 switch off | Idle node, against Layer 7 | A chat turn | Hub, `origin.datasets` on, idle | A dataset job | kiwix enabled, never opened | kiwix-serve started (until Stop, the idle stop or daemon stop) | Basis |
|---|---|---|---|---|---|---|---|---|
| processes | 0 | 0 | 0 | 0 | 0 | 0 | +1 (kiwix-serve) | design |
| listeners | 0 | 0 | 0 | 0 (routes on :11180) | 0 | +1 (the panel's, in 11190-11229; LAN-bound with LAN sharing, §9.1 (5)) | +1 loopback (11250), beside the panel's | design |
| threads | 0 | 0 | 0 | 0 | +1 (`111d-dataset`) | 0 | the runner's, as any child | design |
| timers | 0 | 0 (the existing 5 s housekeeping interval, its owner moved to a daemon-level task, §4.3) | 0 | 0 | 0 (a busy source is retried on the capacity pass after a gap of 1, 2, 4 … 32 passes, §4.3 step 6) | 0 | kiwix's own 1 Hz wake; the idle stop is a counter on the existing pass (§4.3 step 9) | R07g3 §1 |
| open files | 1 (`node/lock`, held for the daemon's lifetime, §13.2) | 1 (`node/lock`; the node-state store is closed) | +3 (`state.db`, `-wal`, `-shm`) until the next idle pass | each file while it streams | the `.part`, the sidecar, a socket | the panel's listening socket | the ZIMs | design |
| writes | 0 | the disk-figure commits Layer 7's idle pass still makes whenever another write moved free space, removed (§13.4; measured at build) | measured at build for the schema as written (R07g1 §2's 12.4 KB, about 25 KB with the checkpoint, counted three pages, §2.8); a read writes nothing | 0 | the file and its sidecar once, one `fsync`; reads (§7.11): each block read back by `valid_ranges`, one more sequential read of the file for its sha256 (and, on a first copy, BLAKE3 and the sidecar) when the pull did not run front to back (§7.3 (3)(e)), every marked block once on a resume under another boot id, one read of each in-place or cache file re-hashed after a change or an unclean shutdown (§5.5); an import or `add`: one read, and one write in copy mode | 0 | measured at build | R02 finding 13; R04 finding 7 (before M1c); R07g1 §2 (derived); measured at build |
| memory | 0 | 0 | the store's page cache, measured at build | 64 KiB per stream | one block buffer (16 MiB or 4 MiB) | the listener's, measured at build | RSS measured at build | R07g3 finding 9 |
| disk | 0 | `state/storage.json` | the node-state store | the served files | the dataset and its sidecar | the tarball, measured at build | as enabled | design |

**Build-time weight, not resident:** the `111` binary delta and the `111-agent` delta (measured at build); the Storage chunks `l8` and `l8-hub` with the storage adapter and the
`storage` card, at most 12 KB gzip together inside the 17.1 KB the lazy bucket leaves (§15.4); the `map` bucket,
about 84 KB gzip (a scout's esbuild figure, R05 finding 13, measured again at build); +0 crates.

**Rule.** The build records each figure, measured on the release binaries, in `8-storage/TASKS.md` beside Layer 6's (`111` 19,404,064 B at 19.7 MB idle RSS,
`111-agent` 4,392,280 B at 5.3 MB, listeners :11180 (+ the best-effort :80) and :11182, `6-transport/TASKS.md:82`), with the protocol of OB §3: `VmRSS` and
`Pss` from `/proc/<pid>/smaps_rollup` (anonymous against file-backed, which settles kiwix-serve's figure), `ss -ltnup` before and after, `strace -c -f -e
trace=write,fsync` over ten idle minutes, `/proc/<pid>/io` `write_bytes` over an idle hour and `/sys/block/mmcblk0/stat` (§13.4's saving against Layer 7's
tree, with another program writing on the same filesystem), binary and cache sizes, and the time and reads the budget adds to the pass with the node-state
store open. On the largest store the defaults allow, in one sitting: the export, the migration copy (its seconds per GiB set §13.6 (5)'s bound), the two
integrity checks, the name's sha256 pass of a pull beside the import pass, and the per-turn WAL figure of §2.8. A change to the first two columns fails the gate until the
owner says otherwise.

## 20. Deferred / later versions

| Item | What it needs |
|---|---|
| Tier 2 transfer; hub-to-hub bulk over the internet | `bao-tree` 0.17, bulk on its own connection or a non-blocking `Link::send`, the Layer 5 amendment list Layer 8 writes (§8) |
| automatic encrypted snapshots to the home hub | Tier 2 and a hub that opted in; the §13.1 format |
| a live export | a verified live-snapshot route on SQLCipher (the backup API or `VACUUM INTO`) and `rusqlite`'s `backup` feature |
| Mode T for both stores | a hardware-sealed key (L5 §10.2) |
| the fabric borrowing room the node-state store leaves unused (§4.2 item 3) | a measured need; one watermark that moves with the other classes' use |
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
| a hub's encyclopedia and map read by unpaired members | a read-only LAN session (`lan_read_only`, reserved, `4-interface/README.md:591`) |
| the Termux deb and APK pins served by a hub (§11.5) | each pin's licence verified in `LICENSING.md` |

## 21. Cross-layer seams & amendments

- **Layer 5 (locked v1.2), six additive spec amendments:** (1) §10.1: `journal_size_limit` (4 MiB) joins the pragma list, and "One DB: `fabric/store.db`"
  (`5-messaging/README.md:425`) gains "Layer 8 keeps a second file, `node/state.db`, for the node's own state" · (2) §10.3: the migration copy before any
  migration (§13.6) · (3) §10.4 step 3 (`:460`) and §21 (`:865`): "`age`-based export" → "the L2 §8.5(i) export (Layer 8 §13.1)" · (4) §11 (`:477-479`): the
  updater intake at `fabric/blobs/<hash>` → superseded by `state/origin/blob/<sha256>` (L7 §9.1); "`KB` deltas and git bundles (Layer 7)" → the KB rides the
  release payload (L7 §9.4; git bundles were rejected, D30 F6); `SYNC` stays reserved with no consumer in v1 (§12; no cursor table and no prune step for
  `Class::Sync`, R03 finding 10) · (5) §13: `child` panels are built for plain-HTTP children (§9.1); a `child` panel's id is its component's id (`panelId` = `ServiceId` or
  `ComponentId`), it has no `bundle`, it is enabled while its component is enabled, its health is the component's port, it is framed from Storage →
  Datasets, and its answers carry an appended `connect-src 'self'` (§9.1 (3), (5)); WebSocket and SSE children and `detected` stay reserved ·
  (6) the dispatch: the consumer registry for 0x10-0x14 (§12.3), shared with Layer 9's F13 item 13, with its order: a consumer's durable write, then
  the sequence commit and the acknowledgement; a consumer error leaves the frame unacknowledged. **Code seams** (no spec text, no wire byte): S1
  `derive_tagged_key` and `KeyProvider::key_for`, S2 `open_encrypted` and `migrate_with`, S3 the orphan sweep beside a set-aside store (§2.7), the disk
  figures unpersisted (§13.4; `save_capacity` on change is built by Layer 7 row M1c; the module note `M/store/prune.rs:110-114` changes with it). No wire byte changes in v1. L5 §19's Layer 8 row ("`SYNC` class + per-peer cursors; the append-only log is an
  opaque consumer", `:830`) is closed by §12.
- **Layer 7 (locked v1.0), additive:** (1) §9.1: the tree holds datasets, sidecars and the catalogue files; `/origin/v1/catalogue/*` and `/origin/v1/notice/*`
  join the routes; one admission rule over four pin sources (§11.1; the Termux pins off in v1); `origin.datasets` and `origin.components` gate their own files and live in
  `state/storage.json` (§22 C1); the Range arm and the stream cap; the release's source tarball served beside its payload (§11.6) · (2) §10.5: "≈ 69 KB"
  (`7-relay/README.md:951`) → 83.9 KB, bucket 90 KB · (3) §12: dataset fetch bytes count in the same window as carrier bytes; `ConsentRequired.action` gains
  six values after Layer 7's four (§16.2); one sentence: "A dataset job asks the metered module which rule matches before every block, pauses with the carrier's
  reserve (10 % of the cap, at least 1 MiB) left in the window, and adds no override bytes to it, so a dataset never closes the carrier's gate (Layer 8
  §7.8)." · (3a) §7.2: the role table gains bit 7 `ROLE_DATASETS` (set while `origin.datasets` is consented, with the hub role and LAN sharing; named
  `datasets`), defined in node111 beside bits 3-5 and Layer 9's bit 6 (Layer 8 §12.4); bit 5 is unchanged · (3b) §9.1: the release manifest's `[[deb]]` and
  `[apk.*]` pins are a pin source under `origin.components`, not `origin.serve` (§11.1, §11.5); the updater of a hub with `origin.serve` fetches the
  `[source]` pin after staging its payload (§11.6) · (4) §17: the Tier 2 row gains its owner, Layer 8 (later); the map bucket row (`:1265`) moves to Layer 8 v1 · (5) §9.5 T6:
  `UserTypedDataset` joins the typed exceptions · (6) the update page (`UI/src/views/settings/l7/updates.tsx`) shows an update refused for space
  (`/status.update.last_result` `refused_space`) and offers the `update_without_copy` consent through `POST /api/v1/storage/update-without-copy`
  (§13.6 (5), §16.1); the manifest's `[stores]` table and the updater's bound are Layer 1's (below). **No effect on §5.8:** an unset
  `quota.transit` is written once at its effective value when the budget first sets the watermark, so L7 §5.8's figure holds until a human changes it (§4.2
  item 4). **Cross-layer request:** the migration copy in Layer 7's build if its migration is the first after a signed release, and the update guard's two
  restore rules (the legacy files' own trigger included) in Layer 7's build if its release is the one before the first release with the node-state store
  (§13.6 (3), (4)). §9.6's "signed by their
  publisher" (`:923`) is made concrete by §6.
- **Layer 6 (locked v1.0):** §2.1 rule 1, the whole additive set of §9.2-§9.3: `adapter` optional in the parser, nullable in `Component` (kept in
  `Component.required`, `NetworkId` or null) and in the UI row (null = a server component); an adapterless component's state and log directory `state/<id>/`, removed by "remove files"; a per-target `program` key in
  `[payload.targets.<t>]` that `{payload}` names; `[run] start = "on_demand"` (enable stops after the install, `resume_enabled` skips it, claim, pre-bind
  and start at the first session mint); `[run] health = "listen"`; the route `POST /api/v1/bridge/components/{id}/stop`, for an `on_demand` component
  only · §2.1 rule 3 (`6-transport/README.md:145-146`): the
  install-time fetch tries the home hub's origin first (§11.3); "There is no runtime fetch" stands · G7 covers a declared, disabled kiwix · the applier's
  `DecisionFile` (`N/devices/applier.rs:581-660`) writes through J9's three decision calls (§2.4 rule 5), and the harnesses that read
  `agent/decisions/<id>.json` read the store instead: `6-transport/verify/devices.sh` through `decision_file` (`6-transport/verify/spine/node.sh:239`);
  `6-transport/verify/bridge.sh:696-700`, which seeds a proposal file and reads it by id, keeps working through the spool sweep (§2.4 rule 2).
  **Lock-amendment candidate (the owner's):** Layer 6's G5 crash-guard assertions in `devices.sh` move from the decision file to `GET
  /api/v1/proposals/{id}` and its `decision.applied` (`:351-352`, `:356`, `:363-364`, `:367-368`), and the one made while the daemon is dead after
  `kill -9` (`:358-359`) moves to Layer 8's G13 (12), a node111 test that reopens the store (§18 test homes): the gate keeps its property and changes its
  means.
- **Layer 4 (locked):** the §16 contract and its pin list · `/settings/l8` · the `map` bucket in `check-budget.mjs` · `4-interface/README.md:717` "storage
  (L8)" leaves the deferred list · `UI/src/views/settings/model.ts:222` · the conventions' "Not gated" list (`4-interface/api/README.md:210`) gains
  `/origin/v1/catalogue/*` and `/origin/v1/notice/*` · `4-interface/verify/smoke.sh:515-529` (phase 5f asserts `agent/decisions/prop-smoke.json`) reads the
  decision through `GET /api/v1/proposals/prop-smoke` (§2.4 rule 5, §18) · additive contract changes: `Proposal.decision` gains the optional `applied`
  member as stored (§16.2), `Status.update.last_result` gains `refused_space` (§13.6 (5)), `ConsentRequired.action` gains `store_start_empty`, one
  `DatasetEntry.state` / `Holding.state` enum with `DatasetEntry.location`, the routes `POST /storage/update-without-copy` (a session or loopback),
  `DELETE /storage/set-aside/{file}` and `POST /storage/start-empty` (loopback only), the search route's `publisher` (§16.1, §16.6 (10)) · `panelId`
  widened to `ServiceId` | `ComponentId`, `Component.adapter` typed `NetworkId` or null and kept in `required`, the components stop route,
  `ConsentRequired.facts` shown under the text and never hashed (§16.6 (7)) · the conventions (§16.6 (9)): the `job-…` id family,
  `PATCH /storage/settings`, `POST /datasets/adopt`, `DELETE /storage/set-aside/{file}` and `POST /storage/start-empty` among the loopback-only writes, the `storage` frame's fields and the `dataset` coalescing,
  `range_not_satisfiable` · the storage adapter seam `getStorage()` (§16.6 (8)) · the Storage page in two lazy chunks inside Layer 8's 12 KB allowance
  (§15.4).
- **Layer 3 (locked):** an additive note under `3-intelligence/README.md:50`: "As built by Layer 8: conversations, proposals, tasks and decisions live in
  `111d`'s encrypted `node/state.db`; the agent holds no key, sends nothing but GETs, and spools proposals and tasks under `agent/` until `111d` ingests them
  (Layer 8 §2)." The agent contract (`3-intelligence/TASKS.md:22`, `:27-28`, `:37`): the chat body's `history` (at most 512 KiB, the body limit 2 MiB), the
  turn's `message` frames before `done` (each at most 512 KiB) and `messages` in a non-streamed answer, proposals and tasks as an atomic spool with
  `.tmp-` temp names removed at the agent's start (§2.3, §2.4). Tools +3, 20 → 23, appended after Layer 7's (§16.5), `encyclopedia.search` among them
  (with an optional `publisher`), and `storage.read` makes one GET per call. The `conversations` count comes from `111d`.
- **Layer 2 (locked v2):** one block §13.3 "Layers 8 and 9 amendments": **A19** domain tags: `111/catalogue/v1` (Ed25519 by the publisher over `tag ‖
  publisher ‖ catalogue bytes`, §6.1) · `111/export/v1` (Ed25519 by the node over `tag ‖ NodeId ‖ length ‖ BLAKE3(the blob before the signature)`, §13.1) ·
  `111/node-state/v1` (a key-derivation input, the `111/store/v1` pattern of A9) · reserved `111/catalogue-vouch/v1` · Layer 9's `111/fix/v1` and
  `111/fix-vouch/v1` (J5; their constructions are Layer 9's). **A20:** §8.5(i) is built (§13.1): its tag, the chunked framing, the prehash. The root node key
  signs catalogues and exports (`L2:91`, control artifacts). A note under §8.4: the announce counter gains a second caller, the catalogue `seq` minted at
  each publish (Layer 8 §6.4); a mint only moves the counter up, so the restore floor and Layer 5's epochs are unaffected. A note under A17: `appdata` role
  bit 7 is Layer 8's `ROLE_DATASETS` (named in L7 §7.2, carried as opaque by `id111` and `mesh111`).
- **Layer 1 (locked v2.2):** layout rows: `node/` (0700; `state.db` 0600) · `state/storage.json`, `state/datasets.json` (0600) · `state/origin/` widened
  (`blob/` holds datasets, sidecars and the source tarball; `catalogue/`, `incoming/`, `drafts/`) · `state/kiwix/` · `state/heal/` (0700, Layer 9, J2 and J7)
  · `node/lock` (0600, the daemon's lifetime `flock`, §13.2) · `state/origin/import.lock` (§7.6) and `state/origin/` as a possible mount point (§5.1) ·
  `agent/legacy-v0/` and `state/heal/legacy-v0.jsonl` (§2.5) · `*.pre-v<N>.tmp`, `*.pre-v<N>`, `*.rolled-back-v<N>`, `*.damaged-<n>`, `*.replaced-<n>` beside a store · verbs `111 store
  export|import` and `111 dataset add|import` (the subcommand set, `NI/cli.rs:153-192`) · the fetcher's `get_to_file` and `UserTypedDataset`;
  `PayloadFetcher::fetch_pin` with the origin list, the root installer reading `home_hub.address` from `fabric/settings` (§11.3); the manifest's `[source]`
  pin (§11.6), fetched by the updater of a node with `origin.serve` on after it staged its payload, and `[stores]` table (§13.6 (5)) · the update guard's store
  restore, in `revert_to` and as a content rule at every `ExecStartPre`, and its own trigger for the legacy files (§13.6 (3)); the updater's space and time
  bound before the flip, its state `last_result: refused_space`, its read of the `update_without_copy` consent from `state/storage.json` at its next
  check, and `EXTEND_TIMEOUT_USEC` while a migration runs, the node-state store's included at the first start of a newer build (§2.2 rule 7, §13.6 (1),
  (5)) · the daemon's exit with status 75 ("a store verb is running") while a store verb holds `node/lock`, restarted by the unit (§13.2) · the housekeeping interval as a daemon-level task (§4.3) · the child log ring (§13.8) · text: "a USB drop into the same directory is the offline
  alternative" (`L1:252-266`) gains "or the home hub's origin (Layer 8 §11)"; "Updater intake (seam)" (`L1:247-248`) is superseded by `state/origin/blob/`;
  the machine contracts (`L1:162`) gain `/origin/v1/catalogue/*` and `/origin/v1/notice/*` · dependencies, signed off under "Dependency policy" in
  `1-bootstrap/TASKS.md` with the `blake2 = "=0.11.0"` precedent (`1-bootstrap/crates/node111/Cargo.toml:69-71`): direct `rusqlite` (=0.40.2,
  `bundled-sqlcipher-vendored-openssl`), `bao-tree` (=0.16.1), `blake3` (=1.8.7); +0 crates.
- **Layer 0:** `candidate` entries `projects/kiwix-tools.md`, `projects/pmtiles.md` (PMTiles, go-pmtiles, protomaps-leaflet, Leaflet) and
  `projects/p2panda-core.md` (the named log target), at the desk-research cap (`0-knowledge/README.md:93`); `projects/age.md`'s summary "planned for history
  export only" (`:5`) → "a possible format for hub-side tooling; 111's export uses the L2 §8.5(i) format (Layer 8 §13.1)".
- **`LICENSING.md` and `DECISIONS.md`:** §5 rows for the direct edges (no new crate); §8 rows for the hub's conveyance duty (§11.7), kiwix-tools as a
  component and the three map packages (Leaflet BSD-2-Clause, protomaps-leaflet and pmtiles BSD-3-Clause), with `THIRD-PARTY-NOTICES.md` regenerated from the
  UI lockfile; `:151`'s 0.17 note stands until Tier 2. §8 also gains: a row per member of the kiwix static tarball's closure (Xapian, ICU, libmicrohttpd,
  libcurl, zstd, lzma, pugixml; R01 finding 2), each licence read at pin time, since the notice a hub serves must name them (§11.7); a "content a hub serves"
  row for the dataset licences an operator conveys (CC BY-SA 4.0 for a ZIM, ODbL for a map; §9.7, §17 "Legal"), kept apart from §7's list of licences
  incompatible with what `111` links or the dashboard bundles (CC-BY-SA is on it, `LICENSING.md:195`): served content is neither; a row for a label font if the map build bundles one (§10 (8)); and the line "the ZIM test
  fixture and the kiwix-tools tarball are not vendored: the lock run fetches them by pinned hash; the PMTiles fixture is generated by a script of ours
  from no third-party data" (G16, G17). The Termux deb and APK rows (`:64-65`) gain their
  verified licences before §11.5's pin source is offered. D26 (5) gains the §11.7 sentence; D21 gains a note: the exempt `map` bucket, 90 KB gzip (D30 F9
  recorded the bucket with no figure and stands), and, only if the owner raises it, the lazy budget (§22 D22).
- **Hand-off to Layer 9:** §3 (J1-J9), cited by number; the J2 hook records the daemon's exit status 75 (a store verb holds `node/lock`, §13.2) with
  its code and never as a crash; the consumer registry for 0x10 and 0x13 (§12.3); the hub fix shelf, later: a catalogue class `patch`
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
text, now at `:252-266` (the brief cited `:227-246`); `:185` "APK refresh = Layer 9 maintenance task" is Layer 9's (text unchanged here). · (6)
`6-transport/README.md`: the origin list, now at `:145-146` (the brief cited `:113-117`); `adapter` optional. · (7) `2-identity/README.md` §13.1 A4: the tags
join the registry (§21 L2, as §13.3, C4); §8.5(i) names its tag. · (8) `LICENSING.md:56`: **already done** ("Text verified 2026-11-01", read at the
pinned head); `:151` already carries the 0.17 note; the §8 rows and the notices: §21. · (9) `plan/DECISIONS.md`: the D26 (5) sentence and the D21 note (§21). · (10)
`0-knowledge/projects/age.md:5` and the new candidates (§21 L0). · (11) `4-interface/README.md` "storage (L8)", now at `:717` (the brief cited `:697`);
`UI/src/views/settings/model.ts:222` (cited `:221-222`). · (12) This folder's research: R01 §4's "Cat 4A on Pi" rows contradict
`NI/install/categorize.rs:102-105` (Cat 4A needs ≥ 512 GB free and ≥ 8 GB RAM); R06 §4's "3,396 × 64 B" is 3,397 pairs = 217,408 B; R01 and R02 quote the
March 2024 mini (13 GB) without the November 2024 figure (7.4 GB); the plan README's "cap 2" gap-fill scouts reads three. For the lead, with the status
update. · (13) Layer 9's research (incidents at `state/incidents.jsonl`, a per-record node-key signature, two tag spellings) gives way to §3. · (14)
`/api/status`: Layer 1 says it needs no session (`L1:162`) while the code gates it (`N/dashboard/routes.rs:16-17`, `:483-485`). The brief also cited
`routes.rs:28` (now `:33`) as calling it exempt; that line exempts it from the `localhost` redirect, not from the session (C13). A Layer 1/4 text fix; no dataset route
copies that posture (§16.7).

**Where the code overrules the brief or a report.** **C1** `state/relay.json` treats an unknown key as an invalid file: it is moved aside and every Layer 7
setting and consent falls back to off (`N/relay/settings.rs:21-26`). A Layer 8 key there would be read as invalid by an older release after a rollback. Layer
8's settings live in `state/storage.json`, which keeps and ignores an unknown key (§15.1). · **C2** Free space changes with any write, so "write the disk
figures only on change" (built since Layer 7 row M1c, `M/store/prune.rs:1216-1224`) still writes whenever anything else wrote on the disk: the figures are
not persisted at all (`M/store/prune.rs:1337-1350`, `:1279-1295`; `N/fabric/mod.rs:2386`). · **C3** The brief's lines moved since 2026-11-02 and are re-pinned here: `DEFAULT_HIGH_WATERMARK` `prune.rs:131` (was `:79`),
`save_capacity` `:1337-1350` (was `:869-888`), `PRUNE_EVERY` `N/fabric/mod.rs:183` (was `:115`), the capacity pass `:2235` (was `:1131`),
`apply_key_and_pragmas` `M/store/mod.rs:433-493` (was `:396-456`), `A/src/events.rs:19-21` (was `:3`, `:17-21`), `L2:91`, `:216-218`, `:234`, `:311` (were
`:85`, `:206-208`, `:224`, `:296`). · **C4** Layer 7's tags are already registered as A14 (`L2:449-456`, recorded 2026-11-11), so J5's "one amendment" carries
the tags of Layers 8 and 9 (§21 L2). · **C5** `Proposal.kind` holds nine kinds (`N/dashboard/api_v1/proposals.rs:104-114`, `relay` built) and the agent's
tool count is 20 at the pinned head (`A/tests/daemon_tools.rs:322`; Layer 7 row A1 added its three): every bump is stated relative, 20 → 23 (§16.5,
§16.6). · **C6** The fabric schema is v5 at the pinned head
(`M/store/schema.rs:76`; Layer 7 rows M1b and M1c committed); no signed release has shipped (`NI/manifest.rs:140-143`), so the migration copy's first field use is the first
migration after one (§13.6 item 4). · **C7** The probe has no storage-medium fact (`NI/install/probe.rs:70-75`, `:372`): §4.2's `keep` rule (D5). · **C8**
`proposal.write` takes only a title and a body (`A/src/tools.rs:113-121`): a typed proposal needs its own tool (D1). · **C9** The components cache is
root-owned (`L1:142`): component blobs are served in place, never copied into `state/origin/blob/` (§11.1; D18). · **C10** The chat relay drops the agent's
stream when the browser leaves (`N/dashboard/api_v1/relay.rs:585-612`): the relay task owns it (§2.3 rule 4). · **C11** The consents and the metered window
are stubs at the pinned head (`N/relay/consent.rs`, `N/relay/metered.rs:1-9`); the origin is built (Layer 7 row D7, `N/origin/mod.rs:1-37`; a stub when
the brief was written):
Layer 8's origin work builds on D7 and waits for Layer 7 rows D10 and D11 (§14). · **C12** mesh111's `rusqlite` has no `backup` feature (`5-messaging/crates/mesh111/Cargo.toml:17`): the export runs with the daemon
stopped (D6). · **C13** `N/dashboard/routes.rs:33` (the brief cited `:28`; the code at `:142-144`) concerns the `localhost` redirect, not the session gate (brief §4 item 14). · **C14** The Transit default
is a quarter of the custody quota when one is set, else of the high watermark (`M/store/prune.rs:52-66`, `:925-944`): left unset it would scale once the
budget sets the watermark, so the budget writes its effective value once (§4.2 item 4). · **C15** The agent
counts conversation files for its status (`A/src/store.rs:88-95`): `111d` fills the count from the store (§2.3 rule 6). · **C16** kiwix-serve wakes once a
second while it runs (R07g3 §1), but the panel's proxy and `111d`'s search route see every request, so an idle stop needs no timer of ours: a counter on
the existing capacity pass, reset by both, stops kiwix-serve after 30 passes with no request (§9.3 (3), §4.3 step 9; D21). · **C17** D37 (decided on 2026-11-11, after the brief)
fixed the cross-layer seam ids and the runner's map (`verify/seams.toml`, `verify/checks.toml`): §18 uses its Layer 8 ids and adds three rows,
`L6-L8:server-component`, `L5-L8:consumer-registry` and `L5-L8:child-panel`. · **C18** The brief's "reset account", the one control it gives
foreign databases (OB F3, `options.md:217`), already exists: Layer 6's "remove files" deletes `bridge/<adapter>/` with the account store
(`6-transport/README.md:153-154`), so §4.1 names it and no control is added.

**Deviations from D34 and the brief (each needs the owner's acknowledgement).** **D1** three agent tools (`storage.propose` beside the brief's two read tools;
C8) · **D2** the node-state store opens on use and closes on an idle pass (F1's cost line named a resident second connection) · **D3** Layer 8's switches live
in `state/storage.json`, not Layer 7's file (C1) · **D4** the disk figures are not persisted at all (C2) · **D5** the min(2 GiB, 15 %) rule applies to every
non-hub Linux node (C7) · **D6** export and import with the daemon stopped in v1 (C12) · **D7** tiles are served as `application/octet-stream` (no PMTiles
media type could be verified from this box), where the brief named "a `.pmtiles` content type" · **D8** a member checks its hub's catalogue on attribution
only while the hub's signed announce carries the new role bit `ROLE_DATASETS` (F11 A: "on the same link-attribution event"; §12.4), so a hub that serves
none costs nothing, and the bit is public to every card holder · **D9**
`ob_log = 12` below 1 GiB (the brief fixes 14 at 1 GiB and above) · **D10** two or more ranges answer 416 · **D11** the export's ciphertext uses Layer 5's
chunked object framing and its signature a BLAKE3 prehash (L2 §8.5(i) reads as one AEAD over the whole state) · **D12** with the node-state store closed, chat
is answered and not saved, and history answers `503 store_unavailable` (a new code) · **D13** the J2 file names and the J7 ring path are pinned here
(`state/heal/…`); the joint contract named none · **D14** a catalogue is published from a dashboard route, where `111d` holds the identity handle · **D15**
the log ring copies the tail and truncates, because the child holds an appending descriptor · **D16** J5's "one amendment" is the §13.3 block for Layers 8 and
9 (C4) · **D17** the watermark never drops below 512 MiB until a human sets `keep` (the inert default) · **D18** component payloads are served in place from
`cache/packages/`, where F4 says every public file lives in the tree (C9) · **D19** an update that migrates a store whose rollback copy does not fit above
the floor, or cannot be made inside the probation window, is refused before the flip (the updater state `refused_space`) unless a human accepts "update without a
rollback copy", after which a revert leaves that store closed until the next update (§13.6 (5); the lead's ruling on the panel's fork, both rules in that
order; F12 named the copy and no bound) · **D20** with every Layer 8 switch off the daemon holds one descriptor of Layer 8 for its lifetime, the `flock` on
`node/lock` (§13.2), so §19's first two columns read 1 open file where v0.1 read 0: no process, poll, timer or write is added · **D21** kiwix-serve
also stops on the existing capacity pass when no request reached its panel's proxy or its search route for 30 passes, and the next panel open starts it again (§9.3 (3); the
lead's ruling, by the Pi-floor rule), where F8 A says it is "stopped by the user or at daemon stop" · **D22** the lazy UI budget: after Layer 7's U2
the lazy bucket leaves 17.1 KB of D21's 256 KB (§15.4); Layer 8 splits its page (`l8`, `l8-hub`) and takes at most 12 KB of it; if its chunks cannot fit,
raising D21's budget is the owner's, and this spec assumes no raise · **D23** Q24's default reads "each measured on a Pi before the spec locks"
(`options.md:545-546`): the numbers instead stand as defaults at the lock and are measured on the owner's real-device run (D36 (3)), recorded in
`8-storage/TASKS.md`, since every lock gate is a netns gate · **D24** F8 A's "it may propose turning it on" (`options.md:324`) is not built: no
proposal kind enables a component (§9.4), so the agent says the encyclopedia is off · **D25** the brief's `kb.search` is named `encyclopedia.search`,
because `kb.` is the Layer 0 knowledge base's prefix in the agent (`kb.read`) and the budget (§9.4); the route stays `/api/v1/datasets/search`.

**Numbers.** Kept as the lead's defaults (Q24; defaults at the lock, measured on the owner's real-device run, §22 D23): `journal_size_limit` 4 MiB · four bulk streams · the `keep` percentages
· the 90 % low mark · `ob_log = 14` at 1 GiB and above. The drafter's own, for the panel: the shares (fabric 3/4, the node-state store 1/4, `heal` at most a
quarter of it) · `logs.max_file_bytes` 4 MiB · `ob_log = 12` below 1 GiB · kiwix's port 11250 · 1,000 entries and 1 MiB per catalogue · `encyclopedia.search` at
most 20 results and 8 KiB (R05's 8 KB) · `origin.max_streams` 1..=16. The lead's, from the panel's triage (defaults, measured at build): `history` the
newest whole messages up to 512 KiB · the agent's chat body limit 2 MiB · each `message` frame at most 512 KiB (§2.3) · a sidecar whose exact length exceeds
8 MiB refused (§5.2). From the panel's reports: the `logs` reserve at each cap × 1.5 plus 1 MiB (§4.2 item 3) · the witness inbox at the newest 256 lines
(J2). The drafter's, set in v0.2 (default; measured at build): a J9 `list` or `read_stream` call returns at most 1,000 rows. The lead's, from the
triage (defaults, measured at build): the carrier reserve a dataset job leaves in a metered window, 10 % of the rule's daily cap and at least 1 MiB
(§7.8) · kiwix-serve's idle stop after 30 capacity passes with no request (§9.3). From the panel's reports (defaults, measured at build): a blob body that
accepts no byte for 60 s is closed (§7.2) · the `updates`, `bridge` and `kb` walks kept for at most 60 s (§4.1) · a busy source retried after a gap of 1 to
32 passes, doubling (§4.3 step 6). The drafter's, set in v0.2 (default; measured at build): `get_to_file`'s stall bound, a range's length at a floor rate
of 64 KiB/s (§7.3 (1)) · an upstream window of 16 MiB per call (§7.3 (1), §7.5) · a size walk stops at 100,000 entries (§4.1). The drafter's, set in v0.2 from the integration report (default; measured at build): a
`dataset` frame at most once per 5 s per job, plus state changes (§7.4, §16.3) · Layer 8's lazy UI allowance, 12 KB gzip for its chunks, adapter and card
(§15.4). Set in v0.3 from the re-check (fixed values, not defaults; nothing in them is measured): `<publisher short>` is the first 16 hex characters of the
publisher's NodeId (§5.1, the lead's) · the daemon's exit status while a store verb holds `node/lock` is 75 (§13.2, the drafter's; `EX_TEMPFAIL`) · a
`version` matches `^[A-Za-z0-9][A-Za-z0-9_.+-]{0,39}$`, the 40 characters it already had (§5.3).

## Changelog

- 2026-11-11 (v0.1): first draft from D34, D36, the options brief, research R00-R07g3 and the code of head `f0b6806`, for the adversarial panel.
- 2026-11-12 (v0.2): the panel's three reports (store, datasets, integration; 92 finding ids) applied under the lead's triage (`review/triage.md`), each id
  mapped to its sections in `review/fix-map.md`; the forks ruled as the triage states (S5 with I22, D1 with D2 and I3, D29 with I5); citations re-pinned.
- 2026-11-13 (v0.3): the re-check of v0.2 (`review/recheck.md`) applied with the lead's ruling: the seven partly fixed ids (the sidecar's name proven,
  the re-pin finished at one head, the consent text's list) and R1-R21 (the adopt route's body, the update-without-copy route, the legacy files' own
  trigger, `Proposal.decision.applied`, the two set-aside routes, one dataset state enum, and the minors); the fix-map gains a v0.3 section.
- 2026-11-13 (v1.0): locked under D38: the second re-check (`review/recheck-2.md`: verdict lock) applied, its eight minors folded in and two editorial
  lines; no rule, route, field or gate added or renumbered; the citation pin stays at `8936a8e`.
