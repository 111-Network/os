# Layer 0 — Build tasks

Execution per `os/plan/EXECUTION.md`. Spec: `os/knowledge/README.md` (locked, D7).
Subagents write files only; the reviewer (main session) runs checks, reads diffs, bounces failures with specific fixes, and commits each verified task. No attribution lines in commits.

Legend: ⬜ todo · 🔧 in flight · 🔁 bounced · ✅ verified + committed

| # | Task | Files owned | Acceptance criteria | Verifiable check | Status |
|---|---|---|---|---|---|
| T0 | Plan scaffolding + locked spec | `plan/*`, `knowledge/README.md`, `knowledge/TASKS.md` | committed clean | git log | ✅ |
| T1 | Meta docs | `_meta/schema.md`, `_meta/predicates.md`, `_meta/vocabularies.md`, `_meta/index-header.md`, `_meta/planned.md` | schema.md §1 = rule→severity table; full 12-field definitions + per-category matrix + YAML subset rules; faithful to spec, nothing invented | reviewer reads; build.py validates against it | ✅ (subagent + reviewer fixes: block lists allowed, tally wording) |
| T2 | Validator + generator | `_meta/build.py`, `_meta/fixture/**` | spec §Tooling duties; `--check` mode; deterministic regeneration; golden-fixture self-test green | `python3 _meta/build.py --selftest` exits 0; `--check` behaves | ✅ (subagent partial → quota 403; completed by reviewer; selftest + real run green) |
| T3 | Agent contract files | `AGENTS.md` (os root), `knowledge/AGENTS.md` | knowledge/AGENTS.md ≤400 tokens (chars/4), exact ritual commands, tally override line, untrusted-data line, degraded path, pointers only | reviewer reads; token budget measured | ✅ (963 chars ≈ 241 tok) |
| T4 | Entry: Meshtastic | `projects/meshtastic.md` | schema-valid; 6 fixed sections; `### Alternatives considered` with desk-research notes; `status: candidate`; facts per research sweep; body ≤1500 tok | `build.py --check` clean; reviewer reads | ✅ (0 errors; relations: runs_on heltec, alternative_to reticulum) |
| T5 | Entry: Reticulum | `projects/reticulum.md` | same shape | same | ✅ (custom license recorded as LicenseRef-Reticulum, flagged in entry; 0 errors) |
| T6 | Entry: Nostr | `protocols/nostr.md` | same shape; protocol category rules (upstream = spec home) | same | ✅ (license field omitted: NIPs are README-declared public domain, no SPDX file; noted in Details; 0 errors) |
| T7 | Entry: llama.cpp | `projects/llama-cpp.md` | same shape | same | ✅ (0 errors; uses qwen3-4b, used_in opencode — inverse edges on both targets) |
| T8 | Entry: Qwen3 4B | `models/qwen3-4b.md` | same shape; model rules (SPDX license required) | same | ✅ (Apache-2.0; 0 errors) |
| T9 | Entry: OpenCode | `projects/opencode.md` | same shape | same | ✅ (repo moved sst → anomalyco in 2026, recorded; 0 errors) |
| T10 | Entry: Automerge | `projects/automerge.md` | same shape | same | ✅ (0 errors) |
| T11 | Entry: Heltec WiFi LoRa 32 V3 | `devices/heltec-wifi-lora-32-v3.md` | same shape; device category rules | same | ✅ (0 errors; hosted_by meshtastic inverse edge) |
| T12 | Entry: Install Meshtastic on Heltec V3 | `recipes/install-meshtastic-on-heltec-wifi-lora-32-v3.md` | naming rule (last `-on-` split resolves); carries `depends_on` + `runs_on` relations; ≤2500 tok | same + relation resolution | ✅ (0 errors; N05 warnings expected — targets are candidate until field gates pass) |
| T13 | Generation + hook wiring | `hooks/pre-commit`, all `llms.txt` files, `_meta/planned.md` updates | root + 6 category indexes generated; regeneration is no-op diff; hook committed, activation documented | run generator twice, diff empty; `build.py --check` green | ✅ (regeneration no-op diff verified; hook committed — activation `git config core.hooksPath hooks` pending user approval) |

## Summary (filled at completion)

_built: all 13 tasks — machinery (T0–T3) plus 9 candidate seed entries (T4–T12, all desk-research dated 2026-10-12) plus deterministic index generation and pre-commit hook (T13). `build.py --check`: 0 errors, 4 warnings (2× N05 recipe targets not yet proven, 2× R06 recipe inverse edges — all skeleton-inherent, expected to clear as field gates pass). Root index: 0 proven · 0 testing · 9 candidate._
_deferred: hook activation (`git config core.hooksPath hooks`) pending explicit user approval; ~35-entry full seed inventory, skills/ dir, publish pipeline per spec Deferred section._
