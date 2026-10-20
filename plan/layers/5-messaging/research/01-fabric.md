# Layer 5 research, lens 1: the store-and-forward fabric between 111 nodes

Scout run 2026-10-14. Question: what carries 111's own sealed envelopes (L2 section 6.4) between 111 nodes across heterogeneous transports, with store-and-forward, so one fabric serves messages, agent ops, KB sync, updates and patches. Numbers in brackets, e.g. [S9], point to section 6. "(unverified)" marks anything not confirmed from a fetched primary source. "(computed)" marks arithmetic done from fetched constants.

## 1. Summary

- No adopted project carries a 111 sealed envelope across LoRa-sized frames (184 to 255 B), without a wall clock, with Cat-1 portability. The closest standard, IETF BPv7 (RFC 9171), has mandatory timestamp and lifetime fields, repeats a 37 to 57 B primary block on every fragment (computed), and has no maintained LoRa convergence layer. [S9][S14]
- Recommended fabric core: a thin custom Rust store-and-forward core ("111f"), whose wire is the L5 envelope on top of the L2 sealed packet, behind a small transport-adapter trait. This is the only place custom code is justified; everything below it is adopted.
- Adopt for the pipes, not the fabric: iroh for IP links (same raw Ed25519 NodeId model as L2; LAN-only should work with the Minimal preset plus mDNS (unverified in practice); iroh-blobs for KB and update transfers); RNode and MeshCore KISS-modem firmware as raw LoRa modems; Meshtastic via its client API (PRIVATE_APP, 233 B); Reticulum/LXMF as a bridged network only. [S24][S25][S26][S27][S28]
- BPv7 via dtn7-rs (MIT OR Apache-2.0, Rust, ECLA external-CLA seam) is the best "do not write a store" option and stays available as an optional gateway bridge later. Rejected as the core: clock-based lifetime, per-fragment overhead, single-maintainer daemon with global state, sled 0.34 store, loopback HTTP API with no documented authentication. [S1][S2][S3][S5]
- Reticulum is usable only as a bridged network. The only external interfaces that carry arbitrary bytes without "import RNS" are the MeshChat HTTP/WS API (MIT), the lxmd `on_inbound` hook, and rnsd config (PipeInterface runs the other way). Independent Rust and Go reimplementations exist (Leviculum AGPL, Beechat MIT, go-reticulum MIT); upstream publicly disputes the standing of such projects. [S16][S17][S18][S20][S21][S22][S23]
- P2P messenger cores (Berty/wesh, qaul, Veilid, SSB, Delta Chat) each bring their own identity and none speaks LoRa. All rejected as fabric. Willow is a good reference for Layer 8 shared state, not for this layer. [S31][S32][S33][S34][S35][S36]

## 2. Candidates

License column: V = read from the upstream LICENSE or manifest this session, U = not verified.

