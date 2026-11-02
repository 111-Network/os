# Layer 9 options brief: the judge's check (round 2)

Judge pass, round 2, 2026-11-02, read-only, on the tree `980b473` (the brief and the two review folders are untracked; the brief's
`00-code-seams.md` was line-read on `a65ac6d`, and every code line I re-read still sits where the brief says). Read whole: `options.md`
(598 lines), `research/06-critic.md`, then the report passages every fact and pick in the table below rests on (`00`–`05`, `07-gap-1`,
`07-gap-2`, `07-gap-3`), the Layer 8 brief's F1, F2 (the box), F5, F11, F12 and §6, the Layer 8 reports the joint contract descends from
(`04-local-store.md` §4 Fork 5, `07-gap-1.md` §4, `03-shared-log.md` §2/§4, `07-gap-2.md` §3.6, `06-critic.md` conflict 9), the Layer 7
brief's status line and headings, `7-relay/README.md` (§3.6, §3.7, §7.2, §9.1, the A4 tag list) and the code and spec lines under "What I
verified". No web fetch was needed: every number I checked rests on a report that names its own source and marks what it could not fetch,
and the brief carries those marks.

**Verdict: ready.** No blocker, no major. Round 1's two majors and six minors are closed (the table "Round 1, confirmed closed" below).
Four new minors, none of which moves a pick; the lead can fold them into the decision sheet without a third round.

## Findings (round 2)

| id | severity | where | what | fix |
|---|---|---|---|---|
| K1 | minor | Fact 19 (`options.md:87`); §4 item 16 (`:494-495`) | "the 2 s agent-relay poll, `relay.rs:111,892`" names no directory, between two `N/…` paths; the file is `N/dashboard/api_v1/relay.rs` (`absent_poll: Duration::from_secs(2)` at `:111`, the sleep at `:892`: both hold). A reader looks under `N/` or `A/` and finds nothing. | Write `N/dashboard/api_v1/relay.rs:111,892`. |
| K2 | minor | Fact 16 (`options.md:75-76`); leaned on by F10 "a 233 B bearer only when it fits 16 fragments" (`:325`) and F11 "`tcp-lan` and 233 B bearers" (`:348`) | The fact quotes 03 finding 1's Tier 1 capacities (≈1.07 KB at 160 B, ≈2.2 KB at 233 B), which 03 computed for unsigned fragments, beside the SIGNED 3–5 B at 160 B from `07-gap-2` F4. `PATCH` and `AGENT_OP` are SIGNED-mandatory (`5-messaging/README.md:139-140`), and `07-gap-2` F4 puts a SIGNED fragment at 233 B at 76–80 B, so 16 fragments hold about 1.2 KB, not 2.2 KB. The picks survive (both say "only when it fits"), but the owner reading fact 16 would size the LoRa path at nearly twice what a fix or op can use. | Add the SIGNED figure and label the other: "≈2.2 KB at 233 B unsigned; ≈1.2 KB SIGNED (07-gap-2 F4), the figure that binds `PATCH` and `AGENT_OP`". |
| K3 | minor | §5 Q23 (`options.md:541-542`) | The question cites "`review/options-check.md` J2", a judge report that is rewritten each round (this file is round 2). The claim already rests on `07-gap-2` F5 and `05` §4.2, which the same line cites. | Drop the report citation, or write "round 1 J2". This file keeps the round-1 table below so the reference resolves either way. |
| K4 | minor | F13 A header (`options.md:391-392`) against item 5 (`:394`) | The header counts "one removal and two corrections, marked" (items 2, 8, 15). Item 5, "the probation wording (§4)", is a third correction of locked text: `1-bootstrap/README.md:158` "sustained across the 60s probation window" → 60 s of sustained green inside a 600 s deadline (§4 item 1, `probation.rs:57`, `N/mod.rs:133-176`). | "three corrections", and mark item 5 "(a wording correction, §4 item 1)" the way items 8 and 15 are marked. |

