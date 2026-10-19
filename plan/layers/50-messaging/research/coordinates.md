# Layer 5 — research coordinates

Resume coordinates for the Layer 5 (Messaging & Data) research sweep. Reports land in `layers/50-messaging/research/`.
Sessions die (quota walls, restarts); this file is what the next session needs.

## Run 1 — 5-lens sweep + critic + gap-fill (2026-10-14)

- Workflow run id: `wf_a4b7e5d6-d10`
- Script copy (resume needs a path under the repo): `plan/layers/50-messaging/runs/workflows/research.js`
- Harness script path: `~/.claude/projects/-home-kia-projects-111network-os/3a9939aa-635d-4885-a9ea-6584b070c677/workflows/scripts/layer-5-research-wf_a4b7e5d6-d10.js`
- Journal: `~/.claude/projects/-home-kia-projects-111network/3a9939aa-635d-4885-a9ea-6584b070c677/subagents/workflows/wf_a4b7e5d6-d10/journal.jsonl`
- Resume: `Workflow({scriptPath: "<script copy, absolute>", resumeFromRunId: "wf_a4b7e5d6-d10"})` — keep the script byte-identical.
- Models: scouts sonnet (effort high) · critic opus (xhigh) · gap-fill sonnet. Session on the Anthropic plan (roster map in `INDEX.md`).

| Lens | Report | Question |
|---|---|---|
| 1 fabric | `01-fabric.md` | what carries 111's sealed envelopes node-to-node with store-and-forward: thin Rust core vs BPv7 (dtn7-rs) vs LXMF-via-API vs adopted P2P cores |
| 2 bridges + budgets | `02-bridges-and-budgets.md` | Meshtastic / MeshCore / LXMF / Briar / Delta Chat as bridge targets; per-transport MTU + throughput table |
| 3 hosted panels | `03-hosted-panels.md` | how one dashboard hosts other projects' UIs (HA Ingress, Sandstorm, …) + the panel inventory |
| 4 envelope | `04-envelope.md` | message ids, fragmentation, ACK proofs, groups, inner signatures, ratchet space, encoding, time, classes |
| 5 data | `05-data-store-and-bulk.md` | encrypted store, capacity pruning, bulk payloads (KB sync, updates, patches), multi-device |
| critic | `06-critic.md` | completeness verdict, gaps, conflicts |
| gap-fill | `07-gap-N.md` | one per blocking gap (cap 2) |

Next step after the run: lead writes `layers/50-messaging/options.md` (options brief), owner decides → `DECISIONS.md` → spec draft
`os/50-messaging/README.md` → fable adversarial panel → `TASKS.md` → build.

## Run 2 — gap-3 scout (MeshMonitor vs web client + sidecar vs loopback Web Serial), 2026-10-14

- Agent tool (sonnet), name `scout-meshmonitor`; report `layers/50-messaging/research/07-gap-3-meshmonitor.md`.
- Critic gaps handled by the lead inside `layers/50-messaging/options.md`: G4 (online modality → not v1 fabric traffic; random link
  key + attestation rule), G5 (one tiered fragmentation design sized against the built `seal.rs`), G6–G9 noted.

## Status

All 7 forks decided (D19, 2026-10-14). Spec draft: `os/50-messaging/README.md`. Next: fable adversarial panel (`layers/50-messaging/review/`) → fixes → lock → `os/50-messaging/TASKS.md` → build.

## Run 3 — adversarial panel on the spec draft (2026-10-14)

- Workflow run id: `wf_c4054cfb-f80` · script copy: `plan/layers/50-messaging/runs/workflows/spec-review.js` · 3 fable judges (xhigh):
  `crypto` (§3–§12, §18, L2 amendments) · `panels` (§13–§15 proxy + Meshtastic) · `integration` (scope, seams, buildability).
- Reviews land in `plan/layers/50-messaging/review/{crypto,panels,integration}.md`; the lead's triage goes to `plan/layers/50-messaging/review/triage.md`.
- Resume: `Workflow({scriptPath: "<script copy, absolute>", resumeFromRunId: "wf_c4054cfb-f80"})`.

## Run 4 — build Wave 0 (2026-10-14)

- Workflow run id: `wf_3da6437a-9fa` · script copy: `layers/50-messaging/runs/workflows/wave-0.js` · 8 agents: I1, C2, C3, C4 (opus) · C1, C5, C6, C7 (sonnet).
- Resume: `Workflow({scriptPath: "<script copy, absolute>", resumeFromRunId: "wf_3da6437a-9fa"})`. Agents never commit; the lead
  reviews each task (run its check, read the diff) and commits one per task — `50-messaging/TASKS.md` is the board.

