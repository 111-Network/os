# Layer 8 research 06: completeness critique

Critic: opus, 2026-11-02. Inputs: `00`–`05` read in full, with tree spot checks (cited as file:line) and five web reads (§8).
`7-relay/README.md` is being edited live, so Layer 7 is cited by section.

## 1. Verdict

The sweep is enough for the brief on maps, encyclopedias, where datasets live, the shared log (none in v1), SD wear and the Storage page.
It is **not** enough on agent-state encryption: both scouts' mechanisms break a locked rule (G1). The nine conflicts below are settled in the brief.
The README §8 clause "software updates" beyond the 111 release is uncovered (G2). Run G1 and G2 as the two gap-fill scouts.

## 2. Coverage

| §8 clause / hand-off | Covered by | State |
|---|---|---|
| encrypted store · capacity pruning | 00, 04, 05 F1 | agent state plaintext (G1); "datasets never auto-pruned" is the owner's reading to confirm |
| encyclopedias, maps | 01, 05 | covered; kiwix-serve RSS unmeasured |
| software updates | 01 §3.8 (111 release only) | **G2** |
| patches | 01 §3.9 | thin; Layer 9 plans a v1.1 hub "fix shelf" (L9 `03` §4.3) |
| shared log | 03 | covered |
| L3, L4, L5, L7 hand-offs | 00–05 | conflicts 1–5 |
| Pi floor, licences | 05 §3.17, 01 §3.12 | numbers missing (§5); Wikipedia/OSM only (G5) |

## 3. Conflicts

1. **Tier 2 owner.**
   - 01 E: stays with Layer 7.
   - 02 B1: Layer 8, because "Layer 7 has none" as a consumer. That premise is false: L7 §11.1's BROADCAST `POINTER` is "fetched by Tier 2".
   - L7 §17 defers Tier 2 with no owner.
2. **Agent-state encryption.**
   - 00 §6 A: its Q3 puts SQLCipher in `agent111`, which means the agent holds the key. That breaks "the agent holds **zero** key material" (`2-identity/README.md:224`).
   - 04 1 B: the agent writes over its bearer. That breaks D26 (2), "refused on every non-GET" (`3-intelligence/README.md:119-122`).
   - Unseen by both: 111d already relays `/api/v1/chat` (`node111/src/daemon/dashboard/api_v1/relay.rs:626-736`).
3. **Map bucket.** L7 §10.5 and 01 say ≈69 KB. 05 measured 83.9 KB with CSS. Take about 90 KB and correct L7.
4. **SYNC in v1.**
   - 02 wants a v1 catalogue-head frame.
   - 03 wants no SYNC consumer in v1.
   - 00 Q7 wants the textless-row trap fixed now.
   - L7 §9.3 (an HTTP check at link attribution) and §11.1 (a signed `POINTER`, v1.1) make the v1 head frame unneeded. The row fix stays a separate bug question.
5. **The metered cap covers carrier bytes only** (L7 §12). 02 F15 and 05 F4 assume it covers datasets, so an L7 amendment is needed now, in L7's fix wave. The window is already defined: a rolling 24 h on `CLOCK_BOOTTIME`.
6. **Dataset tree.** One sha256 tree with `state/origin/` (00), `datasets/<id>/` (01), or the origin layout (02). This is the lead's call.
7. **Sizes and RSS.** Mini ZIM: 13 GB (01, 02, Mar 2024) against 7.4 GB (05, Nov 2024). kiwix-serve RSS: 128–256 MB (01) against "under 50 MB" (05).
8. **Export format.** L2 §8.5(i) against L5 §21 and the KB `age` entry (04 F14).
9. **Layer 9's sweep.**
   - Incidents: L9 writes them as plaintext `state/incidents.jsonl` (L9 `01:18,92`). 04 Fork 5 has an encrypted `heal/journal`.
   - Audit trail: L9 plans an own node-key hash chain (`05:161`). 03 says that would "reinvent p2panda-core".
   - Node-key domain tags appear in 01 Q3, 03 F8, L9 `03:114` (`111/fix/v1`) and L7 §11.1 (`111/broadcast/v1`). One L2 rule should cover them all.

## 4. Gaps

| # | Gap | Rating |
|---|---|---|
| G1 | No permitted mechanism for encrypted agent state (conflict 2) | blocking |
| G2 | Component payloads are "never shipped as bytes in a release": offline, only a USB stick brings them (`1-bootstrap/README.md:228-246`). Also uncovered: Termux debs, APKs, mirroring duties (GPL, MPL-2.0, Reticulum License). Missed candidates: fdroidserver repo, apt-cacher-ng/aptly | major |
| G3 | 02's custom sha256 chunk sidecar has adoptable equivalents (below the table) | major |
| G4 | Missed Pi prior art: Internet-in-a-Box (LICENSE is the GPL-2.0 text; three PMTiles files served with Range on a Pi Zero 2 W, 256 GB card; SCALE 2026 slides, snippet) and Kiwix Hotspot/offspot (Pi 3B+–500, 1 GB+ RAM, Docker; `offspot/overview` README). Their configs are the best published evidence for conflict 7 | major |
| G5 | Non-commercial content in the Kiwix library; per-image licences | minor |
| G6 | Signer trust anchor: L7 §11.1 "a publisher the user follows" plus the home-hub NodeId answers it | minor |
| G7 | ◐ on Cat 2A/3 undefined | minor |
| G8 | No KB candidate entries for kiwix-serve, PMTiles, protomaps-leaflet, p2panda-core | minor |
| G9 | No threat-model lens (ZIM JavaScript, libmicrohttpd on the LAN, DoS on a 57 GB route, bbox privacy) | minor |

