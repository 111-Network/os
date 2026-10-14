# Layer 5 — Messaging & Data: options brief

Status: 🧭 deciding · 2026-10-14 · lead-written from the 8-agent sweep (`layers/05-messaging/research/`: 5 lenses, opus critic,
3 gap scouts; ~1.9M tokens, ~700 tool calls, 62–75 sources per lens). Reads with: `os/README.md` §5 · D13 (identity
wire contract) · D17 (device decisions) · D18 (this cycle's scope) · `cross-layer/devices.md` · `layers/05-messaging/research/00-code-seams.md`.

Owner scope for this cycle (D18): **Layer 5, designed with Layer 6 in mind** · the unified dashboard **hosts** adopted
projects' own panels ("we do ALL the heavy lifting") · Meshtastic = identified + service row with its panel + read-only
presence, no bridge.

## 0. What the research settled (facts that bound every option)

1. **No adopted project carries our envelope.** Nothing moves an L2 sealed packet across ~160–255 B LoRa frames without
   a wall clock, with our identity, and with a Cat-1 port. IETF Bundle Protocol v7 (RFC 9171) is the closest standard
   and fails on three counts: a mandatory clock-based lifetime (no "never expires"), a 37–57 B primary block repeated on
   every fragment, and no maintained LoRa convergence layer. Its best Rust implementation (dtn7-rs, MIT/Apache) is a
   daemon with global state, a sled store and an unauthenticated loopback API, last tagged 2024-03. (lens 1)
2. **Reticulum can only be a bridged network, never the fabric.** The only ways to move bytes through rnsd without
   `import RNS` are an adopted app's API (MeshChat/MeshChatX HTTP+WS: content, image, audio, file fields only), lxmd's
   inbound hook, and rnsd config. Both reimplementations in Rust/Go are non-upstream (one AGPL, one MIT, one AI-ported)
   and upstream publicly disputes their standing. D17 #12 stands. (lens 1, gap 1)
3. **The LoRa floor is 160 B, not 233 B.** MeshCore repeaters flood only `GRP_DATA` (163 B payload); its 184 B
   `RAW_CUSTOM` type is never flood-routed (verified in `Mesh.cpp`). Meshtastic `PRIVATE_APP` is 233 B but routers set
   to `CORE_PORTNUMS_ONLY` drop it. RNode frames are 254 B. So one fragment size, **F = 160 B**, fits every bearer.
   (lens 2, critic)
4. **The built sealed packet costs 91 B before any Layer 5 field**: header 11 B (version, mode, counter, extra_len) +
   epk 32 B + tag 16 B + the sender's 32 B NodeId inside the ciphertext (`id111/src/seal.rs`). At 160 B that leaves
   69 B; at 233 B, 142 B. The sender NodeId is also only a *claim*: the construction is Noise pattern "N" (sender
   unauthenticated), so a forged packet can poison the claimed sender's L2 replay window. Both facts drive the envelope
   picks in §3 and the L2 amendments in §4. (lens 4, critic conflict 3, seal.rs)
5. **Hosting other projects' panels is a solved pattern, but only one shape fits an offline LAN.** Home Assistant
   Ingress (sub-path, same origin) hands third-party JS our cookie and breaks apps with absolute URLs (MeshChat, Flutter
   clients); Sandstorm's wildcard subdomains need DNS we do not have on `111.local`; Umbrel/CasaOS use **one port per
   app**. Framing headers are a proxy rewrite (`frame-ancestors` wins over `X-Frame-Options`). Secure-context APIs (Web
   Serial/Bluetooth) work only on loopback origins and cannot be fixed by hosting. (lens 3)
6. **Encrypted store: the adopted path exists and was built.** rusqlite with `bundled-sqlcipher-vendored-openssl` is
   Delta Chat's default; it compiles musl-static for x86_64 and aarch64 with our toolchains (measured: +6.4 / +5.4 MB on
   a 7.0 / 5.7 MB binary; plain SQLite +1.9 MB; redb +0.8 MB). BLAKE3 + bao-tree gives verified streaming of bulk
   objects at +94 KB and is transport-agnostic; iroh-blobs is QUIC-only, pre-production, +13.3 MB. (lens 5)
