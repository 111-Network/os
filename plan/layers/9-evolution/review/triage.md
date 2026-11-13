# Layer 9 spec v0.1 — the lead's triage of the panel (2026-11-13)

Three reports: `safety.md` (S1–S28), `observe.md` (O1–O29), `integration.md` (I1–I32). Each verdict: amend-then-lock, no redesign; the blockers
are S1–S3, O1–O2 and I1–I3 (eight). The owner's instruction stands (go with the lead's recommended defaults and build A to Z; the owner
fine-tunes on real tests; the root README is never reworded or contradicted: D36), so every fork below is ruled here. A ruling that departs from
the brief or narrows what the root README promises is marked "listed for the owner": the spec names it in §24 "Deviations", and a narrowed README
sentence also gets its row in §0's v1 scope-limit table. **Default ruling: a finding not named in the tables below is accepted with the fix its
report gives.** Layer 8's triage (`plan/layers/8-storage/review/triage.md`) binds everything Layer 9 touches in Layer 8 (section "What Layer 9
takes from Layer 8's triage"); where `8-storage/README.md` and that triage differ, the triage holds. The fix pass produces `9-evolution/README.md`
v0.2 with this file as the law; one re-check reads v0.2 against this file and against the Layer 8 text that locks.

## Rulings on the check (they override the tables below where the two differ)

A read-only judge checked this triage as drafted: `triage-check.md` (items C1–C21, each with the collision in full). The lead's ruling on each
follows; an author applies the ruling here first and the tables below second.

