# Layer 5 research, lens 2: foreign meshes as bridge targets + physical budgets

Scout run 2026-10-14. Tags: **(verified)** = fetched from upstream this run; **(derived)** = my arithmetic from verified constants; **(unverified)** = not confirmed.

## 1. Summary

- Every foreign network can be bridged through an adopted program's **external API**, with no RNS import: Meshtastic (0x94 0xC3 protobuf stream over serial/TCP/BLE), MeshCore (companion protocol, incl. `send_raw_packet`), LXMF (MeshChat REST + WebSocket, lxmd `on_inbound` hook), Delta Chat (`deltachat-rpc-server` JSON-RPC), Briar (briar-headless REST, but **Tor-only on Linux**, so gateway-class, not offline).
- **Fragment floor for Layer 5: 160 B of bearer payload on LoRa-class links** (derived). The tightest real ceiling is MeshCore group data (163 B companion / 165 B firmware); Meshtastic gives 233 B (221 with PKI/AEAD), RNode 254 B per frame. After L2's 56 B seal that leaves 104 B app bytes per fragment if sealed per fragment, 152 B if you seal once and fragment the ciphertext.
- Airtime law dominates: EU 869.4-869.65 MHz allows 10%, most other EU sub-bands 1%. At Meshtastic LongFast one 160 B fragment costs about 1.5 s, so an 8-fragment note needs about 21 min of 1% budget (derived). Layer 5 needs fragment-level retry, tiny ACKs, and capacity classes, not policy caps.
- Licenses: **MeshCore is MIT** (firmware, meshcore_py, meshcore-cli all verified; conflict closed). **Everything Meshtastic is GPL-3.0**, including the web SDK and transports, so run it as separate processes or a separate static bundle and never import `@meshtastic/sdk` into our dashboard bundle. LXMF/lxmd/Reticulum are the custom Reticulum License; MeshChat is MIT.
- Hosting Meshtastic's panel: a plain-HTTP dashboard (`http://111.local`) is **not a secure context**, so Web Serial/Web Bluetooth are unavailable there and in iframes under it. meshtasticd serves HTTPS only (9443, self-signed); ESP32 nodes serve HTTP. The panel needs a hub-side shim or local HTTPS.
- Meshtastic 2.8 (2.8.1 alpha, 2026-10-01) adds signed broadcasts, AEAD channels, ack proofs and key-derived node numbers. The bridge must label 2.7 and 2.8 peers differently.

## 2. Candidates

