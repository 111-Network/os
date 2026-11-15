# os/plan — Build Index

Working area for speccing and building the 111network mesh OS, layer by layer.
The main `os/README.md` is the vision map; each layer gets its own proper README/spec.
Deep architecture content currently in `os/README.md` will be **moved out into the layer
specs** once their homes exist — the main README then becomes user-facing and general.

Folder layout, file naming and where logs/journals/handoffs go: [`README.md`](README.md). Each layer's research,
options brief, reviews and run logs live in its own folder under [`layers/`](layers/) (see the Plan column below);
things spanning layers live in [`cross-layer/`](cross-layer/). Layer folders are named `N-<slug>` (`N` = the layer number)
at the repo root and under `layers/`, with the same name in both places.

## Process (per layer)

1. **Research** — workflow fan-out over candidate projects/standards (k3-256k fleet)
2. **Options brief** — 2–4 options with recommendation, presented to user
3. **Decision** — recorded in `DECISIONS.md` with rejected alternatives
4. **Adversarial pass** — fable/opus judge panel attacks the spec before locking
5. **Build** — per `EXECUTION.md`: small tasks with criteria + checks → parallel subagents on disjoint file sets → self-review (run checks, read diff, bounce failures back with fixes) → commit per verified task → built/deferred summary

## Model roster

| Job | Model |
|---|---|
| Main session, orchestration, decision briefs | k3[1m] |
| Research fan-out, parallel readers | k3-256k |
| Bulk background / scaffolding / mechanical generation | kimi-for-coding (K2.8) |
| Adversarial design review, judge panels | fable (opus alternate) |
| Spec prose drafting | sonnet |
| Tiny mechanical transforms | haiku |

Harness: Claude Code (this session) + Workflow orchestration.
Provider note: the roster names above are the Kimi plan's. A session running on the Anthropic plan maps them: k3[1m]/k3-256k/kimi-for-coding → **sonnet** for research + build subtasks (reviewer pass is the quality gate), fable/opus → **fable** for adversarial review, haiku unchanged.

## Layer status

