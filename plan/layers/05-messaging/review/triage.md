# Layer 5 — adversarial review triage

Panel: 3 fable judges (`wf_c4054cfb-f80`, 2026-10-14, ~565k tokens) on `os/messaging/README.md` v1 draft. Reviews:
`layers/05-messaging/review/{crypto,panels,integration}.md`. Verdicts: crypto **not-ready** (2 blocking / 10 major / 11 minor) ·
panels **fix-then-lock** (1 / 8 / 8) · integration **fix-then-lock** (1 / 7 / 9). All three: the architecture (D19)
stands; every blocker is a text-level fix. Disposition below; the rewrite is spec **v1.1** (same file).

Legend: ✅ accepted as proposed · ✅* accepted with a lead modification · ⛔ rejected (reason).

## Crypto / envelope / protocol

| id | sev | disposition |
|---|---|---|
| B1 | blocking | ✅ `auth = pair_mac(16) ‖ [sig(64) if SIGNED]`; only `pair_mac` keys replay state; G4 vector "a re-sealed valid signature never advances a window". |
| B2 | blocking | ✅ L2 amendment **A11**: for kinds carrying `msg_seq`, replay authority = `(sender, stream, msg_seq)`; the L2 window is not enforced below high-water for them; control kinds idempotent per kind. §18.1 order = `pair_mac → msg_seq dedup → store`. DTN residual stated. |
| M1 | major | ✅ ACK gains `n_partial ‖ (seq ‖ frag_bitmap u16)*`. |
| M2 | major | ✅ ACK block = `ack_len ‖ …`, placed before `payload`; `n_strong` byte always present. |
| M3 | major | ✅ control kinds: `msg_seq = 0`, exempt from `rx_state`/ACK, idempotent per kind. |
| M4 | major | ✅ `msg_seq = (epoch << 32) ‖ n`; epoch = a freshly committed L2 announce-counter value at stream (re)start (unclean open, restore); `n` committed (`synchronous = FULL`) before the first fragment is sealed; receiver collapses the prefix on a gap ≥ 2^32. This is A3's text. |
| M5 | major | ✅ signed input gains `dest` (recipient / group id / zeros for public) and `flags`. |
| M6 | major | ✅ stale announce with identical keys ⇒ accept, no state change; card requests answered from stored bytes; a connection is attributed to X only after a `pair_mac`-verified packet; push/forward gate on attribution; hello/announce authenticate nothing. |
| M7 | major | ✅* v1: INTRO is `tcp-lan`-only; LoRa first contact needs a card. The card-in-fragment-0 + bounded pending-INTRO design is recorded for Layer 6 (§21). |
| M8 | major | ✅* `tcp-lan 0x05 chunk` reserved; `session_tag` = 8 random bytes from the sealed `OBJ_REQ`; mode `0x03` struck; with J5 the transfer protocol is pinned but built in v1.1. |
| M9 | major | ✅ KCI residual written in §18; 18.5 reworded ("static-static ECDH never protects confidentiality"). |
| M10 | major | ✅ `ack-sig`, `custody-sig`, `obj` inputs defined; `111/ack/v1` deleted. |
| m1–m11 | minor | ✅ all: shim byte in the table · one RATCHET bit (outer), inner reserved bits reject · no two wires under one version (the 32 B fallback is a pre-release build gate only) · TLV `0x21` reserved for `signer_pubkey` · enrollment code rules + keyed-hash definition · `object_key` random, nonce = chunk index; store KDF switched to keyed BLAKE2b (one hash family) · card-request oracle stated and narrowed (enrolled contacts, past hello, never forwarded) · "proves receipt", COMMIT = max(cursor) · one-hub scope sentence (J1) · pair keys cached at the identity handle · reassembly bounds pinned. |

## Hosted panels + Meshtastic

| id | sev | disposition |
|---|---|---|
| P1 | blocking | ✅ strip every `Access-Control-*` header, never answer preflights; refuse `Sec-Fetch-Site: cross-site` for every method; no loopback skip (ticket → cookie everywhere); G3 cross-site GET check. |
| P2 | major | ✅* option (i): `111d` serves `/api/v1/toradio|fromradio` itself by reframing bodies onto a per-session meshmud **TCP** downlink (no protobuf decode, no GPL); meshmud's HTTP downlink is not configured. This dissolves P8 and removes the only browser-speakable upstream. Residual: `:11230` TCP is reachable by local apps (P15). |
| P3 | major | ✅ (generic proxy, reserved): drop upstream `Set-Cookie` named `s111`/`p111_*`; exact-name cookie reads, every copy tried. |
| P4 | major | ✅ lifecycle text: valid cookie ignores tickets; spent ticket ⇒ `403 panel_ticket_expired`; session-bound; `state/panel-sessions.json`; revoked on rotate/disable/LAN-off; ticket redacted from logs. |
| P5 | major | ✅ lint = headless Chrome over CDP in the netns recording off-origin requests; manifest `[offline] optional_hosts`; honest copy ("an internet-connected browser will contact …"); CI-built bundle from the pinned tag when no asset exists; GPL offer names the tag. |
| P6 | major | ✅ Termux cannot open USB serial: phone path = `tcp://<board-ip>:4403` uplink (Wi-Fi boards) or Web Bluetooth from the phone's Chrome; `can_run_here` honest; §20 gate reworded. |
| P7 | major | ✅ reader truncates/strips names; daemon `O_NOFOLLOW`, regular file ≤ 1 MiB, versioned schema, ≤ 4096 nodes, invalid ⇒ keep last; `nearby.read` returns names in an untrusted block; UI text-only. |
| P8 | major | ✅ dissolved by P2 (i): each panel session owns a TCP downlink. |
| P9 | major | ✅ `panel: {id, port, …}`; UI builds the origin from `location.hostname`; only the session reply carries a URL. |
| P10–P17 | minor | ✅ all: Origin = full authority, WebSocket/SSE opens guarded, OPTIONS refused · per-request `frame-src`/`frame-ancestors` from the accepted Host, `nosniff`, `no-referrer`, hop-by-hop, timeouts, `502 upstream_error` never `index.html` on `/api/v1/*` · valid TOML manifest, validated at build · Web Serial copy vs the bound port; `localhost` prefill as a G3 check · `port_busy` Problem code; identify child becomes the service child in place · Termux residual stated · "the panel transmits" copy · serial path validated, config 0600, `advertise = false`. |