7. **LoRa carries kilobytes.** A 4 KB manifest is 19–33 frames (40–69 s at LongFast); a 32 KB KB delta 5–9 min; the
   5.7 MB binary 15–27 h of airtime. EU duty cycles (1% typical, 10% in one sub-band) multiply wall time 10–100×. Bulk
   classes belong on IP links; this is physics the fabric advertises, not policy. (lens 2, lens 5)
8. **Two owner decisions need a sentence changed, not reversing.** D17 #1 ("RNode default makes the board an rnsd
   interface") collides with 111 owning the port; D15's iframe ban becomes "no iframe of anything that is not a
   registry-declared panel". `cross-layer/devices.md` §0.7 is stale (Sideband is active, 2.2.0 on 2026-10-01; still CC-NC, still
   rejected). (gap 1, lens 3)

## 1. What Layer 5 is, in one paragraph

A **fabric** (`mesh111`, a Rust library crate under `os/messaging/crates/`, wired into `111d` as a module so the
identity keyset never leaves the daemon) that stores, forwards and delivers **envelopes** sealed with the Layer 2
construction, over **links** that Layer 6 plugs in (`tcp-lan` built now; `kiss-modem`, `meshtastic-client`,
`rns-bridge`, `ble` as seams); an **encrypted store** with capacity-only pruning; **contacts** (cards, QR, `node1…`,
message requests); a **home-hub mailbox** for store-and-forward; a real **Messages** view; **hosted panels** for
adopted projects; and the **agent tools** to read and send. Human chat, agent operations, KB sync, updates and patches
are traffic classes of the same fabric. Everything below the fabric is adopted; the fabric itself is the one piece of
custom code this layer justifies, and it is seven small parts (§2.1).

## 2. Polled forks (owner decides; recommendation first)

### 2.1 Fabric core

| Option | What it is | Trade-off |
|---|---|---|
| **A. Thin custom Rust core over adopted pipes (recommended)** | `mesh111` reimplements exactly: envelope codec + message ids · encrypted store with capacity eviction (adopted DB engine, §2.5) · duplicate suppression + anti-entropy summaries · fragmentation with selective retransmit · authenticated delivery and custody receipts · contact tracking from cards/announces · the `Link` trait + local API + agent tools. Not reimplemented: crypto (L2 `id111`), radios (RNode/MeshCore/Meshtastic firmware), IP transport, bulk verification (BLAKE3/bao). BPv7 stays an optional later gateway bridge (envelope-in-bundle via the `bp7` codec, dtn7-rs/uD3TN as a separate process). | We own ~7 components (small, testable, C-portable for Cat 1). Fits 160 B frames, no clock, our identity. |
| B. BPv7 via dtn7-rs as the core | Envelope rides as a bundle payload; dtn7-rs daemon does store + routing; we write an external CLA (ECLA JSON over WS) for LoRa. | Zero store/routing code, but: clock-based lifetime, +37–57 B per fragment (leaves 71–91 B at MeshCore sizes), we still write the LoRa CLA, single-maintainer daemon, sled store, loopback API with no auth (on Android any app can reach it), 9–10 MB tarball, no aarch64 build. A second protocol on air. |
| C. LXMF/Reticulum (or Berty, qaul, Veilid, SSB) as the fabric | Least code. | Fails D17 #12 (Reticulum dependence) and rule 2 (their identity replaces ours); none speaks 160 B LoRa except LXMF. Listed so the choice is visible. |

**What flips A → B:** valuing "zero custom store-and-forward" above LoRa efficiency and the no-clock rule, and accepting a
custom LoRa CLA plus clock workarounds as the custom part instead.

### 2.2 What v1 delivers (keys, custody, "relayed")

Layer 2 says v1 emits no announces, and Layer 7 (presence/routing) does not exist. So v1 must say how two nodes learn
each other's keys and who carries a message when the recipient is away.