| # | Layer | Status | Spec | Plan |
|---|---|---|---|---|
| 0 | Knowledge | ✅ built — spec locked (D7), all 13 tasks done: 9 candidate seed entries, generator + `--check` green (0 errors), hook committed (activation: `git config core.hooksPath hooks`) | `os/0-knowledge/README.md` ✅ locked | [`layers/0-knowledge/`](layers/0-knowledge/) |
| 1 | Bootstrap | ✅ built — spec v2.2 (D9 + D12 + D14d identity-seam amendment), all 17 tasks done: ~90 cargo tests + 21 shim cases + namespace e2e green; `verify/local-smoke.sh` ALL LOCAL GATES GREEN; external HW gates (V1–V3, V8, V11) in runbooks before first signed release | `os/1-bootstrap/README.md` ✅ locked · `os/1-bootstrap/TASKS.md` ✅ | [`layers/1-bootstrap/`](layers/1-bootstrap/) |
| 2 | Identity | ✅ built — spec v2.1 locked (D13 + D14 + D17 torsion-free amendment), all 15 tasks done: `id111` crate (167 unit + 44 conformance tests), 15 frozen vector files incl. 4 mixed-order reject rows, Monocypher 4.0.3 C harness 442/442 PASS (own `[ℓ]P` leg) + smoke log (F3 CLOSED), L1 wired (node111 314 tests + local-smoke GREEN); Cat-1 C core + relay commissioning bytes deferred to the firmware-owning layer | `os/2-identity/README.md` ✅ locked · `os/2-identity/TASKS.md` ✅ | [`layers/2-identity/`](layers/2-identity/) |
| 3 | Local Intelligence | ✅ built — D10; `agent111` crate (`111-agent`): T0–T8 verified + committed, 93 tests, `check.sh` green; wired into L1 (`111d` spawns `111-agent`/`llama-server`, additive `/api/status.intelligence`), L2 (node id stamps proposals; no key material in agent), L0 (grounds in the release's verified `share/kb/`); real-binary e2e on a staged root passed. Open: adversarial pass on spec (queued), pre-hub gates G1–G4 (real llama-server, opencode offline SDK prefetch, LAN offload, model in manifest), L4 relay = L4 build item | `os/3-intelligence/README.md` (as-built appendix) · `os/3-intelligence/TASKS.md` ✅ | — |
| 4 | Interface & Dashboard | ✅ built — spec v2 locked (D11+D15), as-built lock D16: UI (588 vitest, 147.6 KB gz) + daemon side in `node111` (314 cargo tests) + install→dashboard on Linux and phone; I3 = 3 fable judges, all findings fixed; `check.sh` ALL GREEN · `4-interface/verify/smoke.sh` 118 checks · `local-smoke.sh` green. Open: on-device phone gate V1, real-Android pair-link check | `os/4-interface/README.md` ✅ locked · `os/4-interface/TASKS.md` ✅ | [`layers/4-interface/`](layers/4-interface/) |
| 5 | Messaging & Data | ✅ built — spec v1.2 locked (D20) + as-built lock **D24** (2026-10-19): `mesh111` store/envelope/fabric/`tcp-lan` (163 tests + musl G2 run), the fabric inside `111d` behind additive `/api/v1` + SSE, hosted panels with guards and offline lint, Meshtastic service (meshmud, shim, GPL presence reader in a hash-locked venv), agent tools with the loopback bearer, Messages/Contacts/Nearby/Panel/Settings UI (1201 vitest); V4 = one fable judge on the running system → fix-then-lock → fixed (Layer 2 A12 unclean-reboot verdict, honest hub reachability + §9 empty-fetch answer, children die with 111d, keep-alive timeout, manifest hashes); gates: `check.sh` ALL GREEN (834 cargo), smoke 118, three-node ×3 green, panel-lint 144; tag `checkpoint-layer-5`. Open: external hardware/CI gates (D24) | `os/5-messaging/README.md` ✅ locked · `os/5-messaging/TASKS.md` ✅ | [`layers/5-messaging/`](layers/5-messaging/) |
| 6 | Transport & Bridge | ✅ built — spec v1.0 locked (D27) + as-built lock **D29** (2026-10-23): the bridge core with stub, Meshtastic (our own client on meshmud's downlink), LXMF (own helper on piped stdio) and Delta Chat/Madmail adapters as zero-weight on-demand components, the transport spine (rnsd config + secrets, posture, phone relay, QR join) and the device core (uevent hotplug, consented identify, the four-action applier) in `node111`; `mesh111` store V3 + foreign fabric calls; 7 agent tools; the UI (networks rail, foreign threads, fan-out, edges and consent, `/settings/l6`); gates G1 132 · G3 147 · G4 129 · G5 99 · G7 GREEN (after fix wave B) (the Pi floor measured: 111d idle 44.7 MB debug, 19.7 MB release; nothing resident added), the Layer 1 aggregate stages, Layers 2–3, smoke 118 / three-node 159 / panel-lint 144; two defects found by the gates and fixed before the lock (a client rnsd after a restart, a helper forgetting its peers); V5 = one fable judge on the running system → fix-then-lock → fixed (the inline applier run, resend supersession, the Backbone port, empty bodies); CI/CD + GitHub hygiene D28; post-lock fix wave C (2026-11-01: five Summary follow-ups closed, the CI runner differences fixed, the release build measured). Open: the owner questions and follow-ups in the board's Summary, hardware/bench gates, G2 (musl) on the owner's machine | `os/6-transport/README.md` ✅ locked · `os/6-transport/TASKS.md` ✅ | [`layers/6-transport/`](layers/6-transport/) |
| 7 | Relay & Presence | 🔨 building — decided **D30** (2026-11-01), amendments **D31**, spec v1.0 locked **D32** (2026-11-02; `layers/7-relay/review/`); build board `7-relay/TASKS.md` per `EXECUTION.md`. **Wave 0 ✅** with every mesh111 fix row (the fabric store is at schema v5). **Wave 1** (2026-11-11 to 13), each row with the lead's re-run and an independent review: K1, K2, K4, U1–U3; D4 + D4b + D4c, D9, D3 + D3b, D7, D5 + D5b + D5c, D6 + D6b (pairing), A1 + A1b (the agent tools). **D2 (presence) is committed unfinished and unreviewed** (its author was cut off). Owed: D2's finish and review, D6c, D8, D10 (stage C), Waves 2 and 3 (their workflow scripts are not written), the lock amendment D33, the board Summary. State and next steps: `HANDOFF-2026-11-15.md` | [`7-relay/README.md`](../7-relay/README.md) (v1.0, locked) · `7-relay/TASKS.md` | [`layers/7-relay/`](layers/7-relay/) |
| 8 | Storage & Shared State | 🔒 spec locked — **decided D34** (2026-11-11), spec v1.0 locked **D38** (2026-11-13): `8-storage/README.md`. v0.1 → three judges (`layers/8-storage/review/{store,datasets,integration}.md`: 92 findings) → the lead's triage (`review/triage.md`) → v0.2 (`review/fix-map.md`) → a re-check (`review/recheck.md`: amend-then-lock, R1–R21) → v0.3 → a second re-check (`review/recheck-2.md`: lock) → the lock edit. **Nothing is built.** The board `8-storage/TASKS.md` is written (2026-11-15, `layers/8-storage/runs/workflows/board.js`: an author, a critic with 13 gaps and 10 other findings, all applied; T0 plus 101 rows in ten waves); next: T0 (the lead's scaffold), then the waves per `EXECUTION.md`. Workflows: `layers/8-storage/runs/workflows/{spec,fix,amend}.js` |
| 9 | Self-Healing & Evolution | ✍️ speccing — **decided D35** (2026-11-11: the owner took every pick and default of the brief). `9-evolution/README.md` is a draft, **not locked**: v0.1 → three judges (`layers/9-evolution/review/{safety,observe,integration}.md`: each amend-then-lock, 8 blockers) → the lead's triage (`review/triage.md`: 8 merged groups, 22 forks, and the rulings on the 21 items of `review/triage-check.md`) → v0.2 **in progress**: the safety and observe reports are applied (`review/fix-map.md`), the integration author was cut off. Owed: the fix pass from `integration` (`layers/8-storage/runs/workflows/fix.js`), the re-check, the amend (`amend.js`), the lock entry D39, the board `9-evolution/TASKS.md` and the build. No code before the lock. Before that: research `research/00`–`07-gap-3`, options brief `layers/9-evolution/options.md` (forks F1–F14; `review/options-check.md`: ready); F3 is one contract with Layer 8's F2 | [`9-evolution/README.md`](../9-evolution/README.md) (draft v0.2 in progress) | [`layers/9-evolution/`](layers/9-evolution/) |

Statuses: ⬜ pending · 🔬 researching · 🧭 deciding · ✍️ speccing · 🔨 building · ✅ done

## Cross-layer

| Item | Status | Where |
|---|---|---|
| Test system (D36, D37): a seam map, a check catalog and one runner that names the layer seam a failure belongs to | 🔨 building — X1–X4 with their fix rows (X1b, X1c, X2b, X2c, X3b, X3c, X4b, X4c) and the first real run (R1, R1b) landed: `verify/run.py` with `--lint`, `--list`, `--matrix`, `--explain <id>`, `--changed` on the real map (104 seams, 130 checks, 30 fakes; unit suite 343 tests). **The fast tier runs green for real** (`--tier fast`: 311 PASS, 0 FAIL, 15 SKIPPED); its first run found a leaked process in the Termux dry-run harness, since fixed. Open: R2 (the slow tier's first real run), the namespace tier, X5–X9, N1–N11 | `verify/` · [`cross-layer/test-system/`](cross-layer/test-system/) (`options.md`, `matrix.md`, `TASKS.md`) |
