# Layer 9 - Self-Healing & Evolution: spec v0.1 (draft for the adversarial panel)

The layer that makes a node notice its own failures, explain them and fix the ones it can fix safely, with a human at the last step. **`111d` observes**
every layer the node runs from events it already receives: no timer, no poll, no new process. It records each failure as an **incident**, checks it
against signed **playbooks** without a model, asks Layer 3's model only where one already runs and only to choose among the playbook's causes, and turns
the answer into a **proposal of typed actions**. When the human accepts, `111d` applies each action with a persisted **undo**, probes that the capability
works, and reverts by itself on failure or after its own death. A phone asks its **home hub**. A validated typed fix can be **sent, sealed**, to the home
hub or a pinned contact, where it is a proposal again. A **code fix** is a draft on a hub that opted in, exported for the human to open as a pull request.
Nothing applies code, nothing is shared without a human, and the agent never holds a key or writes anything but a proposal.

Status: **draft v0.1 (2026-11-11)** for the adversarial panel. Written from D35 (the owner took every pick, detail pick, plan correction and default of
the options brief), D36, the brief (`plan/layers/9-evolution/options.md`), its judge's check (`review/options-check.md`: ready), research R00-R07g3, the
Layer 8 draft (`8-storage/README.md` v0.1, whose §3 is the one joint storage contract) and the as-built code. Nothing here is built. No Layer 9 code is
written before the lock; the build board `9-evolution/TASKS.md` follows the lock.

## 0. Reading guide

Reads with: `os/README.md` §9, the layer map row 9, "Category 2A", "Category 3", the operating principles · `os/0-knowledge/README.md` "File
rules", "Frontmatter", "Status lifecycle", "Tooling" · `os/1-bootstrap/README.md` "Supervision", "Updates & rollback" · `os/2-identity/README.md` §6.1,
§9, §13.1 A4, §13.2 A14 · `os/3-intelligence/README.md` "Interfaces", "Tasks" · `os/4-interface/README.md` §6.2, `os/4-interface/api/README.md` ·
`os/5-messaging/README.md` §3, §4, §5, §19 · `os/6-transport/README.md` §2.1, §8, §12, §13 · `os/7-relay/README.md` §3.7, §5, §7.2, §12, §15, §16, §18 ·
`os/8-storage/README.md` §2, §3 (J1-J9), §4, §12.3, §15, §16, §18 · `os/plan/DECISIONS.md` **D35** (the law), **D36**, D37 (the seam map), D29-D32
(the lock precedents), D26 (2), D10 · the options brief (OB) and research R00 (seams), R01-R05 (lenses), R06 (critic), R07g1-R07g3 (gap-fills)
under `os/plan/layers/9-evolution/`.

**Citations.** `N/` = `1-bootstrap/crates/node111/src/daemon` · `NI/` = `1-bootstrap/crates/node111/src` · `M/` = `5-messaging/crates/mesh111/src` ·
`ID/` = `2-identity/crates/id111/src` · `A/` = `3-intelligence/crates/agent111` · `UI/` = `4-interface/ui` · `API` = `4-interface/api/openapi.yaml` ·
`L0`…`L8` = the layer READMEs; locked Layer 7 and the Layer 8 draft are cited by section and item, never by line. Every `file:line` was read on 2026-11-11
at head `fa1daeb`. The working tree carries uncommitted Layer 7 edits under `5-messaging/crates/mesh111/src/store/` and `4-interface/ui/src/`; no citation
here reads them: the two UI files cited (`proposals.tsx`, `settings/index.tsx`) are read from the committed tree. **Evidence:** *(src)* source read ·
*(doc)* docs read by a scout · *(design)* not built · *(unverified from this box)* a claim no scout could fetch (R06 §7, OB §6).

**Map.** §1 scope and the Pi floor · §2-§13 the decided areas in the brief's order (F1 autonomy, F2 observer, F3 state, F4 diagnosis, F5 playbooks,
F6 engine, F7 code fixes, F8 pull requests, F9 signing, F10 sharing, F11 delegation, F12 evolution) · §14 the Repairs page · §15 sequencing (F14) · §16
the sixteen amendments (F13) · §17 settings and consent · §18 the contract · §19 security · §20 verification · §21 Pi-floor accounting · §22 deferred ·
§23 cross-layer seams and amendments · §24 plan corrections, deviations and what stays open for the owner.

**Normative:** numbered rules, byte and path tables, settings with defaults, gate assertions. **Informative:** "why" sentences, arithmetic, cost tables,
§21-§24. Words: an **incident** is one recorded failure or degradation; a **playbook** is a checked procedure for one kind of incident; a **typed action**
is one entry of a closed list of changes `111d` knows how to plan, apply, probe and undo.

**v1 and later** (D35 F14):