## Run 5 — build Wave 1 (2026-10-14)

- Wave 0 closed at `a14736b` (C2 + C3 landed together after their bounces; identity I1, contract C1, node111 plumbing C4, docs C5,
  knowledge C6, sidecar/assets C7 committed earlier). Known interim red: `pnpm test` in `40-interface/ui` (12 adapter/mock tests)
  until U1 lands; bootstrap `check.sh` stops at its UI stage for the same reason.
- Workflow run id: `wf_f537132f-cdb` · script copy: `layers/50-messaging/runs/workflows/wave-1.js` · 5 agents in parallel on disjoint files:
  F1 fabric runtime (opus, xhigh) · P1 hosted panels (opus) · M1 Meshtastic (opus) · U1 UI data layer + Messages + Contacts
  (sonnet) · A1 agent tools (opus). The F1 brief names the store API as landed after C3 (`receive_message` is the only
  production path that records a peer's `n`; `Store::offer` is test-only; `complete_partials` for the restart path) and the
  evicted-partial / SACK interaction to resolve in `frag.rs`/`outbox.rs`.
- Resume: `Workflow({scriptPath: "<script copy, absolute>", resumeFromRunId: "wf_f537132f-cdb"})`. Never message in-flight
  workflow agents (a SendMessage forks them). The lead reviews each task (check + diff), bounces via a fresh agent, commits one
  per task; then Wave 2 (F2, U2, B1), Wave 3 (V1–V4).
- **Crash + resume (19:03→19:30):** the session died ~62 min into the run; only A1 had returned (reviewed + committed
  `3c605fd`). Resumed with `resumeFromRunId` after adding a RESUMPTION NOTICE to the four unfinished briefs (partial work
  on disk; inventory, keep, finish). Lesson: the resume cache is a hash chain over the sequence of `agent()` calls — editing
  the first call's prompt re-ran every later one, including the finished A1 (started live twice, stopped both times,
  nothing on disk changed); moving A1 first changed its index, so it re-ran again. Fix that worked: delete the finished
  call from the script and take its result from `journal.jsonl`. Current run = 4 live agents F1 P1 M1 U1
  (task `wdhag90l2`). Put bounce-prone tasks LAST in a fan-out.
- **Wave 1 landed** (2026-10-14 evening): F1 `c8a093c` · P1 `79ca52d` · M1 `99f5e0a` · U1 `858077f` (board `f283c9a`). Every
  check green except the UI bundle budget (255.4 KB vs 200 KB; owner decision pending). Review run `wf_c857767c-bde`
  (script `layers/50-messaging/runs/workflows/wave-1-review.js`): F1/P1/M1 fable xhigh, U1 opus; findings only → lead bounces via fresh agents.
- **Wave 1 review returned** (F1 fix-then-lock 2/13 · P1 ready 0/6 · M1 fix-then-lock 1/9 · U1 fix-then-lock 1/16; triage
  `plan/layers/50-messaging/review/wave-1.md` with the lead's calls: ignore = client-side fold, bearer never mints, contract text →
  F2). Bounce run `wf_62ea271c-e73` (script `layers/50-messaging/runs/workflows/wave-1-bounce.js`): fresh agents P1 M1 U1 (opus/sonnet)
  + F1 (opus xhigh) on disjoint files; lead re-checks + commits per bounce. Wave 2 script drafted:
  `layers/50-messaging/runs/workflows/wave-2.js` (B1, U2, F2 — F2 carries the contract additions).
- **Wave 1 bounces landed** (F1 `db6159d` · P1 `0c6fb0d` · M1 `6697379` · U1 `cac21e8`; board `38a963b`); bootstrap
  `check.sh` ALL GREEN end to end (every cargo stage, shellcheck, notices, systemd, UI 945 tests + D21 budget).
  **Wave 2 = run `wf_97dba36d-e98`** (script `layers/50-messaging/runs/workflows/wave-2.js`): B1 gates (opus) · U2 UI (sonnet) · F2 daemon
  wiring (opus xhigh, last in the list). Resume: `Workflow({scriptPath, resumeFromRunId: "wf_97dba36d-e98"})`; delete
  finished calls before resuming (hash-chain cache). Then Wave 3: V1 netns smoke, V2 CDP panel lint, V3 gates + D22/INDEX,
  V4 fable integrated review.
