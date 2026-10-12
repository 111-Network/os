# Layer 0 — Build tasks

Execution per `os/plan/EXECUTION.md`. Spec: `os/knowledge/README.md` (locked, D7).
Subagents write files only; the reviewer (main session) runs checks, reads diffs, bounces failures with specific fixes, and commits each verified task. No attribution lines in commits.

Legend: ⬜ todo · 🔧 in flight · 🔁 bounced · ✅ verified + committed

| # | Task | Files owned | Acceptance criteria | Verifiable check | Status |
|---|---|---|---|---|---|
| T0 | Plan scaffolding + locked spec | `plan/*`, `knowledge/README.md`, `knowledge/TASKS.md` | committed clean | git log | 🔧 |
| T1 | Meta docs | `_meta/schema.md`, `_meta/predicates.md`, `_meta/vocabularies.md`, `_meta/index-header.md`, `_meta/planned.md` | schema.md §1 = rule→severity table; full 12-field definitions + per-category matrix + YAML subset rules; faithful to spec, nothing invented | reviewer reads; build.py validates against it | ⬜ |
| T2 | Validator + generator | `_meta/build.py`, `_meta/fixture/**` | spec §Tooling duties; `--check` mode; deterministic regeneration; golden-fixture self-test green | `python3 _meta/build.py --selftest` exits 0; `--check` behaves | ⬜ |
| T3 | Agent contract files | `AGENTS.md` (os root), `knowledge/AGENTS.md` | knowledge/AGENTS.md ≤400 tokens (chars/4), exact ritual commands, tally override line, untrusted-data line, degraded path, pointers only | reviewer reads; token budget measured | ⬜ |
| T4 | Entry: Meshtastic | `projects/meshtastic.md` | schema-valid; 6 fixed sections; `### Alternatives considered` with desk-research notes; `status: candidate`; facts per research sweep; body ≤1500 tok | `build.py --check` clean; reviewer reads | ⬜ |
| T5 | Entry: Reticulum | `projects/reticulum.md` | same shape | same | ⬜ |
| T6 | Entry: Nostr | `protocols/nostr.md` | same shape; protocol category rules (upstream = spec home) | same | ⬜ |
| T7 | Entry: llama.cpp | `projects/llama-cpp.md` | same shape | same | ⬜ |
| T8 | Entry: Qwen3 4B | `models/qwen3-4b.md` | same shape; model rules (SPDX license required) | same | ⬜ |
| T9 | Entry: OpenCode | `projects/opencode.md` | same shape | same | ⬜ |
| T10 | Entry: Automerge | `projects/automerge.md` | same shape | same | ⬜ |
| T11 | Entry: Heltec WiFi LoRa 32 V3 | `devices/heltec-wifi-lora-32-v3.md` | same shape; device category rules | same | ⬜ |
| T12 | Entry: Install Meshtastic on Heltec V3 | `recipes/install-meshtastic-on-heltec-wifi-lora-32-v3.md` | naming rule (last `-on-` split resolves); carries `depends_on` + `runs_on` relations; ≤2500 tok | same + relation resolution | ⬜ |
| T13 | Generation + hook wiring | `hooks/pre-commit`, all `llms.txt` files, `_meta/planned.md` updates | root + 6 category indexes generated; regeneration is no-op diff; hook committed, activation documented | run generator twice, diff empty; `build.py --check` green | ⬜ |

## Summary (filled at completion)

_built: — · deferred: —_
