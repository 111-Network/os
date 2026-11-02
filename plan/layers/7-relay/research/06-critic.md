# Layer 7 research, 06: completeness critic

Critic, 2026-11-01. I read all six reports in full and checked a few claims against the repo and the web. The checks I ran myself are in §4.

## 1. Verdict

The sweep is close to ready for an options brief. These areas are well grounded, with file:line evidence and verified licences:

- presence and the heartbeat numbers;
- the timer baseline;
- relay inside the fabric;
- updates and the KB;
- the UI and the Pi floor.

The heartbeat tension is quantified and left to the owner. Two gaps block the brief:

- **The global carrier fork is not finished.** The scouts did not apply the KB rule for adopting versus building, and they missed carriers that need no new process.
- **Three reports propose three different cross-hub custody designs.** These must be reconciled before "may Layer 7 amend Layer 5" can be put to the owner as a concrete question.

## 2. Gaps

| # | Gap | Severity |
|---|---|---|
| G1 | **The carrier sweep is incomplete.** README §7 says the transport is "adopted via the Knowledge Layer". The KB rule (`0-knowledge/README.md:97`) allows building only when nothing passes the testing gates. Every carrier entry is still a `candidate`, and lens 2's pick A is our own code. Three candidates were missed. **ntfy** is an HTTP mailbox that 111d's existing HTTPS client can reach, with no new process. **Yggdrasil/yggstack** gives hubs reach with no public address and needs no root. An **adopted tunnel** (WireGuard or Headscale) could carry tcp-lan. Lens 2 also picked Noise via `snow`, a new and unaudited dependency, although the rustls already in the tree supports RFC 7250 raw public keys (§4). | blocking |
| G2 | **The cross-hub shapes conflict** (conflict 2 in §3). Nobody answered how a hub learns which hub a non-member `dest` belongs to. `CUSTODY_PUT` carries the dest but not its home-hub hint. | blocking |
| G3 | **Public signed frames have no options.** Three things are missing: the `BROADCAST 0x30` format, attestation gossip and labels (an L6 §18 hand-off), and what "global broadcasts" a 5B gateway actually streams. Nostr was not assessed as a readable-text Adapter for publishers the user follows. | major |
| G4 | **Mesh-epoch E-A is underspecified.** It does not say what advances a hub's counter. Gateway NTP would be a wall clock by the back door; uptime alone is just E-C. Lens 1 also missed lens 4's freeze protection as a first consumer. L5 §4's `epoch` is not L2's mesh-epoch, and the brief must not conflate them. | major |
| G5 | **Resident costs without numbers.** A `_111._tcp` record on a hub whose dashboard is not LAN-bound would start the mdns-sd daemon: one thread, a UDP 5353 listener, and a 5 s IP check, about 12 wakeups/min (`dashboard/mod.rs:247`). Three more costs have no number: the keepalive on the paired-hub connection, the timer behind E-A, and the WAN probe's bytes per day. | minor |
| G6 | **The airtime tables are a little low.** They assume a 138 B announce. The real v1 announce is 141 or 149 B plus the 1 B shim, so the figures are 3–9 % low. Duty cycle is given for the EU only; US and other regional rules are not stated (rule 7). | minor |
| G7 | **The agent's part in Intent-Based Routing was not covered.** That means its tools, whether it proposes or acts, and how a registry hit becomes the private IP literal that `A/src/config.rs:324-335` requires. | minor |
| G8 | **No report states what "Layer 7 on Cat 1" means.** It is the stock repeater's own floods and adverts, and the Cat-1 C core stays deferred (D13 H1). The MeshCore room server, an adoptable store-and-forward, was not assessed. | minor |
| G9 | **L6 v1.1 "hub-member routing of foreign DMs" is unaddressed.** The brief should mark it deferred. | minor |

## 3. Conflicts to resolve before the owner chooses

1. **Gateway advertisement.**
   - Lens 1: a signed "willing to gateway" bit in the announce `appdata`.
   - Lens 5: never a public bit.
   - Lens 2: says both.

   An announce is public in practice (link-up, cards, INTROs).
2. **Cross-hub shape.**
   - Seams: the gateway-as-hub accepts *any* dest, and A+ is *pull* through per-origin cursors over `MAILBOX_FETCH`/`COMMIT`.
   - Lens 2: a rendezvous hub with a new gateway member class.
   - Lens 3: *push* of `Dir::Transit` rows to a paired hub chosen by the home-hub hint, with one hop.

   A+: v1 (lens 2) or v1.1 (lens 3)?
3. **Gateway probe.**
   - Seams: keep `install.111.network` as the target, triggered by netlink.
   - Lens 2: never probe 111's own servers.
   - Lens 5: Termux needs a polling fallback.
4. **Reticulum rule.** Lens 2 treats a new verb on the LXMF helper as needing no rule question; lens 3 says it does. The conservative reading is lens 3's.
5. **Link security.** Lens 2 picks `snow`, but rustls RFC 7250 is already in the tree, and L2 §6.6 allows it on Cat 2/4 TCP. An adopted tunnel would need neither.
6. **Corrections to locked text.** All scouts agree on these; each needs the owner's acknowledgement.

   | Locked text says | The scouts found |
   |---|---|
   | D19 b / L5 §9: "A+ follows without redesign" | A+ cannot be built as worded |
   | L5 §9: custody wrap "~90 B" | 120 B (`frag.rs:64`) |
   | L3 `README:48`: "broadcasts … model, load" | model and load are never broadcast |
   | L1: "KB git-sync" | git rejected |
   | README §6: "join in sequence" | no join state machine is needed |
   | `receipt.gateway` | a trap: set once at install, never re-evaluated |

