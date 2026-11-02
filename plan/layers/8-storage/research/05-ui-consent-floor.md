# Layer 8 research 05 - the Storage page, catalogue and download consent, hosted panels, and the Pi-floor weight of every candidate

Lens 5 of the Layer 8 sweep, 2026-11-02. Prefixes: `N/` = `1-bootstrap/crates/node111/src/daemon/`, `UI/` = `4-interface/ui/`, `L7` =
`7-relay/README.md`, `R00` = `research/00-code-seams.md`, `R01` = `research/01-datasets-hub.md`. Network: `download.kiwix.org`,
`library.kiwix.org`, `kiwix-tools.readthedocs.io`, `dumps.wikimedia.org` do not resolve from this box (one try each); GitHub raw, npm, crates.io,
PyPI work. Licences below are read from the upstream file on raw.githubusercontent.com unless marked. I did not run cargo, 111d or any server. The
bundle sizes in section 3 are my own esbuild measurements in the session scratchpad (npm tarballs, `--minify --target=es2020`, `gzip -9`).

## 1. Summary

- **The map bundle Layer 7 recorded is too small.** L7 section 10.5 and its R05 say "Leaflet + PMTiles + protomaps-leaflet about 69 KB gz". Bundled the
  way Vite would (Leaflet 1.9.4 + protomaps-leaflet 5.1.0, which embeds its own pmtiles 3.2.1 and the basemap themes) it is **80.4 KB gz, 83.9 KB
  with Leaflet's CSS**. That is over the 64 KB per-chunk cap and over the lazy bucket's 36.1 KB of headroom, so the exempt "map" bucket is needed and
  must be sized at about 90 KB. A raster-only variant (Leaflet + pmtiles 4.5.0) is 50.8 KB, 54.3 KB with CSS: it fits one chunk but still not the
  lazy total. MapLibre GL 6.16.0 is 451 KB.
- **kiwix-serve is the right hosted encyclopedia, and it is also the agent's search endpoint.** It is GPL-3.0-or-later C++, HTTP only, no
  authentication option at all, and it has a documented public `/search` (full text, `format=xml`), `/raw` and `/catalog/v2` OPDS API
  (`kiwix-serve.rst`), so the agent's "ask the knowledge" needs no libzim binding: one loopback tool. Its RSS is **not published anywhere I could
  reach**; only anecdotes ("under 50 MB", search snippets). It must be measured on the owner's Pi.
- **The Pi floor is a table the owner can say yes or no to (section 2 and finding 14).** Everything third-party is zero-weight while off if it
  follows the Layer 6 component seam; nothing proposed by the other lenses is allowed in the required configuration except what is already in the
  lock (SQLCipher, `blake3`, `bao-tree`, `tar`, `flate2`). Syncthing, rqbit and iroh each add a resident listener and timers and are the three the
  owner is most likely to refuse for the base.
- **The Storage page cannot be honest yet because the API cannot say what is on the disk.** There is no filesystem free/total in `/api/v1`, no
  per-class bytes, and nothing measures `agent/`, `bridge/`, `rns/`, `releases/` or `cache/` (R00 section 3). Also the 60 s capacity pass writes
  `settings` every minute on an idle node (R00 finding 6): a "disk free" figure must be read on request, not persisted.
- **Downloads need consent in the shape the daemon already has** (`409 consent_required` with the exact text and its hash, `N/components/mod.rs:9`)
  plus the L7 metered rule. At a 1 GB/day cap a 57 GB ZIM takes 57 days; the page must say so instead of promising completion.
- **A region extract is a place.** The bounding box of a hub's PMTiles file tells every LAN visitor where the community is. This is the one
  privacy fact of hosting datasets and it belongs in the consent copy of a hub that serves maps.

## 2. Candidates table

Licences "yes" = read from the upstream LICENSE/COPYING file (or file headers) today. Resident weight is for the program **on**; every one is zero
while off if installed as an on-demand component (Layer 6 seam).

