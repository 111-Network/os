# Layer 7 spec v0.2 — fable re-check (2026-11-02)

Judge: fable, read-only, on head `0e9985f` (D31 commit; the spec file as edited in the working tree, 1,468 lines). Input: `custody.md`,
`carriers.md`, `integration.md`, `triage.md`, D30 and D31, the editor's result. Every `file:line` a disposition rests on was re-read in the working
tree or the local cargo registry (`prune.rs:228-245`, `:484-509`, `:818-833`, `rows.rs:92-105`, `:1010-1030`, `fabric.rs:380-451`, `:586-616`,
`:645`, `:1148`, `:1802-1855`, `:2824-2862`, `:3030-3061`, `:3093-3130`, `dashboard/mod.rs:25`, `:259`, `routes.rs:16-18`, `:297-302`, `:469`,
`auth.rs:1023-1029`, `fetch.rs:161`, `:205-215`, `stage.rs:73-90`, `:124-142`, `updater/mod.rs:85-125`, `rnstatus.rs:38-42`, `:86`, `:96`, `:135`,
`sealer.rs:141-148`, `fabric/mod.rs:334-343`, `contacts.rs:141-171`, `N/mod.rs:841-844`, `bridge.sh:18-19`, `probe.rs:259-268`, `http.rs:4-33`,
`services/mod.rs:340-348`, `:1484`, `devices/mod.rs:352`, `fixtures.ts:372-378`, `proposals.rs:103`, `nearby.rs:41-129`, `settings.rs:83-96`,
`:364-380`, `API:596-598`, `:754`, `:888`, `:903`, `:1096-1118`, `:1392-1401`, `:1698-1712`, `gen-sse.mjs:276`, `:407`, `store.ts:425`, the four
README lines, `LICENSING.md:103`, `:112`, `:239`, `MD/service_daemon.rs:696-704`); all hold. `git diff ec9954b 0e9985f` over the cited code trees,
`openapi.yaml`, `LICENSING.md` and the READMEs is empty, as §0 claims. `settings_locked`, `needs_hub`, `component_unavailable`, `rate_limited`,
`unsupported_action` and `upstream_error` are in the `Problem.code` enum; `not_hub` and `pair_warning_required` no longer appear in the spec. No
`checkpoint-layer-6` tag exists (only `-4`, `-5`), as G9 now says. No model name in the spec.

**Verdict: lock.** Every accepted and owner-answered disposition landed where the triage put it; no rejected finding exists (the triage accepted
all 61 and the eleven deviations). The edits introduced no contradiction of weight, no number drift, no default turned on, no v1.1 item in v1, no
thirteenth Layer 5 spec amendment (the `fwd_from` column sits inside amendment 5, the counters inside 11, the prune lines inside 6, `API:888` inside
8; the stream seam and `attributed_on` are §19 code seams under D31 (1), (2)), and no resident cost on a non-hub node. Three minors below are
wording and a stated-limit gap; the lead can fold them into the lock edit (D32) without another panel round.

## Dispositions

