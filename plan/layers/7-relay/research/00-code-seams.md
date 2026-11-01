# Layer 7 research, lens 0: as-built code seams

Scout: opus seams lens (no web), 2026-11-01, base `3bf545b`. Every claim below was read on that base. Path prefixes:
`M/` = `5-messaging/crates/mesh111/src/`, `N/` = `1-bootstrap/crates/node111/src/daemon/`, `NI/` = `1-bootstrap/crates/node111/src/`,
`ID/` = `2-identity/crates/id111/src/`, `A/` = `3-intelligence/crates/agent111/`, `UI/` = `4-interface/ui/src/`,
`API` = `4-interface/api/openapi.yaml`, `L5`/`L6` = the layer READMEs.

## 1. Summary

- **Six of the seven Layer 6 fabric findings still hold.** Layer 6 added store V3, `Class::Foreign` and about 35 `foreign_*` calls,
  all of them readable text. It added **no second `Link` driver.** Sealed 111 traffic still travels only over `tcp-lan`, so Layer 7's
  "delivery paths from mesh links + gateways" has **one** sealed bearer today. Sealed carriage over LoRa is Layer 6's "second release".
- **Gateway detection already exists, and it is resident.** A WAN probe sends HTTPS `HEAD` to `install.111.network/204` **every 60 s**
  and feeds the `gateway` service row. The installer's `receipt.gateway` is a trap: it is always `true`, and nothing ever reads it.
- **The required configuration already runs about 11 always-on intervals, about 205 wakeups a minute** (§3.1 a). A 111 heartbeat adds
  almost no CPU next to that. Its real costs are LoRa airtime and, when every beat is freshly minted, an fsync on the shared
  announce/epoch counter.
- **An announce on a 160 B LoRa frame leaves only 10 B of `appdata` room** (18 B without the hub hint). Re-sending the stored
  announce proves no liveness (L2 A6 stale rule).
- **Hub A+ cannot use §4 summary vectors as written.** Hubs see `dest`, `pkt_ref` and their local `custody_seq`, never `msg_id`. The
  cheapest A+ reuses the built `MAILBOX_FETCH`/`COMMIT` cursor shape between hubs.
- **The updater's trust does not depend on the carrier:** minisign, the floor, and sha256 pins. A hub could serve the identical
  bytes. What stands in the way is the absolute HTTPS URLs, not the crypto. The UI's lazy budget has about 36 KB left for a map.

## 2. Candidates

None: lens 0 reads code only (adoption candidates are lenses 1–5).

## 3. Findings that change the plan

1. **The fabric's routing is unchanged since the Layer 6 report.**
   - `route()` returns Direct, Hub or None (`M/fabric.rs:1821-1849`).
   - `sendable_conn` takes the first attributed connection (`:1780-1791`).
   - The plan is fixed at queue time (`:1852-1872`, `:3169-3170`). `seal_data` returns `None` when a fragment exceeds the room, and
     `pump_direct` skips it (`:1968`, `:2141-2160`).
   - No shim codec exists, and every frame gets `hop_limit: None` (`M/link/mod.rs:148,155-176`).
   - INTRO works over `tcp-lan` only (`M/fabric.rs:2552-2558`).
   - Only `add_link` exists, no remove (`:845`, `:1587`).
   - `Link::send` is awaited in the actor (`:1413`).
   - `Dir::Transit` is a schema value. It is written once, by a store test (`M/store/rows.rs:137`, `M/store/tests.rs:1416`).

   Only two types implement `Link`: `TcpLan` (`M/link/tcp_lan.rs:579`) and the test double `LossyEnd` (`M/link/lossy.rs:241`). The
   `KissModem`, `MeshtasticClient`, `RnsBridge` and `Ble` kinds are enum values only (`M/link/mod.rs:228-239`). L6 defers "sealed 111
   carriage over foreign radios" and "the 111 KISS modem link" to its second release (`6-transport/README.md:42-44, 800-803`).
