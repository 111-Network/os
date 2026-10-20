# Layer 6, lens 0: as-built code seams (map, 2026-10-19)

Read-only pass over the tree at `6ae61a0` (no web). Every line number was read this run; re-check before editing.
Prefixes: `M/` = `50-messaging/crates/mesh111/src` · `N/` = `10-bootstrap/crates/node111/src/daemon` · `A/` =
`30-intelligence/crates/agent111` · `UI/` = `40-interface/ui/src` · `API` = `40-interface/api/openapi.yaml` · `ID/` =
`20-identity/crates/id111/src` · `SC/` = `50-messaging/sidecars/meshtastic-presence` · `L5` = `50-messaging/README.md`.

## 1. Summary

- The `Link` trait is built exactly as L5 §8 says (`M/link/mod.rs:255-276`), with three side seams (`LinkEvent`, `AnnounceGate`, `Dialer`). A second driver plugs in through `Fabric::add_link` (`M/fabric.rs:740`). But the fabric is **connection-oriented**: it sends only to an attributed `(link, PeerAddr)` connection, never uses `PeerAddr::Broadcast`, has no transit forwarding, no link removal, no codec for the L6 shim byte, and fixes a message's fragment plan when the message is queued.
- **Foreign messages have no home.** `messages.peer`/`sender` are `CHECK(length = 32)` NodeIds (`M/store/schema.rs:71-72`). The contract's `Message.peer`, `Thread.peer` and `SseMessage.peer` are 64-hex NodeIds (`API:695-731, 938-944`). The UI route `/messages/:peer` accepts only a NodeId (`UI/views/messages/model.ts:194`). Per-network categories need a new table, new routes, a new SSE event and a new UI route. None of them is an overload of the existing ones.
- The daemon wires **one** link (`N/fabric/mod.rs:170-174`, `:481-505`) and **one** Meshtastic binding (`N/devices/store.rs:13-15`). meshmud's downlink is multi-client by design, and the L5 spec already names a later `meshtastic-client` on it (`L5:603-606`). The panel shim reframes protobuf without decoding it (`N/meshtastic/shim.rs:1-10`).
- The only daemon↔sidecar pattern built is a one-way, read-only **snapshot file** (`N/meshtastic/presence.rs:1-25`, a 2 s poll). A test enforces that the sidecar has no send path (`SC/tests/test_presence.py:582`). Children get `stdin` = null (`N/supervise.rs:333`). A two-way bridge needs a new channel. The nearest precedent is the agent's per-spawn env token plus loopback API (`N/intelligence.rs:44`, `:233`).
- rnsd runs with `share_instance = no` (`10-bootstrap/crates/node111/assets/rns/config.tmpl:9`), so no LXMF program can attach to it. The renderer is a constant template with a `schema-version` takeover check (`N/transport/config.rs:9-57`). It restarts rnsd with `pkill -f`, outside the supervisor (`N/transport/mod.rs:178`). rnsd's RNode ports are **not** in the port-owner table.
- Fixed-size registries every addition must bump: `ServiceId` (9, `N/services/mod.rs:80-108`), panel `id` = `ServiceId` (`N/panels/manifest.rs:10`), proposal `KINDS` (5, `N/dashboard/api_v1/proposals.rs:65`), agent tools (10, pinned by tests), L2 `TransportId` (only nostr `0x01`, `ID/attest.rs:150-170`), store `Class` (9, `M/store/rows.rs:32-56`).

## 2. Candidates

None. This lens reads code only.

## 3. Findings that change the plan

