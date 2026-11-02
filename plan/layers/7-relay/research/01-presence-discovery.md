# Layer 7, lens 1: presence, heartbeats, discovery, the Service Discovery Registry, the mesh-epoch source

Research scout report, 2026-11-01. Tree `bdea810`. Fetched facts win over training; "(search snippet)" = seen only in a search result; "(unverified)" = could not be fetched. Code is cited as `file:line` under `/home/user/os`. Airtime numbers are my own Semtech time-on-air arithmetic (script in the session scratchpad, CRC on, 8-symbol preamble, CR 4/5, explicit header); they agree with `0-knowledge/concepts/lora-airtime-budget.md` within a few percent.

## 1. Summary

- **The adopted networks already do the periodic part; Layer 7 adds no heartbeat to them.** Meshtastic NodeInfo 3 h and position 1 h (non-router), MeshCore repeater flood advert 47 h (current `main`; older posts say 3 h or 12 h), Reticulum announces nothing by itself (`lxmd` example config 6 h), and our own LXMF helper already announces every 60 min, `0` = events only (`6-transport/README.md:334`).
- **Pick option A (event-driven) as the base, C (piggyback) for free, B (heartbeat) only as a per-transport opt-in, default off.** On LoRa a 138 B announce every 15 min at SF10 is 0.146 % duty per node and 7.3 % of the channel at 50 nodes (section 3.1). On the LAN, DNS-SD needs no 111 timer. "Last heard" is a local sequence (`contacts.last_seen_seq`), never a peer-claimed time.
- **Service Discovery Registry = three layers, all adopted or already built:** DNS-SD `_111._tcp` TXT (unauthenticated hint, `mdns-sd` already linked), static signed role bits in the L2 announce `appdata` (A8), and a sealed query after connect for anything that changes. Load, model and gateway state never go into a signed blob or a TXT.
- **The required configuration already holds two timers the owner should see:** a 60 s HTTPS HEAD to `install.111.network/204` (`daemon/services/probe.rs:46,549-556`; the only gateway signal today) and a 2 s service tick (`probe.rs:43`), plus `mdns-sd`'s default 5 s IP check while LAN-bound. A netlink hook already exists (`transport/netlink.rs`) that could replace the first and last.
- **Mesh-epoch source: a hub-signed epoch beacon on existing handshakes, floored by the signed release manifest; v1 stays "not evaluated" until a consumer is scheduled.** Reticulum is not clockless: its announce embeds a 5-byte wall-clock stamp (`RNS/Destination.py:283`). A Cat-1 relay (stock MeshCore repeater, D19 c) is asked for nothing.
- **Runtime join-in-sequence should not become a state machine:** every enabled network is a zero-weight-when-off component, so parallel costs nothing extra and dropping one loses a delivery path. Use `[order]` as path preference; sequence only exclusive resources.

## 2. Candidates table