Not findings, recorded so the lead does not re-check them: the brief is 598 lines (the cap is 600; K1–K4 add no lines if applied in place);
`3-intelligence/TASKS.md:72` does support "the agent does not serialise concurrent turns" (it is in the deferred list: "serializing
concurrent turns on one conversation"); the Termux cron job does spawn `curl` every 5 min (`termux/cron/healthcheck.sh:12`), so F2's saving
is right; Layer 8's `111/catalogue-vouch/v1` (v1.1) and `111/log/v1` (its F11 B, not picked) are rightly absent from joint item 5's v1 list;
the Layer 8 brief's rename of the store-key tag (`111/agent-state/v1` in its reports → `111/node-state/v1` in its F1, stated in its §6) is
the spelling the box uses, so fact 23's "second SQLCipher file only `111d` opens" and item 5 agree with Layer 8's pick.

## Round 1, confirmed closed

| id | was | how it is closed now |
|---|---|---|
| J1 | major: the joint contract differed between the two briefs (items 1, 5, 6, 7) | The two boxes (`9-evolution/options.md:169-192`, `8-storage/options.md:168-191`) diff to nothing but the header's name of the other layer; both headers read "the same seven items, verbatim". |
| J2 | major: the phone's automatic `diagnose` `AGENT_OP` had no member-side consent | F11 adds "Ask my home hub to diagnose" (off / on, default off, a `409 consent_required` listing the exact request fields, the L6/L7 shape; under it `111d` sends at incident open with no T0 match, off nothing is composed); the §3 consent block lists it; the §3 privacy line names both egresses with their trigger; §5 Q23 carries the default and the alternative (a human tap per card). |
| J3 | minor: item 5 paraphrased L2 §6.1 as "signs only what leaves the node" | Item 5 now quotes §6.1 and states the "nothing that stays on the node" rule as Layer 9's own, stricter one; F9 echoes it. |
| J4 | minor: fact 10 gave Kali the 2-of-105 figure | "GenProg: 2 of 105 correct; Kali, which only deletes functionality, matched it" (`02` F14). |
| J5 | minor: "eleven reports" | "ten reports" (`options.md:3`). |
| J6 | minor: F13's "additively" covered a removal (item 2) and a correction (item 15) unmarked | Item 2 reads "a removal the placeholder text invited", item 15 "a correction of fact 4"; the header says "one removal and two corrections, marked" (K4 finds a third). |
| J7 | minor: `status.rs:61` and `README.md:156` | `status.rs:62` (fact 19) and `README.md:158` (§3 `update.rollback`). |
| J8 | minor: F7 "land now, whatever the owner picks" against F14's "first task of v1" | F7: "a Layer 3 correction the lead may commit before the Layer 9 decision (they change no Layer 9 behaviour and restore L3 `README.md:55`)"; F14: "(the deny block and `HOME` fix land earlier, F7)"; F13 item 8 says the same. |

## What I verified

**Facts of §0 against the cited lines (25 spot-checks; all hold unless listed above).**

1. Fact 1: `pub enum ChildState {Running, Backoff, Degraded, Stopped}` at `N/supervise.rs:415-421`; `pub fn state_watch()` at `:627`; `00` §1 and §3.1 (a)–(b) for the agent's tools.
2. Fact 2: `let _ = child.wait().await;` at `N/supervise.rs:972`; `crash_loop_threshold: 5`, `crash_loop_window_secs: 60` in `BackoffPolicy::default` at `:61-71`; the `Degraded` arm loops on `wait_for_request` until `restart` at `:996-1009`; `3-intelligence/README.md:51` "restarts a dead child ≤10 s"; `00` trap 1.
3. Fact 3: "You have no tools and cannot run commands" inside `run_model` at `A/tasks.rs:320-345`; `std::fs::write` in `save_proposal` and `save_task` at `A/store.rs:120-146` under the module promise "the writes atomic" (`:1-3`).
4. Fact 4: the applier's vocabulary constants at `N/devices/applier.rs:83-104` (four applied, two one-shots, four refused); the crash sentence at `:35-37`; `struct Snapshot {file, toml, config}` at `:835-839`.
5. Fact 5: `Journal {schema, pending, version, previous, deadline_epoch, health_confirmed}` at `probation.rs:7-17`; `PROBATION_SECS = 600` at `:57`; the 5 s `probation_monitor` interval at `N/mod.rs:136` and the 60 s sustained-green test at `:165`; `1-bootstrap/README.md:158` "sustained across the 60s probation window"; `00` trap 10 (wall-clock epoch seconds).
6. Fact 7: `1-bootstrap/README.md:39` (payload = `bin/111` + `share/kb/` + assets), `:169` ("The daemon can never modify its own payload"); `7-relay/README.md:857` ("9.1 The hub as origin": three file kinds); `00` trap 12.
7. Fact 8: `write_opencode_config` at `A/tasks.rs:490-509` writes provider and model only, no `permission` block; loopback trusted by policy except under Termux at `auth.rs:3-10`; `3-intelligence/README.md:55`; the critic §2.1 and `07-gap-1` F1–F2; `00` trap 4.
8. Fact 9: `07-gap-1` lines 10–11 and F6–F7 (1.7 MB gzipped measured with `git archive`; 349 crates, 87.6 MB; 4.5 and 14.4 MB; a C compiler, make and perl for `node111`; 111–127 MB xz), F9 ("Do not quote a figure for the owner").
9. Fact 10: `02` F14 (`:129-130`, GenProg 2 of 105, Kali matched), F16 (`:139-141`, Google 15 %, Meta 25.5 % landed); the critic's §7 list carries them as snippets, as does the brief's §6.
10. Fact 11: `01:149-152` (2.3 tok/s on a Pi 5 in Ollama, a snippet; 3–4 GB); `3-intelligence/README.md:27-36` (the post-install offer and the category table giving 2B the model).
11. Fact 12: `0-knowledge/README.md:32` (recipes `install-<project>-on-<device>.md`), `:143` (`skills/` deferred); `07-gap-3:30` (`build.py` is Python 3.11+, stdlib `tomllib`).
12. Fact 13: `07-gap-3` §2 table (`:42-46`: the enum 150–250 lines, +10–30 KB; `cel` 0.15.0 MIT, 11–13 to about 23 crates, +0.7–1.5 / +2–3 MB, eight releases 0.13.0 2026-03-12 → 0.15.0 2026-10-06; `serde_json_path` 0.7.2, no substring, counter or quantifier) and §4 A ("about 200 lines").
13. Fact 14: `5-messaging/README.md:173` (`0x10 AGENT_OP … interactive · per stream · SIGNED`), `:139-140` (SIGNED mandatory); `07-gap-2` F1.
14. Fact 15: `3-intelligence/README.md:48` and `:110` ("the LAN binding + pairing auth"); `07-gap-2` F2–F3.
15. Fact 16: `5-messaging/README.md:239` (Tier 2 for "larger objects and every bulk class"), `:453` (public signed objects stored plain); `03:17,49-50` (65,445 B; ≈1.07 KB and ≈2.2 KB); `07-gap-2:86-88` (3–5 B SIGNED at 160 B; 76–80 B at 233 B: K2); the release numbers at `6-transport/TASKS.md:82`.
16. Fact 17: `2-identity/README.md:224` ("the agent holds **zero** key material"), `:85` (the §6.1 sentence, quoted exactly in joint item 5); the A4 table at `:335-347` has `111/msg/v1` and no `fix` tag; the delegation cert at `:217` (expiry in mesh epochs).
17. Fact 18: `termux/sv/111node/finish:8` (code and signal to a log); `termux/cron/crontab.line:5` (`*/5`); `termux/cron/healthcheck.sh:12-15` (`curl -fsS` then an append nothing reads; `00` trap 9); `1-bootstrap/README.md:129` `Restart=always`, `StartLimitIntervalSec=0`; `systemd/111node.service:18,42-47` and no `ExecStopPost=`.
18. Fact 19: `STATUS_TICK = 2 s` at `N/dashboard/api_v1/status.rs:62`; `absent_poll` 2 s at `N/dashboard/api_v1/relay.rs:111`, slept at `:892` (K1); the 30 s interval at `N/dashboard/mod.rs:300`; `00:66` ("≈265 wakeups/min with no agent, against R07's ≈205"); `7-relay/README.md:237` (≈205 → ≈114), §3.6 at `:227`, §3.7 at `:240`.
19. Fact 20: `const PREAMBLE` at `A/chat.rs:37`; the task prompts at `A/tasks.rs:326-333` and `:401-406`.
20. Fact 21: `6-transport/README.md:458-459` verbatim ("it cannot do so").
21. Fact 22: `1-bootstrap/README.md:160` ("APK refresh = Layer 9 maintenance task"); `07-gap-2` F10 (`:163-183`: the three pins, the installer over `adb`, no daemon read, no receipt record, the `receipt.apk` addition, "no automatic install").
22. Fact 23: `8-storage/research/04-local-store.md:91-95` (Fork 5 A, namespaces `agent/conversations`, `agent/proposals`, `agent/tasks`, `heal/journal`); `07-gap-1.md:16,47-51` (`agent/state.db`, `111d` the only opener, pick A); `03-shared-log.md:28,50,117` (no shared log in v1, p2panda-core the target, "would reinvent p2panda-core"); `07-gap-2.md:179` ("a hub-owner-signed fix index"); the Layer 8 critic's conflict 9 at `:44-47` (plaintext `incidents.jsonl`, the hash chain, one L2 rule for the tags) and its §5 (the second SQLCipher connection).
23. Fact 24: `bearer_write_refused` at `auth.rs:927-933` (any bearer on a non-GET is refused); D26 (2) at `DECISIONS.md:392`; `00` trap 3.
24. The joint contract's tag list: Layer 7's `111/epoch/v1`, `111/broadcast/v1`, `111/carrier-topic/v1` at `7-relay/README.md:1285` (and `:631,820,968`); Layer 8's `111/node-state/v1` (its F1, `:145-146`, the rename stated at `:573-574`), `111/catalogue/v1` (its F5, `:259-260`, signed "on a human action"), `111/export/v1` (its F12, `:402`); Layer 8 F1 B is the plaintext fallback the box's item 7 names, and F12 is the export the box's item 6 names.
25. The Layer 7 role bits: `7-relay/README.md:770-772` (3 `ROLE_GATEWAY`, 4 `ROLE_OFFLOAD` "serves model inference", 5 `ROLE_ORIGIN`); `07-gap-2:125-128` (bit 6 free, set in node111 beside `appdata`).

**Fork picks against their reports (re-read this round where the fix pass touched them).** F1: `05:174,206,210` (the scope table, cap 3,
RF never-auto), `parked.md:11`, `devices.md:194`, the critic §2.2. F2: `01:100-101` (64 open ≈64 KiB), `05:76-79` (`ExecStopPost=`,
`OnFailure=` never fires), `05:42-44` (monit AGPL-3.0, Netdata GPL-3.0 with a non-free UI part and ~30–100+ MiB snippets, Prometheus
Apache-2.0). F3: `A/events.rs:17-21` (1 MiB, trim to half). F4: `01:131` (`json_schema` → GBNF), `07-gap-2:79-83` (≈0.5 KB request, the
two confidence fields), the critic G7. F5: `07-gap-3:73,85,152-153` (`within_events` ≤64; depth 3, 16 leaves, 2 KiB; the lint). F6: `02:18-19,37,40,63,103`
(LuCI 90 s snippet, ESP-IDF `PENDING_VERIFY`, "armed at start = revert"), `proposals.rs:170-180` (no `applied` field). F7: `07-gap-1:18-24,31,65-72`
(opencode MIT 1.19.5, 211–222 MB `unpackedSize`; the deny block and the five `OPENCODE_DISABLE_*` flags; `diffy` 0.5.2 both licences; bubblewrap
0.13.1 LGPL-2.1 "or later" unverified; nsjail Apache-2.0; pick B). F8: `03:21,32,65-67,85,164-165` (no agent `Signed-off-by`, `pull_request_target`,
Invariant Labs, `Assisted-by:`). F10: `03:107,114,150,171` (BLAKE3 `fix_id`, `author_seq`, 4 KiB / 64 KiB, 256 fixes ≈1 MiB). F11: `07-gap-2:97`
(composed at incident open with no T0 cause match), `:108-128` (enrolled, attributed, one request per `req_id`, `heal.serve_members` off, no
custody for a hub's own dest), `:142-152` (R1–R3, `111/msg/v1`), `:154` (no carrier ride). F12: `04:27-28` (±13 points at 70 % over ~50 cases),
`:76` (`T:458-459`), `:86` (16 GB / 64 GB), `:158` (60,000 tokens, 3.3 h), the GEPA and Inspect AI rows (MIT). §3: `N/health.rs:24-41` (two-state),
`N/intelligence.rs:10-12` ("No key material ever reaches it"), `3-intelligence/TASKS.md:72`, `1-bootstrap/README.md:158` (`revert-request.json`),
D21 at `DECISIONS.md:216-220` (64 KB per lazy chunk), `4-interface/ui/src/lib/layers.ts:75`, `os/README.md:20,67,79,118`.

**Completeness.** Unchanged from round 1 and re-confirmed: the critic's twelve conflicts are each settled by name (§6 maps them: F6 1 and 3;
F1 2; F12 4; F5 5; F4 6; F7 7; F10 8 and 9's exchange half; F3 9's storage half and 11 with F9; F2 10 and 12), and each settlement sits in the
named fork. The twelve owner forks: 1 → F1; 2 → F2; 3 → F4; 4 → F5 and F12; 5 → F7; 6 → F8; 7 → F10; 8 → §5 Q1; 9 → F9; 10 → §5 Q2 and F8;
11 → F12; 12 → F2 and F3. Gaps G1–G8 closed or honestly left (§6). The critic's thin items (README:20, README:118, the APK hand-off, README:79)
are F8, F11, F11's APK paragraph and §5 Q1. Every hand-off in the plan README is addressed (Layer 1 → F2, F11, F6/§3; Layer 3 → F4, F7, F2, F5,
F8, §3; Layer 4 → F6, §3, F1; Layer 5 → F10, F11, F6; Layer 6 → F6, F1; Layer 7 → F10, fact 7, F9 B; P4 → F6 and §4 item 15). The critic's
unverified-claims list (§7) is carried whole into §6 with the gap-fills' lists merged; §4 item 18 records that three gap-fills exist against
the README's "cap 2".

