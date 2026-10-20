# Layer 5 sweep: completeness critique

Critic run 2026-10-14. Checked 00 to 05 against README §5, identity §6/§10/§13, cross-layer/devices.md, D17/D18, the L3 offload seam (3-intelligence/README.md:48), the L4 Messages stub and LICENSING.md, plus four web checks. Full scout prompts are in the structured return.

## Verdict

The sweep is **not yet sufficient** for an options brief.

Panels, store and envelope crypto are strong. Two blocking gaps sit on "Layer 5 with Layer 6 in mind":
- who owns the LoRa radio, and what a Cat-1 relay runs;
- how v1 nodes learn keys and forward custody when L6/L7 do not exist.

Five cross-report conflicts also change the numbers the owner would choose on. Close G1 and G2, run G3 to G5 in parallel, and then the brief can be written.

## Settled by the critic (cite in the brief)

1. **MeshCore `RAW_CUSTOM` forwarding.** Repeaters forward it only along a known direct path, through the generic direct-forward branch. It is never flood-routed ("don't flood route these (yet)"). So flood carriage on MeshCore means group data (163 B), and 184 B applies only to learned direct paths. Source: https://github.com/meshcore-dev/MeshCore/blob/main/src/Mesh.cpp lines 89-106 and 292-297 (main @ a366955).
2. **Meshtastic `PRIVATE_APP` is not universally relayed.** `rebroadcast_mode = CORE_PORTNUMS_ONLY` forwards only NodeInfo, Text, Position, Telemetry and Routing. `LOCAL_ONLY` and `KNOWN_ONLY` drop packets on channels the router cannot decrypt. Opaque 111 payloads therefore ride existing Meshtastic infrastructure only on shared channels, through routers that are not core-only. Source: https://raw.githubusercontent.com/meshtastic/protobufs/master/meshtastic/config.proto lines 189-220.
3. **iroh-blobs maturity.** Its README says "not yet considered production quality … use iroh-blobs 0.35". This confirms lens 5 over lens 1. Source: https://raw.githubusercontent.com/n0-computer/iroh-blobs/main/README.md
4. **Chrome 154 HTTPS-by-default.** Google's own post says private sites (local IPs, internal networks) are initially excluded, so D15's no-TLS stance stands. Whether `.local` counts as private is still unverified. Source: https://blog.google/security/https-by-defau/
5. **rnsd can attach to a radio we own.** It has `PipeInterface` and a TCP KISS interface, so "111 owns the radio, rnsd rides through it" is a real option. Source: https://raw.githubusercontent.com/markqvist/Reticulum/master/docs/source/interfaces.rst ("Pipe Interface", "TCP KISS Interface").

## Gaps

### G1 (blocking): radio ownership and the Cat-1 relay

- D17 #1 plus cross-layer/devices.md §4.1 C (recommended) give a blank board **RNode firmware as an rnsd interface**.
- Lens 1 has 111f drive RNode or MeshCore as a raw **KISS modem**.
- One serial port has one owner, so both cannot hold.

No scout weighed the options:
- (a) 111f owns the modem and rnsd attaches through Pipe/TCP-KISS, so on-air RNode framing stays unchanged;
- (b) rnsd owns the modem and 111 traffic reaches it only through a no-import carrier (lxmd or MeshChat, 287 B);
- (c) one board per stack.

The Cat-1 relay is also unexamined. Lens 1 ("C port for Cat 1 later") and lens 4 §10 assume a **111-native relay firmware**, which reinvents a LoRa repeater. The adopted alternative is relay-blind carriage through Meshtastic/MeshCore repeaters (findings 1 and 2 above bound it). The fabric fork cannot be chosen until this is settled.

### G2 (blocking): what v1 actually delivers (key distribution, contacts, custody routing)

- L2 §13 says v1 emits no announces, and hands announce-as-routing, relay verification and announce rate policy to L6/7.
- Lens 1 relies on "contact tracking from L2 announces and L7 presence". Neither exists.
- No scout says how two nodes exchange keys in v1. Prior art exists: QR or `node1…` exchange as in Briar or Delta Chat SecureJoin, or announces over LAN/TCP.
- No scout says which node takes custody for whom: hub-as-mailbox, epidemic, spray-and-wait or PRoPHET. dtn7-rs routing modules are MIT/Apache and unevaluated as code.
- No scout says what backs the L4 "relayed" state. `messages/index.tsx` promises "a node in between accepted it", which needs a custody receipt. Lens 4 deferred hop acks to L6.

The owner must choose the v1 scope: (i) direct links plus hub mailbox, or (ii) pull a slice of L7 announces and routing forward.

### G3 (major): Meshtastic presence and panel at once, plus a missed candidate

- D18 #3 asks for read-only presence **and** a hosted panel.
- 00-code-seams §8 records Meshtastic's one-API-client limit. Neither lens resolved the contention.
- Lens 2 proposes custom code: a GPL Python sidecar plus a "hub shim" that re-exposes the HTTP API. Lens 3 falls back to loopback-only Web Serial.

Missed candidate: **MeshMonitor**, https://github.com/Yeraze/meshmonitor.
- BSD-3-Clause, LICENSE file read; v4.17.0 on 2026-10-13; 689 stars.
- Covers Meshtastic (TCP/Serial/BLE) and MeshCore (USB/TCP).
- Has REST API v1 with bearer tokens, a **Virtual Node Server** that multiplexes clients, and reverse-proxy auth.

