# Layer 8 research, lens 3: the lightweight append-only shared log

Date: 2026-11-02. Scope: README section 8, "A lightweight append-only shared log **may** be used for the small subset of state that
benefits from collective consistency - adopted from a proven project, never a custom consensus implementation." Code is cited as
`M/` = `5-messaging/crates/mesh111/src/`, `ID/` = `2-identity/crates/id111/src/`. Web facts carry a URL; "(search snippet)" means the
page was not fetchable on this box; "(unverified)" means no source was reached.

## 1. Summary

- **Nothing in v1 needs a shared log.** Every item the node holds that crosses nodes today is single-writer and already self-ordering by a
  signed counter: the announce (counter, `2-identity/README.md:107,135-137`), the release manifest and the KB release (minisign + monotonic
  version), the dataset catalogue (a publisher-signed manifest; only the newest matters), the mesh-epoch attestation (a max-register, 202 B,
  `ID/attest.rs:6-10,111`). Local-only state (device journals, relay edges, the agent task journal, proposals, conversations) must not leave
  the node. The honest answer to "which state qualifies" is: none yet.
- **The seam is reserved but half-wired.** `Kind::Sync = 0x14` and `Class::Sync = 6` exist, with a refusable quota (`M/envelope.rs:514,564`;
  `M/store/rows.rs:46,83,95`). Two gaps: `prune.rs` never mentions `Class::Sync` (no eviction step; a full store can only refuse), and the
  only `cursors` table is the hub-custody cursor (`M/store/schema.rs:165-168`), not a per-peer SYNC cursor. Both are L5 amendments for the day a
  consumer exists.
- **If a log is ever built, the adoptable shape is a signed hash chain per author (p2panda-core), not a CRDT.** It uses our primitives
  (Ed25519, BLAKE3, CBOR), has prefix pruning and off-chain payloads, and single-writer logs make capacity pruning an author-local decision (no
  coordinated compaction). It is pre-1.0 (0.7.1, 2026-08-21), so adopt only behind a pin with stored header bytes verbatim.
- **Multi-writer CRDT documents (Loro, Automerge, Yjs/yrs) are only justified by a user-facing collaborative-document feature** that no
  layer specifies. They keep history by design; compaction forks the document and breaks sync with peers lacking a common ancestor. Loro
  has the only first-class history trimming (shallow snapshot).
- **The fork-on-restore trap applies to any identity-signed log.** A node restored from words would restart at seq 0 and fork its own log.
  L5 already solved the same problem for message numbers: `epoch` minted from a fresh announce counter. Mapping `log_id := epoch`,
  `seq := n` gives a per-author total order `(epoch, seq)` with no wall clock.
- **Pick: no shared log in v1; the seam stays reserved; record an admission gate; pre-select p2panda-core as the adoption target.**

## 2. Candidates

License column: V = read from the upstream LICENSE file on 2026-11-02, M = crates.io metadata only. Resident weight: none of these has a
daemon when used as a library; "binary delta" is not measured (no cargo on this task). Dependency counts are non-optional normal deps from
crates.io for the named version.