| Name | Upstream | Licence (verified?) | Language | Maturity | Resident weight when on | Fit | Why |
|---|---|---|---|---|---|---|---|
| kiwix-serve (kiwix-tools 3.8.2, libkiwix 14.2.1, libzim 9.8.2) | kiwix/kiwix-tools | GPL-3.0-or-later (COPYING + source header, yes); libzim GPL-2.0-or-later (header, yes) | C++17 | tools 3.8.2 in Termux (`termux-packages/packages/kiwix-tools/build.sh`), Alpine 3.8.2-r0 (search snippet) | RSS unpublished (anecdote under 50 MB, search snippet); mmaps the ZIM; 4 threads default; one TCP listener; no timers documented; reads only | **adopt** (component) | The encyclopedia server and the agent's search. No auth: bind loopback. |
| Leaflet 1.9.4 + CSS | Leaflet/Leaflet | BSD-2-Clause (npm + L7 R05 raw LICENSE) | JS | 2023-05-18 | browser only: 42.4 + 3.5 KB gz (measured) | adopt (map bucket) | The smallest real map library. |
| protomaps-leaflet 5.1.0 | protomaps/protomaps-leaflet | BSD-3-Clause (npm; L7 R05 raw) | JS | 2025-06-18 | browser: 36.7 KB gz alone, bundles pmtiles 3.2.1 (measured) | adopt (map bucket, vector) | Vector PMTiles on a canvas, no worker. |
| pmtiles (JS) 4.5.0 | protomaps/PMTiles | BSD-3-Clause, spec CC0 (raw LICENSE, yes) | JS | 2026-08-10 | browser: 7.4 KB gz (measured) | adopt (raster path only) | Range reader. |
| MapLibre GL JS 6.16.0 | maplibre/maplibre-gl-js | BSD-3-Clause (npm) | JS | 2026-10-25 | browser: 294 + 147 + 10 KB gz (measured) | reject | 7 x the chunk cap. |
| go-pmtiles | protomaps/go-pmtiles | BSD-3-Clause (raw, yes) | Go CLI | n/a | none unless `serve` is run; operator tool | reference | `extract` makes a region file off-node. |
| Martin 1.16.1 | maplibre/martin | MIT OR Apache-2.0 (LICENSE-MIT/-APACHE both 200, yes) | Rust | 2026-10-30; crate source 1.0 MB | unmeasured; PMTiles directory cache default 32 MB, later a 512 MB tile cache default (search snippet): both must be set down | reference | A Range route in 111d replaces it. |
| tileserver-gl | maptiler/tileserver-gl | BSD-2-Clause (LICENSE.md, yes) | Node >= 20 + native renderer; a `-light` pure-JS image exists (README) | active | Node RSS unmeasured | reject | Server-side raster rendering we do not need. |
| Syncthing 2.1.7 | syncthing/syncthing | MPL-2.0 (LICENSE, yes) | Go | in Termux 2.1.7 (build.sh) | idle under 100 MB typical; 130 MB (v1.30) to 200 MB (v2.0.1) on one 27 GB folder; OOM kills on 512 MB Pis (forum snippets); listeners 22000/8384/21027 (unverified); scan and index timers | **reject for the base**; component only | Resident, periodic, and it moves files we already route. |
| rqbit 9.1.0 | ikatson/rqbit | Apache-2.0 (LICENSE, yes) | Rust | 2026-10-29; Termux 9.1.0 | "usually a few tens of MB" (its README:103); HTTP API :3030, DHT UDP | reject for the base | DHT/trackers expose the node's IP to the internet; duplicates Tier 2. |
| iroh 1.4.1 / iroh-blobs 0.104.0 | n0-computer/iroh, iroh-blobs | MIT OR Apache-2.0 (LICENSE-MIT/-APACHE both 200, yes, both repos) | Rust | 2026-10-21 / 2026-10-28 (crates.io) | holds a persistent relay QUIC connection while idle (search snippet, third-party); unmeasured; **absent from our lock** (R00 section 2) | reference | Lens 2 owns it; flag: resident connection, not on the Pi floor. |
| `zim` crate 0.5.0 | dignifiedquire/zim | MIT OR Apache-2.0 (LICENSE-MIT/-APACHE 200 on `master`, yes) | Rust | 2026-07-20 | links `zstd`, `xz2`, old `memmap` (its Cargo.toml) | reject for grounding | Reads and extracts only; no full-text index; adds C codecs. |
| zstd 0.14.3 / zstd-sys 2.1.2 | gyscos/zstd-rs | BSD-3-Clause for sys (crates.io metadata) | Rust + C | 2026-11-01 | +C compile; crate source 772 KB | not needed | ZIM and PMTiles are already compressed; nothing to recompress. |
| ruzstd 0.9.1 | KillingSpark/zstd-rs | MIT (crates.io metadata) | pure Rust | 2026-10-11 | n/a | reference | The pure-Rust fallback if a zstd reader is ever needed. |
| tar 0.4.46, flate2 (rust_backend), `blake3`, `bao-tree`, SQLCipher | already in `Cargo.lock` | in the existing LICENSING rows | Rust | locked | already counted in 111d's RSS | adopt | Zero new weight. |
| Kiwix Android 3.x | kiwix/kiwix-android | GPL-3 (COPYING 200, yes) | Kotlin | Android 3.14.1, 2026-05-14 (search snippet) | n/a | reference | Download-UI prior art (finding 5). |