| Option | Scope | Needs | Trade-off |
|---|---|---|---|
| **A. Direct links + home-hub mailbox + contact cards (recommended)** | Contacts enter by **card** (the L2 genesis announce as a QR / 220-char text; works with zero connectivity), by **`node1…`** (the node fetches and verifies the announce over any link; Delta Chat SecureJoin shape), or by **INTRO** (first message carries the sender's announce; shows as a *message request* the user accepts or ignores). Announces are emitted **on demand only** (card, link-up, INTRO), never on a timer. A node enrolls with a **home hub** (pair-code ceremony reused from L4 §6.9); the hub is custodian: `CUSTODY_PUT / CUSTODY_ACK / MAILBOX_FETCH / MAILBOX_COMMIT` over sealed frames on a length-prefixed TCP stream (the same frames later ride LoRa). Enrolled-only; `store_full` refuses honestly. | L6: only the `tcp-lan` adapter (built with L5). L7: nothing. L2: 4 small amendments (§4.1). | Smallest, deterministic, works for Termux phones (no multicast). No multi-hop discovery yet. |
| A+. A + hub-to-hub replication | Hubs with configured peers exchange summary vectors and replicate custody. | Same | Cheap on always-on hubs; storage multiplies; replicas evict first. Follows A without redesign. |
| B. LAN multicast announces + epidemic among all nodes | Every node announces periodically; strangers TOFU-pinned; epidemic forwarding. | Multicast adapter; L7 presence + announce rate policy; per-announce fsync. | Phones excluded (no multicast receive); flash wear; pulls L7 policy forward. |
| C. Announce-as-routing across hubs | Dynamic custody choice by announce flooding. | `kiss-modem` adapter, Cat-1 verify policy, mesh-epoch source. | Most of Layer 7 now. |

**Delivery states, made honest** (the L4 stub promises four): `queued` = in outbox, no path or no key · `sent` = frame
written to a link (LoRa: modem accepted TX) · `relayed` = an authenticated `CUSTODY_ACK ACCEPTED` from a mailbox-role
node (a repeater echo is *not* relayed; it is detail on `sent`) · `delivered` = the recipient's own ACK, which alone
releases the sender's copy. Custodians: hubs and always-on laptops; phones opt-in.

### 2.3 Who owns the LoRa radio, and what a Cat-1 relay runs (Layer 6 direction, decided now)

One serial port has one owner. D17 #1 + `cross-layer/devices.md` §4.1 give a blank board RNode firmware *as an rnsd interface*,
which leaves 111 with no LoRa path of its own. The fabric fork above cannot be locked without this.

| Option | Who holds the port | Cat-1 relay | Trade-off |
|---|---|---|---|
| **1. 111-owned boards run the MeshCore KISS modem; Cat-1 = stock MeshCore repeater (recommended)** | 111d, as a raw modem (255 B frames, CSMA, `SetRadio`, airtime report; MIT, documented protocol). 111d builds MeshCore `GRP_DATA` frames (163 B payload) that **any stock MeshCore repeater floods blind** (seen-table, path, 64 hops). Reticulum users keep RNode on a second board, or opt into "shared" mode (option 2) on one board. | **No firmware of ours**: the relay is MIT, released, already deployed; we register a data-type range with MeshCore (`number_allocations.md`). | Amends D17 #1 for 111-owned boards (blank board → "111 modem" offered first, RNode/Meshtastic as the alternatives). Images for the KISS-modem env are not in upstream releases (we CI-build and pin, per D17 #9). Two-board bench before L6 locks (flood policy, scoped regions, neighbours' airtime etiquette). |
| 2. RNode everywhere; 111d owns the port in HOST mode; rnsd attaches by loopback TCP-KISS | 111d sets radio params and fans RX out to the fabric and to rnsd's `TCPClientInterface kiss_framing=True` (documented for "external devices and programs"). On-air RNode frames stay Reticulum-compatible; 111 frames are RNode-framed and so **invisible to MeshCore/Meshtastic repeaters** — they travel hub to hub only. | **111-native relay firmware, later** (reinvents a LoRa repeater; per-board build, signing, OTA). | Keeps D17 #1 whole; rnsd loses RSSI/airtime detail and we render `bitrate`/`announce_cap` for it. Writes the RNode host driver from GPL firmware source only (never from `RNodeInterface.py`). |
| 3. Defer the firmware choice to Layer 6 | Layer 5 locks only the **160 B** fragment floor and the `Link` trait. | — | Nothing is reversed, but the Cat-1 story stays open and the fabric's frame budget is designed blind. |

Rejected: rnsd owns the port and 111 traffic rides LXMF (fails D17 #12; no clean carrier); one board per stack as the
only answer (phones cannot run RNodeInterface anyway).

### 2.4 Hosting adopted projects' panels (amends D15)

| Option | Mechanics | Trade-off |
|---|---|---|
| **A. One origin per panel, by port, behind 111d's proxy (recommended)** | 111d binds one listener per enabled panel (block 11190–11229; loopback always, LAN when LAN sharing is on) and reverse-proxies it (hyper-util glue or the MIT `axum-reverse-proxy` crate: WebSocket upgrades, SSE, large bodies). Per panel: a manifest `assets/panels/<id>.toml` (source `static|child|detected`, entry path, health, `needs_secure_context`, allowed features, auth mode). Auth: the dashboard mints a one-time ticket (`POST /api/v1/panels/{id}/session`), the panel listener swaps it for a host-only cookie `p111_<id>` (no `Domain`, HttpOnly, SameSite=Strict); exact-Host + `Sec-Fetch-Site` guards on mutations (this is where MeshChat's missing access control is supplied); `s111` stripped upstream. Response rewrite: drop `X-Frame-Options`, replace `frame-ancestors` with the dashboard origins, strip `Domain=` from `Set-Cookie`, never rewrite bodies. Dashboard: LIVE page gains a **Panel tab** (`<iframe sandbox allow=…>`), "Open full client" kept; dashboard CSP keeps `frame-ancestors 'none'` and adds an exact `frame-src` list. Panels flagged `needs_secure_context` are framed only on a loopback origin, with honest copy on LAN. Offline lint: every panel must run in a no-internet netns. | Works for any app and any host form; LAN-safe. Cookie jar is shared across ports on one host (mitigated above). More listeners. |
| B. `<id>.111.localhost` subdomains on one listener | Separate cookie jars per panel. | Loopback only (no wildcard mDNS; Android does not resolve `.local`); Safari before macOS 26 fails. Additive later, never alone. |
| C. Home-Assistant-style sub-path under the dashboard origin | One listener. | Hands third-party JS our origin and cookie; breaks MeshChat and Flutter apps (absolute URLs). Rejected. |
| D. Keep D15 link-out only | Zero risk. | Not "hosted"; contradicts D18 #2. |

### 2.5 Store engine

| Option | Cost | Trade-off |
|---|---|---|
| **A. SQLCipher via rusqlite `bundled-sqlcipher-vendored-openssl` (recommended)** | +6.4 MB x86_64 / +5.4 MB aarch64; C + vendored OpenSSL 3.6 build (~2 min cold; perl+make present); pin the 4.x file format (5.0 beta is incompatible; plan `cipher_migrate`). | Adopted and in production (Delta Chat core). Page-level AES-256-CBC + HMAC. SQL gives dedup (`UNIQUE`), threads, FTS, migrations. Needs the L1 dependency sign-off (C backend). |
| B. Plain SQLite + our per-record XChaCha20-Poly1305 | +1.9 MB | L2-native primitives, but we invent and audit a record format and leak metadata (sizes, indexes). |
| C. Pure-Rust KV (redb/fjall) + per-value AEAD | +0.8–1.0 MB | Smallest, no C; hand-built indexes and search; better for Cat-1 rings than for a Messages view. |

Store key (either way): one 32 B raw key behind a `KeyProvider` seam — **Mode S** default = HKDF-SHA256 from the L2
keyset (`111/store/v1`; a salvaged DB opens after restore-from-words); **Mode T** on Cat 4 with TPM2/SE050B = random key
sealed in hardware (L2 §5). Never bind to an OS keyring without a recovery path (Signal Desktop lesson). Blobs are files
named by BLAKE3 hash with a refcount table; private blobs encrypted once with an object key carried in the envelope.

### 2.6 Meshtastic this cycle: identify + hosted panel + presence, with one API-client slot

_Filled from the gap-3 scout (`research/07-gap-3-meshmonitor.md`) — see the poll._

### 2.7 Second hosted panel this cycle

| Option | Trade-off |
|---|---|
| **A. MeshChatX (recommended)** | 0BSD+MIT fork of MeshChat, listed in the Reticulum manual's software chapter, PyPI wheel with bundled UI, `--headless --no-https --auth`, proxy-aware WebSocket Origin check, Termux guide; v4.10.7 2026-10-11. Gives users LXMF messaging inside the dashboard on day one, proves the WebSocket proxy, and exercises D17 #5 (`share_instance = yes`). Needs its own venv with `rns` installed from PyPI (separate adopted program importing Reticulum in-process: allowed; LICENSING.md gets a line). Unaudited fork: the panel checks provenance. |
| B. Original MeshChat (MIT) | Simpler, 1.3k stars, but no auth and no Origin check (the proxy must supply both) and root-absolute paths. |
| C. Meshtastic only this cycle | Smallest. The Reticulum side stays "transport LIVE" with no messaging surface. |

## 3. Detail-level picks (spec-reviewed, not polled — say so if you disagree)

- **Envelope layout.** L6 shim (1 B: frame_type, hop_limit — mutable, outside the sealed packet, never in AD) ‖ L5 outer
  header in the sealed packet's `extra` (prio_flags 1 B: 2-bit relay priority, GROUP, RATCHET=0; `dest_hint` 4 B =
  NodeId[0..4], a locator never an authenticator) ‖ epk ‖ ciphertext (inner: `sender_ref` 4 B, `kind`, `flags`, varint
  `msg_seq`, optional TLVs, payload, `pair_mac` 16 B or `sig` 64 B) ‖ tag. Hand-packed fixed offsets outside, compact
  binary inside, deterministic CBOR (ciborium) for Cat 2+ message bodies. **Not COSE** (no XChaCha20 in the registry;
  +31 B per packet), not protobuf/postcard as the wire.