| Name | Upstream | License | Lang | Maturity | Fit | Why |
|---|---|---|---|---|---|---|
| Meshtastic firmware + protobufs | github.com/meshtastic/firmware, /protobufs | GPL-3.0 (verified) | C++ | 2.7.26 beta (2026-06); 2.8.1 alpha 2026-10-01; 2.8.0 revoked; 8.4k stars | bridge | Foreign mesh; client API is the bridge surface. Generated code from GPL protobufs is GPL-derived; keep it in a separate process |
| meshtastic-python | github.com/meshtastic/python | GPL-3.0 (verified) | Python | 2.7.11 (2026-07-17); README says it lacks a consistent maintainer | bridge | Fastest read-only presence sidecar; supervise as a child with its own GPL source dir |
| meshtasticd | in firmware repo | GPL-3.0 (verified) | C++ | same as firmware | adopt | Only for SPI/CH341 radios; TCP 4403; HTTPS web 9443. Does not drive USB boards (matches cross-layer/devices.md) |
| Meshtastic Web | github.com/meshtastic/web | GPL-3.0-only (verified, all packages) | TS/React | v2.7.2 (2026-08-09) | adopt | Hosted panel as separate static bundle; do not import SDK |
| MeshCore firmware | github.com/meshcore-dev/MeshCore | MIT (verified, license.txt) | C++ | companion-v1.17.1 (2026-08-14); pushed 2026-10-14 | bridge | Foreign mesh; no routing for companions |
| meshcore_py / meshcore-cli | github.com/meshcore-dev/meshcore_py, meshcore-cli | MIT (verified) | Python | v2.3.15, v1.6.5 (2026-10) | bridge | Serial/BLE/TCP companion client; `send_raw_packet`. MIT lets us reimplement framing in Rust |
| MeshCore web/Android/iOS apps | app.meshcore.nz (Liam Cottle) | (unverified) | n/a | n/a | reference | Source and license not confirmed; link-out only until known |
| reticulum-meshchat | github.com/liamcottle/reticulum-meshchat | MIT (verified) | Python+JS | v2.4.0 (2026-07-06); pushed 2026-10-06 | adopt | LXMF app with REST+WS API; Docker/RPi/Termux per README. Imports RNS itself, which is fine as a separate adopted program |
| lxmd (LXMF) | github.com/markqvist/LXMF | Reticulum License (verified, non-OSI) | Python | 1.2.0 (2026-09-30) | bridge | Connected service per D17 #12; inbound hook and propagation node only |
| Sideband | github.com/markqvist/Sideband | CC BY-NC-SA 4.0 (verified) | Python | 2.2.0 (2026-10-01) | reject | Fails gate. Still maintained (cross-layer/devices.md finding 7 is stale) |
| Columba | github.com/torlando-tech/columba | MPL-2.0 (verified) | Kotlin | v2.2.6 (2026-09-25) | reference | Phone LXMF client; no external API seen in README |
| RNode firmware | github.com/markqvist/RNode_Firmware | GPL-3.0 (verified) | C++ | 1.86 (2026-04-24) | adopt | L6 modem; open KISS protocol |
| briar-headless | code.briarproject.org/briar/briar | GPL-3.0-or-later (verified) | Kotlin/JVM 8+ | release-1.5.22 (2026-10-09) | bridge | REST+WS; Tor only on Linux, so L7 gateway only |
| chatmail relay | github.com/chatmail/relay | MIT (verified) | Python deploy + MTA | releases 2026-09 | adopt | Optional hub service; Debian 12 |
| chatmail core / deltachat-rpc-server | github.com/chatmail/core | MPL-2.0 (verified) | Rust | v2.63.0 (2026-10-09) | bridge | JSON-RPC over stdio child |
| nostr relays (strfry, nostr-rs-relay) | github.com/hoytech/strfry, scsibug/nostr-rs-relay | GPL-3.0 / MIT (verified headers) | C++ / Rust | strfry release 2026-09-04 | reference | Layer 7 only |

## 3. Findings that change the plan