| Name | Upstream | License | Language | Maturity (crates.io dates) | Resident weight | Fit | Why |
|---|---|---|---|---|---|---|---|
| p2panda-core (+ p2panda-sync) | https://github.com/p2panda/p2panda | MIT OR Apache-2.0 (V: `LICENSES/MIT.txt`, `LICENSES/Apache-2.0.txt`) | Rust, edition 2024, rust-version 1.96 | 0.7.1, 2026-08-21; 12 versions since 2024-12; 23k downloads; README: "APIs are not yet considered stable" until 1.0; NGI-funded | library; core deps: blake3, cbor-core, ed25519-dalek, hex, rand 0.8, serde, serde_bytes, thiserror; no process, no socket | **bridge** (adopt `-core` at first consumer) | Signed per-author log: header = version, key, signature, payload_size, payload_hash, seq_num, backlink, extensions (`operation/header.rs:41-79`); no mandatory timestamp field in this header; many logs per author; `PruneFlag` marks "points in a log where all preceding operations can be deleted"; payload deletable independently of the header; README: fork-tolerant, usable over "packet radio, LoRa, BLE, USB stick". `-sync` runs two-party protocols over a `Sink`/`Stream` pair (fits tcp-lan; sealed frames need our own cursor glue). `-store` is sqlx/SQLite: do **not** adopt (a second SQLite binding beside SQLCipher); keep ops in our store. |
| Loro | https://github.com/loro-dev/loro | MIT (V: LICENSE, "Copyright (c) 2023 Loro") | Rust (also JS/WASM) | 1.17.1, 2026-10-22; 48 versions; 1.07M downloads; 8 direct deps (loro-internal, generic-btree, loro-kv-store, ...) | library; binary delta unknown (a 2026 guide quotes 180 kB WASM min+gz, no method: unverified) | **reference / contingency for documents** | Delta export from a version vector (pure function, no interactive session: fits cursors). Shallow snapshot "works like Git shallow clone": keeps state, drops history before a point (README; benchmark 273,561 B default snapshot vs 63,352 B shallow on the 182k-insert editing trace, loro.dev docsize page, search snippet). Younger than Automerge; maintainer depth unknown. |
| Automerge (`automerge` crate) | https://github.com/automerge/automerge ; KB `0-knowledge/projects/automerge.md` | MIT (V: LICENSE, "2019-2021 the Automerge contributors") | Rust core (+WASM/C bindings) | 0.12.0, 2026-09-16; 795k downloads; 16 non-optional deps (flate2, sha2, hexane, itertools, ...) | library; unknown | **reference** | Sync is a stateful two-party exchange: "assumes a reliable in-order stream between two peers", per-peer `State` (`rust/automerge/src/sync.rs:1-12`, fetched). Fits tcp-lan; sealed SYNC frames would need session emulation, and L5 delivers by prefix+ranges, not strictly in order (`5-messaging/README.md:202-213`). History is kept forever; "compaction" saves without history and the result cannot sync with a document lacking a common ancestor (automerge.org glossary, search snippet). |
| yrs (Yjs port) | https://github.com/y-crdt/y-crdt | MIT (V: LICENSE) | Rust | 0.28.0, 2026-09-17; 4.5M downloads; 10 deps incl. dashmap, async-lock, arc-swap | library; unknown | **reference** | State-vector delta sync; GC on by default but "a rising floor size" and Y.Map keys reclaim poorly (discuss.yjs.dev threads, search snippet). No signatures, no history. Fastest, least fit for audit. |
| hypercore (Rust port) + hypercore-protocol | https://github.com/datrs/hypercore ; JS original https://github.com/holepunchto/hypercore | Rust port: MIT OR Apache-2.0 (V: LICENSE-MIT; Cargo for protocol); JS: MIT (V) | Rust "limited port" / JS | 0.17.0-alpha.1, 2026-08-07 (alpha, not 1.0); JS 11.39.6 | Rust: 20 deps incl. blake2, sha2, futures, random-access-*; JS needs Node/Bare (a resident runtime) | **reference** | Merkle-tree single-writer log with sparse replication and "clear range" (local deletion without breaking the tree): the closest match to capacity pruning. Its replication is a Noise-framed duplex protocol with its own handshake, not our sealed frames; multi-writer (Autobase) exists only in JS. |
| iroh-docs | https://github.com/n0-computer/iroh-docs | MIT/Apache-2.0 (V: LICENSE-MIT "Copyright 2026 N0") | Rust | 0.101.0, 2026-06-15; 216k downloads; 29 deps (iroh, iroh-blobs, iroh-gossip, redb, tokio...) | pulls the iroh QUIC stack and a redb file | **reject** (pattern only) | Entries carry a timestamp and are signed by namespace+author keys (README): last-writer-wins on a wall clock, forbidden here. Range-based set reconciliation (Meyer, arXiv 2212.13567) is the useful idea for reconciling a catalogue of signed records. |
| iroh-willow / willow_rs | https://crates.io/crates/iroh-willow ; https://codeberg.org/worm-blossom/willow_rs | MIT/Apache-2.0 (M) | Rust | iroh-willow 0.0.1, 2025-02-07 (889 downloads, abandoned); willow-data-model 0.8.0, 2026-08-10 | n/a | **reject for v1; reference** | Willow'25 data model prunes by newer entry at a path prefix, which would make "pruned by capacity" a data-model rule. Sync status: a third-party port says Confidential Sync is not yet usable there (search snippet, not official). Codeberg and willowprotocol.org are denied on this box: entry timestamps and their required semantics are unverified. |
| crdts crate | https://github.com/rust-crdt/rust-crdt | Apache-2.0 (V: LICENSE) | Rust | 7.3.2, **2023-08-08** (three years idle); 2.7M downloads | library | **reject** (semantics reference) | Primitives only (GCounter, LWWReg, ORSWOT, VClock); no sync, no signatures, no storage. The cases that matter here (max-register, grow-only set) are a few lines. |
| Secure Scuttlebutt feeds (kuska-ssb) | https://ssbc.github.io/scuttlebutt-protocol-guide/ (public domain per its README, V) | kuska-ssb: AGPL-3.0 (M) | Rust, last release 2022-02-19 | dormant | n/a | **reject** (reference) | Per-author signed hash chain, the design p2panda and Bamboo refine. Rust crate is dormant. Replication follows the social graph (names people) and a restored identity forks its feed. |
| Bamboo (spec) | https://github.com/AljoschaMeyer/bamboo | spec license unverified | spec | "Status: Stable" | n/a | **reference** | Single-writer log with lipmaa links for transitive partial replication and local deletion. The author now says "append-or-delete log" is the honest term, since forks need deterministic invalidation. |
| Syncthing BEP (pattern) | https://github.com/syncthing/syncthing | MPL-2.0 (V: LICENSE) | Go, separate program | active | a resident daemon | **reject as program; pattern** | Delta index exchange: `sequence` is "the value of a device local monotonic clock", `max_sequence` plus `index_id` detect gaps and database loss, per-file version vectors detect conflicts (`specs/bep-v1.rst:330-336,472-481`, fetched). This is the per-peer cursor design: cursor = `(index_id, max_sequence)`. |
| Briar sync, Earthstar, Matrix DAG, Delta Chat | see section 5 | n/a | n/a | n/a | n/a | **reference only** | Prior art. |
| Plain per-author hash chain, own code | n/a | ours | Rust | n/a | none | **only via the bridge row above** | A hash chain is not consensus, but writing one ourselves would reinvent p2panda-core; the gate says adopt first. |