- **Authentication of sender = `pair_mac`** (16 B; key = BLAKE2b(`111/pair/v1` ‖ X25519(static, static) ‖ both pubkeys);
  covers header ‖ epk ‖ inner). Fixes the Noise-"N" gap with no L2 construction change; crypto-mode `0x02` (fold the
  static-static secret into the L2 KDF, 0 B) is *reserved* as the 16 B saver. **Signatures** (`111/msg/v1`, 64 B,
  over sender ‖ stream ‖ seq ‖ kind ‖ BLAKE2b(payload), so they survive re-fragmentation) are mandatory for stored,
  public and code classes: group posts, anything a hub holds for others, KB entries, patches, update manifests.
- **Message identity = (sender, stream, `msg_seq`)**: 1–3 B on the wire, O(1) receiver state (contiguous prefix +
  bounded ranges), never forgets below the prefix, no clock. Duplicates are re-ACKed, never redelivered. Content hash
  (BLAKE3) only for blobs. Restore/unclean loss ⇒ `msg_seq` jumps 2^32 (L2 §8.4 amendment).
- **Fragmentation, three tiers.** Tier 0: one packet (chat). Tier 1 (≤ ~16 fragments): **fragment-then-seal** — every
  fragment is its own authenticated packet (`frag idx,count` +2 B), resumable, no fragment poisoning; re-seal on
  retransmit with ≤ 16 outstanding so the L2 32-entry window never starves. Seal-once-then-fragment is rejected (raw
  slices cannot be verified; one bad slice = whole message). Tier 2 (bulk objects): a sealed **advert** (BLAKE3 root,
  size, chunk group, `coding`) then **hash-verified chunks** (bao-tree): unsealed when the object is public or already
  encrypted to its recipient (needs the L2 carve-out in §4.1), sealed otherwise. Erasure coding reserved (`coding` field;
  RaptorQ after IPR review, Reed-Solomon as the alternative), not built.
