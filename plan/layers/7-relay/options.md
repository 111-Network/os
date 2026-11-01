# Layer 7 — Relay & Presence: options brief

Status: ✅ decided (D30, 2026-11-01: every pick and default taken by the owner) · sweep complete (opus seams scout, 5 sonnet lens scouts, opus critic, 2 sonnet gap-fill scouts; run
`wf_60f165f1-80f`; the critic's digest, the gap-fill digest and the unverified-claims list are §6) · lead-written from the nine reports in `research/` (`00-code-seams.md` line-verified on
`3bf545b`; `01`–`05` web-verified where this box's network policy allowed: package indexes and raw GitHub files only, most documentation
sites denied — every claim the scouts could not fetch is marked in the reports and collected by the critic). Reads with: `os/README.md` §7
and "Category 5" · the hand-offs listed in [`README.md`](README.md) · D17 #5/#12, D19 c, D20, D22, D23–D29 · `AGENTS.md` (the Pi floor).

Owner scope: Layer 6 is locked (D29); Layer 7 is the next layer in `INDEX.md`; `INDEX.md` "Process (per layer)" starts it with research → this brief
→ the owner's decision → spec → fable panel → build (`EXECUTION.md` applies once the spec is decided). **Decided 2026-11-01 (D30); the spec `7-relay/README.md` follows, then the fable panel, the lock and the build.**

## 0. What the research settled (facts that bound every option)

1. **Sealed 111 traffic has one bearer today: `tcp-lan`.** Layer 6 added no second `Link` driver; `route()` knows Direct, Hub, None;
   the fragment plan is fixed at queue time; `Dir::Transit` is a schema value nobody writes; there is no `remove_link`; `Link::send` is
   awaited inside the actor; INTRO rides `tcp-lan` only. Six of the seven Layer 6 seam findings still hold (00 §3 finding 1; the labels
   A1–A7 in 03 §1). Any "delivery path formed dynamically" is built on this.
2. **The required configuration is not timer-free, and nothing gates that.** All components off, `111d` runs about eleven always-on
   intervals, about 205 wakeups a minute, plus one outbound HTTPS `HEAD` per minute to `install.111.network/204` (the WAN probe that
   defines the `gateway` service row today). G7 checks processes, listeners and RSS, not timers or outbound traffic (00 §3.1 a; 05 §3.1).
   A 111 heartbeat adds almost no CPU next to that; its real cost is radio airtime and, when every beat is freshly minted, an fsync on
   the shared announce counter.
3. **The adopted networks already do the periodic part.** Meshtastic sends NodeInfo every 3 h and position every 1 h and calls a node
   "online" when heard within 2 h; MeshCore repeaters flood an advert every 47 h on current `main`; Reticulum announces nothing by itself
   (the 2 % announce cap is per interface; emitters are applications); our LXMF helper already announces every 60 min, `0` = events only
   (01 §1, §3 findings 1–4). Presence of **111** peers, by contrast, is invisible: Nearby admits only `meshtastic` and `reticulum`
   kinds, the fabric's `heard` set and `contacts.last_seen_seq` are not exposed, no 111 peer is discovered on a LAN (`mdns-sd`
   advertises only the dashboard), a non-hub node's `:11183` is loopback-bound (00 finding 8).
4. **The airtime arithmetic of a heartbeat** (one 138 B announce, own Semtech time-on-air computation, within 3 % of the KB): per node
   every 15 min is 0.025 % duty at SF7 and 0.146 % at SF10; **50 nodes at 15 min on SF10 spend 7.3 % of the channel on "I am here"**, at
   3 h 0.61 %; LongFast 50 nodes at 15 min 6.8 % (01 §3.1; 05 §3.2). On a LAN the `tcp-lan` keepalive (25 s per connection) and DNS-SD
   already exist, so a LAN heartbeat adds nothing. Wakeups, not bytes, are the phone cost; Doze bounds any phone timer to ≥ 9 min.
5. **An announce on a 160 B LoRa frame leaves 10 B of `appdata`** (18 B without the hub hint): v1 `appdata` is `[1][roles u16][0]` plus an
   optional 8 B home-hub id; role bits 0 (mailbox) and 1 (always-on hub) are set, bit 2 `ROLE_LORA_ATTACHED` is defined and never set,
   bits 3–15 are free. A replayed stored announce is accepted silently and proves no liveness; a fresh one mints a counter (00 finding 7).
6. **The gateway bit the installer records is dead data** (`receipt.gateway` is always `true`, nothing reads it); the only live gateway
   signal is the 60 s probe, which reports reachability of **111's own server** to 111's own server, 1,440 times a day — a first-party
   phone-home at 60 s granularity, awkward under "no telemetry" (00 finding 3–4; 02 finding 14; 01 finding 5). A timer-free hook exists:
   `transport/netlink.rs` subscribes to `RTMGRP_LINK` into a 2 s debounce; adding the address and route groups is a mask change in a
   Layer 6 file and would let a probe fire on change and `mdns-sd` drop its 5 s IP check (whether Termux allows the bind: unverified).
