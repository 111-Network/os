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
| 1 | Bootstrap | 🔬 researching — scope locked w/ user 2026-10-12 (Linux + phone-via-USB-C; running minimal node; minimal cache + standard offer) | — |
| 2 | Identity | 🔬 researching — scope locked w/ user 2026-10-12 (node identity only, no human/contact/trust model; generate+store+use+backup/recovery, rotation/revocation deferred; crypto picked on merit — NOT Nostr-locked, Nostr itself not final; agentic-native lens: agents are first-class key users; must run on all 6 device categories incl. ESP32-class) | — |
| 3 | Local Intelligence | ⬜ pending | — |
| 4 | Interface & Dashboard | ⬜ pending | — |
| 5 | Messaging & Data | ⬜ pending | — |
| 6 | Transport & Bridge | ⬜ pending | — |
| 7 | Relay & Presence | ⬜ pending | — |
| 8 | Storage & Shared State | ⬜ pending | — |
| 9 | Self-Healing & Evolution | ⬜ pending | — |

Statuses: ⬜ pending · 🔬 researching · 🧭 deciding · ✍️ speccing · 🔨 building · ✅ done
