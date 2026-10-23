# Third-party licensing and dependency boundaries

Status: 2026-10-19 (dependency facts compiled 2026-10-14 from files in this repo and from locally readable package metadata:
`cargo metadata`, `cargo tree`, `pnpm licenses`; Layer 6 rows added 2026-10-19 from upstream files read that day, task S6 of `6-transport/TASKS.md`). It is a statement of facts and boundaries, not legal advice.
Anything not verifiable from the repo or local metadata is marked (verified 2026-10-14: upstream `LICENSE` = 0BSD for the fork's changes plus MIT for the MeshChat parts).

**Our own license (owner decision, 2026-10-19, `plan/DECISIONS.md` D22):** free secure communication is everyone's right. 111network's own code is free software
under the **GNU General Public License, version 3 or (at your option) any later version** (`GPL-3.0-or-later`; verbatim text and copyright notice in `LICENSE.md`):
free for anyone, for any purpose, including commercial use; whoever distributes a changed version must do so under the same license, with the source.
The earlier source-available draft (PolyForm Noncommercial 1.0.0 plus a no-military-use condition, 2026-10-14 to 2026-10-19) is withdrawn; it was never part of a release.
**Contributions of any kind are appreciated**; they are accepted under the same license (section 9 item 10). Section 7 checks that our dependencies allow this.

## 1. Purpose and the boundary principle

This file says which third-party code the 111network OS runs, how each piece relates to our code, and which obligations follow.
Three rules decide everything:

1. **Separate process = not a derivative.** A program we start and talk to over its own interfaces (command line, config file,
   socket, HTTP) stays a separate work under its own license. We do not become a derivative of it.
2. **Vendored = its license travels with it.** Source copied into this tree keeps its license text and notices.
3. **Linked = its license applies to the binary.** Every Rust crate compiled into `111` or `111-agent`, and every JS package bundled
   into the dashboard, puts its notice requirements on what we ship.

**Reticulum (owner decision, 2026-10-14; `plan/cross-layer/devices.md` section 7 item 12, `plan/cross-layer/parked.md` P5).** Reticulum is used **as-is and
separately**, as a connected service, like Meshtastic: a separate, unmodified upstream program, always the latest upstream release, started
and talked to by our daemon over its own interfaces. **No Reticulum code is incorporated into, linked into, or derived in `111d` or any other crate of ours.** One program of ours, the LXMF helper `lxmf-sidecar` (Layer 6, `plan/DECISIONS.md` D26 #4, which amends D17 #12), imports the user-installed `rns` and `lxmf` packages: it is a separate process in its own venv, never combined with `111d`, and it credits Reticulum (sections 3 and 8). The
Reticulum *protocol design* (dedicated to the public domain in 2016, per the KB) may inform our own mesh core, which is ours, in Rust.
The Reticulum License is a custom, non-OSI license that includes an AI-training restriction (KB: `projects/reticulum.md`,
`protocols/reticulum-protocol.md`). 111network must therefore never be mistaken for a derivative work of Reticulum. Evidence in the tree:
no crate depends on Reticulum; `node111` only writes/reads rnsd's INI config and spawns the `rnsd` executable
(`1-bootstrap/crates/node111/src/daemon/transport/mod.rs`, `daemon/devices/rns.rs`); D13 rejected Reticulum code adoption. Since 2026-10-19 the one file of ours that imports `RNS` and `LXMF` is the
helper's Python source (`6-transport/sidecars/lxmf-sidecar/`, not a crate, run in its own venv); it carries none of their bytes.

## 2. Our own code

| Tree | What | Notes |
|---|---|---|
| `1-bootstrap/` | crate `node111` (binary `111`: installer, `111d` daemon, updater), POSIX install shim, systemd units, Termux scripts, release scripts | links `id111` |
| `2-identity/` | crate `id111`, frozen test vectors, `monocypher-harness/harness.c` | harness is a host-side test, not shipped |
| `3-intelligence/` | crate `agent111` (binary `111-agent`) | |
| `4-interface/` | dashboard UI (`ui/`, React + TypeScript), HTTP contract `api/openapi.yaml`, generated types `types.gen.ts` | generated from our contract by a dev tool; output is ours |
| `0-knowledge/` | KB entries and generator `_meta/build.py` | our text; names upstream licenses, reproduces no upstream text |
| `6-transport/` | spec, task board, verification scripts, and `sidecars/lxmf-sidecar/` (the LXMF helper: Python, imports user-installed `rns` and `lxmf`) | the Rust parts of Layer 6 live in `1-bootstrap/` and `5-messaging/`; the helper's own licence is pending (section 3, section 9 item 16) |
| `plan/`, READMEs | specs and decision logs | |

`website/` is outside `os/` and is not covered here.

**License declarations in manifests (changed 2026-10-19).** Until 2026-10-14 the Cargo manifests declared `license = "MIT OR Apache-2.0"`; from
2026-10-14 to 2026-10-19 they pointed at the source-available draft through `license-file`. Since 2026-10-19 `node111`, `id111`, `agent111` and `mesh111`
declare `license = "GPL-3.0-or-later"`, and `4-interface/ui/package.json` sets `"license": "GPL-3.0-or-later"`. `cargo metadata` parses in all four workspaces.

## 3. Connected services (separate programs; not linked, not vendored)

| Component | Upstream | License | How we use it | Obligations and notes |
|---|---|---|---|---|
| **Reticulum** (`rns`: `rnsd`, `rnodeconf`, and its pip closure) | github.com/markqvist/Reticulum | `LicenseRef-Reticulum`: custom, non-OSI since April 2025; adds a no-harm clause and bans use, directly or indirectly, in AI training datasets (KB). Exact text is not in the repo **(unverified — read upstream LICENSE)** | Installed unmodified from PyPI into a venv by the user's own installer (`pip install --require-hashes -r rns-lock.txt`). `111d` runs `rnsd` as a supervised child and drives it through its config file and its own interfaces. Our identity is independent of rns identity | Section 8. We ship only a requirements file with hashes, never Reticulum code. Policy: latest upstream release (see open item 3). *Verified 2026-10-19 on PyPI:* `rns` 1.5.7 (2026-10-05) and `lxmf` 1.2.1 (2026-10-13), both `Reticulum License`; the GitHub repository is a lagging mirror, so the pin resolves from PyPI |
| **Meshtastic** (`meshtasticd`, firmware) | github.com/meshtastic/firmware | GPL-3.0-only (KB `projects/meshtastic.md`) | Never installed by us. Detection only: a LISTEN check on its local port. *Amended 2026-10-19:* the bridge is built in Layer 6 on the `meshtastic` crate (section 5); `meshtasticd -s` is a test fixture only (row below) | User installs it themselves. Nothing of it is in our releases |
| **meshmud** (Meshtastic multiplexer: one serial or TCP uplink, many TCP downlinks) | github.com/Rikonardo/meshmud, pinned tag `v1.0.0` (commit and tarball sha256 in `1-bootstrap/scripts/meshmud-build.sh`) | Own code `MIT` (Copyright (c) 2026 Kai Shimizu, upstream `LICENSE`). Go closure: 14 modules, all MIT / BSD-2-Clause / BSD-3-Clause, plus the Go standard library (BSD-3-Clause), listed in `THIRD-PARTY-NOTICES.md` | Built **in CI** from the pinned upstream tarball into static linux/amd64 and linux/arm64 binaries (`meshmud-build.sh`; Go is a CI-only toolchain, nothing builds or downloads on a node), pinned by hash in the channel manifest. A supervised child of `111d`: TCP downlink on `127.0.0.1:11230` only, no HTTP downlink, `advertise = false`. Never copied into `os/` | Own code MIT; Go closure of 14 permissive modules; **no Meshtastic protobuf module at v1.0.0** (`go.mod` lists none and `internal/common/protobuf` is a hand-written wire parser), so the binary is **conveyed under MIT with its notices**. `meshmud-build.sh` re-checks the built binary (module list plus a `*.pb.go` / import scan) and writes a verdict file; only if that scan ever finds a Meshtastic protobuf module is the binary conveyed under GPL-3.0 with the source offer in `THIRD-PARTY-NOTICES.md` (D20 b's conservative premise, corrected at build review). A courtesy source pointer is listed either way. Ship `LICENSE.meshmud` and the Go closure notices beside the binary. Section 9 item 12 |
| **meshtastic-python** (`meshtastic` and its pip closure) with our reader `meshtastic-presence` | github.com/meshtastic/python, PyPI `meshtastic` 2.7.11. Reader: `5-messaging/sidecars/meshtastic-presence/` (ours) | `meshtastic` `GPL-3.0-only`. Closure of 15 more locked packages: MIT, BSD-2/3-Clause, Apache-2.0, PSF-2.0 (`typing-extensions`), MPL-2.0 (`certifi`, unmodified), one `Apache-2.0 OR BSD-2-Clause` (`packaging`). The reader is `GPL-3.0-or-later` (license file and SPDX headers in its directory), because it imports `meshtastic` | A separate supervised process in its own venv (`/var/lib/111/venvs/presence`), built on the node by `pip install --require-hashes -r meshtastic-presence-lock.txt` (every published file of each pinned version hashed). We ship the lock and the reader's source, never the packages. The reader only reads (`want_config` once, then events) and writes a JSON file that `111d` validates; no Rust code imports it | GPL source offer names `meshtastic` 2.7.11 and the lock (in `THIRD-PARTY-NOTICES.md`). The reader's `LICENSE` stays beside its source. The BLE stack (`bleak`, `dbus-fast`) comes with `meshtastic`'s required dependencies and is never used by the reader. Reproduce the lock with `gen_lock.py` in the reader's directory |
| **Meshtastic web client** (static bundle, hosted as a panel) | github.com/meshtastic/web, pinned tag `v2.7.2` (commit and hashes in `1-bootstrap/scripts/meshtastic-web-build.sh`) | `GPL-3.0-only` (upstream `LICENSE` and `package.json`). It bundles third-party JavaScript (React, Radix UI, maplibre-gl, sqlite-wasm and more, mostly MIT / ISC / BSD / Apache-2.0) under their own licenses | Served **unmodified** from its own directory by the panel listener (`[license] served = "unmodified"`), never `rust-embed`ded into `111`. Built in CI by `meshtastic-web-build.sh`: the upstream release asset `build.tar` when complete, otherwise a build of the pinned tag. For v2.7.2 the asset is **incomplete** (it leaves out the sqlite `.wasm` and the Inter `.woff2` fonts), so the bundle is built from the tag; that build is byte-identical to the asset's files and its tree hash is pinned. The browser contacts `tiles.openfreemap.org` and `api.meshtastic.org` only when online (declared `optional_hosts`, shown to the user) | Source offer names the tag and commit (`THIRD-PARTY-NOTICES.md`); ship the GPL-3.0 text beside the bundle (`LICENSE.meshtastic-web`). "Meshtastic" is a registered mark (spec section 14), used nominatively. Whether the bundled JavaScript's own notices must be reproduced is open: section 9 item 13 |
| **MeshChatX** (parked, P7; **not adopted**) | upstream as named in `plan/layers/5-messaging/options.md` (a fork of MeshChat, v4.10.7 2026-10-11 there) **(unverified — check upstream)** | `0BSD` plus `MIT` per `plan/layers/5-messaging/options.md` and `plan/cross-layer/parked.md` **(unverified — check upstream)** | It would run in its own venv with `rns` installed from PyPI and **import Reticulum in-process, inside its own program**. That makes it a separate adopted program (same boundary as `rnsd`): Reticulum's License would reach that program, never our Rust binaries. Nothing of it is in `os/` or in a release | Needs its own row and a read of its license and of the Reticulum License before any panel for it ships. Linking Reticulum into `111` stays banned (section 1). *Amended 2026-10-19:* no longer the LXMF carrier (that is the helper, row below); it may return as an optional hosted panel (`6-transport/README.md` section 17), and its upstream README announces a sunset on 2027-01-01 |
| **llama.cpp** (`llama-server`) | github.com/ggml-org/llama.cpp | MIT (KB `projects/llama-cpp.md`) | Pinned binary, supervised child of `111d`, HTTP on `127.0.0.1:11181` | Not in the release manifest yet (deferred). When shipped, its MIT notice must accompany the binary |
| **opencode** | github.com/anomalyco/opencode | MIT (KB `projects/opencode.md`) | Hub tiers only. `111-agent` runs it as a subprocess (headless JSONL), one local provider | Not in the manifest yet. v1.18.18 fetched its provider SDK `@ai-sdk/openai-compatible` from the network on first use (`3-intelligence/README.md`); that package and any other it pulls have licenses **(unverified — check upstream)** |
| **Termux, Termux:Boot, Termux:API** (v0.118.3 / v0.8.1 / v0.53.0 APKs) | github.com/termux | believed GPL-3.0 **(unverified — check upstream; not recorded anywhere in the repo)** | Pinned APKs from the signed manifest, installed unmodified over adb | We redistribute these APKs in our GitHub Release. GPL conveyance needs the license text plus a source offer or link to the matching upstream tag. Do not modify |
| **Termux `.deb` closure** (openssh, termux-services, cronie, python, python-cryptography and dependencies; list final after spike V3) | Termux package repo | various, per package **(unverified — check upstream)** | Pinned debs in the release `debs/` directory, installed on the phone | Each package keeps its own license. GPL members carry the same conveyance duties as the APKs |
| **rnodeconf and RNode firmware** | github.com/markqvist/RNode_Firmware | firmware GPL-3.0: *verified 2026-10-19* from upstream's `LICENSE` (the GPL-3.0 text) and README ("GNU General Public License v3.0", an MIT SX1276 driver inside; no "or later" wording seen); `rnodeconf` is inside the `rns` wheel, so the Reticulum License applies to it | `rnodeconf` runs as a separate process; `rnodeconf --info` is the adopted identify probe (Layer 6), and an offline install seeds rnodeconf's own update cache (KB `projects/rnode-firmware.md`). Flashing is human-confirmed. Firmware images pinned in the signed manifest are planned (D17 #9). **None are in the manifest today** | Same boundary as Reticulum. GPL-3.0 normally allows redistributing unmodified binaries with license text and corresponding source, but the repo has no copy of upstream's license text, so this needs a **legal read** before the first image ships. Fetching from upstream at flash time (rnodeconf default) avoids redistribution but breaks offline nodes |
| **adb / Android platform-tools** | Android SDK | distro `adb` package preferred; fallback is a sha256-pinned Google platform-tools zip. License and SDK terms **(unverified — check upstream)** | Run on the provisioning host to install the phone | Confirm redistribution terms before hosting the zip ourselves; today the manifest slot exists but is optional |
| **Host packages** (python3, python3-venv, iw, ca-certificates, systemd) | distro | distro packages | Installed or cached by the user's package manager on their machine | Not redistributed by us |
| **`lxmf-sidecar`** (our LXMF helper) with the user-installed `rns` and `lxmf` | Helper: `6-transport/sidecars/lxmf-sidecar/` (ours). Packages: PyPI `rns` 1.5.7, `lxmf` 1.2.1 (pins taken from PyPI at release time) | Helper: **pending the owner's contact with upstream** (how it sits with the Reticulum License is a legal read, `6-transport/README.md` section 15). Packages: `LicenseRef-Reticulum` (above) | A separate process in its own venv (`venvs/lxmf`), spawned only while the Reticulum bridge is enabled, never combined with `111d`. It imports the **user-installed** `rns` and `lxmf` (`pip install --require-hashes`, closure cached at install time, no bytes of ours or theirs in the release) and speaks JSON Lines over piped stdio | Credit in its header, README, `THIRD-PARTY-NOTICES.md` and Settings: "Reticulum and LXMF are the work of the Reticulum project (reticulum.network); 111 uses them as released and is not affiliated." It ships behind the enable gate with the notice (section 8). Its lock must agree with `rns-lock.txt`. Section 9 item 16 |
| **Madmail** (Rust chatmail relay, optional component) | github.com/themadorg/madmail, **pinned v2.30.5** (tag commit `22db4602768364955670b0f08dd97f03db6e22cc`) in `1-bootstrap/crates/node111/assets/components/madmail.toml`: `madmail-linux-amd64` (77 331 312 B, sha256 `ad89fb23…b989d1`) and `madmail-linux-arm64` (73 479 752 B, sha256 `27e6e74d…f20b5`); no armv7 or Android build | `AGPL-3.0-or-later`: *read 2026-10-19* from the upstream `LICENCE` (GNU AGPL version 3 text), README and `Cargo.toml`; the notice points at `LICENCE` at the v2.30.5 tag. *Owner review of the final pin pending* | Separate process, never in a 111 payload, IP-only self-signed mode on the hub. Not pre-downloaded by `111 install` (`[install] prefetch = false`, the Raspberry Pi floor): `sudo 111 install --components deltachat,madmail` downloads the pinned file hash-checked into `cache/packages/madmail/` after its notice; enable installs from that cache and never downloads | AGPL section 13: users who interact with it over the network are offered the corresponding source; the source offer names the pinned release. Needs a glibc 2.38 host. Standard ports need `CAP_NET_BIND_SERVICE` (it runs without it on high ports). Section 9 item 18 |
| **`deltachat-rpc-server`** (chatmail core) | github.com/chatmail/core, **pinned v2.63.0** (tag commit `de6f46971002c64b293d6095e66d8a1b7a11fd43`) in `assets/components/deltachat.toml`: `deltachat-rpc-server-x86_64-linux` (26 811 824 B, sha256 `57cab329…40d517`), `-aarch64-linux` (21 009 952 B, `040b0c36…bdbd13`), `-arm64-v8a-android` (21 897 944 B, `3e2d0c93…9d658c`); no armv7 build | `MPL-2.0` (upstream `LICENSE`, read 2026-10-19; the notice points at it at the v2.63.0 tag). *Owner review of the final pin pending* | Separate process, JSON-RPC over stdio, run unmodified. Not pre-downloaded by `111 install` (`prefetch = false`); fetched hash-checked by `sudo 111 install --components deltachat,madmail`, installed from the cache on enable | MPL-2.0 file-level terms follow the binary: keep the licence text and point to the pinned tag's source. Compatible with GPL-3.0 by its section 3.3 either way. Section 9 item 18 |
| **SimpleX Chat CLI and `smp-server`** (optional, built after Delta Chat) | github.com/simplex-chat/simplex-chat, github.com/simplex-chat/simplexmq; **no pin in this release** (`assets/components/simplex.toml` is `kind = "none"`: declared, nothing pinned, cached or downloaded; enabling refuses) | `AGPL-3.0` (upstream `LICENSE` and README, read 2026-10-19). The SimpleX name, logo and graphic assets are outside the licence (separate TRADEMARK and ASSETS_LICENSE files) | When the owner selects it: separate processes, per-target pins fetched hash-checked into the install-time cache, glibc hosts only (refused on Termux) | AGPL section 13 source offer naming the pinned tags; the name and logo are not reused. Section 9 item 18 |
| **Briar headless** (`briar-headless`) | github.com/briar/briar | `GPL-3.0-or-later` (upstream `LICENSE.txt` notice, read 2026-10-19; the KB entry is updated to match) | **Documented, not built.** It registers only the Tor plugin on Linux, so it is online-only | If ever built it is a separate process under the same duties as the other GPL programs (section 7). Nothing of it is in a release |
| **Test fixtures:** `meshtasticd` (`-s` simulator), **Meshtasticator**, **Sideband**, **Columba** | github.com/meshtastic/firmware; github.com/meshtastic/Meshtasticator; github.com/markqvist/Sideband; github.com/torlando-tech/columba | `meshtasticd` GPL-3.0; Meshtasticator `CC-BY-4.0` (its README, read 2026-10-19; no licence file on the default branch); Sideband `CC BY-NC-SA 4.0`; Columba `MPL-2.0` | Test tools only. `meshtasticd` and Meshtasticator run in tests on a developer or CI machine and are **never shipped**. Sideband is **never shipped** and fails the KB's OSI gate. **Columba is linked from the dashboard** (a link-out, spec section 7), no APK is shipped | Meshtasticator's licence is a content licence, which is acceptable for a tool that stays out of releases. A Columba link needs no licence action |

The Layer 5 libraries that **are** linked into `111` (`rusqlite` / `libsqlite3-sys` with SQLCipher and OpenSSL, `blake3`, `bao-tree`, `ciborium`) and the dashboard's `jsQR` are not
separate programs, so they are in section 5, not in this table. `meshtasticd` and the Meshtastic firmware are still never installed by us (row above).

Candidates named in `plan/cross-layer/devices.md` but **not adopted**: Columba (MPL-2.0), Sideband (CC BY-NC-SA 4.0, fails the KB's OSI gate), Dire Wolf (GPL-2.0),
Briar (`GPL-3.0-or-later`, updated 2026-10-19; headless is in the table above), MeshCore (MIT: the 2026-10-14 KB pass read the firmware `license.txt`, which closes the GPL-3.0 vs MIT conflict in `plan/layers/2-identity/options.md`), ble-reticulum (MIT, a Linux BLE plugin for `rnsd`, reference only), Leviculum (AGPL-3.0-or-later, a Reticulum reimplementation that is not upstream, reference only; Codeberg was not reachable for a re-read on 2026-10-19). Any of
these would be a new connected service and gets its own row here before it ships.

## 4. Vendored third-party code

| Item | Version | License | Where | Notes |
|---|---|---|---|---|
| **Monocypher** (`monocypher.c/.h`, `optional/monocypher-ed25519.c/.h`) | 4.0.3 (tarball sha256 `8cc9bc34...8ee66`) | `BSD-2-Clause OR CC0-1.0`, dual (vendored `LICENCE.md`) | `2-identity/monocypher-harness/vendor/` | Test harness only: it proves the C code agrees with Rust `id111`. Not linked into `111`, `111-agent` or the dashboard. Copyright lines: Loup Vaillant 2017-2023, Michael Savage 2017-2019, Fabio Scotoni 2017-2023. `LICENCE.md` travels with the sources; files are hash-pinned (`SHA256SUMS`) and never edited in place |

Nothing else is vendored. The only other C files in `os/` are our `harness.c`.
Layer 5 keeps that true: `meshmud` and the Meshtastic web client are **built in CI from pinned upstream sources** (tag, commit and tarball or asset sha256 in their build scripts) and their source is never copied into `os/`.
The upstream-derived files that are committed are data, not source: the pinned tag's `go.mod` and `go.sum` (`1-bootstrap/scripts/notices/meshmud-<tag>.go.{mod,sum}`, compared byte for byte with the tarball by `meshmud-build.sh`),
our own hand-written license lists beside them, and the unmodified GPL-3.0 text in the presence reader's `LICENSE`. The reader's source is ours. Test vectors reproduce short published values (RFC 8032, RFC 7748,
SLIP-0010) as data. `2-identity/test-vectors/crosscheck.py` is a developer script that imports the Python `cryptography` package; it is not shipped
and that package's license is **(unverified — check upstream)**.

## 5. Linked libraries

**Rust crates** (license field from `cargo metadata`/`cargo tree`, Linux musl targets x86_64 and aarch64, normal dependencies, our own crates excluded).
The count includes compile-time proc-macro crates. The full `Cargo.lock` graph, with other platforms and dev/build dependencies, is larger.

| License family | `111` (bootstrap, includes `id111`) | `111-agent` |
|---|---|---|
| `MIT OR Apache-2.0` (all spellings) | 121 | 39 |
| MIT only | 40 | 19 |
| Apache-2.0 only (`sync_wrapper`) | 1 | 1 |
| Multi-license with another permissive option (Unlicense, Zlib, 0BSD, BSD-3-Clause, ISC, BSL-1.0) | 10 | 2 |
| Single BSD-3-Clause / ISC / CC0-1.0 | 9 | 0 |
| Conjunctive, all terms apply: `ring` (Apache-2.0 AND ISC), `matchit` (MIT AND BSD-3-Clause), `unicode-ident` ((MIT OR Apache-2.0) AND Unicode-3.0) | 3 | 2 |
| CDLA-Permissive-2.0 data: `webpki-roots` | 1 | 0 |
| **Total** | **185** | **63** |

187 distinct crate versions across both binaries. Full lock graphs (all platforms, dev and build): bootstrap 255, identity 64, intelligence 108.

- **Copyleft:** none linked in the lock graphs scanned on 2026-10-14. *Layer 6 changes this when task B3 lands:* the `meshtastic` crate is GPL-3.0 (table below). `r-efi 6.0.0` offers `LGPL-2.1-or-later` as a third alternative to MIT and Apache-2.0. It appears only in the bootstrap and
  intelligence lock graphs, not in the Linux builds. We take the MIT/Apache option if it ever links.
- **Single BSD-3/CC0/ISC crates:** `ed25519-dalek`, `curve25519-dalek`, `x25519-dalek`, `subtle` (BSD-3-Clause); `bip39`, `bitcoin_hashes`, `hex-conservative`
  (CC0-1.0); `rustls-webpki`, `untrusted` (ISC).
- **`ring`:** its `LICENSE` points to separate ISC, BoringSSL (Apache-2.0 and ISC parts) and `once_cell` (Apache-2.0/MIT) license files (read locally in the cargo registry).
- **`webpki-roots`:** embeds Mozilla's CA roots as data under CDLA-Permissive-2.0. Sharing the data requires the agreement text to accompany it (clause 2.1).
- `mdns-sd` and `qrcode` (added for Layer 4) are Apache-2.0/MIT and MIT/Apache-2.0 (`1-bootstrap/TASKS.md`).

**JavaScript** (`4-interface/ui`, `pnpm licenses list --json`). What reaches the dashboard bundle is the production tree:

| Family | Production (bundled) | Full tree incl. dev tooling (146 versions) |
|---|---|---|
| MIT | 5 (`react`, `react-dom` 19.3.0, `scheduler`, `use-sync-external-store`, `regexparam`) | 125 |
| Unlicense | 1 (`wouter` 3.13.0, per its `package.json`) | 1 |
| ISC / BSD-3-Clause / Apache-2.0 | 0 | 9 / 1 / 5 |
| Not MIT/ISC/BSD/Apache, all dev-only | 0 | `lightningcss` + `lightningcss-linux-x64-gnu` (MPL-2.0, Tailwind's build tool), `argparse` (Python-2.0), `caniuse-lite` (CC-BY-4.0 data), `type-fest` (MIT OR CC0-1.0) |

The dev-only tools are not shipped, so their terms do not attach to our output. The built `dist/` carries no license banners for React or the others (no `@license` comments),
so the MIT notices are not in the shipped dashboard today (see section 8).

**Layer 5 additions (linked; they enter the `111` binary when `mesh111` is wired into `111d`, task F2).** Licenses read from the crates in the local cargo registry and from upstream; the
Rust ones then appear in `THIRD-PARTY-NOTICES.md` by themselves (the generator reads `1-bootstrap/Cargo.lock`, and notes the bundled native code), and the counts above change.

| Component | Version in `5-messaging/Cargo.lock` | License | Notes |
|---|---|---|---|
| `rusqlite` | 0.40.2 | MIT | Store engine (spec section 10) |
| `libsqlite3-sys` | 0.38.2 | MIT | Compiles the bundled sources: **SQLCipher** (BSD-3-Clause style, Copyright (c) 2008-2020 Zetetic LLC, crate file `sqlcipher/LICENSE`) on top of SQLite (public domain) |
| OpenSSL 3.x, through `openssl-src` | 300.6.1+3.6.3 (`openssl-sys` 0.9.117) | OpenSSL 3.x is Apache-2.0; the `openssl-src` crate is `MIT OR Apache-2.0` | Vendored and linked statically so the musl binary has no dynamic libcrypto (`bundled-sqlcipher-vendored-openssl`) |
| `blake3` | 1.8.7 | `CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception` | Content addressing; the notices reproduce the Apache-2.0 option |
| `bao-tree` | 0.16.1 | `MIT OR Apache-2.0` | Verified streaming of bulk objects |
| `ciborium` | 0.2.2 | Apache-2.0 | Deterministic CBOR message bodies |
| `jsQR` (JavaScript, dashboard) | added by task U1 | Apache-2.0 per spec section 2 **(read the package license when U1 adds it)** | QR decoding for contact cards; bundled into the dashboard, so its notice joins the JS list |

None is copyleft, so the section 7 rule (no GPL, LGPL, AGPL or SSPL linked) still holds for Layer 5.

**Layer 6 additions (linked; they enter the `111` binary when task B3 wires the Meshtastic client into `111d`; D25 F2 and D27 pre-approve the crate).** Licenses from crates.io and the upstream `LICENSE` read on 2026-10-19; exact versions and the closure are recorded by B3 against the real lock, and `THIRD-PARTY-NOTICES.md` then lists them by itself.

| Component | Version | License | Notes |
|---|---|---|---|
| `meshtastic` | 0.1.9 (crates.io, 2026-05-25) | `GPL-3.0` as declared (crates.io; the repository `LICENSE` is the GPL-3.0 text) | The official Meshtastic client crate: framing, handshake, heartbeat. **Copyleft, linked.** Features to trim: serial and BLE, if the build proves it (otherwise `serialport` and `btleplug` link in) |
| the crate's normal closure | 49 crates by the research measurement: 31 `MIT OR Apache-2.0`, the GPL-3.0 crate itself, and `serialport` 4.10.1 (`MPL-2.0`, only if the serial feature stays) | permissive, plus one MPL-2.0 | `prost` 0.14 (Apache-2.0), `prost-derive`, `bytes` (MIT) are in it; B3 records the exact list and the binary-size delta (about +372 KiB x86_64, +332 KiB aarch64 measured) |
| Meshtastic protobuf snapshot (`github.com/meshtastic/protobufs`, one pinned commit) | commit hash and checksum recorded beside the notices | `GPL-3.0` text in `LICENSE`, **with no version notice** ("only" or "or later" is not stated) | The generated code and the KB region table come from this one snapshot. At pin time we confirm whether the crates.io package ships `.proto` files or generated Rust. Section 9 item 17 |

**Effect (D25 F2):** the shipped `111d` binary is **effectively `GPL-3.0-only`**, because it links GPL-3.0 code whose licence text names no later version. Our own files stay `GPL-3.0-or-later`; the combined binary is conveyed under the narrower terms. This does not conflict with our license (section 7).

**Fonts** (`4-interface/FONTS.md`, self-hosted woff2, byte-for-byte copies; four files): Geist Mono and Geist Pixel Line (SIL OFL 1.1, Copyright 2023 Vercel with
basement.studio) and Silkscreen 400 and 700 (SIL OFL 1.1, Copyright 2001 The Silkscreen Project Authors). OFL texts ship next to the files and in `dist/fonts/`.

## 6. AI models (GGUF weights)

| Model | License | Source of the claim | Notes |
|---|---|---|---|
| Qwen3 4B, Q4_K_M GGUF (standard tier) | Apache-2.0 | KB `models/qwen3-4b.md` (upstream huggingface.co/Qwen) | Not in the manifest yet (G4: no model section defined). The repo does not name which party produced the GGUF file **(unverified — check upstream)**. Redistribution needs the license text and any NOTICE, plus a note that the weights were quantized |
| Qwen3 8B / 32B / 30B-A3B (hub ladder) | the KB says only that they are the same family | none in the KB | **(unverified — check upstream)** per size before any is pinned |

## 7. Compatibility with our own license

Our own code is `GPL-3.0-or-later` (`LICENSE.md`). Two things follow for dependencies:

1. **Everything linked into `111` or `111-agent`, or bundled into the dashboard, must be GPL-3.0-compatible**: released under terms that let the combined work be
   conveyed under GPL-3.0. Compatible: the permissive licenses (MIT, BSD-2/3-Clause, ISC, Zlib, Unlicense, CC0, Apache-2.0, Unicode-3.0, BSL-1.0, CDLA-Permissive-2.0 for data),
   MPL-2.0 (through its section 3.3 secondary-license clause), LGPL, and GPL-3.0 itself. **Incompatible, so a BLOCKER:** proprietary, source-available or field-of-use-restricted
   terms (SSPL, BUSL, PolyForm, Commons Clause, the Reticulum License, CC-BY-NC), GPL-2.0-only, EPL-1.0, CDDL, OSL, CC-BY-SA, and the original 4-clause BSD.
2. **The copyleft is ours to honour.** Every conveyed binary of our own code must come with its Corresponding Source or a GPL section 6 offer (section 8).

**Scan, 2026-10-14; re-read against rule 1 on 2026-10-19.** Every license field of all packages in the three Cargo lock graphs (255 + 64 + 108, all platforms, dev and build included) and all 146 JS
package versions, matched for GPL, LGPL, AGPL, SSPL, MPL, EPL, CDDL, EUPL, OSL, CC-BY-SA, BUSL, Elastic, NC and custom or missing licenses.

**Result: zero BLOCKERs.** Every crate linked into `111` or `111-agent` and every package bundled into the dashboard is under a GPL-3.0-compatible license (section 5). Hits that needed a second look:

| Item | License | Why it is not a blocker | If it ever changes |
|---|---|---|---|
| Apache-2.0 crates (`sync_wrapper`, `ciborium`, `ring`'s BoringSSL parts, OpenSSL 3.x through `openssl-src`) | Apache-2.0 | Compatible with GPL-3.0 (it is not compatible with GPL-2.0: one reason our license is version 3 "or later", never 2) | Keep "version 3 or later" |
| `r-efi` 6.0.0 | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | All three options are compatible. Not in the Linux musl dependency trees anyway (`cargo tree -i` finds nothing; UEFI only) | n/a |
| `lightningcss` 1.32.0 and its Linux binary | MPL-2.0 (file-level copyleft) | Dev-only build tool under Tailwind and Vite, not in `dist/`; MPL-2.0 is GPL-compatible in any case | n/a |
| `caniuse-lite`, `argparse` | CC-BY-4.0 (data), Python-2.0 | Dev-only, never shipped | n/a |
| Fonts | SIL OFL 1.1 | OFL is not itself GPL-compatible, but the font files are data placed beside the program, not part of it, and OFL allows bundling with software under any license. The font files stay OFL and may not be sold on their own | Keep the OFL texts beside the files |
| `webpki-roots` | CDLA-Permissive-2.0 (data) | Permissive; the agreement text must accompany the data (clause 2.1) | Keep the text in the notices |
| `ring`, `libc` and the static musl build | native C and asm: ISC, BoringSSL parts; musl libc (MIT) and compiler runtime: license not recorded in the repo | `ring`'s C and asm are permissive; musl is MIT. Whether every runtime library the static link pulls in is permissive or carries a runtime exception is not checked by anything today **(unverified — check the link step)** | Confirm the link step |

**Rule for new dependencies:** any crate, package or vendored file whose license is not GPL-3.0-compatible (rule 1) is a **BLOCKER**. The fix is to replace it, or to
move it behind a process boundary (a separate program talked to over its own interfaces). No such check runs in `scripts/check.sh` today (section 9 item 11).

**Linked GPL-3.0 code (Layer 6, 2026-10-19).** The `meshtastic` crate and its protobuf types are GPL-3.0 and are *linked* into `111d` (section 5). GPL-3.0 code is compatible with `GPL-3.0-or-later` code (rule 1), so there is no BLOCKER, but the **shipped `111d` binary is effectively `GPL-3.0-only`**: the combined work can be conveyed only under terms every part allows, and the protobufs' `LICENSE` names no later version. Our own files stay "or later", so a build without the crate would again be "or later". Whether "or later" survives for the protobufs at all is a legal read (GPLv3 section 14; section 9 item 17). The Corresponding Source duty in section 8 already covers it.

**AGPL programs as separate processes.** Madmail and SimpleX (AGPL-3.0) run only as their own processes, fetched into the install-time cache (`111 install`; the ~104 MB Delta Chat + Madmail payloads are not pre-fetched by default and come later with `sudo 111 install --components deltachat,madmail`), never on enable and never linked: AGPL-3.0 and GPL-3.0 may be combined (GPLv3 section 13), but our design keeps them apart, and their network-use source offer (AGPL section 13) is theirs to meet and ours to name. `deltachat-rpc-server` (MPL-2.0) is also a separate process.

**GPL programs as separate processes.** `meshtasticd`, the Termux app and packages, the RNode firmware, the Meshtastic web client bundle, the `meshtastic-presence` reader with `meshtastic-python`, `meshmud`, and any later Dire Wolf or Briar run as
their own programs: we start them, talk to them over their own interfaces, redistribute them unmodified and next to our code, and never patch them. Since 2026-10-19 that boundary is a
**design decision** (D17, D19, D20: adopt upstream unmodified, never vendor, never fork), no longer something our license forces: their GPL-3.0 terms are the same as ours, so linking
one would be license-compatible. The redistribution duties are unchanged (license text, source offer, no modification; section 8).

**The Reticulum boundary is the one that stays license-forced.** The Reticulum License is neither open source nor GPL-compatible, so no Reticulum code may ever be incorporated, linked or
derived in our code (sections 1 and 8); the helper `lxmf-sidecar` is a separate process that imports the user-installed packages and is never combined with `111d` (D26 #4). How the helper's own licence sits with the Reticulum License waits on the owner's contact with upstream (section 9 item 16). It stays a separate, unmodified, latest-upstream program that the user installs from PyPI; the Reticulum License is an agreement between the user and upstream.

**Other policies do not conflict.**
- The KB rubric's OSI-license gate (`0-knowledge/README.md`) applies to *adopted components*, not to our own license. Our license is OSI-approved, so the project may describe itself as
  open source; the README says so (section 9 item 8).
- The Reticulum boundary is unchanged (above).

## 8. Obligations we actually have to meet

- [ ] **Third-party notices in every release.** `THIRD-PARTY-NOTICES.md` now exists: 187 Rust crates, 6 JS packages (5 MIT plus `wouter`), the 4 font files,
  Monocypher, the Layer 5 additions (14 Go modules of `meshmud`, 16 Python packages of the presence venv, the Meshtastic web client bundle, GPL source offers naming the pinned tags), and the license texts including the CDLA agreement for `webpki-roots` and the GPL-3.0 text. Generated by `1-bootstrap/scripts/gen-notices.py` and gated by `check.sh`. Still to do: ship it in the release payload as a manifest item and link it from the
  dashboard (Settings, Essentials, "Licenses").
- [ ] **Our own license (GPL-3.0-or-later):** ship `LICENSE.md` beside the third-party notices in every release. For every conveyed binary of our own code, publish the
  Corresponding Source of that exact release from the same place as the binaries (the tagged source at the release's commit; GPL-3.0 section 6(d)) and name the tag and
  commit in the release notes. Keep the copyright notice and the "no warranty" statement intact in anything we ship (GPL-3.0 sections 4 and 5). The release checklist records the source offer as a
  publish step (`1-bootstrap/docs/release-checklist.md` step 5, "Corresponding Source", D22: the tagged `git archive` beside every binary release, tag and commit in the release notes).
- [ ] **OFL fonts:** keep copyright and license text beside the font files, never sell the fonts alone, keep them unmodified (OFL clause 3 restricts reserved font names on modified versions). Met today.
- [ ] **Monocypher:** keep `LICENCE.md` with the vendored sources. If a C port ever ships in firmware, carry the BSD-2-Clause notice or elect CC0. Met today (test-only).
- [ ] **GPL components stay separate programs (design rule; see section 7).** Termux APKs and debs, RNode firmware if ever pinned, and `meshtasticd` are never linked, merged or patched. For
  anything GPL that we redistribute (APKs, debs, firmware): ship the license text, point to the exact upstream source for that version, and never modify it. Not yet written into the release process.
- [ ] **Reticulum:**
  - no Reticulum code in our tree, and no import, link or copy of its source, docs text or data;
  - our mesh core takes the public-domain protocol design only, clean-room;
  - never use Reticulum code, documentation or data to train or fine-tune any model, directly or indirectly (no on-node fine-tuning exists, D10);
  - the one exception is the helper `lxmf-sidecar` (D26 #4): it imports the **user-installed** packages at run time in its own process and carries none of their bytes; its header, README, `THIRD-PARTY-NOTICES.md` and the Settings copy credit Reticulum ("111 uses them as released and is not affiliated");
  - run the latest upstream release unmodified, and let the user install from upstream (PyPI) on their own machine;
  - keep this boundary in the README and release notes so nobody reads 111network as a derivative.
- [ ] **Layer 5 GPL components (Meshtastic web client, `meshtastic-python` and the reader, `meshmud` if it ever links GPL code).** Keep them separate processes or files; serve the web bundle unmodified; ship the GPL-3.0 text beside each
  (`LICENSE.meshtastic-web`, the reader's `LICENSE`); keep the source offer in `THIRD-PARTY-NOTICES.md` naming the pinned tags, commits and the lock. `THIRD-PARTY-NOTICES.md` already carries them; shipping it in the payload is the open step above.
- [ ] **meshmud:** ship `LICENSE.meshmud` and the Go closure notices beside the binaries; file the build's verdict file with the release; any change of pin means new snapshots and license rows first (`gen-notices.py --check` fails otherwise).
- [ ] **User-installs notice rule (D26 #5; `6-transport/README.md` sections 2.1 and 15).** Every third-party program a component needs (the helper's `rns` and `lxmf` closure, Madmail, `deltachat-rpc-server`, SimpleX) is downloaded hash-locked from its public source at install time (`111 install`; a payload the manifest marks `prefetch = false` comes later with `sudo 111 install --components <id>`; never at enable time, nothing downloads at runtime) and is **never in a 111 payload**. Before any download or enable, 111 shows a notice naming the program, its licence and where to read it, and records the user's acknowledgement (in `state/bridge.json` for enable; without it nothing is installed or started, `409 consent_required`). `111 install` prints the same one-line notice when it pre-downloads a payload and records program, licence and URL in the install receipt beside the artifact hashes, so the acknowledgement exists before any download; the Layer 1 `rns` venv gets the same line. The user installs; 111 and its author are not the distributor. **To do until task C3 lands:** `gen-notices.py --check` will hash the helper's lock and every component pin and list a row per package of the lxmf closure, and the release checklist asserts that the helper's `rns` pin equals `rns-lock.txt`.
- [ ] **AGPL components (Madmail, SimpleX).** Separate processes only. Read the licence file at the pinned release before the pin ships, name the pinned tag in the source offer (AGPL section 13 for network users), keep the licence text beside the downloaded binary, and do not reuse SimpleX branding. **MPL-2.0 (`deltachat-rpc-server`):** keep the licence text and point to the pinned tag.
- [ ] **Linked GPL-3.0 (the `meshtastic` crate and protobuf snapshot, Layer 6).** Ship the GPL-3.0 text (already in the notices), name the crate version and the snapshot commit in the source offer, and state in the release notes that the `111` binary is conveyed under GPL-3.0 (section 7). Sign-off rows in `1-bootstrap/TASKS.md` (Dependency policy, Layer 6 additions).
- [ ] **MIT components we ship later** (`llama-server`, `opencode`): include their notices next to the binaries.
- [ ] **Models:** when a GGUF is hosted by us, include the model license (and NOTICE if any) in the release.
- [ ] **Termux debs:** collect each package's license file when the closure is final (spike V3).

## 9. Open items for the owner

1. **Project license (done 2026-10-19).** Relicensed to `GPL-3.0-or-later` (owner decision, `plan/DECISIONS.md` D22); `LICENSE.md` holds the verbatim FSF text and the copyright notice
   ("111network contributors"). A standard, unchanged FSF license needs no lawyer's review of its own text; the legal reads that remain are the third-party ones (items 2, 4 and 14).
   Still to do: the release-time source offer (section 8) and, optionally, an SPDX header line in our own source files.
2. **Legal read: RNode firmware redistribution.** Needed before any firmware image is pinned in the manifest (section 3). Includes confirming upstream's current license text (read 2026-10-19: GPL-3.0 `LICENSE`; the "or later" question is not answered by the files).
3. **Latest-upstream rule (decided, 2026-10-14).** Each 111 release pins the *then-latest* upstream Reticulum release by hash, and the updater refreshes that pin on every 111 release.
   The hash protects integrity; it does not freeze the version. Nothing is vendored and nothing is patched, so `rns-lock.txt` stays a requirements file with hashes. Between 111 releases the
   pinned version does not move. To do: make the release checklist resolve the latest `rns` at release time (the `rns==1.5.6` in `1-bootstrap/README.md` is a placeholder for that; PyPI had RNS 1.5.7 on 2026-10-19, and the KB entry now says so), and have the updater treat a changed pin as a normal manifest refresh.
4. **Legal read: the AI-training clause.** The clause says "directly or indirectly". The agent reads `rnsd` logs and config as inference-time context (`logs.read`) and does not train. Whether that is outside the clause is not settled by anything in the repo.
5. **Reticulum License text.** Copy it into `0-knowledge/` or this file's references after reading upstream, and record the version of the text.
6. **Unverified rows in section 3 and 6:** Termux package licenses, platform-tools terms, opencode's runtime packages, the GGUF producer, larger Qwen3 sizes. (MeshCore's license is MIT, read from the firmware `license.txt` on 2026-10-14; the KB entry records it.)
7. **Notices pipeline (done 2026-10-14).** `1-bootstrap/scripts/gen-notices.py` (Python 3 standard library, offline, deterministic) regenerates `THIRD-PARTY-NOTICES.md` from `cargo metadata`/`cargo tree` for `bootstrap` and `intelligence`, `pnpm licenses`, the font files and Monocypher.
   `gen-notices.py --check` compares the lockfile SHA-256s recorded in that file with the current lockfiles and fails when they differ. It runs as the "notices" step in `1-bootstrap/scripts/check.sh`.
   Layer 5 adds a Go path (the pinned `meshmud` tag's `go.mod` / `go.sum` snapshot plus `1-bootstrap/scripts/notices/go-licenses.txt`) and a Python-venv path (the presence lock plus `python-licenses.txt`); `--check` also hashes those, the build-script pins, and fails when a Go module or locked package has no license row.
   Still to do: a release-checklist step that ships the file in the payload, the dashboard "Licenses" link, and the components not shipped yet (GPL-3.0 text and source offer for Termux APKs and debs, MIT notices for `llama-server` and `opencode`, the model license).
8. **README wording (updated 2026-10-19).** `os/README.md` now says "open-source operating layer (GPL-3.0-or-later: free for anyone, for any purpose, including commercial use)" and that contributions of any kind are appreciated. Its statements about usage policies still say the software imposes none *at runtime* (no telemetry, no content policy, no policing), and that the license terms are a separate matter.
9. **Manifests (updated 2026-10-19).** The four crates declare `license = "GPL-3.0-or-later"` and `package.json` sets the same SPDX id. `LicenseRef-111network` is no longer needed.
10. **Contributor terms (resolved by the license, 2026-10-19).** Contributions are accepted under `GPL-3.0-or-later`, the same terms the project is published under (inbound = outbound). No copyright assignment and no contributor license agreement; contributors keep their copyright. Any sort of contribution is appreciated. Optional later: a Developer Certificate of Origin sign-off line.
11. **Automate the compatibility gate.** Add a license allow-list check (for example `cargo deny` with the GPL-3.0-compatible set of section 7, and a JS license export) to `scripts/check.sh` so an incompatible dependency fails the build, and confirm which runtime libraries the static musl link pulls in.
12. **meshmud conveyance (D20 b says "conveyed as GPL-3.0, verified at build").** The v1.0.0 source shows no Meshtastic protobuf in the closure (own wire parser; `go.mod` lists none), so the binary looks like MIT plus permissive modules. `meshmud-build.sh` writes the verdict per build. Owner: keep the GPL-3.0 conveyance wording as the safe default, or record the MIT result once the CI verdict has been seen. The source offer is in the notices either way.
13. **Web bundle: upstream asset and third-party notices.** (a) Upstream's `build.tar` for v2.7.2 omits the sqlite `.wasm` and the Inter fonts, so we build from the tag (byte-identical where they overlap); re-check at each pin move, and tell upstream. (b) The bundle's minified third-party JavaScript carries no license banners; `meshtastic-web-build.sh --licenses FILE` lists the workspace's production packages (793 versions at v2.7.2, a superset of what is bundled). Decide whether to reproduce a trimmed list in `THIRD-PARTY-NOTICES.md`.
14. **GPL source-offer form.** `THIRD-PARTY-NOTICES.md` names the upstream tags and commits for the web bundle, `meshmud` (if GPL) and `meshtastic-python`. Whether pointing at upstream satisfies GPL-3.0 section 6 for files we host in our own release (as opposed to a written offer or a mirror of the source) needs the same legal read as items 2 and 4.
15. **Termux venv.** The presence venv's closure includes `bleak` and `dbus-fast` (a compiled package) because `meshtastic` requires them; pip's wheels may not match Android. This is an external gate (spec section 20), noted here because a source build there would pull build-time packages that the hash lock does not cover.
16. **The helper's licence (Layer 6, D26 #4).** `lxmf-sidecar` is ours and imports the user-installed `rns` and `lxmf`. Its own licence, and how it sits with the Reticulum License (non-OSI, with a no-harm clause and an AI-training clause), follow the owner's contact with upstream and a legal read (`6-transport/README.md` section 15). Until then it is unlicensed in the tree and ships only behind the enable gate with the notice (section 8).
17. **Legal read: the protobufs' notice (Layer 6, R06 G9).** The `meshtastic/protobufs` `LICENSE` holds the GPL-3.0 text with no version notice, so GPLv3 section 14 decides whether "or later" survives for the generated types linked into `111d`. Until the read, the shipped binary is treated as `GPL-3.0-only` (section 7). Also decide whether the `meshtastic` crate's own `GPL-3.0` declaration means "only".
18. **Which components ship as natives (Layer 6; the owner, after the build review).** Madmail, `deltachat-rpc-server` and SimpleX are optional, on-demand components downloaded by the user. Whether any of them is ever bundled in a release, or built in CI like `meshmud`, is the owner's call and needs the licence rows in section 3 re-read at that pin; today none is in a payload (D26 #5).
