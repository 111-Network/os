# Layer 6 — Transport & Bridge: options brief

Status: 🧭 deciding · 2026-10-19 · sweep complete (6 scouts, opus critic, 1 gap-fill scout; ~2.0M tokens, 895 tool calls, 47–62 sources per lens); the gap-fill ran the LXMF endpoint and settled F3's shape · lead-written from the 6-scout sweep
(`layers/60-transport/research/`: seams scout opus, 5 lens scouts sonnet, each web-verified; critic opus). Reads with: `os/README.md` §6 ·
D17 (device decisions) · D19 c (LoRa ownership) · D20 (Layer 5 hand-offs) · D22 (GPL-3.0-or-later) · **D23** (this kickoff) ·
`cross-layer/devices.md` · `layers/60-transport/research/00-code-seams.md` (line-verified seam map).

Owner scope (D23): the bridge carries **readable text both ways**; foreign networks appear as **their own categories in Messages** while their
own UIs stay hosted panels; v1 networks **Meshtastic + Reticulum/LXMF + phone apps via Termux**; **no hardware yet** (full test setup later);
the Layer 6 build starts after the Layer 5 lock.

## 0. What the research settled (facts that bound every option)

1. **Foreign messages have no home in the built tree.** `messages.peer`/`sender` are `CHECK(length = 32)` NodeIds in the mesh111 store, the
   contract's `Message`/`Thread`/`SseMessage` are NodeId-keyed, and the UI route `/messages/:peer` accepts only a NodeId. Per-network
   categories need new tables, new routes, a new SSE event and a new UI route; none is an overload of the existing ones. Pseudo-NodeIds
   would enter the ACK and contact logic and break honest labels. (seams §1, §3.1 a)
2. **The fabric as built cannot carry sealed 111 traffic over a radio yet.** It is connection-oriented (sends only to an attributed
   `(link, PeerAddr)`, never to `Broadcast`), has no transit forwarding (`Dir::Transit` is a schema value only), fixes the fragment plan at
   queue time (a message planned for `tcp-lan` never leaves over LoRa), has no codec for the L6 shim byte, drops INTRO on non-`tcp-lan` links,
   has no `remove_link`, and awaits `Link::send` inside its actor. The §8 link seams exist as types; carrying sealed frames over Meshtastic
   or LoRa means amending the locked mesh111 crate in six places. (seams §3 findings 1–7)
3. **D22 unlocked the in-binary Meshtastic path.** prost types generated from a pinned snapshot of the GPL-3.0 `meshtastic/protobufs`
   cost **+152 KiB (x86_64 musl) / +132 KiB (aarch64)**, measured. Consequence: the shipped `111d` binary becomes effectively GPL-3.0-only
   (our files stay "or later"); two KB entries and Layer 5 §14's "no Meshtastic protobuf code enters our Rust binary" must be restated.
   (lens 1 §3.16–17)
4. **Meshtastic firmware facts that shape the bridge** (read from firmware source at v2.7.26): the firmware rewrites `from`, so the bridge
   speaks as the board's own node (name set in the panel, never by us); text from the API is rate-limited to **one per 2 s per API instance**,
   shared by every meshmud downlink (web client and bridge); the board's API session **lapses after 15 minutes without any `ToRadio`**, and the
   Layer 5 presence reader deliberately sends no heartbeat, so **Nearby silently freezes on serial boards today** (latent Layer 5 defect);
   heartbeat nonce 1 broadcasts NodeInfo (a transmit, never send it); DMs are PKI-only and need the recipient's key in the board's NodeDB;
   a channel broadcast can only ever reach "relayed" (a heard rebroadcast), a DM reaches "delivered" on the recipient's ACK; usable text is
   about 232 B plain / 220 B PKI, the app's convention is 200 B; the board's phone queue holds 8–32 packets, so texts received while no client
   reads are lost; packets on channels with MQTT uplink leave the mesh; HAM mode forbids obscured content. (lens 1 §3.1–13)
5. **meshmud is a verbatim forwarder** (one release, two commits, one author): any downlink's `want_config_id` restarts the config stream for
   every client; no DTR/RTS handling; a slow client loses inbound text. It stays the multiplexer; it is the wrong place for a bridge. (lens 1 §3.10)