| name · upstream | license (verified) | runtime | maturity and published intervals | resident weight | fit | why |
|---|---|---|---|---|---|---|
| Reticulum announce, path request · markqvist/Reticulum | Reticulum License (yes) | Python | `ANNOUNCE_CAP` 2 %, announce 167 B, path request 51 B, paths expire 1 week, auto path request min 20 s (`RNS/Reticulum.py:114`, `Transport.py:126,138`); LAN AutoInterface multicast every 1.6 s (`AutoInterface.py:61-64`) | rnsd, already a child | bridge | read via `rnstatus -A -j`, `rnpath -t -r -j` (flags verified); never imported (LICENSING §1) |
| LXMF / `lxmd` · markqvist/LXMF | Reticulum License (yes) | Python | propagation announce example 360 min, at start `yes` (`lxmd.py:974-985`); peer announce unset ⇒ none (`:85-88`) | none for us | bridge | our helper already owns the 60 min choice |
| Meshtastic NodeInfo, Position, NodeDB · meshtastic/firmware | GPL-3.0 (yes) | C++ | NodeInfo 3 h, neighbor info 6 h, telemetry and position 1 h (router 12 h) (`Default.h:16-29`); online = heard < 2 h (`NodeDB.cpp:1695`); replies to one requester suppressed 12 h (`NodeInfoModule.cpp:15`) | firmware | bridge | already feeds Nearby (`presence.rs:1`) |
| MeshCore adverts · meshcore-dev/MeshCore | MIT (yes, `license.txt`) | C++ | flood advert 47 h repeater/room, 0 sensor, 3-168 h; zero-hop 60-240 min (`cli_commands.md:649-674`, `MyMesh.cpp:904-905`); advert = key + 4 B stamp + 64 B sig (`Mesh.cpp:257-284`) | firmware | reference | stock repeater is the Cat-1 relay (D19 c) |
| `mdns-sd` · keepsimple1/mdns-sd | Apache-2.0 OR MIT (yes, crates.io 0.21.0; latest 0.21.6, 2026-10-19; 7.07 M downloads) | Rust | one thread that blocks in `poll` until a timer; `IP_CHECK_INTERVAL` default 5 s, 0 disables (`service_daemon.rs`, `main`) | one thread; ours runs only while LAN sharing is on (`dashboard/mdns.rs:40-65`) | adopt | already linked and signed off (`Cargo.toml:50`, `LICENSING.md:120`); a second `ServiceInfo` adds no thread |
| libp2p mDNS, rendezvous · libp2p/specs, rust-libp2p | MIT (yes) | Rust | `_p2p._udp.local` + TXT `dnsaddr=`; browser re-queries every 5 min, TTL 6 min (`mdns/src/lib.rs:77-78`); rendezvous TTL 2 h, max 72 h | n/a | reference | the shape: service type + TXT hint; re-query is the browser's cost |
| Yggdrasil multicast · yggdrasil-go | LGPL-3.0 + linking exception (yes) | Go | beacon `ff02::114:9001`, send interval grows 1 s per beacon to 15 s, then constant (`multicast.go:71,366-373`) | a timer per interface, forever | reference | the always-beaconing cost we avoid |
| cjdns, Briar LAN, BLE | GPL-3.0 (cjdns, yes) · GPL (Briar, text yes) | C, Java | cjdns beacon mode 0/1/2 with a quiet accept-only mode (`configure.md:105-110`); Briar shares LAN `ipPorts` only with contacts (`LanTcpConstants.java`); BLE later (`ble-reticulum.md`) | n/a | reference | opt-in beaconing; address hints inside contact exchange (our enrol QR) |
| Roughtime RFC 10049 | n/a | n/a | Experimental, Oct 2026 (search snippet) | n/a | reference (unverified) | optional gateway-side anchor for an epoch |

## 3. Findings that change the plan