## 4. Settled here

- **rustls 0.23.45** (`1-bootstrap/Cargo.lock:2003`) supports RFC 7250 raw public keys on both sides:
  - `only_raw_public_keys` at `src/client/client_conn.rs:127` and `src/server/server_conn.rs:132`;
  - `requires_raw_public_keys` at `src/verify.rs:143,272` (local registry source).

  `tokio-rustls` is not in the lock.
- **mdns-sd 0.21.5:** the IP check defaults to 5 s (`service_daemon.rs:69`). The daemon starts only when the dashboard is LAN-bound (`dashboard/mod.rs:247`).
- **ntfy** is dual-licensed Apache-2.0 / GPL-2.0, from https://raw.githubusercontent.com/binwiederhier/ntfy/main/LICENSE and `LICENSE.GPLv2`. Its defaults (`server/server.yml:152,223-225`): a 12 h message cache; attachments up to 15 MB, kept 3 h.
- **yggstack** runs Yggdrasil as a SOCKS proxy or port forwarder with no TUN and no root. It is LGPL-3.0 with a linking exception (https://raw.githubusercontent.com/yggdrasil-network/yggstack/master/LICENSE). It is untested on Termux.
- **Termux** targets SDK 28 (lens 5). The Android 17 local-network permission applies from SDK 37, so it does not bind Termux today.

## 5. Owner forks

1. **Heartbeat:** events only (A), an opt-in interval per transport (B), or a literal heartbeat (C).
2. **WAN probe:** keep it, trigger it from netlink, retarget it, or make it user-off. Also: trim the 2 s ticks, and add a timer gate to G7.
3. **Layer 5:** may Layer 7 amend it, and with which cross-hub shape?
4. **Sealed LoRa:** keep Layer 7 v1 IP-only, or pull in Layer 6's second release, which is hardware-gated?
5. **Carriers for v1:** our own hub, chatmail, Nostr, LXMF, or a G1 candidate.
6. **Link security:** TLS raw public keys, `snow`, or a tunnel.
7. **Gateway advertisement:** sealed, a signed static bit, or public.
8. **Gateway defaults:** licensed boards (hard block or per-link consent); metered links (a budget); chatmail (a default list or always user-named).
9. **The rule on the LXMF helper:** may it get a verb that carries sealed packets?
10. **A second Rust carrier binary** (Nostr or iroh) under the Pi-floor rule.
11. **Registry shape:** this amends L3; offload stays on the LAN or moves to the fabric.
12. **A phone that loses its hub:** a QR rescan, a /24 sweep, or wait for the app.
13. **Mesh-epoch:** define it now, or at its first consumer. Also the freeze stance.
14. **Updates:** which hub listener; `--from`; the L1 trust fixes first; respin or overlay for the KB.
15. **Nearby:** radar only or a map bucket; the angle; 111 peers; position sharing.
16. **Scope:** one hop or chains; LoRa hint custody; MeshCore adverts.
17. **Public frames:** attestation gossip, `0x30`, and the broadcast source, in v1 or deferred.

## 6. Unverified claims: verify on the owner's machine

**A. Denied hosts, or search snippet only**
- RFCs 10049, 9162, 6762/6763 and 6693.
- Briar BSP.
- Arti onion services.
- Reticulum maintainer status (discussion 1069).
- MeshCore PR 650.
- NsdManager MulticastLock.
- Termux issue 536.
- Android metered and Data Saver APIs; the AOSP netlink restriction.
- 47 CFR 97.113(a)(4) and other regions' amateur rules.
- F-Droid Nearby.
- apt `Valid-Until`.
- MeshChatX map.
- Map and ZIM sizes.
- Tuwunel RAM.

**B. Live services**
- Public Nostr relay limits, retention and PoW (403).
- Chatmail operator terms for machine traffic.
- ntfy.sh anonymous limits.
- The public LXMF propagation node set.
- Whether the I2P interface needs a router.

**C. Licences not read from the upstream file**
- chatmail relay.
- Madmail limits.
- dtn7-rs.
- Leviculum.
- batman-adv.
- bluer.
- blake3, bao-tree, gix.
- TUF spec.
- libkiwix, libzim.
- snow.

**D. Measurements**
- Binary and RSS deltas: `nostr-sdk`, iroh, `snow`, `tokio-rustls`, rnsd as a transport, babeld, mdns-sd.
- The payload tarball size.
- WAN probe bytes.
- Test counts after Layer 6.
- The Meshtastic tick with no board, at runtime.
- PoW timing.
- The Reticulum 275 B and 300 B arithmetic.
- Airtime and phone energy on hardware.

**E. Android/Termux**
- `NETLINK_ROUTE` address and route groups at SDK 28.
- Held sockets under Doze and the phantom-process killer.
- yggstack.
- LXMF hosting.

**F. Bench or legal**
- Stock MeshCore floods of our `FF00–FFFF` `GRP_DATA`.
- The KISS modem.
- Shaping on a real LoRa link.
- Cat-1 Ed25519 verify time.
- US, AU and other regional duty-cycle and dwell rules.