## 3. Findings that change the plan

**The Storage page**

1. **What exists to build on.** `/settings/l8` is the generic `LayerPage` ("arrives with Layer 8", `UI/src/views/settings/model.ts:221-222`);
   l5 and l6 are special-cased with a lazy chunk (`UI/src/views/settings/index.tsx:14,18-20`, the l6 precedent). The L5 page already has a
   store gauge (`UI/src/views/services/store-gauge.tsx`), the prune copy and a quota editor over `settings.quotas` for six refusable classes
   (`4-interface/README.md` section 6.7, "As built"). Layer 8 should reuse the gauge and the editor, not rebuild them.
2. **The API cannot feed a Storage page.** `FabricStore` is `{open, used, watermark, store_full, restore_pending}` (`openapi.yaml:1026-1035`);
   there is no filesystem free/total, no per-class bytes, and `agent/`, `bridge/`, `rns/`, `releases/`, `cache/` are measured by nothing (R00
   section 3, "Disk is measured in"). Additive fields needed: `disk_free`, `disk_total`, a `classes[]` array (`id, label, layer, bytes, quota,
   prunable`), and the watermark as a setting. Compute them **on request** (one `statvfs`, one directory walk with a cap), never on a timer.
3. **Layout of the page (proposal).** Top: a gauge of the disk ("12.4 GB free of 29.8 GB"). Then a table **by class** (messages and mail, knowledge
   base, updates, datasets, agent state, components cache, bridge accounts, logs) with a layer column, bytes, the quota editor and a "what happens
   when full" cell. A second view **by layer** is a grouping of the same rows, not new data. Honest copy, verbatim candidate: "Nothing here is
   deleted because of its age. When the space set aside is used up, copies that can be fetched again go first, oldest first; your own messages,
   contacts and anything waiting to be sent stay. If that is still not enough, new items are refused and this page says so." That is the L5 section
   10.4 order stated without a promise it does not keep. Datasets are the open question (recommendation F1).
4. **What the agent may propose.** It already proposes through `proposal.write` and a human accepts (`3-intelligence/README.md:108-120`;
   `Proposal.kind` enum, `openapi.yaml:754`). Add one kind, `storage`, whose body is a closed JSON vocabulary like the `device` kind: `quota.set
   {class, bytes|null}`, `dataset.remove {id, version}` (a superseded version only), `dataset.fetch {id, version}` (accepting it opens the consent
   dialog; it never skips it). The agent's bearer reads `storage.read` (a new fixed tool, bumping both pins, R00 section 3(f)); it never
   accepts, never changes a metered setting, never starts a download.
5. **Budget.** The Storage page and the catalogue each get a lazy chunk at or under 64 KB gz (`UI/scripts/check-budget.mjs:14`). Lazy is at
   219.9 of 256 KB (R00 finding 27), so two new chunks of about 20 KB each fit; the map does not (finding 8).

**The catalogue and the download**

6. **Prior art, with what each one gives us.**
   - *Kiwix OPDS* (`libkiwix/static/templates/catalog_v2_entry.xml`): per entry `id (uuid)`, `title`, `summary`, `language`, `name`, `flavour`,
     `category`, `tags`, `articleCount`, `mediaCount`, `publisher`, `dc:issued`, and an acquisition link with `length`. **No hash, no licence, no
     signature field.** The hash is a separate `.sha256` fetch per file on Kiwix's mirrors, plus `.torrent`, `.btih`, `.magnet`, `.meta4`
     (`github.com/kiwix/operations/wiki/Downloads`). Our catalogue row must carry `sha256` (or BLAKE3, lens 2), `size`, `licence`,
     `attribution`, `version`, `publisher`, `used_by`, and the hub owner's signature (R01 finding 3: F-Droid `entry.jar` shape).
   - *Kiwix Android* strings (`kiwix-android/core/src/main/res/values/strings.xml`, read raw): "Allow downloading content via mobile network?
     If you choose Yes you won't be warned in future" (`wifi_only_title`/`wifi_only_msg`), a setting "Download content only via WiFi",
     "Space Available:", "Insufficient space to download.", "Paused (device is offline)", "Stop download?". Three lessons: show free space
     beside the size; "paused" is a state, not an error; **"won't be warned in future" is the pattern to avoid** (a one-time yes becomes a
     standing one; 111 records consent per text hash).
   - *F-Droid* has separate Wi-Fi and mobile-data sliders where the middle position blocks images (forum snippets, `forum.f-droid.org`
     threads 3606, 15668): users could not tell what a middle position meant. Use a plain on/off per rule and name the rule in words.
   - *Organic Maps/CoMaps*: one file per region, a country can exceed 100 MB, and search needs the region downloaded first (search
     snippets, secondary). The region list is the catalogue; there is no per-tile download.
