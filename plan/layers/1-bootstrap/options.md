# Layer 1 (Bootstrap) — Options Brief

Status: 🧭 awaiting user decisions
Source: 7-scout research sweep, 2026-10-12 (k3-256k fleet, workflow `wf_bd54415f-7fa`, 190 tool calls, load-bearing claims live-verified against scripts/docs that day) + completeness-critic pass (24 gaps: 3 blocker, 16 should-fix, 5 nice — all resolved below or carried as open decisions).
Full raw findings: workflow `wf_bd54415f-7fa` journal. This file condenses them into decisions.

## Locked scope (user, 2026-10-12) — not relitigated

- **Targets:** (a) Linux laptops/servers/Raspberry Pi (x86_64 + aarch64); (b) Android phones provisioned over USB-C from a computer running the same one command. NOT v1: manual Termux-only flow, ESP32/LoRa relays, macOS, Windows (as node OR as provisioning host).
- **End state:** running minimal node — identity generated, core services up, dashboard reachable, mesh joined if a network is present.
- **Offline prefetch:** MINIMAL (packages, binaries, KB snapshot, rollback assets). After the node is running it OFFERS an optional standard tier (small local model). Maximal deferred until standard is reached.

## What research converged on (all 7 scouts independently)

Two-stage installer: a tiny POSIX-sh **shim** (function-wrapped, executes only on its last line, artifact sha256 pinned inside — uv/mise pattern) that downloads ONE **musl-static Rust binary** which does the real work (probe → categorize → keygen → install → supervise). The node itself is ONE self-supervising binary (`111d`): identity + embedded dashboard + supervisor + self-updater + KB fetcher (Syncthing is the existence proof for this shape). Identity is generated at install as **secp256k1** (NCC-audited path; keeps the README's Nostr-anchor option open — but see Decision D: Layer 2 owns the final crypto call, and v1 identity is a provisional seam). Layout is FHS-clean with atomic version switching (`/opt/111/releases/<ver>` + `current` symlink). Updates are full signed artifacts behind a rustup-v2-style TOML manifest + minisign, with health-gated auto-rollback. Phones are provisioned by the same command over adb: Termux GitHub APKs + Termux:Boot + runit, with exactly **3 unavoidable user taps** (Developer Options, USB debugging, RSA allow).

---

## Decision A — Installer delivery architecture

**Recommend A1: version-pinned shim on own domain → sha256-pinned static binary from GitHub Releases + own-domain mirror.**

- Shim: POSIX sh, `set -u`, zero top-level code (anti-truncation — Sandstorm 2015 class), detects kernel/arch/libc/Android, downloads + verifies the real installer binary. Pinned per release (`/v0.1.0/install.sh`); `latest` redirects to newest pinned shim.
- Privilege shape (critic gap): shim downloads and verifies **as the invoking user**, then re-execs the binary via `sudo` for the install phase. Never `curl | sudo bash` — the unverified part never runs as root.
- Installer binary: hardware probe, category assignment, keygen, package prefetch, service bring-up, staging, writes `/var/lib/111/receipt.json` (Determinate model), drops `111-uninstall.sh` on disk (k3s model). Idempotent: hash-match skips (k3s), skip-if-newer (mise).
- Telemetry: **none** — every studied installer except Homebrew/Determinate ships zero; offline-first ethos forbids it.
- v1.1: GitHub artifact attestations (`gh attestation verify`, sigstore bundles).

| Option | Verdict |
|---|---|
| **A1: shim + static binary, dual-host (recommended)** | Strongest verified anatomy: pinned hashes catch Codecov-class server compromise; function-wrap kills truncation; mirrors survive hosting failure; shim reviewable, logic testable in Rust |
| A2: package-repo configurator (tailscale/docker pattern) | REJECT — needs sudo + live apt/dnf forever, violates offline-after-install, ~40-distro matrix, no uninstall |
| A3: rustup pattern (TLS-only shim, no artifact verification) | REJECT the verification posture (decade-old known gap, rustup #241/#2028); adopt only its binary-centric split |
| A4: own-domain-only hosting | REJECT as sole channel — polyfill.io 2024 domain-takeover risk; own domain = shim URL + mirror, GitHub Releases = canonical store |

## Decision B — Implementation language (detail — reviewed in spec, not polled)

**Rust, one musl-static binary per arch (x86_64, aarch64).** Unanimous across scouts: k256 (NCC-audited BIP-340), sysinfo/os_info/nusb (pure-Rust probing, musl-safe), axum+rust-embed dashboard (~3–8MB total binary vs ~45MB caddy), minisign-verify in-process. Go: 2nd choice (btcec schnorr fine, weaker USB/sysfs story, larger binaries). Shell+python: rejected for the node core; a ~150-line POSIX-sh detection fallback ships as debug tool.

## Decision C — Minimal node composition ("what runs when the command exits")

**Recommend C2 (Bundle B): kernel + conditional live transport.**

| Bundle | Contents | Footprint | Verdict |
|---|---|---|---|
| C1 "bare kernel" | `111d` (secp256k1 identity, tokio supervision tree, axum dashboard + `/health`, minisign self-updater, KB fetcher) + KB tarball snapshot + mesh detection-stub | ~15MB disk / ~25MB RAM | Viable floor; but fakes "mesh joined if present" — fallback behavior only |
| **C2 "kernel + conditional transport" (recommended)** | C1 + when python3 exists: `rnsd` 1.5.6 pinned in `/opt/111/venvs/rns`, supervised child of 111d, AutoInterface on (zero-config LAN discovery, no radio hardware needed); meshtasticd/radios detection-only | ~35MB disk / ~60MB RAM | Real mesh-join on any LAN day one; degrades cleanly to C1 without python3; meshtasticd never auto-installed (revoked 2.8 alpha, useless without LoRa hardware) |
| C3 "full edge stack" | per-component systemd units, caddy dashboard, rnsd + meshtasticd unconditional | 3–4× | REJECT — non-portable to Android, dashboard dead weight ripped out at Layer 4, empty unit shells for unbuilt layers |

**"Mesh joined" acceptance, made measurable (critic gap):** rnsd child up AND ≥1 AutoInterface peer visible within N=60s when another 111 node exists on the same LAN. Dashboard distinguishes three states: `no transport` (no python3/radios), `isolated` (transport up, no peers — e.g. AP isolation), `joined`. AutoInterface fails on multicast-blocked networks — "joined if present" is best-effort, honestly reported.

**Android caveat (critic gap):** Bundle B on Termux is unproven (rns's `cryptography` dep under prefetched Termux .debs). Pre-build spike required — see verification matrix. If it fails, phones ship C1 in v1 and the brief survives.

## Decision D — Identity v1 (provisional seam; final crypto is Layer 2's call)

**⚠️ Coupling (2026-10-12):** the parallel Layer 2 session locked its scope as "crypto picked on merit — **NOT Nostr-locked, Nostr itself not final**". That supersedes the README's Nostr-anchor line as a *hard* requirement. Research still recommends secp256k1 on merit (NCC-audited k256, trivial static builds, keeps the Nostr option open if Layer 7 adopts Nostr), but Layer 1's identity is explicitly a **provisional placeholder**: if Layer 2 picks a different curve, v1 nodes re-key exactly once when Layer 2 lands. The design below keeps that swap cheap (identity dir is the only key consumer in v1).

**Recommend D1: BIP-39 mnemonic generated silently at install, node key derived at NIP-06 path `m/44'/1237'/0'/0/0`; words stored 0600 and displayed only via explicit post-install `111 identity backup` (folds into the standard-tier offer moment).**

- Keygen: `k256` 0.14 `schnorr` (NCC Group audit 2023-08-25, findings fixed); generated at install time after system RNG seeding (Meshtastic CVE-2025-52464 lesson — never at first boot).
- On disk: `identity/` (0700, service user) under the state dir — `secret.hex` 0600 (64-char hex, canonical — every Nostr lib ingests it), `npub` 0644, `mnemonic` 0600. Re-runs refuse to overwrite (identity is idempotent).
- Encrypted export: NIP-49 `ncryptsec` on demand; `nsec` never written unprompted. No TTY is assumed at install (critic gap — curl|bash pipes and headless servers have no operator; display is always a post-install explicit command).
- ⚠️ NIP-06 is struck-through "unrecommended" in the NIPs index as of 2026-10 — targets client login UX, not seed backup; still implemented by nostr-tools/nak/wallets. Recorded, accepted.
- Critic correction absorbed: scout minimal-node-composition wrote "ed25519 node-identity" — wrong; identity is secp256k1 per the Nostr anchor (scout identity-keygen's proof stands: no Curve25519→secp256k1 conversion exists).

| Option | Verdict |
|---|---|
| **D1: mnemonic-born key, silent at install, `111 identity backup` on demand (recommended)** | Zero install interaction; paper-backup UX preserved; Layer 2 reads the same 32-byte secret via nostr-sdk — no re-keying |
| D2: raw random key + ncryptsec only | Simpler; loses deterministic-restore paper backup forever (cannot reverse a key into words) |
| D3: show 12 words during install | Best backup hygiene; impossible non-interactively — curl|bash has no TTY and headless servers have no operator |
| D4: ed25519/age identity | REJECT — Curve25519 → secp256k1 conversion is mathematically impossible; permanently breaks the Nostr anchor |

## Decision E — Phone-over-USB-C provisioning path

**Recommend E1: GitHub-build (debuggable) Termux APKs + `run-as com.termux` headless channel.** All three APKs (termux-app v0.118.3, Termux:Boot v0.8.1, Termux:API v0.53.0) pinned URL+sha256 from ONE signature source (GitHub) — mixing sources = `INSTALL_FAILED_SHARED_USER_INCOMPATIBLE`. (Critic resolved the cross-scout primitive conflict — installer-anatomy's RUN_COMMAND and service-supervision's input-text lose to run-as, the only verified fully-headless channel.)

- The command drives: preflight `adb devices` (else prints 3-step guide) → existing-Termux collision check (`pm path com.termux` + signature dump; foreign signature → hard stop with instructions) → `adb install -r` ×3 → headless bootstrap poll → `run-as` writes boot scripts, `termux.properties`, sshd keys, payload + prefetched .debs → adb hardening (doze whitelist, appops, per-version phantom-killer fixes) → start now via run-as, survive reboot via Termux:Boot → verify via `adb forward` + LAN poll. Maintenance channel = Termux sshd (adb-tcpip doesn't survive reboot unrooted).
- **Provisioning is an idempotent state machine** (critic gap): every step probes first, resumes partial failures, and `111-uninstall` on the host removes the phone side too (`pm uninstall` ×3, `deviceidle whitelist -com.termux`).
- Node binary: ONE static musl aarch64 build run natively (no proot — 10–44% penalty). **Phones are aarch64-only in v1** (armv7 refused with a clear message — critic gap; avoids a third build target).
- Host-side adb tooling (critic gap): prefer distro `android-tools` package with minimum-version check; fallback = sha256-pinned Google platform-tools zip. One phone per run — error if >1 adb device without `--serial`.
- Phone filesystem layout (critic gap, `$PREFIX`-relative mirror of the FHS layout): payload under `~/111/`, state under `$PREFIX/var/lib/111/` (identity/, kb/<sha>, releases/), boot script `~/.termux/boot/00-111-node.sh`, runit service `$PREFIX/var/service/111node/`.
- ⚠️ **Trust model, explicit (critic blocker):** GitHub Termux builds are `debuggable=true` AND signed with a checked-in public test key — anyone holding adb authorization can read node keys. **(a) RECOMMENDED: accept for v1** — installer revokes USB-debugging authorizations post-install and documents the residue; **(b)** own-signed Termux rebuild/provisioner APK — cleaner, adds a build+sign pipeline from day one; revisit v1.1.
- Unavoidable taps (physics, not a decision): 3 universal (dev options, USB debugging, RSA allow); MIUI/HyperOS adds Mi-account-gated "Install via USB" + one tap per APK — installer detects `ro.miui.version.code` and pre-warns; BBK devices preflight `ro.build.date` (pre-Nov-2024 OxygenOS/ColorOS SIGKILL Termux outright).
- APK update channel (critic gap): APKs pinned for v1; node self-updater covers `111d` only; APK refresh is a maintenance task (adb session or manual), revisited at Layer 9. Recorded risk: Termux stable line frozen since 2025-03, targetSdk-28 existential risk (NLnet-funded APKLF redesign in flight).

| Option | Verdict |
|---|---|
| **E1: GitHub debuggable APKs + run-as (recommended)** | Only route where one command drives ~95% headless; verified on v0.118.3 (TERMUX_IS_DEBUGGABLE_BUILD=1) |
| E2: release APKs + `input text` keystroke injection | No debuggable-build risk; fragile UI automation across OEM keyboards/focus — kept as fallback when run-as fails |
| E3: own tiny provisioner APK granted RUN_COMMAND | Cleanest permission model; second artifact to build/sign from day one — revisit v1.1 |
| postmarketOS | Experimental per-device tier only (SDM845 class), opt-in with bootloader unlock; never default — Xiaomi quota-gates unlocks (1 device/account/yr since 2025) |

## Decision F — Layout, manifest & update channel (detail — reviewed in spec, not polled)

- **Naming table (critic gap — unified):** user `node111` (can't start with digit — Debian NAME_REGEX) · payload `/opt/111/releases/<ver>/` + `/opt/111/current` symlink · state `/var/lib/111/` (identity/ 0700, kb/<sha>/ + current symlink, cache/packages/) · config `/etc/111/` · CLI `/usr/local/bin/111` symlink · unit `111node.service` with `StateDirectory=111` (yields `/var/lib/111` owned by node111). Digits fine in dir/unit names, just not the username.
- **KB (critic gap — single model):** v1 = codeload tarball at installer-pinned SHA → `/var/lib/111/kb/<sha>/`, `current` symlink flip (rolls back with the binary; N-1 kept). Git-repo KB sync (pull through gateways, incremental `git bundle` over sneakernet) arrives with Layer 7 — recorded, not built. KB repo stays git-lfs-free forever (bundles silently drop LFS). If the repo is ever private: KB ships inside the release artifact instead.
- **Manifest:** rustup-v2-inspired TOML per channel (manifest-version, date, channel, release, `min-upgrade-from` downgrade floor, `[pkg.*.target.*]` url+sha256+size, KB section) + `.sha256` + detached **minisign** (Ed25519 pubkey compiled into 111d; fully offline — cosign/Rekor needs network, rejected v1; GPG rejected — rustup's decade of pain). **Key-rotation chain designed BEFORE first signed release:** new keys signed by old, keychain carried in the signed manifest, binary trusts keychain root; a lost key otherwise bricks offline updates for deployed nodes.
- **Updates:** node learns of ~2KB manifests via internet or mesh gossip; fetches full signed artifacts (no deltas v1 — LoRa-constrained deltas are a Layer 6/7 problem), stages to `releases/<new>/`, flips symlink, restarts, health-gated auto-revert. **Watchdog/update interaction (critic gap):** updater sets a state marker suppressing WATCHDOG=1 during the flip; the 60s rollback timer starts at first post-update READY=1 — no watchdog-kill mid-update.
- **Dashboard binding (critic gap — one decision):** single port `11180`; localhost-only by default with `adb forward` / ssh tunnel as access paths; LAN binding is an explicit `111 config` opt-in with a warning until Layer 4 auth exists.
- **Install-time prefetch:** on-target `apt-get download` closure (or `--download-only`) → `cache/packages/`; dnf equivalent via `dnf5 download` + createrepo_c `file://` repo. **Minimal package list (critic gap — enumerated):** Debian/Ubuntu/RPi OS: `python3 python3-venv iw ca-certificates`; Fedora: `python3 iw ca-certificates`; Arch: `python iw ca-certificates`. The static binary needs none of them; each is optional and degrades (no python3 ⇒ Bundle C1 behavior; no iw ⇒ Wi-Fi AP capability "unknown").

## Decision G — Device-category rule set (proposed defaults — confirm or adjust)

Detection: direct `/proc`+`/sysfs` reads inside the binary (sysinfo + os_info + nusb; **never** the libudev crate — breaks static linking), shell-out to `iw phy` for AP mode (absent ⇒ "unknown", never assume), systemd-detect-virt opportunistically, HTTP-204 probes ×2 + DNS + default route. LoRa radios: VID:PID AND product-string match (bridge chip ≠ board); table in field-updatable config.

**Linux hosts — precedence (first match wins), thresholds are scout-proposed placeholders:**

| Rule | Category |
|---|---|
| container/WSL/proot | warn + cap at degraded relay (no net admin) |
| discrete GPU (NVIDIA/AMD) OR (≥8 cores AND ≥32GB RAM) | **4B** compute hub |
| ≥512GB free storage AND ≥8GB RAM | **4A** knowledge hub |
| LoRa radio AND (AP-capable Wi-Fi OR ethernet) — or dual-transport without LoRa | **3** bridge |
| battery present: ≥16GB RAM OR any GPU (incl. Intel iGPU, llama.cpp Vulkan path) ⇒ **2B**, else **2A** | terminal |
| fallback ≥1GB RAM | **1** relay |
| internet up at detect time | + **5** gateway capability (recorded; layers ≥2 re-evaluate dynamically) |

Pi special-cases: Pi 5 8GB ⇒ 2B candidate; Pi 3/4 ⇒ 2A or relay. Pi has no RTC — manifest date comparisons are warn-only on nodes that boot offline (critic gap).

**Phones (critic gap — was unowned):** battery always present ⇒ terminal; categorized on-device via adb `getprop`: ≥8GB RAM or flagship-class SoC ⇒ **2B**, else **2A**; USB-host LoRa from the phone flagged to Layer 6 (per-device permission dialog adb can't pre-grant). Phones never take bridge/hub categories in v1.

## Supervision shape (detail — reviewed in spec, not polled)

- **Linux:** ONE systemd system unit `111node.service` — dedicated static user `node111`, `AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE`, `SupplementaryGroups=dialout` (installer-detected `uucp` on Arch), `StateDirectory=111` 0700, `Type=notify` + `WatchdogSec=30` with real-health-gated `sd_notify`, `Restart=always` + `StartLimit 120s/5`, `After=network.target` ONLY (never `network-online.target` — offline-first), journald capped 100M (Pi SD wear), udev rule stopping ModemManager/brltty grabbing LoRa serial. User-service+linger REJECTED (can't hold capabilities). DynamicUser REJECTED (breaks stable key ownership). Root REJECTED. Zero sudoers/polkit — Layer 9's self-restart primitive = voluntary `exit(0)` under `Restart=always`.
- **Non-systemd (Alpine/OpenRC, Void, s6):** v1 refuses cleanly with a docs pointer — all v1 targets are systemd-default; the niche is shrinking (KDE Plasma 6.6 requires logind).
- **Android:** Termux:Boot (`~/.termux/boot/00-111-node.sh`: wake-lock → profile → runsvdir) + termux-services runit `111node` (crash restart; sleep-throttle guard — runit has no backoff) + cronie `/healthz` one-liner as placeholder seam until Layer 9. Boot-start on Android 15+ treated as best-effort (termux-boot #259); installer offers an adb-reboot verification test + prints per-OEM checklist (MIUI autostart toggle etc. — not scriptable).

## Pre-build verification matrix (critic gap — empirical gates before spec claims hold)

| # | Claim to verify | How | Blocks |
|---|---|---|---|
| V1 | `run-as com.termux` works on Termux v0.118.3 GitHub build, bootstrap unpacks headless | one physical phone, adb | E1 |
| V2 | Termux:Boot reboot autostart actually fires | reboot test on Pixel + Samsung + Xiaomi | phone end-state |
| V3 | `pip install rns==1.5.6` in Termux using only prefetched .debs | one-device spike | C2 on phones |
| V4 | phantom-killer adb settings per Android 12/13/14/15/16 | per-version device/emulator pass | phone reliability |
| V5 | `input text` fallback injection works across OEM keyboards | 2–3 OEM devices | E2 fallback |
| V6 | MIUI per-APK install confirmation flow + pre-warning | one Xiaomi device | E1 on MIUI |
| V7 | Pi 32-bit-userspace-on-64-bit-kernel detection (getconf LONG_BIT) | Pi 3/4 with 32-bit OS | detection on Pi |
| V8 | systemd unit starts, watchdog quiet, dashboard reachable, uninstall clean | one Debian + one Arch VM | Linux end-state |

## Open decisions for user (the only items scouts punted)

1. **Category thresholds (Decision G)** — defaults above are scout-invented; confirm or adjust.
2. **armv7 phones** — recommend refusing (aarch64-only); the alternative adds a third build target for pre-2016 devices.
3. **Debuggable-APK trust model (Decision E ⚠️)** — recommend accepting with post-install revocation + docs for v1; the alternative (own-signed Termux build) adds a pipeline.
4. **Identity crypto coupling (Decision D ⚠️)** — confirm secp256k1 as the *provisional* v1 curve (recommended), accepting a one-time re-key if Layer 2's merit-based pick differs; or block Layer 1 on Layer 2's crypto decision.

## Scope boundaries (v1 refusals, recorded)

macOS/Windows (node and provisioning host) · ESP32/LoRa relays · manual Termux-only flow · non-systemd distros (clean refusal) · armv7 phones (clean refusal) · proot-distro · package-repo installer · GPG signing · telemetry of any kind · auto-installing meshtasticd · USB LoRa from the phone itself (Layer 6) · >1 phone per run without `--serial`.

## Deferred (recorded, not forgotten)

Delta updates for LoRa-constrained nodes (Layer 6/7) · cosign/GitHub artifact attestations v1.1 · own provisioner APK (E3) · own-signed Termux rebuild · postmarketOS experimental tier · git-based KB sync through gateways (Layer 7) · APK update channel (Layer 9) · systemd-creds TPM2 key sealing · polkit rule for Layer 9 host ops · Shizuku watchdog channel for OEM-killed phones · `Proroot` (zero-overhead ptrace) watch · dashboard LAN-auth model (Layer 4) · per-category llms-full bundles.
