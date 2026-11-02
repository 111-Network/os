# Layer 7 - Relay & Presence: spec v0.2 (draft for the re-check)

The layer that makes reachability honest and lets delivery cross islands. **Presence** is event-driven: a node shows who it heard, over which transport, how
long ago by its own clock, and from what evidence. **Hubs pair** with other hubs and hand custody of sealed items to each other, one copy at a time. A hub
with internet and an owner-enabled **carrier** becomes a gateway (Category 5: 5A intermittent, 5B permanent). A hub can be the **update and KB origin** for
nodes that never see the internet. The layer pins the **mesh epoch** that Layer 2's expiries count in, and the **public signed frame** formats. It adds no
routing protocol, no resident process and no timer to a node that is not a hub, and nothing leaves a node unsealed except what the user publishes or
bridges.

Status: ✍️ **v0.2, draft (2026-11-02)**, v0.1 (written from D30, the owner took every pick and every default of the options brief) amended after the
three-judge panel by the lead's triage (`plan/layers/7-relay/review/triage.md`) and D31. Nothing here is built. Next: one re-check
(`plan/layers/7-relay/review/recheck.md`), the lock (D32), `7-relay/TASKS.md`. The six owner forks of D31 are applied as the lead recommends (§19
"Owner forks"); a different answer lands as v0.3.

## 0. Reading guide

Reads with: `os/README.md` §7 and "Category 5" · `os/5-messaging/README.md` §3, §6.2, §7.3, §8.1, §9, §10.3–§10.4, §12, §21 (the locked fabric this layer
amends, exactly twelve times) · `os/2-identity/README.md` §6.1, §6.6, §9, §10, §13, A6, A8, A10 · `os/1-bootstrap/README.md` "Updates & rollback",
"Security & trust model", "Seams for later layers" · `os/6-transport/README.md` §2.1, §11, §15, §18 · `os/3-intelligence/README.md` "Offload" ·
`os/4-interface/api/README.md` · `os/plan/DECISIONS.md` **D30** (the law), **D31** (the amendments after the panel), D29, D24–D27 (the lock precedents) ·
the options brief (OB) and the research reports R00 seam map, R01 presence, R02 gateways, R03 relay, R04 updates, R05 UI and floor, R06 critic, R07g1
carriers and link security, R07g2 custody, all under `os/plan/layers/7-relay/`.

**Citations.** `M/` = `5-messaging/crates/mesh111/src` · `N/` = `1-bootstrap/crates/node111/src/daemon` · `NI/` = `1-bootstrap/crates/node111/src` · `ID/` =
`2-identity/crates/id111/src` · `A/` = `3-intelligence/crates/agent111` · `UI/` = `4-interface/ui/src` · `API` = `4-interface/api/openapi.yaml` · `L1`…`L6`
= the layer READMEs · `RS/` = the rustls 0.23.45 registry source · `MD/` = the mdns-sd 0.21.5 registry source · `LC/` = libc 0.2.190
`src/unix/linux_like/linux/mod.rs`. Every `file:line` was re-read in the working tree on 2026-11-01 (head `ec9954b`). mesh111 is unchanged since R00's base
`3bf545b`; the node111 files touched after it (`dashboard/mod.rs`, `dashboard/routes.rs`, `devices/registry.rs`, `components/mod.rs`, `bridge/lxmf.rs`) and
`openapi.yaml` were re-pinned. **Evidence:** *(src)* source read · *(doc)* docs read by a scout · *(design)* not built · *(unverified from this box)* the
R06 §6 list: this box's network policy denies most documentation sites and every live service. The citations v0.2 adds were re-read on 2026-11-02 (head
`0e9985f`; no file under the cited code trees, `LICENSING.md` or `openapi.yaml` changed since `ec9954b`).

**Finding ids.** An id in a v0.2 text (C1–C18, S1–S27, I1–I16) names the panel finding it answers (`plan/layers/7-relay/review/custody.md`, `carriers.md`,
`integration.md`; dispositions in `triage.md`). `§19 Cn` names this spec's own code corrections; inside §19, a bare `Cn` is §19's and a panel finding is
written "panel Cn".

**Normative:** numbered rules, byte tables, settings with defaults, gate assertions. **Informative:** "why" sentences, arithmetic, cost tables, §16–§19.

**v1 and v1.1** (D30 F10):

| Area | v1 (this release) | v1.1 (recorded, §17) |
|---|---|---|
| Presence (§2) | event-derived `111` Nearby kind; the per-transport heartbeat switch, visible and inert (§2.4), and its bans | heartbeats on LoRa transports once a sealed LoRa link exists; keepalive-only presence |
| Gateway (§3) | event-driven carrier test; the 60 s probe removed; three 2 s ticks trimmed; the no-timer gate | Android metered flag |
| Hub links (§4) | TLS 1.3 RPK for non-LAN hub-to-hub; TOFU pin from the pairing bundle | attested link key (`transport_id` `0x04`) |
| Custody (§5) | single-copy push between paired hubs and through a carrier; the twelve Layer 5 amendments | the radio form (hint custody), chains, signed forward receipts, member-gateways, sweep-before-ack, multi-copy |
| Carriers (§6) | ntfy prototype in `111d`; chatmail glue after the ntfy smoke log | the JSON-lines pipe, `carrier` `LinkKind`, Nostr, LXMF propagation, iroh, yggstack |
| Registry (§7) | DNS-SD `_111._tcp`; role bits 3–5; members-only gateway facts | the sealed registry query (two control kinds pinned in §7.3) |
| Epoch (§8) | beacon bytes, unit (day), advance rule, carriage: pinned only | built with its first consumer, freeze protection for updates |
| Updates (§9) | hub origin; `111 update --from`; the Layer 1 trust amendments | Tier 2 transfer; a release tag in announces |
| Nearby (§10) | SVG radar | the map bucket (D21 amendment), Leaflet + PMTiles |
| Public frames (§11) | `BROADCAST` 0x30 bytes, attestation gossip rule, UI label: pinned | the consumers; a Nostr readable-text Adapter |

**Relation to D30, D31 and the brief.** D30 is the law: every fork pick F1–F11 and every default Q1–Q21; D31 records the amendments after the panel and the
six owner forks, applied here as the lead recommends. The brief's §3 detail picks are the lead's and are open to the panel. Where the brief or a report
disagrees with the code, the code wins and §19 records the correction. Where the code forces a departure from D30 itself, §19 "Deviations" names it for the
owner. Section numbers in R07g2's replacement texts ("Layer 7 section 9a") read as §5 here.

## 1. Scope & non-goals

**What Layer 7 is per device category** (README layer map, row 7):

| Category | Layer 7 there |
|---|---|
| Cat 1 (the stock MeshCore repeater, D19 c) | the repeater's own floods, seen-table, 63-hop cap and adverts. Nothing of ours runs there; it is relay-blind and is asked for nothing (no presence, no epoch, no update). |
| Cat 2A, 2B, 3 (phones, laptops, terminals) | presence from events, the `111` Nearby kind, role bits the owner chooses, `111 update --from`. Nothing resident. |
| Cat 4 hubs | the above plus pairing, forwarding, the gateway role with a carrier, the update origin, the `_111._tcp` record while LAN sharing is on. Every one is a switch, off by default. |
| Cat 5 (overlay) | in v1 a gateway is a hub with internet and a carrier its owner enabled (D30 F4, Q20). A phone that is only a member carries nothing for others. |

**The Raspberry Pi floor (hard rule, AGENTS.md).**
1. A node with nothing configured is byte-identical on the wire to a Layer 6 node, with the removals D30 decided: no HTTPS `HEAD` every 60 s (§3.5), no 2 s
   ticks while no board is bound (§3.6) and, where LAN sharing runs mdns-sd and the second netlink bind succeeds, address events instead of its 5 s IP check
   (§3.3). Listeners, processes and stored bytes are unchanged.
2. Nothing in Layer 7 adds a resident process, a listener, a timer, a periodic poll, an open file or a runtime dependency to the required configuration. The
   one new lockfile entry (`tokio-rustls`, §4.5) is build-time weight; its runtime cost is zero until a hub pairs with a non-LAN hub.
3. Hub-only costs are enumerated, each behind its own switch, each measured at build and recorded in `7-relay/TASKS.md` (§16): one held connection per
   paired hub; a carrier's flush (5A) or one held HTTPS stream on one named thread (5B); a second netlink socket while a carrier or mdns-sd needs it; the
   DNS-SD record inside a daemon that already runs; the origin's files on disk.
4. A Termux hub with a carrier may poll coarsely only while its netlink bind fails (§3.4). Nothing else polls.

**Hard rules carried in:** adopt before you build (ntfy through the client `111d` has, rustls already in the lock, the KB's prototype rule) · no wall clock
in a cryptographic, ordering or freshness decision · no telemetry, no contact with 111's servers for a liveness fact (D30 F2) · the updater's hourly check
stays, visible and user-off (D30 Q2) · no policy limits of ours (README §5) · `111d` imports nothing of Reticulum (`LICENSING.md` §1) · the agent proposes
and a human decides (`6-transport/README.md` §13) · every new setting is off by default.

**Non-goals:** a global 111 swarm, directory or address space (R02 §1) · a routing protocol of ours (the repeater floods; Reticulum routes its own traffic;
hubs form a star) · relay firmware of any kind (D19 c, L2 §15) · a Rust Reticulum (closes P5, §19) · any request to 111's own servers to learn whether a
node is online · IP mesh routing (babeld, Yggdrasil as a router; R03 §4.1 option 4) · a join-in-sequence state machine (`[order]` is path preference, §19) ·
gateway carriage of foreign-network text (a gateway carries sealed 111 frames only).

## 2. Presence (D30 F1)

### 2.1 The evidence model (normative)

1. **Last heard is a local fact.** A presence fact is `(peer, transport, evidence, local_seq, heard_at)`: `local_seq` is the store sequence of the event
   that proved it (monotone, survives restarts), `heard_at` a monotonic `Instant` of this node's own clock, kept in memory for this session only. The UI
   shows the age `now − heard_at`; after a restart a row returns with its next fact.
2. **A peer's clock is never evidence.** `last_heard_hint` stays a labelled display hint (`API:1109`); Meshtastic's 2 h "online" window is cited in the
   copy, never enforced. There is no "online" state of ours: the age is bucketed for display only.
3. **Per source and per transport.** A 111 contact heard on the LAN and through its hub has two facts: `tcp-lan` and `hub`. A Meshtastic or Reticulum node
   stays in its own source and is never merged into a 111 row (a different identity system, `API:1099`).
4. **Built from the fabric's event stream plus one read-only accessor; no Layer 5 spec change beyond the twelve.** The daemon derives facts from
   `FabricEvent::Message` and `::Contact` (`M/fabric.rs:418-451`) and the hub's reachability in `FabricStatus` (`M/fabric.rs:675-688`; `hub_answered`,
   `:1802-1806`). The event carries no route (`Message {id, peer, change}`, `:418-428`; `MessageChange` is `New`/`State`/`Request`, `:380-388`),
   `LinkView.peers` is a count (`:586-597`) and `HubView` knows only the home hub (`:601-616`), so the transport comes from a new read-only
   `Fabric::attributed_on(peer) -> Option<LinkKind>`: the kind of the link whose connection is attributed to `peer` (`Conn.attributed`, `:1148`), or
   `None`. It is a code seam beside the twelve, read-only and byte-identical (§19 "Code seams"; D31 (2)); it is smaller than a `via` on the `Message` view,
   which would need the store to keep each message's path. **Honest limit:** it reads the attribution when the daemon handles the event, not the message's own
   path, so a message that came through the hub while a direct connection to its sender was also attributed is counted as `tcp-lan`. The persisted
   `contacts.last_seen_seq` moves only on a contact upsert (`M/store/rows.rs:685-688`), and the in-memory `heard` set (`M/fabric.rs:1317`) is not exposed;
   both stay as they are (§19 C3).

### 2.2 What bumps last heard (normative)

| Event (daemon side) | Fact | Evidence string |
|---|---|---|
| an inbound message from a contact (`Message`, `dir = in`) | `111` row; transport = `attributed_on(peer)`: `tcp-lan` when a connection is attributed to them, else `hub` ("through your hub") | "a message from them arrived (verified)" |
| an own message turns `delivered` | `111` row; transport by the same rule | "they confirmed your message" |
| the home hub answered on its connection (`hub.reachable`) | the hub's row | "your hub answered" |
| a paired hub answered (a `CUSTODY_ACK` or a forward put, §5.9) | that hub's row | "paired hub answered" |
| an own message turns `relayed` | nothing about the peer | the message shows "your hub holds it"; the peer's row is not refreshed |
| a card, an announce, a hello, a keepalive | nothing | a stored announce is accepted with no state change and proves no liveness (L2 A6; `2-identity/README.md:348-351`) |

### 2.3 The `111` Nearby kind (normative)

- `NearbySource.kind` gains `111` (closed set today: `kind_for`, `N/dashboard/api_v1/nearby.rs:54-56`; `API:1118`). One source, label "111 nodes heard by
  this node", `device_id` null. A node row is created only for a contact with a fact this session; `id` = the NodeId (64 hex), `num` = 0, `long_name` = the
  local alias or the L2 label, `short_name` = the label, `last_heard_hint` = null.
- Additive optional `NearbyNode` fields: `heard_ago_s` (integer or null, from `heard_at`, display only), `heard_seq` (the `local_seq`), `evidence`
  (`[{transport, text}]`, the strings above; `transport` is `tcp-lan` or `hub`, §2.2), `roles` (the contact's signed role names).
- The `111` source rides the same sink and the same cap as the bridge sources: at most 512 rows (`MAX_BRIDGE_NODES`, `N/dashboard/api_v1/nearby.rs:41`), the
  oldest heard forgotten first (`:129`). It publishes the existing SSE `nearby` frame on a change (`API:1170`). No timer refreshes ages: the UI computes them
  from `heard_ago_s` and its own clock between frames.

### 2.4 The per-transport heartbeat switch (normative)

1. `presence.heartbeat.<transport>` = an interval in minutes; **`0` = events only, the default for every transport**; the LXMF helper's precedent
   (`6-transport/README.md:334`). A beat re-mints the node's announce (a fresh counter value, committed with an fsync before it is sent,
   `M/sealer.rs:53-54`) and sends it on that transport only.
2. **Visible and inert in v1 (I4; §19 D15).** No transport takes a beat in v1: `tcp-lan` needs none (the 25 s keepalive already proves the link,
   `M/link/tcp_lan.rs:125`) and the LoRa transports have no sealed link yet. Every write to `presence.heartbeat.<transport>` (a `PATCH /relay/settings` or
   an accepted proposal) answers `409 no_such_transport`, except `rns`, which answers `409 settings_locked` with a `detail` that names
   `PATCH /api/v1/bridge/settings`: the LXMF helper's announce interval (default 60 min) is Layer 6's setting and is shown here, never written through.
   The proposal action `presence.interval` is refused `unsupported_action` in v1 (§13.2). The switch is visible until a sealed LoRa link exists (v1.1).
3. **Bans (the validator, built in v1, reached over HTTP from v1.1):** after the transport check, never on a duty-cycle-limited band (the region table says
   so) or a board marked licensed (`409 licensed_board`); on a phone (Termux category) never below 9 min (`400 bad_request`; Doze fires exact alarms at most
   once per nine minutes, R01 §3 finding 9). Because step 2 refuses every v1 write first, node111 unit tests call the validator with a transport the test
   marks offered, a phone category and a licensed board (§15 G8).
4. The switch is a switch in the Layer 6 §2.1 rule-2 sense (zero weight while off), not a §2.1 component: no `assets/components/` entry, no payload, its
   setting in `state/relay.json` (§6.3; `6-transport/README.md:101-125`). Off means no timer, no task, no counter use.

### 2.5 Announces are unchanged

Announces stay on demand: in a card, on link-up, inside an INTRO, on request (L5 §7; L2 A6). The `tcp-lan` handshake sends the stored bytes first
(`M/link/tcp_lan.rs:10-18`); the daemon re-mints only when `appdata` changes (`N/fabric/mod.rs:912-925`). Layer 7 sets no announce rate limit (R03 §4.5).

### 2.6 Why: airtime, not CPU

One 138 B announce every 15 min is 0.025 % duty at SF7 and 0.146 % at SF10 per node; **50 nodes at 15 min on SF10 spend 7.3 % of the channel on "I am
here"**, at 3 h 0.61 %; LongFast 50 nodes at 15 min 6.8 % (R01 §3.1; R05 §3.2). The real announce is 141 or 149 B plus the shim, so these read 3–9 % low,
and the duty shares are for the EU 1 % sub-band only (OB §3 G6). The adopted networks already beat: Meshtastic NodeInfo 3 h and position 1 h, MeshCore flood
adverts 47 h (R01 §3 findings 3–4). On the host a 15 min beat adds 0.067 wakeups a minute (R05 §3.2): a spectrum cost. README §7's sentence is reworded
(§19).

## 3. The gateway role and the carrier test (D30 F2)

### 3.1 5A and 5B (normative)

- A **gateway** is a hub (`hub_role` on, `M/fabric.rs:356-360`) with `gateway.enabled` and either a configured carrier (§6) or a paired hub reached over the
  internet (§4). Hub-only in v1.
- **5A intermittent** (the default, and the only mode a phone hub offers): no connection between flushes. A flush runs on an event (§6.2) and ends.
- **5B permanent** (opt-in, `carrier.mode = "5B"`): one held connection, either the ntfy subscription stream (§6.2) or the paired hub's `tcp-lan`
  connection, which exists anyway.

### 3.2 "Internet" (normative)

"Internet" means **the owner's chosen carrier endpoint answered at its last check**: the ntfy server answered the check, or a paired non-LAN hub's
connection is attributed. For ntfy **the check is the drain** `GET <server>/<topic(peer→me)>/json?poll=1&since=<last id>` (§6.2), one request: a check that
returns frames is the first step of a flush, so a netlink change costs one request, never a check and then a flush. A check runs on (a) a netlink address
or route change, (b) every carrier attempt (a flush, a held stream's reconnect), (c) a user's "Check now". It never contacts `install.111.network`. The
installer's one-time probe (`NI/install/preflight.rs:14-15`) is unchanged.

### 3.3 Netlink address and route groups (normative)

- Today the spine subscribes to `RTMGRP_LINK` only (`N/transport/netlink.rs:18`, bound at `:42-49`) and debounces 2 s into the rnsd re-render
  (`N/transport/mod.rs:498-517`).
- `NetlinkEvents::new` takes the group mask as an argument (a Layer 6 file change). The spine keeps `RTMGRP_LINK`. A **second** socket with `RTMGRP_LINK |
  RTMGRP_IPV4_IFADDR | RTMGRP_IPV4_ROUTE | RTMGRP_IPV6_IFADDR | RTMGRP_IPV6_ROUTE` (`0x1 | 0x10 | 0x40 | 0x100 | 0x400`, `LC/:2321-2331`) is opened **only
  while a consumer is on**: a configured carrier, or the mdns-sd daemon (running only while LAN sharing is on, `N/dashboard/mod.rs:259`). It closes when the
  last consumer goes. A node with neither has the Layer 6 socket set exactly.
- Its events go through **a second `netlink::debounced` instance** (2 s) that feeds only the carrier check and the mdns-sd re-arm; the spine's instance,
  which calls `spine.on_link_change` (`N/transport/mod.rs:498-517`), is untouched, so a DHCP renew never re-renders rnsd (S15; §19 D8). A burst of route
  updates is one check. A failed bind is logged once; for a carrier it selects §3.4, for mdns-sd it keeps the default check (below).
- **mdns-sd's own 5 s IP check** (`MD/service_daemon.rs:69`) stays at its default until the second netlink socket is bound; only then is it set to 0
  (`set_ip_check_interval`, `:696-701`). On a bind failure the 5 s check stays and the failure is logged once, so the `111.local` and `_111._tcp` records
  never freeze at the start-time addresses (S3).
- **The re-arm (pinned procedure, S2).** The daemon has no "check now": `process_set_option` schedules the next check at now + interval (`:1705-1708`) and
  `schedule_ip_check` re-arms it after every run while the interval is non-zero, and cancels a pending one at 0 (`:1844-1850`). So, on a debounced address
  event: `set_ip_check_interval(1)`, one one-shot `tokio::time::sleep` of 1,500 ms (never an interval), then `set_ip_check_interval(0)`, all inside the one
  task that owns the mdns handle, so two events serialise and no set-0 comes early. G8 asserts `get_ip_check_interval()` (`:704`) reads 0 within 2 s of every
  event and that the record's A records changed (§15). The cleaner form (unregister and re-register the `ServiceInfo` with `enable_addr_auto`, the interval
  left at 0) is a build bench item.

