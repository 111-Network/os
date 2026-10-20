# Layer 5 research, gap 3: MeshMonitor vs alternatives for Meshtastic presence plus a hosted panel

Scout run 2026-10-14. Tags: **(verified)** = fetched or read from upstream this run; **(derived)** = my inference from verified facts; **(unverified)** = not confirmed. Source code was read from the v4.17.0 tarball (https://codeload.github.com/Yeraze/meshmonitor/tar.gz/refs/tags/v4.17.0); blob links below point at the same files.

## 1. Summary

- **MeshMonitor cannot be the multiplexer for Meshtastic's own web client.** Its Virtual Node speaks raw Meshtastic TCP only (default port 4404). The official web client's browser transports are HTTP, Web Serial and Web Bluetooth. MeshMonitor's server has no `/api/v1/fromradio` or `/toradio` route (verified by grep). So the panel and presence do not share one upstream through it.
- **As a presence source it is strong but heavy.** BSD-3-Clause (LICENSE read). Rich node rows including SNR, hops, lastHeard, position and (in stored rows) publicKey. Also covers MeshCore. But it is a full web app: Node 22+, hard native modules, a 189 MB compressed image, default login `admin`/`changeme`, and it sends admin packets to the node by default. Termux is unverified.
- **A missed candidate solves the contention directly: meshmud** (MIT, Go, about 2.5k lines). One serial or TCP uplink fans out to many TCP downlinks plus an HTTP downlink that serves `/api/v1/toradio` and `/api/v1/fromradio` (verified in source). The web client rides the HTTP downlink and a presence reader rides a TCP downlink.
- **Pick: option D** = static Meshtastic web bundle + meshmud + a small GPL Python presence reader as a separate process. It is B with an adopted shim, so the custom shim code disappears. Risk: meshmud is v1.0.0 (2026-03-20) with one maintainer. Vendor and pin the source, and run a hardware test before relying on it.
- **MeshMonitor stays a good optional service** (its own UI as a separate panel, MeshCore presence) but not the answer to "Meshtastic's own panel".
- **Unverified that matter:** how meshmud replays config to a second client; whether the web release has a `build.tar` asset (asset names did not load); any Termux run of MeshMonitor or meshmud.

## 2. Candidates

| Name | Upstream | License | Lang | Maturity | Fit | Why |
|---|---|---|---|---|---|---|
| MeshMonitor | github.com/Yeraze/meshmonitor | BSD-3-Clause (verified) | TS, Node 22+ | v4.17.0 2026-10-12; 689 stars, 102 forks, 4,333 commits, 21 open issues | adopt as presence/bridge, not as panel mux | Presence, REST v1, MeshCore, Virtual Node (TCP only) |
| meshmud | github.com/Rikonardo/meshmud | MIT (verified, LICENSE) | Go 1.24 | v1.0.0 2026-03-20 | adopt as multiplexer and HTTP shim | Serial/TCP uplink, TCP and HTTP downlinks |
| Meshtastic Web | github.com/meshtastic/web | GPL-3.0-only (verified) | TS/React | v2.7.5 marked latest; assets not listed | adopt as static bundle | The panel; HTTP, Web Serial, Web Bluetooth transports |
| meshtastic-python | github.com/meshtastic/python | GPL-3.0 (lens 2, verified) | Python | 2.7.11 | bridge | Presence reader child (TCP/serial) |
| meshtastic-serial-bridge | github.com/Yeraze/meshtastic-serial-bridge | BSD-3-Clause (verified) | shell + socat | about 47 MB Alpine image | reference | MeshMonitor's own serial path; we would call socat directly |
| meshtastic-websocket-proxy | github.com/liamcottle/meshtastic-websocket-proxy | (unverified) | JS | n/a | reject | WebSocket fan-out; the web client has no WebSocket transport |

## 3. Findings

**MeshMonitor: license and maturity**
1. LICENSE is the standard BSD 3-Clause text, "Copyright (c) 2025, meshmonitor contributors". https://github.com/Yeraze/meshmonitor/blob/main/LICENSE . `package.json` also says BSD-3-Clause.
2. The BSD label covers MeshMonitor's own code only. The build needs the `meshtastic/protobufs` git submodule, which is GPL-3.0 (https://github.com/meshtastic/protobufs/blob/master/LICENSE), and the Docker image installs Python `meshtastic` (GPL-3.0), `meshcore-cli` and Apprise into a venv. https://github.com/Yeraze/meshmonitor/blob/v4.17.0/Dockerfile . Run it as a separate process and carry a GPL source offer for those parts.
3. The optional Reticulum bridge image carries the non-OSI Reticulum License and is deliberately kept out of the main image. https://github.com/Yeraze/meshmonitor/blob/v4.17.0/bridge/NOTICE . We would not ship it.
4. Cadence from the changelog: 4.15.0 on 08-21, 4.16.0 on 09-10, 4.16.1 on 09-18, 4.16.2 on 09-29, 4.17.0 on 10-12, with many release candidates between. https://github.com/Yeraze/meshmonitor/blob/v4.17.0/CHANGELOG.md . Breaking changes land in minor releases: `TRUST_PROXY` default flipped in 4.13, and root-level v1 data paths were removed in 4.14 (https://newreleases.io/project/github/Yeraze/meshmonitor/release/v4.14.0). Stars, forks, issues and commits: https://github.com/Yeraze/meshmonitor .

**MeshMonitor: runtime and footprint**
5. Node `>=22` in `engines`; the image is `node:24.21.0-alpine3.24`. https://github.com/Yeraze/meshmonitor/blob/v4.17.0/package.json . Not Docker-only: bare-metal is documented (Node 22+, git, build tools, Python 3). https://github.com/Yeraze/meshmonitor/blob/v4.17.0/docs/deployment/DEPLOYMENT_GUIDE.md . Image platforms are amd64, arm64 and armv7.
6. Native modules: `better-sqlite3`, `bcrypt` and `re2` are hard dependencies. `re2` is a static import in `src/utils/safeRegex.ts`. `zstd-napi` is lazy and optional (ATAK decode). `serialport` is optional (MeshCore USB). The Dockerfile says `re2` has no prebuilt for Alpine/musl on arm, so it compiles in a builder stage. Our musl and aarch64 targets therefore need a build-time compile, never a runtime download (derived).
7. Compressed image size from the GHCR manifests for tag 4.17.0: about 189.5 MB arm64, 188.2 MB amd64, 20 layers each. https://ghcr.io/v2/yeraze/meshmonitor/manifests/4.17.0 . RAM use is unmeasured (unverified).
8. Termux: `nodejs` 26.4.0 and `nodejs-lts` 24.18.0 exist in termux-packages master (https://raw.githubusercontent.com/termux/termux-packages/master/packages/nodejs-lts/build.sh). MeshMonitor's docs never mention Termux. A third party reports `better-sqlite3` 12.11.1 building on Termux aarch64, with npm running out of memory and pnpm working (https://github.com/diegosouzapw/OmniRoute/issues/8669). `re2` and `bcrypt` on Termux are **(unverified)**.
9. There is no bind-address option; the server calls `app.listen(PORT)`. Loopback-only needs a firewall, a netns, a container, or a patch (derived from `src/server/server.ts`).

**MeshMonitor: connections and the Virtual Node**
10. Meshtastic sources are TCP only, normally to port 4403, including meshtasticd. Serial needs the separate socat container; BLE needs a separate privileged BLE bridge container. MeshMonitor used to poll the node's HTTP API and no longer does. https://github.com/Yeraze/meshmonitor/blob/v4.17.0/docs/configuration/serial-bridge.md , https://github.com/Yeraze/meshmonitor/blob/v4.17.0/docs/architecture/MESHTASTICD_ANALYSIS.md . MeshMonitor holds the one API slot upstream, which is why it ships a Virtual Node.
11. Virtual Node: a TCP proxy for the official iOS and Android apps and the Python CLI, per source, historical default port 4404, no client authentication, admin and NodeInfo packets blocked unless "Allow admin commands" is set, about 240 cached config messages replayed to new clients. The docs say an HTTP ingress "can't carry Virtual Node traffic" because it "speaks the raw Meshtastic TCP protocol". https://github.com/Yeraze/meshmonitor/blob/v4.17.0/docs/configuration/virtual-node.md
12. The web client's packages are `transport-http`, `transport-web-serial` and `transport-web-bluetooth` (plus Node and Deno transports that do not run in a browser). https://github.com/meshtastic/web/tree/main/packages . Its docs list HTTP, Bluetooth and Serial only. https://meshtastic.org/docs/software/web-client/ . **Conclusion (derived): the official web client cannot connect through MeshMonitor's Virtual Node.**
13. MeshCore: USB (companion or repeater) and TCP (companion) sources; the docs call support "new and basic". https://github.com/Yeraze/meshmonitor/blob/v4.17.0/docs/features/meshcore.md . A software receive-only switch exists for MeshCore. For Meshtastic, receive-only means turning the radio's `lora.txEnabled` off, which makes the node vanish from the mesh, so it does not suit presence. https://github.com/Yeraze/meshmonitor/blob/v4.17.0/docs/features/receive-only-mode.md
14. MeshMonitor is not a passive reader. On connect it sends `wantConfigId`, one LoRa-config request and (first connect only) 15 module-config requests, then asks for LocalStats every 15 minutes by default. Auto-traceroute, auto-ack, welcome and announce features are opt-in per the automation docs. https://github.com/Yeraze/meshmonitor/blob/v4.17.0/docs/configuration/node-load.md . Defaults of the traceroute scheduler are **(unverified)**.

**MeshMonitor: REST v1**
15. Auth: `Authorization: Bearer mm_v1_<token>`, tokens generated per user in User Settings, scoped by per-source permissions. https://github.com/Yeraze/meshmonitor/blob/v4.17.0/src/server/routes/v1/openapi.yaml . Also a socket.io live feed, and Swagger UI via `swagger-ui-express`.
16. Endpoints: `/api/v1/sources`, `/sources/{sourceId}/nodes` (params `active`, `sinceDays`), `/nodes/{nodeId}`, `/nodes/{id}/position-history`, `/messages` (GET and POST), `/channels`, `/telemetry`, `/traceroutes`, `/network/topology`, `/packets`, `/status`.
17. Documented Node fields: nodeId, nodeNum, longName, shortName, role, hwModel, latitude, longitude, altitude, hopsAway, lastHeard (unix seconds), firstHeard, snr. The handler spreads the full stored row, whose type also holds `publicKey`, `rssi`, `hasPKC`, battery and channel utilization. https://github.com/Yeraze/meshmonitor/blob/v4.17.0/src/server/routes/v1/nodes.ts , https://github.com/Yeraze/meshmonitor/blob/v4.17.0/src/db/types.ts . So `publicKey` is present in code but absent from the OpenAPI schema, and I did not run a live check (unverified).

**MeshMonitor: offline, framing, proxying**
18. The shipped UI has no CDN links (`index.html` has none, CSP `font-src 'self'`). Default basemaps are online: OSM, CARTO, OpenTopoMap and ArcGIS (listed in `src/server/middleware/dynamicCsp.ts`). Offline the map is blank while the rest works (derived). Custom XYZ raster or vector tile servers are supported but need a separate tile server, and the repo's `tiles/` directory is empty. https://github.com/Yeraze/meshmonitor/blob/v4.17.0/docs/configuration/custom-tile-servers.md
19. Update check polls `api.github.com` (`versionCheckService.ts`); `VERSION_CHECK_DISABLED=true` turns it off. Self-upgrade was removed in 4.13 even though the README still advertises it. https://github.com/Yeraze/meshmonitor/blob/v4.17.0/docs/configuration/auto-upgrade.md , https://github.com/Yeraze/meshmonitor/blob/v4.17.0/docs/configuration/updating.md . Other outbound calls are opt-in features: firmware release lookup on GitHub (trigger unverified), elevation API for the terrain tool, translation, MQTT bridges. Analytics default is none. `Dockerfile.proxy` runs `npx` at start (a runtime download), but it is an optional separate image.
20. Framing: Helmet sends `X-Frame-Options: DENY` by default. Setting `IFRAME_ALLOWED_ORIGINS` (list or `*`) disables it and adds CSP `frame-ancestors 'self' <origins>`. https://github.com/Yeraze/meshmonitor/blob/v4.17.0/src/server/server.ts , https://github.com/Yeraze/meshmonitor/blob/v4.17.0/src/server/middleware/dynamicCsp.ts . The 03 panel proxy rewrite works either way.
21. Proxying: `BASE_URL` sub-path, `TRUST_PROXY` (default false), WebSocket upgrade needed, and header-based proxy auth (`PROXY_AUTH_ENABLED`, defaults include `X-Auth-Request-Email` and `Remote-User`). That fits a 111d session-to-header injection (derived). `COOKIE_SECURE` defaults to false in code (the docs table says true), so plain HTTP works; first login is `admin`/`changeme`. https://github.com/Yeraze/meshmonitor/blob/v4.17.0/src/server/config/environment.ts , https://github.com/Yeraze/meshmonitor/blob/v4.17.0/README.md

**meshmud (option D multiplexer)**
22. MIT, Go 1.24, about 2.5k lines, deps include `go.bug.st/serial`, zeroconf and avahi (mDNS only if `advertise = true`). Single copyright holder. https://github.com/Rikonardo/meshmud , https://pkg.go.dev/github.com/Rikonardo/meshmud
23. Config example in the README: one serial uplink, a TCP downlink on 4403 (multiple clients) and an HTTP downlink on 8000 (one client). The HTTP downlink registers `/api/v1/toradio` (PUT) and `/api/v1/fromradio` with CORS `*`, in `internal/downlink/dhttp/listener.go` lines 58-59 and 85-130 (read from the main tarball). https://github.com/Rikonardo/meshmud/blob/main/README.md
24. Not verified: config replay to a second client (it has `mesh_packet_buffer` replay, not a full config cache), behavior of a second HTTP client, static Go builds for musl and Android, and DTR/RTS handling on serial open (grep for DTR, RTS, HUPCL found none). MeshMonitor's serial bridge disables HUPCL explicitly, which shows the reset risk is real. https://github.com/Yeraze/meshtastic-serial-bridge

**Option B and C facts**
25. The node HTTP API a shim must offer is `PUT /api/v1/toradio` and `GET /api/v1/fromradio?all=true`. https://meshtastic.org/docs/development/device/http-api/ . No ready serial-to-`/api/v1` multi-client shim turned up in search other than meshmud.
26. Web client: HTTP mode works on plain HTTP (no secure context). Web Serial and Bluetooth need a secure context, so on a LAN dashboard they only work on loopback. https://meshtastic.org/docs/software/web-client/ , lens 2 finding 10.

## 4. Comparison and recommendation

| Criterion | A: MeshMonitor | B: web bundle + python sidecar + custom shim | C: loopback Web Serial only | D: B with meshmud as the shim |
|---|---|---|---|---|
| Custom code we write | Supervision, port/auth glue, serial socat child; REST client for Nearby | Shim for `/api/v1` HTTP, fan-out, reader | Panel manifest only | Reader glue (about 100 lines), manifest, meshmud config |
| Shows Meshtastic's own panel | No (MeshMonitor's UI instead) | Yes | Yes | Yes |
| GPL boundary | BSD app; bundles GPL protobufs and python parts; separate process | Python reader and web bundle GPL, separate processes | Web bundle GPL, static | Same as B; meshmud is MIT |
| Offline readiness | UI local; tiles online by default; disable version check; native modules built ahead | Fully local after build | Fully local | Fully local |
| One-client contention solved | Presence yes; web client cannot join the Virtual Node | Yes, by our shim | Not needed; no node-side presence | Yes, by meshmud (needs hardware test) |
| Presence fields | Best: id, names, pubkey (in stored rows), snr, rssi, hops, lastHeard, position, battery, history | Good: from the node DB dump (lens 2 finding 6) | None | Same as B |
| Phone / Termux | Unverified; re2, bcrypt, better-sqlite3 compile | Python on Termux unverified | Browser only | Go binary plausible; unverified |
| Footprint | About 189 MB image, Node runtime | Python plus bundle, shim extra | Bundle only | Go binary plus Python reader; sizes unmeasured |
| Main risk | Heavy, fast-moving, writes to the node, binds all interfaces | We own the shim | Fails the presence requirement | meshmud is new with one maintainer |

**Pick and options**
1. **D (recommended).** Adopt meshmud as the single holder of the serial or :4403 slot. Panel via its HTTP downlink, presence via its TCP downlink. Least custom code of the options that meet all three owner goals. Trade-off: depends on a young MIT project that we should vendor and pin.
2. **A as a service only.** Run MeshMonitor against meshmud's TCP downlink, host its UI as an extra panel, and read its REST API for Nearby. Best presence and MeshCore coverage. Trade-off: large footprint, a second UI, and it is not Meshtastic's own panel.
3. **C.** Loopback Web Serial and no node-side presence. Zero new code. Trade-off: goal 3 (presence in Nearby) is not met.

## 5. Open questions for the owner

1. Is MeshMonitor's own UI acceptable as the Meshtastic panel, or must it be Meshtastic's own client? The second answer rules out A as the panel.
2. Accept adopting meshmud (v1.0.0, one maintainer, MIT) by vendoring and pinning its source, or prefer a thin multiplexer of our own?
3. Is a GPL Python presence reader child acceptable (same as lens 2 question 2)?
4. What does "read-only" mean here: no RF transmission initiated by us, or no packets to the node at all? Every option sends `want_config`; MeshMonitor also sends admin and LocalStats requests unless configured off.
5. Is one HTTP panel client at a time acceptable? meshmud's HTTP downlink allows one.
6. Is MeshCore presence in this cycle (lens 2 question 7)? Only MeshMonitor covers it among these candidates.
7. For USB boards, may the consented identify probe be "start the multiplexer child and read `want_config` from its TCP downlink"? That ties the DTR/RTS reset risk to one consent step.

## 6. Sources

MeshMonitor: https://github.com/Yeraze/meshmonitor · https://github.com/Yeraze/meshmonitor/blob/main/LICENSE · https://codeload.github.com/Yeraze/meshmonitor/tar.gz/refs/tags/v4.17.0 (files cited under /blob/v4.17.0/: Dockerfile, package.json, README.md, CHANGELOG.md, bridge/NOTICE, docs/configuration/{virtual-node,serial-bridge,node-load,custom-tile-servers,auto-upgrade,updating,index}.md, docs/features/{meshcore,receive-only-mode}.md, docs/deployment/DEPLOYMENT_GUIDE.md, docs/architecture/MESHTASTICD_ANALYSIS.md, src/server/{server.ts,config/environment.ts,middleware/dynamicCsp.ts,routes/v1/nodes.ts,routes/v1/openapi.yaml}, src/db/types.ts) · https://ghcr.io/v2/yeraze/meshmonitor/manifests/4.17.0 · https://newreleases.io/project/github/Yeraze/meshmonitor/release/v4.14.0 · https://github.com/Yeraze/meshtastic-serial-bridge
Alternatives: https://github.com/Rikonardo/meshmud · https://pkg.go.dev/github.com/Rikonardo/meshmud · https://github.com/meshtastic/web · https://github.com/meshtastic/web/tree/main/packages · https://github.com/meshtastic/web/releases/latest · https://meshtastic.org/docs/software/web-client/ · https://meshtastic.org/docs/development/device/http-api/ · https://github.com/meshtastic/protobufs/blob/master/LICENSE · https://github.com/liamcottle/meshtastic-websocket-proxy
Termux: https://raw.githubusercontent.com/termux/termux-packages/master/packages/nodejs-lts/build.sh · https://raw.githubusercontent.com/termux/termux-packages/master/packages/nodejs/build.sh · https://github.com/diegosouzapw/OmniRoute/issues/8669
Repo context read: os/plan/layers/5-messaging/research/{06-critic,02-bridges-and-budgets,03-hosted-panels,00-code-seams}.md
