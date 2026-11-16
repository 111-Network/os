#!/usr/bin/env bash
# 9-evolution/verify/record-applier-tape.sh — Layer 9 row P1 (9-evolution/TASKS.md; triage "Order for the build" 3;
# G20 (1)): the Layer 6 applier's tape, recorded BEFORE Layer 8 row D7 moves the decision route.
#
#   bash 9-evolution/verify/record-applier-tape.sh [OUTDIR]     (default OUTDIR: 9-evolution/verify/tapes/applier-l6)
#   (a subset run, P1_CASES, replaces only the tapes of its cases; every case is asserted: HTTP 200, a decision record, kept
#   = applied and not reverted, probe-failed = not applied, reverted, interfaces.toml and rns/config back to their bytes)
#
# What it does. It builds `111` (a debug build, so the NODE111_BRIDGE_STUBS hook is compiled in; the hook is left
# unset, the applier does not use it) from a `git archive` EXPORT of the head it runs at (never the working tree; the
# Layer 6 helpers, fakes, goldens and lock are read from the export too; only this script is of the tree:
# other rows edit the tree while this runs), under a scratch dir with its own target dir. Then, in one unprivileged
# user + pid + mount + network namespace whose /dev and /sys/{bus,class/tty} are this script's own trees and whose
# /run/p1 is a tmpfs (so every path a case writes is the same on every run), it runs TEN cases, each on a fresh
# root, a fresh daemon and a fresh network namespace, and accepts ONE `kind: device` proposal in each on a paired LAN
# session, with Layer 6's device fakes (6-transport/verify/devices.sh: the pty board stand-in, the fake rnodeconf):
#
#   upsert-kept, remove-kept, posture-kept, region-kept       the four applied actions, kept
#   upsert-probe-failed, remove-probe-failed,
#   posture-probe-failed, region-probe-failed                 one FORCED probe failure for each, reverted
#   identify, restart                                         the one-shots
#
# How a probe failure is forced (the production daemon has no hook for it, and none is added): the upsert's board
# has no RNode behind its pty, so rnsd shows the interface Down for the whole 30 s window (the same as G5); the
# remove, the posture and the upsert-kept cases put a wrapper on the rnstatus the daemon runs (the rns venv's
# bin/rnstatus is the gate's own fixture, node.sh), which edits what the daemon READS: it keeps listing the removed
# interface (remove), hides the shared instance (posture), shows the Down interface Up (the one kept upsert; there
# is no RNode on this box); the region case rewrites interfaces.toml's region from outside while the probe polls.
# Each failed case therefore takes the full 30 s window. Nothing else is faked: the real daemon, the real rnsd, the
# real spine and the real applier run.
#
# What it writes. For each case, OUTDIR/<case>.json:
#   {case, recorded_at_commit, proposal (the body accepted), http_status, report (the ApplyReport, as the decision
#    record holds it: `agent/decisions/<id>.json` .applied), decision (the record, `at` normalised),
#    files (path relative to the root, mode, sha256 of what the case leaves: state/interfaces.toml, rns/config with
#    the value of its rpc_key line normalised, the decision record normalised as above)}
# Written with `jq -S`, so a second run at the same head is byte-identical (the check is `diff -r`). No wall clock,
# pid, port or random value is in a tape: the node's identity is generated per case but never recorded, the paths
# are the fixed /run/p1/<case> and the device id is a hash of the board's USB key.
#
# The head must precede Layer 8 row D7's commit (that row moves the decision route; this tape is what proves it did
# not change what the applier does). The script does not and cannot know that: the lead runs it before D7 lands.
#
# Prerequisites: unprivileged user + network + mount namespaces, python3 >= 3.11 with venv, ip, nsenter, curl, jq,
# git, tar, cargo, and PyPI through the proxy for the gate venv (6-transport/verify/rns-venv.sh, removed at the end).
# Env: P1_SCRATCH (scratch parent; default TMPDIR, /tmp), P1_TARGET (cargo target dir; default <scratch>/target),
# P1_SKIP_BUILD=1 with P1_TARGET (reuse 111; the tapes then say recorded_at_commit "unbuilt:<head>"),
# P1_CASES ("a b c": run only these), SMOKE_KEEP=1 (keep the scratch root). Nothing of ours is left running or
# listening: every process dies with the pid namespace or is stopped first.
# shellcheck disable=SC2016,SC2034,SC2317,SC2329,SC1091,SC2012
set -u

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
OS=$(cd "$HERE/../.." && pwd)
export PATH="$HOME/.cargo/bin:$PATH:/usr/sbin:/sbin"
LANIP=192.168.77.5
# Everything but this script is of the recorded commit: the export (git archive HEAD) is made first, and the Layer 6
# helpers, device fakes, goldens and the lock are read from it, never from the working tree other rows edit.
if [ "${1:-}" = --inner ]; then
  EXPORT=${P1_EXPORT:?}
