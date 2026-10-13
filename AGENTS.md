# AGENTS.md

This repo (`os/`) is the operating layer of the 111network mesh OS: an offline-first, self-healing layer that turns any device into a mesh node — adopt proven projects, write custom glue only where nothing fits.

## Read first

- Knowledge-base navigation contract: `knowledge/AGENTS.md` — read it before querying `knowledge/`.
- Layer 0 (Knowledge Layer) spec: `knowledge/README.md` — locked; defines entry format, relations, status gates, indexes.
- Layer 1 (Bootstrap) spec: `bootstrap/README.md` — locked v2.2; the `111` binary, installer, daemon, updates.
- Layer 2 (Identity) spec: `identity/README.md` — locked v2; keyset, announces, backup/restore, agentic + transport hooks. Core crate: `identity/crates/id111`.
- Build process and task plan: `plan/INDEX.md` (layer status) · `plan/DECISIONS.md` (every decision + rejected alternatives) · `plan/EXECUTION.md` (build protocol). Per-layer build boards: `<layer>/TASKS.md`.
- Layer 3 (Local Intelligence) spec + as-built appendix: `intelligence/README.md`; build log/contracts: `intelligence/TASKS.md`.
