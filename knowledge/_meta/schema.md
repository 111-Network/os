# Schema — normative authoring and lint contract

Machine-checkable contract for every entry under `knowledge/`. `_meta/build.py` implements this file; where it and the locked spec (`knowledge/README.md`) disagree, the spec wins and this file is corrected.

Scope: entry rules apply to files in the six category folders (`projects/`, `devices/`, `recipes/`, `protocols/`, `models/`, `concepts/`). `_meta/` docs are not entries; generated `llms.txt` files are checked by X-rules only.

## §1 Rule → severity table

**error** = validation fails (`--check` exits non-zero). **warning** = printed in build output, never fatal. Token budgets are warning-only by spec. Summary tone (descriptive only, no imperatives, no superlatives, no second person) and "commands only inside fenced code blocks" are human-reviewed, not linted.

### Frontmatter / YAML

| ID | Rule | Severity |
|---|---|---|
| F01 | Exactly one YAML frontmatter block at file top, parsing within the §3 subset | error |
| F02 | Key outside the 12 known fields, or not lowercase snake_case | error |
| F03 | Required field for the entry's category missing (§2 matrix) | error |
| F04 | Forbidden field present, or field present-but-empty (inapplicable fields are omitted, never empty) | error |
| F05 | `category` outside its enum, or parent folder ≠ `<category>s` | error |
| F06 | `status` outside {candidate, testing, proven, hold} | error |
| F07 | `title` contains `|` or a newline | error |
| F08 | `title` contains the word "and" — split the entry | warning |
| F09 | `summary` missing, >12 words, multi-line, or contains `|` | error |
| F10 | `tags` item not lowercase slug shape (`^[a-z0-9][a-z0-9-]*$`; no `#`, no commas inside a tag) | error |
| F11 | Tag not listed in `_meta/vocabularies.md` — allowed, flagged for review | warning |
| F12 | `platforms` token not listed in `_meta/vocabularies.md` | error |
| F13 | `platforms` token equals an existing device slug absent from `runs_on` | warning |
| F14 | `license` fails SPDX expression shape | error |
| F15 | `sources` item not scheme+host+path (query/fragment present), or more than 5 sources | error |
| F16 | `last_verified` not an ISO 8601 date (`YYYY-MM-DD`) | error |
| F17 | `last_verified` ≠ newest dated line in `## Evidence`, when dated evidence exists | error |

### Body

| ID | Rule | Severity |
|---|---|---|
| B01 | First non-empty body line not verbatim-equal to frontmatter `summary` | error |
| B02 | Fixed H2 sections missing, renamed, or out of order: `## What it is` · `## Use when` · `## Do NOT use when` · `## Details` · `## Evidence` · `## Links` | error |
| B03 | Required `## Evidence` subsection missing for status: `### Alternatives considered` at candidate+ (including hold), `### Smoke test` at testing+, `### Field log` at proven | error |
| B04 | Evidence subsection present above the entry's status tier (e.g. `### Field log` on a candidate) | warning |
| B05 | Body line longer than 120 chars | warning |
| B06 | Prose wikilink not bare `[[slug]]` (pipe, alias, `#` heading, `^` block ref, or `!` embed) | error |
| B07 | Dangling prose wikilink not registered in `_meta/planned.md` | warning |
| B08 | Dangling wikilink case/kebab-normalizes to an existing slug (did-you-mean) | error |
| B09 | Token budget exceeded (chars/4, frontmatter excluded): entry body >1500; non-pointer body <150; recipe >2500; `knowledge/AGENTS.md` >400; category index >1500; root index >1500 or >60 lines | warning |

### Naming

| ID | Rule | Severity |
|---|---|---|
| N01 | Filename not a vault-wide-unique kebab-case slug (`^[a-z0-9]+(-[a-z0-9]+)*\.md$`) | error |
| N02 | Project/device slug contains the token `on` (the `-on-` segment is reserved for recipe grammar) | error |
| N03 | Recipe filename not `install-<project>-on-<device>.md`, or a segment (split on the LAST `-on-`) does not resolve to an existing project/device slug | error |
| N04 | Recipe missing `depends_on` (its project) or `runs_on` (its device) matching the filename segments | error |
| N05 | Recipe `depends_on`/`runs_on` target not `status: proven` | warning |

### Relations

| ID | Rule | Severity |
|---|---|---|
| R01 | Predicate not in `_meta/predicates.md` | error |
| R02 | Relations target does not resolve to an existing entry (edges never dangle; `planned.md` excuses prose links only) | error |
| R03 | Self-edge (target equals own slug) | error |
| R04 | `supersedes` cycle | error |
| R05 | Edge to an entry with `status: hold` | error |
| R06 | Missing inverse edge on an asymmetric predicate | warning |

### Staleness and generated files

