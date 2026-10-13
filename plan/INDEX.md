# os/plan — Build Index

Working area for speccing and building the 111network mesh OS, layer by layer.
The main `os/README.md` is the vision map; each layer gets its own proper README/spec.
Deep architecture content currently in `os/README.md` will be **moved out into the layer
specs** once their homes exist — the main README then becomes user-facing and general.

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

| # | Layer | Status | Spec |
|---|---|---|---|
| 0 | Knowledge | ✅ built — spec locked (D7), all 13 tasks done: 9 candidate seed entries, generator + `--check` green (0 errors), hook committed (activation: `git config core.hooksPath hooks`) | `os/knowledge/README.md` ✅ locked |
| 1 | Bootstrap | ✅ built — spec v2.2 (D9 + D12 + D14d identity-seam amendment), all 17 tasks done: ~90 cargo tests + 21 shim cases + namespace e2e green; `verify/local-smoke.sh` ALL LOCAL GATES GREEN; external HW gates (V1–V3, V8, V11) in runbooks before first signed release | `os/bootstrap/README.md` ✅ locked · `os/bootstrap/TASKS.md` ✅ |
| 2 | Identity | ✅ built — spec v2 locked (D13 + D14), all 15 tasks done: `id111` crate (161 unit + 44 conformance tests), 14 frozen vector files, Monocypher 4.0.3 C harness 411/411 PASS + smoke log, L1 wired (node111 133 tests + local-smoke GREEN); one open user call: §3 mixed-order residual (DECISIONS pending); Cat-1 C core + relay commissioning bytes deferred to the firmware-owning layer | `os/identity/README.md` ✅ locked · `os/identity/TASKS.md` ✅ |
| 3 | Local Intelligence | ✅ built — D10; `agent111` crate (`111-agent`): T0–T8 verified + committed, 88 tests, `check.sh` green; wired into L1 (`111d` spawns `111-agent`/`llama-server`, additive `/api/status.intelligence`), L2 (node id stamps proposals; no key material in agent), L0 (grounds in the release's verified `share/kb/`); real-binary e2e on a staged root passed. Open: adversarial pass on spec (queued), pre-hub gates G1–G4 (real llama-server, opencode offline SDK prefetch, LAN offload, model in manifest), L4 relay = L4 build item | `os/intelligence/README.md` (as-built appendix) · `os/intelligence/TASKS.md` ✅ |
| 4 | Interface & Dashboard | 🔨 building — spec v2 locked (D11 + D15: IA redirect, auth now, hostnames, status dots), contract `interface/api/openapi.yaml`; Wave 1 (111d core + UI scaffold/data layer) in flight per `interface/TASKS.md` | `os/interface/README.md` ✅ locked · `os/interface/TASKS.md` |
| 5 | Messaging & Data | ⬜ pending | — |
| 6 | Transport & Bridge | ⬜ pending | — |
| 7 | Relay & Presence | ⬜ pending | — |
| 8 | Storage & Shared State | ⬜ pending | — |
| 9 | Self-Healing & Evolution | ⬜ pending | — |

Statuses: ⬜ pending · 🔬 researching · 🧭 deciding · ✍️ speccing · 🔨 building · ✅ done
