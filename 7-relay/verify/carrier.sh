#!/usr/bin/env bash
# 7-relay/verify/carrier.sh — Layer 7 gate G10 (spec §15 G10; §5.9, §6.1–§6.4):
#   the ntfy carrier; gateways G and G′ joined only through stub-ntfy.py.
#
#   bash 7-relay/verify/carrier.sh              G10: runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G10=1 bash 7-relay/verify/carrier.sh    lock run: a missing prerequisite is a FAIL
#   bash 7-relay/verify/carrier.sh --smoke      row P1: the prototype's smoke sequence against the stub;
#                                               writes 7-relay/verify/logs/ntfy-smoke-<calver>-stub.md
#
# G10 itself (the two gateway hubs in netns, §15 G10 items 1-7) lands with row V4; until then the
# default path prints SKIPPED (the T0 convention).
#
# --smoke (spec §6.4; row P1): (1) the stub compiles (bytecode under a scratch prefix, never next to
# the source); (2) the stub on its own: started on an ephemeral loopback port, a publish, a poll, a
# 4,097 B body refused with 413, stopped; (3) the smoke sequence: the `smoke_log` test of
# 1-bootstrap/crates/node111/tests/carrier_ntfy.rs, which starts, restarts and kills its own stubs (a
# guard kills them on drop, panics included) and writes the log; (4) the log holds every §6.4 section
# and marks the owner-server items "not run from this box"; (5) nothing of ours is left: no
# stub-ntfy.py process, no listener on the self-check port. The calver is NODE111_BUILD_VERSION when
# a release build stamps one, else the date of the run (UTC). CARGO_TARGET_DIR is honoured.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=7-relay/verify/lib.sh disable=SC1091
. 7-relay/verify/lib.sh

STUB=$L7_VERIFY/stub-ntfy.py

stub_pid=""
scratch=""
cleanup() {
  if [ -n "$stub_pid" ]; then
    kill "$stub_pid" 2>/dev/null
    wait "$stub_pid" 2>/dev/null
  fi
  if [ -n "$scratch" ]; then rm -rf "$scratch"; fi
}

# stub_pids — the pids of python3 processes whose argv runs stub-ntfy.py (not a shell that only
# mentions the name on its command line).
stub_pids() {
  local p
  for p in $(pgrep -f 'stub-ntfy\.py' 2>/dev/null); do
    tr '\0' ' ' 2>/dev/null <"/proc/$p/cmdline" |
      grep -qE '^([^ ]*/)?python3(\.[0-9]+)? (-I )?[^ ]*stub-ntfy\.py( |$)' && printf '%s\n' "$p"
  done
}

# stub_selfcheck — start the stub, publish, poll, 4,097 B ⇒ 413, stop it; sets nothing global but
# stub_pid while it runs.
stub_selfcheck() {
  python3 -I "$STUB" --port 0 --quiet >"$scratch/port" 2>"$scratch/stub.err" &
  stub_pid=$!
  local port=""
  for _ in $(seq 1 50); do
    port=$(head -n 1 "$scratch/port" 2>/dev/null)
    [ -n "$port" ] && break
    sleep 0.1
  done
  case $port in
    "" | *[!0-9]*)
      fail "stub-ntfy.py printed no port ($(head -c 200 "$scratch/stub.err"))"
      return
      ;;
  esac
  ok "stub-ntfy.py bound 127.0.0.1:$port (port 0 asked)"
  local out
  out=$(python3 -I - "$port" <<'EOF'
import json, sys, urllib.error, urllib.request
port = sys.argv[1]
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))  # loopback, never a proxy
base = "http://127.0.0.1:%s" % port
def req(method, path, body=None):
    r = urllib.request.Request(base + path, data=body, method=method)
    try:
        with opener.open(r, timeout=5) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()
