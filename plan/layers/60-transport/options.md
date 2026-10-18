# Layer 6 — Transport & Bridge: options brief

Status: ✍️ draft 2026-10-19 (critic + gap-fill pending; their findings go into §0.13+ and §5) · lead-written from the 6-scout sweep
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

- **A. In-binary Rust thin client (≈300 lines on prost) in `daemon/meshtastic/client.rs`, one more downlink on meshmud's `127.0.0.1:11230`:
  handshake once per connection, keep-alive with nonce 0, one send queue spaced ≥ 2.1 s, allowlist at the lowest layer (`TEXT_MESSAGE_APP`
  now, `PRIVATE_APP` later, `want_config_id` once, never heartbeat 1, no admin, no config).** +~150 KiB; the same client serves the Layer 5
  `meshtastic-client` Link seam later; Nearby folds into it and the Python presence reader retires after parity, which removes the 15-minute
  lapse (fact 4). Consequence to accept: the shipped `111d` is effectively GPL-3.0-only. *(pick)*
  - A2. Adopt the `meshtastic` crate (GPL-3.0, 0.1.9, one author): +372/+332 KiB, brings serial/BLE deps we do not use. Fallback.
- B. Extend the GPL Python presence sidecar into a two-way bridge over a local protocol: proven library, but Python on every Termux node's
  critical path, a respawn means a fresh config stream for every client, and sealed frames would cross a Rust↔Python line.
- C. A feature in meshmud — rejected (unmodified-upstream rule; Go absent; its closure would flip to GPL).

### F3. The LXMF adapter and the Reticulum UI

- **A. MeshChatX as a headless supervised child beside rnsd, from a hash-locked venv pinned from PyPI; `111d` talks to its loopback REST +
  WebSocket through a narrow carrier trait; the same child is the hosted Reticulum panel behind the child-panel proxy (WebSocket) that
  Layer 5 reserved.** No rule change. Labels say "Reticulum, sender unverified" (the API has no verify flag). Its sunset (2027-01-01) falls
  before the owner's hardware exists, so the carrier trait is the insurance. *(pick)*
- B. lxmd only — insufficient (inbound-only, no sender names, no send path); keep as the opt-in propagation-node role.
- C. Our own ~300-line sidecar importing RNS/LXMF in the rns venv: the sturdiest (public API, verify flag, custom fields, no sunset, pure
  Python on Termux) but a **D17 #12 rule change** with license consequences (a separate process under the Reticulum License, never combined
  with `111d`; a legal read). Recommendation: keep C ready and decide it when MeshChatX v5.0.0 lands without a successor.
- D. Rust reimplementations (Leviculum, LXMF-rs) — rejected: not upstream, legal exposure (upstream disputes machine-generated copies).

### F4. What "phone apps" promises in v1

- **A. An app-peer adapter shape plus two reference adapters on a Linux hub: SimpleX (bot WebSocket API + a hub `smp-server` on the LAN) and
  Delta Chat (`deltachat-rpc-server` JSON-RPC; a hub chatmail relay for LAN, public relays when online). Briar documented as online-only (Tor).
  ADB/UI driving of a phone is a user-attended, KB-guided runbook, outside the v1 promise; phone actions, if any, are typed `111d` actions
  behind human approval (the narrow P4 cut), never an agent shell.** *(pick)*
- B. Briar first (the owner's example): JVM-heavy and online-only; a poor fit for an offline mesh.
- C. Delta Chat only: smallest, best Termux story, weak LAN story.
- D. Framework only, apps as KB experiments: the category would promise nothing.
- Sub-decisions: the hub terminates an app's end-to-end encryption to bridge it (plaintext on the hub, per-contact opt-in, labelled) — needed
  for any of A–C; a Debian-class hub floor for SimpleX/chatmail binaries (musl-only boards excluded from this category).

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
- **Identify.** Own byte probes for family detection (RNode 13-byte DETECT, MeshCore `GetVersion`; Meshtastic via meshmud as built), rnodeconf
  only for provisioning, pairing and flashing; identify stays consented once per device (D17 #8); the RNode step is a separate process in the
  rns venv (rule 1 holds).
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

## 5. Open questions for the owner (beyond the polled forks)

1. Accept that the shipped `111d` binary is effectively GPL-3.0-only once GPL-3.0-only generated code links in (our files stay "or later")?
2. May the hub terminate a phone app's end-to-end encryption to bridge it (plaintext on the hub, per-contact opt-in, labelled on both sides)?
3. When the hub has internet, may phone-app adapters use public servers (chatmail relays, SimpleX presets) or only self-hosted ones?
4. Decide the LXMF sidecar (F3 C) now, or when MeshChatX v5.0.0 lands without a successor?
5. May the signed manifest ship a Columba APK (it embeds Reticulum-licensed Python) or only link to it (KB candidate, `-no-sentry` build)?

## 6. Critic and gap-fill (filled in when the run returns)

Pending.
