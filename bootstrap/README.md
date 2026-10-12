# Layer 1 — Bootstrap

The single entry point: **one command** that turns a Linux machine — or an Android phone connected over USB-C — into a running 111 Network node. It detects hardware and environment, assigns the device category, generates a provisional cryptographic identity, installs only what the device needs, prefetches a minimal offline cache, and leaves a **running minimal node** with a reachable dashboard. The only interaction is privilege escalation (sudo/doas/su) and, on phones, the unavoidable on-device taps.

Status of this document: **spec v2, post-adversarial-review** (panel: 5 judges, 99 findings, triage in `os/plan/layer-1-review.md`; decisions: `os/plan/DECISIONS.md` D9; research: `os/plan/layer-1-options.md`). Awaiting user lock.

## The one command

```sh
curl -fsSL https://install.111.network/install.sh | sh          # install this machine
curl -fsSL https://install.111.network/install.sh | sh -s -- --phone   # provision the attached phone
```

- Plain form installs a node on the local machine. `--phone` provisions the single adb-authorized attached Android phone instead. If a phone is attached and the host is already provisioned, the guided path suggests `--phone`. Both at once = host first, phone second. One phone per run (`--serial` to disambiguate).
- Re-running is safe: every step probes content (hashes, markers), not existence, and resumes at the failed step. Existing identity is never overwritten. The installer refuses to stage any version ≤ the installed one (receipt-checked) and anything below `min-upgrade-from`.
- Zero telemetry. Zero interaction beyond privilege escalation; on stock Android exactly 3 taps are physically unavoidable (Developer Options, USB debugging, RSA authorize) — MIUI/HyperOS adds a Mi-account-gated "Install via USB" toggle + one confirm per APK, and a SIM-less Xiaomi cannot proceed at all (preflight refuses up front with the exact prerequisite list). The installer prints an illustrated guide whenever a required tap is detected.

## Architecture