| id | state | where in v0.2 / note |
|---|---|---|
| C1 | landed | §5.2 last bullet, §5.3 am. 5 (`fwd_from`), am. 6, am. 11 (`forward_refused`), §5.5, §5.6 honest limit, §5.8 (refused, share, watermark), §5.9, §14, §16 disk row, G9 item 4. Editor's deviations (32 B column; session counter) are sound; the counter's restart limit is unstated (R3) |
| C2 | landed | §5.2 "from a member" three clauses, §5.5 first sentence, §5.6 "held and will be offered", G9 item 3 |
| C3 | landed | §4.6 step 1 bundle fields (`role`, `via`), step 2 complement rule, `409 pair_role_mismatch` in §13.1, §5.1 last row + paragraph, §5.2, §5.7 (b), G9 item 5. Stricter `via`-match reading; the parser's left-3/right-2 split keeps IPv6 |
| C4 | landed | §5.1 "A hub's own messages", §17 row, §19 Q3 |
| C5 | landed | §5.3 am. 2: the whole scope sentence, `queued` / `sent` as the code at `:3053-3061` |
| C6 | landed | §5.9 "Re-posts" (1)–(4), race stated, `paired_hubs[].reposts` in §13.1, G10 item 3; (4) once-after-restart is an honest addition |
| C7 | landed | §5.9 "Acks", `forward_take` doc, `flush` → `Vec<(Option<u64>, ForwardDone)>`, G10 item 3 |
| C8 | landed | §5.3 am. 6 names `prune.rs:484-509`, `:228-245` (`:247`), `rows.rs:92-105`, `:1010-1030`; §5.8 first bullet. Lines verified |
| C9 | landed | §4.6 outcome + JSON, §5.3 am. 10, §19 code seam 3 |
| C10 | landed | §11.1: 80 / 116 / 79 / 43 (arithmetic re-done) |
| C11 | landed | §5.3 am. 8 cites `API:888` with the widened sentence |
| C12 | landed | §5.4 `n_ext` row; 11 B alone, 10 B beside another TLV |
| C13 | landed | §6.2 Drain: stale by the L2 window (`L5:375-377`) |
| C14 | landed | §5.5 "What a probe shows", §14 exposure row |
| C15 | landed (✅*) | §5.2 "Too large for the carrier", `forward_too_large`, §6.1 item 3, §13.1 `forward.too_large`, §17 row |
| C16 | landed | §5.9 `REJECT_FULL` retry rule, G9 item 6 |
| C17 | landed | §8.1 issuer row + 104 B transferable form, §8.2 write-on-advance, §8.3 |
| C18 | landed | §5.2 "A stale hint": repeated TLV, `:1843-1849` beside `L5:333` |
| S1 | landed | §9.2 "The staging lock", §18 L1, G12 (exit 75, concurrent run stages once). Loop-waits-in-`spawn_blocking` is the editor's fair reading |
| S2 | landed | §3.3 "The re-arm", G8 (0 within 2 s, A records changed); bench item named |
| S3 | landed | §1 rule 1, §3.3 (0 only after the second bind; 5 s stays on failure), §16 timers cell, G8 "reads 5" |
| S4 | landed | §9.1 "The origin base", Host guard cited at `routes.rs:297-302`, G12 |
| S5 | landed | §9.2 "The payload fetch", `record_success` at `fetch.rs:205-215` (verified), §18 L1, G12 |
| S6 | landed | §6.2 own `ureq::Agent`, 5A `spawn_blocking`, 5B thread `111d-ntfy-5b`; §16 threads row; §1 rule 3 |
| S7 | landed | §4.4 `inbound` → `(stream, Option<NodeId>)`, G11 |
| S8 | landed | §9.1 cites `routes.rs:16-18`, `:469`; GET only, no CSRF, same CSP |
| S9 | landed | §4.3 no SNI for an IP literal |
| S10 | landed | §4.5 unverified mark, G11 `cargo tree -e features`, sign-off row + §5 counts (121→122, 187→188 verified), §18 L1 |
| S11 | landed | §6.2 `since=all` unverified |
| S12 | landed | §6.3 server row, §9.5 T6 third use (own agent, never `Fetcher`; token in clear) |
| S13 | landed | §6.2 consent copy "a short tag of the two hubs' ids" |
| S14 | landed | §3.2 the check is the drain; §6.2 5A flush |
| S15 | landed | §3.3 second `netlink::debounced` instance; §19 D8 |
| S16 | landed | §3.7 window 1 with no tracer; window 2 outbound; window 3 optional |
| S17 | landed | §16 5B bytes/day "unmeasured until the smoke log" |
| S18 | landed | §3.5 item 4 `requires` copy and the mock fixture |
| S19 | landed | §6.3 table of every setting with default, values, file |
| S20 | landed | §2.4 item 4, §6.3 preamble; chatmail rides the component |
| S21 | landed | §4.2, §19 C9 and D14 |
| S22 | landed | §4.6 step 3 second `ENROLL` on the first hub's connection; D10 |
| S23 | landed | §7.7 browse gated on one full back-off cycle |
| S24 | landed | §9.5 T2 order; one function for daemon, helper, installer |
| S25 | landed | §3.6 services-registry `Notify`; Meshtastic monitor on the devices `kick` |
| S26 | landed | §16 "−1 request/min (probe), its wakeup delta measured" |
| S27 | landed | §19 item 11 and D12; §5.9, §17 |
| I1 | landed (Q2 b) | §2.1 item 4 (accessor, honest limit), §2.2 row 1, §2.3, §19 code seam 2 with flip, G8 LAN-vs-hub. One gap in §2.3's closed set (R2) |
| I2 | landed | §12 pinned round-trip, four actions, `accept_warning`; §4.1 rule 4, §4.6 step 6, §13.1, §13.2. Metered cap rides `carrier_enable` (no fifth action: consistent with the triage) |
| I3 | landed | §13.1 `needs_hub`; five codes appended (I3's four + C3's); §13.2 six-item pin list ending with the conventions' line |
| I4 | landed (✅*) | §2.4 item 2, §13.1, §13.2, §19 D15, G8 exact codes; `rns` → `409 settings_locked` + detail (code exists). One sentence overlaps (R1) |
| I5 | landed | §7.5, §3.8, §19 D16 |
| I6 | landed | §10.1 rule 1, §13.1 `POST /nearby/hops` (202 / 409 / 429), §13.3, §18 L6, G8 |
| I7 | landed | §9.1 `auth.rs:1023`, `routes.rs:469`; "Not gated" list and L1 contracts; out of `openapi.yaml` (the lighter branch) |
| I8 | landed | §2.3 512-row cap, oldest forgotten |
| I9 | landed (✅*) | §7.2, §18 L5 and L2, §19 code seam 4: node111 only |
| I10 | landed | §10.1 rule 4 coarsest cell; §17, §18 sidecar export v1.1 (the stricter branch) |
| I11 | landed | §3.7 windows, `strace` prerequisite with skip rule; §15 gate homes |
| I12 | landed (✅*) | §2.4 item 3, G8 validator unit tests, G10 item 6 bridge stub |
| I13 | landed | G9 items 1 and 6: committed tape, Layer 6 lock tag (none exists; lead tags it), `REQUIRE_G9=1` |
| I14 | landed | G12 "the flip": node111 tests on a scratch root (`updater_stage.rs`, `helper_guard.rs` exist); `origin.sh` staging only |
| I15 | landed (✅*) | §13.1 `POST /relay/pair`, `DELETE /relay/paired/{node_id}` loopback only; CLI covers the phone |
| I16 | landed | §19 item 11, D12–D16 numbered and cross-referenced |
| D1 (Q1) | landed | §4.4 status + flip, §18 L5, §19 D1 and code seam 1; the L5 §8 row rewording dropped per D31 (1) (`:361`, `:366` stay) |
| D2 · D3 · D4 · D7 · D9 | landed | text unchanged, as the triage asked (§19 entries present) |
| D5 | landed (✅*) | §5.8 128 MiB + `quota.transit / 8` = 16 MiB; §19 D5 |
| D6 (Q4) | landed | §7.3 pinned, built v1.1; §7.5; §19 D6 |
| D8 | landed | §3.3 with S15's wording; §19 D8 |
| D10 (Q5) | landed | §4.6 step 3 with S22's rule; §19 D10 |
| D11 | landed | §6.1, §6.2 with S13's copy; §19 D11 |
| numbers | landed | 15 min (§3.4), 168 h (§6.2), 2026-01-01 (§8.1), 1.10 × (§3.7), 5 s (§7.1), 128 / 16 MiB (§5.8): each marked "a default; the owner may change"; §19 lists them |
| Q3 (C4) · Q6 (C1) | landed | as C4 and C1 above; §19 "Owner forks" names all six |

