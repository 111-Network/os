# AGENTS.md

This repo (`os/`) is the operating layer of the 111network mesh OS: an offline-first, self-healing layer that turns any device into a mesh node — adopt proven projects, write custom glue only where nothing fits.

## Read first

- Knowledge-base navigation contract: `00-knowledge/AGENTS.md` — read it before querying `00-knowledge/`.
- Layer 0 (Knowledge Layer) spec: `00-knowledge/README.md` — locked; defines entry format, relations, status gates, indexes.
- Layer 1 (Bootstrap) spec: `10-bootstrap/README.md` — locked v2.2; the `111` binary, installer, daemon, updates.
- Layer 2 (Identity) spec: `20-identity/README.md` — locked v2; keyset, announces, backup/restore, agentic + transport hooks. Core crate: `20-identity/crates/id111`.
- Build process and task plan: `plan/INDEX.md` (layer status) · `plan/DECISIONS.md` (every decision + rejected alternatives) · `plan/EXECUTION.md` (build protocol). Per-layer build boards: `<layer>/TASKS.md`.
- Layer 3 (Local Intelligence) spec + as-built appendix: `30-intelligence/README.md`; build log/contracts: `30-intelligence/TASKS.md`.
- Layer 4 (Interface & Dashboard) spec: `40-interface/README.md` — locked v2; HTTP contract `40-interface/api/openapi.yaml` (conventions `40-interface/api/README.md`); build board `40-interface/TASKS.md`. The UI is `40-interface/ui`; the Rust side lives in `10-bootstrap/crates/node111/src/daemon/dashboard/`.
- Licensing: our own license is `LICENSE.md` (draft, pending legal review). Third-party licensing and dependency boundaries (Reticulum is a separate service, never incorporated): `LICENSING.md` — update it when a dependency, vendored file, or pinned external program changes. Third-party notices and license texts: `THIRD-PARTY-NOTICES.md` — regenerate it when a lockfile changes.