2. **Layer 6 changed the store.** `SCHEMA_VERSION = 3` with migrations V1–V3 (`M/store/schema.rs:53,58,219-310`). `Class::Foreign = 9`
   (`M/store/rows.rs:50-56`). Typed `foreign_*` calls run from `M/fabric.rs:4012` to `:4367`. None of these touch sealed routing.
3. **A WAN probe is already resident** (`N/services/probe.rs:9-12,46-47,540-557`). It sends HTTPS `HEAD` every 60 s with a 3 s
   timeout, and only an exact 204 counts. The URLs are `https://install.111.network/204`, then `static.111.network/204`
   (`NI/install/preflight.rs:14-15`; `NODE111_PROBE_204` overrides them, `probe.rs:243-256`). The result is `GatewayFacts{online,
   url, checked_at, error}` (`N/services/mod.rs:473-481`), shown as service `gateway` "Internet gateway", `wan_ok` / `wan_offline`
   / `wan_unknown`, `interval_sec: 60` (`:340-348, 1044-1074`). The contract lists `gateway` in `ServiceId` (`API:602`). This is the
   only outbound request every node makes on a timer besides the updater, and it is already in the Pi-floor baseline.
4. **The installer's gateway bit is dead data.** The installer sets `facts.internet = Some(true)` unconditionally, because the
   preflight already fetched the manifest (`NI/install/mod.rs:124-128`). The bit then lands in `receipt.gateway`
   (`NI/install/receipt.rs:1,27`; `/var/lib/111/receipt.json`, root 0644). No daemon code reads the receipt: a grep for `receipt`
   outside `install/` finds only comments.
5. **Timer baseline (required configuration, all components off).** See the table in §3.1 (a). It has about 11 always-on intervals.
   The listener gate does not count them: `6-transport/verify/zero-weight.sh:1-15` checks processes, listeners and RSS only. Nothing
   measures wakeups or outbound requests, so the Pi-floor clause "no periodic poll" has no gate.
6. **When announces are emitted.**
   - They are sent first in every `tcp-lan` handshake (`M/link/tcp_lan.rs:10-18`).
   - They ride as an INTRO card (`M/fabric.rs:1995-2001`) and as card-request answers.
   - They are always the stored bytes (`M/sealer.rs:59-60`). The daemon re-mints only when the `appdata` changes
     (`N/fabric/sealer.rs:222-240`, called at start `N/fabric/mod.rs:476-481` and on a settings change `:912-922`).
   - There is no periodic announce. The `tcp-lan` keepalive is an empty frame every 25 s on each open connection
     (`M/link/tcp_lan.rs:27-29,125`).
7. **How much `appdata` room is left.**
   - An announce is 137 B plus 0–32 B of `appdata` (`ID/announce.rs:42-49`).
   - v1 `appdata` is `[1][roles u16][0]`, plus an optional 8 B hub hint, so 4 or 12 B (`N/fabric/sealer.rs:139-148`;
     `M/contacts.rs:141-171`).
   - Role bits 0 (mailbox) and 1 (always-on hub) are set (`N/fabric/mod.rs:334-343`). Bit 2 `ROLE_LORA_ATTACHED` is defined and never
     set. Bits 3–15 are free.
   - The default MTU is 160 B, shim included (`M/link/mod.rs:34,67-68`), so an announce on a LoRa frame is at most 159 B. That leaves
     22 B of `appdata`: **10 B free** with the hub hint, 18 B without.
   - A replayed stored announce is accepted with no state change (`2-identity/README.md:348-351`), so it is not a liveness proof. A
     fresh heartbeat needs `next_announce_counter`, which fsyncs (`M/sealer.rs:53-54`). That counter is also the §4 epoch source.
