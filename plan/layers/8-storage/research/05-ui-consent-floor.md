# Layer 8 research 05 - the Storage page, catalogue and download consent, hosted panels, and the Pi-floor weight of every candidate

Lens 5, 2026-11-02. Prefixes: `N/` = `1-bootstrap/crates/node111/src/daemon/`, `UI/` = `4-interface/ui/`, `L7` = `7-relay/README.md`,
`R00`/`R01` = `research/00-code-seams.md`/`01-datasets-hub.md`. `download.kiwix.org`, `library.kiwix.org`, `kiwix-tools.readthedocs.io` and
`dumps.wikimedia.org` do not resolve from this box. Licences are read from upstream files on raw.githubusercontent.com unless marked. Bundle sizes are
my own esbuild measurements (npm tarballs, minified, es2020, `gzip -9`). No cargo, 111d or server was run.

## 1. Summary

- **The map bundle Layer 7 recorded is too small.** L7 section 10.5 says Leaflet + PMTiles + protomaps-leaflet is about 69 KB gz. Bundled as Vite
  would, Leaflet 1.9.4 + protomaps-leaflet 5.1.0 (which embeds pmtiles 3.2.1 and the basemap themes) is **80.4 KB gz, 83.9 KB with CSS**: over the
  64 KB chunk cap and over the lazy bucket's 36.1 KB headroom, so the exempt "map" bucket must be about 90 KB. Raster-only (Leaflet + pmtiles 4.5.0)
  is 54.3 KB with CSS: one chunk, still over the lazy total. MapLibre GL 6.16.0 is 451 KB.
- **kiwix-serve is the right hosted encyclopedia and also the agent's search endpoint.** GPL-3.0-or-later C++, HTTP only, no authentication option,
  with a documented public `/search` (full text, `format=xml`), `/raw` and `/catalog/v2` OPDS API (`kiwix-serve.rst`): the agent needs no libzim
  binding, one loopback tool. Its RSS is not published anywhere I could reach (anecdote "under 50 MB", snippet); measure it on a Pi.
- **The Pi floor is one table (finding 17).** Third-party programs are zero-weight while off if they follow the Layer 6 component seam; only what is
  already in the lock (SQLCipher, `blake3`, `bao-tree`, `tar`, `flate2`) belongs in the base. Syncthing, rqbit and iroh are resident and are the
  three to refuse for the base.
- **The Storage page cannot be honest yet:** `/api/v1` has no filesystem free/total or per-class bytes, and nothing measures `agent/`, `bridge/`,
  `rns/`, `releases/` or `cache/` (R00 section 3). The 60 s capacity pass already writes `settings` every minute idle (R00 finding 6): read disk
  figures on request, never persist them.
- **Downloads need consent in the daemon's existing shape** (`409 consent_required` with text and hash, `N/components/mod.rs:9`) plus the L7
  metered rule; at a 1 GB/day cap a 57 GB ZIM takes 57 days and the page must say so.
- **A region extract is a place.** A PMTiles file's bounding box tells every LAN visitor where the community is; the consent copy for serving
  maps must say so.

## 2. Candidates table

Licences "yes" = read from the upstream LICENSE/COPYING file (or file headers) today. Resident weight is for the program **on**; every one is zero
while off if installed as an on-demand component (Layer 6 seam).

