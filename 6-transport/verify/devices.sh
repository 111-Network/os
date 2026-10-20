#!/usr/bin/env bash
# 6-transport/verify/devices.sh — Layer 6 gate G5 (spec §16): the device core.
#
#   bash 6-transport/verify/devices.sh                 runs what it can, SKIPs the rest
#   REQUIRE_G5=1 bash 6-transport/verify/devices.sh    lock run: a missing prerequisite or a TODO step is a FAIL
#
# First cut (C4):
#   1. the device core's tests (cargo test --test devices_core): uevent fixtures kick the tick and the 2-tick
#      debounce holds, port_busy names a port guard (ModemManager/brltty rule, a held fd), PortTable collisions
#      among rnsd/111d/meshmud/an identify probe, a foreign loopback holder caught by the pre-bind check,
#      ownership from interfaces.toml; the identify sequence against a fake rnodeconf, a stub meshmud and a
#      MeshCore pty stub (unknown only after three NoMatch, a missing tool is Error, consent, the 30 s rate limit
#      on a fake clock, timeouts, the offer); the applier's four actions with injected probe failures and the
#      byte-exact revert, region_unset, a crash mid-apply, unsupported_action, the two one-shots;
#   2. the module tests (cargo test --lib devices: uevent parser, the real uevent socket binds unprivileged,
#      the rnodeconf text parser, the KISS deframer, the action vocabulary);
#   3. the uevent fixtures checked independently of the Rust parser (the kernel's NUL-separated shape);
#   4. the unit keeps AF_NETLINK whenever it restricts address families (§18 Layer 1);
#   5. TODO, hardware (external gate G1, the owner's bench): serial open without an unwanted reset under
#      rnodeconf, meshmud and the MeshCore probe on a Heltec V3 and a T-Beam; real hotplug on bare Linux; a real
#      RNode answering rnodeconf --info; a MeshCore KISS modem answering GetVersion/GetIdentity; ModemManager or
#      brltty grabbing a CP210x tty. Listed as SKIPPED here; under REQUIRE_G5=1 each is a FAIL.
#
# CARGO_TARGET_DIR and CARGO_BUILD_JOBS are taken from the environment. The Meshtastic step test needs python3
# (NODE111_SKIP_PYTHON_TESTS=1 skips it visibly). Nothing of ours stays running: the test binaries stop their
# children and stubs; the scratch dir is removed on exit.
set -u
cd "$(dirname "$0")/../.." || exit 1
. 6-transport/verify/lib.sh
REPO=$(pwd)

command -v cargo >/dev/null 2>&1 || skip_or_fail G5 "cargo is not installed"

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/111-devices.XXXXXX") || { fail "mktemp"; finish G5; }
cleanup() { rm -rf "$SCRATCH"; }
trap cleanup EXIT
LOG="$SCRATCH/cargo.log"

# ---- 1. the device core's acceptance tests
if (cd "$REPO/1-bootstrap" && cargo test -q -p node111 --test devices_core) >"$LOG" 2>&1; then
  ok "cargo test --test devices_core: $(grep -E '^test result:' "$LOG" | tail -1)"
else
  tail -n 40 "$LOG" >&2
  fail "cargo test --test devices_core"
fi

# ---- 2. the module tests
if (cd "$REPO/1-bootstrap" && cargo test -q -p node111 --lib devices) >"$LOG" 2>&1; then
  ok "cargo test --lib devices: $(grep -E '^test result:' "$LOG" | tail -1)"
else
  tail -n 40 "$LOG" >&2
  fail "cargo test --lib devices"
fi

# ---- 3. the uevent fixtures, independently (header ACTION@DEVPATH, then KEY=VALUE, NUL-separated)
FIX="$REPO/1-bootstrap/crates/node111/tests/fixtures/uevent"
want_uevent() { # FILE ACTION SUBSYSTEM
  f="$FIX/$1"
  if [ ! -f "$f" ]; then fail "uevent fixture $1 is missing"; return; fi
  fields=$(tr '\0' '\n' <"$f")
  head=$(printf '%s\n' "$fields" | head -n 1)
  case "$head" in
    "$2@"/*) ;;
    *) fail "$1: header $head is not $2@<devpath>"; return ;;
  esac
  if printf '%s\n' "$fields" | grep -qx "ACTION=$2" && printf '%s\n' "$fields" | grep -qx "SUBSYSTEM=$3"; then
    ok "uevent $1: $2 on $3"
  else
    fail "$1: not ACTION=$2 SUBSYSTEM=$3"
  fi
}
want_uevent usb-add.uevent add usb
want_uevent usb-bind.uevent bind usb
want_uevent tty-add.uevent add tty
want_uevent tty-remove.uevent remove tty
want_uevent usb-remove.uevent remove usb
want_uevent net-add.uevent add net
want_uevent usb-change.uevent change usb
if (cd "$REPO/1-bootstrap" && cargo test -q -p node111 --test devices_core uevent) >"$LOG" 2>&1; then
  ok "the uevent cases: $(grep -E '^test result:' "$LOG" | tail -1)"
else
  tail -n 40 "$LOG" >&2
  fail "the uevent cases"
fi

# ---- 4. AF_NETLINK kept where the unit restricts address families
for unit in "$REPO"/1-bootstrap/systemd/*.service; do
  line=$(grep -E '^RestrictAddressFamilies=' "$unit" || true)
  if [ -z "$line" ]; then
    ok "$(basename "$unit"): no RestrictAddressFamilies (AF_NETLINK allowed)"
  elif printf '%s' "$line" | grep -q 'AF_NETLINK'; then
    ok "$(basename "$unit"): $line"
  else
    fail "$(basename "$unit") restricts address families without AF_NETLINK: $line"
  fi
done

# ---- 5. the hardware steps (external gate G1)
todo() { # STEP
  if [ "${REQUIRE_G5:-0}" = 1 ]; then
    fail "TODO hardware: $* (not run)"
  else
    note "TODO hardware: $* — SKIPPED"
  fi
}
todo "serial open without an unwanted reset under rnodeconf, meshmud and the MeshCore probe (Heltec V3, T-Beam)"
todo "real hotplug on bare Linux: plug and unplug published within the kicked tick"
todo "a real RNode answering rnodeconf --info (the text parser is unverified on hardware)"
todo "a MeshCore KISS modem answering GetVersion and GetIdentity"
todo "ModemManager or brltty holding a CP210x tty reads port_busy naming it"

finish G5
