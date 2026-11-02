# Layer 8 — Storage & Shared State: options brief

Status: 🧭 deciding — awaiting the owner (sweep complete, run ids in research/coordinates.md) · sweep: the seams scout, five lens scouts, the
critic and three gap-fill scouts (G1 agent-state encryption `07-gap-1.md`; G2 software updates beyond the release `07-gap-2.md`; G3 the chunk
sidecar and G4 Pi prior art `07-gap-3.md`); the critic's digest, the gap-fill digest and the unverified-claims list are §6 · written for the lead
from the ten reports in `research/` (`00-code-seams.md` line-read on `30ffc1d`; the others web-checked where this box's proxy allowed: package
indexes and raw GitHub files only, most documentation sites denied, so every claim a scout could not fetch is marked in its report and in §6).
Reads with: `os/README.md` §8, the category table and the principles · the hand-offs in [`README.md`](README.md) · D17, D20, D21, D22, D24,
D26, D29, D30, D31, D32 · `AGENTS.md` (the Pi floor) · the Layer 9 sweep (`../9-evolution/research/`) for the joint contract (F2).

Owner scope: Layer 7 is locked (v1.0, D32, 2026-11-02; its spec is cited here by section) and starts building; Layers 8 and 9 were researched side
by side from 2026-11-02. `INDEX.md` "Process (per layer)": research → this brief → the owner's decision → spec → adversarial panel → build.
**No Layer 8 code is written before the owner decides.** F2 is **joint with Layer 9**: the Layer 9 brief carries the same seven items verbatim
(the lead diffs the two boxes before the decision sheet goes out), and the two decisions are taken together.

## 0. What the research settled (facts that bound every option)

1. **The encrypted store holds the fabric and nothing else.** `fabric/store.db` is SQLCipher 4 with a raw key derived from the keyset
   (`M/keys.rs:101-114`). Agent state (conversations; proposals, where a `kind: message` proposal holds the outbound text; tasks; decisions),
   display times, the Delta Chat core's own database, `rns/`, `state/*.json` and `identity/` are plaintext behind 0600/0700 ownership
   (00 §3 (a); 04 finding 1). Layer 3's contract "in the encrypted local store" (`3-intelligence/README.md:50`) is not true as built.
2. **What encryption at rest can buy here, and what it cannot.** A new store needs only a new key-derivation tag, no new secret, and its key is
   recoverable from the 12/24 words (00 finding 1). The words sit in `identity/mnemonic` (0600, plaintext) and the agent runs under the
   daemon's uid (07-gap-1 finding 10), so encryption protects copied files, backups and synced directories, never a stolen disk with a readable
   identity directory and never a compromised agent (04 finding 2). The TPM mode answers `Unsupported` (`M/keys.rs:12-14`).
3. **Both sweep mechanisms for encrypting agent state break a locked rule; a third breaks none (critic conflict 2).** SQLCipher inside
   `agent111` gives the agent key material (`2-identity/README.md:224`: "the agent holds **zero** key material") and costs +5.4 / +6.4 MB on
   aarch64 / x86_64 (`5-messaging/README.md:421-422`) plus a C toolchain in a pure-Rust agent; agent writes over its bearer break D26 (2)
   (`3-intelligence/README.md:119-122`). Neither scout saw that 111d already relays every chat turn and history request
   (`N/dashboard/api_v1/relay.rs:614-736`) and reacts to `proposal.written` and `task.done` (`:797-835`), so 111d can be the only writer of an
   encrypted copy (07-gap-1 §1).
4. **Capacity pruning is built for messaging only.** The high watermark is a fixed 512 MiB (`M/store/prune.rs:79`); `set_watermarks` and
   `set_export_offered` have no caller outside the tests (grep), so L5's "export before prune" step never runs in production; blob files count as
   live bytes; nothing measures `agent/`, `bridge/`, `rns/`, `releases/` or `cache/`; the API exposes no filesystem free or total (00 findings
   5, 6, 25; 05 finding 2).
5. **The idle node writes once a minute.** The capacity pass (every twelfth 5 s housekeeping tick, `N/fabric/mod.rs:115`) commits `store_full`,
   the full-class mask, `disk_free` and `disk_total` every time (`save_capacity`, `M/store/prune.rs:869-888`): 5.9 MB a day, 2.2 GB a year of
   logical WAL writes, derived, not measured (04 finding 7). The agent's child logs are opened create-and-append with no cap
   (`N/supervise.rs:752-767`; 04 finding 4).
6. **A probation rollback can leave messaging down.** Release N migrates the store, fails probation, and N−1 refuses the newer `user_version`
   (`SchemaTooNew`); the fabric boots to an error and no pre-migration copy exists (04 finding 12). Layer 7's V4 migration (`custody.fwd`) is the
   first that can hit this.
7. **Two backup formats are specified for one job (critic conflict 8).** L2 §8.5(i): encrypt to the node's own X25519 key, signed by the node key,
   signature checked before decrypting, hub-storable, never public. L5 §10.4 step 3 and §21: an `age` export. `age` 0.12.1 (MIT OR Apache-2.0,
   licence files read) pulls `hpke`, `ml-kem`, `p256`, `scrypt` and `i18n` into its default tree and carries no sender signature (04 finding 14).
8. **The blob store and Tier 2 exist as a library with no production caller.** BLAKE3 addressing, bao-tree outboards and private-object
   encryption are built (`M/blobs.rs`); the `OBJ_*` kinds are registered and unbuilt; the chunk frame is discarded; `bao-tree 0.16.1` is locked
   while 0.17.0 shipped on 2026-10-22 with fixes on the range paths Tier 2 uses (00 findings 13-14; 02 finding 6). L5's blob writer always writes
   both outboards (`ChunkGroup::ALL`, `M/blobs.rs:96`): 3.56 GB of `ob1k` plus 222.7 MB of `ob16k` for a 57 GB file (07-gap-3 finding 8).
   Datasets must not pass through it.
9. **Two hash spaces.** Releases and Layer 7's origin are addressed by sha256 (`NI/manifest.rs:154-159`; L7 §9.1), fabric blobs by BLAKE3. The
   "updater intake at `fabric/blobs/<hash>`" of `1-bootstrap/README.md` and L5 §11 is not built (00 finding 15).
10. **Nothing in the tree can move a multi-GB file.** No `Range` handling exists anywhere; a static panel file above 64 MiB answers "unavailable"
    (`N/panels/static.rs:39,227`); `Fetcher::get` returns the whole body in memory under one global timeout, capped at 512 MiB for updates
    (00 findings 9, 16; 02 finding 1). A single-range arm is estimated at 60 to 120 lines with no dependency (05 finding 17; L7 §10.5).
11. **The in-lock bao-tree outboard is the dataset sidecar (G3).** At 16 MiB blocks (`BlockSize::from_chunk_log(14)`) an outboard is
    `(blocks − 1) × 64` bytes: 28,224 B for a 7.4e9 B file, 49,536 B for 13e9 B, 217,408 B for 57e9 B (3,398 blocks), 423,424 B for 111e9 B. The
    32 B BLAKE3 root in a signed entry authenticates the sidecar alone (`valid_outboard_ranges`) and each fetched block (`valid_ranges`), both in
    0.16.1; the root is the same at every block size, so Tier 2 can adopt the file later. Each active verify call holds a 16 MiB buffer (4 MiB at
    `from_chunk_log(12)`, where the sidecar for 57 GB is 869,696 B) (07-gap-3 §3 findings 1-6, 9). Lens 2's own sha256 list would be 108.7 KB for
    57 GB but needs its own verifier and has no Tier 2 path (02 finding 4).
12. **Nobody upstream signs datasets.** Kiwix's OPDS entry has no hash, licence or signature field (`catalog_v2_entry.xml`, read); a `.sha256`
    sits beside each ZIM (search snippet); PMTiles has no integrity field; IIAB's map catalogue has no hash field (01 finding 4; 07-gap-3 §4
    finding 8). TUF expires metadata by wall-clock date; F-Droid's `entry.jar` (one small signed object pinning big files by hash and size, any
    mirror acceptable) is the closest shape (01 finding 10).
13. **Dataset sizes are dated and secondary, so the spec states none (critic conflict 7).** English Wikipedia: about 110 GB with pictures and
    57 GB without (Nov 2024), about 111 GB (2025-08); the introductions-only "mini" 13 GB (Mar 2024) or 7.4 GB (Nov 2024), the second fitting
    Kiwix's "about 95 % smaller"; a planet basemap about 120 GB; Berlin 84 MB; IIAB's default maps about 312 MB; IIAB's z14 regional extracts
    13 to 119 MB (01 finding 6; 05 finding 9; 07-gap-3 §4 findings 4, 8; search snippets or third-party files, none from a live listing). A ZIM
    refresh is a whole new file, so a refresh needs room for both copies or removal first (01 finding 7).
