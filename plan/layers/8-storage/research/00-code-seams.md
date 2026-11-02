# Layer 8 research, lens 0: as-built code seams

Scout: opus seams lens (no web), 2026-11-02, base `30ffc1d`. Every claim was read on that base. Prefixes: `M/` =
`5-messaging/crates/mesh111/src/`, `N/` = `1-bootstrap/crates/node111/src/daemon/`, `NI/` = `1-bootstrap/crates/node111/src/`,
`A/` = `3-intelligence/crates/agent111/src/`, `UI/` = `4-interface/ui/src/`, `API` = `4-interface/api/openapi.yaml`,
`L1`/`L5`/`L7` = `1-bootstrap/README.md`, `5-messaging/README.md`, `7-relay/README.md` (v0.1 draft, not locked).

## 1. Summary

- **The encrypted store is real but holds only the fabric.** `fabric/store.db` is SQLCipher 4 with a raw key from a two-stage keyed
  BLAKE2b off the X25519 private half (`N/fabric/sealer.rs:44-47,126-138` then `M/keys.rs:101-114`). Agent state, display times and
  every adopted program's own database sit **outside it, in plaintext** (§3 (a), (e)). The L3 contract "in the encrypted local store"
  is not true as built.
- **Capacity pruning is built and correct in shape, but it is sized for messaging.** The high watermark is a fixed 512 MiB default
  with no API or daemon setter (`M/store/prune.rs:76`; `set_watermarks` has no caller). Blob files count as live bytes
  (`:468-481`). A ZIM or PMTiles file in `fabric/blobs/` would therefore be refused or evicted by the message store's budget.
- **The blob store and Tier 2 exist as a library with no production caller.** BLAKE3 + bao-tree outboards (1 KiB and 16 KiB groups)
  and private-object AEAD are built (`M/blobs.rs:1-25`). Nothing outside `mesh111`'s own tests calls `blob_put_*`. The `OBJ_*`
  kinds are not built, chunk frames are discarded, and `bao-tree 0.16.1`, `blake3 1.8.7` and `iroh-io 0.6.2` are already in the lock.
- **`SYNC` is a class and a kind, not a seam you can use.** An inbound `SYNC` (`0x14`) is stored as a **message row** in the
  sender's 1:1 thread (`M/fabric.rs:2596,2631-2700`), with no admission check. No API sends it, and the only stream is
  `STREAM_1TO1`. The "per-peer cursors" are the §4 `rx_state`/`tx_state`, plus the hub `cursors` table for the mailbox.
- **Hub serving surface:** static bundles stream with ETag but have **no Range support** anywhere in the tree (task brief: "verify"
  → absent). Files over 64 MiB are refused (`N/panels/static.rs:39,227`). The generic reverse proxy that a `kiwix-serve` panel
  needs is reserved (`N/panels/manifest.rs:12-13,398-403`). Components must name a bridge `adapter` (`N/components/manifest.rs:685`).
- **One idle write per minute already happens.** Every 60 s capacity pass commits `capacity.disk_free`/`disk_total` to `settings`
  (`M/store/prune.rs:645,777,866-888`; cadence `N/fabric/mod.rs:115,1259-1261`). That is an SD-wear cost in the required
  configuration today.

## 2. Candidates

None: lens 0 reads code only. Already in `1-bootstrap/Cargo.lock`: `blake3 1.8.7`, `bao-tree 0.16.1`, `iroh-io 0.6.2`,
`range-collections 0.4.6`, `positioned-io 0.3.5`, `rusqlite 0.40.2` / `libsqlite3-sys 0.38.2` (`bundled-sqlcipher-vendored-openssl`,
`5-messaging/crates/mesh111/Cargo.toml:17`), `ciborium 0.2.2`, `blake2 0.11.0`, `chacha20poly1305 0.11.0`, `sha2 0.10.9`+`0.11.1`,
`minisign-verify 0.2.5`, `tar 0.4.46`, `flate2 1.1.10` (rust_backend, `node111/Cargo.toml:45-46`). **Absent:** `iroh`,
`iroh-blobs`, `iroh-base`, `automerge`, `loro`, `yrs`, `age`, `zstd`, `zip`, `memmap2`.

## 3. Findings and seam tables

### (a) The encrypted local store