6. **LXMF without importing RNS is possible through one adopted program.** MeshChatX 4.10.7 (0BSD AND MIT, verified) runs headless beside
   rnsd, attaches to the shared instance, and exposes loopback REST + WebSocket (`POST /api/v1/lxmf-messages/send`, conversation paging, WS
   `lxmf_message`). It is also the hosted panel P7 wanted. **Its README announces the end of development on 2027-01-01** (v5.0.0 last, repo
   archived). Its API is the UI's internal one (no title or custom fields, no signature-valid flag). lxmd's hook is inbound-only with no
   send path. Any path still needs the rnsd posture to change: `share_instance = yes` (Reticulum's own default; our template says `no`),
   a shared `rpc_key`, one client config dir per child. LXMF has **no channels**. Reticulum's truth is PyPI (rns 1.5.7), the GitHub mirror lags.
   (lens 2 §3.1–8, §3.16)
7. **"Phone apps via Termux" is narrower than P2 hoped.** Termux:API cannot drive another app's UI or install an APK silently; ADB wireless
   debugging to the same phone works only as a human-paired, attended session. `briar-headless` registers only the Tor plugin on Linux, so
   Briar is **online-only** as a bridge target. The two apps with a real headless peer plus a LAN-capable self-hosted server are **SimpleX**
   (AGPL; documented bot WebSocket API; `smp-server --ip` on a hub) and **Delta Chat** (MPL-2.0 core; JSON-RPC over stdio; an Android arm64
   build, so a Termux phone can be a node there; the LAN relay is an experimental mail stack). (lens 3 §1, §3.1–11)
8. **No chat-bridge core fits.** matterbridge has no Meshtastic or LXMF backend, clips text silently, keeps its ID map in memory and is loop-safe
   only while one bridge owns a gateway. Every Meshtastic relay found attributes with an in-text prefix; only one mentions a loop guard. The
   core and the loop guard are ours; the patterns (star gateway, prefix template, per-room opt-in relay mode) are worth copying. (lens 4 §3.1–4, §3.11)
9. **D19 c holds with two corrections.** The MeshCore KISS modem exists upstream, documented (115200 8N1, ≤ 255 B data, one pending TX,
   `SetRadio`, `GetAirtime`), with PlatformIO envs for Heltec V3/V4, RAK4631, T-Beam, T3S3; but **CI never builds it** (manifest-pinned CI images
   are mandatory), and the **data-type registry grants an ID only after a working demo**, so "registered before Layer 6 locks" (D20) cannot
   happen without the two-board bench: use the free dev range `FF00–FFFF` until then. (lens 5 §3.1–4)
10. **Hardware-less verification is stronger than assumed (run on this box).** Unprivileged nested network namespaces with veth work; real
    `rnsd 1.5.7` runs a Backbone server and a TCP client across them; a 40-line pty "RNode" stub (no RNS code) brings a real `RNodeInterface`
    to Up; `rnstatus --attach/--detach/--reload` changes interfaces live without restarting rnsd; `meshtasticd -s` is a simulated radio for
    client-API tests. What stays hardware-gated: RF flooding, the two-board bench, resets on serial open, BLE, flashing. (lens 5 §4.1)
11. **Meshtastic 2.8 is a moving target.** Stable is 2.7.26; 2.8.0 was revoked over a signing bug; 2.8.1 (pre-release) changes node
    numbering, adds XEdDSA signing and a Traffic Management module. The presence reader pins `meshtastic==2.7.11`. A10 cannot be satisfied as
    written for a 2.7.x board (an X25519-only key signs nothing). (lens 1 §3.18, lens 5 §3.15–17)
12. **Android reality is unchanged.** Termux has no 2026 release; TCP-to-hub stays the only reliable phone path; no adoptable helper exists
    for MulticastLock, USB-to-TCP or hotspot. (lens 5 §3.21–22)
13. **The critic's own checks (06):** meshmud originates nothing but one `want_config` at uplink connect, so the 15-minute lapse (fact 4) is
    real on the meshmud side too — **routed to the Layer 5 closure now** (the presence reader gains a nonce-0 heartbeat; polish wave HB).
    The official `meshtastic` Rust crate meets lens 1's constraints (nonce-0 heartbeat every 5 min, a config id we choose, any stream), so
    "own client" is not forced by them. MeshChatX requires **Python ≥ 3.11** while our venvs are built from the distro `python3`
    (3.10 on Ubuntu 22.04, 3.9 on Debian 11): the LXMF pick has a device-floor problem that the gap-fill scout is measuring (G1).
    Meshtasticator (CC BY 4.0, test-only) runs N native firmware nodes on TCP 4403+ and forwards packets by simulated range, but needs a
    native firmware build (no prebuilt binary); `meshtasticd -s` has the same prerequisite. **Madmail** (Rust single-binary chatmail
    relay, AGPL-3.0, IP + self-signed mode) was missed by lens 3 and changes Delta Chat's LAN story; `deltachat-rpc-server` speaks stdio
    JSON-RPC, which our supervisor cannot do today (`supervise.rs:333` sets `stdin = null`).
14. **README §6's own words are unaddressed by the sweep:** "a short, ordered list of preferred networks, curated in the Knowledge Layer,
    attempts to join them in sequence … falls back through the list". The brief scopes it in §3 (critic G5).
15. **Provenance of the RNode probe:** lens 5's DETECT bytes and its pty stub were modelled on `RNodeInterface.py`, which is
    Reticulum-licensed source read by LLM agents — the exact case upstream's "fakes" chapter warns about (lens 2 §3.17). The brief
    drops that path (critic G6).
17. **MeshChatX actually runs as the LXMF carrier (gap 1, *run* in two netns).** It attaches to rnsd's shared instance as a client,
    sends and receives both ways (event `lxmf.delivery`, the LXMF hash identical on both ends = a cross-node dedupe key), survives an rnsd
    restart (one delivery, no loss, no duplicate), and sent **zero packets** off-box in 25 minutes. Traps the adapter must handle: start
    order (rnsd must own the socket before any child, else the child becomes the instance and the node partitions silently); `--auth` is a
    no-op until setup plus a config PATCH; default inbound stamp cost 8 (set 0); auto-resend must be off (delivered must be sticky in our
    outbox); a peer must announce before it can be answered (111d schedules announces); field 16 `app_extensions` round-trips intact (the
    relay marker rides there plus a visible prefix). No signature-valid flag: signature-invalid messages are dropped before the event,
    source-unknown ones delivered unflagged.
18. **The LXMF floor is Python ≥ 3.11 (real: `tomllib`, `StrEnum`, `datetime.UTC`).** Debian 12+, Ubuntu 24.04+, Pi OS bookworm+ and
    Termux (3.14) meet it; Debian 11 and Ubuntu 22.04 (the lead box, 3.10) do not. Proven hybrid (*run*): rnsd stays on the distro
    Python, MeshChatX runs from a bundled python-build-standalone 3.11 (~47 MB per arch; repo MPL-2.0, interpreter PSF; a new L1
    dependency). A **slim closure** (35 wheels, 128 MiB on disk, 75 MB RSS, 0.4 s start; voice, WASM plugins, BLE and WebTransport lazily
    absent) carried messaging and has wheels for manylinux and musllinux on x86_64 and aarch64; the full closure (44 wheels, 251 MiB) has
    no aarch64 `miniaudio` wheel. Termux needs two or three CI-built wheels. (gap 1 §3–4)
16. **Frame budget arithmetic:** by the firmware check (255 − 16, minus 12 for PKI) and the encoding of `Data` (2-byte portnum varint plus
    a bitfield), the sealed `PRIVATE_APP` budget is about 231/219 B, not the 233/221 B Layer 5 §8 pins; text about 232/220 B. Settled at
    spec time with a prost encode (critic G7).

## 1. What Layer 6 is, in one paragraph

A **bridge core** inside `111d` (Rust) that gives every foreign network its own category in Messages, sends one text into several
networks as per-target messages, and relays between networks over explicit, human-enabled, loop-guarded edges; **adapters** that only move
native text (Meshtastic in-binary on the existing meshmud downlink; LXMF through a headless MeshChatX child; SimpleX and Delta Chat through
their headless peers); a **transport spine** (`state/interfaces.toml`, the rnsd config renderer v2 with the D17 #5 posture, live attach, a
phone relay over TCP to the hub); the **device core** (hotplug, port ownership, consented identify for RNode/Meshtastic/MeshCore, RNode stanza
proposals, the narrow apply→verify→undo cut for device actions); the **child-panel proxy** Layer 5 reserved, so the Reticulum UI is hosted
like the Meshtastic one; and, as a **second release once boards exist**, the 111 KISS modem link and sealed 111 carriage over foreign radios.
Everything below the core is adopted and run as-is.

## 2. Polled forks (owner decides; recommendation first)

### F1. Where foreign messages live

- **A. New tables in the mesh111 SQLCipher store (amends locked Layer 5 §10.3): schema V3 with `foreign_conversations`, `foreign_messages`,
  `relay_edges`, `send_groups`; typed `Fabric::foreign_*` calls; a `Class::Foreign` placed in the §10.4 prune order.** One key, one prune
  pass, one `local_seq`; the amendment is additive. *(pick)*
- B. A separate bridge store in node111. Layer 5 untouched, but a second key, a second prune and a second ordering.
- C. Memory-only — rejected: history is lost on restart, and people will live in the dashboard.
- D. Pseudo-NodeIds in `messages` — rejected (fact 1).

### F2. The Meshtastic adapter

- **A. In-binary Rust, adopting the official `meshtastic` crate (GPL-3.0, 0.1.9, 31k downloads, one listed author; +372/+332 KiB with
  its default features, less once serial/BLE features are trimmed): one more downlink on meshmud's `127.0.0.1:11230`, handshake once per
  connection with a nonce we choose, the crate's nonce-0 keep-alive, and our own allowlist on top (we call only `send_text` on a channel or
  to a node, never admin, config or heartbeat 1), one send queue spaced ≥ 2.1 s.** "Adopt, don't reinvent" (critic G2); the same connection
  serves the Layer 5 `meshtastic-client` Link seam later; Nearby folds into it and the Python presence reader retires after parity, which
  removes the 15-minute lapse for good. Consequence to accept: the shipped `111d` is effectively GPL-3.0-only (a legal read on the
  protobufs' missing version notice decides whether "or later" survives, critic G9). *(pick)*
  - A1. Our own ≈300-line client on prost types from a pinned protobufs snapshot (+152/+132 KiB, measured): the fallback if the crate's
    stream layer blocks the single queue, the pacing, or the sealed-frame Link; same licence consequence.
- B. Extend the GPL Python presence sidecar into a two-way bridge over a local protocol: proven library, but Python on every Termux node's
  critical path, a respawn means a fresh config stream for every client, and sealed frames would cross a Rust↔Python line.
- C. A feature in meshmud — rejected (unmodified-upstream rule; Go absent; its closure would flip to GPL).
- D. MeshMonitor's REST `POST /messages` as the send path — rejected for the reasons D19 g already gave (Node 22, ~189 MB, admin requests
  to the node, binds all interfaces); it stays a later optional service.
