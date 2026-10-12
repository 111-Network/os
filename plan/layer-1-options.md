# Layer 1 (Bootstrap) — Options Brief

Status: 🧭 awaiting user decisions
Source: 7-scout research sweep, 2026-10-12 (k3-256k fleet, workflow `wf_bd54415f-7fa`, 190 tool calls, all load-bearing claims live-verified against scripts/docs that day) + completeness critic.
Full raw findings: workflow `wf_bd54415f-7fa` journal. This file condenses them into decisions.

## Locked scope (user, 2026-10-12) — not relitigated

- **Targets:** (a) Linux laptops/servers/Raspberry Pi (x86_64 + aarch64); (b) Android phones provisioned over USB-C from a computer running the same one command. NOT v1: manual Termux-only flow, ESP32/LoRa relays, macOS, Windows.
- **End state:** running minimal node — identity generated, core services up, dashboard reachable, mesh joined if a network is present.
- **Offline prefetch:** MINIMAL (packages, binaries, KB snapshot, rollback assets). After the node is running it OFFERS an optional standard tier (small local model). Maximal deferred until standard is reached.

## What research converged on (all 7 scouts independently)

Two-stage installer: a tiny POSIX-sh **shim** (function-wrapped, executes only on its last line, artifact sha256 pinned inside — uv/mise pattern) that downloads ONE **musl-static Rust binary** which does the real work (probe → categorize → keygen → install → supervise). The node itself is ONE self-supervising binary (`111d`): identity + embedded dashboard + supervisor + self-updater + KB fetcher (Syncthing is the existence proof for this shape). Identity is **born secp256k1** (Curve25519 keys can never convert to secp256k1 — the Nostr anchor requirement decides it at birth). Layout is FHS-clean with atomic version switching (`/opt/111/releases/<ver>` + `current` symlink). Updates are full signed artifacts behind a rustup-v2-style TOML manifest + minisign, with health-gated auto-rollback. Phones are provisioned by the same command over adb: Termux GitHub APKs + Termux:Boot + runit, with exactly **3 unavoidable user taps** (Developer Options, USB debugging, RSA allow).

---

## Decision A — Installer delivery architecture

**Recommend A1: version-pinned shim on own domain → sha256-pinned static binary from GitHub Releases + own-domain mirror.**

- Shim: POSIX sh, `set -u`, zero top-level code (anti-truncation — Sandstorm 2015 class), detects kernel/arch/libc/Android, downloads + verifies the real installer binary. Pinned per release (`/v0.1.0/install.sh`); `latest` redirects to newest pinned shim.
- Installer binary: hardware probe, category assignment, keygen, package prefetch, service bring-up, staging, writes `/var/lib/111/receipt.json` (Determinate model), drops `111-uninstall.sh` on disk (k3s model). Idempotent: hash-match skips (k3s), skip-if-newer (mise).
- Telemetry: **none** — every studied installer except Homebrew/Determinate ships zero; offline-first ethos forbids it.
- v1.1: GitHub artifact attestations (`gh attestation verify`, sigstore bundles).