s1, b1 = req("POST", "/111selfcheck", b"AQEAAQc")
s2, b2 = req("GET", "/111selfcheck/json?poll=1&since=all")
s3, _ = req("POST", "/111selfcheck", b"a" * 4097)
s4, _ = req("POST", "/111selfcheck", b"a" * 4096)
first = json.loads(b2.decode().splitlines()[0]) if s2 == 200 and b2 else {}
print(s1, s2, s3, s4, first.get("message", "-"), len(first.get("id", "")))
EOF
  )
  if [ "$out" = "200 200 413 200 AQEAAQc 12" ]; then
    ok "stub self-check: publish 200, poll 200 with the message (12-char id), 4,097 B 413, 4,096 B 200"
  else
    fail "stub self-check: got '$out', want '200 200 413 200 AQEAAQc 12'"
  fi
  kill "$stub_pid" 2>/dev/null
  wait "$stub_pid" 2>/dev/null
  stub_pid=""
  if ss -ltnH 2>/dev/null | grep -q "127.0.0.1:$port "; then
    fail "the self-check stub's port $port still listens"
  else
    ok "the self-check stub stopped (nothing listens on $port)"
  fi
}

smoke() {
  command -v python3 >/dev/null 2>&1 || skip_or_fail G10 "python3 is absent (stub-ntfy.py needs it)"
  command -v cargo >/dev/null 2>&1 || skip_or_fail G10 "cargo is absent (the smoke sequence is a node111 test)"
  scratch=$(mktemp -d "/tmp/l7-carrier-smoke.XXXXXX") || exit 1
  trap cleanup EXIT

  # (1) the stub compiles
  if PYTHONPYCACHEPREFIX="$scratch/pyc" python3 -m py_compile "$STUB"; then
    ok "stub-ntfy.py compiles (python3 -m py_compile)"
  else
    fail "stub-ntfy.py does not compile"
  fi

  # (2) the stub on its own
  stub_selfcheck

  # (3) the smoke sequence, which writes the log
  local calver log stamp
  calver=${NODE111_BUILD_VERSION:-$(date -u +%Y.%m.%d)}
  case $calver in
    "" | *[!0-9.]*)
      fail "calver expected (YYYY.MM.DD[.N]), got '$calver'"
      finish G10-smoke
      ;;
  esac
  log=$L7_LOGS/ntfy-smoke-$calver-stub.md
  stamp=$(date -u +%Y-%m-%dT%H:%MZ)
  mkdir -p "$L7_LOGS"
  rm -f "$log"
  note "smoke sequence: cargo test -p node111 --test carrier_ntfy -- --exact smoke_log (calver $calver)"
  if (cd "$L7_OS/1-bootstrap" &&
    NTFY_SMOKE_LOG="$log" NTFY_SMOKE_CALVER="$calver" NTFY_SMOKE_DATE="$stamp" \
      cargo test -p node111 --test carrier_ntfy -- --exact smoke_log --nocapture) \
    >"$scratch/cargo.log" 2>&1; then
    ok "the smoke sequence passed"
  else
    tail -n 40 "$scratch/cargo.log" >&2
    fail "the smoke sequence failed (cargo test … smoke_log)"
  fi

  # (4) the log
  if [ -s "$log" ]; then
    ok "wrote ${log#"$L7_OS"/}"
    local section
    for section in "## Requests" "## The 4,096 / 4,097 B boundary" "## 429" \
      "## \`since\` after a stub restart" "## Keepalive and the 5B thread" "## RSS during a flush" \
      "## Bytes per frame" "## Owner-server items"; do
      if grep -qF "$section" "$log"; then ok "log section: $section"; else fail "log section missing: $section"; fi
    done
    local unrun
    unrun=$(grep -c "not run from this box" "$log")
    if [ "$unrun" -ge 11 ]; then
      ok "$unrun owner-server items marked \"not run from this box\""
    else
      fail "only $unrun owner-server items marked \"not run from this box\" (want 11)"
    fi
  else
    fail "no log at ${log#"$L7_OS"/}"
  fi

  # (5) nothing of ours is left
  local left
  left=$(stub_pids)
  if [ -n "$left" ]; then
    fail "a stub-ntfy.py process is still running: pid(s) $(printf '%s ' "$left")"
  else
    ok "no stub-ntfy.py process remains"
  fi
  if ss -ltnpH 2>/dev/null | grep -q 'python3'; then
    note "python3 listeners (not necessarily ours): $(ss -ltnpH 2>/dev/null | grep python3 | head -n 3)"
  fi
  finish G10-smoke
}

case ${1:-} in
  --smoke) smoke ;;
  "") skip_or_fail G10 "not built yet (T0 stub; row V4 builds G10, --smoke is row P1's)" ;;
  *)
    printf 'usage: %s [--smoke]\n' "$0" >&2
    exit 2
    ;;
esac
