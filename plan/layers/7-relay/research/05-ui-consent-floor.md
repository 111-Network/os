# Layer 7, lens 5: Nearby radar/map, gateway and presence UI, consent, and the Pi-floor weight of every candidate

Research scout report, 2026-11-01. Fetched facts win over training; "(search snippet)" = seen only in a search result; "(unverified)" = could not be fetched or measured. Code is cited as `file:line` under `/home/user/os`. Bundle sizes are my own: I downloaded each npm tarball from `registry.npmjs.org` and ran `gzip -9` on the shipped dist file (Vite's level may differ by a percent or two). Airtime arithmetic reuses lens 1 (`01-presence-discovery.md` section 3.1), re-derived per hour. Nothing was built or left running.

## 1. Summary

- **A map does not fit the UI budget as it stands; a radar does.** The lazy bucket is 219.9 of 256 KB gzip (`node scripts/check-budget.mjs` on the current `dist`), so **36.1 KB** is free. Leaflet is 42.4 KB + 3.5 KB CSS; Leaflet + PMTiles + protomaps-leaflet is ~69 KB; MapLibre GL is 294 + 147 (worker) + 10 KB CSS = 451 KB, seven times the 64 KB per-chunk cap. A hand-drawn SVG radar is a few KB (estimate). **Ship the radar in v1; make the map an optional bucket the owner exempts from D21 (as fonts are), or leave it to the hosted Meshtastic panel.**
- **Do not draw a bearing the radio does not have.** LoRa gives hops, SNR/RSSI and sometimes a coarse position, never direction. Rings by hops/signal; angle only where both ends have a position; every Meshtastic position drawn as a cell, because the firmware truncates it to a per-channel bit count and centres it (`firmware PositionModule.cpp:486-507`). 111's own nodes share no position by default.
- **The adopted Meshtastic web client already has a map.** It bundles `maplibre-gl` and fetches `tiles.openfreemap.org` only when online, so it is blank offline (`LICENSING.md:60`); MeshMonitor is the same (KB). MeshChatX is the exception: OpenLayers with MBTiles upload (search snippet). Adopt-don't-reinvent: Nearby links to the panel for Meshtastic positions.
- **Pi floor, in one line:** the required configuration already carries ~205 timer wakeups/min and one outbound HTTPS HEAD per minute. A 15-minute presence heartbeat adds 0.07 wakeups/min (CPU) but 4 frames/hour of airtime: **its cost is the radio channel, not the Pi.** Measured third-party components are 29-64 MB RSS while on and zero while off; rnsd as transport, rust-nostr, babeld, a BLE scanner have **no measured RSS** and none should become resident without the owner.
- **Gateway status is a claim and a privacy choice.** Show "internet reachable at last check, N min ago", never "online"; tell members and contacts only (sealed), never a public announce bit by default. **Termux cannot read Android's metered flag** (verified from the Termux:API source), so v1's data budget is a user setting.
- **The licensed-radio rule is already structural in Layer 6**; Layer 7 must consult it on every new path (gateway flush, hub forward) so a sealed frame never enters a licensed link.

## 2. Candidates table

| name | upstream | license (verified?) | runtime | maturity | resident weight | fit | why |
|---|---|---|---|---|---|---|---|
| Own SVG radar | `4-interface/ui/src/views/nearby/` | GPL-3.0-or-later (ours) | TSX | ASCII placeholder `radar.ts` exists | 0 resident; few KB lazy (estimate) | **adopt (build)** | no basemap; fits the 36.1 KB headroom |
| Leaflet | Leaflet/Leaflet | BSD-2-Clause (yes, raw `LICENSE`) | JS | 1.9.4, 2023-05-18 | 42.4 KB gz + 3.5 CSS (measured) | reference / optional bucket | smallest real map, raster tiles |
| PMTiles reader | protomaps/PMTiles | BSD-3-Clause code, spec CC0 (yes, raw `LICENSE`) | JS | 4.5.0, 2026-08-10 | 8.0 KB gz | adopt, only with a map | one static file, HTTP Range, no tile server |
| protomaps-leaflet | protomaps/protomaps-leaflet | BSD-3-Clause (yes) | JS | 5.1.0, 2025-06-18 | 15.2 KB gz | reference | vector tiles on Leaflet |
| MapLibre GL JS | maplibre/maplibre-gl-js | BSD-3-Clause (yes, raw `LICENSE.txt`) | JS, WebGL | 6.16.0, 2026-10-25 | 294 + 147 + 10 KB gz | reject for the bundle; fine inside a panel | 7x the chunk cap; already inside the Meshtastic panel |
| OpenLayers | openlayers/openlayers | BSD-2-Clause (yes, raw `LICENSE.md`) | JS | 10.11.0, 2026-10-04 | full `ol.js` 298 KB gz; tree-shaken unmeasured | reject | MeshChatX's choice; we need not |
| go-pmtiles | protomaps/go-pmtiles | BSD-3-Clause (yes) | Go CLI | `extract` makes regional files | none, a build-time tool | reference | how a hub's region file is made (Layer 8) |
| Meshtastic web (panel) | meshtastic/web v2.7.2 | GPL-3.0-only (`LICENSING.md:60`) | JS | adopted, D20 | 0 until the panel is on | adopt (already) | has the node map, blank offline |
| Termux:API | termux/termux-api | GPLv3 (README); `termux-api-package` scripts MIT (raw `LICENSE`) | Java app + shell | targetSdk 28 | separate app, per call | bridge (optional) | wifi and cell info; no metered flag |
| rnsd | markqvist/Reticulum | Reticulum License (yes, "2016-2026") | Python | `rns` 1.5.7, 2026-10-05 | no published RSS | bridge (already a child) | never imported; D17 #12 |
| rust-nostr | rust-nostr/nostr | MIT (yes) | Rust | `nostr-sdk` 0.45.4, 2026-09-19, 1.84 M dl | unmeasured | adopt in a separate binary only | agrees with lens 2 |
| iroh | n0-computer/iroh | MIT or Apache-2.0 (yes, `LICENSE-MIT`) | Rust | 1.4.1, 2026-10-21 | +13.3 MB binary (`DECISIONS.md:195`) | reference (v1.1) | rejected for v1 |
| babeld | jech/babeld | MIT-style (yes, "2007, 2008 Chroboczek") | C | Debian 1.12.1 | installed 151-249 KB (search snippet); RSS none found | reference | lens 3's call |
| btleplug / bluer | deviceplug/btleplug; bluez/bluer | btleplug BSD-3-Clause (yes, `LICENSE.md`); bluer not fetched (unverified) | Rust | btleplug 0.13.5, 2026-10-10; bluer 0.17.4, 2025-06-06 | needs `bluetoothd`; no BLE on Termux | reference | hardware-gated, bench |

## 3. Findings that change the plan

### 3.1 The Pi-floor table (the one the owner needs)

"Off" is zero for every row except the first.

| item | resident cost while ON | basis | zero while off? | owner decides |
|---|---|---|---|---|
| **111d + 111-agent (required config)** | 111d idle RSS **44.7-46.5 MB**, agent **8.9 MB** (debug, this box); ~**205 wakeups/min**; **1 HTTPS HEAD/min** (`probe.rs:46`) | `6-transport/TASKS.md:82`; `00-code-seams.md` 3.1(a) | n/a (the floor) | trim the baseline (two 2 s Meshtastic ticks with no board)? keep the probe? |
| **Our presence heartbeat (README sect. 7)** | one timer per transport; **+1/T wakeups/min**; airtime in 3.2 | arithmetic | yes if interval = 0 | **owner question (rule 3)**: events only, opt-in, or periodic |
| LXMF helper (ours, venv) | **43.7 MB RSS**, 10 threads, no listener; 5.83 MB cached, 24.9 MB installed | `6-transport/TASKS.md:50` | yes | none new |
| Delta Chat core + Madmail | **~64 MB RSS** together (29 + 35) | `TASKS.md:52,82`; KB | yes | none new |
| **rnsd** (already a child) | **no published RSS**; same CPython + RNS stack as our helper suggests 40-50 MB (inference). Transport mode adds a path table (1-week expiry) and an announce queue capped at 2 % of bandwidth. `enable_transport = true` is already default on Cat 3/4 (`6-transport/README.md:546`) | web search found no figure; Reticulum's manual recommends a Pi as the "personal" Transport Node (`gettingstartedfast.rst:121`) | yes | none as client; **measure on the bench** before Cat 1/2 runs transport |
| rust-nostr (separate carrier binary) | **unmeasured**; adds secp256k1 C lib, `async-wsocket`, `negentropy` (lens 2) | search found only an anecdotal WASM figure | yes as a CI-built binary, **no** if linked into 111d | build-time weight needs the owner (rule 3) |
| iroh | binary **+13.3 MB**; RSS unmeasured | `DECISIONS.md:195` | yes (separate) | stays rejected |
| babeld | tiny binary; no RSS found; one process, one UDP socket (port from memory, unverified) | Debian listings (search snippet) | yes (child) | lens 3 only |
| Tile route in 111d | **0 resident**: one axum handler. A Range handler must be written: no `Range`/`206` in `dashboard/` or `panels/`, and `tower-http` is not in `Cargo.lock` | grep; `Cargo.lock` | yes | may Layer 8 add `state/maps/`? |
| git on the node | zero resident (one-shot CLI) | general | yes | lens 4 |
| BLE scanner | needs `bluetoothd` on Linux; no BLE on Termux | task rule 5 | n/a | bench |
| mDNS | `mdns-sd`: one thread while LAN sharing is on; 5 s IP check | lens 1; `dashboard/mdns.rs:40-65` | yes | replace the 5 s check with netlink events? |

### 3.2 The heartbeat arithmetic (per node, one 138 B announce)

| T | frames/day | bytes/day sent | SF7/BW125 airtime per hour | SF10/BW125 | LongFast | ShortFast | host wakeups/min added |
|---|---|---|---|---|---|---|---|
| 15 min | 96 | 13,248 | 0.90 s (0.025 %) | 5.25 s (0.146 %) | 4.92 s | 0.45 s | 0.067 |
| 60 min | 24 | 3,312 | 0.23 s | 1.31 s (0.036 %) | 1.23 s | 0.11 s | 0.017 |
| 3 h | 8 | 1,104 | 0.075 s | 0.44 s | 0.41 s | 0.04 s | 0.006 |
| 12 h | 2 | 276 | 0.019 s | 0.11 s | 0.10 s | 0.01 s | 0.001 |

Aggregate share is the real figure (lens 1): 50 nodes at 15 min on SF10 spend 7.3 % of the channel on "I am here"; at 3 h, 0.61 %. Meshtastic already sends NodeInfo every 3 h and position every 1 h, so a 111 heartbeat stacks on existing presence. Against 205 baseline wakeups/min, 0.067 is noise: **the heartbeat is a spectrum cost, not a Pi cost.** On the LAN a `tcp-lan` keepalive (25 s per link) already exists, so a LAN heartbeat adds nothing. The biggest "heartbeat" already resident is the WAN probe: 1,440 HEADs/day; with a fresh TLS handshake of a few KB each (my estimate, unmeasured) that is several MB/day, which matters on a phone plan.

### 3.3 Radar and map

1. **UI today.** `/nearby` shows one panel per source with name, SNR/RSSI, hops, position as text, and a last-heard hint (`views/nearby/model.ts`); the radar is an ASCII placeholder of rings and `@` (`radar.ts:25-54`); positions are "never a link and never a map". The contract has `hops_away`, `snr`, `rssi`, `position`, `kind: meshtastic | reticulum` (`openapi.yaml:1096-1121`).
2. **Budget.** Eager 106.5/256 KB; lazy 219.9/256 KB; per-chunk cap 64 KB (largest today `jsQR`, 47.2 KB). Any map needs a second bucket with its own limit or a place outside the bundle. Fonts are already "listed, never counted" (`check-budget.mjs` header): the precedent.
3. **CSP** is `default-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self' data:` with no `worker-src` or `blob:` (`dashboard/routes.rs:78-80`). A same-origin tile route needs no CSP change; any off-origin tile host is blocked, which keeps the offline guarantee. MapLibre's worker ships as a file, so it can be loaded from `'self'`.
4. **Position precision.** `position_precision` is a per-channel `uint32` (`protobufs channel.proto:108-110`); the firmware masks the top `p` bits of the int32 degrees x 1e-7 field and adds half a cell to centre it (`PositionModule.cpp:486-507`); 0 sends nothing (`:173-174`). Cell sides, my arithmetic (1e-7 deg = 1.113 cm latitude; longitude shrinks by cos lat): 32 bits 1.1 cm; 24 bits 2.8 m; 19 bits 91 m; 16 bits 729 m; 13 bits 5.8 km; 10 bits 46.7 km. The snippet figures "13 bits = 2.9 km, 19 bits = 45 m" are half-cell. The UI draws an **uncertainty cell**, not a pin; the node list does not carry the precision today, so the daemon needs an additive field (or derives it from the channel).
5. **Offline-first norms.** Organic Maps/OsmAnd keep one file per region: a city 50-80 MB, a country 150-1,000 MB (search snippet, secondary sources, figures conflict). PMTiles extracts: a small city 2-11 MB at zoom 15, Berlin 68-84 MB, Japan 2.6 GB, planet 100-120 GB (search snippet, third-party). Kiwix: no result. Producing a region file is Layer 8's "regional maps".
6. **A tile source needs a route, not a server.** PMTiles is one file read by HTTP Range; 111d adds `GET /api/v1/tiles/{set}` honouring `Range` behind the session (~60 lines, my estimate). MBTiles would need a z/x/y handler and a plain-SQLite open of a user file beside our SQLCipher store (unverified). Pick PMTiles.

### 3.4 Presence UI semantics

1. **"Last heard" is a local fact per source and transport:** an age from this node's monotonic clock at the last verified frame, plus its evidence: "heard via Meshtastic on Bench radio, 4 min ago, SNR 6.5 dB"; "Reticulum announce, 2 hops via X"; "LAN link, verified packet"; "through the hub (custody held), not heard directly". A node's own clock is never freshness (`openapi.yaml:1109`; UI "clock ahead of this node"). Lens 1 table 4.1 is the vocabulary; the seams scout proposed an additive local freshness field.
2. **No policy limit of our own for "online"**: show the age; bucket it for display. Meshtastic's 2 h window (`NodeDB.cpp:1695`) may be cited, not enforced.
3. **Gateway badge**, four honest states: not a gateway · gateway, internet reachable at last check N min ago (5B with a stream open, 5A flushing on connect) · gateway, last flush N min ago with K pending · off by choice. Never "online". Settings names the carrier the claim rests on, so "internet" is never a probe of 111's own server.
4. **Role and hub registry**, three layers (lens 1 4.3): DNS-SD TXT (unauthenticated hint), signed announce role bits, sealed query after connect. Each row says which: "hint (unverified)", "signed by this contact", "asked just now". All names and roles are untrusted text.
5. **A Cat-1 relay shows nothing but "heard via".** The stock MeshCore repeater runs no 111 code (D19 c) and is visible only as hops or, if MeshCore adverts are ever fed in, as a "repeater" row. Nearby must not invent a relay entry; Layer 6's "relay" (bridge edges) stays on the Bridges page.
6. **The agent.** `nearby.read` returns the same sources; names reach the model only in a labelled untrusted block (`openapi.yaml:1099`). A hub for offload comes from the sealed query, never a TXT string, and stays on the LAN binding.
7. **Radar semantics.** Centre = this node. Ring = hop count (0 = direct is ring 1); within a ring order by SNR. **Angle means nothing** unless both ends report a position (then the bearing to the cell centre, with its uncertainty); otherwise a stable hash of the id, and the legend says so. No hop figure ⇒ an outer "unplaced" ring. The source kind picks the glyph. Plain SVG, so it runs on old phones.

### 3.5 Consent, privacy, metered links, phones

1. **Advertising gateway status.** Lens 2's pick (members and contacts only, sealed, via INTRO/card) fits "announces on demand only" (A6, `2-identity/README.md:348-351`) and static-only `appdata` role bits (A8). "Willing to gateway" is a user decision (it burns an announce counter and an fsync); "online now" is a sealed query. A public bit would tell every listener who has internet and battery; keep it off.
2. **Position sharing.** None by default. If opted in, copy Meshtastic's vocabulary: per-contact precision in bits, 0 = never, truncate and centre in the node, never on the receiver; positions sealed; a public BROADCAST position (kind 0x30, reserved) off by default. No GPS daemon is added; positions come from the board's NodeDB or manual entry.
3. **Licensed radio.** US: 47 CFR 97.113(a)(4) bars messages "encoded for the purpose of obscuring their meaning" on amateur stations; digital modes are allowed if meaning is not obscured (eCFR text, search snippet: confirm the current text on eCFR before relying on it). Other regions: unverified. Layer 6 reads `is_licensed` live, keeps readable relays off until a human confirms, and never sends sealed frames or PKI on a licensed board (`6-transport/README.md:284,703`; `lora-airtime-budget.md:70`). **Layer 7's duty:** every gateway flush or hub forward consults the same fact per outgoing link. A gateway relaying into a licensed radio needs a human.
4. **Metered links.** Android: `NET_CAPABILITY_NOT_METERED` marks networks that do not bill per byte, defer big transfers until one is present (reference, search snippet); `isActiveNetworkMetered()` and `getRestrictBackgroundStatus()` (Data Saver, API 24) are the app-side reads (search snippet). **Termux cannot read them:** `termux-wifi-connectioninfo` returns bssid, frequency, ip, link speed, mac, network id, rssi, ssid, supplicant state (`WifiAPI.java:37-47`, raw); `termux-telephony-cellinfo` returns cell type, signal, mcc/mnc; the manifest asks for `ACCESS_NETWORK_STATE` but no command exposes metered state. v1: a **user setting** ("treat this network as metered", a daily byte cap in Settings), with the SSID as a user-marked hint. An Android helper could read the flag later.
5. **Termux and netlink.** Termux and Termux:API both target SDK 28 (raw `gradle.properties`). Android restricts route-netlink `bind`/`RTM_GETLINK` for apps targeting API 30+ and keeps legacy domains exempt (AOSP sepolicy, search snippet). So `transport/netlink.rs:18-54` likely still binds on Termux, **unverified**; any event-driven replacement of the 60 s probe needs a polling fallback there.
6. **Doze.** Exact alarms fire "no more than once per nine minutes, per app" and Doze suspends network (lens 1 finding 9): a phone heartbeat is bounded below by 9 minutes and, in Doze, by no network. A 5A phone flushes when it gets network, which is the README's own behaviour.

## 4. Recommendation

### 4.1 Nearby: radar and map

| option | what | cost (gz) | trade-off |
|---|---|---|---|
| **R1. Radar only** | own SVG radar in the lazy `nearby` chunk; positions as cells; link to the Meshtastic panel's map | few KB, inside 36.1 KB | no basemap in 111's own UI |
| **R2. R1 + optional map bucket** | Leaflet + pmtiles + protomaps-leaflet loaded only if `/api/v1/tiles` lists a set; exempt "map" bucket in `check-budget.mjs` | +69 KB, only where tiles exist | one Range route and one budget-policy change |
| **R3. R1 + MapLibre** | vector WebGL map in the bundle | +451 KB | breaks D21 and the chunk cap; WebGL on old phones |
| **R4. Link-out only** | no map code | 0 | Reticulum/111 positions never mapped |

**Pick R1 now, R2 when Layer 8 has a region file.** R3 contradicts D21.

### 4.2 Presence timer

| option | cost | trade-off |
|---|---|---|
| **P-A. Event-driven only** (announce on link-up, card, INTRO; custody ACKs; foreign presence feeds Nearby) | zero timers, zero airtime | a quiet node looks stale |
| **P-B. Per-transport opt-in heartbeat, default off** | one timer in an off-by-default component; airtime per 3.2 | owner spends airtime for freshness |
| **P-C. Literal periodic heartbeat on every transport** | 7.3 % of an SF10 channel at 50 nodes/15 min | conflicts with rule 3 and A6 |

**Pick P-A as base, P-B available** (matches lens 1). The owner decides whether README section 7's "continuously announce" is met by P-A or must be reworded; I do not resolve it.

### 4.3 What stays out of the required configuration

Only the heartbeat timer question is open; every optional component in 3.1 is zero while off. The gap: **rnsd-as-transport, rust-nostr, babeld and a BLE scanner have no measured RSS in the repo.** Add a bench task "measure and record in `7-relay/TASKS.md`" before any is enabled by default.

## 5. Open questions for the owner

1. **Heartbeat:** P-A, P-B, or literal periodic? If periodic, what largest LoRa channel share do you accept (3.2)?
2. **Map:** an exempt map bucket plus a Range tile route, or a link to the Meshtastic panel until Layer 8?
3. **Radar angle:** is "angle has no meaning unless positions exist" acceptable, or should the radar become a ring list?
4. **Position sharing for 111's own nodes:** none by default (my pick), or opt-in per contact with precision bits?
5. **Gateway advertisement:** members and contacts only (my pick), or a public role bit?
6. **Metered links:** a user setting in v1 (my pick), or an Android helper later?
7. **Baseline trim:** the 60 s WAN probe and the 2 s Meshtastic ticks with no board bound (ties to lens 1 question 2)?
8. **Licensed boards:** a hard "no gateway or relay path onto a licensed board", or per-link human consent? Needs a regional legal check.

## 6. Sources

Repository (under `/home/user/os`): `README.md` (sect. 7, Cat 1 and 5), `4-interface/README.md:236-250,318-330`, `4-interface/api/openapi.yaml:1096-1121`, `4-interface/ui/src/views/nearby/{model.ts,radar.ts}`, `4-interface/ui/scripts/check-budget.mjs` (run on `dist`), `.../dashboard/routes.rs:74-80`, `1-bootstrap/Cargo.lock`, `6-transport/README.md:284,546,703`, `6-transport/TASKS.md:50,52,82`, `LICENSING.md:60`, `plan/DECISIONS.md:195`, KB `projects/{meshmonitor,meshchatx,deltachat-rpc-server,madmail}.md`, `concepts/lora-airtime-budget.md:70`, `plan/layers/7-relay/research/{00-code-seams,01-presence-discovery,02-gateways-global}.md`.

Fetched:
- https://registry.npmjs.org/maplibre-gl , /leaflet , /ol , /pmtiles , /protomaps-leaflet (versions, dates, tarballs measured)
- https://raw.githubusercontent.com/maplibre/maplibre-gl-js/main/LICENSE.txt · /Leaflet/Leaflet/main/LICENSE · /openlayers/openlayers/main/LICENSE.md · /protomaps/PMTiles/main/LICENSE · /protomaps/protomaps-leaflet/main/LICENSE · /protomaps/go-pmtiles/main/LICENSE
- https://raw.githubusercontent.com/meshtastic/protobufs/master/meshtastic/channel.proto · /meshtastic/firmware/master/src/modules/PositionModule.cpp
- https://raw.githubusercontent.com/termux/termux-api/master/app/src/main/java/com/termux/api/apis/WifiAPI.java , `TelephonyAPI.java`, `README.md`, `gradle.properties`, `app/src/main/AndroidManifest.xml` · /termux/termux-api-package/master/LICENSE · /termux/termux-app/master/gradle.properties
- https://raw.githubusercontent.com/markqvist/Reticulum/master/LICENSE , `docs/source/gettingstartedfast.rst` · /rust-nostr/nostr/master/LICENSE · /n0-computer/iroh/main/LICENSE-MIT · /jech/babeld/master/LICENCE · /deviceplug/btleplug/master/LICENSE.md
- https://crates.io/api/v1/crates/{nostr-sdk,btleplug,bluer,iroh,mdns-sd} · https://pypi.org/pypi/rns/json

Search snippets (not fetched):
- https://meshtastic.org/docs/configuration/radio/channels/ · https://git.quad4.io/RNS-Things/MeshChatX/raw/branch/master/README.md
- https://developer.android.com/reference/android/net/NetworkCapabilities · https://developer.android.com/training/basics/network-ops/data-saver.html
- https://gerrit.omnirom.org/plugins/gitiles/android_system_sepolicy/+/1f7ae8ee3f589aece315dc61662ea95d26786236%5E%21 (netlink restriction)
- https://www.law.cornell.edu/cfr/text/47/97.113 · https://www.ecfr.gov/current/title-47/chapter-I/subchapter-D/part-97/subpart-B/section-97.113
- https://til.simonwillison.net/gis/pmtiles · https://pyviz-tutorial.readthedocs.io/en/latest/protomaps/extract.html
- https://wwwhatsnew.com/2026/04/29/mapas-offline-navegacion-sin-internet-mejores-apps/ · https://packages.debian.org/sk/sid/ppc64/babeld · https://docs.iroh.computer/add-a-relay.md

Denied or empty: github.com HTML, api.github.com, meshtastic.org, docs.rs, reticulum.network, Kiwix.