- Test fixtures (honest, critic G3): unit and integration tests run against the stub meshmud with scripted `FromRadio` frames; multi-node
  behaviour needs Meshtasticator or `meshtasticd -s`, both of which need a native firmware build in CI; RF behaviour needs boards.

### F3. The LXMF adapter and the Reticulum UI

- **A-slim. MeshChatX 4.10.7 as a headless supervised child beside rnsd, slim closure (35 wheels), hash-locked per arch and libc in CI;
  on hosts whose `python3` is older than 3.11 it runs from a bundled python-build-standalone 3.11 while rnsd stays on the distro Python
  (the proven hybrid); `111d` is its loopback REST + WebSocket client through a narrow carrier trait (start order, auth setup, stamp cost 0,
  auto-resend off, scheduled announces, sticky `delivered`, dedupe by LXMF hash); the same child is the hosted Reticulum panel behind the
  child-panel proxy Layer 5 reserved.** No rule change; every hard rule kept; the owner's "full UI plus text in Messages" in one adopted
  program. Labels say "Reticulum, sender unverified". Its sunset (2027-01-01, v5.0.0 last, no successor announced) is why the carrier trait
  exists. Hub only in v1 (phones are Sideband/Columba peers of the hub). *(pick)*
  - Sub-decision: bundled CPython as a new Layer 1 dependency (~47 MB per arch, LICENSING §7 row) **or** raise the Layer 6 LXMF floor to
    Debian 12+ / Ubuntu 24.04+ / Pi OS bookworm+ / Termux. Recommendation: bundle, because Ubuntu 22.04 hubs are common until 2027.
