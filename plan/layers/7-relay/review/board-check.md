# Layer 7 build board — fable check, round 2 (2026-11-02, after the fix pass)

Judge: fable, read-only, on head `980b473` plus the untracked `7-relay/TASKS.md` (141 lines). Input: the spec `7-relay/README.md` v1.0 (read whole),
D30–D32, `EXECUTION.md`, the Layer 6 board (the model), `recheck.md` Notes and `triage.md`, and round 1 of this report. Every path in a Files or Check
cell was tested against the tree: every existing one exists (`tests/updater_stage.rs`, `tests/helper_guard.rs`, `tests/settings_lan.rs`,
`tests/fake_deltachat_rpc.py` included) and every missing one is marked `new` or `T0 stub`, with T0 listing each stub. The cites the fix pass added
were re-read: `crate::test_double::load` (`M/test_double.rs:515`) and the drift gate `frozen_files_match_generation` (`:1654`); `FabricHandle::fabric()`
(`N/fabric/mod.rs:622`); the actor's tick batch and "No timers" (`M/fabric.rs:28-34`); `Counters` (`:645`); `holds_custody` (`:356-360`); the INTRO
rule (`:2552-2558`); `store_secret_from` (`N/fabric/sealer.rs:126`) and `appdata` (`:141-148`); `publish` (`N/fabric/mod.rs:1075`), the `peers` dial
(`:516-522`), `roles_follow_settings` (`:1275`); the updater loop (`N/mod.rs:933`), the category re-probe (`:841-844`); `USAGE` (`NI/cli.rs:196`);
`write_update_request` (`N/updater/stage.rs:124`); the KB status gates (`0-knowledge/README.md:93`, `:97`); `meshcore.md:57` "up to 64 hops";
`crosscheck.py`'s five `load(...)` calls (never a subfolder); `pnpm budget` and `pnpm check` (`package.json:13`, `:15`); ureq with rustls
(`node111/Cargo.toml:28`); the minisign CI step (`ci.yml:55-56`); `LICENSING.md:103`, `:112`, `:239`. `git diff --stat 0e9985f..HEAD` over the cited code
trees, `openapi.yaml` and `LICENSING.md` is empty, so every spec `file:line` still holds. No model word outside the Model column (23 opus · 17 sonnet ·
2 haiku · 1 fable · 1 lead, the Layer 6 vocabulary); no attribution line; legend, Summary stub and External gates present; 141 lines.

