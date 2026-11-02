# Layer 8 research 01 - the knowledge hub's datasets (encyclopedias, maps, updates, patches)

Lens 1 of the Layer 8 sweep. Written 2026-11-02 on a box whose proxy denies most doc sites: download.kiwix.org, library.kiwix.org,
docs.protomaps.com, maps.protomaps.com, geofabrik.de, wiki.openstreetmap.org, en.wikipedia.org, opendatacommons.org and the GitHub
API/HTML were all unreachable (DNS or CONNECT failure). Every licence below was read from an upstream file on
raw.githubusercontent.com / PyPI / npm / crates.io unless marked. Every size is "(search snippet)" unless it comes from a file I read.
Dataset sizes are the weakest part of this report: **no current Kiwix or Protomaps listing was readable**, so the sizes are dated
secondary figures and the owner-facing numbers must be re-read from the live listings before the options brief quotes them.

## 1. Summary

- **Encyclopedias: adopt `kiwix-serve` (kiwix-tools, GPL-3.0-or-later) as an on-demand child program, not a library in 111d.** It is
  HTTP-only (no WebSocket/SSE, so it fits the panel proxy's rules), has `--address`, `--port`, `--urlRootLocation`, `--threads`
  and `--catalogOnly` (`kiwix-serve.rst` options), is packaged in Termux (3.8.2), and costs nothing while off. The blocker is on our
  side: `panels/manifest.rs:11-12` registers `kind = "static"` panels only; `child` is the reserved P7 generic reverse proxy.
- **Maps: a PMTiles file plus a `Range` route in 111d beats any tile server.** `panels/static.rs` has ETag/304 and streaming bodies but
  no `Range` handling (grep: no `Range`/`Accept-Ranges` anywhere in `node111/src`); one Range handler serves PMTiles tiles, resumable
  update-origin blobs and hub-to-hub dataset pulls. The PMTiles spec fixes a 127-byte header and a root directory inside the first
  16 KiB (`spec.md`), which is why a Range client works. Martin (Rust, MIT OR Apache-2.0, 1.16.1 on 2026-10-30) is the on-demand fallback,
  not the default.
- **Nobody upstream signs datasets, so the signing question is ours.** Kiwix publishes a `.sha256` per ZIM (search snippet) and its OPDS
  entry template has no hash, licence or signature field (`catalog_v2_entry.xml`); Protomaps daily builds publish BLAKE3 hashes (search
  snippet); the PMTiles format has no integrity field. Pick: a **hub-owner-signed catalogue** (F-Droid `entry.jar` shape: a small signed
  entry that pins big files by hash and size, mirrors untrusted), not the release key, not bare checksums.
- **Sizes decide who hosts what:** English Wikipedia maxi is ~111 GB (2025-08 build, search snippet), nopic ~57 GB (Nov 2024), intro-only
  mini ~13 GB (Mar 2024); a planet basemap is ~120 GB, Berlin ~84 MB (2025-05 build). A 32 GB SD card hosts mini plus regional maps, not
  nopic; the hub tier needs an SSD. A Termux phone hosts only small ZIMs.
- **Datasets must not live in `fabric/blobs`.** The store's capacity pruning counts "present blob files" as live bytes
  (`mesh111/src/store/prune.rs:3-5`); one 57 GB ZIM there would push message history into the prune order. Give datasets their own
  directory and capacity accounting.
- **Licence facts a hub operator takes on:** serving ZIMs and OSM-derived tiles is "Publicly" conveying under CC BY-SA 4.0 s.3 and ODbL
  s.4.2/4.3 (the notice `Contains information from OpenStreetMap, which is made available here under the Open Database License (ODbL).`
  is the licence's own example). Details in section 3, finding 12.

## 2. Candidates table

| Name | Upstream | Licence (verified?) | Language / runtime | Maturity | Resident weight | Fit | Why |
|---|---|---|---|---|---|---|---|
| kiwix-tools (kiwix-serve, kiwix-manage, kiwix-search) | github.com/kiwix/kiwix-tools | GPL-3.0-or-later (COPYING = GPLv3, README "or later"; verified) | C++17, meson; `static-linkage` option (`meson.build`) | 3.8.2 on main; Alpine 3.8.2-r0 built 2026-03-24, Arch 3.8.2-2 built 2026-04-30 (search snippets); Termux 3.8.2 (`termux-packages/packages/kiwix-tools/build.sh`) | No measurement. Anecdotes (search snippet): 128-256 MB idle, 256-512 MB under load (selfhosting.sh); full dump on a 512 MB Pi Zero 2 W (Lemmy). One TCP listener, 4 threads default. Zero while off. | **adopt** (on-demand component) | The only complete answer: serve + full-text search + OPDS catalogue; separate GPL process, so no linking question (D22). |
| libzim | github.com/openzim/libzim | GPL-2.0-or-later (COPYING v2; README "or later"; source header "version 2 ... or (at your option) any later version"; verified) | C++17; needs ICU, zstd, lzma, Xapian | 9.8.2 (`ChangeLog`, `meson.build`) | n/a (library) | reference | Do not link into 111d: C++ + ICU + Xapian is build-time weight (rule 3). "Or later" would permit GPLv3 linkage if ever wanted. |
| libkiwix | github.com/kiwix/libkiwix | GPL-3.0-or-later (verified) | C++17, libmicrohttpd, libcurl, pugixml | ChangeLog top 14.2.1; main already requires libkiwix >=15 | n/a | reference | Dependency of kiwix-serve; its server code is the Range/OPDS implementation. |
| zim-tools (zimcheck, zimdump, zimsplit...) | github.com/openzim/zim-tools | GPL-3.0-or-later (README; LICENSE is GPL text; verified) | C++ | n/a | zero off | reference | Hub-operator tools: validate a ZIM before the catalogue signs it. |
| python-libzim / zimscraperlib / warc2zim | PyPI libzim 4.0.0 (2026-10-29), zimscraperlib 5.5.1 (2026-10-30), warc2zim 2.3.1 (2026-07-31) | all GPL-3.0-or-later (PyPI metadata; verified) | Python 3.11-3.15; libzim aarch64 wheel 31.5 MB | very active | zero off | reference | Custom bundles (local KB, a community's documents) are authored with these, off-node. |
| zimit | github.com/openzim/zimit | GPL-3.0 (LICENSE file GPL; verified) | Docker + Browsertrix crawler (Chromium) + warc2zim | active | heavy, authoring only | reject for nodes | "intended to be run in Docker" (README); a crawl host, never a Pi or phone. |
| kiwix-js / kiwix-js-pwa | github.com/kiwix/kiwix-js, kiwix-js-pwa | GPL-3.0 (kiwix-js-pwa LICENSE verified; kiwix-js via README badge only) | browser JS | Android app 3.14.1, 2026-05-14 (search snippet) | zero server; browser-side | reject for LAN, reference for loopback | Needs Service Workers for dynamic ZIMs and reads a ZIM the user selects as a local file (README); `http://111.local` is not a secure context (`0-knowledge/concepts/secure-contexts.md`). |
| Xowa | github.com/gnosygnu/xowa | GPL-3.0 or Apache-2.0 (LICENSE.txt; verified) | Java | recency unverified | JVM | reject | Own database import, JVM, no gain over ZIM. |
| PMTiles format + go-pmtiles CLI | github.com/protomaps/PMTiles, go-pmtiles | code BSD-3-Clause; spec public domain / CC0 (LICENSE; verified) | Go single binary | spec v3 | zero off (operator tool) | **adopt** (format and CLI) | One file, HTTP Range, `extract --bbox` pulls a region from a daily build without the planet. |
| pmtiles (JS) 4.5.0 / protomaps-leaflet 5.1.0 / Leaflet 1.9.4 | npm | BSD-3 / BSD-3 / BSD-2 (npm metadata; protomaps-leaflet and PMTiles LICENSE verified raw) | browser JS, ~69 KB gz together (Layer 7 `05-ui-consent-floor.md:7`) | 2026-08-10 / 2025-06-18 / 2023-05-18 | browser only | adopt (UI, Layer 7/4) | The exempt "map" bucket Layer 7 recorded. |
| pmtiles (Rust crate) 0.26.0 | github.com/stadiamaps/pmtiles-rs | MIT OR Apache-2.0 (LICENSE-MIT/-APACHE present; verified) | Rust, mmap + tokio default | 2026-10-29, fuzzed against untrusted archives (README) | none unless linked | reference | Only needed to read a header for catalogue autofill; a 127-byte parse needs no crate. |
| Martin | github.com/maplibre/martin | MIT OR Apache-2.0 (verified) | Rust | 1.16.1, 2026-10-30; crate 1.0 MB | no measurement | reference (fallback) | PMTiles/MBTiles z/x/y server; duplicates a Range route. |
| tileserver-gl | github.com/maptiler/tileserver-gl | BSD-2-Clause (LICENSE.md; verified) | Node + native renderer | unknown | heavy (unmeasured) | reject | Raster rendering we do not need. |
| protomaps/basemaps + planetiler | github.com/protomaps/basemaps, onthegomap/planetiler | BSD-3 + CC0 style + MIT icons; planetiler Apache-2.0 (verified) | Java 21 | planet in 2-3 h on a "modest computer" (README); 512 GB scratch (search snippet) | off-node | reference | Build custom region files on a big machine; not a node job. |
| Organic Maps / CoMaps | github.com/organicmaps, comaps | Apache-2.0 code (verified); data ODbL | C++ apps, `.mwm` | active | n/a | reference | Own format and own app; confirms per-region files; not hostable as a component. |
| Valhalla / BRouter / OSRM / GraphHopper | respective repos | MIT / MIT / BSD-2 / Apache-2.0 (verified) | C++ / Java / C++ / Java | active | resident routing graph | reject for v1 | Routing is not in README s.8; a later opt-in component. |
| Kiwix OPDS catalogue | `/catalog/v2` of kiwix-serve | open spec (OPDS 1.2) | XML | verified in `kiwix-serve.rst` | none | bridge (ingest) | The upstream listing a hub operator ingests; lacks hash, licence, signature. |

## 3. Findings that change the plan

1. **kiwix-serve fits the panel proxy's protocol limits but not its v1 registry.** Options verified from `kiwix-serve.rst`:
   `--address`, `--port`, `--urlRootLocation`, `--threads` (default 4), `--catalogOnly`, `--monitorLibrary`, `--nolibrarybutton`,
   `--ipConnectionLimit`; SIGHUP reloads the library without the watcher. No TLS (search snippet). Panel manifests reject websocket/SSE
   (`panels/manifest.rs:14-15`) and kiwix-serve needs neither. `kind = "child"` is "reserved ... not registered" (`manifest.rs:11-12`),
   so Layer 8's first prerequisite is Layer 6's P7. Panel ports are `11190..=11229` (`panels/mod.rs:56`), reconciled on a 2 s tick
   (`mod.rs:62`), which Layer 7 already plans to trim.
2. **Termux can host kiwix-serve.** `termux-packages` carries kiwix-tools 3.8.2, libzim 9.8.2 and libkiwix 14.2.1 (`build.sh` files),
   depending on libc++, ICU, Xapian, libmicrohttpd, libcurl, pugixml, zstd, lzma, docopt. The release manifest already pins a Termux `[[deb]]`
   closure (`node111/src/manifest.rs` layout comment), so the pattern exists; the closure's size and RSS are unmeasured.
3. **The official static builds are unverified.** A Salix README (search snippet) names `kiwix-tools_linux-aarch64-3.7.0.tar.gz` on
   download.kiwix.org; I could not confirm a 3.8.2 aarch64/musl tarball. The source builds statically with `-Dstatic-linkage=true`
   (`meson.build`). Whether the official tarball is musl is unknown; the 111 manifest would pin whichever bytes pass the smoke.
4. **Kiwix's catalogue has no trust fields.** The OPDS entry template (`libkiwix/static/templates/catalog_v2_entry.xml`) carries `id`
   (urn:uuid), `title`, `updated`, `summary`, `language`, `name`, `flavour`, `category`, `tags`, `articleCount`, `mediaCount`, `author`,
   `publisher`, `dc:issued` and an acquisition link with `length` (bytes). No hash, no licence, no signature. The `.sha256` lives next to the
   ZIM (Kiwix hub FAQ, search snippet). Kiwix itself deprecates XML/YAML library files in favour of OPDS (search snippet).
5. **A ZIM can carry code.** kiwix-js README ("Security"): "ZIM archives can run dynamic code in your browser ... Be sure to get your ZIM
   archives only from a secure source". So a dataset is executable-adjacent content in a panel origin; the signing decision is a security
   decision, not bookkeeping. The panel listener's per-port origin (storage is per origin, `secure-contexts.md`) is the containment.
6. **Sizes (all dated, all secondary).** English Wikipedia ZIM: 102 GB with pictures / 52 GB without / 13 GB intro-only (Mar 2024) and
   110 / 57 (Nov 2024) per the Wikipedia "Kiwix" article (search snippet); newest build 2025-08 at ~111 GB with photos (BunsenLabs forum,
   search snippet); 101.6 GiB for 2023-11 (search snippet). Flavours per Kiwix FAQ: maxi = full, nopic = full text, ~75% smaller, mini =
   intro + infobox, ~95% smaller (get.kiwix.org FAQ, search snippet). Kiwix says incremental ZIM updates do not exist, so every refresh is
   a full file (search snippet). WikiMed was 1.2 GB in 2017 (grant report, search snippet; stale). Simple English, Wiktionary and a current
   medicine ZIM: **no size found** - unverified. PMTiles: planet ~120 GB (docs.protomaps.com via snippet), Berlin 84 MB archive / 88 MB
   transferred at overfetch 0.05 from the 20250514 build, Berlin zoom 0-6 = 813 kB, Puglia maxzoom 14 ~200 MB (pyviz tutorial, search
   snippet); each added zoom roughly doubles size. Geofabrik Europe PBF 28.8 GB (undated snippet; England ~1.5 GB).
7. **A ZIM refresh is a re-download of the whole file**, so a "dataset update" on a 32 GB SD card needs space for old and new at once, or
   removal first. This is the capacity-pruning interaction Layer 8 must define; a half-written file must never replace a served one
   (stage, verify hash, rename).
8. **Update origin: what Layer 8 adds and must not duplicate.** Layer 7 (D30 F6) already serves `channel-111-stable.toml`, its `.minisig`
   and `blob/<sha256>` read-only on the existing dashboard listener while `hub_role` is on, every byte passing the compiled-in key, the
   floor and the pins. Layer 8 adds: (a) the **same hub store** serving dataset files (one listener, one Range handler); (b) a catalogue
   that can list "the release channel exists here" as a pointer with no new trust (the release key stays the only authority over code);
   (c) **hub-to-hub replication of the payload files** as ordinary dataset pulls verified by the signed manifest's pins. It must not
   re-sign releases, mirror the KB (it rides the payload, D30 K1), or add a second updater.
9. **Patches are small and L9's.** Layer 9's PATCH artefacts (recipes, KB deltas, proposals) are KB-scale files, orders of magnitude below a
   ZIM. Layer 8 provides storage, a catalogue class and a Range-served path; the signer of a patch is the proposing node plus the human
   merge gate (README s.9), and "proven" endorsement is a later additional signature on the catalogue entry, not a new format. The
   `PATCH` frame class already exists with consumers in Layer 9 (`5-messaging/README.md` s.19).
10. **Signing models compared** (read, not remembered): TUF (`tuf-spec.md`): root/targets/snapshot/timestamp/mirrors roles; targets can
    delegate; every metadata file has an expiry in wall-clock date-time (spec "Date-time"), which breaks rule 4 (no wall clock in
    ordering); we would need the mesh epoch Layer 7 F8 defines. F-Droid index-v2 (`fdroidserver/index.py:856-905, 1931-1954`): a tiny
    `entry.json` (signed as `entry.jar`) holds the sha256, size and name of `index-v2.json` plus diffs and `maxAge`; the repo is pinned by a
    fingerprint in the URL; mirrors are an unsigned list; the client rejects a hash mismatch. apt: `InRelease` signed, per-file SHA256 + size
    lines, `Valid-Until` (optional) and `Acquire-By-Hash` (Debian handbook and libapt docs, search snippets). Kiwix: checksum only.
    The F-Droid shape is the closest fit: one small signed object, big files pinned, any mirror acceptable, offline-verifiable.
11. **The release manifest is the house style.** `manifest.rs` is rustup-v2 TOML with `deny_unknown_fields`, minisign over the raw bytes,
    parse -> verify -> floor -> pins; sha256+size per artefact. A dataset catalogue in the same style costs no new parser. Files are
    addressed in Layer 5 by BLAKE3 with bao outboards (`mesh111/src/blobs.rs:1-14`; 0.39% overhead at the 16 KiB group), so the catalogue
    should pin **both** the upstream sha256 (ingest check against Kiwix's `.sha256`) and our BLAKE3 root (chunk-verified transfer).
12. **Licence obligations for a hub operator (facts from the licence texts, SPDX copies on raw GitHub).**
    - **CC BY-SA 4.0 s.3(a)**: when You "Share" you must retain creator identification, copyright notice, licence notice, disclaimer notice
      and a URI to the material "if supplied by the Licensor", indicate modification, and include the licence text or URI; "any reasonable
      manner based on the medium" suffices, a hyperlink is allowed. **s.3(b)**: ShareAlike applies when You share Adapted Material **you
      produce**: same or later CC BY-SA or a compatible licence, licence text/URI, and no extra terms or technological measures. Serving a
      Kiwix ZIM unchanged is Sharing; a repack, a subset or a re-index is arguably Adapted Material (legal reading unverified). **s.4**
      treats extraction of a substantial part of a database as Adapted Material for s.3(b). Images carry their own licences (search snippet);
      what a maxi ZIM preserves of per-image licence data is unverified. Mini/nopic drop images.
    - **ODbL 1.0 s.4.2**: if You "Publicly Convey" the Database (or a Derivative Database) you must include a copy or URI of the licence and
      keep copyright and licence notices; "Publicly" means to persons other than You or under Your control, so a hub serving a neighbourhood
      is public (s.1, line 98 of the SPDX text), a private LAN of one household is not. **s.4.3**: publicly using a **Produced Work** needs a
      notice that content came from the Database under ODbL; the example text is above. **s.4.4-4.6**: a Derivative Database that is publicly
      used must be ODbL (or compatible), and You must offer the Derivative Database (or the alterations) in machine-readable form. s.4.5(b):
      making a Produced Work does not create a Derivative Database. Protomaps classes its tilesets as Produced Works needing a visible
      "(c) OpenStreetMap" (search snippet via a mirror of `protomaps/basemaps`; `LICENSE_DATA.md` itself says the example is `© OpenStreetMap`
      and that the OSM-derived water/earth polygons are also ODbL). Whether a re-extracted vector PMTiles is a Produced Work or a Derivative
      Database is the legal question; **the safe operator posture is unmodified Protomaps output + the notice**. Natural Earth is public
      domain (`LICENSE_DATA.md`).
    - **Software**: kiwix-tools is GPL-3.0-or-later; if a hub **mirrors the binary** to other nodes it becomes a distributor owing the
      Corresponding Source (`LICENSING.md` s.8 offer); pinning upstream's URL does not.
13. **Catalogue must carry what the UI and agent need**: `id`, `kind`, `name`, `version`, `size`, `blake3`, `sha256`, `licence` (SPDX id),
    `attribution` (the exact notice text), `source_url`, `built` (upstream date as a string, not used for ordering), `seq` (monotonic),
    `requires` (component id), `fits` (device-category floor), `signers`. Ordering and supersession use `seq`, never a clock (rule 4).

## 4. Recommendation

**Fork A - how encyclopedias are served**
- A1 kiwix-serve as a hosted child panel behind the panel proxy (needs P7). Trade-off: a resident process while open; adopts the whole product.
- A2 Static export or kiwix-js in the dashboard. Trade-off: Service Worker needs a secure context, which LAN HTTP is not; client-side ZIM reading needs the file on the device.
- A3 kiwix-serve started by the supervisor on first panel open and stopped after idle (A1 plus a lifecycle). Trade-off: first-open latency; weight appears only while someone reads.
- A4 Link libzim into 111d. Trade-off: no second process; C++/ICU/Xapian build weight, violates rule 3.
- **Pick A3 on top of A1** (the on-demand rule applied literally); measure RSS on the Pi before the owner sees it.

**Fork B - maps**
- B1 PMTiles + a `Range` route in 111d (+ Leaflet/protomaps-leaflet in the exempt bucket). Trade-off: ~60 lines of Range code (Layer 7 estimate), no tile server.
- B2 Martin on demand. Trade-off: a second Rust process; serves z/x/y for third-party clients.
- B3 tileserver-gl. Trade-off: Node and a renderer; no benefit.
- **Pick B1**, with Martin documented as the fallback if a non-browser client needs z/x/y. The route must send `Accept-Ranges`, `Content-Range`, honour `If-Range` with the existing metadata ETag, and be shared with update-origin blobs and dataset pulls.

**Fork C - who signs a dataset**
- C1 Hub-owner node key signs the catalogue (hash pins inside). Trade-off: trust is in the operator; offline-verifiable; fits "no central authority".
- C2 The 111 release key signs a project-curated catalogue. Trade-off: one anchor, but it ties the code-signing root to routine dataset publishing and to vouching for third-party content; a monthly cadence for the project.
- C3 Bare upstream checksums. Trade-off: integrity only; a hostile mirror wins.
- C4 C1 now, with a `signers` list so a project key can add an endorsement later. 
- **Pick C4.** The release key never signs datasets. Reuse the manifest parser style (TOML + minisign) so the verifier is already written; the hub's node key signature (L2 Ed25519) versus minisign key format is a lead decision (open question 3).

**Fork D - where datasets live and how big the hub tier is**
- D1 Under `fabric/blobs` with a new `Class::Dataset`. Trade-off: reuses refcounts/outboards; pollutes the message store's watermarks.
- D2 `/var/lib/111/datasets/<id>/` owned by `node111`, own quota, outside the store's live-bytes count. Trade-off: a new row in the Layer 1 on-disk-layout table (ownership is the security boundary).
- **Pick D2.** The blob code is still reused for hashing and outboards.

**Fork E - bulk movement between hubs (Tier 2 ownership)**
- E1 Layer 8 owns dataset replication as HTTP Range pulls between paired hubs, verified by pins. Trade-off: no sealed transport, so public data only.
- E2 Layer 7 builds Tier 2 `OBJ_*` over the fabric first. Trade-off: sealed and mesh-routable, but a larger build gating datasets.
- **Pick E1 for datasets; Tier 2 stays Layer 7's for sealed objects** and both use `bao-tree` verification.

**Storage budgets (proposal; all sizes from finding 6, dated):**

| Device | Space for datasets | Hosts | Does not host |
|---|---|---|---|
| Cat 4A hub, SSD/HDD 256 GB-2 TB | whole disk minus OS | EN maxi/nopic, regional or national PMTiles, several languages | planet (120 GB) only on 512 GB+ |
| Cat 4A on Pi, 64 GB SD | ~30-40 GB (guess) | nopic only if nothing else; mini, Simple, regional PMTiles | maxi |
| Cat 4A on Pi, 32 GB SD | ~10-15 GB (guess) | mini (13 GB), small ZIMs, one region | nopic |
| Cat 2B laptop | user choice | anything, reads from a hub by preference | - |
| Cat 2A / 3 phone (Termux, app-private) | a few GB at most | small ZIMs, a city PMTiles; mostly delegates to a hub | EN anything |

SD-card wear: a ZIM or PMTiles download is one large sequential write per refresh; serving is read-only; the cost to state is "a refresh
rewrites the file", not continuous write amplification. kiwix-serve's own writes while serving are unmeasured (unverified).

## 5. Open questions for the owner

1. May the hub operator's internet-connected tool fetch from Kiwix (download.kiwix.org) and Protomaps (maps.protomaps.com daily builds) on a
   user action, under the consent and metered-link rules, or must the first copy always arrive by sneakernet/another hub?
2. Is a project-endorsed catalogue (a second signature) wanted at all, given it makes the project vouch for third-party content?
3. Hub-owner signing: the L2 node key (Ed25519, new domain-separated context) or a minisign key like the release channel?
4. Which dataset classes ship in the first Layer 8 build: one ZIM class (Simple English + a medical selection) and one regional PMTiles only?
5. Does the owner accept Layer 6's P7 (child reverse proxy) as a hard prerequisite for the Kiwix panel, or should Layer 8 ship a "start
   kiwix-serve, open its port" link first?
6. Is the legal read of "vector PMTiles = Produced Work" needed before a hub is allowed to serve a re-extracted region, or is "unmodified
   Protomaps output only" the v1 rule?
7. Hub-to-hub replication of a release payload: allowed in v1 (a hub mirrors upstream's `kiwix-tools` binary)? It makes the hub a GPL distributor.

## 6. Sources

Verified (raw.githubusercontent.com, PyPI, npm, crates.io): kiwix/kiwix-tools `COPYING`, `README.md`, `meson.build`, `docs/kiwix-serve.rst`, `src/server/kiwix-serve.cpp` - kiwix/libkiwix `COPYING`, `ChangeLog`, `static/templates/catalog_v2_entry.xml` - openzim/libzim `COPYING`, `README.md`, `ChangeLog`, `meson.build`, `src/archive.cpp` - openzim zim-tools, zimit, warc2zim, python-libzim `LICENSE` - kiwix-js README, kiwix-js-pwa `LICENSE` - protomaps/PMTiles `LICENSE`, `spec/v3/spec.md`; go-pmtiles `LICENSE`, `extract.go`; protomaps/basemaps `LICENSE.md`, `LICENSE_DATA.md`, `README.md`; protomaps-leaflet `LICENSE` - stadiamaps/pmtiles-rs `README.md`, `Cargo.toml`, licences - maplibre/martin `README.md`, licences - maptiler/tileserver-gl, onthegomap/planetiler, gnosygnu/xowa, brouter, valhalla, osrm-backend, graphhopper, organicmaps, comaps licence files - termux-packages `packages/{kiwix-tools,libzim,libkiwix}/build.sh` - theupdateframework/specification `tuf-spec.md` - gitlab.com/fdroid/fdroidserver `fdroidserver/index.py` - spdx/license-list-data `text/ODbL-1.0.txt`, `text/CC-BY-SA-4.0.txt` - pypi.org/pypi/{libzim,zimscraperlib,warc2zim}/json - crates.io/api/v1/crates/{martin,pmtiles} - registry.npmjs.org/{pmtiles,protomaps-leaflet,leaflet}.

Repo: `README.md` s.8 and Category 4; `plan/DECISIONS.md` D30; `plan/layers/7-relay/options.md` facts 12-15, 18, 21-22; `plan/layers/7-relay/research/05-ui-consent-floor.md:7,71-72`; `1-bootstrap/crates/node111/src/manifest.rs`; `.../daemon/panels/{static.rs,manifest.rs,mod.rs}`; `5-messaging/crates/mesh111/src/{blobs.rs,store/prune.rs}`; `0-knowledge/concepts/secure-contexts.md`.

Search snippets (marked in text): en.wikipedia.org/wiki/Kiwix; get.kiwix.org FAQ (flavours); hub.kiwix.org/weblog/2026/03/question-time-q1-2026 (sha256 beside ZIM); forums.bunsenlabs.org/viewtopic.php?id=9535; docs.protomaps.com/basemaps/downloads; pyviz-tutorial.readthedocs.io/en/latest/protomaps/extract.html; download.geofabrik.de; selfhosting.sh/apps/kiwix/; neilzone.co.uk (Pi 4 kiwix); pkgs.alpinelinux.org and archlinux.org kiwix-tools; kiwix-tools.readthedocs.io; Debian handbook / libapt docs.

Denied: download.kiwix.org, library.kiwix.org and mirrors, dumps.wikimedia.org, docs/maps.protomaps.com, geofabrik.de, wiki.openstreetmap.org, creativecommons.org, opendatacommons.org, en.wikipedia.org, GitHub API.
