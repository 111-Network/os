# Layer 8 research 02 - moving large objects between nodes and between hubs

Lens 2 of the Layer 8 sweep. Written 2026-11-02 by a sonnet scout. Box network: raw.githubusercontent.com, crates.io, pypi.org,
proxy.golang.org reachable; docs.rs, rfc-editor.org, github.com HTML, protomaps.com, mediawiki.org and download.kiwix.org denied or
unresolvable. Every licence below was read from an upstream file on raw.githubusercontent.com or a `Cargo.toml` unless marked
"(unverified)". Release dates come from crates.io (`updated_at`) or proxy.golang.org tag times. Claims taken from a WebSearch
result carry "(search snippet)". Resident-weight figures are almost all published or anecdotal; nothing was run on this box.
Code seams are cited as in `00-code-seams.md` (`M/` = `5-messaging/crates/mesh111/src/`, `N/` = `1-bootstrap/crates/node111/src/daemon/`).

## 1. Summary

- **v1 bulk path = HTTP GET with `Range` + a signed catalogue that pins sha256 per file AND a sha256 list per 16 MiB chunk.** It needs no
  new crate (`sha2` and `ureq` are in the lock), no new process and no new listener. It adds a streaming, resumable fetcher
  (today's `Fetcher::get` reads the whole body into memory under a global timeout, `NI/util/http.rs:26-27,36`; the updater fetcher caps
  at 512 MiB, `N/updater/fetch.rs:100`) and a single-range arm in the hub's file route (no `Range` exists anywhere, `00-code-seams.md` finding 9).
  A 57 GB ZIM has 3,397 chunks and a 109 KB sidecar; a bad chunk costs 16 MiB, not 57 GB.
- **Sneakernet is the same layout on a disk.** A catalogue + `.minisig` + `blob/<sha256>` directory is exactly L7's origin shape
  (D30 F6); `111 update --from <dir|url>` extends to datasets with no new verification model. A USB stick needs exFAT/ext4 (FAT32 caps a file at 4 GiB, a
  generic filesystem fact; a ZIM above that cannot live there unsplit).
- **Tier 2 (`OBJ_*`) should be owned by Layer 8**, built in v1.1 inside `mesh111` as a Layer 5 amendment, and carried hub-to-hub on L7's
  TLS-RPK hub link. Layer 8 is the first consumer with bulk; Layer 7 has none (UPDATE payloads fit the HTTP origin). Two prerequisites:
  bump `bao-tree` 0.16 to 0.17 (its fixes sit on exactly the range-set paths Tier 2 uses) and give bulk its own connection or a
  non-blocking `Link::send` (the fabric awaits `send` inside its actor, L7 options fact 1).