**Meshtastic**
1. Client API is one framing for serial and TCP:4403: bytes 0x94 0xC3 + 2-byte length (max 512), then ToRadio/FromRadio protobuf; send `want_config_id` to receive the node DB. BLE uses MTU 512. https://meshtastic.org/docs/development/device/client-api/
2. Python: `SerialInterface`/`TCPInterface`/`BLEInterface`, `sendText(text, destinationId, wantAck, channelIndex)`, `sendData(..., portNum=PRIVATE_APP, pkiEncrypted, publicKey, hopLimit)`, `nodesByNum`, pubsub receive topics. https://python.meshtastic.org/mesh_interface.html
3. Size: `Data.payload` max **233 B** (excludes the 16 B radio header); LoRa max 255 B; PKC and AEAD each add 12 B, so about **221 B** for PKI DMs or AEAD channels (derived). https://raw.githubusercontent.com/meshtastic/protobufs/master/meshtastic/mesh.options , https://raw.githubusercontent.com/meshtastic/firmware/develop/src/mesh/RadioInterface.h
4. Crypto: channel PSK = AES-CTR (128/256 bit; default key `AQ==` is public; no integrity). DMs = X25519 + AES-256-CCM, keys learned from NodeInfo, no forward secrecy. **2.8**: XEdDSA-signed broadcasts (+66 B, sent unsigned if no room), optional AEAD channels (12 B tag), DM ack proofs. Node number = CRC-32 of the public key (verified in Router.cpp). https://meshtastic.org/docs/overview/encryption/
5. The node decrypts before handing packets to the client, so a bridge host sees Meshtastic plaintext. The firmware normalizes a client-forged `from` back to the local node, so **the bridge always speaks as its own node**; relayed people can only be shown as a text prefix. https://meshtastic.org/docs/overview/encryption/ , https://github.com/meshtastic/firmware/blob/develop/src/mesh/PhoneAPI.cpp
6. Read-only presence: want_config dumps NodeInfo with `user` (id, names, publicKey), `snr`, `hopsAway`, `lastHeard`, `deviceMetrics` (channelUtilization, airUtilTx), `position`. `lastHeard` is node-clock; treat as a hint. https://python.meshtastic.org/mesh_interface.html
7. Airtime: region table EU_868 10% duty / 27 dBm, EU_866 2.5-10%, US 100%; `override_duty_cycle` exists; default hop_limit 3, max 7. Preset rates: LongFast 1.07 kbps, MediumFast 3.52, ShortFast 10.94, LongModerate 0.34. https://meshtastic.org/docs/configuration/radio/lora/ , https://meshtastic.org/docs/overview/radio-settings/ , https://github.com/meshtastic/protobufs/blob/master/meshtastic/config.proto
8. MQTT: gateway nodes uplink raw `ServiceEnvelope` protobufs to `msh/REGION/2/e/CHANNEL/USERID`, plus a JSON topic (not on nRF52). Public broker applies zero-hop and a portnum filter. `ok_to_mqtt` / `ignore_mqtt` flags exist. A hub-side broker is an alternative bridge path but needs node Wi-Fi. https://meshtastic.org/docs/software/integrations/mqtt/
9. meshtasticd: SPI/CH341 radios only, TCP 4403, Bluetooth unsupported; its web server is **HTTPS-only** (`ulfius_start_secure_framework`, default 9443, CORS `*`). https://meshtastic.org/docs/software/linux/usage/ , https://github.com/meshtastic/firmware/blob/develop/src/mesh/raspihttp/PiWebServer.cpp
10. Web client: hosted at client.meshtastic.org (HTTPS only; HTTP mode is ESP32-only and needs a trusted self-signed cert). Serial and Bluetooth modes need a secure context; `http://localhost` is secure, LAN HTTP is not, and iframes only inherit security from a secure parent. Self-hosting is supported. https://meshtastic.org/docs/software/web-client/ , https://developer.mozilla.org/en-US/docs/Web/Security/Secure_Contexts , https://developer.mozilla.org/en-US/docs/Web/API/Web_Serial_API
11. The web monorepo's `sdk`, `transport-http`, `transport-node`, `protobufs`, `apps/web` all declare GPL-3.0-only. https://github.com/meshtastic/web/blob/main/packages/sdk/package.json

**MeshCore**

12. License conflict closed: firmware `license.txt` is MIT (Scott Powell / rippleradios.com), meshcore_py and meshcore-cli are MIT. https://github.com/meshcore-dev/MeshCore/blob/main/license.txt , https://github.com/meshcore-dev/meshcore_py/blob/main/LICENSE
13. Companion link is serial, BLE or TCP (one per build): `MeshCore.create_serial/ble/tcp`, `get_contacts`, `send_msg`, channels with 16-byte secrets, events `CONTACT_MSG_RECV`, `CHANNEL_MSG_RECV`, `ADVERTISEMENT`, `ACK`, plus **`send_raw_packet` and `RAW_DATA`**. BLE default MTU 23, request 512. https://github.com/meshcore-dev/meshcore_py , https://github.com/meshcore-dev/MeshCore/blob/main/docs/companion_protocol.md
14. Caps: packet payload 184 B, path 64 B, 64 hops, text 133 chars per spec, channel data 163 B (companion) / 165 B (firmware). Crypto is **AES-128-ECB with a 2-byte MAC**, 1-byte node/channel hashes; adverts carry Ed25519 signature and a unix timestamp; companions never repeat. https://github.com/meshcore-dev/MeshCore/blob/main/src/MeshCore.h , https://github.com/meshcore-dev/MeshCore/blob/main/src/Utils.cpp , https://github.com/meshcore-dev/MeshCore/blob/main/docs/payloads.md
15. Whether repeaters forward `PAYLOAD_TYPE_RAW_CUSTOM` (0x0F) is **(unverified)** and needs a hardware test before we rely on it.