It could be the presence source, the multiplexer and a second panel at once. Not yet checked: offline map tiles, disabling self-upgrade (a runtime download), Node 22 on aarch64/Termux, framing headers, and the serial "via bridge" path. MeshSense (GPL-3.0, last push 2025-10) is a weaker alternative.

### G4 (major): the interactive and online modality is unowned

The L3 seam says offloaded model calls ride "Layer 5's encrypted fabric". That means streamed HTTP, which needs a live encrypted tunnel, not store-and-forward. No lens designed it or chose a library for L2 §6.6 Noise IK or TLS raw public keys (snow, clatter, rustls RPK, iroh).

Lens 1 Q3 and lens 5 Q7 also miss **L2 §10's hard rule**: a transport key MUST NOT share seed material with the node key (F2 rejected), so the "derived transport key" option is already ruled out. Reusing the identity key for TLS/QUIC handshakes runs into §6.1 ("never routine traffic"). The compliant default is a random link key bound by a §10 attestation, or an explicit L2 amendment.

### G5 (major): fragmentation and sealing must become one design

See conflicts 1 to 3. The brief needs one tiered proposal, sized against the **built** `id111::seal()`, before the owner can choose.

### Minor

- **G6.** Hardy (Rust BPv7, Apache-2.0, cited only in lens 5) was not weighed as fork B's implementation. The protocol objections (lifetime, overhead) still stand.
- **G7.** No consented identify-probe order was researched: RNode detect, Meshtastic `want_config`, MeshCore app-start, and the DTR/RTS reset risk. The adopted CLIs (`rnodeconf`, `meshtastic --info`, `meshcore-cli`) are the likely path.
- **G8.** Modality not run: no adopted program was started offline. That covers Meshtastic `build.tar`, MeshChatX and iroh's Minimal preset in a no-internet netns. There was no hardware test and no Termux run of the SQLCipher build. These are build gates, not brief blockers.
- **G9.** Lens 4 finding 1 (Noise "N" means an unauthenticated sender, and forged counters can poison the window) is a correction to L2. It needs the fable/opus adversarial review, not more research.

## Conflicts to resolve before the owner chooses

1. **Seal and fragment order.**
   - Lens 2 picks seal-once-then-fragment (152 B per fragment).
   - Lens 4 picks fragment-then-seal and rejects seal-once because slices cannot be verified.
   - Lens 5 picks unsealed bao-verified chunks for bulk, while lens 4 Tier 2 wants a new chunk `crypto_mode`.
   - Lens 2 says per-fragment sealing "breaks the 32-entry window beyond 32 fragments". Lens 4 says re-sealing with 16 outstanding avoids that.
2. **Frame floor.**
   - Lens 2: F = 160 B.
   - Lens 1: 184 B.
   - Lens 4: a 233 B cap with per-path profiles.
   - Lens 5 assumes a ~50 B inner header, against lens 4's ~7 B.
   - Settled fact 1 above narrows this: 160 B for flood, 184 B on direct paths only.
3. **Sender field.**
   - Lens 4's budget uses a 4 B `sender_ref` and omits it from its L2 sign-off list.
   - The built `seal.rs` pins a 32 B sender NodeId inside the ciphertext (lines 41 and 96).
   - Lens 1 flags shrinking it as an L2 amendment.
   - Lens 4's 147/163 B payload figures are therefore about 28 B optimistic.
4. **iroh / iroh-blobs.** Lens 1 rates them "adopt (IP adapter, bulk blobs)". Lens 5 rates them "bridge, LAN-only, +13.3 MB, pre-production" (verified).
5. **LXMF app.** Lens 2 adopts MeshChat (MIT) as the API. Lens 3 picks the MeshChatX fork (0BSD+MIT). Whether MeshChatX keeps MeshChat's `/api/v1` send/receive API is unverified.
6. **`share_instance`.** Lens 3 Q4 asks the owner to flip it. D17 #5 already chose `yes` as the default, as lens 2 notes. The question is stale.
7. **Phone-to-hub TCP.** D17 #4 / cross-layer/devices.md §4.2 B means an rnsd TCP server with transport on. Lens 1's `tcp-lan` is 111's own TCP+Noise, "independent of rnsd posture". The brief must say whether there are two links or one.
8. **Delivery states.** L4 shows queued/sent/relayed/delivered. Lens 5 stores queued/sent/acked/delivered. Nothing emits "relayed" (G2).
9. **Meshtastic panel path.** Lens 2 proposes the B3 hub shim (custom). Lens 3 proposes loopback Web Serial. MeshMonitor's virtual node is unevaluated (G3).

## Adopt-don't-reinvent check

These are the places where a scout recommended custom code while an unevaluated candidate exists:
- The presence sidecar and hub shim (MeshMonitor).
- The Cat-1 relay firmware (adopted repeaters carrying opaque payloads).
- The fabric's contact and routing logic (dtn7-rs routers, SecureJoin/Briar QR patterns).
- The `tcp-lan` secure channel (no library chosen).

The custom core (111f) and the envelope codec are well argued.
