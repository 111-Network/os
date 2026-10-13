#!/bin/sh
# verify/local-smoke.sh — the locally-runnable verification subset (T16).
# Every LOCALLY gate from verify/README.md, in one shot. Exits 0 on all-pass;
# skips are printed (SKIPPED), never silently.
set -u

cd "$(dirname "$0")/.." || exit 1
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
for tc in "$HOME"/.local/toolchains/*/bin; do
  [ -d "$tc" ] && PATH="$tc:$PATH"
done
export PATH

FAILS=0
say() { printf '%s\n' "smoke: $*"; }
gate() { # $1 = name; shift = command
  _name=$1
  shift
  if "$@" >/tmp/smoke-gate.log 2>&1; then
    say "PASS  $_name"
  else
    say "FAIL  $_name (see /tmp/smoke-gate.log)"
    FAILS=$((FAILS + 1))
  fi
}

say "== Layer 1 local verification =="

gate "F1 mid-extract kill heals" cargo test --test install_flow f1_torn_stage_heals -- --quiet
gate "F2 crash-before-READY reverts" cargo test --test helper_guard f2_crash -- --quiet
gate "F3 READY-but-503 reverts" cargo test --test helper_guard f3_ready -- --quiet
gate "F4 corrupt identity quarantines" cargo test --lib identity::tests::corrupt -- --quiet
gate "F5 clock skew refused/surfaced" sh -c 'cargo test --test install_flow clock_skew -- --quiet && cargo test --test updater_stage clock_skew -- --quiet'
gate "F6 stale/replay manifest refused" cargo test --test updater_stage -- --quiet
gate "V5 injection bounds (e2e chunker unit)" cargo test --test phone_flow -- --quiet
gate "V7 shim 32-bit + full shim harness" sh shim/tests/run.sh
gate "cross-build refuses a missing/stale UI" sh scripts/tests/cross-build-ui-gate.sh
gate "V12 interrupted-unpack resume (idempotent install)" cargo test --test install_flow happy_path -- --quiet
# V13: p_align from the existing dist binary, else rebuild first (nested quoting via file)
if [ "$(readelf -lW /tmp/dist-test/111-aarch64-unknown-linux-musl 2>/dev/null | awk '$1=="LOAD"{print $NF; exit}')" != "0x4000" ]; then
  scripts/cross-build.sh --allow-no-ui 2026.10.12 /tmp/dist-test >/dev/null 2>&1
fi
gate "V13 16KB p_align (cross-build assert)" sh -c 'readelf -lW /tmp/dist-test/111-aarch64-unknown-linux-musl | grep -q "0x4000"'
gate "Termux assets dry-run" sh termux/tests/dryrun.sh
gate "full cargo suite (all modules)" cargo test --quiet

# namespace e2e attempt (V8-variant) — SKIP when unshare is unavailable
if command -v unshare >/dev/null 2>&1 && unshare -rm true 2>/dev/null; then
  gate "V8-variant namespace install" sh verify/linux-e2e.sh --namespace
else
  say "SKIPPED V8-variant namespace install (unshare unavailable) — VM runbook in verify/linux-e2e.sh"
fi

# Layer 4 end-to-end, last gate: UI build, `111 daemon` + `111-agent` + stub model
# in a user/net/pid namespace, /api/v1 + LAN pairing + relay + events assertions.
if ! command -v pnpm >/dev/null 2>&1; then
  say "SKIPPED L4 interface smoke (pnpm not found — the UI cannot be built) — run ../interface/verify/smoke.sh where pnpm exists"
elif ! unshare -rn true 2>/dev/null; then
  say "SKIPPED L4 interface smoke (unprivileged unshare unavailable) — run ../interface/verify/smoke.sh where it is"
else
  gate "L4 interface smoke (../interface/verify/smoke.sh)" bash ../interface/verify/smoke.sh
fi

echo
if [ "$FAILS" -eq 0 ]; then
  say "ALL LOCAL GATES GREEN"
  exit 0
else
  say "$FAILS gate(s) failed"
  exit 1
fi