1. **Reticulum announces are application-driven.** `ANNOUNCE_CAP = 2` % of interface bandwidth, queued by hop count, max 128 hops, nothing re-announces by itself (`RNS/Reticulum.py:111-125`, `Transport.py:118`; `understanding.rst:241-275`). Periodic emitters are applications: `lxmd` 360 min (`lxmd.py:978-981`), our helper 60 min (`6-transport/README.md:334`). Reticulum's interface *discovery* announce (default 6 h, min 5 min, `Reticulum.py:992-996`) is unused.
2. **Reticulum has a wall clock inside the announce, though no expiry epoch.** The random hash is 5 random bytes + `int(time.time())` as 5 bytes (`RNS/Destination.py:283`); the transport keeps the highest emission stamp per destination and refuses older announces (`Transport.py:1546-1553`). A design we should not copy for replay protection.
3. **Meshtastic and MeshCore timestamp with an RTC and define "online" by a heard-within window.** Meshtastic `NUM_ONLINE_SECS = 2 h` (`NodeDB.cpp:1695`), non-router intervals stretch with online-node count (`Default.cpp:41-61`); MeshCore adverts carry a 4-byte RTC stamp (`src/Mesh.cpp:418-431`). Our Nearby already calls `last_heard_hint` "a display hint, never an ordering or a freshness proof" (`openapi.yaml:1109`).
4. **MeshCore's current defaults: flood advert 47 h (repeater and room server), zero-hop advert 60-240 min and 0 after first configuration.** The PR text and some guides still say 3 h or 12 h (search snippet `github.com/meshcore-dev/MeshCore/pull/650`); `docs/cli_commands.md:657,669` and `MyMesh.cpp:904-905` on `main` win.
5. **The daemon's only gateway signal is a 60 s timer.** `probe.rs:46` `WAN_INTERVAL`, `:259` `wan_probe` (HEAD, only `204` counts, so a captive portal fails), `:549-556` the loop, URLs `https://install.111.network/204` and `https://static.111.network/204` (`install/preflight.rs:14-15`); the installer records `internet` once (`install/mod.rs:125-130`). It proves reachability to *111's own server*, not to any carrier, and is a recurring contact with a project host, which sits awkwardly with "no telemetry". Built; reported, not resolved.
6. **A timer-free connectivity hook exists.** `transport/netlink.rs:18-54` binds `NETLINK_ROUTE` with `RTMGRP_LINK` into a 2 s debounce (`transport/mod.rs:502`). Adding address and route groups would let a probe fire on change and let `mdns-sd` drop its 5 s IP check: a group-mask change in a Layer 6 file.
7. **The fabric has no LAN discovery today.** `tcp-lan` peers are reached by the enrol bundle (QR with address and code, `5-messaging/README.md:322`) or configured `peers` (`daemon/fabric/mod.rs:33-37`); the only mDNS record is `_http._tcp` "111" for the dashboard (`dashboard/mdns.rs:49`). Stated limits: no multicast on phones (§8.1 "Unicast: no MulticastLock needed on phones", `5-messaging/README.md:361`).
8. **mDNS on Android needs a lock and (from Android 17) a permission; Termux holds neither.** NsdManager apps must take their own `MulticastLock` to receive mDNS before Android 13 T-ext 7 (search snippet, `developer.android.com/reference/android/net/nsd/NsdManager`); Termux issue #536 blames the missing lock and says Avahi has no port (search snippet). Android 17 `ACCESS_LOCAL_NETWORK` covers multicast, TCP and `.local` resolution, enforced for apps targeting SDK 37+; lower targets get an implicit grant (fetched: `developer.android.com/privacy/local-network-permission`). Termux's target SDK is unverified.
9. **Doze bounds any phone heartbeat.** Doze suspends network access and defers exact alarms; `setExactAndAllowWhileIdle()` fires "no more than once per nine minutes, per app" (fetched: `developer.android.com/training/monitoring-device-state/doze-standby`). Termux also faces the Android 12+ phantom-process killer, 32 processes across all apps (fetched: `termux-app` README).
10. **L2 already shaped the presence artifact.** Announce = 137 B base, up to 169 B (`2-identity/README.md:100`); v1 `appdata` = `[ver=1][roles u16][reserved]`, bit 0 mailbox, bit 1 always-on hub, bit 2 LoRa-attached, "Layer 7 appends TLVs" (A8, `README.md:357-359`); announces are emitted on demand only and answered from stored bytes (A6, `:348-351`). `contacts` carries `roles`, `home_hub` and `last_seen_seq` (`schema.rs:73-76`). With the 1 B L6 shim a v1 announce is ~141 B, one 160 B fragment.
11. **L2 expiry already takes a caller-supplied epoch.** `Attestation::verify_full(.., current_epoch)` and `is_expired(current)` exist (`2-identity/crates/id111/src/attest.rs:646-657`), and Layer 6's bridge passes `None` today (`daemon/bridge/attest.rs:151`). So Layer 7's epoch source is one function, `current_epoch() -> Option<MeshEpoch>`, plus a unit.

### 3.1 The airtime arithmetic (own computation; one 138 B announce ≈ the v1 announce plus the L6 shim)

| frame | SF7/BW125 | SF10/BW125 | Meshtastic LongFast (SF11/BW250) | ShortFast (SF7/BW250) |
|---|---|---|---|---|
| 60 B | 113 ms | 698 ms | 616 ms | 56 ms |
| 100 B | 174 ms | 1,026 ms | 944 ms | 87 ms |
| 138 B | 226 ms | 1,313 ms | 1,231 ms | 113 ms |
| 170 B | 277 ms | 1,599 ms | 1,436 ms | 138 ms |

