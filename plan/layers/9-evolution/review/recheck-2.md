**Verdict: lock.** Every open id and every R id of `recheck.md` is fixed (R2 with its rule placed in §7.3 and §18.4, acceptable). The amend added four minor regressions and no blocker or major. Each minor has a one-line fix below, for the lead to apply at the lock.

# Layer 9 spec v0.3: the second re-check (2026-11-15)

Read: `git diff -- 9-evolution/README.md` (v0.2 at `abced06` → v0.3, uncommitted), with every touched section read as it now stands, against
`recheck.md` and the lead's ruling, `triage.md`, D35, the root README, `8-storage/README.md` (locked v1.0) and AGENTS.md's Pi floor. Code and contract
lines were read from committed content only (`git show HEAD:<path>`, HEAD = `abced06`). The two modified `relay/` files in the tree belong to
another agent and are not cited by the spec.

## Per id

| id | state | note |
|---|---|---|
| I1 | fixed | §3.3 rule 5: `111d` takes `node/lock` (L8 §13.2) first, and a held lock exits 75 before the marker or undo is touched. A new inbox row covers `result=exit-code code=75` (Linux) and `result=exit code=75` (Termux): it opens nothing, is folded with its code and the L8 J2 evidence, and is never a crash. The "any other line" row excepts it. G19 (3) asserts the byte-unchanged marker and undo file and the no-op line. This matches `8-storage/README.md` J2, §13.2 (`111node.service` `Restart=always`, `RestartSec=5`) and the hand-off. |
| S20 = I25 = O23 | fixed | All four re-pinned and correct at `abced06`: `sealer.rs:466-467` (`fn sign` → `sign_with_tag`); `proposals.rs:418-424` `decide_applied`, `:26-28` its description, `:163-168` `DecisionRec {decision, at}`; `API:2044-2060` ending `registry`; `schema.md:96-111` ending `relations`. The author also moved `tcp_lan.rs:714` → `:734` (`impl Link for TcpLan`), which is correct. No statement names an older head except the historical v0.1 changelog line. §24 C12 names the moves. |
| R1 | fixed | §7.4 step 2: while `state/heal/undo` exists in any phase, an accept and an `apply_unrecorded` answer `409 busy` and nothing is armed. An Undo is refused the same way. Only the named attempt's `/restore` proceeds. The guard is repeated in §4.1 rule 4, §7.5 and §7.7, and §16 rows 15 and 18. G20 (12) asserts an accept between `revert_blocked` and Restore, then a byte-exact Restore. The assertion gap for the guard's other arms is a regression minor below. |
| R2 | fixed | §7.3 rule 1: `111d` decides the proposal through `decision_begin`/`decision_complete`, decision `dismissed`, `applied: {withdrawn: true}`. It sets the `heal/proposals` record `withdrawn` (already a §4.2 state) and appends audit `dismissed` `by: "daemon"` (both already in §4's audit vocabulary). The proposal leaves the open count and Needs you, and a later accept answers `409 already_decided`. The rule lives in §7.3 rule 1 and §18.4's `oneOf`; §18.2 points to §18.4. G19 (12) asserts it. Widening it to `fix` and `human_step` is acceptable: a still-`undecided` received fix files again at the next incident open (§11.4 step 7). |
| R3 | fixed | Row 8 keeps `disk_low` only, with subject `storage`, which is added to the closed fixed-subject list. Row 5 owns `store_full`, since `capacity.store_full` is the same flag (L8 §13.4, §16.3). Row 9 owns the closed store, with Layer 8's `unavailable` as `storage` evidence. Rule (1), §4.1 rule 2 and §3.5's closed evidence kinds follow. G19 (13) counts one incident for each failure. No `node_state_unavailable` remains. |
| R4 | fixed | §18.1 `/dismiss` takes `revert_blocked` or `revert_failed`. §7.4 step 6 records it `dismissed`. §7.5 removes the file after a start-time `revert_failed` once the attempt and incident are written. §14.1 Needs you shows "Leave as is" for `revert_failed`. G20 (11) asserts both. |
| R5 | fixed | (a) §7.2 `settings.set` on a quota key goes through Layer 8's `quota.set` (L8 §16.4: it applies through `PATCH /fabric/settings` with that route's gate), asserted in G20 (13). (b) §11.4 step 1 and §12.3: write-before-return and `msg_id` idempotence (L8 §12.3 rule 2), asserted in G22 (12) and G23 (9). (c) The §18.1 Paging paragraph sets `?after_seq&limit` with `limit ≤ 1,000` (L8's J9 default) and the reply field `next_after_seq`; the playbooks list has no cursor. G20 (17) asserts it, and the bound is under Numbers, marked default. |
| R6 | fixed | The §16 preamble names the switch-independent changes of rows 7, 15 and 18 and counts five corrections (5, 7, 8, 15, 18). Rows 7, 15 and 18 are marked **correction**, citing `N/devices/applier.rs:993` (`self.one.lock().await`, verified). §23 L3, L4 item 18 and L6 item 15 agree, and the Layer 6 posture control shows `busy`. In G20 (1) and (14) the byte-identical claim now covers only an accept made alone. |
| R7 | fixed | §16 row 22 (L8, additive, **new**) lists the seven requests. §23 L8 names them and the three Layer 8 rules relied on unchanged. §24 D46 lists them for the owner. |
| R8 | fixed | §3.5: `state: "open"\|"closed"`, `outcome` with `dismissed`, `attempts: [Attempt]`, `explanation: {task_id, text}\|null` rendered preformatted. §5.2's input omits `attempts` and `explanation`, and its fold sets `diagnosis` or `explanation` by mode. G21 (3) asserts it. |
| R9 | fixed | §14.2 defines `refused` as an accept `111d` refused or a received fix in state `refused`. A human's "no" reads `dismissed`. G20 (16) asserts it. |
| R10 | fixed | §17.1 `heal.fixes.send` ∈ `off`, `home_hub`, `contacts`. Under `contacts` the human picks the home hub or any Layer 5 contact for each send, each send under its own `fix_send` naming the recipient. `pinned_contacts` is not offered and `pinned` gains no meaning. §11.3, §15 item 7 and D8 agree, and G22 (2) asserts it. This closes the safety author's open point. |
| R11 | fixed | §17.1: `111 heal status \| incidents \| fixes \| playbooks \| audit` as loopback GETs printing §18.4's shapes. §16 row 19 and §23 L1 item 19 are widened. The audit and settings reads are "session or loopback". G20 (15) asserts `111 heal incidents`. |
| R12 | fixed | All six assertions are present: the undo re-read refusal and draft-creating `503`/`409 heal_full` (G20 (17)), the unknown setting value (`api_v1_repairs.rs`), the one notification (`heal_observer.rs`), one `(2, identity, …)` incident (G19 (14)), and a vouch for a reclaimed fix (G22 (8)). |
| R13 | fixed | §11.1 `incident: {fingerprint, cause_id ≤ 24, diagnosis ≤ 512 B}`, where the 24 B bound is §6.2's `[[cause]] id` bound. `diagnosis` is the cause's one-line text. G22 (10) refuses a missing `cause_id` or an extra field. |
| R14 | fixed | All eleven places are reworded: the status line, §0, the ledger "always (§4.2)", §7.2 "Layer 8 §9.2's form", §7.3/§23 L4 "L8 §15.1, §16.4", §11.1 "L8 §6.4, §21 L2" (L8 §6.4 mints the catalogue `seq` from the announce counter, verified), §11.4 step 3 indentation, §12.2 and G23 (5) without `queued`, and §12.4 without "counter restarted". The Changelog has v0.1, v0.2 and v0.3 entries. The v0.3 date is a regression minor below. |

The author's ten choices are ruled acceptable as written:
- (1) The `storage` evidence kind and the `storage` subject live in separate closed namespaces, and no Layer 8 ServiceId is named `storage`.
- (2) `after_seq` matches L8 J9's `read_stream(stream, after_seq?, limit)`. Layer 5's `since_seq` stays on its own routes.
- (3) Widening reads to "session or loopback" is what R11 needs. Trusted loopback is the human (§7.6), and Termux answers 401.
- (4) Refusing Undo and letting `/restore` proceed are both required by the guard.
- (5), (7), (8), (9) and (10) need no ruling.
- (6) is ruled under R2 above.

## Regressions the amend introduced

| where | what | fix | severity |
|---|---|---|---|
| status line (`9-evolution/README.md:11`) and Changelog v0.3 entry (`:1818`) | v0.3 is dated 2026-11-13, but the re-check it applies is dated 2026-11-15, and HEAD `abced06` (v0.2) was committed 2026-11-15. | In both places, replace "draft v0.3, 2026-11-13" and "v0.3, 2026-11-13" with 2026-11-15. | minor |
| §7.4 step 2 (`:640-642`), §16 row 18, §23 L4 item 18 vs §20 G20 (12) (`:1355`) | Three rules have no assertion: the undo-file guard's `apply_unrecorded` arm (`PATCH /api/v1/transport/posture` while the file exists, with no lock held), its Undo arm, and "the Layer 6 posture control shows `busy`". G20 (5) covers only the lock span, and G20 (12) covers only an accept. | Append to G20 (12): "a `PATCH /api/v1/transport/posture` and an Undo made in the same span also answer `409 busy`, and the Layer 6 posture control renders `busy` (UI test)". | minor |
| §7.4 step 2 (`:641`), "`409 busy` naming the attempt" | The rule names the attempt, but no shape carries it: `Problem` is `{type, title, status, detail, code}` (`API:682-690`), and §18.2/§18.4 add no member. G20 (12) cannot assert "naming the attempt" without a field. | §7.4 step 2: "answer `409 busy` whose `detail` carries the attempt id (`att-…`)". | minor |
| §18.1 Paging (`:1144-1146`) | `limit` has a maximum (1,000) but no value when it is absent. The new paragraph makes every store-backed list route paged, so a request without `limit` is undefined, and `111 heal audit`/`incidents` (§17.1) send none. | Add to the Paging paragraph: "an absent `limit` reads 1,000". | minor |

Checked and not regressions:
- No root README sentence is touched. §0's quotes are outside every hunk.
- No section, gate or gate item is renumbered. Gate items are only appended: G19 (12)-(14) and G20 (17). §16 row 22 and D46 are appended. Cross-references such as "G19 (7) … G22 (12)" still resolve.
- The Pi floor is unchanged: no process, listener, timer, poll or dependency. The read verbs are one-shot CLI calls.
- The one new number (1,000) has its source (L8 J9) and is marked default under Numbers.
- Locked-layer changes are additive or marked corrections: rows 7, 15 and 18 are corrections, and row 22 is requests listed for the owner.

## Citation pin (HEAD `abced06`, committed content)

Sixty-one `file:line` citations were read with `git show HEAD:<path>`. They cover the files the re-check named as moved and the files changed between `61ae26d` and
`abced06`:
- `N/fabric/sealer.rs:466-467`
- `N/dashboard/api_v1/proposals.rs:26-28`, `:56`, `:104-114`, `:163-168`, `:418-424`, `:638`, `:698`
- `API:2044-2060`, `API:682-690`
- `0-knowledge/_meta/schema.md:96-111`
- `M/link/tcp_lan.rs:734`, `M/link/mod.rs:231-242`, `M/link/lossy.rs:241`
- `N/fabric/mod.rs:471-484`, `:592`
- `N/devices/applier.rs:17-18`, `:35-37`, `:89-91`, `:100`, `:448-475`, `:992-1004`, `:993`
- `N/dashboard/api_v1/fabric.rs:441`, `services.rs:10-14`, `:113`, `bridge.rs:288`, `settings.rs:14-15`, `transport.rs:377-378`
- `relay.rs:111`, `:725`, `:738`, `:826`, `:892`
- `N/services/mod.rs:87-99`, `:707-722`, `:857`, `:1028`, `:1034`, `:1065-1068`, `:1152`, `:1451`, `:1459-1468`, `:1515`, `:1576`
- `N/mod.rs:906`, `:1084`, `:1106`
- `N/intelligence.rs:10-12`, `:172`
- `N/supervise.rs:61-71`, `:682`
- `ID/keyset.rs:475`
- `A/tests/daemon_tools.rs:322`, `A/tests/chat_loop.rs:308`
- `A/src/tools.rs:113`, `:352`, `:382-403`
- `A/src/http.rs:159-164`, `A/src/tasks.rs:32`, `:360`
- `A/src/config.rs:100`, `:169-170`, `:303`
- `M/fabric.rs:1279`, `M/envelope.rs:81`, `:113`, `:1395-1401`, `M/store/prune.rs:44-46`
- `verify/seams.toml:1103`, `verify/t111/schema.py:69`, `7-relay/TASKS.md:91`
- `UI/src/views/agent/proposals.tsx:137`, `UI/src/views/settings/index.tsx:13-14`
- `1-bootstrap/systemd/111node.service:18`, `:42-43`
- `NI/update_guard.rs:94-96`, `NI/update_helper.rs:49-54`

**None is wrong.** One citation is imprecise but not wrong: `N/devices/applier.rs:448-475` (§18.4, `ApplyReport`). Line 448 is blank, `ActionResult` is `:449-464`, `ApplyReport` is `:466-476`, and the range stops one line before `ApplyReport`'s closing brace. It may be tightened to `:449-476` at the lock.

## Notes for the lock commit

- Apply the four minors above. None changes a rule's substance, and none needs a third re-check.
- The lock edit sets the version line to "locked v1.0" with its date and adds the D-number the lead assigns in `plan/DECISIONS.md`. The pin is `abced06` unless code moves before the lock commit. If it moves, re-pin by the files changed since `abced06` (`git diff --stat abced06 HEAD`), as this amend did between `61ae26d` and `abced06`.
- Commit `fix-map.md` with the spec. Its v0.3 table matches the text as amended.
- Carried from `recheck.md`, for the board, not the spec:
  - Name the injection case for `service.log` and `incident.read` in `9-evolution/TASKS.md` (triage, build order 8).
  - The `7-relay/TASKS.md:91` pin moves as that board grows.
