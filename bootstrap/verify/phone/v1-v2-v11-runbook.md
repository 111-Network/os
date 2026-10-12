# Phone verification runbook (V1, V2, V3, V11 blockers + V4–V7, V9, V10, V14)

The phone gates need physical devices (this environment has none). They are
spec BLOCKERS for V1/V2/V3/V11 — run before the first signed release and record
outcomes in the release notes.

## Common preconditions

- A host with the built `111` (or the release-candidate shim) and adb
- The Termux APK bundle from the pinned manifest (termux-app v0.118.3,
  Termux:Boot v0.8.1, Termux:API v0.53.0, GitHub builds)
- Developer Options → USB debugging enabled on the device

## V1 — run-as channel end-to-end (BLOCKER)

Device: one Android 15 physical device, PIN lockscreen set.
1. `111 install --phone --reboot-test` on the host
2. Expect: exactly 3 taps (Developer Options, USB debugging, RSA authorize)
   + one install-confirm per APK
3. Assert: run-as channel used (log line; E2 fallback NOT taken), on-device
   sha256 verified before `dpkg -i`, dashboard reachable via
   `adb forward tcp:11180 tcp:11180` → http://127.0.0.1:11180
4. Assert: `allow-external-apps` absent from termux.properties
5. Assert: lockdown applied as the FINAL labeled step (adb dead afterwards;
   `--keep-adb` variant leaves adb on)

## V2 — reboot autostart with PIN (BLOCKER)

Devices: Pixel + Samsung + Xiaomi (one each).
1. After V1, reboot the phone (PIN set)
2. Assert: node is NOT running before first unlock (FBE/BFU reality — record)
3. Unlock once → within 60s: dashboard answers via forward
4. Record "starts at first unlock" per device in the release notes

## V3 — rns in Termux from prefetched debs (BLOCKER; settles .deb closure)

1. On a V1-provisioned phone, `su` is NOT needed: via run-as/sshd
2. `python3 -m venv /tmp/rnsv && /tmp/rnsv/bin/pip install --require-hashes -r /data/data/com.termux/files/usr/var/lib/111/cache/packages/python/rns-lock.txt --no-index --find-links /data/data/com.termux/files/usr/var/lib/111/cache/packages/python`
3. Must succeed with the prefetched wheels only; any missing wheel/hash =
   closure-list bug — FIX the deb/wheel closure list in the release manifest
4. `/tmp/rnsv/bin/rnsd --version` runs

## V11 — sshd after reboot + doze on a fresh LAN (BLOCKER)

1. Provisioned phone, lockscreen PIN, lockdown applied (adb off)
2. Reboot, unlock, let the phone doze 30 min on battery
3. Move the phone to a FRESH network (not the install LAN — e.g. hotspot)
4. `ssh -p 8022 -i ~/.local/share/111/keys/id_ed25519 <phone-ip>` from a host
   on that network → shell opens, node healthy
5. `111 uninstall --phone` over sshd tears down (node tree gone)

## Non-blocking gates

- V4 phantom persistence: after V2, `settings get global
  settings_enable_monitor_phantom_procs` (12L+) or `device_config get
  activity_manager max_phantom_processes` (12) still applied; document OEM reverts
- V5 injection bounds: force E2 (break run-as), verify ≤4KB chunks, echoed
  sha256 mismatch aborts, screen-timeout handled (screen kept awake), stub
  pulls payload over adb reverse only
- V6 MIUI: full flow on one MIUI device (Mi account + SIM present); SIM-less
  Xiaomi refused up front (already unit-tested) with the exact prerequisite list
- V7 Pi 32-bit userspace: aarch64 kernel + armhf userspace → warning +
  arch-matched binary; armv7l kernel → refusal (shim cases already unit-tested)
- V9 BBK 24h idle: provisioned BBK device idles 24h plugged in; node alive,
  health green, no phantom kills in `dumpsys activity lru`
- V10 phone rnsd runs: `pgrep -f rnsd` on device; `transport_state` not
  `no transport`; join assertion is Linux-only (two Linux nodes reach `joined`
  ≤60s — assert in V8 with two VMs)
- V14 unplug survival: start node via V1, unplug USB mid-run; node + sshd alive
  10 min later
- F7 mid-provision reboot: reboot the phone during `111 install --phone`
  (e.g. at hardening), re-run after unlock — host-side ProvisionState markers
  resume at the failed step (unit-tested; assert the log shows `[skip]` for
  completed steps)
