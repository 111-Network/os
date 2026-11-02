# CI/CD and GitHub hygiene (D28, 2026-10-23)

What runs on GitHub, what the owner sets in the repository settings, and what CD would take. The rule behind the layout:
**parity with the local gates** — a CI job runs the same script a lead runs before a commit, never a re-listing of its
steps, so "green" means the same thing on the board, in a PR and on `main`, and the two cannot drift apart.

## In the repo (landed)

| File | Does |
|---|---|
| `.github/workflows/ci.yml` | Four jobs on every PR, every push to `main`, and by hand. `bootstrap-gate` = `1-bootstrap/scripts/check.sh` (fmt, clippy, node111 tests, shellcheck, the hermetic gate assertions, notices, build-script gates, systemd-analyze, the mesh111 workspace via `5-messaging/scripts/check.sh`, the presence sidecar, the UI's `pnpm check`). `identity` = fmt/clippy/test in `2-identity`. `intelligence` = `3-intelligence/scripts/check.sh` (its musl `cargo check` gets the target and `musl-tools`). `knowledge-contract-hygiene` = the KB check the pre-commit hook runs, the LXMF sidecar's offline tests, contract drift (`pnpm gen:types` must leave the tree unchanged), the one-digit-folder rule and the no-attribution-lines rule from `AGENTS.md`. `permissions: contents: read`; one run per ref at a time (a newer push cancels the older run). |
| `.github/PULL_REQUEST_TEMPLATE.md` | What a PR says: the task row, the gate's last line, the record kept (row, DECISIONS, LICENSING/NOTICES, the Pi floor, no attribution). |
| `.github/dependabot.yml` | Workflow actions only, monthly, at most two PRs. Cargo/npm/pip pins are deliberate and tracked in `LICENSING.md`. |
| `.gitattributes`, `.editorconfig` | LF everywhere; byte-exact fixtures (`*.golden`, `*.patch`, `*.minisig`) never normalised; indent rules per language. |
| `hooks/pre-commit` (already there) | Local: refuses a commit on a stale `main`, runs the KB check. Activate once per clone: `git config core.hooksPath hooks`. |

What CI deliberately does not do: musl / G2 (SKIP on these runners as on a box without the cross toolchains — a lock run stays a
lead's run with `REQUIRE_G2=1`), the netns harnesses (`*/verify/*.sh` need `unshare -Urnm`, which Ubuntu 24.04 runners restrict),
the zero-weight gate G7 (a real process table on a real box), hardware gates. Those stay on the layer boards as lead/bench items.

## Repository settings (owner; not expressible as files)

1. **Branch protection on `main`:** require the four CI checks (`Layer 1 aggregate gate …`, `Layer 2 id111 …`, `Layer 3 agent111 …`,
   `Knowledge base, sidecar, contract drift, repo hygiene`), require a PR (no direct pushes), require the branch to be up to date before
   merging (the stale-base rule in `AGENTS.md`), linear history (squash or rebase; no merge commits), no force-push, no deletion.
2. **Actions:** allow actions from GitHub and verified creators only; workflow permissions "read repository contents" (the workflow
   asks for nothing more); require approval for first-time contributors' runs.
3. **Security:** Dependabot alerts on (cargo, npm, pip — alerts, not update PRs), secret scanning + push protection on.
4. **Merges:** squash by default (one verified task = one commit on `main` keeps the board's commit hashes meaningful); delete head
   branches on merge.
5. Optional later: pin the actions to commit SHAs instead of major tags (Dependabot keeps them fresh either way).

## CD (planned, not enabled)

A release is `1-bootstrap/scripts/cross-build.sh` for the two musl targets plus the Layer 1 release checklist (`1-bootstrap/README.md`,
`1-bootstrap/TASKS.md`): minisign-signed manifests and binaries, the channel file, the notices. A `release.yml` on a `v*` tag could
build the two targets with the musl toolchains installed on the runner and upload the **unsigned** binaries and the generated
notices as workflow artifacts; signing stays on the owner's machine with the owner's key (the key never goes into a repository
secret unless the owner decides so). Two decisions before it exists: (a) whether CI-built binaries are the release binaries at all
(reproducibility: the cross-build pins CC/AR and the vendored OpenSSL/SQLCipher, so a runner build should match a lead build byte
for byte — verify once before trusting it), (b) where the signing key lives. Until then, releases are lead-built; CI proves the
tree, not the artifact.

## Keeping it light

CI is the only resident process this adds, and it runs on GitHub, not on a node. Nothing here changes the required configuration
(AGENTS.md, the Raspberry Pi floor). A job that starts taking more than ~25 minutes warm is a sign a gate grew; split the gate on the
board first, then the job.