## Blockers

none

## Majors

none

## Minors

| id | where | what | fix |
|---|---|---|---|
| R1 | §2.4 item 2 (lines 141–144) | "Every write to `presence.heartbeat.<transport>` (a `PATCH /relay/settings` or an accepted proposal) answers `409 no_such_transport`" and, two sentences later, "the proposal action `presence.interval` is refused `unsupported_action`": an accepted proposal is given two codes. §13.2 and G8 say `unsupported_action`. | Drop "or an accepted proposal" from the first sentence: the PATCH answers `no_such_transport` (or `settings_locked` for `rns`); the proposal action is refused `unsupported_action` before any transport check. |
| R2 | §2.3 bullet 2 vs §2.2 row 4 | New in v0.2: `evidence[].transport` "is `tcp-lan` or `hub`" (a closed set), but §2.2's "a paired hub answered (a `CUSTODY_ACK` or a forward put)" refreshes that hub's row, and a `carrier` pair's ack arrives through `carrier_inbound` with no link at all; the hub's own row ("your hub answered") also needs a value. The set has no member for either. | State the rule: the home hub's and a `tcp` pair's row carry `tcp-lan`; a `carrier` pair's row carries `carrier` (the pair's `via`); add `carrier` to the closed set in §2.3 and to the UI's enum mirror in the §13.2 pin list, or say paired-hub rows reuse `tcp-lan` and a carrier pair gets no presence row in v1. |
| R3 | §5.8 "Refused", §5.3 am. 11, §13.1 `forward` | The editor's result records that `forward_refused` and `forward_too_large` are session counters in `Counters` (`:645`, in-memory like `dropped` and `stale`), so the fabric view's count reads 0 after a restart while refused rows still sit in the store as `committed = 1`, indistinguishable from handed-over rows; the spec text does not say so. | One sentence in §5.8: "The count is this session's (`Counters`); it restarts at zero with the daemon, and a refused row is thereafter an ordinary committed row." Optionally `forward.refused_since` (a monotonic age) in `RelayStatus` so the UI can label it. |