8. **Presence of 111 peers is invisible.**
   - Nearby lists only foreign sources: `kind_for` admits `lxmf → reticulum` only (`N/dashboard/api_v1/nearby.rs:54-56`), plus the
     Meshtastic source (`N/meshtastic/presence.rs:481-498`).
   - `NearbyNode` has no local "heard at"; `last_heard_hint` is the remote node's own clock (`API:1096-1110`).
   - The fabric keeps an in-memory `heard: HashSet<NodeId>` (`M/fabric.rs:1317`) and `contacts.last_seen_seq`, a monotone `local_seq`
     (`M/store/schema.rs:76`). Neither is exposed as presence.
   - No 111 peer is discovered on a LAN. `tcp-lan` dials only configured addresses: the home hub (`M/fabric.rs:1613-1618`) and
     `dial_peer` (`:3767-3776`). mDNS announces only `_http._tcp 111.local` for the dashboard (`N/dashboard/mdns.rs:1-12`).
   - A non-hub node's `tcp-lan` 11183 is loopback-bound (`5-messaging/README.md:63-64`), so nobody can dial it.
9. **`[order]` is display-only.** `DEFAULT_ORDER = ["111","lxmf","meshtastic","app:deltachat","app:simplex"]`
   (`N/transport/interfaces.rs:87`). It is read once, to sort the `bridge` SSE snapshot (`N/bridge/core.rs:1000-1003`). No
   join-in-sequence or fall-back logic exists.
10. **The netlink watch hears link up/down only.** It subscribes to `RTMGRP_LINK` only (`N/transport/netlink.rs:18`) and drives the
    rnsd re-render with a 2 s debounce (`N/transport/mod.rs:498-517`). An "internet appeared" trigger needs `RTMGRP_IPV4_ROUTE` /
    `RTMGRP_IPV4_IFADDR`. That is the same kind of socket, an additive group, and adds no timer. Whether Termux allows the bind is
    (unverified).
11. **The updater trusts the bytes, not the carrier.**
    - The order is fetch, minisign, floor, then pins (`N/updater/fetch.rs:100-150`). The payload is pinned by sha256 and size
      (`NI/manifest.rs:154-159`), and the staged tree is re-verified against `share/payload.manifest` (`N/updater/stage.rs:1-4`).
    - Inputs: `MANIFEST_PUBKEY` is compiled in (`N/updater/mod.rs:18-19`). The bases are `https://install.111.network/stable`,
      falling back to GitHub releases (`fetch.rs:18-20`). The files are `channel-111-stable.toml` and `.minisig` (`:90-91`), and the
      payload URL is **absolute** (`PinnedArtifact.url`). `Fetcher` is meant for "https first-party only" (`NI/util/http.rs:4-7`). A
      success writes `state/last-update-check.json` (`fetch.rs:205-215`).
    - Cadence: every 1 h; a failure doubles from 60 s up to a 24 h cap (`N/updater/mod.rs:22-23,115-150`). Offline nodes therefore
      keep trying.
    - The KB is `current/share/kb/`, inside the payload, verified at start (`N/kb.rs:1-3,33-40`).
12. **The `KB`/`UPDATE`/`PATCH` kinds exist but nobody produces them.**
    - They are kinds `0x11`–`0x13` (`M/envelope.rs:509-516`), marked "built" (`:601-607`) and SIGNED (`:571-576`).
    - The fabric's only public send is `send(to, text)` (`M/fabric.rs:856`).
    - On receive they go through `on_data` like a `MSG` (`:2596`).
    - "Public class" only means the signature binds `dest` = zeros (`:578-585`). The frame is still sealed to one recipient.
    - `Broadcast = 0x30` is refused as not built (`:522-523`, `:1926-1927`). Its format "is pinned with Layer 7's presence frames"
      (`5-messaging/README.md:489-492`).
    - Tier-2 objects are deferred (`:859-861`).