| ID | Rule | Severity |
|---|---|---|
| S01 | `last_verified` older than 90 days (any status) | warning |
| S02 | `last_verified` older than 180 days on a `proven` entry — lint proposes demotion (a human applies it) | error |
| X01 | Generated `llms.txt` missing or carrying an altered generator marker | error |
| X02 | Generated `llms.txt` stale: semantic regeneration under `--check` produces a diff | error |

Procedural, not lintable: renaming a file requires updating inbound links in the same commit; rubric gate facts are human-attested (lint checks structure only, never the facts).

## §2 The 12 frontmatter fields

Field definitions:

- `title` — string. Human-readable name. No `|`, no newline. If the name needs "and", split the entry (F08).
- `category` — enum: `project` | `device` | `recipe` | `protocol` | `model` | `concept`. Must equal the singular of the parent folder name.
- `status` — enum: `candidate` | `testing` | `proven` | `hold`. Lifecycle per spec §Status lifecycle. Forbidden for concept (concepts are timeless).
- `summary` — string. ≤12 words (whitespace-split), single line, no `|`. Descriptive only — no imperatives, no superlatives, no second person (human-reviewed). If the entry has one top contraindication, the summary carries it. Feeds `llms.txt`; verbatim-equal to the first non-empty body line (B01).
- `tags` — flow list of lowercase tokens. Filtering only — NEVER relations. No `#`, no commas inside a tag. Vocabulary in `_meta/vocabularies.md`; unseen tags warn (F11).
- `aliases` — flow list of strings. Obsidian search aid only; feeds nothing machine-read.
- `platforms` — flow list of tokens from `_meta/vocabularies.md`. Family granularity. Search shorthand only; the machine edges are `runs_on`/`hosted_by`.
- `upstream` — URL scalar. Canonical repo (project, model) or spec home (protocol).
- `license` — string. Valid SPDX expression (e.g. `GPL-3.0-only`).
- `last_verified` — ISO 8601 date (`YYYY-MM-DD`). Must equal the newest dated line in `## Evidence` when dated evidence exists (F17). Dated = carries `last_verified`; concepts are timeless.
- `sources` — flow list of URLs, scheme+host+path only (no query, no fragment), at most 5. Provenance.
- `relations` — one nested mapping level: predicate → flow list of double-quoted `[[wikilinks]]`. Predicates from `_meta/predicates.md`; targets must exist (R02).

Per-category applicability matrix. R = required (present and non-empty) · O = optional (may be present, never empty) · — = forbidden (must be absent).

| Field | project | device | recipe | protocol | model | concept |
|---|---|---|---|---|---|---|
| title | R | R | R | R | R | R |
| category | R | R | R | R | R | R |
| status | R | R | R | R | R | — |
| summary | R | R | R | R | R | R |
| tags | R | R | R | R | R | R |
| aliases | O | O | O | O | O | O |
| platforms | R | R | — | — | O | — |
| upstream | R | O | — | R | O | — |
| license | R | O | — | O | R | — |
| last_verified | R | R | R | R | R | — |
| sources | O | O | O | O | O | O |
| relations | O | O | R | O | O | O |

Basis: spec frontmatter paragraph (`upstream`+`license` required for project; `license` required for model; `upstream` required for protocol; `platforms` required for project/device; `status`/`last_verified` forbidden for concept; inapplicable fields omitted, never present-but-empty). Recipe `relations` must include `depends_on` + `runs_on` matching its filename (N03/N04). `tags` is required with ≥1 item so the index line grammar always has keywords.

## §3 YAML authoring subset

Exactly one `---` frontmatter block at the very top of the file, containing only:

- keys: lowercase snake_case, from the 12 known fields;
- scalars: plain or double-quoted; ISO 8601 dates for `last_verified`;
- lists: flow style (`tags: [mesh, lora]`) or block style (`sources:` + indented `- ` lines) — both legal; items are plain or double-quoted scalars;
- `relations:` exactly one nested mapping level, values are flow lists;
- ALL `[[wikilinks]]` double-quoted — unquoted `[[x]]` parses as a list-of-lists and silently corrupts the graph;
- quote any scalar containing `:#[]{}&,*!|>'"%@`;
- no anchors or aliases (`&`, `*`), no multi-line scalars (`|`, `>`);
- no present-but-empty fields: omit instead.

Lint compares parsed YAML semantically, never textually — Obsidian may re-serialize style.

YAML 1.1 traps (PyYAML) to avoid:

- `yes`/`no`/`on`/`off`/`true`/`false` in any case parse as booleans — quote them when meant as strings.
- Unquoted `2026-10-01` parses as a date object — intended for `last_verified`; compare as ISO dates and never write datetimes.
- `~`, `null`, or an empty scalar parses as null — forbidden (F04).
- Leading-zero numbers parse as octal (`010` = 8); `0x…` parses as hex — quote number-like strings.
- `1:20` parses as a sexagesimal integer — quote any scalar containing `:`.
- Unquoted `[[slug]]` parses as a nested list — always double-quote wikilinks.