- A-full. The upstream-supported 44-wheel closure: +120 MiB, an aarch64 `miniaudio` source build in CI, voice and plugins we do not use.
- B. Original Reticulum MeshChat 2.4.0 (MIT): parses on 3.10, but no auth at all, no `app_extensions`, Electron-only releases (a Node
  frontend build in CI). Fallback if the owner refuses both the bundle and the raised floor.
- C. Our own ~300-line sidecar importing RNS/LXMF (Python ≥ 3.7, fits every distro and Termux with no extra interpreter; verify flag,
  custom fields, state callbacks, no web UI to secure, no sunset) — a **D17 #12 rule change** with license consequences (a separate process
  under the Reticulum License, never combined with `111d`; a legal read). Recommendation: keep C ready; decide it when v5.0.0 lands without
  a successor, or now if the owner wants verified/unverified sender labels in v1.
- D. lxmd only — insufficient (inbound-only, no send path); it stays the opt-in propagation-node role. E. Rust reimplementations
  (Leviculum, LXMF-rs) — rejected: not upstream, legal exposure.
- Evidence: A *run* (two netns, both directions, restart, egress log); B and C *source-read*.

### F4. What "phone apps" promises in v1

First the honest frame (critic G4): **P2 as worded — the agent installs and drives apps on the phone — is not feasible.** Termux cannot
drive another app's UI or install silently; ADB to the same phone is an attended, fragile session. The strongest *offline* phone paths are
already the first two networks: Sideband/Columba users reach us over the LXMF bridge, Meshtastic app users over their board. "Phone apps"
therefore means **hub-side adapters for apps that have a headless peer**, and the phone runs the ordinary app.

