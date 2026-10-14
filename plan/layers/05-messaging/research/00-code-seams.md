# Layer 5 — as-built extension seams (map, 2026-10-14)

Where Layer 5 plugs into the built code. Produced by a read-only explore pass at kickoff (commit 0adb8e2); line
numbers were verified then — re-check before editing. Prefixes: `N/` = `bootstrap/crates/node111/src`,
`UI/` = `interface/ui`, `A/` = `intelligence/crates/agent111/src`, `ID/` = `identity/crates/id111/src`,
`API/` = `interface/api`. `~` = approximate.

## 1. Supervised child pattern (how a new daemon child is added)

- `N/daemon/supervise.rs:37` `ChildSpec{name,program,args,env,backoff,stdout_log}` · `:13` `BackoffPolicy` (default `:23`: 250 ms ×2, 60 s cap; 5 restarts/60 s ⇒ Degraded) · `:49` `ChildState{Running,Backoff,Degraded,Stopped}`.
- `:298` `spawn_child(spec) -> ChildHandle` (tokio; stdin null; stdout+stderr appended to `stdout_log`) · `:274` `ChildHandle` (derefs to `ChildControl`, `.control()` `:289`).
- `:137` `ChildControl`: `state()`, `pid()`, `stop()`, `:177` `stop_and_wait(grace)` (SIGTERM → grace → SIGKILL → `KILL_WAIT` 2 s `:131`), `:209` `restart(grace)`.
- Template for a new child module: `N/daemon/intelligence.rs` — `:145` `llama_server_spec`, `:172` `agent_spec` (spec only if `current/bin/<x>` is a file; env `AGENT111_*`; log under `agent_dir`) · `:23-30` fixed loopback ports (11180 dashboard, 11181 model, 11182 agent) · `:68` `IntelligenceHandle` (`state()`, `agent_pid()`, `agent_control()`, `:116` `stop_and_wait`) · `:35` `ChildDisplay` · `:224` `paths::*` · `:256` `ensure_private_dir` (0700) · `:272` `start()`.
- rnsd: `N/daemon/transport/mod.rs:26` `TransportHandle` (`control()` `:46`, `stop_and_wait` `:63`, `rnsd_spec` `:102`, `start()` `:120`); health = child state + `rnsd.log` parse `:208` (no HTTP).
- Wiring `N/daemon/mod.rs` (`daemon_main` `:150`): `:209` transport start · `:235` intelligence start + 2 s mirror into `DashboardState` `:246` · `:293` `services::probe::spawn(.., Sources{..})` · `:301-306` `Supervised::default().with(ServiceId::X, handle.control())` chain = what `services/{id}/restart` reads · `:334-337` shutdown `tokio::join!` of `stop_and_wait(CHILD_STOP_GRACE)` (`:79`, 5 s).
- Restart route `N/daemon/dashboard/api_v1/services.rs:72` (`409 not_supervised` when absent) · `:39` `RESTART_GRACE` · `:143-145` id → log path for `GET /services/{id}/log`.
- Health probe `N/daemon/services/probe.rs:124` `health(port)` = `GET /health` 200 (`http_get` `:85`) · `:46` `Sources` · `:532-548` per-tick probes · grace constants `services/mod.rs:47-49`, applied by `child_rule` `:471`.

## 2. Services registry (how a new service id is added end-to-end)