else
  for tool in git tar mktemp; do
    command -v "$tool" >/dev/null 2>&1 || { echo "FAIL: $tool is not installed"; exit 1; }
  done
  P1_COMMIT=$(git -C "$OS" rev-parse HEAD) || { echo "FAIL: not a git checkout"; exit 1; }
  # the scratch root: P1_SCRATCH, else TMPDIR, else /tmp (the lead points it at the session scratchpad)
  SCRATCH=$(mktemp -d "${P1_SCRATCH:-${TMPDIR:-/tmp}}/111-p1.XXXXXX") || { echo "FAIL: mktemp"; exit 1; }
  EXPORT=$SCRATCH/export
  mkdir -p "$EXPORT"
  git -C "$OS" archive HEAD | tar -x -C "$EXPORT" || { echo "FAIL: git archive"; rm -rf "$SCRATCH"; exit 1; }
fi
L6V=$EXPORT/6-transport/verify
FIXD=$L6V/devices
GOLDENS=$EXPORT/1-bootstrap/crates/node111/tests/fixtures/rnodeconf
CASES_ALL="upsert-kept upsert-probe-failed remove-kept remove-probe-failed posture-kept posture-probe-failed region-kept region-probe-failed identify restart"
. "$L6V/lib.sh"
. "$L6V/rns-venv.sh"
. "$L6V/spine/node.sh"

byid() { printf 'usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_P1D%s-if00-port0' "$1"; }
devpath() { printf '/dev/serial/by-id/%s' "$(byid "$1")"; }

# ------------------------------------------------------------------ inner (in the namespaces)

# this namespace's private /dev, /sys trees and the fixed /run/p1 (G5's fake_trees, the host's are never written)
fake_trees() {
  local keep=$WORK/olddev
  mkdir -p "$keep"
  mount --rbind /dev "$keep" || return 1
  mount -t tmpfs -o mode=755 tmpfs /dev || return 1
  local n
  for n in null zero random urandom tty full; do
    touch "/dev/$n" && mount --bind "$keep/$n" "/dev/$n" || return 1
  done
  mkdir -p /dev/pts /dev/shm /dev/serial/by-id
  mount -t devpts -o newinstance,ptmxmode=0666,mode=620 devpts /dev/pts || return 1
  ln -s pts/ptmx /dev/ptmx
  mount -t tmpfs tmpfs /dev/shm
  ln -s /proc/self/fd /dev/fd
  ln -s fd/0 /dev/stdin
  ln -s fd/1 /dev/stdout
  ln -s fd/2 /dev/stderr
  umount -l "$keep"
  mount -t tmpfs tmpfs /sys/bus || return 1
  mount -t tmpfs tmpfs /sys/class/tty || return 1
  mkdir -p /sys/bus/usb/devices /run/p1
  mount -t tmpfs tmpfs /run/p1 || return 1
}
# plug BOARD — the sysfs USB device + interface + tty, /dev/ttyUSB<n> (a pty served by the stub), the by-id link
plug() {
  local i=$1 bus=1-$1 st=$WORK/g5/pty/$1
  local dir=/sys/bus/usb/devices/$bus
  mkdir -p "$dir/$bus:1.0/ttyUSB$i" "$st"
  printf '10c4\n' >"$dir/idVendor"
  printf 'ea60\n' >"$dir/idProduct"
  printf '00\n' >"$dir/bDeviceClass"
  printf 'Silicon Labs\n' >"$dir/manufacturer"
  printf 'CP2102 USB to UART Bridge Controller\n' >"$dir/product"
  printf 'P1D%s\n' "$i" >"$dir/serial"
  printf 'ff\n' >"$dir/$bus:1.0/bInterfaceClass"
  [ -f "$st/mode" ] || printf 'silent\n' >"$st/mode"
  rm -f "$st/ready"
  python3 -I "$FIXD/meshcore-pty.py" --link "/dev/ttyUSB$i" --state "$st" >"$st/stub.log" 2>&1 &
  HELPERS+=("$!")
  PTYPID[$i]=$!
  wait_for 5 '[ -s "$st/ready" ]' || return 1
  mkdir -p "/sys/class/tty/ttyUSB$i"
  ln -s "$dir/$bus:1.0" "/sys/class/tty/ttyUSB$i/device"
  ln -s "../../ttyUSB$i" "/dev/serial/by-id/$(byid "$i")"
}
dev_of() { # BOARD → $DEV
  api N GET /api/v1/devices
  cp "$W/body" "$W/devices.json" 2>/dev/null
  DEV=$(jq -r --arg s "P1D$1" '[.devices[]? | select(tostring | test("\\b" + $s + "\\b"))] | first | .id // empty' "$W/devices.json" 2>/dev/null)
}
lan() { # METHOD PATH [curl args…] — the paired LAN session
  local m=$1 p=$2
  shift 2
  NSX=(nsenter -t "${HOLD[N]}" -n --)
  req "$m" "http://$LANIP:11180$p" --interface "$LANIP" -b "$JAR" "$@"
  NSX=()
}
lanj() { lan "$1" "$2" -H 'Content-Type: application/json' -d "$3" "${@:4}"; }

