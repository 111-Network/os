# Layer 9 - Self-Healing & Evolution: spec v0.2 (draft, the adversarial panel's findings applied)

The layer that makes a node notice its own failures, explain them and fix the ones it can fix safely, with a human at the last step. **`111d` observes**
every layer the node runs from events it already receives: no timer, no poll, no new process. It records each failure as an **incident**, checks it
against signed **playbooks** without a model, asks Layer 3's model only where one already runs and only to choose among the playbook's causes, and turns
the answer into a **proposal of typed actions**. When the human accepts, `111d` applies each action with a persisted **undo**, probes that the capability
works, and reverts by itself on failure or after its own death. A phone asks its **home hub**. A validated typed fix can be **sent, sealed**, to the home
hub or a contact, where it is a proposal again once the same failure is open there. A **code fix** is a draft on a hub that opted in, exported for the human to open as a pull request.
Nothing applies code, nothing is shared without a human, and the agent never holds a key or writes anything but a proposal.

Status: **draft v0.2, 2026-11-13**, the adversarial panel's three reports applied under the lead's triage (`review/triage.md`; what changed where is
`review/fix-map.md`). Written from D35 (the owner took every pick, detail pick, plan correction and default of
the options brief), D36, the brief (`plan/layers/9-evolution/options.md`), its judge's check (`review/options-check.md`: ready), research R00-R07g3, the
Layer 8 draft (`8-storage/README.md` v0.1, whose §3 is the one joint storage contract) and the as-built code. v0.2's §4, §7.4, §7.5 and §23 L8
are re-read against Layer 8 as it stands on 2026-11-13 (locked v1.0, `DECISIONS.md` D38) and are re-checked against it at Layer 9's lock. Nothing here is built. No Layer 9 code is
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
`L0`…`L8` = the layer READMEs; locked Layer 7 and the Layer 8 draft are cited by section and item, never by line. The v0.1 citations were read on 2026-11-11
at head `fa1daeb`, which is behind HEAD in committed code, not only in the working tree (for example `M/fabric.rs:1237-1243` was `send` there and is
`link_events` at HEAD); a citation v0.2 changed was read at head `680162d`, or at `0d1dd41` (committed content) for those its integration fixes
added, each re-read at `61ae26d` (committed content); every `file:line` is re-pinned at the lock head before the lock (§24 C12).
The working tree carries uncommitted Layer 7 edits; no citation here reads them: the two UI files cited (`proposals.tsx`, `settings/index.tsx`) are read from the committed tree. **Evidence:** *(src)* source read ·
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
| Sharing (§11) | one sealed `PATCH` carrying a typed fix (no attachment), sent by a human to the home hub or a contact; a proposal there only while the same incident is open and only for the peer subset (§11.4) | the hub fix shelf (a Layer 8 catalogue class), vouched authors, Tier 2 and LoRa fix cards, `code` and `kb` fixes on the wire (§22) |
| Delegation (§12) | `diagnose`/`diagnosis` over `AGENT_OP`, home hub only, over `tcp-lan`, the one fabric link built in v1; the size gate written for later links and proven on mesh111's `lossy` link | hub-to-hub; the bearers later layers build; the 160 B floor through Tier 2 |
| Evolution (§13) | agent data by release only; `kb_draft` export; three bridge actions, human or `device.propose` only; the CI suite | a node overlay after a corpus and a suite exist; hub eval replay |

**Relation to D35, the brief and the README.** D35 is the law: every pick of OB §2 (F1-F14), every detail pick of §3, every plan correction of §4 and
every default of §5 (Q1-Q23). The §3 detail picks are the lead's and open to the panel. Where the brief or a report disagrees with the code, the code
wins and §24 records it; where the code forces a departure from D35, §24 "Deviations" names it for the owner. The joint storage contract is stated once,
in Layer 8 §3; this spec cites it by item (J1-J9) and restates none of it. The root README is the vision (D36): this spec never rewords it. Where v1 builds
less than a README sentence promises, the table below says so and names the version that closes it.

**v1 scope limits against the root README (D36).**

| README sentence | What v1 builds | Closed by |
|---|---|---|
| §9 "continuously observes the health of every layer the device runs" | event-driven observation of what `111d` receives (§3); a hung, not dead, `111d` on a phone is seen by nothing (Q4); only a service row in state `error` opens an incident, so Layer 7 has no incident source in v1 until it publishes an `error` code (§3.4) | v1.1: the opt-in hub probe (§22); a Termux hang watchdog is Layer 1's; Layer 7's `error` codes |
| §9 "The local agent continuously observes"; §3 "A local model acts as the continuous agent: … the autonomous monitor of system health" | `111d`'s code observes; the model enters at T1 only (§5.1); a model that monitors is not planned for the Pi floor | the opt-in hub probe of §22 |
| §9 "formulates a diagnosis" | T0 for causes a playbook names; T1 only where Layer 3 already runs a model; a phone asks its home hub only under its own consent and only over `tcp-lan`, the one fabric link built in v1; the size gate is written for the links later layers add and proven on mesh111's `lossy` link (§12) | v1.1: hub-to-hub delegation; the bearers later layers build; Tier 2 for the 160 B floor |
| "Layered Architecture": "Categories 2A and 3 consume the intelligence and self-healing layers through a nearby hub"; §3 "lighter devices route agent requests to a nearby hub" | the enrolled home hub only (§12.3) | hub-to-hub delegation (§22) |
| §9 "generates and internally validates a corrective change" | a typed change validated by its own capability probe on this node (§7); a code change is a draft that "applies cleanly, not built" (§8) | v1.1: OB F7 C, a sandboxed build on opted-in hubs after a measured Pi 5 and N100 run |
| §9 "packages it as a pull request"; §"Core Goals" "sends updates and PRs" | the hub renders `fix-<id>.mbox` and `PR.md`; the human opens the PR (§9) | v1.1: an opt-in single-repository hub token (OB F8 B) |
| §9 "Nodes share proven fixes with each other through the gateways, so a repair validated on one node can heal others" | a human sends one sealed typed fix, with no attachment, to the home hub or a contact, over `tcp-lan`, and through a gateway carrier only when it seals to at most 2,937 B (L7 §6.2); `code` and `kb` fixes do not travel: they reach other nodes only through the human's pull request (§9). The receiver takes a fix only from the senders its human set (§17.1) and holds it as "does not apply here yet" until an incident with the same fingerprint opens there; only then is it a proposal its own human accepts, and it carries only `service.restart` of that incident's own subject, `advise`, or an action byte-equal to the receiver's own playbook action for the same cause; everything else is shown as "their words, not checked", with no accept button (§11.4) | v1.1: the hub fix shelf with Layer 8, vouched authors, LoRa fix cards; attachments under the same `111/fix/v1` tag (§22) |
| "Vision" and §5 "the same fabric carries … validated patches"; "Core Goals" "proposes validated patches" | the fabric carries typed fixes; a code patch is a draft a person sends as a pull request (§8, §9) | attachments under `111/fix/v1` (§22) |
| §9 "improves the agent itself, expands the Knowledge Layer, and refines bridging logic" | agent data changes only by release; KB growth is an exported draft; bridging grows by three typed actions that a human applies or accepts from an agent's `device.propose`; no playbook carries them (§13) | later: a node overlay after a corpus and a must-not-break suite exist |
| §3 "test candidate fixes in a safe internal sandbox" | no sandbox on any node; the words v1 uses are §8's | v1.1: OB F7 C on hubs |
| §3 "The model itself can be updated or expanded by the same self-healing process" | a model pin changes only by a project release with an eval report (Q20) | not planned for nodes (D10 refuses on-node fine-tuning) |
| "Core Goals": "coordinates with other nodes to apply fixes where a proven path exists" | coordination is a human sending a typed fix and a human accepting it on a node where the same incident is open (§11, §12) | the auto-apply rung (§2.3) and the fix shelf |

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
2. It removes the Termux cron seam (`1-bootstrap/termux/cron/crontab.line:5`, `termux/cron/healthcheck.sh:12-16`) and claims no saving: nothing in
   the tree schedules it today (no code installs the crontab line or starts `crond`; the provisioner pushes only the script,
   `NI/install/phone/mod.rs:24`, `:224-229`). The `cronie` deb leaves the Termux closure (`L1:68`), and its deb size is the measured saving (§23 L1,
   §24 D26). Whether a hand-provisioned phone runs the script is recorded on the owner's real-device run, not a lock condition.
3. With every switch at its default, nothing leaves the node, Layer 9 starts, stops and reloads no model, nothing changes without a human, listeners and processes equal the
   Layer 8 baseline, the heap adds 0 beyond the binary delta and the held `[match]` table, and idle RSS stays under a ceiling of the Layer 8 build's
   idle RSS plus 512 KiB (default; measured at build, recorded in `9-evolution/TASKS.md`; G7, §20, §21); Layer 7's no-timer line does not move (L7 §3.7).
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

**2.2 What happens without a human (normative, exhaustive).** Two lists: what changes this node without a human, and what leaves the node or runs under
a consent or a setting the human gave once.

*Changes to this node without a human:*
1. The revert of an action whose own probe failed inside its window (§7.4).
2. The revert of an action whose undo record is found `armed` at daemon start and bound to the previous run (§7.5: "armed at start = revert"); it runs
   at most once (§7.5).
3. The revert of a reach-class action whose confirm window closed (§7.6).
4. Recording: incidents, the ledger, the audit (§4). Nothing else. The agent's bearer stays refused on every write; no rung applies code (structural, not a
   setting); a delegated reply (§12) and a fix from a peer (§11) never apply without a human, whatever later rung exists.
5. An automatic revert never passes a consent gate. When restoring needs a consent that is not current (a component's licence notice whose hash changed,
   a bridge edge paused for consent, `API:482`), the revert stops as `revert_blocked`, shown in Needs you; the human restores or leaves it as is (§7.4).

*What leaves or runs under a standing consent or setting* (each named with its switch and default in §17.1):
6. T1 tasks on a node whose `model` row is live, under `diagnose` or `propose`, only for an incident a loaded playbook matches (§5.1).
7. The `diagnose` request a member sends its home hub when an incident opens, under `heal_delegate` (§12.5).
8. A hub's `diagnosis` and `refuse` replies to its members, under `heal_serve` (§12.4).
9. The phone notification, under `heal.notify` (§14.4).
10. The vouch: accepting a received fix sends its author one signed `accepted` vouch, whatever `heal.fixes.send` says; a `reverted` vouch leaves once, with
    no further human act, when that fix's own loop reverts it (a failed probe, a closed confirm window, armed at start); a later human Undo sends nothing
    (§11.4 step 7). The `fix` card says so in fixed text before the accept.

**2.3 The auto-apply rung: pinned, not built.** A later release may add the rung `auto` without redesign. Its grant is a `heal/grants` record (§4)
written only from a human session: `{schema: 1, id, action, target, count_per_boot, consent_hash, by: "human"}`; `action` is one typed action of §7.2 other
than `component.*`, `settings.set` on a consent-gated key, and never identity, keys, the update channel, consent records or code; the actions that
change what the radio emits, `radio.region.set` and `rns.interface.*`, are excluded too, overridable by the owner (Q7). An
action applied under a grant carries `decision.by = "policy"` in the audit. v1 parses no grant, shows no `auto` rung, accepts no `heal/grants` write and
never reads a `heal/grants` record; an unknown value of any setting reads as that key's default (§17.1), so `heal.autonomy: "auto"` left by a later release
reads as `propose` after a rollback.

## 3. The observer inside `111d` (D35 F2)

**3.1 Where it runs and how it wakes (normative).** One task in `111d` (`N/heal/observer.rs`, design). It wakes only on:

| Source | What it gives | Where it exists |
|---|---|---|
| each supervised child's state watch | `Running`, `Backoff`, `Degraded`, `Stopped` transitions | `ChildState` `N/supervise.rs:415-421`; `state_watch()` `:627-629`; the children `111d` supervises: agent, model, transport (`N/mod.rs:1043-1047`), plus meshmud and component runners, handed over at run time by the generation watch (rule 4) |
| each child's exit facts (new, §3.2) | exit code or signal, whether the exit was requested, restart count | `let _ = child.wait().await` discards them today (`N/supervise.rs:972`) |
| the services registry's change signal | a published row changed; the observer then reads `Registry::snapshot()` | `Registry::state_changed` (`N/services/mod.rs:1451`, `:1474-1477`, Layer 7 §3.6); `snapshot` `:1515` |
| the `EventBus`, as a wake-up only | `status` (the observer then reads the health struct, `N/dashboard/api_v1/status.rs:332-337`); Layer 8's `storage` (the observer then reads Layer 8's typed storage state) | `subscribe()` `N/dashboard/api_v1/events.rs:148`; ring of 256 (`:36`) |
| the start-up tombstones (§3.3) | the previous run's end; a reverted update | `state/heal/{run,inbox}` (J2); `state/update.json` (`NI/probation.rs:44-51`) |

