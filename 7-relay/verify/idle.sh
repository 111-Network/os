#!/usr/bin/env bash
# 7-relay/verify/idle.sh — Layer 7 gate G7 extended (spec §15 G7, §3.7; row V1): the no-timer gate.
#
#   bash 7-relay/verify/idle.sh                 runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G7=1 bash 7-relay/verify/idle.sh    lock run: a missing prerequisite is a FAIL
#   IDLE_INTERNET=1 bash 7-relay/verify/idle.sh the check_internet = true pass (§3.7 step 4, last sentence)
#
# `111 daemon` in a fresh network namespace (loopback plus a veth with no default route; user + net + pid
# namespaces, `unshare -rnpf`, so everything dies with the namespace), every Layer 7 switch off, and
# `updates.check_internet = false` (`state/relay.json`), no browser, no SSE client, no peer. After a settle time:
#
#   window 1  wakeups  the summed `voluntary_ctxt_switches` over /proc/<pid>/task/*/status of 111d and of its
#                      child processes (the agent; the harness stands in a `sleep` for 111-agent, as
#                      6-transport/verify/root.sh does), at t = 0 and t = IDLE_WINDOW s. No tracer runs here
#                      (strace -f ptrace-stops every thread and adds the switches it would count, S16, I11).
#                      A thread that exits inside the window takes its count with it (the figure can only read
#                      low); a thread born inside it counts in full.
#   window 2  outbound frames leaving the veth (6-transport/verify/egress-log.py sniff, the primary; a
#                      positive control proves it sees an off-link datagram first) plus a once-a-second
#                      sampling of /proc/<pid>/net/tcp and tcp6 for non-loopback sockets.
#                      Window 2 also runs `strace -f -e trace=connect,sendto,sendmsg` on every thread of 111d: with no
#                      route a connect() fails ENETUNREACH before any frame or socket exists, so the syscall is the only
#                      proof of an attempt. Link-scope (ARP, mDNS, ff02::) and in-subnet frames count as outbound too.
#   window 3  attribution: on a failure the strace lines of window 2 name the caller. The run also fails when the
#                      daemon is dead, when relay.json was moved aside, or when daemon.log carries "updater: degraded".
#
# Pass: outbound = 0 and wakeups <= 1.10 x baseline-wakeups.txt; while that file holds no number the measured
# rate is a `note:` and the run passes on outbound alone. With IDLE_INTERNET=1 (`check_internet = true`) the
# run puts strace on the daemon from its start and every non-loopback connect() must be port 443 (the
# manifest bases, N/daemon/updater/fetch.rs DEFAULT_MANIFEST_BASE and FALLBACK_MANIFEST_BASE, both https) or a
# resolver lookup (port 53); this box has no route, so the connects are the attempts.
#
# Env: BIN111 (the `111` binary; else $CARGO_TARGET_DIR/{release,debug}/111) · IDLE_SETTLE (30 s) · IDLE_WINDOW
# (60 s) shorten a dev run only (the lock run uses the defaults) · TMPDIR for the scratch root. Nothing of
# ours stays running; the scratch root is removed at the end (IDLE_KEEP=1 keeps it).
set -u
export LC_ALL=C
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=7-relay/verify/lib.sh disable=SC1091
. 7-relay/verify/lib.sh

EGRESS=$L7_OS/6-transport/verify/egress-log.py
BASELINE=$L7_VERIFY/baseline-wakeups.txt
SETTLE=${IDLE_SETTLE:-30}
WINDOW=${IDLE_WINDOW:-60}
VETH=$L7_G7_VETH
PEER=$L7_G7_VETH_PEER
CONTROL_DST=192.0.2.1 # RFC 5737, off-link: the positive control's target