- **Crash #2 (≈22:06 → 23:10):** session died ~1 h 45 min into Wave 2. U2 and B1 had returned → verified and committed
  (U2 `282bff8`: UI 1065 tests; B1 `09dc397`: gates, manifest entries, sign-off; agent-verified ALL GREEN on a clean tree).
  F2 was mid-way (≈5.5k lines on disk, fixing test compile errors) → resumed alone under the same run id with a
  RESUMPTION NOTICE (B1/U2 calls deleted from the script; task `wql91rj2e`). Carried to the Wave 2 review/bounce:
  B1's three `verify_layer5_tree` call sites in the Layer 1 updater (update_helper.rs, install/stage.rs, updater/fetch.rs),
  the OpenSSL native-code note in gen-notices.py, THIRD-PARTY-NOTICES regeneration after F2 (lead), release-checklist +
  presence-venv build step rows, U1 mock's enroll bundle → `111HUB:` shape, U2's open questions (Panel auth-mode field,
  restore-pending flag, Identify button on the device page).
- **Wave 2 landed:** F2 `7554926` (daemon wiring; 269 lib + 7 integration tests), notices fix `43c98bd` (OpenSSL named),
  board `99c5ede`. Full bootstrap `check.sh` run with F2 in (log `scratchpad/full-check-f2.log`): node111 stages, shellcheck,
  notices, build-script gate, systemd green; mesh111/SQLCipher/presence/UI stages were green on the B1 and U2 runs.
  **Review run `wf_938461a1-1cb`** (script `layers/50-messaging/runs/workflows/wave-2-review.js`): B1 + U2 opus, F2 fable xhigh. Carried
  into its triage: UI quota classes (msg/agent_op → 400), offerHub 409 in the contract, cancel route, updater
  `verify_layer5_tree` call sites, release-checklist + presence-venv rows, mock enroll bundle `111HUB:`.
- **Gate milestone (23:45):** `10-bootstrap/scripts/check.sh` **ALL GREEN** on `43c98bd` with Waves 0–2 in — node111 fmt/clippy/
  musl ×2/tests, shellcheck, notices (OpenSSL named), build-script gate, systemd, mesh111 fmt/clippy/tests/musl ×2, SQLCipher
  static link + run (x86_64) and link (aarch64, 8 static test binaries), presence sidecar (16 packages, 335 hashes), UI 1065
  tests, D21 eager 102.8 / lazy 176.4 KB. Wave 3 launched alongside the review: **run `wf_e1d4170c-90d`** (`layers/50-messaging/runs/workflows/wave-3.js`: V2 panel lint sonnet,
  V1 three-node smoke opus xhigh); resume with `resumeFromRunId`, delete finished calls first.
- **Wave 2 review returned** (F2 fix-then-lock 1/9 · U2 fix-then-lock 1/11 · B1 fix-then-lock 0/9; triage
  `plan/layers/50-messaging/review/wave-2.md` with the lead's calls: additive `cancelMessage` route + UI control, manifest pkg/bundle
  names parse-and-ignore (closure stays in verify), id111 gets the restore-pending persist guard). **Bounce run
  `wf_ba5631d2-319`** (script `layers/50-messaging/runs/workflows/wave-2-bounce.js`): B1 opus · U2 sonnet · F2 opus xhigh, parallel with
  Wave 3 `wf_e1d4170c-90d`. Owner questions parked in the triage: manifest evolution path before the first signed release;
  which command sets the Layer 2 restore floor; release-checklist + presence-venv rows.

## Run — Wave 2 bounce, attempt 2 (2026-10-19)

- Context: the 2026-10-15 crash cut off the F2 and B1 fix agents (partial work in WIP commit `6b4f0e5`, folder-renamed on
  2026-10-18). Tree at launch: `13ef253` (after the relicense D22 and the Layer 6 kickoff D23). `10-bootstrap/scripts/check.sh`
  red at its first stage (cargo fmt on four F2 files); the two `restore_pending` tests red; the hermetic cross-build UI gate red.
- Script: `plan/layers/50-messaging/runs/workflows/wave-2-bounce-2.js` · run id `wf_a95026da-e58` · harness transcript dir
  `~/.claude/projects/-home-kia-projects-111network-os/af41ab70-058e-4e3c-bc5e-eec7688f5c7a/subagents/workflows/wf_a95026da-e58/`.
- Shape: phase A = U2m (sonnet; the two mock items `mock/index.ts`) ‖ F2 (opus xhigh, resumed with a RESUMPTION NOTICE);
  phase B = B1 (opus, resumed) after F2 returns, so at most two cargo builds overlap (12 GB box; `CARGO_BUILD_JOBS=6`, private
  target dirs under the session scratchpad). Journals are the repo copies under `runs/journals/`.