Aggregate channel share of N nodes each sending one 138 B frame every T, **before** repeater re-floods (multiply by 1 + repeaters in range for a flooded advert):

| config | N | T = 15 min | T = 60 min | T = 3 h | T = 12 h |
|---|---|---|---|---|---|
| SF7/BW125 | 10 | 0.25 % | 0.06 % | 0.02 % | 0.01 % |
| SF7/BW125 | 50 | 1.25 % | 0.31 % | 0.10 % | 0.03 % |
| SF7/BW125 | 200 | 5.0 % | 1.25 % | 0.42 % | 0.10 % |
| SF10/BW125 | 10 | 1.46 % | 0.36 % | 0.12 % | 0.03 % |
| SF10/BW125 | 50 | 7.3 % | 1.82 % | 0.61 % | 0.15 % |
| SF10/BW125 | 200 | 29 % | 7.3 % | 2.4 % | 0.61 % |
| LongFast | 50 | 6.8 % | 1.71 % | 0.57 % | 0.14 % |

Per node, one 138 B frame every 15 min is 0.025 % duty at SF7 and 0.146 % at SF10: at SF10 that is 15 % of a 1 % EU sub-band budget (`concepts/lora-airtime-budget.md`) spent on "I am here". At 60 min and SF10 it is 3.6 %. The aggregate column is the real constraint: 50 nodes at 15 min on SF10 spend 7.3 % of the channel on presence alone. LoRa bursts also set the Meshtastic reference: NodeInfo every 3 h on 50 nodes is 0.57 % of LongFast.

LAN contrast: a 100 B UDP beacon every 30 s from 20 nodes is ~67 B/s of bandwidth, but 2,880 wakeups per day per sender and 57,600 per receiver (288 and 5,760 at 5 min). Wakeups, not bytes, are the Pi-floor and phone-battery cost; I have no measured energy figure (unknown).

## 4. Recommendation

### 4.1 Presence: what "reachable" honestly means per transport

| transport | what 111 can truthfully say | evidence 111 already has |
|---|---|---|
| `tcp-lan` direct | "a TCP dial to the attributed address succeeded and a `pair_mac`-verified packet arrived" | attribution needs one fresh verified packet (`mesh111/src/fabric.rs:49-56` per the L6 seams scout); link up/down events |
| via a hub | "the hub holds custody" (state `relayed`), not "the peer is up" | custody receipts (L5 §7.3) |
| LoRa / Meshtastic | "a frame from that node was heard N events ago, SNR x" | NodeDB snapshot (`presence.rs`), 2 h online window precedent |
| Reticulum | "an announce for this destination hash arrived; a path exists (hops, interface)" | `rnstatus`/`rnpath` read by the daemon (`transport/rnstatus.rs`, 30 s tick demand-gated by a 600 s window, `rnstatus.rs:945-947`) |
| gateway | "was online at its last probe", never "is online" | the 60 s WAN probe |

"Last heard" should be a **local monotonic sequence plus an in-memory uptime stamp**, shown as "heard 3 events ago / this session"; the node's own wall-clock hint stays a labelled hint (finding 3). That keeps D20's no-wall-clock rule and matches `contacts.last_seen_seq`.

### 4.2 The three presence options

| | A. Event-driven only | B. Adaptive heartbeat, opt-in per transport | C. Piggyback on existing traffic |
|---|---|---|---|
| mechanism | announce on link-up, card, INTRO (already built); `last_seen_seq` bumps on any verified packet; foreign networks' own presence feeds into Nearby; gateway probe as today or event-driven via netlink | a per-transport component switch with an interval: `0` = events only. Reticulum already has it (60 min). LAN: none needed (DNS-SD). LoRa: default **off**, minimum interval chosen from the table (≥ 3 h on SF10, ≥ 1 h on SF7 only for small N), with backoff on observed channel utilisation | custody ACKs, MAILBOX_FETCH, summary vectors (when A+ lands), and foreign presence double as "alive" evidence; no extra frames |
| Pi floor | zero timers | one timer in a component that is off by default; weighs nothing when off | zero |
| LoRa airtime | none beyond card/INTRO | table 3.1 | none |
| phone | none | bounded below by Doze to ≥ 9 min and by no network in Doze | none |

