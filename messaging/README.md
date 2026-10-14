# Layer 5 — Messaging & Data

The fabric that carries every kind of traffic the node needs — human messages first, then agent operations, knowledge
sync, updates and patches — as end-to-end sealed, signed-where-it-matters payloads, with **reliable delivery first and
latency second**, store-and-forward across whatever links exist, and an encrypted local store pruned by capacity, never
by policy. Plus the dashboard's power to **host** an adopted project's own panel, and the first hosted project:
Meshtastic.

Status of this document: **spec v1 — draft for the adversarial panel** (decisions D18, D19 in `os/plan/DECISIONS.md`;
options brief `os/plan/layer-5-options.md`; research `os/plan/layer-5-research/`). Nothing here is built yet. Build
tasks will live in `TASKS.md` per `os/plan/EXECUTION.md`.

Reads with: `os/README.md` §5 (vision) · `os/identity/README.md` §6, §9, §10, §13 (the sealed packet, announces,
delegation, attestation, hand-offs) · `os/interface/README.md` §5–§7, §10–§11 (dashboard, services, contract, auth) ·
`os/intelligence/README.md` (tools, task API) · `os/plan/devices.md` (Layer 6 brief, amended by D19 c) ·
`os/plan/layer-5-research/00-code-seams.md` (where this plugs in).

## 1. Scope & non-goals

**In v1 (this cycle):**
- The **fabric**: `mesh111`, a Rust library crate (`os/messaging/crates/mesh111`), wired into `111d` as the module
  `daemon/fabric/` — envelope codec, message identity, fragmentation, authenticated receipts, contacts, the home-hub
  mailbox, the `Link` trait, the encrypted store, the local API. The identity keyset never leaves the daemon (L2 §13:
  signer pattern); `mesh111` receives a signer/sealer handle, never key bytes.
- One link built: **`tcp-lan`** — sealed frames on a length-prefixed TCP stream between 111 nodes (phone ↔ hub, laptop
  ↔ hub, hub ↔ hub). Every other link (`kiss-modem`, `meshtastic-client`, `rns-bridge`, `ble`) is a **seam** with its
  frame budget pinned here and its driver built by Layer 6.
- **Delivery model v1** (D19 b): direct links + **home-hub mailbox** + contact cards. No multi-hop discovery, no
  periodic announces, nothing from Layer 7.
- **Hosted panels** (D19 d): one origin per panel by port behind `111d`'s reverse proxy; the first panel is
  **Meshtastic's own web client**.
