# Layer 5 — adversarial review: integration · scope honesty · buildability · adopt-don't-reinvent

Lens: integration/seams, scope honesty, buildability, adopt-don't-reinvent. 2026-10-14. Spec: `os/messaging/README.md`
v1 draft. Real checks: byte-budget arithmetic (python); reads of `services/mod.rs`, `api_v1/services.rs`, node111 and
agent111 `Cargo.toml`, agent111 `config.rs`, `install/supervise.rs`, `111node.service`, `openapi.yaml`, `smoke.sh`,
`ui/package.json`, id111 `seal.rs`/`announce.rs`/`counters.rs`/`store.rs` headers. No TOML parser available
(Python 3.10): M1 is by grammar.

## 1. Verdict: **fix-then-lock**

1. D18/D19 are implemented faithfully: every polled fork and §3 detail pick of the brief is present; the D19 c text
   is in `devices.md` §2.1/§7, the D19 d narrowing of D15 in §13/§19. No silent reversal.
2. One blocking integration defect: the agent tools (§17) use loopback `/api/v1`, which the locked L4 auth (D16 a)
   refuses under Termux — a declared v1 target.
3. The delivery model has an unstated consequence that makes `relayed` dishonest: a hub accepts custody for any
   destination, only its members can fetch, and cross-hub addressing is unspecified.
4. Three v1 pieces are hidden subsystems no gate exercises (Tier-2 bulk, WebSocket/SSE proxying, the full identify
   sequence); each cuts to a seam without touching the architecture.
5. The rest is seam hygiene. Nothing contradicts an owner decision; nothing in this lens is a security hole.

## 2. Findings

| id | severity | section | title |
|---|---|---|---|
| B1 | blocking | §17, §15 | Agent tools cannot reach `/api/v1` under Termux (401) |
| J1 | major | §9, §7.3 | Custody for non-members: `relayed` shown for undeliverable items; cross-hub unspecified |
| J2 | major | §14, §15 | `radio` row NOT SET for the board the `meshtastic` row calls LIVE |
| J3 | major | §13, §14 | TCP-connect health probe on the one-client HTTP downlink |
| J4 | major | §14, §2 | Identify: scope creep, no port-lock mechanism, `interfaces.toml` has no owner |
| J5 | major | §5, §11, §20 | Tier-2 bulk + attachments: untested subsystem in v1 |
| J6 | major | §13 | WebSocket/SSE/large-body proxying not needed by the only v1 panel |
| J7 | major | §2, §14, §19 | meshmud labelled "MIT", "vendored"; no Go notices path |
| M1 | minor | §13 | Manifest snippet is not valid TOML |
| M2 | minor | §15, §16 | QR: PNG not buildable with pinned `qrcode`; no decoder adopted |
| M3 | minor | §3, §7.2 | Size-table nits: INTRO never fits 160 B; "41–45" |
| M4 | minor | §8 | `Link` trait: undefined types, not object-safe |
| M5 | minor | §20 | Gates: multi-node harness, Termux run, loss injection |
| M6 | minor | §15, §14, §13, §7.3 | Wording: idle hub LIVE, phones, `embed_url`, two-child restart, QR address |
| M7 | minor | §13, §14 | Pointing the web client at meshmud is unspecified |
| M8 | minor | §19, §21 | Missing seams: MeshCore data-type registration, updater intake of fabric payloads |
| M9 | minor | §8.1, §15 | Card-request oracle on hubs; SSE ring flooding |

### B1 — Agent tools cannot reach `/api/v1` under Termux — blocking · verified
Quoted: §17 "The agent reaches these through `111d`'s loopback `/api/v1` (a `daemon_api_url` config joins
`daemon_health_url`)".
Attack: every `/api/v1` route except `/events`, `/auth/*`, `/health` sits behind `auth::require_session`
(00-code-seams §3). D16 a / interface §11: under Termux "loopback peers need a session like LAN peers". `111-agent` is
a loopback peer with no cookie → `messages.read`, `contacts.read`, `fabric.read`, `nearby.read` answer `401` on every
phone node. Desktop works only because loopback trust is a policy D16 a calls reversible. agent111 `config.rs:23`: the
agent's sole daemon call today is the ungated `GET /health`, so nothing has hit this yet.
Fix: `111d` mints a per-spawn secret into the child env (`AGENT111_DAEMON_TOKEN`, beside the existing `AGENT111_*`);
`require_session` accepts `Authorization: Bearer <token>` from a loopback peer only. Record in §17 and §19 (L3 + L4).
A 0600 unix socket under `/var/lib/111/agent/` is the alternative. Never reopen loopback trust.