1. **Key.** `store_secret = BLAKE2b-256(key = keyset[32..64] (X25519 scalar), "111/store-secret/v1")` (`N/fabric/sealer.rs:70,126-138`).
   `store_key = BLAKE2b-256(key = store_secret, "111/store/v1" ‖ NodeId)` (`M/keys.rs:24,101-114`), handed in through
   `ModeS::new` (`N/fabric/mod.rs:446-461`). Mode T (TPM) answers `Unsupported` (`M/keys.rs:12-14`). The key is never on disk, and
   a copied DB reopens after restore-from-words (`:9-11`). The same derivation pattern makes `bridge_conv_key`
   (`N/mod.rs:192-212`). **A Layer 8 key for another store follows this pattern with a new tag. No new secret is needed.**
2. **Engine.** Raw key `PRAGMA key = "x'…'"`, `cipher_compatibility = 4`, a plaintext SQLite refused (`NoCipher`), a wrong key
   detected on page 1 (`M/store/mod.rs:396-456`). WAL, `temp_store = MEMORY`, `secure_delete = ON`, `foreign_keys`,
   `auto_vacuum = INCREMENTAL` (`:427-440`). `synchronous = NORMAL` by default and `FULL` for sequence allocation, epochs, the
   clean record, receipts and migrations (`:171-177`; `M/store/schema.rs:3-4,329`). Files are 0600 in a 0700 dir (`M/store/mod.rs:224-243`).