7. **The consent dialog (candidate copy, then the shape).** Rows: name, version, publisher, **size**, "this node has 41.2 GB free; after this,
   9.8 GB", licence and attribution line, "used by: Encyclopedia panel, the agent", the connection ("Wi-Fi 'home' - not metered" or "'phone-hotspot'
   is marked metered with a 1.0 GB daily cap: this download will pause at the cap and need about 57 days"), and two buttons "Download" and "Not
   now". Shape: `POST /api/v1/datasets/{id}/fetch` answers `409 consent_required {text, hash}` until repeated with `accept_warning = hash`
   (the `N/components/mod.rs:9` precedent; "changed text asks again", L7 section 12). Progress rides the existing SSE stream as `dataset.progress`
   events **only while a job runs** (no polling timer; the UI page needs no poll). Resumable: the fetcher must stream to a staging file
   with `Range` and verify the hash at the end; the updater's fetcher reads the whole body into memory with a 512 MiB cap and has no
   Range (R00 finding 16), so a new streaming fetcher is a build item. Staging sits under a node111-owned path (the components cache is root-owned,
   R00 traps).
8. **Metered rule for datasets.** L7's `metered = [{match: {iface|ssid}, daily_cap_bytes}]`, default `[]`, rolling 24 h on `CLOCK_BOOTTIME`, SSID
   a hint because Termux cannot read Android's metered flag (L7 section 12). For datasets that means: with `metered = []` nothing is metered and the
   dialog says "no metered link is set"; on a marked link the job **pauses at the cap and resumes in the next window**, and the dialog shows the
   arithmetic. At 1 GB/day the 7.4 GB mini ZIM takes 8 days, the 57 GB nopic 57 days (sizes: next finding). A per-download override is a second
   explicit tap with its own consent text, never the agent's.
9. **Sizes (dated, secondary).** English Wikipedia ZIM, 13 Nov 2024: 110 GB with pictures, 57 GB without, 7.4 GB introductions only (HandWiki
   footnote on `en.wikipedia.org/wiki/Kiwix`, search snippet); Kiwix FAQ: nopic is about 75 % smaller than full, mini about 95 % (legacy.kiwix.org
   FAQ, search snippet). Other reports say about 100 GB for the full file (search snippets). Protomaps planet about 120 GB (docs.protomaps.com,
   search snippet), zoom 0-6 extract about 60 MB, a Half Moon Bay extract about 2 MB (same). R01 has an additional Berlin figure. **No size was
   readable from a live listing**; the catalogue stores the size from the ZIM itself at ingest, so the UI never relies on a remembered number.
10. **"Where a dataset is used".** Each catalogue row carries `used_by` from a closed list: `map` (the Nearby/Map view through the tiles route),
    `encyclopedia` (the kiwix panel), `agent` (grounding via kiwix `/search`), `updates` (L7 origin). The Storage page shows the same chip beside
    the bytes, so "what do I lose if I remove this" is answered before the delete.

**Hosted panels**

11. **kiwix-serve behind the panel proxy.** The panel host registers `kind = "static"` only; `child`/`detected` (the generic reverse proxy,
    P7) are refused at registration (`N/panels/manifest.rs:400-404`, `5-messaging/README.md:32,524`). kiwix-serve is HTTP only and has no
    authentication flags (full usage text, `kiwix-serve.cpp`), so it must never be bound to a LAN address: bind `--address 127.0.0.1`, put the
    panel host's `p111_<id>` cookie gate in front, and give it its own panel listener so `--urlRootLocation` is not needed (each panel has its own
    origin). Until P7 exists the choices are a link-out to a LAN-bound kiwix-serve (unauthenticated, the L7 origin posture) or wait. Options
    in F2. Flags that matter on a Pi: `--threads` (default 4), `--nolibrarybutton`, `--catalogOnly` (no content), `--monitorLibrary` (a file
    watch: leave it off, send `SIGHUP` after a catalogue change), `--ipConnectionLimit` (`kiwix-serve.rst`).
