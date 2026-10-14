# Third-party licensing and dependency boundaries

Status: draft of 2026-10-14, compiled from files in this repo and from locally readable package metadata
(`cargo metadata`, `cargo tree`, `pnpm licenses`). It is a statement of facts and boundaries, not legal advice.
Anything not verifiable from the repo or local metadata is marked **(unverified — check upstream)**.

**Our own license (owner decision, 2026-10-14):** free secure communication is everyone's right; free for personal, non-military and non-commercial use.
A **draft** is in `LICENSE.md` (PolyForm Noncommercial 1.0.0 as the base, plus a narrow no-military-use condition, commercial use by separate agreement).
It is marked "needs a lawyer's review before the first signed release", and its Required Notice names `https://111.network`. The official PolyForm text is pasted into Part A verbatim. Section 7 checks that our
dependencies allow this.

## 1. Purpose and the boundary principle

This file says which third-party code the 111network OS runs, how each piece relates to our code, and which obligations follow.
Three rules decide everything:

1. **Separate process = not a derivative.** A program we start and talk to over its own interfaces (command line, config file,
   socket, HTTP) stays a separate work under its own license. We do not become a derivative of it.
2. **Vendored = its license travels with it.** Source copied into this tree keeps its license text and notices.
3. **Linked = its license applies to the binary.** Every Rust crate compiled into `111` or `111-agent`, and every JS package bundled
   into the dashboard, puts its notice requirements on what we ship.

**Reticulum (owner decision, 2026-10-14; `plan/devices.md` section 7 item 12, `plan/parked.md` P5).** Reticulum is used **as-is and
separately**, as a connected service, like Meshtastic: a separate, unmodified upstream program, always the latest upstream release, started
and talked to by our daemon over its own interfaces. **No Reticulum code is incorporated into, linked into, or derived in our code.** The
Reticulum *protocol design* (dedicated to the public domain in 2016, per the KB) may inform our own mesh core, which is ours, in Rust.
The Reticulum License is a custom, non-OSI license that includes an AI-training restriction (KB: `projects/reticulum.md`,
`protocols/reticulum-protocol.md`). 111network must therefore never be mistaken for a derivative work of Reticulum. Evidence in the tree:
no crate depends on Reticulum; `node111` only writes/reads rnsd's INI config and spawns the `rnsd` executable
(`bootstrap/crates/node111/src/daemon/transport/mod.rs`, `daemon/devices/rns.rs`); D13 rejected Reticulum code adoption.

## 2. Our own code

| Tree | What | Notes |
|---|---|---|
| `bootstrap/` | crate `node111` (binary `111`: installer, `111d` daemon, updater), POSIX install shim, systemd units, Termux scripts, release scripts | links `id111` |
| `identity/` | crate `id111`, frozen test vectors, `monocypher-harness/harness.c` | harness is a host-side test, not shipped |
| `intelligence/` | crate `agent111` (binary `111-agent`) | |
| `interface/` | dashboard UI (`ui/`, React + TypeScript), HTTP contract `api/openapi.yaml`, generated types `types.gen.ts` | generated from our contract by a dev tool; output is ours |
| `knowledge/` | KB entries and generator `_meta/build.py` | our text; names upstream licenses, reproduces no upstream text |
| `plan/`, READMEs | specs and decision logs | |

`website/` is outside `os/` and is not covered here.

**License declarations in manifests (changed 2026-10-14).** The Cargo manifests used to declare `license = "MIT OR Apache-2.0"` (workspace
files in `bootstrap/` and `identity/`, and `agent111`). That contradicted the owner's decision and was removed. `node111`, `id111` and `agent111` now set
`license-file = "../../../LICENSE.md"` (the draft), and `interface/ui/package.json` sets `"license": "SEE LICENSE IN ../../LICENSE.md"`.
`cargo metadata` parses in all three workspaces.

## 3. Connected services (separate programs; not linked, not vendored)