- `N/daemon/services/mod.rs`: `:61` `enum ServiceId` · `:72` `ALL: [ServiceId; 7]` · `:82` `as_str` · `:94` `parse` · `:98` `index` · `:158` `ServiceRecord` (`embed_url` `:175`) · `:181` `Catalogue` / `:190` `catalogue(id)` (plain_name, description, requires, kb_entry) · `:364` `Facts` · `:424` `Computed` · `:870` `compute(&Facts) -> [Computed; 7]` · `:899` `Registry` (2-tick debounce; `tick` `:912`) · `:975` `record()` · `:999` `tick_and_publish` · `:125` `Action{Restart,Log}` · `:137` `Supervised`.
- Rule fns: `agent :547` · `model :575` · `transport :618` · `radio :688` · `kb :767` · `gateway :801` · `codeagent :834`. Facts: `probe.rs` `spawn :486` (2 s tick; slow facts every 10 s), `slow_facts :380`, `file_facts :448`, `SpawnTracker :464`.
- Adding an id touches: the fixed `7` in `ALL`/`compute`/`Registry::tick` + ordering test (`~:1020`) · `api_v1/services.rs` `unknown()` id list (`~:107-111`) · `API/openapi.yaml:232` `ServiceId` enum · generated `UI/src/lib/api/sse.gen.ts:66` + `UI/test/contract.gen.ts` · `UI/src/lib/verdict.ts:35` `NOUN` map (falls back to `plain_name`) · `UI/src/lib/api/mock/fixtures.ts:358, :464`.
- Sidebar services list is dynamic (`UI/src/lib/nav.ts` `visibleNav`, `serviceHref`): a new id appears without nav edits; names/descriptions come from the Rust `catalogue()`. `UI/src/views/services/model.ts` `NO_SETUP_SERVICES` is gateway-only.
- `embed_url` today: no Rust sets it (`Computed.embed_url` always `None`, `services/mod.rs:452`; copied in `record()` `:991`). UI: `views/services/model.ts:171-177` `classifyHref` (absolute http(s) only) → `embedUrl`; `views/services/index.tsx:221-228` `Links` renders only an external "Open full client" link. No iframe anywhere. CSP `N/daemon/dashboard/routes.rs:70`: `default-src 'self'`, `frame-ancestors 'none'`, no `frame-src`. Tests `UI/test/services.test.ts:35`; design note `interface/README.md:235`.

## 3. API routes, SSE bus, type generation

- `N/daemon/dashboard/api_v1/mod.rs`: `:40-49` `pub mod` list · `:224` `router(state)` merges `:226-234` (`events::router()` first; gated group behind `auth::require_session`; `auth::router()` open; `csrf_guard` wraps both — mutating verbs must be JSON) · each module `pub fn router() -> ApiRouter` with literal `/api/v1/...` paths (`events.rs:237`, `services.rs:41`, `devices.rs:124`) · `:104` `ApiState`, `:165` `insert_ext`, `:170` `ext::<T>()` · `:178` `Problem::new(status, code, title)`. Mounted by `N/daemon/dashboard/routes.rs:375` `app()`.
- Events `api_v1/events.rs`: `:36` `RING = 256` (one ring for all event types) · `:119` `EventBus::publish(name, &Value) -> id` · `:148` `subscribe(last_event_id)` (replay or `reset` with `expired|restart`). Publishers: `status.rs:400/415`, `services/mod.rs:999`, `devices/registry.rs:1127`, `relay.rs:820` (agent) + `:830` (proposal), `proposals.rs:479`. Client: `UI/src/lib/api/store.ts:234` `applyFrame` (status, services, devices, proposal, agent, reset → `refreshAll` `:270`).
- Contract `API/openapi.yaml`: `Problem.code` enum `~:227` · `ServiceRecord :236` · SSE payload schemas from `:414` · `x-sse-events.events` map `:511` (new event = schema + map entry).
- Generation: `UI/package.json` `gen:types` = `openapi-typescript ../api/openapi.yaml -o src/lib/api/types.gen.ts && node scripts/gen-sse.mjs`; `gen-sse.mjs` writes `sse.gen.ts` + `test/contract.gen.ts`, `--check` fails when stale; `ENUMS` list `:313-322`, alias list ends `~:309` (new enums/aliases added by hand).

## 4. UI

- Messages stub: `UI/src/views/messages/index.tsx` (one file: `Empty` "arrives with Layer 5" + `DELIVERY_STATES` SpecTable queued/sent/relayed/delivered + `stagePrompt`); lazy route `UI/src/app/routes.tsx:17`. Nav: `UI/src/lib/nav.ts` `NAV` entry `messages` (`/messages`, section `core`) + `titleFor`; a conversation sub-route needs a `titleFor` branch + a route.
- Adapter: `UI/src/lib/api/adapter.ts` `interface Adapter` (copy the `devices` / `flashDevice` / `chat` patterns) · real `UI/src/lib/api/http.ts:71` `createHttpAdapter` (`call(method, path)`; devices `:182-190`) · mock `UI/src/lib/api/mock/index.ts:68` `createMockAdapter` (bus publish `:125`, `flashDevice` `:436`) + `mock/fixtures.ts`, `mock/scenarios.ts`, `mock/bus.ts` · store `store.ts` (`ApiStore` `:134`) needs a messages slice; `hooks.ts` a hook · tests `UI/test/{mock,http,store}.test.ts`, `helpers/validate.ts`.
- Service page `UI/src/views/services/index.tsx`: `ControlPanel :202` (SpecTable of `flattenDetail(service.detail)`), `Links :221`, `ServicePage :259+` (live / starting / unset / unavailable / error); `log.tsx` `LogTail`; `model.ts` `servicePageState :162`. A hosted panel goes here.