1. **Broadcast bearers do not fit the fabric's routing as built.** `route()` returns Direct / Hub / None (`M/fabric.rs:1600-1620`). `sendable_conn` picks the first connection attributed to that NodeId, in `BTreeMap` order of `(link index, PeerAddr)` (`:1570-1581`). Nothing ever sends to `PeerAddr::Broadcast`, and path choice ignores `cost_class`. A LoRa or Meshtastic driver therefore has to present each heard 111 peer as `PeerAddr::Opaque(..)` with `LinkEvent::Connected`, and it must emit its own announce, as `tcp-lan` does in its handshake (`M/link/tcp_lan.rs:10-18`). Attribution then works unchanged: it needs one fresh, `pair_mac`-verified packet (`M/fabric.rs:49-56`).
2. **No transit relay of sealed 111 traffic.** `Dir::Transit` exists only as a schema value (`M/store/rows.rs:127`; grep finds no other use). Sealed traffic crosses networks only through member→home-hub custody (`CUSTODY_PUT`/`MAILBOX_ITEM`, `pump_hub` `M/fabric.rs:1961`). So "a node on several networks relays between them" for 111 traffic means either the hub path or new forwarding code in a locked crate.
3. **The fragment plan is fixed at queue time.** `plan_mtu` uses the current route's MTU, else the smallest MTU among registered links, else 160 B (`M/fabric.rs:1631-1651`). `seal_data` returns `None` when a fragment exceeds the link's room (`:1808-1810`), and `pump_direct` skips it silently (`:1939-1941`). A message planned for `tcp-lan` therefore never leaves over LoRa. Once one 160 B link is registered, every no-route send is planned for 160 B, and texts above the Tier-1 cap fail with `TooLarge` (`:2922-2923`).
4. **The L6 shim byte exists only as a length.** It appears as `SHIM_LEN` (`M/envelope.rs:60`), in `payload_budget` (`:1346`) and in `PathProfile::shim_len` (`M/link/mod.rs:109`). No encoder or decoder exists for `frame_type`/`hop_limit` (L5 §3). Every `Frame` is built with `hop_limit: None` (`M/link/mod.rs:158-176`, `tcp_lan.rs:539`). `FrameType::Foreign` (`M/link/mod.rs:139`) is dropped on receive (`M/fabric.rs:2167`).
5. **INTRO is `tcp-lan`-only.** An INTRO on any other link kind is dropped (`M/fabric.rs:2331-2341`; deferred, `L5:832-836`). A first contact over LoRa needs that deferred work.
6. **No `remove_link`.** Only `add_link` exists (`M/fabric.rs:740`, `:1377`), and connections are keyed by slot index (`:1004`). A hot-unplugged board keeps its slot and reports `LinkState::Down`.
7. **`Link::send` is awaited inside the actor** (`M/fabric.rs:1203`). A radio driver must queue and return at once, as `tcp-lan` does with `try_send` (`tcp_lan.rs:620-626`). A driver that waits for airtime stalls the whole fabric.
8. **The Meshtastic module is singular.** It reads one binding (`N/meshtastic/mod.rs:559`, `:785`; `N/devices/store.rs:13-15`, "at most one device carries a binding"), one downlink (`N/meshtastic/meshmud.rs:30`) and one presence file (`N/meshtastic/presence.rs:36`). Its doc still says "111 itself never transmits on the Meshtastic mesh" (`N/meshtastic/mod.rs:6-7`), and the shim says "no GPL code enters this binary" (`shim.rs:9-10`). D23 and D22 supersede both statements, so they must be restated.
9. **Identify lives inside the Meshtastic module.** It is gated on meshmud and the reader being installed (`N/meshtastic/identify.rs:200-202`) and claims the board port plus the downlink (`N/meshtastic/mod.rs:668-675`). Exit code 3 persists `family: unknown, source: probe` (`identify.rs:24-26`), which only means "not Meshtastic". The registry blocks identify for rnsd-driven boards by inferring ownership from config stanzas, not from the port table (`N/devices/registry.rs:1460-1466`).
10. **Panels cannot host LXMF UIs yet.** `kind = "child" | "detected"` (reverse proxy, WebSocket) is refused (`N/panels/manifest.rs:398-405`), and a panel id must be a `ServiceId` (`:10`). A MeshChatX panel therefore needs both the deferred generic proxy (`L5:832-846`) and a new `ServiceId`.
11. **The A10 bridge identities are not in L2.** `TransportId` has only `NostrSecp256k1 = 0x01` (`ID/attest.rs:150-170`), and the 202 B wire format carries a 32 B `transport_key` (`:6-10`). A10 promised entries for bridge identities and hash binding for 64 B RNS keys (`L5:781`). `transport_sig` is opaque behind the `TransportVerifier` seam (`:23-30`).

### 3.1 Seam tables

