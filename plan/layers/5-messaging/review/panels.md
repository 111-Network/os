# Layer 5 review — lens: hosted-panel proxy security + Meshtastic integration

Judge: fable · 2026-10-14 · spec `os/50-messaging/README.md` v1 draft §13–§15, §18.7–8, §20 G3 · read: D15–D19, `40-interface/README.md` §6.6/§7/§11, `routes.rs`, `api_v1/auth.rs`, `layers/50-messaging/research/00-code-seams.md`, research 03 and 07. "verified" = a real check; "reasoned" = argued from quoted text and sources.

## 1. Verdict: **fix-then-lock**

1. The architecture (one origin per panel by port, ticket → host-only cookie, `frame-ancestors` rewrite, closed dashboard CSP, GPL in separate processes) is right and implements D19 d/g.
2. One blocking hole: forwarded `Access-Control-Allow-Origin: *`, unguarded GETs and the loopback ticket skip let a drive-by page read and drain the user's radio stream on every default desktop node (P1). The fix is three lines of policy.
3. The guards sit on the proxy, but what they protect (meshmud's HTTP downlink on 127.0.0.1:11231) is reachable around it (P2).
4. Ticket/cookie lifecycle, the offline lint and the one-HTTP-client limit are slogans, not behaviour (P4, P5, P8).
5. Two target claims are not honest as written: Termux cannot open USB serial without root (P6); the "offline" panel does fetch from the internet (P5).

## 2. Findings

| id | severity | section | title |
|---|---|---|---|
| P1 | blocking | §13 Auth/Guards/Rewrite, §18.7 | CORS passthrough + unguarded GET + loopback skip = drive-by read of the radio |
| P2 | major | §14 One API client, §2 ports | meshmud's HTTP downlink is reachable around the proxy (DNS rebinding, any local app) |
| P3 | major | §13 Response rewrite | Upstream `Set-Cookie` can overwrite `s111` / `p111_*` |
| P4 | major | §13 Auth (ticket) | Ticket/cookie lifecycle undefined (spent ticket, reload, revocation, lifetime, logs) |
| P5 | major | §13 Offline lint, §14 GPL boundary | Lint unspecified; the Meshtastic client does fetch at runtime |
| P6 | major | §14 One API client, §20 | Termux cannot open serial without root; phone path is TCP-to-board |
| P7 | major | §14 Presence reader, §17 | Presence file carries RF-origin strings: unbounded, unsanitised, straight into the agent |
| P8 | major | §14 "one client", §13 Dashboard side | Panel tab + "Open full client" + LAN viewers break the one-HTTP-client model |
| P9 | major | §13 Contract | `panel.origin` cannot live in a host-independent SSE snapshot |
| P10 | minor | §13 Guards | Origin must match the full authority; WebSocket upgrades and cross-site GETs unguarded |
| P11 | minor | §13 Rewrite/Dashboard side | Per-request `frame-src`; missing nosniff/referrer/hop-by-hop/timeouts; SPA fallback vs API errors |
| P12 | minor | §13 Declaration | The manifest snippet is not valid TOML |
| P13 | minor | §13 Secure-context gate | Web Serial copy ignores that meshmud owns the port; HTTP connect dialog unaddressed |
| P14 | minor | §15 | `409 busy` has no `Problem.code`; identify-profile → service meshmud handoff unspecified |
| P15 | minor | §14, D16(a) | Under Termux every app reaches meshmud's downlinks: say so |
| P16 | minor | §14 Honest copy, §18.8 | "Read-only" needs one sentence: the panel transmits |
| P17 | minor | §14 Config rendered by 111d | Validate the port path; pin `advertise = false` |

### P1 — CORS passthrough + unguarded GET + loopback skip (blocking; verified against code + 07)

Quoted: "`Sec-Fetch-Site` … **for non-GET**" · "pass everything else through" · "Skipped only where `loopback_trusted`" · 07 §23: the HTTP downlink registers `/api/v1/toradio` (PUT) and `/api/v1/fromradio` "with CORS `*`" · 07 §25: the API is `GET /api/v1/fromradio?all=true`.

Attack: default desktop node (loopback trusted, D16 a), board bound. Any web page the user visits loops `fetch("http://127.0.0.1:11190/api/v1/fromradio?all=true")`. GET is unguarded, no cookie is needed (ticket skipped on loopback), and the forwarded `Access-Control-Allow-Origin: *` makes the response readable: the page **reads** the FromRadio stream (node DB, positions, every decoded channel message) and each GET **dequeues** a packet the real panel then misses. Nothing on the browser floor stops it (Chrome's local-network prompt starts at 142; 127.0.0.1 is exempt from mixed-content blocking; Firefox/Safari have no gate). The dashboard shares the exposure but emits no CORS headers and has no state-changing GET; the panel inherits neither. `/toradio` survives only because PUT needs a preflight that hits the non-GET guard.

Fix, all three: (a) strip every `Access-Control-*` header from panel responses and never answer preflights (a panel's JS is same-origin with it); (b) guard **every** request: refuse `Sec-Fetch-Site: cross-site` for any method and any non-`navigate` mode from another origin; (c) delete the loopback skip — the ticket→cookie flow runs everywhere (minting is already free on trusted loopback), so `SameSite=Strict` withholds the cookie from cross-site fetches on browsers without `Sec-Fetch-*`. G3 gains: a cross-site GET of `/fromradio` from a second netns origin returns no body and dequeues nothing.

### P2 — meshmud's HTTP downlink reachable around the proxy (major; reasoned, 07 §23)

Quoted: "HTTP `127.0.0.1:11231` (**one** client: the panel)" · "Guards on every panel request: exact Host match".

The Host guard on :11190 protects nothing a browser can reach on :11231. DNS rebinding: `evil.com` resolves to 127.0.0.1 after the page loads; `fetch("http://evil.com:11231/api/v1/fromradio")` is same-origin for that page and meshmud checks no Host (07 found none); `PUT /toradio` the same — an internet page transmits on the user's radio. The precedent (:11181, :11182) does not radiate. Pick at build and spec it: (i) 111d skips the HTTP downlink and serves `/api/v1/toradio|fromradio` itself by reframing bodies onto a meshmud **TCP** downlink (the same ToRadio/FromRadio bytes behind a 4-byte `0x94 0xC3 len` header — ~100 lines, no protobuf decode, no GPL); this also dissolves P8 and leaves meshmud no browser-speakable listener; or (ii) keep the HTTP downlink only on a unix socket (unverified) or with a pinned upstream Host-check patch. If neither, record the rebinding residual.

### P3 — Upstream `Set-Cookie` can overwrite `s111` / `p111_*` (major; reasoned, RFC 6265 §5.3)

Quoted: "strip `Domain=` from upstream `Set-Cookie`; pass everything else through".

A panel upstream (compromised bundle, a future `child` panel) answers `Set-Cookie: s111=…; HttpOnly; Path=/`: same name, host and path replace the dashboard's cookie — logout or fixation; `p111_<other>` likewise. `document.cookie` cannot replace HttpOnly cookies; an HTTP `Set-Cookie` through our proxy can. Fix: drop upstream `Set-Cookie` named `s111` or `p111_*`; each listener reads `p111_<id>` by exact name and tries every copy (D16 f), since a non-HttpOnly shadow with another `Path` can still be planted by JS.

### P4 — Ticket/cookie lifecycle undefined (major; reasoned)

Quoted: "one-time ticket URL (128-bit, single use, ~30 s). The panel listener swaps it … and redirects to `entry.path`" · "frame chrome ours (title, health dot, reload…)".

Unstated: the ticket URL enters session history and React remounts the frame on tab switch, so reload/back re-requests a spent ticket — answered how? Missing: cookie lifetime, server-side record, revocation on rotate, panel disable and LAN-off, peer binding, log scrubbing. Fix text: "A request with a valid `p111_<id>` cookie ignores any ticket parameter and redirects to `entry.path`; a spent/expired ticket without a cookie answers 403 `panel_ticket_expired` and the dashboard re-mints. Tickets and cookies are bound to the minting session (peer, session hash); `p111_*` are session cookies recorded in `state/panel-sessions.json` (0600); rotate, disable and LAN-off revoke them; the ticket query is redacted from logs."

### P5 — Offline lint unspecified; the client fetches at runtime (major; reasoned, verify at G3)

Quoted: "every panel runs in a no-internet netns; any outbound fetch fails adoption" · §1 "no runtime downloads · no telemetry".

A netns observes nothing by itself: fetches fail and the page still "works". And upstream's client does reach out — at least map tiles (OpenFreeMap) and the device-hardware list (`api.meshtastic.org`). Literally, the lint fails the only panel; loosely, it checks nothing. Fix: lint = headless Chrome over CDP (the I3 harness) in the netns, recording every `Network.requestWillBeSent` off the panel/dashboard origins; the manifest declares `[offline] optional_hosts`, undeclared fails; the UI shows "offline: map blank"; the service page says an internet-connected browser will contact those hosts (the honest "no telemetry"). Also: 07 could not confirm a `build.tar` asset — add "CI-built from the pinned tag when no asset exists"; the GPL offer names the exact tag and we mirror the Corresponding Source.

### P6 — Termux cannot open serial without root (major; reasoned, Android constraint)

Quoted: "uplink = the board's serial port (USB boards) or `tcp://127.0.0.1:4403`" · §20 "Termux build of meshmud + the presence venv".

On Android `/dev/ttyUSB*` is unreadable by an unprivileged app; Termux reaches USB only via `termux-usb`, which hands a file descriptor to a child, while meshmud opens a **path** (`go.bug.st/serial`). A Termux build proves nothing for USB boards; BLE is out too (no BlueZ). The honest phone path is meshmud's TCP uplink to a WiFi board (`tcp://<board-ip>:4403`), unlisted in §14, plus Web Bluetooth from Chrome on the phone (03 §3.9). Fix: add the TCP-to-board uplink as an `interfaces.toml` binding; state "USB-serial boards: Linux only; phones: WiFi board over TCP, or Bluetooth from the phone's browser"; reword the §20 gate.

### P7 — Presence file: RF-origin strings into the agent (major; reasoned)

Quoted: "writes `fabric/presence/meshtastic.json` atomically: per node `{id, num, long_name, short_name, …}`" · "`111d` reads the file on change" · §17 `nearby.read`.

`long_name`/`short_name` arrive over the air, unauthenticated; anyone in LoRa range chooses them. They flow into SSE `nearby`, the Nearby view, logs and the agent's `nearby.read` — an over-the-air prompt-injection channel. A compromised same-uid child can make the file a symlink or 2 GB. Fix: reader truncates names to 40 bytes and strips control characters; daemon opens `O_NOFOLLOW`, requires a regular file ≤ 1 MiB, validates a versioned schema (`schema: 1`, ≤ 4096 nodes, typed fields; invalid ⇒ previous snapshot kept + log counter); `nearby.read` returns names in a labelled untrusted block; the UI renders text only.

### P8 — One HTTP client vs three ways to open the panel (major; reasoned, 07 §23/§25)

Quoted: "HTTP `127.0.0.1:11231` (**one** client: the panel)" · "Panel tab … 'Open full client' in a new tab" · listeners "LAN when LAN sharing is on".

`fromradio` is a stateless GET over one queue: two pollers each get half the packets, and the spec invites that (iframe + new tab; laptop + phone on LAN). Fix: one live `p111_meshtastic` session at a time — a new ticket revokes the previous one and the old frame shows "opened elsewhere"; "Open full client" reuses the session and the dashboard unmounts the iframe. Under P2 (i) each session owns a TCP downlink and the limit disappears; say which.

### P9 — `panel.origin` in a shared SSE snapshot (major; verified, `EventBus::publish`)

Quoted: "`ServiceRecord` gains `panel: {id, origin, entry, …}`" · "Origin = `http://<host as typed>:<port>`". `services` is one payload for every subscriber (`events.rs:119`); a LAN viewer would get a loopback origin. Fix: `panel: {id, port, entry, needs_secure_context, allow[], state}`; the UI builds `http://${location.hostname}:${port}`; only the per-request `POST …/session` reply carries a full URL.

### P10 — Guard wording (minor; verified: `origin_matches_host` compares the full authority)

"`Origin` host == Host" must read "authority (host **and** port)": dashboard and panels share a host, so a host-only check lets panel B forge into panel A on a browser without `Sec-Fetch-*`; reuse `origin_matches_host`. State that WebSocket upgrades and SSE opens are guarded like mutations (cross-site WebSocket hijacking is the classic hole for a future MeshChatX panel) and that OPTIONS is refused, unlike the dashboard's `csrf_guard`.

### P11 — Header and listener details (minor; verified: `routes.rs` `CSP` is a `const`)

`frame-src`/`frame-ancestors` depend on the request `Host` (`127.0.0.1`, `111.local`, LAN IP): the dashboard CSP must become per-request from the accepted host — say so or a builder hardcodes `127.0.0.1`. Panel responses need `nosniff` and `Referrer-Policy: no-referrer` (D16 f parity); the proxy strips hop-by-hop headers and sets header-read/idle timeouts. The SPA fallback must never answer `/api/v1/*` with `index.html` when meshmud is down (the client would parse HTML as protobuf): answer `502 upstream_error`.

### P12 — Manifest snippet is not TOML (minor; reasoned from the TOML 1.0 ABNF: a table-header line admits only whitespace and a comment)

`[source]  kind = "static"` and the other same-line forms do not parse. Since this block is "the declaration" a builder copies, rewrite it as valid TOML and add "validated at build; invalid ⇒ panel not registered".

### P13 — Web Serial copy vs port ownership; HTTP connect dialog (minor; reasoned)

Quoted `secure_note`: "…works only when the dashboard is opened on the node itself." On the node the bound board's port is held by meshmud (`owner`, D19 c); Chrome opening the same tty is at best "busy", at worst two readers on one stream. Reword: "…and only for a radio this node has not bound; the bound board is reached through the panel's HTTP connection." Second: upstream prefills the connect dialog from `window.location.host` except `localhost`, which `host_allowed` accepts — make the prefill a G3 check or drop `localhost` from the panel Host list.

### P14 — Identify contract and handoff (minor; verified: `Problem.code` enum has no busy code)

`409 busy` needs a `Problem.code` (`port_busy`, matching the new `reason_code`); the spec lists it only under `reason_code`. "Start meshmud on the port with an identify profile" leaves open who stops it and how the service meshmud takes the port without a race or a second DTR reset: "the identify child becomes the service child in place (profile swap), never a second open".

### P15 — Termux loopback residual (minor; reasoned, D16 a)

D16 a made loopback untrusted under Termux because every app shares it; §14 then puts an unauthenticated TCP (11230) and HTTP (11231) path to the radio on that loopback. For a WiFi board the exposure equals the board's own LAN port — a residual, not a hole — but say it; §13's "never under Termux" implies the panel path is closed.

### P16 — "Read-only" copy (minor)

§14 defines read-only as 111's own behaviour; the page copy says only "Nothing here is 111-sealed." Add: "What you do in this panel (send, configure, reboot) goes to your radio and out over Meshtastic; 111 does not see or store it."

### P17 — meshmud config rendering (minor; reasoned)

The serial path comes from `/dev/serial/by-id/*`, i.e. USB descriptor strings a device chooses. Validate against `^/dev/(serial/by-id/)?[A-Za-z0-9_.:-]+$` before templating; config 0600; pin `advertise = false` (meshmud's mDNS announce is an unrequested LAN broadcast).

## 3. What is right (do not touch)

- One origin per panel by port, XFO dropped and `frame-ancestors` rewritten at the proxy, dashboard CSP at `frame-ancestors 'none'` plus an exact `frame-src` list (03 §3.2–3.6).
- Ticket → `HttpOnly; SameSite=Strict; Path=/`, no `Domain`, `s111`/`p111_*` stripped upstream; minting behind the session and the JSON+Origin CSRF guard.
- `sandbox` without `allow-top-navigation` is correct for a cross-origin frame (the dangerous `allow-scripts allow-same-origin` pair matters only same-origin); `allow=` delegation is required for serial/bluetooth and is origin-scoped.
- GPL boundary: unmodified bundle, reader and meshmud as separate children, no protobuf in the Rust binary; meshmud as the single slot holder.
- `needs_secure_context = false` for Meshtastic (HTTP mode works on plain HTTP); the gate kept for panels that need it.