## 5. Agent tools

- `A/tools.rs`: `:35` `schemas()` = exactly `kb.read`, `health.read`, `logs.read`, `proposal.write`, `task.submit` ("no shell, no plugins… frozen at T0") · `:108` `dispatch(name, args, &ToolCtx)` with `_ => Err("unknown tool")` · `:19` `ToolCtx{kb,store,journal,cfg,tasks,model}`. New tool = schema entry + dispatch arm + `ToolCtx` field if stateful. `A/chat.rs:200` builds `ToolCtx`, `:249` dispatches, `:289` passes `tools::schemas()` to the model.
- Agent → 111d today: only `tools.rs:172` `probe_daemon` (`ureq` GET of `cfg.daemon_health_url`, `A/config.rs:23/42`, env `AGENT111_DAEMON_HEALTH_URL`). A `messages.*` tool needs a loopback `/api/v1/messages` base URL in config, or a shared store dir. `proposal.write` (`tools.rs:205`) is the agent's only write path.
- Relay `N/daemon/dashboard/api_v1/relay.rs:735-738` (`/chat`, `/chat/history`, `/agent/status`, `/agent/task`); `tool_call` → `tool {name}` frame; `:919` `start_events` republishes the agent journal as `agent` frames (`SseAgentEvent` kinds in openapi). Journal `A/events.rs:66` `Journal::append(kind, detail)`.

## 6. State dir and persistence

- Layout `N/daemon/mod.rs:25` `DaemonLayout{root,etc}` · `state_dir()` `:33` (`root/state`) · `identity_dir()` `:36` · `current_release()` `:39` · `N/util/paths.rs:7` `root()` = `$NODE111_ROOT` or `/var/lib/111`.
- Atomic writers `N/util/fs.rs:13` `atomic_write(path, data, mode)` · `:20` `atomic_write_owned(path, data, mode, Option<(uid,gid)>)` (fchown-before-rename, fchmod exact mode) · wrapper `api_v1/auth.rs:338` `write_json_owned`; `:361` `rotate_files` picks `owner` via `node_user_ids` when euid 0 · `N/config.rs:73` `chown_node111`, `:180` `chown_root_node111`.
- State files: `settings.json` (`settings.rs:270`, 0644) · `devices.json` (`devices/store.rs:158`, 0600) · `dashboard-pair.json`, `dashboard-sessions.json` (`auth.rs:50-51`) · `offer.json`, `facts.json`, `health-confirmed.json`, `revert-request.json` (`daemon/mod.rs:121/133`) · `floor.json`, `update.json` (root-owned).
- Dirs: `N/install/supervise.rs ~:116-127` `node_dirs` (state, state/staging, identity, rns, venvs) + `chown_tree :165`; agent dir at runtime (`intelligence.rs:256`). `bootstrap/systemd/111node.service:33-36`: `ProtectSystem=strict`, `StateDirectory=111` ⇒ `/var/lib/111` is the only writable tree.
- **No database crate** in `node111` or `agent111` (no rusqlite/sled/redb). node111's dependency set is frozen; additions need reviewer sign-off recorded in `bootstrap/TASKS.md`. agent111 deps: anyhow, axum, serde, serde_json, time, tokio, tokio-stream, ureq.
- Agent store `A/store.rs`: `:55` `init` (0700; `conversations/`, `proposals/`, `tasks/`) · `:99` conversations = append-only JSONL · `:138/:145` proposals/tasks = pretty JSON `schema:1` via plain `std::fs::write` (not atomic) · `:78` `events.jsonl` · `:164` `new_id` = nanos + counter.

## 7. id111 crypto surface (what Layer 5 can call)

