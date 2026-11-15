# Layer 9 triage — the read-only check of the draft (2026-11-13)

One judge read the drafted triage against the three reports, the root README, D35, the Layer 8 triage and the locked specs, and returned the
items below. The lead's ruling on each is in `triage.md`, section "Rulings on the check" (same numbers). An item's "proposed" text is the
judge's; where it offers alternatives the ruling picks one.

## C1. I11, 'L8 D7 + I15, with S1 = D3 = I1' bullet  (contradicts-layer-8-contract)

The triage files a NEW writer of Layer 8's object tree under 'What Layer 9 takes from Layer 8's triage': "otherwise `sudo 111 install --components opencode` fetches it with the same origin order, sha256 and size checked before the rename into `blob/` ... and its bytes counted in the metered window". Layer 8's triage (D7 + I15) names one fetcher only: "The `[source]` pin is fetched by the updater of a node with `origin.serve` on". /home/user/os/8-storage/README.md:434 gives the writer of `state/origin/blob/<sha256>` as "`111d`; the import verbs only with the daemon stopped", §7.6 makes verbs copy into `incoming/` and hand over through `POST /api/v1/datasets/adopt`, and §11.3 says "The root installer runs with no daemon" (so it cannot count bytes in the daemon's metered window, and a root-run rename leaves a root-owned file in node111's tree). I11's own fix kept a cached tarball ("the cached tarball's sha256 equals the running release's `[source]` pin").

**Proposed:** Either (a) keep I11 as written: on a hub without `origin.serve` the tarball stays in the root-owned `cache/packages/opencode/` (draft §4 path table), checked against the running release's `[source]` pin before a `patch` task; `state/origin/blob/` only where Layer 8's updater put it; or (b) move the sentence to 'Requests to Layer 8' as a third writer that goes through §7.6's hand-off (`incoming/` then adopt), and drop 'counted in the metered window' for a verb that runs without the daemon. Do not present it as taken from Layer 8's triage.

## C2. O6 with O24 = I28 and O20 = I23 (a)  (unruled-fork)

Two accepted fixes collide on the inbox line. O6 (named blocker-adjacent bullet): "any inbox line whose result is not `success` opens or bumps `(1, 111d, unclean_exit)`, also under a clean marker". O24 = I28 row: "the Termux hook writes `result=exit` or `result=signal`". So on a phone every graceful stop (`result=exit code=0`) is 'not success' and opens `unclean_exit`. Second collision: the O20 row opens `(1, 111d, killed_by_os)` on a Termux line with signal 9, and O6 opens or bumps `unclean_exit` for the same line: two incidents for one event, against O3's 'one source per failure'.

**Proposed:** Rule the (marker, inbox) table in the triage: a line is clean when `result=success` (Linux) or `result=exit code=0` (Termux); every other line opens or bumps `unclean_exit`, except a Termux `result=signal signal=9`, which opens `killed_by_os` only; `watchdog` result opens `watchdog_kill` only.

## C3. S4 with I8 (and I7, O6)  (unruled-fork)

