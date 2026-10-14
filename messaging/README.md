# Layer 5 — Messaging & Data

The fabric that carries every kind of traffic the node needs — human messages first, then agent operations, knowledge
sync, updates and patches — as end-to-end sealed, signed-where-it-matters payloads, with **reliable delivery first and
latency second**, store-and-forward across whatever links exist, and an encrypted local store pruned by capacity, never
by policy. Plus the dashboard's power to **host** an adopted project's own panel, and the first hosted project:
Meshtastic.

Status of this document: **spec v1.2 — locked (D20, 2026-10-14)** after the three-judge adversarial panel and the
re-check (decisions D18, D19, D20 in `os/plan/DECISIONS.md`; options brief `os/plan/layer-5-options.md`; research
`os/plan/layer-5-research/`; triage `os/plan/layer-5-review.md`, re-check `os/plan/layer-5-review/recheck.md`). Nothing here is built yet. Build tasks will live in `TASKS.md` per `os/plan/EXECUTION.md`.

Reads with: `os/README.md` §5 (vision) · `os/identity/README.md` §6, §8.4, §9, §10, §12, §13 (the sealed packet,
announces, restore, delegation, attestation, hand-offs) · `os/interface/README.md` §5–§7, §10–§11 (dashboard, services,
contract, auth) · `os/intelligence/README.md` (tools, task API) · `os/plan/devices.md` (Layer 6 brief, amended by D19 c)
· `os/plan/layer-5-research/00-code-seams.md` (where this plugs in).

## 1. Scope & non-goals

**In v1 (this cycle):**
- The **fabric**: `mesh111`, a Rust library crate (`os/messaging/crates/mesh111`), wired into `111d` as the module
  `daemon/fabric/` — envelope codec, message identity, fragmentation, authenticated receipts, contacts, the home-hub
  mailbox, the `Link` trait, the encrypted store, the local API. The identity keyset never leaves the daemon (L2 §13:
  signer pattern); `mesh111` receives a sealer/signer handle that also caches per-contact pair keys (§3), never key bytes.
- One link built: **`tcp-lan`** — sealed frames on a length-prefixed TCP stream between 111 nodes (phone ↔ hub, laptop
  ↔ hub, hub ↔ hub). Every other link (`kiss-modem`, `meshtastic-client`, `rns-bridge`, `ble`) is a **seam** with its
  frame budget pinned here and its driver built by Layer 6.
- **Delivery model v1** (D19 b): direct links + **home-hub mailbox** + contact cards. Store-and-forward in v1 works
  **between members of the same hub, or over a direct link**; cross-hub custody arrives with hub-to-hub replication
  (A+, §21). No multi-hop discovery, no periodic announces, nothing from Layer 7.
- **Hosted panels** (D19 d): one origin per panel by port, served by `111d`. v1 ships **static** panels plus a built-in
  **Meshtastic HTTP shim**; the generic reverse proxy for `child`/`detected` panels is specified (§13) and reserved.