13. **The custody that A+ would replicate.**
    - Custody table: `(custody_seq AUTOINCREMENT, dest, pkt_ref UNIQUE, bytes, committed, local_seq)` (`M/store/schema.rs:155-163`).
      `pkt_ref` is BLAKE2b-128 over the exact bytes (`M/custody.rs:4-9`), used for dedup (`M/store/rows.rs:1014-1017`).
    - Member cursors: `cursors(hub, cursor)` (`schema.rs:165-168`). `ENROLL_OK` carries `custody_seq_floor` (`M/mailbox.rs:13,195`;
      set `M/fabric.rs:3017`, read `:3124`; computed in `rows.rs:1112-1117`).
    - The gate: the sender must be enrolled, and `dest` must be enrolled and not the hub (`M/fabric.rs:2830-2833`). "Enrolled" means
      the contact's trust is `Enrolled` (`:2507-2509`).
    - The §4 vectors are `(epoch, prefix, ranges)` per (peer, stream) (`M/store/rx.rs:134-143`). A hub never sees those `n`, because
      the items are sealed to `dest` (`5-messaging/README.md:409`). So the A+ sentence at `:406-408` cannot be built as worded.
14. **The attestation bytes exist, but nothing gossips them.** The record is 202 B (`ID/attest.rs:6-10`). `MeshEpoch(u64)` is stored
    and ordered, but expiry is never evaluated (`:41-47,108-131`). `BridgeVerifier` is an in-memory table for `0x02` / `0x03`, and
    `0x01` nostr fails closed as "Layer 7's" (`N/bridge/attest.rs:1-31`). There is no store table, no kind and no publisher (L6
    defers gossip: `6-transport/README.md:804-805`).
15. **There is an interval precedent.** LXMF announces "on a user-set interval (default 60 minutes, `0` = events only)"
    (`6-transport/README.md:334`). As built, on-request announces are bounded to one per 60 s per helper run
    (`N/bridge/lxmf.rs:73,909`). It applies only while the component is enabled.
16. **UI budget headroom.** Each lazy chunk may be at most 64 KB gz, and the lazy total at most 256 KB
    (`4-interface/ui/scripts/check-budget.mjs:1-7,13-14`). The last recorded lazy figure is **219.8 KB** (`6-transport/TASKS.md` C2
    row), which leaves about 36 KB of lazy space for radar plus map plus tiles code.
17. **Offload discovery has a narrow input.** Offload takes `model_base` as an IP literal, private or loopback only, with no DNS
    (`A/src/config.rs:324-335`). A Service Discovery Registry entry must resolve to that form. L3's README says the "traffic rides
    Layer 5's encrypted fabric" (`3-intelligence/README.md:48`), but the as-built client is plain HTTP to a LAN IP.

### 3.1 Seam tables

**(a) Presence: sources, states, "last heard", and the timer baseline**

| file:line | What exists | Seam | Layer 7 adds or changes |
|---|---|---|---|
| `N/dashboard/api_v1/nearby.rs:1-24,40-56,83-140,221-237` | `BridgeNearby` push/clear, caps 512 nodes / 8 sources, `kind_for` closed set; `GET /api/v1/nearby`; SSE `nearby` on change | closed `kind` set, `sink()` | a `111` kind fed by the fabric (verified announces, `last_seen_seq`), never by RF claims |
| `N/meshtastic/presence.rs:96-109,481-498` | `NearbyNode{id,num,long_name,short_name,pubkey,snr,rssi,hops_away,last_heard_hint,position,channel_utilization}` | struct + contract | an additive local freshness field from a monotonic `Instant` (display only), plus a role and path hint |
| `M/fabric.rs:1317`; `M/store/schema.rs:66-79` | `heard` set (memory); `contacts.roles/home_hub/last_seen_seq` | `FabricEvent` (`M/fabric.rs:418`) | a presence event or view from the fabric (announce verified, attribution, disconnect) |
| `M/link/tcp_lan.rs:10-29,125` | announce at handshake; 25 s keepalive per connection | link-up | the event-driven announce point already exists; a LoRa driver needs its own |
| `N/fabric/sealer.rs:139-148,222-240`; `N/fabric/mod.rs:334-343,912-922` | appdata v1, re-mint on change | `appdata(roles, hub)` | role bits (gateway 5A/5B, LoRa-attached, offload hub); ≤ 10 B more on 160 B frames |