- **Meshtastic service** (D18 #3, D19 g): consented identify, `meshmud` multiplexer, read-only presence into Nearby,
  the hosted panel. **No message bridge.**
- **Messages** view, **Nearby** list (Meshtastic presence), **Settings → Messaging & Data**, agent tools.

**Not v1 (recorded in §21):** bridges that relay messages between networks · LoRa drivers · groups beyond pairwise
fan-out · ratchets · erasure coding · hub-to-hub replication · KB/update/patch *consumers* (the classes and the bulk
primitive exist; Layers 7–9 use them) · owner-linked multi-device replicas · MeshChatX panel (parked P7) · TLS.

**Hard rules carried in:** adopt, don't reinvent (every component below the fabric is adopted) · Reticulum is a
connected service, never a dependency (D17 #12; nothing in this layer imports RNS) · no wall clock in any cryptographic
or ordering decision (L2 rule 15) · no runtime downloads · no telemetry · no policy limits — no rate caps, no expiry, no
per-sender quotas; limits are capacity and physics (README §5) · delivery is non-negotiable, speed optional.

## 2. Topology & components

| Component | What | Source | Runs on |
|---|---|---|---|
| `mesh111` crate | fabric library (§3–§12), `KeyProvider` seam, store schema, `Link` trait | custom (this layer) | every node running Layer 5 (Cat 2–5) |
| `111d` `daemon/fabric/` | wires `mesh111` to identity, links, store dir, API, SSE, services registry | custom glue (Layer 1 crate, D15-style sanction) | same |
| `111d` `daemon/panels/` | panel listeners + reverse proxy + ticket auth (§13) | custom glue; proxy via `hyper-util` (already in the graph) or `axum-reverse-proxy` (MIT) — decided at build by size | nodes with the dashboard |
| SQLCipher via `rusqlite` | encrypted store engine (§10) | adopted: `knowledge/projects/rusqlite.md`, `sqlcipher.md` | same |
| `blake3` + `bao-tree` | content addressing + verified streaming of bulk objects (§11) | adopted: `knowledge/projects/blake3-bao.md` | same |
| `meshmud` | Meshtastic multiplexer: one serial/TCP uplink, TCP + HTTP downlinks | adopted: `knowledge/projects/meshmud.md` (MIT, Go; vendored, pinned) | nodes with a Meshtastic board (§14) |
| `meshtastic-presence` | tiny read-only presence reader on `meshtastic-python` | sidecar, GPL-3.0, own dir `os/messaging/sidecars/meshtastic-presence/`, own hash-locked venv | same |
| Meshtastic web client | the hosted panel (static bundle) | adopted: `knowledge/projects/meshtastic-web.md` (GPL-3.0-only; served unmodified) | same |

**Ports (loopback unless stated):** dashboard 11180 · model 11181 · agent 11182 · **fabric `tcp-lan` 11183** (LAN-bound
only when the node is enrolled as a hub or has LAN sharing on, §9) · panel listeners **11190–11229** (bound where the
dashboard is bound) · meshmud TCP downlink 11230 · meshmud HTTP downlink 11231 (panel upstream).

**State (`/var/lib/111`, the only writable tree):** `fabric/store.db` (SQLCipher, 0600) · `fabric/blobs/<blake3>/`
(0700) · `fabric/presence/meshtastic.json` (reader → daemon, 0644 within 0700) · `state/panels.json` (slot assignments)
· `state/interfaces.toml` (Layer 6's; this layer reads `owner` and Meshtastic bindings) · venvs under `venvs/`.

Process shape: `meshmud` and `meshtastic-presence` are supervised children of `111d` exactly like `rnsd` and
`111-agent` (`supervise.rs` `ChildSpec`, back-off, `stop_and_wait`, restart route). The fabric itself is in-process.

## 3. The envelope (normative)

The fabric's unit on the wire is one **sealed packet** (`id111::seal`, L2 §6.4, crypto-mode `0x01`) with this layer's
fields in the places L2 reserved. Byte order LE unless stated; varints are unsigned LEB128.

```
[L6 shim, 1 B — ONLY on broadcast bearers, never on tcp-lan; outside the sealed packet; mutable; never in AD]
   bits 7–6 frame_type (0 reserved, 1 sealed, 2 public: announce / card request, 3 foreign) · bits 5–3 hop_limit · bits 2–0 reserved (0)

[sealed packet — L2 header, AD-covered byte-exact]
   version      1   0x01 (Layer 5 owns this field; unknown ⇒ drop, fail closed)
   crypto_mode  1   0x01 (L2 registry; unknown/weaker ⇒ reject before decrypt)
   counter      8   per-peer send counter (L2 §6.5 state, `CounterStore`)
   extra_len    1   5
   extra        5   prio_flags(1) ‖ dest_hint(4)
   epk          32
   ciphertext   AEAD(inner)       tag 16
```

`prio_flags`: bits 7–6 **relay priority** (0 control · 1 interactive · 2 bulk-small · 3 bulk-large) · bit 5 GROUP (v1
MUST be 0) · bit 4 RATCHET (v1 MUST be 0) · bits 3–0 reserved, MUST be 0 — a receiver rejects any set reserved bit.
`dest_hint` = recipient NodeId[0..4]: a **locator** for relays and mailboxes, never an authenticator (L2 B2).

**Inner plaintext (v1):**

```
   sender_ref   4   sender NodeId[0..4] (resolved against contacts; collisions ⇒ try each candidate's pair key)
   kind         1   §3.2 registry
   flags        1   bit0 HAS_FRAG · bit1 HAS_EXT · bit2 PIGGYBACK_ACK · bit3 SIGNED · bit4 RECEIPT_STRONG ·
                    bit5 INTRO · bit6 RATCHET(0) · bit7 reserved(0)
   msg_seq      varint   per (sender, stream) sequence (§4); stream 0 = the 1:1 conversation
   [frag_idx, frag_count]  varint ×2            if HAS_FRAG (§5)
   [ext TLVs]              type(1) len(1) value  if HAS_EXT (unknown type ⇒ skip; 0x10 ratchet pubkey reserved,
                                                 0x20 delegation trailer ⇒ REJECT in v1 per L2 §6.7)
   [card]                  137–169 B             if INTRO: the sender's own announce (§7)
   payload      …
   auth         pair_mac(16)                     unless SIGNED
                sig(64)                          if SIGNED (replaces pair_mac)
```

**`pair_mac` (sender authentication, L2 amendment A2):** the L2 construction is Noise pattern "N" — the recipient
learns that *someone holding its announced key* sealed the packet, not who. `pair_mac` closes that:
`key = BLAKE2b-256("111/pair/v1" ‖ X25519(sender_enc_sk, recipient_enc_pk) ‖ sender_enc_pk ‖ recipient_enc_pk)`,
`pair_mac = BLAKE2b-128(key = key, data = L2 header bytes ‖ epk ‖ inner bytes before auth)`. Keyed BLAKE2b in MAC mode.
The receiver MUST verify `pair_mac` (or `sig`) **before** touching counter/window state or any store — a forged
`sender_ref` must never advance the claimed sender's window (L2 §6.5 windows are keyed by the *authenticated* sender).
`pair_mac` is deniable (either party could have produced it); that is intended for routine traffic (L2 rule 13).

**`sig` (authorship, transferable):** Ed25519 over `"111/msg/v1" ‖ sender NodeId ‖ stream(varint) ‖ msg_seq(varint) ‖
kind ‖ BLAKE2b-256(payload)`. Covers content, not the sealed packet, so it survives re-sealing and re-fragmentation.
**Mandatory** for every stored-for-others, public or code class: group posts, anything a hub holds on behalf of others
(custody items keep the original sealed bytes, so the sender's choice stands), `KB`, `UPDATE`, `PATCH`, `BROADCAST`.

**Sizes (what fits):**

| Bearer payload | Outer (16 hdr + 32 epk + 16 tag) | Inner hdr (≈7–9) + `pair_mac` | Payload, pair-auth | Payload, SIGNED |
|---|---|---|---|---|
| 160 B (flood floor: MeshCore `GRP_DATA` 163 B) | 64 | 23–25 | **≈ 71–73 B** | ≈ 23–25 B |
| 233 B (Meshtastic `PRIVATE_APP`, RNode single frame ≈ 254) | 64 | 23–25 | **≈ 144–146 B** | ≈ 96–98 B |
| TCP (`tcp-lan`) | 64 | 23–25 | up to 65,535 − 89 per frame | — |

If the L2 panel rejects amendment A2 (4 B `sender_ref` instead of the 32 B inner NodeId as built), subtract 28 B from
every payload column (41–45 B at 160 B). The spec is written for A2; the codec keeps the 32 B form behind a feature gate
until the amendment lands.

### 3.1 Domain tags (registered with L2 §6.1)

`111/pair/v1` · `111/msg/v1` · `111/ack/v1` · `111/ack-sig/v1` · `111/custody/v1` · `111/custody-sig/v1` ·
`111/enroll/v1` · `111/store/v1` (HKDF info, §10.2) · `111/obj/v1` (object adverts, §11).

### 3.2 Kinds (1 B; unknown ⇒ drop and log, never best-effort)

| Kind | Name | Direction | Priority | Note |
|---|---|---|---|---|
| 0x01 | `MSG` | peer ↔ peer | interactive | human message; body = CBOR (§3.3) |
| 0x02 | `ACK` | recipient → sender | control | §6.1 |
| 0x03 | `CUSTODY_PUT` | node → hub | by inner item | §9 |
| 0x04 | `CUSTODY_ACK` | hub → node | control | §6.2 |
| 0x05 | `MAILBOX_FETCH` | node → hub | control | §9 |
| 0x06 | `MAILBOX_ITEM` | hub → node | by item | §9 |
| 0x07 | `MAILBOX_COMMIT` | node → hub | control | §9 |
| 0x08 | `ENROLL` / 0x09 `ENROLL_OK` | node ↔ hub | control | §7.3 |
| 0x10 | `AGENT_OP` | node ↔ node | interactive | agent-to-agent operations (consumers: Layers 7, 9); SIGNED |
| 0x11 | `KB` · 0x12 `UPDATE` · 0x13 `PATCH` · 0x14 `SYNC` | — | bulk | classes defined here; adverts + chunks via §11; consumers later |
| 0x20 | `OBJ_ADVERT` · 0x21 `OBJ_REQ` · 0x22 `OBJ_CHUNK` | — | bulk | §11 |
| 0x30 | `BROADCAST` | one → all | interactive | **reserved** (public signed frames are a Layer 7 format; §12) |

### 3.3 Message body (`MSG`, Cat 2+)

Deterministic CBOR (RFC 8949 §4.2) via `ciborium`: `{1: text, 2: [attachment refs], 3: sent_at?, 4: reply_to?}`.
`sent_at` is the sender's **claim**, shown as "unverified sender time" (§4). Attachment ref = `{hash(32 BLAKE3),
size, name?, object_key(32)}` — the object itself moves by §11. Relays never parse bodies; Cat-1 never sees them.

## 4. Message identity, ordering, time

- **`msg_id = (sender NodeId, stream, msg_seq)`.** `msg_seq` is a strictly increasing per-(sender, stream) counter,
  persisted with the store; wire cost 1–3 B. Receiver state per (sender, stream): a contiguous delivered prefix plus a
  bounded range list (≈ 24–48 B). **Below the prefix nothing is ever forgotten**: a duplicate is re-ACKed and never
  redelivered — this is the content-level idempotency L2 §6.5 hands to this layer, and it also covers flood-mesh
  duplicates. Random ids and content hashes are rejected (both forget under pruning and then redeliver).
- **Restore / unclean loss:** `msg_seq` jumps forward by 2^32 (L2 §8.4 amendment A3); the receiver stores the gap as
  one range entry.
- **Blobs** are content-addressed by BLAKE3 root (§11); `object_id` never identifies a message.
- **No timestamp field anywhere in the envelope.** Order within a conversation = `msg_seq` per direction plus
  `seen_peer_seq` (ext TLV 0x01, varint: the highest `msg_seq` the sender had seen from the peer), which gives
  happens-before across directions. Display time = **local receipt time, labelled** ("received"); `sent_at` from the
  body is shown as "unverified". Threads are sorted by receipt sequence, never by claimed time.
- **Dedup at relays** (Cat-1, mailboxes, flood bearers) uses `epk[0..8]` — fresh per sealed packet, free — in a
  256-entry ring (2 KB). That is flood suppression, not idempotency.

## 5. Fragmentation (three tiers)

| Tier | When | How | Overhead |
|---|---|---|---|
| **0** | message fits one bearer frame (the chat case) | one sealed packet, no `HAS_FRAG` | 0 |
| **1** | ≤ 16 fragments (≈ 1.1 KB at the 160 B floor, ≈ 2.3 KB at 233 B) | **fragment-then-seal**: every fragment is a complete sealed packet (`frag_idx`, `frag_count`), independently authenticated, resumable, no fragment poisoning. Sender keeps ≤ **16 outstanding** unACKed fragments per peer and **re-seals on retransmit** (fresh epk + counter), so the L2 32-entry window never starves. Reassembly at the destination only; the receiver ACKs fragments by SACK (§6.1). | ≈ 35–45 % at 160 B; the price of verifiable fragments |
| **2** | larger objects and every bulk class | sealed **`OBJ_ADVERT`** (`{hash, size, chunk_group, coding, object_key?}`, SIGNED when the class requires), then **hash-verified chunks** (§11): `OBJ_CHUNK` frames carry `session_tag(8) ‖ chunk_idx ‖ bao-verified bytes`, **unsealed** when the object is public (`KB`, `UPDATE`, `PATCH`, `BROADCAST` attachments) or already encrypted to its recipient (private attachments encrypted once under `object_key`), sealed otherwise. Needs L2 carve-out A5. | ≈ 22 B per chunk |

**Seal-once-then-fragment is rejected** (raw slices cannot be verified; one bad slice forces a whole-message
retransmit; counters per fragment would be unavailable). Erasure coding: `coding` field reserved (0 none · 1 Reed-Solomon
· 2 RFC 6330); none built (RaptorQ only after IPR review).

**MTU profiles:** the fabric sizes fragments from the *destination path's* profile exposed by the link (`mtu_bytes`,
`reliable`, `broadcast`); relays never re-fragment sealed packets. Default profile when unknown: **160 B**.

## 6. Receipts

### 6.1 Delivery ACK (`kind = ACK`)

Payload: `ack_prefix(varint) ‖ n_ranges(1 ≤ 4) ‖ [start, len](varint ×2)* ‖ [strong: n(1) ‖ (seq varint, BLAKE2b-128(payload))*]`
for the (sender, stream) being acknowledged — the stream is the ACK's own stream (0 for 1:1). Authentication is the
packet's own `pair_mac` (or `sig` when the ACK is requested transferable: `111/ack-sig/v1`, 64 B, on demand only);
`kind` is inside the authenticated region, so a data packet cannot be replayed as an ACK. `RECEIPT_STRONG` on a message
asks for the payload hash in the ACK (proof the recipient decrypted *this* content; used for `AGENT_OP`, `PATCH`).
ACKs piggyback on reverse-direction data (`PIGGYBACK_ACK`: the ACK payload is appended before `auth`). Duplicates are
re-ACKed. **Only the recipient's ACK moves a message to `delivered` and releases the sender's copy.** Hop-by-hop link
ACKs are the link's business (Layer 6).

### 6.2 Custody receipt (`kind = CUSTODY_ACK`)

Payload: `status(1: 1 ACCEPTED · 2 DUPLICATE · 3 REJECT_FULL · 4 REJECT_UNENROLLED) ‖ custody_seq(varint) ‖
pkt_ref(16) = BLAKE2b-128("111/custody/v1" ‖ exact sealed bytes of the item)`. Hashing the exact bytes forces the
custodian to hold them. `ACCEPTED` from a **mailbox-role** node is the only evidence for the UI state `relayed`. A
transferable form (`111/custody-sig/v1`, 64 B) exists on request.

## 7. Contacts, cards, announces

Keys are not derivable from a NodeId (L2 §2: the X25519 key is an independent derivation), so a sender needs the peer's
**announce** (L2 §6.2, 137–169 B, self-authenticating). v1 emits announces **on demand only** — in a card, on link-up,
inside an INTRO — never on a timer (L2 amendment A6).

### 7.1 Contacts table (store)

`node_id(32)` · `enc_pubkey(32)` · `announce_counter` · `trust` (`oob` card/QR · `tofu` INTRO · `enrolled` hub ↔
member) · `alias` · `roles` (from `appdata`) · `home_hub` hint · `first_seen_seq` · `last_seen_seq` · `request`
(pending message request) · `pinned` (never pruned). **This table is authoritative for user-added peers**; L2's
256-entry pin store is a cache of it (L2 amendment A7). Key material is immutable once pinned (L2 §6.3): a card or
announce with different keys for a known NodeId is rejected and surfaced.

### 7.2 Three ways in

1. **Card** — the node's own genesis announce as a binary QR (≈ v7) or base32 text (~220 chars; bech32 is for the
   NodeId only). `GET /api/v1/contacts/me/card`. Scanning a card adds the contact with `trust = oob`. Works with zero
   connectivity.
2. **`node1…` only** — typed or scanned (63 chars). The node stores the NodeId and sends a **cleartext card request**
   (`tcp-lan` frame `0x04`, §8.1: the wanted NodeId; it cannot be sealed, since the requester has no key yet) over every
   link; a hub answers from its contacts cache (announces are public, immutable key material) or forwards the request to
   the member when connected; the reply is an announce frame, verified **against the expected NodeId**, then pinned. Until then messages stay `queued`, reason "waiting for their key". (Delta Chat SecureJoin shape:
   one out-of-band fact, the rest over an untrusted channel.)
3. **INTRO** — a first message to a never-contacted peer sets `INTRO` and carries the sender's announce inside the
   ciphertext (so it is SIGNED by construction of the announce). The receiver files a **message request**
   (`trust = tofu`); the user accepts (contact created, message delivered to the thread) or ignores (kept in the
   request list, never auto-pruned while pending, no ACK sent). Accepting a request never ACKs retroactively more than
   the messages shown.

### 7.3 Enrollment with a home hub

The hub shows a one-time **enrollment code** (8 chars, same generator and rate limit as the L4 pair code: `111 fabric
offer` or Settings → Messaging & Data → "Let a node use this hub") together with its card and `tcp-lan` address as a
QR. The member sends `ENROLL {card, code}` sealed to the hub; the hub verifies the code (`111/enroll/v1` keyed hash,
constant-time), stores the contact as `enrolled`, answers `ENROLL_OK {hub card, custody_seq_floor}`. Enrollment is the
owner's **access list**, not a rate policy; a hub refuses `CUSTODY_PUT` from unenrolled nodes with
`REJECT_UNENROLLED`. A node has one home hub in v1 (replicas = deferred A+).

`appdata` v1 (L2 amendment A8): `[ver=1][roles u16 LE][reserved]` — bit 0 mailbox (accepts custody) · bit 1 always-on
hub · bit 2 LoRa-attached; optional 8 B truncated home-hub id (locator only). Layer 7 appends TLVs after v1.

## 8. Links

```rust
trait Link {
    fn id(&self) -> LinkId;                 // stable per adapter instance
    fn kind(&self) -> LinkKind;             // TcpLan | KissModem | MeshtasticClient | RnsBridge | Ble
    fn profile(&self) -> PathProfile;       // mtu_bytes, framing (Stream|Datagram), reliable, broadcast,
                                            // cost_class (Bulk|LowBandwidth), airtime (advertised physics: duty cycle,
                                            // bits/s) — never enforced by the fabric
    fn peers(&self) -> Vec<PeerAddr>;       // opaque per adapter
    async fn send(&self, to: PeerAddr, frame: Frame) -> Result<(), LinkError>;
    async fn recv(&self) -> (PeerAddr, Frame);
    fn state(&self) -> LinkState;           // feeds transport_state / DeviceState
}
```

The fabric never rate-limits; airtime and duty-cycle are facts the adapter advertises and the agent can read.

| Adapter | Drives | Frame budget | Status | Notes |
|---|---|---|---|---|
| **`tcp-lan`** | TCP between 111 nodes: member ↔ hub (D17 #4), hub ↔ hub, laptop ↔ hub | stream; frames ≤ 65,535 B | **built with L5** | §8.1. Unicast: no MulticastLock needed on phones. No Noise/TLS in v1 (end-to-end seal; metadata leak stated in §18). |
| `kiss-modem` | a 111-owned board running the **MeshCore KISS modem** (D19 c): 111d builds MeshCore `GRP_DATA` frames; stock MeshCore repeaters flood them | **160 B** payload per frame (163 B cap) | seam; Layer 6 builds | Region set once (D17 #10). Opt-in "shared" profile: an RNode in HOST mode, rnsd attached by loopback TCP-KISS. One owner per port (`interfaces.toml` `owner`). |
| `meshtastic-client` | a Meshtastic board via meshmud's TCP downlink, port `PRIVATE_APP` | 233 B (221 with PKI/AEAD channels) | seam; no bridge this cycle | The firmware rewrites `from`: a bridge speaks as its own Meshtastic node; one bridge identity per foreign network bound by L2 §10 attestation (A10). Routers in `CORE_PORTNUMS_ONLY` drop it. Never on amateur-licensed regions (sealed = "obscured", devices.md §0.6). |
| `rns-bridge` | LXMF via MeshChat(X) HTTP/WS or lxmd's inbound hook — envelope as an attachment; LXMF address = transport address, our NodeId stays inside | 287 B single packet, larger via Link | seam; deferred | No RNS import, ever. |
| `ble` | Android helper app, later | ~ATT MTU | later | — |
| online links (Noise IK / TLS-RPK, L2 §6.6) | interactive class, L3 offload over the mesh | stream | later (Layer 6) | **Link keys are random and bound by L2 §10 attestation — never seed-derived (F2 rejected), never the identity key (§6.1).** iroh is not in v1 (QUIC-only, +13 MB, blobs pre-production). |

### 8.1 `tcp-lan` wire (normative)

TCP; listener `:11183` (LAN-bound when the node is a hub or LAN sharing is on; loopback is never a peer). Both sides
send `hello = "111F" ‖ 0x01` then frames: `type(1) ‖ len(2 BE) ‖ body`. Types: `0x01` sealed packet · `0x02`
announce (cleartext, verified with `id111::process_announce` before any use; this is the on-demand link-up announce) ·
`0x03` keepalive (empty) · `0x04` card request (32 B NodeId; answered with a `0x02` announce frame from cache or by
forwarding, §7.2). Each side sends its announce first; a peer whose announce fails verification is dropped.
Reconnect on link events with back-off; no timers in the protocol itself (keepalive interval is a transport detail).
The stream leaks `dest_hint`, counters and sizes to an on-path observer (§18); content and sender are sealed.

## 9. Mailbox protocol (hub role)

A hub = any node the owner enrolls others with (Cat 4 by default; an always-on laptop may opt in; phones opt in
explicitly). It holds **custody**: sealed items it cannot read, for recipients that are away.

- `CUSTODY_PUT {dest NodeId(32) ‖ sealed item bytes}` sealed to the hub (the double seal costs ≈ 90 B — irrelevant on
  TCP; on LoRa the hub link is the mailbox's own `kiss-modem`, Layer 6). Hub: dedup by `pkt_ref`, store
  `(dest, bytes, custody_seq)`, reply `CUSTODY_ACK` (§6.2). `REJECT_FULL` when the store refuses new custody (§10.4);
  the sender keeps the item `sent` with reason "mailbox full".
- `MAILBOX_FETCH {cursor varint}` from a member: the hub streams `MAILBOX_ITEM {custody_seq ‖ bytes}` for that member's
  NodeId above the cursor. `MAILBOX_COMMIT {cursor}`: the member has stored everything up to it; the hub marks those
  items **evict-first** (not deleted: capacity decides). Cursors are the hub's local insertion sequence — no clock.
- Members fetch on link-up and on `fabric` events; hubs push `MAILBOX_ITEM` proactively to connected members.
- **Hub-to-hub replication (A+)** is specified as: configured peer hubs exchange per-(dest) summary vectors (the §4
  prefix + ranges form) and replicate custody items; replicas are evict-first. Not built in v1.
- The hub never opens items; it sees `dest`, size, `prio_flags`, `dest_hint`, counters.

**States (UI, normative evidence):** `queued` (outbox; no path, or key missing — reason shown) · `sent` (frame written
to a link; LoRa: modem accepted TX; a repeater echo is detail here) · `relayed` (authenticated `CUSTODY_ACK ACCEPTED`
from a mailbox-role node — "a mailbox node saved it and will hold it for them") · `delivered` (recipient `ACK`).
Retries happen on link and contact events, never on wall-clock timers; the sender's copy is kept until `delivered`.

## 10. Store

### 10.1 Engine (D19 e)

`rusqlite` with `bundled-sqlcipher-vendored-openssl` (SQLCipher 4.x community, OpenSSL 3.6 vendored; musl-static on
x86_64 and aarch64 — measured +6.4 / +5.4 MB). **File format pinned to SQLCipher 4** (`PRAGMA cipher_compatibility =
4`); 5.0 migration planned via `cipher_migrate` behind the Layer 1 updater. Raw key (`PRAGMA key = "x'…'"`, no
PBKDF2 at open), WAL, `temp_store = MEMORY` (transient files are not encrypted), `auto_vacuum = INCREMENTAL` so pruning
frees disk, `secure_delete = ON`. One DB: `fabric/store.db`, 0600, owner `node111`.

### 10.2 Key custody (`KeyProvider` seam)

- **Mode S (default; Cat 2/3/4 without a TPM; phones):** `store_key = HKDF-SHA256(ikm = L2 keyset, salt = NodeId,
  info = "111/store/v1")`, 32 B, computed by the identity holder and handed to the store as a raw key; never written
  to disk. A DB copied elsewhere opens again after restore-from-words. Limit, stated honestly: protects DB-only copies,
  backups and synced app dirs — not a stolen disk with a readable identity dir (L2 §12 residual). L2 amendment A9.
- **Mode T (Cat 4 with TPM2 / SE050B):** a random 32 B key sealed in hardware (L2 §5: the TPM holds the *storage* key,
  never the keyset); Mode S is the documented recovery path if the hardware is lost.
- Never bind the key to an OS keyring without a recovery path (Signal Desktop lesson).

### 10.3 Schema (v1, migrations via `PRAGMA user_version`)

`contacts` (§7.1) · `messages` (`sender`, `stream`, `seq`, `dir own|in|transit`, `kind`, `class`, `size`, `local_seq`
(the age hint), `state queued|sent|relayed|delivered`, `received_local_seq`, `body` or blob refs, `pinned`) ·
`outbox` (sealed bytes awaiting a path, per fragment, outstanding count) · `rx_state` (`(peer, stream)` → prefix +
ranges) · `custody` (hub: `dest`, `custody_seq`, `pkt_ref`, bytes, `committed`) · `cursors` (member: hub → cursor) ·
`blobs` (`hash`, `size`, `refcount`, `public`, `object_key?`) · `requests` (pending INTROs) · `settings`.
Bodies of `MSG` live in the DB (small); attachments and every bulk object are files under `fabric/blobs/` named by
BLAKE3 root — private objects are encrypted once with their `object_key` (XChaCha20-Poly1305 in 64 KiB chunks) and
addressed by **ciphertext** hash, so relays and hubs verify what they carry without reading it; public signed objects
(`KB`, `UPDATE`, `PATCH`) are stored plain.

### 10.4 Pruning (capacity only — normative order)

Trigger: live bytes (DB pages minus freelist + blob dir) above the high watermark **or** free disk below a 1 % floor.
Order: (1) transit custody that is `committed` and public chunks that are re-fetchable — weight = size × age (old and
large first; age = `local_seq` distance, never a clock) · (2) superseded release/KB objects below the Layer 1 monotonic
floor, fully fetched deltas · (3) own conversation history oldest-first **only at hard capacity and only after an
export offer** (§21: `age`-based export) · (4) **never** undelivered outbound, pending requests, pinned items or
contacts — instead the store refuses new custody and new bulk, and surfaces `store_full` on the `fabric` service and to
the agent. Per-class quotas exist as capacity shaping (configurable); per-sender limits and stamp costs are policy and
do not exist.

## 11. Bulk objects (the data half)

One content address and one verified-stream primitive for every link tier: **BLAKE3 root** + **bao-tree** outboard
(chunk group 1 KiB on LoRa/BLE: 6.25 % outboard; 16 KiB on TCP: 0.39 %; the root is identical across groups, so a
chunk fetched over LoRa and one over TCP verify against the same address). Flow: `OBJ_ADVERT` (sealed; SIGNED for
public classes; carries `object_key` for private ones) → `OBJ_REQ {hash, ranges}` → `OBJ_CHUNK` frames (§5 Tier 2),
range-set requests so different carriers can bring different pieces, resume at any chunk boundary. The fabric treats
bulk as `bulk-small` (≤ 64 KiB) or `bulk-large`; bulk classes route to `cost_class = Bulk` links by default and cross
a `LowBandwidth` link only when the user or the agent explicitly asks (physics: a 32 KB KB delta is 5–9 min of LoRa
airtime at 1 % duty; a 5.7 MB binary is 15–27 h). Consumers: attachments in `MSG` (v1), `KB` deltas and git bundles
(Layer 7), `UPDATE` payloads (Layer 1's minisign manifest stays the trust root), `PATCH` (Layer 9), `SYNC` ops for
Layer 8's append-only log (opaque, idempotent ids, per-peer cursors — seam only).

## 12. Groups & broadcast

**Groups v1 = pairwise fan-out**: N sealed packets, each pair-authenticated and forward-secret, per-member ACK, exact
delivery tracking, no shared key state, trivial removal. A group is a local object (`group_id`, members, name); the
`GROUP` flag and a `stream_id` TLV are **reserved** for sender keys (G1: 20 B overhead + 64 B signature per message;
rotation = each member re-distributes its sender key, no clock). MLS only behind a hub delivery service, later. Static
PSK channels (Meshtastic-style) are rejected. **Public broadcast** (`BROADCAST`, kind reserved) is a public *frame*, not a sealed packet: cleartext by
design, always SIGNED, relayed like an announce; a node shows it as a broadcast, never as a message from a contact.
Its byte format is pinned with Layer 7's presence frames, not here.

## 13. Hosted panels (D19 d; amends D15)

A **panel** is an adopted project's own web UI, shown inside the dashboard on the service's LIVE page. Rule: **the
dashboard frames only registry-declared panels, each on its own origin.** Nothing else is ever framed.

- **Declaration:** `assets/panels/<id>.toml` (embedded in the release; hash-pinned):

```toml
id = "meshtastic"                     # = ServiceId and KB link
title = "Meshtastic"
kb_entry = "projects/meshtastic-web"
[source]  kind = "static"             # static | child | detected
          bundle = "panels/meshtastic-web"   # unpacked, hash-pinned in the channel manifest; SPA fallback
[entry]   path = "/"
[health]  tcp = 11231
[frame]   allow = ["serial", "bluetooth"]   needs_secure_context = false   sandbox = "default"
          secure_note = "Connecting to a radio over this browser's USB or Bluetooth works only when the dashboard is opened on the node itself."
[proxy]   websocket = false  sse = false  host_header = "upstream"  max_body = "8MiB"
[[proxy.routes]]  path = "/api/v1/"  upstream = "http://127.0.0.1:11231"   # meshmud HTTP downlink
[auth]    mode = "111-session"        # 111-session | loopback-only
[license] spdx = "GPL-3.0-only"  served = "unmodified"  source_offer = "THIRD-PARTY-NOTICES.md"
```

- **Listeners:** one per *enabled* panel from the block 11190–11229, slot stable in `state/panels.json`, bound
  exactly where the dashboard is bound (loopback; LAN when LAN sharing is on). Origin = `http://<host as typed>:<port>`.
- **Auth (ticket):** the dashboard calls `POST /api/v1/panels/{id}/session` (needs the dashboard session or a trusted
  loopback) and receives a one-time ticket URL (128-bit, single use, ~30 s). The panel listener swaps it for the
  host-only cookie `p111_<id>` (HttpOnly, SameSite=Strict, no `Domain`, Path=/) and redirects to `entry.path`. Skipped
  only where `loopback_trusted` (D16 a: never under Termux). `111d` strips `s111` and every `p111_*` cookie from
  upstream requests and adds `X-Forwarded-For/Host/Proto`.
- **Guards on every panel request:** exact Host match (the dashboard's host guard list); for non-GET, `Sec-Fetch-Site`
  ∈ {same-origin, none} or `Origin` host == Host — a drive-by page cannot blind-POST into an app with no CSRF
  protection of its own.
- **Response rewrite:** drop `X-Frame-Options`; parse every `Content-Security-Policy[-Report-Only]`, remove its
  `frame-ancestors`, then append `Content-Security-Policy: frame-ancestors <dashboard origins on this host: :11180
  and :80>`; strip `Domain=` from upstream `Set-Cookie`; pass everything else through; **never rewrite bodies**; never
  apply the dashboard's own CSP middleware to panel responses.
- **Streaming:** forward WebSocket upgrades, SSE without buffering, bodies up to `max_body`.
- **Dashboard side:** the LIVE service page gains a **Panel** tab: `<iframe src=<ticket URL> sandbox="allow-scripts
  allow-same-origin allow-forms allow-popups allow-modals allow-downloads" allow="<frame.allow, origin-scoped>">` (never
  `allow-top-navigation`), frame chrome ours (title, health dot, reload, "Open full client" in a new tab, "Ask the agent
  about this panel"). The dashboard CSP keeps `frame-ancestors 'none'` and gains an exact `frame-src` list of enabled
  panel origins. No theming promise: adopted UIs are not restyled; `111-panel/1` postMessage (`ready`, `title`,
  `theme`) is reserved for panels we may write later, `event.origin`-checked.
- **Secure-context gate:** if `needs_secure_context = true` and the dashboard origin is not potentially trustworthy
  (LAN IP, `111.local`), the tab shows "open the dashboard on the node itself" with the loopback URL instead of a frame.
  For Meshtastic (`false`): the panel always frames; its Web Serial/Bluetooth buttons work only on loopback origins and
  the `secure_note` says so.
- **Offline lint (build gate):** every panel runs in a no-internet netns; any outbound fetch fails adoption.
- **Health:** manifest `health` (GET expecting 2xx/3xx, or TCP connect), 2-tick debounce like every service.
- **Contract:** `ServiceRecord` gains `panel: {id, origin, entry, needs_secure_context, allow[], state} | null` (additive;
  `embed_url` stays for link-out). `GET /api/v1/panels` lists declared panels and slots.

What stays from D15: one origin for our own UI and `/api/v1`; `frame-ancestors 'none'` + `X-Frame-Options: DENY` on
dashboard responses; no runtime fetch of anything; **no TLS in v1** (Chrome 154's HTTPS-by-default excludes private
addresses; `.local` re-checked at the next Layer 4 pass); the Host guard on the main listener; loopback-trust policy.
Rejected: sub-path hosting under the dashboard origin; body rewriting; wildcard DNS or per-panel `.local` names
(`<id>.111.localhost` may be added later as an additive prettification on loopback).

## 14. Meshtastic (service `meshtastic`, D19 g)

Goal this cycle: a Meshtastic board is **identified** (consented), gets a **service row** whose page hosts
**Meshtastic's own web client**, and its **nodes-heard** feed **Nearby**. No message bridge. "Read-only" means: 111
never transmits on the Meshtastic mesh and never changes node configuration; config *reads* (`want_config`) are fine.

- **One API client.** A Meshtastic node (and meshtasticd) serves one client. **`meshmud`** holds that slot: uplink =
  the board's serial port (USB boards) or `tcp://127.0.0.1:4403` (meshtasticd for SPI/CH341 radios); downlinks = TCP
  `127.0.0.1:11230` (many clients: the presence reader, the CLI, later `meshtastic-client`) and HTTP
  `127.0.0.1:11231` (**one** client: the panel). Config rendered by `111d` from `state/interfaces.toml`; meshmud is a
  supervised child; vendored and version-pinned source, CI-built static binaries in the channel manifest.
- **Presence reader** (`meshtastic-presence`, GPL-3.0 Python, own source dir + hash-locked venv like `rns-lock.txt`):
  connects to the TCP downlink, sends `want_config` once, subscribes to NodeInfo / position / telemetry events, writes
  `fabric/presence/meshtastic.json` atomically: per node `{id, num, long_name, short_name, pubkey?, snr?, rssi?,
  hops_away?, last_heard_hint (node clock, display only), position?, channel_utilization?}`. It never sends text, admin
  or any user packet. `111d` reads the file on change → `GET /api/v1/nearby` + SSE `nearby`.
- **Identify branch** (D17 #8, the Layer 6 identify sequence, Meshtastic step built now): `POST /devices/{id}/identify`
  (consent recorded per device) → port lock → `rnodeconf --info` (rns venv, if present) → **start meshmud on the port
  with an identify profile and read `want_config` once through the TCP downlink** → MeshCore query (`meshcore-cli`,
  when Layer 6 adds it). Result sets `DeviceRecord.firmware {family, version, source: probe}` and, for Meshtastic,
  binds the board: `driver.stack = meshtastic`, `service_id = meshtastic`. DTR/RTS reset risk is tied to this one
  consented step; meshmud's serial open behaviour is a hardware gate (§20).
- **Service row** `meshtastic` (kind `child`): LIVE when meshmud is running **and** the reader has produced a node DB;
  NOT SET when a Meshtastic board is identified but not bound ("Set up with the agent"); ERROR when meshmud crash-loops
  or the uplink is lost > 60 s. `detail`: board, firmware version, nodes heard, channel utilization, panel state.
  Actions: `restart` (child), `log`. The `radio` service keeps its meaning (rnsd-bound RNode, meshtasticd on SPI).
- **Nearby:** real list, grouped by source ("heard via Meshtastic on <board alias>"), fields above, `last_heard` shown
  as a hint. No fake peers; a Meshtastic node is never a contact (different identity system) — "Message them" is not
  offered until a bridge exists (P1).
- **Honest copy** on the service page (from `02-bridges-and-budgets.md` §4.3): "Meshtastic protects messages with its
  own keys; the default channel's key is public. Nothing here is 111-sealed." Meshtastic is a registered mark, used
  nominatively.
- **GPL boundary:** the web bundle is served unmodified from its own directory; the presence reader and meshmud are
  separate processes; no Meshtastic protobuf code enters our Rust binary. Source offer in `THIRD-PARTY-NOTICES.md`.

## 15. API contract (additive, `/api/v1`; binding form: `interface/api/openapi.yaml`)

| Route | Verb | Purpose |
|---|---|---|
| `/messages/threads` | GET | threads (peer, alias, last message, unread, states summary) |
| `/messages/threads/{peer}` | GET | messages with `state`, `received_seq`, `sent_at_claim`, attachments |
| `/messages` | POST | `{to, text, attachments?}` → 202 `{id}`; stays `queued` until a path exists |
| `/messages/{id}/retry` | POST | re-queue (`409` unless `queued|sent`) |
| `/contacts` · `/contacts/{id}` | GET · PATCH · DELETE | list; alias, accept request, pin; forget |
| `/contacts` | POST | `{card}` or `{node_id}` (§7.2) |
| `/contacts/me/card` | GET | own card: `{text, qr_png_base64?}` (QR rendered by `qrcode`, already a dep) |
| `/fabric` | GET | links (+profile, state), hub (enrolled, reachable, cursor), custody (count, bytes), store (used, watermark, `store_full`) |
| `/fabric/hub/enroll` | POST | `{address, code}` (member side) |
| `/fabric/hub/offer` | POST | hub side: mint an enrollment code (loopback-only like `offer`) |
| `/fabric/settings` | PATCH | `hub_role`, `listen_lan`, `custody_opt_in`, per-class quotas |
| `/nearby` | GET | §14 presence (per source) |
| `/panels` · `/panels/{id}/session` | GET · POST | §13 |
| `/devices/{id}/identify` | POST | §14 (consent flag in body; `409 busy` when the port is held) |

SSE (same `/events` stream, additive names): `message` (state change, new message, request), `contact`, `fabric`
(link/hub/custody/store), `nearby`, `panels`. Enum growth: `ServiceId` + `fabric`, `meshtastic` ·
`DeviceRecord.actions` + `identify` · `reason_code` + `store_full`, `mailbox_full`, `waiting_for_key`, `port_busy`.
Unknown enum values render as `unknown` (U1b rule).

**Services:** `fabric` ("Messaging"): LIVE when the store is open and (enrolled hub reachable, or acting as hub, or any
peer link up); NOT SET when the store is open but no hub/link ("Set up with the agent" → enroll); ERROR on store
failure or `store_full`. `meshtastic`: §14.

## 16. UI (Layer 4 tree; one component tree, density flags)

- **Messages** (`/messages`, core): thread list (alias/label, last line, state glyphs, unread), **requests** section
  (INTROs: accept / ignore), composer with attachment (size shown against the path profile: "over LoRa this takes
  about N minutes"), per-message state with the exact copy of §9 and the reason when `queued`, "received" time with
  the unverified sender claim behind a disclosure. Advanced appends: raw `msg_id`, path, custody receipts.
- **Contacts** (`/messages/contacts`): list, add by card/QR scan (camera via `getUserMedia` on loopback/secure
  origins; file upload fallback) or paste `node1…`, own card (QR + text + copy), pin/alias/forget.
- **Nearby** (`/nearby`): real list per source (§14); radar/map stay Layer 7.
- **Service pages**: `fabric` (links, hub, custody, store gauge, set-up prompts); `meshtastic` (LIVE panel from
  `detail` + **Panel tab** §13 + nodes heard + honest copy).
- **Settings → Messaging & Data** (`/settings/l5`): home hub (enrolled hub card, enroll by QR/code), "Let a node use
  this hub" (offer code; hub role toggle; LAN listener), custody opt-in, store usage + quotas, export (CLI instruction
  until §21's `age` export lands). Loopback-only rows disabled over LAN like Essentials.
- **Agent prompts** (one `buildPrompt`, context goals): `messaging.setup` (enroll with a hub), `messaging.fix`
  (`store_full`, hub unreachable), `meshtastic.setup` (identify → bind), `panel.explain`.

## 17. Agent tools (Layer 3 set grows; fixed, shell-free, schema-validated — D10)

| Tool | Kind | Shape | Gate |
|---|---|---|---|
| `messages.read` | read | `{peer?, since_seq?, limit}` → threads/messages with states | none |
| `messages.send` | write intent | `{to, text}` → **proposal** `kind: message`; the human accepts in the UI, `111d` sends | human accept; never autonomous |
| `contacts.read` | read | → contacts + requests | none |
| `fabric.read` | read | → links, hub, custody, store (same as `GET /fabric`) | none |
| `nearby.read` | read | → §14 presence | none |

The agent reaches these through `111d`'s loopback `/api/v1` (a `daemon_api_url` config joins `daemon_health_url`);
tools run inside `111-agent`, writes only via the proposal path (L3 "only write path" rule preserved).

## 18. Security & threat model (hard rules)

1. Verify before state: `pair_mac`/`sig` → counter/window → dedup → store. A failure at any step leaves no trace
   except a bounded log counter.
2. Fail closed on unknown `version`, `crypto_mode`, reserved bits, unknown `kind`, non-empty delegation trailer.
3. The keyset never enters `mesh111`; sealing, opening, signing and the pair-key ECDH happen behind the identity
   holder's handle inside `111d`. No key material on any HTTP endpoint, ever.
4. Metadata stated honestly: `dest_hint` (4 B), counters, sizes and `prio_flags` are cleartext; `tcp-lan` has no
   transport encryption in v1, so an on-path observer learns who talks to which hub and how much. Content, sender and
   kind are sealed. Hubs see `dest`, sizes and timing of custody items.
5. Forward secrecy = L2's: sender-side per packet; recipient-side residual until ratchets (§21). Pairwise groups
   inherit it; no static-static-only protection anywhere.
6. Mailbox abuse bounds are capacity and the enrollment list; there is no proof-of-work and no rate limit (policy).
7. Panels: third-party code runs on its own origin; the proxy supplies Host/CSRF guards; `s111` never reaches an
   upstream; dashboard CSP stays closed; panels that need a secure context are never faked.
8. Meshtastic: the presence reader has no send path; the panel's writes go to the *board* through meshmud (the user's
   own radio, their decision), never to the fabric.
9. Agent: reads freely, sends only through proposals.
10. Amateur-licensed regions: no sealed traffic on that radio (devices.md §0.6); region/callsign are human facts (D17 #10).

## 19. Cross-layer seams & amendments

- **Layer 2 (identity) — amendments this spec needs (panel crypto judge):** A1 AD scope = the sealed packet's own
  header; the L6 shim is outside and mutable · A2 inner `sender_node_id(32)` → `sender_ref(4)` + `pair_mac(16)`
  (§3; fixes the Noise-"N" sender gap; windows keyed by the authenticated sender; full NodeId only in INTRO) ·
  A3 `msg_seq` jump 2^32 on restore/unclean loss (§8.4) · A4 domain tags §3.1 · A5 crypto-mode reservations `0x02`
  (static-static folded into the KDF: the 16 B saver) and `0x03` (bulk chunk mode) + the carve-out that hash-verified
  chunk frames of public or already-encrypted objects may travel unsealed after a sealed advert · A6 §13 wording: v1
  emits no *unsolicited or periodic* announces; on-demand announces allowed · A7 L5 `contacts` authoritative, pin store
  a cache · A8 `appdata` v1 layout · A9 store-key HKDF (`111/store/v1`) with a test vector · A10 `transport_id`
  entries for bridge identities + hash binding for 64 B RNS keys; link keys never seed-derived.
- **Layer 1 (bootstrap):** new deps with sign-off recorded in `bootstrap/TASKS.md` — `rusqlite`/`libsqlite3-sys`
  (`bundled-sqlcipher-vendored-openssl`), `blake3`, `bao-tree`, `ciborium`, proxy glue; new state dirs (`fabric/`,
  `fabric/blobs`, `fabric/presence`, venvs for the presence reader); ports §2; `111 fabric offer|enroll|status` CLI
  verbs; manifest entries for meshmud binaries, the Meshtastic web bundle, the presence venv lock; `gen-notices`
  covers the GPL source offers; `check.sh` builds SQLCipher for both targets.
- **Layer 3:** tools §17; offload stays on the LAN binding + pairing auth (not v1 fabric traffic).
- **Layer 4:** D15 amendment §13; Messages/Contacts/Nearby/Settings views §16; `ServiceId` growth; `panel` object;
  `relayed` copy.
- **Layer 6 (`devices.md`):** D19 c amendments (recorded); `Link` trait §8 is the contract its drivers implement;
  `interfaces.toml` gains `owner` and the Meshtastic uplink binding; identify sequence §14.
- **Layer 7:** presence from announces/`nearby` sources; hub-to-hub replication; `KB` consumer; `appdata` TLVs;
  announce-as-routing questions stay open there.
- **Layer 8:** `SYNC` class + per-peer cursors; the append-only log is an opaque consumer.
- **Layer 9:** `PATCH` class, `AGENT_OP`, `RECEIPT_STRONG`; the apply→verify→undo engine (P4).

## 20. Verification (lock gates; netns, no hardware)

G1 two nodes + one hub in a netns (`interface/verify/smoke.sh` pattern, real `111d` binaries): card exchange, INTRO
request accept, `queued → sent → relayed → delivered`, hub killed mid-transfer and resumed by cursor, duplicates
re-ACKed not redelivered, Tier-1 fragments at a forced 160 B profile with loss injection, `store_full` refusal path ·
G2 SQLCipher static builds (x86_64 + aarch64) in `check.sh`; the aarch64 binary opens and prunes a store under Termux
(emulator or device) · G3 the Meshtastic panel runs offline in the netns: bundle served, ticket → cookie → iframe, HTTP
mode talks to a **stub** meshmud HTTP downlink, framing headers rewritten, drive-by POST refused · G4 envelope test
vectors: seal/open with `sender_ref` + `pair_mac` (frozen files like `id111`'s), SIGNED classes, forged `sender_ref`
never touches a window, reserved-bit rejection · G5 fable panel: crypto (§3–§6, §19 L2 amendments), panel proxy
security (§13), integration/scope honesty (§9 states, §14 read-only, §16 copy).

**External gates before the first signed release (hardware):** meshmud with a real Meshtastic board — serial open
without an unwanted reset, config replay to a second TCP client, one HTTP client limit observed, Termux build of
meshmud + the presence venv · two-board MeshCore bench for the `kiss-modem` seam (Layer 6).

## 21. Deferred (recorded, not forgotten)

Hub-to-hub replication (A+) · multi-hop discovery and announce-as-routing (L7) · `kiss-modem`, `meshtastic-client`,
`rns-bridge`, `ble`, online Noise IK / TLS-RPK links (L6) · message bridges (P1) · sender-key groups and MLS behind a hub
DS · rolling recipient ratchets (RATCHET flag, TLV 0x10) · erasure coding (`coding` field; RaptorQ IPR review) ·
crypto-mode `0x02` · owner-linked device replicas (Delta Chat ceremony + L2 attestation) · `age` recipient-mode export of
history and the "export before prune" offer · SQLCipher 5 migration · MeshMonitor as an optional presence/MeshCore
service · MeshChatX panel (P7) · `<id>.111.localhost` panel origins · TLS · camera QR scanning on LAN origins (needs a
secure context) · KB/UPDATE/PATCH consumers (L7/L9) · LXMF-style stamp costs (rejected as policy unless a network
imposes them) · per-panel postMessage protocol.

## 22. KB adoption list (entries written at build; all `candidate`)

`rusqlite` · `sqlcipher` · `blake3-bao` · `meshmud` · `meshtastic-web` · `meshtastic-python` (presence reader dep;
GPL note) · `ciborium` · `age` (deferred export) · `lxmf` (format facts; bridge target; Reticulum License) ·
`bundle-protocol-v7` (protocol; why not the core) · `dtn7-rs` (later gateway bridge) · `meshmonitor` (optional later
service) · `meshchatx` (parked P7; MeshChat under Alternatives) · case studies: `briar-mailbox`, `delta-chat-securejoin`,
`home-assistant-ingress`, `sandstorm-sandboxing` · concepts: `lora-airtime-budget`, `secure-contexts`,
`store-and-forward-states` · updates: `meshcore` (KISS modem, `GRP_DATA` flood, repeater as Cat-1), `meshtastic`
(`PRIVATE_APP`, rebroadcast modes, 2.8 signing, one-client limit), `reticulum` (no-import carriage facts). New tags:
`messaging`, `panel`, `dtn`.
