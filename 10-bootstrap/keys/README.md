# Release signing keys (custody)

- `manifest-pub.key` — the minisign **verify** key compiled into the `111`
  binary (updater, installer, helper). Managed by scripts/release.sh (T15).
- The CURRENT file is a **DEV-ONLY** key (same throwaway keypair as the test
  fixtures, `crates/node111/tests/fixtures/minisign/test.key`). It exists so
  dev builds and integration tests can verify signed manifests end-to-end.
  IT MUST BE REPLACED before the first real signed release; the cold-custody
  root key never lives in this repo.
- Rotation: new keys are signed by old (keychain in the channel manifest,
  monotonic generations); compromise recovery = re-anchor via a new
  shim/installer release. Pubkey published in repo + docs + release notes.
