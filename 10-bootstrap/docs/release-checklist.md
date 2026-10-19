# Release checklist — `111` channel release (calver `YYYY.MM.DD[.N]`)

One release = the full pinned artifact set (spec §Artifacts). Order matters;
every pin flows from the signed channel manifest.

## 0. Preconditions

- `REQUIRE_G2=1 scripts/check.sh` ALL GREEN on the commit to be released (G2, the SQLCipher
  static link, must run: a skipped G2 is "GREEN (G2 SKIPPED)", never a release).
- musl cross toolchains present (`~/.local/toolchains`), `minisign` on PATH.
- Release signing key available (cold-custody root key stays offline; day-to-day
  releases sign with the current keychain generation — see keys/README.md).
- Termux APK pins refreshed (Layer 9 task; v1: termux-app v0.118.3,
  Termux:Boot v0.8.1, Termux:API v0.53.0) and the .deb closure re-validated
  (monthly CI; the closure list settled by spike V3).
- Layer 5 build toolchains on the release runner (CI only; nothing here runs on a node and
  nothing downloads at runtime): Go (version in `scripts/meshmud-build.sh`, `MESHMUD_GO_MIN`) for
  `meshmud`; git + pnpm + node for `scripts/meshtastic-web-build.sh` when upstream's asset is
  incomplete (`--mode auto` falls back to a source build). The pins in those two scripts are the
  only source of truth; moving one means new notices first (`gen-notices.py --check`).
- KB snapshot packaged as `kb.tar.gz` (immutable release asset — never a
  codeload pin; GitHub regenerates codeload bytes).

## 1. Build

```sh
scripts/cross-build.sh 2026.10.12 dist/releases/2026.10.12
```

Produces `dist/releases/2026.10.12/111-{x86_64,aarch64}-unknown-linux-musl`
(static-asserted; aarch64 p_align=0x4000 asserted; SQLCipher + OpenSSL strings asserted).

Layer 5 release files (D20; `50-messaging/README.md` §19 L1), each built by CI from a pinned
upstream and never copied into `os/`:

```sh
scripts/meshmud-build.sh --out dist/meshmud/<tag>                # Go; needs the pinned tarball
scripts/meshtastic-web-build.sh --out dist/meshtastic-web/<tag>  # the pinned web client tag
```