**LXMF / Reticulum**

16. Reticulum MTU 500, MDU 464, single-packet encrypted MDU 383; LXMF overhead 112 B, so opportunistic content is at most **287 B** and link-packet content **319 B** (derived from constants; README says 111 B). Link MTU discovery is on by default, so TCP (HW_MTU 262144), Backbone (1 MiB) and Auto (1196) links grow; RNode stays 508 and splits any packet over 254 B into two LoRa frames. https://github.com/markqvist/Reticulum/blob/master/RNS/Reticulum.py , https://github.com/markqvist/LXMF/blob/master/LXMF/LXMessage.py , https://github.com/markqvist/RNode_Firmware/blob/master/Config.h
17. Announces capped at 2% of interface bandwidth; 128 max hops; RNodeInterface has `airtime_limit_long/short` options. https://reticulum.network/manual/interfaces.html
18. **No-RNS-import paths.** MeshChat (`--headless --host --port --reticulum-config-dir`): `POST /api/v1/lxmf-messages/send`, `GET /api/v1/lxmf/conversations`, `/api/v1/announces`, WebSocket `/ws`. It has **no authentication** and binds 127.0.0.1 by default, so 111d must front it with pairing auth. lxmd offers `on_inbound = <program>` (receives the saved-message path; inbound only) and a propagation node (500 MB store, 256 KB/message, stamp cost 16, peering cost 18 by default; keeps new and small messages first). Shared-instance default is on (ports 37428/37429, AF_UNIX on Linux), so our rnsd with `share_instance = yes` serves them as clients. https://github.com/liamcottle/reticulum-meshchat/blob/master/meshchat.py , https://github.com/markqvist/LXMF/blob/master/LXMF/Utilities/lxmd.py
19. LXMF message-id = SHA-256(dest, source, payload); timestamps are wall-clock doubles; Ed25519 signed; link delivery is documented as forward-secret; opportunistic packets are only documented as per-packet ECDH keys. https://github.com/markqvist/LXMF
20. Reticulum License forbids use in systems able to purposefully harm humans and any use in AI/ML training datasets; the LXMF README warns about LLM-generated "fakes" violating its license. This reinforces the process boundary. https://github.com/markqvist/Reticulum/blob/master/LICENSE

**Briar / Delta Chat / nostr**