**ONE musl-static Rust binary** (`111`, x86_64 + aarch64) with subcommands: `install` (the installer), `daemon` (the node, exec'd by systemd/runit), `identity backup|export`, `config`, `uninstall`. Plus a tiny POSIX **shim**.

1. **Shim** (`install.sh`, ~200 lines): `set -u`, fully function-wrapped, execution only on the last line (anti-truncation). Detects `uname -s/-m` and `getconf LONG_BIT` (32-bit userspace on 64-bit kernel; armv7l kernels ⇒ clean refusal). Downloads the `111` binary — **GitHub Releases is the primary/canonical source**, own-domain mirror is fallback (brief A1/A4 hierarchy). Verifies **sha256 pinned inside the shim** in a `mktemp -d` (0700), then `sudo install -m 0755` to a root-owned path and execs **that** copy (closes the verify→exec TOCTOU). Privilege order: already-root → `sudo` → `doas` → print exact `su -c` line and exit. sha256 tool: `sha256sum` with busybox fallback. Published at version-pinned URLs (`/v0.1.0/install.sh`); the binary refuses to install below a compiled-in minimum version (stale-shim redirects can't downgrade). Each shim's sha256 + minisig is published on the GitHub release page (independent channel).
2. **`111 install`**: preflight → probe → categorize → keygen → package install + prefetch → stage → supervise → verify → receipt (`/var/lib/111/receipt.json`, root-owned 0644) → `/usr/local/bin/111-uninstall`. The minisign verify key is compiled into the binary; the installer MUST verify the channel manifest's detached minisig before trusting any pin in it (hard failure otherwise) — this binds every second-hop artifact (payload, APKs, KB tarball, platform-tools fallback) to the signed manifest.
3. **`111 daemon`** (the node): provisional identity holder, tokio supervision tree (restart-in-place per task with exponential backoff + cap — the Layer 9 seam), embedded axum dashboard + `/health`, minisign-verifying self-updater (A/B + probation auto-rollback), KB server, and — when python3+venv exist — parent of a hash-locked `rnsd` child. systemd/runit are thin `Restart=always` wrappers, never the supervisor of record.

## Artifacts per release (all pinned URL+sha256+size in the signed manifest)

| Artifact | Notes |
|---|---|
| `install.sh` | regenerated per release with fresh pins; shim sha256+minisig on the release page |
| `111` | `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`; aarch64 built with `-z max-page-size=16384` (16KB-page Android kernels), CI asserts `p_align=0x4000` |
| release payload tarball | `releases/<ver>/` = `bin/111` + `share/kb/` (KB snapshot, immutable release asset — never a codeload pin; GitHub regenerates codeload bytes, 2023 incident) |
| Termux APK bundle | termux-app v0.118.3 + Termux:Boot v0.8.1 + Termux:API v0.53.0, arm64-v8a, GitHub builds, single source |
| Termux .deb closure | enumerated per-release (minimum: openssh, termux-services, cronie, python, python-cryptography; final list settled by spike V3), per-deb sha256, Termux repo snapshot pinned, monthly CI revalidation |
| `rns-lock.txt` | pip requirements with hashes (`rns==1.5.6` + transitive closure), `--require-hashes`; lockfile sha256 in manifest |
| `channel-111-stable.toml` + `.sha256` + `.minisig` | rustup-v2-style; minisign-signed; carries keychain |

## Install flow — Linux host

State machine; every step probes content first, resumes on partial failure, records outcome in the receipt:

1. **Preflight:** systemd present (`/run/systemd/system`) else refuse with docs pointer · privilege contract (above) · free space ≥ (payload + packages) ×2 · clock sanity: HTTP `Date` header of the connectivity probe vs local time (loud refusal beyond threshold — Pi has no RTC) · connectivity = HTTPS HEAD against the real artifact endpoints + HTTP-204 ×2 + DNS + default route (probe endpoints named: `install.111.network/204` + own-domain fallback only) · WSL/container: systemd-less containers refuse; systemd-ful WSL/containers warn and cap at degraded relay (`--allow-degraded` silences the warn) with a degraded unit drop-in (AmbientCapabilities stripped to the container's CapBnd).
2. **Probe:** facts from `/proc`+`/sysfs` (rule table below). All facts degradable to "unknown".
3. **Categorize** (deterministic table below) + record gateway capability (install-time internet is guaranteed, so this records intent for later layers).
4. **Identity:** BIP-39 mnemonic (12 words) → secp256k1 key (NIP-06 path `m/44'/1237'/0'/0/0`, k256 `schnorr`); trio (`secret.hex` 0600, `npub` 0644, `mnemonic` 0600) written to a temp dir and **atomically renamed** into `identity/` (0700, node111); never overwrite. Keygen runs at install, system RNG seeded.
5. **Packages (online):** best-effort `apt-get update` first; then **install** `python3 python3-venv iw ca-certificates` (dnf5/pacman equivalents per family: Fedora `python3 iw ca-certificates`, Arch `python iw ca-certificates`) — documented host mutation, and what makes Bundle B real on fresh images. The resolved closure is also **downloaded** into `cache/packages/` (root-owned 0755) via `apt-get download` on the closure / `dnf5 download --resolve` + createrepo_c / `pacman -Sw` — offline reinstall/rollback assets. Plus `pip download` of the rns lockfile closure into `cache/packages/python/` (offline venv rebuild path). Every cached artifact hash-recorded in the receipt.
6. **Stage:** fetch release payload tarball → `releases/<ver>.tmp-<pid>/`, verify sha256 + extracted-tree manifest, fsync, rename to `releases/<ver>/`; flip `current` symlink (single atomic unit — KB rides inside the payload, so binary+KB can never split-brain). Prune to exactly 2 releases, only after probation passes (never delete an open probation's revert target).
7. **Supervise:** create `node111` system user (dialout/uucp by `getent`); write unit + distro drop-in + udev rule; `daemon-reload`; `udevadm control --reload --trigger`; `systemctl restart systemd-logind`-free (LogNamespace needs no global restart); `enable --now`.
8. **Verify:** `/health` 200 within 60s; dashboard reachable on `127.0.0.1:11180`; socket asserted loopback-bound; receipt written; print npub + dashboard URL + `sudo -u node111 111 identity backup` hint.
9. **Offer (dismissible):** standard tier = small local model. v1 semantics: accepting records intent in `state/offer.json` + `/api/status` field and downloads **nothing** — Layer 3 replaces the hook. Maximal tier not offered until standard exists.

## Install flow — phone over USB-C

`111 install --phone` runs **as the invoking user** (adb authorization is per-user — never run adb as root; downloads to `~/.cache/111`, maintenance keypair at `~/.local/share/111/keys` 0600). Idempotent state machine with per-step probes; any failed probe re-enters at that step:

1. **Preflight:** adb (distro package — `adb` on Debian/Ubuntu, `android-tools` on Fedora/Arch — with min-version check; fallback sha256-pinned Google platform-tools zip) · exactly one authorized device or `--serial` · SDK ≥24 · arm64-v8a (armv7 refused) · free space · BBK gate via `ro.build.date.utc` (scoped to the documented ColorOS/OxygenOS builds, `--allow-degraded` override) · MIUI/HyperOS detection (`ro.miui.version.code` AND `ro.mi.os.version.code`) with SIM + Mi-account prerequisite check → up-front refusal listing exact steps when "Install via USB" can't be enabled.
2. **Collision check:** `pm path com.termux` + signature dump. The GitHub Termux builds are signed with a **public** test key — signature match authenticates nothing; the manifest's sha256 pins are the only trust anchor. This check exists solely to avoid Android's shared-uid signature conflict: foreign-signature Termux ⇒ hard stop with instructions.
3. **Install APKs** (`adb install -r` ×3, manifest-pinned) → verify versionCode/signature post-install.
4. **Headless channel:** launch TermuxActivity; poll bootstrap-unpack completion (`dpkg -l` works + `$PREFIX/etc` present — never existence-of-files); `run-as com.termux` writes `~/.termux/boot/00-111-node.sh`, pushed payload + prefetched .debs (on-device sha256 against the manifest before `dpkg -i`). **Never** writes `allow-external-apps=true` (no v1 consumer; grants RUN_COMMAND to any on-device app — removed after review). Fallback when run-as fails (E2): chunked ≤4KB `input text` stub with echoed per-chunk sha256, screen kept awake, stub pulls the real payload over `adb reverse` from an HTTP server on the host.
5. **Harden (adb):** `dumpsys deviceidle whitelist +` for `com.termux` AND `com.termux.api` AND `com.termux.boot`; appops `RUN_IN_BACKGROUND`/`RUN_ANY_IN_BACKGROUND`/`SYSTEM_ALERT_WINDOW`; `pm grant … POST_NOTIFICATIONS` (Android 13+); phantom-killer full ordered sequence (Android 12: `device_config set_sync_disabled_for_tests persistent` FIRST, then `max_phantom_processes`; 12L/13/14+: `settings put global settings_enable_monitor_phantom_procs false`) with **readback verification**, re-verified after the reboot test. Documented: OTA/power-down/OEM can silently revert these; re-provisioning is the repair.
6. **Start now:** on-device keygen by the payload binary (host never sees phone key material; sshd **host** keys generated on-device, only the user's authorized_keys pushed); start detached — `setsid nohup … </dev/null >>log 2>&1 &` via run-as, or start runsvdir detached and `sv start` (unplug survival; phantom-fix dependency noted). Termux:Boot owns reboot; sshd (key-only, 8022) is the maintenance channel.
7. **Verify:** `adb forward tcp:11180` + on-device `curl 127.0.0.1:11180/health` via run-as (assert loopback bind; **no LAN poll** — LAN binding is a non-default opt-in); sshd login verified via forward AND on a fresh LAN connection (fresh network, not the install LAN) before any lockdown.
8. **Then, in order:** optional adb-reboot verification test (re-creates the forward after reboot; with PIN lockscreen set, node starts at first **unlock** — BFU/FBE reality, recorded in the receipt) → print per-OEM checklist → **final, labeled point-of-no-return:** `settings put global adb_enabled 0` (scriptable lockdown; stored authorizations persist but are inert while off; re-enabling needs on-device taps; true revocation is a manual Developer-Options step in the checklist). `--keep-adb` escape hatch. Nothing after this step may need adb.
9. **Teardown:** `111 uninstall --phone` works over sshd post-lockdown or adb pre-lockdown: `pm uninstall` ×3, `deviceidle whitelist -` ×3, appops resets.

Phone layout mirrors the host under Termux: payload+releases under `$PREFIX/var/lib/111/releases/`, state `$PREFIX/var/lib/111/` (identity/, state/, cache/), boot script `~/.termux/boot/00-111-node.sh`, runit service `$PREFIX/var/service/111node/`. Phones run `111 daemon` with the same A/B update machinery.

## Detection & category rules (deterministic; one config table)

Facts: arch/cores/RAM/storage (sysinfo) · distro (os_info) · init (`/proc/1/comm`) · virt (systemd-detect-virt else `/.dockerenv`+cgroup+DMI) · battery (`/sys/class/power_supply/BAT*`) · Wi-Fi AP mode (`iw phy`, installed at step 5; absent ⇒ "unknown") · BLE (`/sys/class/bluetooth/hci*`) · LoRa USB (nusb: VID:PID AND product-string — `10c4:ea60` Heltec V3 · `1a86:*` T-Beam · `303a:*` ESP32-S3 native · `239a:4405/0029` T-Echo · `239a:8029` RAK4631; table ships inside the signed release artifact, never an unsigned channel) · GPU (PCI class 0300/0302/0380, vendors 10de/1002/8086) · Pi model (`/proc/device-tree/model`) · internet (named probes). Category re-probes on each daemon start + udev trigger; the recorded category is provisional and updates.

First match wins (thresholds = D9 defaults):

| # | Rule | Category |
|---|---|---|
| 0 | container/WSL without `--allow-degraded` | refuse (systemd-less) / degraded relay (systemd-ful, warned) |
| 1 | discrete GPU (10de/1002) OR (≥8 cores AND ≥32GB RAM) | 4B compute hub |
| 2 | ≥512GB free AND ≥8GB RAM | 4A knowledge hub |
| 3 | LoRa radio AND (AP-capable Wi-Fi OR ethernet); or AP-capable Wi-Fi + ethernet | 3 bridge |
| 4 | Pi 5 ≥8GB | 2B |
| 5 | battery present AND (≥16GB RAM OR any GPU) | 2B |
| 6 | battery present (phones: RAM via `/proc/meminfo`; ≥8GB ⇒ 2B) | 2A / phone-2B |
| 7 | Pi 3/4, or fallback ≥1GB RAM | 1 relay / 2A |
| overlay | internet up at detect time | + 5 gateway capability (recorded; re-evaluated by later layers) |

## Identity (provisional seam — final crypto is Layer 2's call, D9)

- secp256k1 from BIP-39 mnemonic at NIP-06 path, generated at install (RNG seeded — CVE-2025-52464 lesson).
- `identity/` 0700 node111: `secret.hex` 0600 · `npub` 0644 · `mnemonic` 0600 — atomic-trio write, never overwrite.
- Boot integrity check: unreadable/corrupt identity ⇒ quarantine dir, distinct dashboard/`/api/status` error state, **never silent re-key**; regeneration requires explicit `111 identity recover`.
- `sudo -u node111 111 identity backup` displays words; `… identity export` writes NIP-49 `ncryptsec` (interactive passphrase w/ confirmation, refuses pipes). Neither is reachable over HTTP, ever.
- Seam contract: the identity dir is the only key consumer in v1; Layer 2 swap = replace dir + reader.

## On-disk layout (naming normative; ownership is the security boundary)

| Path | Owner | Contents |
|---|---|---|
| `/var/lib/111/releases/<ver>/` | node111 0755 | payload: `bin/111` + `share/kb/` (one atomic unit) |
| `/var/lib/111/current` | node111 | symlink → active release; exactly 2 kept, pruned only after probation |
| `/var/lib/111/identity/` | node111 0700 | key trio |
| `/var/lib/111/state/` | node111 0755 | `update.json` (probation journal) · `floor.json` (monotonic floor) · `facts.json` · `offer.json` |
| `/var/lib/111/cache/packages/` | **root** 0755 | prefetched .debs/.rpms + python wheels (offline reinstall assets; consumed by a privileged `111 admin` flow or documented manual command — never `dpkg -i` by node111 without hash re-verify against the receipt) |
| `/var/lib/111/receipt.json` | **root** 0644 | install receipt; treated as untrusted input on re-read (re-probe, never trust) |
| `/var/lib/111/rns/` | node111 0755 | rnsd `--config` + state (survives A/B swaps) |
| `/etc/111/` | root; env file 0640 root:node111 | minimal config |
| `/usr/local/bin/111` | root | symlink → `current/bin/111` |

Provisioning host: downloads `~/.cache/111`; maintenance keys `~/.local/share/111/keys` 0600.

## Supervision

**Linux** — `/etc/systemd/system/111node.service`: `Type=notify` · `User=node111` + `SupplementaryGroups=dialout|uucp` (getent-detected) · `AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE` (= bounding set; **stripped in the degraded container drop-in**) · `NoNewPrivileges=true` · `ProtectSystem=strict` (StateDirectory=111 keeps `/var/lib/111` writable — the ONLY writable tree; updates stage/flip inside it) · `ProtectHome=true` `PrivateTmp=true` · `PrivateDevices=false` (LoRa hotplug) · `Restart=always` `RestartSec=5` · **`StartLimitIntervalSec=0`** (never give up permanently; bad releases are handled by probation, not burst caps) · `WatchdogSec=30` — the daemon heartbeats `WATCHDOG=1` ≤15s **unconditionally from a dedicated task** (health gates the dashboard/API, never the heartbeat); update windows extend the watchdog via `sd_notify WATCHDOG_USEC=` and restore after READY=1 · `After=network.target` only · `LogNamespace=111node` (100M cap on the namespace — no host-global journald mutation) · udev rule `ID_MM_DEVICE_IGNORE` for known LoRa IDs (+ brltty conflict documented). No sudoers, no polkit. Layer 9 self-restart primitive: voluntary `exit(0)`.

**Android** — Termux:Boot script (`termux-wake-lock` → profile → runsvdir) + runit `111node` (sleep-throttle guard) + cronie `/health` check (placeholder seam until Layer 9). Boot-start on lockscreen phones begins at first unlock (FBE); on Android 15+ best-effort (termux-boot #259).

**Non-systemd distros:** clean refusal with docs pointer.

## Dashboard v1 (deliberately minimal — real dashboard is Layer 4)

Single binary-embedded page, **strictly GET/read-only**, on `127.0.0.1:11180` (LAN binding = explicit `111 config` opt-in with warning until Layer 4 auth; Host-header allowlist `127.0.0.1`/`localhost` against DNS rebinding; identity material never on any endpoint). Shows: npub, category, version, KB sha, transport state (`no transport`/`isolated`/`joined`/`transport degraded`), uptime/health, last-successful-update-check, clock-fault state when present, standard-tier offer (intent-recording stub). `/health` (200/503 per the component checklist) + `/api/status` (JSON; schema pinned in TASKS.md — it is the Layer 4/9 seam surface) on the same port. No KB browsing in v1. "Mesh joined" = rnsd child up AND ≥1 AutoInterface peer within 60s when another node exists on the LAN (asserted for Linux nodes; phones run rnsd but multicast receive needs a MulticastLock no CLI can acquire — phones may sit at `isolated` until the v1.1 helper APK; accepted scope, D9 follow-up).

## Transport (conditional, Bundle B)

When python3+venv exist post-step-5: `/var/lib/111/venvs/rns` built with `pip install --require-hashes -r rns-lock.txt` (hash-locked closure, lockfile sha256 in the signed manifest); `rnsd` as supervised child of `111 daemon` (env `HOME=/var/lib/111`, `--config /var/lib/111/rns`), AutoInterface on, exponential-backoff restart with cap, sustained crash-loop ⇒ dashboard `transport degraded`. Venv integrity check at daemon start; rebuild from `cache/packages/python/` when torn. 111d regenerates rnsd config on netlink interface-change events; config carries a `schema-version` field Layer 6 bumps on takeover. **rns identity is independent of node identity in v1.** No python3 ⇒ detection-only stub (dashboard `no transport`, receipt records the degrade branch). meshtasticd never auto-installed.

## Updates & rollback

- **Manifest:** `channel-111-stable.toml` (manifest-version, date, channel, release, `min-upgrade-from`, `[pkg.*.target.*]` url+sha256+size, APK + Termux-deb + rns-lock + KB sections, keychain) + `.sha256` + detached **minisign**. Verify key compiled into the binary (installer AND daemon). **Keychain rotation before first signed ship:** new keys signed by old, monotonically increasing generation numbers, carried in the signed manifest; root key in cold custody; compromise recovery = re-anchor via a new shim/installer release. Pubkey published in repo + docs + release notes (never extractable-only-from-the-artifact).
- **Notice:** internet only in v1 (mesh gossip arrives with Layer 7); exponential backoff + jitter; last-successful-check on the dashboard; clock-fault class surfaces instead of silent TLS death (minisign verify is time-free — fetches degrade, verification doesn't).
- **Monotonic floor:** `state/floor.json` persists {highest release, highest manifest date, highest keychain generation}; the updater refuses anything lower **regardless of what the incoming manifest says** (replayed old-but-valid signatures included). Node111-writable in v1 (defends external replay; privileged-helper hardening is v1.1).
- **Probation (the actor that survives the restart):** before flipping, the updater writes `state/update.json` = {pending release, previous release, absolute deadline}. At **every** daemon boot: journal pending + (READY+health not achieved by deadline) ⇒ flip `current` back + `systemctl reset-failed` + restart — covering crash-before-READY (no timer dependency), READY-but-503, and torn-corrupt releases (startup hash-check of the payload flips to N-1 on mismatch). Health gate = `/health` component checklist (identity loaded · KB resolves · transport child alive-or-deliberately-absent · updater idle), **sustained** across the 60s probation window; watchdog kills during probation count as probation failures; the deadline lives in the journal, not any process lifetime.
- No deltas v1 (Layer 6/7 problem). KB git-sync is Layer 7 (v1: KB inside the release payload). APKs pinned; self-update covers the `111` binary only; APK refresh = Layer 9 maintenance task.

## Security & trust model (post-review — states mechanisms, not adjectives)

- **Trust chain:** GitHub Releases = canonical artifact store; shim fetches from it first (own-domain mirror = fallback); shim pins the binary's sha256; the binary embeds the minisign key and verifies the signed manifest before trusting any pin in it; manifest pins authenticate every other artifact (payload, APKs, debs, wheels, KB, platform-tools). **The shim hop itself is TLS-only** — residual risk stated plainly; mitigations: shim sha256+minisig published on the GitHub release page (independent channel), compiled-in minimum installer version.
- **Termux GitHub builds:** debuggable + public test key — anyone with adb authorization can read node keys. Accepted (D9 E1) with: lockdown via `adb_enabled 0` as the final labeled step, on-device keygen (keys never touch the host), no `allow-external-apps`, sshd verified on a fresh LAN before lockdown, documented recovery ceremony (re-enabling adb = on-device taps; true revocation = manual UI step).
- **Key material:** 0600/0700, never logged, never transmitted, never on HTTP; transport keys (X25519) are later layers' (derivation/attestation). Phone maintenance key on the host: `~/.local/share/111/keys` 0600 — its theft = phone shell; recorded.
- **Connectivity probes** hit first-party endpoints only (`install.111.network/204`); zero telemetry = no data collection beyond standard HTTP access logs.
- **`dialout`** grants broad serial access — accepted for single-purpose nodes. **`allow-external-apps` is never set.** Privileged consumption of `cache/packages/` re-verifies hashes against the receipt.
- Root-owned receipt/cache close the node111→root escalation (dropped-.deb and forged-receipt paths).

## Refusals & boundaries (v1)

macOS/Windows (node and provisioning host) · ESP32/LoRa relays · manual Termux-only flow · non-systemd distros · armv7 phones AND armv7l Linux hosts · 32-bit-userspace Pi (proceeds arch-matched with warning; armv7l kernel refuses) · proot-distro · package-repo install · >1 phone without `--serial` · auto-installing meshtasticd · USB LoRa from the phone itself (Layer 6) · postmarketOS (experimental per-device tier, not this spec) · privilege escalation absent (clean message + exact `su -c` line). Every refusal = clear message + docs pointer.

## Pre-build verification gates

**Blockers:** V1 run-as channel end-to-end on Termux v0.118.3 GitHub build, PIN lockscreen set, one Android 15 device (physical) · V2 reboot autostart with PIN set (Pixel + Samsung + Xiaomi; records starts-at-first-unlock) · V3 `pip install --require-hashes rns==1.5.6` in Termux from prefetched debs only (settles the .deb closure list) · V8 Linux end-to-end on Debian 12 + Arch + Fedora VMs (unit start, watchdog quiet, dashboard, uninstall) · V11 sshd reachable after reboot + doze on a fresh LAN.
**Non-blocking:** V4 phantom-setting persistence across reboot (real devices only, readback) · V5 injection fallback (size/corruption/screen-timeout bounds) · V6 MIUI flow incl. SIM-less refusal · V7 Pi 32-bit-userspace detection · V9 BBK 24h idle survival · V10 phone rnsd runs (join asserted Linux-only) · V12 interrupted-unpack resume · V13 16KB emulator exec + Termux bootstrap · V14 adb-unplug survival post-start.
**Fault injection:** F1 kill installer mid-extract → re-run heals · F2 crash-before-READY release → auto-revert · F3 READY-but-503 release → auto-revert · F4 corrupt identity → error state + `recover`, never silent re-key · F5 skewed clock → clock-fault surfaced · F6 stale signed manifest → refusal (incl. on RTC-less Pi) · F7 phone reboot mid-provision → resume after unlock.

## Acceptance criteria (v1 done = all true)

1. Fresh Debian 12 VM (incl. a sudo-absent variant): the one command → running node, dashboard 200 on 127.0.0.1:11180 (asserted loopback), identity 0600/0700, receipt present, `cache/packages` non-empty, `111-uninstall` removes everything host-side (unit, udev rule, user, trees).
2. Re-run: idempotent no-change; an **older** pinned shim on an updated node refuses to downgrade.
3. Two Linux nodes on one LAN: both report `joined` within 60s.
4. 24h two-node soak with update + relay traffic: zero watchdog kills; SIGSTOP-injected hang triggers exactly one within 35s.
5. Update: staged release flips and serves; crash-before-READY and READY-but-503 releases both auto-revert (journal-driven); stale signed manifest refused.
6. Phone (V1-passing device, PIN set): command → dashboard reachable via `adb forward`, sshd maintenance works on a fresh LAN, node starts after reboot+unlock, uninstall --phone tears down.
7. Offline after install (WAN pulled): node runs, dashboard serves, KB snapshot present + integrity-verified at `current/share/kb/`, transport intact (rns rebuilds from cache if the venv is torn).

## Seams for later layers

Layer 2: identity dir swap (above) · Layer 3: offer hook (`state/offer.json` + dashboard element + `/api/status` field) + hub-offload endpoint · Layer 4: dashboard surface/port/auth + `/api/status` schema · Layer 5/6: rnsd child → real transport management (`/var/lib/111/rns`, config schema-version, netlink trigger, `transport_state` enum), radio table · Layer 7: gateway re-evaluation, KB git-sync, manifest gossip · Layer 9: supervision-tree handles, `/health` contract, exit(0)-restart, watchdog heartbeat, per-OEM phone watchdog channel, privileged update helper (v1.1).

Build tasks: `os/bootstrap/TASKS.md` (created at build start per `os/plan/EXECUTION.md`).