**Pick: A as the base, C included by construction (it costs nothing), B only as a per-transport switch, default off, and never on a duty-cycle-limited or amateur band.** Reasons: it is the only option that adds no timer to the required configuration; the adopted networks already carry the periodic part; and a heartbeat's only unique value (telling the user a *silent* peer is still there) is what the 2 h-style "online" window and a user-triggered ping (`announce on request`, the L6 helper already does it) give on demand. If the owner wants the README sentence kept literally, the faithful reading is B on the LAN only, which DNS-SD already provides with TTL-driven re-announcement and no 111 timer.

### 4.3 The Service Discovery Registry (what to adopt)

Layered, each field in the cheapest layer that can hold it:

1. **LAN: DNS-SD `_111._tcp.local.` in the existing `mdns-sd` daemon.** Instance name = the 10-char L2 label (`xxxx-xxxx-xx`); SRV port 11183 (the `tcp-lan` listener); TXT `v=1`, `r=<roles u16 hex>`, `k=<NodeId, base32>`. Register as a second `ServiceInfo` beside `_http._tcp` in `dashboard/mdns.rs`: no new thread, no new socket. Browsing is on demand only (start the browse when the user opens Nearby or the agent needs a hub, stop it after a few seconds): the announcer is silent between events, the browser pays the multicast query. (rust-libp2p re-queries every 5 min, TTL 6 min; a one-shot browse is a few small multicast packets, my estimate.) The TXT is **unauthenticated by design**: the real binding is the `tcp-lan` handshake ("hello and announce authenticate nothing", attribution after a `pair_mac`-verified packet, `5-messaging/README.md:375`). A forged TXT can only make a browser dial a wrong address once.
2. **Signed capability: the L2 announce `appdata` roles.** Bits 0-2 exist (A8). Layer 7 may add *static* bits (e.g. "willing to act as gateway", "offload hub", "knowledge hub") and TLVs after the v1 triple. Static means: changes only on a user decision, because every change burns an announce counter value with an fsync (`2-identity/README.md:97`) and a stale signed copy is served from storage (A6). Hop counts and anything transport-mutable stay out (`:97`).
3. **Dynamic facts: a sealed query after attributed connect.** Model name, queue depth, load, free disk, "gateway online right now" are fetched from the peer over `tcp-lan` when needed. They must **never** be in the TXT or in a signed blob: a signed blob cannot track load without a new announce per change, and the TXT would broadcast the node's state to the LAN. This matches `3-intelligence/README.md:48`: offload "stays on the LAN binding + pairing auth" and "a hub never proxies inference to the WAN".
4. **Never in any advertisement:** alias or display name, load, battery, location, loaded model paths, anything identifying a user rather than a node role; hub address and code travel only inside the enrol bundle.
5. **Phones.** A Termux node cannot browse or answer mDNS (finding 8), so it is discovered *through its hub*, not on the LAN, and it finds a hub from the enrol bundle it already has. The unsolved case is a hub whose DHCP address changed while the phone is away: the phone cannot re-find it with mDNS. Options are a QR rescan, a fallback `/24` unicast sweep on `:11183` (a policy choice for the owner), or waiting for the later Android app / BLE. Open question below.


### 4.4 The mesh-epoch source

Requirements from L2 (§6.7, §9, §13, `README.md:217-220`): certs and attestations expire in a `u64` mesh-epoch counter; "suppressing epoch advancement extends compromised-cert lifetimes; forced advancement is DoS"; v1 evaluates nothing; no wall clock in crypto. The consumer API is already a caller-supplied epoch (finding 11).

**Survey of clockless designs.** Lamport and vector clocks order events but say nothing about elapsed time (we already use them as `msg_seq` and the announce counter). Reticulum has no expiry epoch, only the announce wall-clock stamp (finding 2). Nostr `created_at` and NIP-40 `expiration` are unix timestamps (fetched `01.md`, `40.md`), as are Meshtastic and MeshCore RTC stamps (finding 3). SSB sequence numbers and Matrix DAG `depth` order events inside one log, not time (fetched `matrix-spec` `rooms/v1.md:105,119`; SSB unverified). BPv7 without a clock sets creation time to zero and sums dwell time in a Bundle Age block (`bundle-protocol-v7.md:43-44`): a local-elapsed design. Certificate Transparency's signed tree heads with consistency proofs and gossip detect split views, which is the right idea for a ratchet that cannot go back but needs gossip we do not have (search snippet, IETF CT-gossip drafts). Roughtime (RFC 10049, Experimental, Oct 2026, search snippet, unverified) can anchor a gateway hub's epoch, not the whole mesh.