12. **Agent grounding over a ZIM.** `kiwix-serve` documents `/search?pattern=&books.name=&format=xml&pageLength=` (public, capped at 140 results),
    `/raw/ZIMNAME/content/PATH` (no server-side processing, public) and `/suggest` (JSON, listed as private). So the tool is `kb.search {query}`
    against `127.0.0.1:<port>` only while the encyclopedia is on; the agent reads title and snippet, never a page body over 8 KB. libzim
    bindings are not needed: `python-libzim` is a 31.5 MB wheel (R01), `zim-rs`/`zim-sys` bind libzim in C++, the pure `zim` crate has no
    full-text index. A cold ZIM costs a process start: the tool answers "the encyclopedia is off; propose turning it on" through the proposal
    path rather than starting a server itself.
13. **The map page.** Measured with esbuild (IIFE, es2020, minified, gzip -9; npm tarballs): Leaflet 1.9.4 42.4 KB + CSS 3.5 KB; pmtiles 4.5.0 alone
    7.4 KB; Leaflet + pmtiles 50.8 KB; protomaps-leaflet 5.1.0 alone **36.7 KB** (it bundles pmtiles 3.2.1 and `@protomaps/basemaps`: 38 KB of 128 KB
    raw is theme data); Leaflet + protomaps-leaflet 80.4 KB; MapLibre GL 6.16.0 294.0 + worker 146.9 + CSS 10.2 KB. So **L7's "about 69 KB" should read
    about 84 KB with CSS**; the exempt bucket's ceiling is 90 KB. Other map facts: the dashboard CSP is `img-src 'self' data:` with `connect-src 'self'`
    (`N/dashboard/routes.rs:78-80`), no `blob:`; protomaps-leaflet draws to a canvas, and a raster path would need `blob:` or data URLs (a CSP edit).
    Whether protomaps-leaflet's label fonts need a remote font file offline is **unverified**; the bundle references `FontFace` and `sans-serif`
    (dist/esm/index.js). A tiles route needs `Range` (206, `Accept-Ranges: bytes`), and none exists: `panels/static.rs` has ETag and 304 only and
    refuses files over 64 MiB (`:39,227`); a grep of `static.rs`, `listener.rs`, `mod.rs` for "range" finds only `RangeInclusive`. The Range arm
    is the same code that serves resumable dataset downloads and the L7 origin.

**Consent and privacy**

14. **LAN, members, internet.** Four postures a hub can take for datasets: off (default); LAN, unauthenticated (the L7 origin posture, files
    are public by design); LAN, only paired sessions (the panel cookie); and a signed catalogue offered over a paired hub link (L7 section 4). The
    reason to prefer the cookie for ZIMs is not secrecy but that kiwix-serve cannot authenticate itself. A dataset **never** leaves on a carrier or
    on LoRa (L7 section 6; `lora-airtime-budget.md`). What a hub reveals by hosting: the list of datasets and versions it holds, the bounding box
    of any map extract, and (if served) request times; nothing about members. The consent text for turning dataset serving on should say all three.
15. **Obligations a hub operator takes on (facts).** *Wikipedia text:* CC BY-SA 4.0 (and GFDL unless noted), attribution naming the article and
    its URL and a link to the licence, changes under a compatible licence (Wikipedia:Reusing, search snippet; the CC clause wording was not
    confirmed). A kiwix-serve viewer shows the ZIM's own pages, so the panel should show a one-line notice and link the ZIM's `License` and `Source`
    metadata. ZIM metadata keys include Title, Creator, Publisher, Date, Name, Flavour, Source, License (openZIM gist, search snippet). *OSM-derived
    tiles:* ODbL 1.0; a map shown publicly is a Produced Work and the notice "Contains information from OpenStreetMap, which is made available
    here under the Open Database License (ODbL)" with the URIs, placed where a viewer expects it, is required for public use (sections 4.2-4.3,
    search snippets of OSM legal-talk and wiki pages; the licence text itself not fetched). Share-alike applies to a **derived database**, not to a
    Produced Work (same sources, debated). The map view must therefore draw the OSM credit on the map, not only in a settings page. *Software
    served unmodified:* kiwix-serve is GPL-3.0-or-later; as a separate program it does not touch D22. *Mixed licences inside a ZIM* (images)
    **(unverified)**: the notice should point at the ZIM, not claim a single licence.