Timer baseline, all components off, as read (`N/mod.rs` unless noted):

| Timer | file:line | Period | Wakeups/min |
|---|---|---|---|
| daemon tick (intelligence mirror + bridge `tick_once`) | `:893-915` | 2 s | 30 |
| transport mirror | `:865-878` | 5 s | 12 |
| probation monitor (reads `state/update.json`) | `:133-140` | 5 s | 12 |
| services probe (files, `/proc/net/tcp`, health of running children; slow facts + sysfs every 5th tick) | `N/services/probe.rs:43,50,570` | 2 s | 30 |
| WAN probe (outbound HTTPS) | `probe.rs:46,548-557` | 60 s | 1 |
| device registry (+ netlink kicks) | `N/devices/registry.rs:97`; `N/devices/mod.rs:388,404-415` | 5 s | 12 |
| panels reconcile | `N/panels/mod.rs:62,650-656` | 2 s | 30 |
| Meshtastic monitor (started unconditionally `:1072`) | `N/meshtastic/mod.rs:186,640-655` | 2 s | 30 |
| Meshtastic panel revocations | `N/meshtastic/mod.rs:488-500` (`:1036`) | 2 s | 30 |
| fabric housekeeping (facts; prune every `PRUNE_EVERY`) | `N/fabric/mod.rs:177,600,1249-1266` | 5 s | 12 |
| systemd watchdog (only with `NOTIFY_SOCKET`; parked on Termux) | `N/watchdog.rs:12,35-45` | 15 s | 4 |
| rnstatus facts tick (spawns Python only on demand) | `N/transport/rnstatus.rs:16-27,945-947,1099-1115` | 30 s | 2 |
| updater | `N/updater/mod.rs:22-23` | 1 h (fail: 60 s ×2 → 24 h) | ≈0 |
| per connection: SSE keep-alive / `tcp-lan` keepalive | `N/dashboard/api_v1/mod.rs:88`; `tcp_lan.rs:125` | 15 s / 25 s | per client |

That is **≈205 wakeups/min**, plus one outbound HTTPS HEAD per minute. A heartbeat every N minutes adds 1/N wakeups/min. Whether to
also cut the baseline (for example, the two 2 s Meshtastic ticks with no board bound) is a separate owner call.

**(b) Transit forwarding and multi-link delivery in mesh111**

| file:line | What exists | Seam | Layer 7 adds or changes (cost) |
|---|---|---|---|
| `M/fabric.rs:1821-1849` | `Route::{Direct,Hub,None}` | route enum | `Route::Via(gateway/relay)` or cost-class choice: medium, locked L5 |
| `M/fabric.rs:1852-1872,3169-3170` | plan fixed at queue | `plan_mtu` | re-plan unsent fragments, or plan to the smallest MTU of the candidate paths: medium |
| `M/store/rows.rs:137`; `M/fabric.rs:2366-2389` | `Dir::Transit` unused; `Chunk`/`Foreign` dropped | frame dispatch | a transit row + forward = a new hop-limited relay plane: large; contradicts L5 v1 scope |
| `M/link/mod.rs:116-122,148,155-176`; `M/envelope.rs:60,1336-1346` | `PeerAddr::Broadcast`, `hop_limit`, 1 B shim length only | shim | a shim codec (`frame_type`, `hop_limit`): small, needed by any flood/presence frame |
| `M/fabric.rs:845,1587,1413` | no `remove_link`; awaited send | link table | `remove_link` + a non-blocking-send contract: small, needed for intermittent gateway links |
| `M/fabric.rs:2552-2558` | INTRO `tcp-lan`-only | intro gate | INTRO over other links (L5 §21 pending buffer): medium |