# the rnstatus the daemon runs: the gate's counting wrapper plus a mode file (<root>/rnstatus-mode) that edits what a
# plain read (`rnstatus --config DIR`, the probe's) returns. Attach, detach and reload calls pass through untouched.
write_rnstatus_wrapper() { # ROOT
  local root=$1
  cat >"$root/venvs/rns/bin/rnstatus" <<EOF
#!/bin/sh
# P1 fixture: the real rnstatus (the gate venv's), its plain read edited per $root/rnstatus-mode
real="$RNS_VENV/bin/rnstatus"
mode=\$(cat "$root/rnstatus-mode" 2>/dev/null)
if [ "\$#" -ne 2 ] || [ "\$1" != --config ] || [ -z "\$mode" ]; then exec "\$real" "\$@"; fi
out=\$("\$real" "\$@") || { rc=\$?; printf '%s\n' "\$out"; exit "\$rc"; }
case "\$mode" in
  up-all) printf '%s\n' "\$out" | sed 's/^\\( *Status *: *\\)Down/\\1Up/' ;;
  noshared) printf '%s\n' "\$out" | awk '/^ [A-Za-z][A-Za-z ]*\\[/ { skip = (\$0 ~ /^ Shared Instance\\[/) } /^ *\$/ { skip = 0 } !skip { print }' ;;
  keep:*) name=\${mode#keep:}
    printf '%s\n' "\$out"
    case "\$out" in *"[\$name"*) ;; *) printf '\n RNodeInterface[%s]\n    Status    : Up\n    Rate      : 1.20 kbps\n' "\$name" ;; esac ;;
  *) printf '%s\n' "\$out" ;;
esac
EOF
  chmod 755 "$root/venvs/rns/bin/rnstatus"
}

# one case: its actions body, the rnstatus mode, the injector (region only)
case_body() { # CASE → $BODY, $MODE, $WANT_STATUS_NOTE
  local d
  BODY=""
  MODE=""
  case $1 in
    upsert-kept | upsert-probe-failed)
      BODY=$(jq -nc --arg d "$BOARD1" '{why: "P1 upsert", actions: [{action: "rns.interface.upsert", device_id: $d, type: "RNodeInterface",
        params: {frequency: 869525000, bandwidth: 125000, txpower: 14, spreadingfactor: 8, codingrate: 5}}]}')
      [ "$1" = upsert-kept ] && MODE=up-all
      ;;
    remove-kept | remove-probe-failed)
      BODY=$(jq -nc '{why: "P1 remove", actions: [{action: "rns.interface.remove", name: "P1 board 2"}]}')
      [ "$1" = remove-probe-failed ] && MODE="keep:P1 board 2"
      ;;
    posture-kept | posture-probe-failed)
      BODY='{"why":"P1 posture","actions":[{"action":"rns.posture.set","enable_transport":true}]}'
      [ "$1" = posture-probe-failed ] && MODE=noshared
      ;;
    region-kept | region-probe-failed)
      BODY='{"why":"P1 region","actions":[{"action":"radio.region.set","region":"US"}]}'
      ;;
    identify)
      BODY=$(jq -nc --arg d "$BOARD1" '{why: "P1 identify", actions: [{action: "identify", device_id: $d}]}')
      ;;
    restart)
      BODY='{"why":"P1 restart","actions":[{"action":"service.restart","service_id":"transport"}]}'
      ;;
  esac
}