- **Meshtastic service** (D18 #3, D19 g): consented identify (the Meshtastic step), `meshmud` multiplexer, read-only
  presence into Nearby, the hosted panel. **No message bridge.**
- **Messages** view, **Contacts**, **Nearby** list (Meshtastic presence), **Settings → Messaging & Data**, agent tools.

**Not v1 (recorded in §21):** attachments and the bulk-transfer protocol on the wire (v1.1; the blob store, BLAKE3
addressing and reserved kinds ship now) · bridges that relay messages between networks · LoRa drivers · INTRO over LoRa
(v1 INTRO is `tcp-lan`-only; LoRa first contact needs a card) · groups beyond pairwise fan-out · ratchets · erasure
coding · hub-to-hub replication · KB/update/patch *consumers* (classes defined; Layers 7–9 use them) · owner-linked
multi-device replicas · MeshChatX panel (parked P7) · WebSocket/SSE proxying · TLS.

**Hard rules carried in:** adopt, don't reinvent (every component below the fabric is adopted) · Reticulum is a
connected service, never a dependency (D17 #12; nothing in this layer imports RNS) · no wall clock in any cryptographic
or ordering decision (L2 rule 15) · no runtime downloads · no telemetry · no policy limits — no rate caps, no expiry, no
per-sender quotas; limits are capacity and physics (README §5) · delivery is non-negotiable, speed optional · GPL code
never links into our Rust binary.

## 2. Topology & components

| Component | What | Source | Runs on |
|---|---|---|---|
| `mesh111` crate | fabric library (§3–§12), `KeyProvider` seam, store schema, `Link` trait | custom (this layer) | every node running Layer 5 (Cat 2–5) |
| `111d` `daemon/fabric/` | wires `mesh111` to identity, links, store dir, API, SSE, services registry | custom glue (Layer 1 crate, D15-style sanction) | same |
| `111d` `daemon/panels/` | panel listeners: static serving, ticket auth, header policy, the Meshtastic HTTP shim; the generic reverse proxy is reserved (§13) | custom glue (`hyper`/`axum` already in the graph) | nodes with the dashboard |
| SQLCipher via `rusqlite` | encrypted store engine (§10) | adopted: `knowledge/projects/rusqlite.md`, `sqlcipher.md` | same |
| `blake3` + `bao-tree` | content addressing + verified streaming of bulk objects (§11) | adopted: `knowledge/projects/blake3-bao.md` | same |
| `meshmud` | Meshtastic multiplexer: one serial/TCP uplink, many TCP downlinks | adopted: `knowledge/projects/meshmud.md` — own code MIT; the CI build records a license verdict and the binary is conveyed as GPL-3.0 only if its module list shows Meshtastic's GPL-3.0 Go protobufs linked (the v1.0.0 source as read on 2026-10-14 has none — a hand-written varint framing — so the expected verdict is MIT-only); pinned by upstream tag + tarball sha256 + commit, CI-built, only `go.mod`/`go.sum` data snapshots in `os/` | nodes with a Meshtastic board (§14) |
| `meshtastic-presence` | tiny read-only presence reader on `meshtastic-python` | sidecar, GPL-3.0, own dir `os/messaging/sidecars/meshtastic-presence/`, own hash-locked venv | same |
| Meshtastic web client | the hosted panel (static bundle) | adopted: `knowledge/projects/meshtastic-web.md` (GPL-3.0-only; served unmodified from disk, never `rust-embed`) | same |
| `jsQR` | QR decoding in the dashboard (contact cards) | adopted (Apache-2.0), Layer 4 dependency sign-off | dashboard |

**Ports (loopback unless stated):** dashboard 11180 · model 11181 · agent 11182 · **fabric `tcp-lan` 11183** (LAN-bound
only when the node acts as a hub or LAN sharing is on, §9; loopback is never a peer) · panel listeners **11190–11229**
(bound where the dashboard is bound) · meshmud TCP downlink **11230** (loopback; no HTTP downlink is configured).

**State (`/var/lib/111`, the only writable tree):** `fabric/store.db` (SQLCipher, 0600) · `fabric/blobs/<blake3>/`
(0700) · `fabric/presence/meshtastic.json` (reader → daemon, 0644 within 0700) · `state/panels.json` (slot
assignments) · `state/panel-sessions.json` (0600) · `state/devices.json` (gains the Meshtastic binding; Layer 6 migrates it
into `interfaces.toml` when that file is born) · venvs under `venvs/`. `fabric/` is created at runtime like `agent/`.

Process shape: `meshmud` and `meshtastic-presence` are supervised children of `111d` exactly like `rnsd` and
`111-agent` (`supervise.rs` `ChildSpec`, back-off, `stop_and_wait`, restart route). The fabric itself is in-process.

## 3. The envelope (normative)

The fabric's unit on the wire is one **sealed packet** (`id111::seal`, L2 §6.4, crypto-mode `0x01`) with this layer's
fields in the places L2 reserved. Byte order LE unless stated; varints are unsigned LEB128.

```
[L6 shim, 1 B — ONLY on broadcast bearers, never on tcp-lan; outside the sealed packet; mutable; never in AD]
   bits 7–6 frame_type (0 reserved, 1 sealed, 2 public, 3 foreign) · bits 5–3 hop_limit
   bits 2–0: reserved (0) for frame_type 1; for frame_type 2 the public sub-type (0 announce · 1 card request · 2 chunk)

[sealed packet — L2 header, AD-covered byte-exact]
   version      1   0x01 (Layer 5 owns this field; unknown ⇒ drop, fail closed)
   crypto_mode  1   0x01 (L2 registry; unknown/weaker ⇒ reject before decrypt)
   counter      8   per-peer send counter (L2 §6.5 state, `CounterStore`; strictly increasing on the send side)
   extra_len    1   5
   extra        5   prio_flags(1) ‖ dest_hint(4)
   epk          32
   ciphertext   AEAD(inner)       tag 16
```

`prio_flags`: bits 7–6 **relay priority** (0 control · 1 interactive · 2 bulk-small · 3 bulk-large) · bit 5 GROUP (v1
MUST be 0) · bit 4 RATCHET (v1 MUST be 0; the only RATCHET bit) · bits 3–0 reserved, MUST be 0 — a receiver rejects
any set reserved bit. `dest_hint` = recipient NodeId[0..4]: a **locator** for relays and mailboxes, never an
authenticator (L2 B2).

**Inner plaintext (v1):**

```
   sender_ref   4   sender NodeId[0..4] (resolved against contacts; collisions ⇒ try each candidate's pair key)
   kind         1   §3.2 registry
   flags        1   bit0 HAS_FRAG · bit1 HAS_EXT · bit2 PIGGYBACK_ACK · bit3 SIGNED · bit4 RECEIPT_STRONG ·
                    bit5 INTRO · bits 6–7 reserved, MUST be 0 (set ⇒ reject)
   msg_seq      epoch(varint) ‖ n(varint)   per (sender, recipient, stream) sequence (§4); stream 0 = the pair's
                    1:1 stream; control kinds carry 0 ‖ 0
   [frag_idx, frag_count]  varint ×2            if HAS_FRAG (§5)
   [n_ext(1) ‖ ext TLVs]   type(1) len(1) value  if HAS_EXT (n_ext ≥ 1; unknown type ⇒ skip; duplicate type ⇒
                                                 reject; 0x01 seen_peer_seq; 0x10 ratchet pubkey and 0x21 signer_pubkey
                                                 are reserved ⇒ REJECT in v1; 0x20 delegation trailer ⇒ REJECT per L2 §6.7)
   [ack block]             §6.1                  if PIGGYBACK_ACK (length-prefixed, before payload)
   [card_len(1) ‖ card]    137–169 B             if INTRO: the sender's own announce (§7; v1: tcp-lan only; MSG and
                                                 ENROLL only, never together with HAS_FRAG)
   payload      …
   auth         pair_mac(16) ‖ [sig(64) if SIGNED]
```

**`pair_mac` (sender authentication — always present; L2 amendment A2):** the L2 construction is Noise pattern "N":
the recipient learns that *someone holding its announced key* sealed the packet, not who. `pair_mac` closes that:
`key = BLAKE2b-256("111/pair/v1" ‖ X25519(sender_enc_sk, recipient_enc_pk) ‖ sender_enc_pk ‖ recipient_enc_pk)`,
`pair_mac = BLAKE2b-128(key = key, data = L2 header bytes ‖ epk ‖ inner bytes before auth)`. Keyed BLAKE2b in MAC mode;
the key order `sender ‖ recipient` makes it direction-bound (no reflection). The receiver MUST verify `pair_mac`
**before** touching replay state or any store — a forged `sender_ref` never advances the claimed sender's state, and
**only `pair_mac` keys replay state; a valid `sig` never does** (a signed plaintext is re-sealable by anyone who holds
it). The identity handle derives the per-contact pair key once (the ECDH runs behind the handle) and hands the **derived
symmetric key** to `mesh111`, which runs the MAC; the keyset and the ECDH secret never cross (§18.3); the key is cached
per contact and zeroized on forget. `pair_mac`
is deniable (either party could have produced it) — intended for routine traffic (L2 rule 13); its KCI residual is in §18.

**`sig` (authorship, transferable; present only when SIGNED, in addition to `pair_mac`):** Ed25519 over
`"111/msg/v1" ‖ sender NodeId ‖ dest ‖ stream(varint) ‖ msg_seq(epoch ‖ n varints) ‖ kind ‖ flags ‖ frag_idx ‖
frag_count (varints, 0 ‖ 0 when unfragmented) ‖ BLAKE2b-256(payload)`, where
`dest` = the recipient NodeId (32 B) for 1:1 kinds, the `group_id` (32 B) for group posts, 32 zero bytes for public
classes. Covers this packet's payload, its transmitted flags byte and the addressee — not the sealed packet — so it
survives re-sealing (retransmit) but can never be redirected to a third party. Only the originating sender fragments
(relays never re-fragment, §5), so a fragmented SIGNED message carries one signature per fragment, each over that
fragment's payload; re-fragmenting is re-signing, which only the author can do. **Mandatory** for every stored-for-others, public or code class: group posts,
`AGENT_OP`, `KB`, `UPDATE`, `PATCH`, `BROADCAST`.

**Sizes (what fits; the L6 shim byte is inside the bearer payload on broadcast bearers):**

| Bearer payload | Shim | Outer (16 hdr + 32 epk + 16 tag) | Inner hdr (≈8–10) + `pair_mac` | Payload, pair-auth | Payload, SIGNED (+64) |
|---|---|---|---|---|---|
| 160 B (flood floor: MeshCore `GRP_DATA` 163 B) | 1 | 64 | 24–26 | **≈ 69–71 B** (Tier 1 with frag fields: 67–69) | ≈ 5–7 B |
| 233 B (Meshtastic `PRIVATE_APP`; RNode single frame ≈ 254) | 1 | 64 | 24–26 | **≈ 142–144 B** | ≈ 78–80 B |
| TCP (`tcp-lan`) | 0 | 64 | 24–26 | up to 65,535 − 90 per frame | — |

SIGNED classes are Tier-2 or TCP traffic in practice; the 160 B SIGNED column is stated for honesty. If the L2 panel
rejects amendment A2 (4 B `sender_ref` instead of the 32 B inner NodeId as built), subtract 28 B from every payload
column (41–43 B at 160 B). **One wire per version:** the 32 B form is a pre-release build gate only and is removed
before the first signed release; two inner layouts never share `version = 0x01`.

### 3.1 Domain tags (registered with L2 §6.1; each with its pinned signed/MAC input)

`111/pair/v1` (§3) · `111/msg/v1` (§3) · `111/ack-sig/v1` (§6.1) · `111/custody/v1` and `111/custody-sig/v1` (§6.2) ·
`111/enroll/v1` (§7.3) · `111/store-secret/v1` and `111/store/v1` (§10.2) · `111/obj/v1` (§11: `Ed25519("111/obj/v1" ‖ publisher ‖ hash ‖
size ‖ class)` for public object adverts).

### 3.2 Kinds (1 B; unknown ⇒ drop and log, never best-effort)

| Kind | Name | Direction | Priority | `msg_seq` | Idempotency |
|---|---|---|---|---|---|
| 0x01 | `MSG` | peer ↔ peer | interactive | per stream | §4 |
| 0x02 | `ACK` | recipient → sender | control | 0 | cumulative; replay harmless |
| 0x03 | `CUSTODY_PUT` | member → hub | by inner item | 0 | `pkt_ref` dedup |
| 0x04 | `CUSTODY_ACK` | hub → member | control | 0 | bound to `pkt_ref` |
| 0x05 | `MAILBOX_FETCH` | member → hub | control | 0 | idempotent |
| 0x06 | `MAILBOX_ITEM` | hub → member | by item | 0 | inner item deduped by its own `msg_seq` |
| 0x07 | `MAILBOX_COMMIT` | member → hub | control | 0 | `max(cursor)` |
| 0x08 / 0x09 | `ENROLL` / `ENROLL_OK` | member ↔ hub | control | 0 | code single-use |
| 0x10 | `AGENT_OP` | node ↔ node | interactive | per stream | §4; SIGNED |
| 0x11–0x14 | `KB` · `UPDATE` · `PATCH` · `SYNC` | — | bulk | per stream | classes defined here; adverts via §11; consumers later |
| 0x20–0x22 | `OBJ_ADVERT` · `OBJ_REQ` · `OBJ_CHUNK` | — | bulk | 0 | §11 (pinned; built v1.1) |
| 0x30 | `BROADCAST` | one → all | interactive | — | **reserved** (public signed frames are a Layer 7 format; §12) |

Control kinds (`msg_seq = 0`) are exempt from `rx_state` and ACKs; each is idempotent as listed, so a lost control
packet never leaves a hole.

### 3.3 Message body (`MSG`, Cat 2+)

Deterministic CBOR (RFC 8949 §4.2) via `ciborium`: `{1: text, 2: [attachment refs] (v1.1), 3: sent_at?, 4: reply_to?}`.
`sent_at` is the sender's **claim**, shown as "unverified sender time" (§4). Attachment ref = `{hash(32 BLAKE3),
size, name?, object_key(32)}` — the object moves by §11. Relays never parse bodies; Cat-1 never sees them.

## 4. Message identity, ordering, time

- **`msg_id = (sender, recipient, stream, epoch, n)`.** `n` is a strictly
  increasing per-(peer, stream) counter starting at 1 (so `0 ‖ 0` is only ever a control value)
  (`tx_state` is keyed by `(peer, stream)`: stream 0 is the pair's 1:1 stream, never a global counter), **committed to
  the store (`synchronous = FULL`) before the first fragment is sealed**. On the wire `msg_seq` is `epoch(varint) ‖
  n(varint)` — 2–4 B typical; every `seq` in ACK lists and `seen_peer_seq` uses the same two-varint form.
- **Epochs.** `epoch` is a freshly committed L2 announce-counter value (`next_announce_counter()`, fsync), **minted
  once per store open and shared by every stream started in that run**, when any of these holds: the clean-shutdown
  record is absent; the recorded `announce_last` differs from the live identity's; a restore floor was set since the
  last run (L2 §8.4); `n` would wrap 2^32. The store records L2 `announce_last` at clean close (§10.1). So a crashed
  sender never reuses a number for new content, a `store.db` restored from a backup onto a live identity gets a new
  epoch even though it closed cleanly, and a second words-only restore never collides with the first (L2 amendment
  A3). While the identity holder reports `RestoreFloorRequired`, **nothing is sealed**: outbox items stay `queued`,
  reason `restore_pending`. Residual, stated: one consistent older image of store **and** identity dir together is L2
  §8.4's state-present case and is undetectable locally.
- **Receiver state** per (peer, stream, epoch): a contiguous delivered prefix plus a bounded range list (≈ 24–48 B).
  **Below a prefix nothing is ever forgotten**: a duplicate is re-ACKed and never redelivered — this is the
  content-level idempotency L2 §6.5 hands to this layer, and it also covers flood-mesh duplicates and multi-path
  delivery. A `pair_mac`-verified packet whose epoch **exceeds** the current one opens a new (prefix, ranges, partials)
  entry; the previous epoch's entry is **frozen and kept**, and old-epoch packets are still delivered against it
  (custody and multi-path legitimately deliver an earlier epoch after a later one). `rx_state` keeps the newest **4**
  epochs per (peer, stream); older epochs are stale and dropped, and a per-(peer, stream) **floor** records the
  highest evicted epoch so an evicted epoch can never re-open. Range lists are bounded (8 per epoch): a packet that
  would open a ninth isolated range is **deferred** — not recorded, not delivered, not ACKed; the sender keeps its copy
  and retransmits — never evicted, so a delivered message is never forgotten; gap-filling and adjacent packets are
  always accepted, so this cannot stall. An allocated `n` is never abandoned: only the atomic queue-and-allocate path
  hands out numbers, and a message the user cancels before it leaves is sent as an empty tombstone, so no permanent
  hole can wedge an epoch. Random ids and content hashes are rejected (both
  forget under pruning and then redeliver).
- **Replay authority (L2 amendment A11).** For kinds carrying `msg_seq`, replay protection is this state, not the L2
  32-entry window — under store-and-forward a custody item can legitimately arrive hundreds of counters behind a
  packet that took a direct path. A11 relaxes *rejection*, not bookkeeping: **every verified packet is still offered to
  the L2 window** (`observe_counter`, high-water advanced), and `msg_seq` kinds are processed for content whatever the
  verdict; but **attribution of a connection, card answers and proactive pushes require a packet the window judged
  fresh** (§8.1) — a replayed packet verifies but is never fresh (L2's unclean-reboot jump covers the last-N replays).
  Control kinds are idempotent (§3.2). Residual, stated: delay-and-replay of a never-delivered old packet delivers it
  once — DTN behaviour, not a hole.
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
| **1** | ≤ 16 fragments (≈ 1.07 KB at the 160 B floor, ≈ 2.2 KB at 233 B) | **fragment-then-seal**: every fragment is a complete sealed packet (`frag_idx < frag_count ≤ 16`), independently authenticated, resumable, no fragment poisoning. Sender keeps ≤ **16 outstanding** unACKed fragments per peer and **re-seals on retransmit** (fresh epk + counter). Reassembly at the destination only; the receiver ACKs fragments by SACK bitmap (§6.1). Partial reassemblies are bounded by capacity, evicted oldest-first. **INTRO in v1 is `tcp-lan`-only** (a card never fits a flood frame; the card-in-fragment-0 design for LoRa first contact is recorded in §21). | ≈ 35–45 % at 160 B; the price of verifiable fragments |
| **2** | larger objects and every bulk class | sealed **`OBJ_ADVERT`** (`{hash, size, chunk_group, coding, object_key?}`, SIGNED when the class requires), sealed `OBJ_REQ {hash, ranges, session_tag}` where `session_tag` = 8 random bytes chosen by the requester (never hash-derived: no linkability across fetches), then **hash-verified chunks** (§11): `OBJ_CHUNK` frames (`session_tag ‖ chunk_idx ‖ bao-verified bytes`) **unsealed** when the object is public or already encrypted to its recipient, sealed otherwise. Unsealed chunks ride `tcp-lan` frame `0x05` and shim type 2 with a sub-type byte. Pinned here; **built in v1.1** (J5); the L2 carve-out for unsealed chunks on broadcast bearers is asked then, not now. | ≈ 22 B per chunk |

**Seal-once-then-fragment is rejected** (raw slices cannot be verified; one bad slice forces a whole-message
retransmit; counters per fragment would be unavailable). Erasure coding: `coding` field reserved (0 none · 1 Reed-Solomon
· 2 RFC 6330); none built (RaptorQ only after IPR review).

**MTU profiles:** the fabric sizes fragments from the *destination path's* profile exposed by the link (`mtu_bytes`,
`reliable`, `broadcast`); relays never re-fragment sealed packets. Default profile when unknown: **160 B**.

## 6. Receipts

### 6.1 Delivery ACK (`kind = ACK`, or the piggybacked block)

```
ack block = ack_len(varint) ‖ ack_epoch(varint) ‖ ack_prefix(varint)        -- current epoch, delivered prefix n
            ‖ n_ranges(1, ≤ 8) ‖ [epoch, start_n, len](varint ×3)*           -- current-epoch ranges + frozen epochs, absolute
            ‖ n_partial(1) ‖ [epoch, n (varint ×2), frag_bitmap(u16 LE)]*    -- fragments received of in-flight messages
            ‖ n_strong(1, 0 allowed) ‖ [epoch, n (varint ×2), BLAKE2b-128(payload)]*
```

for the (sender, stream) being acknowledged — the stream is the ACK's own stream (0 for 1:1). The block carries at
most 8 ranges across all kept epochs: the range containing the packet being acknowledged comes first, then the current
epoch's ranges newest-first, then frozen epochs newest-first — so a retransmitted packet is always acknowledged and a
sender can never be made to retransmit forever. `ack_len` covers
everything after it, so a piggybacked block (placed before `payload`, §3) is unambiguous. Authentication is the
packet's own `pair_mac`; `kind`/flags sit inside the authenticated region, so a data packet cannot be replayed as an
ACK. A **transferable** receipt is produced only on request and only in a standalone `ACK` packet: `ack_sig =
Ed25519("111/ack-sig/v1" ‖ acker ‖ sender ‖ stream ‖ ack block bytes)` as the last 64 B of that packet's payload,
outside `ack_len`; never piggybacked. `RECEIPT_STRONG` on a message asks for its payload hash in the
ACK (proof the recipient decrypted *this* content; used for `AGENT_OP`, `PATCH`). Duplicates are re-ACKed. **Only the
recipient's ACK moves a message to `delivered` and releases the sender's copy.** Hop-by-hop link ACKs are the link's
business (Layer 6).

### 6.2 Custody receipt (`kind = CUSTODY_ACK`)

Payload: `status(1: 1 ACCEPTED · 2 DUPLICATE · 3 REJECT_FULL · 4 REJECT_UNENROLLED · 5 REJECT_UNKNOWN_DEST) ‖
custody_seq(varint) ‖ pkt_ref(16) = BLAKE2b-128("111/custody/v1" ‖ exact sealed bytes of the item)`. Hashing the exact
bytes proves the custodian **received** them (holding is a promise the store's "never prune undelivered custody for
members" rule keeps). `ACCEPTED` from a **mailbox-role** node for a **known member** is the only evidence for the UI
state `relayed`. Transferable form on request: `Ed25519("111/custody-sig/v1" ‖ hub ‖ dest ‖ status ‖ custody_seq ‖
pkt_ref)`, carried as the last 64 B of the `CUSTODY_ACK` payload. `custody_seq` is ≥ 1 for ACCEPTED/DUPLICATE and 0 for
every REJECT.

## 7. Contacts, cards, announces

Keys are not derivable from a NodeId (L2 §2: the X25519 key is an independent derivation), so a sender needs the peer's
**announce** (L2 §6.2, 137–169 B, self-authenticating). v1 emits announces **on demand only** — in a card, on link-up,
inside an INTRO — never on a timer (L2 amendment A6). **Stale rule:** an announce with the same key material and a
counter ≤ the stored one is accepted with no state change (reconnects and genesis cards re-present stored announces;
only a key conflict or a bad signature drops), and card requests are answered from **stored announce bytes**, never
minted (L2 amendment A6).

### 7.1 Contacts table (store)

`node_id(32)` · `enc_pubkey(32)` · `announce` (stored bytes) · `announce_counter` · `trust` (`oob` card/QR · `tofu`
INTRO · `enrolled` hub ↔ member) · `alias` · `roles` (from `appdata`) · `home_hub` hint · `first_seen_seq` ·
`last_seen_seq` · `request` (pending message request) · `pinned` (never pruned). **This table is authoritative for
user-added peers**; L2's 256-entry pin store is a cache of it (L2 amendment A7). Key material is immutable once pinned
(L2 §6.3): a card or announce with different keys for a known NodeId is rejected and surfaced.

### 7.2 Three ways in

1. **Card** — the node's own genesis announce as a QR (module bitmap rendered by the UI; ≈ v7) or base32 text (~220
   chars; bech32 is for the NodeId only). `GET /api/v1/contacts/me/card`. Scanning a card (camera on a secure or
   loopback origin via `jsQR`, or file upload) adds the contact with `trust = oob`. Works with zero connectivity.
2. **`node1…` only** — typed or scanned (63 chars). The node stores the NodeId and sends a **cleartext card request**
   (`tcp-lan` frame `0x04`: the wanted NodeId; it cannot be sealed, since the requester has no key yet) to its hub; a hub
   answers **only for its enrolled contacts, only to a member on an attributed connection (§8.1), from stored bytes,
   and never forwards the request** (§18: the request is a membership oracle, bounded to members). The reply is an announce frame,
   verified **against the expected NodeId**, then pinned. Until then messages stay `queued`, reason "waiting for their
   key". (Delta Chat SecureJoin shape: one out-of-band fact, the rest over an untrusted channel.)
3. **INTRO** (`tcp-lan` only in v1) — a first message to a never-contacted peer sets `INTRO` and carries the sender's
   announce inside the ciphertext (self-authenticating by the announce signature; `pair_mac` is verified after the card
   is pinned provisionally, in that order). The receiver files a **message request** (`trust = tofu`); the user accepts
   (contact created, message delivered to the thread) or ignores (kept in the request list, never auto-pruned while
   pending, no ACK sent). Accepting a request never ACKs retroactively more than the messages shown. Pending requests
   are **capacity-bounded** (a pending-request quota, default 256, and refusal at `store_full`): a refused INTRO is
   dropped without an ACK and counted — otherwise anyone who can reach the node could mint identities until the disk
   is full.

### 7.3 Enrollment with a home hub

The hub shows a one-time **enrollment code** (8 chars from the L4 pair-code generator; `111 fabric offer` or Settings →
Messaging & Data → "Let a node use this hub") together with its card and `tcp-lan` address (the LAN IP, D16 e) as a QR.
The member sends `ENROLL` sealed to the hub with the `INTRO` flag set — its card rides the §3 card slot, so the hub pins it
provisionally and verifies `pair_mac` before parsing the payload (§7.2 (3)); the payload is `{code}`. The hub verifies
`BLAKE2b-128(key = code_secret, data = "111/enroll/v1" ‖ member NodeId ‖ code)` in constant time (`code_secret` = 32
random bytes minted with the offer), stores the contact as
`enrolled`, answers `ENROLL_OK {hub card, custody_seq_floor}`. Code rules (mirroring the pair code, D15): single-use,
invalidated when the hub owner dismisses the offer, attempt lockout after 5 failures — a defense, carved out of "no
policy limits" like the pair code; `DELETE /fabric/hub/offer` dismisses an outstanding offer (loopback-only). Enrollment is the owner's **access list**, not a rate policy; a hub refuses
`CUSTODY_PUT` from unenrolled nodes with `REJECT_UNENROLLED`. A node has one home hub in v1 (replicas = deferred A+).

`appdata` v1 (L2 amendment A8): `[ver=1][roles u16 LE][reserved]` — bit 0 mailbox (accepts custody) · bit 1 always-on
hub · bit 2 LoRa-attached; optional 8 B truncated home-hub id (locator only). Layer 7 appends TLVs after v1.

## 8. Links

```rust
pub struct PathProfile { pub mtu_bytes: u16, pub framing: Framing /* Stream | Datagram */, pub reliable: bool,
                         pub broadcast: bool, pub cost_class: CostClass /* Bulk | LowBandwidth */,
                         pub airtime: Option<Airtime> /* advertised physics: duty_cycle_permille, bits_per_s */ }
pub enum PeerAddr { Tcp(SocketAddr), Broadcast, Opaque(Vec<u8>) }
pub struct Frame { pub frame_type: FrameType, pub hop_limit: Option<u8>, pub bytes: Vec<u8> }
pub enum LinkState { Down, Up { peers: usize }, Degraded(String) }
pub enum LinkError { Closed, TooLarge(usize), Io(std::io::Error) }

pub trait Link: Send + Sync {                       // object-safe: futures are boxed
    fn id(&self) -> LinkId;
    fn kind(&self) -> LinkKind;                     // TcpLan | KissModem | MeshtasticClient | RnsBridge | Ble
    fn profile(&self) -> PathProfile;
    fn peers(&self) -> Vec<PeerAddr>;
    fn send(&self, to: PeerAddr, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>>;
    fn recv(&self) -> BoxFuture<'_, (PeerAddr, Frame)>;
    fn state(&self) -> LinkState;                   // feeds transport_state / DeviceState
}
```

The fabric never rate-limits; airtime and duty-cycle are facts the adapter advertises and the agent can read.

| Adapter | Drives | Frame budget | Status | Notes |
|---|---|---|---|---|
| **`tcp-lan`** | TCP between 111 nodes: member ↔ hub (D17 #4), hub ↔ hub, laptop ↔ hub | stream; frames ≤ 65,535 B | **built with L5** | §8.1. Unicast: no MulticastLock needed on phones. No Noise/TLS in v1 (end-to-end seal; metadata leak stated in §18). |
| `kiss-modem` | a 111-owned board running the **MeshCore KISS modem** (D19 c): 111d builds MeshCore `GRP_DATA` frames; stock MeshCore repeaters flood them | **160 B** payload per frame (163 B cap) | seam; Layer 6 builds | Region set once (D17 #10). Opt-in "shared" profile: an RNode in HOST mode, rnsd attached by loopback TCP-KISS. One owner per port. A data-type range is registered with MeshCore before Layer 6 locks (D19 c). |
| `meshtastic-client` | a Meshtastic board via meshmud's TCP downlink, port `PRIVATE_APP` | 233 B (221 with PKI/AEAD channels) | seam; no bridge this cycle | The firmware rewrites `from`: a bridge speaks as its own Meshtastic node; one bridge identity per foreign network bound by L2 §10 attestation (A10). Routers in `CORE_PORTNUMS_ONLY` drop it. Never on amateur-licensed regions (sealed = "obscured", devices.md §0.6). |
| `rns-bridge` | LXMF via MeshChat(X) HTTP/WS or lxmd's inbound hook — envelope as an attachment; LXMF address = transport address, our NodeId stays inside | 287 B single packet, larger via Link | seam; deferred | No RNS import, ever. |
| `ble` | Android helper app, later | ~ATT MTU | later | — |
| online links (Noise IK / TLS-RPK, L2 §6.6) | interactive class, L3 offload over the mesh | stream | later (Layer 6) | **Link keys are random and bound by L2 §10 attestation — never seed-derived (F2 rejected), never the identity key (§6.1).** iroh is not in v1 (QUIC-only, +13 MB, blobs pre-production). |

### 8.1 `tcp-lan` wire (normative)

TCP; listener `:11183` (LAN-bound when the node acts as a hub or LAN sharing is on; loopback is never a peer). Both
sides send `hello = "111F" ‖ 0x01` then frames: `type(1) ‖ len(2 BE) ‖ body`. Types: `0x01` sealed packet · `0x02`
announce (cleartext; `id111::process_announce` with the §7 stale rule, before any use — this is the on-demand link-up
announce) · `0x03` keepalive (empty) · `0x04` card request (32 B NodeId; §7.2) · `0x05` chunk (reserved, §5 Tier 2).
Each side sends its announce first; a peer whose announce fails signature verification or conflicts with pinned keys
is dropped. **Hello and announce authenticate nothing**: a connection is attributed to node X only after a
`pair_mac`-verified packet from X **that the L2 window judged fresh** (§4) has arrived on it — a replayed packet
verifies but is never fresh; proactive mailbox pushes, card answers and card forwarding all gate
on attribution (an attacker replaying a public announce gets nothing but a silent socket). Frames above the length
cap are refused; reconnect on link events with back-off; no timers in the protocol itself (keepalive interval is a
transport detail). The stream leaks `dest_hint`, counters and sizes to an on-path observer (§18); content, sender and
kind are sealed.

## 9. Mailbox protocol (hub role)

A hub = any node the owner enrolls others with (Cat 4 by default; an always-on laptop may opt in; phones opt in
explicitly). It holds **custody**: sealed items it cannot read, for members that are away. **v1 scope sentence:** a
hub accepts custody only for its own members (and only from members); store-and-forward therefore works between
members of one hub or over a direct link — cross-hub delivery needs A+ (§21). The UI says so: a message to a contact
whose hub is not yours stays `queued`, reason "their hub is not yours; it is delivered when you meet directly".

- `CUSTODY_PUT {dest NodeId(32) ‖ sealed item bytes}` sealed to the hub (the double seal costs ≈ 90 B — irrelevant on
  TCP; on LoRa the hub link is the mailbox's own `kiss-modem`, Layer 6). Hub: sender ∈ enrolled else
  `REJECT_UNENROLLED`; **`dest` ∈ enrolled else `REJECT_UNKNOWN_DEST`** (sender keeps `sent`, reason "your hub does not
  know them"); dedup by `pkt_ref`; store `(dest, bytes, custody_seq)`; reply `CUSTODY_ACK` (§6.2). `REJECT_FULL` when
  the store refuses new custody (§10.4); the sender keeps the item `sent`, reason "mailbox full".
- `MAILBOX_FETCH {cursor varint}` from a member on an **attributed** connection (§8.1): the hub streams
  `MAILBOX_ITEM {custody_seq ‖ bytes}` for **that member's own NodeId** above the cursor, nothing else.
  `MAILBOX_COMMIT {cursor}`: the member has stored everything up to it; the hub keeps `max(cursor)` and marks those
  items **evict-first** (not deleted: capacity decides). Cursors are the hub's local insertion sequence — no clock.
- Members fetch on link-up and on `fabric` events; hubs push `MAILBOX_ITEM` proactively on attributed connections.
- **Hub-to-hub replication (A+)** is specified as: configured peer hubs exchange per-(dest) summary vectors (the §4
  prefix + ranges form) and replicate custody items; replicas are evict-first; a hub then also accepts custody for a
  peer hub's members. Not built in v1.
- The hub never opens items; it sees `dest`, size, `prio_flags`, `dest_hint`, counters.

**States (UI, normative evidence):** `queued` (outbox; no path, key missing, or their hub is not yours — reason shown)
· `sent` (frame written to a link; LoRa: modem accepted TX; a repeater echo is detail here; also a custody reject with
its reason) · `relayed` (authenticated `CUSTODY_ACK ACCEPTED` from a mailbox-role node for a known member — "a mailbox
node saved it and will hold it for them") · `delivered` (recipient `ACK`). Retries happen on link and contact events,
never on wall-clock timers; the sender's copy is kept until `delivered`.

## 10. Store

### 10.1 Engine (D19 e)

`rusqlite` with `bundled-sqlcipher-vendored-openssl` (SQLCipher 4.x community, OpenSSL 3.6 vendored; musl-static on
x86_64 and aarch64 — measured +6.4 / +5.4 MB). **File format pinned to SQLCipher 4** (`PRAGMA cipher_compatibility =
4`); 5.0 migration planned via `cipher_migrate` behind the Layer 1 updater. Raw key (`PRAGMA key = "x'…'"`, no
PBKDF2 at open), WAL, `synchronous = FULL` on sequence-allocating transactions, `temp_store = MEMORY` (transient files
are not encrypted), `auto_vacuum = INCREMENTAL` so pruning frees disk, `secure_delete = ON`, a clean-shutdown record
(`settings.clean = 1` **and** the L2 `announce_last` value, written on close; the §4 epoch rules read both at open). One DB: `fabric/store.db`, 0600, owner `node111`.

### 10.2 Key custody (`KeyProvider` seam)

- **Mode S (default; Cat 2/3/4 without a TPM; phones):** two keyed-BLAKE2b steps, the same primitive family as L2
  §6.4 (no second hash family): `store_secret = BLAKE2b-256(key = the X25519 private key (keyset bytes 32..64),
  data = "111/store-secret/v1")`, computed behind the identity handle (`Sealer::store_secret`), then
  `store_key = BLAKE2b-256(key = store_secret, data = "111/store/v1" ‖ NodeId)` in the store's `KeyProvider`; 32 B,
  handed to SQLCipher as a raw key; never written to disk. A DB
  copied elsewhere opens again after restore-from-words. Limit, stated honestly: protects DB-only copies, backups and
  synced app dirs — not a stolen disk with a readable identity dir (L2 §12 residual). L2 amendment A9 (with a vector).
- **Mode T (Cat 4 with TPM2 / SE050B):** a random 32 B key sealed in hardware (L2 §5: the TPM holds the *storage* key,
  never the keyset); Mode S is the documented recovery path if the hardware is lost.
- Never bind the key to an OS keyring without a recovery path (Signal Desktop lesson).

### 10.3 Schema (v1, migrations via `PRAGMA user_version`)

`contacts` (§7.1) · `messages` (`peer`, `sender`, `stream`, `epoch`, `n`, `dir own|in|transit`, `kind`, `class`, `size`, `local_seq`
(the age hint), `state queued|sent|relayed|delivered`, `reason`, `received_local_seq`, `body` or blob refs, `pinned`) ·
`tx_state` (`(peer, stream)` → next `n`, `epoch`) · `outbox` (sealed bytes awaiting a path, per fragment, outstanding count,
SACK bitmap) · `rx_state` (`(peer, stream, epoch)` → prefix + ranges; newest 4 epochs kept) · `rx_floor` (`(peer, stream)` → highest evicted epoch) · `partials` (fragment bitmaps **and the fragment bytes**, one FULL transaction, so a SACKed fragment survives a crash; capacity-evicted oldest-first) · `custody` (hub: `dest`,
`custody_seq`, `pkt_ref`, bytes, `committed`) · `cursors` (member: hub → cursor) · `blobs` (`hash`, `size`,
`refcount`, `public`, `object_key?`) · `requests` (pending INTROs) · `settings`.
Bodies of `MSG` live in the DB (small); attachments (v1.1) and every bulk object are files under `fabric/blobs/` named
by BLAKE3 root — private objects are encrypted once with a **random per-object `object_key`** generated inside the store (never
caller-supplied; XChaCha20-Poly1305 in 64 KiB chunks, nonce = LE chunk index ‖ zeros, AD = chunk index ‖ last-chunk
flag so a boundary truncation fails to decrypt); a relay or hub may hold the ciphertext keyless and addressed by **ciphertext** hash, so relays and hubs verify what they
carry without reading it; public signed objects (`KB`, `UPDATE`, `PATCH`) are stored plain.

### 10.4 Pruning (capacity only — normative order)

Trigger: live bytes (DB pages minus freelist + blob dir) above the high watermark **or** free disk below a 1 % floor.
Order: (1) transit custody that is `committed` and public chunks that are re-fetchable — weight = size × age (old and
large first; age = `local_seq` distance, never a clock) · (2) superseded release/KB objects below the Layer 1 monotonic
floor, fully fetched deltas · (3) own conversation history oldest-first **only at hard capacity and only after an
export offer** (§21: `age`-based export) · (4) **never** undelivered outbound, uncommitted custody for members, pending
requests, pinned items or contacts — instead the store refuses new custody and new bulk, and surfaces `store_full` on
the `fabric` service and to the agent. Per-class quotas exist as capacity shaping (configurable); per-sender limits and
stamp costs are policy and do not exist.

## 11. Bulk objects (the data half)

One content address and one verified-stream primitive for every link tier: **BLAKE3 root** + **bao-tree** outboard
(chunk group 1 KiB on LoRa/BLE: 6.25 % outboard; 16 KiB on TCP: 0.39 %; the root is identical across groups, so a
chunk fetched over LoRa and one over TCP verify against the same address). **v1 builds** the blob store, addressing,
outboard computation and the reserved kinds; **v1.1 builds** the transfer on the wire: `OBJ_ADVERT` (sealed; SIGNED
under `111/obj/v1` for public classes; carries `object_key` for private ones) → `OBJ_REQ {hash, ranges, session_tag}`
→ `OBJ_CHUNK` frames (§5 Tier 2), range-set requests so different carriers can bring different pieces, resume at any
chunk boundary. The fabric treats bulk as `bulk-small` (≤ 64 KiB) or `bulk-large`; bulk classes route to `cost_class =
Bulk` links by default and cross a `LowBandwidth` link only when the user or the agent explicitly asks (physics: a 32 KB
KB delta is 5–9 min of LoRa airtime at 1 % duty; a 5.7 MB binary is 15–27 h). Consumers: attachments in `MSG` (v1.1),
`KB` deltas and git bundles (Layer 7), `UPDATE` payloads (Layer 1's minisign manifest stays the trust root; the updater
gains an intake path for a staged payload at `fabric/blobs/<hash>`, §19), `PATCH` (Layer 9), `SYNC` ops for Layer 8's
append-only log (opaque, idempotent ids, per-peer cursors — seam only).

## 12. Groups & broadcast

**Groups v1 = pairwise fan-out**: N sealed packets, each pair-authenticated and forward-secret, per-member ACK, exact
delivery tracking, no shared key state, trivial removal. A group is a local object (`group_id` 32 B, members, name);
group posts are SIGNED with `dest = group_id`. The `GROUP` flag and a `stream_id` TLV are **reserved** for sender keys
(G1: 20 B overhead + 64 B signature per message; rotation = each member re-distributes its sender key, no clock). MLS
only behind a hub delivery service, later. Static PSK channels (Meshtastic-style) are rejected. **Public broadcast**
(`BROADCAST`, kind reserved) is a public *frame*, not a sealed packet: cleartext by design, always SIGNED, relayed like
an announce; a node shows it as a broadcast, never as a message from a contact. Its byte format is pinned with Layer
7's presence frames, not here.

## 13. Hosted panels (D19 d; amends D15)

A **panel** is an adopted project's own web UI, shown inside the dashboard on the service's LIVE page. Rule: **the
dashboard frames only registry-declared panels, each on its own origin.** Nothing else is ever framed.

**Declaration:** `assets/panels/<id>.toml`, embedded in the release, hash-pinned, parsed with the `toml` crate at
build and at daemon start; unknown keys or an invalid file ⇒ the panel is not registered (logged). The Meshtastic
declaration, as valid TOML:

```toml
id = "meshtastic"                       # = ServiceId and KB link
title = "Meshtastic"
kb_entry = "projects/meshtastic-web"

[source]
kind = "static"                         # static | child | detected  (v1 implements static)
bundle = "panels/meshtastic-web"        # unpacked from the hash-pinned release payload; SPA fallback for non-API paths
shim = "meshtastic-http"                # built-in: /api/v1/toradio|fromradio reframed onto a meshmud TCP downlink (§14)

[entry]
path = "/"

[health]
listen = 11230                          # meshmud TCP downlink LISTEN (never a connect) + child state

[frame]
allow = ["serial", "bluetooth"]
needs_secure_context = false
sandbox = "default"
secure_note = "USB or Bluetooth from this browser works only on the node itself and only for a radio this node has not bound; the bound board is reached through the panel's HTTP connection."

[proxy]                                 # reserved for child/detected panels (P7); true here fails adoption in v1
websocket = false
sse = false
max_body = "8MiB"

[offline]
optional_hosts = ["tiles.openfreemap.org", "api.meshtastic.org"]   # verified at the lint; shown to the user

[auth]
mode = "111-session"                    # 111-session | loopback-only

[license]
spdx = "GPL-3.0-only"
served = "unmodified"
source_offer = "THIRD-PARTY-NOTICES.md" # names the pinned tag; CI builds the bundle from the tag when upstream ships no asset
```

- **Listeners:** one per *enabled* panel from the block 11190–11229, slot stable in `state/panels.json`, bound
  exactly where the dashboard is bound (loopback; LAN when LAN sharing is on); `[auth] mode = "loopback-only"` means
  the listener binds loopback regardless of LAN sharing, with the ticket flow unchanged. Origin =
  `http://<host as typed>:<port>`; the UI builds it from `location.hostname` and the `panel.port` field (never from a
  shared snapshot). The **main dashboard listener answers `Host: localhost` with a 308 to the same path on
  `127.0.0.1`**, so panels never see `localhost` (same host + different port is same-site, so the frame, the ticket
  swap and same-origin fetches pass); `[::1]` is matched bracketed.
- **Auth (ticket → cookie, everywhere — no loopback skip):** the dashboard calls `POST /api/v1/panels/{id}/session`
  (behind the dashboard session; on trusted loopback the session is implicit, the ticket is still minted) and receives
  a one-time ticket URL (128-bit, single use, 30 s). The panel listener swaps it for the host-only cookie `p111_<id>`
  (HttpOnly, SameSite=Strict, no `Domain`, Path=/) and redirects to `entry.path`. Lifecycle: a request carrying a valid
  `p111_<id>` ignores any ticket parameter and redirects to `entry.path` (reload/back after the swap); a spent or
  expired ticket without a cookie answers `403 panel_ticket_expired` and the dashboard re-mints; a ticket ignored
  because a valid cookie is present is also spent; tickets and cookies
  are bound to the minting session (peer, session hash); `p111_*` are session cookies recorded in
  `state/panel-sessions.json` (0600) and revoked on `111 dashboard rotate`, panel disable and LAN-off; the ticket query
  is redacted from logs. Each cookie is read by exact name and every copy is tried (D16 f).
- **Guards on every panel request, every method:** exact Host match against the dashboard's accepted host list
  (`localhost` excluded — the client prefills its connect dialog from it); refuse `Sec-Fetch-Site: cross-site` for any
  method and mode; when `Sec-Fetch-*` is absent, `Origin`/`Referer` authority (host **and** port) must equal the
  request authority for non-GET, WebSocket upgrades and SSE opens; `OPTIONS` is refused (a panel's JS is same-origin
  with it; there is no legitimate preflight); **every `Access-Control-*` response header is stripped** so no other
  origin can read a panel response; `SameSite=Strict` withholds the cookie from cross-site fetches on browsers without
  `Sec-Fetch-*`. Header-read and idle timeouts on every listener.
- **Response policy:** drop `X-Frame-Options`; parse every `Content-Security-Policy[-Report-Only]`, remove its
  `frame-ancestors`, then append `Content-Security-Policy: frame-ancestors <dashboard origins on the request's accepted
  host: :11180 and :80>`; add `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`; strip hop-by-hop
  headers; never rewrite bodies; never apply the dashboard's own CSP middleware to panel responses. For the generic
  proxy (reserved): strip `Domain=` from upstream `Set-Cookie` and **drop any upstream `Set-Cookie` named `s111` or
  `p111_*`**; strip `s111`/`p111_*` from upstream requests; add `X-Forwarded-*`. Shim/API paths answer
  `502 upstream_error` (problem+json) when the upstream is down — the SPA fallback never serves `index.html` under an
  API path.
- **Dashboard side:** the LIVE service page gains a **Panel** tab: `<iframe src=<ticket URL> sandbox="allow-scripts
  allow-same-origin allow-forms allow-popups allow-modals allow-downloads" allow="<frame.allow, origin-scoped>">` (never
  `allow-top-navigation`), frame chrome ours (title, health dot, reload, "Open full client" in a new tab — it re-mints
  a session, "Ask the agent about this panel"). The dashboard CSP becomes **per request**: `frame-ancestors 'none'`
  plus `frame-src` listing the enabled panel origins on the request's accepted host. No theming promise; `111-panel/1`
  postMessage (`ready`, `title`, `theme`) is reserved for panels we may write later, `event.origin`-checked.
- **Secure-context gate:** if `needs_secure_context = true` and the dashboard origin is not potentially trustworthy
  (LAN IP, `111.local`), the tab shows "open the dashboard on the node itself" with the loopback URL instead of a frame.
  For Meshtastic (`false`): the panel always frames; its Web Serial/Bluetooth buttons work only on loopback origins and
  the `secure_note` says so.
- **Offline lint (build gate, G3):** headless Chrome over CDP (the I3 harness) loads the panel in the no-internet
  netns and records every `Network.requestWillBeSent` off the panel and dashboard origins; any host not in
  `[offline] optional_hosts` fails adoption; the service page tells the user which hosts an internet-connected browser
  will contact ("offline: map blank"). That is the honest form of "no telemetry" for third-party UIs.
- **Health:** manifest `health = { listen = <port> }` for one-client upstreams (child state + `listening_under`, never a
  connect), `url` only with `multi_client = true`; 2-tick debounce like every service.
- **Contract:** `ServiceRecord` gains `panel: {id, port, entry, needs_secure_context, allow[], optional_hosts[], state}
  | null` (additive; `embed_url` is kept for link-out and is never set when `panel` is non-null). `GET /api/v1/panels`
  lists declared panels and slots.

What stays from D15: one origin for our own UI and `/api/v1`; `frame-ancestors 'none'` + `X-Frame-Options: DENY` on
dashboard responses; no runtime fetch of anything by us; **no TLS in v1** (Chrome 154's HTTPS-by-default excludes
private addresses; `.local` re-checked at the next Layer 4 pass); the Host guard on the main listener; loopback-trust
policy (D16 a) for the dashboard itself. Rejected: sub-path hosting under the dashboard origin; body rewriting;
wildcard DNS or per-panel `.local` names (`<id>.111.localhost` may be added later as an additive prettification on
loopback); forwarding CORS headers.

## 14. Meshtastic (service `meshtastic`, D19 g)

Goal this cycle: a Meshtastic board is **identified** (consented), gets a **service row** whose page hosts
**Meshtastic's own web client**, and its **nodes-heard** feed **Nearby**. No message bridge. "Read-only" means: 111
itself never transmits on the Meshtastic mesh and never changes node configuration; config *reads* (`want_config`) are
fine. **What the user does in the panel (send, configure, reboot) goes to their radio and out over Meshtastic; 111 does
not see or store it** — this sentence is on the service page.

- **One API client.** A Meshtastic node (and meshtasticd) serves one client. **`meshmud`** holds that slot: uplink =
  the board's serial port (USB boards; **Linux only** — Termux cannot open USB serial), `tcp://<board-ip>:4403` (a
  Wi-Fi board; **the phone path**), or `tcp://127.0.0.1:4403` (meshtasticd for SPI/CH341 radios); downlinks = TCP
  `127.0.0.1:11230` only (the presence reader, the shim's per-session connections, later `meshtastic-client`). **No
  HTTP downlink is configured**: `111d`'s built-in **shim** serves `PUT /api/v1/toradio` and `GET /api/v1/fromradio`
  on the panel origin by reframing bodies onto a per-panel-session meshmud TCP connection (the same ToRadio/FromRadio
  bytes behind the 4-byte `0x94 0xC3 len` header — no protobuf decode, no GPL in our binary), so nothing
  browser-speakable listens outside the guarded panel origin and several viewers each own a downlink. A session's
  downlink opens on its first `/api/v1/*` call and closes on cookie revocation or idle; its `FromRadio` buffer is
  capacity-bounded, drop-oldest, with the drop count surfaced in `panel.state`. Config rendered
  by `111d` from the binding in `devices.json`: uplink path validated against `^/dev/(serial/by-id/)?[A-Za-z0-9_.:-]+$`
  or a parsed `host:port`, `advertise = false` (no unrequested mDNS), file 0600. Supervised child; CI-built static
  binaries pinned in the channel manifest.
- **Presence reader** (`meshtastic-presence`, GPL-3.0 Python, own source dir + hash-locked venv like `rns-lock.txt`):
  connects to the TCP downlink, sends `want_config` once, subscribes to NodeInfo / position / telemetry events, writes
  `fabric/presence/meshtastic.json` atomically: `{schema: 1, nodes: [{id, num, long_name, short_name, pubkey?, snr?,
  rssi?, hops_away?, last_heard_hint (node clock, display only), position?, channel_utilization?}]}` with names
  truncated to 40 bytes and control characters stripped. It never sends text, admin or any user packet. `111d` opens
  the file `O_NOFOLLOW`, requires a regular file ≤ 1 MiB, validates the schema (≤ 4096 nodes, typed fields; invalid ⇒
  keep the previous snapshot, bump a log counter), then publishes `GET /api/v1/nearby` + SSE `nearby`. Names are
  **untrusted RF-origin strings**: rendered as text only, handed to the agent inside a labelled untrusted block.
- **Identify (the Meshtastic step; D17 #8):** `POST /devices/{id}/identify` with the consent flag → the daemon's
  port-owner table (`HashMap<port, Owner>`; held ⇒ `409 port_busy`) → meshmud started on the port with the identify
  profile → the reader's `my_node_info` through the TCP downlink → `DeviceRecord.firmware {family: meshtastic,
  version, source: probe}`, `driver.stack = meshtastic`, `service_id = meshtastic`, binding persisted in `devices.json`;
  **the identify child becomes the service child in place** (profile swap, never a second open, so DTR/RTS reset risk
  is tied to this one consented step). RNode (`rnodeconf --info`) and MeshCore steps are named seams for Layer 6's full
  identify sequence. On phones `can_run_here = false` for USB boards; a Wi-Fi board is bound by address.
- **Service row** `meshtastic` (kind `child`): LIVE when meshmud is running **and** the reader has produced a node DB;
  NOT SET when a Meshtastic board is identified but not bound ("Set up with the agent"); ERROR when meshmud crash-loops
  or the uplink is lost > 60 s. `detail`: board, firmware version, nodes heard, channel utilization, panel state.
  Actions: `restart` (both children; the reader after meshmud), `log`. **`radio` row rule:** `Facts.radio` gains
  `meshmud_bound: Vec<port>`; a meshmud-bound board counts as bound (`radio_bound_meshtastic`, detail "driven by
  meshmud"), so the two rows never contradict.
- **Nearby:** real list, grouped by source ("heard via Meshtastic on <board alias>"), fields above, `last_heard` shown
  as a hint. No fake peers; a Meshtastic node is never a contact (different identity system) — "Message them" is not
  offered until a bridge exists (P1).
- **Panel connection:** the web client's HTTP transport asks for a node address; the panel's honest copy gives the
  exact value (the panel's own `host:port`) and the UI pre-connects via the client's URL parameter if one exists
  (verified at build).
- **Honest copy** on the service page (from `02-bridges-and-budgets.md` §4.3): "Meshtastic protects messages with its
  own keys; the default channel's key is public. Nothing here is 111-sealed." Meshtastic is a registered mark, used
  nominatively.
- **Residual under Termux (D16 a):** every app on the phone can reach meshmud's TCP downlink on loopback — for a Wi-Fi
  board that equals the board's own LAN exposure; stated on the service page, not hidden.
- **GPL boundary:** the web bundle is served unmodified from its own directory; the presence reader and meshmud are
  separate processes; no Meshtastic protobuf code enters our Rust binary. Source offers in `THIRD-PARTY-NOTICES.md`
  name the pinned tags (meshmud + its Go closure, meshtastic-python + its pip closure, the web bundle).

## 15. API contract (additive, `/api/v1`; binding form: `interface/api/openapi.yaml`)

| Route | Verb | Purpose |
|---|---|---|
| `/messages/threads` | GET | threads (peer, alias, last message, unread, states summary) |
| `/messages/threads/{peer}` | GET | messages with `state`, `reason`, `received_seq`, `sent_at_claim` |
| `/messages` | POST | `{to, text}` → 202 `{id}`; stays `queued` until a path exists |
| `/messages/{id}/retry` | POST | re-queue (`409` unless `queued|sent`) |
| `/contacts` · `/contacts/{id}` | GET · PATCH · DELETE | list; alias, accept request, pin; forget |
| `/contacts` | POST | `{card}` or `{node_id}` (§7.2) |
| `/contacts/me/card` | GET | own card: `{text, qr: {size, rows}}` (module bitmap; the UI draws it) |
| `/fabric` | GET | links (+profile, state), hub (enrolled, reachable, cursor), custody (count, bytes), store (used, watermark, `store_full`) |
| `/fabric/hub/enroll` | POST | `{address, code}` (member side) |
| `/fabric/hub/offer` | POST · DELETE | hub side: mint an enrollment code / dismiss the outstanding one (loopback-only like `offer`) |
| `/fabric/settings` | PATCH | `hub_role`, `listen_lan`, `custody_opt_in`, per-class quotas |
| `/nearby` | GET | §14 presence (per source) |
| `/panels` · `/panels/{id}/session` | GET · POST | §13 |
| `/devices/{id}/identify` | POST | §14 (consent flag in body; `409 port_busy` when the port is held) |

SSE (same `/events` stream, additive names): `message` (state change, new message, request — **coalesced per message
per tick** so a 16-fragment transfer never floods the 256-entry ring), `contact`, `fabric` (link/hub/custody/store),
`nearby`, `panels`. Enum growth: `ServiceId` + `fabric`, `meshtastic` · `DeviceRecord.actions` + `identify` ·
`reason_code` + `store_full`, `mailbox_full`, `waiting_for_key`, `hub_not_yours`, `port_busy` · `Problem.code` +
`port_busy`, `panel_ticket_expired`, `upstream_error`. Unknown enum values render as `unknown` (U1b rule).

**Services:** `fabric` ("Messaging"): LIVE when the store is open and (an enrolled hub is reachable, or this hub has at
least one member or an attributed peer link); NOT SET when the store is open but nothing is reachable — a hub with no
members reads "waiting for the first node to enroll", a member "Set up with the agent" → enroll; ERROR on store
failure or `store_full`. `meshtastic`: §14.

## 16. UI (Layer 4 tree; one component tree, density flags)

- **Messages** (`/messages`, core): thread list (alias/label, last line, state glyphs, unread), **requests** section
  (INTROs: accept / ignore), composer ("attachments arrive next"), per-message state with the exact copy of §9 and the
  reason when `queued` or rejected, "received" time with the unverified sender claim behind a disclosure. Advanced
  appends: raw `msg_id`, path, custody receipts.
- **Contacts** (`/messages/contacts`): list, add by card/QR scan (`jsQR` over `getUserMedia` on loopback/secure
  origins; file upload fallback) or paste `node1…`, own card (QR drawn from the bitmap + text + copy), pin/alias/forget.
- **Nearby** (`/nearby`): real list per source (§14); radar/map stay Layer 7.
- **Service pages**: `fabric` (links, hub, custody, store gauge, set-up prompts); `meshtastic` (LIVE panel from
  `detail` + **Panel tab** §13 + nodes heard + honest copy incl. the optional hosts and the "panel transmits" sentence).
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
| `nearby.read` | read | → §14 presence; names inside a labelled untrusted block | none |

The agent reaches these through `111d`'s loopback `/api/v1` with a **per-spawn bearer token**: `111d` mints
`AGENT111_DAEMON_TOKEN` into the child's environment (beside the existing `AGENT111_*`), and `require_session` accepts
`Authorization: Bearer <token>` **from a loopback peer only** — loopback trust is never reopened (D16 a stands under
Termux). Tools run inside `111-agent`; writes only via the proposal path (L3 "only write path" rule preserved).

## 18. Security & threat model (hard rules)

1. Verify before state: `pair_mac` → L2 window verdict (bookkeeping only for `msg_seq` kinds: fresh or stale, never
   rejection; the verdict gates attribution, card answers and pushes) → `msg_seq` dedup (or the kind's idempotency
   rule) → store. A failure at any step
   leaves no trace except a bounded log counter. A valid `sig` alone never advances any state.
2. Fail closed on unknown `version`, `crypto_mode`, reserved bits (outer and inner), unknown `kind`, non-empty
   delegation trailer, `frag_count > 16`, `frag_idx ≥ frag_count`.
3. The keyset never enters `mesh111`; sealing, opening, signing and the pair-key ECDH happen behind the identity
   holder's handle inside `111d`; the derived per-contact pair key (a symmetric 32 B MAC key) does cross into
   `mesh111` so the MAC runs there, cached per contact and zeroized on forget — the keyset and ECDH secrets never do.
   No key material on any HTTP endpoint, ever.
4. Metadata stated honestly: `dest_hint` (4 B), counters, sizes and `prio_flags` are cleartext; `tcp-lan` has no
   transport encryption in v1, so an on-path observer learns who talks to which hub and how much; hello and announce
   frames authenticate nothing (§8.1); a card request is a membership oracle bounded to enrolled contacts and members
   on attributed connections; unauthenticated announces can churn the L2 pin cache (harmless under A7: contacts are authoritative).
   Hubs see `dest`, sizes and timing of custody items. Content, sender and kind are sealed.
5. Forward secrecy = L2's: sender-side per packet; recipient-side residual until ratchets (§21). **KCI residual of
   `pair_mac`:** a recipient whose X25519 key leaks can be fed forged pair-authenticated traffic "from" any contact;
   SIGNED classes are immune. Static-static ECDH never protects confidentiality (only this authentication tag).
6. Mailbox abuse bounds are capacity and the enrollment list; there is no proof-of-work and no rate limit (policy). A
   malicious hub can drop, delay or reorder custody items and learn their metadata; it cannot read, forge or redirect
   them (sealed, `pkt_ref`-bound receipts, `msg_seq` idempotency).
7. Panels: third-party code runs on its own origin; every request is guarded (§13), CORS headers never leave a panel
   origin, no loopback skip; `s111` never reaches an upstream; dashboard CSP stays closed; panels that need a secure
   context are never faked; the reserved proxy drops upstream cookies named like ours.
8. Meshtastic: the presence reader has no send path and its output is untrusted text; the panel's writes go to the
   *board* through meshmud (the user's own radio, their decision), never to the fabric; no HTTP listener exists outside
   the guarded panel origin.
9. Agent: reads freely (bearer-token loopback), sends only through proposals.
10. Amateur-licensed regions: no sealed traffic on that radio (devices.md §0.6); region/callsign are human facts (D17 #10).

## 19. Cross-layer seams & amendments

- **Layer 2 (identity) — amendments this spec needs (panel crypto judge; v1 ask):** A1 AD scope = the sealed packet's
  own header; the L6 shim is outside and mutable · A2 inner `sender_node_id(32)` → `sender_ref(4)` + `pair_mac(16)`
  always present (§3; fixes the Noise-"N" sender gap; replay state keyed by the authenticated sender; full NodeId only
  in INTRO) · A3 `msg_seq` = `(epoch, n)` on the wire as two varints, the epoch a freshly committed announce-counter value
  minted once per store open under §4's conditions, `n` committed before seal, frozen old-epoch receive state (§4) · A4 domain tags §3.1 with their pinned inputs · A6 §13 wording: v1
  emits no *unsolicited or periodic* announces; on-demand announces allowed; stale-with-identical-keys announces are
  accepted without state change; card requests served from stored bytes · A7 L5 `contacts` authoritative, pin store a
  cache · A8 `appdata` v1 layout · A9 store-key KDF = keyed BLAKE2b off a keyset-derived secret (`111/store/v1`) with a
  test vector · A10 `transport_id` entries for bridge identities + hash binding for 64 B RNS keys; link keys never
  seed-derived · **A11** replay authority for `msg_seq` kinds = the §4 state; the §6.5 window is not enforced below
  high-water for them, but every packet is still offered to it and its fresh/stale verdict gates attribution and pushes; TLV `0x21` reserved for `signer_pubkey`. **Asked later (v1.1):** A5 crypto-mode `0x02`
  (static-static folded into the KDF, the 16 B saver) and the carve-out that hash-verified chunk frames of public or
  already-encrypted objects may travel unsealed on broadcast bearers.
- **Layer 1 (bootstrap):** new deps with sign-off recorded in `bootstrap/TASKS.md` — `rusqlite`/`libsqlite3-sys`
  (`bundled-sqlcipher-vendored-openssl`), `blake3`, `bao-tree`, `ciborium`; new state dirs (`fabric/`, `fabric/blobs`,
  `fabric/presence`, the presence venv); ports §2; `111 fabric offer|enroll|status` CLI verbs; manifest entries for
  meshmud binaries (built in CI from the pinned tarball), the Meshtastic web bundle (CI-built from the pinned tag when
  upstream ships no asset), the presence venv lock; `AGENT111_DAEMON_TOKEN` per spawn; `gen-notices.py` gains a Go
  path (`go-licenses` or a hand list gated by `check.sh`) and names the pinned tags in the GPL source offers;
  `LICENSING.md` §3 rows for meshmud (+ Go closure, conveyed as GPL-3.0) and the reader venv, §4 wording kept true (no
  foreign source copied into `os/`); the updater gains an intake path for a staged payload at `fabric/blobs/<hash>`
  verified by the same minisign manifest (seam); `check.sh` builds SQLCipher for both targets.
- **Layer 3:** tools §17 + the bearer token; offload stays on the LAN binding + pairing auth (not v1 fabric traffic).
- **Layer 4:** D15 amendment §13; main listener 308 `Host: localhost` → `127.0.0.1`; `require_session` bearer rule (loopback only); Messages/Contacts/Nearby/Settings
  views §16; `jsQR` dependency; `ServiceId` growth (the fixed-size registry arrays grow to 9); `panel` object; per-request
  CSP; `relayed`/`queued` copy; `radio` row rule §14.
- **Layer 6 (`devices.md`):** D19 c amendments (recorded); `Link` trait §8 is the contract its drivers implement;
  `interfaces.toml` is born there and inherits the Meshtastic binding from `devices.json`; the full identify sequence
  (RNode, Meshtastic, MeshCore; port-owner table reused); the MeshCore data-type range registration; INTRO over LoRa.
- **Layer 7:** presence from announces/`nearby` sources; hub-to-hub replication (A+); `KB` consumer; `appdata` TLVs;
  public broadcast frames; announce-as-routing questions stay open there.
- **Layer 8:** `SYNC` class + per-peer cursors; the append-only log is an opaque consumer.
- **Layer 9:** `PATCH` class, `AGENT_OP`, `RECEIPT_STRONG`; the apply→verify→undo engine (P4).

## 20. Verification (lock gates; netns, no hardware)

G1 **three-node harness** (a named task: `interface/verify/smoke.sh` extended to three `NODE111_ROOT`s in three netns on
a veth bridge, real `111d` binaries): two members + one hub — card exchange, `node1…` card request, INTRO request accept,
`queued → sent → relayed → delivered`, a message to a non-member answered `REJECT_UNKNOWN_DEST` and shown honestly, hub
killed mid-transfer and resumed by cursor, duplicates re-ACKed not redelivered, Tier-1 fragments over a test-only
`lossy-loopback` link at a forced 160 B profile with loss injection and SACK retransmit, `store_full` refusal path, a
replayed announce on a fresh socket receives no push, a replayed sealed packet on a fresh socket never attributes it, a
sender crash with items in custody followed by a direct new-epoch packet delivers every message exactly once · G2 SQLCipher static builds (x86_64 + aarch64) in `check.sh`;
the aarch64 binary opens, writes and prunes a store in an Android emulator in CI (or recorded as an external gate
where no emulator runs) · G3 the Meshtastic panel runs offline in the netns against a **stub** meshmud TCP downlink:
bundle served, ticket → cookie → iframe, shim round-trip (`PUT /toradio`, `GET /fromradio`), framing headers rewritten,
CORS headers absent, a cross-site `GET /fromradio` from a second netns origin returns no body and dequeues nothing, a
drive-by POST refused, a spent ticket answers 403 and the dashboard re-mints, the offline lint's CDP recording shows
only declared optional hosts, the connect-dialog prefill equals the panel authority, the dashboard opened on `http://localhost` frames the panel
after the 308 · G4 envelope test vectors (frozen
files like `id111`'s): seal/open with `sender_ref` + `pair_mac`, SIGNED classes with `dest`, a re-sealed valid
signature never advances any state, forged `sender_ref` never touches a window, reserved-bit rejection, fragment bounds,
ACK block round-trip incl. partial bitmaps, piggyback and a two-epoch ACK (frozen ranges), custody receipt vectors, enrollment keyed hash, store KDF ·
G5 fable re-check of §3–§9, §13–§14, §18–§19 against the panel's accepted fixes.

**External gates before the first signed release (hardware):** meshmud with a real Meshtastic board — serial open
without an unwanted reset, config fan-out to a second TCP client, the shim's per-session downlinks, Termux build of
meshmud (TCP-to-board uplink) + the presence venv · two-board MeshCore bench for the `kiss-modem` seam (Layer 6).

## 21. Deferred (recorded, not forgotten)

Attachments and the Tier-2 transfer on the wire (v1.1; §5, §11 pinned) · hub-to-hub replication (A+) and cross-hub
custody · multi-hop discovery and announce-as-routing (L7) · INTRO over LoRa (card in fragment 0; fragments with an
unresolvable `sender_ref` wait in a capacity-bounded pending-INTRO buffer until fragment 0 lands) · `kiss-modem`,
`meshtastic-client`, `rns-bridge`, `ble`, online Noise IK / TLS-RPK links (L6) · message bridges (P1) · sender-key groups
and MLS behind a hub DS · rolling recipient ratchets (RATCHET flag, TLV 0x10) · erasure coding (`coding` field; RaptorQ
IPR review) · crypto-mode `0x02` and the unsealed-chunk carve-out (A5, v1.1) · owner-linked device replicas (Delta
Chat ceremony + L2 attestation) · `age` recipient-mode export of history and the "export before prune" offer ·
SQLCipher 5 migration · the generic reverse proxy with WebSocket/SSE (first consumer MeshChatX, P7) · MeshMonitor as an
optional presence/MeshCore service · `<id>.111.localhost` panel origins · TLS · camera QR scanning on LAN origins (needs
a secure context) · KB/UPDATE/PATCH consumers (L7/L9) · LXMF-style stamp costs (rejected as policy unless a network
imposes them) · per-panel postMessage protocol.

## 22. KB adoption list (entries written at build; all `candidate`)

`rusqlite` · `sqlcipher` · `blake3-bao` · `meshmud` (license note: own code MIT, binary conveyed GPL-3.0) ·
`meshtastic-web` · `meshtastic-python` (presence reader dep; GPL note) · `ciborium` · `jsqr` · `age` (deferred
export) · `lxmf` (format facts; bridge target; Reticulum License) · `bundle-protocol-v7` (protocol; why not the core) ·
`dtn7-rs` (later gateway bridge) · `meshmonitor` (optional later service) · `meshchatx` (parked P7; MeshChat under
Alternatives) · case studies: `briar-mailbox`, `delta-chat-securejoin`, `home-assistant-ingress`, `sandstorm-sandboxing`
· concepts: `lora-airtime-budget`, `secure-contexts`, `store-and-forward-states` · updates: `meshcore` (KISS modem,
`GRP_DATA` flood, repeater as Cat-1), `meshtastic` (`PRIVATE_APP`, rebroadcast modes, 2.8 signing, one-client limit),
`reticulum` (no-import carriage facts). New tags: `messaging`, `panel`, `dtn`.