Layer 6 loosened nothing here. It deferred all six amendments (`6-transport/README.md:800-803`).

**(c) Hub-to-hub replication (A+)**

| file:line | What exists | Seam | A second hub needs |
|---|---|---|---|
| `M/store/schema.rs:155-168` | `custody` per `dest`, `cursors` per hub | `custody_seq` is local and monotone | a per-(peer hub) cursor table (the member-cursor shape) |
| `M/custody.rs:4-9`; `M/store/rows.rs:1010-1030` | `pkt_ref` dedup, `Duplicate(seq)` | idempotent put | replicas dedup for free; mark them evict-first (`committed` or a new flag) |
| `M/fabric.rs:2830-2833,2507-2509` | dest must be enrolled here | gate | accept dest ∈ peer hub's members: needs member-list exchange (signed list or per-member enrol mirror) |
| `M/mailbox.rs:13,195`; `M/fabric.rs:3017,3124` | `ENROLL_OK{custody_seq_floor}` | floor | a floor per origin hub, or keep the floor local (a replica gets a new local seq) |
| `5-messaging/README.md:406-409` | A+ "per-(dest) summary vectors (§4 form)" | spec text | an amendment: summaries over `custody_seq` ranges per origin hub, or `pkt_ref` sets; §4 `msg_id` vectors are invisible to hubs |

**(d) The gateway role**

| file:line | What exists | Seam | Layer 7 adds or changes |
|---|---|---|---|
| `N/services/probe.rs:540-557`; `N/services/mod.rs:473-481,1044-1074` | 60 s WAN probe → `gateway` row | `GatewayFacts`, `Facts.gateway` | 5A/5B classification (`online` history, user flag "permanent"); feed role bits and the registry |
| `N/transport/netlink.rs:18`; `N/transport/mod.rs:498-517` | `RTMGRP_LINK` debounced callback | `LinkEvents` trait | add route/addr groups → probe on change (could replace the 60 s poll) |
| `N/updater/fetch.rs:205-215` | `last-update-check.json` | file | weak signal (hourly); do not use as the trigger |
| `NI/install/mod.rs:124-128`; `receipt.rs:27` | `gateway: true` always | — | none; drop or document (trap) |
| `N/bridge/mod.rs:330-365` | `Adapter`: readable native text, `ConvRef`, `Inbound::{Text,Conversation,Nearby,Link,Dropped}` | adapter contract | the right shape **only** for a carrier that relays readable text (for example public Nostr notes) |
| `M/link/mod.rs:255-276` | `Link`: sealed `Frame` to `PeerAddr` | link contract | the right shape for a global store-and-forward carrier of **sealed 111 envelopes**, but `route()` has no "via gateway" (b) |
| `M/fabric.rs:2182-2200,2830-2841` | member → hub `CUSTODY_PUT` | hub path | "gateway as hub": a gateway-role hub accepts unknown `dest` and hands the sealed item to a global carrier; the envelope stays unchanged |
| `N/components/mod.rs:1-38`; `NI/../assets/components/*.toml` | manifest, pinned payload, consent, ports, `[weight]`, `prefetch = false` | zero-weight component | any adopted gateway program ships the same way |
| `N/transport/join.rs:1-18` | hub join descriptor (rnsd TCP) + QR; the fabric's hub offer/enrol bundle | rendezvous pattern | a gateway rendezvous descriptor reuses it (paired session, secret never in the QR) |
| `ID/attest.rs:6-10,157,198`; `N/bridge/attest.rs:26` | `0x01` nostr reserved, fail closed | `TransportVerifier` | a secp256k1 verifier if Nostr is adopted (k256 sign-off, LICENSING row) |