| Name | Upstream | License | Language / runtime | Maturity (as of 2026-10-14) | Fit | Why |
|---|---|---|---|---|---|---|
| Custom thin core "111f" | n/a (ours) | ours | Rust, musl-static; C port for Cat 1 later | n/a | build | Only option meeting all of: 184 B frames, no clock, 111 identity, Cat-1 port. Honest reimplementation list in section 4. |
| dtn7-rs (+ bp7 crate) | github.com/dtn7/dtn7-rs | MIT OR Apache-2.0 (V: LICENSE-MIT, LICENSE-APACHE, Cargo.toml) | Rust, tokio; daemon `dtnd` + lib; sled store | v0.21.0 2024-03-27; master pushed 2026-09-09; 112 stars; bp7 0.10.7 2025-06-01 | bridge | BPv7, TCPCLv4, MTCP, UDP, HTTP, ECLA, 5 routers. Wrong MTU and clock model for the core; good gateway bridge. |
| uD3TN | gitlab.com/d3tn/ud3tn | AGPL-3.0-or-later since early 2024 (V) | C, POSIX, SQLite storage | v0.15.1 2026-09-18; 16 stars (GitLab) | reference | Space-tested BPv7 + BPv6; MTCP, TCPCLv3, SPP, BIBE; no custody; STM32 support dropped. Separate-process use is licence-safe, but no phone or LoRa story. |
| ION-DTN | github.com/nasa-jpl/ION-DTN | BSD-3-style Caltech licence (V: license.txt) | C, multi-process | 4.2.0 2026-08-30 | reject | Heavyweight multi-process SDR design (Termux fit unverified); no small-MTU story. |
| HDTN | github.com/nasa/HDTN | NASA Open Source Agreement 1.3 (V: LICENSE.md; GitHub metadata claims Apache-2.0, file wins) | C++17, Boost, ZeroMQ | v3.0.0 2026-08-18 | reject | High-rate design, x86 optimizations, Boost; NOSA is, from memory, not FSF-free (unverified). |
| LXMF + lxmd | github.com/markqvist/LXMF | Reticulum License, custom non-OSI (V) | Python | 1.2.0 2026-09-30 | bridge | Carriage only. 112 B overhead, wall-clock timestamp, own addressing. |
| Reticulum MeshChat | github.com/liamcottle/reticulum-meshchat | MIT (V) | Python + JS | v2.4.0 2026-07-06; 1,298 stars | bridge | Only adopted LXMF app with an HTTP/WS API; Termux recipe in repo. |
| NomadNet | github.com/markqvist/NomadNet | GPL-3.0 (U: API metadata) | Python TUI/daemon | 1.4.4 2026-09-30 | reference | `--daemon` mode, micron pages; no machine API found. |
| Leviculum | codeberg.org/Lew_Palm/Leviculum | AGPL-3.0-or-later (V) | Rust, `no_std` core, musl-static `lnsd` | nightly 2026-10-07, commits 2026-10-14; 39 stars | reference | Drop-in `rnsd` replacement claiming wire compatibility; not upstream Reticulum. |
| Reticulum-rs (Beechat) | github.com/BeechatNetworkSystemsLtd/Reticulum-rs | MIT (V: README, fork LICENSE) | Rust | pushed 2026-08-22; 346 stars | reject | Transports TCP, serial, Kaonic; no LXMF mention; provenance of code unverified. |
| go-reticulum | github.com/svanichkin/go-reticulum | MIT (V: API) | Go | v1.1.521 2026-04-28; 45 stars | reject | README says it was built with LLM reading of the reference implementation: licence risk. |
| iroh (+ blobs, gossip, docs) | github.com/n0-computer/iroh | MIT OR Apache-2.0 (V: LICENSE-MIT/APACHE heads, Cargo.toml) | Rust, QUIC (noq), tokio | iroh 1.3.0 2026-09-28; blobs 0.103.1 2026-10-06; 12.7k stars | adopt (IP adapter, bulk blobs) | NodeId = raw Ed25519 key. QUIC needs 1200 B datagrams: no LoRa. |
| Berty / wesh | github.com/berty/weshnet | Apache-2.0 OR MIT (V) | Go, gRPC | weshnet v2.3.0 2025-05-23; Berty v2.471.14 2026-07-03 | reject | Own account/device/group keys; protocol doc says unaudited, no revocation. |
| qaul (libqaul) | github.com/qaul/qaul.net | AGPL-3.0 (V) | Rust on libp2p 0.56; Flutter UI | v2.0.0-rc.8 2026-08-30 | reject | Release candidate; own routing and identity; BLE module optional. |
| Veilid | gitlab.com/veilid/veilid | MPL-2.0 (V) | Rust | v0.5.7 2026-07-19 | reject | IP only, DHT with bootstrap servers; no offline store-and-forward. |
| Scuttlebutt (ssb-server, Manyverse) | github.com/ssbc | MIT / LGPL-3.0-only (V manifests) | JS | ssb-server last push 2022; Manyverse last release 2023-10 | reject | Stagnant; append-only feeds per identity. |
| Willow (willow_rs) / Earthstar | codeberg.org/worm-blossom/willow_rs | Apache-2.0 + MIT (V) | Rust library | updated 2026-10-12; GitHub copy archived | reference (Layer 8) | Data model, Meadowcap, sideloading; library, no daemon; WGPS sync listed as "eventually". |
| Delta Chat core (chatmail) | github.com/chatmail/core | MPL-2.0 (V: Cargo.toml) | Rust, JSON-RPC over stdio | v2.63.0 2026-10-09 | reference (L7 gateway) | Transport is IMAP/SMTP, identity OpenPGP; not a mesh fabric. |
| Zenoh | github.com/eclipse-zenoh/zenoh | EPL-2.0 OR Apache-2.0 (U: README badges only) | Rust | 1.10.1 2026-09-07 | reject | Pub/sub and query; no custody store-and-forward found; not fully investigated. |
| RNode firmware | github.com/markqvist/RNode_Firmware | GPL-3.0 (V) | C++ (ESP32, nRF52) | pushed 2026-09-27 | adopt (as modem) | KISS-style host protocol, `SINGLE_MTU 255`; separate firmware. |
| MeshCore firmware | github.com/meshcore-dev/MeshCore | MIT (V: license.txt) | C++ | pushed 2026-10-14 | adopt (as modem) | Ships a standard KISS TNC mode with raw 255 B frames. |
| Meshtastic firmware | github.com/meshtastic/firmware | GPL-3.0 (U: API metadata) | C++ | pushed 2026-10-14 | bridge | Client API; `PRIVATE_APP` payload max 233 B. |