**Rules.** Pi floor: every fork states its resident cost (F2 "+0 timers, +0 processes, +0 listeners"; F1, F3–F6, F8–F12 zero resident), hub-only
costs are quantified or marked unmeasured (opencode RSS, the delegated diagnosis's per-request RSS, a cold `cargo check`, the model reload),
the one build-time weight on every `111-agent` (`diffy`) is named and sent to the owner (F7), and the gate (G7 extended plus Layer 7's no-timer
line) is in §3. Adopt-don't-reinvent: F5 against `cel`, `serde_json_path`, `jaq`, `rhai` and six prior-art systems; F6 against NETCONF, Nix,
ostree, A/B; F2 against monit, Netdata, Prometheus; F3 C rejected for p2panda-core; F8 adopts the `git am` mbox; F12 names GEPA and Inspect AI.
Offline-first, consent, no telemetry, capacity, no wall clock: pruning by capacity (joint item 3), `accepted_by_n` as a count (F10), the ledger
count-based (F6), 90 s monotonic while pending only (F6), git's fixed datestamp (F8), 03's hourly cap dropped (§5 Q18), both egresses under
their own off-by-default consent (F10 send, F11 "Ask my home hub"), README:79 squared in §5 Q1. D22: every adopted or pinned program's
licence was read from the upstream file by a scout and the "or later" and crates.io-only cases are marked. D26 (2) and the L2 zero-key rule:
F1 A leaves D26 (2) untouched, F1 B is the owner's call with the critic's reading cited; F9 keeps all signing in `111d` under two registered
tags; `AGENT_OP` payloads ride `111/msg/v1`; the hub's reply is never auto-applied. Additive amendments: F13's sixteen items are each
owner-visible, the removal and the corrections marked (K4 asks for one more mark); none silently reverses a DECISIONS.md entry (D12 in F7 C,
D29 byte-for-byte in F6, D30's git rejection in F7 and F8, D21 in F13 item 10) or contradicts a locked spec; the wire-visible additions are named.

**The joint contract.** Stated in F3, boxed, marked joint, identical to the Layer 8 brief's box save the header's name of the other layer
(J1 closed), with the fallback (item 7), the §6.1 quotation (J3 closed) and the two Layer 8 report sections it descends from cited.

**Form.** The Layer 7 brief's shape (§0 facts, §1 paragraph, §2 polled forks with the pick first, §3 detail picks, §4 plan corrections, §5 open
questions with a bold default each, §6 critic, gap-fill and unverified claims) and status line ("Status: 🧭 deciding — awaiting the owner …
lead-written from the ten reports"); 598 lines; no model name or identifier (grep for the roster names and the model family names: none); no
attribution line; every §2 fork has its options, its costs (F13 and F14 excepted, as the Layer 7 brief's permission and sequencing forks are),
the pick first and the locked text it amends; every §5 question (1–23 with their sub-questions) carries a bold default.