## Notes

- The deviations the editor reports are all within the triage's reading: `fwd_from` is the minimum the per-origin share needs and lives inside
  amendment 5 (+32 B per forward row, in §16); `via` in the bundle is the fail-closed form of C3; five codes rather than four is C3's `pair_role_mismatch`
  (D31 (7) says "four appended error codes" — the lock entry should say five); `attributed_on` over a `via` on the view is the smaller seam and its
  limit is stated; the metered cap riding `carrier_enable` matches I2's own fix text.
- The new §0 "Finding ids" note resolves the `Cn` collision between §19's code corrections and the panel's C findings; "panel Cn" is used consistently (4 sites).
- §5.8/§14 "the per-origin share bounds what a dead pair can pin" is true per origin; across origins a target that never answers can still pin up to
  the Transit quota. Honest enough for a cap, but the lock text could say "per origin".
- §7.5's "offer JSON gains `gateway {carrier, mode}`" was in v0.1 and the panel accepted it (I5); note for the build that the `111HUB:` text bundle
  has fixed fields (`N/fabric/cli.rs:74`), so the fact reaches the human on the hub's offer screen, never the member node, exactly as §7.5's last
  sentence admits.
- Changelog C3 line reads "the shower's proposed role" (showing hub); a typo for the lock edit.
- §5.10's "6.3× the item" was 6.5× at 1,235 B / 190 B in v0.1 too; informative arithmetic, not a regression.
- Build watch carried from the panel, unchanged by v0.2: the single-actor forwarder (probe after the ack), `class_bytes`/`evictable` by `fwd`,
  the re-post rule and ack lifecycle in G10, the role check at `POST /relay/pair`, the mdns-sd re-arm as G8's first test, and the `111` Nearby
  id being 64 hex where `is_hex32` expects 32.