- **Budget, honest (F = 160 B flood frame; 233 B Meshtastic/RNode):** with the 4 B `sender_ref` amendment, Tier-1
  payload ≈ 69 B / 142 B per frame (pair-auth) or 21 B / 94 B (signed). Without the amendment (32 B NodeId inside),
  41 B / 114 B. Bulk classes default to IP links by cost class; a user or the agent may push a small object over LoRa
  explicitly. The adapter advertises MTU, duty-cycle and airtime as physics; the fabric never rate-caps.
- **Receipts.** `kind = ACK` packets with cumulative `ack_prefix` + up to 4 SACK ranges (~4 B each), authenticated by
  `pair_mac`, domain `111/ack/v1`; optional `RECEIPT_STRONG` (BLAKE2b-128 of payload) for agent ops and patches; optional
  transferable signed receipt (`111/ack-sig/v1`) on request. Custody receipt `111/custody/v1` = status (ACCEPTED,
  DUPLICATE, REJECT_FULL, REJECT_UNENROLLED) + custody_seq + BLAKE2b-128 over the exact sealed bytes (forces the custodian
  to hold them). ACKs piggyback on reverse traffic.
- **Groups v1 = pairwise fan-out** (N sealed packets, per-member ACK, exact delivery tracking, no key state). `GROUP`
  flag + `stream_id` reserved for sender keys (G1); MLS only ever behind a hub delivery service (Cat 2/4), not v1.
  Public broadcast is a separate kind: cleartext by design, always signed.
