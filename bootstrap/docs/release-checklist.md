# Release checklist — `111` channel release (calver `YYYY.MM.DD[.N]`)

One release = the full pinned artifact set (spec §Artifacts). Order matters;
every pin flows from the signed channel manifest.

## 0. Preconditions

- `scripts/check.sh` ALL GREEN on the commit to be released.
- musl cross toolchains present (`~/.local/toolchains`), `minisign` on PATH.
- Release signing key available (cold-custody root key stays offline; day-to-day
  releases sign with the current keychain generation — see keys/README.md).
- Termux APK pins refreshed (Layer 9 task; v1: termux-app v0.118.3,
  Termux:Boot v0.8.1, Termux:API v0.53.0) and the .deb closure re-validated
  (monthly CI; the closure list settled by spike V3).
- KB snapshot packaged as `kb.tar.gz` (immutable release asset — never a
  codeload pin; GitHub regenerates codeload bytes).

## 1. Build

```sh
scripts/cross-build.sh 2026.10.12 dist/releases/2026.10.12
```

Produces `dist/releases/2026.10.12/111-{x86_64,aarch64}-unknown-linux-musl`
(static-asserted; aarch64 p_align=0x4000 asserted).

## 2. Assemble artifacts

Into `dist/releases/2026.10.12/` place: the two `111` binaries, the two
`payload-<triple>.tar.gz` (bin/111 + share/kb/ + share/payload.manifest), the
three Termux APKs, `rns-lock.txt`, `kb.tar.gz`, and `debs/` (the pinned
Termux .deb closure).

## 3. Shim

```sh
scripts/gen-shim.sh 2026.10.12 dist/releases/2026.10.12 dist
```

## 4. Manifest + signature

```sh
manifest/gen-manifest.py \
  --version 2026.10.12 --artifacts-dir dist/releases/2026.10.12 \
  --min-upgrade-from 2026.10.01 \
  --url-base https://github.com/111-Network/os/releases/download/v2026.10.12 \
  --keychain-gen 1 --pubkey "$(cat keys/manifest-pub.key)" \
  --debs-dir dist/releases/2026.10.12/debs \
  --out dist/channel-111-stable.toml
minisign -S -s <release-secret-key> -m dist/channel-111-stable.toml
sha256sum dist/channel-111-stable.toml > dist/channel-111-stable.toml.sha256
```

Round-trip check: `cargo test generated_manifest_parses` must pass on the
generated file (it runs gen-manifest.py + the T2 parser).

## 5. Publish

1. GitHub Release `v2026.10.12` (canonical): upload the artifact set,
   `channel-111-stable.toml{,.minisig,.sha256}`, and — on the release page —
   the shim's sha256 + minisig (independent channel).
2. Mirror `install.111.network/v2026.10.12/` with the same files; update
   `install.111.network/install.sh` → the new shim and
   `install.111.network/stable/channel-111-stable.toml{,.minisig}` → the new
   manifest (+204 probe endpoints stay up).
3. Verify the trust chain end-to-end from a clean VM: the one command →
   running node (V8 gate).

## 6. Key rotation (when it happens)

- New keypair generated offline; the NEW pubkey is signed by the CURRENT key
  (detached minisig) → keychain entry with monotonic generation.
- Manifest ships the extended keychain; the floor's keychain generation
  ratchets forward (old manifests replay-proof).
- Compromise recovery: re-anchor via a new shim/installer release and note it
  in the release notes; pubkey published in repo + docs + release notes.