**(a) Foreign-network messages, per-network categories: store, API, SSE, UI**

| file:line | What exists | Seam | Layer 6 adds or changes |
|---|---|---|---|
| `M/store/schema.rs:43-46, 69-89` | `SCHEMA_VERSION = 2`, `MIGRATIONS = [V1, V2]`; `messages` keyed by 32 B `peer`/`sender`, `UNIQUE(peer,sender,stream,epoch,n)` | append-only migration list | a V3 with `foreign_messages` (network, conversation key, sender as that network's id, dir, text, local_seq, state) **or** a separate store; never pseudo-NodeIds in `messages` |
| `M/store/rows.rs:32-56` | `Class` 0–8, `ALL: [Class; 9]` | §10.4 pruning by class | a `Class::Foreign` (or similar) and its place in the §10.4 order (`M/store/prune.rs:8-22`) |
| `M/fabric.rs:871-876` | `with_store(f)`: run a closure in the actor ("do not mutate sequence or outbox state") | a daemon-side writer for non-sequence rows | typed `Fabric::foreign_*` calls are cleaner than `with_store` for writes |
| `M/fabric.rs:337-360` | `FabricEvent::{Message{id,peer:NodeId},Contact,Fabric,Settings}` | the events the daemon drains | a foreign event variant, or a separate bus owned by the bridge module |
| `N/fabric/json.rs:30-41, 50-56, 81-103` | `msg-<row id>`, `parse_node_id` (64 hex only), the `Message` shape | id namespace | a distinct id prefix for foreign rows (the `MessageId` pattern allows it, `API:686`) |
| `N/fabric/times.rs:1-13` | display times keyed by `i64` row id in `fabric/display-times` | per-store keys | a namespaced key, or a second file |
| `N/dashboard/api_v1/messages.rs:54-58, 265-301` | `/messages/threads`, `/threads/{peer}`, `POST /messages {to: NodeId}` (`404` unless a contact) | literal route strings | new resources, e.g. `/api/v1/networks` (categories) and `/networks/{net}/conversations[/{conv}]`, plus a send route in that network's native addressing |
| `N/dashboard/api_v1/nearby.rs:21-31`; `API:895-904` | Nearby = Meshtastic sources only, `kind: [meshtastic]` | `NearbySource.kind` enum | `reticulum`/`lxmf` sources (additive enum value) |
| `API:695-731, 938-944, 955-967` | `Message`, `Thread`, `SseMessage` all NodeId-keyed; `x-sse-events` map | additive event names | a new SSE event (e.g. `network_message`) and `Network`/`ForeignConversation`/`ForeignMessage` schemas; regenerate with `pnpm gen:types` (`40-interface/ui/package.json:14`) |
| `N/dashboard/api_v1/events.rs:119` | `EventBus::publish(name, &Value)` accepts any name | — | none; the contract is the authority |
| `UI/app/routes.tsx:39-41`; `UI/views/messages/index.tsx:11-31`; `model.ts:194` | `/messages`, `/messages/contacts` (placed before `:peer`), `/messages/:peer` = NodeId | route order | `/messages/net/:network[/:conv]` placed before `:peer` |
| `UI/views/messages/list.tsx:1-60`; `UI/lib/api/messages.ts:148-151` | list = requests + conversations, `byPeer` | list sections | per-network sections or tabs (D23: "keep it categorized") |
| `UI/lib/api/store.ts:145, 208, 350-399` | threads keyed by `peer`; the SSE switch has no `default` (unknown events ignored) | new `case` | a `network_message` case and a foreign-thread cache |
| `UI/views/messages/thread.tsx:213`; `model.ts:218` | `composeGate(contact, …)` needs a 111 contact | compose gate | a foreign compose gate (network up, airtime fact, honest label) |

**(b) A second Link driver**

| file:line | What exists | Seam | Layer 6 adds or changes |
|---|---|---|---|
| `M/link/mod.rs:255-276` | `Link` trait (`id`, `kind`, `profile`, `peers`, `send`, `recv`, `state`) | the contract | the driver implements it; `send` must not block (finding 7) |
| `M/link/mod.rs:66-112, 116-123, 128-140` | `PathProfile{mtu,framing,reliable,broadcast,cost_class,airtime}` with `unknown()`/`tcp_lan()`; `PeerAddr::{Tcp,Broadcast,Opaque}`; `FrameType` | constructors | per-driver profiles (160 B KISS, 233/221 B Meshtastic, 287 B LXMF), advertised `Airtime` |
| `M/link/mod.rs:228-252`; `API:784` | `LinkKind` has all five kinds; the contract enum matches | — | none |
| `M/link/mod.rs:280-303, 308-332` | `LinkEvent`, `EventSink`, `AnnounceGate`, `Dialer` (`SocketAddr`-only) | side seams | datagram drivers report `Connected` per Opaque peer; `Dialer` is TCP-shaped (optional) |
| `M/link/tcp_lan.rs:95-121, 290-330, 579-656` | `TcpLanConfig{own_announce, inbound_queue, …}`, `TcpLan::new(id,cfg,events,gate)`, the `Link` impl with `try_send` and a bounded inbound queue | the pattern to copy | the same shape: config holds `own_announce`; bounded inbound queue; `set_announce` on re-mint |
| `M/link/lossy.rs:154-155, 273` | test bearer with an MTU, loss, `Opaque` addrs and shim accounting | netns-free test double | template for stub radio links (D23 #4: stubs first) |
| `M/fabric.rs:1587-1598, 1600-1651` | `profile()` / `room()` / `route()` / `plan_mtu()` | path choice | a cost-class-aware route and a re-plan or smallest-MTU rule (finding 3) |
| `M/fabric.rs:2145-2168` | `on_frame` dispatch; `Chunk`/`Foreign` dropped | frame types | a shim codec plus a `Foreign` handoff if foreign frames ever ride 111 bearers |
| `N/fabric/mod.rs:170-174, 474-505, 985-995` | `Live{fabric, sealer, tcp: Option<TcpLan>}`; one `add_link`; shutdown stops `tcp` only | daemon wiring | a links table (id → handle, stop on shutdown), registered after `tcp-lan`, none while restore-pending (`:474-479`) |
| `N/dashboard/api_v1/fabric.rs:58-61`; `N/fabric/json.rs:195-218` | `GET /fabric` lists every registered link with its profile | — | free: a new link appears in `GET /fabric` automatically |

**(c) Daemon ↔ sidecar bridge protocol**

| file:line | What exists | Seam | Layer 6 adds or changes |
|---|---|---|---|
| `N/supervise.rs:37-46, 298, 333` | `ChildSpec{name,program,args,env,backoff,stdout_log}`; `stdin` null; stdout/err go to the log | supervised child | the channel cannot be stdio; use files, a socket or loopback HTTP |
| `N/meshtastic/meshmud.rs:350-389` | `spec()` and `reader_spec()` (`python -I script --endpoint 127.0.0.1:11230 --out <dir>`) | child specs | a bridge sidecar spec in the same form |
| `N/meshtastic/presence.rs:1-25, 297-340, 362-470` | hardened snapshot ingest (`O_NOFOLLOW`, regular file, ≤ 1 MiB, schema 1, caps, previous snapshot kept on reject), polled every tick (2 s, `mod.rs:520-528`) | file ingest | a lossless **stream**: a spool dir (one file per message, atomic rename, daemon deletes after commit, idempotency key) or a socket; snapshots lose messages |
| `SC/meshtastic_presence.py:30-38, 373-460`; `SC/tests/test_presence.py:582` | read-only `ToRadio` filter; AST test bans any "send" name | the hard rule | a two-way bridge is a **separate program** (or this rule is amended) |
| `SC/requirements.in`; `SC/README.md:1-30` | one pin `meshtastic==2.7.11`, a hash-locked closure, `gen_lock.py`, no-network tests | venv lock pattern | copy the pattern for any LXMF/Briar sidecar |
| `N/intelligence.rs:44, 233`; `N/dashboard/api_v1/auth.rs:803-840, 897-915` | per-spawn `DaemonToken` in env plus loopback `/api/v1`; the bearer may read only (`messages.rs:26-28`) | credentialed loopback | a sidecar token class with write scope to bridge-only routes, or the inverse (111d connects to the sidecar's loopback port) |
| `L5:63-65` | ports 11180–11183, 11190–11229, 11230 | port registry | a new loopback port, if a socket is chosen |
| `N/meshtastic/shim.rs:1-33, 49-60` | 0x94 0xC3 framing over the downlink, no decode, one downlink per session | in-binary option | after D22, a Rust Meshtastic client inside 111d on its own downlink (no sidecar protocol for Meshtastic); LXMF still needs a process (rule 1) |

**(d) `state/interfaces.toml` and rnsd renderer v2**

| file:line | What exists | Seam | Layer 6 adds or changes |
|---|---|---|---|
| `N/mod.rs:37-38` | `DaemonLayout::state_dir()` = `<root>/state` | home | `state/interfaces.toml` (does not exist; grep finds no code) |
| `N/devices/store.rs:1-21, 70-82` | `devices.json` `meshtastic` binding (uplink, node_num, …); "Layer 6 migrates it into `interfaces.toml`" | migration source | one-time migration; the binding becomes an `owner = 111d` entry |
| `assets/rns/config.tmpl:1-15` | `schema-version`, `enable_transport = no`, `share_instance = no`, AutoInterface only | template | v2: posture (D17 #5 default `share_instance = yes`), RNode stanzas, a TCP server |
| `N/transport/config.rs:9-57` | `render()` = template + version; `write_if_changed`; `existing_schema_version` | the takeover check | in one binary, v2 = bump `SCHEMA_VERSION` and render from the TOML; v1 releases then stop regenerating (safe rollback) |
| `N/transport/mod.rs:149-158, 168-185` | regenerate unless a newer version owns the file; netlink → regenerate → `pkill -f venvs/rns/bin/rnsd` | restart | use `ChildControl::restart` (`N/supervise.rs:209`), not `pkill` |
| `N/transport/mod.rs:102-113, 208-240` | `rnsd --config <root>/rns`, `HOME=<root>`; "joined" parsed from log lines with the wall clock | health | `transport.read` wants interface facts; the shared instance makes them readable (devices.md §3) |
| `N/devices/rns.rs:1-28`; `N/devices/registry.rs:1460-1466` | reads RNode stanzas from `rns/config` to infer the driver | ownership inference | read ownership from `interfaces.toml` and claim the ports in the `PortTable` for `rnsd` |

**(e) Full identify sequence (RNode, Meshtastic, MeshCore) over the port-owner table**

| file:line | What exists | Seam | Layer 6 adds or changes |
|---|---|---|---|
| `N/ports.rs:1-15, 89, 106-190` | `PortTable` with `claim`/`release`/`transfer`/`held`, normalized keys, ticks not clocks | reuse (its doc says so) | the `rnsd` and `111d` (KISS) owners claim here too |
| `N/meshtastic/identify.rs:1-36, 183-260` | Meshtastic step: claim → meshmud identify profile → reader `--once` → persist → transfer the claims → the child becomes the service child | step template | move orchestration out of `meshtastic/` into `devices/identify`; add the RNode step (`rnodeconf --info` from the rns venv: a separate process, rule 1 OK) and the MeshCore step (companion query); order per devices.md §3.2 |
| `N/devices/registry.rs:91, 435-465, 1434-1480` | `identify` offered for a serial board with no driver, `identify_usb` view gate; `identify_target` | offer rules | not gated on meshmud being installed; offered while any probe can run |
| `N/devices/mod.rs:131-185` | `FirmwareFamily{Rnode,Meshtastic,Meshcore,Kiss,None,Unknown}`, `Stack{Reticulum,Meshtastic,Meshcore,Dashboard}` | enums ready | none |
| `N/dashboard/api_v1/devices.rs:181-186`; `API:81-104` | `/devices/{id}/identify` is live; `/devices/{id}/flash` is contract-only (no route) | — | the flashing core (D17 #1, #9) |
| `assets/lora-table.toml` | 6 VID:PID rows, with notes saying "Layer 6 refines" | table | refine the over-matching rows |

**(f) New agent tools**

| file:line | What exists | Seam | Layer 6 adds or changes |
|---|---|---|---|
| `A/src/tools.rs:48-176, 178-192` | `schemas()` (10) and the `dispatch` match | add a schema and an arm | `devices.read`, `transport.read`, `device.identify`, `device.propose`, `service.restart` (devices.md §3), plus a bridge read/send |
| `A/src/tools.rs:419, 483, 815-860` | `daemon_get` (GET only, size-capped), `node_id_arg`, `messages.send` → proposal `kind: message`, body `to: <hex>` | the proposal path | foreign sends follow the same proposal pattern, with a new body contract naming network and recipient |
| `N/dashboard/api_v1/proposals.rs:65-67`; `N/fabric/proposal.rs:1-31`; `API:556` | `KINDS` = 5; the A1 parser for `message` | contract enum | `device` (devices.md §3) and a bridge-send kind or body form |
| `A/tests/daemon_tools.rs:306`; `A/tests/chat_loop.rs:308` | `assert_eq!(…, 10)` | count pins | bump them |
| `UI/lib/prompts.ts:124-128`; `UI/lib/layers.ts:54-58`; `UI/views/settings/index.tsx:15` | per-service goals; L6 settings page = structure only | UI hooks | an `l6` settings page (posture, region, bridges opt-in) |

### 3.2 Traps (look like seams, are not)

- **`with_store`** looks like a write path, but it runs inside the actor and shares `local_seq`. Foreign writes through it bypass event emission (`M/fabric.rs:871-876`).
- **`PeerAddr::Broadcast`** exists (`M/link/mod.rs:120`), but the fabric never sends to it (finding 1).
- **`FrameType::Foreign`** and the shim's `frame_type 3` exist, but frames are dropped and no codec exists (finding 4).
- **`Dir::Transit`** exists, but nothing writes it (finding 2).
- **The `schema-version` takeover** was designed for "newer software owns the file". In a single `111d` it just means "render v2". The `pkill` restart is outside supervision.
- **`rns_interface` / stanza ownership** is inferred from the config file, not claimed (finding 9).
- **`messages.peer` as a pseudo-NodeId** (a hash of network and sender) would satisfy `CHECK(length=32)`, but it would enter the key cache, the ACK and contact logic, and it breaks D23 rule 3 (honest labels).
- **The presence file** cannot carry messages: snapshots overwrite, and a rejected file keeps the old snapshot (`N/meshtastic/presence.rs:22-25`).
- **The meshmud downlink is one port shared by every client** (reader, shim sessions, a future client). It is claimed once for the `meshmud` task (`N/meshtastic/mod.rs:671`). Adding a downlink client needs no new claim, and a second board collides on it (`identify.rs:6-8`).
- **`LinkKind::MeshtasticClient`** carries the doc comment "no bridge this cycle" (`M/link/mod.rs:233`), and L5 §8 says "seam; no bridge this cycle". D23 changes this, but the Link seam covers sealed carriage only, not readable text.

### 3.3 Test counts (`cargo test -- --list`, run 2026-10-19; vitest; unittest)

| Crate or suite | Count | Breakdown |
|---|---|---|
| `mesh111` (`50-messaging`) | **161** | lib 136 · contacts 1 · flood 1 · hub_flow 9 · links 1 · lossy 7 · recovery 1 · requests 3 · doctests 2 |
| `node111` (`10-bootstrap`) | **472** | lib 274 · main 0 · 22 integration binaries 198 (api_v1_devices 15, api_v1_meshtastic 11, api_v1_messages 11, api_v1_panels 18, api_v1_relay 9, api_v1_status_services 20, …) · doctests 0 |
| `agent111` (`30-intelligence`) | **115** | lib 14 · main 0 · chat_loop 17 · config_guard 22 · daemon_tools 15 · events_status 15 · kb_ritual 17 · model_client 8 · task_engine 7 |
| `id111` (`20-identity`) | **236** | lib 185 · conformance 44 · layer5_amendments 7 |
| UI (vitest) | **1153** in 35 files | all passed, 2.9 s |
| presence sidecar | **69** | 9 skipped (real-library tests without the venv) |

Check commands: `10-bootstrap/scripts/check.sh` (fmt · clippy · musl ×2 · test · shellcheck · musl-env self-test · notices · build-scripts gate · systemd-analyze · messaging · G2 static link · presence sidecar · ui; `REQUIRE_G2=1` for lock runs) · `50-messaging/scripts/check.sh` (fmt, clippy, test, musl ×2) · `30-intelligence/scripts/check.sh` (fmt, clippy, test, musl x86_64) · `40-interface/ui`: `pnpm check` (typecheck, test, build, budget) · `SC/run-tests.sh`.

## 4. Recommendation

The forks this lens exposes (the owner decides; my pick last):

- **Foreign-message store.** (A) Additive V3 tables in the mesh111 SQLCipher store, plus typed `Fabric` calls. This gives one key, one pruning pass and one `local_seq`, but it amends locked L5 §10.3. (B) A separate bridge store in node111. Layer 5 stays untouched, at the cost of a second key and a second prune. (C) Pseudo-NodeIds in `messages`: reject. **Pick A.**
- **Bridge channel to sidecars.** (A) A spool dir, one file per message, reusing the presence hardening. (B) A loopback socket with a per-spawn token. (C) An in-binary Rust Meshtastic client on the meshmud downlink (D22), with a process kept only for LXMF. **Pick C for Meshtastic and A for LXMF and phone apps.**
- **Sealed carriage on radios.** Opaque per-peer connections plus a shim codec in the driver, the fragment-plan fix, and a cost-aware route.

## 5. Open questions for the owner

1. May Layer 6 amend the locked L5 store and fabric (schema V3, a `Class` for foreign rows, cost-aware routing, re-planning unsent fragments, a shim codec, `remove_link`)? Or must the bridge live entirely outside mesh111?
2. Does "a node on several networks relays between them" cover sealed 111 traffic too (new transit forwarding: `Dir::Transit` is unbuilt), or only readable text in each network's native format?
3. Meshtastic: an in-binary client (D22 allows GPL-3.0 linking) or a separate GPL sidecar like the presence reader? The presence reader's no-send rule stays either way.
4. LXMF needs `share_instance = yes` (config template line 9) plus a separate LXMF process, and its hosted UI needs the deferred child-panel proxy and a new `ServiceId`. Should both be in Layer 6 scope?
5. A10: which `TransportId` values and binding form for Meshtastic (an X25519-only board key, no signing key on the board) and Reticulum (64 B key, hash binding)? This is an L2 registry amendment.

## 6. Sources (files read this run)

`M/link/mod.rs`, `M/link/tcp_lan.rs`, `M/link/lossy.rs` (grep), `M/fabric.rs` (1-215, 312-760, 960-1075, 1179-1230,
1377-1460, 1570-1660, 1747-1815, 1915-1970, 2140-2175, 2311-2342), `M/envelope.rs` (grep, 487-530, 1215-1235, 1335-1355), `M/contacts.rs`,
`M/store/{schema,rows,mod,prune}.rs`, `M/lib.rs`; `N/fabric/{mod,json,times,settings,proposal}.rs`,
`N/dashboard/api_v1/{mod,messages,contacts,nearby,devices,proposals,events,auth}.rs`,
`N/meshtastic/{mod,meshmud,shim,presence,identify}.rs`, `N/ports.rs`, `N/devices/{mod,registry,rns,store,detect}.rs`,
`N/transport/{mod,config}.rs`, `N/services/{mod,probe}.rs`, `N/supervise.rs`, `N/intelligence.rs`, `N/panels/manifest.rs`,
`N/mod.rs`, `assets/{rns/config.tmpl,lora-table.toml,panels/meshtastic.toml}`; `API` (schemas, x-sse-events);
`UI/views/messages/*`, `UI/lib/api/{messages,store,nearby}.ts`, `UI/views/{nearby,devices,settings}/*` (names and models),
`UI/lib/{layers,prompts}.ts`, `UI/app/routes.tsx`; `A/src/tools.rs`, `A/tests/{daemon_tools,chat_loop}.rs`; `ID/attest.rs`,
`20-identity/README.md §10`; `SC/{meshtastic_presence.py,README.md,requirements.in,tests/test_presence.py}`;
`L5` §3, §8, §10.3, §14, §21; `plan/cross-layer/devices.md` §3, §8–§9; `plan/DECISIONS.md` D23; the four check scripts.
