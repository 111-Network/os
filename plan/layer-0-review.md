# Layer 0 — Adversarial Review Triage (2026-10-12)

Panel: 4 judges (2× fable, 2× opus), lenses consistency / small-model / fidelity / redteam.
Verdicts: 4× **fix-then-ship**. Findings: 5 blocker, 31 major, 26 minor (62 raw, heavy cross-judge convergence).
Full raw reviews: workflow `wf_9b84224a-fc1` journal + `/tmp` capture.

## Disposition: 42 accepted → folded into spec v2 · 13 minors deferred to backlog · 7 rejected as over-engineering

### Accepted — blockers (5/5)

1. **Evidence has no machine-readable home** (3 judges) → sixth fixed body section `## Evidence` with status-tiered subsections (`### Alternatives considered` at candidate+, `### Smoke test` at testing+, `### Field log` at proven); lint asserts presence by status.
2. **Spec's own frontmatter example fails its own lint** (3 judges) → example uses only seeded slugs, `status: candidate`; cold-start rule: relations target existing entries only, forward refs go in prose placeholder wikilinks.
3. **`head -3` contract physically impossible** (2 judges) → "first non-empty body line after the closing `---`"; pinned extraction command; body line 1 == frontmatter `summary` verbatim (linted equal).
4. **stdlib-only Python cannot parse the mandated YAML** → drop stdlib-only; pin PyYAML (boring, system-packaged) + define a strict linted YAML subset (quoted wikilinks mandatory — unquoted `[[x]]` parses as list-of-lists and silently corrupts the graph).
5. **Per-category field/gate matrix missing** (2 judges) → matrix added: `license` required for project+model (Gemma-flag lesson), `upstream` (renamed from `repo`) covers spec URLs for protocols, `concept` entries are status-free, gates adapted per category (protocol = spec stability + ≥2 independent implementations; device = availability + our install test).

### Accepted — majors (24, deduplicated)

- Summary lint: ≤12 words, no `|`/newline, generator fails never truncates; index keywords = first 3 tags, sorted.
- Token budgets: measured as chars/4, **warning-only**, exclude frontmatter; pointer entries exempt from floor; recipes ≤2500; AGENTS.md ≤400 tokens; full retrieval ritual ≤3.5k tokens composed.
- `timeless/dated/pointer`: no new field — derived: dated = carries `last_verified` (required for all categories except concept); rule stays as guidance.
- `hold` placement: excluded from root index, listed in category `Optional` marked `| hold`; new edges to hold entries = error; recipes targeting non-proven = error.
- Staleness unified (4 judges found the contradiction): `last_verified` >90d = warning (any status); proven >180d = error + demotion proposal; `last_verified` must equal the newest dated Evidence log line (date-bumping without evidence fails lint); wording downgraded to "lint-proposed demotions" pre-CI.
- Predicate symmetry table in the locked spec: `alternative_to`/`relates_to` symmetric; added `implemented_by`, `uses` inverses; missing-inverse = warning (expected noisy during skeleton).
- Per-folder index format defined: same proven/Optional split, slug byte-ascending order, empty proven sections omitted, header from `_meta/index-header.md`, relpath relative to index folder.
- Severity table added (rule → error/warning) — normative home is `_meta/schema.md` §1.
- Wikilink resolution made executable: unique-basename lookup via generated index/glob, never path computation; bare `[[slug]]` only (no pipes/headings/embeds); dangling prose link that normalizes to an existing slug = did-you-mean error; true placeholders must be listed in `_meta/planned.md` (unlisted = warning, printed in build output); folder authoritative, category↔folder mismatch = error; all paths relative to `os/` repo root (root AGENTS.md = `os/AGENTS.md` — the fidelity judge caught that `111network/` is not a repo).
- Retrieval ritual pinned: exact commands (`grep -ril`, filenames-first, `_meta/` excluded, one-file-at-a-time loop), no-tool degraded path (answer from llms.txt alone), worked example, ≤400-token AGENTS.md carrying ritual + pointers only (no schema duplication — one normative copy per fact).
- Empty-proven-at-birth: root index header carries a status tally ("0 proven · 8 candidate — full catalog under Optional, read it first"), repeated verbatim in AGENTS.md.
- Seed amendment proposed: +1 recipe seed `install-meshtastic-on-heltec-wifi-lora-32-v3.md` (both endpoints already seeded; exercises the recipe naming + generator path). 8 → 9 seeds.
- Content trust: summaries descriptive-only (no imperatives/superlatives/second person); commands only in fenced code blocks; sources stripped to scheme+host+path, ≤5; AGENTS.md declares entry content untrusted data.
- Pre-CI enforcement: `build.py --check` (validate, write nothing, non-zero on diff) + committed git hook; Python ≥3.11 pinned; golden-fixture self-test so generator rot is visible.
- Publisher honesty: wikilinks are NOT CommonMark — publisher's single obligation written down (resolve `[[slug]]` → route of unique slug.md); unresolved wikilinks render as plain text, never 404 links.
- `platforms` vs `runs_on`: platforms = family-granularity search shorthand; lint warns on divergence from `runs_on` edges.
- Lint compares parsed YAML semantically, not textually (Obsidian re-serialization safe).
- Recipe filenames: `-on-` forbidden inside project/device slugs; split on LAST `-on-`; both segments must resolve to existing slugs; recipe must also carry `depends_on` + `runs_on` relations.
- Licenses: valid SPDX expressions (no bare `GPL-3.0` ambiguity); quarterly manual re-verify includes license check until CI.
- Promotion to proven requires maintainer countersign or second named party reproducing the smoke test, recorded in the log.
- "short lines" restored (≤~120 chars, warning); own-docs coverage line added (concepts/ holds pointer entries to os/plan docs).
- Summary carries the single top contraindication when one exists (Lost-in-the-Middle mitigation).

### Deferred to backlog (recorded in spec's Deferred section)

Identity-systems inventory (required before Layer 2 build — the fresh Layer 2 session's research will draft entries) · recipe/concept entries beyond the one seed · per-category llms-full bundles · LLM freshness audit · publish CI wiring (lychee etc.).

### Rejected as over-engineering (with reason)

- `entry_type: timeless|dated|pointer` frontmatter field — derivable from `last_verified` presence + category; 13th field not worth its lint cost.
- OKF-style `{predicate, object, date, source}` triples — invisible in Obsidian graph; map form stays.
- Evidence sidecar files (`evidence/` dir) — `## Evidence` section suffices; budgets exclude frontmatter and allow compact dated log lines.
- Reopening skeleton scope to full 35-entry seed — user decision F2 stands.
- MCP wrapper / SKILL.md packaging — already deferred by design, judges raised nothing new.
- Two-way alias-aware wikilink grammar — bare-slug-only rule makes it unnecessary.
- Merge-governance beyond countersign line — single-maintainer reality; full governance is a later problem.
