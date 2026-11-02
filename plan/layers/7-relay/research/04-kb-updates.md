# Layer 7 research, lens 4: KB, updates and manifest distribution over the mesh; the knowledge hub

Scout: sonnet lens 4, 2026-11-01, base `3bf545b`. Code facts were read on that base (`N/` = `1-bootstrap/crates/node111/src/`).
Web facts came from raw.githubusercontent.com, crates.io, PyPI and WebSearch; anything not fetched from a primary file is marked.

## 1. Summary

- **A hub can already be a trustless update origin; the crypto is carrier-free.** The updater fetches a manifest, verifies the minisign
  signature and the floor, then sha256-pins the payload (`N/daemon/updater/fetch.rs:135-161`), and the root helper repeats every check
  from the staging bundle alone (`N/update_helper.rs:131-138`). What blocks a hub is plumbing: two hard-wired HTTPS bases
  (`fetch.rs:16-20`) and absolute payload URLs inside the signed manifest (`N/manifest.rs:156,295-302`). Fetching each pin by its
  sha256 from a hub, with the manifest's own URL as the fallback, needs no manifest change.
- **Pick: one offline-origin code path serves both the hub and the USB stick.** An origin is "a directory of files"
  (`channel-111-stable.toml`, `.minisig`, payload tarballs named by sha256). The hub serves that directory read-only over HTTP;
  `111 update --from <dir>` reads it from a stick. Tier 2 bao-verified transfer is the v1.1 upgrade for non-IP bearers.
  **Nothing here puts a resident process, listener or timer on a non-hub node.**
- **The KB does not need git on the node.** The whole KB is 348 KB raw, **96.6 KB** as `tar.gz` (measured: 67 entries plus the
  generated `llms.txt` files). A whole-monorepo `git bundle` is 5.2 MB (measured), 54 times larger, and git is GPL-2.0-only. Ship the
  KB as a signed snapshot. `build.py` stays at the origin (it is stdlib-only, so it could run on Termux, but the node never needs it).
- **LoRa never carries a release.** At Meshtastic LongFast the 5.7 MB working figure is **15 h** of pure airtime unsealed and
  **26 h** sealed, which is **62 to 110 days** under a 1 % duty cycle. The 97 KB KB snapshot is 15 to 27 min of airtime
  (1 to 1.9 days at 1 %). A signed manifest with its signature (about 3.2 KB) is 30 to 55 s, so "is there a newer release" is
  affordable on LoRa and "fetch it" is not. A stock MeshCore repeater (Cat 1) carries none of this.
- **"Manifest gossip" is not needed.** Pull beats push here: a node that reaches its hub checks the hub's manifest once, driven by
  the link coming up (event-driven, no new timer). An optional 5-byte release tag in an announce's `appdata` fits the 10 B that
  remain on a 160 B LoRa frame, but it is an unsigned nudge, so it is a later add-on, not v1.
- **Four defects in the as-built trust path matter more once a hub is the origin** (§3 items 6 to 9): the manifest's `channel` is
  never compared with the expected channel; the keychain rotation chain is parsed but never verified; there is no freeze
  protection (a hub can serve an old signed manifest forever); `min-upgrade-from` is enforced only by the installer.

## 2. Candidates