## 3. Findings that change the plan

1. **Every cross-node state that exists today is single-writer and counter-ordered, so no conflict semantics are needed.**
   - Identity and home-hub hint: the announce; "highest announce counter wins", key material immutable once pinned (`2-identity/README.md:107`);
     replay is handled by counters, "no wall-clock anywhere" (`:135-137`). A log adds nothing.
   - Release and KB: minisign over a monotonic manifest; the KB rides the release (L1) and git bundles (L7). One writer (the release key).
   - Dataset catalogue (L8 lens 1): a manifest per publisher with a version counter; superseded versions are garbage. A log would preserve history
     nobody wants.
   - Mesh epoch: `MeshEpoch(u64)` is stored and ordered, expiry never evaluated (`ID/attest.rs:41-47,108-131`); its source is Layer 7's
     (`2-identity/README.md:218`). A grow-only max-register: "take the larger signed value".
   - Presence and registry: local observation answered by a sealed query (Layer 7 section 7); publishing it to a log is churn plus a location leak.
2. **Local-only state must not become shared.** Device journals, relay edges (`M/store/schema.rs:278`), the agent task journal, proposals and
   conversations are per-node. Replicating them is telemetry (rule 4). The L3 contract "State <- Layer 8" (`3-intelligence/README.md:50`) is the
   local encrypted store, not a log.