**Options:**

- **E-A. Hub-signed epoch beacon.** Each hub keeps a persisted counter advanced on its own clock policy (unit: day or week) and signs `"111/epoch/v1" ‖ issuer ‖ epoch` (~105 B). A node adopts the maximum from its **pinned home hub**. The beacon rides ENROLL_OK, the MAILBOX handshake and cards; it is never broadcast. *Suppression:* a frozen hub extends certs for its own members only. *Forced advance (DoS):* a compromised hub can jump its members, who already trust it for custody. *Revocation:* certs die at the next beacon past `expiry`. *Cat-1 relay:* nothing (stock MeshCore repeater evaluates no certs, D19 c); a future 111-firmware relay stores one `u64` and verifies one Ed25519 signature (~45 ms Cortex-M4 class, L2 crypto table).
- **E-B. Highest-seen, signed, step-bounded gossip.** A pinned contact may carry its epoch (an announce `appdata` TLV); a node adopts a greater value only if signed by a pinned issuer and by at most +1 per distinct issuer per observation. *Suppression:* islands drift apart and merge by max. *DoS:* one compromised pinned issuer pushes +1 per observation (bounded rate, not total). Costs extra announce bytes on LoRa and a pin table on the relay.
- **E-C. Release-anchored ratchet plus local dwell.** The project-signed manifest (minisign, already verified by L1) carries `epoch_floor`; between releases a node adds its own persisted *powered-on* monotonic time divided by the unit. *Suppression:* a node that never updates ages only by uptime (off time does not age certs). *DoS:* impossible without the project key, which already controls every update. *Revocation:* coarse, release cadence; a fresh verifier gives an old revoked cert a full lifetime from first sight. *Cat-1 relay:* nothing.

**Pick: E-A as the source, E-C as its floor.** A hub already carries the trust that matters (it holds the mailbox), so a hub-signed beacon adds no new trust root; the manifest floor stops a hub from freezing time forever when the user updates. E-B is the "advance when you see a greater one" design the prompt names; I rank it last because its extra LoRa bytes and per-island drift buy little over E-A, and its natural failure (one forged large value) is exactly the DoS L2 warns about. **v1 should keep "not evaluated"** until the first real consumer (cert delegation, L9, or the first attestation that needs an expiry) is scheduled; the epoch unit is the one decision to take with the owner now, because attestations and certs already encode `expiry` as a bare `u64`.

### 4.5 Runtime join in sequence and fall back

Today (`concepts/preferred-networks.md`): the `111` fabric binds `tcp-lan` only while `hub_role` or `listen_lan` is on and dials the home hub (`daemon/fabric/mod.rs:37-41`); Meshtastic is a bound board plus a 2 s presence read while on; Reticulum is rnsd plus the helper; Delta Chat/Madmail is an IMAP session. All are separate on-demand components, all enabled ones on at once.

**Layer 7 should not add a start-first-found, stop-the-rest state machine.** Delivery first: stopping a working second network removes a path; an enabled network costs only what the user enabled; the PortTable already enforces the exclusive cases (one serial port, one owner). **Use `[order]` as path preference** (which network to try first per peer, rail order, agent proposals). Sequence only where a resource is exclusive: a board that either 111 or rnsd drives, a region or band where sealed frames are forbidden (never a sealed frame into HAM mode; a gateway relaying into a licensed radio needs a human), and a phone on battery. There the "join" is a user-consented switch the agent proposes, not an autonomous sequence.

## 5. Open questions for the owner