21. briar-headless: REST on 127.0.0.1:7000 with bearer token (`/v1/contacts`, `/v1/contacts/add/link`, `/v1/messages/{id}`, `/v1/blogs/posts`) and WebSocket `/v1/ws`; Linux jars for x86_64, aarch64, armhf. But `HeadlessModule.kt` registers **only the Tor plugin** on Linux: no LAN, no Bluetooth. Contacts are added by exchanging `briar://` links. Private message text max 30,720 B (32 KiB minus 2 KiB). The offline transports (Bluetooth, Wi-Fi/LAN, memory cards) exist only in the Android app, which has no API; its Android background behavior is (unverified). https://code.briarproject.org/briar/briar/-/tree/master/briar-headless , https://code.briarproject.org/briar/briar/-/raw/master/briar-headless/src/main/java/org/briarproject/briar/headless/HeadlessModule.kt , https://briarproject.org/how-it-works/
22. chatmail relay: Debian 12, 1 GB RAM, SMTP/IMAPS/HTTPS ports, deployed over SSH by `cmdeploy`; **experimental IP-only mode** (self-signed TLS, no DKIM, `_`-prefixed domain, `dclogin:` login codes) suits a LAN without DNS. https://chatmail.at/doc/relay/getting_started.html
23. `deltachat-rpc-server`: JSON-RPC 2.0 over stdio, `DC_ACCOUNTS_PATH`, OpenRPC spec via `--openrpc`; core is MPL-2.0 Rust. A hub can be a bot contact. https://github.com/chatmail/core/blob/main/deltachat-rpc-server/README.md
24. nostr (Layer 7, one paragraph): NIP-17 private DMs = unsigned kind-14 rumor, sealed as kind 13, gift-wrapped as kind 1059, NIP-44 encrypted, with randomized `created_at` up to 2 days back. Keys are secp256k1, so the gateway needs a separate key bound by L2 section 10 attestation (`transport_id` 0x01 already reserved). Three nested encryptions make events far larger than any LoRa frame, so gateway-only. Relay software: strfry (GPL-3.0), nostr-rs-relay (MIT). https://github.com/nostr-protocol/nips/blob/master/17.md , https://github.com/nostr-protocol/nips/blob/master/59.md
25. EU duty cycle (ETSI EN 300 220-2): 863-865 0.1%; 865-868 1%; 868-868.6 1%; 868.7-869.2 0.1%; 869.4-869.65 10%; 869.7-870 1%. https://www.thethingsnetwork.org/docs/lorawan/duty-cycle/

## 4. Recommendation

### 4.1 Budget table

Airtime from the Semtech formula with each preset's published parameters; my model reproduces Meshtastic's published rates (derived). Frames/h counts 255 B frames.

| Transport | Payload ceiling for our bytes | Throughput | Latency class | Duplex | Airtime law |
|---|---|---|---|---|---|
| Meshtastic LoRa (PRIVATE_APP on a channel) | 233 B; 221 B with PKI/AEAD | LongFast 1.07 kbps (255 B = 2.16 s; 16 frames/h at 1%, 166/h at 10%); ShortFast 10.9 kbps (0.20 s) | seconds to minutes (flood, 3 hops default) | half | EU868 10% (869.4-869.65) or 1%; US none |
| MeshCore LoRa | raw 184 B; group data 163 B; text 133 chars | US narrow SF7/BW62.5 ~2.7 kbps (255 B = 0.82 s; 190 B = 0.62 s) | seconds to minutes (flood; direct once path learned) | half | same as above |
| RNode / Reticulum LoRa | 254 B per frame; 500 B packet = 2 frames; LXMF opportunistic 287 B | SF8/BW125 3.1 kbps (255 B = 0.73 s) | seconds to minutes | half | same, plus `airtime_limit_*`, announces 2% |
| BLE (host or phone to radio) | 512 B GATT value; 20 B if MTU not negotiated | not measured (unverified) | ms | full per characteristic; Meshtastic allows one client | none |
| Wi-Fi / TCP LAN | Reticulum links 1196 B (Auto) up to 1 MiB | Mb/s class | ms | full | none |
| Phone TCP uplink to hub | as TCP (Reticulum TCP 262144 B) | cell/Wi-Fi bound | tens of ms to seconds; reconnect after doze | full | none |
| Internet (Tor, IMAP/SMTP, relays) | 30,720 B Briar text; mail-sized for Delta Chat | variable | seconds | full | none |