**Android / Termux**

16. **Storage rules on a phone node.** Termux has app-private storage (`$HOME`, `$PREFIX`, our `$PREFIX/var/lib/111`, `1-bootstrap/README.md:76`)
    plus a shared-storage symlink set after `termux-setup-storage` (Termux docs, search snippets). 111 needs neither the permission nor Termux:API:
    datasets stay app-private, which avoids the shared-storage layer (reported slower since Android 11, and removed USB-drive access; 2021-era
    sources, search snippets). A ZIM is read by `mmap`, which is fine on app-private storage and is the case to test on a real phone. Two
    consequences to put in copy: uninstalling Termux removes the datasets with the app; and background limits mean kiwix-serve must be started
    on demand by the panel host, not left resident (L1 phantom-process hardening is already a known fragility). `kiwix-tools 3.8.2`, `libkiwix`,
    `libzim` and `syncthing` and `rqbit` exist as Termux packages (`termux-packages` build.sh files, read raw); `martin`, `go-pmtiles`,
    `tileserver-gl`, `iroh` do not. A phone is category 2A and should host only small ZIMs (R01: nopic English alone is 57 GB).

**The Pi floor**

17. **The one table.** "Boot" = what the required configuration pays. Baseline: 111d idle RSS 19.7 MB release (the task brief) and 44.7-46.5 MB
    debug with 111-agent 8.9 MB (`6-transport/TASKS.md:82`); listeners :11180 (+ best effort :80) and :11182.