3. **Schema V1–V3** (`M/store/schema.rs:53,58`; V4 `custody.fwd` is Layer 7's, not built): `settings`, `contacts`, `messages`, `tx_state`,
   `outbox`, `rx_state`, `rx_frags`, `rx_floor`, `custody`, `cursors`, `blobs`, `requests` (V1), `pending_node_ids` (V2),
   `foreign_conversations`, `send_groups`, `foreign_messages`, `relay_edges`, `foreign_seen` (V3). Tables are STRICT. A newer
   `user_version` is refused, never downgraded (`:311-347`).
4. **Prune classes** (`M/store/rows.rs:29-56`): Control 0, Human 1, Agent 2, Kb 3, Update 4, Patch 5, Sync 6, Object 7, Custody 8,
   Foreign 9. Refusable: Kb, Update, Patch, Sync, Object, Custody (`:92-97`). Quota-able: those plus Foreign (shaping only, `:103-105`).
5. **"Capacity"** = DB pages minus freelist × page_size, plus `sum(disk_bytes)` of present blobs (`M/store/prune.rs:468-481`). The
   trigger fires above the high watermark or below 1 % free disk (`:3-6,108-121`). The default high is **512 MiB**, low = high − high/8
   (`:76,130-135`). Disk figures come from the daemon's `statvfs` on `fabric/` (`N/fabric/mod.rs:319-330,1131-1150`).
6. **Order as built:** quota eviction first (Foreign first, then the refusable classes) → step 1 committed custody, refetchable public
   blobs, delivered transit copies (weight size × `local_seq` age) → step 2 `superseded` blobs (marked by the consumer,
   `blob_mark_superseded`, `M/store/blobs.rs:328`) → 2a/2b foreign → step 3 own history only at hard capacity **and** after
   `set_export_offered` → step 4 never (outbound, uncommitted custody, requests, pinned, contacts) ⇒ `store_full`
   (`M/store/prune.rs:8-46,645-760`). **`set_export_offered` and `set_watermarks` have no caller outside `mesh111`** (grep), so in
   production step 3 never runs and the watermark is never configured. Only `set_quota` is wired (`M/fabric.rs:3819`).
7. **Admission.** `admit(class, bytes)` gates `blob_put_received` and custody puts (`M/store/blobs.rs:128`, `M/store/rows.rs:1018`).
   Messages never pass it (`M/store/prune.rs:805-807`).
8. **Write pattern.** Every received sequenced item commits FULL with its `n` (`M/store/rx.rs:688-699`). Each blob write is
   `sync_all` on the file plus an fsync of the temp dir and the root (`M/blobs.rs:444,466-488`). `secure_delete = ON` rewrites freed
   pages. **The idle node commits `settings` every 60 s** (finding 6 of §1). WAL NORMAL means no fsync per commit, but pages are still
   written.

| file:line | Exists | Seam | Layer 8 adds or changes |
|---|---|---|---|
| `M/keys.rs:85-92`; `N/fabric/sealer.rs:126` | `KeyProvider`, Mode S | a tag-separated KDF | the agent store key (new tag), or the agent moves into `store.db` |
| `M/store/prune.rs:76,523-540` | fixed 512 MiB watermark | `set_watermarks` (no caller) | a per-category default (disk-relative) and a setting; a decision on whether datasets count at all |
| `M/store/prune.rs:581-590` | export offer flag | never set | the "export before prune" offer (L5 §21, `age` export) |
| `M/store/prune.rs:866-888` | disk figures persisted every pass | `save_capacity` | write only on change (cut the idle per-minute write) |
| `M/store/schema.rs:58` | `MIGRATIONS = [V1, V2, V3]` | append-only | V5+ (after L7's V4) for any L8 table: a log, a catalogue, cursors |

**Plaintext outside the store (as built):** `agent/` (conversations, proposals, tasks, `events.jsonl`, logs, §(e)) ·
`fabric/display-times{,.foreign}` (id → unix seconds, `N/fabric/times.rs:7-8,29-31`) · `fabric/announce` and `fabric/settings`
(`N/fabric/sealer.rs:49-50`) · `fabric/presence/meshtastic.json` (L1:200) · `state/panels.json`, `state/panel-sessions.json`
(0600), `state/bridge.json` (0600, consent) · `bridge/<adapter>/` (Delta Chat accounts, `N/bridge/deltachat.rs:1935-1947`; Madmail
state) · `rns/` (rnsd) · `identity/` (`mnemonic` and `keyset` 0600 plaintext, `2-identity/README.md:294`) · **public blobs are
stored plain by design** (`M/blobs.rs:4-6`; L5 §10.3).

### (b) Dataset hosting on a hub

9. **No Range anywhere.** A grep for `RANGE`/`Content-Range`/`206` across the Rust tree finds nothing. `panels/static.rs` streams in
   64 KiB chunks with an inode/mtime/len ETag, `no-cache`, GET/HEAD only (`N/panels/static.rs:1-24,39-42,221-250`). Files above
   **64 MiB** answer "unavailable" (`:227`). No `.zim`/`.pmtiles` type exists (`N/dashboard/routes.rs:319-339`, octet-stream fallback).
10. **Auth posture.** The dashboard app has an `open` router (`/`, `/health`, the UI catch-all) and a `gated` one
    (`N/dashboard/routes.rs:466-486`); `/api/v1/*` carries its own session gate. Panel listeners always need a `p111_<id>`
    cookie, with no loopback skip (`N/panels/listener.rs:4-19`). L7 §9.1 puts the origin routes on the existing listener,
    unauthenticated, only while `hub_role` **and** LAN sharing **and** `origin.serve` are on, with blobs under `state/origin/`.
11. **Hosting an adopted server.** Panels: `static` only. `child`/`detected` (the reverse proxy a `kiwix-serve` would need) refuse
    registration (`N/panels/manifest.rs:12-13,398-403`). Components: payload `pip`, `binary` (one file or a `.tar.gz` of one) or `none`
    (`N/components/manifest.rs:14-32`); `adapter` must be a network id (`:134,685-686`). The cache is `cache/packages/<id>/`, which is
    **root-owned** (L1:237-247). Enable never downloads (`N/components/mod.rs:36-40`). Zero weight is held by G7.
12. **Category facts.** 4A = ≥ 512 GB free on `/var/lib/111` and ≥ 8 GB RAM (`NI/install/categorize.rs:102-104`;
    `NI/install/probe.rs:372`), re-probed into `state/facts.json` at daemon start (`N/mod.rs:106`).

| file:line | Exists | Seam | Layer 8 adds |
|---|---|---|---|
| `N/panels/static.rs:203-250` | streaming file response | `file_body(file, len)` | a `Range: bytes=a-b` → 206 arm (single range; PMTiles and Kiwix readers need it), a size cap per dataset, not 64 MiB |
| `N/dashboard/routes.rs:466-486` | open + gated routers | `open.merge(...)` | `/datasets/v1/...` (or under L7's `/origin/v1/`) on the same listener, posture decided by the owner |
| `N/components/manifest.rs:134,685` | `adapter` required | manifest schema | an optional `adapter` (a dataset server is not a network), or a separate `kind` |
| `N/panels/manifest.rs:398-403` | proxy reserved (P7) | `SourceKind` | only if `kiwix-serve` is adopted as a panel; otherwise our Range route plus a JS reader |
| L7 §9.1 `state/origin/blob/` | origin store (draft) | sha256-addressed | one object store for releases and datasets (§(c) finding 15) |

### (c) Object transfer

13. **Tier 2 as specified** (L5 §5 Tier 2, §11): sealed `OBJ_ADVERT {hash,size,chunk_group,coding,object_key?}`, sealed
    `OBJ_REQ {hash,ranges,session_tag}`, then `OBJ_CHUNK` frames that travel unsealed for public objects on `tcp-lan` frame `0x05`.
    **As built:** kinds `0x20`–`0x22` are registered and `built_in_v1() = false` (`M/envelope.rs:515-522,601-607`). An inbound one hits
    `_ => {}` (`M/fabric.rs:2616`). `FrameType::Chunk` is discarded on receive (`M/fabric.rs:2388`) and refused on send
    (`M/link/tcp_lan.rs:24-25,225-233`).
14. **Blob layer:** `<fabric>/blobs/<blake3hex>/{data,ob1k,ob16k}`. The root is the same for both chunk groups
    (`M/blobs.rs:1-10,97-120`). `write_verified` refuses bytes whose BLAKE3 does not match (`:350`). Private objects use
    XChaCha20-Poly1305 over 64 KiB chunks, with a random key minted inside (`:12-21`). Rows carry `refcount`, `pinned`,
    `refetchable`, `superseded`, `present` (`M/store/schema.rs:170-184`). A temp-dir sweep runs at open (`M/store/mod.rs:262`).
    **No caller in `node111` or `agent111`.**
15. **Two hash spaces.** Releases are pinned by **sha256** (`NI/manifest.rs:154-159`); L7's origin is `blob/<sha256>`. Fabric blobs
    are **BLAKE3**. The L1 "updater intake at `fabric/blobs/<hash>`" (L1:222-223; L5 §11) is **not built**: grep finds no `blobs`
    in `N/updater/` or `NI/update_helper.rs`. A dataset served by HTTP and fetched over Tier 2 needs both hashes in its
    manifest, or one store keyed by one hash.
16. **The simplest transfer already exists:** the updater's fetcher streams `GET` with a byte cap and reads the payload **whole into
    memory** (`N/updater/fetch.rs:100,161`; cap 512 MiB). That is unusable for multi-GB datasets: it would need streaming to disk
    plus resume (`Range`), which the fetcher does not do (`NI/util/http.rs:14-47`).

| file:line | Exists | Seam | Layer 8 needs |
|---|---|---|---|
| `M/envelope.rs:515-522,601-607` | kinds pinned, unbuilt | the `built_in_v1` gate | if Tier 2 is Layer 8's: build `OBJ_*` (L7 D30 F10 lists Tier 2 as v1.1 and names no owner) |
| `M/link/mod.rs:136-137`; `tcp_lan.rs:225-233` | `Chunk` 0x05 reserved | frame type | the encoder and decoder plus a session table; LoRa never (`0-knowledge/concepts/lora-airtime-budget.md`) |
| `M/store/blobs.rs:56-130` | `blob_put_public/private/received` | the store API | the producer (catalogue import, a dataset fetch) and the `superseded` marking |
| `N/updater/fetch.rs:161` | whole-body GET | `Fetcher` | a streaming, resumable `get_to_file(url, range, sha256/blake3)` for hub-to-hub or a USB import |

### (d) The `SYNC` seam

17. `Kind::Sync = 0x14` is per stream, bulk, **not** SIGNED, not public-class (`M/envelope.rs:514,557-585`). `Class::Sync = 6`,
    refusable (`M/store/rows.rs:44-45,92-97`).
18. **Receive path.** It falls into `k if k.carries_msg_seq() => self.on_data` (`M/fabric.rs:2596`). From a non-contact it is
    dropped (`:2637-2639`). From a contact it becomes a `messages` row of class Sync, through `receive_message` (FULL, no
    admission), in that peer's thread. `message_view` decodes text only for `MSG` (`:4748-4756`), so the UI would show a
    textless row. The same holds for inbound `KB`/`UPDATE`/`PATCH`/`AGENT_OP` today.
19. **Send path:** none. The fabric's only producer is `send(to, text)` (`M/fabric.rs:856`), on `STREAM_1TO1` only
    (`:3165,3175`). Tier 1 bounds one item to 16 fragments, and `MAX_BODY` is 1 MiB (`M/store/rows.rs:17,27`).
20. **Cursors.** Only (a) `tx_state`/`rx_state` per (peer, stream, epoch), with prefix and ranges (`M/store/schema.rs:104-153`;
    `M/store/rx.rs`), and (b) the member's `cursors(hub, cursor)` over the hub's `custody_seq` (`schema.rs:155-168`). Nothing keys
    by "log" or "topic". Pairwise sealed delivery means a log must fan out per peer: no multicast object exists. (Hubs see only `dest`
    and `pkt_ref`; this is the L7 lens-0 finding 13, re-read here.)

| file:line | Exists | Seam | Layer 8 needs |
|---|---|---|---|
| `M/fabric.rs:2590-2617` | kind dispatch | a `Kind::Sync` arm | route `SYNC` to a log consumer instead of `on_data`'s thread rows (an L5 amendment) |
| `M/fabric.rs:856` | `send(to,text)` | public API | `send_sync(to, stream, bytes)` or an opaque `send_kind`; allocate stream ids |
| `M/store/schema.rs:104-111` | `tx_state(peer,stream)` | per-stream `n` | gives exactly-once delivery per (peer, stream) for free; the log's own ids sit above it |
| `M/store/prune.rs:484-511` | `class_bytes(Sync)` counts `messages` rows | quota | quota eviction only picks blob/custody candidates (`:227-270`), so Sync message rows can be refused by nothing and evicted by nothing except history step 3 |

### (e) Agent and task state

21. `111-agent` gets `AGENT111_STATE_DIR = <root>/agent` (`N/intelligence.rs:224,251`; default `/var/lib/111/agent`,
    `A/config.rs:97,155`). `Store::init` makes `conversations/`, `proposals/`, `tasks/` at 0700 (`A/store.rs:55-70`).
22. **Formats.** Conversations are `conversations/<id>.jsonl`, append-only and **unbounded** (`A/store.rs:82-117`). Proposals
    are `proposals/<id>.json` and tasks `tasks/<id>.json`, written with `std::fs::write`: **not atomic**, no fsync (`:120-146`).
    `events.jsonl` is a 1 MiB FIFO (`A/events.rs:3`). Logs go to `agent/{111-agent,llama-server}.log` (`N/intelligence.rs:274-280`).
    Decisions are `agent/decisions/<id>.json`, written atomically by 111d (`N/dashboard/api_v1/proposals.rs:13-16`). All of it is
    **plaintext**, with no pruning; `A/store.rs:2` says "Capacity-pruning policy is Layer 8's".
23. Proposals are untrusted input to 111d (size-capped, the id checked against the file name, `proposals.rs:4-19`). A
    `kind: message` acceptance writes the decision twice around the send (`N/fabric/proposal.rs:17-22`).

| file:line | Exists | Seam | Layer 8 adds |
|---|---|---|---|
| `A/store.rs:49-178` | a `Store` struct (dir only) | one module owns every write | swap the backend: an encrypted SQLite (a second SQLCipher DB keyed by a new tag), or `age`-sealed files |
| `N/dashboard/api_v1/proposals.rs:133-134` | 111d reads `agent/proposals` | a file contract | must stay readable by 111d: a shared key or an API, never two decoders |
| `A/events.rs:3` | 1 MiB FIFO | capacity-bounded | the model for conversations: total-bytes cap, oldest conversation first |

### (f) The UI

24. Settings `l8` is the generic `LayerPage` (`UI/views/settings/index.tsx:16-38`). Its rows say `"arrives with Layer 8"`
    (`UI/views/settings/model.ts:221-222`). The layer copy is at `UI/lib/layers.ts:66-70`. "storage (L8)" is in L4's deferred list
    (`4-interface/README.md:697`).
25. **Disk facts exposed today:** only `FabricStore{open, used, watermark, store_full, restore_pending}` (`API:1026-1035`;
    `M/fabric.rs:3628-3630`), drawn by `UI/views/services/store-gauge.tsx:1-25` and the l5 row (`model.ts:216`). There is **no
    filesystem free/total** in the API: the `statvfs` figures live in `settings` only. Components report `cached_bytes` and
    `installed_bytes` (`N/components/registry.rs:103-107,169`). The "4A" free-storage fact is in `state/facts.json` only.
26. **Consent precedent:** `409 consent_required` with the exact notice text and its hash, recorded in `state/bridge.json`
    (`N/components/mod.rs:13-15`; L1:236-238). A dataset download dialog reuses this shape. The metered-link byte cap is L7 v1
    (D30 F10), not built.
27. **Budget:** eager and lazy are 256 KB each, every lazy chunk ≤ 64 KB gz (`4-interface/ui/scripts/check-budget.mjs:1-7,13-14`). Last
    figure: eager 106.5 KB, lazy 219.9 KB (`6-transport/TASKS.md` FC4). **The exempt "map" bucket is a recorded amendment
    (D30 F9; L7 §10.5), not built**: the script has no exemption. A Storage page needs its own lazy chunk (the `l6` precedent,
    `index.tsx:13`).

| file:line | Exists | Seam | Layer 8 adds |
|---|---|---|---|
| `UI/views/settings/index.tsx:18-20` | l5/l6 special cases | the route switch | `if (layer === "l8") return <StorageSettings/>` as a lazy chunk |
| `API:1026-1035` | `FabricStore` | additive fields | `disk_free`, `disk_total`, per-class bytes, the configurable watermark; a `/api/v1/datasets` catalogue plus consent |
| `check-budget.mjs:13-14` | no exempt bucket | buckets | the "map" exemption (Leaflet + PMTiles ≈ 69 KB gz, L7 §10.5) |
| `A/tools.rs:184` `fabric.read` | 17 tools | the tool table | a `storage.read`/`datasets.read` tool; bump both pins (L7 lens 0 §3.1 (f)) |

### Timers and disk measurement (re-verified against L7 R00 §3.1 (a); lines shifted slightly, periods unchanged)

| Timer | file:line | Period |
|---|---|---|
| daemon tick | `N/mod.rs:902` | 2 s |
| transport mirror | `N/mod.rs:869` | 5 s |
| probation monitor | `N/mod.rs:136` | 5 s |
| services probe; WAN probe | `N/services/probe.rs:43,46,570` | 2 s; 60 s |
| devices registry | `N/devices/registry.rs:100`; `N/devices/mod.rs:410` | 5 s |
| panels reconcile | `N/panels/mod.rs:62,654` | 2 s |
| Meshtastic monitor and revocations | `N/meshtastic/mod.rs:186,495,650` | 2 s each |
| fabric housekeeping (prune every 12th) | `N/fabric/mod.rs:115,177,1252-1265` | 5 s (prune 60 s) |
| watchdog | `N/watchdog.rs:12,41` | 15 s |
| rnstatus facts | `N/transport/rnstatus.rs:945` | 30 s |
| updater | `N/updater/mod.rs:22-23` | 1 h |

D30 F2 removes the WAN probe and trims the 2 s Meshtastic and panels ticks; that is **not built yet**. Layer 8 adds no timer if it hangs
dataset bookkeeping on the existing 60 s capacity pass, or on events only.

**Disk is measured in:** the fabric's `statvfs` (`N/fabric/mod.rs:319-330`, every 60 s); `live_bytes`/`class_bytes`
(`M/store/prune.rs:468-511`); `tree_bytes` for components (`N/components/registry.rs:112`, on request); the installer's
`free_bytes("/var/lib/111")` (`NI/install/probe.rs:372`). Nothing measures `agent/`, `bridge/`, `rns/`, `releases/` or `cache/`.

## 4. Traps (they look like seams, but are not)

- **"Static files with Range support exist"** is not true: there is no Range anywhere, and the static cap is 64 MiB (finding 9).
- **`fabric/blobs` as the dataset store:** it counts toward the 512 MiB message-store watermark and is plaintext for public objects.
  It has no production caller, and it is BLAKE3-addressed while releases are sha256 (findings 5, 14, 15).
- **The "updater intake at `fabric/blobs/<hash>`"** is a sentence in L1 and L5, not code (finding 15).
- **`SYNC` "per-peer cursors"**: these are only §4 sequencing. An inbound SYNC becomes a textless chat row, bypasses admission and
  escapes quota eviction (findings 18, 20).
- **`Class::Kb`/`Update` quotas** shape blobs and message rows, but only blob candidates are evictable (finding 20 table).
- **"Agent state lives in the encrypted store" (L3 README:50)**: as built it is plaintext, unbounded JSON (finding 22).
- **`set_export_offered` and `set_watermarks`** exist but nothing in the daemon calls them. Step 3 of §10.4 is dead in production (finding 6).
- **The components cache is root-owned** (`cache/packages/<id>/`). The daemon cannot write a downloaded dataset there; a daemon fetch
  needs a node111-owned path such as `state/` or `fabric/` (L1:113-117).
- **`/api/status` is gated** in code (`N/dashboard/routes.rs:472-478`), although the L1 text and `routes.rs:28` call it exempt. Do not
  copy that posture for a dataset route without reading both.
- **The KB `[kb]` manifest pin** is parsed and never consumed (`NI/manifest.rs:541`; L7 §9.4). The KB rides only in the payload
  (`N/kb.rs:1-3`). `build.py` is CI-only (`.github/workflows/ci.yml:112-113`): the node ships `llms.txt` files and runs no Python
  for the KB. Python exists only where rns or a pip component is installed.

## 5. Test counts (as recorded; nothing run)

`mesh111` 166 lib + 25 integration/doctests · `id111` 195 + 44 + 4 + 7 · `agent111` lib 19 plus suites (layer6_tools 20, daemon_tools 15,
chat_loop 17, …) · `node111` 34 suites green with the 8 known root-only failures (FC gates) · UI vitest 1418 (FC4) · LXMF helper 128 ·
G1 132, G3 147, G4 128, G5 98, G7 GREEN, smoke 118, three-node 159, panel-lint 144. Sources: `6-transport/TASKS.md` V4/FC rows,
`plan/layers/7-relay/research/00-code-seams.md` §3.3.

## 6. Recommendation (seam-level forks for the brief)

- **Where datasets live.**
  - (A) A separate node111-owned tree `datasets/<id>/` with its own capacity rule, outside `store.db`'s watermark. The datasets are
    public, signed, plaintext files.
  - (B) `fabric/blobs` with a raised watermark: one pruner, but message-store pressure and dataset bytes compete.
  - (C) L7's `state/origin/` extended into the single object store.

  **Pick A, shared with L7's origin** (one sha256-pinned object directory serves releases and datasets). The dataset index joins the
  message pruner only as a "capacity reporter".
- **Agent state.**
  - (A) A second SQLCipher DB `agent/state.db`, keyed by a new tag through `KeyProvider`. It needs no new dependency, but 111d must read
    proposals through it.
  - (B) Move it into `fabric/store.db` as V5 tables.
  - (C) Keep the files, add capacity caps, and leave encryption off.

  **Pick A** for the encryption the L3 contract promises. The cap rule copies `events.jsonl`'s FIFO.
- **`SYNC`.**
  - (A) An L5 amendment: a `Kind::Sync` dispatch arm plus `send_kind`, delivering opaque bytes to an adopted log engine.
  - (B) Leave `SYNC` reserved; a log rides HTTP between hubs.

  The adoption lenses decide. Either way, fix the textless-row trap.
- **The Range route:** a small single-range arm in `panels/static.rs` reused by the dataset route. No dependency is needed.

## 7. Open questions for the owner

1. May the store's high watermark become disk-relative per category (4A hubs today are capped at 512 MiB), and may Layer 8 wire
   `set_watermarks` and the export offer?
2. Do datasets count toward the encrypted store's capacity, or get their own budget (and does pruning a dataset need consent)?
3. Must agent state be encrypted at rest, as L3 promises, even though that adds a second SQLCipher DB to the agent (build weight in
   `agent111`, today without SQLCipher)?
4. Should the idle per-minute `settings` write be cut to on-change only (an L5 edit, SD wear)?
5. Who owns Tier 2 (`OBJ_*`)? Layer 7 lists it as v1.1 and names no owner. Layer 8 is the first consumer with bulk.
6. Is the dataset route unauthenticated on the LAN (the L7 origin posture), or pairing-gated (kiwix/map readers in a browser
   session)?
7. Should inbound `KB`/`UPDATE`/`PATCH`/`SYNC`/`AGENT_OP` stop landing in chat threads now (an L5 bug-level amendment), before any
   producer exists?

## 8. Sources (files read)

Every file cited above, plus `5-messaging/README.md` §3.2, §5, §10.3–10.4, §11, §19, §21; `1-bootstrap/README.md` "On-disk layout" and
the L5/L6 amendments; `7-relay/README.md` §9–10; `plan/DECISIONS.md` D30; `plan/layers/7-relay/research/00-code-seams.md`;
`6-transport/TASKS.md` (Pi floor, FC rows); `plan/INDEX.md` rows 6–9.