- **A. The app-peer adapter shape plus one reference adapter, Delta Chat: `deltachat-rpc-server` (MPL-2.0, Rust, arm64 Android build so a
  Termux phone can itself be a node) with **Madmail** (Rust single-binary chatmail relay, AGPL-3.0, IP + self-signed mode) on the hub for
  offline LANs and public relays when online. Needs one supervisor extension (a child with piped stdio, or a tiny socket wrapper) because
  `supervise.rs` sets `stdin = null`. Briar documented as online-only (Tor). SimpleX as a second adapter only on glibc hubs (its binaries are
  Haskell builds for Ubuntu), outside the musl floor. ADB/UI driving: a user-attended, KB-guided runbook, outside the v1 promise; phone
  actions, if any, are typed `111d` actions behind human approval (the narrow P4 cut), never an agent shell.** *(pick)*
- B. SimpleX first (documented bot WebSocket API, `smp-server --ip` on the hub): the best API of the lot, but glibc-only and no Android peer.
- C. Briar first (the owner's example): JVM-heavy and online-only; a poor fit for an offline mesh.
- D. Framework only, apps as KB experiments: the category would promise nothing.
- Sub-decision for any of A–C: the hub terminates an app's end-to-end encryption to bridge it (plaintext on the hub, per-contact opt-in,
  labelled on both sides). Evidence level: doc-only; no phone or app was run.

### F5. Sequencing and the first signed Layer 6 release

- **A. Bridge-first on a minimal spine.** Wave 0: posture v2 + `interfaces.toml` + renderer + live attach + `transport.read`, store V3 +
  contract, A10 `TransportId` amendment. Wave 1: bridge core + Meshtastic client + Messages categories + agent tools. Wave 2: LXMF (MeshChatX
  child, carrier, child-panel proxy) + SimpleX/Delta Chat adapters + phone relay v1 (Backbone listener, QR join). Wave 3: device core (hotplug,
  identify probes for RNode/MeshCore, RNode stanza proposals, the narrow applier) + Settings → Layer 6. Wave 4: netns verification with stub
  adapters + fable review. **First signed release = all of the above (R-B).** The 111 KISS modem link, its CI image, the MeshCore ID PR and
  sealed carriage over foreign radios form the **second release** once the owner's bench exists. *(pick)*
- B. Transport-first in the devices.md §8 order: follows the brief, delays the headline goal, puts the hardware-gated items first.
- C. Two parallel tracks: more throughput, but they meet in `interfaces.toml` and the Messages UI, and the box allows few build agents.
- Release cut alternatives: R-A (bridge + spine + phone relay only; device core later) if R-B slips; R-C (R-B + kiss-modem) rejected for the first release.

### F6. Sealed 111 traffic over foreign radios (the Layer 5 §8 seams)

