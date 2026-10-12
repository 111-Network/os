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

## Layer status

| # | Layer | Status | Spec |
|---|---|---|---|
| 0 | Knowledge | ✅ built — spec locked (D7), all 13 tasks done: 9 candidate seed entries, generator + `--check` green (0 errors), hook committed (activation: `git config core.hooksPath hooks`) | `os/knowledge/README.md` ✅ locked |
| 1 | Bootstrap | ✍️ spec v2.1 locked (D9 + D12 amendments) — ready to build per EXECUTION.md on your go | `os/bootstrap/README.md` ✅ locked |
| 2 | Identity | ✍️ speccing — decisions locked (D13): Ed25519+X25519 dual keyset, raw-pubkey NodeId, Reticulum-shaped wire contract (clean-room), BIP-39+SLIP-0010 backup, agentic hook, attestation bridge; spec draft + adversarial pass in flight | — |
| 3 | Local Intelligence | ✍️ speccing — decisions locked (D10): hybrid harness (thin `111-agent` loop + opencode on hubs), llama-server child of 111d, qwen3-4b standard tier; spec v1 draft minimal, adversarial pass queued | `os/intelligence/README.md` (draft) |
| 4 | Interface & Dashboard | ✍️ speccing — decisions locked (D11): React+Vite→rust-embed, single-tree simple/advanced mode, REST+one-SSE `/api/v1` contract w/ interim `openapi.yaml`; scope = full UI on mock adapters, wiring deferred; spec draft next, adversarial pass queued | `os/interface/README.md` (draft) |
| 5 | Messaging & Data | ⬜ pending | — |
| 6 | Transport & Bridge | ⬜ pending | — |
| 7 | Relay & Presence | ⬜ pending | — |
| 8 | Storage & Shared State | ⬜ pending | — |
| 9 | Self-Healing & Evolution | ⬜ pending | — |

Statuses: ⬜ pending · 🔬 researching · 🧭 deciding · ✍️ speccing · 🔨 building · ✅ done