Read `meshmud-<tag>.verdict` before going on: it says which license the binaries must be conveyed
under (GPL-3.0 when they link Meshtastic's Go protobufs). It, the `.SOURCE` files, `.buildinfo`,
`LICENSE.meshmud` and `LICENSE.meshtastic-web` go on the release page beside the binaries
(`LICENSING.md` section 8). The presence reader needs no build: its lock is
`50-messaging/sidecars/meshtastic-presence/meshtastic-presence-lock.txt` (re-check with
`gen_lock.py --verify`; `scripts/check.sh` runs it).

## 2. Assemble artifacts

Into `dist/releases/2026.10.12/` place: the two `111` binaries, the two
`payload-<triple>.tar.gz` (layout below), the three Termux APKs, `rns-lock.txt`,
`kb.tar.gz`, `debs/` (the pinned Termux .deb closure), and the Layer 5 files:

| Release file (artifacts dir) | Built by | Rides in each payload as | Manifest section (gen-manifest.py) |
|---|---|---|---|
| `meshmud-<tag>-linux-amd64`, `meshmud-<tag>-linux-arm64` | `scripts/meshmud-build.sh` (CI, pinned upstream tarball) | `bin/meshmud` (0755): amd64 in the x86_64 payload, arm64 in the aarch64 one | `[pkg.meshmud.target.<triple>]` |
| `meshtastic-web-<tag>.tar` + `meshtastic-web-<tag>.tree` | `scripts/meshtastic-web-build.sh` | the `.tar` unpacked at `panels/meshtastic-web/` | `[bundle.meshtastic-web]` (`tree-sha256` = sha256 of the `.tree`) |
| `meshtastic-presence-lock.txt` | `50-messaging/sidecars/meshtastic-presence/` (`gen_lock.py`) | `share/meshtastic-presence/` with `meshtastic_presence.py`, the lock and `LICENSE` | `[presence-lock]` |

The Layer 5 files are optional to the generator, so a missing one is a silently thinner release
(the node then reports meshmud or the panel honestly absent). A full release has all four
Layer 5 sections (two `meshmud` targets, the bundle, the presence lock); step 4 asserts it.
Keep exactly one file per pattern (more is an error).

Payload layout (identical bytes to the artifacts above; the unpack refuses links and `..`):

```
bin/111  bin/meshmud
share/kb/...  share/meshtastic-presence/{meshtastic_presence.py,meshtastic-presence-lock.txt,LICENSE}
panels/meshtastic-web/...  share/payload.manifest
```

`share/payload.manifest` (lines `sha256␠␠relpath`) must LIST every other regular file in the
payload, Layer 5 assets included. The rule, from `manifest.rs` and `daemon/updater/stage.rs`:
`verify_tree` hashes only the files the manifest lists and never looks for files it does not list.
`ChannelManifest::verify_layer5_tree` (the Layer 5 pins above) runs after it in the root helper,
the installer and the updater, so a bad Layer 5 asset is refused at install and update time
either way. But `update-guard` (before every daemon start) runs `verify_tree` alone on the
current release, so an unlisted `bin/meshmud`, bundle file or reader script is never re-hashed
at boot and a torn one goes undetected. Neither `gen-manifest.py` nor the node checks that the
listing is complete, so this step is on you. From the payload tree:

```sh
find bin share panels -type f ! -path share/payload.manifest -print0 \
  | LC_ALL=C sort -z | xargs -0 sha256sum > payload.manifest.tmp \
  && mv payload.manifest.tmp share/payload.manifest
# prove it: nothing unlisted, nothing phantom, every hash holds (unpack the tarball you will ship)
T=$(mktemp -d); find bin share panels -type f ! -path share/payload.manifest | LC_ALL=C sort > "$T/files"
sed 's/^[0-9a-f]\{64\}  //' share/payload.manifest | LC_ALL=C sort | diff "$T/files" -
sha256sum --quiet -c share/payload.manifest
```

No output from `diff` and `sha256sum` means green. Then pack the tree with member names that do
not start with `./` (the node refuses them): `tar -czf payload-<triple>.tar.gz bin share panels`
from inside it. Pack it after the listing is written, never before.

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
generated file (it runs gen-manifest.py + the T2 parser); so must
`cargo test --test manifest_layer5` (the Layer 5 sections and the generator's cross-check).

The Layer 5 sections are written when their files are in the artifacts dir, and the generator
refuses a payload whose `bin/meshmud`, `panels/meshtastic-web/` tree or presence lock differs
from the pins it is about to write (and a `meshmud` with no `111` binary and payload of its
triple). Assert a full release before signing:

```sh
grep -E '^\[(pkg\.meshmud\.target\.|bundle\.meshtastic-web|presence-lock)' dist/channel-111-stable.toml
# expect four lines: meshmud x86_64 + aarch64, bundle.meshtastic-web, presence-lock
```

## 5. Publish

1. GitHub Release `v2026.10.12` (canonical): upload the artifact set (the Layer 5 files
   too: every manifest URL points at them),
   `channel-111-stable.toml{,.minisig,.sha256}`, and — on the release page —
   the shim's sha256 + minisig (independent channel) and the Layer 5 license/source files
   from step 1 (`.verdict`, `.SOURCE`, `.buildinfo`, `LICENSE.meshmud`,
   `LICENSE.meshtastic-web`).
2. Corresponding Source (D22; `LICENSING.md` section 8): publish the tagged source of exactly
   this release beside every binary release, on the GitHub Release and on the mirror (an
   immutable asset, not GitHub's regenerated codeload archive):
   ```sh
   git archive --format=tar.gz --prefix=111-os-2026.10.12/ \
     -o dist/releases/2026.10.12/111-os-source-2026.10.12.tar.gz v2026.10.12
   sha256sum dist/releases/2026.10.12/111-os-source-2026.10.12.tar.gz
   ```
   The release notes name the tag and its commit (`git rev-parse v2026.10.12^{commit}`) and
   point at this file; `LICENSE.md` and `THIRD-PARTY-NOTICES.md` sit beside it. It is not a
   manifest artifact: nothing pins it and `gen-manifest.py` ignores it. No source, no release.
3. Mirror `install.111.network/v2026.10.12/` with the same files; update
   `install.111.network/install.sh` → the new shim and
   `install.111.network/stable/channel-111-stable.toml{,.minisig}` → the new
   manifest (+204 probe endpoints stay up).
4. Verify the trust chain end-to-end from a clean VM: the one command →
   running node (V8 gate).

## 6. Key rotation (when it happens)

- New keypair generated offline; the NEW pubkey is signed by the CURRENT key
  (detached minisig) → keychain entry with monotonic generation.
- Manifest ships the extended keychain; the floor's keychain generation
  ratchets forward (old manifests replay-proof).
- Compromise recovery: re-anchor via a new shim/installer release and note it
  in the release notes; pubkey published in repo + docs + release notes.