**Derived fragmentation size.** Floor = min over LoRa bearers = MeshCore group data 163 B, rounded to **F = 160 B bearer payload**. Every other LoRa bearer fits it (Meshtastic 221-233, RNode 254, MeshCore raw 184). Sealed per fragment: 160 - 56 = 104 B app (35% overhead). Seal once, fragment ciphertext, 8 B fragment header: 152 B per fragment, 56 B paid once. Cost of an 8-fragment note: LongFast 12.3 s airtime (20.6 min of 1% budget), MediumFast 3.7 s, RNode SF8 3.9 s, MeshCore US 4.3 s (derived).

Rules this implies for Layer 5:
- The bearer adapter reports its MTU; the envelope never assumes more than 160 B unless told. Bulk classes (KB sync, updates, patches) default to TCP/LAN/BLE and cross LoRa only on explicit demand, as a physics fact, not a policy.
- Fragment-level selective retry and a cumulative ACK per message. Ack frames are small. Duplicates are normal (flood meshes), so L2's 32-entry window cannot be the only duplicate defense; message-id idempotency is mandatory.
- Never put wall-clock into our fields. Foreign timestamps (LXMF double, MeshCore advert unix time, Meshtastic `lastHeard`) are display metadata only.

### 4.2 Forks and picks

**Fork A: bridge process shape**
- A1 Python sidecars using each project's own library, supervised by 111d like rnsd: least code; Meshtastic sidecar must carry GPL-3.0 and sit in its own source dir.
- A2 Rust adapters in-tree: no Python; fine for MeshCore (MIT, public framing) but a Meshtastic adapter built on prost-generated GPL protobufs makes our binary GPL-derived.
- A3 No bridge this cycle: show panels and presence only.
- **Pick:** A3 plus a read-only A1 Meshtastic presence sidecar this cycle (matches D18 #3); A2 for MeshCore later; LXMF only through MeshChat's API.

**Fork B: Meshtastic panel hosting (reopens D15)**
- B1 link out to client.meshtastic.org: needs internet, breaks offline-first.
- B2 self-host the static `apps/web` build under a separate path, iframe it, offer GPL source.
- B3 B2 plus a small hub shim exposing the node HTTP API (`/api/v1/fromradio|toradio`) for USB-attached boards, so any LAN browser works over plain HTTP.
- B4 serve the dashboard over local HTTPS so Web Serial/Bluetooth work from LAN (L4 decision).
- **Pick:** B2 + B3; B4 is an owner call (open question 1). Never build our own Meshtastic UI.

**Fork C: seal/fragment order (needs L2 sign-off)**
- C1 seal once, fragment ciphertext: 56 B once, one counter per message.
- C2 seal each fragment: independent fragments but 35% overhead and the 32-entry window breaks beyond 32 fragments.
- **Pick:** C1 for LoRa-class links.

**Fork D: how foreign users appear**
- D1 one bridge identity per network; senders shown as a prefix like `[alice]`.
- D2 per-user puppet identities: impossible on Meshtastic (the node rewrites `from`), heavy elsewhere.
- **Pick:** D1, with the bridge's foreign key bound to its NodeId by an L2 section 10 attestation. L2 needs new `transport_id` values (Meshtastic X25519, MeshCore Ed25519, RNS destination hash, Delta Chat fingerprint, Briar author id); RNS's 64 B public key does not fit the 32 B `transport_key`, so bind a hash.

### 4.3 What the bridge must show honestly (where foreign crypto ends)

| Network | Foreign protection | Boundary text for the UI |
|---|---|---|
| Meshtastic default channel | public key `AQ==`, AES-CTR, no integrity | "Anyone can read and forge this." Plain text relayed here has no 111 protection |
| Meshtastic private channel / DM | PSK AES-CTR; DM AES-CCM, no forward secrecy; 2.8 adds signing/AEAD | "Protected by Meshtastic, ends at the bridge node; the bridge host sees plaintext" |
| Meshtastic carrying 111 blobs | Meshtastic is transport only | "111 end-to-end sealed; Meshtastic cannot read it" |
| MeshCore | AES-128-ECB, 2-byte MAC, 1-byte hashes | "Weak transport crypto; integrity only 16 bits" unless a 111 sealed blob rides raw |
| LXMF | AES-128 link (forward secret) or opportunistic; Ed25519 signed | "Reticulum identity, not your 111 identity; ends at MeshChat/lxmd on the hub" |
| Briar (Tor) | Briar E2E; contact is the bridge's Briar account | "The hub is a Briar contact; messages are visible to the hub" |
| Delta Chat | OpenPGP via chatmail; relay is zero-state | "Visible to the hub bot; relay sees metadata" |
| nostr | NIP-44 gift wrap, secp256k1 | "Separate nostr key bound by attestation" |

### 4.4 Legal and ToS limits on automated relaying

- **Meshtastic**: no ToS for RF. Licensed (amateur) mode sends unencrypted and signs instead; amateur rules forbid obscuring messages (47 CFR 97.113(a)(4), cited in cross-layer/devices.md, not re-fetched), so **never send sealed blobs on amateur regions**. Public MQTT: zero-hop and filtered portnums; do not feed the default key to a private broker. Honor `ok_to_mqtt`. Trademark: "Meshtastic" is a registered mark, so show it nominatively. https://meshtastic.org/docs/legal/
- **MeshCore**: no ToS found; regional presets are community-coordinated; automated flood traffic burns shared airtime. FCC/ETSI specifics beyond the duty-cycle table are (unverified).
- **Reticulum/LXMF**: license clauses above; stamp costs on propagation nodes are network-imposed, but on hub-run lxmd they are our config. Public Reticulum transport nodes have no ToS found.
- **Briar, Delta Chat, nostr**: no ToS found for self-run peers; public chatmail relays set their own limits (relay release notes mention new address-creation limits, details unverified). Self-hosted relays avoid that.
- Per-node human fact stays as D17 #10: region and callsign are asked once; the agent suggests and never sets.

## 5. Open questions for the owner

1. Meshtastic panel over the LAN: serve the dashboard over **local HTTPS** (self-signed or local CA, needs a trust step on each phone) or stay plain HTTP and rely on the hub shim (Fork B)?
2. May Meshtastic and MeshCore bridges be **Python sidecars** (GPL-3.0 source for the Meshtastic one) under 111d's child supervision, or must everything be Rust?
3. Is sending 111 sealed blobs on a user's Meshtastic channel acceptable by default, or only on a dedicated private channel we create (EU 1% regions suffer most)?
4. Adopt **MeshChat** as the one LXMF app (MIT, no built-in auth, loopback only, fronted by 111d), or wait for a different LXMF client?
5. Briar: accept "Tor-only gateway contact" for now, or park until the phone-agent idea (P2) can drive the Android app?
6. OK to ask L2 for new `transport_id` registry entries and a hash-based binding for RNS identities?
7. Does MeshCore need to be in this cycle's presence scope, or Meshtastic only as D18 says?
8. OK with F = 160 B as the LoRa floor, accepting a hardware test of RAW_CUSTOM forwarding before widening it to 184 B?

## 6. Sources

Meshtastic: https://meshtastic.org/docs/development/device/client-api/ · https://python.meshtastic.org/mesh_interface.html · https://github.com/meshtastic/python · https://raw.githubusercontent.com/meshtastic/protobufs/master/meshtastic/mesh.options · https://github.com/meshtastic/protobufs/blob/master/meshtastic/config.proto · https://raw.githubusercontent.com/meshtastic/firmware/develop/src/mesh/RadioInterface.h · https://github.com/meshtastic/firmware/blob/develop/src/mesh/PhoneAPI.cpp · https://github.com/meshtastic/firmware/blob/develop/src/mesh/raspihttp/PiWebServer.cpp · https://meshtastic.org/docs/overview/encryption/ · https://meshtastic.org/docs/configuration/radio/lora/ · https://meshtastic.org/docs/overview/radio-settings/ · https://meshtastic.org/docs/software/integrations/mqtt/ · https://meshtastic.org/docs/software/linux/usage/ · https://meshtastic.org/docs/software/web-client/ · https://github.com/meshtastic/web · https://github.com/meshtastic/web/blob/main/packages/sdk/package.json · https://meshtastic.org/docs/legal/ · https://github.com/meshtastic/firmware/blob/develop/LICENSE · https://github.com/meshtastic/python/blob/master/LICENSE.md · https://github.com/meshtastic/web/blob/main/LICENSE · https://github.com/meshtastic/firmware/releases.atom
MeshCore: https://github.com/meshcore-dev/MeshCore/blob/main/license.txt · https://github.com/meshcore-dev/MeshCore/blob/main/README.md · https://github.com/meshcore-dev/MeshCore/blob/main/docs/packet_format.md · https://github.com/meshcore-dev/MeshCore/blob/main/docs/payloads.md · https://github.com/meshcore-dev/MeshCore/blob/main/docs/companion_protocol.md · https://github.com/meshcore-dev/MeshCore/blob/main/docs/faq.md · https://github.com/meshcore-dev/MeshCore/blob/main/src/MeshCore.h · https://github.com/meshcore-dev/MeshCore/blob/main/src/Utils.cpp · https://github.com/meshcore-dev/meshcore_py · https://github.com/meshcore-dev/meshcore-cli · https://github.com/meshcore-dev/meshcore.js
Reticulum / LXMF: https://github.com/markqvist/Reticulum/blob/master/LICENSE · https://github.com/markqvist/Reticulum/blob/master/RNS/Reticulum.py · https://github.com/markqvist/Reticulum/blob/master/RNS/Link.py · https://github.com/markqvist/Reticulum/blob/master/RNS/Packet.py · https://github.com/markqvist/LXMF · https://github.com/markqvist/LXMF/blob/master/LXMF/LXMessage.py · https://github.com/markqvist/LXMF/blob/master/LXMF/Utilities/lxmd.py · https://reticulum.network/manual/interfaces.html · https://github.com/markqvist/RNode_Firmware (Config.h, Framing.h, LICENSE) · https://github.com/liamcottle/reticulum-meshchat (README, meshchat.py, LICENSE) · https://github.com/torlando-tech/columba · https://github.com/markqvist/Sideband · https://github.com/markqvist/NomadNet
Briar: https://code.briarproject.org/briar/briar/-/tree/master/briar-headless · https://code.briarproject.org/briar/briar/-/raw/master/briar-headless/README.md · https://code.briarproject.org/briar/briar/-/raw/master/briar-api/src/main/java/org/briarproject/briar/api/messaging/MessagingConstants.java · https://github.com/briar/briar/blob/master/LICENSE.txt · https://briarproject.org/how-it-works/ · https://briarproject.org/manual/
Delta Chat / chatmail: https://github.com/chatmail/relay · https://chatmail.at/doc/relay/getting_started.html · https://github.com/chatmail/core · https://github.com/chatmail/core/blob/main/deltachat-rpc-server/README.md
Nostr: https://github.com/nostr-protocol/nips/blob/master/17.md · https://github.com/nostr-protocol/nips/blob/master/59.md · https://github.com/hoytech/strfry · https://github.com/scsibug/nostr-rs-relay
Law and platform: https://www.thethingsnetwork.org/docs/lorawan/duty-cycle/ · https://developer.mozilla.org/en-US/docs/Web/Security/Secure_Contexts · https://developer.mozilla.org/en-US/docs/Web/API/Web_Serial_API · https://developer.mozilla.org/en-US/docs/Web/API/Web_Bluetooth_API
Repo context read: os/README.md · os/LICENSING.md · os/identity/README.md (sections 6, 9, 10, 13) · os/plan/cross-layer/devices.md · os/plan/DECISIONS.md (D18) · os/plan/cross-layer/parked.md
