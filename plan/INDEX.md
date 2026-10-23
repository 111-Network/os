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
| 6 | Transport & Bridge | ✅ built — spec v1.0 locked (D27) + as-built lock **D29** (2026-10-23): the bridge core with stub, Meshtastic (our own client on meshmud's downlink), LXMF (own helper on piped stdio) and Delta Chat/Madmail adapters as zero-weight on-demand components, the transport spine (rnsd config + secrets, posture, phone relay, QR join) and the device core (uevent hotplug, consented identify, the four-action applier) in `node111`; `mesh111` store V3 + foreign fabric calls; 7 agent tools; the UI (networks rail, foreign threads, fan-out, edges and consent, `/settings/l6`); gates G1 130 · G3 147 · G4 128 · G5 98 · G7 GREEN (the Pi floor measured: 111d idle 44.7 MB, nothing resident added), the Layer 1 aggregate stages, Layers 2–3, smoke 118 / three-node 159 / panel-lint 144; two defects found by the gates and fixed before the lock (a client rnsd after a restart, a helper forgetting its peers); V5 = one fable judge on the running system → fix-then-lock → fixed (the inline applier run, resend supersession, the Backbone port, empty bodies); CI/CD + GitHub hygiene D28. Open: the owner questions and follow-ups in the board's Summary, hardware/bench gates, G2 (musl) on the owner's machine | `os/6-transport/README.md` ✅ locked · `os/6-transport/TASKS.md` ✅ | [`layers/6-transport/`](layers/6-transport/) |
| 7 | Relay & Presence | ⬜ pending | — | — · reserved folder `7-relay/` |
| 8 | Storage & Shared State | ⬜ pending | — | — · reserved folder `8-storage/` |
| 9 | Self-Healing & Evolution | ⬜ pending | — | — · reserved folder `9-evolution/` |

Statuses: ⬜ pending · 🔬 researching · 🧭 deciding · ✍️ speccing · 🔨 building · ✅ done
