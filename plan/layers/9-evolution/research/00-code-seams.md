# Layer 9 research, lens 0: as-built code seams

Scout: opus, 2026-11-02, no web. Read only; nothing run. Every claim was read at the cited line of the tree at `a65ac6d`. Path prefixes:
`N/` = `1-bootstrap/crates/node111/src/daemon/`, `NI/` = `1-bootstrap/crates/node111/src/`, `A/` = `3-intelligence/crates/agent111/src/`,
`M/` = `5-messaging/crates/mesh111/src/`, `ID/` = `2-identity/crates/id111/src/`, `API` = `4-interface/api/openapi.yaml`, `UI/` = `4-interface/ui/src/`.

## 1. Summary

- **The observer belongs in `111d`, not in `111-agent`.** Every event source Layer 9 needs is already an in-process channel in `111d`: each child's
  `watch::Receiver<ChildState>` (`N/supervise.rs:627`), the `EventBus` with `subscribe()` (`N/dashboard/api_v1/events.rs:119,148`), and the netlink
  sources (`N/transport/netlink.rs:12`, `N/devices/hotplug.rs:34`). Subscribing to them costs no timer. The agent is a crashable child itself. Its tools
  cannot read the services registry, the service logs or the update state. It may not restart anything (D26 #2).
- **The supervisor observes too little.** It drops the exit status (`N/supervise.rs:972`), logs nothing on an ordinary exit, and keeps `Degraded`
  only in memory. `Degraded` is terminal until someone asks for a restart (`:996-1009`). The default policy degrades a child after 5 exits in 60 s
  (`:61-71`). Layer 9's first job is an incident record: exit code or signal, restart count, and state transitions, persisted.
- **The agent's diagnose and propose tasks are tool-less.** Each one is a single model call. Its context is `/health`, the agent's own 30-event
  journal and KB refs (`A/tasks.rs:320-392`). The output is a Markdown proposal with no structured action, unsigned and written non-atomically
  (`A/store.rs:120-140`). Accepting a `diagnose`, `propose` or `patch` proposal only records `{decision, at}` (`N/dashboard/api_v1/proposals.rs:412-475`).
  Nothing reads `decisions/` except the writers themselves.
- **The device applier is the template, and only half of it is generic.** The generic half: the pending record first, a one-at-a-time lock, the
  per-action plan → snapshot → apply → probe → keep or revert loop, a detached task, and the `OneShots`/`ApplyProbe`/`DecisionRecord` traits
  (`N/devices/applier.rs:526-600,899-1000`). The device-specific half: the snapshot itself (`interfaces.toml` + `rns/config`, `:1196-1203`) and the
  probes. Nothing watches a change once it is kept, and a crash mid-apply leaves a pending record that nobody reverts.
- **A node can validate only at runtime.** It has the health predicate, the services compute, the applier probes, update probation (60 s
  sustained, 600 s window) and the payload and KB hash checks. Everything that tests code needs cargo, git, `unshare` and jq (`6-transport/verify/bridge.sh:1064`)
  and runs only in CI or on a dev box. On a node, a "fix" is a typed reversible action, or a signed release from an origin.
- **Sharing has no path today.** `PATCH` (0x13) is defined, signed and public-class, but nothing produces it. It is a bulk class, so it needs Tier-2
  `OBJ_*`, which is v1.1. No domain tag exists for a proposal or a patch. The agent holds no key, and agent delegation is pinned but rejected in v1. The
  hub's opencode runs in an empty work dir with no source tree. Nothing installs opencode.

## 2. Candidates

None. This is a code lens.

## 3. Findings that change the plan

### 3.1 Seam tables

**(a) Observation sources and the timer baseline**

| file:line | What exists | Seam | Layer 9 must add or change |
|---|---|---|---|
| `N/supervise.rs:416-421,853,935-996` | `ChildState {Running, Backoff, Degraded, Stopped}` on a `watch` per child; `state_watch()` `:627` | event source, zero timer (the component runner already waits on it, `N/components/runner.rs:229-240`) | subscribe once per child; derive incidents from transitions |
| `N/supervise.rs:972,926-929` | `let _ = child.wait().await`: the exit status is discarded; only a spawn failure logs | — | keep the `ExitStatus` (code or signal) and a restart counter; publish them on the watch or a new channel |
| `N/supervise.rs:51-71,465-493` | 250 ms ×2, cap 60 s, ±20 % jitter; 5 exits in 60 s ⇒ `Degraded` | `BackoffPolicy` per spec | `Degraded` waits forever (`:996-1009`); persist it; it is the "needs repair" signal |
| `N/services/mod.rs:80-105,136-141,256-279,412-420` | 9 fixed ids; `Live/Unset/Error/Starting`; closed `reason_code` (`child_degraded`, `stack_down`…); `ChildFacts` has no exit code and no restart count | `Registry::tick` publishes `services` after two identical ticks (`:1448`) | key incidents on `(service id, reason_code)`; optional components are **not** rows (use `Components::status`, `N/components/mod.rs:420`) |
| `N/health.rs:24-41`; `N/dashboard/routes.rs:573-595` | `/health` is **two-state** (identity, kb, transport, updater_idle) | `evaluate()` pure | the tri-state is the agent's own `/health` (`3-intelligence/README.md:84`), not 111d's |
| `N/dashboard/api_v1/events.rs:36,119,148` | ring of 256, monotonic ids, broadcast; names `API:1699-1712` (no incident or health event) | in-process `subscribe()` | an `incident` event name (additive contract); the `status` heartbeat fills the ring, so the observer should subscribe live, not replay |
| `N/mod.rs:133-176`; `NI/probation.rs:8-18,57`; `NI/update_helper.rs:238-266` | the probation monitor (5 s tick) writes `health-confirmed.json`/`revert-request.json`; the helper leaves a tombstone `update.json` (`pending:false`, `health_confirmed:false` on revert) | `update.json` tombstone | read the tombstone at start ⇒ an "update reverted" incident; today only `eprintln` from the root helper |
| `NI/update_guard.rs:62-80` | torn payload ⇒ revert to N-1, `GuardReport.torn_payload` printed only | ExecStartPre | persist the guard report |
| `N/services/log.rs:16-18,59`; `N/dashboard/api_v1/services.rs:10-14,176` | tails of agent, model, rnsd and meshmud logs, sanitized, ≤500 lines | `GET /services/{id}/log` | `111d`'s own stderr is not tailable: journald `LogNamespace=111node` (`1-bootstrap/systemd/111node.service:52`), Termux `$PREFIX/var/log/111node.log` (`termux/sv/111node/run:23-35`) |
| `1-bootstrap/termux/sv/111node/finish:8`; `termux/cron/healthcheck.sh:12-15`; `crontab.line:5` | the runit finish hook logs the exit code and signal; cron curls `/health` every 5 min into `state/health-failures.log` | Android placeholder seam | `health-failures.log` has **no reader** (grep: none); ingest it at start |
| `N/transport/netlink.rs:12-19`; `N/devices/hotplug.rs:13,34` | `RTMGRP_LINK` and `KOBJECT_UEVENT` sockets | `LinkEvents` trait | link and device incidents without a poll |
| `N/fabric/mod.rs:59,319-326`; `N/services/mod.rs:493-516` | statvfs free/total; `store_full`, `restore_pending`, `hub_reachable` | `FabricFacts` provider | disk-pressure and custody incidents from facts already computed each tick |

Timer baseline: I re-checked R07 `00-code-seams.md` §3.1. Its rows still hold, but its line numbers have moved (the daemon tick is now
`N/mod.rs:902`, the transport mirror `:869`). It **misses** three timers:

| Timer | file:line | Period | Wakeups/min |
|---|---|---|---|
| `status` heartbeat (rebuilds the snapshot and publishes on change or every 28 s) | `N/dashboard/api_v1/status.rs:62,446-466`; started `N/mod.rs:1044` | 2 s | 30, always |
| agent events relay `absent_poll` while the agent is not running | `N/dashboard/api_v1/relay.rs:97,110,887-893` | 2 s | 30 on every node without `111-agent` (the Pi floor) |
| LAN address refresh | `N/dashboard/mod.rs:297-310` | 30 s | 2, only with LAN |
| Termux cron `/health` (spawns `curl`) | `termux/cron/crontab.line:5` | 5 min | 0.2 + a process |

As read, that makes **≈265 wakeups/min** with no agent, against R07's ≈205. L7 §3.7's `idle.sh` should measure it. **What a Layer 9 observer
adds:** zero, if it only awaits watches and the bus. A "sustained for N s" rule is a one-shot `sleep` armed on a transition, never an interval.
Tasks it submits cost a model call each, so a debounce per incident key is mandatory. `TaskEngine::submit` has no concurrency cap and starts one OS
thread per task (`A/tasks.rs:137-185`).

**(b) Diagnosis inputs the agent can read today**

| file:line | Readable | Missing |
|---|---|---|
| `A/tools.rs:60-318,320-341` | 17 tools: `kb.read`, `health.read` (the agent's own state plus 111d `/health`, `:380-394`), `logs.read` (**the agent's own journal only**, `:430-436`), `proposal.write`, `task.submit`, and GET wrappers for messages, contacts, fabric, nearby, networks, bridge edges, devices and transport (`:820-1910`) | `/services` (states, `reason_code`), `/services/{id}/log`, `/status` (update state, floor, offer), `/components`, `update.json`, incident history, exit codes |
| `A/tasks.rs:320-392` | `diagnose`/`propose` context = `daemon_health`, `recent_events` (30), KB refs (≤8 × 8 KiB). The system prompt: "You have no tools" | the task path never calls the tools; Layer 9 must pass an incident record in `refs`/`note`, or give tasks read tools |
| `A/chat.rs:32,295` | the chat loop uses the tools, ≤4 rounds | a chat turn is human-initiated only |
| `A/kb.rs`; `N/kb.rs:1-3` | grep-then-read over `current/share/kb/` | one recipe exists (`0-knowledge/recipes/`); no repair playbooks |

**(c) The change vocabulary already typed**

| file:line | Change | Reversible? | Who may trigger |
|---|---|---|---|
| `N/devices/applier.rs:7-24,87-104` | `rns.interface.upsert`/`remove`, `rns.posture.set`, `radio.region.set` | snapshot → apply → probe ≤30 s (1 s steps) → keep or revert | accepted `kind: device` proposal; posture PATCH via `apply_unrecorded` (`N/dashboard/api_v1/transport.rs:377-388`) |
| same | `identify`, `service.restart` (one-shots) | nothing to revert | the same proposal; restart = `restart_service` (`N/dashboard/api_v1/services.rs:113`) → `ChildControl::restart` (`N/supervise.rs:688-726`, which clears `Degraded`) |
| `N/components/mod.rs:12-23,480-517,895-911` | component enable (consent → requires → pin → install → ports → start → health), disable, preflight | disable is the undo; files kept | human API only; **not** in any proposal vocabulary |
| `N/dashboard/api_v1/settings.rs:1-27` | `alias`, `lan_enabled` (takes effect on restart), `offer` | no snapshot | human; loopback-only fields |
| `API:351` | `PATCH /fabric/settings`: hub role, LAN listener, custody, quotas | no snapshot | human, loopback-only |
| `N/updater/stage.rs:124-142`; `NI/update_helper.rs:116-232` | the update request → the root helper re-verifies, promotes, journals, floors, flips | probation + revert + guard: the only transactional **code** change | updater loop or `111 update --from` (L7 §9.2) |
| `N/dashboard/api_v1/actions.rs:5-9` | node restart (exit 0, `Restart=always`) | — | human |

What is generic in the applier: `run(Option<id>, actions)` (`N/devices/applier.rs:986-1000`), the record discipline (`DecisionRecord::begin/finish/reopen`,
`:578-590`), `apply_detached` (`:909-915`), and `probe_until` (`:1283-1298`). What is device-specific: `Action` (`:114-137`), `Snapshot`/`restore`
(`:1196-1215`), `SpineProbe`. What is missing: (1) a probation after keep (the 30 s probe is the whole watch); (2) start-up reconciliation of an
`applied: {pending: true}` record. A crash between the apply and the probe leaves the change in place and the proposal reading decided (module doc `:35-38`;
no resume code, grep). (3) An `Action` trait, so `component.enable`, `settings.set` and `update.rollback` can join the same loop.

**(d) Validation a node can run versus CI only**

| Check | Where | Node? |
|---|---|---|
| health predicate, services compute (pure) | `N/health.rs:30`; `N/services/mod.rs:1397` | yes, in-process |
| applier probes (rnstatus, file re-parse, helpers attached) | `N/devices/applier.rs:533-545` | yes, where rnsd and Python run |
| update probation 60 s green/red, 600 s deadline | `N/mod.rs:133-176`; `NI/probation.rs:57` | Linux only: the helper calls `systemctl` (`NI/update_helper.rs:226,262`); no Termux path |
| payload/KB hash verify | `N/updater/stage.rs:53`; `N/kb.rs:32-40` | yes |
| G7 `zero-weight.sh` (reads `/proc`, needs bash) | `6-transport/verify/zero-weight.sh:1-15` | yes: the one gate a node can run on itself |
| G1/G3/G4/G5 | `6-transport/verify/{bridge,lxmf,spine,devices}.sh` need cargo, `unshare -Urnm` (`lib.sh:26`), git, jq, nsenter (`bridge.sh:1064`) | hub/dev box with user namespaces; never a phone |
| `build.py --check` | `0-knowledge/_meta/build.py:8-13,30-35`: Python ≥3.11 + PyYAML | CI installs PyYAML (`.github/workflows/ci.yml:111`); a node has PyYAML only inside `venvs/presence` (`5-messaging/sidecars/meshtastic-presence/meshtastic-presence-lock.txt`); no packaging script shows `_meta/` in `share/kb/` |
| cargo fmt/clippy/test | `ci.yml:75-77,94` | CI only |

**(e) Proposal → decision → application**

| Step | file:line | State |
|---|---|---|
| write | `A/store.rs:14-24,120-140`: `{schema:1,id,node_id,task_id,kind,title,body,created_at}` via `std::fs::write` (not atomic, despite `:2-3`); `A/tools.rs:1226-1236` journals `proposal.written` | exists; unsigned |
| surface | `N/dashboard/api_v1/relay.rs:797-836` → SSE `proposal`; `UI/views/agent/proposals.tsx:1-3,121` ("Applying accepted proposals arrives with Layer 9 (Repairs)") | exists |
| decide | `N/dashboard/api_v1/proposals.rs:636-700`; the agent's bearer is refused on every non-GET (`N/dashboard/api_v1/auth.rs:927-933`) | human only |
| record | `decisions/<id>.json` `{schema:1,decision,at,applied?}`, 0600 atomic (`proposals.rs:455-475`) | exists |
| apply | `message`, `bridge_*`, `device`: applied in the handler. `diagnose`, `propose`, `patch`, `chat`: **recorded only** (`proposals.rs:103-112,688-700`) | **missing for the Layer 9 kinds** |
| audit | `applied` is on disk but not in the API `Proposal` (`proposals.rs:170-180`; `API:746-758`); the session label `by` only on the Layer 6 path (`proposals.rs:681`) | partial: "reverted" is invisible in the UI |

**(f) Packaging and sharing**

| file:line | What exists | Gap |
|---|---|---|
| `ID/keyset.rs:471-480` | `sign_with_tag`, the only signing entry; tags `111/announce/v1`, `attest-node`, `attest-transport`, `commission-challenge` (`ID/announce.rs:37`, `ID/attest.rs:80,83`, `ID/commission.rs:71`) | no `111/proposal/v1` or `111/patch/v1`; register in L2 §6 |
| `N/intelligence.rs:10-12` | "No key material ever reaches" the agent | "the agent signs … with the node key" (`3-intelligence/README.md:49`) can only mean `111d` signs on its behalf |
| `2-identity/README.md:212-221` | agent delegation cert (`111/delegation/v1`, scope, mesh-epoch expiry) pinned; v1 verifiers reject it | an L9 activation would be a soft upgrade; epoch from L7 §8 |
| `M/envelope.rs:512-513,568-590,603-608`; `M/store/rows.rs:43` | `Patch` 0x13: signed, public class, bulk `Class::Patch` | **no producer** (grep); bulk needs Tier-2 `OBJ_*`, not built in v1 (`5-messaging/README.md:238-240`); Tier 1 ≤ ≈1.07–2.2 KB |
| `M/ack.rs:86-107`; `M/envelope.rs:106` | `RECEIPT_STRONG` built and ACK-wired | ready for "the receiving node decrypted this fix" |
| `A/tasks.rs:30-37,394-460` | opencode `run --format json --pure`, a local provider only, `env_clear` + PATH/HOME, 600 s timeout | the work dir holds only `opencode.json`: **no source to patch**; output is a Markdown body |
| `N/kb.rs:1-3`; `7-relay/README.md` §9.4 | KB inside the payload, hash-pinned; "a KB change is a calver respin" | an on-node KB edit turns `kb` red ⇒ `/health` 503 |
| `plan/DECISIONS.md:490` | git bundles rejected (D30) | a fix artefact is not a git object on the node |

### 3.2 Traps (look like seams, are not)

1. **"≤10 s restart"** (`3-intelligence/README.md:51`) holds for the first four exits only; the fifth within 60 s is `Degraded` forever (`N/supervise.rs:61-71,996-1009`).
2. **`/health` "tri-state"** is the agent's. 111d's is ok/error (`N/health.rs:24-41`).
3. **`decisions/*.json` is not an authority.** Every 111 process runs as `node111` and can write it. A Layer 9 applier must act only inside the
   HTTP decision handler, never on a file that appears.
4. **Loopback is the human on Linux** (`N/dashboard/api_v1/auth.rs:3-10,996`). opencode runs with PATH and HOME (`A/tasks.rs:420-436`), and it ships
   a shell tool *(opencode docs; not verified here)*. It could therefore reach `127.0.0.1:11180` with no bearer and decide a proposal. I read this in
   the code and did not test it. A Layer 9 "patch" sandbox must cut the child's loopback access to the API (netns, or a `trust_loopback` exception).
5. **The `node_id` stamp is not a signature.** `proposals.rs` checks it names this node, nothing more.
6. **opencode "adopted on hubs"** has no installer, no component manifest (`1-bootstrap/crates/node111/assets/components/` holds deltachat, lxmf,
   madmail and simplex), and a KB status of `candidate` (`0-knowledge/projects/opencode.md:4`). It is used only if it is on PATH (`A/tasks.rs:207-209`).
7. **KB recipes** must be `install-<project>-on-<device>` (`0-knowledge/README.md:31-32`, a locked spec). A repair playbook is not a valid recipe
   without an amendment.
8. **The UI copy promises a sandbox** ("tests fixes in a sandbox", `UI/lib/layers.ts:75`). No sandbox exists on any node.
9. **The Termux "placeholder seam"** only appends to a log nobody reads (`termux/cron/healthcheck.sh:12-15`).
10. **The probation deadline** is wall-clock epoch seconds (`NI/update_helper.rs:316-319`, `NI/update_guard.rs:91-94`). It is a timer, not crypto or
    ordering, but a clock jump shortens or extends it.
11. **P4 and D26 disagree.** P4 (`plan/cross-layer/parked.md:11`) says "node-child restarts may skip approval but are rate-limited (D17 #7)".
    D26 #2 (`plan/DECISIONS.md:392`) says every restart is a proposal. The owner must choose.
12. **The L7 hub origin** serves only signed release manifests and pinned blobs (`7-relay/README.md` §9.1). It does not carry a patch.

### 3.3 Test counts (as recorded; run nothing)

| Suite | Last recorded | Where |
|---|---|---|
| `node111` | 34 suites green with exactly the 8 known root-only failures (fix wave C) | `6-transport/TASKS.md:90` |
| `agent111` | lib 19 · layer6_tools 20 · daemon_tools 15 · chat_loop 17 · config_guard 22 · events_status 15 · kb_ritual 17 · model_client 8 · task_engine 7 | R07 `00-code-seams.md` §3.3 |
| `id111` | 195 + 44 + 7 + 4 | `6-transport/TASKS.md:61` |
| `mesh111` | 166 lib + 25 | R07 §3.3 |
| UI vitest | 1418 (42 files) | `6-transport/TASKS.md:90` |
| Gates | G1 132 · G3 147 · G4 128 · G5 98 · G7 GREEN · smoke 118 · three-node 159 · panel-lint 144 · LXMF helper 128 | `6-transport/TASKS.md:61,90` |
| Pi floor | release `111` 19,404,064 B, `111-agent` 4,392,280 B; idle RSS 111d 19.7 MB, 111-agent 5.3 MB | `6-transport/TASKS.md:82` |

## 4. Recommendation

**Fork 1: where the observer lives.**
- **(A) Inside `111d`, event-driven:** subscribe to the child watches, `EventBus`, netlink and the start-up tombstones; persist incidents; call the
  agent's task API with the incident as context. Zero timers, no new process. This needs supervisor exit statuses.
- (B) Inside `111-agent`: new read tools plus a poll. The observer dies with the thing it observes, and on a Pi with no agent it is absent.
- (C) A separate Layer 9 process: resident weight on the floor. No.

**Pick A.** On Cat 2A and 3, the record exists without a model, and diagnosis is offloaded or left to a human.

**Fork 2: what an accepted repair applies.**
- (1) The Layer 9 kinds stay advisory, and every change goes through typed kinds.
- (2) **A `repair` kind carrying typed actions:** the device applier's loop is generalised behind an `Action` trait (`component.enable/disable`,
  `service.restart`, `settings.set`, `update.rollback`, the device four). Each action gets a snapshot, a probe, a post-keep probation and start-up
  reconciliation of pending records.