| Name | Upstream | Licence (verified?) | Language | Maturity | Fit | Why |
|---|---|---|---|---|---|---|
| kiwix-serve (kiwix-tools 3.8.2, libkiwix 14.2.1, libzim 9.8.2) | kiwix/kiwix-tools | GPL-3.0-or-later (COPYING + header, yes); libzim GPL-2.0-or-later (header, yes) | C++17 | 3.8.2 in Termux (`build.sh`), Alpine 3.8.2-r0 (snippet) | **adopt** (component) | Encyclopedia server and the agent's search; no auth, so loopback only. |
| Leaflet 1.9.4 | Leaflet/Leaflet | BSD-2-Clause (npm; L7 R05 raw) | JS | 2023-05-18 | adopt (map bucket) | Smallest real map library. |
| protomaps-leaflet 5.1.0 | protomaps/protomaps-leaflet | BSD-3-Clause (npm; L7 R05 raw) | JS | 2025-06-18 | adopt (map bucket) | Vector PMTiles on a canvas, no worker. |
| pmtiles (JS) 4.5.0 | protomaps/PMTiles | BSD-3-Clause, spec CC0 (raw, yes) | JS | 2026-08-10 | adopt (raster path only) | Range reader. |
| MapLibre GL JS 6.16.0 | maplibre/maplibre-gl-js | BSD-3-Clause (npm) | JS | 2026-10-25 | reject | 451 KB gz, 7 x the chunk cap. |
| go-pmtiles | protomaps/go-pmtiles | BSD-3-Clause (raw, yes) | Go CLI | n/a | reference | `extract` makes a region file off-node. |
| Martin 1.16.1 | maplibre/martin | MIT OR Apache-2.0 (both files, yes) | Rust | 2026-10-30 | reference | A Range route in 111d replaces it. |
| tileserver-gl | maptiler/tileserver-gl | BSD-2-Clause (LICENSE.md, yes) | Node >= 20 + native | active | reject | Server-side raster rendering we do not need. |
| Syncthing 2.1.7 | syncthing/syncthing | MPL-2.0 (yes) | Go | Termux 2.1.7 | reject for the base | Resident, periodic. |
| rqbit 9.1.0 | ikatson/rqbit | Apache-2.0 (yes) | Rust | 2026-10-29 | reject for the base | DHT exposes the node's IP; duplicates Tier 2. |
| iroh 1.4.1 / iroh-blobs 0.104.0 | n0-computer | MIT OR Apache-2.0 (both repos, yes) | Rust | 2026-10-21 / 2026-10-28 | reference | Lens 2 owns it; holds a relay connection (snippet); not in our lock. |
| `zim` crate 0.5.0 | dignifiedquire/zim | MIT OR Apache-2.0 (yes) | Rust | 2026-07-20 | reject | Extraction only, no full-text index; links `zstd`, `xz2`. |
| zstd-sys / ruzstd 0.9.1 | crates.io | BSD-3-Clause / MIT (metadata) | Rust+C / Rust | 2026-11-01 / 2026-10-11 | not needed | ZIM and PMTiles are already compressed. |
| tar, flate2, `blake3`, `bao-tree`, SQLCipher | in `Cargo.lock` | existing LICENSING rows | Rust | locked | adopt | Zero new weight. |

Resident weights for the programs above are in one table, finding 17.

## 3. Findings that change the plan

**The Storage page**

1. **What exists.** `/settings/l8` is the generic `LayerPage` (`UI/src/views/settings/model.ts:221-222`); l6 is a lazy chunk
   (`index.tsx:14,18-20`). The L5 page already has a store gauge (`views/services/store-gauge.tsx`), the prune copy and a quota editor over six
   refusable classes (`4-interface/README.md` section 6.7). Reuse them.
2. **The API cannot feed a Storage page.** `FabricStore` is `{open, used, watermark, store_full, restore_pending}` (`openapi.yaml:1026-1035`);
   there is no filesystem free/total, no per-class bytes, and `agent/`, `bridge/`, `rns/`, `releases/`, `cache/` are measured by nothing (R00
   section 3). Needed, additive: `disk_free`, `disk_total`, `classes[]` (`id, label, layer, bytes, quota, prunable`), the watermark as a setting;
   computed **on request** (one `statvfs`, one capped directory walk), never on a timer.
3. **Layout (proposal).** A disk gauge ("12.4 GB free of 29.8 GB"); a table **by class** (messages and mail, knowledge base, updates, datasets,
   agent state, components cache, bridge accounts, logs) with layer, bytes, quota editor; a by-layer grouping of the same rows. Candidate copy:
   "Nothing here is deleted because of its age. When the space set aside is used up, copies that can be fetched again go first, oldest first; your
   own messages, contacts and anything waiting to be sent stay. If that is still not enough, new items are refused and this page says so" (the L5
   section 10.4 order). Datasets: F1.