14. **kiwix-serve is both the encyclopedia and the agent's search, and it costs while it runs.** GPL-3.0-or-later C++ (COPYING read), HTTP only,
    no authentication, with `/search`, `/raw` and `/catalog/v2`; a Termux package (3.8.2). Its main loop wakes every 1000 ms whether or not
    anyone reads, and `SIGHUP` reloads its library (`kiwix-serve.cpp:423,451`, read upstream by the gap-fill scout). No source publishes its
    memory use: the only primary figure is IIAB's "kiwix-serve needs >= 1024MB on Zero 2 W" (RAM plus a 1,536 MB swap file,
    `default_vars.yml:259-260`); 01's 128-256 MB and 05's "under 50 MB" are anecdotes (07-gap-3 §4 findings 3, 5, 6). A ZIM can run JavaScript
    in the reader's browser (kiwix-js README; 01 finding 5).
15. **Our panel host cannot host it yet.** It registers `kind = "static"` panels only; `child` and `detected` (the generic reverse proxy) are
    refused at registration (`N/panels/manifest.rs:12-13,398-403`); a component must name a network `adapter` (`N/components/manifest.rs:685-686`)
    (00 finding 11; 05 finding 11).
16. **The map bundle Layer 7 recorded is low (critic conflict 3).** Leaflet 1.9.4 with protomaps-leaflet 5.1.0 is 80.4 KB gzip, 83.9 KB with its
    CSS; raster Leaflet with pmtiles 4.5.0 is 54.3 KB; MapLibre GL 451 KB (the lens scout's own esbuild measurement); L7 §10.5 says about 69 KB.
    The lazy bucket holds 219.9 of 256 KB with a 64 KB per-chunk cap (05 finding 13; 00 finding 27).
17. **What a hub operator takes on under the content licences (facts from the texts).** Serving a ZIM is "Sharing" under CC BY-SA 4.0 s.3(a)
    (creator, notices, licence link); a repack or subset is arguably Adapted Material under s.3(b) (legal read unverified). Serving OSM-derived
    tiles to a neighbourhood is "Publicly" conveying under ODbL s.1 and s.4.2; a publicly used Produced Work needs the notice "Contains information
    from OpenStreetMap, which is made available here under the Open Database License (ODbL)." (s.4.3) (01 finding 12; 05 finding 15). IIAB names
    OpenStreetMap but never prints that sentence, and ships CC BY-NC-SA satellite imagery as an opt-out (07-gap-3 §4 finding 9).
18. **Components have no offline route but a USB drop, yet their pins are already origin-independent (G2).** Each pin (URL, sha256 and size, or
    a hash-locked lock) is embedded in the signed `111` binary, so a hub can serve the bytes as `blob/<sha256>` with no new trust anchor. Delta
    Chat core plus Madmail are 104.1 MB (x86_64) or 94.5 MB (aarch64); the LXMF closure is 5.83 MB of wheels, but a pip lock holds hashes, not
    file names, so a files table is needed; Termux debs and APKs are pins a provisioning host consumes, and APKs cannot be re-signed (07-gap-2
    §3.1-3.3; `1-bootstrap/README.md:227-246`).
19. **A hub that mirrors a third-party binary is the one who conveys it.** GPL-3.0 §6(d), GPL-2.0 §3, MPL-2.0 §3.2(a), AGPL-3.0 §6 (its §13 only
    for a modified program), Apache-2.0 §4 and the Reticulum License's notice clause each bind whoever hands over a copy. D26 (5) ("the user
    installs; 111 and its author are not the distributor") does not cover a third person's disk, and the same duty already exists, unrecorded,
    for the GPL-3.0 `111` payload the Layer 7 origin serves (07-gap-2 §3.4 items 15-22). The Reticulum License text is now read (1,510 B);
    `LICENSING.md:56` still calls it unverified.
20. **Nothing in v1 needs a shared log.** Every state that crosses nodes has one writer and is ordered by a signed counter (the announce, the
    release and its KB, a dataset catalogue, the mesh-epoch beacon), and local state must not leave the node (03 findings 1-2). `SYNC` is a kind
    and a class, not a usable seam: an inbound `SYNC` from a contact becomes a textless chat row with a full-durability commit, no admission check
    and no prune step, and inbound `KB`, `UPDATE`, `PATCH` and `AGENT_OP` do the same today (00 findings 17-20; 03 finding 10).
21. **If a log is ever needed, the adoptable shape is a per-author signed chain.** p2panda-core 0.7.1 (2026-08-21; MIT OR Apache-2.0, licence
    files read; pre-1.0; rust-version 1.96) uses our primitives (Ed25519, BLAKE3, CBOR) and prunes at author-written points; using L5's epoch as
    the log id avoids a forked log after a restore from words. CRDT documents (Loro, Automerge, yrs) keep history and need every peer to agree on
    a compaction point, which is the consensus README §8 forbids (03 findings 6-9, 13).
22. **The metered cap Layer 7 locked counts carrier bytes only (critic conflict 5).** L7 §12: a rolling 24 h window on `CLOCK_BOOTTIME`, persisted.
    At a 1 GB/day cap a 7.4 GB ZIM takes 8 days and a 57 GB one 57 days (05 finding 8; 02 finding 15).
23. **The Pi-floor baseline and the resident candidates.** Release build, every component off: `111` 19,404,064 B at 19.7 MB idle RSS, `111-agent`
    4,392,280 B at 5.3 MB, listeners :11180 (plus a best-effort :80) and :11182 (`6-transport/TASKS.md:82`). Syncthing, rqbit and iroh are
    resident while on (listeners, DHT or relay keepalives) and none is in our lock; SQLCipher, `blake3`, `bao-tree`, `tar` and `flate2` already
    are (05 finding 17).
24. **Cat 4A means at least 512 GB free and 8 GB RAM** (`NI/install/categorize.rs:102-104`). A Pi on an SD card is never 4A by our own probe: a Pi
    that hosts datasets is a Cat 2B or 3 box with `hub_role` on (01's budget table calls those rows "Cat 4A on Pi"; correction in §4).
25. **Layer 9 plans state of its own (critic conflict 9).** Its incidents are a plaintext `state/incidents.jsonl` ring written by 111d (L9 `01`
    §3.2); its audit trail is append-only, hash-chained and signed per record with the node key (L9 `05` §4.1 item 5) or a capacity-pruned log
    (L9 `02` fork D); its pending delegation requests and its armed undo records need a home (L9 `07-gap-2` F5; L9 `02` §1, finding F1, fork B).
    Layer 2 already requires every signed artifact to lead with its own ASCII tag and keeps the registry (`2-identity/README.md:85,335-347`);
    Layer 7 adds three tags (L7 §18); Layer 9's scouts proposed two spellings for theirs (L9 critic conflict 11).

## 1. What Layer 8 is, in one paragraph

Layer 8 keeps each node's own state encrypted and bounded by the disk the node has, and lets a hub hold the large public files a community
shares. One node budget, set from the size of the disk and shown on a Storage page, decides when copies that can be fetched again give way;
nothing is deleted because of its age, and a dataset the user fetched is never deleted behind the user's back. The fabric store stays as built;
the agent's conversations and proposals and Layer 9's repair records move into a second encrypted file that only `111d` opens. A hub serves
encyclopedias (an adopted kiwix-serve, started on demand), regional maps (PMTiles files behind a `Range` route), the pinned payloads of the
components it already holds and, later, repair patches: all as files named by their sha256 in the one object tree Layer 7 built for releases,
listed in a small catalogue signed by the publisher's node key, and checked block by block. It adds no shared log in v1, and no resident process,
timer or new crate to the required configuration; the code it adds to `111` and the map's UI assets are build-time weight the owner approves by
deciding this brief, measured and recorded at build. **◐ on Cat 2A and 3** means: the local store runs natively; datasets are read from a hub over
the LAN, kept locally only when the user fetches a small one, and never served. **Hand-offs answered:** Layer 3 "State ← Layer 8" (F1, F2) ·
Layer 4's Storage stub and its deferred "storage (L8)" (F3, §3) · Layer 5's prune classes, Tier 2 and `SYNC` (F3, F7, F11) · Layer 7's origin
store, the dataset signer, the map's region file and the unverified sizes (F4, F5, F9, fact 13) · Layer 1's KB in the release payload (unchanged,
D30 K1) and its on-disk layout (one new row, F1).

## 2. Polled forks (owner decides; recommendation first)

### F1. What "the encrypted local store" covers, and who writes the agent's state (critic fork 2; conflict 2; G1)

- **A. `111d` is the only writer of a second SQLCipher file, the node-state store; the agent holds no key and writes nothing over HTTP.**
  Chat (M1 in 07-gap-1): 111d persists each turn from the `done` frame it already relays (an additive `messages` array, stripped before the
  browser sees it) and sends the stored `history` with the next turn; the history route is answered by 111d and works while the agent is down
  (503 today). Proposals and tasks (M2): the agent keeps writing them as a spool, made atomic (temp file, rename); 111d ingests each file on
  `proposal.written` or `task.done` and in a startup sweep, idempotent by id, then deletes it; running-task markers stay readable by the agent.
  Key: `BLAKE2b-256(key = store_secret, "111/node-state/v1" ‖ NodeId)` through the existing `KeyProvider` (the `store_key` pattern); the sweep
  called them `111/agent-state/v1` and `agent/state.db` (00, 04, 07-gap-1), renamed here for the `node/` namespace (§6). A separate file, not V5
  tables in `store.db`: a rollback or Layer 7's V4 migration must not take agent state down with the fabric (fact 6). Plaintext left,
  stated in the spec: a proposal on flash for a sub-second window while 111d is subscribed (until the startup sweep after a 111d crash; a deleted
  file may leave flash remnants); `events.jsonl` (a 1 MiB FIFO of kinds, ids and titles), `agent/work/` and the logs. **Cost:** no new crate,
  process, listener or timer; the agent's dependency list is unchanged; binary delta and the second connection's RSS unmeasured; about 12.4 KB of
  WAL per chat turn, about 25 KB with the checkpoint, about 2.5 MB on a 100-turn day (derived, 07-gap-1 §2 and finding 13), beside today's idle
  5.9 MB a day. One relay fix: persist on `done` even when the browser has left (`relay.rs:585-612`), as an acceptance test.
  *(pick: 07-gap-1 §4 A; 04 fork 1 B's "111d is the sole decoder", without its bearer writes)*
- B. Plaintext with atomic writes and capacity caps (M5): no weight, a day of work, honest; but README §8 and Layer 3's promise are not met for
  agent state, and `kind: message` bodies stay readable. The fallback if the owner declines A.
- C. A scoped write token for the agent (M3). **This reverses D26 (2)** ("no bearer exception at all"): the one-line write gate becomes an
  allowlist. Only if the owner reopens D26.
- D. The key handed to the agent (M4): breaks `2-identity/README.md:224`, adds +5.4 / +6.4 MB and a C toolchain to `agent111`. Rejected.
- Everything else stays as it is: the Delta Chat core's own database stays plaintext (upstream marked its database encryption deprecated, 04 §2);
  fscrypt and Android's file-based encryption are documented hardening, never required (04 fork 1 C); datasets are plaintext on purpose,
  because they are public and signed (04 finding 11).
- **Amends:** `3-intelligence/README.md:50` (where state lives) and the agent contract lines of `3-intelligence/TASKS.md` (22, 27-28, 37:
  additive fields, 07-gap-1 §2); one row in Layer 1's on-disk layout (`node/`, 0700); one key-derivation tag in L2 §13.1 A4. A code seam exposes
  the store's key-and-pragma open (`M/store/mod.rs:396-456`, private today); no Layer 5 spec text changes.

### F2. The joint storage contract with Layer 9 (joint with Layer 9; critic conflict 9; L9 critic fork 12 and conflict 11)

> **Joint storage contract (joint with Layer 9; the Layer 9 brief carries the same seven items, verbatim).**
> 1. **Home.** Layer 9's durable records (incidents, apply attempts, the loop ledger, kept-change snapshots, received fixes, the audit trail) live in
>    Layer 8's node111 state service under the namespace `heal/`, inside the `111d`-owned encrypted file Layer 8 opens for agent state (Layer 8 F1).
>    `111d` is their only writer; the agent reads them only through GET tools.
> 2. **Before the store opens.** Three small files stay plaintext under `state/` (0600, atomic writes, the probation-journal pattern), because a shell hook
>    writes them or they must be read before the store opens: the run marker, the witness inbox (one line per daemon exit: code, signal, boot id; no free
>    text) and the armed undo record of an apply in progress. `111d` reads them at start before serving, folds them into `heal/` and clears them; if the
>    store does not open, they stay and the Repairs page says so.
> 3. **Pruning.** By capacity only, as its own class in Layer 8's node budget: oldest closed incident first by the store's sequence; never an open
>    incident, an armed undo, an undecided proposal or a running task. A prune leaves one marker record (`pruned through seq N, K records`). No age, no
>    expiry, no wall clock.
> 4. **Order and integrity.** The store's monotonic sequence orders the records, with the boot id and monotonic time beside it; wall time is a display
>    claim. No own hash chain and no shared log in v1. A future shared fix-attestation feed must pass Layer 8's admission gate (a named consumer, one
>    writer per key, the never-list, a measured Pi delta), with p2panda-core as the adoption target.
> 5. **Signatures and tags.** Identity keys sign "announces, proofs, and control artifacts only — never routine traffic" (L2 §6.1). Layer 9's own rule
>    is stricter: it signs nothing that stays on the node (for Layer 9 the node key signs only its two outbound artefacts, `fix/v1` and `fix-vouch/v1`),
>    and no incident, proposal or audit record is signed. Every new node-key signature carries its own `111/<name>/v1` tag, registered in the one L2
>    table (§13.1 A4) in one amendment: Layer 7's `111/epoch/v1` and `111/broadcast/v1`, Layer 9's `111/fix/v1` and `111/fix-vouch/v1`, and Layer 8's
>    `111/catalogue/v1` and `111/export/v1`. Key-derivation inputs (Layer 7's `111/carrier-topic/v1`; Layer 8's `111/node-state/v1`, which derives the
>    key of the item 1 file) sit in the same table as key-derivation tags, never as signing tags.
> 6. **Nothing leaves.** Incidents, evidence and the audit never leave the node, except inside the user's own encrypted store export (Layer 8 F12),
>    which only this node's key opens. A shared fix carries only the `fix/v1` fields `111d` builds; a delegation carries only the `AGENT_OP` request fields.
> 7. **Fallback.** If Layer 8's agent-state file is not built when Layer 9 builds (or the owner takes Layer 8 F1 B), `heal/` is a `111d`-owned JSONL
>    ring under `state/heal/` (0600, 1 MiB, the `events.jsonl` pattern) under rules 2–6, moved into the store once it exists.

- **A. The contract above.** *(pick: 04 fork 5 A; 07-gap-1 §4 A; L9 `07-gap-2` F5; L9 `01` §3.2 for the record shape)* Why encrypted: incident
  evidence can carry free text from child logs (bridge adapters can log contact names, L9 `07-gap-2` F6), so it belongs behind the same key as
  conversations: one file, one budget, one export. Why no own hash chain, which the critic said would "reinvent p2panda-core": SQLCipher already
  checks every page with an HMAC (`cipher_integrity_check`, 04 finding 10), so corruption is caught without one; a chain whose head is anchored
  nowhere off the node detects nothing a same-uid process could not rewrite; and identity keys sign "announces, proofs, and control artifacts
  only — never routine traffic" (`2-identity/README.md:85`). Considered as a log and not adopted, because nothing here replicates: p2panda-core,
  Hypercore (an alpha Rust port; the original needs a JavaScript runtime), Scuttlebutt feeds (a dormant Rust crate) (03 §2). **Cost:** no crate,
  process, listener or timer; an incident record of 600 to 1,000 B (L9's estimate, `01` §3.2), so 1 MiB holds about 1,000 to 1,700; write volume
  unmeasured; the three boot-time files are a line or a record each.
- B. Layer 9 keeps its own files (`state/incidents.jsonl`, the `events.jsonl` FIFO pattern): no build-order dependency, but plaintext evidence on
  flash and a second budget outside Layer 8's figures. Kept only as item 7's fallback.
- C. An own node-key hash chain for the audit (L9 `05` §4.1 item 5), signed per record or at checkpoints: rejected for the three reasons under A.
- D. Layer 9's state as V5 tables in `fabric/store.db`: a failed migration or a rollback takes repair state down with messaging (fact 6), and the
  migration queues behind Layer 7's V4. Rejected.
- **Amends:** L2 §13.1 A4 (registry rows, one additive amendment); Layer 9's research picks (not locked); Layer 1's layout (F1's `node/` row and
  `state/heal/`).

### F3. One node budget; datasets never pruned automatically (critic forks 1, 11)

- **A. One `keep` figure per node, sized from the disk, plus a free-disk floor; each class pruned by its own capacity rule; datasets only by
  the user.** A thin node111 `budget` module asks each owner for `bytes()` and calls `reclaim(n)` on the existing 60 s pass (no new timer);
  mesh111's pruner keeps its classes, and `set_watermarks` is finally called. Below the floor every class refuses growth and the page shows
  `disk_low`. Rules, none by age: fabric (L5 §10.4, unchanged); agent (whole conversations by monotonic sequence, never the one being written;
  decided proposals with their task and decision; never an undecided proposal or a running task; down to 90 %, the lead's number, §5 Q24);
  heal (F2); child logs (a size ring per file); foreign databases (shown only; the one control is the user's "reset account"); components cache
  and releases (as built, two releases kept); **datasets: never automatically; only the user, or a proposal the user accepts, removes one, and a
  fetch checks free space first.** Proposed defaults, derived, not measured (04 fork 2): a phone min(2 GiB, 10 % of free space at install); a Pi
  on SD min(2 GiB, 15 %); a hub 25 % of the volume for the fabric and agent classes, datasets outside it. Page copy (05 finding 3): "Nothing
  here is deleted because of its age. When the space set aside is used up, copies that can be fetched again go first, oldest first; your own
  messages, contacts and anything waiting to be sent stay." **Cost:** code only, no timer of its own. On the existing 60 s pass the budget reads
  what the pass reads today for the fabric (page and freelist counts, the blob table's byte sum, one `statvfs`: `M/store/prune.rs:468-476`,
  `N/fabric/mod.rs:1133`), the node-state store's page count and per-namespace byte counters (kept by 111d as it writes, 04 fork 5 A) and one
  `stat` per capped child-log file; no directory walk on the pass. The classes the budget never reclaims (foreign databases, the components
  cache and releases under their own rules, datasets) are counted only on the Storage page: one `statvfs` and one capped directory walk per
  request (05 finding 2). The pass's added work is unmeasured (§3). *(pick: 05 F1 (b); 04 fork 2)*
- B. Datasets in the same pruner: a 57 GB file competes with message history, and a 50 GB file the user paid to fetch vanishing is the policy
  surprise README §8 forbids.
- C. Datasets pruned like re-fetchable blobs below 1 % free disk: kept as a later opt-in.
- **Amends:** nothing locked (L5's order and API stay; the fixed default watermark becomes a setting through the existing API); additive
  `/api/v1` fields (`disk_free`, `disk_total`, `classes[]`, `disk_low`). The owner confirms that "pruned by capacity rather than by imposed
  rules" reads as A (§5 Q1).

### F4. One object tree for datasets, checked block by block (critic conflict 6; G3)

- **A. Every public file a node keeps (release payloads, component pins, datasets and their sidecars) lives in Layer 7's sha256-addressed
  tree `state/origin/blob/<sha256>` (L7 §9.1), whether or not the node serves it.** The conflict: the seams scout wanted one sha256 tree shared
  with the origin, lens 1 a `datasets/<id>/` tree, lens 2 the origin layout, gap-fill 2 one blob directory under `state/origin/` with one
  admission rule; the critic left it to the lead. One tree wins: one Range handler, one admission rule, one place on disk, and the catalogue maps
  an id to its hash. A dataset's sidecar is the bao-tree post-order outboard at `ob_log = 14` (16 MiB blocks), stored as one more blob and named
  by its own sha256 (an address, not a trust anchor: the BLAKE3 root checks it, fact 11). Datasets never enter `fabric/blobs` (no dual outboard,
  no message watermark). A catalogue entry, TOML in the release manifest's style (`deny_unknown_fields`): `id, kind, name, version, seq, size,
  sha256, blake3, ob_log, outboard_sha256, licence (SPDX), attribution (the exact notice), source_url, built (a display string, never used for
  ordering), requires (a component id), fits (a category floor), used_by`; a newer `seq` supersedes (01 finding 13). **Cost:** no new crate
  (`bao-tree`, `blake3`, `sha2`, `toml` are linked; node111 gains a direct edge to `bao-tree`), no listener, no timer; the sidecar is 217,408 B for
  a 57e9 B file; one import pass computes sha256, BLAKE3 and the outboard together (throughput unmeasured; Pi 4 versus Pi 5 unverified); a 16 MiB
  buffer per active verify call. *(pick: 00 §6 A; 07-gap-3 fork A A1; 07-gap-2 §3.6)*
- B. A separate `datasets/<id>/` tree (01 fork D D2): a second place on disk and a second admission rule for files the origin already serves.
- C. `fabric/blobs` with a new class: the message watermark, 6.6 % extra disk for the two outboards, BLAKE3-only names. Rejected.
- The sidecar: **A1 the bao outboard** *(pick)*; A2 lens 2's sha256 list (a verifier of our own, no Tier 2 path); A3 Metalink 4 `<pieces>` (an
  XML parser and a second program, no root to bind the list to; RFC text unread). Metalink and upstream `.sha256` files stay an operator aid: an
  importer may read them when the first copy comes from a mirror (07-gap-3 fork A A4).
- **Amends:** L7 §9.1 additively (the tree holds datasets and sidecars; the catalogue files join the origin routes; F10's admission rule); the
  Layer 1 layout row for `state/origin/` (its purpose widened).

### F5. Who signs a dataset (critic fork 5; G6)

- **A. The publisher's node key under a Layer 2 tag, with one trust rule for every published artifact.** 111d signs a hub owner's catalogue on a
  human action: Ed25519 over `"111/catalogue/v1" ‖ publisher NodeId ‖ catalogue bytes`, a detached 64 B signature beside the file. A node shows
  the catalogue of its pinned home hub and of publishers the user follows (the L7 §11.1 rule for broadcasts) and checks nothing else; an
  endorsement (v1.1) is a further signature over the same entry under its own tag (`111/catalogue-vouch/v1`, the pattern of Layer 9's
  `111/fix-vouch/v1`), shown as "vouched for by <alias>", never as automatic trust; the release key never signs datasets. Freeze stance as
  L7 §9.5 T4: a node keeps the highest `seq` seen per publisher, and a stale mirror can hold it on an old catalogue until it reaches another.
  **Cost:** no new key to keep (v1 signs with the root node key, L2 §9, which the words recover), and no crate, process, listener or timer (the
  Ed25519 verifier and the TOML parser are already in the binary); a 64 B signature per catalogue and one verify per catalogue fetched.
  *(pick: 01 fork C C4, with the signer made the node key; D30 Q12)*
- B. A minisign key per hub owner (the release channel's format, 01 Q3): a second long-lived key to back up and rotate, not recoverable from
  the words.
- C. The 111 release key over a project catalogue: ties the code-signing root to third-party content and makes the project vouch for it.
  Rejected.
- D. Bare upstream checksums: integrity only; a hostile mirror wins.
- **Amends:** L2 §13.1 A4 (one row); L7 §9.6's "signed by their publisher" made concrete.

### F6. Moving datasets in v1, the first copy, and the metered cap (critic forks 8, 9; conflict 5)

- **A. HTTP Range pulls and sneakernet on the Layer 7 origin layout, under consent and the metered cap.** Serving: one single-range arm (206,
  `Accept-Ranges: bytes`, `Content-Range`, 416, `If-Range` against the existing ETag; always `Content-Length`; never gzip or chunk a `.pmtiles`,
  which is IIAB's workaround for a PMTiles bug, 07-gap-3 §4 finding 9), shared by origin blobs, the map's tiles and dataset pulls. Fetching: a
  streaming `get_to_file(url, range, pins)` with per-read idle timeouts into a `.part` file under a node111-owned staging path, a block bitmap as
  a hint, a check of each block against the root, then a rename on the same filesystem (a half-written file never replaces a served one).
  Hash-named URLs make a resume against any mirror safe, and two hubs can serve different ranges of one file with no extra protocol (02 finding
  3). The first copy: a hub with internet fetches a URL the user names (a Kiwix mirror, a Protomaps build), checks the upstream sha256 the user or
  the mirror supplies, computes BLAKE3 and the sidecar in the same pass, and only then may the owner sign its entry; or `111 dataset import
  --from <dir>` copies from a disk in the origin layout. Nothing is fetched without a consent that names the size, the free space before and
  after, the licence and attribution, the signer, `used_by` and the connection (05 finding 7), in the pinned `409 consent_required` shape (L7
  §12); progress rides the SSE stream only while a job runs. Reach in v1: the LAN and hubs with a reachable address (a public address or Layer 7's
  reach layer); a dataset never rides a carrier or LoRa (Berlin's 84 MB is about 9 days of LongFast airtime, 02 finding 12). **Cost:** nothing
  resident; a fetch thread and one bounded buffer only during a job; one sequential write per file plus its sidecar, one fsync and a rename (57 GB
  is about 47 min at an assumed 20 MB/s, 02 finding 13); no new listener; a bad block costs 16 MiB, where a whole-file check would cost 57 GB
  again (12.7 h at 10 Mbit/s, 02 finding 4). *(pick: 02 fork A A1; 01 §5 Q1 answered "both")*
- B. Sneakernet only: the first copy always arrives on a disk or from another hub; safest, slowest.
- C. A hosted rqbit or iroh-blobs for multi-hub swarming: resident while on, a second identity and listener (rqbit "a few tens of MB", a
  published claim; iroh +13.3 MB of binary and n0's public relays by default; 02 §2). Not in v1 (§5 Q12).
- **On a metered link** (L7 §12): (a) refuse on any marked link; (b) pause at the daily cap and resume in the next window, the dialog showing the
  days; (c) (b) plus a per-download override, a second explicit tap with its own consent text, never the agent's. *(pick (c), default (b); 05 F4)*
- **Amends:** L7 §12 (locked v1.0) additively, "dataset fetch bytes count in the same window as carrier bytes": the critic asked for it in
  Layer 7's fix wave, Layer 7 locked first, so it is now the owner's call; Layer 1's fetcher gains `get_to_file` (`Fetcher::get` unchanged); a
  verb `111 dataset import --from <dir>` beside L7's `111 update --from`; appended consent actions (§3).

### F7. Who owns Tier 2 (`OBJ_ADVERT`, `OBJ_REQ`, `OBJ_CHUNK`) (critic fork 4; conflict 1)

- **A. Layer 8 owns the Tier 2 build, in v1.1, as an additive Layer 5 amendment list Layer 8 writes and the owner sees; Layer 7's broadcast
  pointer and Layer 9's patches use it when it lands.** The conflict: lens 2 gave it to Layer 8 because "Layer 7 has none" as a consumer, which
  is false (L7 §11.1's `POINTER` object is "fetched by Tier 2"); lens 1 left it with Layer 7; L7 §17 defers it and names no owner. Resolution:
  Layer 8 has the largest consumer (hub-to-hub datasets) and owns the storage half Tier 2 needs (`admit(class, bytes)`, `superseded`, resume);
  v1 needs none of it (F6). Prerequisites: `bao-tree` 0.16 → 0.17 (a lock bump, MSRV 1.85, the `LICENSING.md:143` row, regenerated notices) and
  bulk on its own connection or Layer 7's v1.1 non-blocking `Link::send`, because a 57 GB transfer on the chat connection would stall chat (02
  finding 9). Hub-to-hub over the internet then rides L7's TLS-RPK link. **Cost:** none in v1; in v1.1 the two prerequisites, no listener of
  its own, a 16 MiB buffer per active block check (fact 11), binary and RSS deltas unmeasured. *(pick: 02 fork B B1, its premise corrected)*
- B. Layer 7 owns it: its only v1.1 bulk consumer is the pointer object; update payloads already ride the HTTP origin.
- C. Nobody until a second consumer: no peer-authenticated bulk transfer at all; HTTP stays public and unauthenticated.
- **Amends:** L7 §17's row gains an owner; L5 §11's v1.1 wire is built under the Layer 8 list.

### F8. Encyclopedias, and what a hub shows the LAN (critic forks 6, 7)

- **A. kiwix-serve as an on-demand hub component behind a newly built HTTP-only `child` panel source; raw dataset files on the open origin
  route behind their own switch.** The panel host gains the reserved `child` source kind for children that speak plain HTTP (WebSocket and SSE
  stay reserved, D20 (a)). Once the component is enabled, kiwix-serve runs bound to loopback, is started on the first panel open and stopped
  by the user or at daemon stop, behind the panel's `p111_<id>` cookie and on its own origin, which contains a ZIM's JavaScript (fact 14);
  launch line `--library <file> --threads 2 --nolibrarybutton --port <p>`, reload by `SIGHUP`, never `--monitorLibrary` (07-gap-3 fork C C1;
  the thread count unmeasured).
  The agent gets `kb.search` against its loopback `/search` while it runs, and "the encyclopedia is off" otherwise; it may propose turning it on,
  never starts it (05 F5). Raw files (ZIMs, PMTiles, sidecars) are served read-only and unauthenticated on `/origin/v1/blob/`, like release files,
  only while `hub_role`, LAN sharing and a new `origin.datasets` switch (default off) are on, with a consent text that says what the hub reveals
  (the datasets and versions it holds, the bounding box of any map, request times if logged; 05 finding 14) and a cap on concurrent bulk streams
  (4 proposed; kiwix's `--ipConnectionLimit` is the precedent). **Cost:** while off, nothing (the G7 zero-weight gate); while on, one loopback
  listener, kiwix's own 1 Hz wake, RSS unmeasured (fact 14); the payload is the Termux deb closure (kiwix-tools 3.8.2, libzim 9.8.2, libkiwix
  14.2.1 with ICU, Xapian, libmicrohttpd, libcurl, pugixml, zstd, lzma, docopt; size unmeasured) or the upstream static tarball (a 3.8.2 aarch64
  or musl build unverified); licences GPL-3.0-or-later (kiwix-tools, libkiwix) and GPL-2.0-or-later (libzim), all read upstream; a separate
  program, so D22's linking boundary is untouched. *(pick: 05 F2 (a); 01 fork A A3)*
- B. A LAN link-out to an unauthenticated kiwix-serve on its own port behind a recorded consent: no code, but an unauthenticated C++ HTTP server
  on the LAN. Only if the owner wants the encyclopedia before the proxy.
- C. Search only (the agent's `kb.search`, no reading view): cheapest; leaves out the encyclopedia itself.
- D. A ZIM reader in 111d, or libzim linked: C++, ICU and Xapian as build weight. Rejected (01 A4; 05 F2 (c)).
- **Amends:** L5 §13 (locked) additively: the reserved `child` kind built for HTTP-only children; Layer 6's component manifest (locked)
  additively: `adapter` optional, because a server component names no network; Layer 3's tool table (+ `kb.search`); L7 §9.1's "served only
  while" list gains `origin.datasets`.

### F9. Maps (critic forks 14, 16; conflict 3)

- **A. Vector maps: Leaflet 1.9.4 (BSD-2-Clause) and protomaps-leaflet 5.1.0 (BSD-3-Clause, which embeds pmtiles 3.2.1 and the basemap
  themes) in an exempt "map" bucket of 90 KB gzip, loaded only where `GET /api/v1/tiles` lists a set; PMTiles files from the object tree through
  the Range arm; v1 data is files as Protomaps publishes them and `pmtiles extract` regional subsets made off-node with go-pmtiles
  (BSD-3-Clause), with the ODbL sentence drawn on the map itself.** Measured at 83.9 KB with CSS (fact 16). The map is Layer 7's v1.1 item
  waiting for "a Layer 8 region file" (L7 §17); it lands here. **Cost:** browser-side only; on the node, the Range arm and a `.pmtiles` content
  type; no CSP change (vector tiles draw to a canvas, so `blob:` stays out); offline label fonts unverified; build-time, about 84 KB gzip of UI
  assets in every payload, fetched by a browser only on the map page. *(pick: 05 F3 (a))*
- B. Raster: Leaflet with pmtiles 4.5.0, 54.3 KB; one chunk, but over the lazy total, raster files are larger (unverified) and the CSP may need
  `blob:`.
- C. MapLibre GL, 451 KB and WebGL on old phones. Rejected. D. A link to the hosted Meshtastic map, which is blank offline.
- A regional extract tells every LAN visitor where the community is: the serving consent says so (05 §1).
- **Amends:** D21 (a note: the exempt "map" bucket at 90 KB; D30 F9 recorded the bucket with no figure and stands) and L7 §10.5's "≈ 69 KB"
  (`7-relay/README.md:951`, a correction); `check-budget.mjs`; the Layer 4 tiles route.

### F10. Software updates beyond the 111 release (G2; critic fork 15)

- **A. The hub origin also serves the pinned payloads of the components it holds (binaries; wheels through a files table; the Termux deb and
  APK pins) as `blob/<sha256>`, behind a separate switch `origin.components` (default off) with its own consent and a licence text and source
  link per blob; the 111 release's own source tarball joins as a pinned blob.** One admission rule for every blob: a hub serves a hash only if
  it is in a pin list it holds (the verified release manifest, an embedded component declaration of an installed release, a catalogue it
  verified under F5); a node's install-time fetcher tries `[home hub, pinned upstream URL]`, and a miss (404, wrong size, wrong hash) falls
  through (07-gap-2 §3.2 items 5-8, §3.6). No new trust anchor, tool, key or process; enable stays offline; "no runtime fetch" holds, since the
  CLI verb runs at install time. **Cost:** disk only for what the operator cached for itself, up to 104.1 MB (x86_64) or 94.5 MB (aarch64) for
  Delta Chat core and Madmail plus about 6 MB of wheels per target if it pre-caches to mirror; the Rust sources and manifests alone are about
  1.7 MB gzipped (L9 `07-gap-1` §1), the full tarball unmeasured. The licence duty is the real cost (fact 19): the switch's consent names it,
  and a member's acknowledgement records only the member's own install. The `lxmf` wheels wait on §5 Q25; serving ends with the file (Q26).
  *(pick: 07-gap-2 §4 option 1)*
- B. A plus a Termux apt directory (`termux-apt-repo`, Apache-2.0, a script on the publisher's machine) and an APK index signed by the hub owner
  over unmodified APKs (`fdroidserver`, AGPL-3.0, a JDK chain on the publisher): a new key, a per-phone onboarding step, an unmeasured mirror.
  v1.1.
- C. Nothing beyond the 111 release in v1, README §8's "software updates" read as "the 111 release": honest and free, but the first offline hub
  has no route for the 104 MB it needs.
- Rejected as resident: apt-cacher-ng, devpi and pypiserver (listeners); bandersnatch (a runner and a Python 3.12 floor) (07-gap-2 §2, §3.5).
- **Amends:** `1-bootstrap/README.md:227-246` ("from a hub origin first"), `6-transport/README.md:113-117` (an origin list), L7 §9.1 (the
  admission rule) and §12 (an appended consent action), `LICENSING.md` §8 rows, `gen_lock.py` (a files table), and **one sentence in D26 (5)**:
  "when a hub is the source, the hub's operator is the one who conveys; a member's acknowledgement records the member's own install". Not a
  reversal, but it touches an owner decision, so it is the owner's to take.

### F11. The shared log (critic forks 3, 13; conflict 4)

- **A. No shared log in v1; `SYNC` stays reserved; the spec records an admission gate and names p2panda-core as the adoption target.** The gate:
  a named consumer, one named writer per key, the never-list (keys and seeds, message bodies, alias and contact graph, location and presence,
  device ids, conversation or agent text, relay edges, anything its subject has not agreed to publish; 03 finding 5), a measured Pi delta, and an
  adopted engine, never our own consensus. No v1 catalogue-head frame (conflict 4: lens 2 wanted one, lens 3 none, and the seams scout asked
  only for the textless-row fix): a member checks its hub's catalogue on the same link-attribution event L7 §9.3 uses for releases (one GET, no
  gossip, no timer), and a followed publisher's new entry will travel as a signed `BROADCAST` pointer (L7 §11.1, v1.1). **Cost:** none in v1
  (no crate, frame, process, listener or timer); the textless-row fix saves a full-durability commit per unconsumed frame (fact 20). *(pick: 03 §4 A)*
- B. A plus a design-only contract (HAVE/OPS inside `SYNC`, the L5 epoch as the log id, p2panda-shaped operations, a `111/log/v1` tag, test
  vectors, no code): settles the signing question early, may fit one pre-1.0 library too closely.
- C. p2panda-core at the first consumer (a Layer 9 attestation feed, say): a pre-1.0 crate in the binary (build weight: the owner's say, or a
  cargo feature absent from the Pi build), operations stored in our store, a Layer 2 signing decision.
- D. CRDT documents (Loro first, for its shallow snapshots): only for a user-facing shared document, which no spec asks for.
- **The textless-row bug, decided apart (critic fork 13; §5 Q20):** inbound kinds with no consumer stop landing as chat rows. The fabric
  dispatch gains a small consumer registry (the hook Layer 9's `AGENT_OP` needs anyway, L9 `07-gap-2` F8 R3); an unconsumed `SYNC`, `KB`,
  `UPDATE` or `PATCH` is dropped and counted, never stored.
- **Amends:** nothing for A (the reservation stands); the bug fix amends L5's dispatch (locked) additively.

### F12. Backup, SD wear and migrations (critic forks 10, 12, 17; conflict 8)

- **A. One export format, the export-before-prune offer in v1, writes only on change, a copy before every migration.** (1) `111 store export`
  writes L2 §8.5(i)'s blob (encrypted to the node's own X25519 key, signed under a new `111/export/v1` tag, checked before decrypting) for
  `store.db` and the node-state store; CLI only, never over HTTP (the `identity backup` rule, `2-identity/README.md:296`); no new crate. (2) The
  offer: Layer 8 calls `set_export_offered`, so L5's step 3 can run at all. (3) `save_capacity` writes only when a value changes, removing the
  derived 5.9 MB a day; `journal_size_limit` is set (4 MiB proposed, about the 1,000-page autocheckpoint, 04 fork 4); full durability stays where
  custody receipts need it; child logs get a size cap. (4) Before any migration (Layer 7's V4, Layer 8's next): `wal_checkpoint(TRUNCATE)`, copy
  `store.db` to `store.db.pre-v<N>` (same key, same owner), deleted when probation confirms (04 finding 12). (5) `quick_check` and SQLCipher's
  `cipher_integrity_check` run only after an unclean shutdown (the store's existing clean record), never on every boot. **Cost:** no timer or
  process; (4) writes one transient copy of the store per migration; (5) reads the whole store once after an unclean shutdown (time on SD
  unmeasured). *(pick: 04 forks 3 and 4)*
- B. The same with `age`: a standard file other tools open, but its default dependency tree is build weight the owner has not accepted (a
  `LICENSING.md` row and a Pi-floor size measurement first) and it carries no sender signature.
- C. Words only (today): history, contacts, custody and the queued outbox are lost with the disk.
- `synchronous = OFF` or a RAM write buffer: rejected, because custody receipts promise the hub holds the item (`M/store/rows.rs:1008`).
- **Amends:** L5 §10.4 step 3 and §21 ("`age`-based export" → the L2 §8.5(i) format) and the KB `age` entry; L5 §10.1's pragma list and
  `save_capacity`'s cadence (Layer 5 code, additive); Layer 1's update path (the copy); L2 §8.5(i) moves from "specified now, built later" to
  built, with its tag.

### F13. Sequencing and the first Layer 8 release

- **A. v1 = F1 A (the node-state store, chat and the proposal spool through 111d, atomic agent writes) · F2 (the joint contract, built with
  Layer 9's first state) · F3 A (the budget, the Storage page as a lazy chunk, the additive API fields) · F12 A (export, the offer, writes on
  change, the migration copy) · F4 A and F5 A (the tree, the sidecar, the signed catalogue) · F6 A (the Range arm, `get_to_file`, sneakernet
  import, consent and the metered amendment) · F8 A (the HTTP-only `child` source, the kiwix component, `kb.search`) · F9 A (the map bucket) ·
  F10 A (component pins from the hub, the source tarball) · F11 A (no log; the consumer-registry fix). v1.1 = Tier 2 (F7: `bao-tree` 0.17,
  `OBJ_*`, hub-to-hub bulk over TLS-RPK) · automatic encrypted snapshots to the home hub (Tier 2 and a hub that opted in) · the Termux apt
  directory and the APK index (F10 B) · endorsements in the UI · a catalogue pointer by `BROADCAST` · Layer 9's hub fix shelf on the same tree ·
  `zimdiff` bundles only after a measured gain.** *(pick)*
- B. Datasets first (F4-F10), the store later: Layer 3's promise stays false and the idle write stays another cycle.
- C. The store only (F1-F3, F12): README §8's hub half stays unbuilt.
- D. Everything at once, Tier 2 included: blocks datasets on the `bao-tree` bump and on a Layer 7 v1.1 seam, while the bytes ride `tcp-lan` anyway.
- Every check stays a netns check on this box; the Pi measurements of §3 wait for hardware.

## 3. Detail picks (reviewed by the spec panel, not polled)

- **The Storage page** (`/settings/l8`, its own lazy chunk ≤ 64 KB; two chunks of about 15 KB fit the lazy bucket, 05 finding 5): a disk gauge
  ("12.4 GB free of 29.8 GB"), a table by class (messages and mail, knowledge base, updates, datasets, agent state, repair records, components
  cache, bridge accounts, logs) with layer, bytes, quota and a `used_by` chip per dataset (`map`, `encyclopedia`, `agent`, `updates`), so "what do I
  lose" is answered before a delete (05 findings 3, 10). Every figure is computed on request; the idle write of fact 5 is never copied.
- **Consent:** the pinned `ConsentRequired` round-trip (L7 §12) gains appended actions `dataset_fetch`, `dataset_override` (the metered
  override), `datasets_serve` (`origin.datasets`) and `components_serve` (`origin.components`); "paused (offline)" and "paused (metered cap)" are
  states, never errors; "you won't be warned in future" is the pattern refused (05 finding 6: every changed text asks again).
- **The agent:** read tools `storage.read` and `kb.search` (both tool-count pins bumped; Layer 7 takes the set from 17 to 20, L7 §18); a proposal
  kind `storage` with a closed vocabulary (`quota.set {class, bytes|null}`, `dataset.remove {id, version}`, `dataset.fetch {id, version}` whose
  acceptance opens the consent dialog and never skips it). It never prunes, accepts, starts a download or touches a metered setting (05 finding 4).
- **Refresh and removal:** a refresh stages, checks and renames; the old version is removed only when the user accepts the refresh, whose consent
  names the bytes freed; a fetch that does not fit says so before any byte moves.
- **Patches (README §8's "network troubleshooting patches"):** Layer 9's fix artefacts are small (under 4 KiB, a 64 KiB attachment cap, L9 `03`
  §4.1) and travel in v1 as sealed `PATCH` messages, Layer 9's pick; the hub's fix shelf (v1.1) is a catalogue class `patch` on the same tree,
  signed by its author and vouched for with Layer 9's `111/fix-vouch/v1`. Layer 8 adds storage, a class and the Range path, never a format (01 finding 9).
- **Sneakernet:** copy mode, and on a hub register-in-place (hash once, no write, rechecked after an unclean shutdown); FAT32 caps a file at
  4 GiB, so the docs name exFAT or ext4 (02 findings 2, 14). On Termux, datasets stay app-private and the copy says uninstalling Termux removes
  them; a phone holds only small ZIMs (05 finding 16).
- **The credits page:** the ODbL sentence verbatim; CC BY-SA attribution per ZIM from its own `License` and `Source` metadata (search snippet);
  a "non-commercial" tag on any CC BY-NC dataset; no single licence claimed for a ZIM that mixes image licences (07-gap-3 fork D D1).
- **Threat model (critic G9):** a ZIM's JavaScript stays inside a per-port panel origin, and ZIMs come only from a catalogue the user follows;
  kiwix-serve (libmicrohttpd) is never LAN-bound by default; the bulk route is bounded by the stream cap and 64 KiB streaming; the bounding box
  and the holdings list are named in the serving consent; requests are not logged by default.
- **No clock anywhere:** catalogue `built` is a display string and `seq` orders; the metered window is L7's `CLOCK_BOOTTIME` count; conversation
  ids that embed wall-clock nanoseconds (`A/store.rs:164-171`, 07-gap-1 finding 14) never order anything once the rows carry a store sequence.
- **Pi measurement protocol, recorded in `8-storage/TASKS.md`:** `VmRSS` and `Pss` from `/proc/<pid>/smaps_rollup` (anonymous versus file-backed,
  which settles conflict 7 for kiwix-serve), `ss -ltnup` before and after, `strace -c -f -e trace=write,fsync` over ten idle minutes,
  `/proc/<pid>/io` `write_bytes` over an idle hour and `/sys/block/mmcblk0/stat` (fact 5's 5.9 MB a day), binary and cache sizes (05 finding 17;
  04 fork 4); the time and reads the budget adds to the 60 s pass (F3 A), with the node-state store open.
- **The KB (critic G8):** `candidate` entries at build for kiwix-tools, PMTiles with protomaps-leaflet and Leaflet, and p2panda-core as the named
  log target; desk research caps them at `candidate` (`0-knowledge/README.md:93`).

## 4. Plan corrections this brief records (the lead applies them after the decision)

1. `os/README.md` §8: "an encrypted local store" gets its honest scope (the fabric store plus the node-state store; datasets plaintext and
   signed; encryption protects copies, not a stolen disk, F1); "software updates" reads as the 111 release and its components' pins (F10).
2. `3-intelligence/README.md:50`: agent state "in the encrypted local store" is false as built (fact 1); reworded per F1. `agent111/src/store.rs:2-3`
   promises atomic writes, while `save_proposal` and `save_task` use `std::fs::write` (`:139`, `:145`): fixed under every option (07-gap-1 finding 6).
3. `5-messaging/README.md`: §10.4 step 3 and §21 "`age`-based export" → the L2 §8.5(i) format (F12); §11's "intake path for a staged payload at
   `fabric/blobs/<hash>`" is unbuilt and superseded by L7's `state/origin/blob/` (fact 9); §11's "`KB` deltas and git bundles (Layer 7)" (git
   bundles were rejected, D30 F6); §11's "`SYNC` ops … per-peer cursors" has no cursor table and `Class::Sync` has no prune step (03 finding 10;
   00 finding 20); §10.1's "One DB: `fabric/store.db`" stays true for Layer 5 and gains a pointer to Layer 8's second file.
4. `7-relay/README.md` (locked v1.0; each item is an additive amendment for the owner): §10.5 "≈ 69 KB" → 83.9 KB, bucket 90 KB (F9); §12 dataset
   bytes in the metered window (F6); §9.1 the admission rule, the catalogue files and `origin.datasets` / `origin.components` (F4, F8, F10), and
   the GPL source offer for the release payload the origin already serves, which §9 never mentions (07-gap-2 item 22); §17 Tier 2's owner (F7).
5. `1-bootstrap/README.md`: the on-disk layout gains `node/` (0700) and widens `state/origin/` (F1, F4); `:227-246` "a USB drop … is the offline
   alternative" gains "or the home hub's origin" (F10); `:160` "APK refresh = Layer 9 maintenance task" is Layer 9's (its critic noted it).
6. `6-transport/README.md:113-117`: the install-time fetch gains an origin list (F10); the component manifest's `adapter` becomes optional (F8).
7. `2-identity/README.md` §13.1 A4: the tags of F2 item 5 join the registry in one amendment, one spelling each (`111/node-state/v1` for the
   second store's key); §8.5(i) names its signature tag, since it says only "signed by the node key".
8. `LICENSING.md:56`: the Reticulum License text is read (1,510 B, 07-gap-2 item 20); `:143` `bao-tree` 0.16.1 while 0.17.0 shipped (F7); §8 gains
   the hub-conveyance rows (F10), kiwix-tools as a component and the three map packages (`THIRD-PARTY-NOTICES.md` regenerated with the lockfile).
9. `plan/DECISIONS.md`: D26 (5) one sentence (F10); a D21 note with the map bucket's 90 KB (F9; D30 F9 carries no figure and stays).
10. `0-knowledge/projects/age.md`: "planned for history export only" → a possible format for hub-side tooling (F12); new `candidate` entries (§3).
11. `4-interface/README.md:697` "storage (L8)" leaves the deferred list; `UI/views/settings/model.ts:221-222` drop "arrives with Layer 8".
12. This folder: `research/01-datasets-hub.md` §4's "Cat 4A on Pi, 64 GB / 32 GB SD" rows contradict `categorize.rs:102-104` (fact 24) and their
    budgets are marked "(guess)"; `06-critic.md` §4's "3,396 × 64 B" is 3,397 pairs = 217,408 B (07-gap-3 §1); 01 and 02 quote the March 2024 mini
    (13 GB) without the November 2024 figure (7.4 GB) (fact 13).
13. Layer 9's research: incidents at `state/incidents.jsonl` (L9 `01` §3.2), the per-record node-key signature (L9 `05` §4.1) and two tag
    spellings (L9 `00`, `03`) give way to the joint contract (F2) once both owners' decisions agree.
14. Found in passing (00 §4): `/api/status` is session-gated in code (`N/dashboard/routes.rs:472-478`) while Layer 1's text and `routes.rs:28` call
    it exempt; a Layer 1/4 text fix, and no dataset route copies that posture.

## 5. Open questions for the owner (beyond the polled forks; the lead's default in each)

1. Is "datasets are never deleted automatically; only you, or a proposal you accept, removes one" your reading of "pruned by capacity rather than
   by imposed rules"? *Default: yes (F3 A).*
2. Who sets the node budget on a phone (critic fork 11)? *Default: the install-time figure, min(2 GiB, 10 % of free space); you change it on the
   Storage page; the agent may propose; nothing else sets it.*
3. Does "encrypted" cover the durable copy, with a sub-second plaintext spool for proposals, or must plaintext never touch flash (then only F1 C,
   D or a sealed spool qualify, each with a cost)? *Default: the durable copy.*
4. Accept the agent contract edits (`history` in the chat request, `messages` in the `done` frame, proposals and tasks as a spool) as additive
   Layer 3 changes? *Default: yes.*
5. May Layer 8 prune an agent conversation, and Layer 9 a closed incident, without an export offer (the fabric's own history gets one)?
   *Default: one offer per class before its first prune, then capacity order.*
6. `events.jsonl`, `agent/work/` and the logs outside the encrypted claim, with log size caps a Layer 8 task? *Default: yes, stated in the spec.*
7. Delta Chat's own database stays plaintext, documented, with fscrypt named for Linux operators? *Default: yes.*
8. May a hub fetch the first copy of a dataset from upstream on your action (critic fork 8), or must it arrive on a disk or from another hub?
   *Default: both, each under consent and the metered cap.*
9. Which datasets does v1 ship or list (critic fork 16)? *Default: none: no dataset bytes and no default catalogue entries ship (the D30 Q4
   precedent: you name the source); v1 is tested with one small Kiwix ZIM and one Protomaps regional extract.*
10. Only Protomaps files as published and `pmtiles extract` subsets until the legal read of "a re-extracted vector tileset is a Produced Work"
    (critic fork 16)? *Default: yes; anything rebuilt from OSM data waits.*
11. May a hub list and serve non-commercial content (CC BY-NC satellite imagery, for one) (G5)? *Default: yes, tagged "non-commercial" in the
    catalogue and the consent, never preselected; 111 refuses nothing (no policy of ours).*
12. Syncthing, rqbit, iroh (critic fork 18): never, or as opt-in components? *Default: none in v1, not even opt-in; rqbit and iroh-blobs may be
    reopened as hub-only components only if Range pulls and Tier 2 fall short; Syncthing never (resident, rescans).*
13. May a hub convey third-party binaries to other people's nodes, or only to its owner's own devices (critic fork 15)? *Default: allowed behind
    `origin.components` (off), with the licence text and source link per blob and the duty named in the operator's consent; whether serving your
    own devices is "conveying" stays the open legal read (`LICENSING.md` item 14).*
14. `origin.components` and `origin.datasets` as switches apart from `origin.serve`? *Default: yes, both separate and off.*
15. The dataset route unauthenticated on the LAN, or for paired sessions only (critic fork 7)? *Default: unauthenticated for the signed raw files
    behind `origin.datasets` and a 4-stream cap; kiwix-serve always behind the panel cookie.*
16. Build the HTTP-only `child` panel source now, or a LAN link-out to kiwix-serve first (critic fork 6)? *Default: build it; no link-out.*
17. kiwix-serve fetched only on a hub by `sudo 111 install --components kiwix` (`prefetch = false`), never on a phone by default? *Default: yes;
    the Termux closure is measured before any phone path.*
18. The sidecar block on a 512 MB board (16 MiB or 4 MiB), and who computes the first BLAKE3 and sidecar? *Default: `ob_log = 14` for files of
    1 GiB and more, a Pi hub may choose 12 after measurement; the importing hub, in one pass.*
19. Register in place on an attached disk; FAT32 sticks split with `zimsplit` or documented as exFAT/ext4 only? *Default: in place on hubs;
    exFAT/ext4 documented, no splitting.*
20. Inbound kinds with no consumer dropped and counted now, a Layer 5 bug-level amendment shared with Layer 9's `AGENT_OP` hook (critic fork 13)?
    *Default: yes.*
21. A copy of `store.db` before Layer 7's V4 migration (critic fork 17), while Layer 7 is locked and building? *Default: ask Layer 7's build to
    take it (one step in the store's migration path), recorded as a cross-layer request.*
22. Bump `bao-tree` 0.16 → 0.17 now, or when Tier 2 starts? *Default: when Tier 2 starts; v1's block checks run on 0.16.1 (07-gap-3 finding 10,
    an inference from the call sites, not a test).*
23. Is there a multi-writer shared object you want (a notice board, shared checklists, group rosters)? *Default: none in v1, which closes F11 at A.*
24. The lead's numbers (`journal_size_limit` 4 MiB, the 4-stream cap, the budget percentages, the agent class's 90 % low mark, the sidecar
    block): keep? *Default: keep, each measured on a Pi before the spec locks.*
25. Does the pending Reticulum contact of D26 (4) (`LICENSING.md` item 16, the helper's licence) also cover a hub serving the `rns` and `lxmf`
    wheels under F10, or only the helper (07-gap-2 §6 Q3)? *Default: the wheels are served only after your upstream answer; until then
    `origin.components` excludes the `lxmf` closure.*
26. Does a hub keep serving a component blob after its operator disables the component and removes `cache/packages/<id>/` (which "only a
    separate call deletes", `1-bootstrap/README.md:241`; 07-gap-2 §6 Q6)? *Default: serving ends with the file.*

## 6. Critic and gap-fill

**The critic (`research/06-critic.md`): "enough for the brief on maps, encyclopedias, where datasets live, the shared log (none in v1), SD wear
and the Storage page"; not enough on agent-state encryption (G1, blocking: both scouts' mechanisms broke a locked rule) or on software updates
beyond the release (G2, major).** How each gap ended: G1 closed by 07-gap-1 (F1); G2 closed by 07-gap-2 (F10, facts 18-19); G3 closed by 07-gap-3
from bao-tree's own source (F4, fact 11); G4 half-closed by 07-gap-3: IIAB and offspot were read, no kiwix-serve memory figure is published, so
conflict 7 stays open until measured (fact 14, §3); G5 (non-commercial content, per-image licences) is §5 Q11, the per-image licences unverified;
G6 (the signer's trust anchor) is F5 (the pinned home hub plus followed publishers); G7 (◐ on 2A and 3) is §1; G8 (KB entries) is §3; G9 (the
threat model) is §3. The nine conflicts: 1 Tier 2's owner → F7; 2 agent-state encryption → F1; 3 the map bucket → F9; 4 `SYNC` in v1 → F11;
5 the metered cap → F6; 6 the dataset tree → F4; 7 sizes and memory → facts 13-14 (no figure in the spec); 8 the export format → F12; 9 Layer 9 →
F2. The eighteen owner forks: 1 F3 · 2 F1 · 3 F11 · 4 F7 · 5 F5 · 6 F8, Q16 · 7 F8, Q15 · 8 F6, Q8 · 9 F6 · 10 F12 · 11 Q2 · 12 F12 · 13 F11, Q20 ·
14 F9 · 15 F10, Q13 · 16 F9, Q9-Q10 · 17 F12, Q21 · 18 Q12. Custom code the critic flagged: the chunk sidecar (now the adopted bao-tree outboard)
and Layer 9's own audit chain (F2 drops it: SQLCipher's page HMACs already catch corruption, and a same-disk chain proves nothing). Resident
costs it found without a number (the second SQLCipher connection, kiwix-serve and its stop, `quick_check`, the persisted metered counter, the
p2panda-core and `age` deltas) are marked unmeasured where they appear.

**Gap-fill (three scouts).** `07-gap-1.md` (G1) tabled six mechanisms against D26 (2), L2's zero-key rule and the Pi floor; only "111d persists
chat and ingests the proposal spool" breaks none, with plaintext-and-caps as the fallback; it also found that "zero key material" is enforced by
which code holds the key, not by the OS (same uid) — folded into F1, facts 2-3 and §5 Q3-Q6. `07-gap-2.md` (G2) showed the component pins are
already origin-independent and that the hub operator becomes the conveying party, with the Reticulum License now read — F10, facts 18-19, §4
items 4 and 8, §5 Q13-Q14 and Q25-Q26. `07-gap-3.md` (G3, G4) computed the outboard sizes from source, found the dual-outboard trap and the
16 MiB buffer, read IIAB and offspot, found kiwix-serve's 1 Hz wake, and reconciled the two mini sizes — F4, F8, F9, facts 8, 11, 13-14. One
tension between them, settled here: 07-gap-1 names the file `agent/state.db` (as 00 does) and its key tag `111/agent-state/v1` (as 04 does);
F1 names them `node/state.db` and `111/node-state/v1`, so Layer 9's `heal/` sits beside the agent's namespaces under one name, and F2 item 5
registers only that spelling.

**Unverified claims (the lead checks them on the owner's machine).**
- *Sizes:* English Wikipedia maxi, nopic and mini today (13 GB or 7.4 GB), Simple English, Wiktionary, WikiMed; whether `.sha256`, `.torrent`
  and `.meta4` files (and at what piece length) sit beside each ZIM; Protomaps planet and region sizes and its BLAKE3 hashes; the Geofabrik
  date; raster against vector PMTiles sizes; the Termux kiwix closure, the APK trio, the debs; the 111 release source tarball.
- *Binaries:* kiwix-tools 3.8.2 static tarballs for aarch64 and x86_64, and whether they are musl; release dates of kiwix-tools, libzim and
  libkiwix; Syncthing's current tag; libtorrent's date; kiwix-serve's bind-address flag name.
- *Measurements:* kiwix-serve memory (anonymous against file-backed), wakeups and writes; Martin, rqbit (and whether its DHT turns off),
  Syncthing v2 and iroh idle cost; the second SQLCipher connection; 111d's idle `write_bytes` (5.9 MB a day is derived) and SD amplification;
  BLAKE3 and sha256 throughput on Pi 4 and Pi 5; the p2panda-core, Loro, Automerge and `age` binary deltas; `quick_check` time on SD; the Range
  arm and fetcher size; the 16 MiB buffer beside kiwix-serve on a 512 MB board; the Storage page chunk.
- *Behaviour:* SQLCipher's `VACUUM INTO`, `sqlcipher_export` and the backup API on an encrypted file; WAL `NORMAL` durability (an sqlite.org
  snippet); `age` from a raw scalar; on Termux, file-based encryption, `statvfs`, file modes, USB OTG and ZIM `mmap`; protomaps-leaflet's offline
  label fonts; journald's `Storage=`; the chat relay with a closed browser (`emit`); whether llama-server logs prompts; Willow timestamps and
  Loro's shallow snapshots; Martin's cache; Syncthing's ports; Termux `apt` with a third-party signed repo; Android's "install unknown apps"; the
  F-Droid client adding a repo by fingerprint; RFC 5854's `<pieces>` element.
- *Legal and licences:* whether a repacked ZIM is Adapted Material; whether a re-extracted vector PMTiles is a Produced Work or a Derivative
  Database; Protomaps' ODbL statement (read through a mirror); per-image licences inside a maxi ZIM; whether serving your own devices is
  "conveying" and whether a source link on another server meets GPL-3.0 §6(d) (`LICENSING.md` item 14); the Wikipedia reuse wording; licences
  not read from the upstream file: casync's version suffix, zsync, kiwix-js (badge only), rusqlite (metadata), Bamboo, kuska-ssb, apt-cacher-ng,
  reprepro, devpi, deltachat-android, Termux:Boot and Termux:API, and the "or later" clauses of aria2 and IIAB.