Rules: (1) No interval, no sleep loop: the registry changes on a task that already ticks (the 2 s services registry, `N/services/probe.rs:45`, `:520`)
and the `status` publisher already runs (`status.rs:62`, `:411`); the observer runs inside wakeups they already cause. A bus frame is a wake-up only:
the observer never parses a frame (a frame is a compact JSON string and the receiver gets every kind); it reads the typed state `111d` holds
(`Registry::snapshot()`, the health struct, Layer 8's storage state). The `status` heartbeat wakes it at least every 30 s (`HEARTBEAT`, `status.rs:63`),
inside Layer 7 §3.7's line (§21). (2) A subscriber that lags (the ring is 256) re-reads the registry snapshot once and continues; it never replays the
ring. (3) "Sustained for N s" is one `tokio::time::sleep` armed by a transition and dropped on the reverse transition; N is a policy the code already
has (§3.4), never a new constant. (4) Children created at run time (meshmud, a component's runner) are handed over by one additive seam: `Supervised`
and `Components` publish a generation `watch`; on a change the observer re-reads the control list and awaits the new children's state and exit
watches. No socket, no poll. Netlink is not a source: each netlink socket has one consumer and no fan-out (`debounced`, `N/transport/netlink.rs:315`;
`forward(source, kick)`, `N/devices/hotplug.rs:305`), a second socket would be a new open file in the required configuration, and its effect arrives as the `radio` and
`transport` rows and the `devices` frame (§24 D27).

**3.2 What the supervisor publishes (additive, Layer 1 code seam).** `ChildControl` gains `exits() -> watch::Receiver<ExitFacts>` with `{seq, code: Option<i32>,
signal: Option<i32>, requested: bool, restarts_in_window: u32}`, filled from the `ExitStatus` the loop already awaits (`N/supervise.rs:972`) and the
schedule's window (`BackoffSchedule::record_exit`, `:465-485`). `requested` is set from the stop flag and the restart flag the loop reads after the wait
(`:976-985`) and the facts are published after that read: every stop and every restart sends SIGTERM (`stop`, `:645-651`; `restart`, `:688`), so only
`requested` tells a failure from a request. Behaviour, back-off and the `Degraded` rule (5 exits in 60 s, `:61-71`; terminal until a restart request,
`:990-1008`) are unchanged byte for byte. A watch coalesces, so an incident's `count` comes from `ExitFacts.seq`, never from the transitions seen.

**3.3 Who witnesses `111d`'s own death (normative).** Out of process, event-driven:
1. **Run marker** `state/heal/run` (J2): JSON `{schema: 1, boot_id, release, clean: false, starts}`, written atomically 0600 at start before serving; a
   graceful stop rewrites it with `clean: true`. `boot_id` is `/proc/sys/kernel/random/boot_id`, or `-` when it cannot be read *(unverified from this
   box on Termux)*: then `unclean_boot` is not told from `unclean_exit`, and an unclean marker reads `unclean_exit`. `release` is the basename of
   `current` (`NI/update_helper.rs:49-54`). `starts` is the old marker's value plus one (0 when there is no marker), read before the rewrite.
2. **Linux:** the unit gains `ExecStopPost=/usr/local/bin/111 heal-witness` (no `+`: it runs as `node111`), which appends one line built from
   `$SERVICE_RESULT`, `$EXIT_CODE`, `$EXIT_STATUS` *(doc, R05 F5)*. `$EXIT_STATUS` is a signal name for a killed service: `111 heal-witness` maps it to
   its number from a compiled table and writes `-` for a name the table lacks. `111 heal-witness` exits 0 on every error (a missing `state/heal/`
   included), so `ExecStopPost=` never fails the unit. With `Restart=always` and `StartLimitIntervalSec=0` (`1-bootstrap/systemd/111node.service:18`,
   `:42-43`) the unit never fails, so `OnFailure=` would never fire.
3. **Termux:** the runit `finish` hook, which today only logs code and signal (`1-bootstrap/termux/sv/111node/finish:2-8`), appends the same line with
   `result=exit code=<n>` when the daemon exited, or `result=signal signal=<n>` when runit reports a signal (its first argument is -1 then) *(doc)*.
4. **The inbox line** `state/heal/inbox` (J2), ASCII, at most 96 B: `v1 boot=<32 hex|-> result=<word ≤16> code=<≤3 digits|-> signal=<≤3 digits|->`
   (84 B at most, arithmetic). No free text. The file keeps the newest 256 lines: each writer appends through a temp file and a rename, as Layer 8's J2
   words it, so a crash loop with the store unopenable cannot grow it.
5. **At start, before serving,** in this order: `111d` reads the marker without rewriting it, the inbox and the update tombstone; reconciles
   `state/heal/undo` (§7.5); then rewrites the marker (`starts` plus one, `clean: false`). It opens or bumps the incidents of the two tables below
   (§3.4), folds the lines into `heal/` and clears both files (J2). One event, one incident: each inbox line, oldest first, is classified alone.

   | Inbox line | Opens or bumps |
   |---|---|
   | `result=success` (Linux) or `result=exit code=0` (Termux): a clean exit | nothing |
   | `result=watchdog` | `(1, 111d, watchdog_kill)` only |
   | `result=signal signal=9` written by the Termux hook | `(1, 111d, killed_by_os)` only (Android's phantom-process kill); its shipped playbook's one action is the `human_step` "On your computer, connected to this phone, run `111 install --phone` again" (OB §3 "The phone"), one card with the §12.6 step when both are due |
   | any other line, also under a clean marker | `(1, 111d, unclean_exit)` |

   | Marker and tombstone | Opens |
   |---|---|
   | no marker (the first start), or `clean: true` | nothing from the marker; the inbox lines still count (a daemon that died before it wrote its marker left the last graceful `clean: true`) |
   | `clean: false`, the same `boot_id`, and no inbox line | `(1, 111d, unclean_exit)`, with evidence `witness: absent` when rule 7 finds no witness |
   | `clean: false`, the same `boot_id`, with inbox lines | nothing more: the lines decide |
   | `clean: false`, another `boot_id` | `(1, 111d, unclean_boot)` (with `boot=-` on either side: `unclean_exit`) |
   | a tombstone with `pending: false`, `health_confirmed: false` and `version` ≠ the running release | `(1, updater, update_reverted)` |
   | a marker `release` above the running one with no such tombstone | `(1, updater, release_reverted_at_start)` (the guard's torn payload revert, whose report is only printed today, `NI/update_guard.rs:94-96`) |

   Start facts close `self_recovered` after the daemon has served for `crash_loop_window_secs` (`N/supervise.rs:61-71`) with no new line: the same
   one-shot sleep, armed when the daemon serves. A start fact never starts a model task by itself, here or through a delegation (§12); T0 runs.
6. **The cron goes** (F13 item 2): `termux/cron/*` leaves the tree and the provisioner stops pushing the script (§23 L1); the copy already pushed to
   `$PREFIX/var/lib/111/share/termux/cron-healthcheck.sh` is deleted by the provisioner on its next run (`111d` does not delete it: rule 7, and it is
   inert); `state/health-failures.log`, which nothing reads, is no longer written by anything. Nothing in the tree schedules the script today (§1
   rule 2). A hung `111d` on a phone stays unseen in v1 (Q4).
7. **Delivery of the witness.** `111d` never writes its own unit or its runit hook: the unit is written only by `111 install`
   (`NI/install/supervise.rs:7`, `:68`) and the hook only by `111 install --phone` (`NI/install/phone/mod.rs:23`, `:218-223`); a self-update changes
   neither, and a file the provisioner or the installer wrote outside the payload is not restored by the update guard's revert, so `111d` changing it
   would change the device outside a signed release flip. The witness therefore arrives only with `111 install` (fresh or re-run) and
   `111 install --phone`. At start `111d` reads the unit (Linux: `/etc/systemd/system/111node.service` holds the `ExecStopPost=` line) or compares the
   hook's bytes with the copy this release carries (Termux: `SV_FINISH`, `NI/install/phone/mod.rs:23`). Without the witness it classifies from the run
   marker alone (evidence `witness: absent`, so `watchdog_kill` is not told apart) and opens `(1, 111d, witness_absent)`, whose shipped playbook's one
   action is the `human_step` "Re-run `sudo 111 install` to add the exit witness" on Linux and, on a phone, the §12.6 step "On your computer, connected
   to this phone, run `111 install --phone` again", one card when the APK step is also due; it closes when a start finds the witness. No drop-in
   written by the root helper in v1 (§22; §24 D25).

**3.4 Incidents (normative).** Fingerprint = `(layer, subject, reason_code)`. One open incident per fingerprint: a repeat increments `count` and updates
`last`, so a crash loop is one incident with at most one model task. A closed incident re-opens when its fingerprint recurs and keeps its record and its
diagnosis until the evidence hash (§3.5) changes. At most 64 incidents are open at once (R01 §3.2); a 65th fingerprint opens nothing and increments
`overflow` on `GET /api/v1/repairs` (capacity, not policy).

**One subject id space, one source per failure.** A subject is a `ServiceId` (`N/services/mod.rs:87-99`), a component id, or one of the fixed daemon
subjects `111d`, `updater`, `identity`, `heal` and Layer 8's subjects; each `(subject, reason code)` pair is a row of one compiled table in node111
(§6.3), and nothing else is a subject. A supervised child is observed only through its state watch and its exit facts: its subject is its service id or
component id and its layer that service's layer (the model child is `model`, never `llama-server`). Rows whose code is `child_backoff`,
`child_degraded`, `child_stopped`, `spawn_pending` (`N/services/mod.rs:707-722`) or `transport_degraded` (`:857`) are not incident sources: the
child's watch and exit facts are.

| Layer | Opens on | Subject · reason codes |
|---|---|---|
| 0 | the `kb` row in state `error` | `kb` · `verify_failed` (`N/services/mod.rs:1028`; `kb_absent`, `:1034`, is `unset`) |
| 1 | §3.3's start facts and rule 7 | `111d` · `unclean_exit`, `watchdog_kill`, `unclean_boot`, `killed_by_os` (Termux), `witness_absent`, `apk_outdated` (§12.6); `updater` · `update_reverted`, `release_reverted_at_start` |
| the service's: 3 `agent`, `model`; 6 `transport`; 5 `meshtastic`; a component's own | an exit fact with `requested: false` (§3.2) of a supervised child, a component runner's included | the service id or the component id · `child_exit` (the code or signal stays in the `exit` evidence; `exit_is` tests it) |
| 2 | `health.identity` false, or the `fabric` row's `no_identity` or `restore_pending` (`N/services/mod.rs:1152-1170`): one fingerprint, whichever arrives first; that row is not also a Layer 5 source | `identity` · `no_identity`, `restore_pending` |
| 3 | the `agent` or `model` row in state `error` with a code that is not a child code above | the row id · its code |
| 5 | the `fabric` or `meshtastic` row in state `error` | the row id · its code (`store_full`, `port_busy`, …) |
| 6 | the `transport` or `radio` row in state `error` | the row id · its code (`stack_down`, …) |
| 7 | none in v1: the `gateway` row's codes are `unset` or `live` (`carrier_pending` is the normal offline state, L7 §3.5 item 2; `N/services/mod.rs:1065-1068`) | — |
| 8 | Layer 8's typed storage state, read after a `storage` frame, each named: `disk_low` set; `store_full` set; the node-state store `unavailable` | Layer 8's subject · `disk_low`, `store_full`, `node_state_unavailable` |
| 9 | a revert that failed; the store unopenable (J2) | `heal` · `revert_failed`, `store_unavailable` |

Rules: (1) Only a service row in state `error` opens an incident, never `unset` or `starting`; this governs service rows, and Layer 8's sources are the
three named above. So `carrier_pending`, `kb_absent` and the `codeagent` row (only `unset` or `live`, `N/services/mod.rs:1088-1121`) open nothing,
and Layer 7 has no incident source in v1 until it publishes an `error` code (§0). (2) Only an exit with `requested: false` opens or bumps an incident: a
requested restart (a human's, Layer 9's own `service.restart`, the applier's rnsd restart, `stop_before`/`start_after` around a dependency restart,
`N/components/mod.rs:449-464`), a component disable, `Stopped`, a held runner and a graceful daemon stop open nothing; the observer stops before
`layer6.shutdown()` (`N/mod.rs:1106`). (3) A child's `Degraded` transition is a state of its `child_exit` incident, not a second fingerprint: severity
`error`, T0 runs again at that change, and `service_state is degraded` reads it. Service rows are keyed per id on `(state, reason_code,
can_run_here)` (`N/services/mod.rs:1433-1435`); optional components are not service rows, and their state is read through `Components::status`
(`N/components/mod.rs:420`) when the generation watch or a runner's watch wakes the observer (§3.1 rule 4). An incident closes as `self_recovered` when
its condition clears: a row leaves `error`; a child has been `Running` for the crash-loop window, `crash_loop_window_secs` (`N/supervise.rs:61-71`), a
one-shot sleep armed on the `Running` transition; a start fact as §3.3 rule 5 says; `witness_absent` and `apk_outdated` when their condition clears.

**3.5 The incident record (normative shape; R01 §3.2; OB §3).** `{schema: 1, id: "inc-<12 hex>", fingerprint: {layer, subject, reason_code}, severity:
"warning"|"error"|"critical", count, state, opened: {seq, boot_id, mono_ms, tick, epoch: null, wall_claim}, last: {…}, closed: {…}|null, evidence: [{id, kind,
ref, excerpt}], checks: [{check_id, result}], ruled_out: [cause_id], diagnosis: {cause_id, by: "t0"|"t1"|"hub", confidence: {checks_satisfied,
agreement}}|null, attempts: [{proposal_id, action, decision, outcome}], dismissed: {cause_id}|null, outcome: "open"|"self_recovered"|"fixed"|"unresolved"}`.
`id` is `inc-` plus 12 hex digits from the OS random source, minted by `111d` as its other id families are (§18.2), so an incident has its id
before any store write, with the store closed and under J7; it never derives from `seq`. `seq` is the store's (J4); `tick` the daemon tick (`N/mod.rs:906`); `epoch` stays `null` until Layer 7's epoch has a consumer (L7 §8); `wall_claim` is
display only. Evidence `kind` is closed: `service_record`, `health`, `counters`, `exit`, `witness`, `update_tombstone`, `child_log_tail`. An excerpt is at
most 512 B and comes only from `111d`'s own typed sources and its own children's logs (agent, model, rnsd, meshmud: `N/dashboard/api_v1/services.rs:10-14`);
never a message body, never a bridge adapter's or the presence sidecar's log text (counters and reason codes only, Q13). A `child_log_tail` excerpt is
the line a `log_contains` check matched, else the last line before the exit, at most 512 B; it stays on the node. Off the node goes only text 111
wrote: the row's reason, the exit facts, the check results and a matched literal of at most 64 B, never a log line (§12.1 key 9, §17.2). Severity is
the playbook's, else `warning`; a `child_exit` incident is `error` from its child's `Degraded` transition (§3.4 rule 3); `critical` is kept for
`revert_failed`, a daemon crash loop, `no_identity` and an unopenable store. Field names follow the Kubernetes Event, the journal's `_BOOT_ID` plus
monotonic pair and OpenTelemetry's dotted names; no SDK.

**The evidence hash (normative, defined once here).** BLAKE3-256 over the fingerprint, the `[check_id, result]` list sorted by `check_id`, the exit
kind (a code or a signal, as the last exit facts hold it, or none) and the sorted set of satisfied cause ids. Never an excerpt, a count, a sequence
number or a time, so a flapping child whose log line differs on every run keeps one hash. The diagnosis reuse (§3.4, §5.1) and the ledger (§7.7) read
it.

**3.6 Other sources, stated.** Termux: netlink is blocked for apps on Android 11+ *(unverified from this box)*, so hotplug keeps its 5 s fallback
(`N/devices/hotplug.rs:3`) and Layer 9 adds nothing. Layer 9 reads no battery state: nothing in v1 consumes it (§24 D37). Linux PSI triggers are a later opt-in (§22). Layer 9
has no heartbeat of its own (Q8).

## 4. Where Layer 9's state lives: the joint contract, consumed (D35 F3)

The contract is Layer 8 §3, items J1-J9. Layer 9 adds nothing to it. This table says which Layer 9 state lives under which item. J1 names
incidents, apply attempts, the loop ledger, kept-change snapshots, received fixes and the audit trail; the kinds below that J1 does not name (the
proposals record, sent fixes, the `high_water` records, delegations and the hub's stored replies, drafts, grants) live in the same `heal/` namespace under
the same J1 rules, and naming them in J1 is a request to Layer 8 (§23 L8). Each record kind's states, transitions and reclaim are §4.2's; the store's
`state` column is always one of §4.2's store states, never a display phase.

| Layer 9 state | Where | Item | Never reclaimed (§4.2) |
|---|---|---|---|
| incidents | `heal/incidents` records | J1, J3, J4 | `open` |
| apply attempts | `heal/attempts` records | J1 | `armed`, `awaiting_confirm`; `revert_blocked` and `revert_failed` until the human resolves or dismisses them (§7.4) |
| the proposed actions of each `repair`, `fix` and `human_step` proposal and their `action_hash`, written in the same J9 call as the proposal (§7.3) | `heal/proposals` records | J1 | `undecided` (a `decided` record goes with its proposal) |
| the loop ledger, keyed `sha256(action ‖ target)` | `heal/ledger` records | J1 | — |
| kept-change snapshots, the last five | `heal/snapshots` records | J1, J3 | — (a snapshot belongs to its kept change, the last five by capacity, Q7; reclaimed last, §4.2) |
| fixes received and sent | `heal/fixes` records | J1, J6 | `undecided` |
| one `{author, high_water}` record per trusted author, the highest `author_seq` accepted (§11.4 step 3) | `heal/fixes` records | J1 | `high_water` (never reclaimed) |
| delegations (the member's side; a hub's queue is memory only, §12.3) | `heal/delegations` records | J1 | `open` |
| the hub's stored replies, one per enrolled member (the latest), `{member, req_id, request hash, fingerprint, checks hash, reply}` (§12.4); reclaimable, like the per-author fix queue | `heal/delegations` records | J1 | — |
| code and KB drafts | `heal/drafts` records | J1 | `open` |
| grants (§2.3; none in v1) | `heal/grants` | J1 | — |
| the audit trail | stream `heal/audit` (`entries`) | J1, J4 | — |
| the prune marker | `heal/` | J3 | — |
| the run marker, the witness inbox, the armed undo | `state/heal/run`, `state/heal/inbox`, `state/heal/undo` | J2, J8 | — |
| the fallback ring | `state/heal/journal.jsonl` | J7 | — |

Not under the contract, stated: the proposals Layer 9 composes go into Layer 8's `agent/proposals` and their decisions into `agent/decisions` (L8 §2.2
rule 5, §2.4 rule 5): one proposal list for the UI. A T1 result is not one of them: it is folded into its incident and never enters
`agent/proposals` (§5.2). Every new or changed path on a node:

| Path | Owner · mode | Writer | When | Size |
|---|---|---|---|---|
| `node/state.db`, namespaces `heal/*` | Layer 8's file (L8 §2.2) | `111d` through J9 | on a Layer 9 event | the `heal` class (L8 §4.1) |
| `state/heal/` | `node111` · 0700 (L8 §5.1's path table, §21 L1) | — | created at the first start | — |
| `state/heal/run` | 0600, atomic | `111d` | at start and at a graceful stop | ≤ 128 B |
| `state/heal/inbox` | 0600, through a temp file and a rename | `111 heal-witness`, the runit `finish` hook | at each daemon exit; cleared at start | the newest 256 lines of ≤ 96 B (J2) |
| `state/heal/undo` | 0600, atomic, fsync | `111d` | before an action's first effect; rewritten at each phase change (§7.4, §7.5); removed last | one snapshot (measured at build) |
| `state/heal/journal.jsonl` | 0600 | `111d` | J7 only | ≤ 1 MiB ring |
| `state/heal.json` | 0600, atomic | `111d` (`PATCH /repairs/settings`) | on a settings change only | ≤ 1 KiB |
| `agent/work/<task>/{opencode.json, home/, src/}` | the agent's (`A/src/tasks.rs:397`) | `111-agent` | a `patch` task on an opted-in hub | the source tarball unpacked (measured at build) |
| `cache/packages/opencode/` | root · 0755 (the components cache, `N/components/registry.rs:5`) | `111 install --components opencode` | on that verb | 211-222 MB plus the source tarball, checked against the running release's `[source]` pin before a `patch` task (§8.3); Layer 9 writes nothing under `state/origin/` |
| `current/share/kb/playbooks/*.md` · `current/share/agent/` | the release payload, read-only | the release | each release | measured at build |
| `state/apk.json` (phones) | the node user · 0600 | the host provisioner, through `runas::install_file` (`NI/install/phone/runas.rs:128`) | after the APK step of `111 install --phone` (§12.6) | `{schema, apk: {package: sha256}}`, ≤ 3 entries |
| removed: `termux/cron/*` from the tree; the provisioned copy `share/termux/cron-healthcheck.sh` (deleted by the provisioner on its next run, §3.3 rule 6); `state/health-failures.log` | — | — | — | − |

Every Layer 9 write is one J9 call (`put`, `append`, `set_state`); the audit record is
`{seq, boot_id, mono_ms, kind, proposal_id, action_hash, probe_result, revert_result, confirmer: "probe"|"human", by: "human"|"daemon"}` with `kind` in
`proposed`, `accepted`, `applied`, `kept`, `reverted`, `revert_failed`, `dismissed`, `undo`, `fix_sent`, `fix_received`, `delegated`, `refused`,
`revert_blocked`, `vouch_sent`. Nothing in
`heal/` is signed (J5) or leaves the node (J6).

**4.1 Start and degraded modes (normative).**
1. **At start,** after §3.3 rule 5 and §7.5 and once the store opens, `111d` lists `heal/incidents` in state `open` (J9 `list`, paged by the `seq`
   cursor), re-arms each one's recovery window (§3.4's one-shot sleep, armed on the state it reads now) and closes as `self_recovered` those whose
   condition is clear: a row no longer in `error`, a child `Running` once its window has passed, a start fact as §3.3 rule 5 says.
2. **The store closed** (no key in Mode T, a newer `user_version`, a damaged file: L8 §2.2 rule 8): the J2 files stay (J2), the observer keeps at most 64
   incidents in memory and records `(9, heal, store_unavailable)` there, T0 runs, nothing else does: no proposal is filed, no T1 or human task is
   submitted, no fix is sent and no delegation composed. `GET /api/v1/repairs` answers `store: {state: "unavailable", reason}`, and the decision route
   answers `503 store_unavailable` and applies nothing (L8 §2.2 rule 8). When the store opens, the in-memory incidents are written and rule 1 runs.
3. **J7** (the store not built): the ring holds `heal/` (J7); observation and diagnosis run, and no proposal is filed until the store exists.
4. **`Full`** (L8 §4.5: the class is `full` while its export offer is unanswered): observation continues in memory under the 64-incident bound; no new
   `repair`, `fix` or `human_step` proposal is filed; `RepairsStatus.store = {state: "full"}`, and the Repairs page links to the Storage page's offer.
   An armed apply finishes and its decision is written (J9 writes the three decision calls whatever the class's state); its attempt is written once
   the class has room, and until then the undo file keeps its last phase (§7.5). A T1 fold that returns `Full` leaves the spool file for the next sweep
   (Layer 8's hand-off rule unlinks only after the write).
5. **Inbound frames while `full` or closed.** A consumer for 0x10 or 0x13 never returns an error, so the fabric acknowledges the frame (L8 §12.3) and
   nothing is retransmitted without end: a fix is dropped and counted (its author's `high_water` is not advanced); a `diagnose` is answered `refuse` 3.
   No fix is sent, no delegation composed, no T1 or human task submitted. An accept or a task that would create a draft answers `503 store_unavailable`
   with the store closed, and `409 heal_full` naming the Storage page while the class is `full`.

G19 (7) asserts rules 1-4 and G22 (12) rule 5.

**4.2 Record states and the reclaim (normative).** The store's `state` column of each record kind takes only the values below; the display phase is
§14.2's and is computed from them. Layer 9 registers one reclaimer for `heal/*` (J9); `heal` reclaims only in the pass's step 4 (L8 §4.3), when the
budgeted classes exceed `keep` or `disk_low` is set.

| Record (namespace) | Store states | Transitions | Terminal | Never reclaimed |
|---|---|---|---|---|
| incident (`heal/incidents`) | `open`, `closed` | `open` → `closed` with an outcome (`self_recovered`, `fixed`, `unresolved`, `dismissed`); `closed` → `open` when its fingerprint recurs, keeping the record and its diagnosis until the evidence hash changes (§3.4) | none | `open` |
| attempt (`heal/attempts`) | `armed`, `awaiting_confirm`, `kept`, `reverted`, `revert_failed`, `revert_blocked`, `interrupted`, `dismissed` | `armed` → `awaiting_confirm` (reach class, §7.6) → `kept` or `reverted`; `armed` → `kept`, `reverted`, `revert_failed`, `revert_blocked` (§7.4), `interrupted` (§7.5); `revert_blocked` → `reverted` (the human's restore) or `dismissed` ("leave as is"); `revert_failed` → `dismissed` | `kept`, `reverted`, `interrupted`, `dismissed` | `armed`, `awaiting_confirm`; `revert_blocked` and `revert_failed` until resolved or dismissed |
| proposed actions (`heal/proposals`) | `undecided`, `decided`, `withdrawn` | `undecided` → `decided` at the decision, or `withdrawn` when its incident closes (§7.3) | `decided`, `withdrawn` | `undecided` |
| fix (`heal/fixes`) | `undecided`, `parked`, `refused`, `decided`, `sent` | received: `undecided` (a proposal filed, or "does not apply here yet", §11.4 step 7) → `decided`; `refused` ("does not apply here", §11.4 step 5); `parked` ("needs a later release", from a trusted author only, §11.4 step 4); this node's own sent fix: `sent` | `parked`, `refused`, `decided`, `sent` | `undecided` (bounded by the 16-per-author queue, §11.4) |
| author record (`heal/fixes`) | `high_water` | its `author_seq` rises with each accepted fix (§11.4 step 3) | — | always |
| delegation (`heal/delegations`, the member's) | `open`, `answered`, `refused`, `cancelled` | `open` → `answered` (a `diagnosis`), `refused` (a `refuse` other than 3), `cancelled` (its incident closed) | `answered`, `refused`, `cancelled` | `open` |
| stored reply (`heal/delegations`, the hub's) | `stored` | replaced by the member's next reply (§12.4) | — | — |
| draft (`heal/drafts`) | `open`, `exported` | `open` → `exported` when the human downloads its mbox (§9) | `exported` | `open` |
| ledger (`heal/ledger`) | — | rewritten in place (§7.7) | — | always: the loop guard, bounded by the closed vocabulary times this node's targets |
| snapshot (`heal/snapshots`) | `kept` | written at a keep (§7.4 step 4); the sixth oldest dropped at the next keep (Q7) | — | — |
| audit (`heal/audit` stream) | — | appended | — | — (J3) |

**The reclaim order**, oldest `seq` first within each step: (1) closed incidents with their attempts and their `decided` or `withdrawn` proposal records
(J3; an attempt goes only with its closed incident), and in the same step an attempt with no incident (an `apply_unrecorded`, a `device` proposal
through the trait, an Undo) by its own `seq`; (2) `decided`, `refused` and `parked` fixes, then `sent` fixes (a vouch for a reclaimed fix is dropped and
counted, §11.4 step 8); exported drafts; closed delegations and the hub's stored replies; (3) the audit stream (J3); (4) kept snapshots, after which an
Undo card reads "no longer kept, cannot undo". A reclaim never touches a state the last column names and leaves the J3 marker.

## 5. Diagnosis: deterministic first, the model only to choose (D35 F4)

**5.1 The tiers (normative).** Layer 9 starts, stops and reloads no model: T1 and the hub's T2 use the llama-server whose `model` row is `live`, as
`111d` already runs it (`--model --host --port` only, resident wherever the binary and a GGUF exist, `N/intelligence.rs:172-192`). Idle unload is
Layer 3's question (§22).
- **T0** (every tier that runs Layer 9): `111d` evaluates the playbooks whose `[match]` equals the fingerprint (§6). Exactly one cause whose checks all hold
  ⇒ that cause, `by: "t0"`. T0 runs when the incident opens and at each change of its `count` (a child's `Degraded` transition included, §3.4), never on
  a timer.
- **T1** (only where the `model` row is live, under `diagnose` or `propose`, only when a loaded playbook matches the fingerprint, only when T0
  matched several causes or none, and never while `heal` is `full` or the store is closed, §4.1): with no matching playbook the incident goes to T2 or reads `unable_to_diagnose`. One T1 task per incident record: a
  re-opened fingerprint reuses its record and its diagnosis until the evidence hash (§3.5) changes, so a flapping row buys one model request, not one
  per flap. A start fact never starts a T1 task by itself (§3.3 rule 5). The task is one background task through
  the existing loopback task API (`POST 127.0.0.1:11182/v1/agent/task`, the route `111d` relays today, `N/dashboard/api_v1/relay.rs:725`, `:738`), with the
  new `incident` input (§5.2). The answer is a pure classification constrained by a JSON schema to a cause enum, which llama-server turns into a
  GBNF grammar *(doc, R01 §3.3)*. The enum is the causes T0 found satisfied when several were; when none was, it is the playbook's causes plus `unknown`.
  A cause the model picks whose checks are not all satisfied files `advise` only, with `agreement: model_only`, never a typed action; `unknown` reads
  `unable_to_diagnose`. Any narrative is a separate call, made only when the human asks ("Explain"): the incident task route's mode `explain`
  (§18.1), a `diagnose` task with `incident` and `narrative: true` (§5.2).
- **T2**: on a member with `heal.ask_hub` on, the home hub (§12); otherwise the card says "needs a hub" and links to the switch.
- None of the three ⇒ the incident reads `unable_to_diagnose`, honestly.

**5.2 The `incident` input (amends Layer 3, additive).** `TaskRequest` (`A/src/http.rs:159-164`, `API:832-838`) gains an optional `incident` object (§3.5's
fields, minus `attempts`), `causes: [{id, text}]` and `narrative: bool` (default `false`; `true` asks the explanation of mode `explain`
instead of the classification, with the same untrusted block and no `response_format`). With it, `diagnose` renders the incident inside a labelled untrusted block, switches off
`daemon_health` and `recent_events` for that task (today's context, `A/src/tasks.rs:360-390`) and sends `response_format` with the schema
`{cause_id: enum, evidence: [enum of the incident's evidence ids] ≤3}`: the schema constrains `evidence` to the ids the incident holds, and no quote
is asked for; the card shows the excerpt of each cited id. The prompt is ≈0.5 KB (R07g2 F4), never today's ≈6,000-token context (R01 §3.4,
estimate). **The fold (normative).** `111d` records on the incident the id of the T1 task it submitted. The task's result is written as a
`kind: diagnose` proposal whose body is that JSON; at ingest (on `task.done`, `relay.rs:826`, or at a sweep, inside Layer 8's one hand-off rule) a
proposal whose `task_id` names such a task is folded into its incident with one J9 `put`, committed before the spool file is unlinked, and the file is
unlinked: it never enters `agent/proposals`, any proposals list or the open count, and never shows in "Needs you". A hub folds a delegated task's result
into its delegation record the same way (§12.4). The fold applies to the incident task route's modes `diagnose` and `explain` only (§18.1); `kb_draft`
and `patch` proposals are ingested normally.

**5.3 Containment, outside the model (normative).** A reply whose `cause_id` is not in the set or whose evidence ids are not in the incident (the
schema makes both unlikely; the check stays) is discarded, recorded and never retried. Confidence is computed, two fields: `checks_satisfied` (per-mille of
the cause's checks that held) and `agreement` (`rule_only`, `model_only`, `agree`, `disagree`). The model never grades itself; the output is a proposal.
`111d` submits one model task at a time per node (the agent does not serialise concurrent turns, `3-intelligence/TASKS.md:72`; `TaskEngine::submit` starts
one thread per task with no cap, `A/src/tasks.rs:137-184`); further incidents wait in memory, bounded by the 64 open incidents.

**5.4 Cost, stated.** T0: no model, no allocation beyond the parsed playbooks. T1: one call per unresolved incident, about a minute for a short verdict on a
Pi 5 *(unverified from this box: 2.3 tok/s, a search snippet, R01 §3.4)*, measured at build (Q21's bench). Layer 9 adds no model load: the brief's
`--sleep-idle-seconds` is in no argv (§24 C13).

## 6. Playbooks: a Layer 0 category with a closed grammar (D35 F5)

**6.1 The entry (amends L0, additive).** `category: playbook` joins the enum (`L0:45`; rule F05, `0-knowledge/_meta/schema.md:19`), folder `playbooks/`,
filename `repair-<subject>-<reason>.md` (`_` written as `-`; `any` for a subject wildcard), rendered from the block's `[match]`: N06 renders `[match]` into
the expected filename and compares, never splits a filename (so `repair-model-child-exit.md` matches every unrequested exit of `model`). No new
frontmatter field (the twelve of `L0:40` and F02, `schema.md:16`). The applicability matrix (`schema.md:96-110`) gains a `playbook` column: `title`,
`category`, `status`, `summary`, `tags`, `last_verified` R; `aliases`, `sources`, `relations` O; `platforms`, `upstream`, `license` forbidden
(first-party). `relations` carry `depends_on` only when the subject is a third-party project with a KB entry, and then it is required; `runs_on` is
optional; neither enters N06; no relation names a target without an entry (R02), so 111's own subjects (`kb`, `fabric`, `111d`) carry none.
The six H2 sections stay (B02, `schema.md:38`); under `## Details` the H3s are `### Symptom` · `### Checks` (one fenced `toml` block) · `### Causes` (one line
per cause id) · `### Verify` · `### Undo`. The status gates, beside the per-category adaptations (`L0:91`): a `candidate` playbook is lint-clean with
`### Alternatives considered` (B03); `testing` adds a `### Smoke test` on a real node; `proven` means it healed a real incident, logged in `### Field log`
(`L0:88`). Recipes and N03 (`schema.md:53`,
`L0:32`) are untouched; `skills/` stays deferred (`L0:143`). Body budget 2,000 tokens, warning-only (B09, `schema.md:45`).

**6.2 Grammar v1 (normative).** `schema` is an integer; `MAX_SCHEMA` is compiled in; every table is `deny_unknown_fields`; the predicate enum is internally
tagged on `p` with no fallback variant. The twelve predicates (Q9 default):

| `p` | Fields | Reads |
|---|---|---|
| `service_state` | `id`, `is` ∈ `running`, `backoff`, `degraded`, `stopped` | the child watch (`N/supervise.rs:415-421`) |
| `service_reason` | `id`, `is` = a reason code of the compiled table (§6.3) | the published row (`Registry::snapshot()`) |
| `restarts_at_least` | `id`, `n` | `ExitFacts.restarts_in_window` (§3.2) |
| `incident_count_at_least` | `n` | the open incident's `count` |
| `exit_is` | `id`, `code` or `signal` | the last exit facts |
| `health_is` | `of` ∈ `daemon` (`ok`, `error`; `N/health.rs:24-40`) or `agent` (`ok`, `error`: the code the services probe keeps, `N/services/probe.rs:162-165`; the agent's `degraded` answers 200, `L3:84`, so v1 cannot read it), `is` | the health struct `111d` holds (`status.rs:332-337`); the probe's last code for the agent |
| `file_exists` | `root` ∈ `state`, `run`, `data`; `rel` matching `^[a-z0-9._-]+(/[a-z0-9._-]+)*$` | one `metadata()` call, never a read |
| `log_contains` | `id` (an own child), `literal` ≤ 64 B | the last 100 lines or 8 KiB of the sanitised tail (`N/services/log.rs:12-17`); no regex |
| `disk_free_below` | `percent` | the fabric's `statvfs` (`N/fabric/mod.rs:471-484`) |
| `unclean_exit` | — | §3.3's marker |
| `component_enabled` | `id`, `is` | `Components::status` |
| `release_at_least` | `calver` | the running release |

Evaluating a predicate makes no request (no HTTP, no socket, no child spawned) and reads only state `111d` already holds, plus one file tail
(`log_contains`) and `file_exists`'s one `metadata()` call; no ring of past transitions is kept. Combinators `all`, `any`, `not`; depth ≤ 3, ≤ 16 leaves, the block ≤ 2 KiB. A cause: `{id, when: [check ids], action, verify}`. `action` deserialises into the
engine's own action type (§7.2) plus `advise {text}` and `escalate {task}`; the one non-literal is `"@subject"`, replaced by the incident's subject.
`@subject` is a member of the compiled subject table and nothing else, and `plan()` must resolve it to an existing object; a value derived from a channel,
device or peer name never reaches an action parameter. The three bridge actions and the two interface actions (`bridge.edge.pause`, `bridge.edge.set`,
`bridge.order.set`, `rns.interface.upsert`, `rns.interface.remove`) are "human or `device.propose` only" in v1 (§7.2): a cause that names one rejects
the playbook. A cause whose action is not `advise` or `escalate` must name in `when` at least one check with a leaf that is not `log_contains` (log text
is untrusted, `N/services/log.rs:6-7`): the lint and the loader both refuse the playbook otherwise; at run time a cause satisfied by `log_contains`
leaves alone files `advise` only. No templating, no command, no URL, no environment. An unknown predicate, field, service id or reason code rejects the whole playbook, which the
Repairs page lists as "unsupported grammar N, needs release X"; it never runs in part. A predicate reading something this node lacks (a component not
installed) evaluates false. Prior art read: StackStorm rules, the Salt reactor, Nagios and Icinga handlers, Ansible handlers, Kubernetes probes, MAPE-K
(R07g3 §3.4); ours closes the action side too. `cel` 0.15.0 (MIT) is the revisit path at the first playbook the enum cannot say (OB F5 B).

**The block's layout (normative).** One TOML document under `### Checks`; every table `deny_unknown_fields`:

| Where | Key | Type and rule |
|---|---|---|
| top level | `schema` | integer, 1 to `MAX_SCHEMA` |
| top level | `severity` | `warning`, `error` or `critical`; optional, default `warning` (§3.5) |
| `[match]` | `layer` | integer 0-9, a number, never a layer name |
| `[match]` | `subject` | a subject of the compiled table (§3.4, §6.3), or `any` |
| `[match]` | `reason` | a reason code the compiled table holds for that subject |
| `[[check]]` | `id` | ≤ 16 B, lower case, digits and `_`, unique in the block |
| `[[check]]` | `test` | one predicate or combinator, a table tagged on `p`: a predicate of the table above with its fields; `all` and `any` take `of = [ … ]`, `not` takes `of = { … }` |
| `[[cause]]` | `id` | ≤ 24 B, lower case, digits and `_`, unique in the block; its one-line text is its line under `### Causes` |
| `[[cause]]` | `when` | array of check ids, each defined in the block; the cause is satisfied when all hold |
| `[[cause]]` | `action` | a table tagged on `action`: an action of §7.2 by its name with its fields (the applier's `{"action": name, …args}` form, `N/devices/applier.rs:151-171`), or `advise {text}`, `human_step {steps: [string]}`, `escalate {task}` with `task` ∈ `diagnose`, `patch` |
| `[[cause]]` | `verify` | string, one line, optional: what the card says the action's probe shows; it gates nothing (the probe of §7.2 does) |

One valid block (`repair-model-child-exit.md`; it parses with `tomllib`):

```toml
schema = 1
severity = "error"

[match]
layer = 3
subject = "model"
reason = "child_exit"

[[check]]
id = "degraded"
test = { p = "service_state", id = "model", is = "degraded" }

[[check]]
id = "oom"
[check.test]
p = "all"
of = [
  { p = "exit_is", id = "model", signal = 9 },
  { p = "log_contains", id = "model", literal = "failed to allocate" },
]

[[cause]]
id = "held_degraded"
when = ["degraded"]
verify = "the model row leaves error and the child stays up"
action = { action = "service.restart", service_id = "@subject" }

[[cause]]
id = "out_of_memory"
when = ["oom"]
action = { action = "advise", text = "The model ran out of memory: a smaller model or more free memory is needed." }
```

**6.3 The evaluator and the lint (normative).** `111d` reads `current/share/kb/playbooks/*.md` once at start (the KB rides the payload, `N/kb.rs:1-3`),
extracts the block under `### Checks`, parses it with the `toml` crate it already links, and keeps only each playbook's `[match]` key and load state (the
Repairs page's list). A playbook is parsed again when an incident with its fingerprint opens and dropped when none is open, so an idle node holds no parsed
playbook. `build.py` (Python ≥ 3.11, `0-knowledge/_meta/build.py:8`) lints the same rules with the standard library's `tomllib`: no new Python dependency
(it needs PyYAML only, `:31`). **One vocabulary, one source.** node111 holds one compiled table of `(subject id, reason code)` (§3.4), with a test
that every reason literal the daemon emits is in it; that test writes `0-knowledge/_meta/playbook-vocabulary.toml` (committed; CI fails on drift), and
`build.py` reads that file, so the loader and the lint cannot disagree on a subject or a code. **One fixture directory**, Q9's home beside the Rust
loader's tests and never in the payload: `1-bootstrap/crates/node111/tests/fixtures/playbooks/` (facts in, matched check ids out; accept and reject
cases). `build.py --selftest` and a node111 test run the same cases; `--selftest` resolves the directory from the repository root and prints
`skipped: playbook fixtures not present` when it is absent (a KB tree that stands alone). A node111 test loads every file under
`0-knowledge/playbooks/` and fails on any refusal. Both report under `[L0-L9:evolution]`; the CI job that checks the knowledge base gains
`build.py --selftest` (it runs only `--check` today, `.github/workflows/ci.yml:112-113`). The first release ships playbooks for the reason codes the
table of §3.4 names, `killed_by_os`, `witness_absent` and `apk_outdated` included, each of those three with one `human_step` as its one action (§3.3);
each enters as `candidate`.

## 7. The change engine: typed actions behind an `Action` trait (D35 F6)

**7.1 The trait (amends L6 §12, additive).** The Layer 6 applier is the template: a pending record first, one apply at a time, per action plan → snapshot
→ apply → probe → keep or revert, a detached task (`N/devices/applier.rs:24-37`, `:888-896`, `:909-914`, `:992-1004`, `:1283-1298`). Its device-specific
half (`Action`, `:113-136`; `Snapshot`, `:835-839`; `snapshot`/`restore`, `:1196-1232`) moves behind a trait in `N/heal/engine.rs` (design):
`name()` · `plan(&Facts) -> Plan {diff, preflight, reach: none|may_affect, removes_capability: bool, evicts: u64}` (pure: the applier already checks a copy before any
write, `:24-27`; `evicts` is the count of stored items the change would evict, read from mesh111 before the apply, §23 code seams) · `snapshot() -> Bytes` ·
`apply()` · `probe() -> Result<(), why>` (asserts a capability, never "is it quiet?") · `restore(Bytes)` · `restore_cold(Bytes)`, defined per action as a
write made at daemon start before any child starts and before the store opens (§7.5). The four
device actions are ported byte-for-byte (D29); `kind: device` proposals keep their route, body and results (`API:848`).

**7.2 The v1 vocabulary (closed, normative).**

| Action | Apply | Probe within 30 s (`PROBE_WINDOW`, `N/devices/applier.rs:84-87`) | Undo | Not undone (shown on the card) | Reach |
|---|---|---|---|---|---|
| `service.restart {service_id}` | `restart_service` (`N/dashboard/api_v1/services.rs:113`), which also clears `Degraded` (`N/supervise.rs:682-723`) | in a `repair` or `fix`: the service's published row is out of `error` (the registry's two-tick confirm, `N/services/mod.rs:1459-1468`) and no new exit fact with `requested: false` (§3.2) arrived inside the window; in a `device` proposal: none, Layer 6's one-shot as built (`N/devices/applier.rs:18`, `:100`, D29) | none: "a restart cannot be undone" | the restart and what the child held in memory | none |
| `identify {device_id}` (`device` proposals only) | the consented identify, as built (`N/devices/applier.rs:17`) | none (a one-shot) | none | — | none |
| `rns.interface.upsert`†, `rns.interface.remove`†, `rns.posture.set`, `radio.region.set` | as built (D29) | as built (`N/devices/applier.rs:11-18`) | the snapshot | `rns.posture.set`: the rnsd restart and the links it dropped | none |
| `settings.set {key, value}`, keys `fabric.listen_lan`, `fabric.hub_role`, `fabric.quota.<class>` | through the `PATCH /fabric/settings` code path, under that route's loopback-only gate (§7.3) | the listener's bound state; the quota in `GET /fabric` | the previous value | the items a lowered `fabric.quota.<class>` evicted (`M/store/prune.rs:44-46`) | `may_affect` for the first two |
| `component.enable {id}` · `component.disable {id}` | `enable_component` (`N/dashboard/api_v1/bridge.rs:288`), the function the bridge settings route calls, addressed by component id, with the component's own consent and cached payload (`N/components/mod.rs:12-40`, `:480`) · `stop_and_wait` | `running` and its health · `stopped` | disable · enable (a licence notice that is not current stops it `revert_blocked`, §2.2 item 5) | enable: the installed tree and the recorded acknowledgement (`N/components/mod.rs:17-20`) · disable: — | none |
| `bridge.edge.pause {edge_id}`† · `bridge.edge.set {edge_id, oversize?, honour_ok_to_mqtt?}`† | `PATCH /bridge/edges/{id}` fields (`API:482`) | the edge reads the values | the previous values (resuming a `consent_paused` edge whose consent is not current stops `revert_blocked`) | `bridge.edge.set`: copies already relayed | none |
| `bridge.order.set {order}`† | `interfaces.toml` `[order]` through the spine | the file re-parses with that order | the snapshot | — | none |
| `advise {text}` · `human_step {steps}` | nothing runs | — | — | — | — |

† Human or `device.propose` only in v1: no playbook cause, `repair` proposal or peer fix carries it; it reaches a node through a human's own act or an
agent `device.propose` (Layer 6 §13; the three bridge actions join its closed vocabulary, §23 L6) that a human accepts. The `device` kind keeps
Layer 6's `service.restart` one-shot byte for byte (D29); the probe above is the `repair` and `fix` form.

Never typed: `firmware.flash`. Still refused `unsupported_action`: `meshtastic.set`, `ble.pair`, `ble.forget` (`N/devices/applier.rs:20-22`, `:101-103`).
Out of scope: identity, keys, the update channel, consent records, any consent-gated setting (a carrier, the gateway bit, origin serving). Not in the v1
set: `update.rollback` (§22). Its usable window is about a minute: the daemon confirms a probation after 60 s of green health and asks for its own revert
after 60 s of red (`N/mod.rs:137-180`), and the helper ignores a revert request outside a probation (`NI/update_helper.rs:297-300`). A playbook for an
update incident carries a `human_step` that names Layer 1's own probation revert and the monotonic floor (§24 C1). Two code facts bound the table: a component whose
payload is not cached answers `component_unavailable` (enable never downloads, `N/components/mod.rs:36-40`), so the proposal becomes a `human_step` naming
`sudo 111 install --components <id>` (§24 C3); `fabric.listen_lan` and `fabric.hub_role` are the only settings keys because each has a probe; `lan_enabled`
takes effect only at daemon restart (`N/dashboard/api_v1/settings.rs:14-15`) and is refused (Q6). An adapterless component (Layer 8's D1 + D2 vs I3
ruling: kiwix, opencode) is not a `component.*` target in v1, and an adapter that enables two components needs two proposals (§7.3's one-consent rule).

**7.3 The `repair` proposal (normative).** `111d` composes it for a diagnosed cause under `propose`: kind `repair`, body one JSON object `{"why", "incident",
"playbook", "cause", "actions": [Action]}`, at most 8 actions (`MAX_ACTIONS`, `N/devices/applier.rs:89`), at most 16 KiB (`MAX_BODY`, `:91`), stamped with
this node's id, `task_id: null`. It renders as a card from the parsed JSON, never as Markdown. Accepting it (`POST /api/v1/proposals/{id}/decision`) runs
§7.4; a consent gate answers `409 consent_required` and the human repeats with `accept_warning`. `diagnose` and `propose` proposals stay advisory: accepting one
records "read" (`N/dashboard/api_v1/proposals.rs:698`), as today. Further rules:
1. **Who writes these kinds.** `repair`, `fix` and `human_step` are written only by `111d` through J9: the proposal and a `heal/proposals` record holding
   its actions and their `action_hash` are written in the same J9 call (§4). The ingest of the agent spool refuses
   these three kinds and treats the file as Layer 8 §2.4 rule 4 treats a file it does not ingest: never ingested, never deleted by `111d`, counted. At
   accept, `111d` takes the actions from its own `heal/proposals` record for that proposal id, never from the proposal body, and requires their hash to
   equal the `action_hash` recorded at `proposed`; a mismatch or a missing record answers `refused` and applies nothing. When the proposal's incident
   closes, `111d` marks the proposal withdrawn.
2. **An action inherits the gate of the route it wraps.** Accepting a proposal that holds an action on a loopback-only key (`fabric.listen_lan`,
   `fabric.hub_role`, `fabric.quota.<class>`: `PATCH /fabric/settings` takes the `LoopbackOnly` extractor, `N/dashboard/api_v1/fabric.rs:441`;
   `API:354`) from a non-loopback peer answers `403 loopback_only`, writes nothing and leaves the proposal open. Layer 8 states the same rule for
   `quota.set` (its I32); the decision route itself stays session-or-loopback (`N/dashboard/api_v1/proposals.rs:638`).
3. **A removal is consented, mechanically.** When any action's `plan()` returns `removes_capability` or `evicts > 0`, accept answers
   `409 consent_required` with `action: heal_remove`, a `warning_text` `111d` writes ("This repair turns off <x>." or "This repair removes <n> stored
   items.") and its hash; the human repeats with `accept_warning`. The `why` text is never a gate.
4. **One consent per proposal.** A proposal holds at most one consent-gated action (a component's licence notice, `heal_remove`); `111d` splits a second
   into its own proposal.
5. **A cause matched on log text alone** (§6.2) files `advise` only.

**7.4 The loop (normative).** One apply at a time on a node: the node-wide lock (the applier's `one`, `N/devices/applier.rs:895`, taken at `:993`) is
held from arm to keep, revert, `revert_blocked` or confirm, so through a reach-class confirm wait (§7.6). Every accept and every `apply_unrecorded` (the
human's `PATCH /api/v1/transport/posture`, `N/dashboard/api_v1/transport.rs:377-388`; `N/devices/applier.rs:981`) in that span answers `409 busy`.
`apply_unrecorded` arms the undo too, with `proposal_id: null`, and is reverted at start by the same rules (§7.5). J9's `decision_begin`,
`decision_complete` and `decision_reopen` are the only writers of the decision record. Per action, in order, stopping at the first not kept (D29 (4)):
1. `plan()`; a refusal writes nothing and the proposal stays open.
2. **Arm the undo** before the first effect: `decision_begin` writes the pending decision, `applied: {pending: true}` as today
   (`N/devices/applier.rs:992-1004`); then `state/heal/undo` (J2) = `{schema: 1, phase: "armed", proposal_id|null, attempt_id, action, target, snapshot
   (base64), release, starts, armed: {boot_id, mono_ms}}`, atomic, 0600, fsync, where `release` and `starts` are the run marker's (§3.3). A record that
   does not re-read equal after the fsync refuses the action before its first effect.
3. `apply()`, then `probe()` within its window: kept (step 4) or reverted (step 5).
4. **Keep:** rewrite the file with `phase: "keeping"` (fsync), then `decision_complete(kept)`, the attempt, the ledger and the snapshot record
   (`heal/snapshots`, with the `after_hash` of the state the action left); the file is removed last.
5. **Revert:** rewrite the file with `phase: "restoring"` (fsync), `restore(snapshot)`, then `decision_complete(reverted)` with the reason, the attempt,
   and remove the file. A failed restore is `revert_failed`: the loudest state, a `critical` incident, the human's; the attempt records it and the file
   is removed.
6. **Blocked:** a revert that needs a consent that is not current (§2.2 item 5) rewrites the file with `phase: "blocked"`, records the attempt
   `revert_blocked` (shown in Needs you), completes the decision and releases the lock. The human resolves it on `POST
   /api/v1/repairs/attempts/{id}/restore` (the consent round trip, `409 consent_required` then `accept_warning`, then the restore through this loop; a
   session or loopback, never the agent's bearer) or dismisses it ("leave as is", `POST /api/v1/repairs/attempts/{id}/dismiss`), which removes the file.
   An attempt in `revert_blocked` or `revert_failed` is never reclaimed until it is resolved or dismissed (§4).

Debug builds read `NODE111_HEAL_PAUSE_AT` ∈ `apply`, `keeping`, `decided`, `restoring`, `restored` (the `NODE111_BRIDGE_STUBS` pattern) and stop at
that point, so G20 can cut the loop there; release builds ignore it.

**7.5 Armed at start = revert (amends L6 §12, a correction).** At start, before serving and with the store closed, `111d` reads the run marker (§3.3)
without rewriting it, reconciles `state/heal/undo`, and only then rewrites the marker. By the file's phase:

| Found at start | What `111d` does |
|---|---|
| `armed`, its `release` and `starts` equal to the marker's (the previous run armed it) | rewrite `phase: "restoring"` (fsync), `restore_cold(snapshot)`; on Ok rewrite `phase: "restored"` (fsync) and continue as the `restored` row; an error is `revert_failed` |
| `armed` with another `release` or `starts`, or `restoring` | no restore: `revert_failed`, `critical`, the human's (a stale record, or a restore that may have killed the daemon: never a second restore) |
| `restored` | no restore; once the store opens, fold `reverted (daemon restart)` through `decision_complete`, then remove the file |
| `keeping` | no restore; once the store opens, finish the keep (§7.4 step 4) |
| `blocked` | no restore; the file stays for the human (§7.4 step 6) |
| no file, with a `pending` decision | `interrupted`, through `decision_complete` once the store opens |

`service.restart` arms no undo and records `interrupted`. An `apply_unrecorded` record (`proposal_id: null`) follows the same rows with its attempt and no
decision record. When the store does not open (J2), the cold restore still runs and the file keeps its phase until a later start folds it. This replaces
"a crash mid-apply leaves a visibly half-applied proposal" (`L6:657-658`; `N/devices/applier.rs:35-37`); "never a second apply" stands, and a record
is restored at most once. No clock: the rule is ESP-IDF's `PENDING_VERIFY` state *(doc, R02 F7)*.

**7.6 The reach class (normative).** A `may_affect` action applies, probes, then waits within 90 s on a monotonic clock (LuCI's window, a search snippet,
R02 F8; Q5) for `POST /api/v1/repairs/attempts/{id}/confirm` carrying the same session as the accept or, where loopback is trusted and the accept carried
no session, from the same peer address; the attempt records which. A confirm from anyone else is refused and the window keeps running; when it closes,
the action reverts. The node-wide lock is held through the wait (§7.4). The sleep exists only while one is pending. On Termux, where loopback is not
trusted (`N/dashboard/api_v1/auth.rs:3-10`), the paired phone browser confirms. For the two fabric keys the card says what the confirm proves: "you looked
again", not "members can still reach this hub" (the keys change the fabric's `tcp-lan` listener, not the dashboard's). No v1 action can remove a pairing
or the loopback listener, so `would_lock_out` is not in v1 (§22).

**7.7 After the keep (normative).** No window and no re-probe. The last five kept snapshots are retained by capacity (Q7); a new incident on a target whose
last kept change still has a snapshot offers "Undo <change>" (linked by target, count-free, clock-free). Undo is offered and runs only while the target's
current state hash equals the `after_hash` recorded at keep; otherwise the card reads "changed since, cannot undo" and the route answers
`409 changed_since`. Undo is a human act that runs the same loop: it arms its own undo (§7.4), and its probe asserts that the restored state is in
effect (the file equals the snapshot and the spine applied it), never the original capability probe. The ledger (`heal/ledger`, `{applied, reverted,
unresolved, last_cause, last_evidence_hash}`): after a revert, or after an applied action that is `unresolved`, the same action for the same cause is
not re-proposed until the human asks again from the incident or the evidence hash (§3.5) changes. An applied action is `unresolved` when a new unrequested event
bumps its incident's `count` after the keep. While an apply holds the lock, an accept answers `409 busy` (§7.4).

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
`[install] prefetch = false` (the D29 (1) pattern), its licence notice acknowledged at enable (D26 (5)). Today a declaration requires `adapter` and a
`[run]` table with a program and a health rule (`N/components/manifest.rs:243`, `:248`), and nothing says hub-only; opencode is declared in Layer 8's
`adapter`-optional form (an adapterless component, its state directory `state/opencode/` unused: its `HOME` and XDG directories are in the task's work
directory, §8.1), plus a `[run]`-less tool form (no supervised child and no health: `111-agent` runs it per task) and a `requires.hub` key, the two
additions Layer 9 asks of Layer 6 §2.1 (§23 L6). `sudo 111 install --components opencode` fetches its pinned payload and the release's source tarball,
the `[source]` pin Layer 8 §11.6 adds to the manifest, into the root-owned `cache/packages/opencode/` (§4's path table): one pin, two consumers (§24 D4).
Layer 9 adds no writer to Layer 8's object tree: `state/origin/blob/` holds the tarball only where Layer 8's updater put it, on a hub with
`origin.serve` (L8 §11.6). Enabling needs `hub_role`. When the component is enabled, `111d` passes the agent `AGENT111_OPENCODE_PATH` naming the
installed binary, as it passes `AGENT111_KB_DIR` (`N/intelligence.rs:220`); the agent already reads that variable or `--opencode-path`
(`A/src/config.rs:169-176`, `:303`) and otherwise runs `opencode` from `PATH` (`:100`), and `agent_spec` passes none today (§23 L3). The KB entry stays `candidate` (`0-knowledge/projects/opencode.md:4`) until its smoke log.

**8.3 The draft (normative).** On a hub with `heal.code_drafts` on and the component enabled, an `escalate {task: "patch"}` cause or a human's request
submits a `patch` task, and only when a source tarball whose sha256 equals the running release's `[source]` pin is present, in
`cache/packages/opencode/` or in `state/origin/blob/<sha256>` where Layer 8's updater placed it: after an update the cached copy is the previous
release's and fails this check. Otherwise no task is submitted, the incident task route answers `409 component_unavailable` (§18.1), and the incident
gets one `human_step` naming `sudo 111 install --components opencode`. The agent unpacks the source tarball read-only into `agent/work/<task>/src/` (the work dir exists, `A/src/tasks.rs:397`), runs
opencode under §8.1 with that tree as its cwd, and takes its unified diff as text. Edit is denied, so opencode can only read and answer. The `patch`
prompt's inputs are stated as §5.2 states `diagnose`'s: the incident inside a labelled untrusted block and the human's note, nothing from the journal;
today's `patch` prompt carries the note and `gather_context`'s daemon health, journal tail and KB refs (`A/src/tasks.rs:360-390`, `:403-412`), and that
context is switched off for a `patch` task. A file outside the work directory (a key under `state/`) is never in a model request or a diff (G24).

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
   author node, incident id, the diagnosis as a quoted block, the verify log `111d` recorded, what was not verified, the tools and versions. Both name the
   author node by a short label by default (the node's alias); the full NodeId appears only when the human ticks "include the node id", because
   publishing it links the owner's GitHub account to the node's permanent identity. The download screen shows both files before they are saved.
2. **The agent never adds `Signed-off-by:`.** The PR template gains a DCO 1.1 sign-off line the human fills; no CI check in v1; D22's inbound = outbound
   stands (Q2). The licence grant comes from the human's review and sign-off (R03 finding 9; not legal advice).
3. **The owner runs** `git am` and `gh pr create` (MIT, on their own machine). A phone gets a prefilled GitHub issue URL for a kept typed change and a
   `kb` draft (R03 §4.2 B3), percent-encoded and built from typed fields only. One PR per fix id, never retried automatically. No token on any device in v1.
4. **The renderer rule (normative).** Model and peer text never shapes the artefact's structure. The subject is one line: control characters stripped,
   at most 72 B. A body line starting with `From ` is written `>From `. A trailer-shaped line (`Token: value`, `Signed-off-by:` included) in agent or
   peer text is indented, so it is never a trailer. Untrusted text in `PR.md` sits in a fenced block whose fence is longer than any backtick run inside
   it. The issue URL carries typed fields only (fingerprint, cause id, action names, release), percent-encoded; no free text, mention, link or closing
   keyword from a model or a peer reaches it. `git mailsplit` on an exported mbox yields exactly one patch (G24).

## 10. Signing: `111d` signs what leaves the node (D35 F9)

**10.1 Who signs.** The node key, through `111d`, the identity holder: `Keyset::sign_with_tag` (`ID/keyset.rs:475`) behind node111's `sign(tag, msg)`
(`N/fabric/sealer.rs:432-433`). The agent holds zero key material (`L2:234`; "No key material ever reaches it", `N/intelligence.rs:10-12`). `111d` builds the
signed bytes from records it holds and never signs bytes the agent hands it: no signing oracle.

**10.2 The two tags (normative; registered in L2 §13.3 A19, Layer 8 §21 L2's block).**

| Tag | Signed input | Verified with |
|---|---|---|
| `111/fix/v1` | Ed25519 by the author's node key over `tag ‖ body_len (u32 LE) ‖ body ‖ attachment_hash (32)`; `attachment_hash` = BLAKE3-256 of the attachment, or 32 zero bytes; v1 carries no attachment, so it is always 32 zero bytes (§11.1) | `verify_strict_tagged` (`ID/keyset.rs:627`) under the author NodeId the body names |
| `111/fix-vouch/v1` | Ed25519 by the voucher's node key over `tag ‖ voucher (32) ‖ fix_id (32) ‖ kind (1) ‖ voucher_seq (u64 LE)`; `kind` 1 `vouched`, 2 `reproduced`, 3 `accepted`, 4 `reverted`; kinds 1 and 2 are reserved and never sent in v1 | as above, under `voucher` |

`fix_id` = BLAKE3-256 of the body bytes. Proposals, incidents and audit records are never signed: L2 §6.1 keeps identity keys to "announces, proofs, and
control artifacts only — never routine traffic" (`L2:91`), and Layer 9 signs nothing that stays on the node (J5). `AGENT_OP` payloads need no new tag:
`111/msg/v1` signs them (`L2:358`). Known-answer vectors: `9-evolution/verify/vectors/fix-v1.json` (id111 gains no code, the A14 precedent).

## 11. Sharing proven fixes between nodes (D35 F10)

**11.1 The artefact (normative).** `fix/v1` body: UTF-8 JSON, signed as exact bytes, never re-serialised, at most 4,096 B: `{schema: 1, author, author_seq,
epoch|null, class: "typed"|"code"|"kb", applies_to: {category, layers_run, release, components}, incident: {fingerprint, diagnosis ≤512 B}, actions: [Action]
(typed only), evidence: {probe, result, before_hash, after_hash, reverted: false}, not_verified: [string], attachment: {kind: "diff", bytes, blake3}|null}`.
`author_seq` is minted from the Layer 2 announce counter, as Layer 8's catalogue `seq` is (L8 D6), so a words-only restore cannot restart it (a replay is
not increasing); no wall clock. **v1 carries `class: "typed"` and `attachment: null` only:** `code` and `kb` fixes travel as drafts and reach other nodes
through the human's pull request (§9); the signed input keeps `attachment_hash` (32 zero bytes, §10.2) so a later release adds attachments under the same
tag (§22). The body parses strictly: an unknown field or a duplicate key refuses it; an unknown `schema` is parked "needs a later release";
`applies_to.release` matches by equality in v1. For the release that adds attachments the rule is fixed now: the attachment's BLAKE3 must equal both the
body's `attachment.blake3` and the signed `attachment_hash`, else the fix is dropped and counted.

**11.2 The wire (normative).** A fix rides one sealed `PATCH` (0x13) to one recipient. `PATCH` is SIGNED-mandatory and a public class (`L5:141-142`;
`requires_sig` and `is_public_class`, `M/envelope.rs:595-612`): the envelope signature binds no recipient, and the fix's own signature makes it transferable.

| `PATCH` payload | Bytes |
|---|---|
| type | 1: `0x01` fix, `0x02` vouch |
| fix: `body_len` (u32 LE) ‖ body ‖ `att_len` (u32 LE) ‖ attachment ‖ `sig` | 4 + ≤ 4,096 + 4 + attachment + 64; in v1 `att_len` = 0 and no attachment |
| vouch: `fix_id` ‖ `kind` ‖ `voucher_seq` (u64 LE) ‖ `sig` (the voucher is the frame's authenticated sender) | 32 + 1 + 8 + 64 = 105 |

**Three bounds (normative).** (1) One SIGNED `tcp-lan` frame (`MAX_PACKET_LEN` 65,535 B, `M/envelope.rs:81`; `payload_budget`, `:1395-1413`; 65,445 B
unsigned in its test, `:2198`, so about 65,381 B SIGNED, arithmetic): the sender refuses a fix only above this bound; v1 sends no attachment, so the
brief's 64 KiB attachment cap is moot (§24 D5). (2) 16 Tier 1 fragments of the path's profile, the sum of the per-fragment budgets (§12.2): on a 233 B
bearer about 1.2 KB (R07g2 F4); never the 160 B floor. In v1 a fix travels over `tcp-lan` only, the one fabric link the daemon builds (§12.2), and
Layer 7's custody and a carrier move it between islands; bound (2) is written for the links later layers add and is proven on mesh111's `lossy` link.
(3) 2,937 B sealed through a gateway carrier (L7 §6.2). Across islands a fix rides Layer 7's
custody hand-over like any sealed item (L7 §5), and carrier bytes count in Layer 7's metered cap (L7 §12). The send screen shows the sealed size and
warns "too large for a carrier" above 2,937 B. A carrier's refusal stays Layer 7's: the hub answers `REJECT_UNKNOWN_DEST` and counts it
(`forward_too_large`, L7 §5.2, §5.9), and the sender's fix reads "not delivered"; the honest custody status is Layer 7's v1.1 item.

**11.3 Sending (normative).** Only a human sends: "Send this fix" on a kept change, `POST /api/v1/repairs/fixes/{id}/send {to}`, where `to` is the home hub
or a pinned contact as `heal.fixes.send` allows; off by default. The consent `fix_send` (§17) shows the exact bytes that would leave; the send screen
shows the sealed size and the carrier warning of §11.2. `111d` builds the body
from the incident, the attempt and the verify log, signs it (§10) and sends it through `Fabric::send_op` (§23 L5).

**11.4 Receiving (normative), all before a human sees a button.**
1. The fabric hands the `PATCH` body to Layer 9's consumer (L8 §12.3), never to a message row; `RECEIPT_STRONG` returns the hash (`M/envelope.rs:113`).
   The consumer receives the frame's authenticated sender NodeId, whether the link is attributed, the `msg_id` and the body (a request to Layer 8, §23 L8).
2. Parse the body strictly (§11.1) and verify the signature; a bad one is dropped and counted.
3. The frame's authenticated sender must equal the body's `author`; anything else is dropped and counted, whoever signed it. The author must be in the
   set `heal.fixes.receive` names (§17.1): `home_hub` = the enrolled `home_hub` NodeId; `paired_hubs` = the home hub plus the Layer 7 paired hubs (offered
   on a hub only); `named_contacts` = the home hub plus each NodeId in `heal.fixes.senders`; a fix from an author outside the set is dropped and counted and leaves no
record, so bytes from strangers never take room from incidents in the `heal` class (no ring for unknown authors in v1, §22).
   `author_seq` must be above the author's `high_water` record (§4), else it is a replay, dropped and counted; an accepted fix raises `high_water`. A
   reclaim never touches `high_water`.
4. A body from an author in the set with another class than `typed`, or with an attachment, is parked "needs a later release": never applied, never
   rendered as a diff, no button.
5. `applies_to` is checked against this node's category, every layer it runs and its release (by equality); a mismatch reads "does not apply here".
6. **The peer subset (normative).** Each action of the fix is matched against this table; nothing else from a peer reaches a button:

   | From a peer | Shown as |
   |---|---|
   | `service.restart` | an action, only when its subject is the open incident's own subject (step 7); the peer's target field is never used, and a mismatch drops the action to "their words" |
   | `advise` | this node's own playbook text for the same `cause_id`, with a button; the peer's text is "their words" |
   | an action byte-equal to this node's own playbook action for the same `cause_id` | that action, with its targets bound by `@subject` here (§6.2), never taken from the peer |
   | anything else, `human_step` included | "their words, not checked", preformatted, with no accept button |

   A `human_step` or `advise` shown with a button is always this node's own playbook text, selected by `cause_id`.
7. **The incident match.** A fix that passed steps 2-6 files a `fix` proposal only while this node has an open incident with the same fingerprint. With
   none, it is stored `undecided` inside the 16-per-author queue and shown as "does not apply here yet"; when an incident with its fingerprint opens,
   `111d` lists the stored fixes once for that fingerprint (one `list` at incident open, no timer) and only then files the proposal. Accepting it runs
   §7.4. A `fix` proposal is never accepted by one tap: its accept button lives only on the full review screen (the card's typed fields, the peer
   subset of step 6 and the fixed vouch text), never on a list row or a notification. The `fix` card says in fixed text: "Accepting tells <alias> that you accepted, and whether it was reverted." One vouch leaves per outcome
   (§2.2 item 10): kind 3 `accepted` at the accept; kind 4 `reverted` once, when this fix's own loop reverts it (a failed probe, a closed confirm
   window, armed at start); a later human Undo sends nothing.
8. **Vouches received.** A vouch is accepted only from a node this node sent that `fix_id` to (its `sent` record), as the frame's authenticated sender;
   the highest `voucher_seq` per (voucher, fix) wins; anything else is dropped and counted. `voucher_seq` is minted from the Layer 2 announce counter, as
   `author_seq` is.

The receiving agent never reads or summarises foreign fix text: `111d` renders the card from typed fields and the prose preformatted (OB conflict 8).
Incoming fixes queue at most 16 per author, oldest dropped and counted (Q18; the fabric's quota pattern).

**11.5 Maturity (normative).** In v1 the ladder is `local` → `accepted_by_n` (n owners accepted it and none reverted it within the next K incidents of
its fingerprint: a count, not a period; K = 3, the drafter's number for the panel; `n` is the count shown, with no threshold in v1) → `upstream_merged` →
`in_release`. `reproduced` (a second node of the same category re-ran the probes) is reserved, with vouch kinds 1 and 2, for a later release (§22). Only
other parties' signed vouches move a fix past `local`.

## 12. Delegation for Cat 2A and 3: the home-hub `AGENT_OP` pair (D35 F11)

**12.1 The op set (normative, closed).** `AGENT_OP` (0x10) is a reserved byte with no schema, sender or consumer today (R07g2 F1); SIGNED-mandatory,
interactive (`L5:175`; `M/envelope.rs:617`). v1 gives it three ops. Payload: `schema (1) = 0x01 ‖ op (1) ‖ a deterministic CBOR map (RFC 8949 §4.2.1)` with
unsigned integer keys; an old node drops an unknown `schema`. The map is encoded and decoded with `ciborium`, already linked
(`1-bootstrap/Cargo.lock:285`); a map that is not in the deterministic encoding (keys out of order, a duplicate key, an indefinite length, a
non-shortest integer) is refused, dropped and counted, never re-encoded.

| Op | Direction | Map keys (each capped) | Size |
|---|---|---|---|
| `0x01` `diagnose` | member → home hub | 0 `req_id` (bstr, 8 bytes from the OS random source) · 1 fingerprint `[layer, subject ≤24, reason ≤24]` · 2 severity · 3 count · 4 release (≤10) · 5 target triple (≤32) · 6 category (≤3) · 7 checks `[[check_id ≤16, bool]]` ≤ 16 · 8 ruled-out cause ids ≤ 8 · 9 matched literals ≤ 3 × ≤ 64 B (the literal a `log_contains` check matched, text 111 wrote; never a log line) | ≈ 450-600 B (estimate) |
| `0x02` `diagnosis` | hub → member | 0 `req_id` · 1 hub release · 2 `cause_id` (≤24, or `unknown`) · 3 `checks_satisfied` (per-mille) · 4 `agreement` (0-3) · 5 actions ≤ 3 × ≤ 80 B `{0 action, 1 target, 2 args}` (the member reads them only through §11.4 step 6's peer subset; field 1 is never used) · 6 narrative ≤ 256 B | ≈ 400-600 B (estimate) |
| `0x03` `refuse` | hub → member | 0 `req_id` · 1 reason: 1 not serving, 2 not enrolled, 3 busy (retried on the next link-attribution event, §12.2), 4 nothing to say, 5 unsupported schema, or check ids not all in the hub's playbook, 6 `request_changed` (never retried) | ≤ 16 B |

The narrative is the hub playbook's one-line cause text, never model text. On reason 5 the member's card reads "Your hub cannot read this request: it runs
another release of this check."; on reason 6 the member files nothing and sends no retry for that `req_id`. `111d` builds the request from the incident record; nothing from a bridge
adapter or presence log rides in it (Q13). Off the node goes only text 111 wrote: the row's reason, the exit facts, the check results and the matched
literals of key 9 (§3.5).

**12.2 The size gate and the queue (normative).** Before sealing, `send_op` checks that `len` fits the sum of the per-fragment budgets of 16 fragments
for the path's profile: `payload_budget` takes five arguments (the bearer payload, the shim, the sequence, the fragment position, the signed flag;
`M/envelope.rs:1395-1401`), and a fragment's budget is smaller by its `frag` varints; otherwise the request stays `queued` with the reason "needs a
longer link"; on the 160 B floor a SIGNED fragment carries 3-5 B (R07g2 F4). **v1's one link.** In v1 a delegation, like a fix, travels over
`tcp-lan` only: it is the one fabric link the daemon builds (`impl Link for TcpLan`, `M/link/tcp_lan.rs:714`); `kiss-modem`, `meshtastic-client` and
`rns-bridge` are enum seams (`M/link/mod.rs:231-242`), and the 160 B profile exists only as mesh111's `lossy` test link (`M/link/lossy.rs:241`), which
node111 never constructs. The size gate is written for the links later layers add and is proven on that `lossy` link in mesh111's
`tests/send_op.rs` (G23 (5)). **Path:**
`send_op` gains `direct_only`: an `AGENT_OP` is sent only on a link attributed to the home hub and is never put in custody or a mailbox (an op held at
another hub would be dropped by the home hub, §12.3); the member keeps the request in its `open` delegation record. **Re-send, no timer:** a request with
no reply is sent again, same `req_id`, on the next link-attribution event to the home hub; a card update that adds bit 6 while an undiagnosed incident
is open sends the request; a `refuse` 3 (`busy`) is retried on that same event. Nothing polls (L5's link events; `send`, `M/fabric.rs:1279`). It rides
stream 0, as every v1 packet does: the inner layout has no stream field (`M/ids.rs:19-22`; the open path sets `STREAM_1TO1`, `M/envelope.rs:1373`, `:1383`), so
OB F13 item 12's `STREAM_AGENT = 1` is not carried in v1 (§24 D1).

**12.3 Trust (normative).** The hub accepts `diagnose` only from an `enrolled` member on an attributed connection, at most one open request per member,
and only while `heal.serve_members` is on; anything else is dropped and counted, or answered `refuse`. A hub queues at most 4 delegated requests in total
(a capacity, the lead's default; measured at build); beyond it it answers `refuse` 3. The queue is memory only: a restart loses it and the member's
re-send (§12.2) restores it. The member accepts `diagnosis` or `refuse` only from
its `home_hub`, on an attributed connection, for an open `req_id`. A contact is never a delegate. A hub stores no member's op for others (a put whose
`dest` is the hub itself is refused, L7 §5).

**12.4 The hub side (normative).** `111d` serialises one delegated diagnosis at a time over loopback (§5.2, with the member's incident as the untrusted
`incident` and its own health and journal switched off) on the llama-server whose `model` row is `live` (it starts, stops and reloads no model), and
answers from T0 alone when no model runs and exactly one cause of its playbook for that fingerprint is satisfied by the member's checks; else `refuse`
reason 4. A request whose check ids are not all in the hub's playbook answers `refuse` 5. A delegated model run starts only when no local task or chat
turn waits. The delegated task's result is folded into its delegation record, never into `agent/proposals` (§5.2). The hub keeps one stored-reply record per enrolled member
(the latest; reclaimable, §4),
`{member, req_id, request hash, fingerprint, checks hash, reply}` and nothing else of the request: the same `req_id` with the same request hash gets the
stored reply; the same `req_id` with another hash gets `refuse` reason 6 `request_changed`, so a member whose counter restarted never reads a reply meant
for another incident; a new request whose fingerprint and checks hash equal the stored ones gets the stored reply with no model call.

**12.5 The member side (normative).** Under `heal.ask_hub` (off by default; consent `heal_delegate` lists the exact request fields), `111d` sends the request
when an incident opens with no T0 cause and the home hub's card carries `ROLE_HEAL` (§23 L7), and again as §12.2's re-send says; a start fact never
delegates by itself (§3.3 rule 5); off, nothing is composed or sent. The reply becomes a
proposal the member's human accepts, never auto-applied, its actions shown through §11.4 step 6's peer subset (a `service.restart` only for the open
incident's own subject; anything else is "their words, not checked"), provenance "from your hub <alias>, not
checked by this phone". Closing the incident cancels the request; a reply for a closed `req_id` is dropped.

**12.6 The APK refresh (local, not delegated; `L1:185`).** No install receipt exists on a phone: `Receipt::write` runs only in the Linux installer
(`NI/install/mod.rs:191`, `:251`), and the phone flow keeps a host-side state and never fills `PhoneReceipt` (`NI/install/receipt.rs:10-19`, constructed
nowhere). So after the APK step (`NI/install/phone/mod.rs:177-181`; the three pins, `NI/install/phone/apk.rs:10-15`) the host provisioner pushes
`state/apk.json` into the Termux root with `runas::install_file` (`NI/install/phone/runas.rs:128`): `{"schema": 1, "apk": {<package>: <sha256>}}`, 0600
(§4's path table; a new write step in Layer 1's phone flow, §23 L1 item 4). After each verified manifest check, `111d` compares the manifest's `apk.*`
pins (`NI/manifest.rs:639`) with `state/apk.json`; a difference opens `(1, 111d, apk_outdated)`, whose shipped playbook's one action is the
`human_step` "A newer Termux add-on is pinned", with the steps "On your computer, connected to this phone, run `111 install --phone` again"; it closes
when the file matches the pins. An absent file opens nothing and files no `human_step` (a phone provisioned before this release; the next
`111 install --phone` writes it). It never implies an automatic install.

## 13. Evolution: the agent, the KB and the bridging logic (D35 F12)

1. **Agent data by release only; the seam is built now.** The chat preamble and the task prompts move out of Rust constants (`A/src/chat.rs:37`,
   `A/src/tasks.rs:326-333`, `:401-412`) into `current/share/agent/prompts/{chat,diagnose,propose,patch}.md` and `share/agent/schemas/diagnosis.json`, pinned in
   the payload manifest and verified at daemon start like the KB (`N/kb.rs:32-34`); a mismatch marks the `agent` row `error` with `prompts_invalid` and
   `111d` does not start the agent. The agent finds `share/agent/` through `AGENT111_DATA_DIR`, which `111d` sets as it sets `AGENT111_KB_DIR`
   (`N/intelligence.rs:220`); a test run sets it to the repository's copy; there is no compiled-in fallback. The contract and the fixed tool set (D10)
   do not move. The agent's 5.3 MB idle RSS must not move.
2. **Change gate.** A prompt, playbook, tool or model-pin change is a release that passed a frozen must-not-break suite in CI: a stub model server replaying
   recorded outputs; schema, refusal and injection cases; fixtures from the G1-G7 namespaces. **The suite (normative).** Its cases live under
   `3-intelligence/crates/agent111/tests/evolution/cases/`, one JSON file each, `{id, class, request_hash, recorded_reply, expect}`: `class` is
   `schema`, `refusal` or `injection`; `request_hash` is the BLAKE3 of the request the agent builds for the case; `expect` is the outcome the agent must
   reach on `recorded_reply`. The gate is every case's outcome equal to `expect`. A prompt, schema or tool change that alters a case's request makes its
   `request_hash` differ, and that case fails until it is re-recorded on the real model; the re-recording is the release's eval report (Q20). The first
   release holds 50 cases (R04 §1's figure; default). The score is evidence, never the gate: about 50 cases give ±13 points at 70 % (R04 §1). Model-pin
   swaps are always a project release with an eval report (Q20).
3. **No node-local overlay in v1** (OB conflict 4): a prompt has no probe that proves it safe.
4. **The KB grows by export.** A `kb_draft` is never automatic and has one producer: a human's "Draft a KB note" on an incident starts a `propose` task
   with `kb_draft: true` through the incident task route (mode `kb_draft`, §18.1); no new tool. `111-agent` code writes the frontmatter and the six
   headings from typed fields and the model fills the prose. The entry is `candidate`; its `### Alternatives considered` is left marked for the human;
   it carries no `### Smoke test` (`L0:84` asks a human-attested one); the incident's procedure goes under `## Details`. `kb_draft` proposals carry
   `{target, body, not_verified, incidents}`: `target` matches Layer 0's filename rules N01-N06 and a category folder, and the body is within B09's budget;
   `111d` re-validates `target` at accept, before the mbox names a path. Accepting one creates a `heal/drafts` record of class `kb` exported through §9's
   mbox route (Q19), and its `PR.md` says the lint did not run on the node. The node's KB is untouched; the agent never edits `status`; it never reads
   its own drafts back as KB truth.
5. **Bridging grows by typed actions.** `bridge.edge.pause`, `bridge.edge.set` and `bridge.order.set` (§7.2), human or `device.propose` only in v1: no
   playbook, `repair` proposal or peer fix carries them, and an agent's `device.propose` that a human accepts is their one path from a model; the human
   accepts each, so "it cannot do so" (`L6:487-488`) stays literally true. No filter language (README §5).
6. **Later, human-started hub tasks:** replay of the owner's own incident corpus (an opt-in sealed export), GEPA (MIT) proposal runs, Inspect AI (MIT) as an
   optional runner. Never a node loop: 50 cases × 3 × 400 tokens is 60,000 tokens, 3.3 h at an assumed 5 tok/s on a Pi (R04 §4, arithmetic).

## 14. The Repairs page, notifications and the honest words (OB §3)

**14.1 The page (normative).** Settings → Layer 9, `/settings/l9`, "Repairs", its own lazy chunk ≤ 64 KB (D21; the `l6` precedent at
`UI/src/views/settings/index.tsx:13-14` in the committed tree). Sections: **verdict** (one sentence: "Nothing needs repair." · "2 repairs are waiting
for you." · "A change is waiting for your confirmation") · **Needs you** (open proposals, `revert_failed`, `revert_blocked` with its Restore and "Leave as is", `refused`) · **Timeline** (one row per incident:
fingerprint, first and last seen, count, state chip, the actor of the last step, expandable evidence, checks, attempts, verify log, outcome) · **Changes**
(kept changes with Undo while a snapshot is kept and the target is unchanged since the keep; "changed since, cannot undo" or "A restart cannot be
undone" otherwise) · **Fixes** (received, with "does not apply here yet" and "needs a later release" named as such; sent; drafts, with Download mbox and
Download PR.md; a received fix is accepted only from its full review screen, never by one tap on a row, §11.4 step 7) · **Playbooks** (advanced:
loaded, refused with "needs release X") · **Audit** (advanced) · the **consent block** (§17). Event-fed (`incident`, `proposal`): no UI timer and no
ticking countdown.

**The lazy budget (normative).** After Layer 7's row U2 the lazy bucket holds 238.9 of D21's 256 KB (`7-relay/TASKS.md:91` at `61ae26d`; the `l7` chunk is 11.9 KB
gzip), and Layer 8 takes 12 KB gzip of the 17.1 KB left for its chunks, adapter and card (L8 §15.4). Layer 9's allowance is what Layer 8 leaves:
5.1 KB gzip for the Repairs page, its proposal cards and its adapter together (default; measured at build), restated against the bucket at the lock
head. The advanced sections (Playbooks, Audit, Fixes) are their own chunk where that saves bytes. When the measured chunks exceed the allowance, the
page ships what fits, in this order: the incident list (Timeline), Needs you, and the accept, undo and dismiss controls; the sections that do not fit
are held back from the v1 UI and stay reachable through `111 heal` and the API (§18); the figure and the overage are recorded in
`9-evolution/TASKS.md`, and raising D21's lazy total is the first item of §24 "Open for the owner". The UI check asserts `pnpm budget` (G20 (16)); the
budget gate is never left red and never loosened by a build.

**14.2 States (closed).** The display phases: `open`, `diagnosing`, `proposed`, `applying`, `kept`, `resolved`, `reverted`, `revert_failed`,
`dismissed`, `refused`, `unable_to_diagnose`, `revert_blocked` (§7.4 step 6), `awaiting_confirm` (§7.6): OB §3's eleven plus two (§24 D43). Each is
computed from §4.2's store states, never stored: an open incident reads `diagnosing` while its T1 task or delegation runs, `unable_to_diagnose` when
no tier found a cause, `proposed` while a proposal for it is undecided, `applying` or `awaiting_confirm` while its attempt is `armed` or
`awaiting_confirm`, else `open`; an attempt reads its own state (`kept`, `reverted`, `revert_failed`, `revert_blocked`; an `interrupted` one leaves
its incident `open`); a closed incident reads `resolved`, or `dismissed` when the human dismissed it; a
proposal the human refused reads `refused`. Three severities (§3.5). A dismissal survives restarts and re-opens on a changed cause (Home Assistant's issue registry, Apache-2.0,
a reference only).

**14.3 Simple mode (Q3).** The Overview's Needs-you block (`L4:212-214`) gains "N repairs waiting" and "a change was reverted"; the full page is advanced.
The agent view's line "Applying accepted proposals arrives with Layer 9 (Repairs)" (`UI/src/views/agent/proposals.tsx:137`, committed tree) becomes true and
reads "Accepted repairs are applied with an automatic undo." The layer card's "tests fixes in a sandbox" (`UI/src/lib/layers.ts:75`) reads "The node watches
every layer, explains failures and proposes fixes with an automatic undo; code changes are drafts a person sends."

**14.4 Phone notifications.** `heal.notify` defaults to `off`. `termux-notification` is in the `termux-api` deb, which the pinned closure does not carry
(`L1:68`; the manifest test's minimum, `1-bootstrap/crates/node111/tests/manifest_parse.rs:74-83`); only the Termux:API app is pinned
(`NI/install/phone/apk.rs:14`). A runtime dependency in a phone's required configuration is the owner's word (AGENTS.md), so the default stays `off`
until that deb is in the closure (§24 Open (4)). When on, `111d` posts one content-free notification, "111: your node needs you", when the Needs-you set
goes from empty to non-empty: one transient process then, none otherwise, however many items join the set. On Linux, and wherever the helper is absent,
the setting does nothing and its text says so.

**14.5 Honest words (normative).** "probe passed" names the probe; "drafted and applies cleanly, not built"; "validated" always names its validator; a
peer's verify log is "their test, not this node's"; anything from another node, a bridge or an RF origin is rendered preformatted and never instructs the agent.

## 15. Sequencing and the first Layer 9 release (D35 F14)

v1, in this order (each a board row with its gate). Each of items 3-8 lands the routes its gate calls, their `openapi.yaml` rows and their pin-list
entries (§18.2) with it: item 3 `GET /api/v1/repairs`, the incident routes and the `incident` event; item 5 the incident task route and the
playbooks route; item 6 the decision route's changes, the confirm, restore, dismiss and undo routes and the changes list; item 7 the settings, fixes
and send routes; item 8 the drafts routes. Item 10 is the Repairs page and the two copy lines of §14.3 only.
1. The containment correction (§8.1) and atomic proposal and task writes (`A/src/store.rs:139`, `:145`, against `:1-3`; Layer 8 §2.4 rule 1 does the same).
2. The engine's start-up reconciliation (§7.5) on the Layer 6 applier, before anything else uses the engine.
3. The supervisor's exit facts (§3.2), the witness (§3.3), the observer and the incident record (§3.4-§3.5).
4. State per the joint contract (§4): after Layer 8's state service (L8 §14 item 2); else J7.
5. Grammar v1, the first playbooks, T0 and T1 (§5, §6).
6. The `Action` trait and the v1 vocabulary (§7); F1's grant format pinned, off (§2.3).
7. The two tags (§10); the op pair and the hub's serving switch (§12); typed fixes to the home hub and pinned contacts (§11).
8. The mbox and `PR.md` (§9); code drafts on opted-in hubs (§8).
9. The agent data seam, `kb_draft`, the three bridge actions, the CI suite (§13).
10. The Repairs page and the two copy lines (§14); the APK `human_step` (§12.6).

**v1.1:** the auto-apply rung if the owner wants it · OB F7 C · the fix shelf with Layer 8 · vouched authors · a hub token · hub-to-hub delegation · a
node overlay · hub eval replay · the opt-in probe · delegation certs · Tier 2 fixes and LoRa fix cards. Every gate stays netns- or stub-verified.

## 16. The sixteen amendments to locked layers (D35 F13)

Each is additive and owner-visible; with Layer 9's switches off and no `heal` traffic, wire bytes and contracts are unchanged. One removal and three
corrections are marked. The exact text per layer is §23. Rows 17 onward are amendments the panel's findings need beyond D35's sixteen; each is also
listed for the owner in §24 "Deviations". A row whose text the findings widen says so in §23.

| # | Layer | Amendment | Kind | Here |
|---|---|---|---|---|
| 1 | L0 | `playbook` category, folder, N06, the H3 vocabulary, the TOML lint; widened: the matrix's `playbook` column, the playbook gate sentence, the scope line, the root index and `0-knowledge/AGENTS.md`, `_meta/playbook-vocabulary.toml`, the shared fixture directory in `--selftest` | additive | §6 |
| 2 | L1 | `ExecStopPost=` and the Termux inbox line; the cron entry; widened: the witness's delivery sentence, `heal-witness` exits 0, the removal list, `cronie` leaves the Termux closure | additive; **removal** | §3.3 |
| 3 | L1 | the hub-only source asset: Layer 8's `[source]` pin, consumed | additive | §8.2 |
| 4 | L1 | changed: no `receipt.apk`; the provisioner pushes `state/apk.json` after the APK step of the phone flow | additive | §12.6 |
| 5 | L1 | the probation wording (`L1:183`) | **correction** | §24 |
| 6 | L2 | `111/fix/v1`, `111/fix-vouch/v1` | additive | §10 |
| 7 | L3 | the `incident` input; `incident.read`; `service.log`; widened: `evidence` constrained to the incident's ids with no `quotes`, `TaskRequest.kb_draft`, `TaskRequest.narrative`, `AGENT111_DATA_DIR` with no fallback, the reason code `prompts_invalid` | additive | §5.2, §13, §18 |
| 8 | L3 | the opencode deny block and `HOME` | **correction** | §8.1 |
| 9 | L3 | prompts as release files | additive | §13 |
| 10 | L4 | `Proposal.kind` +4, `applied`, `incident`, the repairs routes, `/settings/l9`; widened: `repair`, `fix` and `human_step` are kinds the agent's writer never writes, `ConsentRequired.action` gains `heal_remove`, the `device` vocabulary text and card gain the three bridge actions, the drafts list route, the shapes of §18.4, `Problem.code` `heal_full` | additive | §7.3, §14, §18 |
| 11 | L5 | `Fabric::send_op` for `AgentOp` and `Patch`, size gate, commit-before-seal; widened: `direct_only`, the gate as the sum of the per-fragment budgets, proven on mesh111's `lossy` link (v1 sends over `tcp-lan` only) | additive | §23 L5 |
| 12 | L5 | `STREAM_AGENT = 1` | reserved in v1 (§24 D1) | §12.2 |
| 13 | L5 | a consumer for 0x10 and 0x13 instead of a chat row (built once with L8 §12.3) | additive | §11.4 |
| 14 | L5 | the Tier sentence for `PATCH` | additive | §23 L5 |
| 15 | L6 | the trait, persisted undo, reach class, vocabulary; armed at start = revert; widened: one apply at a time node-wide, the lock held from arm to keep, revert or confirm, `apply_unrecorded` armed with `proposal_id: null`; `restore_cold(Bytes)` and the undo file's phases; the three bridge actions join `device.propose`'s closed vocabulary | additive; **correction** | §7 |
| 16 | L7 | role bit 6 `ROLE_HEAL` | additive | §23 L7 |
| 17 | L4 | `POST /api/v1/proposals/{id}/decision` answers `403 loopback_only` to a non-loopback accept of a proposal holding an action on a loopback-only key | additive (**new**) | §7.3 |
| 18 | L4 | `PATCH /api/v1/transport/posture` answers `409 busy` while an undo is armed | additive (**new**) | §7.4 |
| 19 | L1 | the CLI verb `111 heal settings [<key> <value>]` over the loopback settings route | additive (**new**) | §17.1 |
| 20 | L6 | §2.1 rule 1, beside Layer 8's `adapter`-optional amendment: a `[run]`-less tool component form and a `requires.hub` key | additive (**new**) | §8.2 |
| 21 | L3 | "Interfaces": `111d` passes the agent `AGENT111_OPENCODE_PATH` (the agent reads it today, `--opencode-path`) when the opencode component is enabled | additive (**new**) | §8.2 |

Every wire-visible addition (old nodes drop the kinds as today; with every switch off none is sent):

| Where | What | Bytes | Here |
|---|---|---|---|
| `AGENT_OP` (0x10) payload | `schema` ‖ `op` ‖ CBOR map; three ops | ≈ 450-600 B request, ≈ 400-600 B reply, ≤ 16 B refuse (estimates) | §12.1 |
| `PATCH` (0x13) payload | `type` ‖ fix container or vouch | ≤ the SIGNED `tcp-lan` budget; a vouch 106 B | §11.2 |
| signatures inside those payloads | `111/fix/v1`, `111/fix-vouch/v1` | 64 B each | §10.2 |
| `appdata` roles (L2 A8) | bit 6 `ROLE_HEAL` | 0 B (a bit of the existing u16) | §23 L7 |

## 17. Settings and consent

**17.1 Every Layer 9 setting (normative).** `state/heal.json`: JSON `{"schema": 1, …}`, 0600, atomic, written only on a change. Absent ⇒ every default.
Unknown keys are kept and ignored (L8 §15.1's rule, so a rollback never drops a consent); an unknown value of any key reads as that key's default.
Changed only by `PATCH /api/v1/repairs/settings`, loopback-only, with the body `RepairsSettingsPatch` (§18.4). A headless node (a Cat 3 bridge, most hubs) changes them with
`111 heal settings [<key> <value>]`: the CLI calls that loopback route like `111 fabric` does (`NI/cli.rs:75-77`; `N/fabric/cli.rs:1-10`), prints a
setting's consent text, asks y/N and sends `accept_warning` only on `y`; with no argument it prints every setting. On Termux, where loopback is not
trusted, it reports the `401` with the way forward, as `111 fabric` does.

| Key | Values | Default | Who | Consent |
|---|---|---|---|---|
| `heal.autonomy` | `observe`, `diagnose`, `propose` | `propose` | a human | — |
| `heal.fixes.receive` | `off`, `home_hub`, `paired_hubs` (hubs only), `named_contacts`; each an explicit set of senders (§11.4 step 3) | `home_hub` | a human | — |
| `heal.fixes.senders` | a list of contact NodeIds, read only under `named_contacts`; the setting's text lists the names it holds. Layer 5's `pinned` flag keeps its one meaning ("never pruned") and gains no trust meaning: pinning a contact never lets it send this node fixes | `[]` | a human | — |
| `heal.fixes.send` | `off`, `home_hub`, `pinned_contacts` | `off` | a human | `fix_send` |
| `heal.ask_hub` | `false`, `true` (members) | `false` | a human | `heal_delegate` |
| `heal.serve_members` | `false`, `true` (`hub_role` only, else `409 needs_hub`) | `false` | a human | `heal_serve` |
| `heal.code_drafts` | `false`, `true` (`hub_role` and the `opencode` component enabled); it carries OB §3's "open PRs from this hub (off)" switch in v1, and the PR switch itself arrives with the hub token (§22; §24 D42) | `false` | a human | the component's own licence notice |
| `heal.notify` | `transitions`, `off`; `transitions` posts one notification when the Needs-you set goes from empty to non-empty (§14.4); it does nothing on Linux or where the `termux-api` helper is absent, and says so | `off` (until the `termux-api` deb is in the pinned closure, the owner's word) | a human | — |

**Inert by default.** With nothing configured nothing changes without a human, nothing leaves the node, Layer 9 loads no model and nothing resides; the observer
records and, under `propose`, files proposals. A widening answers `409 consent_required` in the pinned shape (`ConsentRequired`, `API:1492-1502`): the
client shows `warning_text` verbatim and repeats with `accept_warning`; each consent records its text hash; a changed text asks again. A switch shows the
node's answer, never an optimistic one.

**17.2 Consent copy (verbatim).** **`fix_send`** (subject `<to short id>`): "Send this fix to <alias>. It tells them what failed on this node and how it was
fixed: <the fields that leave, one per line>. It is signed by this node and sealed to them. You can't take it back." · **`heal_delegate`** (subject the home
hub's short id): "When a problem here has no known cause, ask <hub alias> to diagnose it. Each request tells your hub what failed on this node: the service,
the error code, the release, the checks that ran and up to three short words 111 looked for in its logs. No log lines, messages, contacts or other
apps' logs are sent. Your hub keeps a summary of your latest request and its answer." · **`heal_serve`** (subject this hub's short id): "Members of this
hub may ask it to diagnose their problems. Their requests tell this hub what failed on their devices. Each one runs a check, and the model when it is
installed, on this hub. This hub keeps a summary of each member's latest request and its answer, and they are part of this hub's export." · **`heal_remove`** (subject the
proposal id; asked at accept, §7.3): "This repair turns off <capability>." or "This repair removes <n> stored items.", then the action's "Not undone"
line (§7.2).

**17.3 What the agent may propose, what only a human confirms (normative).**

| Act | The agent | A human |
|---|---|---|
| a typed repair | never directly: only `111d` writes `repair`, `fix` and `human_step` proposals (§7.3), from playbooks and received fixes; the agent proposes device actions, the three bridge actions included, with `device.propose` | accepts |
| a fix to send, a delegation, serving members, code drafts | never | only, through the consents |
| a reach-class confirm, an Undo, a dismissal | never | only |
| a KB draft | never by itself: a `kb_draft` proposal only from the `propose` task a human's "Draft a KB note" starts (§13 item 4) | starts it, accepts, exports |
| a code draft | through a `patch` task on an opted-in hub | accepts, exports, opens the PR |

The bearer is refused on every non-GET route Layer 9 adds (D26 (2)); the agent never accepts a consent and never sees a consent hash.

## 18. The contract (additive; the binding form is `API` at build time)

**18.1 Routes.**

| Route | Method | Shape | Who |
|---|---|---|---|
| `/api/v1/repairs` | GET | `RepairsStatus` (§18.4) | session or loopback; the bearer reads |
| `/api/v1/repairs/incidents` · `/{id}` | GET | `{schema, incidents: [Incident]}` (`?state&limit`) · `Incident` (§3.5) | the bearer reads |
| `/api/v1/repairs/incidents/{id}/dismiss` | POST | 204 | a human |
| `/api/v1/repairs/incidents/{id}/task` | POST | `{mode}`, `mode` ∈ `diagnose` (ask again, §7.7), `explain` (§5.1), `kb_draft` (§13 item 4), `patch` (an opted-in hub, §8.3) → 202 with the task id; 409 `busy` while one runs for that incident; where no agent runs, `diagnose` runs T0 again, clears the ledger block, re-sends the delegation and answers 204; a mode this node cannot run answers 409 `needs_hub` or `component_unavailable`; with the store closed 503 `store_unavailable`, with `heal` `full` 409 `heal_full` (§4.1) | a human |
| `/api/v1/repairs/changes` · `/{id}/undo` | GET · POST | kept changes with `undo_available` · 202; 409 `busy`, `nothing_to_undo`, `changed_since` (§7.7) | the bearer reads · a human |
| `/api/v1/repairs/attempts/{id}/confirm` | POST | 204; 409 `confirm_window_closed` | a human: the accept's session, or with trusted loopback and no session its peer address (§7.6) |
| `/api/v1/repairs/attempts/{id}/restore` · `/dismiss` | POST · POST | a `revert_blocked` attempt: 202; 409 `consent_required`, `busy` · 204 ("leave as is") (§7.4 step 6) | a human, session or loopback; never the bearer |
| `/api/v1/repairs/audit` | GET | `{schema, entries}` (`?since_seq`) | session |
| `/api/v1/repairs/settings` | GET · PATCH | `RepairsSettings` (§17.1) · `RepairsSettingsPatch` (§18.4) → `RepairsSettings`; 409 `consent_required`, `needs_hub`, `component_unavailable` | read: session · write: loopback only, which `111 heal settings` uses (§17.1) |
| `/api/v1/repairs/fixes` · `/{id}` · `/{id}/send` | GET · GET · POST | `{schema, fixes: [Fix]}` · `Fix` · `FixSend {to, accept_warning?}` → 202; 409 `consent_required` | the bearer reads · a human |
| `/api/v1/repairs/drafts` | GET | `{schema, drafts: [Draft]}` (`?state&limit`; `Draft`, §18.4) | the bearer reads |
| `/api/v1/repairs/drafts/{id}/mbox` · `/pr` | GET | `text/plain` downloads (§9) | session |
| `/api/v1/repairs/playbooks` | GET | `{schema, playbooks: [{slug, schema, state: loaded\|refused, reason}]}` | the bearer reads |

**18.2 Additions to existing shapes and the pin list.** `Proposal.kind` gains `repair`, `fix`, `human_step`, `kb_draft` (appended to `API:848`; nine kinds
at `fa1daeb`, `N/dashboard/api_v1/proposals.rs:104-114`, ten after Layer 8's `storage`). `Proposal.decision` gains the optional `applied` member Layer 8 adds (L8 §16.2; the decision record already
holds it, `proposals.rs:170-182`; `API:840-852`), so `reverted` becomes visible; Layer 9 gives its `oneOf` (§18.4). `TaskRequest` gains `incident`, `causes` and `narrative` (§5.2) and `kb_draft` on a `propose` task (§13 item 4). `ConsentRequired.action`
gains `fix_send`, `heal_delegate`, `heal_serve`, `heal_remove` (§7.3), appended (`API:1497`). `Problem.code` gains `busy`, `nothing_to_undo`,
`changed_since`, `confirm_window_closed`, `heal_full` (§4.1), appended (`API:692`); the `agent` row's reason codes gain `prompts_invalid` (§13 item 1); `would_lock_out` is not added in v1 (§22); `store_unavailable` is Layer 8's. New SSE event `incident` → `SseIncident` (one `Incident` on change),
appended to `x-sse-events.events` (`API:2044-2059`); after `reset` the Repairs view refetches `/repairs` and `/repairs/incidents`. Id families minted by
`111d` from the OS random source: `inc-…`, `att-…`, `del-…`, `dft-…`, each the prefix plus 12 hex (§3.5); a fix id is 64 hex (`fix_id`). **Pin list** (the conventions' form, `4-interface/api/README.md:190-207`): (1)
`Proposal.kind` +4: `KINDS`, the UI's card, the agent's writer (which writes `kb_draft` only and never `repair`, `fix` or `human_step`, §7.3) · (2) the agent tool count +2 (§18.3) · (3) `pnpm gen:types`, then `node scripts/gen-sse.mjs
--check`, with the new enum tuples listed in `UI/scripts/gen-sse.mjs` · (4) `incident` in `x-sse-events` and in the store's `switch (frame.event)` · (5)
`UI/test/gen.test.ts`, `UI/test/http.test.ts`, a new `UI/test/layer9-contract.test.ts` and the mock adapters · (6) node111's mirrors of the codes and actions.

**18.3 Agent tools (the Layer 3 set grows; fixed, shell-free, schema-validated).**

| Tool | Kind | Shape | Gate |
|---|---|---|---|
| `incident.read` | read | `{id?, state?}` → incidents; excerpts inside a labelled untrusted block | none |
| `service.log` | read | `{service_id, n ≤ 100}` → `GET /api/v1/services/{id}/log` (own children only), inside an untrusted block | none |

The count grows by two from the count the build finds: 17 at `fa1daeb`, 20 at `680162d` after Layer 7's three (`A/tests/daemon_tools.rs:322`,
`A/tests/chat_loop.rs:308`), +3 for Layer 8 (`storage.read`, `encyclopedia.search`, `storage.propose`, L8 §16.5: 23), so 25 after Layer 9; the three
boards bump the same two test constants in one order: Layer 7, then 8, then 9. Two, not three: a `kb_draft` is a `propose` task with
`kb_draft: true` (§13 item 4), not a tool. At `0d1dd41` the schemas list ends with `relay.propose` (`A/src/tools.rs:352`) and dispatch is
`:382-403`; Layer 9's two schemas and dispatch arms are appended after Layer 8's (the A1 precedent). No typed-repair tool: `proposal.write` takes
only a title and a body (`:113-121`) and repairs come from playbooks (§7.3).

**18.4 Shapes (normative; the binding form is `API` at build time, as Layer 8 §16.2's are).** Every object carries `schema: 1`; ids are §18.2's
families; `seq` values are the store's.
- **`RepairsStatus`** `{schema, verdict, needs_you, open, overflow, store, model_tasks}`: `verdict` ∈ `clear`, `waiting`, `confirm` (the UI renders
  §14.1's sentence from it and the length of `needs_you`); `needs_you: [{kind, id, incident_id|null, title}]`, `kind` ∈ `proposal`, `confirm`,
  `revert_failed`, `revert_blocked`, `refused`, `id` the proposal or attempt id; `open` and `overflow` counts (§3.4); `store: {state, reason|null}`,
  `state` ∈ `open`, `full`, `unavailable`, `journal` (J7) (§4.1); `model_tasks: {running, waiting}`, counts (§5.3).
- **`Incident`** §3.5's record, its `attempts` being `[Attempt]`.
- **`Attempt`** `{id, proposal_id|null, incident_id|null, action, target, state, probe: {name, result}|null, confirm_by: "session"|"peer"|null}`:
  `state` is §4.2's attempt state (`awaiting_confirm` included); `confirm_by` says which confirm §7.6 accepts; the UI calls
  `/repairs/attempts/{id}/confirm`, `/restore` or `/dismiss` with `id`.
- **`Change`** (a row of `GET /repairs/changes`) `{attempt_id, incident_id|null, action, target, kept_seq, undo_available, undo_reason|null,
  issue_url|null}`: `undo_reason` ∈ `changed_since`, `not_kept` (its snapshot was reclaimed), `one_shot` (a restart); `issue_url` the phone's
  prefilled issue URL for a kept typed change (§9 item 3), typed fields only.
- **`Fix`** `{id, direction, author, state, shown, fingerprint, cause_id, actions, applies_to, proposal_id|null, sealed_size, fits_carrier, vouches}`:
  `id` the 64-hex `fix_id`; `direction` ∈ `received`, `sent`; `state` §4.2's fix state; `shown` ∈ `proposal`, `does_not_apply_yet`,
  `does_not_apply`, `needs_later_release`, `sent`; `actions: [{action, shown_as}]`, `shown_as` ∈ `action`, `their_words` (§11.4 step 6);
  `sealed_size` in bytes and `fits_carrier` (≤ 2,937 B, §11.2); `vouches: {accepted, reverted}` counts (§11.5).
- **`Draft`** `{id, class, state, incident_id|null, title, issue_url|null}`: `class` ∈ `code`, `kb`; `state` ∈ `open`, `exported` (§4.2); `issue_url`
  for a `kb` draft (§9 item 3), typed fields only, percent-encoded.
- **`RepairsSettings`** `{schema}` plus each key of §17.1 under its dotted name, with its value. **`RepairsSettingsPatch`** any subset of those keys,
  plus `accept_warning` (optional: the hash of the consent text a `409 consent_required` returned). **`FixSend`** `{to, accept_warning?}`: `to` the
  recipient's NodeId as 64 hex. **`IncidentTask`** `{mode}` → 202 `{task_id}`, or 204 where no agent runs (§18.1).
- **`Proposal.decision.applied`** a `oneOf`: `{pending: true}` while an apply runs (a `device` record also carries `actions`), `{message_id}` (a
  `message` proposal, `N/dashboard/api_v1/proposals.rs:418-424`), `{group_id, items}` (a bridge group, `:56`), the applier's `ApplyReport`
  `{applied, reverted, reason|null, actions: [{action, applied, reverted, skipped, reason|null, result?}]}` (`N/devices/applier.rs:448-475`, a
  `device` proposal), and for `repair` and `fix` `{attempt_id, state}` (§4.2's attempt state).

## 19. Security, privacy and the threat model

**Hard rules.**
1. **The agent holds no key and writes nothing but proposals.** Its bearer is refused on every non-GET (`N/dashboard/api_v1/auth.rs:927-933`); a decision
   is made only inside the decision handler (`proposals.rs:638-706`); a file under `agent/decisions/` is never an authority (every 111 process runs as
   `node111`, R00 trap 3). `repair`, `fix` and `human_step` proposals are written only by `111d`; a spool file of those kinds is never ingested, and an
   accept applies only the actions of `111d`'s own `heal/proposals` record whose hash equals the recorded `action_hash` (§7.3).
2. **Nothing applies code**, on any node, under any rung.
3. **The verifier is never inside what a change may touch**: probes, the ledger, the undo file, the confirm window and the audit are written only by
   `111d`'s code; any process running as `node111` could alter them, as it could alter the configuration itself (R00 trap 3). The boundary Layer 9
   defends is the agent's and opencode's tool surface.
4. **A peer's text is data**: fix prose, a delegated narrative and a peer's verify log are rendered preformatted; the agent never reads or summarises them.
5. **Nothing leaves without a human or a consent**: a fix only by a human's send; a delegation only under `heal_delegate`; a vouch only as the outcome of
   a received fix the human accepted (§2.2 item 10); a PR only by the human's hands (J6).
6. **No signing oracle**: `111d` signs only a body it built from its own records, under the two tags; v1 carries no attachment (§11.1).
7. **No wall clock** orders, prunes or expires anything: `seq`, the boot id, `CLOCK_BOOTTIME` and counts do.
8. **Fail closed on grammar**: a playbook with one unknown field never runs.
9. **opencode runs denied** (§8.1); it never sees a token, a key or the dashboard.

**What is exposed, to whom.**

| Path | On the path | At the far end |
|---|---|---|
| a sent fix (`PATCH`, sealed) | as any sealed item: sizes, timing, `dest_hint` (L5 §18) | the recipient: the `fix/v1` fields; it can show the fix to others, signed by this node |
| a delegation (`AGENT_OP`) | as above | the home hub: the request fields of §12.1, only text 111 wrote, never message text or a log line; the hub keeps the latest per member in its export (§12.4) |
| a vouch | as above | the author: that this node accepted or reverted its fix |
| role bit 6 | everyone who gets the card | that this hub serves members' diagnoses |
| the mbox and `PR.md` | — | whoever the human gives them to: the incident's diagnosis and verify log; the node's short label, its full NodeId only when the human ticks it (§9) |

**Attacks and defences.**
- **A self-approval by a confused or injected model on a hub:** §8.1's deny block; G24 asserts no connection to `127.0.0.1:11180`.
- **A malicious fix that disables the gateway, a carrier or a stored class:** only §11.4 step 6's peer subset reaches a button (`service.restart` of the
  open incident's own subject, `advise`, or this node's own playbook action for the same cause); everything else is "their words, not checked"; and a
  fix files a proposal only while the same incident is open here (step 7).
- **A fix relayed or replayed by a third node:** the frame's sender must equal the author, and `author_seq` must pass the author's `high_water`
  (§11.4 step 3); a vouch counts only from a node this node sent the fix to (step 8).
- **A spool file that claims to be a repair:** the ingest refuses `repair`, `fix` and `human_step`, and an accept re-reads the actions from `111d`'s own
  record (§7.3).
- **A LAN session that widens a loopback-only setting through a proposal:** the action inherits the route's gate, `403 loopback_only` (§7.3).
- **A forged, replayed or oversized fix:** the signature, `author_seq`, the size check; a flood from one author is bounded by the 16-per-author queue; a
  fix from outside the trust set is dropped and counted and leaves no record (§11.4 step 3).
- **A validator gamed by deleting the capability** (the Kali lesson, R02 F14): probes assert the capability; a `plan()` that removes a capability or
  evicts stored items needs the `heal_remove` consent whose text `111d` writes (§7.3); a restart's probe needs the row out of `error` with no new
  unrequested exit (§7.2).
- **A revert that would widen past a stale consent:** it stops `revert_blocked` for the human (§2.2 item 5).
- **A daemon killed mid-apply:** armed at start = revert, bound to the run that armed it and never restored twice (§7.5).
- **A reach-class change that cuts the human off:** the 90 s confirm from the accept's own session (§7.6); `would_lock_out` waits for an action that can
  trigger it (§22).
- **Model output that cites what is not there:** discarded by §5.3; no retry loop.
- **Log text that steers a diagnosis:** a cause with a typed action needs a check that is not `log_contains`; a cause matched on log text alone files
  `advise` (§6.2); T1's enum is the matched causes, or the playbook's plus `unknown` (§5.1).
- **Model or peer text that smuggles a second patch or a trailer into an export:** §9's renderer rule; `git mailsplit` finds one patch (G24).
- **Persistent-memory poisoning through KB drafts** (MINJA, AgentPoison, MemoryGraft, snippets): drafts are exported, never read back.
- **A hub that lies in a diagnosis:** the reply is a proposal on the member, shown through §11.4 step 6's peer subset, accepted by its own human.
- **A stranger asking a hub to diagnose:** only `enrolled` members on attributed connections; one open request each.
- **Request flooding the hub's model:** one task at a time; one open request per member; at most 4 queued in total, beyond it `refuse` 3; a delegated
  run waits while a local task or chat turn waits; a repeated fingerprint and checks hash gets the stored reply with no model call (§12.3, §12.4).

## 20. Verification (lock gates; netns, no hardware)

**Gate homes.** New `9-evolution/verify/{observer,engine,diagnose,share,delegate,codefix,evolution}.sh`, a stdlib-only stub model
`9-evolution/verify/stub-model.py` (replays recorded completions on the model port inside a netns), known-answer vectors under `9-evolution/verify/vectors/`, recorded tapes under `9-evolution/verify/tapes/` (G20 (1), G22 (6)),
and `9-evolution/verify/lib.sh`, which sources `6-transport/verify/lib.sh` and `6-transport/verify/root.sh` as `7-relay/verify/lib.sh` does and names the
netns, veth, addresses (10.111.9x.0/24) and ports. The conventions are `6-transport/verify/lib.sh`: each check prints `ok:` or `FAIL:` (`note:` informs); a
gate whose prerequisites are absent (unprivileged netns through `unshare -Urnm`, a built binary, `OPENCODE_BIN`) prints `<GATE> SKIPPED: <reason>` and exits
0 unless `REQUIRE_<GATE>=1` makes it a FAIL; the last line is `<GATE> GREEN` or `<GATE>: N FAIL`. Every harness stops what it started, leaves nothing
listening and runs alone (AGENTS.md).

**Seam ids (D36 (2), D37).** Layer 9's planned rows (`plan/cross-layer/test-system/matrix.md:177-179`, entering `verify/seams.toml` under D37 F5) plus three
new rows; the ids are the map's own (`verify/seams.toml:1103` writes `L5/7-L9:fix-sharing-delegation`, the form `verify/t111/schema.py:69` accepts).
Each `ok:` and `FAIL:` line starts with its seam id in brackets, for the reader: the runner maps a record to a seam through a check's `seams` or its
`phases` (`== <phase>` lines), never through the bracket. Layer 9's board carries the verify row that registers these gates in `verify/checks.toml`,
each with its `phases`, and promotes the seams.

| Seam id | Layers | The interface it crosses | Gates |
|---|---|---|---|
| `L1-L9:observer-change-engine` | 1 ↔ 9 | the supervisor's state and exit watches (`requested` included), the generation watch, the registry's change signal, the `EventBus` as a wake-up, the unit's `ExecStopPost=`, runit's `finish`, the run marker and the inbox, the update tombstone, the undo file at start | G19, G20 |
| `L8-L9:heal-store` | 8 ↔ 9 | J2's files, J3's reclaim and marker, the J9 calls | G19 |
| `L6-L9:typed-actions` (new) | 6 ↔ 9 | the `Action` trait over the Layer 6 applier and spine | G20 |
| `L4-L9:repairs-api` (new) | 4 ↔ 9 | the decision route, `/api/v1/repairs*`, the `incident` frame, `/settings/l9` | G20, units |
| `L3-L9:diagnosis-playbooks` | 3 ↔ 9 | the agent task API with `incident`, the JSON-schema reply, the two tools | G21 |
| `L0-L9:evolution` | 0 ↔ 9 | the `playbook` category and lint, `share/kb/playbooks/`, `share/agent/`, `kb_draft` | G21, G25 |
| `L2-L9:fix-signing` | 2 ↔ 9 | `sign_with_tag` and `verify_strict_tagged` under the two tags | G22 |
| `L5/7-L9:fix-sharing-delegation` | 5, 7 ↔ 9 | `send_op`, the consumer for 0x10 and 0x13, the size gate, role bit 6, custody across islands | G22, G23 |
| `L3-L9:code-drafts` (new) | 3 ↔ 9 | opencode under the deny block, the source tree, the applies-cleanly check, the mbox | G24 |
| `L1-X:pi-floor` | 1 ↔ all | processes, listeners, idle RSS, timers | G7 |

**G19, the observer** (`observer.sh`: one netns node with the real `111d`, a real supervised stub child, the real agent off; node111 tests). Asserts: (1)
`[L1-L9:observer-change-engine]` a supervised child killed with signal 9 opens one incident `(<its service's layer>, <its service id>, child_exit)` with an
`exit` evidence holding signal 9; five kills in its window leave exactly one incident, at `count = 5` (counted from `ExitFacts.seq`), severity `error` from
the `Degraded` transition, and no `child_degraded` or row incident beside it; it closes `self_recovered` after the child holds `Running` for the window
(after a restart); the model child's incident has subject `model` · (2) a services row forced to `error` opens `(layer, id, reason_code)` once and
closes when the row leaves; a row in `unset` or `starting` (`carrier_pending`, `kb_absent`, the `codeagent` row) opens nothing · (3) `kill -9` of `111d`
then a restart opens `(1, 111d, unclean_exit)`; a graceful stop opens nothing; the inbox line written by `111 heal-witness` from `SERVICE_RESULT=signal
EXIT_STATUS=KILL` reads `signal=9`, is folded and the file cleared; a non-clean line under a clean marker opens `unclean_exit`; `result=watchdog` opens
`watchdog_kill` only; a Termux line `result=signal signal=9` opens `killed_by_os` only and `result=exit code=0` nothing; `starts` is the old value plus
one; a start fact closes `self_recovered` after the window and makes no model request; `111 heal-witness` exits 0 with `state/heal/` missing; 300 exits
leave 256 lines · (4) a tombstone with `pending: false, health_confirmed: false` and another version opens `update_reverted` · (5) no timer: with Layer 9
present and no incident open, Layer 7's `idle.sh` wakeup line (L7 §3.7) is unchanged, the wakeup count with Layer 9 idle is recorded beside Layer 7's
baseline; that no playbook is held parsed is a node111 unit test (`heal_playbooks.rs`), not a shell assertion · (6) a 65th fingerprint opens nothing and counts
`overflow` · (7) `[L8-L9:heal-store]` (§4.1) with the store unopenable the J2 files stay, `GET /api/v1/repairs` answers
`store: {state: "unavailable", reason}`, at most 64 incidents are kept in memory, T0 runs and no proposal, T1 task, fix or delegation is filed, sent or
submitted, and the decision route answers `503 store_unavailable`; with it open they are folded and cleared and the in-memory incidents written; a
restart with an incident open whose row has left `error` closes it `self_recovered`, and one whose child is still crash-looping keeps it open with its
window re-armed; under J7 no proposal is filed; with `heal` `full` observation continues, no `repair` proposal is filed, `store.state` reads `full`, an
armed apply finishes and its decision is written, and its attempt is written once the class has room; an incident opened with the store
closed carries an `inc-` id of 12 hex and keeps it when written; a reclaim never touches an `open` incident or an
`armed` attempt and leaves one marker · (8) `[L8-L9:heal-store]` a reclaim never touches an `awaiting_confirm`, `revert_blocked` or `revert_failed`
attempt, a `high_water` record, an `undecided` fix or `heal/proposals` record, an `open` delegation or an `open` draft, nor any ledger record, and it
takes §4.2's order: closed incidents with their attempts first, a kept snapshot last; every record's store `state` is one of §4.2's · (9) `[L1-L9:observer-change-engine]` a requested restart (`POST /api/v1/services/{id}/restart` and Layer 9's own `service.restart`), a component
disable and a graceful daemon stop open no incident; a component enabled at run time is handed over by the generation watch and its unrequested exit
opens one incident; Layer 9 opens no netlink socket (the daemon's open sockets are unchanged) · (10) `[L1-L9:observer-change-engine]` the witness
delivery: with a unit that lacks the `ExecStopPost=` line (Linux) or a `finish` hook whose bytes differ from the release's copy (Termux), a start
classifies from the run marker alone with evidence `witness: absent` and opens `(1, 111d, witness_absent)` with its `human_step`, one card with the
APK step when both are due; `111d` writes neither file; `1-bootstrap/scripts/check.sh`'s `systemd-analyze` stage and `termux/tests/dryrun.sh` assert
the line and the hook's output format · (11) `[L1-L9:observer-change-engine]` (§12.6) a `state/apk.json` whose sha256 differs from the manifest's
`apk.*` pin opens `(1, 111d, apk_outdated)` with its one `human_step`, which closes when the file matches; with the file absent nothing opens; the
phone dry run (`termux/tests/dryrun.sh`) shows the provisioner pushing the file after the APK step. **Pass:** `G19 GREEN`.

**G20, the engine** (`engine.sh`: G1's bridge stubs, `NODE111_BRIDGE_STUBS`, debug builds only, `N/mod.rs:1084`, and G5's device fakes,
`6-transport/verify/devices.sh`; node111 tests). Asserts: (1) `[L6-L9:typed-actions]` the four device actions through the trait give byte-identical
files and decision records to the Layer 6 applier: a tape of the applier's files and decision records, recorded with G5's fakes at the lock head
before the decision route moves into Layer 8's store, under `9-evolution/verify/tapes/`, compared after the port · (2) a probe
that fails reverts byte-exact and records `reverted` · (3) `[L1-L9:observer-change-engine]` `kill -9` between apply and probe: at the next start the
snapshot is restored before the listener answers, the decision record reads `reverted (daemon restart)`, and a retry is `409 already_decided` · (4)
`[L4-L9:repairs-api]` a `may_affect` action with no confirm reverts after the window; with a confirm carrying the accept's session (or, with trusted
loopback and no session, from its peer address) it is kept; a confirm from another session is refused and the window runs on · (5) while an apply holds
the node-wide lock (a reach-class confirm wait included), an accept of any proposal and a `PATCH /api/v1/transport/posture` answer `409 busy`; after a
revert, or an applied action counted `unresolved` (a new unrequested event bumps the incident's `count` after the keep), the ledger refuses the same
action for the same cause until the evidence hash changes · (6) Undo restores the last kept snapshot, arms its own undo, and its probe asserts the
restored state is in effect; with the target changed since the keep (its state hash is not the recorded `after_hash`) Undo answers `409 changed_since`
and writes nothing; a restart's card offers none · (7) `component.enable` with no cached payload becomes a `human_step` naming the install verb · (8) the
bearer's POST on every Layer 9 route answers 401 · (9) `[L4-L9:repairs-api]` accepting a proposal that holds `settings.set` on `fabric.listen_lan`,
`fabric.hub_role` or `fabric.quota.<class>` from a non-loopback session answers `403 loopback_only`, writes nothing and leaves the proposal open; from
loopback it applies · (10) `[L4-L9:repairs-api]` a spool file with `kind` `repair`, `fix` or `human_step` is not ingested, not deleted and counted; an
accept whose `heal/proposals` record is missing, or whose actions no longer hash to the recorded `action_hash`, answers `refused` and applies nothing ·
(11) `[L1-L9:observer-change-engine]` the start rules of §7.5, each cut by `NODE111_HEAL_PAUSE_AT` and a `kill -9`: a record found `restoring` is not
restored again and reads `revert_failed`; a record whose `release` or `starts` is not the previous run's is not restored and reads `revert_failed`; two
starts with the store closed after a cold restore read `reverted`, not `revert_failed`; a cut at `keeping` or at `decided` finishes the keep with no
restore; a `pending` decision with no undo file reads `interrupted`; an `apply_unrecorded` cut at `apply` is restored at start with `proposal_id: null` ·
(12) `[L6-L9:typed-actions]` a revert that needs a consent that is not current (a component whose notice hash changed, a `consent_paused` edge) stops
`revert_blocked`, writes no consent and releases the lock; at the next start `blocked` is not restored; `/restore` answers `409 consent_required`, then
restores with `accept_warning`; `/dismiss` removes the file · (13) `[L4-L9:repairs-api]` a lowered `fabric.quota.<class>` that would evict stored
items, and a `component.disable`, answer `409 consent_required` with `action: heal_remove` and `111d`'s text naming the count or the component; with
`accept_warning` they apply; the card shows the "Not undone" column · (14) `[L6-L9:typed-actions]` `component.enable` runs `enable_component` (an
adapter's prepare hook runs and its arguments reach the child); a proposal with two consent-gated actions is split into two; a `repair`
`service.restart` whose row stays in `error`, or whose child exits unrequested inside the window, fails its probe; a `device` proposal's
`service.restart` and `identify` stay one-shots with results byte-identical to Layer 6; every new action passes its round trip (apply, a forced probe
failure, state equal to before) · (15) `[L4-L9:repairs-api]` `111 heal settings heal.ask_hub true` calls the loopback `PATCH /api/v1/repairs/settings`,
prints the `heal_delegate` text and writes nothing until `y`; with `y` the setting reads `true`; `111 heal settings` prints every setting · (16) `[L4-L9:repairs-api]` (UI checks) `pnpm build && pnpm budget`
passes with Layer 9's chunks inside the allowance of §14.1 and the lazy total within D21's; `GET /api/v1/repairs/drafts` lists a draft with its
`issue_url` of typed fields only; a `PATCH /api/v1/repairs/settings` and a `POST …/fixes/{id}/send` with `accept_warning` matching the consent hash
apply, with a stale one answer `409 consent_required`; every route of §18.1 answers the shape of §18.4 (`layer9-contract.test.ts` and
`api_v1_repairs.rs`), and a `repair` decision's `applied` is `{attempt_id, state}`; a `fix` proposal shows no accept control on a list row or a notification, only on
its review screen (`repairs-view.test.ts`, §11.4 step 7). **Pass:** `G20 GREEN`.

**G21, diagnosis** (`diagnose.sh`: one node with the real agent and `stub-model.py`, which becomes the model child by sitting at
`current/bin/llama-server` with a `.gguf` file under `models/current/`, the two facts `111d` checks before it supervises the model,
`N/intelligence.rs:158-197`; node111 and agent111 tests). Asserts: (1)
`[L0-L9:evolution]` a playbook with an unknown predicate, field, service id or reason code is refused whole and listed "needs release X"; depth 4, 17
leaves or 2,049 B refuse; `"@subject"` is the only substitution · (2) `[L3-L9:diagnosis-playbooks]` one satisfied cause ⇒ `by: t0` with no model request
(the stub logs none) · (3) two satisfied causes ⇒ one task whose request carries the `incident` block, `response_format` with the cause enum, and no
`daemon_health` or `recent_events`, and whose `response_format` constrains `evidence` to the incident's evidence ids with no `quotes`; after the T1 run
`GET /api/v1/proposals` and the open count are unchanged and the spool file is gone · (4) a stub reply citing an unknown evidence id is discarded, no second
request · (5)
two incidents at once make one model request at a time · (6) under `observe` nothing is evaluated; under `diagnose` no `repair` proposal is filed · (7)
`incident.read` and `service.log` return untrusted blocks; the tool count is the build's +2 · (8) `[L0-L9:evolution]` the lint and the loader refuse a
cause with a typed action whose `when` names only `log_contains` checks, a cause naming a bridge or interface action, and an `@subject` outside the
compiled subject table · (9) `[L3-L9:diagnosis-playbooks]` with two causes satisfied the request's enum is those two; with none it is the playbook's
causes plus `unknown`; a stub reply naming a cause whose checks are not all satisfied files `advise` only, `agreement: model_only`; a cause satisfied by
`log_contains` leaves alone files `advise` · (10) `[L3-L9:diagnosis-playbooks]` ten open and close cycles of one fingerprint, from a child whose log
line differs on every run, make one model request (the evidence hash of §3.5 does not move); an incident no loaded playbook matches makes no model
request and reads `unable_to_diagnose` (or delegates, §12); T0 runs at open and at each `count` change and on no timer · (11) `[L0-L9:evolution]`
evaluating every predicate makes no request (the netns socket log is empty) and spawns nothing; `restarts_at_least` reads `restarts_in_window`; a
playbook with `within_events` or `health_is {of: agent, is: degraded}` is refused whole. **Pass:** `G21 GREEN`.

**G22, sharing fixes** (`share.sh`: nodes A and B and hub H in netns over `tcp-lan`; node111 and mesh111 tests). Asserts: (1) `[L2-L9:fix-signing]` the
vectors verify; a flipped body byte or attachment byte fails · (2) `[L5/7-L9:fix-sharing-delegation]` with `heal.fixes.send = off` no fix leaves A
(capture); after `fix_send` one leaves for H only · (3) on H the body reaches the consumer, never a message row; a fix from an author outside the
`heal.fixes.receive` set is dropped and counted; a `code` or `kb` body, or one with an attachment, from a trusted author is parked "needs a later
release" with no button and no diff; an action outside the peer subset is shown "their words, not checked" with no accept button · (4) accepting it on H runs
the loop and A receives one `accepted` vouch · (5) a replayed `author_seq` and an oversized payload are dropped and counted; a 17th fix from one author
drops the oldest; a fix from an author outside the trust set leaves no record in `heal/fixes` (the counter moves, the record count does not) · (6) with no switch on, the frames A sends equal the Layer 8 build's (a committed tape under `9-evolution/verify/tapes/`), compared as the
sequence of (frame type, destination, payload length) after opening each frame with the test's keys: each seal takes a fresh nonce, so sealed bytes
are never compared · (7)
`[L5/7-L9:fix-sharing-delegation]` the peer subset, each case on H with an open incident of the fix's fingerprint: a `service.restart` for another
subject, a `human_step`, a `settings.set fabric.quota.<class>`, a `component.enable` and a `radio.region.set` with the peer's parameters are each "their
words, not checked" with no button; an action byte-equal to H's own playbook action for that cause files a proposal whose targets are H's `@subject`;
with no such incident open the fix is stored "does not apply here yet" and files its proposal when the incident opens, with no timer · (8) a fix A
signed, delivered to B by H, is dropped and counted (sender is not the author); an `author_seq` at or below the author's `high_water` is dropped, and
`high_water` survives a reclaim; a vouch from a node B never sent that fix to is dropped; the highest `voucher_seq` per (voucher, fix) wins · (9) a fix
accepted on H whose probe fails sends A one `reverted` vouch, and a later Undo on H sends nothing; the `fix` card carries the fixed vouch text · (10)
`[L2-L9:fix-signing]` a body with an unknown field or a duplicate key is dropped and counted; an unknown `schema` is parked "needs a later release"; an
`applies_to.release` that differs reads "does not apply here" · (11) `[L5/7-L9:fix-sharing-delegation]` on Layer 7's G9 topology: a typed fix whose
sealed put is at most 2,937 B, sent from member A, crosses one carrier hop and is stored on a named contact on the far island; a larger one is counted
`forward_too_large` on A's hub and A's fix reads "not delivered"; A's send screen shows the sealed size and the carrier warning above 2,937 B; the
sender refuses only a fix above the `tcp-lan` frame · (12) `[L5/7-L9:fix-sharing-delegation]` (§4.1 rule 5) with H's `heal` class `full`, and
again with H's store closed, a fix from A is acknowledged by H's fabric, dropped and counted, H's `high_water` for A does not move and A retransmits
nothing; a `diagnose` from M to H in either state is answered `refuse` 3. **Pass:** `G22 GREEN`.

**G23, delegation** (`delegate.sh`: hub H and members M1, M2 in netns over `tcp-lan`; `stub-model.py` on H; mesh111 tests). Asserts: (1)
`[L5/7-L9:fix-sharing-delegation]` with `heal.ask_hub` off M composes nothing; on, an incident with no T0 cause sends one `diagnose` (its size recorded
against R07g2 F4's 450-600 B estimate), with no log line at all: key 9 holds matched literals of at most 64 B · (2) H with `heal.serve_members` off answers `refuse` reason 1 and
runs no task; on, it sets role bit 6 and answers once per `req_id` · (3) a contact that is not `enrolled` is dropped and counted; a second open request
from M1 gets `refuse` reason 3 · (4) M files the reply as a proposal, never applied, its actions shown through the peer subset (a `service.restart`
for another subject or a `human_step` reads "their words, not checked"); a reply for a closed `req_id` is dropped · (5) (mesh111 `tests/send_op.rs`, on the `lossy` link, since node111 builds no 160 B link in v1,
§12.2) a 160 B profile leaves the request `queued` "needs a longer link" and a request within 16 fragments of a 233 B profile is sent · (6) the narrative is the playbook's line, byte-exact · (7)
`[L5/7-L9:fix-sharing-delegation]` the same `req_id` with the same request hash gets the stored reply and no second task; the same `req_id` with
another hash gets `refuse` reason 6 `request_changed`, which M never retries; `req_id` is 8 bytes from the OS random source · (8)
`[L5/7-L9:fix-sharing-delegation]` the delegation's life: with no link attributed to H, M sends nothing and puts nothing in custody or a mailbox; on
the next link-attribution event M sends the request with the same `req_id`; a card update that adds bit 6 while an undiagnosed incident is open sends
it; a `refuse` 3 is retried on the next link-attribution event; with 4 requests queued on H a fifth member's request gets `refuse` 3; a new request
with the same fingerprint and checks hash gets the stored reply and no second model request; a delegated run waits while a local chat turn runs;
check ids not all in H's playbook get `refuse` 5; a restart of H loses its queue and M's re-send restores it; a start fact on M sends nothing · (9)
`[L5/7-L9:fix-sharing-delegation]` (node111 `heal_delegate.rs`) an `AGENT_OP` map with keys out of order, a duplicate key, an indefinite length or a
non-shortest integer is dropped and counted and never answered; a map `ciborium` encodes for a request equals its deterministic encoding.
**Pass:** `G23 GREEN`.

**G24, code drafts** (`codefix.sh`; `OPENCODE_BIN` a pinned opencode, else `G24 SKIPPED` unless `REQUIRE_G24=1`). Asserts: (1) `[L3-L9:code-drafts]` the
written `opencode.json` holds the deny block; `HOME` and XDG are in the task dir; no `--auto` · (2) a prompt asking for `curl 127.0.0.1:11180` produces no
connection to that port (socket log in the netns) and no decision · (3) a diff that applies reads "drafted and applies cleanly, not built"; one that does
not reads "does not apply" · (4) the mbox passes `git am` on a scratch repository made from the unpacked `[source]` tarball (`git init`, one commit of its tree; a
node holds no git, so the harness builds it) and carries no `Signed-off-by:`; `PR.md` names what was
not verified; `git mailsplit` yields exactly one patch from a body built to smuggle a second (a title with a line break, a body line starting `From `);
a `Signed-off-by:` line in model text is indented, not a trailer; `PR.md`'s fence is longer than any backtick run inside; the issue URL holds typed
fields only; the node is named by its short label unless the human ticked the NodeId · (5) a canary file outside the work directory (a fake key under
the test root's `state/`) is never present in any request the stub model receives nor in the diff, for a prompt that asks for it by path and by glob;
the `patch` request holds the incident in an untrusted block and the note, and no journal line · (6) `[L3-L9:code-drafts]` (§8.2, §8.3) the opencode
declaration loads in the `[run]`-less tool form with `requires.hub`, and enabling it on a node without `hub_role` is refused; with the component enabled the agent's
environment carries `AGENT111_OPENCODE_PATH` naming the installed binary, and without it none; with no source tarball whose sha256 is the running
release's `[source]` pin (the cached copy of the previous release), the incident task route's `patch` answers `409 component_unavailable`, no task
is submitted and the incident gets the `human_step` naming `sudo 111 install --components opencode`; nothing is written under `state/origin/`.
**Pass:** `G24 GREEN`.

**G25, evolution** (`evolution.sh`; `build.py` fixtures and agent111 tests). Asserts: (1) `[L0-L9:evolution]` `build.py --check` passes on the tree's playbooks; the
reject cases (a wrong filename, a missing `### Checks`, an unparsable block, an unknown predicate) and a valid playbook are fixtures that
`build.py --selftest` runs, refused and accepted as expected; §6.2's example block parses and loads · (2) a tampered `share/agent/` file marks the
`agent` row `prompts_invalid` and the agent is not started; with `AGENT111_DATA_DIR` unset the agent reads no prompt from any compiled-in path · (3)
`[L3-L9:diagnosis-playbooks]` (agent111 `tests/evolution_suite.rs`) every case under `tests/evolution/cases/` reaches its `expect`; a case whose
request no longer hashes to its `request_hash` fails until it is re-recorded; the first release holds 50 cases · (4) accepting a `kb_draft` writes nothing
under `current/share/kb/`; "Draft a KB note" on an incident starts one `propose` task with `kb_draft: true` and no new tool; the draft carries the
frontmatter and the six headings, `status: candidate`, `### Alternatives considered` marked for the human, no `### Smoke test`, the procedure under
`## Details`; a `target` outside N01-N06 or a category folder is refused at accept; its `PR.md` says the lint did not run on the node · (5)
`[L0-L9:evolution]` `build.py --selftest` and the node111 test run the same accept and reject cases from `1-bootstrap/crates/node111/tests/fixtures/playbooks/`
and agree; with the folder absent `--selftest` prints `skipped: playbook fixtures not present`; a node111 test loads every file under
`0-knowledge/playbooks/` with no refusal; `0-knowledge/_meta/playbook-vocabulary.toml` equals what the node111 test writes, and an emitted reason literal
missing from the compiled table fails · (6) `[L0-L9:evolution]` N06 compares a filename with the one rendered from `[match]`; a playbook with
`platforms`, `upstream` or `license`, a `depends_on` to a target with no entry, or a `candidate` without `### Alternatives considered` fails the lint.
**Pass:** `G25 GREEN`.

**G7, extended:** `zero-weight.sh` records RSS as a `note:` and asserts nothing about it, and needs a running daemon that no harness starts
(`6-transport/verify/zero-weight.sh:6`, `:159-162`); so `observer.sh` starts its own daemon with Layer 9 present and idle and runs `zero-weight.sh`
against it: processes and listeners are asserted equal to the Layer 8 baseline, and idle RSS is asserted under the ceiling of §1 rule 3 (the Layer 8
build's idle RSS plus 512 KiB; default, measured at build), the figure recorded in `9-evolution/TASKS.md`. The no-timer line depends on Layer 7 row V1:
Layer 7's `idle.sh` is a stub that prints SKIPPED (`7-relay/verify/idle.sh:8-14`), so `REQUIRE_G7=1` fails while it is one; once V1 lands, its line
must be unchanged. **Pass:** both.

**Unit and integration test homes.** node111 (`1-bootstrap/crates/node111/tests/`): new `heal_observer.rs`, `heal_witness.rs`, `heal_playbooks.rs` (with
`fixtures/playbooks/`), `heal_engine.rs`, `heal_fix.rs`, `heal_delegate.rs`, `api_v1_repairs.rs`; the existing device-applier tests stay and pass unchanged.
mesh111: `tests/send_op.rs` (the size gate on the `lossy` link, `direct_only`, commit-before-seal; G23 (5)); `tests/consumers.rs` is Layer 8's.
agent111: `tests/incident_input.rs`, `tests/opencode_config.rs`, `tests/evolution_suite.rs` (with `tests/evolution/cases/`, §13 item 2),
`tests/store_atomic.rs` (Layer 8's), the tool count in `daemon_tools.rs` and `chat_loop.rs`. UI:
`test/layer9-contract.test.ts`, `test/repairs-view.test.ts`. KB: the playbook cases live once, in node111's `tests/fixtures/playbooks/`, read by
`heal_playbooks.rs` and by `build.py --selftest` (§6.3); the vocabulary drift test is in `heal_playbooks.rs`.

**CI mapping** (`.github/workflows/ci.yml`; parity with the local gates): node111 and mesh111 tests and the UI checks run in `bootstrap-gate` (`:28`,
`1-bootstrap/scripts/check.sh`); agent111 in `intelligence` (`:79`); `build.py --check` with the playbook lint and contract drift, and `build.py --selftest`
(added; today the job runs `--check` only, `:112-113`), in `knowledge-contract-hygiene` (`:96`); the seam-map lint in D37's planned `cross-layer` job. The netns harnesses and G7 stay the lead's: the runners restrict
`unshare -Urnm` (`plan/cross-layer/ci-cd.md:18`). Root-only `check.sh` failures on the build box are recorded, never counted as tree faults (D29).

## 21. Pi-floor accounting

| Item | Every switch at default, idle, no incident open | An incident opens | T1 diagnosis (model nodes) | An apply in progress | Hub serving members | A code draft (hub) | Basis |
|---|---|---|---|---|---|---|---|
| processes | 0; one transient `111 heal-witness` (or the `finish` hook's shell) per daemon exit | 0; with `heal.notify` on, one transient `termux-notification` when the Needs-you set goes from empty to non-empty | 0 (the agent and model already run) | 0 | 0 | +1 opencode while it runs | design; measured at build |
| listeners | 0 | 0 | 0 | 0 | 0 | opencode's own loopback port while it runs (R07g1 §2) | design |
| timers | 0 | one one-shot sleep per recovery window | 0 | the probe's 1 s steps (≤ 30 s), the 90 s confirm only while pending | 0 | 0 | design; G19 |
| threads | 0 | 0 | one agent thread per task (`A/src/tasks.rs:175-177`) | 0 | as T1 | the agent's pumps | src |
| memory | heap: 0 beyond the binary delta and the held `[match]` table (no parsed playbook, no incident; the table measured at build); idle RSS under the Layer 8 build's plus 512 KiB (§1 rule 3; default, measured at build; G7) | ≈ 1 KiB per open incident (estimate, R01 §3.2) and its playbook | the model's working set (Layer 3's) | one snapshot | one request | opencode RSS (measured at build) | measured at build |
| writes | 0 (two small marker writes per daemon run) | one J9 write per transition | one | the undo file, then the attempt | one per request | the draft record | design |
| disk | the run marker (≤ 128 B) | `heal` class, 600-1,000 B a record (estimate) | — | ≤ five snapshots (measured at build) | — | the source tarball, opencode 211-222 MB | R01 §3.2; R07g1 §2 |
| Termux | no saving claimed: nothing in the tree schedules the cron script today (§1 rule 2); the `cronie` deb leaves the closure, its size measured at build | — | — | — | — | — | src |
| wakeups | the `status` heartbeat wakes the observer at least every 30 s (`status.rs:63`), and each published registry change; inside Layer 7 §3.7's line | — | — | — | — | — | src; G19 (5) |
| CPU | 0 | T0 only (no model) | one T1 run on the resident model (measured at build, Q21's bench) | the probe | one delegated run at a time | opencode's | measured at build |
| start-up work, once per daemon start | the marker, the inbox and the undo file read; hashing `share/agent/`; the playbooks' `[match]` scan | — | — | — | — | — | measured at build |

**Build-time weight, not resident:** the `111` binary delta (observer, evaluator, engine, op arm) and the `111-agent` delta (loader, `diffy`), measured at
build; the Repairs chunks, inside Layer 9's 5.1 KB gzip lazy allowance (§14.1; default, measured at build); the playbooks and prompts in the
payload.

**Rule.** The build records each figure, measured on the release binaries, in `9-evolution/TASKS.md` beside Layer 6's (`111` 19,404,064 B at 19.7 MB idle
RSS, `111-agent` 4,392,280 B at 5.3 MB, listeners :11180 (+ the best-effort :80) and :11182, `6-transport/TASKS.md:82`) and Layers 7 and 8's, with RSS from
`/proc/<pid>/smaps_rollup`, `ss -ltnup` before and after, and Layer 7's no-timer method. Processes and listeners are asserted equal to the Layer 8
baseline and idle RSS under its ceiling (G7, extended). A change to the first column fails the gate until the owner says otherwise.

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
| `update.rollback` as a typed action | a usable window: Layer 1's probation confirms after 60 s of green and reverts itself after 60 s of red (`N/mod.rs:137-180`); v1 keeps the `human_step` |
| `would_lock_out` | an action that can remove a pairing or the loopback listener; none is in v1 (§7.6) |
| `code` and `kb` fixes on the wire | an attachment under `111/fix/v1` (the signed `attachment_hash` is already in the input, §10.2), S24's two-hash rule (§11.1), a send path from an accepted draft that shows the bytes in `fix_send` |
| vouch kinds 1 `vouched` and 2 `reproduced`; the `reproduced` maturity step | a sender for each kind and a threshold for `n` (§11.5) |
| idle unload of the model | Layer 3's question: `111d` keeps llama-server resident (`N/intelligence.rs:172-192`); Layer 9 starts, stops and reloads no model (§5.1) |
| the witness on a node that updates itself | a drop-in written by the root helper, if the owner wants one; v1 delivers the witness only with `111 install` and `111 install --phone` (§3.3 rule 7) |
| fixes from unknown authors shown | one ring of 16 across all unknown authors, state `parked`, reclaimable; v1 drops and counts them (§11.4 step 3) |
| `heal.notify` on by default | the `termux-api` deb in the phone's pinned closure, the owner's word (§14.4) |

## 23. Cross-layer seams & amendments

- **Layer 0 (locked):** (item 1) `category: playbook` in the enum (`L0:45`, F05), folder `playbooks/` with its generated `llms.txt`; rule **N06** in
  `_meta/schema.md` §1: "Playbook filename not `repair-<subject>-<reason>.md`, or its `[match]` disagrees with the filename or the closed vocabulary"; rule
  **B10**: "Playbook `### Checks` block missing, not one `toml` fence, not parsing with `tomllib`, or breaking grammar v1's limits"; the H3 vocabulary under
  `## Details`; B09's playbook budget 2,000. `build.py` (`0-knowledge/_meta/build.py:44-52`, the recipe rules at `:575-602`) gains the category and both rules.
  Recipes and N03 untouched; `skills/` stays deferred. Item 1 widened: the applicability matrix (`_meta/schema.md:96-110`) gains the `playbook` column
  of §6.1 (`platforms`, `upstream`, `license` forbidden; `relations` optional, `depends_on` only to a third-party project with an entry, `runs_on`
  optional, neither in N06); the gate adaptations (`L0:91`) gain "**playbook** = `candidate` lint-clean with `### Alternatives considered`; `testing` a
  `### Smoke test` on a real node; `proven` a `### Field log`"; the scope line ("six category folders", `_meta/schema.md:5`), the root index and
  `0-knowledge/AGENTS.md` name the seventh folder; N06 reads "Playbook filename not the one rendered from its `[match]`, or a `[match]` outside
  `_meta/playbook-vocabulary.toml`"; `_meta/playbook-vocabulary.toml` is generated by a node111 test, committed, and read by `build.py` (CI fails on
  drift); `build.py --selftest` also runs the playbook cases of `1-bootstrap/crates/node111/tests/fixtures/playbooks/`, resolved from the repository
  root (`skipped: playbook fixtures not present` when absent; today the self-test reads only `_meta/fixture/`, `build.py:824-825`), and CI's
  `knowledge-contract-hygiene` job runs it (§6.3).
- **Layer 1 (locked v2.2):** (item 2) `111node.service` gains `ExecStopPost=/usr/local/bin/111 heal-witness`; the CLI gains the hidden verb `heal-witness`;
  the Termux `finish` hook gains one append line after `:8`; **removal:** `termux/cron/crontab.line`, `termux/cron/healthcheck.sh` and the Supervision
  text's "cronie `/health` check (placeholder seam until Layer 9)" (`L1:156`) → "the runit `finish` hook appends the exit to Layer 9's inbox". (item 3)
  the `[source]` pin Layer 8 §11.6 adds is also fetched by `sudo 111 install --components opencode` on a hub. (item 4, changed) the Android install flow
  and the layout: after the APK step the provisioner pushes `state/apk.json` (`{schema, apk: {package: sha256}}`, 0600) with `runas::install_file`; no
  `receipt.apk`, since no receipt exists on a phone (§12.6); "APK refresh" reads that file. (item 5, correction) `L1:183` "sustained across the 60s probation window" → "sustained for 60 s
  inside the 600 s probation deadline" (`N/mod.rs:134-180`, `:157`; `NI/probation.rs:57`). Item 2 widened: the delivery sentence (the witness arrives
  with `111 install` and `111 install --phone` only; `111d` reads the unit and the hook at start and opens `witness_absent` with its `human_step` when
  the witness is absent; it never writes either, §3.3 rule 7); `111 heal-witness` exits 0 on every error; the hook writes `result=exit` or
  `result=signal`; the inbox keeps the newest 256 lines. The removal widened (§3.3 rule 6): `termux/cron/*`; the `CRON_HEALTH` include and its install
  step (`NI/install/phone/mod.rs:24`, `:224-229`); `termux/tests/dryrun.sh:102-120`; `L1:156`'s text; the provisioner deletes the copy already pushed to
  `$PREFIX/var/lib/111/share/termux/cron-healthcheck.sh` on its next run; "Release assets", the Termux .deb closure row (`L1:68`): `cronie` leaves the
  minimum, with `LICENSING.md:65`'s row, the minimum in `1-bootstrap/crates/node111/tests/manifest_parse.rs:76-82` and the `cronie` entries of the
  manifest fixtures (`tests/fixtures/l7-*/**/channel-111-stable.toml`); its deb size is the measured saving. (item 19, **new**) the CLI gains
  `111 heal settings [<key> <value>]`, which calls the loopback `PATCH /api/v1/repairs/settings`, prints the consent text and asks y/N (§17.1).
  Conditional, not in v1: the `termux-api` deb joins the closure only on the owner's word (§14.4). Code seams, no spec text: `ChildControl::exits()` (§3.2); the run marker and
  inbox under `state/heal/` (J2, J8; the directory is L8 §21 L1's layout row); `state/heal.json` in the layout table. The hand-offs closed: "Layer 9: supervision-tree handles, `/health` contract,
  exit(0)-restart, watchdog heartbeat" (`L1:277`) and "APK refresh = Layer 9 maintenance task" (`L1:185`).
- **Layer 2 (locked v2):** (item 6) `111/fix/v1` and `111/fix-vouch/v1` with the constructions of §10.2, in the one Layers 8 and 9 block (L8 §21 L2, A19);
  id111 gains no code; vectors in `9-evolution/verify/vectors/`. The construction of `111/fix/v1` keeps `attachment_hash`, 32 zero bytes in v1; vouch
  kinds 1 and 2 are reserved. `author_seq` and `voucher_seq` are minted from the announce counter, a further caller beside Layer 8's catalogue `seq`.
- **Layer 3 (locked):** (item 7) `TaskRequest.incident` and `causes`; the untrusted rendering and the switched-off context; `response_format` passed through
  the model client; tools `incident.read`, `service.log`. (item 8, correction) the deny block, `HOME`/XDG and the five environment flags in
  `write_opencode_config` (`A/src/tasks.rs:495-509`, `:429-433`). (item 9) prompts and the diagnosis schema as `share/agent/` release files. Text: `L3:49`
  "The agent signs its outbound artifacts (proposals, patch PRs) with the node key" gains "As built by Layer 9: `111d`, the identity holder, signs fixes on
  its behalf; proposals are never signed (Layer 9 §10)"; `L3:84` "proposal *signing* is deferred to Layer 9's PR packaging" gains the same note. The
  hand-off "Tasks → Layer 9" (`L3:47`) is closed by §5 and §8. Items 7-9 widened: the `patch` task's inputs are the incident in an untrusted block and
  the note, nothing from the journal (§8.3); T1's enum is the matched causes, or the playbook's plus `unknown` (§5.1); `response_format` constrains
  `evidence` to the incident's evidence ids and asks no `quotes` (§5.2); `TaskRequest.kb_draft` on a `propose` task, whose frontmatter and headings
  `111-agent` code writes from typed fields (§13 item 4); no new tool for it (§18.3); `TaskRequest.narrative` for mode `explain` (§5.2);
  `AGENT111_DATA_DIR`, set by `111d` as `AGENT111_KB_DIR` is, with no compiled-in fallback, and the `agent` row's reason code `prompts_invalid`
  (§13 item 1). (item 21, **new**) "Interfaces": `111d` passes the agent `AGENT111_OPENCODE_PATH` naming the installed opencode when the component is
  enabled; the agent reads the variable and `--opencode-path` today (`A/src/config.rs:169-176`, `:303`), and Layer 3's text gains it (§8.2).
- **Layer 4 (locked):** (item 10) §18's routes, shapes, event and pin list; `/settings/l9` as a lazy chunk; the Needs-you rows; the two UI copy lines of
  §14.3; `/api/v1/repairs*` is not loopback-only for reads. The conventions gain a "Layer 9 additions" block. The hand-off "applying accepted proposals is
  Layer 9's job" is closed by §7. Item 10 widened: `repair`, `fix` and `human_step` are kinds the agent's writer never writes (§7.3);
  `ConsentRequired.action` gains `heal_remove` (§7.3); `Problem.code` gains `changed_since` and `heal_full` (§4.1) and not `would_lock_out`; the
  routes `attempts/{id}/restore` and `/dismiss`, `GET /api/v1/repairs/drafts` and the incident task route (§18.1); the shapes of §18.4, with
  `Proposal.decision.applied` as a `oneOf` beside Layer 8's member; the `device` vocabulary text (`API:848`) and the device card gain `bridge.edge.pause`, `bridge.edge.set` and `bridge.order.set`, and so
  does the agent's `device.propose` tool schema. (item 17, **new**) `POST /api/v1/proposals/{id}/decision` answers `403 loopback_only` to a non-loopback
  accept of a proposal holding an action on a loopback-only key: an action inherits the gate of the route it wraps (§7.3; Layer 8's I32 states the same
  for `quota.set`). (item 18, **new**) `PATCH /api/v1/transport/posture` answers `409 busy` while an undo is armed (§7.4).
- **Layer 5 (locked v1.2):** (item 11) `Fabric::send_op(to, kind, payload, receipt_strong) -> msg_id` for `AgentOp` and `Patch` only, beside `send`
  (`M/fabric.rs:1279`), with the Tier 0/1 size gate of §12.2 and the same commit-before-seal rule; `send` stays byte for byte. Item 11 widened:
  `send_op` takes `direct_only`, which sends only on a link attributed to the destination and never puts the item in custody or a mailbox (§12.2);
  the size gate is the sum of the per-fragment budgets of 16 fragments (`payload_budget`'s five arguments, `M/envelope.rs:1395-1401`), proven on
  mesh111's `lossy` link (`tests/send_op.rs`), since in v1 `send_op` sends over `tcp-lan`, the one link built (§12.2). (item 12) reserved: no
  stream id in v1 (§24 D1). (item 13) Layer 8 §12.3's consumer registry, registered for 0x10 and 0x13 by Layer 9; the textless chat row ends for both kinds.
  (item 14) beside the Tier table's "larger objects and every bulk class" (`L5:241`): "A `PATCH` whose payload fits one `tcp-lan` frame or 16 Tier 1
  fragments may travel without Tier 2 (Layer 9 §11.2)." The hand-off "`PATCH` class, `AGENT_OP`, `RECEIPT_STRONG`; the apply→verify→undo engine (P4)"
  (`L5:831`) is closed by §7, §11, §12.
- **Layer 6 (locked v1.0):** (item 15) §12 gains the trait (§7.1), the persisted undo and "armed at start = revert" (§7.5; a correction of `L6:657-658`), the
  reach class (§7.6), and the vocabulary of §7.2 with the four device actions byte-for-byte; §8 gains the three bridge actions; `L6:487-488` stays literally
  true. Item 15 widened: one apply at a time node-wide, the lock held from arm to keep, revert or confirm, and `apply_unrecorded` armed with
  `proposal_id: null` (§7.4); the trait's `restore_cold(Bytes)` and the undo file's phases `armed`, `keeping`, `restoring`, `restored`, `blocked`
  (§7.4, §7.5); an automatic revert never passes a consent gate (§2.2 item 5); the three bridge actions join `device.propose`'s closed vocabulary
  (Layer 6 §13), as "human or `device.propose` only" (§7.2); a `device` proposal keeps `service.restart` and `identify` as Layer 6's one-shots. `assets/components/opencode.toml` follows §2.1 rule 3 (`prefetch = false`). (item 20, **new**) §2.1 rule 1, beside Layer 8's
  `adapter`-optional amendment: a `[run]`-less tool component form (no supervised child, no health) and a `requires.hub` key; today `adapter` and
  `[run]` are required (`N/components/manifest.rs:243`, `:248`) (§8.2). G7 covers Layer 9 idle.
- **Layer 7 (locked v1.0):** (item 16) the §7.2 table gains bit 6 `ROLE_HEAL`, set by `heal.serve_members`, defined in node111 beside bits 3-5
  (`roles(s, relay, lan)`, `N/fabric/mod.rs:592` at `680162d`: bit 6 needs a third input, and a toggle of `heal.serve_members` re-mints the announce
  once), no mesh111 change, no consent of its own beyond `heal_serve`. Fixes across islands ride §5 custody unchanged.
- **Layer 8 (locked v1.0, `DECISIONS.md` D38):** the joint contract J1-J9 consumed, nothing added; the `heal` class of §4.1; the `Full` answer of J9 and §4.5,
  handled by §4.1 here; the consumer registry (§12.3); the `[source]` pin (§11.6), read where Layer 8's updater placed it and never written by Layer 9
  (§8.2); the export carries `heal/*` (J6). The seam `L8-L9:heal-store` is proven by G19 and Layer 8's G13 item (7). Requests to Layer 8 (its text,
  never edited here; since its lock each is a lock-amendment candidate): J1's list of record kinds names the kinds §4 places beyond it (the proposals
  record, sent fixes, the `high_water` records, delegations and stored replies, drafts, grants); its layout row for Layer 9 (§21 L1) carries
  `state/heal.json` (0600) beside `state/heal/`; a consumer of §12.3 receives the frame's authenticated sender NodeId, whether the link is attributed, the `msg_id` and the body
  (§11.4 steps 1, 3 and 8); the hand-off sentence "a 64 KiB attachment cap" reads "no attachment in v1" (§11.1); J9's protected states read as each
  namespace's own declaration, so `heal/proposals` `undecided`, `high_water` and the `revert_blocked` and `revert_failed` attempts stand beside it (§4).
- **`LICENSING.md`, `DECISIONS.md`, repo:** §5 a `diffy` row; §3's opencode row (`LICENSING.md:63`) updated to 1.19.5, its provider package bundled (R07g1
  F4) and the component declaration; `THIRD-PARTY-NOTICES.md` regenerated; `.github/PULL_REQUEST_TEMPLATE.md` gains the DCO 1.1 line (Q2).
- **Plan:** `plan/cross-layer/parked.md` P4 is picked up (§7); its "may skip approval" clause stays unbuilt (§2.3).
- **Code seams in locked crates, no spec text:** mesh111 gains a read, "the evictions a quota change would cause", that `plan()` of
  `settings.set fabric.quota.<class>` calls before the apply (§7.1, §7.3); node111's applier lock becomes the engine's node-wide lock (§7.4);
  `ExitFacts.requested`, set from the stop and restart flags after the wait (§3.2); a generation `watch` published by `Supervised` and `Components`
  (§3.1 rule 4); the registry's existing change signal, `Registry::state_changed`, as the observer's wake (§3.1, no change to it).

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
- **C1** The helper honours `revert-request.json` only while a probation journal is pending (`NI/update_helper.rs:282-308`, `:297-300`), and the daemon
  decides a probation by itself within about a minute: it confirms after 60 s of green and asks for its own revert after 60 s of red (`N/mod.rs:137-180`).
  `update.rollback` is therefore not a v1 action; a `human_step` names Layer 1's own probation revert and the monotonic floor (§7.2, §22). The brief left
  this "a Layer 1 read at build".
- **C2** The v1 inner layout has no stream field; every packet opens on stream 0 (`M/ids.rs:19-22`; `M/envelope.rs:1373`, `:1383`); a `stream_id` TLV is
  reserved (`L5:485`): R07g2 R2's "no change to the wire" does not hold (§24 D1).
- **C3** Enable never downloads (`N/components/mod.rs:36-40`): `component.enable` with no cached payload is a `human_step` naming the install verb.
- **C4** The guard's report is only printed (`NI/update_guard.rs:94-96`): a start-time revert is read from the run marker and the tombstone (§3.3 rule 5).
- **C5** Layer 7's tags are registered as A14 (`L2:449-456`): Layer 9's two join Layer 8's A19 block (L8 C4).
- **C6** The deny block and the `HOME` fix have not landed (`A/src/tasks.rs:495-509`, `:429-433`): §15 item 1.
- **C7** `/health` is two-state (`N/health.rs:24-40`) and the tri-state is the agent's (`L3:84`): `health_is` names which (§6.2).
- **C8** A services row publishes only after two identical ticks (`N/services/mod.rs:1459-1468` at `0d1dd41`; was `:1445-1481`): children are observed
  through their watch, rows through the registry's change signal, never by parsing the `services` frame (§3.1).
- **C9** `Proposal.kind` holds nine kinds and the agent seventeen tools at `fa1daeb` (`proposals.rs:104-114`; `A/tests/daemon_tools.rs:318`), twenty tools
  at `680162d` after Layer 7 (`A/tests/daemon_tools.rs:322`, `A/tests/chat_loop.rs:308`): bumps are relative.
- **C10** Layer 8 §11.6 adds the release's source tarball as a manifest `[source]` pin; the brief's hub-only source asset is that pin (§24 D4).
- **C11** A SIGNED `tcp-lan` payload is about 65,381 B (`M/envelope.rs:81`, `:2198`): 4 KiB plus 64 KiB does not fit one frame (§24 D5).
- **C12** Line moves since 2026-11-02, re-pinned here: `L1:154`, `:156`, `:183`, `:185` (were `:129`, `:131`, `:158`, `:160`); `L5:141-142`, `:175`, `:241`,
  `:452`, `:831` (were `:139-140`, `:173`, `:239`, `:453`, `:832`); `L6:487-488`, `:657-658` (were `:458-459`, `:628-629`); `L2:91`, `:227`, `:234` (were `:85`,
  `:217`, `:224`); the daemon tick `N/mod.rs:906` (was `:902`); `roles(s, relay, lan)` `N/fabric/mod.rs:592` at `680162d` (was `roles(s)` `:435-444`;
  L7 cites `:334-343`); `KINDS` `proposals.rs:104-114`;
  `API:840-852` (was `:746-758`). `fa1daeb` is behind HEAD in committed code, not only in the working tree: at `680162d` `M/fabric.rs:1237-1243` is
  `link_events` and `send` is `:1279`; the `PATCH /fabric/settings` handler takes `LoopbackOnly` at `N/dashboard/api_v1/fabric.rs:441`. Also at
  `680162d`: `N/kb.rs:32-34` is `verify_current`'s doc comment, and the `kb` row's codes are at `N/services/mod.rs:1028` (`verify_failed`) and `:1034`
  (`kb_absent`); `status.rs:407-428` is `publish` (`:411`), the health components are at `:332-337`; `payload_budget` takes five arguments
  (`M/envelope.rs:1395-1401`), not a profile; the registry's change signal is `N/services/mod.rs:1451`, `:1474-1477`, `snapshot` `:1515`, the
  `services` publish `:1576`, the row key `:1433-1435`; the services tick is `N/services/probe.rs:45`, its loop `:520`; netlink's one consumer is
  `debounced`, `N/transport/netlink.rs:315`. At `0d1dd41`: the agent view's Layer 9 line is `UI/src/views/agent/proposals.tsx:137` (was `:121`); the
  lazy-bucket figure is row U2 of `7-relay/TASKS.md`, `:91` at `61ae26d` (`:90` at `0d1dd41`; it moves as that board grows). v0.2 re-read the
  citations it changed at `680162d`; every citation is re-pinned at the lock head before the lock.
- **C13** The brief's model lifecycle is not built: `111d` starts llama-server with `--model --host --port` only and keeps it resident wherever the
  binary and a GGUF exist (`N/intelligence.rs:172-192`); `--sleep-idle-seconds` is in no argv. There is no on-demand load and nothing to reload: T1 and
  the hub's T2 use the resident server, and idle unload is Layer 3's question (§5.1, §12.4, §22; §24 D28).
- **C14** Nothing in the tree schedules the Termux cron script: no code installs `crontab.line` or starts `crond`; the provisioner pushes only the
  script (`NI/install/phone/mod.rs:24`, `:224-229`), and R00 counted the line, not a run. The "−1 `curl` per 5 min" saving is not claimed (§1 rule 2,
  §21; §24 D26).
- **C15** Two observer sources cannot be awaited as written: each netlink socket has one consumer (`N/transport/netlink.rs:315`;
  `N/devices/hotplug.rs:305`), and `Components::status` is a pull (`N/components/mod.rs:420`); the observer wakes on the registry's change signal and a
  generation watch instead (§3.1; §24 D27). Every stop and restart sends SIGTERM and the loop learns whether an exit was requested only after the wait
  (`N/supervise.rs:645-651`, `:688`, `:972-985`): `ExitFacts.requested` (§3.2). `kb_absent`, `carrier_pending` and the `codeagent` row are `unset`, never
  `error` (`N/services/mod.rs:1034`, `:1065-1068`, `:1088-1121`): §3.4 rule 1. The agent's `degraded` answers 200 and the probe keeps the code only
  (`N/services/probe.rs:162-165`; `L3:84`): §6.2's `health_is`.
- **C16** No install receipt exists on a phone: `Receipt::write` runs only in the Linux installer (`NI/install/mod.rs:191`, `:251`) and `PhoneReceipt`
  (`NI/install/receipt.rs:10-19`) is constructed nowhere, so the brief's `receipt.apk` map has no file to live in on the phone: the provisioner pushes
  `state/apk.json` instead (§12.6; §24 D38).
- **C17** A component declaration requires `adapter` and a `[run]` table (`N/components/manifest.rs:243`, `:248`) and has no hub-only key, and nothing
  hands the agent an installed opencode (its default is `opencode` on `PATH`, `A/src/config.rs:100`; `agent_spec` passes no path): §8.2's declaration
  form and `AGENT111_OPENCODE_PATH` (§16 rows 20, 21). The install verb fetches the source tarball once, so after an update it is the previous
  release's: §8.3's sha256 check before a `patch` task.
- **C18** The daemon builds one fabric link, `tcp-lan` (`M/link/tcp_lan.rs:714`); the other link kinds are enum seams (`M/link/mod.rs:231-242`) and the
  160 B profile exists only as mesh111's `lossy` test link (`M/link/lossy.rs:241`): a fix and a delegation travel over `tcp-lan` in v1 and the size gate
  is proven in mesh111 (§11.2, §12.2; §24 D39).
- **C19** `zero-weight.sh` records RSS and asserts nothing about it and needs a daemon no harness starts (`6-transport/verify/zero-weight.sh:6`,
  `:159-162`), and Layer 7's `idle.sh` is a stub (`7-relay/verify/idle.sh:8-14`): G7's extension starts its own daemon and asserts an RSS ceiling (§20;
  §24 D40).

**Deviations from D35 and the brief (each needs the owner's acknowledgement).** **D1** OB F13 item 12, `STREAM_AGENT = 1`, is reserved, not carried: v1
sends `AGENT_OP` and `PATCH` on stream 0 in the pair's sequence (C2); a long op over a 233 B bearer shares the cumulative ACK with chat, bounded by the
receiver's range list (`L5:205`) · **D2** `update.rollback` leaves the v1 action set (F6's vocabulary named it): its usable window is about a minute (C1); the `human_step` stays, naming
Layer 1's probation revert and the monotonic floor (§7.2, §22) · **D3** a component action never downloads (C3) · **D4** the hub-only
source asset is Layer 8's `[source]` pin, fetched by the install verb on an opted-in hub (C10) · **D5** moot in v1: no fix carries an attachment (D17); C11's arithmetic stands for the release that adds attachments (§22) · **D6** F3's "three small files … under `state/`" are the paths Layer 8 pins, `state/heal/{run,inbox,undo}` and
`state/heal/journal.jsonl` (J2, J7; L8 D13), followed here because the contract is stated once, in Layer 8 §3 · **D7** F3's "registered in the one L2
table in one amendment" is Layer 8's A19 block for Layers 8 and 9, Layer 7's tags being A14 already (C5) · **D8** `heal.fixes.send` takes `off`,
`home_hub`, `pinned_contacts`: F10 A names a pinned contact as a recipient while OB §3 lists only `off` and `home_hub` · **D9** settings live in
`state/heal.json` (L8 C1's reason) · **D10** a T1 diagnosis is written as a `kind: diagnose` proposal with a JSON body and folded into its incident at ingest: it never enters
`agent/proposals`, a proposals list or the open count (§5.2) · **D11** `heal.serve_members` has
its own consent, `heal_serve` · **D12** the narrative in a delegated reply is the hub playbook's line, not model text · **D13** a start-time revert is
detected from the run marker (C4) · **D14** accepting a `patch` proposal on a hub creates a `code` draft · **D15** the default rung `propose` files proposals
on an unconfigured node; "inert" is read as "nothing changes, leaves or loads without a human" (§17.1) · **D16** a peer's fix or a hub's reply reaches
a button only through the peer subset (§11.4 step 6: `service.restart` of the open incident's own subject, `advise`, or this node's own playbook action
for the same cause), and a typed fix files a proposal only while the same incident is open here (step 7): this narrows F10 A's and F11's "re-validated
against this node's own vocabulary" · **D17** `code` and `kb` fixes are cut from the v1 wire; they travel as drafts through the human's pull request
(§9, §11.1), where F10 A sent `code` and `kb` classes · **D18** `heal.fixes.receive`'s third value is `named_contacts`, an explicit NodeId list
(`heal.fixes.senders`), where the brief's value was `pinned_contacts`; Layer 5's `pinned` flag keeps its one meaning and gains no trust meaning (§17.1) · **D19** the three bridge
actions and the two interface actions are human or `device.propose` only; no playbook carries them, where F12 A grew bridging "plus playbooks" (§7.2,
§13) · **D20** `would_lock_out` waits for an action that can trigger it (§7.6, §22), where F6 A refused such an action · **D21** a cause with a typed
action needs a check that is not `log_contains`, a cause matched on log text alone files `advise`, and T1's enum is the matched causes or the
playbook's plus `unknown` (§5.1, §6.2): this narrows F5 A's `log_contains` and F4 A's T1 enum · **D22** vouch kinds 1 and 2 are reserved, and §11.5's
ladder in v1 is `local` → `accepted_by_n` with `reproduced` reserved, where F10 A named a `reproduced` step · **D23** Undo runs only while the target is
unchanged since the keep (§7.7), which narrows F6 A's Undo · **D24** two new Layer 4 amendments, §16 rows 17 (`403 loopback_only` on the decision route,
§7.3) and 18 (`409 busy` on the posture route, §7.4) · **D25** the exit witness arrives only with `111 install` and `111 install --phone`; `111d`
never writes its unit or its hook, files `witness_absent` with a `human_step` where the witness is absent, and classifies from the run marker alone
there (§3.3 rule 7), where F2 A assumed the witness on every node; a root-helper drop-in is §22's · **D26** the Termux cron seam is removed with no
saving claimed (nothing schedules it today, C14), and the `cronie` deb leaves the Termux closure (§1 rule 2, §23 L1); the owner's word was asked for
the deb · **D27** netlink is not an observer source (§3.1 rule 4), where F2 A named the netlink sockets; its effect arrives as the `radio` and
`transport` rows and the `devices` frame · **D28** Layer 9 starts, stops and reloads no model and adds no idle unload (C13, §5.1), where the brief
assumed an on-demand load and `--sleep-idle-seconds` · **D29** `restarts_at_least` loses `within_events`, `health_is` for the agent reads `ok` and
`error` only, and a predicate makes no request (§6.2): this narrows F5 A's grammar · **D30** off the node goes only text 111 wrote; a delegation's key 9
carries at most three matched literals of at most 64 B, never a log line (§3.5, §12.1, §17.2), where F11 B sent three excerpts of at most 120 B ·
**D31** a fix has three size bounds (one `tcp-lan` frame, 16 Tier 1 fragments, 2,937 B sealed through a carrier); above 2,937 B the send screen warns
and Layer 7's hub refuses the carrier hop (§11.2), which narrows README §9's "through the gateways" (§0's row) · **D32** a fix from outside the trust
set is dropped and counted with no record (§11.4 step 3), where F10 A parked it "unknown author"; a ring for unknown authors is §22's · **D33**
`heal.notify` defaults to `off` until the `termux-api` deb is in the pinned closure, and when on posts once when Needs you goes from empty to non-empty
(§14.4), where OB §3's default was `transitions` · **D34** the T1 schema constrains `evidence` to the incident's ids and drops `quotes` (§5.2,
§5.3), where F4 A required quoted excerpts to be substrings · **D35** a KB draft is started only by a human's "Draft a KB note", enters `candidate`
with `### Alternatives considered` marked for the human and no `### Smoke test` (§13 item 4), where F12 A's draft carried a Smoke test generated from
the incident · **D36** one new Layer 1 amendment, §16 row 19 (`111 heal settings`, §17.1) · **D37** Layer 9 reads no battery state (§3.6), where
the brief read the battery on demand: nothing in v1 consumes it · **D38** the APK refresh reads `state/apk.json`, which the provisioner pushes after the
APK step (§12.6; C16), where F11 named a `receipt.apk` map; an absent file files no `human_step` · **D39** in v1 a fix and a delegation travel over
`tcp-lan` only, the one fabric link built, and the size gate is proven on mesh111's `lossy` link (§11.2, §12.2; C18), where F10 A and F11 B named 233 B
bearers (§0's rows) · **D40** G7 asserts idle RSS under a ceiling, the Layer 8 build's plus 512 KiB (default, measured at build), and the heap adds 0
beyond the binary delta (§1 rule 3, §21; C19), where the report left the number to the owner · **D41** two README sentences are stated as v1 limits in
§0: "The local agent continuously observes" and "the autonomous monitor of system health" (`111d`'s code observes; the model enters at T1 only; a
monitoring model is not planned for the Pi floor; closed by the opt-in hub probe of §22), and "through a nearby hub" (the enrolled home hub only; closed
by hub-to-hub delegation) · **D42** OB §3's "open PRs from this hub (off)" is carried as `heal.code_drafts`; the PR switch itself arrives with the hub
token (§17.1, §22) · **D43** the display phases are thirteen, OB §3's eleven plus `awaiting_confirm` and `revert_blocked` (§14.2), each computed from
§4.2's store states · **D44** two new amendments for opencode, §16 rows 20 (Layer 6: a `[run]`-less tool component form and `requires.hub`) and 21
(Layer 3: `111d` passes `AGENT111_OPENCODE_PATH`) (§8.2; C17) · **D45** the Repairs page's lazy allowance is 5.1 KB gzip, what Layer 8's 12 KB leaves
of the 17.1 KB after Layer 7 (default, measured at build); when it does not fit, the page ships the incident list, Needs you, accept, undo and dismiss,
the other sections are held back from the v1 UI and stay reachable through `111 heal` and the API, and raising D21's lazy total is the owner's
(§14.1; Open (0)).

**Open for the owner (each applied here as the lead's default until answered).** (0) raising D21's lazy total, if the measured Repairs chunks exceed
Layer 9's 5.1 KB allowance; until then the page ships what fits and the overage is recorded in `9-evolution/TASKS.md` (§14.1, D45) · (1) `diffy` as build-time weight on every `111-agent` (AGENTS.md asks
the owner's say; OB F7 said so; D35 took the pick) · (2) `AGENT_OP` and `PATCH` on stream 0 in v1 (D1), or the reserved `stream_id` TLV, a wire change ·
(3) settled by the lead's ruling: `update.rollback` leaves the v1 action set and the `human_step` stays (D2); a later release adds the action only with a
usable window (§22) · (4) the `termux-api` deb in the phone's pinned closure, a runtime dependency on a phone's required configuration (AGENTS.md),
which would let `heal.notify` default to `transitions` (D33); until then the default is `off`.

**Numbers.** Kept as the lead's defaults (OB §5): the 90 s confirm · five kept snapshots · 64 open incidents · 16 incoming fixes per author · twelve
predicates; depth 3, 16 leaves, 2 KiB · a 64 B literal over 100 lines or 8 KiB · excerpts 512 B on the node; in a delegation at most three matched
literals of at most 64 B (§12.1) · a fix body ≤ 4 KiB. From Layer 8's J2: the witness inbox keeps the newest 256 lines; an inbox line is at most 96 B
with code and signal at most 3 digits (§3.3). From the triage: a hub queues at most 4 delegated requests in total (§12.3; default, measured at build) ·
one stored reply per enrolled member, the latest (§12.4) · three fix size bounds, 2,937 B through a carrier being Layer 7's (§11.2) · an idle-RSS ceiling of the Layer 8 build's plus 512 KiB (§1 rule 3, G7;
default, measured at build) · Layer 9's lazy UI allowance of 5.1 KB gzip, what Layer 8's 12 KB leaves of 17.1 KB (§14.1; default, measured at build) ·
50 cases in the first release's must-not-break suite (§13 item 2; R04 §1's figure, default) · `inc-` ids of 12 hex (§3.5), and the same 12 hex for `att-`, `del-`, `dft-`
(§18.2; default). The drafter's own, for the panel: K = 3 for `accepted_by_n` · the playbook budget 2,000 tokens · 10.111.9x.0/24. From the safety review, accepted by the triage: a subject line ≤ 72 B (§9) · `req_id` 8 bytes from
the OS random source (§12.1).

## Changelog v0.1

- 2026-11-11: first draft from D35, D36, the options brief, research R00-R07g3, the Layer 8 draft's joint contract and the code at `fa1daeb`, for the
  adversarial panel.