3. **The only candidate with real collective semantics is a Layer 9 feed of validation attestations** ("a repair validated on one node can heal
   others", README section 9). Each is a signed fact by one author about one patch hash: a grow-only set, mergeable by union. It may not even need
   a log: `PATCH` frames carry the object and an attestation can ride beside it. Decide after Layer 9's research reports.
4. **Group state is the classic case, and L5 chose against it.** Groups v1 are pairwise fan-out with "no shared key state, trivial removal"
   (`5-messaging/README.md:484`; a group is "a local object"). Roster agreement would be the natural first multi-writer item; it is deferred, not needed.
5. **What must never enter a shared log.** An append-only log cannot erase, and every replica sees plaintext unless a group key is added. So:
   private or sealed keys and seeds; any message body (custody, ACK and `delivered` evidence already exist, `M/store/schema.rs:155-168`);
   alias, contact graph, who-talks-to-whom, location, presence; device identifiers; conversation or agent text; relay edges (topology); anything the
   subject has not consented to publish. Rule: only facts the author would sign on a public notice board.
6. **Conflict semantics without a wall clock.** (a) Single writer per key: no conflict; the author counter orders. (b) Multi-writer register:
   Lamport counter then author id as deterministic tie-break; a per-author hash chain gives a version vector for free (the heads). (c) Sets:
   union, no ordering. (d) Deletion: a signed tombstone by the same author. CRDT libraries use the same machinery internally (Loro LWW map on
   Lamport values: unverified). Never use entry timestamps: iroh-docs and, by the evidence available, Willow require them.
7. **The log id should be the L5 epoch.** L5 mints `epoch` from a fresh announce-counter value on crash, restore or `n` wrap, so a restored node
   never reuses a number (`5-messaging/README.md:192-199`). An identity-signed log restarted at seq 0 after restore-from-words would fork. Using
   `log_id = epoch`, `seq = n` reuses that guarantee (p2panda: "Authors can maintain one or many logs"; Bamboo has a 64-bit `log_id`). Order across
   a restore is `(epoch, seq)`: no clock.
8. **Cross-protocol signing needs domain separation.** p2panda signs its own CBOR header with an Ed25519 key. Signing it with the node identity key
   (`NodeId` is the Ed25519 pubkey) puts a second signing context on the identity key. Options: a derived log-signing subkey (an L2 amendment like
   A9's `store_key`, `M/keys.rs:1-12,101-114`), or a mandatory `111/log/v1` tag inside the first extension. Either is an L2 decision.
9. **Pruning by capacity fits single-writer logs and fights multi-writer documents.** With one writer per log, only the author may write a
   prune point (p2panda `PruneFlag`, plus a snapshot op), so there is nothing to agree on. Replicas of someone else's log are evictable payloads
   (headers kept, bodies dropped) and re-fetchable from the author or any hub. A CRDT document needs every peer to agree on a compaction base, which
   is the consensus problem the README forbids. Rule to carry: **no entry is the only copy of anything irreplaceable**; the author's own log is
   never capacity-pruned without the L5 step-3 export offer (`M/store/prune.rs:19-23`).
10. **The L5 seam needs two amendments before a consumer.** (i) `Class::Sync` rows have no prune step: `prune.rs` never names `Class::Sync`
    (grep), so a full store only refuses new SYNC (`M/store/rows.rs:90-97`). Add "SYNC replicas: bodies evict-first, like re-fetchable public
    chunks (step 1)". (ii) "Per-peer cursors" (`5-messaging/README.md:479`, `:831`) have no table. For author logs the replica rows are the
    cursor: `have = {(author, epoch): max seq}` is computed from the store, about 40 B per author, so a pull exchange (HAVE then OPS) needs no cursor
    table. A cursor table (Syncthing-style `(index_id, max_sequence)`) is only a push optimisation. Also: `requires_sig` is false for SYNC
    (`M/envelope.rs:564-583`), correct for the pairwise envelope, but transitive replication needs the author signature **inside** the opaque payload.
11. **Idempotency is free.** Op id = BLAKE3 of the header (p2panda's `digest`); replays are no-ops, and L5's `rx_state` already dedups the sealed
    frame (`5-messaging/README.md:202-213`). Frames stay `bulk-small` (at most 64 KiB, `:476-478`) and route to `Bulk` links by default, so LoRa
    carries nothing unless the user asks (rule 8; airtime in `0-knowledge/concepts/lora-airtime-budget.md`).
12. **Pi floor, quantified (estimates, unmeasured).** In-binary p2panda-core: adds `cbor-core` and a second `rand` major (0.8), reuses blake3 and
    ed25519-dalek already in the tree; binary delta expected tens of KB, to be measured with the L6 method. Resident cost: zero if sync runs only on
    link-up and contact events, which is the existing L5 retry rule (no timers; `0-knowledge/concepts/store-and-forward-states.md`). Disk: one
    committed insert per own op (`synchronous = FULL` only for sequence-allocating transactions, `M/store/mod.rs:424`); at 100 ops a day, about
    20 KB of payload and well under 1 MB of WAL page rewrites. A CRDT library in the binary is heavier and unmeasured; as a separate program none
    of these candidates has a daemon, so "separate program" only applies to Syncthing or Hypercore-JS, both rejected.
13. **Storage growth by candidate.** p2panda: prefix prune + body deletion, header chain about 170-190 B per op (estimate from the header fields, see the p2panda table row). Hypercore: clear ranges, tree retained. Loro: shallow snapshot. Automerge: none built in; compaction = fork.
    yrs: GC with a rising floor. iroh-docs/Willow: newest entry replaces older ones at the same key.

## 4. Recommendation

The state analysis (finding 1) gives a decision that depends on the owner, not on research: is there a user-facing multi-writer feature?

| Option | What | Trade-off |
|---|---|---|
| **A. No shared log in v1; seam stays reserved; write the admission gate (pick)** | Keep `Kind::Sync`/`Class::Sync` as is; record in the L8 spec: signed single-writer records with counters are the rule; a log needs a named consumer, a named writer per key, a never-list (finding 5) and a measured Pi delta. Fix L5 amendments (finding 10) when the first consumer arrives, not before. | Zero weight, zero code, honors "may". Risk: a later consumer pays a store bump (V5) and an L5 amendment. |
| B. A plus a design-only contract | As A, plus a spec appendix fixing the wire shape (HAVE/OPS inside `SYNC`, `log_id = epoch`, p2panda-shaped ops, 111 domain tag) with test vectors but no code. | Keeps the seam testable and the L2 subkey question settled early; costs doc work and may be overfit to one library before a consumer exists. |
| C. Per-author signed logs via p2panda-core at the first consumer (Layer 9 attestations, say) | In-binary crate, ops in our SQLCipher store (new V5 table), pull exchange over sealed `SYNC` frames or tcp-lan streams. | Adopted primitives, tiny, author-local pruning. Pre-1.0 wire churn (pin, store header bytes verbatim), an L2 signing-context decision, build weight needs the owner's say. |
| D. CRDT documents (Loro first, Automerge second) | Only for a collaborative-document feature (shared notes, a community board). | Heavy, history growth, coordinated compaction; Loro's shallow snapshot is the sole first-class answer. Not justified by any spec today. |

Pick: **A for v1, with C named as the adoption target** (so nobody invents a log later), and B only if the owner wants the seam proven by
vectors before Layer 9 lands. Reject D unless the owner asks for shared documents; then Loro, because history trimming is built in, over
Automerge (KB-recorded, but compaction breaks sync). Reject Syncthing/Hypercore-JS as separate programs: they add a resident runtime for no
state that needs it.

## 5. Open questions for the owner

1. Is there any multi-writer, user-visible shared object you want (a community notice board, shared checklists, group rosters)? If no, option A
   closes this lens.
2. May an identity-signed log use a derived log-signing subkey (an L2 amendment), or must everything sign with the node key under a domain tag?
3. Should Layer 9's "share proven fixes" be PATCH-frames-only (no log), or an attestation feed? Needs Layer 9's research result.
4. If C is ever chosen: may a new in-binary dependency (p2panda-core, `cbor-core`) ship in the single release binary that Pi images use, or must it
   sit behind a cargo feature absent from the Pi build? (AGENTS.md: no build-time weight without your say.)
5. Should a replica of another author's log be evictable by capacity before the author's own log (finding 9), accepting that an unreplicated tail
   can be lost?

## 6. Prior art (reference only)

- **Briar** (Bramble Sync Protocol): offline sync over Bluetooth/Wi-Fi, online over Tor; per-group message offers/requests (Wikipedia, CalyxOS docs,
  search snippets; spec page denied: details unverified). Confirms that group state is a distinct, harder problem.
- **Delta Chat**: no shared log; group membership travels in message headers (unverified here). Matches L5's choice.
- **Scuttlebutt / Bamboo / p2panda**: the per-author signed chain lineage; fork-on-restore and non-erasure are its known costs.
- **Earthstar**: v11 moves to Willow (README of earthstar-project/earthstar); Willow itself is the pruning-in-the-data-model answer.
- **Matrix**: a state DAG with a resolution algorithm; its complexity is the argument for not having multi-writer state without need (unverified,
  not fetched).

## 7. Sources

Local: `/home/user/os/README.md` section 8 and the layer table; `5-messaging/README.md` lines 4, 12, 160-175, 188-215, 424, 470-480, 700-712, 829-831;
`M/envelope.rs:505-583`; `M/store/rows.rs:30-120`; `M/store/prune.rs:1-40`; `M/store/schema.rs:60-170,278`; `M/keys.rs:1-12,95-115`; `ID/attest.rs:111`;
`2-identity/README.md:107,135-137,162,218`; `3-intelligence/README.md:50`; `plan/layers/7-relay/research/00-code-seams.md`; `7-relay/README.md` section 7;
`0-knowledge/projects/{automerge,briar-mailbox,iroh-nodeid}.md`; `0-knowledge/concepts/{store-and-forward-states,lora-airtime-budget}.md`.

Web (fetched 2026-11-02): crates.io API for automerge, autosurgeon, loro, yrs, crdts, hypercore, iroh-docs, iroh-willow, willow-data-model, p2panda-core,
-sync, -store, kuska-ssb, diamond-types (https://crates.io/api/v1/crates/<name>); raw.githubusercontent.com/automerge/automerge/main/LICENSE and
`rust/automerge/src/sync.rs`; loro-dev/loro LICENSE, README; y-crdt/y-crdt LICENSE, README; p2panda/p2panda `LICENSES/MIT.txt`, `LICENSES/Apache-2.0.txt`,
README, `p2panda-core/{README.md,Cargo.toml,src/prune.rs,src/operation/header.rs}`, `p2panda-sync/README.md`, `p2panda-store/README.md`; datrs/hypercore
LICENSE-MIT, README; datrs/hypercore-protocol-rs Cargo.toml; holepunchto/hypercore LICENSE, package.json; n0-computer/iroh-docs LICENSE-MIT, README,
Cargo.toml; rust-crdt/rust-crdt LICENSE; syncthing/syncthing LICENSE; syncthing/docs `specs/bep-v1.rst`; AljoschaMeyer/bamboo README;
ssbc/scuttlebutt-protocol-guide README; earthstar-project/earthstar README.
Search snippets: https://loro.dev/docs/performance/docsize, https://loro.dev/docs/tutorial/tips, https://automerge.org/docs/reference/glossary/,
https://automerge.org/docs/cookbook/modeling-data/, https://discuss.yjs.dev/t/are-gcd-documents-larger-than-non-gc-docs/1840,
https://discuss.yjs.dev/t/map-metadata-overhead/492, https://codeberg.org/Deln0r/willow-go, https://en.wikipedia.org/wiki/Briar_(software),
https://calyxos.org/docs/guide/apps/briar. Denied on this box: loro.dev, willowprotocol.org, codeberg.org, code.briarproject.org, ssbc.github.io.
