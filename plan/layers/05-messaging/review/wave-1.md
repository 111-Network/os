# Layer 5 — Wave 1 review triage (2026-10-14)

Review run `wf_c857767c-bde` (script `layers/05-messaging/runs/workflows/wave-1-review.js`): F1 / P1 / M1 by fable (xhigh), U1 by opus.
Full findings: the run's `journal.jsonl` (one result line per task). Commits reviewed: F1 `c8a093c` · P1 `79ca52d` ·
M1 `99f5e0a` · U1 `858077f`. Every finding below is accepted unless marked; bounces run as fresh agents (one per task,
disjoint files), the lead re-runs each check, reads the diff and commits per bounce.

| Task | Verdict | Majors | Minors | Deviations |
|---|---|---|---|---|
| F1 mesh111 runtime | fix-then-lock | 2 | 13 | 15 accepted · 1 rejected (node1 contacts in memory only) |
| P1 hosted panels | ready | 0 | 6 | 11 accepted |
| M1 Meshtastic | fix-then-lock | 1 | 9 | 14 accepted |
| U1 UI Messages/Contacts | fix-then-lock | 1 | 16 | 10 accepted · 1 rejected (ignore = DELETE) |

## F1 — bounce (all)

- **M1 unbounded actor input queue** (an unauthenticated LAN peer streams 88 B sealed frames faster than the actor opens
  them; the link forwarder drains tcp_lan's bounded channel into an unbounded one) → bounded channel with `send().await`
  back-pressure end to end, or `try_send` + drop + `counters.dropped`; test with a flooding peer.
- **M2 re-enrollment floor loses held custody** (`custody_seq_floor = custody_high()`; a member that re-enrolls sets its
  cursor past items still held for it) → floor = lowest uncommitted custody_seq for that dest − 1 when any is held, else
  `custody_high()`; test: enrol, hold items, re-enrol, fetch gets them.
- Minors m1–m13 as listed by the reviewer: `heard` only for known keys / filed requests · **node1 pending contacts persisted**
  (small store table + migration — the rejected deviation; read marks may stay in memory, documented) · COMMIT kept when
  unsent and sent beside FETCH on hub link-up · `requeue_orphans` plans with `plan_mtu(peer)` · push `high` only when the
  frame was queued · hub hint never pre-empts an enrolled peer · reason cleared on a later direct `sent` · forgetting the
  hub clears `home_hub` + releases the dial · claim mismatch counts as a failed dial for back-off · thread scans newest-N
  (store query) · lossy test asserts SACK-inferred retransmits before the harness hammer · DUPLICATE-as-relayed reading
  recorded · reason codes `cancelled` + `not_enrolled` → contract (F2, Wave 2).

## P1 — ready; small bounce before lock

- m1 Fetch-Metadata tightening: on `/api/*` paths with `Sec-Fetch-Site` present, only `same-origin` passes (same-site GET
  from another origin on the host must not dequeue a panel's frames with the victim's cookie) + test.
- m2 **Lead decision:** bearer callers never mint panel tickets (the doc line stays true; the agent does not open panels).
- m3 harness port race (assert `bound_addrs`, retry with a fresh port) · m4 ETag from (len, mtime, ino) and the API-path
  check on the decoded path · m5 absolute per-request deadline on shim body reads + a per-peer-IP connection cap; the
  main dashboard listener gets the same header-read/idle timeouts if the change is contained · m6 trusted-loopback
  binding bound to a random per-generation nonce rather than a hash of the ~39-bit pair code.

## M1 — bounce (all)

- **M1 forget during an in-flight identify** persists a binding on a forgotten (hidden) entry and leaves the service
  running with no way to unbind → resolve `published` and require `!forgotten` before any store write in
  `record_identify`; on its error, `identify()` tears the service down; test the interleaving.
- Minors m1–m9: the flaky `accepted()` assert → `wait_until` · identify step selects on shutdown · retry when the meshmud
  binary appears · zero-length FromRadio frames skipped · session cap taken atomically · port holder from
  `detail.port_held_by`, not the reason string · `MeshmudFacts.installed` + the registry rule says "meshmud not installed"
  · python3-absent flow tests visible (fail the check or `#[ignore]` with a reason). The openapi `identifyDevice` text
  ("USB or Wi-Fi board") is amended by F2 (Wave 2) with the regen: "USB board (Linux) or meshtasticd; Wi-Fi boards are
  bound by address in a later task".

## U1 — bounce (all)

- **M1 ignore semantics. Lead decision (reversible, no contract change in v1):** "ignore" never calls the daemon; the
  request stays pending (spec §7.2: kept, never auto-pruned, no ACK) and is folded out of the list and out of the
  attention count by a per-device dismissed set `(peer, newest received_seq)` that clears when the peer's request gains a
  newer message. A separately labelled **"forget request"** keeps the DELETE behind the 2-step confirm with honest copy
  ("their held words are discarded; if they write again it comes back"). Tests rewritten accordingly. If the owner wants
  the fold to follow the user across devices, the contract gains `Contact.ignored` in a later wave.
- Minors m1–m16 as listed: decoder loaded before `getUserMedia` (camera never left on) · per-peer request generation on
  `loadThread` · open thread trimmed to the page limit · `reuse()` keyed by id · picture-file pixel cap · reason copy keyed
  on (state, code) · `Object.hasOwn` tables · path gap recorded + "what this state means:" · badge → `/messages` when
  requests are the only items (Overview row is U2's) · agent goal by `reason_code` until U2's goals land · forget copy
  matches F1 (outbox kept) · add-contact result branches on `request` · `dir="auto"` + cleaned raw block · shared
  `Disclosure` · card text parse parity with the daemon (`=` stripping, 512 + slack cap) · vector tests import
  `encoding.json`.

## Carried to Wave 2 / later

- F2: contract additions `reason_code` += `cancelled`, `not_enrolled`; `Problem.code` += `restore_pending`; `Proposal.kind`
  += `message`; `identifyDevice` description; `pnpm gen:types`.
- U2: Overview needs-you row for message requests; `messaging.setup/fix` goals replace the banner's interim questions.
- C3 wish (not blocking): newest-N thread queries landed by the F1 bounce; a `read_seq` column if read marks must survive
  restarts (v1: documented as in-memory).