| item | ruling |
|---|---|
| C1 (I11) | Form (a). On a hub without `origin.serve` the opencode tarball stays in the root-owned `cache/packages/opencode/`, and its sha256 is checked against the running release's `[source]` pin before a `patch` task. `state/origin/blob/` holds it only where Layer 8's updater put it. Layer 9 adds no writer to Layer 8's tree, and the sentence leaves "What Layer 9 takes from Layer 8's triage". |
| C2 (O6, O24 = I28, O20 = I23 a) | One (marker, inbox) table. A line is clean when `result=success` (Linux) or `result=exit code=0` (Termux). A Termux `result=signal signal=9` opens `killed_by_os` only. A `watchdog` result opens `watchdog_kill` only. Every other line opens or bumps `unclean_exit`. One event, one incident. |
| C3 (S4 with I8) | The undo file's phases are `armed`, `keeping`, `restoring`, `restored`, `blocked` (C8), then removed. Order at start: read the marker, reconcile the undo, then rewrite the marker. `restoring` is written before `restore_cold`; `restored` (fsync) after it returns Ok. At start `restored` means no restore: fold `reverted (daemon restart)` when the store opens, then remove the file. G20 (3) gains: two starts with the store closed after a cold restore read `reverted`, not `revert_failed`. |
| C4 (S3, fix states) | Keep and narrow. A signed, trusted, in-scope typed fix with no matching open incident is stored `undecided` inside the 16-per-author queue, shown as "does not apply here yet", and matched when an incident with its fingerprint opens (one `list` at incident open; no timer). Only then is a `fix` proposal filed. This is what makes README §9's "a repair validated on one node can heal others" true for a node that fails later. |
| C5 (I1 b, inbound consumers under `Full`) | While `heal` is `full` or the store is closed, a consumer never returns an error: a fix is dropped and counted (its `high_water` not advanced), a `diagnose` is answered `refuse` 3; no fix is sent, no delegation composed, no T1 or human task submitted; an accept that would create a draft answers `503 store_unavailable`, or `409` naming the Storage page. G19 (7) and G22 assert that the fabric acknowledges the frame. |
| C6 (S18, the contact set) | The stricter form: Layer 5's `pinned` flag keeps its one meaning ("never pruned") and gains no trust meaning. The third value of `heal.fixes.receive` is `named_contacts`: an explicit NodeId list, `heal.fixes.senders`, in `state/heal.json`, written only by the loopback-only settings route; the setting's text lists the names it holds. `paired_hubs` stays as the lead ruled (the home hub plus the Layer 7 paired hubs). No Layer 5 amendment. Listed for the owner (the brief's value was `pinned_contacts`). |
| C7 (I32 with O16) | Three bounds, one sentence in §11.2: the send screen shows the sealed size and warns above 2,937 B; the sender refuses only above the `tcp-lan` frame; a carrier's refusal stays Layer 7's (`forward_too_large` on the hub, and the fix reads "not delivered"). G22: a fix of at most 2,937 B sealed from member A reaches a named contact on the far island and is stored; a larger one is counted on A's hub. |
| C8 (S8, `revert_blocked`) | A revert that needs a stale consent writes `phase: blocked` and releases the node-wide lock. At start `blocked` means no restore. The human resolves it on `POST /api/v1/repairs/attempts/{id}/restore` (the consent round trip, then the restore; session or loopback, never the agent's bearer) or by dismissing ("leave as is"), which removes the file. Attempts in `revert_blocked` and `revert_failed` are never reclaimed until resolved or dismissed. |
| C9 (O3 with O4) | One incident per failing child: `child_exit`. The `Degraded` transition is a state of that incident (severity `error`, T0 run again at that change, readable by `service_state is degraded`), not a second fingerprint. `child_degraded` leaves the incident reasons. G19 (1) asserts exactly one incident after five kills. |
| C10 (the evidence hash) | Defined once, in §3: BLAKE3 over (the fingerprint, the sorted `[check_id, result]` list, the exit kind as code or signal, the set of satisfied cause ids). Never excerpts, counts, sequence numbers or times. G21's flap test uses a child whose log line differs on every run and still makes one model request. |
| C11 (I22, the UI budget) | Layer 9's lazy allowance is what Layer 8 leaves: 5.1 KB gzip (default, to be measured). When the measured chunk exceeds it, the page ships what fits (the incident list, "Needs you", accept, undo, dismiss) and the sections that do not fit are held back from the v1 UI and stay reachable through `111 heal` and the API; the figure and the overage are recorded on `9-evolution/TASKS.md`; raising D21's total is the first item of "Open for the owner". The budget gate is never left red and never loosened by a build. |
| C12 (record states left unplaced) | `sent` fixes are reclaimed after decided fixes (a vouch for a reclaimed fix is dropped and counted). An attempt with no incident is reclaimed in the closed-incident step by its own `seq`. The proposed actions and the `action_hash` live in a `heal/proposals` record (`undecided`, protected; `decided` goes with the proposal), written in the same J9 call as the proposal; when its incident closes, `111d` marks the proposal as the check proposes. |
| C13 (missing owner marks) | O27, S14, S9, S28 and S6 are "listed for the owner in the spec's deviations section", with one line for the battery read. For S28: §11.5's ladder in v1 is `local → accepted_by_n`, with `reproduced` reserved. |
| C14 (§0 rows) | The S3 and S12 rows end "and §0's row for 'share proven fixes' is rewritten to say exactly this"; the safety author owns §0's table and the other two authors add to it without rewording its rows. I21's "closed by" reads "the opt-in hub probe of §22"; "not on the Pi floor" stays the v1 limit. |
| C15 (the lock-amendment list) | Added to "Lock-amendment candidates": Layer 4 item 10 widened (the `device` vocabulary text and card gain the three bridge actions) and the agent's tool schema; a mesh111 read seam "evictions a quota change would cause" (S7's count); a request to Layer 8 §12.3 that a consumer receives (the authenticated sender NodeId, the attributed-link flag, `msg_id`, the body); `LICENSING.md` and the fixture list in the cronie removal. |
| C16 (the subject's domain) | `@subject` is a member of the compiled subject table and nothing else; `plan()` must resolve it. `component.enable` is addressed by component id and calls `enable_component`; an adapter that enables two components needs two proposals (the one-consent rule). |
| C17 (a peer's actions) | A peer's `service.restart` is accepted only for the open incident's own subject; a peer's target field is never used, and a mismatch drops the action to "their words". A `human_step` or `advise` shown with a button is always this node's own playbook text, selected by `cause_id`. |
| C18 (names) | Refuse reason 6 `request_changed` (never retried); the member's text for reason 5 is stated. Reasons `(1, 111d, witness_absent)` and `(1, 111d, apk_outdated)` join the compiled table, each with a shipped playbook whose one action is the `human_step`, closing when the condition clears. The debug pause points are `apply`, `keeping`, `decided`, `restoring`, `restored`. |
| C19 (the task route) | The fold applies to modes `diagnose` and `explain` only; `kb_draft` and `patch` proposals are ingested normally. Where no agent runs, `diagnose` runs T0 again, clears the ledger block, re-sends the delegation and answers 204. `explain` is a `diagnose` task with `incident` and `narrative: true` (Layer 3 item 7). An unavailable mode answers `409` with an existing code (`needs_hub`, `component_unavailable`). |
| C20 (fixes that read two ways) | `heal_remove` is a consent-gated action for S15's count of one. `unresolved` is set when a new unrequested event bumps the incident's `count` after the keep. S14 is structural in the lint and the loader (a cause's `when` names a check with a leaf that is not `log_contains`); at run time a match on `log_contains` alone files `advise`. O2 governs service rows, and the Layer 8 sources are named one by one. The rest as the check proposes. |
| C21 (the fixture home) | Q9's home stays. `build.py --selftest` resolves the playbook cases from the repository root and prints `skipped: playbook fixtures not present` when the folder is absent; the CI job that checks the knowledge base gains `build.py --selftest`; the Layer 0 amendment (item 1) names the path. |
| (the drafter's note 1) | One writer for a class quota: Layer 8's `quota.set`. Layer 9's typed action calls that function; it has no second path to Layer 5's quotas. |
| (the drafter's note 2) | `author_seq` and `voucher_seq` are minted from the Layer 2 announce counter, as Layer 8's catalogue `seq` is (L8 D6), so a words-only restore cannot restart them and S11's `high_water` never drops an author's later fixes. |

## Merged findings (one fix each)

| ids | ruling |
|---|---|
| S20 = I25 = O23, + O22 | Accepted. Re-pin every citation at the lock head (the draft's `fa1daeb` is behind HEAD in committed code, not only in the working tree) and say so in §24 C12; I25's list and O23's three are the known moves (`N/kb.rs:32-34` → the codes at `N/services/mod.rs:1025`, `:1031`; the health components at `status.rs:326-336`; `payload_budget` takes five arguments, so §12.2's gate is the sum of the per-fragment budgets). The observer's wake (O22, I25's second half): it awaits the registry's own change signal (`N/services/mod.rs:1449-1450` at HEAD) and each child's watches; where it needs a bus frame (the `status` health state, Layer 8's `storage`) the frame is a wake-up only and the observer then reads the typed state (`Registry::snapshot()`, the health struct); it never parses a frame. §21 notes the `status` heartbeat wake (at least every 30 s, inside L7 §3.7's line). |
| O3 + O28 + I13 (b, d) + I14 | Accepted as one vocabulary with one source. (1) One subject id space: `ServiceId`, component ids, and the fixed daemon subjects §3.4 names (`111d`, `updater`, `identity`, `heal`, Layer 8's class subjects), each in the compiled table. A supervised child is observed only through its watch and its exit facts; its subject is its service or component id and its layer that service's layer (the model child is `model`, never `llama-server`). Rows whose code is `child_backoff`, `child_degraded`, `child_stopped`, `spawn_pending` or `transport_degraded` are not incident sources. `no_identity` and `restore_pending` open once, under `(2, identity, …)`. G19 runs with a real supervised child and counts one incident per failure. (2) One compiled table of `(subject id, reason code)` in node111, with a test that every emitted reason literal is in it; that test writes `0-knowledge/_meta/playbook-vocabulary.toml` (committed; CI fails on drift); `build.py` reads it; N06 renders the block's `[match]` into the expected filename and compares. (3) One fixture directory read by both implementations, at OB Q9's home beside the Rust loader's tests (`1-bootstrap/crates/node111/tests/fixtures/playbooks/`, never in the payload; Q9 is D35's default, I14's single directory is kept): `build.py --selftest` and a node111 test run the same accept and reject cases, and a node111 test loads every file under `0-knowledge/playbooks/` and fails on any refusal; both report under `[L0-L9:evolution]`. |
| O14 = I12 | Accepted as one Layer 0 amendment (§23 L0; amendment item 1 widened). The applicability matrix gains a `playbook` column: title, category, status, summary, tags, last_verified R; aliases, sources, relations O; platforms, upstream, license forbidden. `relations` carry `depends_on` only when the subject is a third-party project with a KB entry (then it is required); `runs_on` is optional; neither enters N06; no relation names a target without an entry (R02). The gate adaptation line (`L0:91`) gains: a `candidate` playbook is lint-clean with `### Alternatives considered`; `testing` adds a `### Smoke test` on a real node; `proven` a `### Field log`. The schema's scope line ("six category folders"), the root index and `0-knowledge/AGENTS.md` join the amendment. |
| O7 = I9 | Accepted. The saving is not a fact: nothing in the tree installs the crontab line or starts `crond` (the provisioner pushes only the script, `NI/install/phone/mod.rs:24`, `:224-229`). §1 rule 2 and §21's Termux row say "nothing in the tree schedules it today" and claim no saving; whether a hand-provisioned phone runs it is recorded on the owner's real-device run, not a lock condition. The removal list in §23 L1 is both reports' union: `termux/cron/*`, the `CRON_HEALTH` include and its install step, `termux/tests/dryrun.sh:99-110`, `L1:156`'s text, and the copy already pushed to `$PREFIX/var/lib/111/share/termux/cron-healthcheck.sh`, which the provisioner deletes on its next run (`111d` does not delete it: the O5 vs I2 rule below, and it is inert). The `cronie` deb leaves the Termux closure (`L1:68`): the script and the manifest fixtures are its only references in the tree; its deb size is then the measured saving. Listed for the owner in the spec's deviations section (O7 asked for the owner's word); a lock-amendment candidate below. |
| O20 = I23 (a) | Accepted; one name. On Termux an inbox line with signal 9 opens `(1, 111d, killed_by_os)`; the reason joins the compiled table; the first release ships its playbook, whose one action is the `human_step` "On your computer, connected to this phone, run `111 install --phone` again" (OB §3 "The phone"), one card with the §12.6 step when both are due. |
| O24 = I28 | Accepted as one witness rule (§3.3): code and signal at most 3 digits; a signal name is mapped from a compiled table, `-` otherwise; the Termux hook writes `result=exit` or `result=signal`; `boot=-` when `/proc/sys/kernel/random/boot_id` cannot be read *(unverified from this box on Termux)*, and §3.3 says that `unclean_boot` is then not told from `unclean_exit`; `111 heal-witness` exits 0 on every error, so `ExecStopPost=` never fails the unit; the file keeps the newest 256 lines (J2; I1 (a)). |
| I4 = O25 | Accepted in I4's first form, inside Layer 8's one hand-off rule: `111d` records on the incident the id of the T1 task it submitted; at ingest (`task.done` or a sweep) a proposal whose `task_id` names such a task is folded into its incident (one J9 `put`, committed before the unlink) and the spool file is unlinked; it never enters `agent/proposals`, any proposals list or the open count. A hub folds a delegated task's result into its delegation record the same way. G21 (3) asserts `GET /api/v1/proposals` unchanged after a T1 run. |
| S23 + O12 + O26 + I20 | Accepted as one delegation life (§12.2–§12.5). **Path:** `send_op` gains `direct_only`; an `AGENT_OP` is sent only on a link attributed to the home hub and is never put in custody or a mailbox (O26 (a), I20). **Re-send:** a request with no reply is sent again, same `req_id`, on the next link-attribution event to the home hub; a card update that adds bit 6 while an undiagnosed incident is open sends the request; a `refuse` 3 (`busy`) is retried on that same event (I20). `req_id` is 8 bytes from the OS random source (S23). **The hub's stored reply:** one record per enrolled member, `{member, req_id, request hash, fingerprint, checks hash, reply}` and nothing else of the request (O26 (c)); the same `req_id` with the same hash gets the stored reply, the same `req_id` with another hash gets `refuse` (S23), and a new request whose fingerprint and checks hash equal the stored ones gets the stored reply with no model call (O12). **Bounds:** at most 4 delegated requests queued on a hub in total, beyond it `refuse` 3 (O12's number; default); a delegated model run starts only when no local task or chat turn waits (O12); stored replies, one per enrolled member (the latest; the lead's default), reclaimable, are named with the per-author fix queue in §4's table. `refuse` 5 when the member's check ids are not all in the hub's playbook (O26 (b)). G23 asserts the re-send, the stored reply, the total bound and the hash refusal. |

## Forks the reports leave open, or where two reports disagree

| ids | ruling |
|---|---|
| S12 (code and kb fixes on the wire) — the lead's ruling | Cut from the v1 wire: `code` and `kb` fixes travel as drafts and reach other nodes through the human's pull request (§9). `fix/v1` in v1 carries `class: "typed"` and `attachment: null`; the signed input keeps `attachment_hash` (32 zero bytes) so a later release adds attachments under the same tag. A body from a trusted author with another class or an attachment is parked "needs a later release" (S24's word), never applied or rendered as a diff. §11.4 step 4 and the "Submit upstream" path for a received fix go; §19 rule 6 reads "`111d` signs only a body it built from its own records; v1 carries no attachment"; D5 is dropped as moot; S24's two-hash rule stays for the release that adds attachments. Listed for the owner in the spec's deviations section (F10 A sent `code` and `kb` classes). |
| S18 (who may send a fix) — the lead's ruling | Each `heal.fixes.receive` value is an explicit set: `home_hub` = the enrolled `home_hub` NodeId; `paired_hubs` = the home hub plus the Layer 7 paired hubs (offered on a hub only); `pinned_contacts` = that set plus every contact whose Layer 5 §7.1 `pinned` flag is set (a human sets it through `PATCH /api/v1/contacts/{id}`), and the setting's text says that pinning a contact lets it send fixes. §11.4 step 3 cites the three sets. |
| S19 (`@subject` and the actions it cannot name) — the lead's ruling | The three bridge actions (`bridge.edge.pause`, `bridge.edge.set`, `bridge.order.set`) and the two interface actions (`rns.interface.upsert`, `rns.interface.remove`) are "human or `device.propose` only" in v1: no playbook cause, `repair` proposal or peer fix carries them; they reach a node through a human's own act or an agent `device.propose` (Layer 6 §13; the bridge actions join its closed vocabulary under amendment item 15) that a human accepts. `@subject` is an id `111d` minted (it passes `valid_id`) and must resolve to an existing object in `plan()`. §13 item 5's "playbooks for edges" goes. Listed for the owner in the spec's deviations section (F12 A grew bridging "plus playbooks"). |
| S27 (`update.rollback`) — the lead's ruling | Leaves the v1 action set: its usable window is about a minute (the daemon confirms after 60 s of green and asks for its own revert after 60 s of red, `N/mod.rs:137-180`). The `human_step` stays: it names Layer 1's own probation revert and the monotonic floor. §7.2's row, §7.5's special case, §2.3's exclusion entry and G20 (7)'s first half go; the action joins §22; I16's positive-path test is moot; D2 and §24 "Open for the owner" (3) are replaced by this ruling. Listed for the owner in the spec's deviations section (F6's vocabulary named it). |
| O5 vs I2 (the witness on a node that updates itself) | O5's fix, the stricter (rule 4), on both platforms: `111d` never writes its own unit or its runit hook; a file the provisioner or the installer wrote outside the payload is not restored by the update guard's revert, so `111d` changing it would change the device outside a signed release flip. §3.3 states the delivery: the witness arrives only with `111 install` (fresh or re-run) and `111 install --phone`. At start `111d` reads the unit (Linux) or compares the hook's bytes with the release's copy (Termux); without the witness it classifies from the run marker alone (evidence `witness: absent`, so `watchdog_kill` is not told apart) and files one `human_step`: "Re-run `sudo 111 install` to add the exit witness" on Linux; on a phone the §12.6 step, one card when the APK step is also due. No drop-in written by the root helper in v1. G19 asserts both cases; `check.sh`'s `systemd-analyze` stage and `termux/tests/dryrun.sh` assert the line and the hook's output format (I2's gates). Listed for the owner in the spec's deviations section (F2 A assumed the witness on every node). |
| I3 vs O15 (who files a `kb_draft`) | O15's producer, the stricter (never automatic; no new tool): a human's "Draft a KB note" on an incident starts a `propose` task with `kb_draft: true` through the incident task route (I5 row); `111-agent` code writes the frontmatter and the six headings from typed fields and the model fills the prose; the entry is `candidate`, `### Alternatives considered` left marked for the human, no `### Smoke test` (`L0:84` asks a human-attested one), the incident's procedure under `## Details`; `PR.md` says the lint did not run on the node. I3's checks apply to that draft: `target` matches Layer 0's filename rules N01–N06 and a category folder, the body is within B09's budget, and `111d` re-validates `target` at accept, before the mbox names a path. The tool count grows by two (§18.3), not three. Listed for the owner in the spec's deviations section (F12 A's draft carried a Smoke test generated from the incident). |
| O4 vs I13 (c) (the reason of an unrequested exit) | One reason, `child_exit` (O4): the code or signal stays in the `exit` evidence and `exit_is {code \| signal}` tests it, so a child that dies with varying codes or signals is one incident and `repair-<subject>-child-exit.md` matches every unrequested exit of that subject. `child_signal` is not added. |
| S4 with I8 (the undo file at start) | One rule set for §7.4–§7.5, readable before the store opens. The undo file carries `phase` and S4's binding fields `release` and `starts` (the run marker's). A keep (I8's order): rewrite the file with `phase: keeping`, then `decision_complete(kept)`, the attempt, the ledger, the snapshot record, and remove the file last. A revert (S4): rewrite with `phase: restoring`, restore, `decision_complete(reverted)`, the attempt, remove. At start, before serving and with the store closed: `armed` and bound to the previous run ⇒ restore through the trait's new `restore_cold(Bytes)` (per action, a write made before any child starts), folded as `reverted (daemon restart)` through the decision calls once the store opens; `keeping` ⇒ finish the keep, no restore; `restoring`, or a `release` or `starts` that is not the previous run's ⇒ no restore, `revert_failed`, critical, the human's; a `pending` decision with no undo file ⇒ `interrupted` (I8). `service.restart` (no undo) records `interrupted`. `apply_unrecorded` follows the same rule with `proposal_id: null` (S5). G20 (3) gains the second-restore case, the stale record and both I8 cuts, driven by `NODE111_HEAL_PAUSE_AT` (names row). |
| I1 (c) + I6, with S8, S11, S24, O9, O17 (the record states and the reclaim) | One table per record kind in §4 (I6), its `state` column being the store's, the display phase beside it, the transitions and the terminal states: incident `open`, `closed` (a closed incident re-opens when its fingerprint recurs and keeps its record, O9); attempt `armed`, `awaiting_confirm`, `kept`, `reverted`, `revert_failed`, `revert_blocked` (S8), `interrupted` (I8); fix `undecided`, `parked` (only S24's "needs a later release" from a trusted author; an unknown author leaves no record, O17), `refused`, `decided`, `sent`; delegation `open`, `answered`, `refused`, `cancelled`; draft `open`, `exported`; one per-author `high_water` record in `heal/fixes` (S11); ledger and snapshot records. The display phases are §14.2's eleven plus `awaiting_confirm` and `revert_blocked`, recorded in §24 against OB §3's "eleven". Layer 9 registers one reclaimer for `heal/*` (J9). Never reclaimed: incident `open`, attempt `armed` and `awaiting_confirm`, fix `undecided`, delegation `open`, draft `open`, `high_water`, and every ledger record (the loop guard, bounded by the closed vocabulary times this node's targets). Order, oldest `seq` first within each step: closed incidents with their attempts (J3; an attempt goes only with its closed incident), then decided, refused and parked fixes, exported drafts, closed delegations and the hub's stored replies, then the audit stream (J3), then kept snapshots (an Undo card then reads "no longer kept, cannot undo"). "A snapshot goes with its incident" is struck: a snapshot belongs to its kept change, the last five by capacity (Q7). |
| O17 (what a fix from outside the trust set leaves) | Dropped and counted, no record (O17's first form); with S11 a frame whose authenticated sender is not the body's `author` is dropped the same way; the ring of 16 for unknown authors is not built in v1. Why: bytes from strangers never take room from incidents in the `heal` class. Listed for the owner in the spec's deviations section (F10 A parked them "unknown author"). |
| I5 with S12 and O15 (the contract's routes) | I5's Shapes subsection is accepted field by field (`applied` a `oneOf` inside `decision`, `attempts[].id`, the `awaiting_confirm` state), with I29's bodies (`RepairsSettingsPatch`, `{to, accept_warning?}`). Routes: `GET /api/v1/repairs/drafts`; one incident task route, `POST /api/v1/repairs/incidents/{id}/task {mode}`, `mode` ∈ `diagnose` (ask again, §7.7), `explain` (§5.1), `kb_draft` (I3 vs O15), `patch` (an opted-in hub, §8.3); it answers 202 with the task id and `409 busy` while one runs for that incident; a human only. It replaces I5's `/diagnose` name. `POST /repairs/fixes/{id}/draft` is not added (S12: no received fix carries code or kb in v1). The issue URL is a field of a draft and of a kept typed change's export, typed fields only (S13). |
| S21 (`service.restart` in two kinds) | The `device` kind keeps Layer 6's one-shot with no probe (D29 byte for byte: a locked behaviour is not changed for a Layer 9 need); in a `repair` it carries S16's probe. `identify {device_id}` joins §7.2 as a `device`-only one-shot row. |
| S10 (when vouch kind 4 leaves) | Kind 4 `reverted` is sent once, when the accepted fix's own loop reverts it (a failed probe, a closed confirm window, armed at start); a later human Undo sends nothing. Why: the vouch reports the fix's validation on this node, and an Undo is a local choice with many reasons. The `fix` card's fixed text says exactly this; G22 (2) reads "no fix leaves". |
| I32 with O16 (a fix across islands) | Gated, not struck: G22 gains one assertion on Layer 7's G9 topology: a typed fix whose sealed put is at most 2,937 B crosses one carrier hop and files on the far node; a larger one is refused at send with "too large for a carrier" (O16's bound) and nothing is put. Why: the sentence carries README §9's "through the gateways", and Layer 7's G9 exists before Layer 9 builds. |
| O18 (the default phone notification) | `heal.notify` defaults to `off` until the `termux-api` deb is in the pinned closure, which is the owner's word (a runtime dependency on a phone's required configuration, AGENTS.md); the setting does nothing on Linux or where the helper is absent, and says so; when on, one notification when the Needs-you set goes from empty to non-empty (O29). Listed for the owner in the spec's deviations section (OB §3's default was `transitions`). |
| I17 (the Pi-floor ceiling) | Processes and listeners equal the Layer 8 baseline, asserted. Idle RSS is asserted under a ceiling of the Layer 8 build's idle RSS plus 512 KiB (default, to be measured; it bounds the binary delta's resident pages and the held `[match]` table), recorded in `9-evolution/TASKS.md`; §1 rule 3 and §21 say "0 beyond the binary delta" for the heap and the ceiling for RSS. `observer.sh` starts its own daemon and runs `zero-weight.sh` against it; the no-timer line depends on Layer 7 row V1, and `REQUIRE_G7=1` fails while `idle.sh` is a stub. Listed for the owner in the spec's deviations section (I17 left the number to the owner). |
| I22 (the lazy UI budget) | Layer 8's I10 ruling, applied to this page: §14.1 states the figure at the lock head (238.9 of 256 KB after Layer 7, `7-relay/TASKS.md:88`) and the allowance shared with Layer 8's Storage page, splits the advanced sections (Audit, Playbooks, Fixes) into their own chunk where that saves bytes, and the UI check asserts `pnpm budget`. If it cannot fit, raising D21's lazy total is the owner's: listed for the owner in the spec's deviations section, never assumed. |
| S7, I16, I19, I31 (names and numbers the reports leave open) | `ConsentRequired.action` `heal_remove` for S7's removal consent. `NODE111_HEAL_PAUSE_AT=<apply\|keep\|restore>`, debug builds only (the `NODE111_BRIDGE_STUBS` pattern), for I16's and I8's cuts. The must-not-break suite's first release holds 50 cases (R04 §1's figure; default). The agent finds `share/agent/` through `AGENT111_DATA_DIR` (as `AGENT111_KB_DIR`), which a test run sets to the repo's copy; there is no compiled-in fallback. The CBOR crate is `ciborium` (already linked) and a non-canonical map is refused. §3.6's on-demand battery read is dropped: nothing consumes it. |

## Accepted with the report's fix (named here because a builder must not miss them)

Blockers (I2 and I3 are ruled above):
- **S1** (§7.3, §16 item 15, §23 L4, G20): an action inherits the gate of the route it wraps; accepting a proposal that holds an action on a
  loopback-only key (`fabric.listen_lan`, `fabric.hub_role`, `fabric.quota.<class>`) from a non-loopback peer answers `403 loopback_only`, writes
  nothing and leaves the proposal open. Layer 8's I32 states the same rule for `quota.set`. A lock-amendment candidate (Layer 4).
- **S2** (§7.3, §17.3, §19 rule 1, G20): `repair`, `fix` and `human_step` are written only by `111d` through J9; the spool ingest refuses these
  kinds and treats the file as Layer 8 §2.4 rule 4 treats a file it does not ingest (never ingested, never deleted by `111d`, counted); at accept
  `111d` takes the actions from its own `heal/` record for that proposal id and requires their hash to equal the `action_hash` recorded at
  `proposed`, else `refused`. G20 asserts both.
- **S3** (§11.4 step 6 replaced by one normative peer-subset table, §12.5, §19, G22, G23): from a peer only `service.restart`, `advise`, and an
  action byte-equal to this node's own playbook action for the same `cause_id` (targets bound by `@subject` here, never taken from the peer);
  everything else, `human_step` included, is shown as "their words, not checked", preformatted, with no accept button; a typed fix files a proposal
  only while this node has an open incident with the same fingerprint, else "does not apply here"; the sentence at `:731` is struck. Listed for the
  owner in the spec's deviations section (narrows F10 A's and F11's "re-validated against this node's own vocabulary").
- **O1** (§3.2, §3.4, G19): `ExitFacts.requested`, set from the stop and restart flags after the loop reads them (`N/supervise.rs:976-985`);
  only an unrequested exit opens an incident; `Stopped`, a disable and a held runner open nothing; the observer stops before `layer6.shutdown()`;
  G19 asserts that a requested restart, a component disable and a graceful stop open nothing.
- **O2** (§3.4, §0): only a row in state `error` opens an incident, never `unset` or `starting`; `carrier_pending`, `kb_absent` and `codeagent` are
  struck; §0's scope table says Layer 7 has no incident source in v1 until it publishes an `error` code.
- **I1** (§4, §7.4, §7.5, §23 L8): (a) the 4,096 B inbox rule is struck; J2 as Layer 8 v0.2 words it (the newest 256 lines, through a temp file and
  a rename) is the one rule; (b) the `Full` rule: observation continues in memory under the 64-incident bound, no new `repair` proposal is filed,
  `RepairsStatus.store = {state: full}`, an armed apply finishes and its decision is written (J9 writes decisions whatever the class's state), its
  attempt once the class has room; (c) as ruled in the record-states row; (d) §7.4 and §7.5 name J9's three decision calls. Re-checked against the
  Layer 8 text that locks.

Majors whose fix touches a seam with another layer:
- **S5** (§7.4, §7.6, §7.7, §4; L6): one apply at a time on a node; the lock is held from arm to keep, revert or confirm; every accept and every
  `apply_unrecorded` in that span answers `409 busy`; `apply_unrecorded` (the human's `PATCH /transport/posture`) arms too, `proposal_id: null`.
- **S6** (§7.7, G20 (6); L6 snapshots): Undo is offered and runs only while the current state hash equals the `after_hash` recorded at keep, else
  "changed since, cannot undo"; its probe asserts the restored state is in effect; an Undo arms its own undo.
- **S7** (§7.1, §7.2, §7.3; L5 quotas, L6 components): a "Not undone" column in §7.2, shown on the card; when a `plan()` returns
  `removes_capability` or an eviction, accept answers `409 consent_required` (`heal_remove`) with a text `111d` writes and its hash: the
  mechanical form of OB Q6.
- **S8** (§2.2; L6 components, edges): an automatic revert never passes a consent gate; a revert that needs a consent that is not current stops as
  `revert_blocked`, shown in Needs you.
- **S9** (§7.6, §18.2, G20 (4); L4): the confirm carries the accept's session (or, with trusted loopback and no session, its peer address),
  recorded on the attempt; the card says the confirm proves "you looked again" for the two fabric keys; `would_lock_out` and its G20 line move to §22.
- **S11** (§11.4 steps 2–3, §4; L5): the frame's authenticated sender must equal `author`; one `high_water` record per trusted author; a vouch is
  accepted only from a node this node sent that `fix_id` to, the highest `voucher_seq` per (voucher, fix) wins.
- **S13** (§9, G24 (4)): the renderer rule (subject one line, control characters stripped, at most 72 B; `>From `; trailers in untrusted text
  indented; a fence longer than any run inside; the issue URL percent-encoded with typed fields only); `git mailsplit` yields exactly one patch
  from a body built to smuggle a second.
- **S14** (§6.2 lint and loader, §5.1; L0): a cause whose action is not `advise` or `escalate` needs one satisfied leaf that is not
  `log_contains`; when several causes matched the enum is those causes; when none matched it adds `unknown`, and a cause chosen with
  unsatisfied checks files `advise` only, marked `model_only`.
- **S15** (§7.2; L6): `component.enable` calls the function the bridge settings route calls, by adapter id; a proposal holds at most one
  consent-gated action; an adapterless component (Layer 8's D1 + D2 vs I3 ruling: kiwix, opencode) has no adapter id and is not a `component.*`
  target in v1.
- **S16** (§7.2, §7.7; L1): the probe is the published row out of `error` (the registry's two-tick confirm) with no new exit fact whose
  `requested` is false (O1) inside the window; an applied action whose incident did not close counts `unresolved` and blocks the same proposal as
  a revert does.
- **S17** (§8.3, G24; L3 item 8): the canary file is never in a stub-model request or a diff; the `patch` prompt's inputs are the incident in an
  untrusted block and the note, nothing from the journal.
- **O6** (§3.3 rule 5; L1): a table over (marker, inbox); any inbox line whose result is not `success` opens or bumps `(1, 111d, unclean_exit)`,
  also under a clean marker; `starts` is the old marker's value plus one, read before the rewrite; start facts close `self_recovered` after the
  daemon has served for `crash_loop_window_secs` with no new line, and never start a model task by themselves.
- **O8** (§3.1, §24; L1, L6): the netlink row is struck (its effect arrives as the `radio` and `transport` rows and the `devices` frame); run-time
  children are handed over by a generation `watch` that `Supervised` and `Components` publish, then the observer re-reads the control list: no
  socket, no poll. Listed for the owner in the spec's deviations section (F2 A named the netlink sockets).
- **O9** (§5.1, §3.4, G21; L3): T1 runs only when a loaded playbook matches the fingerprint; one T1 task per incident record; a re-opened
  fingerprint reuses its record and its diagnosis until the evidence hash changes; T0 runs at open and at each `count` change, never on a timer;
  ten open and close cycles make one model request.
- **O10** (§5.4, §12.4, §1, §17.1, §22, §24 C13; L3): "loads the model on demand" and `--sleep-idle-seconds` are struck; T1 and the hub's T2 use the
  llama-server whose row is `live`; Layer 9 starts, stops and reloads no model; idle unload is Layer 3's question (§22). Listed for the owner in
  the spec's deviations section (the brief assumed an idle unload).
- **O11** (§6.2; L1, L3): `restarts_at_least {id, n}` compares `ExitFacts.restarts_in_window`, `within_events` is cut; `health_is` for the agent
  takes `ok` and `error`; evaluating a predicate makes no request and reads only state `111d` holds, plus one file tail. Listed for the owner in the
  spec's deviations section (narrows F5 A's grammar).
- **O13** (§3.5, §12.1 key 9, §17.2; L5 wire): off the node goes only text 111 wrote (the row's reason, exit facts, check results, a matched
  literal ≤ 64 B); key 9 becomes "matched literals ≤ 3"; on the node an excerpt is the matching line, else the last line before the exit, ≤ 512 B;
  the `heal_delegate` consent is reworded to match. Listed for the owner in the spec's deviations section (F11 B sent three excerpts of ≤ 120 B).
- **O16** (§11.2, §0; L7): three bounds, one `tcp-lan` frame, 16 Tier 1 fragments, 2,937 B sealed through a carrier; the send screen shows the
  sealed size and "too large for a carrier". Listed for the owner in the spec's deviations section and §0's row (README §9 "through the gateways").
- **O19** (§17.1, §23 L1, G20 (8); L1 CLI): `111 heal settings [<key> <value>]` calls the loopback `PATCH /repairs/settings`, prints the consent
  text and asks y/N. A lock-amendment candidate (Layer 1).
- **I7** (§4.1 "Start and degraded modes", G19 (7); L8): at start `111d` lists `heal/incidents` in state `open`, re-arms each recovery window and
  closes those whose condition is clear; with the store closed the observer keeps at most 64 incidents in memory, files no proposal and `GET
  /repairs` names the reason; under J7 no proposal is filed (observe and diagnose only) until the store exists.
- **I10** (§12.6, §4, §23 L1 item 4; L1 phone flow): after the APK step the provisioner pushes `state/apk.json` (`{schema, apk: {package:
  sha256}}`, 0600) with `runas::install_file`; `111d` compares it with the manifest's `apk.*` pins; absent file, no `human_step`. Listed for the
  owner in the spec's deviations section (F11 named a `receipt.apk` map).
- **I11** (§8.2, §8.3, §23 L6, §23 L3; L6, L3, L8): opencode is declared in Layer 8's `adapter`-optional form plus a `[run]`-less tool form and a
  `requires.hub` key; `111d` passes `--opencode-path` when the component is enabled; a `patch` task runs only when the `[source]` blob for the
  running release's pin is present (Layer 8 section below), else a `human_step` names the install verb.
- **I13 (a), (e)** (§6.2): the layout (`[match]`, `[[check]]`, `[[cause]]`, the action tag, `verify`'s type, severity's place, `layer` a number)
  as a table plus one valid TOML example; `escalate.task` ∈ `diagnose`, `patch`.
- **I15** (§0, §11.2, §12.2; L5): in v1 a fix and a delegation travel over `tcp-lan` only, the one link built; the size gate is written for the
  links later layers add and proven on mesh111's `lossy` link; G23 (5) moves to mesh111 `tests/send_op.rs`. Listed for the owner in the spec's
  deviations section and §0's rows (F10 A and F11 B named 233 B bearers).
- **I16** (G20, test homes; L6): G20 (1) compares against a tape of the Layer 6 applier's files and decision records recorded with G5's fakes
  (`6-transport/verify/devices.sh`) at the lock head, under `9-evolution/verify/tapes/`; one round trip per new action (apply, a forced probe
  failure, state equal to before).
- **I18** (§15): each of items 3–8 lands its own routes, `openapi.yaml` rows and pin-list entries; item 10 is the Repairs page and the two copy
  lines only.
- **I19** (§13 item 2, G25 (3); L3): cases under `3-intelligence/crates/agent111/tests/evolution/cases/` (`{id, class, request_hash,
  recorded_reply, expect}`); the gate is every outcome equal to `expect`; a change that alters a `request_hash` fails that case until it is
  re-recorded on the real model, and the re-recording is the release's eval report (Q20).
- **I21** (§0): two rows, "the local agent continuously observes" / "the autonomous monitor of system health" (v1: `111d`'s code observes; the
  model enters at T1 only; a monitoring model is not planned for the Pi floor) and "through a nearby hub" (v1: the enrolled home hub only;
  closed by hub-to-hub delegation). Listed for the owner.
- **I23 (b), (c), (d)** (§24, §11.4, §14.1, §2.3): the PR switch is carried as `heal.code_drafts` and the "open PRs from this hub" switch arrives
  with the hub token, listed for the owner in the spec's deviations section; a typed fix from a contact is never accepted by one tap, always from
  the full review screen; `radio.region.set` and `rns.interface.*` join §2.3's grant exclusions, "overridable by the owner".
- **O27** (§5.2, §5.3; L3 item 7): `evidence` is constrained by the schema to the incident's evidence ids; `quotes` is dropped; the card shows the
  excerpt of each cited id.

## What Layer 9 takes from Layer 8's triage

- **L8 S12 + I7:** J9's `decision_begin`, `decision_complete`, `decision_reopen` are the only writers of decisions; §7.4, §7.5 and the S4-with-I8
  rule name them; a `pending` decision is never reclaimed; with the store unavailable the decision route answers `503 store_unavailable` and
  applies nothing (I1 (d), I7).
- **L8 S10 + S11 + I8:** the one hand-off rule (ingest on every terminal task event and at the sweeps, `FULL` before the unlink); Layer 9's two
  changes sit inside it: the T1 fold (I4 = O25) and the refused kinds (S2), handled as §2.4 rule 4's file that is not ingested.
- **L8 S2 = I4:** the housekeeping interval has one daemon-level owner; Layer 9 adds no work and no interval to the pass; `heal` reclaims only in
  the pass's step 4, through the reclaimer Layer 9 registers.
- **L8 I13 + S24:** J9's per-namespace reclaimer, `limit` and `seq` cursor, a new `seq` on every write: Layer 9 registers one reclaimer for
  `heal/*` (the record-states row); the Repairs list routes page with the cursor at ≤ 1,000 rows; an incident id never derives from `seq` (I26:
  `inc-` plus 12 random hex).
- **L8 S7, S8:** the `heal` class is `full` while its export offer is unanswered (I1 (b)), and reclaims only when the budgeted classes exceed `keep`
  or `disk_low` is set: the shares order the reclaim, they do not trigger it.
- **L8 I14:** a consumer returns after its own durable write; Layer 9's consumers for 0x10 and 0x13 write their J9 record before returning, so the
  fabric's acknowledgement follows it.
- **L8 D7 + I15, with S1 = D3 = I1:** the release's source tarball is the `[source]` pin and lives in `state/origin/blob/<sha256>`, not in
  `cache/packages/opencode/` (§4's path table); the updater of a node with `origin.serve` on keeps it current; otherwise `sudo 111 install
  --components opencode` fetches it with the same origin order, sha256 and size checked before the rename into `blob/` (an existing blob never
  replaced) and its bytes counted in the metered window; a `patch` task checks it against the running release's pin (I11).
- **L8 D1 + D2 vs I3:** `adapter` is optional and an adapterless component's state and log directory is `state/<id>/`; opencode uses that form (it
  keeps no state there: its `HOME` and XDG directories are in the task's work directory, §8.1); Layer 9 adds the `[run]`-less tool form and
  `requires.hub` to the same Layer 6 §2.1 block (I11).
- **L8 I5 vs D29:** bit 6 `ROLE_HEAL` is Layer 9's and bit 7 `ROLE_DATASETS` Layer 8's, both set by node111's `roles()` beside bits 3–5; bit 6 is
  public to every card holder (§19's exposure table).
- **L8 I21:** Layer 8's three tools are `storage.read`, `encyclopedia.search` and `storage.propose` (§18.3's count base names them); the `kb.`
  prefix stays Layer 0's knowledge base.
- **L8 I32:** budget writes are loopback-only and an accepted `quota.set` applies the route's own check; S1's "an action inherits the gate of the
  route it wraps" is the same rule for `settings.set`.
- **L8 I24:** Layer 9's `component.enable` is not an agent path for kiwix (only `111d` writes `repair`, S2); Layer 8's I24 check stands as ruled.
- **L8 I10, I12:** the lazy-budget rule (I22 row); every Layer 9 number stays a default measured on the owner's real-device run and recorded in
  `9-evolution/TASKS.md`.
- **L8 S21 + I6 + D11:** `111 store export | import` hold `node/lock` and run with the daemon stopped; the export carries `heal/*` (J6);
  `state/heal/{run,inbox,undo}` live outside `node/state.db` and J8 lists them as lost on a words-only restore.
- **L8 S22 = I23 = D25, I2:** citations re-pinned at the lock head (S20 = I25 = O23).
- **J2 as Layer 8 v0.2 words it:** the witness inbox keeps the newest 256 lines through a temp file and a rename (I1 (a), O24 = I28).

Requests to Layer 8 (its draft, for its fix pass or its re-check; Layer 9 never edits it): J1 names the record kinds Layer 9 adds (delegations,
drafts, grants, sent fixes) and its layout row carries `state/heal.json` (I27); the hand-off sentence "a 64 KiB attachment cap" reads "no
attachment in v1" (S12); J9's example list of protected states reads as each namespace's own declaration, so Layer 9's `awaiting_confirm` and
`high_water` stand beside it.

## Lock-amendment candidates

Changes the findings need in locked specs, beyond the sixteen D35 took (marked **new**) or widening one of them (its item named). Each enters
§16's table and §23 marked additive, correction or removal; every **new** one is also listed for the owner in the spec's deviations section. None
is made silently: each goes into its layer's lock amendment.

- **Layer 0 (locked), `0-knowledge/_meta/schema.md` §2 and the status gates (item 1 widened):** the `playbook` column of the applicability matrix;
  the playbook gate sentence beside `L0:91`; the scope line, the root index and `0-knowledge/AGENTS.md`; `_meta/playbook-vocabulary.toml`,
  generated by a node111 test and read by `build.py`; N06 compares the filename with the rendered `[match]` (O14 = I12; O3 + O28 + I13 + I14).
- **Layer 1 (locked v2.2):** "Supervision" (item 2 widened): the delivery sentence (the witness arrives with `111 install` and `111 install
  --phone` only; `111d` reads the unit and the hook at start and files one `human_step` when it is absent); `111 heal-witness` exits 0 on every
  error (O5 vs I2; O24 = I28) · "Release assets", the Termux .deb closure row (`L1:68`) (item 2's removal widened): `cronie` leaves the minimum,
  with O7 = I9's removal list · the Android install flow and the layout (item 4 changed): the provisioner pushes `state/apk.json`; no
  `receipt.apk` (I10) · **new**, the CLI: `111 heal settings [<key> <value>]` (O19) · conditional, not in v1: `termux-api` joins the closure only
  on the owner's word (O18).
- **Layer 3 (locked), "Tasks" and "Interfaces" (items 7–9 widened):** `TaskRequest.kb_draft` on a `propose` task (I3 vs O15); `response_format`
  with `evidence` constrained to the incident's ids and no `quotes` (O27); the `patch` task's inputs, the incident in an untrusted block and the
  note, nothing from the journal (S17); `AGENT111_DATA_DIR` and the reason code `prompts_invalid` (I31) · **new**: the agent's `--opencode-path`
  (I11).
- **Layer 4 (locked), the contract:** **new**: `POST /api/v1/proposals/{id}/decision` answers `403 loopback_only` to a non-loopback accept of a
  proposal holding an action on a loopback-only key (S1; Layer 8's I32 states the same for `quota.set`) · **new**: `PATCH
  /api/v1/transport/posture` answers `409 busy` while an undo is armed (S5) · item 10 widened: `repair`, `fix` and `human_step` are kinds the
  agent's writer never writes (S2); `ConsentRequired.action` gains `heal_remove` (S7); I5's shapes and routes as ruled; `would_lock_out` is not
  added in v1 (S9) · conditional: D21's lazy total, only if the measured page does not fit (I22).
- **Layer 5 (locked v1.2), §5 and §19, `Fabric::send_op` (item 11 widened):** `direct_only` (S23 + O12 + O26 + I20); the size gate as the sum of the
  per-fragment budgets (O23); proven on mesh111's `lossy` link (I15).
- **Layer 6 (locked v1.0):** §12 (item 15 widened): one apply at a time node-wide, the lock held from arm to keep, revert or confirm, and
  `apply_unrecorded` armed with `proposal_id: null` (S5); the trait's `restore_cold(Bytes)` and the undo file's phases (S4 with I8); the three
  bridge actions join `device.propose`'s closed vocabulary (S19) · **new**, §2.1 rule 1, beside Layer 8's `adapter`-optional amendment: a
  `[run]`-less tool component form and a `requires.hub` key (I11).
- **Layer 2 (locked v2) and Layer 7 (locked v1.0):** none beyond items 6 and 16 (S12 keeps `111/fix/v1`'s construction; O16 and I32 use L7 §6.2 and
  G9 as they stand).
- **Code seams in locked crates, no spec text:** `ExitFacts.requested` (O1); the generation `watch` of `Supervised` and `Components` (O8); the
  registry's change signal as the observer's wake (O22).

## Order for the build (from the reports' watch notes)

1. Layer 8's housekeeping-owner move and its state service (J9, the decision calls, the hand-off rule) land before any Layer 9 state row (D35;
   Layer 8's triage order); otherwise J7 applies, with no proposal filed (I7). Lock Layers 8 and 9 in one pass and re-run the integration lens's
   §4 comparison on the two final texts (Layer 8's I13 and I14 are the same seam seen from the other side).
2. The containment correction (§8.1) and the atomic spool writes first (§15 item 1). A pinned opencode on the build box so G24 runs with
   `REQUIRE_G24=1` on the lock run, never SKIPPED (both the safety and the integration notes), S17's canary included: after S1 and S2 the deny block
   and G24 are the only guard against a local process without a bearer.
3. The engine's start reconciliation before any new action joins the vocabulary (safety note): the S4-with-I8 rule, S5's lock, S6, S8, with the
   stale-record, second-restore, consent-gate and both I8 cuts as tests. Record I16's Layer 6 applier tape with G5's fakes before the decision
   route moves into Layer 8's store (integration note). The engine refuses an action whose undo record fails to re-read after the fsync; every
   new action enters with its G20 round trip (apply, forced probe failure, state equal to before).
4. The observer: `count` comes from `ExitFacts.seq`, never from seen transitions (watch channels coalesce; observe note). G19 (5) records the
   wakeup count with Layer 9 idle beside Layer 7's baseline, not only "unchanged". G7's extension waits Layer 7 row V1 (I17).
5. Measure before a number enters the spec, on the owner's devices where it is a device figure: T1 on a Pi 5 and a Pi 4 under the JSON-schema
   grammar, the model's thinking tokens included (observe note; Q21's `llama-bench`); the binary deltas of the observer, the evaluator, the
   engine, the op arm, the agent's loader and `diffy`; idle RSS against I17's ceiling; the held `[match]` table; a hub's RSS per delegated
   diagnosis. Recorded in `9-evolution/TASKS.md`.
6. Role bit 6: node111's `roles()` takes `(s, relay, lan)` at HEAD (`N/fabric/mod.rs:592`); bit 6 needs a third input, and a toggle of
   `heal.serve_members` re-mints the announce once (observe note); Layer 8's bit 7 lands beside it.
7. The agent tool count: Layers 7 (+3), 8 (+3) and 9 (+2) bump the same two test constants; the three boards take that one order (integration note).
8. The injection case for `service.log` and `incident.read` joins the §13 suite: they put log text in front of the chat model, which can file
   `device` proposals (safety note).
9. Every amendment to a unit, a hook, a Termux script or a provisioned file states how it reaches an installed node (integration note: I2, I9 and
   I10 share that cause; the O5 vs I2 ruling is the rule).