### 3.4 The Termux fallback

A Termux hub with a carrier whose netlink bind fails (route-netlink under SDK 28: *unverified from this box*, R05 §3.5 item 5) checks its carrier on fabric
events and on a coarse poll, `gateway.termux_poll_min`, default 15 (above the 9 min Doze floor; measured at build; a default; the owner may change). No
other node polls.

### 3.5 The probe is removed; the `gateway` row stays as the badge's home (D30 Q14)

1. The WAN loop (`N/services/probe.rs:548-557`, interval `WAN_INTERVAL` 60 s at `:46`, `wan_probe` at `:259`) is deleted. `wan_probe_urls` stays for the
   installer's `NODE111_PROBE_204` knob (`:243-256`).
2. The service `gateway` keeps its id (`API:602`). Its facts come from §3.2. Codes (additive to the emitted set, `API:614`; `wan_ok`, `wan_offline`,
   `wan_unknown` retire): `gateway_none` (no gateway configured, state `unset`) · `carrier_ok` (`live`) · `carrier_pending` (`unset`: the last check failed
   or items wait) · `gateway_off` (`unset`: off by choice).
3. `detail` (replaces `{online, probe_url, checked_at, error, interval_sec}`, `N/services/mod.rs:1044-1074`): `{carrier: "ntfy"|"paired-hub"|null, endpoint:
   host or hub label, mode: "5A"|"5B"|null, checked_ago_s, last_flush_ago_s, pending, error}`. Ages come from a monotonic `Instant`; no wall-clock
   `checked_at` (today an RFC 3339 string, `probe.rs:271`).
4. The catalogue copy (`N/services/mod.rs:340-348`, "the node checks once a minute") becomes "Whether this hub carries messages to other islands, and
   whether its carrier answered at the last check. Nothing is checked unless you turn a gateway on." Its `requires` copy ("An internet connection: Wi-Fi,
   Ethernet or a phone hotspot.", `:344`) becomes "A carrier you configured (Settings → Layer 7)". The UI mock fixture (`UI/lib/api/mock/fixtures.ts:372-378`:
   `wan_ok`, `interval_sec`, `last_probe`) moves to the new codes and `detail` (S18).

### 3.6 The timer baseline trim (D30 Q1)

| Tick | file:line | Period | Runs from v1 only while |
|---|---|---|---|
| Meshtastic monitor | `N/meshtastic/mod.rs:186`, loop `:648-660`, started `N/mod.rs:1072-1074` | 2 s | a Meshtastic binding exists in `interfaces.toml` |
| Meshtastic panel revocations | `N/meshtastic/mod.rs:488-505`, started `N/mod.rs:1036` | 2 s | a Meshtastic panel session is open |
| panels reconcile | `N/panels/mod.rs:62`, loop `:652-660`, started `N/mod.rs:1034` | 2 s | a declared panel's service is set up |

Each parks on a `Notify`. The services registry exposes `snapshot()` only (`N/services/mod.rs:1484`), so it gains a state-change `Notify`; the Meshtastic
monitor parks on the devices module's existing `kick` (`N/devices/mod.rs:352`), the panel ticks on the services registry's new one (S25). With no board
bound the three cost nothing: 90 of ≈ 205 wakeups a minute, and with the WAN loop gone ≈ 114 remain (arithmetic on R00 §3.1 (a); measured at build). The
other ticks are untouched; trimming them is not decided.

### 3.7 The no-timer gate (G7 extended; normative method)

`7-relay/verify/idle.sh`, run by the lead beside `6-transport/verify/zero-weight.sh`:
1. Start `111d` in a fresh netns (loopback plus a veth with no default route), every Layer 7 switch off, `updates.check_internet = false`, no browser, no
   SSE client, no peer. Wait 30 s.
2. **Wakeups, nothing attached (window 1, 60 s):** sum `voluntary_ctxt_switches` over `/proc/<pid>/task/*/status` of `111d` and `111-agent` at t = 0 and
   t = 60 s. The difference is the idle wakeups per minute. No tracer runs in this window: `strace -f` ptrace-stops every thread it traces and adds the
   context switches it would count (S16, I11).
3. **Outbound (window 2, 60 s):** packets leaving the veth (the Layer 6 AF_PACKET logger, `6-transport/verify/egress-log.py`, the primary) plus sampling of
   `/proc/<pid>/net/tcp` and `tcp6` (or `ss -tnp`) for non-loopback sockets. **Attribution (window 3, optional):** only when window 2 counts a non-zero
   outbound, `strace -f -e trace=connect` names the caller. `strace` is a listed prerequisite with the skip rule (absent ⇒ window 3 `SKIPPED`; it is not in
   the Pi image).
4. **Pass line:** outbound = 0; wakeups ≤ 1.10 × the post-trim baseline the first lead run records in `7-relay/TASKS.md` (a default; the owner may change).
   A later commit that raises the measured rate above that line fails G7 until the owner accepts a new baseline. With `updates.check_internet = true` the
   only non-loopback `connect()` targets are the two manifest bases (`N/updater/fetch.rs:18-20`).

### 3.8 The gateway badge (four states, never "online")

"Not a gateway" · "Gateway: carrier reachable at last check, N min ago" · "Gateway: last flush N min ago, K waiting" · "Gateway off (by you)". Settings
names the carrier the claim rests on (R05 §3.4 item 3). Shown on Overview and on the `gateway` service row. The badge is the hub's own; it is never a
member's view of its hub: a member learns its hub's gateway state only as §7.5 says (I5; §19 D16).

## 4. Hub-to-hub over the internet: TLS 1.3 raw public keys (D30 F3)

### 4.1 Which links (normative)

1. A paired-hub connection (§5.1) whose address is not private **MUST** run inside TLS 1.3 with raw public keys (RFC 7250) on both sides. Private = RFC
   1918, RFC 6598 (`100.64.0.0/10`), ULA `fc00::/7`, link-local; loopback is never a peer (`M/link/tcp_lan.rs:127`). The owner may mark a pair `lan = true`
   (a LAN or an adopted tunnel); then it runs plain, as today.
2. **Inbound** on `:11183`: the adapter peeks the first byte. `0x16` (a TLS handshake record) goes to the TLS path, and the client MUST present a pinned
   link key or the connection closes before any 111 byte; `0x31` (the `111F` hello, `M/link/tcp_lan.rs:4`) is served plain, as today.
3. Member-to-hub links are not wrapped in v1 (L5 §8.1 unchanged); D30 asks TLS for hub-to-hub only.
4. **Outbound** plain to a non-private paired hub is refused unless the owner accepted the §4.6 warning for that pair (`consent.action = pair_plain`, §13.1).

### 4.2 The link key (normative)

- A fresh Ed25519 key per node, drawn from the OS CSPRNG at the first pairing, stored in `secrets/link-key` (0600, inside the 0700 `secrets/` directory
  Layer 6 created, `6-transport/README.md` §2). **Never seed-derived, never the identity key, never on any API, never in a backup**
  (`2-identity/README.md:372-373`; L5 §8, "Link keys are random"). After a restore the node mints a new key and each paired hub re-pins it from a fresh
  bundle.
- Carried as its SubjectPublicKeyInfo (Ed25519, 44 B DER) in the pairing bundle (§4.6), base32. The far hub pins it **TOFU from the bundle** and compares it
  byte-exact on every handshake. D30 F3 says "pinned from the hub card"; a 44 B SPKI cannot ride the card (`appdata` ≤ 32 B, `ID/announce.rs:40-49`), so the
  bundle carries it (§19 C9, D14; S21).
- **Attested later (D30 Q17):** `transport_id` `0x04` = `111-link-ed25519` is reserved in the L2 registry here (unassigned bytes fail closed today,
  `ID/attest.rs:205-212`). The §10 record (`ID/attest.rs:6-10`) then binds the link key to the NodeId by both signatures and replaces TOFU (v1.1).

### 4.3 The rustls facts *(src)*

- rustls 0.23.45 with `ring` is in the lock (`1-bootstrap/Cargo.lock:2003-2013`; `ring` at `:1876`; no `aws-lc-rs`).
- RFC 7250 on both sides: `only_raw_public_keys` on the client resolver (`RS/client/client_conn.rs:127`) and the server resolver
  (`RS/server/server_conn.rs:132`); `requires_raw_public_keys` on both verifier traits (`RS/verify.rs:143`, `:272`); the signature check
  `verify_tls13_signature_with_raw_key` (`RS/webpki/verify.rs:213`); Ed25519 signing through `any_eddsa_type` (`RS/crypto/ring/sign.rs:73`) and its SPKI
  from `public_key` (`:393`). R07g1 §3 items 9–10 read the same lines.
- Configuration (normative): TLS 1.3 only; Ed25519 only; client authentication mandatory; each verifier compares the presented SPKI byte-exact with the
  pin, then calls `verify_tls13_signature_with_raw_key`; no X.509, no CA. No SNI is sent (the name is an IP literal, and RFC 6066 §3 forbids
  an IP literal in SNI) and no name is checked; only the SPKI pin binds the peer (S9). Estimated ~100 lines of glue (R07g1 §3b). node111's direct `rustls`
  dependency keeps default features off and the `ring` provider, as ureq's does (`logging`, `std`, `tls12`), so no second crypto provider enters the lock.

### 4.4 The `tcp-lan` stream seam (a code seam beside the twelve: §19 D1, "Code seams"; D31 (1))

- `serve` takes a concrete `TcpStream` and splits it with `into_split` (`M/link/tcp_lan.rs:422-432`); accept and dial live inside the adapter (`:333-336`,
  `:658-700`), and no public call takes an outside stream (`:121-414`). TLS cannot wrap `tcp-lan` from node111 alone.
- The seam: `TcpLanConfig.wrap: Option<Arc<dyn StreamWrap>>`, default `None` (byte-identical); `StreamWrap::outbound(addr, expected, TcpStream)` returns a
  boxed `AsyncRead + AsyncWrite`, and `::inbound(remote, TcpStream)` returns `(stream, Option<NodeId>)`, the second the pair whose pinned SPKI the TLS peer
  presented (S7); `serve` becomes generic, splits with `tokio::io::split`, and refuses a first announce whose NodeId differs from the matched pair, ending
  the connection before any other 111 byte rather than waiting for `pair_mac` attribution. mesh111 gains no crypto dependency: node111 implements
  `StreamWrap` with `tokio-rustls`.
- **Status (D31 (1)).** A type generalisation of `serve`'s stream parameter in `M/link/tcp_lan.rs`: no wire byte changes and no Layer 5 spec text (next
  bullet); recorded as a code seam (§19), not a thirteenth spec amendment. **Flip, if the owner says so:** internet pairing is tunnel-only in v1
  (every non-private pair `lan = true` inside an adopted tunnel, §6.6), the seam and `tokio-rustls` move to v1.1 with TLS-RPK, and §4.1 rule 1 then refuses
  a non-private pair without a tunnel.
- It builds L5 §8's reserved "online links (Noise IK / TLS-RPK, L2 §6.6)" row (`5-messaging/README.md:366`; deferred in §21) inside `tcp-lan` rather than as
  a new `LinkKind`. No Layer 5 text changes for it (D31 (1)): the `tcp-lan` row's "No Noise/TLS in v1" (`:361`) stays true of member-to-hub links (§4.1
  rule 3), and the reserved row is the reference for paired-hub links. D30 F5 did not list it.

### 4.5 `tokio-rustls`: the one new lockfile entry (D30 Q17)

`tokio-rustls` 0.26.6 (MIT OR Apache-2.0; crates.io, R07g1 §3 item 10; absent from the lock today), default features off, the `ring` provider only *(the
crate's feature set is unverified from this box: it is not in the local registry)*. rustls's own default features pull `aws_lc_rs` and
`prefer-post-quantum` (`RS/Cargo.toml:70-76`); both stay off, and G11 asserts that `cargo tree -e features` shows no `aws-lc-rs` and no second crypto
provider (S10). Build-time weight only: the binary-size delta for both musl targets is measured at build. `LICENSING.md` gains one row: "`tokio-rustls` (MIT
OR Apache-2.0): TLS 1.3 stream adapter linked into `111d` for paired-hub links (Layer 7 §4); permissive, GPL-3.0-compatible; no runtime weight until a hub
pairs with a non-LAN hub.", and its §5 counts move (`MIT OR Apache-2.0` 121 → 122, `LICENSING.md:103`; 187 → 188 distinct crate versions, `:112`). The
sign-off is a "Layer 7 additions" row under "Dependency policy" in `1-bootstrap/TASKS.md`, where the Layer 5 and Layer 6 rows sit (`LICENSING.md:239`).
`THIRD-PARTY-NOTICES.md` is regenerated.

### 4.6 Pairing (amends L5 §7.3: amendment 12, normative)

1. **Show.** Each owner opens Settings → Layer 7 → "Pair with another hub" (or `111 fabric pair --show`) and names the role this hub proposes for the other
   hub and the link. The hub mints an L5 enrolment offer (code and `code_secret`, L5 §7.3) and shows a **pairing bundle** as QR and text:
   `111PAIR:<card b32>:<code>:<link SPKI b32>:<address or empty>:<role>:<via>`. A new prefix, parsed in node111; the `111HUB:` parser is untouched.

| Field | Text | Value |
|---|---|---|
| prefix | 8 | `111PAIR:` |
| card | base32 | the hub's card (its stored announce, L5 §7) |
| code | as L5 §7.3 | the one-time enrolment code |
| link SPKI | 71 (base32 of 44 B) | the link key (§4.2) |
| address | 0–47 | `host:port`, or empty when this hub has no address the other can dial |
| role | 4–7 | `peer` · `leaf` · `gateway`: the role the **showing** hub records for the scanning hub, from its own side (new in v0.2, C3) |
| via | 3–7 | `tcp` · `carrier` (new in v0.2, C3) |

   Five `:` separators follow the prefix; the parser takes three fields from the left and two from the right, and the address is what remains, so a
   bracketed IPv6 literal may contain `:`. The two new fields add at most 16 characters to the QR text; nothing of the bundle goes on any 111 wire.
2. **Scan.** Each owner scans or pastes the other's bundle and picks, from this hub's side, the far hub's role, `peer` (default) · `leaf` (this hub is its
   gateway) · `gateway` (it is this hub's upstream), and its link, `tcp` (default) or `carrier` (a far gateway reached through this hub's carrier, §6).
   **Roles are complementary (C3).** The pick must be the complement of the bundle's `role` (`peer` ↔ `peer`, `leaf` ↔ `gateway`) with the same `via`, else
   `POST /relay/pair` answers `409 pair_role_mismatch` and nothing is recorded. A `carrier` pair is `peer` only. A `leaf` pair (and so its `gateway` side)
   is `tcp` only: row 3's final put and the §5.5 probe need a connection.
3. **Enrol both ways.** Each hub pins the other's link SPKI, then runs §7.3 `ENROLL` with the other's code, inside TLS when the address is not private
   (§4.1). The two hubs meet once over `tcp-lan`: the same LAN, a tunnel, or one public address (§19 D10; D31 (5)). One hub dials the other's address and
   enrols on its own connection, as L5 §7.3 has the enrolling node do (`L5:323`); **the second `ENROLL` rides the connection the first hub opened**
   (`ENROLL` is accepted on an attributed inbound connection), so one public address suffices (S22). Enrolment never rides a carrier; afterwards a `carrier`
   pair talks only through its carrier.
4. **Outcome.** The enrolling side's `on_enroll_ok` (`M/fabric.rs:3093-3130`) records a pairing: `paired_hubs += PairedHub {node_id, role, via}`, one record
   (amendment 10, C9). It sets no `home_hub` and writes no mailbox cursor (today it sets both, `:3118-3126`). A pair forwards only when both sides hold it,
   and a put whose arrival conflicts with the recorded role and link is refused (§5.2).
5. **One connection per pair.** A hub dials a paired hub through the daemon's `peers` (`N/fabric/mod.rs:516-522`) when it holds the far hub's address and
   either the far hub holds none for it or its own NodeId sorts lower.
6. **The warning (D30 Q21),** shown when a pair's address is not private and TLS-RPK cannot be negotiated (the far hub's build lacks it), verbatim: "This
   hub is on the open internet and this link cannot be encrypted. Anyone on the path between the two hubs can see which hubs talk, when, and how much.
   Message contents stay sealed. Pair anyway?" It is the `409 consent_required` round-trip with `consent.action = pair_plain` (§13.1, I2): accepting
   records the text hash with the pair; refusing pairs nothing.

`fabric/settings` gains an additive key (the file has no unknown-key refusal, `N/fabric/settings.rs:83-96`; schema stays 1). The fabric's record is
`{node_id, role, via}` (`FabricSettings.paired_hubs`, amendment 10, persisted through `FabricEvent::Settings`, `M/fabric.rs:438`, as today); the daemon
keeps `address`, `link_spki`, `lan` and `warning_hash` beside it in the same row (C9):

```json
"paired_hubs": [{"node_id": "<64 hex>", "address": "203.0.113.7:11183", "role": "peer", "via": "tcp", "link_spki": "<base32>",
                 "lan": false, "warning_hash": null}]
```

### 4.7 What the path and the far hub see

| Observer | plain `tcp-lan` (today) | TLS 1.3 RPK |
|---|---|---|
| on path: IPs, ports, timing, record sizes | yes | yes |
| on path: frame types, both announces (the NodeIds), `dest_hint`, counters, `prio_flags` | yes (L5 §8.1, `5-messaging/README.md:380`) | hidden |
| the far hub: `dest`, size, `prio_flags`, `dest_hint`, counters of each item | yes (`5-messaging/README.md:409`) | yes |
| anyone: content, sender, kind | sealed | sealed |

## 5. Cross-hub custody: single-copy push hand-over (D30 F4, F5)

### 5.1 Topology and the `fwd` table (normative)