7. **"The global network" cannot honestly be a global 111 swarm.** 111 has no global address space, no public directory, no broadcast
   source of its own. It is (a) another 111 island's hub reached over the internet, or (b) a third-party mailbox holding opaque sealed
   bytes until the other gateway comes online. Carriers split by what they give: *mailbox* (Nostr relay, chatmail/Madmail, LXMF
   propagation node, SimpleX SMP, a 111 rendezvous hub) versus *pipe* (iroh, Reticulum TCP/I2P links, WireGuard, Tor, a direct dial);
   README §7 asks for the first kind. **Only a 111 hub gives the Layer 5 evidence ladder end to end** (`CUSTODY_ACK` → `relayed`, the
   recipient's `ACK` → `delivered`); Nostr `OK`, SMTP accept and LXMF propagated sends all end at `sent` (02 §1, findings 1–2).
8. **Cross-hub custody does not exist**: a hub accepts custody only for its own members and answers `REJECT_UNKNOWN_DEST` otherwise;
   hub-to-hub replication A+ is specified as "§4 summary vectors" that **a hub cannot compute** (it sees `dest_hint`, size, counters,
   `pkt_ref` and its own `custody_seq`, never sender, stream or `msg_id`). The forward direction is a push of custody rows between hubs and needs
   no cursor at all (07-gap-2 §2: a `fwd` column on the custody table; `Dir::Transit` is a `messages` row needing peer, stream,
   epoch and `n`, which a hub never sees); multi-copy replication needs `pkt_ref` set reconciliation (00 finding 13; 03 findings
   7–8).
9. **A quantitative trap in built code:** `CUSTODY_WRAP` is **120 B** (`frag.rs:64`, pinned by a test), not the "about 90 B" of L5 §9. A
   custody-wrapped `CUSTODY_PUT` cannot be planned on the 160 B flood floor (`MtuTooSmall`) and leaves 24 B of text on a 233 B Meshtastic
   frame. Custody over LoRa needs an unwrapped "hint custody" form first; custody over `tcp-lan` and Reticulum links is fine (03 finding 6).
10. **Reticulum as the router for sealed 111 frames is possible only through our LXMF helper**, and costs: PLAIN and GROUP packets are
    dropped beyond one hop (only SINGLE destinations and links route); a 160 B packet becomes about 275 B on the wire (two RNode frames,
    0.80 s vs 0.46 s at SF8); a link costs 297 B in 3 packets; upstream names only microReticulum and Reticulum-Go as acceptable
    reimplementations and "strongly advises against" others, which closes P5 and any Rust Reticulum in `111d` (03 findings 2–5). The
    Reticulum License text is now verified (an MIT grant plus the no-harm and AI-training clauses); `LICENSING.md:56` still says unverified.
11. **The stock MeshCore repeater floods our frames blind** with its own seen-table and a 63-hop path cap (D19 c holds): RF multi-hop
    device-to-device is solved by an adopted program; what is missing is carry-forward across islands and offline recipients (03 §1,
    finding 1). A Cat-1 relay can verify nothing (1.5–2 s per Ed25519 verify on a Cortex-M4 class part) and carries nothing of
    presence, updates or epochs.
12. **The updater is carrier-free but origin-locked.** Trust is minisign over the raw manifest, then the floor, then sha256+size pins, then
    the staged tree, repeated by the root helper from staging alone; a hub could serve the identical bytes. What blocks it: two hard-wired
    HTTPS bases, absolute payload URLs inside the signed manifest (fetching a pin by its sha256 from a hub needs no manifest change), no
    `111 update` verb, and the home hub is a Layer 5 setting written at enrolment (not in the install receipt) (04 §1, findings 1–5).
13. **Four defects in the as-built trust path matter once a hub is an origin** (04 findings 6–9): the manifest's `channel` is never compared
    with the expected channel (a signed `beta` manifest passes as `stable`); the keychain rotation chain is parsed, never verified (a node
    that sleeps through a rotation cannot follow it); there is no freeze protection (a stale hub can hold a node on an old valid manifest
    forever; TUF's answer is signed expiry, which needs the mesh epoch Layer 7 is to define); `min-upgrade-from` is enforced only by the
    installer. These are Layer 1 amendments.
14. **LoRa never carries a release; it can carry "a release exists".** A 5.7 MB payload is 15 h of LongFast airtime unsealed (62–110 days
    under a 1 % duty cycle); the whole KB is **96.6 KB** as `tar.gz` (measured; 15–27 min of airtime); a manifest plus signature (~3.2 KB)
    is 30–55 s. A whole-repo `git bundle` is 5.2 MB and git is GPL-2.0-only (a separate process at most); `build.py` is stdlib-only (04 §1,
    findings 11, 14).
15. **The `KB`/`UPDATE`/`PATCH` kinds exist and nobody produces them**; "public class" only binds the signature to a zero `dest`, the frame
    is still sealed to one recipient; `BROADCAST` 0x30 is refused as not built; Tier 2 (`OBJ_ADVERT`/`OBJ_REQ`/`OBJ_CHUNK`) is specified for
    v1.1 and the `tcp-lan` chunk frame is reserved (00 finding 12; 04 finding 12).
16. **L2 anticipated Nostr, Meshtastic and Reticulum** as attested transports (`0x01` nostr-secp256k1 fails closed "Layer 7's"), and L2's
    expiry already takes a caller-supplied epoch (`verify_full(.., current_epoch)`, `is_expired(current)`); the bridge passes `None`. So
    Layer 7's epoch source is one function plus a unit; the attestation record exists, nothing gossips it (00 finding 14; 01 finding 11).
17. **Reticulum is not clockless:** its announce embeds a 5-byte wall-clock stamp and the transport refuses older stamps per destination.
    Nostr, Meshtastic and MeshCore timestamp with wall clocks; SSB, Matrix depth and Lamport counters order events, not time; BPv7 without
    a clock sums dwell time (01 §4.4).
18. **A map does not fit the UI budget; a radar does.** The lazy bucket holds 219.9 of 256 KB gz (36.1 KB free; the per-chunk cap is 64 KB).
    Leaflet is 42.4 + 3.5 KB, Leaflet + PMTiles + protomaps-leaflet ~69 KB, MapLibre GL 451 KB; the hosted Meshtastic web client already
    bundles MapLibre and is blank offline. The CSP (`connect-src 'self'`, `img-src 'self' data:`) blocks any off-origin tile host, which
    keeps the offline guarantee; a PMTiles Range route in `111d` would be ~60 lines and no resident weight. LoRa gives hops and signal,
    never bearing; Meshtastic truncates positions to a per-channel bit count and centres them (05 §1, §3.3).
19. **Termux cannot read Android's metered flag** (verified from the Termux:API source); Termux and Termux:API target SDK 28, so the
    netlink bind probably still works there (unverified). Doze suspends network; exact alarms fire at most once per 9 minutes (05 §3.5).
20. **Chatmail numbers, verified from the relay's own template:** 30 MiB messages, 20-day retention (7 days above 200 kB), 60 messages
    per minute (burst 10), encryption enforced; Madmail federates over HTTPS and works IP-only (`--simple --ip`), so an owner-run Madmail
    on a public address is a rendezvous without a domain. Nostr: ephemeral kinds are not stored, replaceable kinds keep only the latest,
    a mailbox needs a regular kind that public relays may refuse; a 160 B packet becomes a ~601 B event (3.8×), a 16-fragment batch ~3.8 KB
    (1.5×); NIP-13 PoW at difficulty 28 is minutes per event; live public-relay limits could not be read from this box (02 findings 4–10).
21. **The Pi-floor table exists** (05 §3.1): every optional program measured so far is 29–64 MB RSS while on and zero while off; rnsd as a
    transport node, rust-nostr, babeld and a BLE scanner have **no measured RSS** anywhere in the repo.
22. **Offload discovery takes a private IP literal only** (`agent111` `model_base`, no DNS); the Layer 3 README's "rides the fabric" is not
    what is built (LAN HTTP + pairing) (00 finding 17).
23. **No adopted carrier can pass the KB's `testing` gate from desk research** (`0-knowledge/README.md:93`), so the adopt-vs-build rule's
    operative branch (`:97`) is "prototype the best candidate and write the smoke log"; three problems the sweep had mixed are distinct:
    a *mailbox* that holds bytes (ntfy, chatmail, Nostr, LXMF, our hub), *reach* for two always-on ends with no public address (Yggdrasil,
    WireGuard, Headscale) and *link crypto*; reach tools are not store-and-forward. ntfy is the lightest mailbox (the existing
    ureq + rustls client; 4,096 B bodies; a 12 h in-memory default cache; the topic name is the only default access control); yggstack is
    rootless reach at the price of a resident Go process on both ends and no semver tag (07-gap-1 §1–§3).
24. **TLS 1.3 raw public keys through the rustls already in the lock is feasible with no new crypto crate** (the verifier and resolver
    hooks and `verify_tls13_signature_with_raw_key` read in the local 0.23.45 source; a tokio adapter or `tokio-rustls` 0.26.6 is the
    only addition); the link key must be random, attested and never the identity key (`2-identity/README.md:372`), so it is a fresh
    Ed25519 key per node pinned from the hub card, and attesting it is an L2 registry change (07-gap-1 §3 items 9–12, §3b).

## 1. What Layer 7 is, in one paragraph

Layer 7 makes a node's reachability honest and makes delivery cross islands: it shows who is near and how that is known (presence per
transport, "last heard" as a local fact), lets a node that has internet become a gateway by consent and carry sealed 111 envelopes to
another island through an adopted store-and-forward carrier, pairs hubs so custody can cross from one hub to another, lets a hub be the
update and KB origin for nodes that never see the internet, and defines the clockless epoch the identity layer's expiries count in. It
adds no routing protocol of its own (the stock MeshCore repeater floods; Reticulum routes its own traffic), no resident process, no
periodic timer the owner has not approved, and nothing that leaves the node unsealed except what the user publishes or bridges.

## 2. Polled forks (owner decides; recommendation first)

### F1. Presence: what "continuously announce reachability through heartbeats" becomes

- **A. Event-driven presence as the base, piggybacked evidence included by construction, a per-transport heartbeat only as an opt-in
  switch that is off by default.** Announce on link-up, card, INTRO and on request (built); `last_seen_seq` bumps on any verified packet;
  custody receipts and the foreign networks' own presence count as "alive"; a `111` kind joins Nearby with a local age and the evidence
  ("LAN link, verified packet", "through the hub, custody held", "heard via Meshtastic on …"). Per transport a component switch with an
  interval, `0` = events only, the LXMF precedent; LoRa default off, never on a duty-cycle-limited or amateur band; a phone never below
  9 min. No timer in the required configuration. *(pick: 00 §4, 01 §4.2, 05 §4.2 agree)*
- B. A literal periodic heartbeat on every transport — fact 4: 7.3 % of an SF10 channel at 50 nodes every 15 min; conflicts with the Pi
  floor and with L5 A6 (announces on demand). Needs the owner's say and an airtime ceiling the owner accepts.
- C. Event-driven only, no switch at all — loses the one thing a heartbeat buys (a silent peer still shown fresh); A keeps it opt-in.
- The owner decides whether README §7's sentence is met by A or must be reworded (the plan correction is recorded in §4 either way).

### F2. The gateway test and the timer baseline

- **A. Make "internet" mean "the owner's chosen carrier endpoint is reachable", evaluated on a netlink address/route change and on a carrier
  attempt, never by contacting `install.111.network`; drop the 60 s HEAD; `mdns-sd`'s 5 s IP check follows the same events.** Changes a
  Layer 4/6 probe cadence (the `gateway` service row keeps its shape; `wan_ok` becomes "carrier reachable at last check, N min ago").
  On Termux the netlink bind is unverified (fact 19), so the event trigger keeps a coarse polling fallback there (the critic's
  conflict 3: the seams scout kept the 111 target, lens 2 moved it to the carrier, lens 5 asked for the fallback; A takes all three).
  *(pick: 00 §4, 01 §5 q2, 02 finding 14, 05 §3.1)*
- B. Keep the 60 s probe as it is (it is already in the measured floor) — but it is a first-party phone-home 1,440 times a day and a
  several-MB-a-day cost on a phone plan; "no telemetry" is the owner's rule to read.
- C. Keep the probe, make it user-off on Pi and phones.
- Beside it, two owner calls the sweep surfaced: **trim the baseline** (the two 2 s Meshtastic ticks and the 2 s panels tick run with no
  board bound — a separate decision, fact 2), and **add a no-timer gate** (wakeups and outbound requests at idle) to G7 so the Pi-floor
  clause "no periodic poll" has a measurement.

### F3. The global store-and-forward carrier(s) for v1

- **A + P (P: the ntfy prototype, 07-gap-1's lettering). A 111 hub reachable over the internet (a hub with a public address, or a home hub exposed through a reach layer; peer hubs
  and 5A/5B gateways dial out; the same envelope, receipts and custody; `tcp-lan` wrapped in TLS 1.3 raw public keys, below) as the
  evidence-grade path, and ntfy as the first adopted mailbox, prototyped behind the hub's carrier hand-off (F4 A) for islands with no
  public address on either side.** A is built code plus glue (`tcp-lan` and the hub mailbox exist; what is new is the forward row of
  F4 and the reach); ntfy is reached by the ureq + rustls client `111d` already has: no new process, crate or listener, zero weight
  while off, works through NAT. The gap-fill scout applied the KB's own gate (`0-knowledge/README.md:93-97`): **no carrier can pass
  `testing` from desk research**, so the rule's operative branch is "prototype the best candidate and write the smoke log", never
  "build"; by release evidence ntfy (v2.36.0 on 2026-10-31, monthly tags; Apache-2.0 / GPL-2.0 dual, both files read) is the healthiest
  and lightest adopted mailbox, with bus factor and issue response unread for every candidate (api.github.com is closed from this box).
  Its shape and limits (07-gap-1 §3): base64 sealed batches in a 4,096 B body (a 16-fragment message at the 160 B floor, ≈ 1.07 KB, fits
  one post; TCP-sized frames need chunking or the attachment path); `GET /<topic>/json?poll=1&since=<id>` to drain; a 12 h **in-memory**
  default cache lost on restart (`cache-file` and `cache-duration` for durability on a self-hosted server); public-instance limits
  (burst 60 then one per 5 s, 30 connections per visitor; ntfy.sh 250 messages a day anonymous), so one batch per message flush; **the
  topic name is the only default access control** (anyone who knows it reads and writes it), so the topic derives from a pairing
  secret, never from the NodeId, or a self-hosted `auth-file` ACL; the operator sees IP, topic, size, timing and the cleartext header
  (`dest_hint`, counters). The carrier leg earns no state (an HTTP accept is no `CUSTODY_ACK`): a sender whose own node is the gateway sees `sent`,
  a member whose hub holds the item for its carrier sees `relayed` (reason `forwarding`, F4 A); `delivered` is the recipient's own
  sealed `ACK` and rides any carrier with a return path (a reverse topic), so it works on ntfy too; carrier loss costs a resend, never
  a message (the sender keeps its copy until `delivered`, L5 §9). *(pick: 07-gap-1 §4.1 P + Q; 02 §4.2; 00 §4 "gateway as hub")*
- B. The chatmail/Madmail mailbox through the Layer 6 Delta Chat component (sealed batches as attachments; verified limits, fact 20):
  already shipped and measured (~64 MB RSS while on, zero off), ceiling `sent`, the metadata is mail addresses and the relay sees sizes
  and timing (the consent copy says so). **Stays available as the carrier that works today**; heavier than ntfy (an rpc-server per
  account). *(07-gap-1 §4.1 R)*
- **The reach layer for A, in the owner's order:** a public address first (nothing new); **yggstack** second, opt-in (Yggdrasil in user
  space: SOCKS5 and `-remote-tcp` port forwarding, no TUN, no root; a hub behind CGNAT exposes 11183 on its Yggdrasil IPv6, which
  `dial_peer`'s configured `SocketAddr` takes; LGPL-3.0 with a linking exception, verified) — but a resident Go process on **both** ends,
  RSS unmeasured, **no semver tag** in the Go proxy (the release gate fails on what could be read), one Termux anecdote, public peers are
  stranger-run nodes (an egress consent item) and upstream calls Yggdrasil "early-stage"; an **adopted tunnel** third, documented and
  never required (WireGuard needs an interface, root on Termux; Headscale needs a public control host; a stock Tailscale client logs to
  its vendor, a KB telemetry auto-fail, unless `TS_NO_LOGS_NO_SUPPORT`). Reach tools are pipes, not mailboxes: two intermittent 5A
  islands never meet through them, which is why the mailbox (ntfy, B) exists at all (07-gap-1 §1, §3 items 7–8).
- C. Nostr relays (L2's reserved `0x01`): the largest free infrastructure, but public acceptance of opaque regular-kind events, PoW and paid
  policy are unverified from this box, the overhead is 1.5–3.8×, `nostr-rs-relay`'s last crates.io release is 2023-08 (the release gate
  fails there), and the client needs secp256k1 and websockets — a **separate CI-built carrier binary**, never in `111d` (a build-time
  weight the Pi-floor rule reserves to the owner). v1.1 after a bench reads real relays.
- D. LXMF propagation (one more helper verb, fact 10): zero new component where network 2 is on; 30-day expiry, 256 KB, sender stamps,
  `sent` only; bus factor 1 upstream (`reticulum.md:49`). v1.1, and it opens the rule question of F4 C.
- iroh (hole punching, self-run relay with allowlist; +13 MB; default discovery is n0's infrastructure) solves NAT, not storage: v1.1 as an
  optional component, on the owner's say. Tor/arti hosting is documented as unfinished; dtn7 loses on IP too (no custody, old deps,
  no aarch64 asset); Meshtastic MQTT is a bridge category, not a 111 carrier.
- If the owner prefers the L2-anticipated path, swap ntfy for C; the cost is the unverified relay policy and the second binary.
- **The internet link's security (conflict 5), costed by the gap-fill (07-gap-1 §3b):** **(a) TLS 1.3 with raw public keys (RFC 7250)
  through `rustls` 0.23.45** *(pick)* — already in the lock with `ring`, RPK on both sides confirmed in the local registry source
  (`only_raw_public_keys`, `requires_raw_public_keys`, `verify_tls13_signature_with_raw_key`, Ed25519 via `any_eddsa_type`), so no new
  crypto crate and no new `LICENSING.md` row; the cost is verifier/resolver glue pinning one SPKI (~100 lines) plus a tokio adapter
  (~150 lines over `read_tls`/`write_tls`, or `tokio-rustls` 0.26.6, MIT OR Apache-2.0: one lockfile entry and one row, the owner's
  call). **(c) an adopted tunnel** under `tcp-lan` (above): zero 111 code, documented as the owner-run alternative. **(b) Noise IK via
  `snow`** 0.10.0 (Apache-2.0 OR MIT; unaudited, L2 §11; buffer-based, no async glue) only if a short spike shows the rustls glue costs
  more than the audit gap. (d) nothing, as locked: `dest_hint`, counters and sizes stay visible on path. What every option hides from
  the path: frame types, `dest_hint`, counters, static keys; what the far hub still sees: `dest`, size, `prio_flags`, `dest_hint`,
  counters (`5-messaging/README.md:409`). **The link key follows L2 (`2-identity/README.md:372`): random, attested, never seed-derived,
  never the identity key** — a fresh Ed25519 key per node pinned from the hub card (TOFU from the card in v1); attesting it needs a new
  `transport_id` in the L2 registry (the upgrade, an owner question).

### F4. Cross-hub delivery inside the fabric (what "delivery paths form dynamically" is in v1)

- **A. Single-copy push hand-over of custody between paired hubs, and from a hub to its own carrier: a star of owner-paired hubs, transit
  only at hubs, no cursor, on IP links.** The gap-fill scout reconciled the three conflicting shapes into this one (07-gap-2 §2, fork F1).
  Pairing is two §7.3 enrolments in both directions; each hub records the other in a new `FabricSettings.paired_hubs {node_id, leaf,
  gateway}` and dials it through the daemon's hand-configured `peers`; a paired hub is never a home hub (`home_hub` stays single). A
  forward row is an ordinary `custody` row whose `dest` is a non-member, plus a `fwd` column (1 from a member, 2 from a `leaf` hub, 3 from
  the carrier) — **not `Dir::Transit`**, which is a `messages` row needing peer, stream, epoch and `n`, none of which a hub can see in a
  sealed item. The widened gate: an enrolled member's `CUSTODY_PUT` for a non-member `dest` is held for forwarding and answered
  `ACCEPTED` when a target exists (a paired hub, else this hub's carrier), else `REJECT_UNKNOWN_DEST` as today; **the receiving hub needs
  no change** (a paired hub is an enrolled member that puts for the receiver's members, which today's gate accepts). A hub never forwards
  what came from a paired hub, except a `leaf` hub's rows onward to this hub's carrier (the 5A/5B gateway); `pkt_ref` stays UNIQUE and is
  kept after hand-over, so a bounced item is `DUPLICATE` and done; loop freedom is structural (the `fwd` table), no TTL. Rows are
  protected while `committed = 0` (other people's undelivered items), `committed = 1` on the target's `ACCEPTED`/`DUPLICATE` and then
  evicted by the existing prune step 1; a quota-only `Class::Transit` keeps strangers' items from starving member custody; over quota
  with only protected rows, new forward puts get `REJECT_FULL`. The forwarder is event-driven (row inserted, link up, contact event; no
  timer). **Home-hub discovery:** the sender already holds the dest's 8 B home-hub hint (`contacts.home_hub`, from the signed appdata)
  and adds it as ext TLV `0x02` inside the hub-sealed `CUSTODY_PUT` (11 B, optional; old hubs skip an unknown TLV, `5-messaging/README.md:110`);
  without it the custodian probes paired hubs in a fixed order, stops at the first `ACCEPTED`/`DUPLICATE` (a yes is proof: the answerer's
  gate checked enrolment) and caches dest → hub in memory (≤ 256 entries, ~16 KB). Signed member lists and enrol mirrors are rejected
  (they leak the roster, go stale with no clock, and do nothing for a carrier that addresses by a tag of `dest`). **Evidence:** the first
  hub's `ACCEPTED` earns `relayed` with a new reason `forwarding` (honest: a mailbox node durably holds it in a protected row; the L5
  §6.2 "for a known member" clause is widened); later hops earn no state; `delivered` is only the recipient's `ACK`, an ordinary item on
  the reverse path. **Cost** (07-gap-2 §4): ≈ 1.2 KB on `tcp-lan` per 100 B text forwarded (6.3× the item) plus ≈ 0.95 KB for the return
  `ACK`; per hub one column, a ~16 KB cache, one held connection per paired hub (the 25 s keepalive: +2.4 % wakeups per pair, ≈ 0.4 MB
  a day); **nothing on a non-hub node** (every new path sits behind `holds_custody()`; `paired_hubs` is empty; nothing dialled or
  stored). The carrier is reached through a **hand-off API** (`forward_take(target, n)`, `forward_done(seq, Taken | Rejected | Retry)`,
  `carrier_inbound(dest, item)`, a "forward pending / target up" event) over a loopback JSON-lines pipe, the LXMF helper's pattern,
  spawned on event (5A) or one held child (5B), zero while off: **the carrier is not a `Link` in v1**, so `remove_link`, a new
  `LinkKind`, the openapi enum and `Route::Via` stay untouched. Exactly the twelve additive or default-off amendments of F5; with no
  pairing and no carrier the hub is byte-identical to today. *(pick: 07-gap-2 §2, §5 F1; 03 §4.2 for the star)*
- A′. The rendezvous form (lens 2; 07-gap-2 fork F2): peer hubs and gateways are members of a rendezvous hub that holds `dest ‖ item`
  under the member hub as `dest` and is drained by `FETCH`/`COMMIT` with cursors. It lets a **non-hub phone carry others' items**
  (member-gateways) and resume by cursor, at the cost of a new `MAILBOX_XITEM` kind, a `custody.dest` that is not the real dest and
  `on_item` changes. Only if the owner says a phone that is merely a member must carry traffic; A does not preclude it.
- A″. A+ pull by per-origin cursors over `MAILBOX_FETCH`/`COMMIT` (the seams scout; 07-gap-2 fork F3): needs member-list exchange, a
  cursor table and replica flags, and the §4 vectors are unbuildable by a hub — the largest and least private shape. Rejected.
- B. Reticulum as the router via the LXMF helper (`rns-bridge`): true multi-hop across IP-less islands with an adopted router, at the cost
  of A1, A3, A5, A6, A7, doubled LoRa airtime for small frames, a resident Python helper plus an rnsd transport role on every cross-hub
  path, a forked relay fleet, and `SENT`-only evidence. v1.1 optional long-haul carrier, after F4 C.
- C. The rule question B opens: may the LXMF helper gain a "carry a sealed 111 packet" verb (the same program that already imports RNS,
  a new use), must that be a separate importing program of ours, or neither (carrier only through the `rncp` spool)? The brief takes the
  conservative reading (lens 3's, the critic's conflict 4): a new verb that carries 111 frames is a rule question for the owner, never
  the default, even inside the existing helper.
- D. Multi-copy epidemic replication between hubs (A+ as specified, with `pkt_ref` set reconciliation in Negentropy's shape and the relay
  flood ring): v1.1+, only if three or more hubs need redundancy.
- E. IP-level mesh routing (babeld, Yggdrasil): not Layer 7 at all — `tcp-lan` reaches any IP address; a Wi-Fi mesh on Linux hubs is at
  most a Layer 6 spine recipe (root, no Termux). (yggstack as a hub's *reach* is F3, not routing.)
- v1.1 (recorded now): the radio form of A — hint custody without the 120 B wrap (fact 9; 131 B with the hint; it exceeds the 160 B
  floor), with A1, A3, A5, A6, A7 (the Layer 6 "second release" set, D25 F6); chains beyond the `fwd` table (a plaintext hop count, an
  envelope change); a signed forward receipt (the dest hub's `custody-sig`); member-gateways (A′); sweep-before-ack (restore today's
  honest `hub_not_yours` when every reachable paired hub refuses); an own-send carrier route for a non-hub node (`Route::Carrier`,
  per-route planning, the `carrier` `LinkKind`).

### F5. Amending locked Layer 5 (permission, not design)

- **A. Layer 7 may amend `mesh111` additively, each amendment named and owner-visible — exactly the twelve of `research/07-gap-2.md` §3,
  all additive or default-off, byte-identical with no pairing and no carrier: (1) the custody gate widened to hold a non-member `dest`
  for forwarding when a target exists (the L5 §9 spec text); (2) the scope sentence ("needs a paired hub or a gateway carrier; otherwise
  the message stays queued, `hub_not_yours`"); (3) the A+ sentences of L5 §9/§21 and line 30 replaced by single-copy push (the §4 vectors
  are not computable by a hub); (4) "≈ 90 B" → 120 B (`CUSTODY_WRAP`, `frag.rs:64`; 131 B with the hint); (5) V4 migration `custody.fwd
  INTEGER NOT NULL DEFAULT 0 CHECK (fwd IN (0,1,2,3))` (§10.3); (6) a quota-only `Class::Transit` (§10.4); (7) ext TLV `0x02 dest_hub(8)`,
  `CUSTODY_PUT` only, in the registry; (8) the `relayed` evidence clause of §6.2 widened "or for forwarding", reason `forwarding`;
  (9) the reason enum in `outbox.rs` and `openapi.yaml` (then `pnpm gen:types`); (10) `FabricSettings.paired_hubs`, a pairing outcome of
  `on_enroll_ok` that does not set `home_hub`, the member count excluding paired hubs; (11) the forwarder arms and the three hand-off
  entry points; (12) the §7.3 sentence "pairing is two enrolments; a paired hub is not a home hub; one home hub in v1 stands". Envelope
  bytes, L2 identity and the 16 B `pkt_ref` are untouched; the wire-visible additions are one optional TLV and one reason string; an old
  hub skips the TLV and rejects as today. `Dir::Transit` and the "delivered transit copies" prune query stay unused; `remove_link` and
  a non-blocking `Link::send` move to v1.1 with the `carrier` `LinkKind` (F4). The rest (A1, A3, A5, A6, A7, the route variant) with the
  radio form in v1.1.** *(pick)*
- B. Layer 7 stays outside `mesh111` entirely: then cross-hub delivery is a sidecar re-implementing custody beside the store — a second
  copy of the same logic, rejected on "never reinvent".
- The same question for Layer 1 (F6, the Layer 1 amendments bullet) and Layer 2 (F8: the epoch unit).

### F6. Updates and the KB where there is no internet

- **A. Hub as the HTTP origin plus `111 update --from <dir|url>`: "an origin is a set of files" (`channel-111-stable.toml`, `.minisig`,
  `blob/<sha256>`); the hub serves them read-only and unauthenticated (public signed bytes, the `/health` posture) only while `hub_role`
  is on, on a listener that already exists; a node tries `[home hub, internet bases]` in an order the user can see; `--from` reads a mounted
  stick or a URL; every byte passes the compiled-in key, the floor and the pins as today.** No resident process, listener or timer on a
  non-hub node. *(pick: 04 §4 fork A2)*
- B. Status quo (internet only) — fails the offline-first promise for every node that never meets a gateway. C. Tier 2 bao-verified chunks
  over the fabric — the right shape for non-IP bearers and Layer 8 datasets; v1.1, same staging directory. D. Sneakernet only — a subset of
  A; keep `--from` as the guaranteed floor, never the only path. LoRa carries the manifest ("release X exists, bring a stick or reach a
  hub"), never a payload (fact 14).
- **The KB:** K1 in the release payload (status quo; a KB change is a calver respin, a few MB on a LAN hub) *(pick for v1)* vs K2 a KB
  overlay with its own signed pin (the `[kb]` pin already parses; needs a root-helper request type and a floor dimension) — pick K2 only if
  the owner wants a KB cadence apart from the binary's. Git bundles and gateways fetching the 111 repo: rejected for the node (fact 14).
- **"Manifest gossip":** M1 pull on link-up (when the fabric link to the home hub comes up, or `--from` runs, check that origin once; no
  gossip, no timer) *(pick)*; M2 a 5-byte release tag in the announce `appdata` (fits the 10 B; unsigned, can only cause a poll) later;
  M3 a signed `UPDATE` notice frame — no gain over M1.
- **Layer 1 amendments first (owner's permission): the `channel` check, the keychain walk (previous key signs the next pubkey, walked
  from the compiled key), `min-upgrade-from` in the helper, and a freeze stance for v1** (accept that a stale hub holds a node until it
  reaches any other origin; add expiry once the epoch exists). A new verb that writes into `state/staging/` and drops
  `update-request.json` (the helper stays the only flip actor) is acceptable under the Layer 1 model; the owner confirms.

### F7. The Service Discovery Registry and gateway advertisement

- **A. Three layers, each field in the cheapest layer that can hold it: DNS-SD `_111._tcp.local.` as a second `ServiceInfo` in the
  `mdns-sd` daemon that already runs while LAN sharing is on (instance = the L2 label, SRV `:11183`, TXT `v`, `r` role bits, `k` NodeId;
  unauthenticated by design — the `tcp-lan` handshake binds; browsing on demand only); static signed capability as role bits and TLVs in
  the L2 announce `appdata` ("willing to gateway", "offload hub", "knowledge hub": static, consent-gated, each change burns a counter);
  dynamic facts (model, load, queue, "internet right now") by a sealed query after an attributed connect, never in a TXT or a signed
  blob.** Never advertised: alias, load, battery, location, loaded models, anything naming a person. *(pick: 01 §4.3)*
- B. A broadcast registry frame on every transport — against A6 and the Pi floor; rejected.
- **Gateway advertisement scope (the critic's conflict 1):** lens 1 put a static signed "willing to gateway" bit in the announce
  `appdata`, lens 5 said never a public bit, lens 2 said both and picked members-only. An announce reaches anyone on link-up, in a card or
  in an INTRO, so a role bit is public. *(pick)* members and contacts only, sealed (the hub offer, INTRO, cards); the signed static bit
  stays available as the owner's choice, with its cost named: every listener learns who has internet and battery.
- Phones: a Termux node cannot browse or answer mDNS; it is discovered through its hub and finds its hub from the enrol bundle. The
  open case is a hub whose DHCP address changed while the phone was away: a QR rescan, a `/24` unicast sweep on `:11183` (a policy
  choice), or wait for an Android app or BLE.

### F8. The mesh-epoch source (L2 §13)

- **A. A hub-signed epoch beacon (E-A) as the source, the release manifest's `epoch_floor` (E-C) as its floor.** What advances a hub's
  counter must be said (the critic's G4): **its own persisted powered-on monotonic time divided by the unit, floored by the manifest** —
  that is E-C evaluated at the hub and *distributed* to its clockless members; never a gateway's NTP clock (a wall clock by the back
  door). So E-A is "hub-distributed E-C"; the beacon advances on events (each handshake reads the counter), with no timer. Each hub
  signs `"111/epoch/v1" ‖ issuer ‖ epoch`; a node adopts the maximum from its pinned home hub;
  the beacon rides `ENROLL_OK`, the mailbox handshake and cards, never a broadcast. A frozen hub extends certs for its own members only;
  a compromised hub can jump its members, who already trust it for custody; the manifest floor stops a hub from freezing time once the
  user updates. A Cat-1 relay is asked for nothing. **v1 keeps "not evaluated"** until the first real consumer (cert delegation, L9, or an
  attestation that needs an expiry) is scheduled — the candidate first consumer is **freeze protection for updates** (F6: an "age by
  mesh-epoch" on the newest manifest seen); the **epoch unit (day or week) is the one decision to take now**, because attestations
  and certs already encode `expiry` as a bare `u64`. Not to be confused with L5 §4's `epoch` (one announce counter per store open), which
  is a different thing. *(pick: 01 §4.4, amended per the critic)*
- B. Highest-seen, signed, step-bounded gossip (E-B, the "advance when you see a greater one" design): extra LoRa bytes, per-island drift,
  and its natural failure (one forged large value) is exactly the DoS L2 warns about. Ranked last.
- C. Release-anchored ratchet plus local dwell only (E-C alone): revocation at release cadence; a fresh verifier gives an old revoked cert a
  full lifetime from first sight. Kept as the floor of A, not the source.

### F9. Nearby: radar, map, positions

- **A. R1 now: an own SVG radar inside the lazy `nearby` chunk (centre = this node; ring = hop count, ordered by SNR within a ring; angle
  only when both ends report a position, else a stable hash and the legend says so; every Meshtastic position drawn as an uncertainty
  cell, never a pin; an outer "unplaced" ring); positions of 111's own nodes never shared by default; a link to the hosted Meshtastic
  panel for its map. R2 when Layer 8 has a region file: Leaflet + PMTiles + protomaps-leaflet (~69 KB) in an exempt "map" bucket (the
  fonts precedent in `check-budget.mjs`) loaded only where `/api/v1/tiles` lists a set, behind a Range route in `111d`.** *(pick: 05 §4.1)*
- B. MapLibre GL in the bundle: 451 KB, seven times the chunk cap, WebGL on old phones — contradicts D21. C. Link-out only: Reticulum and
  111 positions never mapped.
- Owner questions inside: is an exempt map bucket an amendment of D21 (yes, recorded in §4); is "angle has no meaning without positions"
  acceptable or should the radar be a ring list; position sharing none by default (pick) or opt-in per contact with precision bits.

### F10. Sequencing and the first Layer 7 release

- **A. v1 = F1 A (presence, the `111` Nearby kind, the per-transport switch) · F2 A (event-driven gateway test, the probe gone) · F7 A (the
  registry; gateway advertisement to members) · F3 A + P (the hub over `tcp-lan` in TLS-RPK, the ntfy carrier prototype as the first
  v1 task so the KB gets its smoke log; B chatmail stays available) · F4 A + F5 A (hub pairing, the `fwd` push hand-over, the hint TLV,
  the twelve amendments) · F6 A (hub as update origin, `--from`, the Layer 1 trust amendments first) · F8 A (the beacon format and the
  epoch unit, nothing evaluated) · F9 R1 · F11 A (the `BROADCAST` 0x30, attestation-gossip and label formats pinned in the spec) · consent and
  metered-link settings (05 §3.5: a user setting in v1, the licensed-radio fact consulted on every new path). v1.1 = the radio form of F4 (hint custody, A1/A3/A5/A6/A7), chains, a signed forward receipt,
  member-gateways, sweep-before-ack, an own-send carrier route and the `carrier` `LinkKind`, Nostr and LXMF carriers, yggstack as an
  opt-in reach component, iroh, Tier 2 transfer, the map bucket, the first epoch consumer, multi-copy replication, the 0x30 and attestation consumers with the
  Nostr text Adapter (F11), MeshCore adverts as a Nearby feed (bench).** *(pick)*
- B. Carriers first (F3) with presence later — leaves "who is near" a stub for another cycle. C. Presence only, no gateway — leaves README
  §7's second half unbuilt. D. Everything at once — the radio form needs hardware the owner does not have yet (D23 #4).
- Every radio path stays stub-verified in netns; the bench list (hardware or live net) is in §5.

### F11. Public signed frames: `BROADCAST` 0x30, attestation gossip and labels, "global broadcasts" (the critic's G3)

- **A. Pin the formats in the Layer 7 spec, build the consumers in v1.1.** The spec fixes the `BROADCAST` 0x30 frame (publisher, signature,
  a pointer on the 160 B floor, the object by Tier 2), the attestation gossip rule (the 202 B L2 §10 record rides cards, INTRO and the hub
  handshake — never a LoRa announce `appdata`, where 10 B remain — and the label the UI shows for an attested transport identity), and
  the source of "global broadcasts" (publishers the user follows; a 5B gateway streams only those). Nostr as a **readable-text Adapter**
  for followed publishers (the Layer 6 Adapter shape, which fits readable text; the seams scout's point) is the natural first producer and
  sits in v1.1 beside F3 C. *(pick)*
- B. Build attestation gossip in v1 on the existing handshakes (cheap on `tcp-lan`, nothing on LoRa) and leave 0x30 for v1.1.
- C. Defer all three without pinning the formats — rejected: L6 §18 hands attestation gossip and labels to this layer explicitly, and
  an unpinned 0x30 would be designed twice.

## 3. Detail picks (spec-reviewed by the panel, not polled)

- **"Last heard" is a local fact**: a monotone sequence plus an in-session age from this node's clock at the last verified frame, shown
  with its evidence per source and transport; a peer's own clock stays a labelled hint, never ordering or freshness; no "online" policy
  of ours — show the age, bucket it for display, cite Meshtastic's 2 h window, do not enforce it (01 §4.1; 05 §3.4).
- **What a gateway must never do**: proxy inference or any WAN request for a peer; forward anything unsealed except signed public frames
  the user follows or a bridge the user turned on; relay into a licensed or amateur link without a human per link; send carrier bytes on a
  metered connection past the user's budget; run on a metered link by default. Cat 5 is opt-in; the consent copy names who sees what per
  carrier (a chatmail relay: addresses, sizes, timing; a rendezvous hub: `dest`, sizes, timing); a hashed mailbox tag (BLAKE2b of the
  NodeId under a new L2 domain tag) hides the NodeId from relays that do not know it (02 §4.4, §4.6).
- **5A intermittent gateways flush on a connectivity event and on fabric events** (start the carrier child, push the outbox, fetch the
  mailbox, stop; zero weight while off); **5B permanent gateways hold one connection** (a paired hub or one IMAP IDLE session; ~200 kB a day
  of keepalive); a phone that runs a hub is 5A by default (Doze); a phone that is only a member carries nothing in v1
  (F4 A) (02 §4.3).
- **"Global broadcasts" are signed public frames (`0x30`) from publishers the user follows**; on a 160 B floor a broadcast is a pointer
  (publisher, hash, size, short text) and the object comes over a `Bulk` link (Tier 2, v1.1); broadcasts rank below user messages and wait
  for airtime; shaping into LoRa is physics the adapter advertises (02 §4.5).
- **A carrier forwards sealed packets as opaque bytes, never re-fragments**; batching several packets of one message in one carrier payload
  is glue (02 findings 3, 16). **In v1 the carrier is not a `Link`:** the hub hands forward rows to its carrier child through the hand-off
  API of F4 A (`forward_take`, `forward_done`, `carrier_inbound`, a "forward pending / target up" event over a loopback JSON-lines pipe,
  the LXMF helper's pattern), so `remove_link`, a new `LinkKind`, the openapi enum and `Route::Via` stay untouched (07-gap-2 answer 10);
  the `carrier` `LinkKind` (stream, reliable, bulk, no airtime; `PeerAddr::Opaque`; `Link::send` returning at once) is the v1.1 form if a
  non-hub node must carry its own sends.
- **The L2 §13 hand-offs**: announces are identity plus locator, never routing (the 8 B home-hub id is enough to pick a paired hub; a role
  TLV may follow); Cat-1 relays are relay-blind (the stock repeater cannot verify, 111 writes no relay firmware, verify-before-relay hands an
  attacker a CPU lever; defences sit at the endpoints); no announce rate limit of ours (emission is on demand; receivers are bounded by
  capacity; Reticulum's 2 % cap stays rnsd's default) (03 §4.5).
- **Loop and duplicate suppression, layer by layer**: the repeater's seen-table and hop cap; the hub's `pkt_ref` dedup, kept after
  hand-over (`DUPLICATE` ends a forward); the `fwd` table (never forward what came from a paired hub, except a `leaf` hub's rows onward
  to this hub's carrier); the receiver's prefix and range state. No hop counter on `tcp-lan`; the shim `hop_limit` stays unbuilt until a
  111 transit node floods on a broadcast bearer (03 §4.4; 07-gap-2 answer 6).
- **Runtime "join in sequence, fall back" is not a state machine**: every enabled network is a zero-weight-when-off component, so parallel
  costs nothing and stopping one loses a path; `[order]` is path preference (which network to try first per peer, the rail, agent
  proposals); sequence only exclusive resources (one serial port, a band where sealed frames are forbidden, a phone on battery), as a
  user-consented switch the agent proposes (01 §4.5).
- **Gateway badge, four honest states**: not a gateway · gateway, carrier reachable at last check N min ago · gateway, last flush N min ago
  with K pending · off by choice; never "online"; Settings names the carrier the claim rests on (05 §3.4).
- **Registry rows say which layer answered**: "hint (unverified)", "signed by this contact", "asked just now"; all names and roles are
  untrusted text; the agent's hub for offload comes from the sealed query, never a TXT, and stays on the LAN binding (05 §3.4; fact 22).
- **Metered links**: a user setting in v1 ("treat this network as metered", a daily byte cap; the SSID a user-marked hint); an Android
  helper may read the flag later (05 §3.5).
- **The hub serving surface for updates**: the signed public files on the existing dashboard listener while `hub_role` is on (no third
  port), fetched by sha256; transient RSS during an update equals the tarball size (not resident; recorded when it changes) (04 §4, q1).
- **The Reticulum fleet stays one**: 111 frames flood on MeshCore, Reticulum traffic on RNode; microReticulum_Firmware is a reference only
  unless F4 B ever ships (03 §2).
- **Resident costs the critic asked for (G5):** `_111._tcp` is free only while `mdns-sd` already runs (the dashboard LAN-bound); on a
  hub whose dashboard is not shared it would start the daemon's thread, a UDP 5353 listener and the 5 s IP check (~12 wakeups/min) — so
  the registry record follows the dashboard's LAN switch, never starts the daemon by itself; a paired-hub connection's keepalive is the
  `tcp-lan` 25 s frame (~70 B each, ~240 kB a day); the epoch beacon adds no timer (F8); the WAN probe's bytes per day are unmeasured
  (a TLS handshake per HEAD; several MB a day is an estimate).
- **Sizes and regions (G6):** the airtime tables use a 138 B announce; the real v1 announce is 141 or 149 B plus the 1 B shim, so the
  figures read 3–9 % low; the duty-cycle shares are for the EU 1 % sub-band only — US, AU and other regional dwell and duty rules are
  not stated and are a legal read (rule 7), never a limit of ours.
- **The agent's role in Intent-Based Routing (G7):** read-only tools (`presence.read`, `gateway.read`, or an extended `nearby.read`; both
  pins bumped); it proposes (enable a gateway, pair a hub, set an interval), never acts; a registry hit becomes an offload target only as
  the private IP literal `agent111` requires, taken from the sealed query, never from a TXT.
- **"Layer 7 runs natively on Cat 1" means the stock repeater's own floods and adverts (G8)**: nothing of ours runs there; the Cat-1 C core
  stays deferred (L2 §15, D13 H1). The MeshCore **room server** (an adoptable LoRa store-and-forward for group text) was not assessed:
  a bench item for v1.1, beside MeshCore adverts as a Nearby feed.
- **Deferred from Layer 6 and not researched here (G9):** v1.1 hub-member routing of foreign DMs, and gateway carriage of foreign-network
  text (a gateway carries sealed 111 envelopes only in v1).
- **Bench items** (hardware or live net, not blockers): rnsd RSS as a transport node on a Pi; real public Nostr relay acceptance and limits;
  public chatmail relay behaviour for 30 MiB machine mails; a rustls RPK handshake against OpenSSL 3.2+; ntfy `since=<id>` after the id is evicted and ntfy.sh's
  real retention; yggstack RSS and Termux survival; arm64 Madmail; Termux netlink bind and background
  survival; MeshCore adverts as a Nearby feed; the licensed-band legal read per region.

## 4. Plan corrections this brief records

1. README §7's "nodes continuously announce reachability through heartbeats" is read as F1 A unless the owner says otherwise; the built
   rules it meets are L5 A6 (announces on demand) and the Pi floor (no timer without the owner's say).
2. `receipt.gateway` is always `true` and never read (drop or document); the `[order]` table sorts the display only; `ROLE_LORA_ATTACHED`
   is defined and never set; `mdns-sd` advertises the dashboard, not fabric peers; the phone-relay QR joins rnsd's Backbone, not the fabric.
3. L5 §9/§21 "hub-to-hub replication A+ as per-(dest) summary vectors (§4 form)" cannot be built as worded; L5 §9 "about 90 B" custody wrap
   is 120 B as built.
4. The updater has no `111 update` verb, two hard-wired HTTPS bases and absolute payload URLs; the manifest's `channel` is never compared;
   the keychain chain is parsed, not verified; `min-upgrade-from` lives only in the installer; the `[kb]` pin is parsed and unconsumed.
5. `LICENSING.md:56`: the Reticulum License text is verified now (the helper's own licence stays pending the owner's contact with upstream);
   `bao-tree` is pinned at 0.16 while 0.17.0 shipped on 2026-10-22 (note before the v1.1 Tier 2 build); `minisign-verify` 0.3.0 exists.
6. The Layer 3 README's "offload rides Layer 5's encrypted fabric" describes what is not built (LAN HTTP + pairing, a private IP literal).
7. G7 says nothing about timers or outbound traffic; the required configuration carries ~205 wakeups a minute and one outbound HEAD a
   minute today.
8. D21 (the UI budget) needs an amendment if a map ever ships: an exempt bucket, the fonts precedent.
9. `plan/cross-layer/parked.md` P5 (an own Rust Reticulum if our transport ever depends on Reticulum code) is closed by upstream's own
   statement and by F4 A: no Rust Reticulum in `111d`, ever.
10. Against locked text, every scout agrees (the critic's table; each needs the owner's acknowledgement): D19 b / L5 §9 "A+ follows
    without redesign" (it cannot be built as worded) · L3 README:48 "broadcasts hub role, model, load" (model and load are never in a
    broadcast; the registry answers them by a sealed query) · L1 "KB git-sync is Layer 7" (git is rejected for the node; the KB ships as
    a signed snapshot) · README §6 "join in sequence" (no join state machine; `[order]` is preference) · MeshCore's hop cap is 63 (a
    6-bit `path_length`), not 64.

## 5. Open questions for the owner (beyond the polled forks)

1. Trim the timer baseline (the 2 s Meshtastic and panels ticks with no board bound)? Add a no-timer gate to G7? (F2)
2. Is a recurring contact with `install.111.network` acceptable under "no telemetry" at all, even for the updater's hourly check? (F2)
3. May the first Layer 7 release ship an optional CI-built carrier binary (Nostr, iroh) at all — build-time weight is the owner's call. (F3)
4. Ship a default list of public chatmail relays, or must the user always name the relay (operator terms for machine traffic unverified)? (F3)
5. Is "one inter-hub hop, a star of paired hubs" an acceptable v1 shape, or must v1 route across a chain of hubs (a plaintext hop count,
   an envelope change)? (F4)
6. Hint custody on LoRa (v1.1) makes a hub a passive collector of every sealed frame it overhears, for its members' `dest_hint`s — accept? (F4)
7. May the agent read `rnpath -t -j` hop counts as a Nearby "hops" fact (read-only, no import) before any Reticulum carrier exists? (F4)
8. A hard "no gateway or relay path onto a board marked licensed", or per-link human consent? Needs the regional legal read. (F3, 05 §3.5)
9. The epoch unit, day or week; freeze the beacon format with the layer or at the first consumer? (F8)
10. Phones that lose their hub's address: QR rescan, a `/24` sweep on `:11183`, or wait for an Android app? (F7)
11. KB cadence apart from the binary's (F6 K2) or not; may the Layer 1 trust amendments land before hub distribution is built? (F6)
12. Who signs Layer 8 datasets served from the same hub object store: the release key, the hub owner's key, or the publisher's checksum?
    (F6; note only, Layer 8's question)
13. Map: an exempt bucket plus a Range tile route now, or the Meshtastic panel's map until Layer 8? Radar angle rule acceptable? (F9)
14. Should the required configuration's `gateway` service row survive as the gateway badge's home, or does a Layer 7 "gateway" page
    replace it? (F2, F9)
15. May the carrier prototype use the public ntfy.sh for tests (a third party sees IP, topic and timing), or an owner-run server from
    day one? Is a 12 h default retention acceptable when documented, or must 111 ship a recommended self-host config (`cache-file`,
    `cache-duration`)? (F3)
16. Which gate failures are tolerable for a carrier prototype — ntfy's bus factor and issue response are unread (api.github.com is
    closed from this box), yggstack has no semver tag? (The evidence a carrier leg earns is item 18.) (F3)
17. `tokio-rustls` as a new lockfile entry (build-time weight, one `LICENSING.md` row) or the ~150-line adapter? May a gateway use
    public Yggdrasil peers (stranger-run nodes, egress to them), or only peers the user names? Is the attested link key's new
    `transport_id` (an L2 registry change) in Layer 7's scope, or is pin-from-card enough for v1? (F3)
18. May Layer 7 amend locked Layer 5 with exactly the twelve items of `07-gap-2.md` §3 (F5 A), nothing else? Is `relayed` (reason
    `forwarding`) at the first hub acceptable, or must the state stay `sent` until the dest's hub accepts (a new `CUSTODY_ACK` status and a
    second ACK that is hearsay unless signed)? (F4, F5)
19. The home-hub hint TLV in v1, or probe-only (which drops one amendment and leaks each unknown dest to every paired hub on first send)?
    A number for the Transit quota default? (F4)
20. Must a non-hub phone carry others' items in v1 (then the rendezvous form A′), or is 5A/5B "a hub with internet" (A)? An own-send
    carrier route for a non-hub node (`Route::Carrier`): v1 or v1.1? Sweep-before-ack: v1 or v1.1? (F4)
21. Hub pairing over the public internet before TLS-RPK lands: require a tunnel, or warn (the path sees `dest_hint`, counters and
    sizes)? (F3, F4)

## 6. Critic and gap-fill

**Critic (opus, `research/06-critic.md`): "close to ready for an options brief"** — presence, the heartbeat numbers, the timer baseline,
relay inside the fabric, updates and the KB, and the UI floor are covered with file:line evidence and verified licences; the heartbeat
tension is quantified and left to the owner. Two blocking gaps: **G1** the global carrier sweep did not apply the KB's adopt-vs-build gate
and missed ntfy, Yggdrasil/yggstack and an adopted tunnel, and picked `snow` without seeing that `rustls` already does RFC 7250
(settled by the critic, F3); **G2** three reports proposed three cross-hub custody shapes and none said how a hub learns a non-member
`dest`'s hub (F4). Majors: **G3** public signed frames had no options (F11); **G4** the epoch option E-A did not say what advances a
hub's counter (F8, restated). Minors G5–G9 are folded into §3. Six cross-report conflicts are resolved in F2, F3, F4, F7 and §4 item
10. The critic's own checks settled: `rustls` 0.23.45 RPK on both sides; `mdns-sd` 0.21.5's 5 s IP check and that our daemon starts it
only when the dashboard is LAN-bound; ntfy's dual licence and defaults; yggstack's licence; Termux at SDK 28 (Android 17's local-network
permission does not bind it); the UI lazy headroom 36.1 KB.

**Gap-fill (two sonnet scouts, G1 and G2):** `research/07-gap-1.md` re-scored lens 2's carriers with the KB gates and added ntfy,
Yggdrasil/yggstack, WireGuard and Headscale/Tailscale, costed TLS-RPK against `snow` and a tunnel from the local rustls source, and
found that no candidate can pass `testing` from desk research (the rule says prototype the best, not build) — folded into F3, facts 23
and 24, §3's bench list and §5 items 15–17 and 21 (bus factor, issue response and commit counts are unread for every candidate;
wireguard-go's licence, yggstack's tags and every RSS figure are unverified). `research/07-gap-2.md` reconciled the three cross-hub
shapes into single-copy push hand-over with a `fwd` column (no `Dir::Transit`, no A+ pull, no cursor), answered how a hub learns a
non-member dest's hub (the sender's 8 B hint as ext TLV `0x02`, else a probe of paired hubs), sized the bytes and the resident cost,
and listed the twelve additive amendments — folded into F4 A, F5 A, fact 8 and §5 items 18–21. One tension between the two is resolved
here: 07-gap-1 wants the carrier "behind a `carrier` link-kind seam", 07-gap-2 shows the hand-off API needs no `Link` change at all;
v1 takes the hand-off (the carrier is never a `Link`), and the `carrier` `LinkKind` is the v1.1 form for an own-send route on a
non-hub node. 07-gap-2's "a VPN until Layer 6's online Noise IK" reads as "until F3's TLS-RPK" (§5 item 21).

**Unverified claims (the lead verifies on the owner's machine; the critic's §6 is the list):** denied hosts or search snippets only
(Roughtime RFC 10049, CT gossip RFC 9162, RFC 6762/6763 TXT limits, RFC 6693 constants, Briar BSP, Arti onion-service status, MeshCore
PR 650 cadences, NsdManager MulticastLock, Termux issue 536, Android NetworkCapabilities and Data Saver, the AOSP route-netlink
restriction, 47 CFR 97.113(a)(4) and other regions' amateur rules, F-Droid Nearby, apt `Valid-Until`, the MeshChatX map, PMTiles /
Organic Maps / Kiwix sizes, Tuwunel RAM) · live services (public Nostr relay limits, chatmail operators' terms for machine traffic,
ntfy.sh anonymous limits, the public LXMF propagation set, whether `I2PInterface` needs an external router) · licences not read from the
upstream file (chatmail relay, Madmail limits, dtn7-rs, Leviculum, batman-adv, bluer, blake3 / bao-tree / gix registry records, the TUF
spec, libkiwix, libzim, snow) · measurements (musl and RSS deltas for `nostr-sdk`, iroh, snow, `tokio-rustls`; rnsd RSS as a transport
node on a Pi; rust-nostr and babeld RSS; the real payload tarball size; WAN-probe bytes per day; the mdns-sd thread's RSS; per-crate
test counts after Layer 6; the Meshtastic monitor tick with no board; PoW at 2 MH/s; the Reticulum 275 B and ~300 B arithmetic;
airtime and phone energy on real hardware) · device-gated (netlink groups under SDK 28, held sockets under Doze and the phantom-process
killer, yggstack and an LXMF host on Termux) · bench or legal (stock MeshCore floods of our `FF00–FFFF` `GRP_DATA`, the KISS modem, real
LoRa rate shaping, Ed25519 verify time on Cat 1 hardware, duty-cycle and dwell rules per region).