1. **Event-driven (A) plus per-transport opt-in (B), or a literal periodic heartbeat for README §7?** LoRa numbers are in 3.1; the LAN needs none with DNS-SD.
2. **The 60 s WAN probe, the 2 s service tick and `mdns-sd`'s 5 s IP check are already in the required configuration.** Move to netlink-triggered probes and address events (finding 6)? Is a recurring contact with `install.111.network` acceptable under "no telemetry"?
3. **Epoch unit (day or week) and timing:** freeze the beacon format with the layer, or define it at the first consumer?
4. **Phones that lose a hub's address** (DHCP change, no multicast on Termux): QR rescan, a `/24` sweep on `:11183`, or wait for an Android app?
5. **A static "willing to gateway" role bit** (signable, consent-gated) separate from live "online now" (queried, never signed): agreed, in coordination with lens 2?
6. **Announce-as-routing vs identity-only** multiplies presence cost by the flood factor; not decided here (relay lens). **MeshCore adverts as a Nearby feed** via our KISS boards: now or at the bench?

## 6. Sources

Fetched (raw GitHub unless noted): `markqvist/Reticulum/master/{LICENSE,RNS/Reticulum.py,RNS/Transport.py,RNS/Destination.py,RNS/Link.py,RNS/Interfaces/AutoInterface.py,RNS/Utilities/rnstatus.py,RNS/Utilities/rnpath.py,docs/source/understanding.rst,docs/source/interfaces.rst}` · `markqvist/LXMF/master/{LICENSE,LXMF/LXMRouter.py,LXMF/Utilities/lxmd.py}` · `meshtastic/firmware/master/{LICENSE,src/mesh/Default.h,src/mesh/Default.cpp,src/mesh/NodeDB.cpp,src/modules/NodeInfoModule.cpp}` · `meshcore-dev/MeshCore/main/{license.txt,docs/cli_commands.md,docs/packet_format.md,src/Mesh.cpp,examples/simple_repeater/MyMesh.cpp}` · `yggdrasil-network/yggdrasil-go/master/{LICENSE,src/multicast/multicast.go}` · `cjdelisle/cjdns/master/{LICENSE,doc/configure.md}` · `libp2p/specs/master/{discovery/mdns.md,rendezvous/README.md}` · `libp2p/rust-libp2p/master/{LICENSE,protocols/mdns/src/lib.rs}` · `briar/briar/master/{LICENSE.txt,bramble-api/.../LanTcpConstants.java}` · `keepsimple1/mdns-sd/main/{README.md,src/service_daemon.rs}` · https://crates.io/api/v1/crates/mdns-sd · `nostr-protocol/nips/master/{01.md,40.md}` · `matrix-org/matrix-spec/main/content/rooms/v1.md` · https://developer.android.com/privacy/local-network-permission · https://developer.android.com/training/monitoring-device-state/doze-standby · `termux/termux-app/master/README.md`.

Search snippets only: https://github.com/meshcore-dev/MeshCore/pull/650 · https://docs.meshcore.nz/cli_commands · https://developer.android.com/reference/android/net/nsd/NsdManager · https://github.com/termux/termux-app/issues/536 · https://www.helpnetsecurity.com/2026/02/27/android-17-beta-privacy-updates/ · https://www.rfc-editor.org/rfc/rfc10049 (fetch denied, 403) · https://datatracker.ietf.org/doc/html/draft-ietf-trans-gossip (RFC 9162 host not allowed).

Repo files read: `README.md` (§6, §7, Category 1 and 5, Intent-Based Routing) · `2-identity/README.md` (§6.2, §6.7, §9, §10, §13, §13.1 A6-A8) · `5-messaging/README.md` (§3, §8.1) · `6-transport/README.md:330-338,836-850` · `3-intelligence/README.md:48` · `4-interface/api/openapi.yaml:1096-1125` · KB `concepts/{preferred-networks,lora-airtime-budget}.md`, `protocols/bundle-protocol-v7.md` · node111 `daemon/{services/probe.rs,dashboard/mdns.rs,transport/{netlink,mod,rnstatus}.rs,fabric/mod.rs,meshtastic/presence.rs}`, `install/{preflight,mod}.rs` · `2-identity/crates/id111/src/attest.rs` · `mesh111/src/store/schema.rs`.