| Area | v1 (this release) | Later (recorded, §22) |
|---|---|---|
| Autonomy (§2) | the ladder observe · diagnose · **propose**; the undo of a failed probe and of an apply cut by a daemon death run alone | the auto-apply rung inside a human-written grant (format pinned, §2.3) |
| Observer (§3) | in `111d`, events only; the systemd and runit witness; the Termux cron removed | an opt-in deterministic probe on hubs; PSI triggers; a Termux hang watchdog (Layer 1's question) |
| State (§4) | `heal/*` in Layer 8's `node/state.db` (J1-J9); the J7 ring if the store is not built | a shared fix-attestation feed, only through L8 §12.2's gate |
| Diagnosis (§5) | T0 playbooks on every tier; T1 where Layer 3 runs a model; T2 the home hub | hub-to-hub delegation; T1 numbers after a bench on a Pi |
| Playbooks (§6) | a Layer 0 `playbook` category; grammar v1, twelve predicates | `cel` behind a feature, at the first playbook the enum cannot say |
| Engine (§7) | the `Action` trait; the persisted undo; the reach confirm; the closed v1 vocabulary | more typed actions, each by release |
| Code fixes (§8) | drafts on hubs that opted in: "drafted and applies cleanly, not built" | sandboxed `cargo check`/`test` of the two pure-Rust workspaces on opted-in hubs |
| Pull requests (§9) | `fix-<id>.mbox` and `PR.md`; the human sends | an opt-in single-repository hub token |
| Signing (§10) | the node key through `111d`, two new tags | the L2 delegation cert, with the epoch's first consumer |
| Sharing (§11) | one sealed `PATCH`, sent by a human to the home hub or a pinned contact | the hub fix shelf (a Layer 8 catalogue class), vouched authors, Tier 2 and LoRa fix cards |
| Delegation (§12) | `diagnose`/`diagnosis` over `AGENT_OP`, home hub only, `tcp-lan` and 233 B links | hub-to-hub; the 160 B floor through Tier 2 |
| Evolution (§13) | agent data by release only; `kb_draft` export; three bridge actions; the CI suite | a node overlay after a corpus and a suite exist; hub eval replay |

**Relation to D35, the brief and the README.** D35 is the law: every pick of OB §2 (F1-F14), every detail pick of §3, every plan correction of §4 and
every default of §5 (Q1-Q23). The §3 detail picks are the lead's and open to the panel. Where the brief or a report disagrees with the code, the code
wins and §24 records it; where the code forces a departure from D35, §24 "Deviations" names it for the owner. The joint storage contract is stated once,
in Layer 8 §3; this spec cites it by item (J1-J9) and restates none of it. The root README is the vision (D36): this spec never rewords it. Where v1 builds
less than a README sentence promises, the table below says so and names the version that closes it.

**v1 scope limits against the root README (D36).**

| README sentence | What v1 builds | Closed by |
|---|---|---|
| §9 "continuously observes the health of every layer the device runs" | event-driven observation of what `111d` receives (§3); a hung, not dead, `111d` on a phone is seen by nothing (Q4) | v1.1: the opt-in hub probe (§22); a Termux hang watchdog is Layer 1's |
| §9 "formulates a diagnosis" | T0 for causes a playbook names; T1 only where Layer 3 already runs a model; a phone asks its home hub only under its own consent and only over `tcp-lan` or a 233 B bearer (§12) | v1.1: hub-to-hub delegation; Tier 2 for the 160 B floor |
| §9 "generates and internally validates a corrective change" | a typed change validated by its own capability probe on this node (§7); a code change is a draft that "applies cleanly, not built" (§8) | v1.1: OB F7 C, a sandboxed build on opted-in hubs after a measured Pi 5 and N100 run |
| §9 "packages it as a pull request"; §"Core Goals" "sends updates and PRs" | the hub renders `fix-<id>.mbox` and `PR.md`; the human opens the PR (§9) | v1.1: an opt-in single-repository hub token (OB F8 B) |
| §9 "Nodes share proven fixes with each other through the gateways, so a repair validated on one node can heal others" | a human sends one sealed fix to the home hub or a pinned contact; there it is a proposal its own human accepts (§11) | v1.1: the hub fix shelf with Layer 8, vouched authors, LoRa fix cards |
| §9 "improves the agent itself, expands the Knowledge Layer, and refines bridging logic" | agent data changes only by release; KB growth is an exported draft; bridging grows by three typed actions (§13) | later: a node overlay after a corpus and a must-not-break suite exist |
| §3 "test candidate fixes in a safe internal sandbox" | no sandbox on any node; the words v1 uses are §8's | v1.1: OB F7 C on hubs |
| §3 "The model itself can be updated or expanded by the same self-healing process" | a model pin changes only by a project release with an eval report (Q20) | not planned for nodes (D10 refuses on-node fine-tuning) |
| "Core Goals": "coordinates with other nodes to apply fixes where a proven path exists" | coordination is a human sending and a human accepting (§11, §12) | the auto-apply rung (§2.3) and the fix shelf |

## 1. Scope & non-goals

**What Layer 9 is per device category** (README layer map, row 9: — · ◐ · ✓ · ◐ · ✓ · ✓):

| Category | Layer 9 there |
|---|---|
| Cat 1 | nothing: no 111 code runs on the stock repeater. |
| Cat 2A, 3 (◐) | the observer, the incident record, T0 playbooks, the typed engine, the proposal and the decision run natively; diagnosis beyond T0 is delegated to the home hub (§12) under its own consent; no model loads. |
| Cat 2B (✓) | as 2A, plus T1 where the owner took Layer 3's post-install model offer (`L3:33`). |
| Cat 4A, 4B (✓) | as 2B (T1 with Layer 3's model, `L3:35-36`), plus serving members' delegations, code drafts and the PR export, each behind its own switch, off by default. |

**The Raspberry Pi floor (hard rule, AGENTS.md).**
1. Layer 9 adds no resident process, listener, timer, periodic poll or runtime dependency to the required configuration. The observer is a task inside
   `111d` that awaits channels other tasks already feed (§3). "Sustained for N s" is a one-shot sleep armed by a transition, never an interval.
2. It removes one recurring cost: the Termux cron job that spawns `curl` every 5 min (`1-bootstrap/termux/cron/crontab.line:5`,
   `termux/cron/healthcheck.sh:12-16`).
3. With every switch at its default, nothing leaves the node, Layer 9 loads no model, nothing changes without a human, and listeners, processes and idle
   RSS equal the Layer 8 baseline (G7, §20); Layer 7's no-timer line does not move (L7 §3.7).
4. Hub-only and opt-in costs are enumerated in §21, each behind its own switch and measured at build into `9-evolution/TASKS.md`: one model task at a
   time where a model runs; a hub's delegated diagnoses; opencode while a code draft runs (211-222 MB on disk, R07g1 §2); the release source tarball on
   an opted-in hub.
5. Build-time weight: the observer, evaluator, engine and op arm in `111` and the loader in `111-agent` (deltas measured at build); `diffy` in
   `111-agent` (§8.4).

**Hard rules carried in:** adopt before you build (the Layer 6 applier generalised, systemd's `ExecStopPost=` and runit's `finish` hook, llama-server's
JSON-schema grammar, opencode under a deny block, `diffy`, git's mbox, Layer 8's store) · no wall clock in an ordering, freshness, pruning or crypto
decision · no telemetry · no policy limit of ours: capacity bounds only (Q1) · the agent proposes and a human decides: its bearer is refused on every
non-GET (`N/dashboard/api_v1/auth.rs:927-933`, D26 (2)) · the agent holds zero key material (`L2:234`) · `111d` imports nothing of Reticulum
(`LICENSING.md` §1) · the root README is never reworded (D36).

**Non-goals:** a resident monitor (monit, Netdata, Prometheus: rejected, OB F2 C) · a model loop that asks "anything wrong?" · applying code on a node,
ever · git on a node or a hub (OB F7, D30 F6) · a hash chain or a signature over local records (J4, J5) · a shared log (L8 §12) · a node-local prompt or
playbook overlay (OB F12) · on-node fine-tuning (D10) · a filter or policy language for bridges (README §5) · a project GitHub App or an intake Action
(OB F8 C) · open fix gossip (OB F10 D) · telemetry of any kind.

## 2. Autonomy in v1: propose, with an automatic undo (D35 F1)

**2.1 The ladder (normative).** One setting, `heal.autonomy` (§17), three rungs:

| Rung | What `111d` does | Model calls |
|---|---|---|
| `observe` | records incidents (§3); evaluates nothing | none |
| `diagnose` | + T0 playbook checks, T1 and T2 (§5); the diagnosis is shown on the incident; no action is proposed | at most one at a time, where a model runs |
| **`propose`** (default) | + files a `repair` proposal of typed actions for a diagnosed cause (§7) | as `diagnose` |

`propose` is the default the owner took (D35 F1 A). It changes nothing by itself: a proposal waits for a human.

**2.2 What happens without a human (normative, exhaustive).**
1. The revert of an action whose own probe failed inside its window (§7.4).
2. The revert of an action whose armed undo record is found at daemon start (§7.5: "armed at start = revert").
3. The revert of a reach-class action whose confirm window closed (§7.6).
4. Recording: incidents, the ledger, the audit (§4). Nothing else. The agent's bearer stays refused on every write; no rung applies code (structural, not a
   setting); a delegated reply (§12) and a fix from a peer (§11) never apply without a human, whatever later rung exists.

**2.3 The auto-apply rung: pinned, not built.** A later release may add the rung `auto` without redesign. Its grant is a `heal/grants` record (§4)
written only from a human session: `{schema: 1, id, action, target, count_per_boot, consent_hash, by: "human"}`; `action` is one typed action of §7.2 other
than `component.*`, `update.rollback`, `settings.set` on a consent-gated key, and never identity, keys, the update channel, consent records or code. An
action applied under a grant carries `decision.by = "policy"` in the audit. v1 parses no grant, shows no `auto` rung and accepts no `heal/grants` write.

## 3. The observer inside `111d` (D35 F2)

**3.1 Where it runs and how it wakes (normative).** One task in `111d` (`N/heal/observer.rs`, design). It wakes only on:

| Source | What it gives | Where it exists |
|---|---|---|
| each supervised child's state watch | `Running`, `Backoff`, `Degraded`, `Stopped` transitions | `ChildState` `N/supervise.rs:415-421`; `state_watch()` `:627-629`; the children `111d` supervises: agent, model, transport (`N/mod.rs:1043-1047`), plus meshmud and component runners |
| each child's exit facts (new, §3.2) | exit code or signal, restart count | `let _ = child.wait().await` discards them today (`N/supervise.rs:972`) |
| the `EventBus` | `services` rows on a debounced state change (`N/services/mod.rs:1445-1481`, published `:1537-1548`); `status` with the `/health` components (`N/dashboard/api_v1/status.rs:407-428`); `fabric`; Layer 7's `relay`; Layer 8's `storage` | `subscribe()` `N/dashboard/api_v1/events.rs:148`; ring of 256 (`:36`) |
| netlink link events and uevents | interface and device changes | `RTMGRP_LINK` `N/transport/netlink.rs:18-22`; `NETLINK_KOBJECT_UEVENT` `N/devices/hotplug.rs:33-34` |
| the start-up tombstones (§3.3) | the previous run's end; a reverted update | `state/heal/{run,inbox}` (J2); `state/update.json` (`NI/probation.rs:44-51`) |

Rules: (1) No interval, no sleep loop: frames come from tasks that already tick (the 2 s services registry, `N/services/probe.rs:43`, `:570`; the 2 s status
publisher, `status.rs:62`) and the observer runs inside wakeups they already cause. (2) A subscriber that lags (the ring is 256) re-reads the registry
snapshot once (`Registry::snapshot`, `N/services/mod.rs:1484-1490`) and continues; it never replays the ring. (3) "Sustained for N s" is one
`tokio::time::sleep` armed by a transition and dropped on the reverse transition; N is a policy the code already has (§3.4), never a new constant.

**3.2 What the supervisor publishes (additive, Layer 1 code seam).** `ChildControl` gains `exits() -> watch::Receiver<ExitFacts>` with `{seq, code: Option<i32>,
signal: Option<i32>, restarts_in_window: u32}`, filled from the `ExitStatus` the loop already awaits (`N/supervise.rs:972`) and the schedule's window
(`BackoffSchedule::record_exit`, `:465-485`). Behaviour, back-off and the `Degraded` rule (5 exits in 60 s, `:61-71`; terminal until a restart request,
`:990-1008`) are unchanged byte for byte.

**3.3 Who witnesses `111d`'s own death (normative).** Out of process, event-driven:
1. **Run marker** `state/heal/run` (J2): JSON `{schema: 1, boot_id, release, clean: false, starts}`, written atomically 0600 at start before serving; a
   graceful stop rewrites it with `clean: true`. `boot_id` is `/proc/sys/kernel/random/boot_id`; `release` the basename of `current`
   (`NI/update_helper.rs:49-54`).
2. **Linux:** the unit gains `ExecStopPost=/usr/local/bin/111 heal-witness` (no `+`: it runs as `node111`), which appends one line built from
   `$SERVICE_RESULT`, `$EXIT_CODE`, `$EXIT_STATUS` *(doc, R05 F5)*. With `Restart=always` and `StartLimitIntervalSec=0`
   (`1-bootstrap/systemd/111node.service:18`, `:42-43`) the unit never fails, so `OnFailure=` would never fire.
3. **Termux:** the runit `finish` hook, which today only logs code and signal (`1-bootstrap/termux/sv/111node/finish:2-8`), appends the same line.
4. **The inbox line** `state/heal/inbox` (J2), ASCII, at most 96 B: `v1 boot=<32 hex> result=<word ≤16> code=<int|-> signal=<int|->`. No free text. A writer
   appends only while the file is under 4,096 B (the drafter's number); further exits are counted by the run marker's `starts`.
5. **At start, before serving:** `111d` reads the marker, the inbox and the update tombstone, opens the matching incidents (§3.4), folds the lines into
   `heal/` and clears both files (J2). Same `boot_id` and `clean: false` ⇒ `daemon.unclean_exit` (or `watchdog_kill` when the result word says so); another
   `boot_id` and `clean: false` ⇒ `host.unclean_boot`; a tombstone with `pending: false`, `health_confirmed: false` and `version` ≠ the running release ⇒
   `updater.update_reverted`; a marker `release` above the running one with no such tombstone ⇒ `updater.release_reverted_at_start` (the guard's torn
   payload revert, whose report is only printed today, `NI/update_guard.rs:94-96`).
6. **The cron goes** (F13 item 2): `crontab.line` and `cron-healthcheck.sh` leave the Termux payload; `state/health-failures.log`, which nothing reads, is no
   longer written. A hung `111d` on a phone stays unseen in v1 (Q4).

**3.4 Incidents (normative).** Fingerprint = `(layer, subject, reason_code)`. One open incident per fingerprint: a repeat increments `count` and updates
`last`, so a crash loop is one incident with at most one model task. At most 64 incidents are open at once (R01 §3.2); a 65th fingerprint opens nothing
and increments `overflow` on `GET /api/v1/repairs` (capacity, not policy).

| Layer | Opens on | Subject · reason codes |
|---|---|---|
| 0 | the `kb` row in `error` | `kb` · its `reason_code` (`kb_absent`, `verify_failed`; `N/kb.rs:32-34`) |
| 1 | a child exit with a non-zero code or a signal; a child entering `Degraded`; §3.3's start facts | the child's name · `exit_<code>`, `signal_<n>`, `child_degraded`; `111d` · `unclean_exit`, `watchdog_kill`, `unclean_boot`; `updater` · `update_reverted`, `release_reverted_at_start` |
| 2 | `health.identity` false; `restore_pending` | `identity` · `no_identity`, `restore_pending` |
| 3 | the `agent`, `model` or `codeagent` row in `error` | the row id · its code |
| 5 | the `fabric` or `meshtastic` row in `error` | the row id · its code (`store_full`, `port_busy`, …) |
| 6 | the `transport` or `radio` row in `error`; a component runner leaving `running` | the row or component id · its code (`stack_down`, …) |
| 7, 8 | the `gateway` row's `carrier_pending`; Layer 8's `disk_low`, `store_full` or `node_state: unavailable` | the row or class · its code |
| 9 | a revert that failed; the store unopenable (J2) | `heal` · `revert_failed`, `store_unavailable` |

Service rows are keyed on the published `(id, reason_code)` (`N/services/mod.rs:1420-1424`); optional components are not service rows and are read
through `Components::status` (`N/components/mod.rs:420`). An incident closes as `self_recovered` when its condition clears: a row leaves `error`; a child
has been `Running` for the crash-loop window, `crash_loop_window_secs` (`N/supervise.rs:61-71`), a one-shot sleep armed on the `Running` transition.

**3.5 The incident record (normative shape; R01 §3.2; OB §3).** `{schema: 1, id: "inc-<seq>", fingerprint: {layer, subject, reason_code}, severity:
"warning"|"error"|"critical", count, state, opened: {seq, boot_id, mono_ms, tick, epoch: null, wall_claim}, last: {…}, closed: {…}|null, evidence: [{id, kind,
ref, excerpt}], checks: [{check_id, result}], ruled_out: [cause_id], diagnosis: {cause_id, by: "t0"|"t1"|"hub", confidence: {checks_satisfied,
agreement}}|null, attempts: [{proposal_id, action, decision, outcome}], dismissed: {cause_id}|null, outcome: "open"|"self_recovered"|"fixed"|"unresolved"}`.
`seq` is the store's (J4); `tick` the daemon tick (`N/mod.rs:906`); `epoch` stays `null` until Layer 7's epoch has a consumer (L7 §8); `wall_claim` is
display only. Evidence `kind` is closed: `service_record`, `health`, `counters`, `exit`, `witness`, `update_tombstone`, `child_log_tail`. An excerpt is at
most 512 B and comes only from `111d`'s own typed sources and its own children's logs (agent, model, rnsd, meshmud: `N/dashboard/api_v1/services.rs:10-14`);
never a message body, never a bridge adapter's or the presence sidecar's log text (counters and reason codes only, Q13). Severity is the playbook's, else
`warning`; `critical` is kept for `revert_failed`, a daemon crash loop, `no_identity` and an unopenable store. Field names follow the Kubernetes Event, the
journal's `_BOOT_ID` plus monotonic pair and OpenTelemetry's dotted names; no SDK.

**3.6 Other sources, stated.** Termux: netlink is blocked for apps on Android 11+ *(unverified from this box)*, so hotplug keeps its 5 s fallback
(`N/devices/hotplug.rs:3`) and Layer 9 adds nothing; battery is read only on demand (the page open). Linux PSI triggers are a later opt-in (§22). Layer 9
has no heartbeat of its own (Q8).

## 4. Where Layer 9's state lives: the joint contract, consumed (D35 F3)

The contract is Layer 8 §3, items J1-J9. Layer 9 adds nothing to it. This table says which Layer 9 state lives under which item.

| Layer 9 state | Where | Item | J9 states a reclaim never touches |
|---|---|---|---|
| incidents | `heal/incidents` records | J1, J3, J4 | `open` |
| apply attempts | `heal/attempts` records | J1 | `armed` |
| the loop ledger, keyed `sha256(action ‖ target)` | `heal/ledger` records | J1 | — |
| kept-change snapshots, the last five | `heal/snapshots` records | J1, J3 | — (a snapshot goes with its incident) |
| fixes received, sent and drafted | `heal/fixes` records | J1, J6 | `undecided` |
| delegations (member and hub side) | `heal/delegations` records | J1 | `open` |
| code and KB drafts | `heal/drafts` records | J1 | `undecided` |
| grants (§2.3; none in v1) | `heal/grants` | J1 | — |
| the audit trail | stream `heal/audit` (`entries`) | J1, J4 | — |
| the prune marker | `heal/` | J3 | — |
| the run marker, the witness inbox, the armed undo | `state/heal/run`, `state/heal/inbox`, `state/heal/undo` | J2, J8 | — |
| the fallback ring | `state/heal/journal.jsonl` | J7 | — |

Not under the contract, stated: the proposals Layer 9 composes go into Layer 8's `agent/proposals` and their decisions into `agent/decisions` (L8 §2.2
rule 5, §2.4 rule 5): one proposal list for the UI. Every new or changed path on a node:

| Path | Owner · mode | Writer | When | Size |
|---|---|---|---|---|
| `node/state.db`, namespaces `heal/*` | Layer 8's file (L8 §2.2) | `111d` through J9 | on a Layer 9 event | the `heal` class (L8 §4.1) |
| `state/heal/` | `node111` · 0700 (J8) | — | created at the first start | — |
| `state/heal/run` | 0600, atomic | `111d` | at start and at a graceful stop | ≤ 128 B |
| `state/heal/inbox` | 0600, append | `111 heal-witness`, the runit `finish` hook | at each daemon exit; cleared at start | ≤ 4,096 B |
| `state/heal/undo` | 0600, atomic, fsync | `111d` | before an action's first effect; removed at keep or revert | one snapshot (measured at build) |
| `state/heal/journal.jsonl` | 0600 | `111d` | J7 only | ≤ 1 MiB ring |
| `state/heal.json` | 0600, atomic | `111d` (`PATCH /repairs/settings`) | on a settings change only | ≤ 1 KiB |
| `agent/work/<task>/{opencode.json, home/, src/}` | the agent's (`A/src/tasks.rs:397`) | `111-agent` | a `patch` task on an opted-in hub | the source tarball unpacked (measured at build) |
| `cache/packages/opencode/` | root · 0755 (the components cache) | `111 install --components opencode` | on that verb | 211-222 MB plus the source tarball |
| `current/share/kb/playbooks/*.md` · `current/share/agent/` | the release payload, read-only | the release | each release | measured at build |
| the install receipt's `apk` map | the receipt (`NI/install/receipt.rs:21-45`) | the phone installer | `111 install --phone` | ≤ 3 entries |
| removed: `share/termux/cron-healthcheck.sh`, the crontab entry, `state/health-failures.log` | — | — | — | − |

Every Layer 9 write is one J9 call (`put`, `append`, `set_state`); the audit record is
`{seq, boot_id, mono_ms, kind, proposal_id, action_hash, probe_result, revert_result, confirmer: "probe"|"human", by: "human"|"daemon"}` with `kind` in
`proposed`, `accepted`, `applied`, `kept`, `reverted`, `revert_failed`, `dismissed`, `undo`, `fix_sent`, `fix_received`, `delegated`, `refused`. Nothing in
`heal/` is signed (J5) or leaves the node (J6).

## 5. Diagnosis: deterministic first, the model only to choose (D35 F4)

**5.1 The tiers (normative).** Layer 9 never loads a model by itself.
- **T0** (every tier that runs Layer 9): `111d` evaluates the playbooks whose `[match]` equals the fingerprint (§6). Exactly one cause whose checks all hold
  ⇒ that cause, `by: "t0"`.
- **T1** (only where the `model` row is live, under `diagnose` or `propose`, and only when T0 matched several causes or none): one background task through
  the existing loopback task API (`POST 127.0.0.1:11182/v1/agent/task`, the route `111d` relays today, `N/dashboard/api_v1/relay.rs:725`, `:738`), with the
  new `incident` input (§5.2). The answer is a pure classification constrained by a JSON schema to the playbook's cause ids, which llama-server turns into a
  GBNF grammar *(doc, R01 §3.3)*. Any narrative is a separate call, made only when the human asks ("Explain").
- **T2**: on a member with `heal.ask_hub` on, the home hub (§12); otherwise the card says "needs a hub" and links to the switch.
- None of the three ⇒ the incident reads `unable_to_diagnose`, honestly.

**5.2 The `incident` input (amends Layer 3, additive).** `TaskRequest` (`A/src/http.rs:159-164`, `API:832-838`) gains an optional `incident` object (§3.5's
fields, minus `attempts`) and `causes: [{id, text}]`. With it, `diagnose` renders the incident inside a labelled untrusted block, switches off
`daemon_health` and `recent_events` for that task (today's context, `A/src/tasks.rs:360-390`) and sends `response_format` with the schema
`{cause_id: enum, evidence: [id] ≤3, quotes: [string ≤120 B] ≤3}`. The prompt is ≈0.5 KB (R07g2 F4), never today's ≈6,000-token context (R01 §3.4,
estimate). The result is a `kind: diagnose` proposal whose body is that JSON; `111d` ingests it on `task.done` (`relay.rs:826`), folds it into the
incident, and never shows it in "Needs you".

**5.3 Containment, outside the model (normative).** A reply whose `cause_id` is not in the set, whose evidence ids are not in the incident, or whose
quotes are not substrings of its excerpts is discarded, recorded and never retried. Confidence is computed, two fields: `checks_satisfied` (per-mille of
the cause's checks that held) and `agreement` (`rule_only`, `model_only`, `agree`, `disagree`). The model never grades itself; the output is a proposal.
`111d` submits one model task at a time per node (the agent does not serialise concurrent turns, `3-intelligence/TASKS.md:72`; `TaskEngine::submit` starts
one thread per task with no cap, `A/src/tasks.rs:137-184`); further incidents wait in memory, bounded by the 64 open incidents.

**5.4 Cost, stated.** T0: no model, no allocation beyond the parsed playbooks. T1: one call per unresolved incident, about a minute for a short verdict on a
Pi 5 *(unverified from this box: 2.3 tok/s, a search snippet, R01 §3.4)*; the reload after `--sleep-idle-seconds` is measured at build (Q21's bench).

## 6. Playbooks: a Layer 0 category with a closed grammar (D35 F5)

**6.1 The entry (amends L0, additive).** `category: playbook` joins the enum (`L0:45`; rule F05, `0-knowledge/_meta/schema.md:19`), folder `playbooks/`,
filename `repair-<subject>-<reason>.md` (`_` written as `-`; `any` for a subject wildcard). No new frontmatter field (the twelve of `L0:40` and F02, `schema.md:16`):
`relations` carry `depends_on` the project or component it repairs and `runs_on` the devices it applies to; `upstream` and `license` are omitted (first-party).
The six H2 sections stay (B02, `schema.md:38`); under `## Details` the H3s are `### Symptom` · `### Checks` (one fenced `toml` block) · `### Causes` (one line
per cause id) · `### Verify` · `### Undo`. `proven` means it healed a real incident, logged in `### Field log` (`L0:88`). Recipes and N03 (`schema.md:53`,
`L0:32`) are untouched; `skills/` stays deferred (`L0:143`). Body budget 2,000 tokens, warning-only (B09, `schema.md:45`).

**6.2 Grammar v1 (normative).** `schema` is an integer; `MAX_SCHEMA` is compiled in; every table is `deny_unknown_fields`; the predicate enum is internally
tagged on `p` with no fallback variant. The twelve predicates (Q9 default):

| `p` | Fields | Reads |
|---|---|---|
| `service_state` | `id`, `is` ∈ `running`, `backoff`, `degraded`, `stopped` | the child watch (`N/supervise.rs:415-421`) |
| `service_reason` | `id`, `is` = a compiled `reason_code` | the published row (`API:708` lists the emitted set) |
| `restarts_at_least` | `id`, `n`, `within_events` ≤ 64 | the last exit facts (§3.2) |
| `incident_count_at_least` | `n` | the open incident's `count` |
| `exit_is` | `id`, `code` or `signal` | the last exit facts |
| `health_is` | `of` ∈ `daemon` (`ok`, `error`; `N/health.rs:24-40`) or `agent` (`ok`, `degraded`, `error`; `L3:84`), `is` | the `status` frame; the agent's `/health` |
| `file_exists` | `root` ∈ `state`, `run`, `data`; `rel` matching `^[a-z0-9._-]+(/[a-z0-9._-]+)*$` | one `metadata()` call, never a read |
| `log_contains` | `id` (an own child), `literal` ≤ 64 B | the last 100 lines or 8 KiB of the sanitised tail (`N/services/log.rs:12-17`); no regex |
| `disk_free_below` | `percent` | the fabric's `statvfs` (`N/fabric/mod.rs:420-432`) |
| `unclean_exit` | — | §3.3's marker |
| `component_enabled` | `id`, `is` | `Components::status` |
| `release_at_least` | `calver` | the running release |

Combinators `all`, `any`, `not`; depth ≤ 3, ≤ 16 leaves, the block ≤ 2 KiB. A cause: `{id, when: [check ids], action, verify}`. `action` deserialises into the
engine's own action type (§7.2) plus `advise {text}` and `escalate {task}`; the one non-literal is `"@subject"`, replaced by the incident's subject (a closed
set). No templating, no command, no URL, no environment. An unknown predicate, field, service id or reason code rejects the whole playbook, which the
Repairs page lists as "unsupported grammar N, needs release X"; it never runs in part. A predicate reading something this node lacks (a component not
installed) evaluates false. Prior art read: StackStorm rules, the Salt reactor, Nagios and Icinga handlers, Ansible handlers, Kubernetes probes, MAPE-K
(R07g3 §3.4); ours closes the action side too. `cel` 0.15.0 (MIT) is the revisit path at the first playbook the enum cannot say (OB F5 B).

**6.3 The evaluator and the lint (normative).** `111d` reads `current/share/kb/playbooks/*.md` once at start (the KB rides the payload, `N/kb.rs:1-3`),
extracts the block under `### Checks`, parses it with the `toml` crate it already links, and keeps only each playbook's `[match]` key and load state (the
Repairs page's list). A playbook is parsed again when an incident with its fingerprint opens and dropped when none is open, so an idle node holds no parsed
playbook. `build.py` (Python ≥ 3.11, `0-knowledge/_meta/build.py:8`) lints the same rules with the standard library's `tomllib`: no new Python dependency
(it needs PyYAML only, `:31`). Fixtures
(facts in, matched check ids out) live beside the Rust loader's tests, never in the payload (Q9). The first release ships playbooks for the services'
reason codes the table of §3.4 names; each enters as `candidate`.

## 7. The change engine: typed actions behind an `Action` trait (D35 F6)

**7.1 The trait (amends L6 §12, additive).** The Layer 6 applier is the template: a pending record first, one apply at a time, per action plan → snapshot
→ apply → probe → keep or revert, a detached task (`N/devices/applier.rs:24-37`, `:888-896`, `:909-914`, `:992-1004`, `:1283-1298`). Its device-specific
half (`Action`, `:113-136`; `Snapshot`, `:835-839`; `snapshot`/`restore`, `:1196-1232`) moves behind a trait in `N/heal/engine.rs` (design):
`name()` · `plan(&Facts) -> Plan {diff, preflight, reach: none|may_affect, removes_capability: bool}` (pure: the applier already checks a copy before any
write, `:24-27`) · `snapshot() -> Bytes` · `apply()` · `probe() -> Result<(), why>` (asserts a capability, never "is it quiet?") · `restore(Bytes)`. The four
device actions are ported byte-for-byte (D29); `kind: device` proposals keep their route, body and results (`API:848`).

**7.2 The v1 vocabulary (closed, normative).**

| Action | Apply | Probe within 30 s (`PROBE_WINDOW`, `N/devices/applier.rs:84-87`) | Undo | Reach |
|---|---|---|---|---|
| `service.restart {service_id}` | `restart_service` (`N/dashboard/api_v1/services.rs:113`), which also clears `Degraded` (`N/supervise.rs:682-723`) | the child is `Running` and the row leaves `child_degraded` | none: "a restart cannot be undone" | none |
| `rns.interface.upsert`, `.remove`, `rns.posture.set`, `radio.region.set` | as built (D29) | as built (`N/devices/applier.rs:11-18`) | the snapshot | none |
| `settings.set {key, value}`, keys `fabric.listen_lan`, `fabric.hub_role`, `fabric.quota.<class>` | through the `PATCH /fabric/settings` code path | the listener's bound state; the quota in `GET /fabric` | the previous value | `may_affect` for the first two |
| `component.enable {id}` · `component.disable {id}` | `Components::enable` with the component's own consent and cached payload (`N/components/mod.rs:12-40`, `:480`) · `stop_and_wait` | `running` and its health · `stopped` | disable · enable | none |
| `bridge.edge.pause {edge_id}` · `bridge.edge.set {edge_id, oversize?, honour_ok_to_mqtt?}` | `PATCH /bridge/edges/{id}` fields (`API:482`) | the edge reads the values | the previous values | none |
| `bridge.order.set {order}` | `interfaces.toml` `[order]` through the spine | the file re-parses with that order | the snapshot | none |
| `update.rollback {}` | writes `state/revert-request.json` while a probation is pending; the helper flips and restarts (`NI/update_helper.rs:282-308`) | at the next start: the running release is the journal's `previous` (the tombstone, §3.3) | none (Layer 1's probation) | none |
| `advise {text}` · `human_step {steps}` | nothing runs | — | — | — |

Never typed: `firmware.flash`. Still refused `unsupported_action`: `meshtastic.set`, `ble.pair`, `ble.forget` (`N/devices/applier.rs:20-22`, `:101-103`).
Out of scope: identity, keys, the update channel, consent records, any consent-gated setting (a carrier, the gateway bit, origin serving). Three code facts
bound the table: `update.rollback` acts only during an open probation, because the helper ignores a revert request outside one
(`NI/update_helper.rs:297-300`); outside it the action is refused and a `human_step` explains Layer 1's monotonic floor (§24 C1). A component whose
payload is not cached answers `component_unavailable` (enable never downloads, `N/components/mod.rs:36-40`), so the proposal becomes a `human_step` naming
`sudo 111 install --components <id>` (§24 C3). `fabric.listen_lan` and `fabric.hub_role` are the only settings keys because each has a probe; `lan_enabled`
takes effect only at daemon restart (`N/dashboard/api_v1/settings.rs:14-15`) and is refused (Q6).

**7.3 The `repair` proposal (normative).** `111d` composes it for a diagnosed cause under `propose`: kind `repair`, body one JSON object `{"why", "incident",
"playbook", "cause", "actions": [Action]}`, at most 8 actions (`MAX_ACTIONS`, `N/devices/applier.rs:89`), at most 16 KiB (`MAX_BODY`, `:91`), stamped with
this node's id, `task_id: null`. It renders as a card from the parsed JSON, never as Markdown. Accepting it (`POST /api/v1/proposals/{id}/decision`) runs
§7.4; a consent gate answers `409 consent_required` and the human repeats with `accept_warning`. A proposal whose net effect removes a capability is
refused unless that is its stated purpose in `why` and the human accepted that text (Q6). `diagnose` and `propose` proposals stay advisory: accepting one
records "read" (`N/dashboard/api_v1/proposals.rs:698`), as today.

**7.4 The loop (normative).** Per action, in order, stopping at the first not kept (D29 (4)):
1. `plan()`; a refusal writes nothing and the proposal stays open.
2. **Arm the undo** before the first effect: `state/heal/undo` (J2) = `{schema: 1, proposal_id, attempt_id, action, target, snapshot (base64), armed:
   {boot_id, mono_ms}}`, atomic, 0600, fsync. The decision record reads `applied: {pending: true}` as today (`N/devices/applier.rs:992-1004`).
3. `apply()`, then `probe()` within its window: kept, or `restore(snapshot)` and `reverted` with the reason. A failed restore is `revert_failed`: the loudest
   state, a `critical` incident, the human's.
4. Kept: the snapshot moves to `heal/snapshots`, the undo file is removed, the attempt and the ledger are written, the decision record gets the result.

**7.5 Armed at start = revert (amends L6 §12, a correction).** `111d` reads `state/heal/undo` at start before serving. Present ⇒ restore the snapshot, record
`reverted (daemon restart)` and finish the decision record; for `service.restart`, record `interrupted`; for `update.rollback`, whose effect is the
restart, the tombstone decides `kept` or `not_applied`. This replaces "a crash mid-apply leaves a visibly
half-applied proposal" (`L6:657-658`; `N/devices/applier.rs:35-37`); "never a second apply" stands. No clock: the rule is ESP-IDF's `PENDING_VERIFY` state
*(doc, R02 F7)*.

**7.6 The reach class (normative).** A `may_affect` action applies, probes, then waits for `POST /api/v1/repairs/attempts/{id}/confirm` from a human session
through the same dashboard URL within 90 s on a monotonic clock (LuCI's window, a search snippet, R02 F8; Q5); else it reverts. The sleep exists only while
one is pending. On Termux, where loopback is not trusted (`N/dashboard/api_v1/auth.rs:3-10`), the paired phone browser confirms. An action that would
remove the last pairing or the loopback listener is refused `409 would_lock_out`.

**7.7 After the keep (normative).** No window and no re-probe. The last five kept snapshots are retained by capacity (Q7); a new incident on a target whose
last kept change still has a snapshot offers "Undo <change>" (linked by target, count-free, clock-free); Undo is a human act that runs the same loop. The
ledger (`heal/ledger`, `{applied, reverted, last_cause, last_evidence_hash}`): after a revert, the same action for the same cause is not re-proposed until
the human asks again from the incident or the evidence hash changes. A target with an armed undo answers `409 busy`.

**7.8 Validation by category, said plainly in the UI.** Every node runs `plan()` and the action's own probe (a phone gets exactly that). A Pi adds supervisor
events as probe inputs. A hub adds, for pull requests only, §8. "Validated" is never self-assigned: the card names the probe that passed.

## 8. Code fixes: drafts on a hub that opted in (D35 F7)

**8.1 The containment correction (amends Layer 3; lands first, §15).** As built, opencode runs with no `permission` block (`A/src/tasks.rs:495-509` writes a
provider and a model only), keeps the node's `PATH` and `HOME` (`:429-433`), and its defaults allow bash and web fetch *(doc, R06 §2.1; R07g1 F1)*. With
loopback trusted as the human on Linux (`N/dashboard/api_v1/auth.rs:3-10`, `:995-996`), a confused model could decide a proposal with a bare `curl` to
`127.0.0.1:11180`: a self-approval path, read in code. The fix, under every option: `write_opencode_config` writes `{"permission": {"*": "deny", "read":
"allow", "glob": "allow", "grep": "allow", "edit": "deny", "bash": "deny", "webfetch": "deny", "websearch": "deny", "external_directory": "deny", "task":
"deny", "skill": "deny", "lsp": "deny", "question": "deny"}, "autoupdate": false, "share": "disabled"}` (R07g1 F2); `HOME` and the XDG directories point
into the task's work directory; the child gets the five `OPENCODE_DISABLE_*` flags R07g1 F2 lists set to `1` (`…_MODELS_FETCH`, `…_AUTOUPDATE`,
`…_LSP_DOWNLOAD`, `…_EXTERNAL_SKILLS`, and the flag that stops it reading another assistant's home config); `--auto` is never passed (`OPENCODE_ARGV`,
`A/src/tasks.rs:32`); the `npm` value stays the constant (a different one triggers a download, R07g1 F4). This restores `L3:55` ("no host shell for the
agent") on hubs. Not landed at `fa1daeb`.

**8.2 The patch component (amends L1 and L6 additively).** opencode 1.19.5 (MIT, licence read, R07g1 §2) becomes a declared optional component,
`1-bootstrap/crates/node111/assets/components/opencode.toml` (none exists: the folder holds `deltachat`, `lxmf`, `madmail`, `simplex`), hub targets only,
`[install] prefetch = false` (the D29 (1) pattern), its licence notice acknowledged at enable (D26 (5)). `sudo 111 install --components opencode` fetches
its pinned payload and the release's source tarball, the `[source]` pin Layer 8 §11.6 adds to the manifest: one pin, two consumers (§24 D4). Enabling
needs `hub_role`. The KB entry stays `candidate` (`0-knowledge/projects/opencode.md:4`) until its smoke log.

**8.3 The draft (normative).** On a hub with `heal.code_drafts` on and the component enabled, an `escalate {task: "patch"}` cause or a human's request
submits a `patch` task. The agent unpacks the source tarball read-only into `agent/work/<task>/src/` (the work dir exists, `A/src/tasks.rs:397`), runs
opencode under §8.1 with that tree as its cwd, and takes its unified diff as text. Edit is denied, so opencode can only read and answer.

**8.4 "Applies cleanly" (normative).** `111-agent` parses the diff and applies it to an in-memory copy of the pinned tree with `diffy` 0.5.2 (MIT OR
Apache-2.0, both licence files read, R07g1 §2): no git, no `patch`. The proposal says **"drafted and applies cleanly, not built"** or "drafted, does not
apply". Accepting a `patch` proposal on a hub creates a `heal/drafts` record of class `code`; nothing is applied anywhere. `diffy` is build-time weight on
every `111-agent` (delta measured at build; a `LICENSING.md` §5 row and a notices regeneration); D35 took the pick, and §24 "Open for the owner" asks
for the AGENTS.md sign-off on the weight.

**8.5 What "validated" means for code.** README §9's "internally validates" means the action's own probe for a typed change, and "applies cleanly" before
the PR plus upstream CI after it for code (OB F7, conflict 7). A pre-PR sandbox build (bubblewrap 0.13.1, LGPL-2.1, or nsjail, Apache-2.0; a pinned Rust
toolchain, 111-127 MB xz) is OB F7 C, later, after a measured cold `cargo check` of `3-intelligence` on a Pi 5 and an N100 (Q12). `patch` ops stay out of
§12's op set until §8.1 is built and G24 is green.

## 9. Pull requests: the human sends (D35 F8)

1. **The export.** For a `code` or `kb` draft, `GET /api/v1/repairs/drafts/{id}/mbox` renders `fix-<id>.mbox`, a `git am` mbox: the fixed first line
   `From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001` (git's datestamp, so no wall clock), `From: <node label> <node@111.invalid>` (an
   RFC 2606 name the human replaces with `git commit --amend --reset-author`), `Subject: [PATCH] <title>`, the body, `Assisted-by:` trailers naming each tool
   and its version, the diff, and `base-commit:` set to the release's tagged commit when the `[source]` pin names it. `GET …/pr` renders `PR.md`: fix id,
   author node, incident id, the diagnosis as a quoted block, the verify log `111d` recorded, what was not verified, the tools and versions.
2. **The agent never adds `Signed-off-by:`.** The PR template gains a DCO 1.1 sign-off line the human fills; no CI check in v1; D22's inbound = outbound
   stands (Q2). The licence grant comes from the human's review and sign-off (R03 finding 9; not legal advice).
3. **The owner runs** `git am` and `gh pr create` (MIT, on their own machine). A phone gets a prefilled GitHub issue URL for `typed` and `kb` fixes (R03
   §4.2 B3). One PR per fix id, never retried automatically. No token on any device in v1.

## 10. Signing: `111d` signs what leaves the node (D35 F9)

**10.1 Who signs.** The node key, through `111d`, the identity holder: `Keyset::sign_with_tag` (`ID/keyset.rs:475`) behind node111's `sign(tag, msg)`
(`N/fabric/sealer.rs:432-433`). The agent holds zero key material (`L2:234`; "No key material ever reaches it", `N/intelligence.rs:10-12`). `111d` builds the
signed bytes from records it holds and never signs bytes the agent hands it: no signing oracle.

**10.2 The two tags (normative; registered in L2 §13.3 A19, Layer 8 §21 L2's block).**

| Tag | Signed input | Verified with |
|---|---|---|
| `111/fix/v1` | Ed25519 by the author's node key over `tag ‖ body_len (u32 LE) ‖ body ‖ attachment_hash (32)`; `attachment_hash` = BLAKE3-256 of the attachment, or 32 zero bytes | `verify_strict_tagged` (`ID/keyset.rs:627`) under the author NodeId the body names |
| `111/fix-vouch/v1` | Ed25519 by the voucher's node key over `tag ‖ voucher (32) ‖ fix_id (32) ‖ kind (1) ‖ voucher_seq (u64 LE)`; `kind` 1 `vouched`, 2 `reproduced`, 3 `accepted`, 4 `reverted` | as above, under `voucher` |

`fix_id` = BLAKE3-256 of the body bytes. Proposals, incidents and audit records are never signed: L2 §6.1 keeps identity keys to "announces, proofs, and
control artifacts only — never routine traffic" (`L2:91`), and Layer 9 signs nothing that stays on the node (J5). `AGENT_OP` payloads need no new tag:
`111/msg/v1` signs them (`L2:358`). Known-answer vectors: `9-evolution/verify/vectors/fix-v1.json` (id111 gains no code, the A14 precedent).

## 11. Sharing proven fixes between nodes (D35 F10)

**11.1 The artefact (normative).** `fix/v1` body: UTF-8 JSON, signed as exact bytes, never re-serialised, at most 4,096 B: `{schema: 1, author, author_seq,
epoch|null, class: "typed"|"code"|"kb", applies_to: {category, layers_run, release, components}, incident: {fingerprint, diagnosis ≤512 B}, actions: [Action]
(typed only), evidence: {probe, result, before_hash, after_hash, reverted: false}, not_verified: [string], attachment: {kind: "diff", bytes, blake3}|null}`.
`author_seq` is a per-author counter (a replay is not increasing); no wall clock.

**11.2 The wire (normative).** A fix rides one sealed `PATCH` (0x13) to one recipient. `PATCH` is SIGNED-mandatory and a public class (`L5:141-142`;
`requires_sig` and `is_public_class`, `M/envelope.rs:595-612`): the envelope signature binds no recipient, and the fix's own signature makes it transferable.

| `PATCH` payload | Bytes |
|---|---|
| type | 1: `0x01` fix, `0x02` vouch |
| fix: `body_len` (u32 LE) ‖ body ‖ `att_len` (u32 LE) ‖ attachment ‖ `sig` | 4 + ≤ 4,096 + 4 + attachment + 64 |
| vouch: `fix_id` ‖ `kind` ‖ `voucher_seq` (u64 LE) ‖ `sig` (the voucher is the frame's authenticated sender) | 32 + 1 + 8 + 64 = 105 |

Size: the whole payload fits one SIGNED `tcp-lan` frame (`MAX_PACKET_LEN` 65,535 B, `M/envelope.rs:81`; `payload_budget`, `:1395-1413`; 65,445 B unsigned
in its test, `:2198`, so about 65,381 B SIGNED, arithmetic), so a code attachment is at most that budget minus the body and 73 B of framing, not the
brief's 64 KiB (§24 D5). A 233 B bearer carries a fix only when it fits 16 SIGNED fragments, about 1.2 KB (R07g2 F4); never the 160 B floor. Across
islands it rides Layer 7's custody hand-over like any sealed item (L7 §5); carrier bytes count in Layer 7's metered cap (L7 §12).

**11.3 Sending (normative).** Only a human sends: "Send this fix" on a kept change, `POST /api/v1/repairs/fixes/{id}/send {to}`, where `to` is the home hub
or a pinned contact as `heal.fixes.send` allows; off by default. The consent `fix_send` (§17) shows the exact bytes that would leave. `111d` builds the body
from the incident, the attempt and the verify log, signs it (§10) and sends it through `Fabric::send_op` (§23 L5).

**11.4 Receiving (normative), all before a human sees a button.**
1. The fabric hands the `PATCH` body to Layer 9's consumer (L8 §12.3), never to a message row; `RECEIPT_STRONG` returns the hash (`M/envelope.rs:113`).
2. Verify the signature; a bad one is dropped and counted.
3. The author must be the home hub, or a pinned contact, as `heal.fixes.receive` allows; else the fix is parked "unknown author" with no action.
4. `code` and `kb` classes show a read-only diff and offer only "Submit upstream" (§9), never "apply".
5. `applies_to` is checked against this node's category, every layer it runs and its release; a mismatch reads "does not apply here".
6. Typed actions are re-validated against this node's own vocabulary (§7.2) and a scope list that refuses anything touching the gateway, a carrier,
   identity, keys, the update channel or consent records.
7. Only then a `fix` proposal is filed. Accepting it runs §7.4; the outcome goes back to the author as a `fix-vouch` (`accepted` or `reverted`).

The receiving agent never reads or summarises foreign fix text: `111d` renders the card from typed fields and the prose preformatted (OB conflict 8).
Incoming fixes queue at most 16 per author, oldest dropped and counted (Q18; the fabric's quota pattern).

**11.5 Maturity (normative).** `local` → `reproduced` (a second node of the same category re-ran the probes) → `accepted_by_n` (n owners accepted it and none
reverted it within the next K incidents of its fingerprint: a count, not a period; K = 3, the drafter's number for the panel) → `upstream_merged` →
`in_release`. Only other parties' signed vouches move a fix past `local`.

## 12. Delegation for Cat 2A and 3: the home-hub `AGENT_OP` pair (D35 F11)

**12.1 The op set (normative, closed).** `AGENT_OP` (0x10) is a reserved byte with no schema, sender or consumer today (R07g2 F1); SIGNED-mandatory,
interactive (`L5:175`; `M/envelope.rs:617`). v1 gives it three ops. Payload: `schema (1) = 0x01 ‖ op (1) ‖ a deterministic CBOR map (RFC 8949 §4.2.1)` with
unsigned integer keys; an old node drops an unknown `schema`.

| Op | Direction | Map keys (each capped) | Size |
|---|---|---|---|
| `0x01` `diagnose` | member → home hub | 0 `req_id` (bstr ≤ 8) · 1 fingerprint `[layer, subject ≤24, reason ≤24]` · 2 severity · 3 count · 4 release (≤10) · 5 target triple (≤32) · 6 category (≤3) · 7 checks `[[check_id ≤16, bool]]` ≤ 16 · 8 ruled-out cause ids ≤ 8 · 9 excerpts ≤ 3 × 120 B | ≈ 450-600 B (estimate) |
| `0x02` `diagnosis` | hub → member | 0 `req_id` · 1 hub release · 2 `cause_id` (≤24, or `unknown`) · 3 `checks_satisfied` (per-mille) · 4 `agreement` (0-3) · 5 actions ≤ 3 × ≤ 80 B `{0 action, 1 target, 2 args}` · 6 narrative ≤ 256 B | ≈ 400-600 B (estimate) |
| `0x03` `refuse` | hub → member | 0 `req_id` · 1 reason: 1 not serving, 2 not enrolled, 3 busy, 4 nothing to say, 5 unsupported schema | ≤ 16 B |

The narrative is the hub playbook's one-line cause text, never model text. `111d` builds the request from the incident record; nothing from a bridge
adapter or presence log rides in it (Q13).

**12.2 The size gate and the queue (normative).** Before sealing, `send_op` checks `payload_budget(profile) × 16 ≥ len` for the path's profile; otherwise the
request stays `queued` with the reason "needs a longer link". v1 therefore delegates over `tcp-lan` and 233 B bearers; on the 160 B floor a SIGNED fragment
carries 3-5 B (R07g2 F4). The request waits in the member's outbox until a path exists; nothing polls (L5's outbox, `M/fabric.rs:1237-1243`). It rides
stream 0, as every v1 packet does: the inner layout has no stream field (`M/ids.rs:19-22`; the open path sets `STREAM_1TO1`, `M/envelope.rs:1373`, `:1383`), so
OB F13 item 12's `STREAM_AGENT = 1` is not carried in v1 (§24 D1).

**12.3 Trust (normative).** The hub accepts `diagnose` only from an `enrolled` member on an attributed connection, at most one open request per member,
and only while `heal.serve_members` is on; anything else is dropped and counted, or answered `refuse`. The member accepts `diagnosis` or `refuse` only from
its `home_hub`, on an attributed connection, for an open `req_id`. A contact is never a delegate. A hub stores no member's op for others (a put whose
`dest` is the hub itself is refused, L7 §5).

**12.4 The hub side (normative).** `111d` serialises one delegated diagnosis at a time over loopback (§5.2, with the member's incident as the untrusted
`incident` and its own health and journal switched off), loads the model on demand, and answers from T0 alone when no model runs and exactly one cause of
its playbook for that fingerprint is satisfied by the member's checks; else `refuse` reason 4. A duplicate `req_id` gets the stored reply.

**12.5 The member side (normative).** Under `heal.ask_hub` (off by default; consent `heal_delegate` lists the exact request fields), `111d` sends the request
when an incident opens with no T0 cause and the home hub's card carries `ROLE_HEAL` (§23 L7); off, nothing is composed or sent. The reply becomes a
proposal the member's human accepts, never auto-applied, its actions re-validated against this node's vocabulary, provenance "from your hub <alias>, not
checked by this phone". Closing the incident cancels the request; a reply for a closed `req_id` is dropped.

**12.6 The APK refresh (local, not delegated; `L1:185`).** The phone installer writes a new additive `receipt.apk` map (package → sha256) into the install
receipt (`NI/install/receipt.rs:21-45`; the three pins, `NI/install/phone/apk.rs:10-15`). After each verified manifest check, `111d` compares the manifest's
`apk.*` pins (`NI/manifest.rs:639`) with `receipt.apk`; a difference files one `human_step` "A newer Termux add-on is pinned", with the steps "On your
computer, connected to this phone, run `111 install --phone` again". It never implies an automatic install.

## 13. Evolution: the agent, the KB and the bridging logic (D35 F12)

1. **Agent data by release only; the seam is built now.** The chat preamble and the task prompts move out of Rust constants (`A/src/chat.rs:37`,
   `A/src/tasks.rs:326-333`, `:401-412`) into `current/share/agent/prompts/{chat,diagnose,propose,patch}.md` and `share/agent/schemas/diagnosis.json`, pinned in
   the payload manifest and verified at daemon start like the KB (`N/kb.rs:32-34`); a mismatch marks the `agent` row `error` with `prompts_invalid` and
   `111d` does not start the agent. The contract and the fixed tool set (D10) do not move. The agent's 5.3 MB idle RSS must not move.
2. **Change gate.** A prompt, playbook, tool or model-pin change is a release that passed a frozen must-not-break suite in CI: a stub model server replaying
   recorded outputs; schema, refusal and injection cases; fixtures from the G1-G7 namespaces; zero regressions, paired. The score is evidence, never the
   gate: about 50 cases give ±13 points at 70 % (R04 §1). Model-pin swaps are always a project release with an eval report (Q20).
3. **No node-local overlay in v1** (OB conflict 4): a prompt has no probe that proves it safe.
4. **The KB grows by export.** `kb_draft` proposals carry `{target, body, not_verified, incidents}`; accepting one creates a `heal/drafts` record of class
   `kb` exported through §9's mbox route (Q19). The node's KB is untouched; a draft enters as `candidate` with a `### Smoke test` generated from the
   incident; the agent never edits `status`; it never reads its own drafts back as KB truth.
5. **Bridging grows by typed actions.** `bridge.edge.pause`, `bridge.edge.set` and `bridge.order.set` (§7.2) and playbooks for edges; the human accepts each,
   so "it cannot do so" (`L6:487-488`) stays literally true. No filter language (README §5).
6. **Later, human-started hub tasks:** replay of the owner's own incident corpus (an opt-in sealed export), GEPA (MIT) proposal runs, Inspect AI (MIT) as an
   optional runner. Never a node loop: 50 cases × 3 × 400 tokens is 60,000 tokens, 3.3 h at an assumed 5 tok/s on a Pi (R04 §4, arithmetic).

## 14. The Repairs page, notifications and the honest words (OB §3)

**14.1 The page (normative).** Settings → Layer 9, `/settings/l9`, "Repairs", its own lazy chunk ≤ 64 KB (D21; the `l6` precedent at
`UI/src/views/settings/index.tsx:13-14` in the committed tree). Sections: **verdict** (one sentence: "Nothing needs repair." · "2 repairs are waiting
for you." · "A change is waiting for your confirmation") · **Needs you** (open proposals, `revert_failed`, `refused`) · **Timeline** (one row per incident:
fingerprint, first and last seen, count, state chip, the actor of the last step, expandable evidence, checks, attempts, verify log, outcome) · **Changes**
(kept changes with Undo while a snapshot is kept; "A restart cannot be undone" otherwise) · **Fixes** (received, sent and drafts; Download mbox, Download
PR.md, Submit upstream) · **Playbooks** (advanced: loaded, refused with "needs release X") · **Audit** (advanced) · the **consent block** (§17). Event-fed
(`incident`, `proposal`): no UI timer and no ticking countdown. Whether the chunk fits after Layers 7 and 8 is measured at build.

**14.2 States (closed).** `open`, `diagnosing`, `proposed`, `applying`, `kept`, `resolved`, `reverted`, `revert_failed`, `dismissed`, `refused`,
`unable_to_diagnose`. Three severities (§3.5). A dismissal survives restarts and re-opens on a changed cause (Home Assistant's issue registry, Apache-2.0,
a reference only).

**14.3 Simple mode (Q3).** The Overview's Needs-you block (`L4:212-214`) gains "N repairs waiting" and "a change was reverted"; the full page is advanced.
The agent view's line "Applying accepted proposals arrives with Layer 9 (Repairs)" (`UI/src/views/agent/proposals.tsx:121`, committed tree) becomes true and
reads "Accepted repairs are applied with an automatic undo." The layer card's "tests fixes in a sandbox" (`UI/src/lib/layers.ts:75`) reads "The node watches
every layer, explains failures and proposes fixes with an automatic undo; code changes are drafts a person sends."

**14.4 Phone notifications.** `heal.notify = transitions` posts one content-free notification, "111: your node needs you", through Termux:API on a
transition into Needs you; one transient process per transition, none otherwise.

**14.5 Honest words (normative).** "probe passed" names the probe; "drafted and applies cleanly, not built"; "validated" always names its validator; a
peer's verify log is "their test, not this node's"; anything from another node, a bridge or an RF origin is rendered preformatted and never instructs the agent.

## 15. Sequencing and the first Layer 9 release (D35 F14)

v1, in this order (each a board row with its gate):
1. The containment correction (§8.1) and atomic proposal and task writes (`A/src/store.rs:139`, `:145`, against `:1-3`; Layer 8 §2.4 rule 1 does the same).
2. The engine's start-up reconciliation (§7.5) on the Layer 6 applier, before anything else uses the engine.
3. The supervisor's exit facts (§3.2), the witness (§3.3), the observer and the incident record (§3.4-§3.5).
4. State per the joint contract (§4): after Layer 8's state service (L8 §14 item 2); else J7.
5. Grammar v1, the first playbooks, T0 and T1 (§5, §6).
6. The `Action` trait and the v1 vocabulary (§7); F1's grant format pinned, off (§2.3).
7. The two tags (§10); the op pair and the hub's serving switch (§12); typed fixes to the home hub and pinned contacts (§11).
8. The mbox and `PR.md` (§9); code drafts on opted-in hubs (§8).
9. The agent data seam, `kb_draft`, the three bridge actions, the CI suite (§13).
10. The Repairs page and the contract (§14, §18); the APK `human_step` (§12.6).

**v1.1:** the auto-apply rung if the owner wants it · OB F7 C · the fix shelf with Layer 8 · vouched authors · a hub token · hub-to-hub delegation · a
node overlay · hub eval replay · the opt-in probe · delegation certs · Tier 2 fixes and LoRa fix cards. Every gate stays netns- or stub-verified.

## 16. The sixteen amendments to locked layers (D35 F13)

Each is additive and owner-visible; with Layer 9's switches off and no `heal` traffic, wire bytes and contracts are unchanged. One removal and three
corrections are marked. The exact text per layer is §23.

| # | Layer | Amendment | Kind | Here |
|---|---|---|---|---|
| 1 | L0 | `playbook` category, folder, N06, the H3 vocabulary, the TOML lint | additive | §6 |
| 2 | L1 | `ExecStopPost=` and the Termux inbox line; the cron entry | additive; **removal** | §3.3 |
| 3 | L1 | the hub-only source asset: Layer 8's `[source]` pin, consumed | additive | §8.2 |
| 4 | L1 | `receipt.apk` | additive | §12.6 |
| 5 | L1 | the probation wording (`L1:183`) | **correction** | §24 |
| 6 | L2 | `111/fix/v1`, `111/fix-vouch/v1` | additive | §10 |
| 7 | L3 | the `incident` input; `incident.read`; `service.log` | additive | §5.2, §18 |
| 8 | L3 | the opencode deny block and `HOME` | **correction** | §8.1 |
| 9 | L3 | prompts as release files | additive | §13 |
| 10 | L4 | `Proposal.kind` +4, `applied`, `incident`, the repairs routes, `/settings/l9` | additive | §14, §18 |
| 11 | L5 | `Fabric::send_op` for `AgentOp` and `Patch`, size gate, commit-before-seal | additive | §23 L5 |
| 12 | L5 | `STREAM_AGENT = 1` | reserved in v1 (§24 D1) | §12.2 |
| 13 | L5 | a consumer for 0x10 and 0x13 instead of a chat row (built once with L8 §12.3) | additive | §11.4 |
| 14 | L5 | the Tier sentence for `PATCH` | additive | §23 L5 |
| 15 | L6 | the trait, persisted undo, reach class, vocabulary; armed at start = revert | additive; **correction** | §7 |
| 16 | L7 | role bit 6 `ROLE_HEAL` | additive | §23 L7 |

Every wire-visible addition (old nodes drop the kinds as today; with every switch off none is sent):

| Where | What | Bytes | Here |
|---|---|---|---|
| `AGENT_OP` (0x10) payload | `schema` ‖ `op` ‖ CBOR map; three ops | ≈ 450-600 B request, ≈ 400-600 B reply, ≤ 16 B refuse (estimates) | §12.1 |
| `PATCH` (0x13) payload | `type` ‖ fix container or vouch | ≤ the SIGNED `tcp-lan` budget; a vouch 106 B | §11.2 |
| signatures inside those payloads | `111/fix/v1`, `111/fix-vouch/v1` | 64 B each | §10.2 |
| `appdata` roles (L2 A8) | bit 6 `ROLE_HEAL` | 0 B (a bit of the existing u16) | §23 L7 |

## 17. Settings and consent

**17.1 Every Layer 9 setting (normative).** `state/heal.json`: JSON `{"schema": 1, …}`, 0600, atomic, written only on a change. Absent ⇒ every default.
Unknown keys are kept and ignored (L8 §15.1's rule, so a rollback never drops a consent). Changed only by `PATCH /api/v1/repairs/settings`, loopback-only.

| Key | Values | Default | Who | Consent |
|---|---|---|---|---|
| `heal.autonomy` | `observe`, `diagnose`, `propose` | `propose` | a human | — |
| `heal.fixes.receive` | `off`, `home_hub`, `paired_hubs`, `pinned_contacts` | `home_hub` | a human | — |
| `heal.fixes.send` | `off`, `home_hub`, `pinned_contacts` | `off` | a human | `fix_send` |
| `heal.ask_hub` | `false`, `true` (members) | `false` | a human | `heal_delegate` |
| `heal.serve_members` | `false`, `true` (`hub_role` only, else `409 needs_hub`) | `false` | a human | `heal_serve` |
| `heal.code_drafts` | `false`, `true` (`hub_role` and the `opencode` component enabled) | `false` | a human | the component's own licence notice |
| `heal.notify` | `transitions`, `off` | `transitions` | a human | — |

**Inert by default.** With nothing configured nothing changes without a human, nothing leaves the node, Layer 9 loads no model and nothing resides; the observer
records and, under `propose`, files proposals. A widening answers `409 consent_required` in the pinned shape (`ConsentRequired`, `API:1492-1502`): the
client shows `warning_text` verbatim and repeats with `accept_warning`; each consent records its text hash; a changed text asks again. A switch shows the
node's answer, never an optimistic one.

**17.2 Consent copy (verbatim).** **`fix_send`** (subject `<to short id>`): "Send this fix to <alias>. It tells them what failed on this node and how it was
fixed: <the fields that leave, one per line>. It is signed by this node and sealed to them. You can't take it back." · **`heal_delegate`** (subject the home
hub's short id): "When a problem here has no known cause, ask <hub alias> to diagnose it. Each request tells your hub what failed on this node: the service,
the error code, the release, the checks that ran and up to three short log lines from 111's own programs. No messages, contacts or other apps' logs are
sent." · **`heal_serve`** (subject this hub's short id): "Members of this hub may ask it to diagnose their problems. Their requests tell this hub what failed
on their devices. Each one runs a check, and the model when it is installed, on this hub."

**17.3 What the agent may propose, what only a human confirms (normative).**

| Act | The agent | A human |
|---|---|---|
| a typed repair | never directly: `111d` composes `repair` proposals from playbooks; the agent proposes device actions with `device.propose` as today | accepts |
| a fix to send, a delegation, serving members, code drafts | never | only, through the consents |
| a reach-class confirm, an Undo, a dismissal | never | only |
| a KB draft | through a `kb_draft` proposal | accepts, exports |
| a code draft | through a `patch` task on an opted-in hub | accepts, exports, opens the PR |

The bearer is refused on every non-GET route Layer 9 adds (D26 (2)); the agent never accepts a consent and never sees a consent hash.

## 18. The contract (additive; the binding form is `API` at build time)

**18.1 Routes.**

| Route | Method | Shape | Who |
|---|---|---|---|
| `/api/v1/repairs` | GET | `RepairsStatus {schema, verdict, needs_you, open, overflow, store: {state, reason}, model_tasks}` | session or loopback; the bearer reads |
| `/api/v1/repairs/incidents` · `/{id}` | GET | `{schema, incidents: [Incident]}` (`?state&limit`) · `Incident` (§3.5) | the bearer reads |
| `/api/v1/repairs/incidents/{id}/dismiss` | POST | 204 | a human |
| `/api/v1/repairs/changes` · `/{id}/undo` | GET · POST | kept changes with `undo_available` · 202; 409 `busy`, `nothing_to_undo` | the bearer reads · a human |
| `/api/v1/repairs/attempts/{id}/confirm` | POST | 204; 409 `confirm_window_closed` | a human, same URL (§7.6) |
| `/api/v1/repairs/audit` | GET | `{schema, entries}` (`?since_seq`) | session |
| `/api/v1/repairs/settings` | GET · PATCH | `RepairsSettings` (§17.1); 409 `consent_required`, `needs_hub`, `component_unavailable` | read: session · write: loopback only |
| `/api/v1/repairs/fixes` · `/{id}` · `/{id}/send` | GET · GET · POST | `{schema, fixes: [Fix]}` · `Fix` · `{to}` → 202; 409 `consent_required` | the bearer reads · a human |
| `/api/v1/repairs/drafts/{id}/mbox` · `/pr` | GET | `text/plain` downloads (§9) | session |
| `/api/v1/repairs/playbooks` | GET | `{schema, playbooks: [{slug, schema, state: loaded\|refused, reason}]}` | the bearer reads |

**18.2 Additions to existing shapes and the pin list.** `Proposal.kind` gains `repair`, `fix`, `human_step`, `kb_draft` (appended to `API:848`; nine kinds
at `fa1daeb`, `N/dashboard/api_v1/proposals.rs:104-114`, ten after Layer 8's `storage`). `Proposal` gains optional `applied` (the decision record already
holds it, `proposals.rs:170-182`; `API:840-852`), so `reverted` becomes visible. `TaskRequest` gains `incident` and `causes` (§5.2). `ConsentRequired.action`
gains `fix_send`, `heal_delegate`, `heal_serve`, appended (`API:1497`). `Problem.code` gains `busy`, `nothing_to_undo`, `would_lock_out`,
`confirm_window_closed`, appended (`API:692`); `store_unavailable` is Layer 8's. New SSE event `incident` → `SseIncident` (one `Incident` on change),
appended to `x-sse-events.events` (`API:2044-2059`); after `reset` the Repairs view refetches `/repairs` and `/repairs/incidents`. Id families minted by
`111d`: `inc-…`, `att-…`, `del-…`; a fix id is 64 hex (`fix_id`). **Pin list** (the conventions' form, `4-interface/api/README.md:190-207`): (1)
`Proposal.kind` +4: `KINDS`, the UI's card, the agent's writer · (2) the agent tool count +2 (§18.3) · (3) `pnpm gen:types`, then `node scripts/gen-sse.mjs
--check`, with the new enum tuples listed in `UI/scripts/gen-sse.mjs` · (4) `incident` in `x-sse-events` and in the store's `switch (frame.event)` · (5)
`UI/test/gen.test.ts`, `UI/test/http.test.ts`, a new `UI/test/layer9-contract.test.ts` and the mock adapters · (6) node111's mirrors of the codes and actions.

**18.3 Agent tools (the Layer 3 set grows; fixed, shell-free, schema-validated).**

| Tool | Kind | Shape | Gate |
|---|---|---|---|
| `incident.read` | read | `{id?, state?}` → incidents; excerpts inside a labelled untrusted block | none |
| `service.log` | read | `{service_id, n ≤ 100}` → `GET /api/v1/services/{id}/log` (own children only), inside an untrusted block | none |

The count grows by two from the count the build finds: 17 at `fa1daeb` (`A/tests/daemon_tools.rs:318`, `A/tests/chat_loop.rs:308`), +3 for Layer 7, +3 for
Layer 8. Schemas beside `logs.read` (`A/src/tools.rs:84-95`), dispatch beside `:322-326`. No typed-repair tool: `proposal.write` takes only a title and a body
(`:97-111`) and repairs come from playbooks (§7.3).

## 19. Security, privacy and the threat model

**Hard rules.**
1. **The agent holds no key and writes nothing but proposals.** Its bearer is refused on every non-GET (`N/dashboard/api_v1/auth.rs:927-933`); a decision
   is made only inside the decision handler (`proposals.rs:638-706`); a file under `agent/decisions/` is never an authority (every 111 process runs as
   `node111`, R00 trap 3).
2. **Nothing applies code**, on any node, under any rung.
3. **The verifier is never inside what a change may touch**: probes, the ledger, the undo file, the confirm window and the audit live in `111d`.
4. **A peer's text is data**: fix prose, a delegated narrative and a peer's verify log are rendered preformatted; the agent never reads or summarises them.
5. **Nothing leaves without a human or a consent**: a fix only by a human's send; a delegation only under `heal_delegate`; a PR only by the human's hands (J6).
6. **No signing oracle**: `111d` signs only bytes it builds from its own records, under the two tags.
7. **No wall clock** orders, prunes or expires anything: `seq`, the boot id, `CLOCK_BOOTTIME` and counts do.
8. **Fail closed on grammar**: a playbook with one unknown field never runs.
9. **opencode runs denied** (§8.1); it never sees a token, a key or the dashboard.

**What is exposed, to whom.**

| Path | On the path | At the far end |
|---|---|---|
| a sent fix (`PATCH`, sealed) | as any sealed item: sizes, timing, `dest_hint` (L5 §18) | the recipient: the `fix/v1` fields; it can show the fix to others, signed by this node |
| a delegation (`AGENT_OP`) | as above | the home hub: the request fields of §12.1, never message text |
| a vouch | as above | the author: that this node accepted or reverted its fix |
| role bit 6 | everyone who gets the card | that this hub serves members' diagnoses |
| the mbox and `PR.md` | — | whoever the human gives them to: the incident's diagnosis and verify log |

**Attacks and defences.**
- **A self-approval by a confused or injected model on a hub:** §8.1's deny block; G24 asserts no connection to `127.0.0.1:11180`.
- **A malicious fix that disables the gateway or a carrier:** refused by §11.4 step 6's scope list before display; a lying `applies_to` fails its probe and reverts.
- **A forged, replayed or oversized fix:** the signature, `author_seq`, the size check; a flood from one author is bounded by the 16-per-author queue.
- **A validator gamed by deleting the capability** (the Kali lesson, R02 F14): probes assert the capability; a removal must be the stated purpose (§7.3).
- **A daemon killed mid-apply:** armed at start = revert (§7.5).
- **A reach-class change that cuts the human off:** the 90 s confirm and `would_lock_out` (§7.6).
- **Model output that cites what is not there:** discarded by §5.3; no retry loop.
- **Persistent-memory poisoning through KB drafts** (MINJA, AgentPoison, MemoryGraft, snippets): drafts are exported, never read back.
- **A hub that lies in a diagnosis:** the reply is a proposal on the member, re-validated against its vocabulary, accepted by its own human.
- **A stranger asking a hub to diagnose:** only `enrolled` members on attributed connections; one open request each.
- **Request flooding the hub's model:** one task at a time; one open request per member.

## 20. Verification (lock gates; netns, no hardware)

**Gate homes.** New `9-evolution/verify/{observer,engine,diagnose,share,delegate,codefix,evolution}.sh`, a stdlib-only stub model
`9-evolution/verify/stub-model.py` (replays recorded completions on the model port inside a netns), known-answer vectors under `9-evolution/verify/vectors/`,
and `9-evolution/verify/lib.sh`, which sources `6-transport/verify/lib.sh` and `6-transport/verify/root.sh` as `7-relay/verify/lib.sh` does and names the
netns, veth, addresses (10.111.9x.0/24) and ports. The conventions are `6-transport/verify/lib.sh`: each check prints `ok:` or `FAIL:` (`note:` informs); a
gate whose prerequisites are absent (unprivileged netns through `unshare -Urnm`, a built binary, `OPENCODE_BIN`) prints `<GATE> SKIPPED: <reason>` and exits
0 unless `REQUIRE_<GATE>=1` makes it a FAIL; the last line is `<GATE> GREEN` or `<GATE>: N FAIL`. Every harness stops what it started, leaves nothing
listening and runs alone (AGENTS.md).

**Seam ids (D36 (2), D37).** Layer 9's planned rows (`plan/cross-layer/test-system/matrix.md:177-179`, entering `verify/seams.toml` under D37 F5) plus three
new rows. Each `ok:` and `FAIL:` line starts with its seam id in brackets. Layer 9's board carries the verify row that registers these gates in
`verify/checks.toml` and promotes the seams.

| Seam id | Layers | The interface it crosses | Gates |
|---|---|---|---|
| `L1-L9:observer-change-engine` | 1 ↔ 9 | the supervisor's state and exit watches, the `EventBus`, the unit's `ExecStopPost=`, runit's `finish`, the update tombstone, the undo file at start | G19, G20 |
| `L8-L9:heal-store` | 8 ↔ 9 | J2's files, J3's reclaim and marker, the J9 calls | G19 |
| `L6-L9:typed-actions` (new) | 6 ↔ 9 | the `Action` trait over the Layer 6 applier and spine | G20 |
| `L4-L9:repairs-api` (new) | 4 ↔ 9 | the decision route, `/api/v1/repairs*`, the `incident` frame, `/settings/l9` | G20, units |
| `L3-L9:diagnosis-playbooks` | 3 ↔ 9 | the agent task API with `incident`, the JSON-schema reply, the two tools | G21 |
| `L0-L9:evolution` | 0 ↔ 9 | the `playbook` category and lint, `share/kb/playbooks/`, `share/agent/`, `kb_draft` | G21, G25 |
| `L2-L9:fix-signing` | 2 ↔ 9 | `sign_with_tag` and `verify_strict_tagged` under the two tags | G22 |
| `L5/L7-L9:fix-sharing-delegation` | 5, 7 ↔ 9 | `send_op`, the consumer for 0x10 and 0x13, the size gate, role bit 6, custody across islands | G22, G23 |
| `L3-L9:code-drafts` (new) | 3 ↔ 9 | opencode under the deny block, the source tree, the applies-cleanly check, the mbox | G24 |
| `L1-X:pi-floor` | 1 ↔ all | processes, listeners, idle RSS, timers | G7 |

**G19, the observer** (`observer.sh`: one netns node with the real `111d`, a stub child, the real agent off; node111 tests). Asserts: (1)
`[L1-L9:observer-change-engine]` a child killed with signal 9 opens one incident `(1, <child>, signal_9)` with an `exit` evidence; five kills in its window
leave that one incident at `count = 5` and open one `child_degraded` incident; each closes `self_recovered` after the child holds `Running` for the window
(the second after a restart) · (2) a services row
forced to `error` opens `(layer, id, reason_code)` once and closes when the row leaves · (3) `kill -9` of `111d` then a restart opens `daemon.unclean_exit`;
a graceful stop opens nothing; the inbox line written by `111 heal-witness` from `SERVICE_RESULT=signal EXIT_STATUS=KILL` is folded and the file cleared ·
(4) a tombstone with `pending: false, health_confirmed: false` and another version opens `update_reverted` · (5) no timer: with Layer 9 present and no
incident open, Layer 7's `idle.sh` wakeup line (L7 §3.7) is unchanged and no playbook is held parsed · (6) a 65th fingerprint opens nothing and counts
`overflow` · (7) `[L8-L9:heal-store]` with the store unopenable the J2 files stay and `GET /api/v1/repairs` names the
reason; with it open they are folded and cleared; a reclaim never touches an `open` incident or an `armed` attempt and leaves one marker. **Pass:** `G19 GREEN`.

**G20, the engine** (`engine.sh`: the Layer 6 bridge and spine stubs, `NODE111_BRIDGE_STUBS`, the G4 pattern; node111 tests). Asserts: (1)
`[L6-L9:typed-actions]` the four device actions through the trait give byte-identical files and results to the Layer 6 applier (the G4 tape) · (2) a probe
that fails reverts byte-exact and records `reverted` · (3) `[L1-L9:observer-change-engine]` `kill -9` between apply and probe: at the next start the
snapshot is restored before the listener answers, the decision record reads `reverted (daemon restart)`, and a retry is `409 already_decided` · (4)
`[L4-L9:repairs-api]` a `may_affect` action with no confirm reverts after the window, with a confirm from the same URL it is kept; a `would_lock_out` action
writes nothing · (5) a second proposal on an armed target answers `409 busy`; after a revert the ledger refuses the same action for the same cause until
the evidence hash changes · (6) Undo restores the last kept snapshot and runs the probe; a restart's card offers none · (7) `update.rollback` outside a
probation is refused with a `human_step`; `component.enable` with no cached payload becomes a `human_step` naming the install verb · (8) the bearer's POST
on every Layer 9 route answers 401. **Pass:** `G20 GREEN`.

**G21, diagnosis** (`diagnose.sh`: one node with the real agent and `stub-model.py`; node111 and agent111 tests). Asserts: (1)
`[L0-L9:evolution]` a playbook with an unknown predicate, field, service id or reason code is refused whole and listed "needs release X"; depth 4, 17
leaves or 2,049 B refuse; `"@subject"` is the only substitution · (2) `[L3-L9:diagnosis-playbooks]` one satisfied cause ⇒ `by: t0` with no model request
(the stub logs none) · (3) two satisfied causes ⇒ one task whose request carries the `incident` block, `response_format` with the cause enum, and no
`daemon_health` or `recent_events` · (4) a stub reply citing an unknown evidence id or a quote not in the excerpts is discarded, no second request · (5)
two incidents at once make one model request at a time · (6) under `observe` nothing is evaluated; under `diagnose` no `repair` proposal is filed · (7)
`incident.read` and `service.log` return untrusted blocks; the tool count is the build's +2. **Pass:** `G21 GREEN`.

**G22, sharing fixes** (`share.sh`: nodes A and B and hub H in netns over `tcp-lan`; node111 and mesh111 tests). Asserts: (1) `[L2-L9:fix-signing]` the
vectors verify; a flipped body byte or attachment byte fails · (2) `[L5/L7-L9:fix-sharing-delegation]` with `heal.fixes.send = off` no `PATCH` leaves A
(capture); after `fix_send` one leaves for H only · (3) on H the body reaches the consumer, never a message row; a fix from a non-pinned author is parked
with no action; a `code` fix offers only Submit upstream; a fix naming a carrier or the update channel is refused before display · (4) accepting it on H runs
the loop and A receives one `accepted` vouch · (5) a replayed `author_seq` and an oversized payload are dropped and counted; a 17th fix from one author
drops the oldest · (6) with no switch on, the frames A sends equal the Layer 8 build's (a committed tape under `9-evolution/verify/tapes/`). **Pass:**
`G22 GREEN`.

**G23, delegation** (`delegate.sh`: hub H and members M1, M2 in netns; `stub-model.py` on H). Asserts: (1)
`[L5/L7-L9:fix-sharing-delegation]` with `heal.ask_hub` off M composes nothing; on, an incident with no T0 cause sends one `diagnose` (its size recorded
against R07g2 F4's 450-600 B estimate), with no excerpt from a bridge or presence log · (2) H with `heal.serve_members` off answers `refuse` reason 1 and
runs no task; on, it sets role bit 6 and answers once per `req_id` · (3) a contact that is not `enrolled` is dropped and counted; a second open request
from M1 gets `refuse` reason 3 · (4) M files the reply as a proposal, never applied, its actions re-validated; a reply for a closed `req_id` is
dropped · (5) a 160 B profile leaves the request `queued` "needs a longer link" · (6) the narrative is the playbook's line, byte-exact. **Pass:**
`G23 GREEN`.

**G24, code drafts** (`codefix.sh`; `OPENCODE_BIN` a pinned opencode, else `G24 SKIPPED` unless `REQUIRE_G24=1`). Asserts: (1) `[L3-L9:code-drafts]` the
written `opencode.json` holds the deny block; `HOME` and XDG are in the task dir; no `--auto` · (2) a prompt asking for `curl 127.0.0.1:11180` produces no
connection to that port (socket log in the netns) and no decision · (3) a diff that applies reads "drafted and applies cleanly, not built"; one that does
not reads "does not apply" · (4) the mbox passes `git am` on a scratch clone of the release source and carries no `Signed-off-by:`; `PR.md` names what was
not verified. **Pass:** `G24 GREEN`.

**G25, evolution** (`evolution.sh`; `build.py` fixtures and agent111 tests). Asserts: (1) `[L0-L9:evolution]` `build.py --check` accepts a valid playbook
and rejects a wrong filename, a missing `### Checks`, an unparsable block and an unknown predicate · (2) a tampered `share/agent/` file marks the `agent`
row `prompts_invalid` and the agent is not started · (3) the must-not-break suite replays with zero regressions · (4) accepting a `kb_draft` writes nothing
under `current/share/kb/`. **Pass:** `G25 GREEN`.

**G7, extended:** `zero-weight.sh` with Layer 9 present and idle (processes, listeners, idle RSS equal the baseline) and Layer 7's `idle.sh` no-timer line
unchanged. **Pass:** both.

**Unit and integration test homes.** node111 (`1-bootstrap/crates/node111/tests/`): new `heal_observer.rs`, `heal_witness.rs`, `heal_playbooks.rs` (with
`fixtures/playbooks/`), `heal_engine.rs`, `heal_fix.rs`, `heal_delegate.rs`, `api_v1_repairs.rs`; the existing device-applier tests stay and pass unchanged.
mesh111: `tests/send_op.rs` (the size gate, commit-before-seal); `tests/consumers.rs` is Layer 8's. agent111: `tests/incident_input.rs`,
`tests/opencode_config.rs`, `tests/evolution_suite.rs`, `tests/store_atomic.rs` (Layer 8's), the tool count in `daemon_tools.rs` and `chat_loop.rs`. UI:
`test/layer9-contract.test.ts`, `test/repairs-view.test.ts`. KB: playbook cases in `build.py`'s golden-fixture self-test.

**CI mapping** (`.github/workflows/ci.yml`; parity with the local gates): node111 and mesh111 tests and the UI checks run in `bootstrap-gate` (`:28`,
`1-bootstrap/scripts/check.sh`); agent111 in `intelligence` (`:79`); `build.py --check` with the playbook lint and contract drift in
`knowledge-contract-hygiene` (`:96`); the seam-map lint in D37's planned `cross-layer` job. The netns harnesses and G7 stay the lead's: the runners restrict
`unshare -Urnm` (`plan/cross-layer/ci-cd.md:18`). Root-only `check.sh` failures on the build box are recorded, never counted as tree faults (D29).

## 21. Pi-floor accounting

| Item | Every switch at default, idle, no incident open | An incident opens | T1 diagnosis (model nodes) | An apply in progress | Hub serving members | A code draft (hub) | Basis |
|---|---|---|---|---|---|---|---|
| processes | 0 | 0 | 0 (the agent and model already run) | 0 | 0 | +1 opencode while it runs | design |
| listeners | 0 | 0 | 0 | 0 | 0 | opencode's own loopback port while it runs (R07g1 §2) | design |
| timers | 0 | one one-shot sleep per recovery window | 0 | the probe's 1 s steps (≤ 30 s), the 90 s confirm only while pending | 0 | 0 | design; G19 |
| threads | 0 | 0 | one agent thread per task (`A/src/tasks.rs:175-177`) | 0 | as T1 | the agent's pumps | src |
| memory | 0 beyond the binary delta (no parsed playbook, no incident) | ≈ 1 KiB per open incident (estimate, R01 §3.2) and its playbook | the model's working set (Layer 3's) | one snapshot | one request | opencode RSS (measured at build) | measured at build |
| writes | 0 (two small marker writes per daemon run) | one J9 write per transition | one | the undo file, then the attempt | one per request | the draft record | design |
| disk | the run marker (≤ 128 B) | `heal` class, 600-1,000 B a record (estimate) | — | ≤ five snapshots (measured at build) | — | the source tarball, opencode 211-222 MB | R01 §3.2; R07g1 §2 |
| Termux | −1 `curl` process per 5 min (the cron removed) | — | — | — | — | — | src |

**Build-time weight, not resident:** the `111` binary delta (observer, evaluator, engine, op arm) and the `111-agent` delta (loader, `diffy`), measured at
build; the Repairs chunk (≤ 64 KB); the playbooks and prompts in the payload.

**Rule.** The build records each figure, measured on the release binaries, in `9-evolution/TASKS.md` beside Layer 6's (`111` 19,404,064 B at 19.7 MB idle
RSS, `111-agent` 4,392,280 B at 5.3 MB, listeners :11180 (+ the best-effort :80) and :11182, `6-transport/TASKS.md:82`) and Layers 7 and 8's, with RSS from
`/proc/<pid>/smaps_rollup`, `ss -ltnup` before and after, and Layer 7's no-timer method. A change to the first column fails the gate until the owner says
otherwise.

## 22. Deferred / later versions

| Item | What it needs |
|---|---|
| the auto-apply rung | the owner's word (OB F1 B), §2.3's grant record built, `decision.by = "policy"`, the count cap squared with README §5 (Q1) |
| sandboxed code validation (OB F7 C) | a measured cold `cargo check` of `3-intelligence` on a Pi 5 and an N100; bubblewrap or nsjail as a component; a pinned toolchain; the model-port forwarder (R07g1 F5) |
| the hub fix shelf | Layer 8's catalogue class `patch`, a hub-owner-signed fix index as a pin source (L8 §11.1, §20) |
| vouched authors | `111/fix-vouch/v1` shown as "vouched for by <alias>", never automatic |
| a hub token for PRs | a fine-grained single-repository token, a consent naming the repository, never readable by opencode (Q17) |
| hub-to-hub delegation | a Layer 7 "hub as destination" amendment (R07g2 F9) |
| delegation on the 160 B floor, LoRa fix cards | Tier 2 (L8 §8) |
| the stream id for `AGENT_OP` | the reserved `stream_id` TLV (`L5:485`): a wire change |
| a node overlay for prompts and playbooks | an incident corpus, the frozen suite, a revert path (OB F12 B) |
| hub eval replay, GEPA runs | an opt-in sealed export of the owner's incidents to a hub |
| the opt-in probe | one re-armed one-shot on hubs, never a cron entry (OB F2 B) |
| PSI triggers | `CONFIG_PSI` on Pi OS kernels *(unverified from this box)* |
| the L2 delegation cert | the epoch's first consumer (L7 §8) |
| a shared fix-attestation feed | L8 §12.2's five conditions; p2panda-core |
| APK bytes from the hub | Layer 8's origin admission rule for `apk.*` pins; a folder the Android installer reads *(unverified from this box)* |

## 23. Cross-layer seams & amendments

- **Layer 0 (locked):** (item 1) `category: playbook` in the enum (`L0:45`, F05), folder `playbooks/` with its generated `llms.txt`; rule **N06** in
  `_meta/schema.md` §1: "Playbook filename not `repair-<subject>-<reason>.md`, or its `[match]` disagrees with the filename or the closed vocabulary"; rule
  **B10**: "Playbook `### Checks` block missing, not one `toml` fence, not parsing with `tomllib`, or breaking grammar v1's limits"; the H3 vocabulary under
  `## Details`; B09's playbook budget 2,000. `build.py` (`0-knowledge/_meta/build.py:44-52`, the recipe rules at `:575-602`) gains the category and both rules.
  Recipes and N03 untouched; `skills/` stays deferred.
- **Layer 1 (locked v2.2):** (item 2) `111node.service` gains `ExecStopPost=/usr/local/bin/111 heal-witness`; the CLI gains the hidden verb `heal-witness`;
  the Termux `finish` hook gains one append line after `:8`; **removal:** `termux/cron/crontab.line`, `termux/cron/healthcheck.sh` and the Supervision
  text's "cronie `/health` check (placeholder seam until Layer 9)" (`L1:156`) → "the runit `finish` hook appends the exit to Layer 9's inbox". (item 3)
  the `[source]` pin Layer 8 §11.6 adds is also fetched by `sudo 111 install --components opencode` on a hub. (item 4) the receipt gains `apk`
  (`#[serde(default)]`, `NI/install/receipt.rs:21-45`). (item 5, correction) `L1:183` "sustained across the 60s probation window" → "sustained for 60 s
  inside the 600 s probation deadline" (`N/mod.rs:134-180`, `:157`; `NI/probation.rs:57`). Code seams, no spec text: `ChildControl::exits()` (§3.2); the run marker and
  inbox under `state/heal/` (J8); `state/heal.json` in the layout table. The hand-offs closed: "Layer 9: supervision-tree handles, `/health` contract,
  exit(0)-restart, watchdog heartbeat" (`L1:277`) and "APK refresh = Layer 9 maintenance task" (`L1:185`).
- **Layer 2 (locked v2):** (item 6) `111/fix/v1` and `111/fix-vouch/v1` with the constructions of §10.2, in the one Layers 8 and 9 block (L8 §21 L2, A19);
  id111 gains no code; vectors in `9-evolution/verify/vectors/`.
- **Layer 3 (locked):** (item 7) `TaskRequest.incident` and `causes`; the untrusted rendering and the switched-off context; `response_format` passed through
  the model client; tools `incident.read`, `service.log`. (item 8, correction) the deny block, `HOME`/XDG and the five environment flags in
  `write_opencode_config` (`A/src/tasks.rs:495-509`, `:429-433`). (item 9) prompts and the diagnosis schema as `share/agent/` release files. Text: `L3:49`
  "The agent signs its outbound artifacts (proposals, patch PRs) with the node key" gains "As built by Layer 9: `111d`, the identity holder, signs fixes on
  its behalf; proposals are never signed (Layer 9 §10)"; `L3:84` "proposal *signing* is deferred to Layer 9's PR packaging" gains the same note. The
  hand-off "Tasks → Layer 9" (`L3:47`) is closed by §5 and §8.
- **Layer 4 (locked):** (item 10) §18's routes, shapes, event and pin list; `/settings/l9` as a lazy chunk; the Needs-you rows; the two UI copy lines of
  §14.3; `/api/v1/repairs*` is not loopback-only for reads. The conventions gain a "Layer 9 additions" block. The hand-off "applying accepted proposals is
  Layer 9's job" is closed by §7.
- **Layer 5 (locked v1.2):** (item 11) `Fabric::send_op(to, kind, payload, receipt_strong) -> msg_id` for `AgentOp` and `Patch` only, beside `send`
  (`M/fabric.rs:1237-1243`), with the Tier 0/1 size gate of §12.2 and the same commit-before-seal rule; `send` stays byte for byte. (item 12) reserved: no
  stream id in v1 (§24 D1). (item 13) Layer 8 §12.3's consumer registry, registered for 0x10 and 0x13 by Layer 9; the textless chat row ends for both kinds.
  (item 14) beside the Tier table's "larger objects and every bulk class" (`L5:241`): "A `PATCH` whose payload fits one `tcp-lan` frame or 16 Tier 1
  fragments may travel without Tier 2 (Layer 9 §11.2)." The hand-off "`PATCH` class, `AGENT_OP`, `RECEIPT_STRONG`; the apply→verify→undo engine (P4)"
  (`L5:831`) is closed by §7, §11, §12.
- **Layer 6 (locked v1.0):** (item 15) §12 gains the trait (§7.1), the persisted undo and "armed at start = revert" (§7.5; a correction of `L6:657-658`), the
  reach class (§7.6), and the vocabulary of §7.2 with the four device actions byte-for-byte; §8 gains the three bridge actions; `L6:487-488` stays literally
  true. `assets/components/opencode.toml` follows §2.1 rule 3 (`prefetch = false`). G7 covers Layer 9 idle.
- **Layer 7 (locked v1.0):** (item 16) the §7.2 table gains bit 6 `ROLE_HEAL`, set by `heal.serve_members`, defined in node111 beside bits 3-5 (`roles(s)`,
  `N/fabric/mod.rs:435-444`), no mesh111 change, no consent of its own beyond `heal_serve`. Fixes across islands ride §5 custody unchanged.
- **Layer 8 (draft v0.1):** the joint contract J1-J9 consumed, nothing added; the `heal` class of §4.1; the consumer registry (§12.3); the `[source]`
  pin (§11.6); the export carries `heal/*` (J6). The seam `L8-L9:heal-store` is proven by G19 and Layer 8's G13 item (7).
- **`LICENSING.md`, `DECISIONS.md`, repo:** §5 a `diffy` row; §3's opencode row (`LICENSING.md:63`) updated to 1.19.5, its provider package bundled (R07g1
  F4) and the component declaration; `THIRD-PARTY-NOTICES.md` regenerated; `.github/PULL_REQUEST_TEMPLATE.md` gains the DCO 1.1 line (Q2).
- **Plan:** `plan/cross-layer/parked.md` P4 is picked up (§7); its "may skip approval" clause stays unbuilt (§2.3).

## 24. Plan corrections recorded

**From the brief §4 (items 1-19), with what this spec does.** (1) `L1:183` (the brief cited `:158`): the probation wording corrected (§23 L1 item 5); the 5 s
probation tick (`N/mod.rs:140`) runs with no probation pending, a Layer 1 timer for the trim list; the deadline is wall-clock epoch seconds
(`NI/update_guard.rs:90-93`). · (2) `L1:156` (cited `:131`) and `L1:185` (cited `:160`): §3.3 and §12.6. · (3) `L3:51` "restarts a dead child ≤10 s" holds
for the first four exits; the fifth in 60 s is `Degraded` until a restart request (`N/supervise.rs:61-71`, `:990-1008`). · (4) `L3:55` does not hold on hubs
as built (§8.1); `L3:19`, `:35-36` call opencode adopted on hubs while nothing installs it (§8.2); the code comment pins 1.18.18 (`A/src/tasks.rs:33-36`). ·
(5) `L3:86` G2's "fetches its provider SDK" needs a re-test with the five flags (R07g1 F4). · (6) `L3:48`, `:110` LAN offload is not built on the hub side;
a Layer 3/7 item outside Layer 9 (Q14). · (7) `L3:49`, `:84`: §23 L3. · (8) `L3:50` is true once Layer 8's file lands. · (9) `A/src/store.rs:139`, `:145` write
non-atomically under the module's promise (`:1-3`): §15 item 1. · (10) `UI/src/lib/layers.ts:75` and README §3: §14.3 rewords the UI line only; the root
README stays (D36), recorded in §0's scope limits. · (11) `API:840-852` lacks `applied`: §18.2. · (12) `L5:241` (cited `:239`) and the textless rows: §23 L5
items 13-14. · (13) `L6:487-488` (cited `:458-459`) stays true. · (14) `L0:32` recipes stay install-only; `L0:143` `skills/` deferred. · (15) P4 picked up
(§23). · (16) the timer baseline: the 2 s `status` tick (`status.rs:62`), the 2 s agent-relay `absent_poll` (`N/dashboard/api_v1/relay.rs:111`, `:892`) and the
30 s LAN refresh belong in Layer 7's baseline row. · (17) the L2 tags: Layer 7's are already A14 (`L2:449-456`); Layers 8 and 9's go in one block (C5). ·
(18) the plan README's "cap 2" gap-fills reads three. · (19) Home Assistant core is Apache-2.0; P4's "may skip approval" is narrowed by D26 (2) (R06 §2.2).

**Where the code overrules the brief or a report.**
- **C1** The helper honours `revert-request.json` only while a probation journal is pending (`NI/update_helper.rs:282-308`, `:297-300`): `update.rollback` is
  probation-only and a `human_step` otherwise (§7.2). The brief left this "a Layer 1 read at build".
- **C2** The v1 inner layout has no stream field; every packet opens on stream 0 (`M/ids.rs:19-22`; `M/envelope.rs:1373`, `:1383`); a `stream_id` TLV is
  reserved (`L5:485`): R07g2 R2's "no change to the wire" does not hold (§24 D1).
- **C3** Enable never downloads (`N/components/mod.rs:36-40`): `component.enable` with no cached payload is a `human_step` naming the install verb.
- **C4** The guard's report is only printed (`NI/update_guard.rs:94-96`): a start-time revert is read from the run marker and the tombstone (§3.3 rule 5).
- **C5** Layer 7's tags are registered as A14 (`L2:449-456`): Layer 9's two join Layer 8's A19 block (L8 C4).
- **C6** The deny block and the `HOME` fix have not landed (`A/src/tasks.rs:495-509`, `:429-433`): §15 item 1.
- **C7** `/health` is two-state (`N/health.rs:24-40`) and the tri-state is the agent's (`L3:84`): `health_is` names which (§6.2).
- **C8** A services row publishes only after two identical ticks (`N/services/mod.rs:1445-1481`): children are observed through their watch, rows through
  the frame (§3.1).
- **C9** `Proposal.kind` holds nine kinds and the agent seventeen tools at `fa1daeb` (`proposals.rs:104-114`; `A/tests/daemon_tools.rs:318`): bumps are relative.
- **C10** Layer 8 §11.6 adds the release's source tarball as a manifest `[source]` pin; the brief's hub-only source asset is that pin (§24 D4).
- **C11** A SIGNED `tcp-lan` payload is about 65,381 B (`M/envelope.rs:81`, `:2198`): 4 KiB plus 64 KiB does not fit one frame (§24 D5).
- **C12** Line moves since 2026-11-02, re-pinned here: `L1:154`, `:156`, `:183`, `:185` (were `:129`, `:131`, `:158`, `:160`); `L5:141-142`, `:175`, `:241`,
  `:452`, `:831` (were `:139-140`, `:173`, `:239`, `:453`, `:832`); `L6:487-488`, `:657-658` (were `:458-459`, `:628-629`); `L2:91`, `:227`, `:234` (were `:85`,
  `:217`, `:224`); the daemon tick `N/mod.rs:906` (was `:902`); `roles(s)` `N/fabric/mod.rs:435-444` (L7 cites `:334-343`); `KINDS` `proposals.rs:104-114`;
  `API:840-852` (was `:746-758`).

**Deviations from D35 and the brief (each needs the owner's acknowledgement).** **D1** OB F13 item 12, `STREAM_AGENT = 1`, is reserved, not carried: v1
sends `AGENT_OP` and `PATCH` on stream 0 in the pair's sequence (C2); a long op over a 233 B bearer shares the cumulative ACK with chat, bounded by the
receiver's range list (`L5:205`) · **D2** `update.rollback` is probation-only (C1) · **D3** a component action never downloads (C3) · **D4** the hub-only
source asset is Layer 8's `[source]` pin, fetched by the install verb on an opted-in hub (C10) · **D5** the attachment cap is the SIGNED frame budget minus
the body and framing, not 64 KiB (C11) · **D6** F3's "three small files … under `state/`" are the paths Layer 8 pins, `state/heal/{run,inbox,undo}` and
`state/heal/journal.jsonl` (J2, J7; L8 D13), followed here because the contract is stated once, in Layer 8 §3 · **D7** F3's "registered in the one L2
table in one amendment" is Layer 8's A19 block for Layers 8 and 9, Layer 7's tags being A14 already (C5) · **D8** `heal.fixes.send` takes `off`,
`home_hub`, `pinned_contacts`: F10 A names a pinned contact as a recipient while OB §3 lists only `off` and `home_hub` · **D9** settings live in
`state/heal.json` (L8 C1's reason) · **D10** a T1 diagnosis returns as a `kind: diagnose` proposal with a JSON body, folded into the incident and not shown in Needs you · **D11** `heal.serve_members` has
its own consent, `heal_serve` · **D12** the narrative in a delegated reply is the hub playbook's line, not model text · **D13** a start-time revert is
detected from the run marker (C4) · **D14** accepting a `patch` proposal on a hub creates a `code` draft · **D15** the default rung `propose` files proposals
on an unconfigured node; "inert" is read as "nothing changes, leaves or loads without a human" (§17.1).

**Open for the owner (each applied here as the lead's default until answered).** (1) `diffy` as build-time weight on every `111-agent` (AGENTS.md asks
the owner's say; OB F7 said so; D35 took the pick) · (2) `AGENT_OP` and `PATCH` on stream 0 in v1 (D1), or the reserved `stream_id` TLV, a wire change ·
(3) `update.rollback` outside a probation stays a `human_step` (D2), or Layer 1's helper honours a revert request after confirmation, against its monotonic
floor.

**Numbers.** Kept as the lead's defaults (OB §5): the 90 s confirm · five kept snapshots · 64 open incidents · 16 incoming fixes per author · twelve
predicates; depth 3, 16 leaves, 2 KiB · a 64 B literal over 100 lines or 8 KiB · `within_events` ≤ 64 · excerpts 512 B, 120 B in a delegation · a fix
body ≤ 4 KiB. The drafter's own, for the panel: the 4,096 B inbox cap · K = 3 for `accepted_by_n` · the playbook budget 2,000 tokens · 10.111.9x.0/24.

## Changelog v0.1

- 2026-11-11: first draft from D35, D36, the options brief, research R00-R07g3, the Layer 8 draft's joint contract and the code at `fa1daeb`, for the
  adversarial panel.