## Integration / scope / buildability

| id | sev | disposition |
|---|---|---|
| B1 | blocking | ✅ `111d` mints `AGENT111_DAEMON_TOKEN` per spawn; `require_session` accepts `Authorization: Bearer` from a loopback peer only. Loopback trust is never reopened. |
| J1 | major | ✅ hub checks `dest` ∈ enrolled: status `5 REJECT_UNKNOWN_DEST` (sender keeps `sent`, reason "your hub does not know them"); v1 scope sentence (custody within one hub's members, or direct links); `MAILBOX_FETCH` = members, `pair_mac`-authenticated, own items only; A+ lifts it. |
| J2 | major | ✅ `Facts.radio.meshmud_bound`; `radio()` treats a meshmud-bound board as bound (`radio_bound_meshtastic`, detail "driven by meshmud"). |
| J3 | major | ✅ health for meshmud = child state + LISTEN on the TCP downlink (`listening_under`); never a connect. |
| J4 | major | ✅ identify v1 = Meshtastic step only; daemon-side port-owner table (`port_busy`); binding persisted in `devices.json` (`Summary`), migrated by Layer 6 into `interfaces.toml`; rnodeconf/MeshCore steps named seams. |
| J5 | major | ✅* (a) attachments → v1.1; v1 builds the blob store, BLAKE3 addressing, reserved kinds, the pinned transfer protocol (unbuilt). A5 dropped from the v1 L2 ask. |
| J6 | major | ✅ `websocket`/`sse` reserved (true fails adoption until the proxy implements them, P7); with P2 (i) the generic reverse proxy itself is reserved in v1 — v1 ships static panels + the Meshtastic shim. |
| J7 | major | ✅ meshmud: pin by tag + tarball sha256, CI-built, not copied into `os/`; label "MIT (own code); the binary links Meshtastic's GPL-3.0 Go protobufs ⇒ conveyed as GPL-3.0 (verify at build)"; LICENSING §3 rows for meshmud + the reader venv; `gen-notices.py` Go path; LICENSING §4 "vendored" wording kept true. |
| M1–M9 | minor | ✅ all: TOML · QR as module bitmap/SVG + `jsQR` (Apache-2.0) in the UI, named in §22 and the L4 sign-off · INTRO size note, "43–45" · `Link` trait types + object-safe form · G1 three-node harness extension + `lossy-loopback` test link; G2 emulator-in-CI or external · wording (idle hub NOT SET; phones `can_run_here`; `embed_url` never set with `panel`; restart order; QR address = LAN IP) · client pre-connect instruction (URL param verified at build) · MeshCore data-type registration + updater intake seams · card-request narrowing; SSE coalescing per message per tick. |

## Lead scope calls (reversible by the owner)

1. Attachments in messages → v1.1 (J5 a). 2. Generic reverse proxy + WebSocket/SSE → reserved; v1 = static panels +
built-in Meshtastic shim (P2 i, J6). 3. INTRO over LoRa → Layer 6 (M7). 4. Identify = Meshtastic step only (J4).

## Re-check (fable, `layers/05-messaging/review/recheck.md`)

Verdict **fix-then-lock**: 53 fixes landed, 4 partial, 0 missing. New findings, all accepted into **v1.2**:
N1 (blocking) frozen old-epoch receive state, newest 4 epochs, ACK ranges absolute `[epoch, start, len]` (≤ 8) ·
N2 every verified packet is still offered to the L2 window; attribution, card answers and pushes need a **fresh**
verdict · N3 `msg_seq` on the wire = `epoch ‖ n` varints; sizes table re-done (inner hdr 8–10) · N4 `tx_state` keyed
`(peer, stream)`; `msg_id` gains the recipient · N5 epoch minted once per open on: no clean record, `announce_last`
mismatch, restore floor set, `n` wrap; `restore_pending` while the floor is pending · N6 main listener 308
`localhost` → `127.0.0.1` · N7 card answers only to members on attributed connections · N8 `ENROLL` rides the INTRO
card slot, payload `{code}` · N9 `ack_sig` standalone only · N10 `loopback-only` defined · N11 shim downlink lifecycle
+ bounded buffer · N12 shim sub-type in bits 2–0 of frame_type 2 · N13 D20 lists the D19 deltas. Partials closed by
N1/N3/N5 (M4), N2/N7 (M6, m7), and the §3/§18.3 sentence that the derived pair key crosses into `mesh111` (m10).
Clean per the re-check: same-site iframe path, CORS stripping, Referer/ticket leak, genesis-over-newer pin.