G3's equivalents:
- the in-lock bao-tree at a 16 MiB BlockSize gives a ≈217 KB outboard for 57 GB (3,396 × 64 B), under the BLAKE3 root 01 already pins;
- Metalink 4 `<pieces>` (RFC 5854);
- aria2 (COPYING is the GPL-2.0 text; its README lists "chunk checksum validation in Metalink").

The lead settles G3 from bao-tree's source and G4 with a Pi measurement.

## 5. Custom code, and resident costs without a number

- Custom code where an adoptable project exists: the chunk sidecar and fetcher (G3), and Layer 9's audit chain. The TOML+minisign catalogue is justified (it is the house parser; TUF needs wall-clock expiry).
- Resident costs with no number yet:
  - the second SQLCipher connection (04 F17);
  - kiwix-serve while on, and its idle-stop detector (01 A3);
  - the startup `quick_check` full read of the store on SD (04 F10);
  - the persisted metered counter;
  - the p2panda-core and `age` binary deltas.

## 6. Owner forks

1. Are datasets never auto-pruned, or pruned under disk pressure?
2. Agent state: encrypted (the G1 mechanism) or plaintext with atomic writes and caps?
3. Shared log: none in v1 (p2panda-core named), a design-only contract, or one at the first consumer?
4. Who owns Tier 2: Layer 8, Layer 7, or nobody?
5. Dataset signer: the hub-owner key (node key under a domain tag, or minisign), the release key, or checksums? Make it one rule with L7 §11.1 and Layer 9.
6. Kiwix: P7 first, an unauthenticated LAN link-out, or search only?
7. Is the dataset route unauthenticated on the LAN or limited to paired sessions?
8. First copy: an upstream fetch on a user action, or sneakernet only?
9. On a metered link: pause and resume, or refuse; is there an override?
10. Export: the L2 §8.5(i) format or `age`; export-before-prune in v1?
11. A disk-relative node budget: who sets it on phones?
12. Capacity facts written on change only, plus `journal_size_limit`?
13. Stop control kinds from landing as textless chat rows now?
14. Map: vector (a 90 KB bucket) or raster?
15. May hubs mirror third-party binaries (G2)?
16. Only unmodified Protomaps output in v1? Which datasets ship first?
17. A pre-migration snapshot before V4 and V5?
18. Syncthing, rqbit, iroh: never, or opt-in components?

## 7. Unverified claims to verify on the owner's machine

- **Sizes:**
  - EN Wikipedia maxi, nopic and mini; Simple English; Wiktionary; WikiMed;
  - whether `.sha256`, `.torrent` and `.meta4` (with pieces) sit beside each ZIM;
  - PMTiles planet and region sizes; Protomaps BLAKE3 hashes; the Geofabrik date.
- **Binaries:**
  - kiwix-tools 3.8.2 static aarch64/x86_64 tarballs, and whether they are musl;
  - release dates for kiwix-tools, libzim and libkiwix; the Syncthing tag; libtorrent's date;
  - the casync, zsync, kiwix-js, Kiwix Android and rusqlite licences; "or later" for IIAB and aria2.
- **Measurements (Pi or phone):**
  - kiwix-serve RSS, timers and writes; the Termux kiwix closure size;
  - Martin, rqbit (and whether DHT can be turned off) and Syncthing v2 RSS; iroh's idle cost;
  - the second SQLCipher connection; 111d idle `write_bytes` (5.9 MB/day is derived) and SD amplification;
  - BLAKE3 and sha256 throughput on Pi 4 and 5; the p2panda-core, Loro, Automerge and `age` deltas; `quick_check` time.
- **Behaviour:**
  - SQLCipher `VACUUM INTO`, `sqlcipher_export` and the backup API; WAL `NORMAL` durability; `age` from a raw scalar;
  - on Termux: FBE, `statvfs`, file modes, USB OTG, ZIM `mmap`;
  - protomaps-leaflet offline fonts; raster vs vector sizes; journald `Storage=`;
  - how a textless SYNC row renders in the UI (a code read);
  - Willow timestamps; Loro shallow snapshots; the Bamboo and kuska-ssb licences; Martin's cache; Syncthing's ports.
- **Legal:**
  - whether a repacked ZIM is Adapted Material;
  - whether a re-extracted vector PMTiles is a Produced Work or a Derivative Database;
  - Protomaps' ODbL statement; per-image licences.

## 8. Sources

Repo:
- `3-intelligence/README.md:50,119-122`
- `2-identity/README.md:224`
- `1-bootstrap/README.md:228-246`
- `relay.rs:626-736`
- D26 (2)
- L7 §9.3, §10.5, §11.1, §12, §17
- L9 `research/01:18,92`, `03:10,114,134`, `05:161,213`

Web:
- raw.githubusercontent.com: `iiab/iiab/master/LICENSE`, `offspot/overview/main/README.md`, `aria2/aria2/master/{COPYING,README.rst}`, `protomaps/protomaps-leaflet/main/README.md`
- Search snippets: socallinuxexpo.org (SCALE 2026 IIAB slides), datatracker.ietf.org/doc/html/rfc5854.html, hub.kiwix.org/weblog/2026/03/question-time-q1-2026