**(e) KB/UPDATE distribution**

| file:line | What exists | Seam | Layer 7 adds or changes |
|---|---|---|---|
| `N/updater/fetch.rs:18-47,86-97` | `UpdaterConfig{manifest_base,fallback_base}` (HTTPS) | base strings | a hub/LAN base (or a fabric fetch) as a third source; manifest-relative payload resolution (L1 amendment: URLs are absolute) |
| `N/updater/fetch.rs:136-150`; `NI/manifest.rs:154-159` | minisign + floor + sha256/size | carrier-free trust | none: a hub may serve the identical files |
| `N/kb.rs:1-40` | KB inside the payload, verified at start | payload tree | git-sync needs a second signed object (KB manifest) or rides releases only |
| `M/envelope.rs:509-516,571-607` | `KB`/`UPDATE`/`PATCH` kinds, signed, public-class sig | kinds | a producer API and a hub fan-out; Tier-2 bulk for payloads (deferred) |

**(f) Nearby UI radar and map, plus the contract (additive only)**

| file:line | What exists | Seam | Layer 7 adds |
|---|---|---|---|
| `UI/views/nearby/radar.ts:1-3,29-54`; `index.tsx:1-4,185-191` | ASCII rings, "never a peer"; copy "arrive with Layer 7" | pure functions | peers placed by hops/link class (no position needed) |
| `UI/app/routes.tsx:16` | `Nearby` already lazy | route | a map as a nested `lazy()` chunk ≤ 64 KB gz; offline tiles from disk, never fetched |
| `UI/lib/api/store.ts:105,467-468` | `nearby` from SSE `{sources}`; unknown events ignored | SSE switch | new events are safe; `sameSources` reuse |
| `API:1096-1121,1687-1702` | `NearbySource.kind: [meshtastic, reticulum]`, required `num`, `last_heard_hint` | enums | `kind: 111` (or `mesh`), optional `heard_ago_s`, `roles`, `via` (link/gateway) |
| `API:602,641`; `N/services/mod.rs:1044-1074` | `gateway` service, `transport_state` | detail JSON | gateway detail panel = the service row's `detail` plus 5A/5B |
| `A/src/tools.rs:60,184-199,320-340,947-960`; `A/tests/daemon_tools.rs:318`; `A/tests/chat_loop.rs:308` | 17 tools; `nearby.read` and `fabric.read` are GET wrappers | add schema + arm | an extended `nearby.read`, or `presence.read`/`gateway.read`; bump both pins |

### 3.2 Traps (look like seams, are not)

- **`receipt.gateway`** is always true and never read (finding 4).
- **`[order]`** sorts the display only. It is not a join order (finding 9).
- **Replaying a stored announce** is not a heartbeat: the stale rule accepts it silently (finding 7).
- **`last_heard_hint`** is the remote node's clock. It must never order or prove freshness (`API:1108`).
- **The A+ "§4 summary vectors"** are over ids that hubs cannot see (finding 13).
- **"Public class" KB/UPDATE** frames are still sealed per recipient. They are not a broadcast (finding 12).
- **`ROLE_LORA_ATTACHED`** is defined and never set (`M/contacts.rs:146`).
- **The phone relay QR** joins rnsd's Backbone (Reticulum), not the 111 fabric (`N/transport/join.rs:1-18`).
- **mDNS `111.local`** advertises the dashboard, not fabric peers.
- **`G7 GREEN`** says nothing about timers or outbound traffic (finding 5).
- **Under the agent's honesty guard** a registry hit is usable only as a private IP literal (finding 17).

### 3.3 Test counts (as recorded, run nothing)