- Runs alongside the Layer 6 research sweep `wf_7407f773-81c` (read-only). Next: lead re-checks + commits per bounce, then V1/V2
  verification, the P1 guard fix + smoke.sh `jq -e` fix, V3 gates + lock entry (the lock will be D24; D22 went to the relicense,
  D23 to the Layer 6 kickoff), V4 fable review.
- **Landed (2026-10-19):** U2m `596da10` · F2 `913bf19` · B1 `0e0b1d3`. Lead gate `10-bootstrap/scripts/check.sh` ALL GREEN on the bounced
  tree (792 cargo tests, UI 1153, G2 on both targets, hermetic gate tests 12 + 6 + 11 cases). F2's owner question (m8): no Layer 2 command sets
  a learned restore floor; a node whose counters file vanishes while running stays restore-pending (held safely) until the file is put back or a
  restart enters quarantine + words-only restore (asked with the lock questions). F2 residual: a hub whose only enrolled contact is a custody-opted
  member shows it as home hub after a lost settings file (mesh111 follow-up: store-side hub/member marker).
- **Wave 3 fix wave** `plan/layers/50-messaging/runs/workflows/wave-3-fix.js`: phase A = P1g guard Fetch-Metadata fallback (opus) ‖ DOCS
  (release-checklist rows, Overview copy; sonnet); phase B = VENV presence venv build (opus) ‖ V1v smoke verification + `jq -e` audit (opus xhigh).
  Run id `wf_8f3fabca-305` (launched 2026-10-19 02:45 CEST from `c84d834`). Then the lead: panel-lint.sh, every gate, D24 lock entry, INDEX, tag; then V4 fable review.
- **Fix wave landed (2026-10-19):** DOCS `353ec88` · smoke `40cd9ee` · tapes `8a08b2c` · VENV part 1 `51d7ad9` · P1g `29b4b5b`
  (lead panel-lint GREEN 144/0). **Polish wave** `plan/layers/50-messaging/runs/workflows/wave-3-polish.js`: A = VENVW (daemon-start
  rebuild + service-row reason + fresh-Debian python3-pip gap) ‖ F2P (enrol-refusal dial leak, `FabricStatus.counters`, `hub_not_yours`
  wording, `layers` reports 5) ‖ UIP (stale comments, mock layers) ‖ HB (presence reader nonce-0 heartbeat every 5 min: the board's API
  session lapses after 15 idle minutes); B = MANP (gen-manifest refuses an incomplete payload listing). Run id `wf_0b7a0a07-28c`
  (launched 2026-10-19 03:30 CEST from `f025c98`).
- **Owner decisions for the lock (2026-10-19, asked with the Layer 6 forks):** the panel-lint residual #2 is **accepted** and stated in
  spec §13 "Guards" (the P1g agent's proposed sentence); the restore floor is **accepted as is** (a words-only restore keeps announcing from
  the provisional floor; the hub-learned floor is a Layer 7 item; documented in D24). Manifest evolution path: the lead's reversible call
  stands (unknown names parse and are ignored; a reader-version field when the first incompatible change comes). Identify button: Layer 6.
- **Polish wave interrupted** (session ended mid-run, 2026-10-19 ~04:00): finished agents are read from the run journal; unfinished ones are
  re-run as fresh Agent-tool subagents with a RESUMPTION NOTICE over their partial work (ultracode is off: no Workflow resume).
- **Polish wave landed (2026-10-19 evening, one commit per task):** HB `553076d` · UIP `6278590` · VENVW `1a8488e` · F2P `ba9cf89` · MANP `796a92d`;
  §13 residual sentence `04a7dd5`; journals exported `5a88122`. Lead gates on that tree: `10-bootstrap/scripts/check.sh` ALL GREEN
  (node111 492 · mesh111 163 host + 163 in the musl-x86_64 G2 run · UI 1192 · notices current), smoke 118, panel-lint 144, identity
  185 + 44 + 7 + harness OK, intelligence green, knowledge 0 errors, presence sidecar 90. Three-node smoke was RED once for a wrong
  expectation (C LIVE at step 6 had been satisfied by the leaked hub dial F2P closed; §8.1: attribution needs a sealed packet) — harness
  corrected `b0f8c52`, 156 checks GREEN; one flake seen in three runs (hub-crash delivery wait, `fab_diag` notes added). **V4** fable judge
  launched as an Agent-tool subagent (prompt = `runs/workflows/wave-4-review.js`, adapted for today's tree) → `review/integrated.md`;
  then D24, board summary, INDEX row 5 ✅, tag `checkpoint-layer-5`.
