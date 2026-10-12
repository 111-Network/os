# Layer 1 — Bootstrap

The single entry point: **one command** that turns a Linux machine — or an Android phone connected over USB-C — into a running 111 Network node. It detects hardware and environment, assigns the device category, generates a provisional cryptographic identity, installs only what the device needs, prefetches a minimal offline cache, and leaves a **running minimal node** with a reachable dashboard. No configuration is required from the user.

Status of this document: **spec v1, pending adversarial review and user lock** (process: `os/plan/INDEX.md`; decisions: `os/plan/DECISIONS.md` D9; options + research: `os/plan/layer-1-options.md`).

## The one command

```sh
curl -fsSL https://install.111.network/install.sh | sh
```

- Same command on a Linux laptop installs the node locally; with an adb-authorized Android phone attached, `111 install --phone` (or the shim's guided path) provisions the phone instead. One phone per run (`--serial` to disambiguate).
- Re-running is safe: every step is idempotent (hash-match skips, probe-before-act). Existing identity is never overwritten.
- Zero telemetry, zero required interaction. On phones, exactly 3 taps are physically unavoidable (Developer Options, USB debugging, RSA authorize) — the installer prints an illustrated 3-step guide when no authorized device is present.

## Architecture

Two stages, both version-pinned:

1. **Shim** (`install.sh`, POSIX sh, ~200 lines): `set -u`, fully function-wrapped with execution only on the last line (anti-truncation). Detects `uname -s/-m/-o` (Android), musl vs glibc (`ldd --version`), 32-bit-userspace-on-64-bit-kernel (`getconf LONG_BIT`). Downloads the real installer binary from GitHub Releases (primary: own-domain mirror; fallback: GitHub), verifies **sha256 pinned inside the shim**, then re-execs it via `sudo`. Published at version-pinned URLs (`/v0.1.0/install.sh`); `latest` redirects to the newest pinned shim. The unverified part never runs as root.
2. **Installer binary** (`111-install`, musl-static Rust, x86_64 + aarch64): probe → categorize → keygen → prefetch → stage → supervise → receipt. Writes `/var/lib/111/receipt.json` (Determinate model) and `/usr/local/bin/111-uninstall` (k3s model). Idempotent re-runs; refuses cleanly on unsupported targets.

The node itself is ONE self-supervising binary, **`111d`** (musl-static Rust): provisional identity holder, tokio supervision tree (restart-in-place per task — the Layer 9 seam), embedded axum dashboard + `/health`, minisign-verifying self-updater (A/B swap + health-gated auto-rollback), KB fetcher, and — when python3 exists — parent of a pinned `rnsd` child process. Syncthing is the existence proof for this shape. systemd/runit wrappers are thin `Restart=always` shells, never the supervisor of record.

## Build matrix (artifacts per release)

| Artifact | Targets | Notes |
|---|---|---|
| `install.sh` | any POSIX sh + curl/wget | regenerated per release with fresh sha256 pins |
| `111-install` + `111d` | `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` | one combined binary acceptable (`111` with subcommands) |
| Termux APK bundle | `termux-app` v0.118.3 + Termux:Boot v0.8.1 + Termux:API v0.53.0, arm64-v8a, GitHub builds only | pinned URL+sha256 in the manifest; monthly CI revalidation |
| `channel-111-stable.toml` + `.sha256` + `.minisig` | — | rustup-v2-style manifest, minisign-signed |
| KB snapshot tarball | codeload SHA pinned per release | extracted to `kb/<sha>/` |

## Install flow — Linux host

State machine; each step probes first and resumes on partial failure:

1. Preflight: systemd present (`/run/systemd/system`) else refuse with docs pointer; not WSL/container without `--allow-degraded` (caps at relay); internet reachable (HTTP-204 ×2 + DNS + default route).
2. Probe: facts from `/proc`+`/sysfs` (see category rules); LoRa VID:PID AND product-string match; `iw phy` when present (absent ⇒ AP "unknown").
3. Category assignment (rule table below) + gateway capability note.
4. Identity: generate BIP-39 mnemonic → derive secp256k1 key (NIP-06 path `m/44'/1237'/0'/0/0`, k256 `schnorr`) → write `identity/` files (below). Never overwrite existing.
5. Prefetch: `apt-get download` closure (or `--download-only`) of `python3 python3-venv iw ca-certificates` (Fedora: `python3 iw ca-certificates`; Arch: `python iw ca-certificates`) into `cache/packages/`. Each package optional-degradable.
6. Stage: `/opt/111/releases/<ver>/` payload, KB tarball → `/var/lib/111/kb/<sha>/`, flip `current` symlinks, `/usr/local/bin/111` symlink.
7. Supervision: create `node111` system user (`useradd --system`, dialout/uucp group by `getent`), write `111node.service` (+distro drop-in, journald cap, LoRa udev rule), `daemon-reload`, `enable --now`.
8. Verify: `/health` 200 within 60s; dashboard reachable on `127.0.0.1:11180`; receipt written; print node npub + dashboard URL + `111 identity backup` hint.
9. Offer (post-install, dismissible): standard tier = small local model download. Maximal tier is not offered until standard exists.

## Install flow — phone over USB-C

Same command, `--phone` (or auto-guided when an adb device is present and host is `--phone`-less):

1. Preflight: adb present (distro `android-tools` with min-version check; fallback sha256-pinned Google platform-tools zip); exactly one device authorized; `getprop ro.product.cpu.abi` = arm64-v8a (armv7 refused with message); BBK devices: `ro.build.date` ≥ 2024-11 (else refuse — SIGKILLs Termux); MIUI detected ⇒ print per-install tap pre-warning.
2. Collision check: `pm path com.termux` + installer-signature dump; foreign-signature Termux ⇒ hard stop with instructions.
3. Install pinned GitHub APKs (`adb install -r` ×3) — debuggable builds, single signature source.
4. Headless channel: launch TermuxActivity, poll bootstrap unpack; `run-as com.termux` writes `~/.termux/termux.properties` (`allow-external-apps=true`), `~/.termux/boot/00-111-node.sh`, sshd keys, payload, prefetched .debs. Fallback when run-as fails: `input text` base64 payload injection (E2).
5. Harden via adb: `dumpsys deviceidle whitelist +com.termux`; appops `RUN_IN_BACKGROUND`/`RUN_ANY_IN_BACKGROUND`/`SYSTEM_ALERT_WINDOW`; phantom-killer fix per Android version (12: `device_config` pair in order; 12L/13/14+: `settings_enable_monitor_phantom_procs false`); lift Termux:Boot out of stopped state (`am start .BootActivity`).
6. Start node now via run-as with clean env (`PREFIX`, `LD_LIBRARY_PATH`, unset `LD_PRELOAD`); Termux:Boot owns reboot survival; sshd (key-only, port 8022) is the maintenance channel.
7. Verify: `adb forward tcp:11180 tcp:11180` + LAN poll; dashboard reachable; receipt on device.
8. Post-install: revoke USB-debugging authorizations (debuggable-build mitigation), print per-OEM battery checklist, offer adb-reboot verification test.

Phone layout mirrors the host under Termux: payload `~/111/`, state `$PREFIX/var/lib/111/` (identity/, kb/<sha>/, releases/), runit service `$PREFIX/var/service/111node/`.

## Detection & category rules

Facts: arch/cores/RAM/storage (sysinfo), distro (os_info), init (`/proc/1/comm`), virt (systemd-detect-virt else `/.dockerenv`+cgroup+DMI), battery (`/sys/class/power_supply/BAT*`), Wi-Fi phys + AP mode (`iw phy`), BLE (`/sys/class/bluetooth/hci*`), LoRa USB (nusb: `10c4:ea60` Heltec V3 · `1a86:*` T-Beam · `303a:*` ESP32-S3 native · `239a:4405/0029` T-Echo · `239a:8029` RAK4631 · AND product strings; table in embedded, field-updatable config), GPU (PCI class 0300/0302/0380 + vendor 10de/1002/8086), Pi model (`/proc/device-tree/model`), internet (HTTP-204 ×2 + DNS + route). A ~150-line POSIX-sh fallback mirrors the fact schema for debugging.

Precedence (first match wins; thresholds are D9 defaults, adjustable in one config table):

| Rule | Category |
|---|---|
| container/WSL/proot without `--allow-degraded` | warn, cap at degraded relay |
| discrete GPU (10de/1002) OR (≥8 cores AND ≥32GB RAM) | 4B compute hub |
| ≥512GB free storage AND ≥8GB RAM | 4A knowledge hub |
| LoRa radio AND (AP-capable Wi-Fi OR ethernet); or AP-capable Wi-Fi + ethernet | 3 bridge |
| battery present: ≥16GB RAM OR any GPU ⇒ 2B, else 2A | terminal |
| fallback ≥1GB RAM | 1 relay |
| internet up at detect time | + 5 gateway capability (recorded; re-evaluated dynamically by later layers) |

Phones: always terminals — ≥8GB RAM or flagship SoC ⇒ 2B, else 2A. Pi 5 8GB ⇒ 2B candidate; Pi 3/4 ⇒ 2A or relay. Pi has no RTC: manifest date comparisons are warn-only.

## Identity (provisional seam — final crypto is Layer 2's call, D9)

- secp256k1 keypair from BIP-39 mnemonic at NIP-06 path, generated at install time (system RNG seeded — Meshtastic CVE-2025-52464 lesson).
- `identity/` dir 0700, owned by `node111`: `secret.hex` 0600 (canonical 64-char hex), `npub` 0644, `mnemonic` 0600. O_EXCL, umask 077, fsync file+dir; re-runs refuse to overwrite.
- `111 identity backup` displays words on demand; `111 identity export` writes NIP-49 `ncryptsec`. Neither ever fires unprompted.
- Seam contract for Layer 2: the identity dir is the only key consumer in v1; swap = replace dir contents + reader.

## On-disk layout (naming is normative)

| Path | Contents |
|---|---|
| `/opt/111/releases/<ver>/` | versioned payload (bin/, share/) |
| `/opt/111/current` | symlink → active release (atomic switch; exactly 2 kept) |
| `/var/lib/111/` | state: `identity/` (0700) · `kb/<sha>/` + `current` · `cache/packages/` · `receipt.json` |
| `/etc/111/` | minimal config (0600 env file, k3s pattern) |
| `/usr/local/bin/111` | CLI symlink → `current/bin/111` |

Unit `111node.service`, `StateDirectory=111` (yields `/var/lib/111` owned by `node111`). Provisioning-host downloads: `~/.cache/111` only.

## Supervision

**Linux** — `/etc/systemd/system/111node.service`: `Type=notify` · `User=node111` + `SupplementaryGroups=dialout` (installer-detected `uucp` on Arch) · `AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE` (= `CapabilityBoundingSet`) · `NoNewPrivileges=true` · `ProtectSystem=strict` `ProtectHome=true` `PrivateTmp=true` · `PrivateDevices=false` (LoRa hotplug) · `Restart=always` `RestartSec=5` · `StartLimitIntervalSec=120` `StartLimitBurst=5` · `WatchdogSec=30` (daemon sends READY=1 at startup-complete, WATCHDOG=1 ≤15s gated on real internal health) · `After=network.target` only (NEVER network-online.target) · journal + `journald.conf.d` cap 100M · distro drop-in dir + `99-111-node.rules` udev (`ID_MM_DEVICE_IGNORE` for known LoRa IDs; brltty conflict documented). No sudoers, no polkit. Layer 9 self-restart primitive: voluntary `exit(0)` under `Restart=always`.

**Android** — Termux:Boot script (`termux-wake-lock` → profile → runsvdir) + termux-services runit `111node` (sleep-throttle guard; no backoff in runit) + cronie `/healthz` check as the placeholder seam until Layer 9. Boot-start on Android 15+ is best-effort (termux-boot #259).

**Non-systemd distros**: clean refusal with docs pointer.

## Dashboard v1 (deliberately minimal — real dashboard is Layer 4)

Single binary-embedded page (axum + rust-embed) on `127.0.0.1:11180` (LAN binding is an explicit `111 config` opt-in with warning until Layer 4 auth). Shows: npub, category, version, KB sha, transport state (`no transport` / `isolated` / `joined` — distinct, honest), uptime/health, standard-tier offer. `/health` (200/503) and `/api/status` (JSON) on the same port. "Mesh joined" = rnsd child up AND ≥1 AutoInterface peer within 60s when another node exists on the LAN.

## Transport (conditional, Bundle B)

When python3 exists at install: `/opt/111/venvs/rns` with `rns==1.5.6` pinned, `rnsd` as supervised child of 111d, AutoInterface enabled, detected radios reported. No python3 ⇒ detection-only stub (dashboard says `no transport`). meshtasticd is never auto-installed (revoked 2.8 alpha; useless without LoRa hardware). 111d regenerates rnsd config on interface changes — flagged seam for Layer 6 review.

## Updates & rollback

- Manifest: `channel-111-stable.toml` (manifest-version, date, channel, release, `min-upgrade-from`, `[pkg.*.target.*]` url+sha256+size, KB section) + `.sha256` + detached **minisign**; Ed25519 verify key compiled into 111d. **Key rotation before first signed ship:** new keys signed by old, keychain carried in the signed manifest.
- Flow: notice (~2KB manifest via internet or mesh gossip) → verify minisig + sha256 + `min-upgrade-from` → fetch full signed artifact → stage `releases/<new>/` → flip symlink → restart → health gate: `/health` within 60s of first post-update READY=1 else auto-revert (binary + KB symlink revert together). Updater suppresses WATCHDOG=1 during the flip.
- No deltas in v1 (LoRa-constrained deltas are a Layer 6/7 problem). KB git-sync arrives with Layer 7 (v1: tarball per release, KB repo stays git-lfs-free).
- APKs pinned for v1; self-update covers 111d only; APK refresh = Layer 9 maintenance task.

## Security & trust model

- Installer verification = sha256-pinned-in-shim (catches server compromise — Codecov class) + function-wrap (anti-truncation) + minisign-signed SHASUMS for out-of-band checks; GitHub artifact attestations in v1.1. GPG rejected.
- Termux GitHub builds are debuggable + test-key-signed (D9 E1, accepted): anyone with adb authorization can read node keys ⇒ installer revokes authorizations post-install; hardware-backed storage is a later-layer hardening.
- Key material: 0600/0700, never logged, never transmitted; transport keys (X25519) derived/attested by later layers — v1 ships identity only.
- `dialout` grants broad serial access — accepted for single-purpose nodes, recorded here.
- Zero telemetry.

## Refusals & boundaries (v1)

macOS/Windows (node and provisioning host) · ESP32/LoRa relays · manual Termux-only flow · non-systemd distros · armv7 phones · proot-distro · package-repo install · >1 phone without `--serial` · auto-installing meshtasticd · USB LoRa from the phone itself (Layer 6). Each refusal = clear message + docs pointer, never a silent skip.

## Pre-build verification gates (must pass before build tasks start)

V1 `run-as com.termux` on Termux v0.118.3 GitHub build, headless bootstrap (one physical phone) · V2 Termux:Boot reboot autostart (Pixel + Samsung + Xiaomi) · V3 `pip install rns==1.5.6` in Termux from prefetched .debs only · V4 phantom-killer adb settings per Android 12–16 · V5 `input text` fallback across 2–3 OEM keyboards · V6 MIUI per-APK install flow · V7 Pi 32-bit-userspace detection · V8 systemd unit start/watchdog/dashboard/uninstall on Debian + Arch VMs. V1–V3 are blockers; V4–V8 findings degrade features, not the release.

## Acceptance criteria (v1 done = all true)

1. Fresh Debian 12 VM: the one command → running node, dashboard 200 on 127.0.0.1:11180, identity files 0600/0700, receipt present, `111-uninstall` removes everything.
2. Re-run: idempotent no-change (hash match, identity untouched).
3. Two nodes on one LAN: both report `joined` within 60s.
4. Kill -9 111d: systemd restarts it ≤10s; watchdog fires only on genuine hang.
5. Update: staged new release flips and serves; forced-bad release auto-reverts.
6. Phone (V1-passing device): command drives install to dashboard-reachable via `adb forward`, survives reboot (V2), sshd maintenance channel works.
7. Offline after install: node runs, dashboard serves, KB browsable — WAN cable pulled.

## Seams for later layers

Layer 2: identity dir swap (above) · Layer 3: standard-tier offer hook + hub offload endpoint · Layer 4: dashboard surface/port contract + auth · Layer 5/6: rnsd child → real transport management; radio detection table · Layer 7: gateway re-evaluation, KB git-sync, manifest gossip · Layer 9: supervision-tree handles, `/health` contract, exit(0)-restart primitive, watchdog heartbeat, per-OEM phone watchdog channel.

Build tasks: `os/bootstrap/TASKS.md` (created at build start per `os/plan/EXECUTION.md`).