4. **What the agent may propose.** Writes go through `proposal.write`, a human accepts (`3-intelligence/README.md:98-134`; `openapi.yaml:754`).
   Add a kind `storage` with a closed vocabulary like `device`: `quota.set {class, bytes|null}`, `dataset.remove {id, version}` (a superseded
   version only), `dataset.fetch {id, version}` (acceptance opens the consent dialog, never skips it). A read tool `storage.read` bumps both pins
   (R00 section 3(f)). It never accepts, never changes a metered setting, never starts a download.
5. **Budget.** Storage page and catalogue each get a lazy chunk at most 64 KB gz (`check-budget.mjs:14`). Lazy is at 219.9 of 256 KB (R00 finding 27):
   two chunks of about 15 KB fit, the map does not (finding 13).

**The catalogue and the download**

6. **Prior art, with what each one gives us.**
   - *Kiwix OPDS* (`libkiwix/static/templates/catalog_v2_entry.xml`): per entry `id (uuid)`, `title`, `summary`, `language`, `name`, `flavour`,
     `category`, `tags`, `articleCount`, `mediaCount`, `publisher`, `dc:issued`, and an acquisition link with `length`. **No hash, no licence, no
     signature field.** The hash is a separate `.sha256` fetch per file on Kiwix's mirrors, plus `.torrent`, `.btih`, `.magnet`, `.meta4`
     (`github.com/kiwix/operations/wiki/Downloads`). Our catalogue row must carry `sha256` (or BLAKE3, lens 2), `size`, `licence`,
     `attribution`, `version`, `publisher`, `used_by`, and the hub owner's signature (R01 finding 3: F-Droid `entry.jar` shape).
   - *Kiwix Android* strings (`kiwix-android/core/src/main/res/values/strings.xml`, raw): "Allow downloading content via mobile network? If you
     choose Yes you won't be warned in future", "Download content only via WiFi", "Space Available:", "Insufficient space to download.", "Paused
     (device is offline)". Lessons: free space beside the size; "paused" is a state, not an error; **"won't be warned in future" is the
     pattern to avoid** (111 records consent per text hash).
   - *F-Droid*: separate Wi-Fi and mobile-data sliders whose middle position blocks images; users could not tell what it meant (forum snippets).
     Use plain on/off and name the rule in words.