- `ID/seal.rs:340` `seal(recipient_enc_pubkey:&[u8;32], header:&SealHeader, sender:&NodeId, plaintext:&[u8]) -> Result<Vec<u8>, SealError>` · `:422` `open(recipient:&Keyset, packet:&[u8]) -> Result<OpenedPacket, OpenError>` · `SealHeader{version,crypto_mode,counter,extra≤255}` (`:119`; `::v1(counter)` `:137`) · `OpenedPacket{header,sender,plaintext}` (`:164`). Envelope `version` + `extra` are Layer 5's; replay checking is the caller's; `MIN_PACKET_LEN` 59 B.
- `ID/announce.rs`: `Announce::create(&Keyset, counter, appdata≤32)` `:179` · `parse :204` · `encode :230` · `verify :249` · `process_announce(&mut PinStore, &NodeId, wire) -> PinOutcome` `:291`; 137–169 B.
- `ID/store.rs` `PinStore::open(dir)` `:191` / `open_with_cap :198` / `get/get_mut/touch :229-240` / `apply_announce :262` / `persist :308`; cap 256; file `peers`.
- `ID/counters.rs` `CounterStore` (file `counters`): `create/open/restore_pending :315/:346/:376` · `next_announce_counter :421` (committed before transmit) · `next_send_counter(&NodeId) :443` · `on_receive(&NodeId, counter) :458` (32-entry window) · `commit :477` · `shutdown :470`; batch N = 64.
- `ID/keyset.rs:453` `public()` · `:462` `node_id()` · `:475` `sign_with_tag` · `:498` `ecdh`.
- **Daemon status:** no daemon code uses seal/announce/PinStore/CounterStore yet (CounterStore only in a test, `N/identity/mod.rs:745`). `N/identity/mod.rs:354` `load()` → `Identity` (`keyset()` `:94`), but `daemon_main` (`daemon/mod.rs:170-173`) keeps only `node_id_hex` and drops the keyset — Layer 5 must keep an identity handle alive in the daemon (signer-daemon pattern, L2 §9/§13).

## 8. Device registry seam (Meshtastic identify + presence)

- `N/daemon/devices/registry.rs`: `match`/`firmware` computed in `serial_record :496` and `usb_record :612`; `match` from `rule_for :404` / `rule_matches :237` against `lora_table()` (`N/install/probe.rs`) → `Confidence::Likely`, rule `lora-table VID:PID`; firmware hard-set `DeviceFirmware::unknown()` at `:549/:570/:645/:658`; module doc `:10-14`: a serial port is never opened.
- Types `devices/mod.rs:152` `DeviceFirmware`; `:131` `FirmwareFamily{Rnode,Meshtastic,Meshcore,Kiss,None,Unknown}`; `:142` `FirmwareSource{Probe,User,Agent,Unknown}`; persisted in `store.rs:46` `Summary`.
- `driver`: `rnode_driver :477` matches `rns::bindings` (`devices/rns.rs:120`, enabled `RNodeInterface` stanzas) to a tty, requires rnsd running. Meshtastic driver only for SPI bridges: `SPI_BRIDGES :69`, `spi_bridge :250`, set at `:635-639` when `Probe::Open`. USB-serial Meshtastic boards get no driver today.
- Identify hook: `HARDWARE_ACTIONS :71` = `rename, forget, agent`; openapi device actions already reserve `identify`, `pair`, `authorize`, `flash`; `/devices/{id}/flash` (`API/openapi.yaml:79`) answers 404; `api_v1/devices.rs:124` routes list/get/patch/delete only. A consented identify = new route there + a branch setting `Draft.firmware` (source `Probe`/`User`).
- Tick/probe: `tick :921` builds `TickFacts :162`; `probe() :138` returns Open/Refused by reading `/proc/net/tcp{,6}` via `services::probe::listening_under` (`probe.rs:173`), never connects (`MESHTASTICD_PROBE :64`: meshtasticd serves one API client).
- Radio service: `MESHTASTIC_PORT` `services/mod.rs:53`; `probe.rs:547` `port_listening(4403)` per tick; `StackWatch::observe` (`probe.rs:409`) sets `meshtastic_seen`; `radio() :701` emits `radio_bound_meshtastic` with kb_entry `projects/meshtastic.md`. **A Meshtastic client child holding the :4403 connection becomes "the one API client"** — the LISTEN-only probes keep working, but the hosted web client and our client cannot both hold meshtasticd's TCP API; USB-serial boards are driven over serial by our client instead.
