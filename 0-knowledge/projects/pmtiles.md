---
title: PMTiles
category: project
status: candidate
summary: Single-file map tile archive read by HTTP Range; offline labels unverified.
tags: [storage, offline, panel]
platforms: [linux, android]
upstream: https://github.com/protomaps/PMTiles
license: BSD-3-Clause AND BSD-2-Clause
last_verified: 2026-11-02
sources:
  - https://github.com/protomaps/PMTiles
  - https://github.com/protomaps/go-pmtiles
  - https://github.com/protomaps/protomaps-leaflet
  - https://github.com/Leaflet/Leaflet
---

Single-file map tile archive read by HTTP Range; offline labels unverified.

## What it is

PMTiles is a one-file archive of map tiles (spec v3) that a browser reads with HTTP `Range` requests.
Four pieces are adopted together in Layer 8: the format, the go-pmtiles CLI (`pmtiles extract --bbox`, run off-node),
protomaps-leaflet (vector tiles drawn on a canvas) and Leaflet (the map widget).
The node serves the file through a Range route; the browser lazy-loads one map chunk.

## Use when

- A hub should hold a regional map as one file and serve it to the dashboard's Nearby map tab.
- A region must be cut from a Protomaps daily build without downloading the planet.

## Do NOT use when

- Raster tiles or a tile server with z/x/y routes is wanted; that is another design.
- A tileset would be rebuilt from OpenStreetMap data: the ODbL reading of a re-extracted tileset is unsettled.

## Details

Licences as the Layer 8 spec states them: go-pmtiles and the PMTiles code BSD-3-Clause, the spec public domain or CC0,
protomaps-leaflet 5.1.0 BSD-3-Clause (it embeds pmtiles 3.2.1 and the basemap themes), Leaflet 1.9.4 BSD-2-Clause.
Scout measurement with esbuild: Leaflet plus protomaps-leaflet is 80.4 KB gzip, 83.9 KB with CSS.
Tile data from Protomaps is OpenStreetMap-derived (ODbL): the safe posture is unmodified output with the
"(c) OpenStreetMap" notice. Labels need fonts and glyphs that load with no network; unverified from this box.
Alternatives to a Range route exist (Martin, tileserver-gl) but add a server; none is adopted.

## Evidence

### Alternatives considered

- Martin — Rust tile server, MIT OR Apache-2.0; duplicates a Range route and adds a process.
- MapLibre GL — about 450 KB gzip with its worker; too heavy for the lazy chunk budget.
- pmtiles-rs crate — reference only; a 127-byte header parse needs no dependency.
- 2026-11-02 — desk research (Layer 8 reports 01, 03, 05): licences and versions read upstream by the scouts; the entry was written later and nothing was re-read, fetched, built or run on this box.

## Links

- [PMTiles repository](https://github.com/protomaps/PMTiles)
- [go-pmtiles repository](https://github.com/protomaps/go-pmtiles)
- [protomaps-leaflet repository](https://github.com/protomaps/protomaps-leaflet)
- [Leaflet repository](https://github.com/Leaflet/Leaflet)