A hub's forward targets are its paired hubs. From this hub's side a pair has a role, `peer`, `leaf` (this hub is its gateway) or `gateway` (this hub's
upstream), and a link, `tcp` or `carrier` (a far gateway reached through this hub's carrier, §6). A forward row is an ordinary `custody` row whose `dest` is
not a member, plus a `fwd` column; never `Dir::Transit`, a `messages` row that needs peer, stream, epoch and `n` (`M/store/rows.rs:131-148`), none of which
a hub sees in a sealed item. The longest path is leaf → gateway → carrier → gateway′ → leaf′, and each step is fixed by the table:

| `fwd` | Held when | May go to | Never |
|---|---|---|---|
| 0 | `dest` is a member here (today's row) | the member (`MAILBOX_ITEM`, push) | forwarded |
| 1 from a member | an enrolled member puts a non-member `dest` and a target exists (§5.2) | the hint's pair; else a probed `tcp` pair; else this hub's `gateway` pair; else a `carrier` pair | anywhere else |
| 2 from a leaf hub | the sender's pair is `leaf` here and a `carrier` pair exists | a `carrier` pair only | a `tcp` hub |
| 3 from the carrier | a `carrier` pair's put whose `dest` is a member of a `leaf` pair here | that leaf hub, as a final put | a carrier again |
| from a `tcp` `peer`, this hub's `gateway`, or against the pair's record | never a forward row: the gate is today's (`dest` must be a member here) | none | forwarded |

Every pair's role and link are complementary on its two sides (§4.6 step 2, C3): a `carrier` pair is `peer`, and its puts arrive only through
`carrier_inbound` (row 3); a `leaf` pair and its `gateway` side are `tcp`. A put whose arrival conflicts with the pair's record (a `CUSTODY_PUT` over
`tcp-lan` from a `carrier` pair, a carrier frame from a `tcp` pair) meets the last row.

**A hub's own messages (the v1 limit, D31 (3); C4).** `route()` knows only the home hub (`M/fabric.rs:1821-1841`) and a hub's own outbox never enters its
custody table, so in v1 a hub's own messages reach another island only through its own home hub, if it has one: an always-on hub with no home hub sees
`queued` for every contact on another island. An own-send route into the hub's own forward table (`Route::Forward`, local, no wire) is v1.1, beside the
own-send carrier route (D30 Q20; §17).

### 5.2 The widened gate (amendment 1)

`on_custody_put` (`M/fabric.rs:2824-2862`) keeps its first branch (holds custody and the sender is enrolled, else `REJECT_UNENROLLED`, `:2830-2831`) and its
member branch. The non-member branch (`REJECT_UNKNOWN_DEST`, `:2832-2833`) becomes `forward_ok(origin, dest, hint, size)`; `origin` is the sender's pair
entry when it has one (a paired hub is never treated as a member), else "member". The arm stays a synchronous arm of the single actor and answers
`CUSTODY_ACK` before it returns, so it decides only whether a target **exists**, holds the row and answers; it never awaits a pair's round trip
(sweep-before-ack is v1.1, D30). A `carrier` pair is a target only for a put that fits its frame bound (§6.1):
- **from a member (C2):**
  - a hint that matches a pair's `NodeId[0..8]` ⇒ hold `fwd = 1`, offered first to that pair;
  - a hint that matches no pair ⇒ hold `fwd = 1` when this hub has a `gateway` pair or a fitting `carrier` pair, else `REJECT_UNKNOWN_DEST`;
  - no hint ⇒ when a target exists (any `tcp` or `gateway` pair, or a `carrier` pair and the put fits its bound): hold `fwd = 1`, answer `ACCEPTED`, then
    probe per §5.5, event-driven, after the ack; no target at all ⇒ `REJECT_UNKNOWN_DEST`.
- **from a leaf** (a pair recorded `leaf`, over `tcp`): a `carrier` pair exists and the sealed put fits its frame bound (§6.2) ⇒ `fwd = 2`, to the hint's
  `carrier` pair, else the first in order; otherwise `REJECT_UNKNOWN_DEST`.
- **through the carrier (`carrier_inbound`, a pair recorded `peer` over `carrier`):** `dest` a member ⇒ `fwd = 0`; a member of a `leaf` pair (hint, then
  probe) ⇒ `fwd = 3`; else `REJECT_UNKNOWN_DEST`, returned through the carrier.
- **from a `tcp` `peer`, from this hub's `gateway`, or against the pair's record** (§5.1, C3): today's gate, `REJECT_UNKNOWN_DEST` for a non-member.
- **A stale hint (C18):** a hint naming this hub while `dest` is not a member here ⇒ `REJECT_UNKNOWN_DEST`. A member never adds the TLV for its own hub
  (`hinted_elsewhere`, `M/fabric.rs:1843-1849`), so the case is a TLV repeated at a later hop (or a member that lies); the hint is a locator only and never
  pre-empts the gate (`L5:333`).
- **Too large for the carrier (C15):** a put whose only targets are `carrier` pairs it does not fit is answered `REJECT_UNKNOWN_DEST`, the only refusal v1's
  wire has; the hub counts it (`forward_too_large`, §5.9) and its fabric view shows "too large for the carrier: N B > 2,937 B". The member then reads
  `hub_not_yours` ("your hub does not know them", `API:903`), which is not the cause; the honest status is v1.1 (§17).
- A held forward row is answered `ACCEPTED`; over its origin's share, or over the Transit quota with only protected rows, `REJECT_FULL` (§5.8).

### 5.3 The twelve Layer 5 amendments (D30 F5; R07g2 §3), the only Layer 5 changes D30 lists

All additive or default-off: with no pairing and no carrier a hub is byte-identical and the columns stay 0 and NULL. Envelope bytes, L2 identity and
`pkt_ref` are untouched; the wire gains one optional TLV and one reason string. The twelve are the Layer 5 **spec** amendments; the code seams beside them
(§4.4's stream parameter, §2.1's read-only `attributed_on`) change no Layer 5 spec text and no wire byte, and are listed in §19 "Code seams" (D31 (1), (2)).

| # | Where | Change (replacement text for locked sentences) | Additive? |
|---|---|---|---|
| 1 | gate `M/fabric.rs:2830-2833`; L5 §9 `L5:392-394` | "Hub: sender ∈ enrolled else `REJECT_UNENROLLED`; **`dest` ∈ enrolled else `REJECT_UNKNOWN_DEST`**" → "Hub: sender ∈ enrolled else `REJECT_UNENROLLED`; **`dest` ∈ enrolled ⇒ a member row; else, with a forward target for this sender (a paired hub or this hub's carrier, Layer 7 §5.2) ⇒ a forward row and `ACCEPTED`; else `REJECT_UNKNOWN_DEST`**" | default-off |
| 2 | scope sentence L5 §9 `L5:386-389` | the whole sentence, from "a hub accepts custody only for its own members" to "cross-hub delivery needs A+ (§21).", → "a hub holds custody for its members, from its members; with a paired hub or a carrier it also holds forward rows for non-member dests from its members, its leaf hubs and its carrier (Layer 7 §5); a message to a contact whose hub is not yours and not reachable through a pair stays `queued` (your hub unreachable) or `sent` (your hub answered `REJECT_UNKNOWN_DEST`), reason `hub_not_yours`." (C5; the `sent` reading is the code's, `M/fabric.rs:3053-3061`, and `API:903` says both) | text |
| 3 | A+ sentences `L5:28-30`, §9 `L5:406-408`, §21 `L5:861-862` | §9 bullet → "**Cross-hub custody (Layer 7 §5)** is single-copy push: the origin hub hands each held item to the dest's hub, or to its carrier, with `CUSTODY_PUT`, and the receiving hub applies its own gate. Multi-copy replication, if built, reconciles `pkt_ref` sets; the §4 vectors are not computable by a hub (`n` is sealed)." · `L5:29-30` "cross-hub custody arrives with hub-to-hub replication (A+, §21)" → "cross-hub custody is Layer 7's single-copy push (Layer 7 §5)" · §21 "hub-to-hub replication (A+) and cross-hub custody" → "multi-copy hub replication (Layer 7 §17)" | text |
| 4 | L5 §9 `L5:391` | "(the double seal costs ≈ 90 B" → "(the double seal costs 120 B, `CUSTODY_WRAP` (`M/frag.rs:64`), 131 B with the Layer 7 hint TLV" | correction |
| 5 | `M/store/schema.rs:53,58`; L5 §10.3 | `SCHEMA_VERSION` 3 → 4; V4 = `ALTER TABLE custody ADD COLUMN fwd INTEGER NOT NULL DEFAULT 0 CHECK (fwd IN (0,1,2,3));` and `ALTER TABLE custody ADD COLUMN fwd_from BLOB;` (the 32 B NodeId of the member or pair that put a forward row, NULL for `fwd = 0`; the per-origin share needs it, §5.8, C1) · §10.3 "`custody` (hub: `dest`, `custody_seq`, `pkt_ref`, bytes, `committed`)" → "… `committed`, `fwd` (0 member · 1 from a member · 2 from a leaf hub · 3 from the carrier; Layer 7 §5.1), `fwd_from`)" · a V4 store is refused by a V3 build (`SchemaTooNew`, `:312-316`) | append-only |
| 6 | `M/store/rows.rs:31-56,92-105,1010-1030`; `M/store/prune.rs:228-245,484-509`; L5 §10.4 | `Class::Transit = 10`, refusable, quota-only, never produced by `from_kind`; `Class::ALL` 10 → 11 · `class_bytes` (`prune.rs:484-509`, today the whole `custody` table for `Class::Custody`): `Custody` = `… FROM custody WHERE fwd = 0`, `Transit` = `WHERE fwd != 0` · `evictable` (`prune.rs:228-245`, which labels every committed row `Class::Custody` at `:247`) assigns `Class::Transit` to `fwd != 0` rows · `is_refusable` and `has_quota` (`rows.rs:92-105`) include `Transit` · `custody_put` (`rows.rs:1010-1030`, today admitting under `Class::Custody`) takes `fwd` and `fwd_from` and admits under the row's class, a forward row also under its origin's share (§5.8) · step 4 "uncommitted custody for members" → "uncommitted custody, forward rows included; a forward row marked refused is committed" (C8, C1) | additive |
| 7 | `M/envelope.rs:113-120`; L5 §3 `L5:110-112` | the ext TLV registry gains "`0x02 dest_hub` (8 B, the dest's home-hub hint; `CUSTODY_PUT` only, any other kind ⇒ reject)" | registry entry |
| 8 | L5 §6.2 `L5:277-278`, §9 `L5:413-414`; `M/fabric.rs:3043-3049`; `API:888` | "`ACCEPTED` from a **mailbox-role** node for a **known member** is the only evidence for the UI state `relayed`" → "`ACCEPTED` from a **mailbox-role** node for a **known member, or held for forwarding** (Layer 7 §5), is the only evidence for the UI state `relayed`; the member shows reason `forwarding` when the dest's hint names another hub" · `MessageState` (`API:888`) "relayed: an authenticated CUSTODY_ACK from a mailbox node for a known member" → "… for a known member, or held for forwarding to another hub (Layer 7 §5)" (C11: `pnpm gen:types` does not catch prose) | widened |
| 9 | `M/outbox.rs:64-87`; `API:903` | `Reason::Forwarding`, code `forwarding`, sentence "Your hub holds it and passes it to their hub."; `pnpm gen:types` | enum value |
| 10 | `M/fabric.rs:344-354` (`FabricSettings`), `:3093-3130`, `:3593-3600`; `N/fabric/settings.rs:83-96,364-380` | `paired_hubs: Vec<PairedHub {node_id, role: Peer \| Leaf \| Gateway, via: Tcp \| Carrier}>`, one record the fabric's gate reads, persisted through `FabricEvent::Settings` (`:438`) as today; the daemon keeps address, SPKI, `lan` and the warning hash beside it in `fabric/settings` (C9) · an enrolment started as a pairing records it and sets neither `home_hub` nor a cursor; the member count and `rebuild_home_hub` skip paired hubs | additive |
| 11 | new arms in `M/fabric.rs` | the forwarder (§5.9): a pending map `pkt_ref → custody_seq`; an `on_custody_ack` arm for senders in `paired_hubs` (today only the home hub's acks are read, `:3030-3032`); `forward_take` (acks consumed on take, C7), `forward_done`, `carrier_inbound`; `FabricEvent::Forward`; per-pair `reachable` and `pending` in `FabricStatus`; the refused mark (§5.8) and two counters in `Counters` (`:645`), `forward_refused` and `forward_too_large` (C1, C15) | additive |
| 12 | L5 §7.3 `L5:330` | "A node has one home hub in v1 (replicas = deferred A+)." → "A node has one home hub in v1. Pairing two hubs (Layer 7 §4.6) is two enrolments, one in each direction; a paired hub is never a home hub." | text |

`Dir::Transit` and step 1's "delivered transit copies" query (`M/store/prune.rs:9-11`) stay unused. `remove_link` and a non-blocking `Link::send` move to
v1.1 with the `carrier` `LinkKind`.

### 5.4 The hint TLV `0x02` (normative bytes)

| Field | Bytes | Value |
|---|---|---|
| `n_ext` | 1 | the TLV count; present whenever `HAS_EXT` is set (`M/envelope.rs:99-100`), so it is a new byte only when `0x02` is the only TLV |
| type | 1 | `0x02` |
| len | 1 | `8` |
| value | 8 | the dest contact's `home_hub` hint (`contacts.home_hub`, `M/store/schema.rs:74`), the dest's own signed `appdata` |

- **Placement:** the inner header of the hub-sealed `CUSTODY_PUT` (L5 §3 inner layout). It cannot follow the payload: `decode_put` takes the rest as the
  item (`M/mailbox.rs:91-97`). 11 B when it is the only TLV (with `n_ext`); 10 B when another TLV is already present (C12).
- **Sender:** `pump_hub` (`M/fabric.rs:2182`) adds it when `hinted_elsewhere` is true (`:1844-1849`). `plan_mtu` reserves `CUSTODY_WRAP + 11` for
  `Route::Hub` (`:1855`).
- **Forwarder:** a hub that forwards a row repeats the TLV it received on its own put, so a gateway and a far gateway can match it.
- **An old hub** skips the type as unknown (`M/envelope.rs:735-738`) and applies today's gate.

### 5.5 Discovery without a hint

A contact known only by `node1…` has no hint. The row was held and acknowledged first (§5.2, C2); the probe is ordinary puts the forwarder sends after the
ack. The custodian probes its `tcp` `peer` and `leaf` pairs in `paired_hubs` order (the order the owner sees), then its `gateway` pair, then its `carrier`
pairs, one put at a time, and stops at the first `ACCEPTED` or `DUPLICATE`: the answerer's gate checked enrolment, or the answerer holds it for forwarding.
A `REJECT_UNKNOWN_DEST` moves to the next pair; a pair that is down keeps its turn until its next link-up; when every target has refused, the row is
refused (§5.8, C1). **What a probe shows (C14):** each probed pair receives the full sealed item and learns its `dest`, size and `pkt_ref`; it can read
nothing, and a pair that refuses keeps nothing (D30 Q19; §14). Hits are cached in memory, `dest → pair`, at most 256 entries (≈ 16 KB, R07g2 §2 answer 1),
oldest evicted, emptied on restart. A refusal is never cached beyond its connection.

### 5.6 Evidence (normative)

- The first hub's `ACCEPTED` earns `relayed` with reason `forwarding` when the dest's hint names another hub (`hinted_elsewhere`); without a hint the UI
  shows plain `relayed`. Honest: a mailbox node durably holds the item in a protected row. It means "held and will be offered", **never "the dest's hub was
  found"** (C2).
- **The honest limit (C1; D30 Q18).** A row every target refused is marked refused at the first hub and may be evicted; the member still shows `relayed`,
  because v1 has no later status for it. The sender keeps its own copy until `delivered` (L5 §9), so meeting the contact directly still delivers it;
  sweep-before-ack, which would keep `hub_not_yours` honest here, is v1.1 (§17).
- Later hops earn nothing visible to the sender; they are internal hub facts ("committed").
- `delivered` comes only from the recipient's `ACK` (`apply_ack`, `M/fabric.rs:2793`), an ordinary item on the reverse path, forwarded the same way.
- The sender keeps its copy until `delivered` (L5 §9). A held fragment is not re-sent in the same session (`M/fabric.rs:2207-2209`; the flag is in memory,
  `M/frag.rs:146-147`, and is lost at a restart, `:170`), so **no hub may let go of a forward row before the next custodian's `CUSTODY_ACK`** (§5.8; §19
  C5).

### 5.7 Loops and duplicates

(a) `pkt_ref` is UNIQUE (`M/store/schema.rs:155-162`; dedup `M/store/rows.rs:1014-1017`; `DUPLICATE` at `M/fabric.rs:2840`) and is kept after hand-over, so
a bounced item answers `DUPLICATE` and counts as done. (b) The `fwd` table, with roles complementary on both sides of every pair (enforced at pairing,
§4.6, C3): no row ever returns to where it came from, so a star of pairs has no cycle and needs no TTL or hop count. (c) The receiver's prefix and ranges
never redeliver (L5 §4). (d) Retransmits reuse the stored sealed bytes (`M/fabric.rs:2189-2198`) and deduplicate at the hub.

### 5.8 Quotas and eviction (normative)

- **`Class::Transit`**, quota-only, settings key `quota.transit`. Default: one quarter of the custody quota when one is set; L5 sets none by default (a
  quota is a setting, unset = no cap, `M/store/prune.rs:543-560`; `admit_inner`, `:818-833`), so the default base is the store's high watermark, 512 MiB
  (`DEFAULT_HIGH_WATERMARK`, `:79`): **128 MiB** (a default; the owner may change). The custody class counts `fwd = 0` rows only and Transit counts
  `fwd != 0` rows (amendment 6, C8), so strangers' items never count against members' custody quota.
- **The per-origin share (C1 (b); D31 (6); §19 D5).** Each origin of forward rows, a member (`fwd = 1`) or a pair (`fwd = 2` from a leaf, `fwd = 3` from a
  carrier pair), told apart by `fwd_from`, may hold at most `quota.transit / 8` of Transit: **16 MiB** of the default (derived from `quota.transit`, no
  setting of its own). Over its share, that origin's forward puts get `REJECT_FULL` while other origins are still admitted, so one member, one leaf hub or
  one carrier pair cannot starve the star.
- **Protected while `committed = 0`:** other people's undelivered items sit in step 4's "never" tier (`M/store/prune.rs:32-38`).
- **Committed** on the next custodian's `ACCEPTED` or `DUPLICATE`: over `tcp-lan` at once, through a carrier when the far hub's `CUSTODY_ACK` comes back
  through it. Then step 1 evicts the row by size × age (`evictable`, `M/store/prune.rs:228-245`); `pkt_ref` stays until it does.
- **Refused (C1 (a); D31 (6)).** A forward row is marked refused (`committed = 1`) once every target the §5.1 table allows it has answered a refusal (a
  `REJECT_UNKNOWN_DEST` on an attributed connection, a carrier's `REJECT_UNKNOWN_DEST` or `Rejected`, §5.9) or the item fits no target left. Step 1 may then
  evict it like any committed row; `forward_refused` counts it in `FabricStatus` and the hub's fabric view shows the count. A target that is down keeps the
  row protected until it answers; the per-origin share bounds what a dead pair can pin. The member keeps `relayed` (§5.6).
- **The high watermark (C1 (c)).** Protected Transit rows count toward the store's high watermark (`live_bytes` in `admit_inner`, `M/store/prune.rs:818-833`).
  At the 512 MiB default up to 128 MiB of it may be other people's items, so member custody is refused once the store is full even when no member quota is
  reached; the owner's `quota.transit` bounds that exposure.
- Over the Transit quota with only protected rows left, new forward puts get `REJECT_FULL`; puts for members are admitted until the high watermark.

### 5.9 The forwarder and the hand-off API (amendment 11, normative)

- **Event-driven.** The forwarder runs when a forward row is inserted, when a paired hub's connection attributes, on a contact event, and when a carrier
  calls `forward_take`. No timer. A pair that is down is redialled by the existing back-off, 0.5 s to 60 s (`M/link/tcp_lan.rs:128-129`).
- **Over `tcp-lan`** the fabric seals a `CUSTODY_PUT` to the pair with the stored item bytes and the hint TLV, records `pkt_ref → custody_seq`, and reads
  the pair's `CUSTODY_ACK`: `ACCEPTED` or `DUPLICATE` commits the row; `REJECT_UNKNOWN_DEST` tries the next target (with none left, the row is refused,
  §5.8); `REJECT_FULL` keeps the row for that pair and is retried on the pair's next attribution and on every forwarder run for that pair (a row insert, a
  `CUSTODY_ACK` from it), never on a timer, since a held `tcp-lan` connection may never cycle (C16); `REJECT_UNENROLLED` marks the pair broken ("the other
  hub no longer lists this hub") and returns the row to the other targets.
- **Through a carrier** the same frames go through the hand-off API, in-process in v1 (the JSON-lines pipe for an external program is v1.1; §19 D12).

```rust
// mesh111 (amendment 11): the fabric side.
impl Fabric {
    /// Sealed frames owed to `target`, a `carrier` pair: forward puts (custody_seq = Some) and CUSTODY_ACKs owed to it
    /// (None). An ack is consumed by this call and never returned again (C7).
    pub async fn forward_take(&self, target: NodeId, max: usize) -> Result<Vec<CarrierFrame>>;
    /// What the carrier did with a put it took.
    pub async fn forward_done(&self, custody_seq: u64, done: ForwardDone) -> Result<()>;
    /// A sealed frame from `from` (a `carrier` pair) that arrived through the carrier; verified like any sealed packet.
    pub async fn carrier_inbound(&self, from: NodeId, frame: Vec<u8>) -> Result<()>;
}
pub struct CarrierFrame { pub custody_seq: Option<u64>, pub bytes: Vec<u8> }    pub enum ForwardDone { Taken, Rejected { why: &'static str }, Retry }
// FabricEvent::Forward { target: NodeId, pending: u32 } — "forward pending / target up", coalesced per target per tick.

// node111 (§6): the carrier side.
pub trait Carrier: Send + Sync {
    fn id(&self) -> &'static str;                       // "ntfy"
    fn max_frame(&self) -> usize;                       // the largest sealed frame one carrier message holds
    fn flush<'a>(&'a self, pair: &'a Pair, out: Vec<CarrierFrame>) -> BoxFuture<'a, Vec<(Option<u64>, ForwardDone)>>;  // None: an ack
    fn drain<'a>(&'a self, pair: &'a Pair) -> BoxFuture<'a, Result<Vec<Vec<u8>>, CarrierError>>;   // 5A: one poll
    fn hold<'a>(&'a self, pair: &'a Pair) -> BoxStream<'a, Result<Vec<u8>, CarrierError>>;          // 5B: the held stream
}
```

- `Taken` = the carrier's mailbox accepted the message: an internal "posted" mark, kept in memory by the carrier. The row stays `committed = 0` until the far
  hub's `CUSTODY_ACK` arrives through `carrier_inbound` (§5.6; §19 C5).
- **Re-posts (C6).** A posted row with no ack is posted again only (1) when a drain that began after its post returned a frame from the far hub (proof the
  far hub flushed after it) and still no ack for it; (2) on "Flush now"; (3) on a `since` miss, the sign that the server lost its cache (a server or stub
  restart); (4) once after this hub restarts, since the posted mark is in memory. Never on a timer, never on every flush. The one race (the far hub's frame
  was posted after this hub's post by a flush whose drain ran before it) costs at most one extra re-post per row, answered `DUPLICATE`. `RelayStatus`
  counts re-posts per pair (`paired_hubs[].reposts`, §13.1).
- **Acks (C7).** A `CUSTODY_ACK` that `forward_take` returned (`custody_seq = None`) is consumed on take and never retried, whatever `flush` reports for it
  (`flush` returns `Option<u64>` so its fate can be logged). A lost ack heals by itself: the far hub re-posts the put under the rule above, and the next
  `carrier_inbound` answers `DUPLICATE`, a fresh ack. Until this hub posts a later frame, the far hub's row stays protected; its "Flush now" or a `since`
  miss re-posts it sooner.
- `Retry` = transient (no route, HTTP 429 or 5xx): the row stays for the next flush. `Rejected` = permanent (413, 401, 403): the row returns to the other
  targets; with none left it is refused and counted (§5.8, C1). Never silently dropped.
- `carrier_inbound` processes a frame with no connection, the way a mailbox item is (INTRO allowed, `M/fabric.rs:2552-2558`); a `carrier` pair's put meets
  the `fwd = 3` branch of §5.2.

### 5.10 Costs

Per message, a 100 B text in one fragment (item ≈ 190 B), on `tcp-lan`, 3 B frame header counted (R07g2 §2 answer 8):

| Hop | Frame | Bytes |
|---|---|---|
| member → hub A `CUSTODY_PUT` | item + 120 wrap + 11 hint + 3 | ≈ 324 |
| A → member `CUSTODY_ACK` | 64 + 8 + 18 + 16 + 3 | ≈ 110 |
| A → hub B `CUSTODY_PUT` | item + 120 (+ 11 when repeated) + 3 | ≈ 313 |
| B → A `CUSTODY_ACK` | as above | ≈ 110 |
| B → dest `MAILBOX_ITEM` | item + 90 + 3 | ≈ 283 |
| dest → B `MAILBOX_COMMIT` | 64 + 8 + 3 + 16 + 3 | ≈ 95 |
| forward total | | ≈ 1.2 KB (6.3× the item); the dest's `ACK` back ≈ 0.95 KB more |

Through ntfy each frame grows by 4/3 (base64) plus the batch's 2 B per frame and the HTTP request (measured at build).

**Resident, hubs only:** one held connection per pair; each side sends a 3 B keepalive every 25 s (`M/link/tcp_lan.rs:125`): ≈ 2.4 sends and 2.4 receives a
minute, +2.4 % of the ≈ 205 wakeups a minute (R07g2 §2 answer 9), ≈ 0.4 MB a day plain. Inside TLS a 3 B record grows by 22 B (5 B header, 1 B content type,
16 B tag): ≈ 0.53 MB a day (arithmetic; measured at build). Plus the ≤ 16 KB cache and the pending map, bounded by the Transit quota.

### 5.11 Nothing on a non-hub node

Every new path starts behind `holds_custody()` (`M/fabric.rs:356-360`; the custody gate `:2830`, the fetch `:2911`, the commit `:2942`, the member count
`:3593`). Pairing is refused unless `hub_role` is on, so `paired_hubs` is empty; nothing is dialled, stored, or forwarded; the carrier is never constructed;
the second netlink socket never opens. G9 asserts it (§15).

## 6. Carriers: the contract and the ntfy prototype (D30 F3 P)

### 6.1 The carrier contract (normative)

1. A carrier moves **opaque sealed frames** between two paired hubs: the same `CUSTODY_PUT` and `CUSTODY_ACK` packets the pair would exchange on `tcp-lan`,
   sealed hub to hub. It never opens, re-fragments or reorders them.
2. **Batching** is glue: frames go out in custody order, packed into as few carrier messages as the carrier's body limit allows, once per flush; never one
   message per fragment.
3. A carrier declares `max_frame`; a carrier target is a target only for a put that fits (§5.2), so an item that fits no target gets today's
   `REJECT_UNKNOWN_DEST`, counted as too large in the fabric view (§5.2, C15), and never occupies Transit space. Larger frames need a carrier's attachment
   path (v1.1).
4. **A carrier is not a `Link` in v1** (`M/link/mod.rs:255-276`): rows move through §5.9, and `LinkKind` (`:228-252`), `remove_link` and the contract's enum
   stay untouched.
5. A carrier earns no 111 state by itself: its accept is a "posted" mark; the far hub's `CUSTODY_ACK` is the only commit.
6. Before every flush the carrier consults the consent record, the metered budget and the licensed-board rule (§12, §14).

### 6.2 The ntfy carrier (normative shape; a prototype until its smoke log)

**Adopted:** binwiederhier/ntfy, Apache-2.0 and GPL-2.0 dual licensed (both files read, R06 §4), v2.36.0 on 2026-10-31 with monthly tags (R07g1 §2); bus
factor and issue response unread (accepted for a prototype behind the switch, D30 Q16). The server is the owner's; ntfy.sh is for a manual check only (D30
Q15). The client is ureq with rustls, which `111d` already links (`1-bootstrap/crates/node111/Cargo.toml:28`): no new process, crate or listener. **The
carrier builds its own `ureq::Agent` (S6)**, as `wan_probe` builds its own today (`N/services/probe.rs:259-268`), not the updater's `Fetcher`: ureq is
synchronous, and `Fetcher`'s one capped `get` and global timeout (`NI/util/http.rs:21-30`) fit neither a POST nor a held stream. 5A requests run in
`spawn_blocking` with connect and global timeouts. The 5B stream runs in one named std thread, `111d-ntfy-5b`, with no global timeout and a read timeout of
3 × the server's keepalive interval (taken from the smoke log), reconnecting with back-off; that thread exists only while 5B is on (§16).

**Topic** (derived from the pairing, never from a NodeId; new L2 domain tag `111/carrier-topic/v1`, §18):
- `topic_key = BLAKE2b-256(key = X25519(own_enc_sk, peer_enc_pk), data = "111/carrier-topic/v1" ‖ lo(enc_pk_a, enc_pk_b) ‖ hi(enc_pk_a, enc_pk_b))`,
  computed behind the identity handle like `store_secret` (L5 §10.2). Only the two paired hubs can compute it. It names a mailbox; it protects no content
  (L5 §18 rule 5: static-static ECDH never protects confidentiality, and the frames are sealed anyway).
- `topic(X→Y) = "111" ‖ base32-lowercase(BLAKE2b-160(key = topic_key, data = NodeId_X ‖ NodeId_Y))`: 35 characters, one topic per direction (ntfy's topic
  alphabet and length limit: *unverified from this box*).
- `carrier.ntfy.topic_secret` (32 B, set on both sides) replaces the X25519 output as the key, to re-key without re-pairing.

**Batch** (the body of one ntfy message, base64url without padding, ≤ 4,096 B; "longer messages are treated as attachments", R07g1 §3 item 2):

| Field | Bytes | Value |
|---|---|---|
| version | 1 | `0x01` |
| n | 1 | frames in this batch, ≥ 1 |
| per frame: len | 2 | big-endian |
| per frame: frame | len | one sealed hub-to-hub packet |

4,096 B of base64 carry 3,072 raw bytes, so `max_frame` = 3,068 B and the largest item a put can carry is 3,068 − 120 − 11 = 2,937 B.

**Requests:**
- Publish: `POST <server>/<topic(me→peer)>`, body = the batch, `Content-Type: text/plain`; with an `auth-file` server, the owner's access token (header per
  ntfy's docs, *unverified from this box*).
- Drain (5A): `GET <server>/<topic(peer→me)>/json?poll=1&since=<last id>`, newline-delimited JSON (upstream `docs/subscribe/api.md:240-300`, R07g1 §3 item
  2); each body's frames go to `carrier_inbound`, and the last id per topic is saved in `state/relay.json` once the fabric took them. An evicted id (a
  `since` miss): read again with `since=all` *(unverified from this box: R07g1 item 2 read `since=<id>|duration|timestamp`; the OB §3 bench item)*.
  Already-processed frames are dropped as stale by the L2 window (`L5:375-377`) before any `pkt_ref` check; nothing unprocessed is lost, because the id is
  saved only after the fabric took the batch (C13). A `since` miss is also re-post trigger (3) of §5.9.
- Held (5B): the same `GET` without `poll=1`, kept open; the server streams new messages and its own keepalive events (interval set by the server, measured
  at build); a drop reconnects with back-off.

**Retention and limits.** Default cache 12 h, in memory, lost on a restart; durability needs `cache-file` and `cache-duration` (R07g1 §3 item 3). Defaults:
a burst of 60 requests, then one per 5 s, 30 open connections per visitor (§3 item 4); ntfy.sh's anonymous limit of 250 messages a day and its retention are
*unverified from this box*. A lost batch costs a re-post (the row waits for the far ack), never a message. A 429 is `Retry`.

**The recommended self-host config** ships as `7-relay/carriers/ntfy/server.yml` (a documented file, keys from upstream's `server/server.yml`, R07g1 §3
items 3–5): `cache-file` set; `cache-duration: "168h"` (longer than the expected gap between two gateways' flushes; a default; the owner may change);
`auth-file` with `auth-default-access: "deny-all"` and one read-write token per gateway pair; attachments off.

**5A flush** (D30 §3 pick "flush on events"): on a debounced netlink change after which the server answers (the check of §3.2 is the flush's first drain);
on `FabricEvent::Forward` for a `carrier` pair; on "Flush now"; on the Termux fallback. A flush is: drain (acks first, so acked rows are not posted again),
post the frames owed (`forward_take`, re-posts only by §5.9's rule), drain once, stop. Between flushes there is no process, connection or timer. **5B:**
the held stream plus one post per `Forward` event.

**What the operator sees:** the hub's IP address, two topic names, message sizes and times, and the outer headers of hub-to-hub packets (the far hub's 4 B
`dest_hint`, counters, `prio_flags`). Not the items' own headers (they are sealed inside), not who they are for, not any content. Whether a server forwards
messages to a third-party push service is recorded in the smoke log.

**Consent copy (verbatim; shown before the switch turns on, its hash recorded):** "To carry messages between this hub and <far hub>, 111 will post sealed
packets to the ntfy server at <server> and read packets back from it. The server's operator can see this hub's internet address, the topic names and a
short tag of the two hubs' ids, how many packets are posted, their sizes and when. The operator cannot read them or tell which person they are for. Packets
stay only as long as the server keeps them (12 hours on a default server); a lost packet is sent again. Nothing is sent while this switch is off."

### 6.3 The switch and every Layer 7 setting (S19)

Every Layer 7 setting lives in `state/relay.json` (0600; invalid ⇒ moved aside, the `fabric/settings` rule, `N/fabric/settings.rs:17-21`) except
`quota.transit`, a store quota (the store's quota table, key `quota.<class>`, `M/store/prune.rs:94-96`; set through `Store::set_quota`, `:552`), and the
pairs, which live in `fabric/settings` (§4.6). The carrier and the heartbeat are switches in the Layer 6 §2.1 rule-2 sense (zero weight while off), not
§2.1 components: no `assets/components/` entry, no payload, their consent in `state/relay.json` (S20); the chatmail carrier (§6.5) rides the Delta Chat
component.

| Setting | Default | Values | File |
|---|---|---|---|
| `gateway.enabled` | `false` | `true` only on a hub | `state/relay.json` |
| `gateway.termux_poll_min` | `15` | minutes, ≥ 9; read only on a Termux hub whose netlink bind failed (§3.4) | `state/relay.json` |
| `carrier.kind` | `"none"` | `"ntfy"`; `"chatmail"` after §6.5 lands | `state/relay.json` |
| `carrier.ntfy.server` | `null` | an `https://` URL; `http://` only for a private address, and then the token crosses that LAN in clear (§9.5 T6, S12) | `state/relay.json` |
| `carrier.ntfy.token` | `null` | an access token; a secret, never on any API | `state/relay.json` |
| `carrier.ntfy.topic_secret` | `null` (derived) | 32 B base32; a secret, never on any API | `state/relay.json` |
| `carrier.mode` | `"5A"` | `"5B"` (refused on a Termux hub) | `state/relay.json` |
| `carrier.consent` | `null` | the hash of the consent text the human accepted (`carrier_enable`, §13.1) | `state/relay.json` |
| `presence.heartbeat.<transport>` | `0` (events only) | minutes; every write refused in v1 (§2.4) | `state/relay.json` |
| `discovery.lan_record` | `false` | `true` (§7.1) | `state/relay.json` |
| `discovery.public_gateway_bit` | `false` | `true`, after its consent (`gateway_bit`, §12) | `state/relay.json` |
| `discovery.offload_bit` | `false` | `true` (§7.2) | `state/relay.json` |
| `origin.serve` | `false` | `true` on a hub with LAN sharing, after its consent (`origin_serve`, §12) | `state/relay.json` |
| `origin.targets` | the hub's own target | target triples to hold and serve (§9.1) | `state/relay.json` |
| `updates.check_internet` | `true` | `false` (§9.3) | `state/relay.json` |
| `metered` | `[]` | `[{match: {iface \| ssid}, daily_cap_bytes}]` (§12) | `state/relay.json` |
| `quota.transit` | 128 MiB (a quarter of the custody quota when one is set, §5.8) | bytes; the per-origin share is `quota.transit / 8`, no setting of its own | the store's quota table |

`state/relay.json` also keeps state that is not a setting: the drain's last id per topic (§6.2), the consent hashes and the metered window (§12). With
`carrier.kind = "none"` or `gateway.enabled = false` nothing is constructed: no socket, task, thread, file write or netlink subscription.

### 6.4 The KB entry and the smoke log

`0-knowledge/projects/ntfy.md` enters as `candidate` (desk research, `0-knowledge/README.md:93`) with the unread gates recorded. The adopt-vs-build rule's
branch is "prototype the best one" (`:97`). G10 writes `7-relay/verify/logs/ntfy-smoke-<calver>.md` against an owner-run server: the server version and
config; per request the method, status, body bytes and latency; the 4,096 / 4,097 B boundary; 429 behaviour; retention across a server restart with and
without `cache-file`; `since=<id>` after eviction; the held stream's keepalive interval; whether the server forwards to a push service; end-to-end delivery
between two gateway hubs in netns; the client's RSS during a flush. That log is what moves the entry to `testing`.

### 6.5 Chatmail, the alternative carrier

Layer 6 already ships the Delta Chat component (`deltachat-rpc-server`, MPL-2.0, and an optional Madmail relay, AGPL-3.0; ≈ 64 MB RSS while on, zero off; OB
F3 B) for readable text. As a 111 carrier it needs one glue piece: a second `Carrier` that sends each batch as an attachment of a Delta Chat message to the
far gateway's address and takes attachments only from that address, never shown in Messages. Verified relay defaults (operators change them): 30 MiB
messages, 20 days (7 days above 200 kB), 60 a minute, burst 10 (R02 finding 9). The relay is always the user's choice; there is no default list (D30 Q4).
Built after the ntfy smoke log, behind `carrier.kind = "chatmail"`, default off. Consent copy: "To carry messages between this hub and <far hub>, 111 will
send sealed packets as mail attachments through <relay>, which you chose. The relay's operator sees both mail addresses, the attachment sizes and when they
are sent, and may keep mail for up to 20 days. It cannot read them. Nothing is sent while this switch is off."

### 6.6 The reach layer (D30 F3, in the owner's order)

1. **A public address:** a pair's `address`; nothing new. 2. **yggstack** (Yggdrasil in user space; LGPL-3.0 with a linking exception): v1.1, an
opt-in component once it has a release tag (none in the Go proxy, R07g1 §2), user-named peers only (D30 Q17), a resident Go process on both ends, RSS
unmeasured. 3. **An adopted tunnel** (WireGuard, Headscale), documented in `7-relay/docs/tunnel.md`, never required; inside a tunnel the pair is marked `lan
= true`. A stock Tailscale client logs to its vendor unless `TS_NO_LOGS_NO_SUPPORT` (R07g1 §3 item 8). Reach tools are pipes: two 5A islands never meet
through them, which is why a mailbox carrier exists.

### 6.7 Nostr, LXMF propagation, iroh (v1.1)

Nostr needs a bench that reads real public relays' acceptance of opaque regular-kind events, their PoW and paid policies (unread from this box), and a
separately built carrier binary for secp256k1 and websockets (D30 Q3: none in v1); 1.5–3.8× overhead (R02 finding 4). LXMF propagation needs the owner's
answer to the rule question of a helper verb that carries sealed 111 frames (OB F4 C); it ends at `sent` (R02 finding 11). iroh solves NAT, not storage,
defaults to its vendor's discovery and costs a second Rust program (+13 MB, R05 §3.1); it needs the owner's say. Each is one more `Carrier`.

## 7. The Service Discovery Registry and gateway advertisement (D30 F7)

Each fact lives in the cheapest layer that can hold it honestly.

**7.1 DNS-SD, an unauthenticated hint.** A second `ServiceInfo` in the mdns-sd daemon the dashboard already runs (`N/dashboard/mdns.rs:40-80`), which starts
only while the dashboard is LAN-bound (`N/dashboard/mod.rs:259`); the registry never starts the daemon itself.
- Type `_111._tcp.local.`; instance = the L2 label (`xxxx-xxxx-xx`); host = the dashboard's mDNS host (`111.local.` or its rename, `mdns.rs:15-16`); SRV
  port 11183; A records only (the daemon disables IPv6, `mdns.rs:44-47`; the fabric listener is `0.0.0.0`, `N/fabric/mod.rs:173`).
- TXT, 78 B with length bytes: `v=1` · `r=<roles, 4 hex>` · `k=<NodeId, 64 hex>` (RFC 6763 size guidance *unverified from this box*).
- Present only while `discovery.lan_record` (default `false`), the dashboard is LAN-bound and the fabric listener is LAN-bound (`hub_role` or `listen_lan`,
  `N/fabric/mod.rs:691-700`).
- Browsing is on demand only (Nearby's "Look for 111 nodes here", §7.7): `browse` for at most 5 s (a default; the owner may change), then `stop_browse`
  (`MD/service_daemon.rs:441`, `:476`), and only where the daemon already runs.
- A row binds nothing: the `tcp-lan` handshake and a verified packet do (`5-messaging/README.md:375`). A forged TXT makes a browser dial a wrong address
  once.

**7.2 Signed capability: role bits.** `appdata` v1 is `[1][roles u16 LE][0]` plus an optional 8 B hint (`N/fabric/sealer.rs:141-148`); bits 0 and 1 are set
from the settings (`N/fabric/mod.rs:334-343`), bit 2 is defined and never set (`M/contacts.rs:146`; bits 0–2 are mesh111 constants, `:141-146`). Bits 3–5
and their name map (for `NearbyNode.roles`) are defined in node111, beside `appdata` in `N/fabric/sealer.rs` and set in `roles(s)`
(`N/fabric/mod.rs:334-343`): no mesh111 change (I9; §19 "Code seams"). New bits, each a setting, default off:

| Bit | Name | Set by | Consent |
|---|---|---|---|
| 3 | `ROLE_GATEWAY` | `discovery.public_gateway_bit` | the cost is named (§12): every listener learns this node has internet |
| 4 | `ROLE_OFFLOAD` | `discovery.offload_bit` | serves model inference to paired LAN members (L3 offload) |
| 5 | `ROLE_ORIGIN` | `origin.serve` (§9.1) | serves the verified update files |

A change re-mints the announce once: one counter value committed with an fsync before it is sent (`M/sealer.rs:53-54`; `2-identity/README.md:96`). No
per-event fact rides here. **No TLVs in v1:** `AppData::parse` reads bytes 4..12 as the hint whenever they exist (`M/contacts.rs:160-171`), so a TLV region
may only follow a present hint (§19 C8); a LoRa announce keeps 10 B free with the hint, 18 B without (R00 §3 finding 7).

**7.3 Dynamic facts: the sealed query (bytes pinned; built in v1.1).** Decision: one new control-kind pair, not an existing request.

| Kind | Name | Direction | `msg_seq` | Payload |
|---|---|---|---|---|
| `0x0C` | `REGISTRY_QUERY` | node → an attributed contact or its hub | 0 | `want(1)`: bit 0 gateway · 1 offload · 2 origin |
| `0x0D` | `REGISTRY_REPLY` | answer, same connection | 0 | CBOR map ≤ 255 B: `gateway {carrier, reachable, checked_ago_s}`, `offload {model, queue}`, `origin {release}` |

Why: an existing request cannot carry it without a Layer 5 change anyway (mailbox payloads reject trailing bytes, `M/mailbox.rs:16-18`; the fabric's only
public send is `send(to, text)`, `M/fabric.rs:856`); a kind pair is additive (an old node drops an unknown kind, L5 §3.2) and works between any attributed
pair, not only member and hub. `0x0A` stays free: an envelope test relies on it being unknown (`M/envelope.rs:1644`). Answered only on an attributed
connection, never on a broadcast bearer, never to a stranger. It is a registry amendment beyond D30's twelve, so it lands in v1.1 with its first cross-node
consumer (§19 D6). In v1 dynamic facts appear only on the node's own dashboard.

**7.4 Never advertised:** an alias or display name, load, battery, location, loaded models, anything that names a person. A hub's address and code travel
only inside a bundle.

**7.5 Members-only gateway advertisement.** No public bit by default. In v1 a member learns its hub is a gateway only from the hub's offer at enrolment
(the offer JSON gains an additive `gateway {carrier, mode}`), once, and from its own messages' `forwarding` evidence (§5.6); in v1.1 from the sealed query
(D31 (4)). A member that enrolled before the carrier was turned on, and every contact, learn nothing until then: INTRO and cards carry nothing new in v1
(card bytes are the announce itself, §19 C9), and a TLV on an existing hub-to-member payload would be a thirteenth Layer 5 change. A member's dashboard shows
nothing about its hub's gateway state (§3.8). This narrows D30 F7's "advertised to members and contacts only, sealed (the hub offer, INTRO, cards)" (I5;
§19 D16).

**7.6 Rows say which layer answered:** "hint (unverified)" (DNS-SD) · "signed by this contact" (role bits) · "asked just now" (a query reply, v1.1). Every
string is untrusted and rendered as text. An offload target is only ever the private IP literal `guard_model_base` admits (`A/src/config.rs:325-335`), typed
by the user (v1) or taken from a query reply (v1.1), never from a TXT.

**7.7 Lost-hub recovery.** The existing back-off keeps redialling the last address (`M/link/tcp_lan.rs:658-700`). On the user's "Find my hub", or on a
netlink address change while the home hub's connection has been down for one full back-off cycle (its redial back-off has reached `backoff_max`, 60 s,
`M/link/tcp_lan.rs:128-129`; S23: a DHCP renew with the hub up costs nothing), a member whose dashboard is LAN-bound browses `_111._tcp` for 5 s and
matches `k`; the new address is kept only after a handshake names the hub's NodeId (the dial's expected check, `tcp_lan.rs:30-35`). Otherwise: rescan the
hub's QR. **No `/24` sweep** (D30 Q10). **Termux:** no MulticastLock, so no mDNS browse or answer (R01 §3 finding 8): a phone finds its hub by address or
QR and is found through its hub; held sockets die in Doze, so a phone hub is 5A.

## 8. The mesh epoch (D30 F8)

**8.1 The beacon (normative bytes; built with its first consumer).**

| Field | Bytes | Value |
|---|---|---|
| `issuer` | 32 | the issuer's NodeId; **only in the transferable form** (API objects), never on the wire (C17) |
| `epoch` | 8 | LE u64; one epoch = **one day** of the issuer's powered-on time (D30 Q9) |
| `sig` | 64 | Ed25519 by the issuer's identity key over `"111/epoch/v1" ‖ issuer(32) ‖ epoch(8 LE)` |

On the wire (TLV `0x03`, §8.3) the beacon is `epoch ‖ sig`, **72 B**: the issuer is the sealed frame's authenticated sender (`pair_mac`). The signature
makes the value transferable, which needs the issuer beside it: the transferable form is `issuer ‖ epoch ‖ sig`, **104 B**, and the API objects carry it as
`epoch_beacon = {issuer: <64 hex>, epoch: <integer>, sig: <128 hex>}` (pinned now, built in v1.1; C17). **Floor:** the release process writes `epoch_floor`
(days from 2026-01-01, this spec's origin day (a default; the owner may change), to the release's calver date, computed by the release tool) into the
payload at `share/epoch-floor`; nodes never read a clock. It is not a manifest key because the manifest parser refuses unknown keys and any other
`manifest-version` (`NI/manifest.rs:62-64`, `:211`, `:343-345`; §19 C4). A manifest-version 2 field comes with the first consumer.

**8.2 What advances a hub's counter.** Persisted powered-on monotonic time: `CLOCK_BOOTTIME` (counts suspend, stops at power-off, settable by nobody),
accumulated in `fabric/epoch.json` (0600) `{schema, anchor_floor, anchor_s, powered_s}`, written only when the day value advances (checked at each issue,
so at most once a day of powered-on time, never on every fetch answer) and at clean shutdown, with normal durability (C17); a crash loses only the time since
the last write, at most a day, so the counter runs slow, never back. `epoch = max(last_issued, floor, anchor_floor + (powered_s −
anchor_s) / 86400)`; a release with a higher floor re-anchors. Read at each issue: no timer, no wall clock, never a gateway's NTP time.

**8.3 Carriage.** On `ENROLL_OK` and the hub's mailbox answers (an item-less `MAILBOX_ITEM` or a fetch's first item) as ext TLV `0x03 epoch_beacon` (72 B),
because those payloads reject trailing bytes (`M/mailbox.rs:16-18`); the TLV is a Layer 5 registry entry made with the consumer. Cards: the offer and card
API objects gain an additive `epoch_beacon` in the transferable form with its issuer (§8.1), never inside card bytes. Never a broadcast.

**8.4 Adoption.** A node adopts the greatest valid beacon from its **pinned home hub** only, never below the floor of its installed release. A hub uses its
own counter. Nobody adopts a paired hub's or a stranger's value (the E-B gossip design is rejected, D30).

**8.5 v1: not evaluated.** v1 builds nothing for the epoch. Attestation and certificate `expiry` is a bare u64 (`ID/attest.rs:111`; `is_expired`,
`:646-648`); `verify_full` takes a caller's epoch (`:654-665`) and the bridge passes `None` (`N/bridge/attest.rs:151`). Pinning the unit now lets every
record count in days. **First consumer: freeze protection for updates** (§9.5 T4): a manifest whose floor is older than N epochs behind the home hub's
beacon is shown stale.

**8.6 Threats.** A frozen hub (powered off, or a stuck counter) extends certificates for its own members only, and the release floor catches up once a
member updates from any origin. A compromised hub can jump its members forward (their certificates die early: denial of service, the direction L2 names,
`2-identity/README.md:220`); they already trust it with custody; the remedy is enrolling elsewhere. The floor stops any hub from holding time below a
release. A Cat-1 relay is asked for nothing.

**8.7 Hand-off and distinction.** This closes L2 §13's "mesh-epoch source" hand-off (`2-identity/README.md:301`). It is not L5 §4's `epoch`, one
announce-counter value minted per store open for `msg_seq` (L2 A3, `2-identity/README.md:323-328`).

## 9. Updates and the KB from a hub (D30 F6)

**9.1 The hub as origin (normative).** An origin is a set of files: `channel-111-stable.toml`, `channel-111-stable.toml.minisig`, `blob/<sha256>`.
- Routes on the existing dashboard listener: `GET /origin/v1/channel-111-stable.toml`, `GET /origin/v1/channel-111-stable.toml.minisig`, `GET
  /origin/v1/blob/{sha256}`. Read-only, unauthenticated and GET-only (405 otherwise): the `/health` posture. `/origin/v1/*` joins the open set, mounted
  beside `/health` outside the session gate (`N/dashboard/routes.rs:16-18` documents the open set, `/health` is mounted at `:469`; `require_session`,
  `N/dashboard/api_v1/auth.rs:1023-1029`, covers `/api/v1/*` only): GET only, no CSRF, the same CSP (S8, I7). It is listed in the conventions' "Not gated"
  list (`4-interface/api/README.md:143`) and in Layer 1's machine contracts (`1-bootstrap/README.md:137`), and stays out of `openapi.yaml`, whose scope is
  `/api/v1`. No third port.
- **The origin base (S4).** A member fetches `origin_base = http://<home_hub.address host>:11180/origin/v1/`: the host of the home hub's fabric address
  (`host:11183`) on the dashboard's `DEFAULT_PORT` (`N/dashboard/mod.rs:25`), never `:80` (the dashboard binds 80 only when it can, `:259`). The hub serves
  the routes on every LAN-bound dashboard listener. The Host guard admits the hub's own addresses (`host_allowed(.., own_addrs)`,
  `N/dashboard/routes.rs:297-302`), so an IP-literal `Host` passes.
- Served only while `hub_role` **and** LAN sharing **and** `origin.serve` (default `false`) are on; the dashboard is LAN-bound only with LAN sharing
  (`N/dashboard/mod.rs:259`; §19 C6). Otherwise 404.
- The hub serves only bytes it verified: the manifest and signature that passed `parse_verified`, and blobs whose sha256 and size match a pin
  (`NI/manifest.rs:154-159`), kept in `state/origin/` (written by the daemon, size shown). It holds its own target's payload; another only when listed in
  `origin.targets` (default: the hub's own target). Bodies stream from disk.

**9.2 `111 update --from <dir|url>`.** A new verb (today's set: `NI/cli.rs:1-3`, `:188-191`, usage `:196-210`), a oneshot run as the node user (`sudo -u
node111 111 update --from /media/stick`). It reads the three names from the directory or below the URL, runs the updater's one check (`check_once`,
`N/updater/fetch.rs:100-200`) against that origin, writes only `state/staging/<ver>/`, and drops `state/update-request.json` (`N/updater/stage.rs:124-142`).
The root helper stays the only flip actor (`1-bootstrap/README.md:155-156`). No resident weight.
- **The staging lock (new, S1).** No lock exists today: `run_with_hooks` runs `check_once` in a bare `spawn_blocking` (`N/updater/mod.rs:85-125`) and
  `fetch.rs` and `stage.rs` take none, so the verb (a second process) and the daemon's loop could both write `state/staging/<ver>/` and the request file. New:
  `state/staging.lock`, an `flock` opened `O_CLOEXEC`, taken by `check_once` for the whole fetch–stage–request sequence, in the daemon's loop and in
  `111 update --from` alike. The verb takes it without waiting: held ⇒ it prints "the daemon is checking now" and exits 75 (`EX_TEMPFAIL`), staging
  nothing. The daemon's loop waits for it inside its `spawn_blocking`. G12 asserts that a concurrent run stages once.
- **The payload fetch (S5).** `check_once` fetches the payload at the absolute `payload.url` inside the signed manifest (`N/updater/fetch.rs:161`), which
  names an internet base. For a hub origin or `--from` the payload is fetched at `<origin>/blob/<sha256 of the pin>` and `payload.url` is ignored; the
  internet bases use `payload.url` as today. The pin's sha256 and size are checked either way (`stage_payload`, `N/updater/stage.rs:73-90`, and the root
  helper). `record_success` (`N/updater/fetch.rs:205-215`: `last-update-check.json`, the dashboard's "last successful check") is written only by an
  internet-base check; hub-origin and `--from` answers go to the per-origin record that `GET /api/v1/update/origins` shows (§9.3).

**9.3 Origin order and when it is checked.** The visible order is `[home hub, install.111.network, GitHub Releases]` (the bases,
`N/updater/fetch.rs:18-20`); the home hub counts only while its announce carries `ROLE_ORIGIN`, so a node whose hub serves nothing makes no new request.
The home hub's origin (`origin_base`, §9.1) is checked once when the fabric link to it attributes, and on `--from`: no gossip, no timer (M1). The internet
bases keep the hourly loop (`N/updater/mod.rs:22-23`), started unconditionally today (`N/mod.rs:933-952`), which gains `updates.check_internet` (default
`true`, today's behaviour; user-off, D30 Q2). `GET /api/v1/update/origins` lists the order and each origin's last answer.

**9.4 The KB** stays inside the release payload at `current/share/kb/` (`N/kb.rs:1-3`; K1); a KB change is a calver respin. The `[kb]` pin stays parsed and
unconsumed (`NI/manifest.rs:541`).

**9.5 The Layer 1 trust amendments (normative; they land before the hub origin, D30 Q11).**
- **T1 channel.** `parse_verified` refuses a manifest whose `channel` is not the channel asked for (`stable`). Today only emptiness is checked
  (`NI/manifest.rs:346`) and the accessor (`:512-514`) has no caller in the updater.
- **T2 keychain walk.** The order (S24): parse the untrusted bytes → walk the chain from the compiled key → verify the minisig under the generation reached
  → check the floor. Today `parse_verified` verifies the minisig under the compiled key before it parses (`NI/manifest.rs:495-507`), and the walk needs the
  manifest parsed first. The same function serves the daemon (`N/updater/fetch.rs:135`), the root helper (`NI/update_helper.rs:138`) and the installer
  (`NI/install/preflight.rs:109`). The compiled key (`N/updater/mod.rs:18-19`) anchors the generation whose `pubkey` equals it. A later generation counts
  only if its `signature_by_previous` verifies over its `pubkey` under the previous generation's key. The manifest's signature must verify under the
  highest generation reached, which must be ≥ the floor's `highest_keychain_generation`; a broken link ends the chain. Today the chain is checked for
  consecutive numbers and present signatures (`NI/manifest.rs:448-470`), the signature uses the compiled key only (`:495-507`), and the floor trusts the
  manifest's own maximum generation (`:682-697`).
- **T3 `min-upgrade-from`.** The root helper and the daemon's staging decision refuse a manifest whose `min-upgrade-from` is above the installed release.
  Today only the installer checks it (`NI/install/stage.rs:216-221`).
- **T4 freeze stance (v1).** A stale origin can hold a node on an old valid manifest until the node reaches any other origin. The UI shows "newest release
  seen: <calver> from <origin>", and every reachable internet base is tried in order. Expiry by mesh epoch comes with §8's consumer.
- **T5 the verb** writes only `state/staging/<ver>/` and the request file (§9.2).
- **T6 http origins.** The fetcher's rule "nothing in production code may pass" an `http://` URL (`NI/util/http.rs:4-7`) gains two exceptions: the home
  hub's LAN address learned at enrolment (`origin_base`) and a URL the user typed for `--from`. Every byte still passes minisign, the floor, the pins and the
  tree check (`N/updater/fetch.rs:136-171`; `NI/update_helper.rs:131-169`). The transport is never trusted. A third `http://` use, outside the updater: a
  `carrier.ntfy.server` on a private address (§6.3), through the carrier's own agent (§6.2), never `Fetcher`; its access token then crosses that LAN in
  clear, and the settings row says so (S12).

**9.6 Notes.** The fetching node reads the payload whole into memory (`N/updater/fetch.rs:161`; cap 512 MB, `:101`), so RSS rises by the tarball size during
an update: not resident, recorded when the payload changes. LoRa carries no release: 5.7 MB is 15 h of LongFast airtime unsealed, 62–110 days under a 1 %
duty cycle; the 96.6 KB KB 15–27 min; a manifest with its signature (≈ 3.2 KB) 30–55 s (R04 §3 item 11). v1 sends nothing about releases over LoRa; a 5-byte
"release exists" tag is v1.1. Layer 8 datasets in the same store are signed by their publisher (D30 Q12): Layer 8's question; `state/origin/blob/` is the
seam.

## 10. Nearby radar (D30 F9)

**10.1 Rules (normative).** An own SVG radar in the lazy `nearby` chunk (`UI/app/routes.tsx:16`), replacing the ASCII placeholder
(`UI/views/nearby/radar.ts:27-52`) and the line "Radar and map views arrive with Layer 7" (`UI/views/nearby/index.tsx:191`).
1. The centre is this node. Ring = hop count + 1: a direct LAN peer and a Meshtastic `hops_away = 0` node sit on ring 1; a contact heard only through the
   home hub on ring 2. Reticulum hops come from `rnpath -t -j` (read-only, D30 Q7), run only by a human's `POST /api/v1/nearby/hops` (§13.1), never by a
   GET: the agent's bearer reads every GET and must not spawn Python at will (I6). One run goes through the `rnstatus` runner and its timeout
   (`<root>/venvs/rns/bin/rnstatus --config <root>/rns`, `N/transport/rnstatus.rs:135`, here `rnpath`; a spawn or timeout failure, `:86`) and its owner
   guard (no spawn unless the supervised rnsd holds the instance, `RnstatusError::NotOwned`, `:38-42`, `:96`); its output is parsed as untrusted JSON into
   `hops_away` of the `reticulum` source only and published on the `nearby` frame.
2. Within a ring, stronger SNR sits nearer the inside; without SNR, the more recent `heard_ago_s`.
3. Angle only when this node and the peer both report a position: the bearing to the peer's cell centre. Otherwise a stable hash of the id fixes the angle,
   and the legend says so.
4. Every Meshtastic position is an **uncertainty cell**, never a pin: the side follows the channel's precision bits (32 bits 1.1 cm, 19 bits 91 m, 16 bits
   729 m, 13 bits 5.8 km, 10 bits 46.7 km; R05 §3.3 item 4), from an additive `NearbyNode.position_precision_bits` when known. **v1 knows none:** the
   presence sidecar exports `{lat, lon, alt?}` only (`5-messaging/sidecars/meshtastic-presence/meshtastic_presence.py:183-184`, `:237-239`), so v1 draws the
   coarsest cell (10 bits, 46.7 km) for every position and the field stays null; the sidecar's export of the precision bits is v1.1 (§17, §18; I10).
5. No hop figure: the outer "unplaced" ring. The glyph follows the source kind. Plain SVG, so it draws on old phones.
6. Legend (verbatim): "Rings count hops: the inner ring is heard directly. Direction means nothing unless both nodes shared a position; otherwise a node's
   place on its ring comes from its id. Shaded areas are positions the sender's radio rounded. The outer ring holds nodes whose hop count is unknown."

**10.2 Labels.** Each dot's row is its evidence string and age (§2.2). **10.3 Positions:** 111 nodes never share a position by default; v1 has no position
setting. **10.4** A link opens the hosted Meshtastic panel's map (blank offline, R05 §1).

**10.5 The map bucket (a recorded D21 amendment, for Layer 8; v1.1).** An exempt "map" bucket in `4-interface/ui/scripts/check-budget.mjs` (the fonts
precedent, `:1-7`): Leaflet + PMTiles + protomaps-leaflet ≈ 69 KB gz, loaded only where `GET /api/v1/tiles` lists a set, behind a Range route in `111d` (≈
60 lines, R05 §3.3 item 6). The lazy bucket holds 219.9 of 256 KB with a 64 KB per-chunk cap (R05 §3.3 item 2; `check-budget.mjs:13-14`); the radar is
measured at build. The CSP stays `connect-src 'self'` (`N/dashboard/routes.rs:78`).

## 11. Public signed frames (D30 F11)

**11.1 `BROADCAST` 0x30 (bytes pinned; built in v1.1).** L5 §12 leaves the format to this layer (`5-messaging/README.md:489-491`).

| Field | Bytes | Value |
|---|---|---|
| `version` | 1 | `0x01` |
| `kind` | 1 | `0x30` |
| `flags` | 1 | bit 0 `POINTER`; bits 1–7 reserved, MUST be 0 |
| `publisher_ref` | 8 | publisher NodeId[0..8]; collisions ⇒ try each followed candidate |
| `seq` | 4 | LE u32, per publisher, strictly increasing; ≤ the last seen ⇒ duplicate (no clock) |
| `obj_hash`, `obj_size` | 32 + 4 | only with `POINTER`: the BLAKE3 root and size of the object, fetched by Tier 2 (L5 §11) |
| `text_len`, `text` | 1 + n | UTF-8 |
| `sig` | 64 | Ed25519 by the publisher over `"111/broadcast/v1" ‖` all preceding bytes |

Fixed part 1 + 1 + 1 + 8 + 4 + 1 + 64 = **80 B** (`text_len` counted), **116 B** with a pointer: on the 160 B floor (159 B after the shim,
`M/link/mod.rs:34`) that leaves **79 B** of text, or **43 B** with a pointer (C10). A frame is verified only against a publisher the user follows; any
other is dropped and counted. It ranks below user messages and waits for airtime. Carriage: on broadcast bearers as shim `frame_type` 2, public sub-type
**3** (L5 §3 assigns 0–2); on `tcp-lan` only once v1.1 negotiates it, because an unknown frame type drops the connection (`M/link/tcp_lan.rs:23-26`). The
inner `Kind::Broadcast` stays refused as not built (`M/envelope.rs:602-607`, `:1927`). "Global broadcasts" are the publishers the user follows; a 5B
gateway streams only those.

**11.2 Attestation gossip.** The 202 B L2 §10 record (`ID/attest.rs:6-10`) rides INTRO and `ENROLL` / `ENROLL_OK` as ext TLV `0x04` (202 B ≤ the 255 B TLV
limit, `M/envelope.rs:683`), and the card API object as an additive `attestations` array. Never a LoRa announce (`appdata` ≤ 32 B, `ID/announce.rs:40-49`).
Supersedure as L2 §10: the greatest `expiry_epoch` wins.

**11.3 The label.** Only a two-sided record (`0x03` Reticulum today; `0x04` link keys later) may show `attested-111` (L6 §3), with the copy "Linked to your
111 contact <alias>: both keys signed the link." A one-sided `0x02` record never does (`ID/attest.rs:155-180`; `N/bridge/attest.rs:16-24`). The store table,
the verifier's use, the label's rendering and a Nostr readable-text Adapter are v1.1.

## 12. UI, settings, consent, metered and licensed links

- **Settings → Layer 7** at `/settings/l7`, "Relay & presence", its own lazy chunk ≤ 64 KB (the Layer 6 precedent `/settings/l6`,
  `UI/views/settings/index.tsx:2`, `:14`, `:19`). Sections: Presence (the interval per transport, shown and inert in v1, §2.4; the airtime line of §2.6;
  the bans) · Gateway (on, carrier, server, mode, consent, "Check now", "Flush now", the badge) · Paired hubs (my pairing bundle, scan theirs, role, link
  state per pair: `tls rpk`, `lan`, `plain (warned)`; unpair) · Discovery (the LAN record; role bits 3–5 with their costs) · Updates (the origin order, the
  internet check, "serve updates for", the `--from` instruction) · Metered networks.
- **Consent copy, verbatim:** ntfy §6.2 · chatmail §6.5 · the pairing warning §4.6 · the public gateway bit: "This adds 'willing to act as a gateway' to
  this node's signed card. Everyone who receives the card, on any network, learns that this node has internet. Your hub's members learn it without this." ·
  origin serving: "Any device on this network can download this hub's verified update files without signing in. They are public files signed by the 111
  release key; nothing about you is in them." Each consent records the text hash; changed text asks again. **Every Layer 7 consent is the pinned
  round-trip (I2):** `409 consent_required` with `consent` = `ConsentRequired {action, subject, warning_text, hash, addons}` (`API:1392-1401`; conventions
  "Consent round-trip"), the client shows `warning_text` verbatim and repeats the call with `accept_warning` = the hash (on `POST /proposals/{id}/decision`
  too). `action` gains four values, appended to the closed enum (`API:1397`; in the pin list, §13.2): `carrier_enable` (ntfy or chatmail on; subject
  `<kind>:<server or relay>`), `pair_plain` (the §4.6 warning; subject the far hub's NodeId), `gateway_bit` (the public bit), `origin_serve` (origin serving).
  A metered cap is part of the carrier's consent: setting or raising `daily_cap_bytes` while the carrier is on re-asks `carrier_enable` with the cap in
  `warning_text` and `metered` in `addons`.
- **Metered links (a user setting in v1).** `metered = [{match: {iface | ssid}, daily_cap_bytes}]`, default `[]`. Carrier bytes (bodies and headers, both
  ways) count in a rolling 24 h window on `CLOCK_BOOTTIME`, persisted, never the wall clock. Past the cap no flush runs and the badge says "metered cap
  reached". The SSID is a hint the user marks: Termux cannot read Android's metered flag (R05 §3.5 item 4).
- **Licensed boards: a hard block.** Every Layer 7 path (a flush, a forward, a heartbeat, a broadcast) refuses a link onto a board marked licensed (Layer 6
  reads `is_licensed` live, `6-transport/README.md` §5, §15 item 5), with no override in v1 (D30 Q8, until the regional legal read). v1 has no sealed LoRa
  path, so the block is structural today and stays a rule for v1.1.
- **The agent may propose** enabling a gateway, opening the pairing screen, a heartbeat interval (refused `unsupported_action` in v1, §2.4) and a flush
  (§13.3). It never accepts a consent, never types a code, never sees a secret.

## 13. API contract additions and agent tools (additive; the binding form is `4-interface/api/openapi.yaml` at build time)

### 13.1 Routes and events

| Route | Method | Shape | Who |
|---|---|---|---|
| `/api/v1/relay` | GET | `RelayStatus` (below) | session or loopback; the agent's bearer reads |
| `/api/v1/relay/settings` | PATCH | a partial `RelaySettings`; a consent gate answers 409 `consent_required` with `consent` (`ConsentRequired`, §12) until the repeat carries `accept_warning`; a heartbeat write answers 409 `no_such_transport` or, for `rns`, 409 `settings_locked` (§2.4) | a human; bearer 401 |
| `/api/v1/relay/pair/offer` | POST | `{role, via}` (the role this hub proposes for the other, §4.6 step 1) → `{bundle, code, link_spki}`; dismissed with the L5 offer route | loopback only |
| `/api/v1/relay/pair` | POST | `{bundle, role, via, accept_warning?}` → 202 `PairedHub`; 409 `consent_required` (`pair_plain`), 409 `pair_role_mismatch` (§4.6), 409 `needs_hub` | loopback only, like its Layer 5 siblings (I15); the CLI `111 fabric pair` covers the phone case |
| `/api/v1/relay/paired/{node_id}` | DELETE | unpair; forward rows return to the other targets | loopback only (I15) |
| `/api/v1/relay/carrier/check` · `/flush` | POST | 202; 409 `consent_required`, `metered_cap`, `licensed_board` | a human |
| `/api/v1/registry` · `/registry/browse` | GET · POST | `{schema, rows: [RegistryRow]}` · 202, ≤ 5 s; 409 `lan_off` | session · a human |
| `/api/v1/nearby/hops` | POST | 202; the hop counts arrive on the `nearby` frame (§10.1 rule 1); 409 `component_unavailable` while the Reticulum component is off; 429 `rate_limited` within 30 s of the last run | a human; bearer 401 (I6) |
| `/api/v1/update/origins` | GET | `{schema, origins: [{kind, url, last_answer, newest_seen}]}` | session |
| `/origin/v1/{file}` | GET | the §9.1 files; 404 when not serving; outside `openapi.yaml` (§9.1) | none (the `/health` posture) |

New SSE events: `relay` (a full `RelayStatus` on change) and `registry` (the rows); after `reset` the Layer 7 views refetch `/relay` and `/registry`.
**Problem codes (I3):** "needs hub" is the existing `409 needs_hub` (conventions "Errors", `4-interface/api/README.md:16`); five codes are appended to the
closed `Problem.code` enum (`API:596-598`): `no_such_transport`, `licensed_board`, `metered_cap`, `lan_off`, `pair_role_mismatch`. The pairing warning is a
consent (`pair_plain`), never a code of its own.

```json
{"schema": 1,
 "gateway": {"enabled": true, "state": "carrier_ok", "carrier": "ntfy", "endpoint": "ntfy.example.org", "mode": "5A",
             "checked_ago_s": 240, "last_flush_ago_s": 240, "pending": 0, "metered_cap_reached": false},
 "paired_hubs": [{"node_id": "<64 hex>", "label": "abcd-efgh-ij", "role": "peer", "via": "tcp", "tls": "rpk", "reachable": true, "pending": 2,
                  "reposts": 0}],
 "forward": {"refused": 0, "too_large": 0},
 "presence": {"heartbeat": [{"transport": "rns", "interval_min": 60, "owner": "layer6"}]},
 "discovery": {"lan_record": false, "public_gateway_bit": false, "offload_bit": false},
 "origin": {"serve": false, "targets": ["x86_64-unknown-linux-musl"], "bytes": 0},
 "updates": {"check_internet": true}}
```

`PATCH /relay/settings` takes the same keys (`{"carrier": {"kind": "ntfy", "server": "https://ntfy.example.org", "mode": "5A"}}`); secrets
(`carrier.ntfy.token`, `topic_secret`) are write-only, answered as `"set": true`, never echoed, logged or sent on SSE. `paired_hubs[].reposts` counts the
carrier re-posts of §5.9 (C6); `forward` mirrors the fabric's `forward_refused` and `forward_too_large` counters (§5.8, §5.2).

### 13.2 Additions to existing shapes

- `NearbySource.kind` gains `111` (`API:1118`); `NearbyNode` gains optional `heard_ago_s`, `heard_seq`, `evidence`, `roles`, `position_precision_bits`
  (`API:1096-1111`). A `111` row: `{"id": "<64 hex>", "num": 0, "long_name": "Sam's laptop", "short_name": "abcd-efgh-ij", "last_heard_hint": null,
  "heard_ago_s": 95, "heard_seq": 18342, "evidence": [{"transport": "tcp-lan", "text": "a message from them arrived (verified)"}], "roles": ["mailbox"]}`.
- `Message.reason_code` vocabulary gains `forwarding` (state `relayed`: "Your hub holds it and passes it to their hub.", `API:903`).
- The `gateway` service row: codes and `detail` of §3.5, in the emitted set (`API:614`) · the hub offer gains `gateway {carrier, mode}` (§7.5).
- Proposal `kind` gains `relay` (`API:754`): the body is one JSON object `{"why": string, "actions": [Action]}` from the closed vocabulary `gateway.enable
  {carrier, mode}` · `hub.pair {}` · `presence.interval {transport, minutes}` · `carrier.flush {}`. Accepting it runs the same validation, consent and
  refusal paths as the routes above; a consent gate answers 409 `consent_required` and the human repeats the decision with `accept_warning`. In v1
  `presence.interval` is refused `unsupported_action` (§2.4, I4). It renders as a card.
- `ConsentRequired.action` gains `carrier_enable`, `pair_plain`, `gateway_bit`, `origin_serve` (appended, §12).
- **The pin list (I3; the conventions' form, `4-interface/api/README.md:126`; the change is not done until all are green):**
  1. `Proposal.kind` 8 → 9 (`relay`): `KINDS` in `N/dashboard/api_v1/proposals.rs:103` (`[&str; 8]` → 9; an unlisted kind is `502 upstream_error`), the
     UI's proposal-kind handling (a card from the parsed JSON, as `device`), and the agent's proposal writer.
  2. Agent tool count 17 → 20: `A/tests/daemon_tools.rs:318` and `A/tests/chat_loop.rs:308`, the `DAEMON_TOOLS` list, and `A/src/tools.rs` schemas (beside
     `nearby.read`, `:192`) and dispatch (beside `:331`), §13.3.
  3. `pnpm gen:types`, then `node scripts/gen-sse.mjs --check`; the new enum tuples and aliases (`NearbySource.kind`, `Problem.code`,
     `ConsentRequired.action`, `Proposal.kind`, the two events) are listed in `4-interface/ui/scripts/gen-sse.mjs` (`ALIASES`, `:276`; `ENUMS`, `:407`).
  4. `x-sse-events.events` (`API:1698-1712`) gains `relay` → `SseRelay` and `registry` → `SseRegistry`, both new schemas, and the UI store's
     `switch (frame.event)` (`UI/lib/api/store.ts:425`) handles them.
  5. The contract tests: `4-interface/ui/test/gen.test.ts` (event names and enum mirrors), `test/http.test.ts` (every operation has an adapter call),
     `test/layer6-contract.test.ts` (closed enums and enum growth, the precedent a Layer 7 contract test follows), and the mock adapters, whose frames
     validate against `x-sse-events`.
  6. node111's `dashboard/api_v1/` mirrors: the `NearbySource` kinds (`kind_for`, `N/dashboard/api_v1/nearby.rs:54-56`), the `Problem` codes and the
     `ConsentRequired` actions, beside the new routes.

### 13.3 Agent tools (the Layer 3 set grows; fixed, shell-free, schema-validated)

| Tool | Kind | Shape | Gate |
|---|---|---|---|
| `presence.read` | read | `{peer?}` → the `111` source rows with their evidence, inside a labelled untrusted block | none |
| `gateway.read` | read | → `RelayStatus` (no secret is ever in it) | none |
| `nearby.read` (extended) | read | now includes the `111` source and, after a human ran `POST /nearby/hops`, Reticulum hop counts (never triggers a run) | none |
| `relay.propose` | write intent | `{actions, why}` → proposal `kind: relay` | a human accepts |

- The count grows from 17 to 20: bump `A/tests/daemon_tools.rs:318` and `A/tests/chat_loop.rs:308`; schemas beside `nearby.read` (`A/src/tools.rs:192`),
  dispatch beside `:331`.
- The agent reads freely and writes nothing: the bearer is refused on every non-GET (`6-transport/README.md` §13). It never accepts a consent, enters a code
  or sees a secret. Names and evidence reach the model only as untrusted text.
- **Offload:** a registry row becomes an offload target only as the private IP literal `guard_model_base` admits (`A/src/config.rs:325-335`), from the user
  (v1) or a query reply (v1.1), never from a TXT record; a hub never proxies inference to the WAN (`3-intelligence/README.md:48`).

## 14. Security, privacy and legal (threat model; hard rules)

**What is exposed, to whom:**

| Path | On the path | At the far end |
|---|---|---|
| member ↔ hub, `tcp-lan` (unchanged) | IPs, timing, sizes, NodeIds in announces, `dest_hint`, counters | the hub: `dest`, size, `prio_flags`, `dest_hint`, counters |
| hub ↔ paired hub, TLS-RPK | IPs, ports, timing, record sizes | the paired hub: as a hub sees its members' items |
| hub ↔ paired hub, plain (LAN, tunnel, or warned) | as member ↔ hub | as above |
| probe without a hint (§5.5) | as the link it rides | every probed pair: `dest`, size, `pkt_ref` of the item, which it cannot read (C14) |
| ntfy carrier | the TLS client's IP and SNI to the server | the operator: IP, topic names, sizes, times, hub-to-hub outer headers |
| chatmail carrier | as for mail | the relay: both addresses, sizes, times, retention up to 20 days |
| DNS-SD record | everyone on the LAN: label, NodeId, role bits, port | — |
| role bits | everyone who gets the card, on any network | — |
| hub origin | everyone on the LAN: which release the hub holds | — |

**Hard rules.**
1. **Never proxy for a peer:** no inference to the WAN, no WAN request on a member's behalf.
2. **Never forward anything unsealed**, except public signed frames from publishers the user follows (§11, v1.1) and a bridge the user turned on (Layer 6).
3. **Never into a licensed or amateur link** (§12), and never a sealed frame into one.
4. **Never on a metered link past its cap**, and never on a metered link at all unless the user set a cap.
5. **No contact with 111's servers for liveness**: the only recurring contact is the updater's check, visible and user-off.
6. **Topic secrecy:** a topic derives from the pair's X25519 secret or the user's secret, never from a NodeId; tokens and secrets never reach any API, log,
   SSE frame or agent output.
7. **Link keys:** random, per node, never seed-derived, never the identity key, never on any API; pinned byte-exact; an unpinned key ends the connection
   before any 111 byte.
8. **A hub holds others' items honestly:** protected until the next custodian's `CUSTODY_ACK`, or until every target refused the row (§5.8); an HTTP
   accept is never a custody receipt.

**Attacks and defences.**
- **A forged hint or TXT:** the receiving hub's own gate checks enrolment and refuses, and the forwarder tries the next target; a hint equal to the
  custodian's own id is refused (§5.2); a forged TXT costs one wrong dial, and a recovered address needs a handshake naming the hub (§7.7).
- **Custody starvation** (strangers' items fill a hub): the Transit quota (128 MiB by default) refuses new forward puts with `REJECT_FULL`, and each origin,
  a member, a leaf hub or a carrier pair, holds at most its share (`quota.transit / 8`, 16 MiB) before its own puts are refused while other origins still
  forward (C1). A row no target will take ends as refused and becomes evictable (§5.8), so nothing pins Transit forever but a target that never answers,
  which the share bounds. Protected Transit rows count toward the store's high watermark: at the cap, member custody is refused too, and the owner's
  `quota.transit` bounds that exposure. A member that floods forward puts is the owner's member, visible in the fabric view.
- **Topic squatting or guessing** (someone posts to or reads a topic): the name is 160 bits from a secret the two hubs share; a frame posted by anyone but
  the pair fails `pair_mac` or names another sender and is dropped and counted; reading the topic yields sealed bytes only.
- **A malicious paired hub** can drop, delay or misreport custody for items it holds (L5 §18 rule 6 for any hub); it cannot read, forge or redirect them.
  The sender keeps its copy until `delivered`; unpairing takes one click. It learns `dest`, sizes and timing of what it carries.
- **Replayed announces** prove nothing (L2 A6); presence never counts them (§2.2). A replayed carrier batch is deduplicated by `pkt_ref` and the L2 window.
- **A frozen or compromised hub for the epoch:** §8.6. **A stale hub freezing updates, or a rogue origin on the LAN:** §9.5 T4; every byte is checked
  against the compiled key, the floor and the pins, so an origin can withhold a release but never install a wrong one.

**Legal.** Licensed-band rules are the licensee's fact (Layer 6 §15 item 5); the duty-cycle figures in §2.6 are the EU 1 % sub-band only, and other regions'
rules are a legal read, never a limit of ours. ntfy's dual licence, chatmail's relay licence and Madmail's AGPL §13 offer are recorded in `LICENSING.md`
when their glue lands.

## 15. Verification (lock gates; netns, no hardware)

**Gate homes.** New `7-relay/verify/{presence,forward,carrier,tls,origin,idle}.sh`, each skipped without its prerequisites (unprivileged netns via `unshare
-Urnm`; `NTFY_BIN`; OpenSSL ≥ 3.2; `strace` for §3.7's optional window 3) unless its `REQUIRE_*` is set for a lock run, as Layer 6's gates do; a
stdlib-only ntfy stub `7-relay/verify/stub-ntfy.py`; shared helpers from `6-transport/verify/lib.sh`. Every harness stops what it started and leaves
nothing listening.

**G8, presence and the registry** (`presence.sh` plus node111 tests). Asserts: a `111` row appears on an inbound message and on a `delivered` change, with
the §2.2 evidence string and a `heard_ago_s` that grows · **the LAN-vs-hub split (I1):** a contact whose message arrives on an attributed `tcp-lan`
connection gets `transport = tcp-lan`, the same contact's message through the hub with no direct connection gets `hub`, and both facts show · a replayed
announce, a card and a keepalive create or refresh nothing · **heartbeat, the exact codes (I4):** a write of `presence.heartbeat.<t>` answers 409
`no_such_transport` for `tcp-lan`, `kiss-modem` and `meshtastic-client`, 409 `settings_locked` naming `PATCH /api/v1/bridge/settings` for `rns`, and the
`presence.interval` proposal action `unsupported_action`; nothing is written · **the bans (I12):** node111 unit tests of the validator answer
`licensed_board` for a licensed board and `400` below 9 min for a phone category (the category is a hardware re-probe, `N/mod.rs:841-844`, so a netns node
cannot be a phone); with the Layer 6 bridge stub (`NODE111_BRIDGE_STUBS`, `6-transport/verify/bridge.sh:18-19`) reporting a licensed board, no Layer 7
path sends to it · **the mdns-sd re-arm (S2, the first test written):** `get_ip_check_interval()` reads 0 within 2 s of every address event and the record's
A records changed; with the second netlink bind failing it reads 5 · `_111._tcp` exists only with `lan_record`, LAN sharing and a LAN-bound listener, its TXT
is exactly `v`, `r`, `k` · a browse stops within 5 s · no alias, load, battery, position or model in any record · a role-bit change re-mints once · the
`gateway` row walks its four codes · **the hop read (I6):** one `rnpath` spawn per `POST /nearby/hops`, none on any GET, `409 component_unavailable` with the
component off, `429` within 30 s · lost-hub recovery dials no address it was not given (egress log). **Pass:** all green.

**G9, cross-hub forward** (`forward.sh`: hubs A, B, C in netns, A and B on a TEST-NET address so TLS applies; members M on A, D on B; mesh111 tests for the
gate, the TLV, V4, `Class::Transit`). Asserts, after R07g2 §2 Sequence A:
1. With no pairing, A answers a non-member put `REJECT_UNKNOWN_DEST`, and the frames A sends equal a Layer 6 build's: a committed tape under
   `7-relay/verify/tapes/` (the `6-transport/verify/tapes/` precedent) (I13).
2. With the hint: M's put carries TLV `0x02` (11 B); A holds `fwd = 1, committed = 0`; M shows `relayed`, reason `forwarding`; B answers `ACCEPTED`; A
   commits; D receives once; D's `ACK` crosses B → A as an ordinary forward and M shows `delivered`.
3. Without a hint: A answers M `ACCEPTED` before any probe (C2), probes in `paired_hubs` order, the first `ACCEPTED` wins, and a second message to D is one
   put with no probe (cache). With no pair at all, A answers `REJECT_UNKNOWN_DEST`.
4. **Every pair refusing (C1):** the row is marked refused, `committed = 1`, step-1 evictable, and counted in `forward_refused`; M stays `relayed`; a pair
   that is down keeps the row protected until its link-up. **A second origin:** with M over its per-origin share (a test-sized `quota.transit`), M's next
   forward put gets `REJECT_FULL` while a second member's forward put is `ACCEPTED`.
5. Loops: B never forwards a put from A (a `peer`) for a non-member; with A, B, C all peers, an item for nobody's member never passes hop 1; an item bounced
   back to A answers `DUPLICATE`; a hint naming A at A is refused. **Asymmetric roles (C3):** a scan whose role is not the bundle's complement answers 409
   `pair_role_mismatch` and records nothing; a `leaf` pick with `via = carrier` is refused; a put that arrives against the pair's recorded role is refused
   `REJECT_UNKNOWN_DEST`.
6. A restart of A with protected rows resumes on link-up and B deduplicates · a test-sized Transit quota answers forward puts `REJECT_FULL` while a member's
   put for a member is `ACCEPTED` · a `REJECT_FULL` row is retried on the pair's next attribution and on the next forwarder run for that pair (C16) · an
   old hub (a Layer 6 build as B) skips the TLV and answers as today: the harness builds that binary from the Layer 6 lock tag (the
   `checkpoint-layer-<n>` precedent; none exists yet for Layer 6, so the lead tags it before G9's lock run), and the item is `SKIPPED` without it unless
   `REQUIRE_G9=1` (I13).
7. A non-hub node refuses `paired_hubs`, dials nothing, keeps its gate; a deleted `fabric/settings` never rebuilds a paired hub as home hub; a half pair
   forwards nothing; no plaintext `111F` hello crosses the TEST-NET veth (capture). **Pass:** all green.

**G10, the carrier** (`carrier.sh`: gateway hubs G and G′ in two netns, each with a leaf hub, joined only through the ntfy stub; `NTFY_BIN` selects a pinned
real ntfy release, `REQUIRE_NTFY=1` for the lock run). Asserts, after R07g2 §2 Sequences B and C:
1. 5A: a forward row makes no request until an event; a netlink route added in the netns, a `Forward` event or "Flush now" makes one drain, one post and one
   drain; then no connection and no process remains (`/proc` and socket check).
2. A row commits only when G′'s `CUSTODY_ACK` returns through the reverse topic; the leaf member sees `relayed` at hop 1 only; the far member receives once
   and its `ACK` returns the same way.
3. 5B holds exactly one connection, on the one `111d-ntfy-5b` thread; a stub restart (cache lost, a `since` miss) causes one re-post and one delivery
   (`pkt_ref` dedup). **Re-posts (C6):** over several flushes of G while G′ is silent, each row is posted once; it is re-posted only after G′ flushed (one
   post per row per far-hub round), and `reposts` counts it. **Acks (C7):** a taken ack is never posted twice; a dropped ack heals by G′'s re-post and a
   `DUPLICATE`.
4. A 4,097 B body is never posted; a 413 is `Rejected`; a 429 is `Retry` with back-off; an item over 2,937 B is refused at the gate.
5. A third client polling the topic reads sealed bytes only; a frame it posts is dropped and counted; the wrong topic yields nothing.
6. The metered cap stops flushes; with the Layer 6 bridge stub reporting a licensed board, no Layer 7 path sends to it (I12); with the switch off there is no
   `connect()` at all.
7. The smoke log of §6.4 is written.
**Pass:** all green, and with `NTFY_BIN` the smoke log carries the real server's version.

**G11, TLS-RPK** (`tls.sh` plus node111 tests). Asserts: a handshake with both pinned SPKIs; a wrong client key, a wrong server key, an X.509 certificate,
TLS 1.2 and a non-Ed25519 key are each refused before any 111 byte · plain outbound to a non-private pair is refused without a recorded warning · plain
inbound members are still served · the `StreamWrap` default (`None`) leaves the `tcp-lan` tests byte-identical · a TLS peer whose SPKI matched pair X and
whose first announce names another NodeId is refused before any other 111 byte (S7) · `cargo tree -e features` shows no `aws-lc-rs` and no second crypto
provider (S10) · `REQUIRE_OPENSSL_RPK=1` runs an OpenSSL 3.2+ peer against our server and client (its RPK options *unverified from this box*). **Pass:**
all green; no cleartext announce on the veth.

**G12, the hub origin and the Layer 1 trust fixes** (`origin.sh` plus node111 tests). Asserts: T1 a validly signed `beta` manifest under the stable name is
refused · T2 a generation-1 release passes only through a valid chain from the compiled key; a broken `signature_by_previous` is refused; the floor
generation holds · T3 the root helper refuses below `min-upgrade-from` and flips nothing · `--from <dir>` stages and writes the request file; a tampered
blob is refused and staging removed · **the staging lock (S1):** a `--from` run while the daemon's check holds `state/staging.lock` exits 75 and stages
nothing, and a concurrent run stages once · **the flip (I14):** the helper's flip is root-only, so the flip assertions (T3's, and a `--from` stage that the
helper flips) are node111 tests on a scratch root, beside the update tests in `bootstrap-gate` (`1-bootstrap/crates/node111/tests/updater_stage.rs`,
`helper_guard.rs`), which sign their fixtures with minisign in CI (`.github/workflows/ci.yml:55-56`); `origin.sh` asserts staging and the request file only
· the payload of a hub origin or `--from` is fetched at `blob/<sha256>`, never at `payload.url`, and `last-update-check.json` is not written by it (S5) · a
member with no internet route updates from its hub at `origin_base` (S4); the hub serves only with `hub_role`, LAN sharing and `origin.serve`; non-GET 405;
no session needed; a missing blob 404s and the next origin is tried · with `updates.check_internet = false` no internet base is contacted (egress log) ·
the transient RSS of a staged update is recorded. **Pass:** all green.

**G7, extended:** `zero-weight.sh` unchanged, plus `idle.sh`, the no-timer gate of §3.7 (wakeups and outbound in separate windows). **Pass:** §3.7 step 4.

**CI mapping** (`.github/workflows/ci.yml`; parity with the local gates, `plan/cross-layer/ci-cd.md`): the mesh111 and node111 tests behind G8–G12 run in
`bootstrap-gate` (`1-bootstrap/scripts/check.sh`, which includes `5-messaging/scripts/check.sh`); the agent's pins in `intelligence`; contract drift (`pnpm
gen:types` leaves the tree unchanged) and the KB entry in `knowledge-contract-hygiene`. The netns harnesses and G7 stay the lead's (runners restrict
`unshare -Urnm`, `ci-cd.md:18-19`). **Root-only tests:** the harnesses run unprivileged; the known root-only `check.sh` failures on the build box are
recorded, never counted as tree faults (D29), and CI and the owner's machine run `check.sh` whole.

## 16. Pi-floor accounting

| Item | Non-hub node | Hub, nothing paired | Per paired hub | ntfy 5A | ntfy 5B | LAN-sharing node (+ the `_111._tcp` record) | Basis |
|---|---|---|---|---|---|---|---|
| processes | 0 | 0 | 0 | 0 | 0 | 0 | design |
| listeners | 0 | 0 | 0 | 0 | 0 | 0 (the record rides mdns-sd's socket) | design |
| timers | 0; −90 wakeups/min (trim); −1 request/min (probe), its wakeup delta measured | 0 | 0 (keepalive belongs to the connection) | 0 | 0 | −12/min (the 5 s IP check off) where the second netlink bind succeeds; 0 where it fails | R00 §3.1 (a); R06 G5; measured at build |
| threads | 0 | 0 | 0 | 0 (5A requests borrow `spawn_blocking`) | 1 (`111d-ntfy-5b`, only while 5B is on) | 0 | design (S6) |
| held connections | 0 | 0 | 1 | 0 between flushes | 1 HTTPS stream | 0 | design |
| wakeups/min | baseline ≈ 114 after the trim | +0 | ≈ +4.8 | per flush only | server keepalives | per address event | estimated; measured at build |
| bytes/day | −1,440 HEADs | 0 | ≈ 0.4 MB plain, ≈ 0.53 MB TLS | per message (§5.10), plus re-posts (§5.9) | unmeasured until the smoke log (the server's keepalive interval) | a few bytes per mDNS answer | estimated |
| memory | binary delta only | 0 | TLS session + share of the ≤ 16 KB cache | transient per flush | one stream | 0 | measured at build |
| disk | 0 | 0 | +32 B per forward row (`fwd_from`, V4) | `state/relay.json` | as 5A | 0 | design |
| netlink sockets | 0 | 0 | 0 | +1 while configured | +1 | +1 while mdns-sd runs | design |

The origin adds no resident cost and stores the payload per served target (measured at build). **Rule:** the build records each figure, measured on the
release binaries, in `7-relay/TASKS.md` beside Layer 6's (`111` 19.4 MB, idle RSS 19.7 MB, D29 post-lock), and any change to a non-hub column fails the gate
until the owner says otherwise.

## 17. Deferred / v1.1 (D30 F10)

| Item | What it needs |
|---|---|
| the radio form of §5 (hint custody) | an unwrapped hint-custody form (the 120 B wrap cannot plan on the 160 B floor, `M/frag.rs:64`; 131 B with the hint) and Layer 6's second-release seams A1, A3, A5, A6, A7; the hardware (D23 #4) |
| chains of hubs | a plaintext hop count: an envelope change |
| a signed forward receipt | the dest hub's `custody-sig` on request (L5 §6.2) |
| member-gateways | a nested `MAILBOX_XITEM` kind (OB F4 A′) |
| sweep-before-ack | probing every pair before answering, to keep today's `hub_not_yours` (and to end the §5.6 limit: a refused row's member keeps `relayed`) |
| a hub's own-send forward route (C4; D31 (3)) | `Route::Forward` into the hub's own forward table, local, no wire; in v1 a hub's own messages cross islands only through its own home hub (§5.1) |
| an honest "too large for the carrier" status (C15) | a new custody status or reason on the wire; v1 answers `REJECT_UNKNOWN_DEST` and counts it (§5.2) |
| Meshtastic position precision (I10) | the presence sidecar exports the Position packet's precision bits (or the channel's `position_precision`); v1 draws the coarsest cell (§10.1) |
| the heartbeat on a transport (I4) | a sealed LoRa link; until then every write is refused (§2.4) |
| an own-send carrier route, the `carrier` `LinkKind`, the JSON-lines pipe | `Route::Carrier`, per-route planning, `remove_link`, a non-blocking `Link::send`, the contract's `LinkKind` enum; the §5.9 line shapes and a supervised child per carrier |
| Nostr, LXMF propagation, iroh carriers · yggstack | §6.7; §6.6 |
| Tier 2 transfer | `OBJ_ADVERT` / `OBJ_REQ` / `OBJ_CHUNK` on the wire (L5 §11) |
| the map bucket | a Layer 8 region file, the D21 amendment, a Range route (§10.5) |
| the first epoch consumer | freeze protection, a manifest-version 2 `epoch-floor`, TLV `0x03` (§8) |
| multi-copy replication | `pkt_ref` set reconciliation (Negentropy's shape) |
| the `0x30` and attestation consumers, a Nostr text Adapter | §11; TLV `0x04`; a store table |
| the sealed registry query · the attested link key · keepalive-only presence | kinds `0x0C` / `0x0D` (§7.3) · `transport_id` `0x04` (§4.2) · a read-only `Fabric::presence()` over the in-memory `heard` set (`M/fabric.rs:1317`) |
| MeshCore adverts as a Nearby feed, the room server | a bench with boards |

## 18. Cross-layer seams & amendments

- **Layer 5 (locked v1.2), additive:** the twelve spec amendments of §5.3 (D30 F5). Beside them, code seams that change no Layer 5 spec text and no wire
  byte (§19 "Code seams", D31 (1), (2)): the `tcp-lan` stream parameter (§4.4; it builds L5 §8's reserved row `L5:366`, whose text, like `:361`'s, stays)
  and the read-only `Fabric::attributed_on` (§2.1). No mesh111 role-bit constant is added (bits 3–5 live in node111, §7.2). Named now, made in v1.1 with
  their consumers: ext TLVs `0x03 epoch_beacon` and `0x04 attestation`, kinds `0x0C` / `0x0D`, shim public sub-type 3, and the presence sidecar's export of
  Meshtastic position precision bits (`5-messaging/sidecars/meshtastic-presence/`, §10.1 rule 4). L5 §12 delegated the `0x30` format here (`L5:489-491`).
  L5 §19's Layer 7 hand-offs (presence, A+, the KB consumer, `appdata` TLVs, public frames) are closed by §2, §5, §9.4, §7.2 and §11.
- **Layer 6 (locked v1.0):** `NetlinkEvents::new` takes a group mask, plus the second socket (§3.3) · the WAN loop removed and the `gateway` row redefined
  (§3.5) · the three 2 s ticks parked (§3.6) · mdns-sd's IP check on events (§3.3) · `[order]` read as path preference (§19) · `receipt.gateway` documented
  as an install fact the daemon never reads (`NI/install/mod.rs:125-128`, `:189`; `NI/install/receipt.rs:27`) · G7 gains `idle.sh` · the `reticulum` Nearby
  source gains `rnpath` hop counts, read only through the human-only `POST /api/v1/nearby/hops` and the `rnstatus` runner (§10.1, §13.1; I6) · the `rns`
  heartbeat stays Layer 6's `PATCH /bridge/settings` interval (§2.4) · the bridge stub (`NODE111_BRIDGE_STUBS`) serves G8 and G10's licensed-board items.
- **Layer 2 (locked v2):** `transport_id` `0x04` `111-link-ed25519` reserved (§4.2) · domain tags `111/carrier-topic/v1`, `111/epoch/v1`, `111/broadcast/v1`
  · the epoch unit (a day) and source (§8), closing §13's hand-off, with the transferable beacon form `issuer ‖ epoch ‖ sig` (§8.1, C17) · the attestation
  carriage rule (§11.2) · `appdata` role bits 3–5 ("Layer 7 appends", A8; bits only in v1, defined in node111, §7.2) · one derivation behind the identity
  handle for the topic key (the A9 pattern).
- **Layer 1 (locked v2.2):** T1–T6 (§9.5) · the verbs `111 update --from` and `111 fabric pair` (`NI/cli.rs:196-210`) · the origin list and
  `updates.check_internet` · `state/origin/`, `state/relay.json`, `state/staging.lock` (§9.2, S1), `secrets/link-key`, and in v1.1 `fabric/epoch.json` ·
  the hub-origin and `--from` payload fetch at `blob/<sha256>` and `record_success` for internet checks only (§9.2, S5) · `/origin/v1/*` in the machine
  contracts beside `/health` (`1-bootstrap/README.md:137`; §9.1, I7) · direct `rustls` and `tokio-rustls` with their sign-off row ("Layer 7 additions"
  under "Dependency policy" in `1-bootstrap/TASKS.md`) and the `LICENSING.md` §5 counts (§4.5, S10) · text: "Notice: internet only in v1 (mesh gossip
  arrives with Layer 7)" (`1-bootstrap/README.md:159`) → "Notice: the internet bases hourly (user-off) and the home hub's origin on link-up (Layer 7 §9.3);
  no gossip" · "KB git-sync is Layer 7" (`:160`) → "The KB rides the release payload (Layer 7 §9.4); nodes never run git" · "Connectivity probes hit
  first-party endpoints only (`install.111.network/204`)" (`:167`) → "The installer's one-time probe hits first-party endpoints only; the daemon runs no
  recurring probe (Layer 7 §3.5); the updater's check is its one recurring contact, visible and user-off" · the seams line "Layer 7: gateway re-evaluation,
  KB git-sync, manifest gossip" (`:252`) is delivered as above.
- **Layer 3:** the registry of §7 replaces "Layer 7 broadcasts hub role, model, load" (`3-intelligence/README.md:48`, §19) · `presence.read`,
  `gateway.read`, `relay.propose` (17 → 20 tools) · offload keeps the private IP literal rule.
- **Layer 4:** the contract additions of §13 and its six-item pin list (§13.2) · `/origin/v1/*` added to the conventions' "Not gated" list
  (`4-interface/api/README.md:143`), outside `openapi.yaml` · the gateway row's mock fixture (§3.5) · `/settings/l7` · the radar in the `nearby` chunk · the
  D21 map amendment recorded now, made in v1.1 (§10.5).
- **Layer 0:** a new `candidate` entry `projects/ntfy.md` with its gate gaps; `concepts/store-and-forward-states.md` gains `forwarding`;
  `concepts/preferred-networks.md` reads `[order]` as preference.

## 19. Plan corrections recorded

**From the brief §4 (OB §4 items 1–10).**
1. README §7 "Nodes continuously announce reachability through heartbeats." → "Nodes make their reachability known on every link-up and in every card, and
   show who they heard and how; a periodic heartbeat is the owner's choice per transport (Layer 7 §2)." Met by L5 A6 and the Pi floor.
2. `receipt.gateway` is always `true` and never read (documented, §18 L6) · `[order]` sorts the display only (`N/transport/interfaces.rs:87`) ·
   `ROLE_LORA_ATTACHED` is defined and never set (`M/contacts.rs:146`) · mdns-sd advertises the dashboard only (`N/dashboard/mdns.rs:1-12`) · the
   phone-relay QR joins rnsd's Backbone, not the fabric.
3. L5 §9/§21 "Hub-to-hub replication (A+) … per-(dest) summary vectors (the §4 prefix + ranges form)" cannot be built: a hub never sees `n` (amendment 3).
   L5 §9 "≈ 90 B" is 120 B as built (amendment 4).
4. The updater has no `update` verb, two hard-wired bases, absolute payload URLs; `channel` is never compared; the keychain is parsed, not verified;
   `min-upgrade-from` lives only in the installer; the `[kb]` pin is unconsumed (§9).
5. `LICENSING.md:56` "Exact text is not in the repo (unverified — read upstream LICENSE)" → the Reticulum License text is verified (R03 §5 q5); the helper's
   own licence stays pending the owner's contact. `bao-tree` is pinned at 0.16 while 0.17.0 shipped on 2026-10-22 (note before the v1.1 Tier 2 build);
   `minisign-verify` 0.3.0 exists.
6. L3 README:48 "(Layer 7 broadcasts hub role, model, load); traffic rides Layer 5's encrypted fabric" → "(Layer 7's registry: the hub role as a signed bit,
   model and load by a sealed query, Layer 7 §7); in v1 offload rides the LAN binding with pairing and a private IP literal."
7. G7 measured no timers or outbound traffic; the required configuration ran ≈ 205 wakeups and one outbound `HEAD` a minute (§3.6, §3.7).
8. D21 needs an amendment if a map ships: an exempt bucket, the fonts precedent (§10.5).
9. `plan/cross-layer/parked.md` P5 is closed: no Rust Reticulum in `111d`, ever (R03 §3 finding 5).
10. Locked text: D19 b and L5 §9 "A+ follows without redesign" → single-copy push · L3 README:48 (item 6) · L1 "KB git-sync is Layer 7" (§18 L1) · README §6
    "attempts to join them in sequence … falls back through the list" → "keeps every enabled network on, in parallel, and tries them in the user's preferred
    order per peer" · MeshCore's hop cap is 63 (a 6-bit `path_length`), not 64.
11. The brief's F4 A reaches the carrier "over a loopback JSON-lines pipe, the LXMF helper's pattern" (OB §2 F4 A and §3), and D30 F3 says "no new
    process": v1 resolves both by running the carrier in-process behind the hand-off API (§5.9, §6.1); the pipe is the v1.1 form for an external carrier
    program (§17; D12; S27, I16).

**Where the code overrules the brief or a report.**
- **C1** L5 has no default custody quota (unset = no cap, `M/store/prune.rs:543-560`); the Transit default is a quarter of the 512 MiB high watermark
  (`:79`) unless a custody quota is set (§5.8).
- **C2** `tcp-lan` takes a concrete `TcpStream` (`M/link/tcp_lan.rs:422-432`): TLS needs a seam in mesh111 (§4.4).
- **C3** `contacts.last_seen_seq` moves only on an upsert (`M/store/rows.rs:685-688`): presence comes from the fabric's events (§2.1).
- **C4** The manifest refuses unknown keys and other versions (`NI/manifest.rs:62-64`, `:211`, `:343-345`): the epoch floor rides the payload (§8.1).
- **C5** R07g2 §2 Sequence B commits on the carrier's accept; a held fragment is not re-sent in-session (`M/fabric.rs:2207-2209`; `M/frag.rs:146-147`,
  `:170`), so a carrier loss after such a commit would strand the item: commit on the far hub's `CUSTODY_ACK` (§5.9).
- **C6** The dashboard is LAN-bound only with LAN sharing (`N/dashboard/mod.rs:259`): the origin needs it, plus its own switch (§9.1).
- **C7** `rebuild_home_hub` would pick a paired hub (`N/fabric/settings.rs:364-380`), and today's `on_enroll_ok` writes a cursor for it
  (`M/fabric.rs:3122-3126`): amendment 10 excludes paired hubs from both.
- **C8** `AppData::parse` reads bytes 4..12 as the hint (`M/contacts.rs:160-171`): role bits only in v1 (§7.2).
- **C9** Card bytes are the announce (`ID/announce.rs:40-49`), the `111HUB:` bundle splits into fixed fields (`N/fabric/cli.rs:74`) and mailbox payloads
  reject trailing bytes (`M/mailbox.rs:16-18`): beacons and attestations ride ext TLVs and API objects (§8.3, §11.2). The same holds for the link key: a
  44 B SPKI cannot ride the card (`appdata` ≤ 32 B), so it is pinned from the new `111PAIR:` bundle, not the hub card D30 F3 names (§4.2; D14; S21).
- **C10** No existing request carries a registry query without a Layer 5 change (§7.3).
- **C11** The fetcher forbids `http://` in production (`NI/util/http.rs:4-7`): T6 (§9.5).
- **C12** The updater has no user-off switch (`N/mod.rs:933-952`): `updates.check_internet` (§9.3).
- **C13** The member side reads `CUSTODY_ACK` only from its home hub (`M/fabric.rs:3030-3032`): amendment 11 adds the paired-hub arm.
- **C14** README "Category 5": "Any node across Categories 2, 3, or 4 that establishes active reachability to the public internet becomes a gateway." → "In
  v1 a gateway is a hub whose owner turned on a carrier (Layer 7 §3); a member-only phone carries nothing for others."

**Deviations from D30 (each needs the owner's acknowledgement).** D1 the `tcp-lan` stream seam: a code seam beside F5's twelve, not a spec amendment (C2,
§4.4; D31 (1); flip: tunnel-only internet pairing in v1, TLS-RPK in v1.1) · D2 presence comes from fabric events, `last_seen_seq` is not bumped per packet
(C3) · D3 a forward row commits on the far hub's `CUSTODY_ACK`, not a carrier's accept (C5) · D4 the epoch floor rides the payload, not a manifest key (C4)
· D5 the Transit default is 128 MiB (C1), with a per-origin share of `quota.transit / 8`, 16 MiB (panel C1; §5.8; D31 (6)) · D6 the sealed query is pinned
now and built in v1.1 (C10; D31 (4)) · D7 the origin also needs LAN sharing and `origin.serve` (C6) · D8 the netlink address and route groups ride a second
socket opened only for a consumer, through a second `netlink::debounced` instance (2 s) that feeds only the carrier check and the mdns-sd re-arm; the
spine's instance is untouched (§3.3) · D9 v1 builds nothing for the epoch (§8.5) · D10 two hubs must meet once over `tcp-lan` to pair, and the second
`ENROLL` rides the connection the first hub opened (§4.6; D31 (5)) · D11 a carrier carries sealed hub-to-hub frames, not raw `dest ‖ item` (§6.1), and its
consent copy names the outer header's short tag of the two hubs' ids (§6.2) · **D12** the carrier runs in-process in v1; the brief's loopback JSON-lines
pipe is the v1.1 external-carrier form (§5.9, §17; brief item 11) · **D13** D30 F7's "role bits and TLVs" in `appdata` is role bits only in v1 (C8, §7.2) ·
**D14** D30 F3's link key "pinned from the hub card" is pinned from the `111PAIR:` bundle (C9, §4.2) · **D15** D30 F1's per-transport heartbeat switch
exists and is visible but inert in v1: every write is refused until a sealed LoRa link exists (v1.1), and `rns` stays Layer 6's interval (§2.4; I4) ·
**D16** D30 F7's gateway willingness "advertised to members and contacts only, sealed (the hub offer, INTRO, cards)" is, in v1, the offer at enrolment and
the `forwarding` reason; members enrolled earlier and every contact learn nothing until the v1.1 sealed query (§7.5, §3.8; I5).

**Code seams beside the twelve (D31 (1), (2); not Layer 5 spec amendments, each read-only or byte-identical; listed again in the lock entry).**
1. The `tcp-lan` stream parameter (§4.4; D1, Q1): `TcpLanConfig.wrap`, default `None`, byte-identical; `serve` generic over the stream; `inbound` returning
   the matched pair (S7). mesh111 gains no dependency.
2. `Fabric::attributed_on(peer) -> Option<LinkKind>` (§2.1; I1, Q2): read-only, no state, no wire byte. **Flip:** one `111` fact per contact with the
   transport unknown (`evidence[].transport = "111"`), and no accessor.
3. The `PairedHub {node_id, role, via}` record shape (panel C9): inside amendment 10, the one record the fabric's gate reads; byte-identical with no pairing.
4. The role-bit names (I9): bits 3–5 and their name map in node111 (`N/fabric/sealer.rs`, `roles(s)` at `N/fabric/mod.rs:334-343`); no mesh111 change.

**Owner forks (D31 (1)–(6), each applied as the lead recommends; a different answer lands as v0.3).** Q1 the stream seam as a code seam (§4.4, D1) · Q2
code seams beside the twelve, with `attributed_on` (§2.1, above) · Q3 a hub's own messages cross islands only through its home hub in v1 (§5.1, §17; panel
C4) · Q4 the sealed registry query in v1.1 (§7.3, D6) · Q5 hubs meet once over `tcp-lan` to pair (§4.6, D10) · Q6 refused forward rows end and Transit has
a per-origin share (§5.8; panel C1). **Numbers kept as the lead's defaults** (each marked "a default; the owner may change" where it is set): the Termux
fallback poll 15 min (§3.4) · ntfy `cache-duration` 168 h (§6.2) · the epoch origin day 2026-01-01 (§8.1) · the no-timer pass line 1.10 × the first
recorded baseline (§3.7) · the DNS-SD browse window 5 s (§7.1) · Transit 128 MiB, per-origin 16 MiB (§5.8).

## Changelog v0.1

- 2026-11-01: first draft from D30, the options brief and research R00–R07g2, for the adversarial panel.

## Changelog v0.1 → v0.2

2026-11-02: the lead's triage of the three-judge panel (`plan/layers/7-relay/review/triage.md`) and D31 applied; one line per finding.

- **C1** §5.2, §5.5, §5.6, §5.8, §5.9, §14, G9: a forward row every target refused is marked refused (committed, evictable, `forward_refused`); a per-origin
  share `quota.transit / 8` (V4 gains `fwd_from`); protected Transit rows count toward the high watermark; the member keeps `relayed` (Q6).
- **C2** §5.2, §5.5, §5.6, G9: a no-hint put is held and `ACCEPTED` when any target exists, then probed after the ack; none ⇒ `REJECT_UNKNOWN_DEST`; the
  hinted-unmatched clause matches; `ACCEPTED` means "held and will be offered".
- **C3** §4.6, §5.1, §5.2, §5.7, §13.1, G9: the bundle carries the shower's proposed role and link; the pick must be the complement or 409
  `pair_role_mismatch`; carrier pairs `peer` only, leaf pairs `tcp` only; a put against the record is refused.
- **C4** §5.1, §17: a hub's own messages cross islands only through its home hub in v1; the own-send forward route is v1.1 (Q3).
- **C5** §5.3 amendment 2: the whole scope sentence replaced, with the `queued` / `sent` reading the code gives.
- **C6** §5.9, §6.2, §13.1, G10: re-posts only after a newer far-hub frame, on Flush now, on a `since` miss or once after a restart; `reposts` per pair.
- **C7** §5.9, G10: acks consumed on take and never retried; `flush` returns `Vec<(Option<u64>, ForwardDone)>`.
- **C8** §5.3 amendment 6, §5.8: `class_bytes` and `evictable` split by `fwd` at `prune.rs:484-509` and `:228-245`; `is_refusable`, `has_quota` and
  `custody_put` named.
- **C9** §4.6, §5.3 amendment 10: one `PairedHub {node_id, role, via}` record in `FabricSettings`; the daemon's fields beside it.
- **C10** §11.1: 80 B fixed, 116 B with a pointer; 79 B or 43 B of text.
- **C11** §5.3 amendment 8: `API:888` widened beside `:903`.
- **C12** §5.4: the `n_ext` row reworded; 11 B alone, 10 B beside another TLV.
- **C13** §6.2: re-read frames are dropped as stale by the L2 window, not by `pkt_ref`.
- **C14** §5.5, §14: a probed pair sees `dest`, size and `pkt_ref`; a row in the exposure table.
- **C15** §5.2, §6.1, §13.1, §17: too large for the carrier is `REJECT_UNKNOWN_DEST`, counted and shown with its sizes; the honest status is v1.1.
- **C16** §5.9, G9: a `REJECT_FULL` row retries on the pair's next attribution and every forwarder run for it.
- **C17** §8.1–§8.3: the transferable beacon carries the issuer (104 B, an API object); `epoch.json` is written only when the day advances.
- **C18** §5.2: the stale hint is the repeated TLV; `hinted_elsewhere` cited beside `L5:333`.
- **S1** §9.2, §18, G12: a new `state/staging.lock` (flock, `O_CLOEXEC`) over fetch–stage–request in the loop and the verb; held ⇒ exit 75.
- **S2** §3.3, G8: the pinned re-arm (interval 1, one 1,500 ms sleep, interval 0, in the task that owns the handle); G8 asserts 0 within 2 s.
- **S3** §1, §3.3, §16: the interval goes to 0 only after the second netlink bind; on failure the default 5 s check stays.
- **S4** §9.1, §9.3, G12: `origin_base = http://<home hub host>:11180/origin/v1/`; every LAN-bound listener serves; the Host guard stated.
- **S5** §9.2, §18, G12: hub and `--from` payloads at `blob/<sha256>`, `payload.url` ignored; `record_success` for internet checks only.
- **S6** §1, §6.2, §16: the carrier's own `ureq::Agent`; 5A in `spawn_blocking`; 5B on one named thread with a read timeout; a threads row.
- **S7** §4.4, G11: `inbound` returns the matched pair; a mismatched first announce ends the connection.
- **S8** §9.1: the open set cited at `routes.rs:16-18`; `/origin/v1/*` joins it, GET only, no CSRF, same CSP.
- **S9** §4.3: no SNI is sent for an IP literal; only the pin binds.
- **S10** §4.5, §18, G11: the `tokio-rustls` feature claim marked unverified; `cargo tree -e features` asserted; the sign-off row and §5 counts named.
- **S11** §6.2: `since=all` marked unverified.
- **S12** §6.3, §9.5 T6: the private-address `http://` carrier server listed with T6; its token crosses that LAN in clear.
- **S13** §6.2: the consent copy names the short tag of the two hubs' ids.
- **S14** §3.2, §6.2: the ntfy check is the drain; a check that returns frames starts the flush.
- **S15** §3.3: a second `netlink::debounced` instance; the spine's untouched.
- **S16** §3.7: wakeups measured with no tracer, outbound in a separate window.
- **S17** §16: 5B bytes per day unmeasured until the smoke log.
- **S18** §3.5: the catalogue's `requires` copy and the UI mock fixture named.
- **S19** §6.3: one table of every Layer 7 setting with its default and file.
- **S20** §2.4, §6.3: the carrier and heartbeat are switches in the Layer 6 §2.1 rule-2 sense, not components.
- **S21** §4.2, §19 C9: the link key pinned from the bundle, not the card, recorded.
- **S22** §4.6: the second `ENROLL` rides the connection the first hub opened.
- **S23** §7.7: the lost-hub browse gated on the hub connection down for one back-off cycle.
- **S24** §9.5 T2: the order parse → walk → verify → floor; one function for daemon, helper and installer.
- **S25** §3.6: a state-change `Notify` on the services registry; the Meshtastic monitor parks on the devices `kick`.
- **S26** §16: the probe's removal is −1 request a minute; its wakeup delta measured.
- **S27** §19 item 11, D12: the carrier in-process in v1, the JSON-lines pipe the v1.1 external form.
- **I1** §2.1–§2.3, §19, G8: a read-only `Fabric::attributed_on(peer)` gives `tcp-lan` or `hub`, recorded as a code seam (Q2); its honest limit stated.
- **I2** §4.1, §4.6, §12, §13.1: every Layer 7 consent in the pinned `ConsentRequired` shape; four actions appended; `pair_warning_required` dropped.
- **I3** §13.1, §13.2: `needs_hub` for "not a hub"; codes appended to the enum; the six-item pin list.
- **I4** §2.4, §12, §13.1, §13.2, §19 D15, G8: every heartbeat write refused (`no_such_transport`; `rns` 409 `settings_locked` pointing at
  `PATCH /bridge/settings`); `presence.interval` refused `unsupported_action`.
- **I5** §3.8, §7.5, §19 D16: members-only gateway advertisement in v1 is the enrolment offer and the `forwarding` reason.
- **I6** §10.1, §13.1, §13.3, §18, G8: a human-only `POST /api/v1/nearby/hops` runs one `rnpath` through the `rnstatus` runner; never on a GET.
- **I7** §9.1, §18: the gate cited at `auth.rs:1023-1029` and `routes.rs:469`; `/origin/v1/*` in the "Not gated" list and L1's machine contracts, outside
  `openapi.yaml`.
- **I8** §2.3: the `111` source rides the bridge sink's 512-row cap.
- **I9** §7.2, §18, §19: bits 3–5 and their names in node111; no mesh111 change.
- **I10** §10.1, §17, §18: v1 draws the coarsest cell; the sidecar's precision export is v1.1.
- **I11** §3.7, §15: separate windows; `strace` an optional third window with its skip rule.
- **I12** §2.4, G8, G10: the 9-min floor and the licensed ban as validator unit tests; the licensed path through the Layer 6 bridge stub.
- **I13** G9: a committed tape under `7-relay/verify/tapes/`; the old hub built from the Layer 6 lock tag, else `SKIPPED` unless `REQUIRE_G9=1`.
- **I14** G12: the flip assertions are node111 tests on a scratch root; `origin.sh` asserts staging and the request file.
- **I15** §13.1: `POST /relay/pair` and `DELETE /relay/paired/{node_id}` loopback-only.
- **I16** §19: the pipe resolution (item 11) and the deviations D12–D16 numbered and cross-referenced.
- **D1** §4.4, §18, §19: the stream seam recorded as a code seam with its flip (Q1); the L5 §8 row rewording dropped (D31 (1): no Layer 5 spec text).
- **D5** §5.8, §19: 128 MiB total with a 16 MiB per-origin share.
- **D6** §7.5, §19: the sealed query stays v1.1 as drafted (Q4); cross-referenced.
- **D8** §3.3, §19: S15's wording.
- **D10** §4.6, §19: S22's rule (Q5).
- **D11** §6.2, §19: S13's consent copy.
- **D2**: accepted as recorded; text unchanged.
- **D3**: accepted as recorded; text unchanged.
- **D4**: accepted as recorded; text unchanged.
- **D7**: accepted as recorded; text unchanged.
- **D9**: accepted as recorded; text unchanged.
- **Numbers** §3.4, §3.7, §6.2, §7.1, §8.1, §5.8, §19: each marked "a default; the owner may change".