- **A. Second release.** The in-binary Meshtastic client gains the `meshtastic-client` Link (sealed `PRIVATE_APP` frames to attested 111
  peers) together with the mesh111 amendments (opaque per-peer connections, a shim codec, re-planning unsent fragments, cost-aware routes,
  `remove_link`, non-blocking send). Nothing can prove flooding or range without boards; `meshtasticd -s` only tests the client API. *(pick)*
- B. Build the Link and the amendments in the first release, verified against stubs only: more Layer 5 surgery before any radio exists.
- C. Never: contradicts D19 c and README §6.

### F7. Relay semantics and defaults (one decision, three defaults)

- **Edges are explicit, directed, non-transitive, between channel/group conversations only (a DM cannot be an endpoint), enabled by a human on
  a paired or loopback session; the agent proposes. Guard: a relayed copy is never relayed again; own-echo suppression; a visible `[name/TAG] `
  marker on Meshtastic (8–14 B), a structured `FIELD_CUSTOM_*` on LXMF; an ID map bounded by capacity; two 111 bridges on one pair only warn in
  v1.** *(pick)* Defaults to confirm: (i) no courtesy notice on the source channel when an edge is enabled (airtime) — *off*; (ii) honour
  Meshtastic `ok_to_mqtt` as "do not take my message off the mesh" — *on*; (iii) relay into an amateur-licensed radio only after a human
  confirmation, sealed frames never — *as stated*; (iv) a backlog is held to capacity, then shed oldest with a visible counter.

### F8. Compose fan-out and the size problem

- **A. A targets picker; one send group; one message row per target with its own state, retry and cancel; a no-side-effect preview per target
  (UTF-8 bytes, parts, airtime estimate, region/TX status, refusals); oversize is refused by default, split is an explicit per-send choice with
  numbered parts, truncation never happens silently.** *(pick)*
- B. Cap every target at the smallest limit (≈ 200 B): simple, wastes LXMF and 111 capacity.
- C. Auto-truncate or auto-split: silently changes the user's words; Meshtastic parts arrive unordered.

## 3. Detail picks (spec-reviewed by the fable panel, not polled)

- **Categories.** A network rail: "111" first, then one per network, each split into channels and DMs; thread view per conversation; every
  foreign message shows the network chip, the native id, the untrusted name as text and an authentication label; a foreign address is never
  a 111 contact — "Send a Meshtastic DM from this node (readable, not 111-sealed)" opens a foreign DM; "Open the sealed thread" appears only
  when an A10 attestation or an in-band card links it to a NodeId; labels and copy come from an adapter manifest (no per-network UI code).
- **Delivery states** stay `queued | sent | relayed | delivered | failed` with an `evidence` string the UI prints (fact 4; LXMF `DELIVERED` =
  recipient proof, propagated = `relayed`).
- **A DM to the board's own Meshtastic node** lands in this node's Messages (its paired humans); hub-member routing of foreign DMs is v1.1
  with multi-device.
- **Meshtastic defaults.** All configured channels appear read-only from first run; sending needs the channel chosen; relaying the default
  public channel is allowed only by explicit edge; the bridge never writes node configuration (name set in the panel); HAM boards keep
  readable bridging off until a human confirms, sealed frames never; the shim drops `ToRadio.disconnect` so a panel close cannot end the
  session for everyone; received `via_mqtt` is tagged "via internet"; a channel with MQTT uplink warns before the first send.
- **A10 amendment (Layer 2 registry).** `TransportId` gains `meshtastic` (node number + X25519 key, bound by `node_sig` only on 2.7.x; a
  signed 2.8 broadcast later) and `reticulum` (SHA-256 of the 64 B public key; Ed25519 `transport_sig` from the child's identity file, which
  `111d` reads and signs but never parses further).