| Option | Verdict |
|---|---|
| **A1: shim + static binary, dual-host (recommended)** | Strongest verified anatomy: pinned hashes catch Codecov-class server compromise; function-wrap kills truncation; mirrors survive hosting failure; shim reviewable, logic testable in Rust |
| A2: package-repo configurator (tailscale/docker pattern) | REJECT — needs sudo + live apt/dnf forever, violates offline-after-install, ~40-distro matrix, no uninstall |
| A3: rustup pattern (TLS-only shim, no artifact verification) | REJECT the verification posture (decade-old known gap, rustup #241/#2028); adopt only its binary-centric split |
| A4: own-domain-only hosting | REJECT as sole channel — polyfill.io 2024 domain-takeover risk, no provenance anchor; own domain = shim URL + mirror, GitHub Releases = canonical store |

## Decision B — Implementation language (detail — reviewed in spec, not polled)

**Rust, one musl-static binary per arch (x86_64, aarch64).** Unanimous across scouts: k256 (NCC-audited BIP-340), sysinfo/os_info/nusb (pure-Rust probing, musl-safe), axum+rust-embed dashboard (~3–8MB total binary vs ~45MB caddy), minisign-verify in-process, self_update crate optional. Go: 2nd choice (8–15MB binaries, btcec schnorr fine, but no audited-identity-lib advantage and weaker USB/sysfs story). Shell+python: rejected for the node core (runtime deps, no auditability); a ~150-line POSIX-sh detection fallback ships as debug tool.

## Decision C — Minimal node composition ("what runs when the command exits")

**Recommend C2 (Bundle B): kernel + conditional live transport.**

| Bundle | Contents | Footprint | Verdict |
|---|---|---|---|
| C1 "bare kernel" | `111d` (identity, tokio supervision tree, axum dashboard + `/health`, minisign self-updater, KB fetcher) + KB tarball snapshot + mesh detection-stub | ~15MB disk / ~25MB RAM | Viable floor; but fakes "mesh joined if present" — fallback behavior only |
| **C2 "kernel + conditional transport" (recommended)** | C1 + when python3 exists: `rnsd` 1.5.6 pinned in `/opt/111/venvs/rns`, run as supervised child of 111d, AutoInterface on (zero-config LAN peer discovery, no radio hardware needed); meshtasticd/radios detection-only, reported in dashboard | ~35MB disk / ~60MB RAM | Real mesh-join on any LAN day one; degrades cleanly to C1 without python3; meshtasticd never auto-installed (revoked 2.8 alpha, useless without LoRa hardware) |
| C3 "full edge stack" | per-component systemd units, caddy dashboard, rnsd + meshtasticd unconditional | 3–4× | REJECT — non-portable to Android (no systemd), dashboard dead weight ripped out at Layer 4, empty unit shells for unbuilt layers |

## Decision D — Identity v1 (born secp256k1, Layer 2 adopts without re-keying)

**Recommend D1: generate BIP-39 mnemonic at install (non-interactive), derive node key at NIP-06 path `m/44'/1237'/0'/0/0`; store words 0600; OFFER to display/export post-install (folds into the standard-tier offer moment).**

- Keygen: `k256` 0.14 `schnorr` (NCC Group audit 2023-08-25, findings fixed); keypair generated at install time after system RNG seeding (Meshtastic CVE-2025-52464 lesson — never at first boot).
- On disk: `/var/lib/111/identity/` (0700, service user) — `secret.hex` 0600 (64-char hex, canonical — every Nostr lib ingests it), `npub` 0644, `mnemonic` 0600. Re-runs refuse to overwrite (identity is idempotent).
- Encrypted backup: NIP-49 `ncryptsec` export on demand (`111 identity export`); `nsec` never written unprompted.
- ⚠️ NIP-06 is struck-through "unrecommended" in the NIPs index as of 2026-10 — that tag targets client login UX, not seed backup; still implemented by nostr-tools/nak/wallets. Recorded, accepted.

| Option | Verdict |
|---|---|
| **D1: mnemonic-born key, display deferred to post-install offer (recommended)** | Zero install interaction; paper-backup UX preserved; Layer 2 reads the same 32-byte secret via nostr-sdk — no re-keying either way |
| D2: raw random key + ncryptsec only | Simpler; loses deterministic-restore paper backup forever (cannot reverse a key into words) |
| D3: show 12 words during install | Best backup hygiene; but the only interactive step in a "zero configuration" command |
| D4: ed25519/age identity | REJECT — Curve25519 → secp256k1 conversion is mathematically impossible; permanently breaks the Nostr anchor |

## Decision E — Phone-over-USB-C provisioning path

**Recommend E1: GitHub-build (debuggable) Termux APKs + `run-as com.termux` headless channel.** All three APKs (termux-app v0.118.3, Termux:Boot v0.8.1, Termux:API v0.53.0) pinned with sha256 from ONE signature source (GitHub) — mixing sources = `INSTALL_FAILED_SHARED_USER_INCOMPATIBLE`.

- The command drives: preflight `adb devices` (else prints 3-step guide) → `adb install -r` ×3 → headless bootstrap poll → `run-as` writes boot scripts, `termux.properties` (`allow-external-apps=true`), sshd keys, payload + prefetched .debs → adb hardening (doze whitelist, appops, per-version phantom-killer fixes) → start now via run-as, survive reboot via Termux:Boot → verify via `adb forward` + LAN poll. Maintenance channel = Termux sshd (adb-tcpip doesn't survive reboot unrooted).
- Node binary: ONE static musl aarch64 build run natively (no proot — 10–44% penalty).
- ⚠️ **Security consequence to accept explicitly:** debuggable builds mean anyone holding adb authorization can read node keys. Mitigations: installer revokes USB-debugging authorizations post-install; hardening moves to later layers.
- Unavoidable taps (physics, not a decision): 3 universal (dev options, USB debugging, RSA allow); MIUI/HyperOS adds Mi-account-gated "Install via USB" + one tap per APK — installer detects `ro.miui.version.code` and pre-warns; BBK devices preflight `ro.build.date` (pre-Nov-2024 OxygenOS/ColorOS SIGKILL Termux outright).

| Option | Verdict |
|---|---|
| **E1: GitHub debuggable APKs + run-as (recommended)** | Only route where one command drives ~95% headless; run-as gives full Termux-home control; verified working on v0.118.3 (TERMUX_IS_DEBUGGABLE_BUILD=1) |
| E2: release APKs + `input text` keystroke injection | Universal (no debuggable-build risk) but fragile UI automation across OEM keyboards/focus; keep as fallback when run-as fails |
| E3: own tiny provisioner APK granted RUN_COMMAND | Cleanest permission model; but a second artifact to build/sign from day one — revisit v1.1 |
| postmarketOS | Experimental per-device tier only (SDM845 class), opt-in with bootloader unlock; never default — Xiaomi quota-gates unlocks (1 device/account/yr since 2025) |

## Decision F — Layout, manifest & update channel (detail — reviewed in spec, not polled)

- **Layout:** `/opt/111/releases/<ver>/` payload + `/opt/111/current` symlink (atomic switch, keep exactly 2 releases = rollback assets) · `/var/lib/111/` state (identity/, cache/packages/, kb/ git repo) · `/etc/111/` config · `/usr/local/bin/111` symlink. `~/.local/share` REJECTED on node (per-user, no pre-login services); `~/.cache/111` only on provisioning hosts.
- **Manifest:** rustup-v2-inspired TOML per channel (manifest-version, date, channel, release, `min-upgrade-from` downgrade floor, `[pkg.*.target.*]` url+sha256+size, KB section) + `.sha256` + detached **minisign** (Ed25519 pubkey compiled into 111d; verifies fully offline — cosign/Rekor needs network, rejected for v1; GPG rejected — rustup's decade of pain). **Key-rotation chain (new key signed by old, carried in manifest) is designed BEFORE the first signed release** — a lost key bricks offline updates for deployed nodes.
- **Updates:** node learns of ~2KB manifests via internet or mesh gossip; fetches full signed artifacts (no deltas v1 — LoRa-constrained deltas are a Layer 6/7 problem), stages to `releases/<new>/`, flips symlink, restarts, health-gated auto-revert (`/health` within 60s else revert; KB `<sha>` dirs roll back with the binary).
- **Install-time prefetch:** on-target `apt-get download` closure (or `--download-only`) → `/var/lib/111/cache/packages/`; dnf equivalent via `dnf5 download` + createrepo_c `file://` repo. KB snapshot: GitHub codeload tarball at pinned SHA → `/var/lib/111/kb/` (fallback if repo ever goes private: KB ships inside the release artifact); KB sync post-install via git pull through gateways, incremental `git bundle` over sneakernet/mesh (KB repo must stay git-lfs-free forever — bundles silently drop LFS).

## Decision G — Device-category rule set (proposed defaults — confirm or adjust)

Detection: direct `/proc`+`/sysfs` reads inside the binary (sysinfo + os_info + nusb; **never** the libudev crate — breaks static linking), shell-out to `iw phy` for AP mode (absent ⇒ "unknown", never assume), systemd-detect-virt opportunistically, HTTP-204 probes ×2 + DNS + default route for connectivity. LoRa radios: VID:PID AND product-string match (bridge chip ≠ board — CP2102 is on thousands of devices); table lives in field-updatable config.

Precedence (first match wins), thresholds are scout-proposed placeholders:

| Rule | Category |
|---|---|
| container/WSL/proot | warn + cap at degraded relay (no net admin) |
| discrete GPU (NVIDIA/AMD) OR (≥8 cores AND ≥32GB RAM) | **4B** compute hub |
| ≥512GB free storage AND ≥8GB RAM | **4A** knowledge hub |
| LoRa radio AND (AP-capable Wi-Fi OR ethernet) — or dual-transport without LoRa | **3** bridge |
| battery present: ≥16GB RAM OR any GPU (incl. Intel iGPU, llama.cpp Vulkan path) ⇒ **2B**, else **2A** | terminal |
| fallback ≥1GB RAM | **1** relay |
| internet up at detect time | + **5** gateway capability (recorded; layers ≥2 re-evaluate dynamically) |

Pi special-cases: Pi 5 8GB ⇒ 2B candidate; Pi 3/4 ⇒ 2A or relay. Phone-side categorization happens on-device via adb getprop.

## Supervision shape (detail — reviewed in spec, not polled)

- **Linux:** ONE systemd system unit `111node.service` — dedicated static user `node111` (name can't start with a digit — Debian NAME_REGEX), `AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE`, `SupplementaryGroups=dialout` (installer-detected `uucp` on Arch — systemd fails the whole unit on a missing group), `StateDirectory=111node` 0700, `Type=notify` + `WatchdogSec=30` with real-health-gated `sd_notify`, `Restart=always` + `StartLimit 120s/5`, `After=network.target` ONLY (never `network-online.target` — offline-first), journald capped 100M (Pi SD wear), udev rule stopping ModemManager/brltty grabbing LoRa serial. User-service+linger REJECTED (can't hold capabilities). DynamicUser REJECTED (breaks stable key ownership). Root REJECTED. Zero sudoers/polkit — Layer 9's self-restart primitive = voluntary `exit(0)` under `Restart=always`.
- **Non-systemd (Alpine/OpenRC, Void, s6):** v1 refuses cleanly with a docs pointer — all v1 targets are systemd-default; the niche is shrinking (KDE Plasma 6.6 requires logind).
- **Android:** Termux:Boot (`~/.termux/boot/00-111-node.sh`: wake-lock → profile → runsvdir) + termux-services runit `111node` (crash restart; sleep-throttle guard — runit has no backoff) + cronie `/healthz` one-liner as placeholder seam until Layer 9. Boot-start on Android 15+ treated as best-effort (termux-boot #259); installer offers an adb-reboot verification test + prints per-OEM checklist (MIUI autostart toggle etc. — not scriptable).

## Scope boundaries (v1 refusals, recorded)

macOS/Windows · ESP32/LoRa relays (different delivery shape — firmware flash) · manual Termux-only flow · non-systemd distros (clean refusal) · proot-distro (10–44% penalty) · package-repo installer · GPG signing · telemetry of any kind · auto-installing meshtasticd · USB LoRa from the Android phone itself (per-device permission dialog adb can't pre-grant — flagged to Layer 6).

## Deferred (recorded, not forgotten)

Delta updates for LoRa-constrained nodes (Layer 6/7) · cosign/GitHub artifact attestations v1.1 · per-OEM physical device test matrix (build-time gate) · postmarketOS experimental tier · own provisioner APK (E3) · systemd-creds TPM2 key sealing · polkit rule for Layer 9 host ops · Shizuku watchdog channel for OEM-killed phones · `Proroot` (zero-overhead ptrace) watch · dashboard LAN-auth model (Layer 4) · 32-bit-userspace-on-64-bit-kernel Pi detection edge (getconf LONG_BIT probe in shim).

## Critic gaps

_(appended after the completeness-critic pass lands)_