| Component | Upstream | License | How we use it | Obligations and notes |
|---|---|---|---|---|
| **Reticulum** (`rns`: `rnsd`, `rnodeconf`, and its pip closure) | github.com/markqvist/Reticulum | `LicenseRef-Reticulum`: custom, non-OSI since April 2025; adds a no-harm clause and bans use, directly or indirectly, in AI training datasets (KB). Exact text is not in the repo **(unverified — read upstream LICENSE)** | Installed unmodified from PyPI into a venv by the user's own installer (`pip install --require-hashes -r rns-lock.txt`). `111d` runs `rnsd` as a supervised child and drives it through its config file and its own interfaces. Our identity is independent of rns identity | Section 8. We ship only a requirements file with hashes, never Reticulum code. Policy: latest upstream release (see open item 3) |
| **Meshtastic** (`meshtasticd`, firmware) | github.com/meshtastic/firmware | GPL-3.0-only (KB `projects/meshtastic.md`) | Never installed by us. Detection only: a LISTEN check on its local port. Bridge is parked (P1) | User installs it themselves. Nothing of it is in our releases |
| **llama.cpp** (`llama-server`) | github.com/ggml-org/llama.cpp | MIT (KB `projects/llama-cpp.md`) | Pinned binary, supervised child of `111d`, HTTP on `127.0.0.1:11181` | Not in the release manifest yet (deferred). When shipped, its MIT notice must accompany the binary |
| **opencode** | github.com/anomalyco/opencode | MIT (KB `projects/opencode.md`) | Hub tiers only. `111-agent` runs it as a subprocess (headless JSONL), one local provider | Not in the manifest yet. v1.18.18 fetched its provider SDK `@ai-sdk/openai-compatible` from the network on first use (`intelligence/README.md`); that package and any other it pulls have licenses **(unverified — check upstream)** |
| **Termux, Termux:Boot, Termux:API** (v0.118.3 / v0.8.1 / v0.53.0 APKs) | github.com/termux | believed GPL-3.0 **(unverified — check upstream; not recorded anywhere in the repo)** | Pinned APKs from the signed manifest, installed unmodified over adb | We redistribute these APKs in our GitHub Release. GPL conveyance needs the license text plus a source offer or link to the matching upstream tag. Do not modify |
| **Termux `.deb` closure** (openssh, termux-services, cronie, python, python-cryptography and dependencies; list final after spike V3) | Termux package repo | various, per package **(unverified — check upstream)** | Pinned debs in the release `debs/` directory, installed on the phone | Each package keeps its own license. GPL members carry the same conveyance duties as the APKs |
| **rnodeconf and RNode firmware** | github.com/markqvist/RNode_Firmware | firmware GPL-3.0 per `plan/devices.md` **(unverified against upstream's current LICENSE)**; `rnodeconf` is inside the `rns` wheel, so the Reticulum License applies to it | `rnodeconf` runs as a separate process. Flashing is human-confirmed. Firmware images pinned in the signed manifest are planned (D17 #9). **None are in the manifest today** | Same boundary as Reticulum. GPL-3.0 normally allows redistributing unmodified binaries with license text and corresponding source, but the repo has no copy of upstream's license, so this needs a **legal read** before the first image ships. Fetching from upstream at flash time (rnodeconf default) avoids redistribution but breaks offline nodes |
| **adb / Android platform-tools** | Android SDK | distro `adb` package preferred; fallback is a sha256-pinned Google platform-tools zip. License and SDK terms **(unverified — check upstream)** | Run on the provisioning host to install the phone | Confirm redistribution terms before hosting the zip ourselves; today the manifest slot exists but is optional |
| **Host packages** (python3, python3-venv, iw, ca-certificates, systemd) | distro | distro packages | Installed or cached by the user's package manager on their machine | Not redistributed by us |

Candidates named in `plan/devices.md` but **not adopted**: Columba (MPL-2.0), Sideband (CC BY-NC-SA 4.0, fails the KB's OSI gate), Dire Wolf (GPL-2.0),
Briar (GPL-3.0-only, KB case study), MeshCore (KB says MIT; `plan/layer-2-options.md` records a GPL-3.0 vs MIT conflict, **unverified**). Any of
these would be a new connected service and gets its own row here before it ships.

## 4. Vendored third-party code

| Item | Version | License | Where | Notes |
|---|---|---|---|---|
| **Monocypher** (`monocypher.c/.h`, `optional/monocypher-ed25519.c/.h`) | 4.0.3 (tarball sha256 `8cc9bc34...8ee66`) | `BSD-2-Clause OR CC0-1.0`, dual (vendored `LICENCE.md`) | `identity/monocypher-harness/vendor/` | Test harness only: it proves the C code agrees with Rust `id111`. Not linked into `111`, `111-agent` or the dashboard. Copyright lines: Loup Vaillant 2017-2023, Michael Savage 2017-2019, Fabio Scotoni 2017-2023. `LICENCE.md` travels with the sources; files are hash-pinned (`SHA256SUMS`) and never edited in place |

Nothing else is vendored. The only other C files in `os/` are our `harness.c`. Test vectors reproduce short published values (RFC 8032, RFC 7748,
SLIP-0010) as data. `identity/test-vectors/crosscheck.py` is a developer script that imports the Python `cryptography` package; it is not shipped
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

- **Copyleft:** none linked. `r-efi 6.0.0` offers `LGPL-2.1-or-later` as a third alternative to MIT and Apache-2.0. It appears only in the bootstrap and
  intelligence lock graphs, not in the Linux builds. We take the MIT/Apache option if it ever links.
- **Single BSD-3/CC0/ISC crates:** `ed25519-dalek`, `curve25519-dalek`, `x25519-dalek`, `subtle` (BSD-3-Clause); `bip39`, `bitcoin_hashes`, `hex-conservative`
  (CC0-1.0); `rustls-webpki`, `untrusted` (ISC).
- **`ring`:** its `LICENSE` points to separate ISC, BoringSSL (Apache-2.0 and ISC parts) and `once_cell` (Apache-2.0/MIT) license files (read locally in the cargo registry).
- **`webpki-roots`:** embeds Mozilla's CA roots as data under CDLA-Permissive-2.0. Sharing the data requires the agreement text to accompany it (clause 2.1).
- `mdns-sd` and `qrcode` (added for Layer 4) are Apache-2.0/MIT and MIT/Apache-2.0 (`bootstrap/TASKS.md`).

**JavaScript** (`interface/ui`, `pnpm licenses list --json`). What reaches the dashboard bundle is the production tree:

| Family | Production (bundled) | Full tree incl. dev tooling (146 versions) |
|---|---|---|
| MIT | 5 (`react`, `react-dom` 19.3.0, `scheduler`, `use-sync-external-store`, `regexparam`) | 125 |
| Unlicense | 1 (`wouter` 3.13.0, per its `package.json`) | 1 |
| ISC / BSD-3-Clause / Apache-2.0 | 0 | 9 / 1 / 5 |
| Not MIT/ISC/BSD/Apache, all dev-only | 0 | `lightningcss` + `lightningcss-linux-x64-gnu` (MPL-2.0, Tailwind's build tool), `argparse` (Python-2.0), `caniuse-lite` (CC-BY-4.0 data), `type-fest` (MIT OR CC0-1.0) |

The dev-only tools are not shipped, so their terms do not attach to our output. The built `dist/` carries no license banners for React or the others (no `@license` comments),
so the MIT notices are not in the shipped dashboard today (see section 8).

**Fonts** (`interface/FONTS.md`, self-hosted woff2, byte-for-byte copies; four files): Geist Mono and Geist Pixel Line (SIL OFL 1.1, Copyright 2023 Vercel with
basement.studio) and Silkscreen 400 and 700 (SIL OFL 1.1, Copyright 2001 The Silkscreen Project Authors). OFL texts ship next to the files and in `dist/fonts/`.

## 6. AI models (GGUF weights)

| Model | License | Source of the claim | Notes |
|---|---|---|---|
| Qwen3 4B, Q4_K_M GGUF (standard tier) | Apache-2.0 | KB `models/qwen3-4b.md` (upstream huggingface.co/Qwen) | Not in the manifest yet (G4: no model section defined). The repo does not name which party produced the GGUF file **(unverified — check upstream)**. Redistribution needs the license text and any NOTICE, plus a note that the weights were quantized |
| Qwen3 8B / 32B / 30B-A3B (hub ladder) | the KB says only that they are the same family | none in the KB | **(unverified — check upstream)** per size before any is pinned |

## 7. Compatibility with our own license

The draft `LICENSE.md` limits commercial and military use of our code. That works only if nothing **copyleft** (GPL, LGPL linked statically or
bundled, AGPL, SSPL) is linked into our binaries or bundled into the dashboard, because copyleft would force its own terms on the combined work.

**Scan, 2026-10-14.** Every license field of all packages in the three Cargo lock graphs (255 + 64 + 108, all platforms, dev and build included) and all 146 JS
package versions, matched for GPL, LGPL, AGPL, SSPL, MPL, EPL, CDDL, EUPL, OSL, CC-BY-SA, BUSL, Elastic, NC and custom or missing licenses.

**Result: zero BLOCKERs.** No copyleft crate is linked into `111` or `111-agent`, and no copyleft package is bundled into the dashboard. Hits that are not blockers:

| Item | License | Why it is not a blocker | If it ever changes |
|---|---|---|---|
| `r-efi` 6.0.0 | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | Not in the Linux musl dependency trees (`cargo tree -i` finds nothing). Pulled by `getrandom` for UEFI only. We take MIT or Apache-2.0 | Elect MIT or Apache-2.0 |
| `lightningcss` 1.32.0 and its Linux binary | MPL-2.0 (file-level copyleft) | Dev-only build tool under Tailwind and Vite. Not in `dist/` and never distributed by us | Do not ship it; keep it out of the payload |
| `caniuse-lite`, `argparse` | CC-BY-4.0 (data), Python-2.0 | Dev-only; attribution or permissive, not copyleft | n/a |
| Fonts | SIL OFL 1.1 | OFL allows bundling with software under any license. The font files stay OFL and may not be sold on their own | Keep the OFL texts beside the files |
| `ring`, `libc` and the static musl build | native C and asm: ISC, BoringSSL parts; musl libc and compiler runtime: license not recorded in the repo | `ring`'s C and asm are permissive. Whether any GPL or LGPL runtime library is linked into the static binaries is not checked by anything today **(unverified — check the link step)** | Confirm that only permissive or runtime-exception code is linked |

Every other dependency is MIT, Apache-2.0, BSD, ISC, CC0, Unlicense, Zlib, Unicode-3.0 or CDLA-Permissive-2.0 (section 5). These allow us to ship the combined work
under our own terms as long as their notices travel with it (section 8). **Rule for new dependencies:** any GPL, LGPL (static or bundled), AGPL or SSPL crate,
package or vendored file is a **BLOCKER**. The fix is to replace it, or to move it behind a process boundary. No such check runs in `scripts/check.sh` today.

**GPL programs as separate processes are fine, and must stay separate.** `meshtasticd`, the Termux app and packages, the RNode firmware and any later Dire Wolf or Briar run as
their own programs. We start them, talk to them over their own interfaces, redistribute them unmodified and next to our code, and never link, merge or patch them.
Distribution next to our code is mere aggregation under the usual GPL-3.0 terms, which the repo does not contain a copy of **(legal read to confirm)**.

**Other policies do not conflict.**
- The KB rubric's OSI-license gate (`knowledge/README.md`) applies to *adopted components*, not to our own license. A source-available license on our code does not
  affect it. It does mean the project must not describe itself as "open source"; the README was reworded accordingly (open item 8).
- The Reticulum boundary is unchanged: separate process, unmodified, latest upstream, no code incorporated. Our noncommercial and no-military terms do not relicense or
  restrict Reticulum, and the Reticulum License stays an agreement between the user and upstream.

## 8. Obligations we actually have to meet

- [ ] **Third-party notices in every release.** `THIRD-PARTY-NOTICES.md` now exists: 187 Rust crates, 6 JS packages (5 MIT plus `wouter`), the 4 font files,
  Monocypher, and the license texts including the CDLA agreement for `webpki-roots`. Still to do: ship it in the release payload as a manifest item, link it from the
  dashboard (Settings, Essentials, "Licenses"), and put its generator in the repo.
- [ ] **Our own license:** ship `LICENSE.md` (drafting notes removed, Required Notice URL confirmed) beside the third-party notices in every release, and keep the Required Notice line.
- [ ] **OFL fonts:** keep copyright and license text beside the font files, never sell the fonts alone, keep them unmodified (OFL clause 3 restricts reserved font names on modified versions). Met today.
- [ ] **Monocypher:** keep `LICENCE.md` with the vendored sources. If a C port ever ships in firmware, carry the BSD-2-Clause notice or elect CC0. Met today (test-only).
- [ ] **GPL components stay separate programs.** Termux APKs and debs, RNode firmware if ever pinned, and `meshtasticd` are never linked or merged into our code. For
  anything GPL that we redistribute (APKs, debs, firmware): ship the license text, point to the exact upstream source for that version, and never modify it. Not yet written into the release process.
- [ ] **Reticulum:**
  - no Reticulum code in our tree, and no import, link or copy of its source, docs text or data;
  - our mesh core takes the public-domain protocol design only, clean-room;
  - never use Reticulum code, documentation or data to train or fine-tune any model, directly or indirectly (no on-node fine-tuning exists, D10);
  - run the latest upstream release unmodified, and let the user install from upstream (PyPI) on their own machine;
  - keep this boundary in the README and release notes so nobody reads 111network as a derivative.
- [ ] **MIT components we ship later** (`llama-server`, `opencode`): include their notices next to the binaries.
- [ ] **Models:** when a GGUF is hosted by us, include the model license (and NOTICE if any) in the release.
- [ ] **Termux debs:** collect each package's license file when the closure is final (spike V3).

## 9. Open items for the owner

1. **Project license.** Review and accept the draft `LICENSE.md`. Part A now holds the verbatim PolyForm Noncommercial 1.0.0 text. Still open: fill in the Licensor and commercial-contact placeholders, and get the lawyer's review before the first signed release (D17). The scope (charities, schools and public bodies allowed), armed forces doing relief, and contributor terms are listed as open points in the drafting notes.
2. **Legal read: RNode firmware redistribution.** Needed before any firmware image is pinned in the manifest (section 3). Includes confirming upstream's current license text.
3. **Latest-upstream rule (decided, 2026-10-14).** Each 111 release pins the *then-latest* upstream Reticulum release by hash, and the updater refreshes that pin on every 111 release.
   The hash protects integrity; it does not freeze the version. Nothing is vendored and nothing is patched, so `rns-lock.txt` stays a requirements file with hashes. Between 111 releases the
   pinned version does not move. To do: make the release checklist resolve the latest `rns` at release time (the `rns==1.5.6` in `bootstrap/README.md` is a placeholder for that, and the KB
   entry still reports RNS 1.3.5), and have the updater treat a changed pin as a normal manifest refresh.
4. **Legal read: the AI-training clause.** The clause says "directly or indirectly". The agent reads `rnsd` logs and config as inference-time context (`logs.read`) and does not train. Whether that is outside the clause is not settled by anything in the repo.
5. **Reticulum License text.** Copy it into `knowledge/` or this file's references after reading upstream, and record the version of the text.
6. **Unverified rows in section 3 and 6:** Termux package licenses, platform-tools terms, opencode's runtime packages, the GGUF producer, larger Qwen3 sizes, MeshCore's license.
7. **Notices pipeline.** `THIRD-PARTY-NOTICES.md` was generated by a script that is not in the repo yet. Add a self-contained, deterministic generator (for example under `bootstrap/scripts/`)
   and a release-checklist step that regenerates the file when a lockfile changes. Components not yet covered, because they are not shipped yet: GPL-3.0 text and source offer for Termux APKs and debs, MIT notices for
   `llama-server` and `opencode`, the model license.
8. **Wording that conflicted with the draft license (done 2026-10-14).** `os/README.md` now says "source-available operating layer (free for personal, non-commercial, non-military use)". Its two statements about usage policies now say the software imposes none *at runtime* (no telemetry, no content policy, no policing), and that the license terms are a separate matter.
9. **Manifests (done 2026-10-14).** The three crates point at `LICENSE.md` through `license-file`, and `package.json` uses `SEE LICENSE IN`. If a registry ever needs an SPDX string, use `LicenseRef-111network`.
10. **Contributor terms.** Offering commercial agreements needs a contributor license or assignment; none exists.
11. **Automate the copyleft gate.** Add a license allow-list check (for example `cargo deny` and a JS license export) to `scripts/check.sh` so a copyleft dependency fails the build, and confirm which runtime libraries the static musl link pulls in.
