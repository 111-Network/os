# Lens 3 — One dashboard that hosts any project's own panel

Scout report · 2026-10-14 · scope: hosting mechanics for third-party web UIs inside the Layer 4 dashboard, plus the panel inventory. No code written, no repo file touched except this report. "(unverified)" marks anything I could not confirm from a fetched source.

## 1. Summary

- **Pick: one origin per panel, reached by port.** `111d` binds one extra listener per enabled panel (loopback always, LAN only when LAN sharing is on) and reverse-proxies it. Same hostname, different port, so a different origin; no path rewriting, no wildcard DNS. Umbrel and CasaOS work this way; Home Assistant Ingress (sub-path) and Sandstorm (wildcard subdomains) do not fit an offline LAN.
- **D15's iframe ban can be narrowed.** Mixed content disappears while everything is plain HTTP; framing headers are solved by a proxy rewrite; secure-context APIs remain a real limit on non-loopback hosts (§3.9). Amendment text: §4.3.
- **Panels should talk to node-side endpoints, not browser-side hardware.** A LAN visitor's Web Serial/Bluetooth reaches the visitor's own ports, and `http://<lan-ip>` is not a secure context. Meshtastic's web client has an HTTP transport. Browser-side serial/BLE is a loopback-only bonus.
- **Inventory.** Adopt: Meshtastic web client (GPL-3.0, static `build.tar`). Bridge: MeshChatX (fallback MeshChat); both need rnsd `share_instance = yes` (open item #4, `devices.md`). Later: MeshCore Open web, `config.meshcore.io`, ttyd. Syncthing GUI only if Layer 8 picks it. Reject: Sideband (CC BY-NC-SA), Delta Chat (dev-only browser edition), Briar (no web UI), `meshcore-web` (superseded).
- **Two corrections.** Sideband is actively released (2.2.0, 2026-10-01), so `devices.md` §0.7 "no longer updated" is stale (verdict unchanged: NC, reject). Chrome 154 makes HTTPS-by-default the norm; private addresses are reportedly excluded at first (secondary source), so D15's no-TLS stands but needs a re-check.
- **Glue stays small:** MIT `axum-reverse-proxy` (or ~50 lines of hyper-util) plus the manifest, a ticket endpoint and the header/cookie rewrite.

## 2. Candidates table (panels and helpers)

License "✔" = the upstream `LICENSE` file was fetched and read; "API" = GitHub license detection only. Dates are from the GitHub API, 2026-10-14.

| Name | Upstream | License | Runtime | Maturity | Fit | Why |
|---|---|---|---|---|---|---|
| Meshtastic Web client | github.com/meshtastic/web | GPL-3.0 ✔ | React/TS static bundle; release asset `build.tar` (2.1 MB) | v2.7.2 2026-08-09; pushed 2026-10-11; 864★; official client | **adopt** | Static files; transports HTTP, Web Serial, Web Bluetooth. Served unmodified, no linking. HTTP mode is "limited to ESP32"; the hosted copy needs HTTPS, ours does not. |
| meshtasticd web UI / ESP32 HTTP API | github.com/meshtastic/firmware | GPL-3.0 (API) | C++; meshtasticd serves UI on HTTPS :9443 (self-signed, `config-dist.yaml`); `/api/v1/fromradio`+`toradio` | v2.7.26 2026-06-24 | **adopt** (as proxy upstream) | Only SPI/CH341 radios and ESP32 WiFi nodes. API sends `Access-Control-Allow-Origin: *`. Proxy to https loopback, verify off. |
| MeshChatX | github.com/Quad4-Software/MeshChatX | 0BSD + MIT (upstream part) ✔ | Python (PyPI wheel with bundled UI), Docker, Termux guide | v4.10.7 2026-10-11; listed in the Reticulum manual's software chapter; fork; no audit known | **bridge (pick)** | Headless, `--no-https`, `--auth`, `--reticulum-config-dir`, proxy-aware WebSocket Origin check, offline-ready map. Own LXMF identity (bridge, not fabric). |
| MeshChat (original) | github.com/liamcottle/reticulum-meshchat | MIT ✔ | Python aiohttp + Vue; `/ws` WebSocket | v2.4.0 2026-07-06; pushed 2026-10-06; 1.3k★ | **bridge (fallback)** | Headless flags exist. No auth/Origin handling in `meshchat.py` (grep). Root-absolute `/api/v1/...`: needs an origin root. Not named in the Reticulum manual. |
| NomadNet | github.com/markqvist/NomadNet | GPL-3.0 ✔ | Python TUI; `--daemon` | 1.4.4 2026-09-30; 2.6k★ | **reference** | No web UI. MeshChat(X) already browses Nomad pages. Only via ttyd if wanted. |
| ttyd | github.com/tsl0922/ttyd | MIT ✔ | C; `-i` unix socket, `-b` base path, `-O` check-origin, read-only unless `-W` | 1.7.7 2024-03-30 (old); pushed 2026-08-12; 12k★ | **bridge (helper)** | Generic "TUI in a panel" host. wetty (MIT, Node, v4.0.1 2026-10-14) is the alternative. |
| Sideband | github.com/markqvist/Sideband | CC BY-NC-SA 4.0 ✔ | Python/Kivy native | 2.2.0 2026-10-01 (active) | **reject** | NC. No web UI. |
| Columba | github.com/torlando-tech/columba | MPL-2.0 (API) | Kotlin Android | v2.2.6 2026-09-25 | **reference** | Native phone app (Layer 6), not a web panel. |
| MeshCore official web app | app.meshcore.nz | (unverified) | Flutter web (`flutter_bootstrap.js`), GitHub Pages | updated 2026-10-14 | **reject** | No source repo found in the `meshcore-dev` org; cannot self-host or verify. |
| MeshCore Open (web build) | github.com/zjs81/meshcore-open | MIT ✔ | Flutter/Dart; web = Web Serial only, Chrome | pushed 2026-10-12; 633★ | **bridge (later)** | Only self-hostable MeshCore client found. Flutter CanvasKit may fetch from a CDN (unverified): must pass the offline lint. |
| MeshCore config web | github.com/meshcore-dev/config.meshcore.io | MIT ✔ | Static web, Web Serial | pushed 2026-09-26 | **bridge (later)** | Repeater/room config. Serial-bound, so loopback-only. |
| `liamcottle/meshcore-web` | github.com/liamcottle/meshcore-web | MIT (API) | Vue | last commit 2025-05-05 "use new web app" | **reject** | Superseded. |
| Syncthing GUI | github.com/syncthing/syncthing | MPL-2.0 ✔ | Go; GUI :8384 or unix socket | v2.1.6 2026-10-06; 89k★ | **adopt if Layer 8 picks it** | Sends `X-Frame-Options: SAMEORIGIN` unless `insecureAllowFrameLoading`; Host check when bound to localhost. |
| Delta Chat | github.com/deltachat/deltachat-desktop | GPL-3.0-or-later (README) | Electron; browser edition | v2.62.0 2026-09-27 | **reject** (as panel) | README: browser edition is "highly experimental… only meant for developers and automated testing". |
| Briar | code.briarproject.org/briar/briar | GPL-3.0 (KB, not re-read) | Android | KB-tracked | **reject** (as panel) | No web UI. A `briar-headless` project was not reachable (unverified). |
| PotatoMesh | github.com/l5yth/potato-mesh | Apache-2.0 ✔ | Ruby/Sinatra + SQLite + Python ingestor | v0.7.5 2026-08-24; pushed 2026-10-14; 522★ | **reference** | Node map for Meshtastic, MeshCore, Reticulum (announce-only). Offline blockers: tiles from two CDNs (https only), `FEDERATION=1` default. A Layer 7 Nearby pattern, not a panel now. |
| Reticulum visualizers | none found | — | — | — | **none** | Searched top GitHub "reticulum" repos. The manual lists RNMon (InfluxDB) and Reticulated (simulator). Not evaluated. |
| axum-reverse-proxy | github.com/tom-lubenow/axum-reverse-proxy | MIT ✔ | Rust crate | 2.2.0 2026-08-29 | **adopt** (glue) | WebSocket upgrades, hop-by-hop stripping, `X-Forwarded-*`. |

## 3. Findings that change the plan

1. **Home Assistant Ingress is a sub-path, same-origin proxy.** Route `/api/hassio_ingress/{token}/{path}`; a session cookie `ingress_session` is checked, the add-on gets `X-Ingress-Path`, only the proxy's address may connect, and both HTTP and WebSocket (16 MiB frames) are bridged. Response headers pass through except length/encoding/type, so an add-on's own `X-Frame-Options` would survive. Add-ons must therefore work under a prefix. https://developers.home-assistant.io/docs/add-ons/presentation/ · https://github.com/home-assistant/supervisor/blob/main/supervisor/api/ingress.py · https://github.com/home-assistant/core/blob/dev/homeassistant/components/hassio/ingress.py
2. **Sub-path hosting gives the hosted app the dashboard's origin.** Sandstorm's design notes: path-based URLs are "ruled out by the same-origin policy"; every app needs its own host, with per-session random hostnames and a cookie-checked proxy. Under a shared origin, third-party panel JS could call `/api/v1` with our cookie; Home Assistant accepts that because add-ons are trusted, adopted projects are not. https://raw.githubusercontent.com/sandstorm-io/sandstorm/master/docs/administering/wildcard.md
3. **Umbrel and CasaOS use one port per app and open the app on that port.** Umbrel: manifest `port` is unique across the store, `path` is the entry path, `app_proxy` adds shared login (`PROXY_AUTH_ADD/WHITELIST/BLACKLIST`); users reach the app via any hostname or IP. CasaOS: `x-casaos` has `index` (entry path) and `port_map`. Umbrel's own UI refuses to be framed (`iframe-checker.tsx`), and a code search found no app iframe in `packages/ui` (only the PDF viewer and init). https://github.com/getumbrel/umbrel-apps/blob/master/.agents/skills/umbrel-package-app/SKILL.md · https://github.com/getumbrel/umbrel/blob/master/packages/ui/src/components/iframe-checker.tsx · https://github.com/IceWhaleTech/CasaOS-AppStore/blob/main/Apps/Jellyfin/docker-compose.yml
4. **YunoHost SSOwat is an auth gateway, not an embedder.** One signed JWT cookie (`cookie_name`), per-app `permissions` with URL lists and `auth_header`, a portal that redirects. https://github.com/YunoHost/SSOwat/blob/dev/README.md
5. **Well-built apps ship anti-framing defaults with opt-outs; the proxy should not depend on them.** Grafana `allow_embedding=false` sends `X-Frame-Options: deny`, `cookie_samesite=lax`, `serve_from_sub_path` off. Syncthing sets `X-Frame-Options: SAMEORIGIN` unless `insecureAllowFrameLoading`, and checks Host when bound to localhost (`insecureSkipHostCheck`). MeshChatX drops `X-Frame-Options` and sends `frame-ancestors` only in demo mode. https://grafana.com/docs/grafana/latest/setup-grafana/configure-grafana/ · https://github.com/syncthing/syncthing/blob/main/lib/api/api.go · https://github.com/Quad4-Software/MeshChatX/blob/main/docs/en/installation.md
6. **`frame-ancestors` obsoletes `X-Frame-Options`, but only the header you leave in place counts.** CSP2 §7.7.1: when both are present, enforce `frame-ancestors`, ignore XFO. It is not covered by `default-src`, ignores `<meta>`, and checks every ancestor. Mechanic: drop XFO, remove any upstream `frame-ancestors`, append our own CSP header carrying only `frame-ancestors`. https://www.w3.org/TR/CSP2/#frame-ancestors-and-frame-options · https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/frame-ancestors
7. **`*.localhost` is not universal.** RFC 6761 §6.3: resolvers SHOULD return loopback, applications MAY special-case. Chromium treats it as localhost (`IsLocalHostname` in `url_util.cc`); Firefox 84+ hardcodes it (bug 1220810); Safari/macOS failed through 15.7 and works only on macOS 26 per a September 2025 report (WebKit bug 160504). iOS: (unverified). Secure Contexts lists `localhost` and `*.localhost` as potentially trustworthy only if name resolution is loopback-pinned; `127.0.0.0/8` and `::1` literals are unconditional. https://www.rfc-editor.org/rfc/rfc6761.html#section-6.3 · https://bugzilla.mozilla.org/show_bug.cgi?id=1220810 · https://bugs.webkit.org/show_bug.cgi?id=160504 · https://github.com/chromium/chromium/blob/main/net/base/url_util.cc · https://w3c.github.io/webappsec-secure-contexts/
8. **mDNS cannot give per-panel names.** It has no wildcards (each name needs its own announce); nss-mdns' built-in heuristic accepts only two-label names (`host.local`) unless `/etc/mdns.allow` says otherwise; D16(e) already records that Android does not resolve `.local`. `panel.111.local` is therefore not viable; per-panel `.local` names would also be unreachable from Android. https://github.com/avahi/nss-mdns
9. **Secure-context rule, exact form.** A framed document is a secure context only if its own origin is potentially trustworthy AND every ancestor is. So `http://127.0.0.1`, `http://[::1]`, `http://localhost`, `http://*.localhost` (conforming browsers) count; `http://111.local` and `http://192.168.x.x` do not, nor frames inside them. Web Serial, Web Bluetooth and WebUSB need a secure context, and a cross-origin frame needs the iframe `allow="serial"` / `"bluetooth"` / `"usb"` delegation (default allowlist is `self`). Web Serial is Chromium-only on desktop; Chrome for Android reportedly gained it in 148 (Bluetooth RFCOMM per the blink-dev intent; USB unclear). Web Bluetooth: Chromium only, none on iOS. https://developer.mozilla.org/en-US/docs/Web/Security/Secure_Contexts · https://developer.mozilla.org/en-US/docs/Web/API/Web_Serial_API · https://developer.mozilla.org/en-US/docs/Web/API/Web_Bluetooth_API · https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Permissions-Policy/serial · https://www.notebookcheck.org/Chrome-148-Beta-para-Android-anade-soporte-para-Web-Serial-y-SharedWorker.1269767.0.html (secondary) · https://groups.google.com/a/chromium.org/g/blink-dev/c/BqUGCcurReE/m/XbuAYkRxEQAJ
10. **Cookies are shared across ports; storage is not.** RFC 6265 §8.5: no port isolation (read or write). Origin-scoped storage (localStorage, IndexedDB, service workers) is per host+port. A sibling subdomain can set a cookie for its parent (§8.6). In the port model `s111` rides to every panel port on that host and panels can plant or shadow cookies. Existing hardening covers part of it (HttpOnly, every `s111` copy tried, JSON-only + Origin==Host). Add: strip our cookies upstream, drop `Domain=` from upstream `Set-Cookie`, check `Sec-Fetch-Site`/Origin on panel mutations. Do not rewrite cookie names (apps reading CSRF cookies in JS would break; Syncthing likely, unverified). https://www.rfc-editor.org/rfc/rfc6265.html#section-8.5
11. **MeshChat has no access control; MeshChatX has the hooks a proxy needs.** In `meshchat.py` the server binds `127.0.0.1:8000` by default, serves static files at `/`, exposes `/ws`, and has no password/token/Origin code (grep). The frontend calls root-absolute `/api/v1/...` with hash routing, so it cannot live under a sub-path. MeshChatX adds `--auth`, `--no-https`, a same-authority WebSocket Origin check (`/ws`, `/ws/telephone/audio`) that needs `MESHCHAT_TRUSTED_PROXIES` plus `X-Forwarded-Host/Proto/Port` behind a proxy, and Landlock/seccomp sandboxes. Audio calls need `localhost` or https (AudioWorklet). https://github.com/liamcottle/reticulum-meshchat · https://github.com/Quad4-Software/MeshChatX/blob/main/docs/en/installation.md
12. **Reticulum clients embed Reticulum and attach as a shared-instance client.** The first program creates the shared instance, later ones join over a local socket (`share_instance = Yes` is the stock default; ours is `no`). MeshChat calls `RNS.Reticulum(config_dir)` in-process. This fits hard rule 1 (separate process, unmodified upstream, nothing in our tree), but it needs the `share_instance` posture change, and each client carries its own identity. The manual's software chapter lists MeshChatX, NomadNet, Sideband, Columba. https://reticulum.network/manual/using.html · https://reticulum.network/manual/software.html
13. **Meshtastic's web client does not need a browser-side radio.** Modes are HTTP (ESP32 only), Web Serial ("least platform support"), Web Bluetooth. The hosted client demands HTTPS and a trusted self-signed node cert; plain-HTTP hosting on our side removes that friction. The ESP32 firmware HTTP API (`/api/v1/toradio`, `/fromradio`) replies with `Access-Control-Allow-Origin: *`; meshtasticd's `Webserver` block defaults to port 9443 with generated TLS. https://meshtastic.org/docs/software/web-client/ · https://github.com/meshtastic/web · https://github.com/meshtastic/firmware/blob/master/src/mesh/http/ContentHandler.cpp · https://github.com/meshtastic/firmware/blob/master/bin/config-dist.yaml
14. **Two browser-policy risks to the no-TLS stance.** Chrome Local Network Access prompts launched in Chrome 142 for public-to-local requests, with all cross-origin local-network requests planned later (secure contexts only), so panel-to-device calls should go through `111d`, not direct cross-origin. Chrome 154 (October 2026) enables "Always Use Secure Connections" by default; secondary reports exclude private addresses initially; `localhost`/`.local` treatment (unverified). https://developer.chrome.com/blog/local-network-access · https://cyberinsider.com/chrome-will-default-to-https-for-all-public-sites-starting-october-2026/
15. **Current L4 code assumes no framing and one CSP.** `routes.rs` sets `default-src 'self'; … frame-ancestors 'none'` on every response when absent, and tests assert `X-Frame-Options: DENY`. `default-src 'self'` also covers `frame-src`. `ServiceRecord.embed_url` already exists in the OpenAPI (string). The panel proxy must be exempt from that middleware and the dashboard CSP gains an exact `frame-src` list.

## 4. Recommendation

### 4.1 Where a real fork exists

| # | Fork | Options (one-line trade-off) | Pick |
|---|---|---|---|
| 1 | Hosting model | **A** link-out only (zero risk, not "hosted"). **B** HA-style sub-path (needs app cooperation, shares dashboard origin; breaks MeshChat, Flutter apps). **C** port per panel (works for any app, any host form, LAN-safe; more listeners, cookie jar shared across ports). **D** `<id>.111.localhost` on one listener (separate cookie jars; unreliable on Safari <macOS 26, useless on LAN, still needs C there). | **C**; add D later as an additive prettification for the node's own browser. |
| 2 | Panel data path | **Node-side** (proxy to meshtasticd / ESP32 HTTP / app backends) vs **browser-side** (Web Serial/BLE from the panel). | Node-side first; browser-side only where `needs_secure_context` is satisfied (loopback), e.g. Chrome on the same phone for BLE. |
| 3 | Reticulum client panel | MeshChatX (auth, proxy-aware, PyPI wheel, manual-listed fork) vs MeshChat (original, MIT, simpler, no auth). | MeshChatX; keep MeshChat as fallback; the review panel checks the fork's provenance. |
| 4 | TUI-only apps | ttyd panel vs not offering them. | Not now. Offer ttyd only if the owner wants NomadNet in the UI. |

### 4.2 Pinned mechanics for `111d` (axum/hyper)

1. **Listeners.** Reserve a block above 11180/11181/11182, e.g. 11190–11229; one listener per enabled panel, slot stable in `state/panels.json`, bound where the dashboard is bound. Address: `http://<dashboard-host>:<panel-port>` (host as the user typed it).
2. **Upstream kinds.** `static` (hash-pinned bundle in `<root>/panels/<id>/`, SPA fallback; Meshtastic web); `child` (supervised, behind loopback port or unix socket; MeshChatX, Syncthing, ttyd); `detected` (external; meshtasticd). Bundles ride the hash-locked install path: no runtime downloads.
3. **Auth (ticket, like HA's session endpoint).** The dashboard calls `POST /api/v1/panels/{id}/session` and gets a one-time ticket URL (128-bit, ~30 s). The panel listener swaps it for host-only cookie `p111_<id>` (HttpOnly, SameSite=Strict, no `Domain`) and redirects to the entry path. Skipped only where `loopback_trusted` (D16a: not Termux). 111d strips `s111`/`p111_*` upstream and adds `X-Forwarded-*`.
4. **Request guards on every panel listener.** Exact Host match; for non-GET, `Sec-Fetch-Site` same-origin (or Origin host == Host), so a drive-by page cannot blind-POST into an app with no CSRF protection. This is where MeshChat's missing access control is supplied.
5. **Response rewrite.** Drop `X-Frame-Options`. Parse every `Content-Security-Policy[-Report-Only]`, delete its `frame-ancestors`, then append `Content-Security-Policy: frame-ancestors <dashboard origins>`, where the dashboard origins are this host on the dashboard ports (11180 and 80). Strip `Domain=` from `Set-Cookie`. Pass every other upstream header through. Never apply the dashboard's CSP to panel responses. Never rewrite bodies.
6. **Streaming.** Forward WebSocket upgrades, SSE (no buffering), large uploads (MeshChat accepts 50 MB). Manifest `host_header = preserve|upstream`: `preserve` satisfies MeshChatX's Origin check; otherwise set `MESHCHAT_TRUSTED_PROXIES=127.0.0.1` and send `X-Forwarded-*`.
7. **Dashboard side.** LIVE gains a Panel tab: `<iframe sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-modals allow-downloads" allow="…from manifest">` (no `allow-top-navigation`), "Open full client" kept beside it. Whether `sandbox` blocks serial/Bluetooth delegation is unverified, hence manifest `sandbox = "default"|"none"`. Dashboard CSP keeps `frame-ancestors 'none'` and adds `frame-src` with exact panel origins.
8. **Secure-context gate.** If `needs_secure_context = true` and the dashboard origin is not potentially trustworthy (LAN IP, `111.local`), do not frame: say "open the dashboard on the node itself" and link the loopback URL. `127.0.0.1` is the safe default.
9. **postMessage.** Adopted UIs will not speak a 111 protocol and a parent cannot restyle a cross-origin app, so do not promise theming. Frame chrome stays ours (title, health dot, reload, open-in-tab, "ask the agent about this panel"). Reserve `111-panel/1` (`ready`, `title`, `theme`) for panels we write later, always checking `event.origin`.
10. **Offline lint.** Each panel must pass a verify run in a no-internet netns; any CDN fetch fails adoption.
11. **Health.** Reuse the 2-tick debounce; manifest `health` is a GET expecting 2xx/3xx or a TCP connect.

### 4.3 Amendment text proposal for D15 (proposed D18, "Hosted panels")

> **Changes.** The D15 rejection "iframe-embedding third-party web UIs (mixed content, framing headers, secure-context APIs — link out instead)" narrows to: *no iframe of any URL that is not a registry-declared panel.* A panel is declared in `assets/panels/<id>.toml`, runs on its own origin (its own port) behind `111d`'s panel proxy, and is framed only by the dashboard. §6.6: a LIVE service page shows the native control panel and a Panel tab; "Open full client" stays as the escape hatch. `ServiceRecord.embed_url` is superseded by an additive `panel` object; `ServiceId` gains one id per hosted project. §11: unchanged, plus `POST /api/v1/panels/{id}/session` and per-panel cookie `p111_<id>`.
> **Stays.** Single origin for our own UI and `/api/v1`; `frame-ancestors 'none'` and `X-Frame-Options: DENY` on dashboard responses; no runtime fetch of anything; no TLS in v1; loopback-trust policy (D16a); the Host guard on the main listener; "everything agentic" (the agent installs and diagnoses panels through typed actions, no per-panel code paths).
> **Reasons now answered.** Mixed content: all origins plain HTTP; if TLS is ever enabled, every panel listener must use it. Framing headers: proxy rewrite (XFO dropped, `frame-ancestors` replaced). Secure-context APIs: *not* universally solvable; panels flagged `needs_secure_context` are framed only on a potentially-trustworthy dashboard origin, and the UI says so on LAN.
> **Rejected.** Sub-path hosting under the dashboard origin (shares authority with third-party JS); body rewriting; wildcard DNS or `.local` per-panel names; any panel that fetches from the internet at runtime.

### 4.4 Manifest shape (per panel; proposal)

```toml
id = "meshchat"                   # also the ServiceId and KB link
title = "MeshChat"
kb_entry = "projects/reticulum-meshchatx"
[source]   kind = "child"         # static | child | detected
           cmd  = ["meshchatx","--headless","--no-https","--auth","--host","127.0.0.1","--port","{port}",
                   "--reticulum-config-dir","{root}/rns"]
           upstream = "http://127.0.0.1:{port}"   # or "unix:/run/111/meshchat.sock", "https://127.0.0.1:9443"
           tls_verify = false     # loopback self-signed upstream only
[entry]    path = "/"             # may carry a #/hash route
[health]   url = "/"  expect = "2xx,3xx"          # or tcp = true
[frame]    allow = ["microphone"] sandbox = "default"
           needs_secure_context = true
           secure_reason = "AudioWorklet calls"
[proxy]    websocket = true  sse = false  host_header = "preserve"  max_body = "50MiB"
           forwarded_env = { MESHCHAT_TRUSTED_PROXIES = "127.0.0.1" }
[auth]     mode = "111-session"   # 111-session | app-native | loopback-only
[requires] rns_shared_instance = true             # drives the rnsd posture setting
[license]  spdx = "0BSD AND MIT"  adopt = "bridge"
```

Meshtastic: `kind = "static"` with a pinned `build.tar`, `[[proxy.routes]] path="/api/" upstream="https://127.0.0.1:9443"` for meshtasticd, `allow = ["serial","bluetooth"]`, `needs_secure_context = true` (the HTTP mode alone does not).

### 4.5 Sequence for this cycle (cost-aware)

Meshtastic panel first (matches the owner's scope): static bundle, proxy, ticket, CSP/XFO rewrite, netns lint. MeshChatX second, proving WebSocket and `share_instance`. A sonnet drafter writes the manifest schema and the additive OpenAPI diff; one bounded Rust task for the proxy/cookie/CSP logic; then a fable review of ticket flow, cookies and the CSRF guard.

## 5. Open questions for the owner

1. Approve narrowing D15 as in §4.3, with **port-per-panel as the one v1 mechanism** (and `<id>.111.localhost` deferred)?
2. Reticulum client panels install Python `rns` from PyPI into their own venv and embed it in-process. Acceptable under the "connected service, unmodified upstream" rule, and does `LICENSING.md` need a line for it?
3. Is a second LXMF identity per client (MeshChat's, not the 111 NodeId) acceptable, i.e. these panels are bridge targets and not the messaging fabric?
4. Should `share_instance = yes` become part of the rnsd posture default whenever a Reticulum client panel is enabled (open item #4 in `devices.md`)?
5. USB-attached Meshtastic boards have no node-side API. Is "Web Serial from Chromium on the node's own machine" acceptable this cycle, with LAN visitors shown an explanation?
6. Is Chrome-on-the-same-phone Web Bluetooth an acceptable interim for "BLE later" (tab-bound, no node-resident presence)?
7. Which panels beyond Meshtastic ship in this cycle: MeshChatX, Syncthing (only if Layer 8 adopts it), a ttyd/NomadNet panel?
8. MeshChatX is manual-listed but unaudited; accept it, or prefer the original MIT MeshChat?
9. Chrome 154 HTTPS-by-default: keep D15's "no TLS in v1" as is, or schedule the LAN-TLS hardening earlier?

## 6. Sources

Repo: `os/README.md` · `os/interface/README.md` §1, §6.6, §7, §11 · `os/plan/DECISIONS.md` D15–D17 · `os/plan/devices.md` · `os/LICENSING.md` · `node111/src/daemon/dashboard/routes.rs` · `interface/api/openapi.yaml` · KB `briar-bramble`, `meshtastic`, `meshcore`.

- https://developers.home-assistant.io/docs/add-ons/presentation/
- https://developers.home-assistant.io/docs/add-ons/configuration/
- https://github.com/home-assistant/supervisor/blob/main/supervisor/api/ingress.py
- https://github.com/home-assistant/core/blob/dev/homeassistant/components/hassio/ingress.py
- https://raw.githubusercontent.com/sandstorm-io/sandstorm/master/docs/administering/wildcard.md
- https://github.com/getumbrel/umbrel-apps/blob/master/.agents/skills/umbrel-package-app/SKILL.md
- https://github.com/getumbrel/umbrel/blob/master/packages/ui/src/components/iframe-checker.tsx
- https://github.com/IceWhaleTech/CasaOS-AppStore/blob/main/Apps/Jellyfin/docker-compose.yml
- https://github.com/YunoHost/SSOwat/blob/dev/README.md
- https://grafana.com/docs/grafana/latest/setup-grafana/configure-grafana/
- https://github.com/syncthing/syncthing/blob/main/lib/api/api.go · https://docs.syncthing.net/users/reverseproxy.html
- https://www.rfc-editor.org/rfc/rfc6761.html#section-6.3 · https://www.rfc-editor.org/rfc/rfc6265.html#section-8.5
- https://w3c.github.io/webappsec-secure-contexts/ · https://developer.mozilla.org/en-US/docs/Web/Security/Secure_Contexts
- https://bugzilla.mozilla.org/show_bug.cgi?id=1220810 · https://bugs.webkit.org/show_bug.cgi?id=160504 · https://github.com/chromium/chromium/blob/main/net/base/url_util.cc
- https://www.w3.org/TR/CSP2/#frame-ancestors-and-frame-options · https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/frame-ancestors
- https://developer.mozilla.org/en-US/docs/Web/API/Web_Serial_API · https://developer.mozilla.org/en-US/docs/Web/API/Web_Bluetooth_API · https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Permissions-Policy/serial
- https://www.notebookcheck.org/Chrome-148-Beta-para-Android-anade-soporte-para-Web-Serial-y-SharedWorker.1269767.0.html · https://groups.google.com/a/chromium.org/g/blink-dev/c/BqUGCcurReE/m/XbuAYkRxEQAJ
- https://developer.chrome.com/blog/local-network-access · https://cyberinsider.com/chrome-will-default-to-https-for-all-public-sites-starting-october-2026/
- https://github.com/avahi/nss-mdns
- https://reticulum.network/manual/using.html · https://reticulum.network/manual/software.html
- https://github.com/meshtastic/web · https://meshtastic.org/docs/software/web-client/ · https://github.com/meshtastic/firmware/blob/master/src/mesh/http/ContentHandler.cpp · https://github.com/meshtastic/firmware/blob/master/bin/config-dist.yaml
- https://github.com/liamcottle/reticulum-meshchat · https://github.com/Quad4-Software/MeshChatX · https://github.com/markqvist/NomadNet · https://github.com/markqvist/Sideband · https://github.com/torlando-tech/columba
- https://github.com/meshcore-dev/MeshCore · https://app.meshcore.nz · https://github.com/zjs81/meshcore-open · https://github.com/meshcore-dev/config.meshcore.io · https://github.com/liamcottle/meshcore-web
- https://github.com/deltachat/deltachat-desktop · https://code.briarproject.org/briar/briar · https://github.com/l5yth/potato-mesh
- https://github.com/tsl0922/ttyd · https://github.com/butlerx/wetty · https://github.com/tom-lubenow/axum-reverse-proxy
