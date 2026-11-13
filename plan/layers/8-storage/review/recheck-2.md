# Layer 8 spec v0.3 — the second re-check (2026-11-13)

**Verdict: lock.** Every open id of `recheck.md` and every R id is fixed; the amend left eight minors of its own, each with a one-line fix below, to
apply at the lock commit. No blocker, no major, no fork open. Read: `git diff -- 8-storage/README.md` in full (989 lines), the sections around each hunk
as they stand (`8-storage/README.md` v0.3, working tree, 1,879 lines; lines below are `README:n`), `recheck.md`, `triage.md`, `fix-map.md` and its diff,
the root README, and the code as committed. The spec pins `8936a8e`; HEAD was `680162d` when this check began and `54c9bbf` when it ended (Layer 7 rows
D6 and D5c landed meanwhile). Citations were read at the pin and at `54c9bbf`.

## The open ids and the R ids

| id | state | where it is now | note |
|---|---|---|---|
| S1 = D3 = I1 | fixed | §5.2 (1) `:454-456`, §7.3 (3)(a) and (e), §7.4 states, §7.6, §17 rule 3 and attacks, G15 (5) | The sidecar's sha256 is computed as it arrives or is read and must equal `outboard_sha256` before its rename; a mismatch ends the job `failed (name mismatch)` before any data byte; "never replaced" names the sidecar; G15 (5) serves the true sidecar under the release payload's name, on a pull and on an import. The lead's addition is met word for word. |
| S22 = I23 = D25 | fixed | §0 `:28-33`, §7.8, §9.1 (5), §12.4, §16.5, §16.6 (1)(2), §18 test homes, §22 (8), C5, C6, C11, C13, `fix-map.md` | One head, `8936a8e`, named once; no "at `6dd9f4f`", `e1cdbd1` or `0b6d80b` remains (`f0b6806` stands only in the changelog's v0.1 line, as history). The tool count is 20 → 23. The four stale lines are corrected. Every sampled citation is right at the pin (below). |
| D9 | fixed | G15 (4) | "free space" left the asserted text list; the `facts` clause carries it. The author also moved `<needed>` and `<free>` out of the `update_without_copy` text into `facts` (§15.2, §16.2): the same rule, correctly applied. |
| R1 | fixed | §5.1 paths row `:442`, §7.6 `:620-630`, §16.1 adopt row, G15 (8) | The request file `incoming/adopt-<n>.json`, the staged catalogue copies, `catalogues`, `drafts`, `in_place`; signature, `seq` and `stat` checked again; all or nothing; G15 (8) asserts the three hand-offs. Regression 6 is its loose end. |
| R2 | fixed | §13.6 (5) `:1004-1017`, §15.1 row and "Who writes them", §15.2, §15.3, §15.4, §16.1, §16.2, §16.6 (10), §21 L1, L4, L7 (6), D19, G13 (14) | The refusal is `/status.update.last_result` = `refused_space` plus the notice; `POST /api/v1/storage/update-without-copy` is a human's (a session or loopback, bearer 401), per release and per store, forgotten once that release is confirmed or reverted; G13 (14) asserts the route, the bearer, the record, the forgetting and the time bound with a test rate. The lead's additions are met. Regressions 4 and 5 are wording. |
| R3 | fixed | §2.5, §13.6 (3) `:990-995`, (4) `:1002-1003`, §21 L7 cross-layer request, G13 (5) | The legacy files have their own trigger, the request covers the release before the first node-state release, G13 (5) runs both revert paths with no store copy. The amend's rewording of the `revert_to` step is regression 1. |
| R4 | fixed | §16.2 `:1212-1214`, §16.6 (10), §18 test homes `:1605-1610`, §21 L4, §21 L6, G13 (12) | `Proposal.decision.applied`, optional, as stored; both harnesses read it through `GET /api/v1/proposals/{id}`; the dead-daemon assertion moves to G13 (12). Listed under Layer 4 as an additive contract change and under Layer 6 as a lock-amendment candidate, as the lead ruled. The stored `applied` values hold no secret and no consent hash (`N/dashboard/api_v1/proposals.rs:26-28`, `:56-59`). |
| R5 | fixed | §0 scope table `:66` | Both quotations are the root README's words (`README.md:95`, `:147`); the limit is in the second column only. |
| R6 | fixed | §21 L7 (3) `:1689` | "six values after Layer 7's four": the five of v0.2 plus `store_start_empty`; §16.2 lists six. |
| R7 | fixed | §4.3 step 8 and closing paragraph, §16.3, G14 (7), (9) | §16.3 is the one list; an open or a close on use sends no frame; "step 3 waits for the fabric". |
| R8 | fixed | §2.2 rule 7 `:158-160`, G13 (8) | The first start of a build that writes a newer node-state version opens, copies and migrates before health. |
| R9 | fixed | §5.3, §5.4, §17 path tricks, G15 (7) | The pattern as given; a catalogue with another `version` is refused whole. |
| R10 | fixed | §5.1 row `:447`, §9.2, §9.4, §16.1, §16.5, §21 L3, G15 (18), G16 part B | `<publisher short>` is defined once, at the symlink, as the first 16 hex characters of the NodeId; `publisher` is optional on the route and the tool, default the home hub. |
| R11 | fixed | §5.5, §7.6, §16.2 `:1207-1209`, §16.6 (10), G15 (8) | One enum, `location` on both; no `absent` or `in_place` state remains. |
| R12 | fixed | §5.5 `:502-504`, G15 (8) | The fabric store's `unclean` of this start (`M/store/mod.rs:156-158`); every out-of-tree file `unverified` when that store did not open. |
| R13 | fixed | §5.5, §9.2 library, G15 (8) | One `stat` per in-place ZIM before the start and before each rebuild; asserted with `origin.datasets` off. |
| R14 | fixed | §4.3 step 9, §9.3 (3), §9.4, C16, D21, G16 part A | A search resets the same counter. |
| R15 | fixed | §3 J2, §13.2 `:948-951`, §21 L1 and the Layer 9 hand-off, Numbers, G13 (9) | Status 75, "a store verb is running", restarted by the unit; `StartLimitIntervalSec=0` (`111node.service:18`) means the unit never gives up while the verb runs. |
| R16 | fixed | §18 "Gate homes" `:1357-1360`, seam table, G13 (1), G7, G16, G18 | The Gates column equals the tags, checked mechanically over G13-G18 and G7: all twelve rows match. The knob is stated once. |
| R17 | fixed | §2.7 S3, §13.7 `:1025-1027`, §15.2, §15.3, §15.4, §16.1 two rows, §16.2, §16.6 (9)(10), §21 L4, G13 (10) | Both routes loopback only, refused to the bearer (G13 (6)). The spec has no typed confirmation anywhere, so the plain `consent_required` round trip is what the lead's ruling asks. Regressions 2, 3 and 7 are its loose ends. |
| R18 | fixed | §9.2, §16.2, §16.6 (7), §21 L4, L6 | `adapter` stays in `required`, typed `NetworkId` or null. |
| R19 | fixed | §2.5 `:218-220`, G13 (5) | `legacy_lines` on the head; G13 (5) adds a turn under N before the revert. |
| R20 | fixed | §3 J9, §4.5, G13 (15) | A spool ingest is written whatever the class's state. |
| R21 | fixed | §17 exposure table `:1312` | The row is added. |

**Count:** 24 rows (7 open ids in 3 rows, 21 R ids): 24 fixed, none partly, none not fixed.

## Regressions the amend introduced

| # | where | what | fix | severity |
|---|---|---|---|---|
| 1 | §13.6 (3), `README:986` against `:990-994` and G13 (5) `:1397-1400` | The `revert_to` step now sends the legacy files to "their own trigger", whose condition is "the running guard's build writes no node-state store". On the guard's path the running guard is release N's, which writes that store, and `revert_to` returns after the flip (`NI/update_guard.rs:22-33`, `:44-52`): N−1's guard does not run in that start. By the letter N−1 starts without its conversations, while `:993-994` says "Both rules run on both revert paths" and G13 (5) asserts it. v0.2 moved the files inside the `revert_to` step. | `:986`: replace "the legacy files move back under their own trigger (below)" with "and it moves the legacy files back whenever they exist (they exist only until the release that imported them is confirmed, item (2), so the release it flips to writes no node-state store)"; `:991` begins "At every other `ExecStartPre`, when …". | minor |
| 2 | §13.6 (5) `:1015` against §16.1 `:1185`, §15.4 `:1144`, §15.2 `store_start_empty` | A no-copy revert "leaves the store closed (`SchemaTooNew`) with §13.7's choices", but the new route answers 400 "for a store that is not closed after a set-aside", the page offers it "for a damaged one" and the consent text says "The damaged file": a too-new store is not set aside, so one of the two choices cannot be taken. | `:1015`: "a revert then leaves the store closed (`SchemaTooNew`) until the next update or `111 store import` (§13.2); Start an empty store is for a damaged store only (§13.7)". It agrees with the `update_without_copy` text ("closed until the next update"). | minor |
| 3 | §13.7 `:1026`, §16.1 `:1185`, G13 (10) | `start-empty` "lifts the closed state, so the next open creates an empty store … (the fabric store through its boot)". The fabric boots once per daemon start (`N/fabric/mod.rs:856`), so nothing says when messaging returns or what the page tells the user. | `:1026`: "the node-state store is created at its next use, the fabric store at the next daemon start (its boot runs once per start), and the page says so after the 204". | minor |
| 4 | §15.1 "Who writes them" `:1086` against the `update_without_copy` row `:1081` | "A paired session reads every setting … and changes none", two lines after the key a paired session writes. | "… and changes none but `update_without_copy`, through its own route". | minor |
| 5 | §13.6 (5) `:1007` against G13 (14) `:1435` | The rule says "refused before the flip"; the gate asserts "nothing is staged". A builder cannot tell whether a refused release may sit staged. | `:1007`: "is refused when its manifest is read, before anything is staged". | minor |
| 6 | §5.1 `:442`, §7.6 `:629` | The staged catalogue copies and `adopt-<n>.json` end only on success ("`111d` deletes them once adopted"). After a refused adopt, or a verb that died, they stay in `incoming/` with no rule. | §7.6, after "nothing is written": "the verb then deletes what it staged, and a dataset verb that takes `import.lock` first deletes any `adopt-*.json` and staged catalogue copy left there". | minor |
| 7 | G13 (10) `:1422-1425` against §16.1 `:1184-1185` | The delete route's "404 for any other name" and `start-empty`'s 400 have no assertion; the first is what keeps a route that deletes by name off a live store. | Add to G13 (10): "`DELETE /api/v1/storage/set-aside/{file}` naming a live store file, or a name no `set_aside_store` notice lists, answers 404 and deletes nothing; `start-empty` for a store that is open answers 400". | minor |
| 8 | §16.6 (2) `:1260-1262` against §16.5 "`names[17..20]` stays Layer 7's" | The pin list names the count pins and the constants but not the two open slice pins that appending three names makes false: `assert_eq!(&names[17..], LAYER7_TOOLS)` (`A/tests/daemon_tools.rs:325`, `A/tests/layer7_tools.rs:316`). | Add to (2): "the two `&names[17..]` pins (`A/tests/daemon_tools.rs:325`, `A/tests/layer7_tools.rs:316`) become `[17..20]`". | minor |

Checked and clean: no section, rule, gate or gate item is renumbered (headings and the item numbers of G13, G14 and G15 compared with the committed
v0.2; §16.6 gains (10) at its end); no root README sentence is reworded or contradicted; every locked-layer change is additive and listed in §21
(`Proposal.decision.applied`, `last_result` + `refused_space`, the consent action, the three routes, the search `publisher`, `adapter` kept in
`required`), and Layer 6's G5 change is named as a lock-amendment candidate; no resident process, listener, timer, poll or dependency is added (the
`stat` per in-place ZIM runs at a start or a rebuild, the consent is read at the updater's own check, the pass knob exists in a debug build only); every
new number has its source (75, 16 hex, the `version` pattern, 600 s, `RestartSec=5`, 20 → 23, 37, six).

## Citations

About 100 `file:line` ranges read from committed content, weighted to the files the re-check named as moved and to every citation the amend added or
changed: `M/store/{schema,mod,prune,rows,blobs}.rs`, `M/fabric.rs`, `N/fabric/{mod,sealer}.rs`, `N/origin/mod.rs`, `N/dashboard/routes.rs`,
`N/dashboard/api_v1/proposals.rs`, `N/panels/{static,manifest,mod}.rs`, `N/services/mod.rs`, `N/components/manifest.rs`, `N/updater/stage.rs`,
`N/relay/metered.rs`, `N/bridge/stub.rs`, `NI/{update_guard,update_helper,probation,manifest,cli}.rs`, `NI/install/categorize.rs`, the four agent111
tests and `A/src/tools.rs`, `API` (`:131`, `:190-195`, `:674`, `:692`, `:696`, `:749`, `:852`, `:1278`, `:1304`, `:1308`, `:1497`, `:2044-2059`), the
unit file (`:21`, `:22`, `:35-37`, `:42-43`), `devices.sh` (`:151`, `:329`, `:351-352`, `:356`, `:358-359`, `:363-364`, `:367-368`, `:402`, `:417`),
`smoke.sh:515-529`, `spine/node.sh:239`, `bridge.sh:696-700`, `4-interface/api/README.md` (`:17`, `:49`, `:87`, `:127`, `:160-161`, `:210`, `:221`),
`4-interface/README.md` (`:591`, `:717`), `7-relay/README.md` (`:951`, `:1002-1004`), `LICENSING.md` (`:56`, `:64-65`, `:151`), `matrix.md:177-178`,
`UI/…/l6/components.tsx` (`:52`, `:63`), `UI/src/views/settings/index.tsx:13-15`, `README.md` (`:95`, `:147`).

- **Wrong at the pinned head `8936a8e`: none.**
- **Moved since the pin** (HEAD `54c9bbf`; Layer 7 rows D6, `ffc1419`, and D5c, `96ed081`, with their board lines). The spec's statement about
  `8936a8e` stays true; these are what the lock re-pin must change:
  - `7-relay/TASKS.md:89` is `:90` (the U2 row; `README:1154` and the fix-map's I23 row).
  - "37 test files at the pinned head" (`README:1598`) is 38 (`relay_pair.rs` is new).
  - `N/fabric/mod.rs` gained 14 lines between `:1945` and `:1980`: every citation past that point moves by +14. `:2235` → `:2249` (`README:1803`),
    `:2235-2236` → `:2249-2250` (`:358`), `:2235-2256` → `:2249-2270` (`:360`), `:2237` → `:2251` (`:363`), `:2372-2394` → `:2386-2408` (`:361`, and
    §1 `:88`), `:2386` → `:2400` (`:361`, `:968`, `:1802`). Its citations below `:1945` hold (`:182-183`, `:229-230`, `:249`, `:592-607`, `:726-745`,
    `:911-928`, `:1301`).
  - `N/panels/mod.rs:536` is `:531` (`let want = setup(p.id()) && p.bundle_dir.is_dir()`); `:22-27` and `:56` hold.
  - `N/origin/mod.rs:1-37` (the module doc) is `:1-42`.
  - `NI/cli.rs:153-192` holds (the file changed elsewhere). `N/panels/static.rs`, `M/`, `A/`, `API` and the harnesses did not change.

## Notes for the lock commit

- Apply regressions 1 to 8 as written: eight sentences, no route, field or gate added.
- Re-pin at the lock head, as §0 promises: the head named at `README:28` and in the fix-map's v0.3 line, and the moved lines listed under
  "Citations". Layer 7 rows are landing while this spec waits (two since the pin): before the commit run
  `git diff --stat 8936a8e HEAD -- 1-bootstrap 3-intelligence 4-interface 5-messaging 6-transport 7-relay LICENSING.md` and re-read every cited
  file it names.
- The title (`README:1`) and the status line (`:11`) still read "draft for the adversarial panel"; `AGENTS.md`, `plan/INDEX.md` and the handoff still
  say "draft v0.2 in progress".
- Two editorial lines: §13.7 `:1027` "Their time on an SD card" now follows the two route sentences and should read "The checks' time"; §2.2 rule 7
  `:160` "migrates and closes it" against G13 (8) "closes the store on the next idle pass": say "and the next idle pass closes it".
- The author's twelve choices stand as written: status 75; `store_start_empty` and its text (with regression 2); `refused_space` appended at
  `API:749`; the `update_without_copy` key outside the PATCH; the request file and the all-or-nothing adopt; `legacy_lines`;
  `NODE111_TEST_HOUSEKEEPING_MS`; the `node_state` frame rule; the tools appended after Layer 7's (with regression 8); the three fixed values in
  Numbers; the live figures moved to `facts`; the 404 and the 400 of the R17 routes (with regression 7).
- For the owner at the lock, already listed in §21: the Layer 6 G5 change of means (R4), the Layer 4 additive set, the cross-layer request to
  Layer 7 for the guard's two rules (R3).
- For the board: the `update_refused_space` notice's lifetime (the route's 400 reads it; it is in memory and rebuilt by the next check) and how a start
  learns the node-state file's version without a write (R8 against G13 (13)) are the builder's to state in their rows.