| Suite | Last recorded | Where |
|---|---|---|
| `mesh111` | 166 lib (B1 bounce) + 25 integration/doctests (10-19 report) | `6-transport/TASKS.md` B1 row; L6 R00 §3.3 |
| `node111` | not recorded per crate after Layer 6 (L5 lock: 834 cargo across workspaces) | `plan/INDEX.md:44` |
| `id111` | 195 lib + 44 conformance + 4 `layer6_attest` (+ 7 `layer5_amendments`) | TASKS S3 row |
| `agent111` | lib 19 · `layer6_tools` 20 · `daemon_tools` 15 · `chat_loop` 17 (+ config_guard 22, events_status 15, kb_ritual 17, model_client 8, task_engine 7) | TASKS B5 row; R00 §3.3 |
| UI vitest | 1416 (after fix wave B) | TASKS Summary |
| LXMF helper | 128 | TASKS Summary |
| Gates | G1 132 · G3 147 · G4 129 · G5 99 · G7 GREEN · smoke 118 · three-node 159 · panel-lint 144 | TASKS Summary; INDEX row 6 |

Check commands: as listed in L6 R00 §3.3 (unchanged), plus `6-transport/verify/zero-weight.sh`.

## 4. Recommendation

- **Presence.**
  - (A) Event-driven: announce on link-up, on an `appdata` change and on request, and mark peers heard from any authenticated packet.
    No new timer.
  - (B) A per-link, user-set interval (LXMF precedent: default 60 min, `0` = events only), minting fresh counters.
  - (C) The fixed heartbeat that README §7 describes literally.

  **Pick A, offer B as an owner switch.** C needs the owner's say: airtime, plus fsync wear on the shared counter.
- **Gateway detection.**
  - (A) Keep the 60 s probe; it adds no weight.
  - (B) Netlink route/addr events trigger a probe only on change. This removes the poll and shrinks the baseline.
  - (C) Use the updater's last success. Too coarse.

  **Pick B.** It changes a Layer 4 probe cadence, so the owner decides.
- **The global store-and-forward seam.**
  - (A) Gateway as hub: custody for unknown `dest`, handed to an adopted carrier behind the hub; mesh111 changes only at the
    custody gate.
  - (B) A new `Link` plus `Route::Via`: general, but it costs the (b) amendments.
  - (C) `Adapter`: readable text only. Reject it for sealed traffic.

  **Pick A** for the first build.
- **A+ replication.**
  - (A) Peer hubs fetch each other's custody with per-origin cursors, reusing `MAILBOX_FETCH`/`COMMIT`.
  - (B) `pkt_ref` set reconciliation.

  **Pick A**, and amend L5 §9's wording.
- **KB/UPDATE distribution.**
  - (A) Hub-as-origin over LAN HTTP, serving the identical signed files.
  - (B) Fabric objects; these need Tier 2.

  **Pick A.**

## 5. Open questions for the owner

1. May Layer 7 amend locked L5 (the route variant, the custody gate for gateway and peer hubs, the A+ wording, `remove_link`, a shim
   codec)? Or must it stay outside mesh111?
2. Heartbeats: events only, a user interval, or fixed? If on an interval, does each beat mint a counter (fsync) or replay?
3. Should the 60 s WAN probe stay, become event-driven, or be user-off on Pi?
4. Should the baseline itself be trimmed? For example, the 2 s Meshtastic and panel ticks run with no board bound.
5. Should a no-timer gate (wakeups or outbound counts at idle) join G7?
6. Updater: may L1 accept a LAN/hub base and manifest-relative payload URLs, and serve them from hubs?
7. Is the map budget a lazy ≤ 64 KB chunk inside the remaining ≈36 KB, or does D21 get amended?
8. Does the offload registry stay HTTP-on-LAN (as built), or move to the fabric (as L3 README:48 says)?

## 6. Sources (files read this run)

Every file cited above by file:line (prefixes in the header), plus `plan/layers/6-transport/research/00-code-seams.md` §3,
`6-transport/{README,TASKS}.md`, `{1-bootstrap,2-identity,3-intelligence,5-messaging}/README.md` and `plan/INDEX.md`.