## §4 Lint rules, mechanically

Setup:

1. Slug index: basenames of all `*.md` files under the six category folders. N01 checks kebab-case shape and vault-wide uniqueness.
2. Frontmatter: text between the first `---` line and the next `---` line; parse with PyYAML `safe_load`. Subset violations (anchors, block sequences, multi-line scalars, unquoted wikilinks surfacing as non-string nodes) are detected on the parse tree, then F01-F17 run on the loaded mapping.
3. Body: everything after the closing `---`. Prose wikilinks are `!?\[\[([^\]]+)\]\]` matches on body lines outside fenced code blocks.
4. Controlled lists: platforms and tags are the first token of each `- ` line under `## Platforms` / `## Tags` in `_meta/vocabularies.md`; registered placeholders are the first token of each `- ` line under `## Registry` in `_meta/planned.md`.

Checks (IDs per §1):

- F01–F04: block parses under §3; keys ⊆ 12 fields and snake_case; §2 matrix: required present and non-empty, forbidden absent; no null or empty values anywhere.
- F05: folder name = `<category>s`. F06–F07: enum and `|`/newline checks. F08: case-insensitive word match `\band\b` in `title`.
- F09: `words = summary.split()`; require 1–12, single line, no `|`.
- F10–F11: each tag matches `^[a-z0-9][a-z0-9-]*$`; then membership in the tag list.
- F12: platforms ⊆ platform list. F13: any platform token that equals a device slug must appear in this entry's `runs_on` targets.
- F14: SPDX expression check (license list pinned in `build.py`).
- F15: each source matches `^[a-z][a-z0-9+.-]*://[^/?#]+(/[^?#]*)?$`; list length ≤5.
- F16: `last_verified` matches `^\d{4}-\d{2}-\d{2}$` and parses as a date. F17: collect all `YYYY-MM-DD` tokens inside `## Evidence`; if any, the max must equal `last_verified`.
- B01: first non-empty body line — `awk '/^---$/{n++; next} n==2 && NF{print; exit}' <file>` — byte-equal to `summary`.
- B02: the H2 sequence is exactly `What it is`, `Use when`, `Do NOT use when`, `Details`, `Evidence`, `Links`, in that order, nothing else at H2.
- B03/B04: required subsections by status — candidate/testing/proven/hold → `### Alternatives considered`; testing/proven → `### Smoke test`; proven → `### Field log`. Any other Evidence subsection present = B04 warning. Concepts (no status) require none.
- B05: any body line with length >120.
- B06: wikilink inner text must match `^[a-z0-9]+(-[a-z0-9]+)*$` with no leading `!`. B07: unresolved target → must be registered in `planned.md`, else warning. B08: normalize the target (lowercase, `[_\s]+` → `-`); if it then equals an existing slug, error.
- B09: tokens = chars/4 of the body, frontmatter excluded; compare against the §1 budgets. Pointer entries are exempt from the 150-token floor (v1 has none; a pointer is designated by human convention, no frontmatter marker).
- N02: slug split on `-` must not contain the token `on` for project/device entries.
- N03/N04: recipe basename split on the LAST `-on-`; left must resolve under `projects/`, right under `devices/`; `relations` must carry `depends_on: ["[[<left>]]"]` and `runs_on: ["[[<right>]]"]`. N05: both targets must be `status: proven`, else warning.
- R01–R06: predicates ⊆ `_meta/predicates.md`; targets ∈ slug index; target ≠ own slug; DFS over `supersedes` edges finds no cycle; no edge target has `status: hold` (the linter cannot distinguish "new" edges, so every edge to hold is flagged); inverse check per the predicate table — symmetric predicates (`alternative_to`, `relates_to`) are satisfied by either direction.
- S01/S02: `days = today − last_verified`; >90 = warning; >180 on `proven` = error plus a proposed-demotion note.
- X01: every generated `llms.txt` carries the marker line verbatim: `<!-- generated by knowledge/_meta/build.py — do not hand-edit -->` (canonical string, pinned here; `build.py` emits it).
- X02: `--check` regenerates every index semantically (parsed header + line set, not text) and fails on any diff. Line grammar: `- [title](relpath): summary | status | tag1,tag2,tag3` — summary from frontmatter (the generator fails on F09 violations, never truncates), keywords = first 3 tags sorted, relpath relative to the index's own folder, entries sorted by slug byte-ascending. Hold entries are excluded from the root index and listed in their category index marked `| hold`. Root tally line: `{P} proven · {T} testing · {C} candidate`; while P=0 it appends ` — full catalog under Optional — read it first` (canonical string; `knowledge/AGENTS.md` repeats the appended clause verbatim).