### J1 — `relayed` for undeliverable items; cross-hub unspecified — major · verified (text)
Quoted: §9 "`CUSTODY_PUT {dest NodeId(32) ‖ sealed item bytes}` … Hub: dedup by `pkt_ref`, store `(dest, bytes,
custody_seq)`, reply `CUSTODY_ACK`"; "a hub refuses `CUSTODY_PUT` from unenrolled nodes"; "`MAILBOX_FETCH {cursor
varint}` from a member"; §7.3 "A node has one home hub in v1".
Attack: enrollment is checked on the *sender*; nothing checks `dest`. Alice (hub H_A) writes to Bob (hub H_B). Alice
PUTs to H_A → `ACCEPTED` → UI shows `relayed`, "a mailbox node saved it and will hold it for them". Bob is no member of
H_A and cannot fetch there (he holds an 8 B truncated hub hint — no address, no key). The item sits until pruned; the
sender sees progress the network never confirmed. Bob's ACK to an absent Alice fails the same way. With A+ deferred,
v1 store-and-forward works only between members of one hub or over a direct link — acceptable, nowhere stated.
Fix: (a) hub checks `dest` against its enrolled list: `ACCEPTED` only for members; status `5 REJECT_UNKNOWN_DEST` →
sender keeps `sent`, reason "your hub does not know them"; (b) §1/§9 state the v1 scope sentence and the `queued`
reason "their hub is not yours"; (c) §9 says who may `MAILBOX_FETCH` (members, `pair_mac`-authenticated, own items
only); (d) note A+ lifts (a). D19 b's definition of `relayed` is untouched; it becomes true.

### J2 — `radio` NOT SET beside `meshtastic` LIVE — major · verified (code)
Quoted: §14 "binds the board: `driver.stack = meshtastic`, `service_id = meshtastic`"; "The `radio` service keeps its
meaning (rnsd-bound RNode, meshtasticd on SPI)".
Attack: `services/mod.rs` `radio()`: `bound = !r.rnode.is_empty() || r.meshtastic_seen`; `meshtastic_seen` is a
LISTEN on 4403. A USB board driven by meshmud over serial leaves both false → `radio_unbound` → "no mesh stack is
bound to it yet … Set up with the agent" while `meshtastic` says LIVE. `devices.md` §8 flagged this; the spec left it.
Fix: `Facts.radio` gains `meshmud_bound: Vec<port>`; `radio()` treats a meshmud-bound board as bound
(`radio_bound_meshtastic`, detail "driven by meshmud") or drops it from `boards`. One rule, written in §15.

### J3 — Health probe may consume the single HTTP client — major · reasoned
Quoted: §13 `[health] tcp = 11231`; "Health: … GET expecting 2xx/3xx, or TCP connect"; §14 "HTTP `127.0.0.1:11231`
(**one** client: the panel)".
Attack: 07-gap-3 #24 marks "behavior of a second HTTP client" unverified. If meshmud counts by connection or first
`fromradio` caller, a 2 s probe steals or flips the slot and the panel loses the board. D16 f recorded the identical
lesson for meshtasticd ("LISTEN check, never a connect").
Fix: health for a one-client upstream = child state + `listening_under(port)` (`probe.rs:173`, exists); manifest
`health = { listen = 11231 }`; connect probes only with `multi_client = true`. Name "second HTTP client behaviour" in
the hardware gates.

### J4 — Identify: scope creep, no port lock, `interfaces.toml` ownerless — major · verified (code)
Quoted: §14 "`POST /devices/{id}/identify` (consent recorded per device) → port lock → `rnodeconf --info` (rns venv,
if present) → start meshmud … → MeshCore query"; §2 "`state/interfaces.toml` (Layer 6's; this layer reads `owner`
and Meshtastic bindings)"; §14 "Config rendered by `111d` from `state/interfaces.toml`".
Attack: D19 g says "Meshtastic branch = start the multiplexer and read want_config once". The spec builds the route,
consent persistence, a port lock, the rnodeconf step and its output parsing — `devices.md` §8 lists "port ownership
lock, ModemManager/brltty guards, consented identify" as Layer 6 opener item 1. `registry.rs` has no lock primitive
(grep: none); "port lock" is a word, not a mechanism. `interfaces.toml` exists in no built code (grep: none); §2
calls it Layer 6's while §14 reads from it, so L5 would create a file whose schema L6 inherits. A builder guesses.
Fix: v1 identify = Meshtastic step only: consent + a daemon-side port owner table (`HashMap<port, Owner>`, surfaced as
`port_busy`) + meshmud with the identify profile + the reader's `my_node_info` → `DeviceRecord.firmware {family:
meshtastic, version, source: probe}`. rnodeconf/MeshCore steps stay named seams. Persist the binding in `devices.json`
(`Summary`, exists); "Layer 6 migrates it when it creates `interfaces.toml`". Drop "(rns venv, if present)".

### J5 — Tier-2 bulk + attachments: untested subsystem — major · verified (gates read)
Quoted: §11 "Consumers: attachments in `MSG` (v1)"; §5 Tier 2 "`OBJ_ADVERT` … `OBJ_REQ {hash, ranges}` →
`OBJ_CHUNK` … range-set requests … resume at any chunk boundary"; §10.3 "encrypted once with their `object_key`
(XChaCha20-Poly1305 in 64 KiB chunks) and addressed by ciphertext hash".
Attack: adverts, sessions, range requests, bao outboards at two chunk groups, a refcounted blob store, object-key
AEAD and the unsealed-chunk carve-out (L2 amendment A5) — the size of the mailbox. G1–G4 never move an attachment;
nothing fails if it is wrong. D19 a adopts bao verification; the protocol around it is ours.
Fix: (a) cut attachments to v1.1 — v1 builds the blob store, BLAKE3 addressing, the reserved kinds; §16's composer
says "attachments arrive next"; or (b) keep them, restricted to Tier-2 over `tcp-lan` (one `OBJ_REQ`, whole object,
16 KiB group, no range sets) and add "10 MB attachment hub-relayed, hub killed and resumed" to G1. Either way drop A5
from the v1 L2 ask (only unsealed chunks on broadcast bearers need it).

### J6 — WebSocket/SSE proxying not needed by the only v1 panel — major · verified (manifest)
Quoted: §13 "Streaming: forward WebSocket upgrades, SSE without buffering, bodies up to `max_body`"; manifest
`websocket = false  sse = false`, `[source] kind = "static"`.
Attack: v1 = static files served by `111d` + one path-prefix HTTP proxy to meshmud. Upgrade forwarding needs
server-side `hyper::upgrade` + a client conn + a bidirectional copy, or `axum-reverse-proxy` with its own client
stack — "decided at build by size" is a guess handed to the builder. First consumer: MeshChatX, parked P7.
Fix: keep the fields; mark `websocket`/`sse` "reserved; true fails adoption until the proxy implements them (P7)". G3
tests what v1 ships: static serve, ticket → cookie, HTTP route, header rewrite, drive-by POST refused.

### J7 — meshmud: "MIT", "vendored", no Go notices path — major · reasoned
Quoted: §2 "`meshmud` … (MIT, Go; vendored, pinned)"; §14 "no Meshtastic protobuf code enters our Rust binary. Source
offer in `THIRD-PARTY-NOTICES.md`"; §19 "`gen-notices` covers the GPL source offers".
Attack: a Meshtastic multiplexer decodes `FromRadio`/`ToRadio` protobufs; the Meshtastic Go bindings and `.proto`
sources are GPL-3.0. 07-gap-3 #22 lists `go.bug.st/serial`, zeroconf, avahi and does not record the protobuf dep —
unverified. If present, the meshmud *binary* we redistribute is a GPL-3.0 combined work: fine across our process
boundary (LICENSING §1 rule 1), but the §2 label is wrong, the source offer must cover meshmud plus its Go closure at
the pinned commit, and `gen-notices.py` walks only cargo/pnpm/fonts/Monocypher. "Vendored" contradicts LICENSING §4
"Nothing else is vendored" unless that section changes and the Go closure's license texts travel in-tree.
Fix: pin by upstream tag + tarball sha256, build in CI from the tarball (not copied into `os/`); record the `go.mod`
closure and licenses in LICENSING §3 with the GPL conveyance line; give `gen-notices.py` a Go path (`go-licenses` or a
hand list gated by `check.sh`). Same row for the reader's venv (meshtastic-python GPL-3.0 + pip closure).

### M1 — Manifest snippet is not valid TOML — minor · reasoned
`[source]  kind = "static"` and `[frame]   allow = [...]   needs_secure_context = false   sandbox = "default"`: TOML
1.0 allows one expression per line; a header and a key/value, or two key/values, cannot share one. Write the binding
shape as valid TOML and say "parsed with the `toml` crate at build; unknown keys fail adoption".

### M2 — QR: PNG not buildable; no decoder adopted — minor · verified
§15 "`qr_png_base64?` (QR rendered by `qrcode`, already a dep)"; §16 camera scan + file upload. `Cargo.toml`:
`qrcode = { version = "0.14", default-features = false }` — no `image`/`svg` (TASKS.md "no image/svg"). Decoding in
the browser needs a JS decoder; `package.json` has only react, react-dom, wouter; §22 names none.
Fix: return the module bitmap (`{size, rows}`) or an SVG string drawn by the UI; name the decoder (`jsQR`,
Apache-2.0, or `zxing-wasm`) in §22 and the Layer 4 sign-off list.

### M3 — Size-table nits — minor · verified (arithmetic)
Outer 64 B and all four payload columns check (71–73 / 23–25 / 144–146 / 96–98). An INTRO carries a 137–169 B card
inside the inner plaintext: at 160 B that is −64 B, so INTRO is always Tier-1 (≥ 3 frames) on LoRa — say so in §7.2.
"subtract 28 B … (41–45 B at 160 B)" is 43–45 B.

### M4 — `Link` trait: undefined types, not object-safe — minor · verified (text)
§8: `Frame`, `LinkError`, `LinkState`, `PeerAddr` (incl. a broadcast address for flood bearers) are never defined;
`async fn` in a trait is unusable behind `Box<dyn Link>` without `async-trait` (not in §19) or boxed-future
signatures. This is the Layer 6 contract: define the four types (`Frame = {frame_type, hop_limit?, bytes}`), pick the
object-safe form.

### M5 — Gates — minor · verified (smoke.sh)
`smoke.sh` runs one `NODE111_ROOT` in one netns; G1 needs three roots, three netns, a veth bridge — name the harness
extension as a task. G2 "under Termux (emulator or device)" is not "netns, no hardware" — move to the external list or
say "emulator in CI". "forced 160 B profile with loss injection" needs a test-only `Link` (`lossy-loopback`) — list it.

### M6 — Wording — minor · reasoned
(a) §15 `fabric` LIVE "or acting as hub": a hub with no members and no links reads "Messaging LIVE"; prefer NOT SET
"waiting for the first node to enroll". (b) §14 never sets `can_run_here=false` on phones (no USB serial under Termux,
`devices.md` §2.2). (c) "`embed_url` stays" vs D19 d "superseded": say "kept for link-out, never set when `panel` is
non-null". (d) `meshtastic` `restart` with two children: define (both; reader after meshmud). (e) §7.3 enrollment QR
address = the LAN IP, per D16 e.

### M7 — Pointing the web client at meshmud — minor · reasoned
§13 `[[proxy.routes]] path = "/api/v1/"`. The client's HTTP transport asks the user for a node address (03 #13,
07-gap-3 #25); nothing says to type the panel's own `host:port`. Pre-connect via a client URL parameter if one exists
(verify at build), else put the exact instruction in the honest copy and `entry.path`.

### M8 — Missing seams — minor · verified (text)
D19 c requires "a data-type range registered with MeshCore" before Layer 6 locks — absent from §19/§20/§21. README §5
promises updates over the fabric; §19 Layer 1 lacks "the updater accepts a staged payload from `fabric/blobs/<hash>`
and verifies it with the same minisign manifest".

### M9 — Card-request oracle; SSE ring — minor · reasoned
§8.1 `0x04` is cleartext and unauthenticated; a hub "answers from its contacts cache … or forwards the request to the
member": any LAN peer can test membership of a NodeId and make the hub push to members. Answer only for `enrolled`
contacts to peers past the announce hello; never forward. §15 publishes `message` per state change on the shared
256-entry ring: one 16-fragment transfer can `reset` every dashboard client — coalesce per message per tick.

## 3. What is right (do not touch)

- The D18/D19 mapping is complete and faithful, including the D19 c text in `devices.md` §2.1/§7 and the D15
  narrowing in §13 — no silent additions beyond the identify branch in J4.
- The size table (§3) is arithmetically correct against `seal.rs` (11 + 5 + 32 + 16 = 64 B outer); the "A2 rejected"
  fallback is honest.
- The adoption boundary: SQLCipher/rusqlite, BLAKE3/bao-tree, ciborium, meshmud, the web bundle and meshtastic-python
  are each the candidate the research named, rejected alternatives recorded; no RNS import; GPL stays in separate
  processes (keep the bundle on disk — never behind `rust-embed`).
- Ports, state dirs and child shape match built code: 11183 / 11190–11229 / 11230–11231 collide with nothing;
  `/var/lib/111` is node111-owned under `StateDirectory=` so `fabric/` can be created at runtime like `agent/`; meshmud
  and the reader fit `ChildSpec` + `Supervised::with` exactly.
- §4's no-timestamp ordering, §9's four states as *definitions*, §10.4's pruning order and "never undelivered
  outbound" are the right honest core — J1 fixes who may rely on `relayed`, not what it means.