- (3) Free-form patches applied on the node: impossible (no cargo) and unsafe.

**Pick 2.** Code fixes travel only as signed releases (Layer 1's transactional path) or as hub-built PRs that a human merges off-node.

**Fork 3: auto-apply scope.** Either D26 stands (every restart is a proposal), or P4's rate-limited auto-restart applies to a `Degraded` child
inside a user-set scope. Owner call, trap 11.

## 5. Open questions for the owner

1. P4 or D26: may a `Degraded` child be restarted without a human, rate-limited, within a scope the user set?
2. Should 111d sign proposals and fix artefacts with the root key now (a new domain tag), or should Layer 9 activate agent delegation (L2 §9)?
3. Should `PATCH` sharing wait for Tier-2 `OBJ_*` (v1.1)? Or should v1 share only small typed `repair` action sets (≤ Tier 1, ≈1 KB), so a fix
   from a peer arrives as a proposal?
4. Should opencode become a pinned, consented component on hubs with a source checkout, and should its child lose loopback access to `:11180`?
5. Should the KB spec admit a `repair-<service>-<reason_code>` recipe grammar (locked L0 spec amendment)?
6. Should the missed timers be trimmed (`absent_poll` 30/min on agent-less nodes; the `status` heartbeat), and does the ≈265/min baseline become L7's G7 line?

## 6. Sources (files read this run)

`N/supervise.rs`, `N/mod.rs`, `N/health.rs`, `N/watchdog.rs`, `N/intelligence.rs`, `N/kb.rs`, `N/services/{mod,log,probe}.rs`,
`N/dashboard/{mod,routes}.rs`, `N/dashboard/api_v1/{events,status,relay,proposals,services,settings,actions,auth}.rs`, `N/devices/applier.rs`,
`N/components/{mod,runner}.rs`, `N/transport/netlink.rs`, `N/devices/hotplug.rs`, `N/fabric/mod.rs`, `N/updater/mod.rs`, `NI/{update_helper,update_guard,probation}.rs`,
`1-bootstrap/systemd/*`, `1-bootstrap/termux/{sv,cron,boot}/*`, `A/{lib,tasks,store,tools,events,config,chat,http}.rs`, `M/envelope.rs`, `M/store/rows.rs`,
`ID/keyset.rs`, `2-identity/README.md` §8.5–9, `5-messaging/README.md` §5, `6-transport/README.md` §12, `6-transport/verify/{lib,zero-weight,bridge}.sh`,
`6-transport/TASKS.md`, `0-knowledge/_meta/build.py`, `0-knowledge/README.md`, `.github/workflows/ci.yml`, `API`, `UI/lib/layers.ts`,
`UI/views/agent/proposals.tsx`, `UI/views/settings/layer-page.tsx`, `7-relay/README.md` §3.6–3.7, §8, §9, §11, `plan/DECISIONS.md` D10, D17, D26, D30,
`plan/cross-layer/parked.md`, `plan/layers/7-relay/research/00-code-seams.md` §3.