## 3. Findings that change the plan

1. **BPv7 forces time into every bundle.** Lifetime is mandatory and counted in ms from creation time; without a clock, age must come from the Bundle Age extension block. There is no "never expires" value; the closest is a huge integer. This collides with "no forced expiry" and "no wall clock", so any BPv7 use needs a maximum lifetime plus a clockless age block. [S9: RFC 9171 lifetime text and section 4.4.2]
2. **BPv7 has no custody transfer.** RFC 9171 moved custody to the BIBE draft, and uD3TN states it does not implement custody. "Reliable delivery first" would therefore be our own end-to-end ack over any BPv7 core. [S9][S11]
3. **BPv7 overhead per bundle is 37 to 57 B before payload (computed).** Hand-encoded CBOR, ipn EIDs, with or without CRC16 and a Bundle Age block. RFC 9171 requires a primary-block CRC unless a BIB covers it, and each fragment carries its own primary block. At 184 B (MeshCore) minus the 56 B seal, only 71 to 91 B remain for content (computed). [S9][S25][S38]
4. **No maintained BPv7-over-LoRa convergence layer was found** (two searches; negative result, unverified). The only link-layer-agnostic work is the IETF draft BTPU (4 B headers, segmentation, repetition instead of acks), which is a draft with no implementation found. dtn7-go has a `bbc` broadcast connector naming LoRa as an example, last release 2022. [S14]
5. **dtn7-rs has a clean seam for foreign transports.** ECLA exposes a JSON API over WebSocket or TCP (`Register`, `ForwardData` with base64 bundle bytes, `Beacon`); any language can implement a CLA. It also has TCPCLv4, MTCP, UDP, HTTP, mDNS discovery, and spray-and-wait, flooding, epidemic, static and sink routing. [S1][S3][S4]
6. **dtn7-rs is daemon-shaped, not embeddable.** Docs refer to global `PEERS` and `CONFIG` statics; its HTTP control API (`/send`, `/register`) is "localhost-only" TCP port 3000 with no authentication documented, which on Android would mean any app on the phone can reach it (inference). Release assets are 9 to 10 MB tarballs for armv7-musl, x86_64 (gnu and musl) and macOS; no aarch64 prebuilt. Last tag is 2024-03-27, though master was pushed 2026-09-09. Manifest uses sled 0.34.7, axum 0.5, hyper 0.14. [S2][S5][S6]
7. **uD3TN is AGPL-3.0-or-later since 2024** (v0.13.0 and before were Apache-2.0/BSD) and supports POSIX only. Fine as a separate process; no embedded or LoRa path. [S10]
8. **L2 and the L5 envelope already fix the crypto layout, so most "message format" features of candidates are dead weight.** The sealed packet costs 56 B and the sender NodeId must travel inside the ciphertext (32 B more); at 184 B this leaves 96 B for everything else (computed from 2-identity/README.md sections 6.4 and 6.7). Fragment-then-seal repeats 56 B per frame; seal-then-fragment amortizes it but needs selective retransmit. This is the biggest design lever and is not candidate-specific. [S38]
9. **LoRa frame limits found in firmware source:** Meshtastic `Data.payload` max 233 B; MeshCore `MAX_PACKET_PAYLOAD` 184 B, `MAX_TRANS_UNIT` 255 B; RNode firmware `SINGLE_MTU` 255, `MTU` 508; Reticulum `MTU = 500`. [S24][S25][S26][S15]
10. **Two shipped LoRa firmwares already act as raw modems.** RNode speaks a KISS-style serial protocol with `CMD_DATA 0x00` plus frequency, bandwidth, SF, CR commands. MeshCore documents a standard KISS TNC mode (115200 8N1, 255 B data frames, one pending TX, built-in CSMA). A 111 host can drive either with no Reticulum or Meshtastic code at all. Both need the board to be dedicated to that firmware (D17 #1 limits flashing to blank boards or consent). [S24][S25]
11. **iroh shares L2's identity model.** Public key is a 32-byte Ed25519 key and `SecretKey::from_bytes(&[u8;32])` exists, so a 111 node can be dialed by its own NodeId. `presets::Minimal` sets no relay and no DNS lookup, and the mDNS address-lookup crate connects directly on a LAN. The docs sample uses `presets::N0` (n0 relays and DNS), so the offline posture is our configuration, not a default; offline behaviour is unverified in practice. QUIC requires 1200 B datagrams (RFC 9000 section 14): LoRa out of scope. The custom-transport API is behind `unstable-custom-transports` (no semver guarantee) and the BLE transport id is only reserved. [S27][S28][S29]
12. **iroh's higher layers add their own semantics.** iroh-gossip is epidemic broadcast (HyParView/PlumTree; no persistence documented); iroh-docs signs entries with separate namespace and author keys and carries timestamps; iroh-blobs is BLAKE3 verified streaming, a good fit for KB sync and updates over IP only. [S30]
13. **LXMF carries the same custom licence as Reticulum**, including the harm and AI-training restrictions. The LXMF README warns that independent "fakes" violate the licence and that downstream users are liable. That statement bears on Leviculum, go-reticulum and Beechat. [S15][S16]
14. **External interfaces to Reticulum that need no `import RNS`:** (a) MeshChat HTTP/WS: `POST /api/v1/lxmf-messages/send` (content, optional image, audio or file_attachments fields only; no arbitrary field), `GET /api/v1/lxmf-messages/conversation/{hash}`, `/ws`, interface and transport toggles, propagation-node sync; (b) lxmd config: `[lxmf] on_inbound` hook, `[propagation] enable_node`, `message_storage_limit` (default 500 MB), `control_allowed`, static peers; (c) rnsd config: interface stanzas, `PipeInterface` (runs a program, packets over stdin/stdout, so it lets rnsd ride on our links, the reverse of what we need); (d) CLI tools `rnstatus`, `rnpath`, `rncp`, `rnx`; (e) NomadNet `--daemon` pages. LXMF single-packet content limit is 287 B (computed: 383 - 112 + 16); LXMF overhead is 112 B and carries a UNIX-epoch timestamp. [S16][S17][S18][S19][S20]
15. **Reimplementations exist.** Leviculum (Rust, AGPL, `no_std` core, static musl `lnsd`, LoRa working; commit messages mention stock LXMF clients) and Beechat Reticulum-rs (Rust, MIT, no LXMF mention in README) and go-reticulum (Go, MIT, AI-assisted port). None is upstream; D17 #12 requires the latest upstream. [S21][S22][S23]
16. **Meshtastic can carry an opaque payload.** `PRIVATE_APP = 256` exists for exactly this; payload cap 233 B; `STORE_FORWARD_APP = 65` is marked work in progress. Per owner decision this cycle, the message bridge is not built; only the seam is specified. [S26]
17. **Berty/wesh, qaul, Veilid, SSB fail on identity or transport.** Wesh: own keys, "impossible to revoke a device", no post-compromise secrecy, protocol page last modified 2023-03-29, unaudited. qaul: rc, libp2p, optional BLE, own routing. Veilid: public-IP bootstrap topology. SSB: ssb-server last push 2022, Manyverse last release 2023. [S31][S32][S33][S34]

## 4. Recommendation

### 4.1 The fork

Options for the fabric core:

- **A. Thin custom Rust core (111f), adapters below.** Trade-off: we own about the surface listed below; fits 184 B frames, no clock, 111 identity, C port for Cat 1.
- **B. BPv7 via dtn7-rs as the core, envelope as bundle payload, ECLA for non-IP links.** Trade-off: no store or routing written; but clock fields, 37 to 57 B per fragment, no LoRa CLA (we would write one anyway), single-maintainer daemon, sled store, Android loopback API.
- **C. A as the native fabric plus B as an optional later gateway bridge** (bridge code = envelope-in-payload translation, `bp7` crate as codec, dtn7-rs or uD3TN as separate process). Trade-off: keeps the "interop with the DTN world" door open at near-zero cost now.
- **D. Reticulum/LXMF or an adopted P2P messenger as the fabric.** Trade-off: least code, but rule 1 (no Reticulum code) and rule 2 (their identity takes over) both fail; offered only so the choice is visible.

**Pick: C (A first).** Rationale: the DTN wheel exists, but at our frame sizes and clock rules it needs a LoRa shim and time workarounds that cost about as much as the missing parts, and a second protocol on air. What A must honestly reimplement (design from RFC 9171 semantics and dtn7-rs routing docs, which are specs, not code):

1. Envelope codec and message-id (idempotency per L2 section 13).
2. Persistent encrypted store with capacity-based eviction (an adopted embedded KV crate; crate choice goes to an L0 entry).
3. Duplicate suppression and anti-entropy on contact (summary vectors; epidemic or spray-and-wait).
4. Fragmentation and reassembly with selective retransmit, built for 184 B frames.
5. End-to-end delivery proof (HMAC `ack_proof`, domain-tagged per L2 section 6.1) and per-hop link acks.
6. Contact tracking from L2 announces and L7 presence (no new discovery).
7. Adapter trait, local API for the dashboard Messages route (L4 stub), and agent tool surface.

Not reimplemented: crypto (L2), IP transport and NAT traversal (iroh or TCP+Noise IK per L2 section 6.6), bulk blob transfer (iroh-blobs), LoRa radio (firmware), mesh flooding where Meshtastic is the mesh.

### 4.2 Transport-adapter seams for Layer 6

One trait, `Link`: `id`, `kind`, `mtu_bytes` (hard frame cap; fabric fragments to it), `framing` (stream or datagram), `reliable` (bool), `broadcast` (bool), `cost_class` (bulk or low-bandwidth), `peer_addr` (opaque, per adapter), `send(peer, frame)`, `recv() -> (peer, frame)`, `state` (feeds `DeviceState`/`transport_state`). Rules: no rate limits in the fabric; duty-cycle and airtime are advertised by the adapter as physics. Per D17 #12 the core has no Reticulum dependency.

| Adapter | What it drives | MTU | Status this cycle | Notes |
|---|---|---|---|---|
| `tcp-lan` | TCP (+ Noise IK, L2 section 6.6) between 111 nodes; phone to hub per D17 #4 | stream | build with L5 | Independent of rnsd posture; unicast only, so no MulticastLock needed (cross-layer/devices.md section 2.2). |
| `iroh` | QUIC dial-by-NodeId, mDNS on LAN, optional self-hosted relay for gateways, iroh-blobs for KB/updates | 1200 B+ datagram | optional, L7/L8 | Reuse the node's Ed25519 seed or a derived transport key (open question 3). |
| `kiss-modem` | RNode firmware or MeshCore KISS firmware as a raw LoRa modem | 255 B | seam only; build with L6 | 111 owns the board; one owner per port (cross-layer/devices.md section 3). |
| `meshtastic-client` | Meshtastic serial/TCP 4403/BLE protobuf, port `PRIVATE_APP` | 233 B | seam only (owner: no bridge yet) | Meshtastic does the multi-hop flooding; Cat-1 relays already forward. |
| `rns-bridge` | rnsd reached through MeshChat HTTP/WS or lxmd `on_inbound`; envelope as attachment | 287 B single packet, larger via link | deferred bridge | LXMF addresses are transport addresses only; our NodeId stays inside the envelope. |
| `ble` | Android helper app (Columba is MPL-2.0 for Reticulum BLE), later | ~ATT MTU (unverified) | later | iroh BLE id is reserved, not built. |

Illustrative LoRa budget (computed; custom-envelope figure is a design target, not a measurement):

| Frame cap | After 56 B seal | After BPv7 37 to 57 B | After 32 B inner NodeId too | Custom envelope ~16 to 24 B |
|---|---|---|---|---|
| 184 (MeshCore) | 128 | 71 to 91 | 39 to 59 | 104 to 112 (before inner NodeId) |
| 233 (Meshtastic) | 177 | 120 to 140 | 88 to 108 | 153 to 161 |
| 255 (KISS, RNode single frame) | 199 | 142 to 162 | 110 to 130 | 175 to 183 |

### 4.3 Per-candidate MTU and LoRa answer

- Custom core: designed for it. BPv7: no MTU concept in the bundle; fragmentation exists (RFC 9171 section 5.8) but each fragment repeats the primary block; no LoRa CLA. LXMF: native on RNode LoRa (500 B packets over 255 B frames, split by firmware, unverified detail), 112 B overhead. iroh: LoRa out of scope. Berty/wesh: BLE and Wi-Fi Direct only on mobile. qaul: no LoRa. Veilid: IP only. SSB: TCP. Willow: transport-agnostic but a library. Delta Chat: email, out.

## 5. Open questions for the owner

1. **Fork:** A-then-C (pick) or B? What would flip it to B: preferring "zero custom store-and-forward" over LoRa efficiency and the clockless rule, accepting a LoRa shim as the custom part.
2. **Reticulum carriage:** restrict to adopted-app APIs (MeshChat, lxmd), or permit a clean-room Rust link-layer client that speaks Reticulum's wire protocol to rnsd? The second is allowed by LICENSING.md only as clean-room design but adds protocol dependence. Also: may Leviculum (AGPL, not upstream) serve as an alternative `rnsd` on Termux, given D17 #12 says latest upstream? Legal review advised because upstream disputes reimplementations.
3. **iroh key use:** reuse the L2 Ed25519 seed as iroh's SecretKey (one NodeId, needs L2 sign-off on key reuse across TLS and announces) or derive a separate transport key bound by announce `appdata`?
4. **Sealing granularity:** L2 section 6.4 says seal "each protected packet". Confirm L5 may seal once per message and fragment the ciphertext (saves 56 B per frame; needs selective retransmit), and whether the 32 B inner NodeId may be shortened to a peer handle after first contact (L2 amendment).
5. **Raw-modem LoRa:** accept that KISS-modem mode applies only to blank or explicitly consented boards (D17 #1), while boards already running Meshtastic go through the client adapter?
6. **Expiry:** confirm the envelope carries no lifetime and a BPv7 bridge sets maximum lifetime and a clockless age block.

## 6. Sources

[S1] https://github.com/dtn7/dtn7-rs (README) · [S2] https://raw.githubusercontent.com/dtn7/dtn7-rs/master/core/dtn7/Cargo.toml · [S3] https://raw.githubusercontent.com/dtn7/dtn7-rs/master/doc/ecla.md · [S4] https://raw.githubusercontent.com/dtn7/dtn7-rs/master/doc/mdns-discovery.md · [S5] https://raw.githubusercontent.com/dtn7/dtn7-rs/master/doc/http-client-api.md · [S6] https://api.github.com/repos/dtn7/dtn7-rs/releases/latest and https://api.github.com/repos/dtn7/dtn7-rs · LICENSE-MIT / LICENSE-APACHE at raw.githubusercontent.com/dtn7/dtn7-rs/master/ · bp7: https://raw.githubusercontent.com/dtn7/bp7-rs/master/Cargo.toml and README.md
[S9] https://www.rfc-editor.org/rfc/rfc9171.txt · [S10] https://gitlab.com/d3tn/ud3tn/-/raw/master/README.md , https://gitlab.com/d3tn/ud3tn/-/raw/master/LICENSE , https://gitlab.com/api/v4/projects/d3tn%2Fud3tn/repository/tags · [S11] https://gitlab.com/d3tn/ud3tn/-/raw/master/doc/architecture-overview.md
ION: https://github.com/nasa-jpl/ION-DTN (license.txt, releases) · HDTN: https://github.com/nasa/HDTN (LICENSE.md, README.md) · [S14] https://datatracker.ietf.org/doc/html/draft-ietf-dtn-btpu-03 · dtn7-go: https://beta.pkg.go.dev/github.com/dtn7/dtn7-go
[S15] https://github.com/markqvist/Reticulum (LICENSE; RNS/Reticulum.py, Packet.py, Identity.py) · [S16] https://github.com/markqvist/LXMF (README.md, LICENSE, LXMF/LXMessage.py) · [S17] https://github.com/markqvist/LXMF/blob/master/LXMF/Utilities/lxmd.py · [S18] https://github.com/liamcottle/reticulum-meshchat (meshchat.py, LICENSE) · [S19] https://github.com/markqvist/NomadNet · [S20] https://raw.githubusercontent.com/markqvist/Reticulum/master/docs/source/interfaces.rst
[S21] https://codeberg.org/Lew_Palm/Leviculum (README, LICENSE) · [S22] https://github.com/BeechatNetworkSystemsLtd/Reticulum-rs · [S23] https://github.com/svanichkin/go-reticulum
[S24] https://github.com/markqvist/RNode_Firmware (Framing.h, Config.h, LICENSE) · [S25] https://github.com/meshcore-dev/MeshCore (src/MeshCore.h, docs/kiss_modem_protocol.md, license.txt) · [S26] https://github.com/meshtastic/protobufs (meshtastic/mesh.options, portnums.proto)
[S27] https://github.com/n0-computer/iroh (README, iroh/Cargo.toml, TRANSPORTS.md, iroh/src/endpoint/presets.rs, iroh-base/src/key.rs) · [S28] https://docs.iroh.computer/connecting/local-address-lookup.md · [S29] https://www.rfc-editor.org/rfc/rfc9000.txt (section 14) · [S30] https://github.com/n0-computer/iroh-gossip , https://github.com/n0-computer/iroh-docs , https://github.com/n0-computer/iroh-blobs
[S31] https://berty.tech/docs/protocol , https://github.com/berty/weshnet · [S32] https://github.com/qaul/qaul.net (rust/libqaul/Cargo.toml, LICENSE) , https://qaul.net/ · [S33] https://gitlab.com/veilid/veilid · [S34] https://github.com/ssbc/ssb-server , https://github.com/staltz/manyverse · [S35] https://codeberg.org/worm-blossom/willow_rs , https://github.com/earthstar-project/willow-rs · [S36] https://github.com/chatmail/core · [S37] https://github.com/eclipse-zenoh/zenoh
[S38] Local: /home/kia/projects/111network/os/2-identity/README.md sections 6.1 to 6.7 and 13; /home/kia/projects/111network/os/LICENSING.md; /home/kia/projects/111network/os/plan/cross-layer/devices.md sections 0, 2, 7. BPv7 size arithmetic: scratchpad script tools/cbor_size.py (hand-encoded CBOR per RFC 9171 section 4).