- **Everything else is a v1.1+ optional component or a rejection.** iroh-blobs (same BLAKE3 root and 16 KiB groups as ours, but "not yet
  production quality" and 29 normal deps behind `iroh`) and rqbit (Apache-2.0, "a few tens of MB", systemd-activatable) are the two worth
  keeping on the shelf; Syncthing, IPFS/kubo, Hypercore, rsync/zsync and casync/desync are rejected or reference only.
- **Delta transfer buys nothing in v1.** ZIM has no incremental updater in the Kiwix ecosystem (`zimdiff`/`zimpatch` exist in zim-tools
  but diff at item level with a `[FIXME]` for redirects); a ZIM refresh is a full file. PMTiles regional files are small enough to refetch.
  Content-defined chunking over zstd/gzip-compressed payloads dedups poorly (inference, not measured).
- **The metered-link cap must cover datasets.** One 57 GB fetch is 1,000x any other traffic L7 caps; at a 1 GB/day cap it is 57 days.
  Enforcement belongs in the single fetch function, with a size-stating consent (the `409 consent_required` shape, `N/components/mod.rs:13-15`).

## 2. Candidates table

| Name | Upstream | Licence (verified?) | Language / runtime | Maturity | Resident weight | Fit | Why |
|---|---|---|---|---|---|---|---|
| HTTP GET + Range + sha256 catalogue (chunk-hash sidecar) | our glue over HTTP | n/a (our GPL-3.0-or-later); `sha2` in lock | Rust, `ureq` + `sha2` already linked | universal; libkiwix already answers `Range` (`libkiwix/src/server/response.cpp:691-720`, `Accept-Ranges: bytes`, 416 handling, verified) | **0**: a route on the existing listener while `hub_role` + `origin.serve`; a fetch thread only during a fetch; one 64 KiB buffer per stream | **adopt (v1)** | Simplest transfer that exists; resume at any chunk; mirrors untrusted. |
| bao-tree 0.17.0 + blake3 1.8.7 | github.com/n0-computer/bao-tree | MIT OR Apache-2.0 (`Cargo.toml` verified); blake3 CC0-1.0 OR Apache-2.0 (from `LICENSING.md:142`, not re-fetched) | Rust | bao-tree 0.17.0 released 2026-10-22 (crates.io; 1.12 M downloads); lock has 0.16.1, `mesh111/Cargo.toml:20` says `"0.16"` | 0 (library; +94 KB x86_64 / +29 KB aarch64 already paid, KB `blake3-bao`) | **adopt (v1.1 Tier 2)** | Verified streaming + range sets + resume; 0.17 fixes `encode_ranges` for partial chunk groups and decode-after-error. |
| iroh-blobs 0.104.0 (+ iroh 1.4.1) | github.com/n0-computer/iroh-blobs | MIT OR Apache-2.0 (`Cargo.toml` + LICENSE-MIT/-APACHE verified) | Rust, QUIC (`iroh`), tokio | iroh-blobs 2026-10-28; iroh 1.4.1 2026-10-21 (3.85 M downloads); README: "not yet considered production quality... use 0.35" (verified) | Binary +13.3 MB (KB `blake3-bao`, Layer 5 measurement); 29 normal deps (crates.io); example binds with `presets::N0` (n0 public relay/discovery by default, README); RSS unknown | **reference** | Same BLAKE3 root and 16 KiB groups as our TCP profile; wrong shape for an offline hub (QUIC + relay story, 29 deps, unstable). Re-open only as a hub-only component if Tier 2 proves too costly. |
| rqbit 9.1.0 / librqbit 9.1.0 | github.com/ikatson/rqbit | Apache-2.0 (LICENSE verified) | Rust; `librqbit` 56 normal deps incl. tokio, reqwest, axum-extra, DHT, uTP, UPnP (crates.io) | 2026-10-29; rqbit 30.5 k, librqbit 174 k downloads | README: "usually within a few tens of megabytes... great for RaspberryPI" (published claim, unmeasured); HTTP API on 127.0.0.1:3030; systemd socket activation documented; Termux unverified | **bridge (v1.1+, optional)** | BEP-14 LSD, BEP-27 private, `.torrent` import, Range streaming. Pieces are SHA-1 (BEP 52 not in its supported list), so the catalogue's whole-file sha256 stays the trust anchor. |
| libtorrent (arvidn) 2.0 | github.com/arvidn/libtorrent | BSD-3-Clause (3-clause text verified in LICENSE) | C++ | RC_2_0 branch header `2.0.15.0`; release date not fetched | library; unmeasured | reference | Supports v2 hashes; C++ build weight; use only if a packaged client is chosen. |
| Syncthing (v2 series) | github.com/syncthing/syncthing | MPL-2.0 (LICENSE verified) | Go, SQLite since 2.0 | v2.0.0 Aug 2025; v2.0.16 merged by a dependency bot 2026-04-07 (search snippet); proxy.golang.org lists only v1.30.0 (2025-06-20), so the current tag is unverified | forum anecdotes: ~500 MB for 300 GB / 667 k files, crashes on a 1 GB Pi 3B at 1.4 GB transfers, 2019-2021, LevelDB era (search snippet, stale); continuous scan/watch, discovery, relays | **reject** (v1/v1.1) | Block exchange sha256 128 KiB-16 MiB blocks (`docs/users/syncing.rst:12-30`, verified), send-only/receive-only/receive-encrypted folder types (`foldertype.go:14-17`) fit, but it is a resident second PKI and scanner for a handful of huge immutable files. |
| desync 1.1.5 / casync | github.com/folbricht/desync, github.com/systemd/casync | BSD-3-Clause (desync LICENSE verified); casync LGPL-2.1 (LICENSE.LGPL2.1 present, version suffix not verified) | Go / C | desync tag 2026-10-05 (proxy.golang.org); casync release unknown | none while off; chunk store = dumb HTTP files | **reject** | Excellent for disk images that change in place; ZIM/PMTiles are compressed (see finding 7). FUSE mount is off the table on Termux. |
| rsync 3.5.1 | github.com/RsyncProject/rsync | GPL-3.0 (COPYING verified) | C | 3.5.1 on 2026-09-21 (NEWS.md) | daemon listener resident; per-file checksum CPU | **reject** | Needs rsync at both ends, rolling checksums over compressed payloads, no per-chunk catalogue verification. |
| zsync | zsync.moria.org.uk | Artistic-2.0 (unverified; raw GitHub 404) | C | unverified | none | reject | Same compression problem; no Kiwix `.zsync` found. |
| fastcdc 5.0.0 | github.com/nlfiedler/fastcdc-rs | MIT (LICENSE verified) | Rust, optional tokio/futures | 2026-08-22, 2.6 M downloads | library | reference | Only needed if we ever build a chunk store; we do not. |
| zim-tools `zimdiff`/`zimpatch` | github.com/openzim/zim-tools | GPL-3.0-or-later (`meson.build` `license: 'GPLv3+'`; zimdiff.cpp header) | C++ | `zimdiff.cpp` dated 2013 (Kiran Mathew Koshy), still in `main`, with a `[FIXME] Handle redirection` at the diff loop | none while off; operator tool | reference | Item-level diff (changed/new items + a `dlist`), not byte-level; not a node job. |
| BitTorrent web seeds (BEP 19) | bittorrent.org | open spec | n/a | read via search snippet only | n/a | reference | A hub's HTTP route can be a web seed for a torrent whose pieces are hash-checked (search snippet); keeps one file set serving both. |
| kubo (IPFS) v0.43.1 | github.com/ipfs/kubo | MIT / Apache-2.0 dual (LICENSE verified; MIT-only for pre-2019-05-06 code) | Go | tag 2026-09-14 (proxy.golang.org) | ~100 MB at start and creeping, needing a hard memory limit and periodic restarts (third-party docs, search snippet); DHT reprovide timer (`Provide.DHT.Interval`, `docs/config.md`) | **reject** | Resident DHT provider + global-network assumption; breaks the Pi floor and offline-first. |
| Hypercore / Hyperswarm | github.com/holepunchto/hypercore | MIT (LICENSE verified) | Node.js, v11.39.6 | `package.json` | Node runtime + DHT | **reject** | A JS runtime and a global DHT to move a file. |
| Sneakernet bundle (`111 update --from <dir>` for datasets) | our glue | GPL-3.0-or-later | Rust | n/a | 0 | **adopt (v1)** | Same signed-origin layout as L7; the only path for a node with no IP link to a hub. |
| Catalogue head over sealed frames, bytes over IP | our glue on L5 `SYNC`/`KB` class | GPL-3.0-or-later | Rust | n/a | 0 | **adopt (v1 head: LAN only, v1.1 LoRa)** | Mirrors L7 fact 14 ("a release exists"); needs the L5 dispatch fix (finding 11). |

## 3. Findings that change the plan

1. **Nothing in the tree can fetch a multi-GB file.** `Fetcher::get` returns `Vec<u8>` and sets `timeout_global` over the whole call
   (`1-bootstrap/crates/node111/src/util/http.rs:26-27,36`); `check_once` builds it with a 512 MiB cap (`N/updater/fetch.rs:100`).
   There is no Range request and no file sink. Layer 8 needs one new `get_to_file(url, range, ...)` with per-read (idle) timeouts, a
   `.part` file under a node111-owned staging path, and chunk verification. It must not reuse `Fetcher::new` as is.
2. **The serving side has no Range either** (`N/panels/static.rs:39,227`, no `206` anywhere; finding 9 of `00-code-seams.md`). A single-range
   arm is ~60 lines and no resident weight (L7 options fact 18 estimates the same for the PMTiles route). One handler then serves PMTiles
   tiles, resumable dataset pulls and the `blob/<sha256>` origin.
3. **Content-addressed URLs make resume trivial and `If-Range` unnecessary.** A hub serves `blob/<sha256>`; bytes under that name never change,
   so a resumed `Range: bytes=N-` against any mirror is safe, and verification (not the server) decides. Two hubs can supply different
   ranges of one file with no extra protocol.
4. **Verification model: whole-file sha256 at the end is not enough at this size.** A hash-at-end failure on 57 GB costs 57 GB again
   (12.7 h at 10 Mbit/s, 63 h at 2 Mbit/s, arithmetic from 57e9 B / rate). Pin per-chunk hashes in a sidecar the signed catalogue entry hashes.
   Numbers (16 MiB chunks, 32 B per chunk): 13 GB mini = 775 chunks / 24.8 KB; 57 GB nopic = 3,397 / 108.7 KB; 111 GB maxi = 6,616 / 211.7 KB;
   Berlin 84 MB = 5 / 0.2 KB. The BLAKE3/bao outboard at 16 KiB groups is 0.39 % (`mesh111/src/blobs.rs:1-10`): 222.7 MB for 57 GB, 433.6 MB for 111 GB,
   too big for a catalogue but fine as a lazily built sidecar (v1.1). Sizes are lens 1's (all secondary).
5. **Two hash spaces, one catalogue.** Releases and L7's origin are sha256 (`NI/manifest.rs:154-159`); fabric blobs are BLAKE3 (`M/blobs.rs`).
   The catalogue entry pins sha256 (upstream `.sha256`, HTTP origin, sneakernet) and optionally the BLAKE3 root (Tier 2). Computing the BLAKE3 root is
   a second full read: ~12 min for 57 GB at an assumed 80 MB/s (unmeasured; Pi 4 has no SHA extension, Pi 5 does, both unverified). Compute it
   once at ingest on hubs only, never on a phone.
6. **bao-tree 0.17.0 is on crates.io (2026-10-22) and the lock pins 0.16.1.** `mesh111/Cargo.toml:20` is `"0.16"`, which cannot take 0.17.
   0.17's changelog (raw `CHANGELOG.md`, verified) lists fixes in what Tier 2 will use: `encode_ranges` wrote partial chunk groups without the inner
   parent pairs the decoder expects; decoding now stops after the first error; empty-range handling; overflow near `u64::MAX`. Breaking: `BlockSize`
   capped at 64 MiB, MSRV 1.85, wire changes only for `experimental-mixed` (unused). iroh-blobs 0.104 already depends on `bao-tree 0.17`, so staying on 0.16 forks
   us from the ecosystem. The bump needs a `LICENSING.md` row edit (`:143`) and a regenerated `THIRD-PARTY-NOTICES.md`.
7. **Delta transfer: the formats do not delta well, or there is no tool.** ZIM:
   - Kiwix has no incremental ZIM update path (search snippet quoted in lens 1 finding 6); `zimdiff` (zim-tools, GPL-3.0+) compares
     items by path and bytes, writes changed/new items plus a `dlist` of removed paths and the start/end UUIDs into a ZIM (`src/zimdiff.cpp`, verified) and
     skips redirects (`[FIXME]`). `zimpatch` exists in the same repo. It is an operator-side authoring tool, not a node feature.
   - ZIM stores compressed clusters (format fact from background knowledge, unverified; no KB entry covers it), so a byte-level delta sees changed ciphertext-like bytes for every
     cluster that shifted (inference).
   - PMTiles: tiles are individually compressed and the data section follows Hilbert TileID order for clustered archives, with deduplicated tiles
     pointing backwards (`spec/v3/spec.md` lines 150-155, verified). A local change moves a local region plus directory offsets, so a byte-level delta
     could work; **no source measures this** (search found none). The sane unit is the regional file (Berlin 84 MB), refetched whole.
   - Therefore: full-file refresh, stage + verify + rename, space for old and new at once or removal first (lens 1 finding 7).
8. **BitTorrent is already how Kiwix distributes.** The Kiwix blog recommends the torrent for large files (search snippet) and a `.zim.torrent`
   sits next to each ZIM on download.kiwix.org (gwern.net listing, search snippet). A hub operator who already has the torrent can seed from the
   same file a hub serves over HTTP (web seed, BEP 19). rqbit (Apache-2.0, 2026-10-29) supports BEP-27 private torrents and BEP-14 LSD (README, verified), so a
   LAN-only swarm with no DHT is configurable in principle; whether DHT can be switched off by a flag is **unverified**. For 2-3 hubs on one LAN the swarm adds nothing over Range.
9. **The fabric is the wrong carrier for the bytes in v1, the right one in v1.1.** Today it is `tcp-lan` only, chunk frame `0x05` is reserved and discarded
   (`M/fabric.rs:2388`, `M/link/tcp_lan.rs:24-25,225-233`); `OBJ_*` kinds are registered and `built_in_v1() = false` (`M/envelope.rs:515-522,601-607`). L7 F3 wraps `tcp-lan`
   between hubs in TLS 1.3 with raw public keys, so a hub-to-hub internet link exists in L7 v1 and would carry `OBJ_CHUNK` frames with no extra listener.
   But `Link::send` is awaited inside the fabric actor (L7 options fact 1; `remove_link` and a non-blocking send are on L7's v1.1 list), so a 57 GB
   transfer on the chat connection would stall chat. Bulk needs a dedicated TCP connection per transfer session, or the non-blocking send first.
10. **NAT.** Hubs behind NAT cannot reach each other over plain HTTP: only the serving hub must be reachable, and only if it has a public address,
    a forwarded port or an L7 tunnel (WireGuard/Yggdrasil are v1.1 candidates in L7 G1). iroh would add QUIC hole punching but its defaults
    use n0's public relays (README `presets::N0`), a third-party service. v1 states: LAN and
    reachable-hub only. L7's carriers (ntfy, mailbox) carry sealed frames and never datasets; that stays true.
11. **The catalogue head can travel as a sealed frame; the catalogue and bytes cannot.** A head is ~80-100 B (catalogue id, `seq`, root hash): one frame, even on
    LoRa (a 138 B announce is the L7 unit). A 200-entry catalogue (~100 KB) is fine on `tcp-lan`, hours on LoRa. The carrier class is `KB`/`SYNC`, but an inbound `SYNC`
    lands as a textless chat row, bypasses admission and escapes quota eviction (`00-code-seams.md` findings 17-20): fix the dispatch arm before any head is sent.
12. **LoRa arithmetic, scaled from L7 fact 14.** 5.7 MB = 15 h of LongFast airtime unsealed. Scaling linearly: Berlin 84 MB ~ 9 days of airtime (~2.5 years at 1 % duty);
    13 GB mini ~ 3.9 years of airtime. LoRa carries the head at most.
13. **Pi floor and SD wear.** v1 HTTP path: zero resident weight. A fetch is one sequential write (57 GB ~ 47 min at 20 MB/s) plus one hash pass (~12 min at 80 MB/s, assumed), then a
    same-filesystem rename (no second copy). A 4A hub is >= 512 GB free (`NI/install/categorize.rs:102-104`): put datasets on the SSD, never the boot SD. Cap concurrent
    bulk streams (kiwix-serve has `--ipConnectionLimit`; ours needs an equivalent, e.g. 4).
14. **Sneakernet specifics.** Reuse the origin layout; `111 update --from` becomes `111 dataset import --from <dir>` (or one verb with a kind). Two modes: *copy* (write
    to `datasets/`, verify, rename) and *register in place* (hash once, record the path, no write; the operator keeps the disk attached). In-place avoids 57 GB of SD writes.
    Android/Termux reading a USB OTG disk without root is **unverified**; assume sneakernet on Termux means an app-private copy from shared storage.
15. **Metered links.** L7 v1 has a "treat this network as metered" setting with a daily byte cap (D30 F10). It must cover datasets: without it, one dataset consumes 57 days of a
    1 GB/day budget. The cap needs a notion of "day"; rule 4 forbids wall clock in crypto/ordering, a quota is neither, but the source of "day" (local clock vs. monotonic uptime) is L7's
    to state. Every dataset fetch asks consent with size and remaining cap; a resumed fetch continues at a chunk boundary the next day.
16. **Who signs is not this lens's call, but the transfer model constrains it.** The signature must travel with the catalogue entry so an untrusted mirror can pass it on (F-Droid
    `entry.jar` shape, lens 1 finding 10). A mirror hub serves bytes and the original signed entry; it never re-signs.

## 4. Recommendation (forks, options, picks)

**Fork A - the v1 bulk path**
- A1 HTTP Range + sha256 + chunk sidecar + sneakernet (picked). Trade-off: no multi-source swarm, no delta, serving hub must be reachable.
- A2 A1 plus a hosted rqbit for hub-to-hub. Trade-off: a resident process while on, SHA-1 piece layer, second listener; benefit is multi-hub swarming and Kiwix's existing torrents.
- A3 iroh-blobs as a hub component. Trade-off: +13.3 MB, QUIC + relay defaults, "not production quality", second identity.
- A4 Build Tier 2 first. Trade-off: biggest build, blocks Layer 8 on L5 amendments and the bao-tree bump; the bytes ride only `tcp-lan` anyway.

**Pick A1.** v1: LAN hub-to-node and hub-to-hub pulls, sneakernet import, both on the L7 origin layout; no new dependency; `get_to_file` + a Range arm.

**Fork B - who owns Tier 2 transfer (`OBJ_*`)**
- B1 Layer 8 owns it, implemented in `mesh111` as an additive L5 amendment, v1.1 (picked). Reason: Layer 8 supplies the requirements (admission via `admit(class, bytes)`, `superseded` marking, resume, range-sets, a consumer that actually has multi-GB objects) and the blob store (`M/store/blobs.rs`) is storage.
- B2 Layer 7 owns it (current wording: "Tier 2 transfer" on L7's v1.1 list). Reason for: L7 owns reach and carriers. Reason against: L7's v1.1 bulk consumer would be UPDATE payloads, already covered by the HTTP origin.
- B3 Nobody until a second consumer appears. Trade-off: no peer-authenticated bulk; HTTP stays public and unauthenticated on the LAN.

**Pick B1**, with L7 contributing the non-blocking `Link::send` / per-session connection as a prerequisite, and the Layer 5 spec amendment list written by Layer 8.

**Fork C - delta**
- C1 None; full-file refresh (picked). C2 `zimdiff` bundles authored by the operator off-node and shipped as ordinary catalogue items (v1.1 maybe). C3 CDC chunk store (desync/casync/FastCDC). Trade-off C3: a second storage format and compressed inputs defeat dedup (inference).

**Pick C1**; revisit C2 only with a measured Wikipedia-to-Wikipedia size.

**Fork D - hub-to-hub over the internet (v1.1)**
- D1 Tier 2 over L7's TLS-RPK hub link, bulk on its own connection (picked). D2 HTTP over an L7 tunnel (WireGuard/Yggdrasil). D3 rqbit or iroh with hole punching. Trade-off D3: more listeners, third-party relays.

**Pick D1**; D2 is free the day L7 ships a tunnel, because A1 already works over any IP path.

**What is v1 / v1.1 / out**
- v1: catalogue + chunk sidecar, `get_to_file`, Range route, consent + metered cap check, sneakernet verb, head frame on LAN.
- v1.1: bao-tree 0.17, `OBJ_ADVERT/REQ/CHUNK`, chunk frame `0x05`, BLAKE3 root + outboard sidecar, hub-to-hub bulk connection, LoRa head.
- Out: delta, swarming, IPFS, Syncthing, Hypercore, rsync.

## 5. Open questions for the owner

1. May Layer 8 own Tier 2 (fork B1), which means Layer 8 writes an additive Layer 5 amendment list and Layer 7 drops "Tier 2 transfer" from its v1.1 list?
2. Is the dataset route unauthenticated on the LAN (L7 origin posture) or pairing-gated? Unauthenticated means anyone on the LAN can pull 57 GB repeatedly; a connection cap bounds it.
3. Does the daily metered cap cover dataset fetches (picked: yes, with size consent)? What is the cap's "day" (L7's call)?
4. Is the bao-tree 0.16 to 0.17 bump acceptable now (lock, `LICENSING.md:143`, notices, MSRV 1.85 check), or only when Tier 2 starts?
5. Should the owner want a swarm (hub A and hub B and node C pulling one dataset from each other) in v1.1, accepting a hosted rqbit (resident while on), or stay with D1?
6. FAT32 sticks: do we split ZIMs with `zimsplit` (zim-tools) for sneakernet, or document exFAT/ext4 only?
7. May a hub register a dataset in place on an attached disk (no copy), keeping hash verification at registration only?

## 6. Sources

- Repo files: `plan/layers/8-storage/research/00-code-seams.md` (findings 9, 13-16, 17-20), `01-datasets-hub.md` (findings 6, 7, 10, 11, 13), `5-messaging/README.md` s.5, s.11, s.19, s.21;
  `plan/layers/7-relay/options.md` facts 1, 12-15, 18; `plan/DECISIONS.md` D30 F3, F6, F10; `0-knowledge/projects/blake3-bao.md`, `0-knowledge/concepts/lora-airtime-budget.md`;
  `1-bootstrap/Cargo.lock` (`bao-tree 0.16.1`, `blake3 1.8.7`, `iroh-io`), `5-messaging/crates/mesh111/Cargo.toml:19-20`, `LICENSING.md:142-143`,
  `1-bootstrap/crates/node111/src/util/http.rs:18-47`, `.../daemon/updater/fetch.rs:100,161`.
- crates.io API (2026-11-02): crates `bao-tree`, `iroh-blobs`, `iroh`, `blake3`, `fastcdc`, `rqbit`, `librqbit`, plus the dependency lists of `librqbit` 9.1.0 and `iroh-blobs` 0.104.0.
- raw.githubusercontent.com: `n0-computer/iroh-blobs/main/{Cargo.toml,README.md,LICENSE-MIT,LICENSE-APACHE}`, `n0-computer/bao-tree/main/{Cargo.toml,CHANGELOG.md}`, `ikatson/rqbit/main/{LICENSE,README.md}`,
  `nlfiedler/fastcdc-rs/master/{LICENSE,README.md}`, `syncthing/syncthing/main/{LICENSE,lib/config/foldertype.go}`, `syncthing/docs/main/users/syncing.rst`,
  `folbricht/desync/master/{LICENSE,README.md}`, `systemd/casync/main/{LICENSE.LGPL2.1,README.md}`, `RsyncProject/rsync/master/{COPYING,NEWS.md,version.h}`, `arvidn/libtorrent/RC_2_0/{LICENSE,include/libtorrent/version.hpp}`,
  `ipfs/kubo/master/{LICENSE,docs/config.md}`, `holepunchto/hypercore/main/{LICENSE,package.json}`, `kiwix/libkiwix/main/src/server/response.cpp`, `openzim/zim-tools/main/{meson.build,src/zimdiff.cpp,src/zimpatch.cpp}`,
  `protomaps/PMTiles/main/spec/v3/spec.md`.
- proxy.golang.org: `github.com/syncthing/syncthing/@latest` and `/@v/list`, `github.com/ipfs/kubo/@latest`, `github.com/folbricht/desync/@latest`.
- Search snippets (not fetched): Kiwix blog on torrents and hashes and the gwern.net listing; `manpages.ubuntu.com/manpages/plucky/man1/zimdiff.1.html` (fetch returned 503); Syncthing forum threads
  (`forum.syncthing.net/t/are-650k-files-to-much-for-syncthing-on-raspberry/21132`, `.../high-memory-usage/15586`); `github.com/syncthing/syncthing/releases/tag/v2.0.14`; kubo-on-SBC deployment docs mirrored at `git.sim1222.com/sim1222-mirror/mitra/.../docs/ipfs.md`;
  `bittorrent.org/beps/bep_0052.html`; `docs.rs/iroh-blobs/latest/iroh_blobs/store/constant.IROH_BLOCK_SIZE.html` (16 KiB).
