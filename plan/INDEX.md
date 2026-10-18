# os/plan — Build Index

Working area for speccing and building the 111network mesh OS, layer by layer.
The main `os/README.md` is the vision map; each layer gets its own proper README/spec.
Deep architecture content currently in `os/README.md` will be **moved out into the layer
specs** once their homes exist — the main README then becomes user-facing and general.

Folder layout, file naming and where logs/journals/handoffs go: [`README.md`](README.md). Each layer's research,
options brief, reviews and run logs live in its own folder under [`layers/`](layers/) (see the Plan column below);
things spanning layers live in [`cross-layer/`](cross-layer/). Layer folders are named `NN-<slug>` (`NN` = layer number × 10)
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
| 0 | Knowledge | ✅ built — spec locked (D7), all 13 tasks done: 9 candidate seed entries, generator + `--check` green (0 errors), hook committed (activation: `git config core.hooksPath hooks`) | `os/00-knowledge/README.md` ✅ locked | [`layers/00-knowledge/`](layers/00-knowledge/) |
| 1 | Bootstrap | ✅ built — spec v2.2 (D9 + D12 + D14d identity-seam amendment), all 17 tasks done: ~90 cargo tests + 21 shim cases + namespace e2e green; `verify/local-smoke.sh` ALL LOCAL GATES GREEN; external HW gates (V1–V3, V8, V11) in runbooks before first signed release | `os/10-bootstrap/README.md` ✅ locked · `os/10-bootstrap/TASKS.md` ✅ | [`layers/10-bootstrap/`](layers/10-bootstrap/) |
| 2 | Identity | ✅ built — spec v2.1 locked (D13 + D14 + D17 torsion-free amendment), all 15 tasks done: `id111` crate (167 unit + 44 conformance tests), 15 frozen vector files incl. 4 mixed-order reject rows, Monocypher 4.0.3 C harness 442/442 PASS (own `[ℓ]P` leg) + smoke log (F3 CLOSED), L1 wired (node111 314 tests + local-smoke GREEN); Cat-1 C core + relay commissioning bytes deferred to the firmware-owning layer | `os/20-identity/README.md` ✅ locked · `os/20-identity/TASKS.md` ✅ | [`layers/20-identity/`](layers/20-identity/) |
| 3 | Local Intelligence | ✅ built — D10; `agent111` crate (`111-agent`): T0–T8 verified + committed, 93 tests, `check.sh` green; wired into L1 (`111d` spawns `111-agent`/`llama-server`, additive `/api/status.intelligence`), L2 (node id stamps proposals; no key material in agent), L0 (grounds in the release's verified `share/kb/`); real-binary e2e on a staged root passed. Open: adversarial pass on spec (queued), pre-hub gates G1–G4 (real llama-server, opencode offline SDK prefetch, LAN offload, model in manifest), L4 relay = L4 build item | `os/30-intelligence/README.md` (as-built appendix) · `os/30-intelligence/TASKS.md` ✅ | — |
| 4 | Interface & Dashboard | ✅ built — spec v2 locked (D11+D15), as-built lock D16: UI (588 vitest, 147.6 KB gz) + daemon side in `node111` (314 cargo tests) + install→dashboard on Linux and phone; I3 = 3 fable judges, all findings fixed; `check.sh` ALL GREEN · `40-interface/verify/smoke.sh` 118 checks · `local-smoke.sh` green. Open: on-device phone gate V1, real-Android pair-link check | `os/40-interface/README.md` ✅ locked · `os/40-interface/TASKS.md` ✅ | [`layers/40-interface/`](layers/40-interface/) |
| 5 | Messaging & Data | 🔨 building — spec v1.2 locked (D20, 2026-10-14) after a 3-judge fable panel + re-check; `mesh111` scaffold (T0) green on host + musl ×2; `os/50-messaging/TASKS.md` Wave 0 (8 tasks) ✅ landed at a14736b (identity amendments, contract, codec + store with 96 tests, node111 plumbing, docs, KB, sidecar/assets); Wave 1 ✅ landed, fable/opus-reviewed and bounced (A1 agent tools · F1 fabric · P1 panels · M1 Meshtastic · U1 UI; bootstrap `check.sh` ALL GREEN incl. the D21 budget); Wave 2 ✅ landed (B1 gates · U2 Panel tab/Nearby/Settings · F2 daemon wiring: the fabric runs inside 111d behind the §15 routes); review + bounce next, then Wave 3 verification (netns smoke, panel lint, fable integrated review) | `os/50-messaging/README.md` ✅ locked · `os/50-messaging/TASKS.md` | [`layers/50-messaging/`](layers/50-messaging/) |
| 6 | Transport & Bridge | ⬜ pending — opener: device-connection core — brief `cross-layer/devices.md` with the 12 owner decisions (D17); parked ideas in `cross-layer/parked.md` | — | [`cross-layer/devices.md`](cross-layer/devices.md) · [`cross-layer/parked.md`](cross-layer/parked.md) · reserved folder `60-transport/`, created at kickoff |
| 7 | Relay & Presence | ⬜ pending | — | — · reserved folder `70-relay/` |
| 8 | Storage & Shared State | ⬜ pending | — | — · reserved folder `80-storage/` |
| 9 | Self-Healing & Evolution | ⬜ pending | — | — · reserved folder `90-evolution/` |

Statuses: ⬜ pending · 🔬 researching · 🧭 deciding · ✍️ speccing · 🔨 building · ✅ done