| Candidate | In the required configuration? | Listeners when on | Timers/polls when on | Disk writes when on | Binary / install weight | RSS when on |
|---|---|---|---|---|---|---|
| Range route + dataset index in 111d | yes, as code in `panels/static.rs` | none new (existing :11180) | none; event-driven | none until a download; then a streaming write of the file size | about 60-120 lines (my estimate, L7 R05); no dependency | 0 idle |
| Dataset fetcher (streaming, resumable) | yes, as code | none | none; a job exists only while downloading | the file size plus one fsync at the end; staging then rename | none (hyper/reqwest already) | one bounded buffer during a job |
| SQLCipher (store, V1-V3) | already in | none | the 60 s prune pass; **one settings write per minute when idle (R00)** | that write | in the lock | in the 19.7 MB |
| `blake3`, `bao-tree`, `iroh-io`, `tar`, `flate2` | already in the lock (R00 section 2) | none | none | none | 0 | 0 |
| automerge in-binary delta | **no**: not in the lock; lens 3 | none | none | per applied change | unmeasured here; measure with `cargo bloat` before the owner says yes | unknown |
| zstd 0.14 / zstd-sys | no | none | none | none | C compile + 772 KB crate source; not needed | n/a |
| kiwix-serve | no: component | 1 TCP, loopback | none documented | none (read-only) | Termux .deb of kiwix-tools + libkiwix + libzim; size unmeasured | unpublished; measure with `/proc/PID/smaps_rollup` |
| Martin / tileserver-gl / go-pmtiles serve | no | 1 TCP | cache, none documented | none | Rust binary / Node + native / Go binary; unmeasured | caches default 32 to 512 MB (Martin, snippet) |
| Syncthing | **no** | 22000, 8384, 21027 (unverified) | rescans, index exchange | database + index | Go binary; Termux 2.1.7 | 100 to 200 MB; OOM on 512 MB (snippets) |
| rqbit | **no** | 3030 + DHT UDP | DHT refresh | piece writes | Rust binary; Termux 9.1.0 | "a few tens of MB" (README) |
| iroh / iroh-blobs | **no** | UDP socket + a relay connection held open (snippet) | keepalives | blob store | crates in no lock | unmeasured |

    Everything below the "no" line can be an on-demand component if it follows the Layer 6 seam (manifest-pinned payload under
    `cache/packages/<id>/`, enable-time install, consent, G7 zero-weight gate). Two seam changes are needed: a component today must name a bridge
    `adapter` (`N/components/manifest.rs:134,685`, R00 finding 11), and a panel cannot be `child` (finding 11). The measuring protocol for the
    owner's Pi: `VmRSS` and `Pss` from `/proc/PID/smaps_rollup` after the first search and after ten; `ss -ltnup` before and after; `strace -c -f
    -e trace=write,fsync` for ten idle minutes; `ls -l` of the binary and `du -sh` of its cache; record in `8-storage/TASKS.md`.

## 4. Recommendation

**F1. Do datasets count toward the store's capacity pruning?**
- (a) Yes, same pruner: one rule, but a 57 GB file competes with message history (R00: the 512 MiB high watermark counts blobs).
- (b) No: datasets live in their own tree, are never auto-pruned, and only the user (or an accepted agent proposal) removes one; the page shows
  them and the free space beside the store gauge.
- (c) Datasets are pruned like refetchable public blobs when disk is under 1 % free.
- **Pick (b).** The README says "pruned by capacity"; deleting a 50 GB file the user fetched at a cost would be the policy surprise it forbids.
  The copy in finding 3 states it. (c) is a later opt-in.

**F2. How is the encyclopedia shown?**
- (a) Build the P7 reverse proxy now (first consumer MeshChatX also needs it): cookie-gated, one origin, loopback kiwix-serve.
- (b) Link-out to kiwix-serve on a LAN port (no code): unauthenticated, LAN only while serving is on.
- (c) Re-implement a ZIM reader in 111d: reject (reinvent; the `zim` crate has no search).
- **Pick (a) as the target, (b) as the v1 stopgap** behind the same consent as L7 origin serving, so no unauthenticated listener appears without a
  recorded text. Needs the owner's say because P7 is parked.

**F3. The map.**
- (a) Vector: Leaflet + protomaps-leaflet, about 84 KB gz, exempt bucket at 90 KB; small files (2 MB per small city extract, search snippet).
- (b) Raster PMTiles: Leaflet + pmtiles, 54 KB, fits a chunk, still needs the exemption for the lazy total; raster files are larger (unverified).
- (c) MapLibre GL: 451 KB, reject. (d) Link-out only to the hosted Meshtastic map: blank offline (L7 R05).
- **Pick (a)** with the correction to L7 section 10.5 recorded. (b) is the fallback if the owner wants a 64 KB chunk.

**F4. Metered rule for datasets.**
- (a) Refuse on any marked metered link. (b) Pause at the daily cap, resume next window, show the days. (c) (b) plus a per-download override.
- **Pick (c), default (b).** Never silent, never the agent.

**F5. Agent grounding.** (a) kiwix-serve `/search` on loopback as a `kb.search` tool; (b) libzim bindings; (c) the `zim` crate.
- **Pick (a).** No new dependency, and the tool answers "off" when the component is off.

**F6. Where does a dataset's trust come from?** Out of scope here (R01 picks the hub-owner-signed catalogue); this lens only requires that the
consent dialog shows the signer ("signed by this hub" or "signed by a hub you follow") next to the size.

## 5. Open questions for the owner

1. Is "datasets are never auto-pruned, only the user removes them" (F1 b) your reading of "pruned by capacity rather than by imposed rules"?
2. May the Storage page add `disk_free`, `disk_total` and per-class bytes to `/api/v1` (read on request, no timer), and may the idle
   once-a-minute `settings` write be cut to on-change (SD wear)?
3. P7: build the generic reverse proxy for kiwix (and MeshChatX), or accept the LAN link-out for v1 (F2)?
4. May the exempt map bucket be 90 KB gz (the L7 figure of 69 KB was low), and may the CSP gain `blob:` only if a raster path is chosen?
5. Does a metered link marked by SSID need an override at all, or is "pause at the cap" enough (F4)?
6. Which of Syncthing, rqbit and iroh may exist as an opt-in component at all, given they are resident and (rqbit) expose the node's IP?
7. May kiwix-serve be installed at `sudo 111 install` time for hubs (4A/4B) only, never phones, with enable-time consent?

## 6. Sources

- Repo (all read 2026-11-02): `N/panels/static.rs:1-60,227`; `N/panels/manifest.rs:380-404`; `N/dashboard/routes.rs:76-110`; `N/components/mod.rs:9,173,194`;
  `assets/panels/meshtastic.toml`; `UI/src/views/settings/{index.tsx,model.ts}`, `UI/src/lib/layers.ts`, `UI/src/views/services/store-gauge.tsx`,
  `UI/scripts/check-budget.mjs:1-14`; `4-interface/README.md` sections 2, 6.7, 15; `4-interface/api/openapi.yaml:754,1026-1051`;
  `3-intelligence/README.md:23-134`; `7-relay/README.md` sections 10.5, 12; `plan/layers/8-storage/research/{00,01}-*.md`;
  `plan/layers/7-relay/research/05-ui-consent-floor.md`; `6-transport/TASKS.md:82`; `1-bootstrap/README.md:76,139-143`.
- Licences (raw.githubusercontent.com): `kiwix/kiwix-tools/main/COPYING` and `src/server/kiwix-serve.cpp` header; `kiwix/libkiwix/main/COPYING`,
  `src/server.cpp` header; `openzim/libzim/main/COPYING`, `src/archive.cpp` header; `maplibre/martin/main/LICENSE-{MIT,APACHE}`, `Cargo.toml`;
  `maptiler/tileserver-gl/master/LICENSE.md`; `syncthing/syncthing/main/LICENSE`; `ikatson/rqbit/main/LICENSE`, `README.md:103`;
  `n0-computer/iroh/main/LICENSE-*`, `iroh-blobs/main/LICENSE-*`; `protomaps/go-pmtiles/main/LICENSE`, `protomaps/PMTiles/main/LICENSE`;
  `kiwix/kiwix-android/main/COPYING`; `dignifiedquire/zim/master/{LICENSE-*,Cargo.toml,README.md}`.
- Kiwix docs (raw): `kiwix/kiwix-tools/main/docs/kiwix-serve.rst`, `src/server/kiwix-serve.cpp` usage text, `meson.build`;
  `kiwix/libkiwix/main/static/templates/catalog_v2_entry.xml`, `ChangeLog`; `kiwix/kiwix-android/main/core/src/main/res/values/strings.xml`.
- Termux packages (raw): `termux/termux-packages/master/packages/{kiwix-tools,libkiwix,libzim,syncthing,rqbit}/build.sh`.
- npm: `registry.npmjs.org/{leaflet,pmtiles,protomaps-leaflet,maplibre-gl}` (versions, dates, tarballs); crates.io API: `zstd`, `zstd-sys`, `ruzstd`,
  `iroh`, `iroh-blobs`, `rqbit`, `martin`, `zim`, `tar`, `bao-tree`.
- Fetched through WebFetch: `https://github.com/kiwix/operations/wiki/Downloads` (mirrors, `.sha256`, `.torrent`, `.meta4` suffixes).
- Search snippets (not fetched): kiwix-serve RSS anecdotes (`neilzone.co.uk/2025/02/...`, `selfhosting.sh/apps/kiwix/`, `peachpie.theatl.social/post/288858`);
  Wikipedia ZIM sizes (`en.wikipedia.org/wiki/Kiwix`, `legacydev.kiwix.org/en/?p=41`); Protomaps sizes (`docs.protomaps.com/basemaps/downloads`,
  `til.simonwillison.net/gis/pmtiles`); Syncthing memory (`forum.syncthing.net/t/ram-utilization/9769`, `.../syncthing-2-0-1-oom/24824`,
  `.../syncthing-uses-too-much-ram-killed-by-oom-reaper/24598`); Martin cache (`maplibre.org/martin/sources-files/`); iroh idle relay
  (`instagit.com/n0-computer/iroh/...`, third-party); Termux storage (`dev.to/terminaltools/...`, `developer.android.com/preview/privacy/storage`);
  ODbL notice (`wiki.openstreetmap.org/wiki/Open_Data_License/Community_Guidelines/Attribution_For_Different_Media_Types`, OSM legal-talk lists);
  Wikipedia reuse (`gwern.net/wiki/Wikipedia:Reuse` mirror); F-Droid sliders (`forum.f-droid.org/t/f-droid-app-doesn-t-have-icons/3606`,
  `.../no-preview-samsung-a50/15668`); ZIM metadata keys (`gist.github.com/rgaudin/326a808965a6efbce56cdf70c4695dd1`).
- My measurements: `/tmp/claude-0/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/scratchpad/bundle/` (esbuild 0.25.0 bundles a.js to e.js).
- Denied or unresolved: `download.kiwix.org`, `library.kiwix.org`, `kiwix-tools.readthedocs.io`, `dumps.wikimedia.org`, GitHub HTML via the browser
  proxy for anything but the Kiwix operations wiki.