# the decision record's raw bytes with only the `at` value replaced (key order, spacing and the trailing newline stay)
norm_decision() { sed 's/"at":"[^"]*"/"at":"NORMALISED"/' "$1"; }
# sha256 of one file as the tape holds it: rns/config's rpc_key value (derived from the per-case identity) and the
# decision record's `at` are normalised, nothing else
file_sha() { # ROOT REL
  case $2 in
    rns/config) sed 's/^\( *rpc_key = \).*/\1NORMALISED/' "$1/$2" | sha256sum | cut -c1-64 ;;
    agent/decisions/*) norm_decision "$1/$2" | sha256sum | cut -c1-64 ;;
    *) sha256sum <"$1/$2" | cut -c1-64 ;;
  esac
}
# the files a case leaves under the root: state/, rns/config, agent/decisions/ (logs, sockets and rns storage are not state)
list_files() { # ROOT
  (cd "$1" && find state agent/decisions rns/config -type f 2>/dev/null | LC_ALL=C sort)
}
# "<rel> <sha>" lines of interfaces.toml and rns/config (the pre-apply snapshot a revert must restore)
snap() { local r; for r in state/interfaces.toml rns/config; do [ -f "$1/$r" ] && printf '%s %s\n' "$r" "$(file_sha "$1" "$r")"; done; }

# write OUTDIR/<case>.json from what the case left
record() { # CASE PID ROOT STATUS
  local c=$1 pid=$2 root=$3 st=$4 rel sha files="[]" dec="null" dfile
  dfile=$root/agent/decisions/$pid.json
  if [ -f "$dfile" ]; then
    dec=$(jq -S '.at = "NORMALISED"' "$dfile")
  fi
  [ -z "${P1_DEBUG_DIR:-}" ] || { cp "$root/rns/config" "$P1_DEBUG_DIR/$c.config" 2>/dev/null; list_files "$root" >"$P1_DEBUG_DIR/$c.files"; }
  while IFS= read -r rel; do
    [ -n "$rel" ] || continue
    [ -f "$root/$rel" ] || continue
    # the three files the applier writes are hashed; the daemon's own state (sessions, pairing, devices, panels,
    # lock) holds tokens and times of the run, so it is listed with its mode only (sha256 null): a new or missing file
    # still shows in a diff
    case $rel in
      state/interfaces.toml | rns/config | agent/decisions/*) sha=$(file_sha "$root" "$rel") ;;
      *) sha="" ;;
    esac
    files=$(jq -c --arg p "$rel" --arg m "$(stat -c %a "$root/$rel")" --arg s "$sha" \
      '. + [{path: $p, mode: $m, sha256: (if $s == "" then null else $s end)}]' <<<"$files")
  done < <(list_files "$root")
  jq -nS --arg c "$c" --arg commit "$P1_COMMIT" --arg body "$BODY" --argjson st "$st" --argjson files "$files" --argjson dec "$dec" \
    '{case: $c, recorded_at_commit: $commit, proposal: ($body | fromjson), http_status: $st,
      report: ($dec.applied // null), decision: $dec, files: $files}' >"$P1_OUT/$c.json"
}

# the outcome a case must have: a tape of a forcing that stopped forcing is a FAIL, not a green run
check_outcome() { # CASE SNAP_BEFORE SNAP_AFTER
  local c=$1 f=$P1_OUT/$1.json want
  [ "$(jq -r '.http_status' "$f")" = 200 ] || { fail "$c: the decision answered HTTP $(jq -r '.http_status' "$f"), not 200"; return 1; }
  [ "$(jq -r '.decision != null and .report != null' "$f")" = true ] || { fail "$c: no decision record with an applied report"; return 1; }
  case $c in
    *-probe-failed)
      want='.report.applied == false and .report.reverted == true'
      [ "$(jq -r "$want" "$f")" = true ] || { fail "$c: the forced probe failure did not happen (report $(jq -c '.report | {applied, reverted, reason}' "$f"))"; return 1; }
      [ "$2" = "$3" ] || { fail "$c: interfaces.toml / rns/config are not back to their pre-apply bytes"; return 1; }
      ;;
    *)
      want='.report.applied == true and .report.reverted == false'
      [ "$(jq -r "$want" "$f")" = true ] || { fail "$c: the apply was not kept (report $(jq -c '.report | {applied, reverted, reason}' "$f"))"; return 1; }
      ;;
  esac
}

run_case() { # CASE
  local c=$1 root=/run/p1/$1 pid=prop-p1-$1 n inj="" st
  W=$WORK/http-$c
  mkdir -p "$W"
  rm -rf "$root"
  ROOT[N]=$root
  JAR=$W/paired.jar
  phase "case $c"
  make_netns N
  if ! { nx N ip link add eth0 type veth peer name eth1 && nx N ip link set eth1 up &&
    nx N ip addr add "$LANIP/24" dev eth0 && nx N ip link set eth0 up; }; then
    fail "$c: the LAN address could not be set up"
    return 1
  fi
  g45_make_root "$root" "# P1: LAN sharing on (the paired session that accepts the proposal)
DASHBOARD_LAN=true"
  write_rnstatus_wrapper "$root"
  : >"$WORK/g5/rnodeconf-calls.log"
  sed -e "s|@G5@|$WORK/g5|g" -e "s|@GOLDENS@|$GOLDENS|g" "$FIXD/fake-rnodeconf.sh" >"$root/venvs/rns/bin/rnodeconf"
  chmod 755 "$root/venvs/rns/bin/rnodeconf"
  cat >"$root/state/interfaces.toml" <<EOF
# P1: board 2 is driven by rnsd (owner = "rnsd"); the region is set so a device upsert renders
schema = 1

[radio]
region = "EU_868"

[[interface]]
name = "P1 board 2"
owner = "rnsd"
type = "RNodeInterface"
port = "$(devpath 2)"
params = { frequency = 869525000, bandwidth = 125000, txpower = 14, spreadingfactor = 8, codingrate = 5 }
EOF
  chmod 600 "$root/state/interfaces.toml"
  NID=$(g45_identity "$root")
  [ -n "$NID" ] || { fail "$c: identity generate failed"; return 1; }
  start_node N
  wait_for 40 '[ -n "$(rnsd_pid N)" ]' || fail "$c: rnsd did not start"
  wait_for 60 'rs N; [ -n "$(rs_block "rns/111")" ]' || fail "$c: the shared instance did not come up"
  wait_for 20 'api N GET /api/v1/devices; jb "[.devices[] | select(.kind == \"radio\")] | length == 2"' || fail "$c: the boards are not listed"
  dev_of 1
  BOARD1=$DEV
  [ -n "$BOARD1" ] || fail "$c: board 1 has no device id"
  NSX=(nsenter -t "${HOLD[N]}" -n --)
  pair_session N "http://$LANIP:11180" "$JAR" --interface "$LANIP"
  NSX=()
  [ "$ST" = 200 ] || fail "$c: pairing answered $ST"
  case_body "$c"
  printf '%s\n' "$MODE" >"$root/rnstatus-mode"
  local before after
  before=$(snap "$root")
  propose_device N "$pid" "$NID" "$BODY"
  if [ "$c" = region-probe-failed ]; then
    # while the probe polls, put the region back to what it was (the write the apply made is overwritten)
    ( end=$((SECONDS + 60))
      while [ "$SECONDS" -lt "$end" ]; do
        if grep -q '^region = "US"' "$root/state/interfaces.toml" 2>/dev/null; then
          sed -i 's/^region = "US"/region = "EU_868"/' "$root/state/interfaces.toml"
          exit 0
        fi
        sleep 0.002
      done ) &
    inj=$!
    HELPERS+=("$inj")
  fi
  REQ_TIMEOUT=150 lanj POST "/api/v1/proposals/$pid/decision" '{"decision": "accepted"}'
  st=$ST
  [ -z "$inj" ] || { kill "$inj" 2>/dev/null; wait "$inj" 2>/dev/null; }
  if [ "$c" = restart ]; then sleep 3; fi
  stop_node N
  [ "$RC" = 0 ] || fail "$c: the daemon did not exit 0 on SIGTERM (rc ${RC:-none})"
  wait_for 15 '[ -z "$(rnsd_pid N)" ]' || fail "$c: rnsd is left"
  after=$(snap "$root")
  record "$c" "$pid" "$root" "$st"
  check_outcome "$c" "$before" "$after" &&
    ok "$c: HTTP $st, applied $(jq -c '.report | {applied, reverted, reason}' "$P1_OUT/$c.json" | head -c 200)"
  kill "${HOLD[N]}" 2>/dev/null
  wait "${HOLD[N]}" 2>/dev/null
  rm -rf "$root"
}

inner() {
  : "${P1_WORK:?}" "${BIN111:?}" "${RNS_VENV:?}" "${P1_OUT:?}" "${P1_COMMIT:?}" "${P1_EXPORT:?}"
  WORK=$P1_WORK
  mkdir -p "$WORK/g5/pty" "$WORK/g5/rnodeconf"
  declare -gA PTYPID
  cleanup_inner() {
    local p
    for p in "${HELPERS[@]}"; do kill -TERM "$p" 2>/dev/null || true; done
    printf 'FAILS=%s\nOKS=%s\n' "$FAILS" "$OKS" >"$WORK/inner.result"
  }
  trap cleanup_inner EXIT
  trap 'exit 130' INT TERM
  ip link set lo up
  fake_trees || { fail "the private /dev, /sys and /run/p1 trees could not be mounted"; return; }
  if ! plug 1 || ! plug 2; then
    fail "the board stand-ins did not come up"
    return
  fi
  printf 'info-provisioned.txt\n' >"$WORK/g5/rnodeconf/P1D1-"
  local c
  for c in ${P1_CASES:-$CASES_ALL}; do
    run_case "$c" || true
  done
  # nothing of ours may be alive in this pid namespace once the helpers are stopped (the daemons, rnsd and the netns
  # holders were stopped by their cases): any process here but this shell is a leak, whatever its command line
  for p in "${HELPERS[@]}"; do kill -TERM "$p" 2>/dev/null || true; done
  local p left=""
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    left=""
    for p in /proc/[0-9]*; do
      p=${p#/proc/}
      [ "$p" = "$$" ] || [ "$p" = "$BASHPID" ] || left="$left $p"
    done
    [ -z "$left" ] && break
    sleep 1
  done
  [ -z "$left" ] || fail "processes left in the namespace:$left"
}

if [ "${1:-}" = --inner ]; then
  inner
  exit $((FAILS > 0))
fi

# ------------------------------------------------------------------ outer
echo "9-evolution/verify/record-applier-tape.sh — Layer 9 row P1: the Layer 6 applier tape"
have_netns || { echo "FAIL: unprivileged network namespaces are not available (unshare -Urnm)"; exit 1; }
for tool in python3 ip nsenter curl jq unshare mount sha256sum cargo stat ss; do
  command -v "$tool" >/dev/null 2>&1 || { echo "FAIL: $tool is not installed"; exit 1; }
done
unshare -Urm sh -c 'mount -t tmpfs tmpfs /sys/class/tty' 2>/dev/null || { echo "FAIL: a mount namespace cannot overmount /sys/class/tty"; exit 1; }
for f in fake-rnodeconf.sh meshcore-pty.py; do
  [ -r "$FIXD/$f" ] || { echo "FAIL: fixture 6-transport/verify/devices/$f is missing"; exit 1; }
done
P1_OUT=${1:-$OS/9-evolution/verify/tapes/applier-l6}
mkdir -p "$P1_OUT" || { echo "FAIL: cannot create $P1_OUT"; exit 1; }
P1_OUT=$(cd "$P1_OUT" && pwd)
CACHE=$SCRATCH/cache
cleanup() {
  [ -z "${RNS_VENV_DIR_P1:-}" ] || rns_venv_remove "$RNS_VENV_DIR_P1"
  if [ "${SMOKE_KEEP:-0}" = 1 ]; then
    echo "note: scratch kept whole at $SCRATCH"
  elif [ "$FAILS" -gt 0 ]; then
    # a failed run keeps its logs only: the export and the target dir (several GB) go
    rm -rf "$SCRATCH/export" "$SCRATCH/target" "$SCRATCH/cache"
    echo "note: logs kept at $SCRATCH"
  else
    rm -rf "$SCRATCH"
  fi
}
trap cleanup EXIT

phase "1. the export of the head, the debug 111"
ok "exported $P1_COMMIT (committed files only) to $EXPORT"
TGT=${P1_TARGET:-$SCRATCH/target}
BIN111=$TGT/debug/111
if [ "${P1_SKIP_BUILD:-0}" = 1 ]; then
  # the binary is not of this export: the tape says so, and no such tape is a recording of the head
  P1_COMMIT="unbuilt:$P1_COMMIT"
  note "P1_SKIP_BUILD=1 — cargo build skipped (the binary is not of this export; tapes carry recorded_at_commit $P1_COMMIT)"
else
  t1=$(now)
  if (cd "$EXPORT/1-bootstrap" && CARGO_TARGET_DIR=$TGT CARGO_INCREMENTAL=0 cargo build --bin 111) >"$SCRATCH/cargo.log" 2>&1; then
    ok "cargo build --bin 111 (debug, from the export; $(since "$t1")s)"
  else
    tail -20 "$SCRATCH/cargo.log" | sed 's/^/  cargo| /'
    fail "cargo build --bin 111"
  fi
fi
[ -x "$BIN111" ] || { echo "FAIL: $BIN111 missing"; exit 1; }

phase "2. the gate venv (rnsd, rnstatus)"
RNS_VENV_DIR_P1=$CACHE/venv-dir
if rns_venv_ensure "$RNS_VENV_DIR_P1"; then
  ok "gate venv from the lock: rns $("$RNS_VENV/bin/pip" show rns 2>/dev/null | sed -n 's/^Version: //p')"
else
  echo "FAIL: $RNS_VENV_ERR"
  exit 1
fi

phase "3. the cases (one fresh daemon each, in the namespaces)"
# only the tapes of the cases about to run are replaced: a subset run never deletes the others
for c in ${P1_CASES:-$CASES_ALL}; do rm -f "$P1_OUT/$c.json"; done
ss -ltnup 2>/dev/null | sort >"$SCRATCH/ss.before"
if env --default-signal true 2>/dev/null; then
  RESET_SIG=(env --default-signal)
else
  RESET_SIG=(python3 -c 'import os,signal,sys
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])')
fi
mkdir -p "$SCRATCH/w"
P1_EXPORT=$EXPORT P1_WORK=$SCRATCH/w P1_OUT=$P1_OUT P1_COMMIT=$P1_COMMIT BIN111=$BIN111 RNS_VENV=$RNS_VENV RNS_LOCK=$RNS_LOCK RNS_LOCKSHA=$RNS_LOCKSHA \
  G45_T0=$T0 P1_CASES=${P1_CASES:-} \
  unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
  "${RESET_SIG[@]}" bash "$HERE/record-applier-tape.sh" --inner 2>&1 | tee "$SCRATCH/inner.log" || true
inner_fails=$(grep -c '^FAIL:' "$SCRATCH/inner.log" || true)
FAILS=$((FAILS + inner_fails))
grep -q '^FAILS=' "$SCRATCH/w/inner.result" 2>/dev/null || fail "the namespace run did not finish"

phase "4. hygiene"
left=""
for p in /proc/[0-9]*; do
  p=${p#/proc/}
  [ "$p" = "$$" ] && continue
  c=$(tr '\0' ' ' <"/proc/$p/cmdline" 2>/dev/null)
  case "$c" in *"$SCRATCH"* | *"$BIN111"* | *meshcore-pty.py* | */run/p1/*) left="$left $p:${c:0:60}" ;; esac
done
ss -ltnup 2>/dev/null | sort >"$SCRATCH/ss.after"
cmp -s "$SCRATCH/ss.before" "$SCRATCH/ss.after" || { left="$left listeners-changed"; diff "$SCRATCH/ss.before" "$SCRATCH/ss.after" | head -5; }
if [ -n "$(ip netns list 2>/dev/null)" ]; then left="$left netns:$(ip netns list | head -c 60)"; fi
if [ -z "$left" ]; then ok "no process of this run is left"; else fail "left:$left"; fi
echo
echo "P1: $(ls "$P1_OUT"/*.json 2>/dev/null | wc -l) tape(s) in $P1_OUT, $FAILS FAIL in $(elapsed)s"
finish P1