7. **The consent dialog.** Rows: name, version, publisher, **size**, free space before and after ("41.2 GB free; after this 9.8 GB"), licence and
   attribution line, "used by", the connection ("'phone-hotspot' is marked metered with a 1.0 GB daily cap: this will pause at the cap and
   take about 57 days"), "Download" and "Not now". Shape: `POST /api/v1/datasets/{id}/fetch` answers `409 consent_required {text, hash}` until
   repeated with `accept_warning = hash` (the `N/components/mod.rs:9` precedent; changed text asks again, L7 section 12). Progress rides the SSE
   stream as `dataset.progress` only while a job runs (no poll). Resumable means a streaming fetcher with `Range` into a staging file, hash
   verified at the end; the updater's fetcher reads the whole body into memory (512 MiB cap, no Range, R00 finding 16), so this is a build item,
   staged under a node111-owned path (the components cache is root-owned).
8. **Metered rule for datasets.** L7's `metered = [{match: {iface|ssid}, daily_cap_bytes}]`, default `[]`, rolling 24 h on `CLOCK_BOOTTIME`, SSID
   a hint because Termux cannot read Android's metered flag (L7 section 12). For datasets that means: with `metered = []` nothing is metered and the
   dialog says "no metered link is set"; on a marked link the job **pauses at the cap and resumes in the next window**, and the dialog shows the
   arithmetic. At 1 GB/day the 7.4 GB mini ZIM takes 8 days, the 57 GB nopic 57 days (sizes: next finding). A per-download override is a second
   explicit tap with its own consent text, never the agent's.
9. **Sizes (dated, secondary).** English Wikipedia ZIM on 13 Nov 2024: 110 GB with pictures, 57 GB without, 7.4 GB introductions only (HandWiki
   footnote, `en.wikipedia.org/wiki/Kiwix`, search snippet); other reports about 100 GB for the full file. Protomaps planet about 120 GB, zoom 0-6
   extract about 60 MB, a small-town extract about 2 MB (`docs.protomaps.com`, `til.simonwillison.net/gis/pmtiles`, snippets). **No size came from a
   live listing**; the catalogue takes size from the file at ingest, so the UI never shows a remembered number.
10. **Where a dataset is used.** Each row carries `used_by` from a closed list: `map` (tiles route), `encyclopedia` (kiwix panel), `agent`
    (kiwix `/search`), `updates` (L7 origin). The Storage page shows the chip beside the bytes, so "what do I lose" is answered before a delete.

**Hosted panels**

11. **kiwix-serve behind the panel proxy.** The panel host registers `kind = "static"` only; `child`/`detected` (the generic reverse proxy,
    P7) are refused at registration (`N/panels/manifest.rs:400-404`, `5-messaging/README.md:32,524`). kiwix-serve is HTTP only and has no
    authentication flags (usage text, `kiwix-serve.cpp`), so bind `--address 127.0.0.1` and put the panel host's `p111_<id>` cookie gate in front;
    a panel's own listener gives it its own origin, so `--urlRootLocation` is not needed. Until P7 exists the alternative is a link-out to a
    LAN-bound, unauthenticated kiwix-serve (F2). Pi flags: `--threads` (default 4), `--monitorLibrary` off (a file watch; send `SIGHUP`
    instead), `--ipConnectionLimit` (`kiwix-serve.rst`).
12. **Agent grounding over a ZIM.** `kiwix-serve` documents `/search?pattern=&books.name=&format=xml&pageLength=` (public, capped at 140 results),
    `/raw/ZIMNAME/content/PATH` (no server-side processing, public) and `/suggest` (JSON, listed as private). So the tool is `kb.search {query}`
    against `127.0.0.1:<port>` only while the encyclopedia is on; the agent reads title and snippet, never a page body over 8 KB. libzim
    bindings are not needed (`python-libzim` is a 31.5 MB wheel, R01; the `zim` crate has no full-text index). With the component off the tool
    answers "the encyclopedia is off" and the agent may propose turning it on; it never starts a server itself.
13. **The map page.** Measured with esbuild (IIFE, es2020, minified, gzip -9; npm tarballs): Leaflet 1.9.4 42.4 KB + CSS 3.5 KB; pmtiles 4.5.0 alone
    7.4 KB; Leaflet + pmtiles 50.8 KB; protomaps-leaflet 5.1.0 alone **36.7 KB** (it bundles pmtiles 3.2.1 and `@protomaps/basemaps`: 38 KB of 128 KB
    raw is theme data); Leaflet + protomaps-leaflet 80.4 KB; MapLibre GL 6.16.0 294.0 + worker 146.9 + CSS 10.2 KB. So **L7's "about 69 KB" should read
    about 84 KB with CSS**; the exempt bucket's ceiling is 90 KB. The CSP has no `blob:` (`N/dashboard/routes.rs:78-80`); vector tiles draw to a canvas, a raster path may need it. Offline label
    fonts are **unverified**. A tiles route needs `Range` (206, `Accept-Ranges: bytes`), and none exists: `panels/static.rs` has ETag and 304 only and
    refuses files over 64 MiB (`:39,227`); a grep of `static.rs`, `listener.rs`, `mod.rs` for "range" finds only `RangeInclusive`. The Range arm
    is the same code that serves resumable dataset downloads and the L7 origin.

**Consent and privacy**

14. **LAN, members, internet.** Postures: off (default); LAN, unauthenticated (the L7 origin posture, public files); LAN, paired sessions only (the
    panel cookie); a paired-hub link (L7 section 4). The cookie is preferred for ZIMs because kiwix-serve cannot authenticate itself, not for
    secrecy. A dataset never rides a carrier or LoRa (L7 section 6; `lora-airtime-budget.md`). A hub reveals the datasets and versions it holds, the
    bounding box of any map extract and request times if logged; nothing about members. The consent text should say all three.
15. **Obligations of a hub operator (facts).** *Wikipedia text:* CC BY-SA 4.0 (GFDL unless noted): attribution naming the article, its URL and a
    licence link; changes under a compatible licence (Wikipedia:Reusing mirror, search snippet; the CC clause wording not confirmed). ZIM metadata
    carries `License` and `Source` (openZIM gist, snippet), so the panel shows a one-line notice linking them. *OSM-derived tiles:* ODbL 1.0; a map
    shown publicly is a Produced Work and needs the notice "Contains information from OpenStreetMap, which is made available here under the Open
    Database License (ODbL)" with URIs where a viewer expects it (sections 4.2-4.3; OSM wiki and legal-talk snippets, licence text not fetched);
    share-alike attaches to a derived database, not a Produced Work (debated). So the credit is drawn on the map itself. *kiwix-serve* is GPL-3.0-or-later and
    separate, so D22 is untouched. Mixed licences inside a ZIM (images) **(unverified)**: point at the ZIM, claim no single licence.

**Android / Termux**

16. **Phone nodes.** Termux has app-private storage (`$PREFIX/var/lib/111`, `1-bootstrap/README.md:76`) and a shared-storage symlink set after
    `termux-setup-storage` (snippets). 111 needs neither that permission nor Termux:API: keep datasets app-private, which avoids the shared-storage
    layer (reported slower since Android 11; 2021-era snippets). Whether `mmap` of a ZIM is fast there is a test for a real phone. Copy must say
    uninstalling Termux removes the datasets, and kiwix-serve must be started on demand by the panel host, never left resident (background
    killing is already a known fragility, L1). `kiwix-tools 3.8.2`, `libkiwix 14.2.1`, `libzim 9.8.2`, `syncthing 2.1.7`, `rqbit 9.1.0` are Termux packages
    (`build.sh`, raw); `martin`, `go-pmtiles`, `tileserver-gl`, `iroh` are not. A phone (2A) should host only small ZIMs.

**The Pi floor**

17. **The one table.** "Boot" = what the required configuration pays. Baseline: 111d idle RSS 19.7 MB release (the task brief) and 44.7-46.5 MB
    debug with 111-agent 8.9 MB (`6-transport/TASKS.md:82`); listeners :11180 (+ best effort :80) and :11182.

| Candidate | In the base? | Listeners | Timers | Disk writes | Install weight | RSS when on |
|---|---|---|---|---|---|---|
| Range route + dataset index | yes, code | none new | none | none until a download | about 60-120 lines (L7 R05 estimate), no dependency | 0 idle |
| Streaming resumable fetcher | yes, code | none | a job only while downloading | file size, one fsync, rename | none | one bounded buffer |
| SQLCipher store V1-V3 | already in | none | 60 s prune; **one `settings` write per minute idle (R00)** | that write | in the lock | in the 19.7 MB |
| `blake3`, `bao-tree`, `iroh-io`, `tar`, `flate2` | in the lock | none | none | none | 0 | 0 |
| automerge in-binary | **no**, lens 3 | none | none | per change | unmeasured: `cargo bloat` first | unknown |
| zstd / zstd-sys | no | none | none | none | C compile, 772 KB crate; unneeded | n/a |
| kiwix-serve | component | 1 TCP loopback | none documented | none (read only) | Termux deb of three packages, unmeasured | unpublished; measure |
| Martin / tileserver-gl / go-pmtiles serve | component | 1 TCP | caches | none | Rust / Node + native / Go; unmeasured | Martin caches default 32 to 512 MB (snippet) |
| Syncthing | **no** | 22000, 8384, 21027 (unverified) | rescans, index exchange | db + index | Go; Termux 2.1.7 | 100-200 MB; OOM on 512 MB (snippets) |
| rqbit | **no** | 3030 + DHT UDP | DHT refresh | piece writes | Rust; Termux 9.1.0 | "a few tens of MB" (README:103) |
| iroh / iroh-blobs | **no** | UDP + a held relay connection (snippet) | keepalives | blob store | in no lock | unmeasured |

    Every "no" row can be an on-demand component under the Layer 6 seam (pinned payload in `cache/packages/<id>/`, enable-time install, consent,
    G7 zero-weight gate). Two seam changes: a component must name a bridge `adapter` (`N/components/manifest.rs:134,685`) and a panel cannot be
    `child` (finding 11). Measuring protocol for the Pi: `VmRSS` and `Pss` from `/proc/PID/smaps_rollup` after the first and the tenth search; `ss
    -ltnup` before and after; `strace -c -f -e trace=write,fsync` over ten idle minutes; binary and cache sizes; record in `8-storage/TASKS.md`.

## 4. Recommendation

**F1. Do datasets count toward the store's capacity pruning?**
- (a) Yes, same pruner: one rule, but a 57 GB file competes with message history (R00: the 512 MiB high watermark counts blobs).
- (b) No: datasets live in their own tree, are never auto-pruned, and only the user (or an accepted agent proposal) removes one; the page shows
  them and the free space beside the store gauge.
- (c) Datasets are pruned like refetchable public blobs when disk is under 1 % free.
- **Pick (b).** The README says "pruned by capacity"; deleting a 50 GB file the user fetched at a cost would be the policy surprise it forbids.
  The copy in finding 3 states it. (c) is a later opt-in.

**F2. How is the encyclopedia shown?**
- (a) Build the P7 reverse proxy (MeshChatX needs it too): cookie-gated, loopback kiwix-serve.
- (b) Link-out to kiwix-serve on a LAN port: no code, unauthenticated.
- (c) A ZIM reader in 111d: reject (reinvent).
- **Pick (a) as the target, (b) as the v1 stopgap** behind a recorded consent text like L7 origin serving. P7 is parked, so this needs the owner.

**F3. The map.**
- (a) Vector: Leaflet + protomaps-leaflet, about 84 KB gz, exempt bucket at 90 KB.
- (b) Raster PMTiles: Leaflet + pmtiles, 54 KB, fits one chunk but not the lazy total; raster files are larger (unverified).
- (c) MapLibre GL, 451 KB: reject. (d) Link-out to the hosted Meshtastic map: blank offline (L7 R05).
- **Pick (a)**, recording the correction to L7 section 10.5.

**F4. Metered rule for datasets.**
- (a) Refuse on any marked metered link. (b) Pause at the daily cap, resume next window, show the days. (c) (b) plus a per-download override.
- **Pick (c), default (b).** Never silent, never the agent.

**F5. Agent grounding.** (a) kiwix-serve `/search` on loopback as a `kb.search` tool; (b) libzim bindings; (c) the `zim` crate.
- **Pick (a).** No new dependency, and the tool answers "off" when the component is off.

Signing is R01's pick (hub-owner-signed catalogue); this lens only requires the consent dialog to show the signer beside the size.

## 5. Open questions for the owner

1. Is "datasets are never auto-pruned, only the user removes them" (F1 b) your reading of "pruned by capacity rather than by imposed rules"?
2. May `/api/v1` gain `disk_free`, `disk_total` and per-class bytes (on request, no timer), and the idle once-a-minute `settings` write become
   on-change (SD wear)?
3. P7: build the generic reverse proxy for kiwix and MeshChatX, or accept the LAN link-out for v1 (F2)?
4. May the exempt map bucket be 90 KB gz (L7's 69 KB was low), and the CSP gain `blob:` only if a raster path is chosen?
5. Which of Syncthing, rqbit and iroh may exist even as an opt-in component, given they are resident and (rqbit) expose the node's IP?
6. May kiwix-serve be installed at `sudo 111 install` time for hubs (4A/4B) only, never phones, with enable-time consent?

## 6. Sources

- Repo (read 2026-11-02): `N/panels/{static.rs:1-60,227,manifest.rs:380-404}`; `N/dashboard/routes.rs:76-110`; `N/components/mod.rs:9`; `assets/panels/meshtastic.toml`;
  `UI/src/views/settings/{index.tsx,model.ts}`, `UI/scripts/check-budget.mjs`; `4-interface/README.md` sections 2, 6.7, 15; `openapi.yaml:754,1026-1051`;
  `3-intelligence/README.md`; `7-relay/README.md` sections 10.5, 12; `research/{00,01}`; `plan/layers/7-relay/research/05-ui-consent-floor.md`;
  `6-transport/TASKS.md:82`; `1-bootstrap/README.md:76`.
- Raw GitHub: `kiwix/kiwix-tools/main/{COPYING,docs/kiwix-serve.rst,src/server/kiwix-serve.cpp,meson.build}`; `kiwix/libkiwix/main/{COPYING,static/templates/catalog_v2_entry.xml,ChangeLog}`;
  `openzim/libzim/main/{COPYING,src/archive.cpp}`; `kiwix/kiwix-android/main/{COPYING,core/src/main/res/values/strings.xml}`; `maplibre/martin/main/{LICENSE-*,Cargo.toml}`;
  `maptiler/tileserver-gl/master/{LICENSE.md,README.md}`; `syncthing/syncthing/main/LICENSE`; `ikatson/rqbit/main/{LICENSE,README.md}`; `n0-computer/{iroh,iroh-blobs}/main/LICENSE-*`;
  `protomaps/{go-pmtiles,PMTiles}/main/LICENSE`; `dignifiedquire/zim/master/*`; `termux/termux-packages/master/packages/{kiwix-tools,libkiwix,libzim,syncthing,rqbit}/build.sh`.
- npm `registry.npmjs.org/{leaflet,pmtiles,protomaps-leaflet,maplibre-gl}`; crates.io API for `zstd-sys`, `ruzstd`, `iroh`, `iroh-blobs`, `rqbit`, `martin`, `zim`.
- Fetched: `https://github.com/kiwix/operations/wiki/Downloads`.
- Search snippets (not fetched): `neilzone.co.uk/2025/02/a-self-hosted-wikipedia-and-other-bits-clone-using-kiwix-on-a-raspberry-pi-4/`, `selfhosting.sh/apps/kiwix/`;
  `en.wikipedia.org/wiki/Kiwix`, `legacydev.kiwix.org/en/?p=41`; `docs.protomaps.com/basemaps/downloads`, `til.simonwillison.net/gis/pmtiles`;
  `forum.syncthing.net/t/ram-utilization/9769`, `.../syncthing-2-0-1-oom/24824`; `maplibre.org/martin/sources-files/`; `instagit.com/n0-computer/iroh/...` (third-party);
  `developer.android.com/preview/privacy/storage`, `dev.to/terminaltools/accessing-android-storage-from-termux-a-practical-guide-50e5`;
  `wiki.openstreetmap.org/wiki/Open_Data_License/Community_Guidelines/Attribution_For_Different_Media_Types`; `gwern.net/wiki/Wikipedia:Reuse`;
  `forum.f-droid.org/t/f-droid-app-doesn-t-have-icons/3606`, `.../no-preview-samsung-a50/15668`; `gist.github.com/rgaudin/326a808965a6efbce56cdf70c4695dd1`.
- My measurements: `/tmp/claude-0/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/scratchpad/bundle/` (esbuild 0.25.0).
- Unreachable: `download.kiwix.org`, `library.kiwix.org`, `kiwix-tools.readthedocs.io`, `dumps.wikimedia.org`.