- **rnsd posture v2** (D17 #5 as decided): `share_instance = yes`, `instance_name = 111`, vault-held `rpc_key`, `enable_transport = yes` on
  Cat 3/4, a `BackboneInterface` listener on the LAN only when LAN sharing is on (IFAC optional, owner-gated), the `111.network` AutoInterface
  kept; a second AutoInterface in the stock `reticulum` group and the propagation-node role are Settings opt-ins, off by default; children start
  after rnsd's socket exists and restart with it; interface changes use `rnstatus --attach/--reload`, never `pkill`. Lens 2 argues an
  endpoint needs no transport; D17 #5 stands because the phone-relay role needs the hub to route.
- **Identify.** Adopted tools, not our own byte probes (critic G6): the RNode step is `rnodeconf --info` from the rns venv (a separate
  process, rule 1 holds), with a fake `rnodeconf` script for the text-parser tests and no stub modelled on Reticulum source; the MeshCore
  step is the documented MIT `GetVersion`/`GetIdentity` exchange; Meshtastic via meshmud as built. Identify stays consented once per device
  (D17 #8). rnodeconf stays the only path for provisioning, pairing and flashing.
- **README §6's ordered list (critic G5).** The "short, ordered list of preferred networks, curated in the Knowledge Layer" becomes a KB
  concept entry plus the adapter order in `state/interfaces.toml` and Settings → Layer 6 (111 fabric first, then Reticulum, Meshtastic,
  LXMF, app peers); at setup the agent proposes enabling them in that order. The runtime "join in sequence, heartbeat, fall back through
  the list" is Layer 7's presence and gateway work and is deferred there explicitly.
- **Node configuration.** In v1 the bridge never writes Meshtastic node configuration (name set in the panel); D17's typed
  `meshtastic.set` allowlist stays reserved vocabulary for a later cut (resolves lens 1 vs devices.md §3).
- **PSKs.** Conversations are keyed by a keyed hash of the channel name and PSK; the PSK itself is never persisted (resolves lens 1 vs 4).
- **MQTT-uplinked channels.** Sends on a channel with MQTT uplink are off by default and warn when enabled (resolves lens 1 vs 5).
- **LXMF relay marker.** A visible `[name/TAG] ` prefix in the body on LXMF too, because MeshChatX's API accepts no custom fields and
  stock Sideband users must see the original sender; plus the per-node hash dedupe (resolves lens 2 vs 4).
- **MeshCore data type**: dev range `FF00–FFFF` until the bench, then the PR (plan correction to D20's hand-off).
- **Versions**: Reticulum pins resolve from PyPI at release time; the Meshtastic bridge is tested on 2.7.26 stable, 2.8.1 is a bench item;
  the presence reader's 2.7.11 pin becomes moot once the in-binary client reaches parity.
- **Legal copy** on the Meshtastic service page replaces "111 never transmits" with "111 transmits only what you send or choose to bridge, as a
  normal text from your board's own node; anyone on that channel can read it" (lens 1 §4 proposed text).
- **The narrow P4 cut** ships for device actions only: `rns.interface.upsert/remove`, `rns.posture.set`, `radio.region.set` (user-confirmed),
  with snapshot → apply → `rnstatus` probe within 30 s → keep or revert, result written to the proposal. Flashing stays contract-only until the bench.

## 4. Plan corrections this brief records

- D20 hand-off "MeshCore data-type range registered before Layer 6 locks" → "dev range until the two-board bench, then the PR" (fact 9).
- `cross-layer/devices.md` §8 "parse rnsd config with `--exampleconfig`" → "start rnsd on a scratch dir and read `rnstatus`" (lens 5 §3.10).
- Layer 5 §14 "111 never transmits on the Meshtastic mesh" and "no Meshtastic protobuf code enters our Rust binary" → restated per D22/D23 (fact 3–4).
- Layer 5 latent defect: the presence reader never heartbeats, so a serial board's API session lapses after 15 minutes (fact 4) — a Layer 5
  follow-up row (nonce-0 heartbeat in the reader) until the in-binary client replaces it.
- KB: `projects/reticulum.md` and `projects/lxmf.md` versions are stale; `meshtastic-python.md` and `meshmud.md` still call in-binary
  protobufs a blocker; `briar-bramble.md` license string is `GPL-3.0-only` (the file says "or any later version").
- Layer 5 §8 frame budget 233/221 B → about 231/219 B once a prost encode settles it (fact 16).
- devices.md §3 "own probes" sequencing → adopted `rnodeconf --info` (fact 15).

## 5. Open questions for the owner (beyond the polled forks)

1. Accept that the shipped `111d` binary is effectively GPL-3.0-only once GPL-3.0-only generated code links in (our files stay "or later")?
2. May the hub terminate a phone app's end-to-end encryption to bridge it (plaintext on the hub, per-contact opt-in, labelled on both sides)?
3. When the hub has internet, may phone-app adapters use public servers (chatmail relays, SimpleX presets) or only self-hosted ones?
4. Decide the LXMF sidecar (F3 C) now, or when MeshChatX v5.0.0 lands without a successor?
5. May the signed manifest ship a Columba APK (it embeds Reticulum-licensed Python) or only link to it (KB candidate, `-no-sentry` build)?
6. Bundled CPython (python-build-standalone) as a Layer 1 dependency, or raise the LXMF floor to Debian 12+ / Ubuntu 24.04+ (F3)?
7. LXMF bridge display name and announce policy (announces cost airtime on LoRa; stock apps learn our key only by announce).
8. Must v1 show verified vs unverified LXMF senders (forces C), or is "unverified" enough?
9. The "send to Reticulum" target with no LXMF channels: a chosen peer address (per-network recipient lists; recommended), a
   distribution-group hub, or Reticulum Relay Chat rooms later?
10. A DM to the board's own Meshtastic node lands in this node's Messages (recommended); hub-member routing waits for multi-device (v1.1).

## 6. Critic and gap-fill

**Verdict (06-critic.md):** strong on seams, Meshtastic firmware behaviour, bridge semantics and transport sequencing; sufficient today
for the store, categories, fan-out, relay and loop-guard forks; **not** sufficient for the LXMF fork until G1 is filled.

| # | Gap | Severity | Where the brief lands |
|---|---|---|---|
| G1 | LXMF endpoint never run; MeshChatX needs Python ≥ 3.11 vs the distro-`python3` venv floor | blocking | **filled** (`07-gap-1.md`): run in two netns, works; floor matrix; hybrid interpreter proven; slim closure sized; F3 rewritten |
| G2 | Own Meshtastic client picked over the adoptable crate; MeshMonitor not weighed | major | F2 pick flipped to the crate; MeshMonitor rejected with reasons |
| G3 | No proven hardware-less Meshtastic harness | major | F2 test-fixture line: stub meshmud for unit/integration, Meshtasticator or `meshtasticd -s` after a native build in CI |
| G4 | Phone apps vs the device floor and offline-first; Madmail missed; stdio blocked | major | F4 reframed (Delta Chat + Madmail first, SimpleX on glibc hubs, honest lead) |
| G5 | README §6's ordered list unaddressed | major | §3 detail pick: KB list + interfaces order now; runtime fall-through in Layer 7 |
| G6 | RNode probe bytes derived from Reticulum source | major | §3: `rnodeconf --info` adopted; the stub dropped |
| G7 | Frame budget 233/221 vs ≈231/219 | minor | fact 16; settled by encode at spec time |
| G8 | No adopt-heavy alternative (Matrix/XMPP hub as the bus) weighed | minor | considered: a hub homeserver plus N bridges does not put text in *our* Messages view (D23) and adds a server to every hub; kept as a Layer 7 internet-gateway option (F5 D) |
| G9 | protobufs LICENSE carries no version notice | minor | legal read; F2 wording hedged |

**Conflicts the critic listed, and how the brief resolves them:** (1) adapter channel → Meshtastic in-binary; LXMF: `111d` is a REST/WS
client of the MeshChatX child; Delta Chat: a child with piped stdio (supervisor extension) — one pattern per adapter, no spool dir unless
an adapter needs one; (2) LXMF marker → visible prefix plus hash dedupe (§3); (3) `enable_transport` → D17 #5 stands (§3 posture);
(4) text ceiling → the firmware's real ceiling is the physics the preview shows, the 200 B app convention is a note; (5) adopt vs own →
crate and rnodeconf (F2, §3); (6) MeshCore ID before the lock → owner (F5 note, §4); (7) node config → never written in v1 (§3);
(8) PSK → keyed hash (§3); (9) MQTT channels → off by default plus warn (§3); (10) sequencing → owner (F5); (11) meshmud has a release
(lens 1 right); (12) MeshCore loop-detect default → irrelevant in v1.

**Evidence level per fork** (critic's honesty note): F1 store *source-read* (schema and seams line-verified) · F2 Meshtastic *source-read*
(firmware and crate source; sizes *run*; nothing on a board) · F3 LXMF *run* (gap 1) · F4 phone apps *doc-only* ·
F5 sequencing and the transport spine *run* (netns rnsd, pty stub, `rnstatus --attach`) · F6 sealed carriage *source-read* · F7/F8 relay
and fan-out *design* on prior art read from source and docs.
