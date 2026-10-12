# Layer 1 verification harness (T16)

Every spec gate V1–V14 / F1–F7 (spec §Pre-build verification gates) has either a
runnable script here or an exact-precondition runbook. Honest status: LOCALLY
means it runs green in CI/dev environments (this box included); EXTERNAL means
it needs hardware/VMs this environment doesn't have — those gates are BLOCKERS
for shipping (spec) and must be run before the first release.

## Matrix

| Gate | What | Status | Where |
|---|---|---|---|
| F1 | kill installer mid-extract → re-run heals | ✅ LOCALLY | `cargo test --test install_flow f1_torn_stage_heals` |
| F2 | crash-before-READY release → auto-revert | ✅ LOCALLY | `cargo test --test helper_guard f2_crash_before_ready_guard_reverts` |
| F3 | READY-but-503 release → auto-revert | ✅ LOCALLY | `cargo test --test helper_guard f3_ready_but_503_helper_reverts_on_daemon_request` |
| F4 | corrupt identity → error state + recover | ✅ LOCALLY | `cargo test --lib identity::tests::corrupt_trio_quarantines_and_never_rekeys` |
| F5 | skewed clock → clock-fault surfaced | ✅ LOCALLY | `cargo test --test install_flow clock_skew_refuses --test updater_stage clock_skew_writes_fault_marker` |
| F6 | stale signed manifest → refusal (incl. RTC-less Pi) | ✅ LOCALLY | `cargo test --test updater_stage` (replay/stale cases) + Pi check in V8 runbook |
| F7 | phone reboot mid-provision → resume | 🔶 RUNBOOK | ProvisionState markers (`cargo test --test phone_flow provision_state_resume_logic`) + physical resume pass in `phone/v1-v2-v11-runbook.md` |
| V1 | run-as end-to-end, Termux v0.118.3, PIN, Android 15 | 🔶 EXTERNAL-BLOCKER | `phone/v1-v2-v11-runbook.md` §V1 |
| V2 | reboot autostart with PIN (Pixel+Samsung+Xiaomi) | 🔶 EXTERNAL-BLOCKER | runbook §V2 |
| V3 | pip --require-hashes rns==1.5.6 in Termux from prefetched debs | 🔶 EXTERNAL-BLOCKER | runbook §V3 (settles the .deb closure list) |
| V4 | phantom persistence across reboot | 🔶 RUNBOOK | runbook §V4 |
| V5 | injection fallback bounds | ✅ LOCALLY (unit) + RUNBOOK | e2e chunker unit (`cargo test --test phone_flow`) + runbook §V5 |
| V6 | MIUI flow incl. SIM-less refusal | ✅ LOCALLY (refusal unit) + RUNBOOK | `cargo test --test phone_flow preflight_miui` + runbook §V6 |
| V7 | Pi 32-bit-userspace detection | ✅ LOCALLY (shim case) + RUNBOOK | `shim/tests/run.sh` (32-bit case) + runbook §V7 |
| V8 | Linux e2e Debian 12 + Arch + Fedora | 🔶 NAMESPACE-variant + EXTERNAL-BLOCKER | `linux-e2e.sh` (unshare variant; VM runbook §V8) |
| V9 | BBK 24h idle survival | 🔶 EXTERNAL | runbook §V9 |
| V10 | phone rnsd runs (join = Linux-only) | 🔶 RUNBOOK | runbook §V10 |
| V11 | sshd reachable after reboot+doze, fresh LAN | 🔶 EXTERNAL-BLOCKER | runbook §V11 |
| V12 | interrupted-unpack resume | ✅ LOCALLY | `cargo test --test install_flow` (resume/idempotent) |
| V13 | 16KB emulator exec + Termux bootstrap | 🔶 EXTERNAL | runbook §V13; p_align assert is in `scripts/cross-build.sh` (LOCALLY ✅) |
| V14 | adb-unplug survival post-start | 🔶 RUNBOOK | runbook §V14 |

## Locally-runnable subset

`verify/local-smoke.sh` — runs every LOCALLY gate above + the shim harness +
Termux dry-run + a namespace install attempt when `unshare` permits. Exits 0
on all-pass (skips are printed, never counted as failures).

## Shipping rule

The spec's BLOCKER gates (V1, V2, V3, V8, V11) must be run on their hardware/VM
targets and recorded in a release note before the first signed release. The
runbooks in `phone/` and `linux-e2e.sh`'s VM section are the exact steps.