The phase set is `armed → keeping | restoring → removed`, and the row says the cold restore is "folded as `reverted (daemon restart)` through the decision calls once the store opens". Nothing says what the file reads between a successful `restore_cold` and that fold. If it reads `restoring` (S4's rewrite) or stays `armed` with the old `starts`, a second start with the store still closed (J2: "if the store does not open, they stay") meets "`restoring`, or a `release` or `starts` that is not the previous run's ⇒ no restore, `revert_failed`, critical": a revert that succeeded is reported as failed. The row also does not say (a) that the file is rewritten `restoring` before the COLD restore (only 'A revert (S4): rewrite ...'), or (b) whether the undo is reconciled before the run marker is rewritten (O6: `starts` is "read before the rewrite"); if the marker is rewritten first and the daemon dies before reconciling, an honest `armed` record is 'not the previous run's' and is never restored.

**Proposed:** Add to the row: order at start = read marker, reconcile the undo, then rewrite the marker; before `restore_cold` rewrite `phase: restoring`; after it returns Ok rewrite `phase: restored` (fsync); at start `restored` ⇒ no restore, fold `reverted (daemon restart)` when the store opens, then remove. G20 (3) gains: two starts with the store closed after a cold restore read `reverted`, not `revert_failed`.

## C4. S3 with I6 (fix states) and README §9  (unruled-fork)

S3 as accepted: "a typed fix files a proposal only while this node has an open incident with the same fingerprint, else 'does not apply here'". The record-states row lists fix `undecided`, `parked` ("only S24's 'needs a later release'"), `refused`, `decided`, `sent`. Nothing says which state a 'does not apply here' fix takes, or whether it is matched again when an incident with its fingerprint opens later. Read as 'evaluated once at receipt', a fix helps only a node that has the same failure open at the moment it arrives (the home hub that receives A's fix normally has no such incident, and S11 forbids it to pass the fix on). README §9: "Nodes share proven fixes with each other ... so a repair validated on one node can heal others".

**Proposed:** Rule it (rule 2, keep and narrow): a signed, trusted, in-scope typed fix with no matching open incident is stored `undecided` inside the 16-per-author queue, shown as 'does not apply here yet', and is matched when an incident with its fingerprint opens (one `list` at incident open, no timer); only then a `fix` proposal is filed. If the lead prefers evaluate-once, state it as a v1 limit in §0's row and name the state (`refused`).

## C5. I1 (b) with L8 I14 / L8 §12.3, S11, I7, I4 = O25  (unruled-fork)

The `Full` rule covers observation, `repair` proposals and an armed apply only. It is silent on Layer 9's inbound consumers. The triage binds them to L8 I14 ("Layer 9's consumers for 0x10 and 0x13 write their J9 record before returning"), and /home/user/os/8-storage/README.md:873-874 says "A consumer error leaves the frame uncommitted and unacknowledged, so the sender retransmits it". With `heal` `full` (it stays `full` until a human answers the export offer, L8 §4.5) or the store closed, a `put` cannot succeed: an author must guess between an error (the peer's fix or a member's `diagnose` is retransmitted without end) and drop-and-acknowledge. Also unruled under `Full` or a closed store: the `high_water` write, a hub's delegation record and stored reply, a `sent` fix record, a draft created by an accept, the T1 fold ("one J9 `put`, committed before the unlink": the spool file is then never unlinked), and whether T1 runs at all with the store closed (I7 says only "files no proposal").

**Proposed:** Add to the I1 bullet: while `heal` is `full` or the store is closed a consumer never returns an error: a fix is dropped and counted (its `high_water` not advanced), a `diagnose` is answered `refuse` 3; no fix is sent, no delegation composed, no T1 or human task submitted; an accept that would create a draft answers `503 store_unavailable` / `409` naming the Storage page. G19 (7) and G22 assert the fabric acknowledges and nothing is retransmitted.

## C6. S18 (the `pinned_contacts` binding, not the lead's `paired_hubs` ruling)  (contradicts-locked-spec)

The row defines `pinned_contacts` as "every contact whose Layer 5 §7.1 `pinned` flag is set (a human sets it through `PATCH /api/v1/contacts/{id}`)". Locked Layer 5 §7.1 (/home/user/os/5-messaging/README.md:297) defines that flag as "`pinned` (never pruned)" and the locked contract says "pin (a pinned contact is never pruned)" (openapi.yaml:306) and `pinned: {description: "never pruned"}` (:1040). The ruling gives a retention flag a trust meaning: contacts already pinned to keep them from pruning become fix senders the moment the setting is chosen, and later pins are made on a route that is session-or-loopback while every Layer 9 setting write is "loopback-only" (§17.1). No Layer 5 or Layer 4 lock-amendment candidate is listed for it (the Layer 5 bullet holds `send_op` only).

**Proposed:** Keep the set, and add: a lock-amendment candidate (Layer 5 §7.1 and `Contact.pinned` / `patchContact` descriptions: 'while `heal.fixes.receive = pinned_contacts`, a pinned contact may send this node fixes'); the pin control shows that sentence while the setting is on; the `heal.fixes.receive` change to `pinned_contacts` lists the currently pinned contacts in its text. Or, stricter and without touching Layer 5: the set is an explicit NodeId list in `state/heal.json` written by the loopback-only route.

## C7. I32 with O16, I15 (and S18)  (unclear-for-an-author)

The I32 row says a fix over 2,937 B sealed "is refused at send with 'too large for a carrier' (O16's bound) and nothing is put". O16's fix is a display ("The send screen shows the sealed size and 'too large for a carrier'"), and locked Layer 7 puts the refusal at the hub: "a put whose only targets are `carrier` pairs it does not fit is answered `REJECT_UNKNOWN_DEST` ... the hub counts it (`forward_too_large`)" (/home/user/os/7-relay/README.md:428-430). A sending member does not know whether its hub will forward through a carrier, so 'refused at send' reads either as a 2,937 B cap on every fix (making O16's other two bounds and the 4,096 B body dead) or as a check the sender cannot make. I15's "in v1 a fix and a delegation travel over `tcp-lan` only" is ruled in another bullet and reads as contradicting a gated carrier crossing. The gate also does not say who the far recipient is: the brief (F11 D) and draft :516-517 say a put addressed to a hub itself is refused, so a `paired_hubs` author has no carrier path.

**Proposed:** One sentence for §11.2 in one row: three bounds (O16); the send screen shows the sealed size and warns above 2,937 B; the sender refuses only above the `tcp-lan` frame; a carrier refusal stays Layer 7's. G22: a fix ≤ 2,937 B from member A reaches pinned contact B on the far island and is stored; a larger one is counted `forward_too_large` on A's hub and A's fix reads 'not delivered'. I15 says 'the only fabric link is `tcp-lan`; custody and a carrier move it between islands'.

## C8. S8 with S5, I5, I6 (and `revert_failed`)  (unruled-fork)

S8 as accepted: a revert that needs a stale consent "stops as `revert_blocked`, shown in Needs you" and (the report) "the human restores". Not ruled: whether the undo file and the node-wide lock (S5: "held from arm to keep, revert or confirm") stay while blocked (kept ⇒ every accept answers `409 busy` until a human acts; removed ⇒ no snapshot to restore from); which route the human uses (I5's ruled routes have confirm, undo, dismiss, task only); what a start finds (the S4-with-I8 phases have no blocked phase, so the next start would run `restore_cold` past the consent gate or mark `revert_failed`). The record-states row protects only `armed` and `awaiting_confirm` attempts, so a `revert_blocked` or `revert_failed` attempt is reclaimed with its incident once that closes, while the human still owes an act.

**Proposed:** Rule: `revert_blocked` rewrites the undo file `phase: blocked` and releases the lock; at start `blocked` ⇒ no restore; the human resolves it on `POST /api/v1/repairs/attempts/{id}/restore` (the consent round-trip, then the restore) or by dismissing ('leave as is'), which removes the file; attempts in `revert_blocked` and `revert_failed` are never reclaimed until resolved or dismissed.

## C9. O3 with O4 and G19 (1)  (unruled-fork)

O3 (accepted): "G19 runs with a real supervised child and counts one incident per failure"; the merged row removes only ROWS whose code is `child_degraded` as sources. Draft §3.4 still opens on "a child entering `Degraded`" from the watch, and G19 (1) asserts five kills "leave that one incident at `count = 5` and open one `child_degraded` incident". After O4 a crash loop is therefore `(layer, subject, child_exit)` plus `(layer, subject, child_degraded)`: two incidents, two playbook files, possibly two T1 tasks and two `service.restart` proposals for one failure. Neither report nor the triage says whether `child_degraded` stays a fingerprint.

**Proposed:** Rule: one incident, `child_exit`; the `Degraded` transition is a state of that incident (severity `error`, T0 re-run at that count change, readable by `service_state is degraded`), not a second fingerprint; `child_degraded` leaves the incident reasons; G19 (1) asserts exactly one incident after five kills.

## C10. O9, S16, §7.7 ledger  (pi-floor)

Two bounds rest on an undefined term. O9: a re-opened fingerprint "reuses its record and its diagnosis until the evidence hash changes" (the gate: "ten open and close cycles make one model request"). §7.7 / S16: a reverted or `unresolved` action is not re-proposed "until ... the evidence hash changes". O13 (accepted) makes the on-node excerpt "the matching line, else the last line before the exit": log lines carry times and pids. If the hash covers excerpts, `count`, `seq` or `mono_ms`, every flap changes it: one T1 call per flap (about a minute of full CPU on a Pi 5 by the draft's figure) and the same proposal re-filed, while G21 stays green on a stub child with constant output.

**Proposed:** Define it in the O9 bullet: evidence hash = BLAKE3 over (fingerprint, the sorted `[check_id, result]` list, the exit kind as code or signal, the set of satisfied cause ids); never excerpts, counts, sequence numbers or times. G21's flap test uses a child whose log line differs on every run.

## C11. I22 (with L8 I10)  (unruled-fork)

The row says §14.1 states "238.9 of 256 KB" and "the allowance shared with Layer 8's Storage page" but gives no figure for Layer 9. Layer 8's draft already takes its part: "17.1 KB remain for everything later layers add ... Layer 8's allowance is 12 KB gzip" (/home/user/os/8-storage/README.md:1101-1103). That leaves about 5.1 KB gzip for the Repairs page (eight sections), three new proposal cards and the adapter; Layer 7's single `l7` chunk is 11.9 KB. Not fitting is the expected case, and the ruling's only branch is "raising D21's lazy total is the owner's ... never assumed", which leaves a build that is told to run A to Z with a red `pnpm budget` gate and no ruled outcome.

**Proposed:** State Layer 9's allowance as a number (the remainder after Layer 8's 12 KB: 5.1 KB, default, to be measured) and rule what the build does when the measured chunk exceeds it: the figure and the overage are recorded in `9-evolution/TASKS.md`, the D21 change is the first item of 'Open for the owner', and the UI row names what ships meanwhile (the gate fails red and is recorded, or the advanced sections are held back). Pick one.

## C12. I1 (c) + I6 row, with S2, S5, S11  (unruled-fork)

The record-states row leaves kinds unplaced. (1) Fix state `sent` is in neither the never-reclaimed list nor the order ("decided, refused and parked fixes"); S11 accepts a vouch "only from a node this node sent that `fix_id` to", so reclaiming it silently drops later vouches. (2) "an attempt goes only with its closed incident": attempts with no incident exist (S5's `apply_unrecorded` with `proposal_id: null`, `device` proposals through the trait, an Undo) and have no reclaim step. (3) S2 makes accept depend on "its own `heal/` record for that proposal id" holding the actions and the `action_hash`; no record kind is named for it. If it is the incident, a `self_recovered` incident is reclaimed first while its proposal is still undecided; if it is the audit entry, a stream trim removes it; a `human_step` (APK, witness) has no incident at all.

**Proposed:** Add: `sent` fixes are reclaimed after decided fixes (a vouch for a reclaimed fix is dropped and counted); an attempt with no incident is reclaimed with the closed-incident step by its own `seq`; the proposed actions and `action_hash` live in a `heal/proposals` record (state `undecided`, protected; `decided` goes with the proposal), written in the same J9 call as the proposal; when its incident closes, `111d` marks the proposal withdrawn.

## C13. O27, S14, S9, S28, S6, I31  (contradicts-decision)

The triage's own rule: "A ruling that departs from the brief ... is marked 'listed for the owner'". These accepted fixes depart from D35 picks without the mark. O27 drops `quotes`: F4 A "quoted excerpts must be substrings of it" and §3 'T1 discipline'. S14 narrows F5 A's `log_contains` (it can no longer carry a typed action alone) and changes F4 A's T1 enum (adds `unknown`, `advise` only) while O11 next to it IS marked "narrows F5 A's grammar". S9 moves `would_lock_out` to §22: F6 A "an action that would remove the last pairing or the loopback listener is refused". S28 (default-accepted) reserves vouch kinds 1 and 2, so F10 A's maturity step `reproduced` cannot be reached in v1 and §11.5 keeps a dead state. S6 narrows F6 A's Undo to an unchanged state hash. I31 drops the brief's on-demand battery read.

**Proposed:** Add "Listed for the owner in the spec's deviations section" to O27, S14, S9, S28 and S6 (and one line for the battery read); for S28 say that §11.5's ladder in v1 is `local → accepted_by_n` with `reproduced` reserved.

## C14. S3, S12 (and I21)  (contradicts-vision)

D36: where v1 delivers less than the README says, the spec states it as a limit. The triage names §0's table for O2, O16, I15 and I21 but not for S3 or S12, which narrow README §9 most ("Nodes share proven fixes with each other through the gateways, so a repair validated on one node can heal others"; README §5 "validated patches"). The draft's §0 row still reads "there it is a proposal its own human accepts": after S3 and S12 it is a proposal only for a typed fix, only on a node with the same incident open, and only for `service.restart`, `advise` or the node's own playbook action; code and KB fixes do not travel. With three authors this row has no owner. I21's row text "a monitoring model is not planned for the Pi floor" gives no 'closed by' for devices above the floor, where the README says the model is "the autonomous monitor of system health".

**Proposed:** Add to the S3 and S12 rows: "and §0's row for 'share proven fixes' is rewritten to say exactly this"; assign the §0 table to one author. In the I21 row give the 'closed by' column a value for capable hubs (the opt-in hub probe of §22, or 'the owner's call'), keeping 'not on the Pi floor' as the v1 limit.

## C15. S19, S7, S11, O7 = I9 (the lock-amendment list)  (contradicts-locked-spec)

Rule 4: no silent change to a locked spec. Missing candidates: (1) S19 puts the three bridge actions into `device.propose`; the Layer 4 contract pins that vocabulary (openapi.yaml:848: "from the closed vocabulary `rns.interface.upsert ...`, `radio.region.set {region}`, plus the one-shots") and the device card renders it; the triage lists the change under Layer 6 only, and the agent's tool schema is not named. (2) S7's consent text "removes <n> stored items" needs a count from mesh111 before the apply: a read seam absent from 'Code seams in locked crates'. (3) S11 (sender = `author`) and §12.3 (attributed connection) need the consumer hook to hand over the authenticated sender and the link's attribution; /home/user/os/8-storage/README.md:867-873 passes "a new frame's body" and the `msg_id` only; this is not in 'Requests to Layer 8'. (4) O7 = I9's removal list omits /home/user/os/LICENSING.md:65 (it names cronie; AGENTS.md requires the update) and the manifest fixtures and `tests/manifest_parse.rs:76-82`, although the row says "the script and the manifest fixtures are its only references".

**Proposed:** Add: Layer 4 item 10 widened (the `device` vocabulary text and card gain the three bridge actions) and the agent tool schema; a mesh111 read seam 'evictions a quota change would cause'; a request to Layer 8 §12.3 that the consumer receives (authenticated sender NodeId, attributed-link flag, msg_id, body); LICENSING.md:65 and the fixture list in the cronie removal.

## C16. S19 with O3 + O28 + I13 (b, d); S15  (missed-merge)

The domain of a subject is ruled twice in different words. S19 row: "`@subject` is an id `111d` minted (it passes `valid_id`) and must resolve to an existing object in `plan()`". Merged vocabulary row: "One subject id space: `ServiceId`, component ids, and the fixed daemon subjects ... each in the compiled table". An author of §6.2 can read S19's wording as admitting run-time ids outside the compiled table (the very widening S19 warns of). A third id space enters through S15: "`component.enable` calls the function the bridge settings route calls, by adapter id", while O3 makes an incident's subject a component id, and the function is keyed by component (`enable_component(api, components, component: &str, ack)`, node111 `dashboard/api_v1/bridge.rs:288-293`). So what `"@subject"` binds in `component.*` is open.

**Proposed:** One sentence in the vocabulary row: `@subject` is a member of the compiled subject table and nothing else; `plan()` must resolve it. In S15: the action is addressed by component id and calls `enable_component`; an adapter that enables two components needs two proposals (the one-consent rule).

## C17. S3 (peer subset), §12.1 reply key 5  (unclear-for-an-author)

The S3 bullet reads "from a peer only `service.restart`, `advise`, and an action byte-equal to this node's own playbook action for the same `cause_id` (targets bound by `@subject` here, never taken from the peer)". The parenthesis attaches to the third item. For a bare `service.restart {service_id}` from a peer it is not said whether `service_id` is the peer's (then a peer can ask this node to restart `transport` while the open incident is on `model`). The hub's reply keeps a wire field "`{0 action, 1 target, 2 args}`" that the rule would have to ignore. It is also open whether a `human_step` byte-equal to the node's own playbook step is 'allowed' or 'their words'.

**Proposed:** State: a peer's `service.restart` is accepted only for the open incident's own subject; a peer's target field is never used and a mismatch drops the action to 'their words'; a `human_step` or `advise` shown with a button is always this node's own playbook text, selected by `cause_id`.

## C18. S23 (+ O26 (b)), O5 vs I2, I10, I16/I8 names  (unruled-fork)

Names the rulings need and do not give. (1) Delegation row: "the same `req_id` with another hash gets `refuse` (S23)": no reason code; the closed list is 1-5, and reason 3 is "retried on that same event" (I20), so the wrong choice loops; O26 (b) reuses 5, whose meaning is "unsupported schema", for a playbook mismatch. (2) O5 vs I2: 111d "files one `human_step`" when the witness is absent, and I10 files one for the APK; neither has a fingerprint or a reason in the compiled closed table (O3 row), so nothing dedupes it across starts or after the human reads it. (3) `NODE111_HEAL_PAUSE_AT=<apply|keep|restore>` has three values for at least five cuts the same row demands (apply/probe, the two I8 keep cuts, inside a restore, after a cold restore).

**Proposed:** Add reason 6 `request_changed` (never retried) and say what a member shows for 5; add reasons `(1, 111d, witness_absent)` and `(1, 111d, apk_outdated)` to the compiled table, each with a shipped playbook whose one action is the `human_step` (the O20 pattern), closing when the condition clears; name the pause points `apply`, `keeping`, `decided`, `restoring`, `restored`.

## C19. I5 with S12 and O15 (the task route), I4 = O25  (unclear-for-an-author)

One route, `POST /api/v1/repairs/incidents/{id}/task {mode}`, "answers 202 with the task id". (1) Mode `diagnose` is "ask again, §7.7": on Cat 2A and 3 no agent runs, and §7.7's ask-again is the ledger unblock plus T0 (and a new delegation), not an agent task; the answer body there is undefined. (2) The fold rule says a proposal "whose `task_id` names such a task [the T1 task 111d submitted] is folded into its incident ... never enters `agent/proposals`"; the same route now starts `kb_draft` and `patch` tasks whose proposals must reach the list to be accepted. (3) `explain` has no `TaskRequest` form in the Layer 3 amendment list. (4) No answer is named for a mode the node cannot run (no model, not a hub, `heal.code_drafts` off, the `[source]` blob absent).

**Proposed:** Add: fold applies to modes `diagnose` and `explain` only; `kb_draft` and `patch` proposals are ingested normally; where no agent runs, `diagnose` re-runs T0, clears the ledger block, re-sends the delegation and answers 204; `explain` is a `diagnose` task with `incident` and `narrative: true` (Layer 3 item 7); an unavailable mode answers `409` with an existing code (`needs_hub`, `component_unavailable`) or a named new one.

## C20. S7 with S15; S16; S14; O2 with O22; S23 + O26 (c)  (unclear-for-an-author)

Accepted fixes that read two ways. (1) S15 "a proposal holds at most one consent-gated action" and S7's new `heal_remove` consent: does `heal_remove` count as that one (one `accept_warning` carries one hash)? (2) S16 "an applied action whose incident did not close counts `unresolved`": an incident closes only after `crash_loop_window_secs` of `Running`, later than the 30 s probe; no moment is given. (3) S14 "needs one satisfied leaf that is not `log_contains`" under "lint and loader": a lint cannot see 'satisfied', and a hub evaluating a member's `[check_id, bool]` list cannot see leaves. (4) O2 "only a row in state `error` opens an incident" against the O22 ruling that keeps "Layer 8's `storage`" frame as a source (not a row; is `disk_low` an incident?). (5) The hub's stored reply holds "nothing else of the request", but a queued request needs its checks until it runs: in memory only, or a protected `open` delegation?

**Proposed:** State: `heal_remove` is a consent-gated action for S15's count; `unresolved` is set when a new unrequested event bumps the incident's `count` after the keep; S14 is structural in the lint and the loader (the cause's `when` names a check with a non-`log_contains` leaf) and at run time a `log_contains`-only match files `advise`; O2 governs service rows, and the Layer 8 sources are named one by one; a hub's queue is memory only (a restart loses it and the member's re-send restores it).

## C21. O28 + I14 (fixture home), I30  (unclear-for-an-author)

The ruling moves Layer 0's playbook cases to `1-bootstrap/crates/node111/tests/fixtures/playbooks/`, "read by both the loader test and `build.py --selftest`". Today the self-test reads its own folder (`fixture = meta_dir / "fixture"`, /home/user/os/0-knowledge/_meta/build.py:825), locked Layer 0 calls it "a golden-fixture self-test (a tiny vault ...)" shipped with the tool (0-knowledge/README.md:119), and CI runs only `build.py --check` (.github/workflows/ci.yml:112-113), so the 'same accept and reject cases' gate has no runner on the Python side. An author must guess how `build.py` finds a sibling layer's folder and what it does when the KB tree stands alone (a published or packaged KB).

**Proposed:** Keep Q9's home, and add: `--selftest` resolves the playbook cases from the repository root and prints `skipped: playbook fixtures not present` when the folder is absent; the `knowledge-contract-hygiene` job gains `build.py --selftest`; the Layer 0 amendment (item 1) names the path.

## Checked and found sound

- The four lead rulings (S12, S18 `paired_hubs`, S19, S27) are written as given; their knock-on edits are consistent with each other (S27 makes I16's rollback test and S4 (b) moot; S12 removes I5's `/fixes/{id}/draft`, D5 and the 64 KiB hand-off sentence; S19 leaves `radio.region.set {region}` and `rns.posture.set` playbook-able, both target-less per openapi.yaml:848).
- Every finding id is either named or falls to the default: unnamed are only S22, S25, S26, S28, O21, I24, I30; none of these adds a process, timer, poll, listener or dependency.
- Blocker count and ids (S1-S3, O1-O2, I1-I3) match the three reports.
- O5 vs I2: O5 on both platforms follows rule 4 and rule 3; the provisioner (not 111d) deleting the old cron script is consistent with it; I2's gates are kept.
- I3 vs O15: O15's producer with I3's target, budget and accept-time checks; tool count +2 is consistent with §18.3 and the build-order note; S2 correctly leaves `kb_draft` agent-writable.
- O4 vs I13 (c): one reason `child_exit` with `exit_is` on the evidence; consistent with the closed reason table.
- I4 = O25 in I4's first form sits inside Layer 8's hand-off rule as /home/user/os/8-storage/README.md:194-200 words it (insert, commit FULL, then unlink).
- S2's handling of refused kinds matches Layer 8 §2.4 rule 4 verbatim ("never ingested and never deleted by `111d`; ... counts it").
- J9 facts the triage relies on hold in the Layer 8 draft: `Full` is "which Layer 9's spec handles", "the three decision calls are written whatever the class's state", the reclaimer is registered per namespace, `heal` is `full` while its export offer is unanswered (README.md:296-300, :399-402).
- Role bits: bit 6 `ROLE_HEAL`, bit 7 `ROLE_DATASETS` agree with Layer 8's triage (I5 vs D29) and its draft §12.4.
- O18 (default off until the owner adds the `termux-api` deb), O10, O11, O13, O8, O16, I15, I10, O17, O7 = I9, I17 and I23 (b) are each marked 'listed for the owner'; O18 and O7 respect the Pi-floor rule (no dependency added, one removed).
- S1 and Layer 8's I32 state the same gate rule; the Layer 4 candidate is listed as new.
- S21: the `device` kind keeps Layer 6's one-shot; `identify` joins as a device-only row; no locked behaviour is changed.
- S10: one vouch per accepted fix outcome, none on a later Undo, matches the brief's privacy pick (the `fix-vouch` reply to a fix the human accepts) and S11's `voucher_seq` rule.
- The S23 + O12 + O26 + I20 merge is one coherent life: `direct_only`, re-send on link attribution (no timer), 8 random bytes of `req_id`, stored reply keyed with the request hash plus fingerprint and checks hash (reconciles O12 with O26 (c)), 4 queued in total.
- O14 = I12 is one Layer 0 amendment with an explicit matrix column, so the two reports' differing wordings are resolved.
- The cronie claim checks out for code: no source file installs the crontab line or starts crond; the references are the cron scripts, Layer 1 README :68 and :156, manifest fixtures and LICENSING.md:65 (the last two missing from the removal list, reported above).

## Not covered

- The drafter's fork summary was cut off in the task text after the Layer 4 lock-amendment entry; I judged the triage file itself, not the missing tail of the summary.
- Code citations inside the triage (`N/services/mod.rs:1449-1450`, `:1025`, `:1031`, `status.rs:326-336`, `N/mod.rs:137-180`, `N/fabric/mod.rs:592`, `N/supervise.rs:976-985`, `NI/install/phone/mod.rs`) were not re-read at HEAD; only `bridge.rs:288-293`, `auth.rs:158-168`, `applier.rs` action names, `build.py:824-870` and `ci.yml:112-113` were opened.
- Layer 0's `_meta/schema.md` matrix (whether O14's column equals `recipe`'s, as I12 says) and rule ids N01-N06, B03, B04, B09 were not checked.
- Locked Layer 1, 3 and 6 texts were not read beyond grep hits; whether each listed lock-amendment section name exists as written is unverified. Layer 7 was read only at §5.2 (lines 411-430, 612-616, 648) and §12.4-related lines; the claim that a put addressed to a hub itself is refused rests on the brief (F11 D) and draft :516-517, not on a Layer 7 line I found.
- Whether the Termux `finish` hook and `sv/` files ride the phone's release payload (needed for 'compares the hook's bytes with the release's copy').
- The 512 KiB RSS ceiling (I17) and the 50-case suite size (I19) were not tested against any measurement; I only checked that each is marked a default.
- The gate texts G19-G25 were not walked assertion by assertion against every ruling (for example G22 (3)'s 'parked' and 'Submit upstream' lines that S12 and O17 make stale).
- `review/options-check.md` and the research reports R00-R07g3 were not read.
- The Layer 8 spec is mid-amendment by another agent; I read it once, and line numbers quoted from it may move.