# ------------------------------------------------------------------ inner: runs inside the namespaces
inner() {
  local work=${G7_WORK:?} bin=${G7_BIN:?}
  local root=$work/root
  local dpid="" spid="" sniff_pid=""
  local mode=local
  [ "${IDLE_INTERNET:-0}" = 1 ] && mode=internet

  cleanup_inner() {
    local p
    for p in ${sniff_pid:-} ${spid:-} ${dpid:-}; do kill "$p" 2>/dev/null; done
    for p in ${sniff_pid:-} ${spid:-} ${dpid:-}; do wait "$p" 2>/dev/null; done
  }
  trap cleanup_inner EXIT

  ok "namespaces: user + net + pid, /proc is this namespace's own"
  ip link set lo up
  if ip link add "$VETH" type veth peer name "$PEER" &&
    ip addr add "$L7_G7_IP/24" dev "$VETH" && ip addr add "$L7_G7_PEER_IP/24" dev "$PEER" &&
    ip link set "$VETH" up && ip link set "$PEER" up; then
    ok "veth $VETH $L7_G7_IP/24 (peer $PEER $L7_G7_PEER_IP/24), no default route"
  else
    fail "cannot create the veth pair"
    return
  fi
  if [ -n "$(ip route show default)" ]; then fail "a default route exists"; fi

  # ---- the scratch node root: identity, a stand-in agent, check_internet per mode
  g6_make_root "$root" ""
  local nid
  nid=$(g6_identity "$root" "$bin")
  if [ -n "$nid" ]; then ok "identity generated ($nid)"; else fail "no identity (see $root/identity.log)"; fi
  mkdir -p "$root/state"
  if [ "$mode" = local ]; then
    printf '{"schema":1,"updates":{"check_internet":false}}\n' >"$root/state/relay.json"
    chmod 600 "$root/state/relay.json"
  fi
  # every other Layer 7 switch is at its default, off (no fabric/settings, no relay.json keys beyond the above)

  # ---- sniffer helpers: one logger per window (a summary per run)
  sniff_start() { # NAME
    local n=$1
    : >"$work/$n.err"
    python3 -I "$EGRESS" sniff --iface "$VETH" --allow "$L7_G7_IP,$L7_G7_PEER_IP" --control-dst "$CONTROL_DST" \
      --out "$work/$n.jsonl" --summary "$work/$n.summary.json" --ready "$work/$n.ready" >"$work/$n.err" 2>&1 &
    sniff_pid=$!
    for _ in $(seq 1 50); do [ -s "$work/$n.ready" ] && return 0; sleep 0.1; done
    return 1
  }
  SNIFF_RES="? ? ? ?"
  sniff_stop() { # NAME -> sets SNIFF_RES to "<off_link_out> <control_out> <link_out> <peer_out>" (not a subshell: it must wait)
    kill -TERM "$sniff_pid" 2>/dev/null
    wait "$sniff_pid" 2>/dev/null
    sniff_pid=""
    SNIFF_RES=$(python3 -I -c 'import json,sys
c = json.load(open(sys.argv[1]))["counts"]
print(c.get("off_link_out", 0), c.get("control_out", 0), c.get("link_out", 0), c.get("peer_out", 0))' "$work/$1.summary.json" 2>/dev/null) || SNIFF_RES="? ? ? ?"
  }

  # ---- positive control: the logger must see an off-link datagram (a temporary route and neighbour entry)
  if sniff_start control; then
    ip route add default dev "$VETH" && ip neigh add "$CONTROL_DST" lladdr 02:00:00:00:00:01 dev "$VETH" nud permanent
    python3 -I -c 'import socket,sys
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.sendto(b"111-g3-control idle", (sys.argv[1], 9))' "$CONTROL_DST" 2>/dev/null
    sleep 1
    ip neigh del "$CONTROL_DST" dev "$VETH" 2>/dev/null
    ip route del default dev "$VETH" 2>/dev/null
    local c
    sniff_stop control
    c=$SNIFF_RES
    c=$(printf '%s' "$c" | awk '{ print $2 }')
    if [ "$c" -ge 1 ] 2>/dev/null; then
      ok "positive control: the logger on $VETH saw its own off-link datagram to $CONTROL_DST"
    else
      fail "positive control: the logger saw no control datagram (counts '$c')"
    fi
  else
    fail "the AF_PACKET logger did not bind $VETH ($(head -c 200 "$work/control.err"))"
  fi
  if [ -n "$(ip route show default)" ]; then fail "the default route was not removed after the control"; fi

  # ---- start the daemon (internet mode: under strace from its first instruction)
  if [ "$mode" = internet ]; then
    command -v strace >/dev/null 2>&1 || {
      fail "IDLE_INTERNET=1 needs strace"
      return
    }
    NODE111_ROOT=$root strace -f -qq -e trace=connect -o "$work/strace-start.log" -- "$bin" daemon >"$work/daemon.log" 2>&1 &
  else
    NODE111_ROOT=$root "$bin" daemon >"$work/daemon.log" 2>&1 &
  fi
  spid=$!
  dpid=$spid
  local up=0
  for _ in $(seq 1 150); do
    if (exec 3<>"/dev/tcp/127.0.0.1/$L7_PORT_DASH") 2>/dev/null; then
      up=1
      break
    fi
    kill -0 "$spid" 2>/dev/null || break
    sleep 0.2
  done
  if [ "$up" != 1 ]; then
    fail "111 daemon never listened on 127.0.0.1:$L7_PORT_DASH"
    sed 's/^/  daemon| /' "$work/daemon.log" | tail -20
    return
  fi
  if [ "$mode" = internet ]; then
    dpid=$(pgrep -P "$spid" -x 111 | head -n 1)
    [ -n "$dpid" ] || dpid=$spid
  fi
  ok "111 daemon up (pid $dpid, check_internet = $([ "$mode" = internet ] && echo true || echo false))"

  # a daemon that exited is not an idle daemon: checked after the settle and after each window (finding: a dead
  # 111d reads 0 wakeups and 0 outbound)
  alive() { # LABEL -> 0 when the daemon runs and has threads
    if kill -0 "$dpid" 2>/dev/null && [ -n "$(ls /proc/"$dpid"/task 2>/dev/null)" ]; then return 0; fi
    fail "the daemon is not running $1 (pid $dpid)"
    sed 's/^/  daemon| /' "$work/daemon.log" | tail -20
    return 1
  }

  # no browser, no SSE client, no peer: nothing of ours but the daemon and its children
  ok "settling ${SETTLE} s, nothing attached"
  sleep "$SETTLE"
  alive "after the settle" || return

  # ---- window 1: wakeups (no tracer; in internet mode strace is attached from the start, so it is not asserted)
  vcs() { # PID... -> "pid:tid count" lines, every thread of each
    local p t c
    for p in "$@"; do
      for t in /proc/"$p"/task/*; do
        c=$(awk '/^voluntary_ctxt_switches:/ { print $2 }' "$t/status" 2>/dev/null) || continue
        if [ -n "$c" ]; then printf '%s:%s %s\n' "$p" "${t##*/}" "$c"; fi
      done
    done
  }
  children() { # every descendant pid of $1
    local queue=("$1") q c
    while [ ${#queue[@]} -gt 0 ]; do
      q=${queue[0]}
      queue=("${queue[@]:1}")
      # shellcheck disable=SC2013
      for c in $(cat /proc/"$q"/task/*/children 2>/dev/null); do
        printf '%s\n' "$c"
        queue+=("$c")
      done
    done
  }
  local plist
  plist="$dpid"
  if [ "$mode" != internet ]; then plist="$dpid $(children "$dpid" | tr '\n' ' ')"; fi
  local a b
  # shellcheck disable=SC2086
  a=$(vcs $plist)
  sleep "$WINDOW"
  # shellcheck disable=SC2086
  b=$(vcs $plist)
  alive "at the end of window 1" || return
  [ -n "$a" ] || fail "window 1: no thread was readable at t = 0"
  [ -n "$b" ] || fail "window 1: no thread was readable at t = $WINDOW s"
  local wake rate nth
  wake=$(awk 'NR == FNR { a[$1] = $2; next } { d = ($1 in a) ? $2 - a[$1] : $2; if (d > 0) s += d } END { print s + 0 }' \
    <(printf '%s\n' "$a") <(printf '%s\n' "$b"))
  rate=$(awk -v w="$wake" -v s="$WINDOW" 'BEGIN { printf "%.1f", w * 60 / s }')
  nth=$(printf '%s\n' "$b" | grep -c .)
  note "window 1: $wake voluntary context switches in ${WINDOW} s over $nth threads of $(echo "$plist" | wc -w) processes = $rate wakeups/minute"
  local base
  base=$(grep -v '^[[:space:]]*#' "$BASELINE" 2>/dev/null | grep -Eo '^[[:space:]]*[0-9]+(\.[0-9]+)?' | head -n 1 | tr -d '[:space:]')
  if [ "$mode" = internet ]; then
    note "window 1 not asserted in the internet pass (strace is attached from the start)"
  elif [ -z "$base" ]; then
    note "baseline-wakeups.txt holds no number: measured $rate wakeups/minute, passing on outbound alone"
  elif awk -v r="$rate" -v b="$base" 'BEGIN { exit !(r <= 1.10 * b) }'; then
    ok "wakeups $rate/min <= 1.10 x baseline $base"
  else
    fail "wakeups $rate/min exceed 1.10 x baseline $base"
  fi

  # ---- window 2: outbound (the logger, tcp sampling, and strace on connect/sendto: a connect() that fails with
  # ENETUNREACH on this route-less box builds no frame and leaves no socket for a once-a-second sample, so only
  # the syscall shows an attempt; it is also window 3's attribution)
  local out2 off ctl lnk prr tcpn
  if [ "$mode" = internet ]; then
    ok "window 2 skipped in the internet pass: the connect targets below are the assertion"
    sleep "$WINDOW"
  else
    local sniff_ok=1 tr_pid="" pl=() t
    sniff_start win2 || {
      fail "the logger did not bind for window 2"
      sniff_ok=0
    }
    : >"$work/strace-attr.log"
    for t in /proc/"$dpid"/task/*; do pl+=(-p "${t##*/}"); done
    if command -v strace >/dev/null 2>&1; then
      timeout "$WINDOW" strace -f -qq -e trace=connect,sendto,sendmsg -o "$work/strace-attr.log" "${pl[@]}" >/dev/null 2>"$work/strace.err" &
      tr_pid=$!
    else
      fail "strace is absent: connect() attempts cannot be observed"
    fi
    tcpn=$(python3 -I - "$dpid" "$WINDOW" 2>"$work/tcp.err" <<'PY'
import sys, time
pid, secs = sys.argv[1], int(sys.argv[2])
LO4 = "0100007F"
LO6 = {"00000000000000000000000001000000", "0000000000000000FFFF00000100007F"}
seen = set()
def rows(name):
    try:
        with open("/proc/%s/net/%s" % (pid, name)) as f:
            next(f)
            for line in f:
                p = line.split()
                yield p[1], p[2], p[3]
    except OSError:
        return
end = time.monotonic() + secs
while time.monotonic() < end:
    for loc, rem, st in rows("tcp"):
        if st != "0A" and (loc.split(":")[0] != LO4 or rem.split(":")[0] != LO4):
            seen.add(("tcp", loc, rem))
    for loc, rem, st in rows("tcp6"):
        if st != "0A" and (loc.split(":")[0] not in LO6 or rem.split(":")[0] not in LO6):
            seen.add(("tcp6", loc, rem))
    time.sleep(1)
print(len(seen))
for s in sorted(seen):
    print(" ", s, file=sys.stderr)
PY
    )
    [ -n "$tr_pid" ] && wait "$tr_pid" 2>/dev/null
    out2="? ? ? ?"
    if [ "$sniff_ok" = 1 ]; then
      sniff_stop win2
      out2=$SNIFF_RES
    fi
    read -r off ctl lnk prr <<<"$out2"
    alive "at the end of window 2" || return
    local nsys
    # a non-loopback AF_INET/AF_INET6 target of connect/sendto/sendmsg, ENETUNREACH included
    nsys=$(grep -E 'sa_family=AF_INET6?' "$work/strace-attr.log" 2>/dev/null |
      grep -Ev 'sin_addr=inet_addr\("127\.|sin6_addr=inet_pton\(AF_INET6, "::1"\)' | grep -c .)
    [ "$nsys" -ge 0 ] 2>/dev/null || nsys='?'
    note "window 2: ${WINDOW} s, off-link frames out on $VETH = $off (control frames $ctl), link-scope frames out = $lnk, in-subnet frames out = $prr, non-loopback tcp sockets sampled = ${tcpn:-?}, non-loopback connect/send attempts = $nsys"
    local bad=0
    [ "$off" = 0 ] && [ "$lnk" = 0 ] && [ "$prr" = 0 ] && [ "${tcpn:-?}" = 0 ] && [ "$nsys" = 0 ] || bad=1
    # the setting must have taken effect: an invalid relay.json is moved aside and defaults (check_internet = true) apply
    if compgen -G "$root/state/relay.json.invalid-*" >/dev/null; then
      fail "state/relay.json was rejected and moved aside (defaults apply, check_internet = true)"
      bad=1
    fi
    if grep -q '111 updater: degraded' "$work/daemon.log" 2>/dev/null; then
      fail "the updater tried the internet with updates.check_internet = false: $(grep -m1 '111 updater: degraded' "$work/daemon.log")"
      bad=1
    fi
    if [ "$bad" = 0 ]; then
      ok "outbound = 0 (no off-link, link-scope or in-subnet frame on $VETH, no non-loopback socket, no connect/send attempt)"
    else
      [ "$nsys" = 0 ] || fail "outbound attempts by the daemon: $nsys non-loopback connect/send syscalls"
      { [ "$off" = 0 ] && [ "$lnk" = 0 ] && [ "$prr" = 0 ]; } || fail "outbound frames on $VETH: off-link $off, link-scope $lnk, in-subnet $prr"
      [ "${tcpn:-?}" = 0 ] || fail "non-loopback TCP sockets sampled: ${tcpn:-?}"
      head -5 "$work/win2.jsonl" 2>/dev/null | sed 's/^/  off-link| /'
      sed 's/^/  tcp| /' "$work/tcp.err" 2>/dev/null | head -5
      note "window 3: attribution (strace connect/sendto/sendmsg over ${WINDOW} s):"
      grep -E 'sa_family=AF_INET6?' "$work/strace-attr.log" 2>/dev/null | head -10 | sed 's/^/  connect| /'
    fi
  fi

  # ---- the internet pass: every non-loopback connect() is a manifest base (443) or a resolver lookup (53)
  if [ "$mode" = internet ]; then
    local bad n443
    bad=$(grep 'connect(' "$work/strace-start.log" 2>/dev/null |
      grep -E 'sa_family=AF_INET6?' | grep -Ev 'sin6?_addr=(inet_addr\("127\.|inet_pton\(AF_INET6, "::1")' |
      grep -Ev 'sin6?_port=htons\((443|53)\)')
    n443=$(grep 'connect(' "$work/strace-start.log" 2>/dev/null | grep -c 'htons(443)')
    note "internet pass: $n443 connect() to port 443 recorded"
    if [ -z "$bad" ]; then
      ok "internet pass: every non-loopback connect() targets port 443 (the two manifest bases) or a resolver (53)"
    else
      fail "internet pass: a connect() outside the two manifest bases:"
      printf '%s\n' "$bad" | head -5 | sed 's/^/  connect| /'
    fi
  fi

  # ---- stop what we started
  cleanup_inner
  trap - EXIT
  ok "daemon and logger stopped"
}

if [ "${1:-}" = --inner ]; then
  inner
  printf 'FAILS=%d\n' "$FAILS" >"${G7_WORK:?}/inner.result"
  exit 0
fi

# ------------------------------------------------------------------ outer
for tool in python3 ip unshare awk; do
  command -v "$tool" >/dev/null 2>&1 || skip_or_fail G7 "$tool is absent"
done
have_netns || skip_or_fail G7 "unprivileged network namespaces are not available (unshare -Urnm)"
unshare --user --map-root-user --net --pid --fork --mount-proc true >/dev/null 2>&1 ||
  skip_or_fail G7 "a pid namespace with its own /proc is not available (unshare --pid --fork --mount-proc)"
BIN111=${BIN111:-}
if [ -z "$BIN111" ]; then
  for d in "${CARGO_TARGET_DIR:-}" 1-bootstrap/target; do
    [ -n "$d" ] || continue
    for prof in release debug; do
      if [ -x "$d/$prof/111" ]; then
        BIN111=$d/$prof/111
        break 2
      fi
    done
  done
fi
if [ -z "$BIN111" ] || [ ! -x "$BIN111" ]; then skip_or_fail G7 "no built 111 binary (set BIN111 or CARGO_TARGET_DIR)"; fi
BIN111=$(cd "$(dirname "$BIN111")" && pwd)/$(basename "$BIN111")
[ -r "$EGRESS" ] || skip_or_fail G7 "missing $EGRESS"
command -v strace >/dev/null 2>&1 || skip_or_fail G7 "strace is absent (an outbound attempt is visible only as a syscall here)"

WORK=$(mktemp -d "${TMPDIR:-/tmp}/l7-idle.XXXXXX") || exit 1
cleanup() {
  if [ "${IDLE_KEEP:-0}" = 1 ]; then note "kept $WORK"; else rm -rf "${WORK:?}"; fi
}
trap cleanup EXIT

if env --default-signal true 2>/dev/null; then
  RESET_SIG=(env --default-signal)
else
  RESET_SIG=(python3 -c 'import os,signal,sys
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])')
fi
INNER_LOG=$WORK/inner.log
G7_WORK=$WORK G7_BIN=$BIN111 IDLE_SETTLE=$SETTLE IDLE_WINDOW=$WINDOW \
  unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
  "${RESET_SIG[@]}" bash "$0" --inner 2>&1 | tee "$INNER_LOG" || true
inner_fails=$(grep -c '^FAIL:' "$INNER_LOG" || true)
if ! grep -q '^FAILS=' "$WORK/inner.result" 2>/dev/null; then
  fail "the namespace run did not finish (killed or crashed, see above)"
fi
FAILS=$((FAILS + inner_fails))

# hygiene: nothing of this run is left
left=""
for p in /proc/[0-9]*; do
  pid=${p#/proc/}
  [ "$pid" = $$ ] && continue
  if { tr '\0' '\n' <"$p/environ"; } 2>/dev/null | grep -qF "NODE111_ROOT=$WORK" ||
    { tr '\0' '\n' <"$p/cmdline"; } 2>/dev/null | grep -qF "$WORK"; then
    left="$left $pid:$(cat "$p/comm" 2>/dev/null)"
  fi
done
if [ -z "$left" ]; then ok "no daemon, agent stand-in or logger of this run is left"; else fail "left running:$left"; fi
finish G7