**Verdict: ready.** No blocker, no major. Round 1's blocker (B1) and both majors (M1, M2) are closed as fixed, the eleven minors are closed, and the
fix pass introduced no regression: ownership stays disjoint within every wave (re-checked cell by cell, Wave 0 through Wave 3 and Gated), mesh111
edits are still exactly the twelve (M1, M2, M3, M4, M7) and the §19 seams (M6 seam 1, D2 seam 2, M4 seam 3, D9 seam 4 in node111), with M4's
`set_carrier_max_frame` an in-memory setter inside amendment 11's hand-off API that V9 records in D33; nothing in §17 is built; D32's order holds
(P1 first; L1–L3 in Wave 0; the twelve as one wave; C1 one task with its six pins); every daemon row carries a **never adds** (D11's "otherwise
nothing is constructed" is the same statement); every §16 figure has an owner, the two rows round 1 asked for included. Five new minors below.

## Round 1 findings: state after the fix pass

| id | was | state | evidence on the board |
|---|---|---|---|
| B1 | blocker | closed | C1: `POST /relay/pair` gains optional `lan: boolean` and `PairedHub` in `RelayStatus` carries `lan`; D6: `lan?` on the route, recorded in M5's `fabric/settings` row, `111 fabric pair <bundle> [--lan]`, a test of a `lan = true` TEST-NET pair running plain with no `pair_plain` consent; D5 names D6 as the source; V9 records it in D33. |
| M1 | major | closed | M2 owns `5-messaging/test-vectors/crosscheck.py` (an additive `layer7/` section, the five frozen sections untouched); its check runs `python3 -I … crosscheck.py` with that section asserted, and `cargo test -p mesh111 envelope::` loads `layer7/…` through `crate::test_double::load` under the drift gate. |
| M2 | major | closed | D8: "Find my hub" is `POST /registry/browse` on a member with a home hub (rows matched against the hub's `k`; no new route; the address-event path needs no call); U1 carries the button with a `layer7-radar.test.ts` item ("shown only with a home hub and sending one browse"). |
| m1–m11 | minor | closed | C1's status waits for A1, D2, D6, D8, D10 (m1) · K3 after D11 (m2) · D9 before D2 (m3) · D12 never beside K4 (m4) · K3 owns the `meshcore.md` line, K2 disclaims it (m5) · `Fabric::set_carrier_max_frame(Option<usize>)` named in M4 and D11, set through `FabricHandle::fabric()` (m6) · U3 has the metered line (m7) · bearer 401 on the browse (D8) and on check/flush (D10) (m8) · `origin.serve` and the pair routes answer `needs_hub`, `origin.serve` without LAN sharing `lan_off` (m9) · the memory and per-pair wakeups rows exist (m10) · "per tick (the actor's input batch …; no timer)" (m11). |

## New findings (round 2)

| id | severity | where | what | fix |
|---|---|---|---|---|
| n1 | minor | D3 Check | "the four codes replace the `wan_*` cases, `:929-952`": the `wan_unknown`, `wan_offline`, `wan_ok` assertions sit at `tests/api_v1_status_services.rs:943`, `:951`, `:955`. | `:943-955`. |
| n2 | minor | T0, P1 | T0 writes the §5.9 `Carrier` trait into `N/carrier/mod.rs` before Wave 0, and P1 lands first in Wave 0; the trait's `flush` takes `CarrierFrame` and returns `ForwardDone`, which are mesh111 types amendment 11 (M4) adds later in the same wave. P1 "behind a fake hand-off" implies stand-ins, but no row says where they live. | T0: `CarrierFrame` and `ForwardDone` are node111-local type names until M4 lands; D11 (which owns `N/carrier/mod.rs`) swaps them for mesh111's. |
| n3 | minor | D3 Check | D3 owns `N/relay/gateway.rs` (the `detail` shape, monotonic ages, the paired-hub carrier fact) but its check runs `--lib services`, `--lib meshtastic`, `--lib panels` and the status test only; `relay::gateway` is first exercised by D10's `--lib relay` and V2. | add `cargo test -p node111 --lib relay::gateway`. |
| n4 | minor | L2 Check | The named items of `--test updater_stage` (concurrent run, `Held`, blob rule, no `record_success`) omit two criteria of the row: T3 (the daemon's staging decision refuses `min-upgrade-from` above the installed release) and `check_internet = false` ⇒ no internet base; V6 later relies on `updater_stage.rs` for T3. | name both in L2's parenthetical so the Wave 0 check asserts the whole row. |
| n5 | minor | C1 Files, Wave 0 header | C1 owns `N/dashboard/api_v1/{proposals,nearby}.rs` (mirrors) in Wave 0 and stays 🔨 until Wave 1's D2 and D10 edit the same files; the ownership rule holds only because C1's edits to those two files end in Wave 0 and its later work is a re-run. | say so in C1's status rule ("no further edit to the two routers after Wave 0; the re-run touches nothing"). |

Not findings, noted: the "unregister-and-re-register form" of triage S2 is on the External gates list (bench) · the recheck's six build-watch items
(single-actor forwarder, `class_bytes`/`evictable` by `fwd`, the re-post and ack rules in G10, the role check at `POST /relay/pair`, the re-arm as G8's
first test, the 64-hex `111` id against `is_hex32`) each sit in the row the header names · `tests/fixtures/` exists and is empty, so `fixtures/l7-L1/`
is correctly `new` · the `checkpoint-layer-6` tag is an owner-side push (DECISIONS Pending), and the board takes the hash `4689dbf` until then.

## Coverage matrix (spec → rows; "—" = pinned, not built, and the board says so)

| Spec | Rows |
|---|---|
| §1 floor rules 1–4, non-goals | D3, D4, D8 (the removals); every daemon row's **never adds**; the §16 record; D4 (Termux poll); V7, V9 |
| §2.1–§2.3 presence, `111` kind, cap, SSE | D2 (facts, `attributed_on`, source, cap, frame), C1 (shapes, evidence set), U1 (evidence rows) |
| §2.4 heartbeat switch: setting · refusals · validator · zero weight | D1 · D10 · D2 (+V2) · D1; U2 shows it inert |
| §2.5, §2.6 | D2 (no rate limit); U2 (airtime line) |
| §3.1–§3.2 modes, "internet", checks (a)(b)(c), never `install.111.network` | D1, D11 (modes, drain-as-check, flush triggers), D4 (a), D10 (c route), D3 |
| §3.3 b1–3 · b4–5 · §3.4 | D4 · D8 · D4 (+D11 flush on the poll) |
| §3.5 items 1–4 · §3.6 · §3.7 · §3.8 | D3 (+C1 codes, U3 fixture) · D3 · V1, V7 · D3, U3, U2 |
| §4.1 rules 1–4 (incl. `lan`) · §4.2 · §4.3 · §4.4 · §4.5 · §4.6 steps 1–6, JSON · §4.7 | D5, D6, C1 · D5 · D5 · M6 · D5, K4, V5 · D6, M4 (3–4), M5 (JSON, 5), M7 (am. 12) · informative |
| §5.1–§5.2 gate, own-send limit, carrier bound · §5.4 · §5.5 · §5.6 · §5.7 · §5.8 · §5.9 · §5.10 · §5.11 | M4, D11 (bound) · M2, M4 · M4 · M4, U3 · M4 (V3) · M1, M4, D10 (`set_quota`), U3 (count) · M4, P1/D11 · V3, V5, V4 records · M4, V3 (7) |
| §5.3 am. 1 · 2 · 3 · 4 · 5 · 6 · 7 · 8 · 9 · 10 · 11 · 12 | M4+M7 · M7 · M7 · M7 · M1+M7 · M1+M7 · M2+M4+M7 · M4+C1+M7 · M3+C1+M7 · M4+M5+M7 · M4+M7 (+`set_carrier_max_frame`, D33) · M7+D6 |
| §6.1 · §6.2 · §6.3 · §6.4 · §6.5 · §6.6 · §6.7 | P1, D11, M4 · P1, M5 (topic key), D11, D1, D10 (consent copy) · D1, D10 · T0, P2, K3, V4 · D12 (gated) · item 3 K2; item 2 — · — |
| §7.1 · §7.2 · §7.3 · §7.4 · §7.5 · §7.6 · §7.7 | D8 · D9 · — · D8, V2 · D9, D3, C1 · D8, A1 · D8, M5, U1 |
| §8 | K1 text; §8.5 nothing built |
| §9.1 · §9.2 · §9.3 · §9.4 · §9.5 T1 · T2 · T3 · T4 · T5 · T6 · §9.6 | D7, K2, C1 · L4, L2 · L2, L4 · — (K1, K2 text) · L1 · L1 · L2, L3 · U2, L4, L2 · L4 · L2 · L4, V6 |
| §10.1–§10.4 · §10.5 | U1, D2 (hops), A1 · K2 (recorded, v1.1) |
| §11.1 · §11.2 · §11.3 | — · K1 (rule) · — |
| §12 settings UI · consents · metered · licensed · proposals | U2 · D10, D6, D12 · D1, D11, D10, U2, U3 · D2, D11, V2, V4 · D10, A1 |
| §13.1 routes `/relay` · `/settings` · pair ×3 · carrier ×2 · registry ×2 · `/nearby/hops` · `/update/origins` · `/origin/v1` · SSE · codes | D10 · D10 · D6 · D10, D11 · D8 · D2 · L4 · D7 · D10, D8, C1 · C1 |
| §13.2 shapes · pin 1 · 2 · 3 · 4 · 5 · 6 | C1 · C1, A1 · A1 · C1 · C1 · C1 · C1, D2, D6, D8, D10 |
| §13.3 | A1 |
| §14 hard rules 1–8 | A1/D7 · design · D2, D11 · D11 · D3 · M5, P1, D1, D10, A1 · D5 · M4, D11; V10 reviews |
| §15 G8 · G9 · G10 · G11 · G12 · G7 · CI | V2 (+D2, D8) · V3 (+M1, M2, M4) · V4 · V5 (+D5) · V6 (+L1–L4) · V1, V7 · V8 |
| §16 figures | the record table (every row owned; memory and per-pair wakeups added) |
| §17 | nothing built: M4 (no `Route::Forward`, no sweep), D10 (`presence.interval` refused), D8 (no query), M6/D11 (no `LinkKind`, in-process), K2 (map recorded only), K1 (`0x04` reserved only), L4 (nothing over LoRa) |
| §18 L5 · L6 · L2 · L1 · L3 · L4 · L0 | M7, M6, D2, M4, D9 · D4, D3, D8, K2, V1/V7, D2, D10, V2/V4 · K1 · L1–L4, D6, K2, K4 · K2, A1 · C1, U3, U2, U1, K2 · K3, P2, T0 |
| §19 seams 1–4 · items 1–11 · deviations · forks | M6, D2, M4, D9 · K2 (1, 2, 6, 8, 9, 10), K3 (10 MeshCore), M7 (3), L1–L4 (4), T0 (5), V7 (7), design (11) · V9 (D33) · header + rows that name them |