- **Ratchets reserved, not built**: RATCHET flag + 4 B `ratchet_id` + a 32 B inner TLV slot (worst case +38 B). A
  Reticulum-style rolling recipient ratchet (capacity-evicted, never time-evicted) is the offline-viable design.
- **No timestamp field.** Order = (sender, stream, seq) + `seen_peer_seq` for happens-before across directions; display
  = local receipt time, labelled; an optional sender-claimed `sent_at` in the CBOR body shown as unverified.
- **Traffic classes**: 2-bit relay priority in the clear (control / interactive / bulk-small / bulk-large) orders queues
  and pruning; fine `kind` (human, agent op, KB, update, patch, ACK, custody, sync) stays inside the ciphertext.
- **Pruning (capacity only)**: trigger on live bytes + a 1% free-disk floor; order = transit custody and re-fetchable
  public chunks (size × age, LXMF-style) → superseded releases/fetched deltas → own history oldest-first only at hard
  capacity after an export offer → **never undelivered outbound** (refuse new custody, show `store_full`). Age = local
  insertion sequence, not a clock. No per-sender limits (policy).
- **Multi-device v1 = hub-is-home**: messages live on the hub; phones and laptops are companions through the paired
  dashboard (built). A Termux node is its own identity. Owner-linked replicas (Delta Chat's QR + one-shot stream pattern)
  wait for L2 attestation. Same seed on two devices stays an error (identity collision).
- **Online/interactive class (L3 offload) is not v1 fabric traffic**: offloaded inference keeps using L3's LAN binding
  behind L4's pairing auth; the fabric carries store-and-forward only. Noise IK / TLS-RPK links (L2 §6.6) land with L6's
  online adapters, with a **random link key bound by L2 §10 attestation** (never seed-derived — F2 rejected; never the
  identity key for handshakes — §6.1). iroh is therefore not in v1 (QUIC-only, +13.3 MB, pre-1.0 blobs); the `tcp-lan`
  link is sealed frames on a length-prefixed TCP stream.
- **Bridges this cycle**: none. Seams specified: `meshtastic-client` (PRIVATE_APP 233 B; the firmware rewrites `from`,
  so a bridge always speaks as its own node; one bridge identity per foreign network, bound by L2 §10 attestation),
  `rns-bridge` (MeshChat(X) API / lxmd inbound hook; LXMF address is a transport address, our NodeId stays inside),
  `meshcore-client`. Honest-boundary copy per network (`research/02-bridges-and-budgets.md` §4.3) goes into the KB entries.
- **Agent tools**: `messages.read` (threads, states, contacts), `messages.send` (**proposal-gated** like every write:
  the agent drafts, the human sends), `contacts.read`, `fabric.read` (links, custody, store usage). Still shell-free,
  fixed, schema-validated (D10).
- **Legal/airtime rule carried from cross-layer/devices.md**: no sealed blobs on amateur-licensed regions; region/callsign asked once
  (D17 #10); Meshtastic trademark used nominatively.

## 4. Sibling amendments this layer needs (recorded with the decision)

### 4.1 Layer 2 (identity) — for the adversarial panel's crypto judge

1. **AD scope**: AD = the sealed packet's own header only; the L6 shim is outside and mutable.
2. **Sender identification**: inner `sender_node_id(32)` → `sender_ref(4)` + `pair_mac(16)`; the full NodeId travels
   only in INTRO/first contact. Rationale: the current inner NodeId is an unauthenticated claim (Noise "N"); a forged
   claim can poison the claimed sender's §6.5 window. Windows must be keyed by the *authenticated* sender.
3. `msg_seq` forward jump (2^32) on restore and unclean loss (§8.4).
4. Domain-tag registry: `111/msg/v1`, `111/ack/v1`, `111/ack-sig/v1`, `111/pair/v1`, `111/custody/v1`,
   `111/custody-sig/v1`, `111/store/v1`.
5. Crypto-mode reservations: `0x02` (static-static folded into the KDF, 0 B pair auth), `0x03` (bulk chunk mode); and a
   carve-out: hash-verified chunk frames of public or already-end-to-end-encrypted objects may travel unsealed after a
   sealed advert.
6. §13 wording: "v1 emits no *unsolicited or periodic* announces; on-demand announces (card, link-up, INTRO) are allowed."
7. Contacts: the L5 `contacts` table is authoritative for user-added peers; the 256-entry pin store is a cache (or
   user-added pins are exempt from LRU).
8. `appdata` v1 layout: `[ver=1][roles u16][reserved]` + optional 8 B home-hub hint; L7 owns extensions.
9. Store key: HKDF-SHA256 off the keyset with info `111/store/v1` (Mode S) or the reserved BIP-85 sibling path; test
   vector either way.
10. `transport_id` registry entries for bridge identities (Meshtastic X25519, MeshCore Ed25519, RNS destination hash,
    Delta Chat fingerprint, Briar author id) + a hash binding for 64 B RNS keys.

### 4.2 Layer 4 (interface) — D15 amendment "hosted panels"

The D15 rejection "iframe-embedding third-party web UIs" narrows to **no iframe of any URL that is not a
registry-declared panel**. A panel is declared in `assets/panels/<id>.toml`, runs on its own origin (own port) behind
111d's proxy, and is framed only by the dashboard. §6.6: a LIVE page shows the native control panel *and* a Panel tab;
"Open full client" stays. `ServiceRecord.embed_url` is superseded by an additive `panel` object; `ServiceId` gains one
id per hosted project. §11 unchanged plus `POST /api/v1/panels/{id}/session` and cookie `p111_<id>`. Stays: single origin
for our UI and `/api/v1`; `frame-ancestors 'none'` on dashboard responses; no runtime fetch; **no TLS in v1** (Chrome 154's
HTTPS-by-default excludes private addresses; re-check `.local`); loopback-trust policy (D16a). Messages view goes real;
`relayed` copy: "a mailbox node saved it and will hold it for them."

### 4.3 Layer 6 brief (`cross-layer/devices.md`) and D17

- D17 #1: "RNode is the default firmware for blank boards" → "a blank board is offered the **111 modem** first (per
  §2.3's pick), RNode and Meshtastic as alternatives; a board already running something is left alone." Each board gets
  `owner = 111d | rnsd` in `state/interfaces.toml`; rnsd attaches to a 111d-owned board only by loopback KISS on request.
- D17 #5 stands (`share_instance = yes` default) and is now *required* whenever a Reticulum client panel is enabled.
- §0.7 corrected (Sideband active, still NC). §2.1 gains the MeshCore KISS modem row and the "one owner per port" rule.

### 4.4 Layer 1 (bootstrap) — dependency sign-offs to record

`rusqlite` + `libsqlite3-sys` (`bundled-sqlcipher-vendored-openssl`; MIT; brings C + OpenSSL 3.6 Apache-2.0) ·
`blake3` + `bao-tree` (MIT/Apache; +94 KB) · `ciborium` (Apache-2.0) · reverse-proxy glue (hyper-util, already in the
graph, or `axum-reverse-proxy` MIT) · `age` (exports only, later). Python venvs per adopted panel (MeshChatX + `rns`) ride
the hash-locked install path like `rns-lock.txt`; panel static bundles are hash-pinned in the channel manifest. No
runtime downloads anywhere.

### 4.5 Layer 3 (intelligence)

Offload stays on the LAN binding + pairing auth in v1 (not fabric traffic). New tools per §3. The agent never sends a
message without a human accept (proposal flow).

## 5. Candidate KB entries (enter `candidate`; written at build)

`rusqlite` · `sqlcipher` · `blake3-bao` (project; verified streaming) · `age` (project; recipient-mode export) ·
`meshtastic-web` (project; hosted panel, GPL-3.0-only, static bundle) · `meshchatx` (project; Reticulum/LXMF panel; MeshChat
under Alternatives) · `lxmf` (project; Reticulum License; format facts; bridge target) · `bundle-protocol-v7` (protocol;
reference; why not the core) · `dtn7-rs` (project; later gateway bridge) · `rnode-firmware`, `meshcore` update (KISS
modem mode, `GRP_DATA` flood facts, repeater as Cat-1), `meshtastic` update (PRIVATE_APP, rebroadcast modes, 2.8 signing)
· `briar-mailbox`, `delta-chat-securejoin` (case studies: mailbox pattern, out-of-band contact exchange) · `home-assistant-ingress`,
`sandstorm-sandboxing` (case studies for hosted panels) · concepts: `lora-airtime-budget` (duty-cycle table, time-on-air),
`secure-contexts` (what works on `http://` LAN), `store-and-forward-states` (queued/sent/relayed/delivered semantics).
Plus whatever §2.6 adopts (MeshMonitor or the presence sidecar). New tags: `messaging`, `panel`, `dtn`.

## 6. Verification gates before the spec locks (netns, no hardware)

G1 two `111d` nodes + a hub in a netns: card exchange → INTRO request → queued/sent/relayed/delivered, hub killed mid-way,
resume by cursor, duplicates re-ACKed not redelivered · G2 SQLCipher musl builds (x86_64 + aarch64) in `check.sh`, a
Termux run of the aarch64 binary opening the store · G3 Meshtastic panel bundle and MeshChatX run offline in the netns
(no CDN fetch; WebSocket through the proxy; ticket → cookie → iframe) · G4 envelope test vectors (seal/open with
`sender_ref` + `pair_mac`, fragment-then-seal at F = 160, window behaviour at 16 outstanding) · G5 the fable panel's
crypto judge on §4.1 (Noise-N finding, pair_mac, chunk carve-out). Hardware (L6 lock, not L5): two-board MeshCore bench
(flood of our `GRP_DATA` type, scoped regions, airtime), rnsd tolerance of foreign frames in shared mode.

## 7. Legal flags (lawyer's list, not blockers)

GPL-3.0 firmware images (RNode, Meshtastic) and the GPL Meshtastic web bundle in a signed manifest carry a source-offer
duty · RaptorQ (RFC 6330) Qualcomm IPR disclosures before any erasure coding ships · MeshChatX imports Reticulum
in-process as a separate adopted program (fine under our boundary; add the line to `LICENSING.md`) · SQLCipher BSD-3 and
OpenSSL Apache-2.0 notices (gen-notices covers) · Meshtastic is a registered mark (nominative use only).

## 8. Cost plan for the rest of the cycle

Spec `os/messaging/README.md` drafted by the lead from this brief (no drafter fleet) → **adversarial panel: 3 fable
judges** (crypto/envelope + L2 amendments · panel-proxy security: tickets, cookies, CSRF, CSP · integration/seams + scope
honesty), run with real netns checks where cheap → fixes → lock → `TASKS.md` → build waves: **opus** for Rust
(`mesh111` crate, store, mailbox, proxy, Meshtastic integration), **sonnet** for UI (Messages view, Panel tab, contacts),
docs and KB entries, **haiku** for mechanical transforms; lead reviews every task against its check before commit.
