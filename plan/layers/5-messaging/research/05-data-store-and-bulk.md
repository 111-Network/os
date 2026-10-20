# Layer 5 research, lens 5: the data half (encrypted store, capacity pruning, bulk payloads)

Date 2026-10-14. Scout report for the Layer 5 options brief. Method: live web + upstream raw files (LICENSE, Cargo.toml, specs), repo sections named in the lens, and **throwaway size probes** (tiny crates in the session scratchpad, built with the repo's own musl cross toolchains and the workspace release profile: thin LTO, strip, panic=abort). No repo file changed except this report. "(unverified)" marks what could not be confirmed this run.

## 1. Summary

- **Store: adopt rusqlite + SQLCipher** (`bundled-sqlcipher-vendored-openssl`). Delta Chat's Rust core ships exactly this feature set as its default. It builds musl-static for x86_64 and aarch64 with our toolchains (measured). Cost: **+6.4 MB (x86_64) / +5.4 MB (aarch64)** on the 7.0 / 5.7 MB `111` binary. Plain SQLite is +1.9 / +1.7 MB; redb +0.8 / +0.6 MB.
- **Keys:** one 32-byte store key fed to SQLCipher as a raw key, behind a `KeyProvider` seam. Default derives it with HKDF from the L2 keyset, so a salvaged DB opens after restore from words. Cat 4 with TPM2 or SE050B seals a random key instead (L2 §5). Attachments are separate files; exports use `age` (its X25519 recipient is our `enc_pubkey`).
- **Pruning is capacity-only, LXMF-style:** old and large first, with priority classes. Undelivered outbound is never pruned; the store refuses new custody and shows a visible `store_full` state instead. Age is a local insertion sequence, not a wall clock.
- **Bulk: one content address (BLAKE3 root) and one verified-stream primitive (bao-tree) for every link tier.** Measured: the root hash is identical at 1 KiB and 16 KiB chunk groups, so LoRa and TCP share addresses. `bao-tree`+`blake3` cost +94 KB; `iroh`+`iroh-blobs` cost **+13.3 MB** and need QUIC (1200 B minimum datagram), so they are a LAN-tier option only, not the fabric.
- **LoRa carries kilobytes, not megabytes.** At LongFast, a 4 KB manifest is ~19 frames; a 32 KB KB delta ~5 min of airtime; the 5.7 MB binary ~15 h at 100% duty. Binaries, APKs, models and maps are LAN/TCP or sneakernet only.
- **Multi-device is unsolved offline by every adopted project** (Delta Chat uses a shared mailbox plus a QR-started iroh transfer; Briar has none, unverified). Recommend hub-is-home for v1, replica sync later. Layer 8's log is a seam only.

## 2. Candidates

| Name | Upstream | License (verified?) | Lang / runtime | Maturity signals | Fit | Why |
|---|---|---|---|---|---|---|
| rusqlite + libsqlite3-sys | github.com/rusqlite/rusqlite | MIT (yes, LICENSE) | Rust + C SQLite | 0.40.2 on 2026-08-08; 125M downloads | **adopt** | Only mainstream Rust path to SQLCipher; bundles SQLCipher 4.17.0 source |
| SQLCipher 4.x community | github.com/sqlcipher/sqlcipher | BSD-3-Clause style, Zetetic (yes, LICENSE.md) | C | 4.19.0 on 2026-09-08; 4.17.0 bundled by rusqlite | **adopt** | Page-level AES-256-CBC + HMAC-SHA512; raw-key mode skips PBKDF2 |
| OpenSSL via openssl-src (vendored) | github.com/alexcrichton/openssl-src-rs | Apache-2.0 for OpenSSL 3.x (yes); wrapper MIT (yes) | C | 3.6.3 (openssl-src 300.6.1) | **adopt** (as SQLCipher's crypto) | Needed for static musl; builds in ~2m15s cold |
| SQLCipher 5.0.0-beta | same repo | BSD-style (yes) | C | beta 2026-09-15 | reference | AES-256-GCM, 8 KiB pages; **incompatible default**, needs migration |
| SQLite3 Multiple Ciphers (`sqlite3mc-src`) | github.com/utelle/SQLite3MultipleCiphers | MIT (yes, LICENSE) | C | Rust crate has 3k downloads | reference | ChaCha20 option, no OpenSSL; thin Rust ecosystem |
| redb | github.com/cberner/redb | MIT OR Apache-2.0 (yes) | pure Rust | 4.4.0 on 2026-10-13; stable file format | reference | Smallest; but you hand-build indexes and encryption |
| fjall | github.com/fjall-rs/fjall | MIT OR Apache-2.0 (MIT file yes) | pure Rust LSM | 3.1.12 on 2026-10-03 | reference | Write-heavy; same hand-built burden |
| sled | github.com/spacejam/sled | MIT OR Apache-2.0 (crates.io) | Rust | last release 1.0.0-alpha.124, 2024-10 | **reject** | Alpha, stale |
| age / rage | github.com/str4d/rage | MIT OR Apache-2.0 (MIT file yes) | Rust | 0.12.1 on 2026-07-14 | **adopt** (exports only) | 168 B header, 64 KiB STREAM chunks, X25519 recipients |
| bao-tree + blake3 | github.com/n0-computer/bao-tree; BLAKE3-team | MIT OR Apache-2.0 (yes); CC0/Apache (yes) | Rust | bao-tree 0.16.1 on 2026-08-26 | **adopt** | Verified streaming, range sets, runtime chunk groups, transport-agnostic |
| iroh + iroh-blobs | github.com/n0-computer/iroh-blobs | MIT OR Apache-2.0 (yes) | Rust, QUIC | iroh 1.3.0; blobs 0.103.1 pre-1.0, "not yet production quality" | **bridge** | NodeId equals EndpointId; LAN tier only; +13.3 MB |
| Syncthing | github.com/syncthing/syncthing | MPL-2.0 (yes) | Go | v2.1.6 on 2026-10-06; Termux package 2.1.6 | **bridge** | Separate process, own panel, LAN/Internet hub-to-hub sync |
| Reticulum Resources / rncp, LXMF propagation | github.com/markqvist/LXMF | Reticulum License, custom non-OSI (yes) | Python | LXMF overhead 111 B | reference (bridge target) | Connected service only; patterns from docs |
| git (bundle) | github.com/git/git | GPL-2.0 only (yes, COPYING) | C | n/a | **adopt** (hub tool, separate process) | Proven sneakernet pattern for KB |
| zstd `--patch-from` | github.com/facebook/zstd | BSD (yes, LICENSE) | C | 0.14.2 Rust wrapper 2026-10-10 | **adopt** (deltas) | Reference-file delta; not measured here |
| BPv7: dtn7-rs, Hardy | github.com/dtn7/dtn7-rs | MIT (yes); Hardy Apache-2.0 (crates.io) | Rust | dtn7 2024-03; Hardy 0.2.0 2026-07 | reference | Mandatory lifetime, finding 10 |
| Automerge 0.12 (+ samod), Loro 1.16, yrs 0.28 | github.com/automerge/automerge etc. | MIT (all yes) | Rust | all released Aug-Sep 2026 | reference (L8 seam) | Seam only; Automerge is the KB entry |
| p2panda 0.7, iroh-docs 0.101, willow_rs 0.8 | github.com/p2panda/p2panda | MIT OR Apache-2.0 (crates.io; p2panda MIT yes) | Rust | pre-1.0 | reference (L8 seam) | Append-log alternatives |
| Delta Chat core, Briar, Signal Desktop, Sideband | chatmail/core, briar/briar, signalapp/Signal-Desktop, markqvist/Sideband | MPL-2.0 / GPL-3.0 (README) / AGPL-3.0 / CC BY-NC-SA 4.0 (all yes) | Rust, Java, TS, Python | all active | reference | At-rest patterns, findings 3-4 |

## 3. Findings that change the plan

1. **`bundled-sqlcipher` alone is not static.** It compiles the bundled SQLCipher 4.17.0 amalgamation but links `libcrypto` dynamically; only `bundled-sqlcipher-vendored-openssl` builds OpenSSL in. https://raw.githubusercontent.com/rusqlite/rusqlite/master/libsqlite3-sys/build.rs and `upgrade_sqlcipher.sh` (pins 4.17.0).
2. **Measured, musl-static, our release profile** (probe crates, base = hello world 414,472 B x86_64 / 377,008 B aarch64):

   | Dependency | x86_64 delta | aarch64 delta |
   |---|---|---|
   | XChaCha20-Poly1305 + HKDF-SHA256 | +37 KB | +16 KB |
   | bao-tree + blake3 | +94 KB | +29 KB |
   | redb 4.4.0 | +811 KB | +618 KB |
   | fjall 3.1.12 | +1,045 KB | +889 KB |
   | SQLite (rusqlite `bundled`) | +1,907 KB | +1,735 KB |
   | SQLCipher + vendored OpenSSL 3.6.3 | **+6,427 KB** | **+5,357 KB** |
   | iroh 1.3 + iroh-blobs 0.103 (default features, mem store) | **+13.3 MB** | not built |

   The SQLCipher probe is static-pie with no NEEDED libs and the aarch64 build keeps the 16 KiB segment alignment. The probe's DB header was random bytes (plain SQLite shows `SQLite format 3`). Against today's `111` (7,026,024 B x86_64; 5.7 MB aarch64) that is about +92% for SQLCipher, +27% for plain SQLite. Cold SQLCipher build: ~2m15s, needing perl, make and the cross gcc already in `~/.local/toolchains`. iroh feature trimming was not tried. L1's dependency policy is frozen and prefers pure-Rust backends (`1-bootstrap/TASKS.md`), so C needs a recorded sign-off; every SQL option needs C, only redb and fjall are pure Rust.
3. **Delta Chat core is the closest precedent.** `rusqlite` with `bundled-sqlcipher-vendored-openssl` is its default `vendored` feature (MPL-2.0). Caveat: its passphrase is optional, so the DB is plaintext by default. Blobs are separate files, and `blob.rs` has no encryption code. https://raw.githubusercontent.com/chatmail/core/main/Cargo.toml, https://raw.githubusercontent.com/chatmail/core/main/src/sql.rs
4. **At-rest patterns across adopted projects.**
   - Briar: H2 with `CIPHER=AES`; a random 32-byte DB key stored as `db.key`, encrypted under a password-derived key, with an optional hardware "key strengthener". https://raw.githubusercontent.com/briar/briar/master/bramble-core/src/main/java/org/briarproject/bramble/db/H2Database.java, `account/AccountManagerImpl.java`.
   - Signal Desktop: SQLCipher (`@signalapp/sqlcipher` 4.1.0, AGPL-3.0). The key moved from plaintext `config.json` to Electron `safeStorage`; a keyring backend change can strand the DB (third-party write-up, https://www.yingtongli.me/blog/2025/08/13/signal-secrets.html). Lesson: OS-bound keys need a recovery path.
   - Sideband: plain Python `sqlite3` file `sideband.db`, no `PRAGMA key` in `core.py`. https://raw.githubusercontent.com/markqvist/Sideband/main/sbapp/sideband/core.py
   - LXMF: messages are end-to-end encrypted to the recipient, so a propagation node stores ciphertext it cannot read. https://raw.githubusercontent.com/markqvist/LXMF/master/README.md
5. **SQLCipher format is about to fork.** 5.0.0-beta (2026-09-15) moves to AES-256-GCM, VFS shims, 8 KiB pages and 512K KDF iterations, and is incompatible with 4.x by default. https://www.zetetic.net/blog/2026/09/15/sqlcipher-5.0.0-beta/ rusqlite lags upstream (bundles 4.17.0 vs 4.19.0). We must pin a 4.x file format and plan `cipher_migrate`.
6. **SQLCipher operational rules.** Raw key `x'<64 hex>'` skips PBKDF2 (256,000 iterations by default; too slow for phones at every open). Transient files are not encrypted, so keep temp storage in memory; rusqlite's bundled build already uses `SQLITE_TEMP_STORE=3` on Android. Page overhead: 16 B IV + 64 B HMAC-SHA512 per 4,096 B page ≈ **1.95%** (derived; exact reserve unverified). https://www.zetetic.net/sqlcipher/sqlcipher-api/ and /design/
7. **Android hardware custody is weaker than assumed.** `termux-keystore` supports only RSA/EC `generate/sign/verify/delete/list`: no AES key, no wrap/unwrap. Termux:API v0.53.0 is already in L1's APK bundle. A deterministic-signature KDF trick is possible but unverified. https://raw.githubusercontent.com/termux/termux-api-package/master/scripts/termux-keystore.in
8. **L2 already reserves the key story.** Cat 4: TPM2 holds the **storage key, not the keyset** (L2 §5). A BIP-85-style subtree for a "storage key" is reserved but unbuilt (§8.1, §15). The mnemonic-at-rest residual (§12) means a seed-derived key is only as safe as the disk. L2 `counters` and `peers` stay owned by `111id`; L5 must not absorb them (§13).
9. **LXMF's pruning knobs are the documented pattern** (read as config documentation; license is the Reticulum License, so reference only): `message_storage_limit` default 500 MB, "prioritises keeping messages that are new and small; large and old removed first"; per-message cap 256 KB; per-sync cap 10 MB, smallest first; `prioritise_destinations`; a stamp-cost (proof of work) gate. https://raw.githubusercontent.com/markqvist/LXMF/master/LXMF/Utilities/lxmd.py. Meshtastic Store & Forward is a ring buffer of ~11,000 records, default 240 min window, 25 per request. https://meshtastic.org/docs/configuration/module/store-and-forward-module/ Briar has no capacity pruning (user and timer driven). Syncthing refuses rather than prunes: `minDiskFree` default 1%. https://docs.syncthing.net/users/config.html
10. **BPv7 does not fit "no expiry".** Lifetime is a mandatory primary-block field; expiry means the BPA MUST delete; storage pressure handling is "an implementation matter". Fragmentation reassembles at the destination. https://www.rfc-editor.org/rfc/rfc9171.html Reference for fragmentation only; Rust implementations are young (Hardy 0.2.0) or stale (dtn7, 2024-03).
11. **iroh-blobs is QUIC-coupled and pre-1.0.** Its README: "not yet considered production quality; use 0.35"; a request opens a QUIC stream and answers with BLAKE3 verified streams. QUIC forbids paths under 1200 B datagrams, so it cannot ride ~200 B LoRa. https://raw.githubusercontent.com/n0-computer/iroh-blobs/main/README.md, https://www.rfc-editor.org/rfc/rfc9000.txt (§14). iroh custom transports are behind `unstable-custom-transports` (https://iroh.computer/blog/iroh-1-0-0-rc-1). The `presets::N0` setup relies on n0 relay and discovery servers (KB: `projects/iroh-nodeid.md`), which we would have to switch off; Delta Chat still pins iroh 0.35 (API churn). Upside: our NodeId is a valid iroh EndpointId. iroh-blobs over a non-QUIC stream: unverified.
12. **Verified streaming without iroh is a ~100 KB job.** Probe: a 1 MiB buffer outboarded at 16 KiB chunk groups gave a 4,032 B outboard (0.38%) and a root **equal to `blake3::hash`**. By construction, 1 KiB groups cost 64 B per KiB (6.25%). bao-tree supports sets of ranges and sync/async code. https://raw.githubusercontent.com/n0-computer/bao-tree/main/README.md, https://docs.rs/bao-tree/latest/bao_tree/struct.BlockSize.html
13. **LoRa budget** (Semtech AN1200.13 formula, 255 B frame, CRC on): LongFast SF11/250 kHz **2.09 s**; MediumFast SF9 0.63 s; ShortFast SF7 0.20 s (Meshtastic presets: https://meshtastic.org/docs/overview/radio-settings/). Payload ceilings: Meshtastic 233 B (L2 §6), MeshCore 184 B (https://docs.meshcore.io/packet_format/), Reticulum MTU 500 B with 465 B data and 297 B to establish a link (https://reticulum.network/manual/understanding.html). At ~124 B app payload per sealed frame (233 - 56 seal - ~50 inner header, assumed) versus ~220 B for an unsealed chunk (assumed):

   | Object | Frames (unsealed / sealed) | LongFast airtime | ShortFast airtime |
   |---|---|---|---|
   | 4 KB channel manifest (4,019 B today) | 19 / 33 | 40 s / 69 s | 4 s / 7 s |
   | 31.7 KB KB delta (gz diff, last 9 KB-touching commits) | 145 / 256 | 5.0 / 8.9 min | 29 / 51 s |
   | 52 KB full KB snapshot (gz) | 238 / 423 | 8.3 / 14.7 min | 48 / 85 s |
   | 5.7 MB `111` binary | 25,910 / 45,968 | 15.0 / 26.7 h | 1.4 / 2.6 h |
   | 1 GB model | 4.5M / 8.1M | 2,640 / 4,684 h | 252 / 448 h |

   Single-hop ideal figures: no ACKs, retransmits, flooding or channel sharing. EU 868 duty cycles (1% typical, 10% in 869.4-869.65 MHz) multiply wall time 10 to 100x; that is law, not node policy, but the agent must see it. Sealing every bulk frame costs ~1.8x airtime.
14. **Real KB sizes.** `0-knowledge/` is 36 Markdown files: 204,800 B tar, 52,450 B gz; the diff over the last 9 KB-touching commits is 101 KB raw, 31.7 KB gz. A whole-repo incremental `git bundle` of 10 commits is 123,501 B; the full 151-commit history bundle is 1.78 MB. L1 ships KB inside the release payload and defers "deltas" and "KB git-sync" to L6/L7 (`1-bootstrap/README.md`). Git bundles with prerequisites and `verify` are the documented offline pattern. https://git-scm.com/docs/git-bundle
15. **Syncthing is a bridge, not a fabric.** Block exchange: 128 KiB to 16 MiB blocks, device ID = SHA-256 of the TLS cert, per-block hash requests (https://docs.syncthing.net/specs/bep-v1.html). The official Android app is discontinued (Dec 2024); Syncthing-Fork (MPL-2.0) continues, and Termux packages Syncthing 2.1.6. https://raw.githubusercontent.com/syncthing/syncthing-android/main/README.md It trusts devices, not publishers, so content still needs our signed manifest.
16. **`age` fits exports.** One-recipient header is 168 B plus a 16 B nonce; payload is 64 KiB ChaCha20-Poly1305 chunks; the recipient is a raw X25519 public key, so a phone can encrypt a backup to the hub's `enc_pubkey` with no new key type. https://raw.githubusercontent.com/C2SP/C2SP/main/age.md This matches L2 §8.5's deferred "recipient-mode export".
17. **Multi-device.** Delta Chat: "Add Second Device" is a QR code (peer ID + auth token) starting a one-shot iroh QUIC stream of the DB and blobs; new messages reach every device through the shared mailbox/relay. https://raw.githubusercontent.com/chatmail/core/main/src/imex/transfer.rs, https://delta.chat/en/help Same-seed duplication is an **error state** in L2 (identity-collision, §6.2/§8.4), so one person on two devices means two NodeIds. Briar: no multi-device (unverified; FAQ unreachable). Sideband/LXMF: identity import exists; concurrent use unverified.
18. **CRDT / log seam (L8).** Automerge 0.12.0 (+ `samod` 0.15.0, an automerge-repo in Rust), Loro 1.16.2, yrs 0.28.0 are all MIT and Rust-cored; p2panda 0.7.1 and willow_rs 0.8.0 are pre-1.0 append-log alternatives. Seam only: sync ops are an opaque message class with idempotent IDs and per-peer cursors, chunkable via the bulk path.

## 4. Recommendation

**Fork A: store engine.**
- **A1 SQLCipher via rusqlite (pick).** Adopted, in production in Delta Chat, SQL gives dedupe (`UNIQUE(id)`), threads, FTS and migrations for free. Cost: +6.4 / +5.4 MB, a C and OpenSSL build, and a 4.x-to-5 migration later.
- A2 plain SQLite + our per-record XChaCha20-Poly1305: +1.9 MB, L2-native primitives, but we invent and audit a record format and leak metadata.
- A3 redb/fjall + per-value AEAD: smallest and pure Rust, but hand-built indexes and search; fits Cat 1 flash rings better than the dashboard's Messages route.
- A4 plaintext DB on disk encryption: impossible in Termux.

**Store layout (pick).** One SQLCipher DB with raw key, WAL, in-memory temp, `auto_vacuum=INCREMENTAL` so pruning frees disk (SQLite does not shrink on delete; unverified, sqlite.org/pragma.html). Messages and metadata in tables; bulk blobs as files named by BLAKE3 hash with a refcount table. Private blobs are encrypted once with a random object key carried inside the sealed envelope and addressed by ciphertext hash (relays can verify them without reading). Public signed blobs (KB, updates, patches) are stored plain: encryption adds nothing. Row metadata: `id` (32 B), `class` (human, agent, kb, update, patch, sync), `size`, `seq` (local monotonic, the age hint), `dir` (own / inbound / transit), `state` (queued, sent, acked, delivered), `pinned`, blob refs. Wall-clock arrival is display-only.

**Key custody (pick).**
- `KeyProvider` seam, two modes. **Mode S** (default, Cat 2/3/4 without TPM, phones): store key = HKDF-SHA256 from the L2 keyset with info `111/store/v1`. **Mode T** (Cat 4 with TPM2 or SE050B, as L2 §5 says): a random key sealed in hardware.
- Mode S needs a small L2 sign-off (a domain-separated KDF off the keyset, or an `m/111'/0'/2'` sibling path with a test vector). Its limit: it protects DB-only copies, backups and synced app dirs, not whole-disk theft.
- Signal lesson: never bind to an OS keyring without a recovery path.

**Pruning (pick).**
- Trigger on live bytes (pages minus freelist, plus blob dir) crossing a high watermark, with a free-disk floor like Syncthing's 1%.
- Order: (1) transit custody and re-fetchable public chunks, weight = size x age, old and large first (LXMF); (2) superseded releases below the monotonic floor and fully fetched KB deltas; (3) own conversation history oldest-first only at hard capacity, after an export offer; (4) never undelivered outbound, instead refuse new custody and surface `store_full` to the agent.
- Per-class quotas are capacity shaping; per-sender limits would be policy and are out.

**Fork B: bulk transfer.**
- **B1 bao-tree + blake3 over the L5 envelope (pick).** One address everywhere; chunk groups of 1 KiB (+6.25% outboard) on LoRa/BLE, 16 KiB or more (+0.39%) on LAN; range-set requests so different carriers can bring different pieces (DTN-friendly); resume at any boundary; +94 KB.
- B2 iroh-blobs for the LAN tier only: free QUIC stack, but +13.3 MB, pre-1.0, n0 defaults to disable. Keep as optional L6 online link since NodeId equals EndpointId.
- B3 delegate to Syncthing (hub to hub, models, maps) and rnsd Resources (Reticulum-reachable peers) as dashboard services. No custom code, but separate identities: bridges, not the fabric. Syncthing could become a registry panel this cycle if the owner agrees.
- Signed manifest on top: reuse L1's minisign channel for releases; peer-published objects get a domain-tagged Ed25519 signature (L2 §6.1). Updates: whole payload first (L1 v1); `zstd --patch-from` deltas later (unmeasured).

**Fork C: private bulk framing on LoRa/BLE (needs L2 sign-off).**
- C1 seal every frame (L2-pure): +56 B per frame, ~1.8x airtime.
- C2 hybrid (pick): the request names the hash and opens a session tag, and is sealed; data frames carry only a random session tag and a BLAKE3-verified chunk of an already end-to-end-encrypted or public object, unsealed. Online links (Noise IK / TLS RPK, L2 §6.6) are already encrypted.

**Fork D: multi-device.**
- **D1 hub-is-home (pick for v1).** Messages live on the hub; phones and laptops are companions through the dashboard (already built, D15). A Termux phone node is its own identity when out of range.
- D2 replica sync between owned devices using Delta Chat's ceremony (QR with peer ID + token, then an authenticated one-shot stream) plus per-origin sequence cursors; needs L2's owner attestation (F1).
- D3 sender fan-out to every linked NodeId: N times the LoRa airtime.
- D4 same seed on two devices: forbidden by L2 identity-collision.

## 5. Open questions for the owner

1. **Store engine:** accept +6.4 / +5.4 MB and a C/OpenSSL build for SQLCipher (A1), or prefer the smaller plain-SQLite or pure-Rust routes (A2/A3) at the cost of writing and auditing our own record encryption? A1 needs a recorded dependency sign-off.
2. **Unsealed data frames** for bulk on LoRa/BLE (Fork C2): is L2's "every protected packet is sealed" allowed to carve out hash-verified chunk frames of public or already-encrypted objects? Needs identity-layer sign-off.
3. **Store key:** is a keyset-derived key (Mode S) acceptable, or do you want the BIP-85-style subtree L2 reserved built now? Should Cat 4 TPM mode be opt-in or automatic?
4. **Own history at capacity:** auto-prune oldest with an export offer, or stop and ask? Do LXMF-style proof-of-work stamps on transit custody count as "policy" under the no-limits rule?
5. **Multi-device v1:** hub-is-home only, or invest in owner-linked devices now (L2 attestation, D2/D3)?
6. **Bridge services this cycle:** add Syncthing (and a Reticulum Resource/rncp row) to the services registry with panels, or only Meshtastic as decided?
7. **iroh as an optional LAN link:** allowed with n0 relays and discovery off, using our own Ed25519 key (needs L2 sign-off)?
8. **Do hubs hold other people's private objects** (propagation-node role)? Layer 7, but it sets store quotas.

## 6. Sources

All under https://raw.githubusercontent.com/ unless a full URL is given.
- rusqlite/rusqlite/master: `Cargo.toml`, `LICENSE`, `libsqlite3-sys/{build.rs,upgrade_sqlcipher.sh,sqlcipher/LICENSE}`; sqlcipher/sqlcipher/master: `LICENSE.md`, `CHANGELOG.md`
- https://www.zetetic.net/sqlcipher/design/ · https://www.zetetic.net/sqlcipher/sqlcipher-api/ · https://www.zetetic.net/blog/2026/09/15/sqlcipher-5.0.0-beta/
- https://crates.io/api/v1/crates/{rusqlite,libsqlite3-sys,openssl-src,redb,fjall,sled,bao-tree,iroh,iroh-blobs,blake3,age,rage,automerge,samod,loro,yrs,libsql,dtn7,bp7,hardy-bpa,p2panda-core,iroh-docs,willow-data-model,sqlite3mc-src,zstd}
- cberner/redb/master: `README.md`, `docs/design.md`, `LICENSE-MIT`, `LICENSE-APACHE`; fjall-rs/fjall/main/LICENSE-MIT; str4d/rage/main/LICENSE-MIT; C2SP/C2SP/main/age.md; utelle/SQLite3MultipleCiphers/main/LICENSE
- chatmail/core/main: `Cargo.toml`, `LICENSE`, `src/{sql,sync,blob}.rs`, `src/imex/transfer.rs`; https://delta.chat/en/help
- briar/briar/master: `bramble-core/.../db/H2Database.java`, `.../account/AccountManagerImpl.java`, `README.md`
- signalapp/Signal-Desktop/main: `package.json`, `LICENSE`; https://www.yingtongli.me/blog/2025/08/13/signal-secrets.html
- markqvist/Sideband/main: `sbapp/sideband/core.py`, `LICENSE`; markqvist/LXMF/master: `README.md`, `LICENSE`, `LXMF/Utilities/lxmd.py`; https://reticulum.network/manual/understanding.html
- n0-computer/iroh-blobs/main: `README.md`, `Cargo.toml`, `LICENSE-MIT`; n0-computer/bao-tree/main: `README.md`, `LICENSE-MIT`; n0-computer/iroh/main/LICENSE-MIT; https://docs.rs/bao-tree/latest/bao_tree/struct.BlockSize.html; https://iroh.computer/blog/iroh-1-0-0-rc-1; https://iroh.computer/blog/iroh-0-97-0-custom-transports-and-noq
- oconnor663/bao/master: `README.md`, `docs/spec.md`, `LICENSE`; BLAKE3-team/BLAKE3/master/LICENSE_CC0
- https://www.rfc-editor.org/rfc/rfc9000.txt · https://www.rfc-editor.org/rfc/rfc9171.html
- https://docs.syncthing.net/specs/bep-v1.html · https://docs.syncthing.net/users/config.html · syncthing/syncthing/main/LICENSE · syncthing/syncthing-android/main/README.md · researchxxl/syncthing-android/main/README.md · termux/termux-packages/master/packages/{syncthing,sqlcipher}/build.sh
- https://git-scm.com/docs/git-bundle · git/git/master/COPYING · facebook/zstd/dev/{programs/zstd.1.md,LICENSE} · termux/termux-api-package/master/scripts/termux-keystore.in
- https://meshtastic.org/docs/overview/radio-settings/ · https://meshtastic.org/docs/configuration/module/store-and-forward-module/ · https://docs.meshcore.io/packet_format/
- automerge/automerge/main/LICENSE · loro-dev/loro/main/LICENSE · y-crdt/y-crdt/main/LICENSE · p2panda/p2panda/main/LICENSES/MIT.txt · dtn7/dtn7-rs/master/LICENSE-MIT · openssl/openssl/master/LICENSE.txt · alexcrichton/openssl-src-rs/main/LICENSE-MIT
- Repo: `os/2-identity/README.md` §2, §5, §6, §8.1, §12, §13, §15 · `os/plan/cross-layer/devices.md` · `os/plan/DECISIONS.md` D13, D17 · `os/LICENSING.md` L20-35 · `os/1-bootstrap/{README.md,TASKS.md,scripts/cross-build.sh}` · `os/0-knowledge/projects/{iroh-nodeid,automerge,briar-bramble}.md`
- Probe builds: session scratchpad (`bench/`, `bench2/`), throwaway, not committed.