| Name | Upstream | License (verified?) | Language | Maturity | Resident weight | Fit | Why |
|---|---|---|---|---|---|---|---|
| minisign (signer CLI) | github.com/jedisct1/minisign | ISC, `LICENSE` read 2026-11-01 (verified) | C | `LICENSE` is dated 2015-2026; Termux packages 0.12 (`termux-packages/packages/minisign/build.sh`, needs libsodium) | none (maintainer-side tool) | **adopt** (already the root) | Ed25519, detached sigs, time-free; signatures are deterministic by default (README "Signature Determinism"), which helps reproducible release bundles |
| `minisign-verify` crate | github.com/jedisct1/rust-minisign-verify | MIT, `LICENSE` read (verified) | Rust | 0.3.0 on crates.io, 2026-09-25 (the tree uses `0.2`, `N/../Cargo.toml:27`) | in the binary already | **adopt** (in use) | Bump to 0.3 is optional; no new dependency |
| `blake3` + `bao-tree` | github.com/n0-computer/bao-tree | `bao-tree` MIT OR Apache-2.0, `blake3` CC0-1.0 OR Apache-2.0 [WITH LLVM-exception], both from crates.io metadata (not the repo file; verified for the registry record only) | Rust | `bao-tree` **0.17.0 on 2026-10-22**; Layer 5 pins `0.16` (`mesh111/Cargo.toml:20`) | +94 KB x86_64 / +29 KB aarch64 (KB `blake3-bao`), already linked | **adopt** for Tier 2 (v1.1) | Same hash for LoRa chunks and TCP; resume at chunk boundaries. Note the 0.16 to 0.17 drift before the v1.1 build |
| `git bundle` | git/git | **GPL-2.0-only**: `COPYING` says "the only valid version of the GPL ... is this particular version (v2, not v3 or any later)" (verified). Termux ships git 2.56.0, `GPL-2.0` | C | `git-bundle(1)` and `gitprotocol-http` read from `Documentation/` on master | none unless run | **bridge** (a human may use it; the node never needs it) | Only usable as a separate process, never linked into a GPL-3.0 binary. Bundles carry history the node does not need |
| gitoxide (`gix-*`) | github.com/GitoxideLabs/gitoxide | MIT OR Apache-2.0 (crates.io record, `gix-pack` 0.77.0, 2026-10-22; not read from a repo file) | Rust | active | build weight, no runtime need | **reject** for the node | Would add build weight to verify a bundle nobody needs on the node |
| TUF specification 1.0.37 | github.com/theupdateframework/specification | **(unverified)**: the repo has no `LICENSE` at the five paths tried; `tuf-spec.md` is dated 2026-10-08 | spec | Graduated CNCF project (from memory, unverified) | n/a | **reference** | Roles and attack taxonomy; §3 item 10 compares it with our single key |
| python-tuf / `tough` | theupdateframework/python-tuf; awslabs/tough | python-tuf `LICENSE` = Apache-2.0 text, PyPI says `Apache-2.0 OR MIT`, 7.0.1 on 2026-09-02 (verified); `tough` `LICENSE-MIT`, 0.24.0 on 2026-07-10 (verified); rust `tuf` is 0.3.0-beta9 (2025-10-20, stale) | Python; Rust | active | a whole second metadata stack | **reject** | Online timestamp and snapshot roles and RFC 3339 `expires` need a clock and a signing service we do not run |
| apt-offline | rickysarraf/apt-offline | GPL-3.0, `LICENSE` header read (verified) | Python | 1.8.6 on 2025-02-08 (search snippet) | none | **reference** | The pattern: the offline box writes a request file, a connected box fills it, the offline box installs it and keeps its own signature check; `--allow-unauthenticated` is discouraged |
| F-Droid client ("Nearby" / swap) | f-droid/fdroidclient | GPL-3.0, `LICENSE` read (verified) | Kotlin | Nearby app listing calls itself "early alpha" (search snippet) | n/a | **reference** | Lesson from a 2014 issue thread (search snippet): discovery cannot carry a repo fingerprint, so a LAN-discovered repo is trust-on-first-use. Our fix is the compiled-in key, so a discovered hub is never a trust anchor |
| Kiwix stack (`kiwix-tools`, `libkiwix`, `libzim`) and Kiwix Hotspot | github.com/kiwix/*, openzim/libzim, offspot | kiwix-tools GPL-3.0 (`COPYING` read); libkiwix GPL-3.0 (`COPYING` header only); libzim "GPLv2 or later" (README line 237); kiwix-hotspot GPL-3.0 and **end-of-life, superseded by offspot** (its README) | C++ | libzim 9.8.2 (`meson.build`) | a process only while serving | **reference** for Layer 8 | `kiwix-serve` is the adopted ZIM server; it belongs to Layer 8 as an on-demand child, not to this layer |
| Syncthing | syncthing/syncthing | MPL-2.0, `LICENSE` read (verified) | Go | mature | resident daemon | **reference** | Block-hash exchange is the shape of Tier 2, but a resident sync daemon fails the Pi rule |
| `rncp` (inside Reticulum) | markqvist/Reticulum | Reticulum License: `LICENSE` read 2026-11-01, it forbids use in "any kind of system which includes amongst its functions the ability to purposefully do harm" and in AI/ML training datasets (verified, non-OSI); `rns` 1.5.7 on 2026-10-05 (PyPI) | Python | docs show `rncp --listen`, send and `--fetch` (`docs/source/using.rst:614-645`) | process only when run | **bridge** (user-run) | A person may move the signed bundle with `rncp` over any Reticulum link. We only verify what lands in a directory. No import of RNS in any new program of ours, and none is proposed |

## 3. Findings that change the plan

1. **The updater is carrier-free but origin-locked.** `UpdaterConfig` holds only `manifest_base` and `fallback_base`, both HTTPS
   first-party (`N/daemon/updater/fetch.rs:16-20,28-48`); `check_once` is called from one place (`N/daemon/updater/mod.rs:107`) on a
   1 h timer with a 24 h backoff cap (`mod.rs:22-23`). There is **no `111 update` verb**: the CLI set is install, daemon, identity,
   config, dashboard, fabric, uninstall, update-helper, update-guard (`N/cli.rs:1-3,188-211`). `--from <dir>` is therefore a new
   verb (small, a oneshot, zero resident weight).
2. **Payload URLs are absolute and fetched whole into RAM.** `fetcher.get(&payload.url)` (`fetch.rs:161`) pulls the tarball into one
   `Vec<u8>` (cap 512 MB, `fetch.rs:101`). A hub path must key off the pin's `sha256` and `size` (already verified, `stage.rs`), not
   the URL: `GET <hub>/blob/<sha256>`, falling back to `payload.url`. Transient RSS equals the tarball size during an update; that is
   not resident, but the Pi owner should know the number (re-measure when the payload size changes).
3. **The trust chain survives any carrier.** Order today: minisign over the raw manifest bytes, then the floor, then parse, then pins
   (`N/manifest.rs:495-507`), then the staged tree against `share/payload.manifest` (`stage.rs:1-4`), then the helper re-verifies from
   `state/staging/<ver>/channel-111-stable.toml` + `.minisig` plus tarball re-hash (`N/update_helper.rs:131-138,156-185`). A hub's
   bytes are never trusted without the compiled-in key (`N/daemon/updater/mod.rs:18-19`).
4. **The `[kb]` pin exists and nothing consumes it.** The manifest parses `kb` (`manifest.rs:202,541`, "pinned here for prefetch"), but
   `grep` finds no caller of `.kb()`; the KB reaches a node only inside the payload at `current/share/kb/`, verified at start
   (`N/daemon/kb.rs:1-3,33-52`). Because `releases/<ver>/` is root-owned and one atomic unit (1-bootstrap README line 111), a
   KB-only update is not possible without either a respin (`YYYY.MM.DD.N` is already legal calver) or a new root-helper request type.
5. **The premise "the installer already records hub pairing" is wrong.** The install receipt carries no hub. The home hub is a
   Layer 5 setting written at enrolment (`N/daemon/fabric/mod.rs:407-418`, `settings::rebuild_home_hub`); `:11183` binds on the LAN
   only while `hub_role` or `listen_lan` is on (`fabric/mod.rs:38-39,694`). A node that never enrolled has no hub to ask, so
   `--from http://host:port` or Layer 7 discovery must supply one.
6. **The manifest's `channel` is never compared with the expected channel.** The field is required non-empty
   (`manifest.rs:346`) and the accessor exists (`manifest.rs:512`), but its only call site is unrelated (`identity/commission.rs:241`).
   A hub holding a validly signed `beta` or `dev` manifest could serve it under the `stable` filename and every check above would
   pass. Fix is one `ensure!(manifest.channel() == "stable")` in `parse_verified`.
7. **The keychain chain is parsed, not verified.** Entries are checked only for consecutive generations and for the presence of
   `signature_by_previous` (`manifest.rs:448-470`); the signature check uses the single compiled key
   (`fetch.rs:135-141`, `update_helper.rs:137`), and the floor trusts the manifest's own `max_gen` claim (`manifest.rs:682-696`). A
   release signed by a generation-1 key is refused by every node, and an offline node that sleeps through a rotation cannot follow
   it from a hub. Rotation needs a verify-the-chain step (previous key signs the next pubkey, the node walks from the compiled key)
   before hub-carried releases are real. This is a Layer 1 amendment, not a Layer 7 one.
8. **No freeze protection, and a hub makes it worse.** The floor ratchets on what was *installed* (`update_helper.rs:212-225`), never
   on what was *seen*. A hostile or stale hub, or a hub that lost its internet a year ago, can answer "up to date" with an old
   valid manifest indefinitely. TUF's defence is signed `expires` (spec lines 178-180), and its own spec admits a "bogus clocks"
   problem (line 1735). We have no mesh epoch until Layer 7 defines it (L2 §13), so expiry is not available in v1.
9. **`min-upgrade-from` is enforced only at install** (`N/install/stage.rs:217-219`; no caller in the updater or helper). A hub-fed
   node could be walked across a gap the installer would refuse. Cheap fix in the helper.
10. **TUF comparison.** Our one manifest does the work of TUF's `targets` and `snapshot` together: it lists every artifact with
    hash and size, so mix-and-match attacks (spec line 185) cannot occur. The floor replaces rollback protection
    (line 189). What we lack: threshold signatures over a root, delegated roles, an online `timestamp` role for freshness. What we
    skip on purpose: expiry (clock), a signing service on a server we would have to keep online, per-target delegation (one
    maintainer, one channel).
11. **Airtime arithmetic** (Layer 0 `concepts/lora-airtime-budget`: 32 KB = 5.0 min unsealed / 8.9 min sealed on LongFast, so 9.4 s/KB
    and 16.7 s/KB; one 255 B frame 2.1 s; the 1 % band stretches wall time 100 times). Computed here, single hop, ideal:

    | Object | Bytes | Airtime unsealed | Airtime sealed | Wall time at 1 % |
    |---|---|---|---|---|
    | manifest + minisig | about 3.2 KB (fixture is 2.9 KB, `tests/fixtures/manifest/channel-111-stable.toml`, plus a signature of a few hundred bytes) | about 30 s | about 53 s | 50 to 90 min |
    | KB snapshot `tar.gz` | 96.6 KB (measured) | 15 min | 27 min | 1.0 to 1.9 days |
    | release payload (L0 working figure) | 5.7 MB | 14.8 h | 26.4 h | 62 to 110 days |
    | current x86_64 `111` alone, uncompressed | 14.8 MB (`1-bootstrap/TASKS.md`, L5 cross-build measurement 14,796,184 B) | 38.5 h | 68.6 h | 160 to 286 days |

    MediumFast (3.52 kbps) divides these by about 3.3. The compressed size of today's real payload tarball is not measured (it needs
    a release build): the 5.7 MB row is the Layer 0 figure and the 14.8 MB row is an upper bound of the binary alone. Release bytes
    are public and signed, so the amateur-band "no obscured content" rule is not the obstacle here (sealing is not needed for
    them); whether a human-run relay into a licensed band may carry them is a legal read the owner has not done, and the
    plan never sends them there.
12. **Tier 2 is specified, not built.** `OBJ_ADVERT` / `OBJ_REQ` / `OBJ_CHUNK` are pinned in 5-messaging README §5 and built in v1.1;
    the `tcp-lan` chunk frame `0x05` is reserved; the blob store and bao outboard are v1 (README lines 468-479, 814). `KB`, `UPDATE`
    and `PATCH` classes exist with no producer (00-code-seams finding 12). L5 §19 already names the updater intake at
    `fabric/blobs/<hash>` "verified by the same minisign manifest". So over IP the mechanism that fills it is HTTP now, Tier 2
    later; both end in the same staging directory.
13. **A Cat-1 relay carries none of this.** The stock MeshCore repeater floods group-data frames of at most 163 to 165 B, blind
    (KB `lora-airtime-budget`, D19 c); it has no storage, no HTTP and no object protocol. It can carry, at most, a 5-byte release tag
    in an announce, and only if our announce rides a flood frame. Releases reach a LoRa-only node by a person, a stick or a gateway.
14. **`build.py` is stdlib-only** (`0-knowledge/_meta/build.py:19-28`: argparse, re, shutil, sys, tempfile, dataclasses, datetime,
    pathlib) and self-tests determinism by generating twice (`build.py:838-840`). The KB's `llms.txt` files are committed
    (`git ls-files` shows them). Termux packages `python` (3.x, `termux-packages/packages/python/build.sh`) and `git` (2.56.0). So
    regeneration on a node is possible but unnecessary if the origin signs the already-generated tree.

## 4. Recommendation

### Fork A: where does a node without internet get a release?
- **A1 internet only (status quo).** Zero work; fails the offline-first promise for every node that never sees a gateway.
- **A2 hub as HTTP origin plus `111 update --from <dir|url>` (pick).** One path: "an origin is a set of files". The hub serves
  `channel-111-stable.toml`, `.minisig` and `blob/<sha256>` read-only and unauthenticated (public signed bytes, same posture as
  `/health` and `/api/status`, 1-bootstrap README line 137), only when `hub_role` is on, on a listener that already exists. The node
  tries `[enrolled home hub, internet bases]` in an order the user can see; `--from` points at a mounted stick or a URL.
  A hub fetches its own copy from the internet whenever it has one (it is then a gateway, 5A or 5B) or from a stick.
  Every byte passes the existing four checks plus the fixes in §3 items 6 to 9.
- **A3 Tier 2 bao-verified chunks over the fabric.** The right shape for resumable transfer over BLE/LoRa-adjacent bearers and
  for Layer 8 datasets; needs v1.1 wire work (§3 item 12). Schedule it after A2, same staging directory.
- **A4 sneakernet only.** Subset of A2; keep it as the guaranteed floor (`--from`), never the only path.
- **LoRa for binaries: not offered.** §3 item 11. LoRa may carry the manifest (a few minutes) so the UI can say "release X exists, bring a stick or reach a hub".

### Fork B: the KB
- **B1 KB in the release payload (status quo, pick for v1).** A KB change costs a respin (`.N`); on a LAN hub that is a few MB, fine.
  Zero new privileged code.
- **B2 KB overlay with its own signed pin.** The `[kb]` pin is already in the manifest; needs a new root-helper request type, a
  `highest_kb_date` floor dimension and a UI "KB newer than binary" state. Pick it only if the owner wants KB cadence different from
  release cadence (e.g. daily KB, quarterly binary). Size is 96.6 KB, so it is even LoRa-reachable in a day.
- **B3 git bundles / dumb-HTTP git export (`git update-server-info`).** Works (`gitprotocol-http`: dumb clients read `info/refs`),
  but a whole-repo bundle is 5.2 MB, the node has no use for history, and git is GPL-2.0-only so it can only run as a separate
  process. Let humans use it; the node does not.
- **B4 gateways `git fetch` the 111 repo when online.** Same cost as B3 plus a second trust root (git signed tags); reject.

### Fork C: "manifest gossip"
- **C1 pull on link-up (pick).** When the fabric link to the home hub comes up, or `--from` runs, the updater checks that origin
  once. No gossip, no new timer.
- **C2 a 5-byte release tag in the announce `appdata`.** Fits the 10 B left on a 160 B LoRa frame (00-code-seams summary, 4th bullet).
  Unsigned (announce signature proves only who said it), so it can only cause a poll; costs zero extra frames. A later nicety.
- **C3 a signed `UPDATE` notice frame.** Needs the release process to sign a second object per release and replay rules; no gain over C1.

### Trust model (for any carrier)
Never trust a hub's bytes: verify the manifest with the compiled-in key before parsing, compare `channel`, apply the floor,
enforce `min-upgrade-from`, pin by sha256 and size, re-verify the tree, and let the root helper repeat it (already so). Rotation:
implement the chain walk before the first rotation. Clock faults: minisign verification is time-free, so a wrong clock never
blocks a hub update (only HTTPS to the internet suffers). Freeze: do not add expiry in v1; mitigate by showing "newest release
seen from <origin>, age by mesh-epoch once defined" and by preferring any internet origin when one is reachable.

## 5. Open questions for the owner

1. **Hub serving surface.** Is an unauthenticated, read-only GET route for signed public files acceptable on a hub's existing
   listener (dashboard `:11180` is LAN-bound only with LAN sharing on and is pair-code gated; fabric `:11183` speaks length-prefixed
   frames, not HTTP)? Or should the hub get a third port (against the "no new listener" posture)?
2. **Freeze stance.** Accept that a stale hub can hold a node on an old release until it reaches any other origin (v1), or add
   an expiry once Layer 7 defines the mesh-epoch source (L2 §13)?
3. **Rotation first.** May the Layer 1 amendment (keychain walk, channel check, `min-upgrade-from` in the helper) land before
   hub distribution is built?
4. **KB cadence.** Respin (B1) or overlay (B2)?
5. **`111 update --from`.** Is a new verb that writes into `state/staging/` and drops `update-request.json` acceptable (the helper,
   not the verb, stays the only flip actor)?
6. **Layer 8 split.** Release files and Layer 8 datasets (ZIM, maps) share one hash-keyed hub object store and one Tier 2 transfer;
   the open part is who signs datasets (the 111 release key, the hub owner's key, or the dataset publisher's checksum).
   The English Wikipedia ZIM is of the order of 100 to 130 GB (search snippet, anecdotal); Kiwix catalog checksum formats were not verified.

## 6. Sources

Code (all at base `3bf545b`): `1-bootstrap/crates/node111/src/daemon/updater/{fetch.rs,mod.rs,stage.rs}` · `manifest.rs` ·
`minisign.rs` · `floor.rs` · `update_helper.rs` · `daemon/kb.rs` · `cli.rs` · `install/stage.rs` · `daemon/fabric/mod.rs` ·
`0-knowledge/_meta/build.py` · `1-bootstrap/README.md` (Updates & rollback, lines 151-160) · `5-messaging/README.md` (§5, §11, §19) ·
`0-knowledge/concepts/lora-airtime-budget.md` · `0-knowledge/projects/blake3-bao.md` · `plan/layers/7-relay/research/00-code-seams.md`.
Measured here: `0-knowledge` as `tar.gz` 96,577 B (348,160 B raw, 67 `.md` files); `git bundle create HEAD` 5,234,183 B for 139 commits.

Web (fetched 2026-11-01): raw.githubusercontent.com/jedisct1/minisign/master/{LICENSE,README.md} ·
.../jedisct1/rust-minisign-verify/master/LICENSE · .../git/git/master/{COPYING,Documentation/git-bundle.adoc,gitformat-bundle.adoc,gitprotocol-http.adoc} ·
.../theupdateframework/specification/master/tuf-spec.md · .../theupdateframework/python-tuf/develop/LICENSE · .../awslabs/tough/develop/LICENSE-MIT ·
.../rickysarraf/apt-offline/master/{LICENSE,README.md} · .../f-droid/fdroidclient/master/{LICENSE,README.md} ·
.../kiwix/kiwix-tools/main/{COPYING,README.md} · .../kiwix/libkiwix/main/COPYING · .../openzim/libzim/main/{README.md,meson.build} ·
.../kiwix/kiwix-hotspot/master/{LICENSE,README.md} · .../syncthing/syncthing/main/LICENSE ·
.../markqvist/Reticulum/master/{LICENSE,docs/source/using.rst} · .../termux/termux-packages/master/packages/{git,python,minisign}/build.sh ·
crates.io API (`minisign-verify`, `bao-tree`, `blake3`, `gix-pack`, `gitoxide`, `tough`, `tuf`) · pypi.org/pypi/{tuf,rns}/json.

Search snippets (not fetched): F-Droid Nearby and swap (f-droid.org/packages/org.fdroid.nearby/, dev.guardianproject.info issue 2978);
apt-offline workflow (dyn.manpages.debian.org/jessie/apt-offline/apt-offline.8); TUF freeze and offline-key commentary
(dev.to articles, redirect.github.com/sigstore/sigstore/issues/1091); Kiwix ZIM sizes (zerokspot.com/weblog/2021/01/18/kiwix-offline-wikipedia/).
One search result carried an instruction-like string inside its text; it was ignored.
