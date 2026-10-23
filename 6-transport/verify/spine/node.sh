#!/usr/bin/env bash
# 6-transport/verify/spine/node.sh — helpers shared by the G4 and G5 gates (spine.sh, devices.sh; spec §16).
# Sourced, never run. Modelled on 4-interface/verify/smoke.sh and 6-transport/verify/lxmf.sh (req/jb/jf/t, the
# holder-process network namespaces, the scratch-root build); V1's root.sh and V2's lxmf.sh are not edited or
# sourced (they are other gates' files).
#
# Counters: OKS / FAILS (ok/fail print `ok:` / `FAIL:` and count; note prints `note:`).
# HTTP: req METHOD URL [curl args…] → $ST (000 = no answer), body in $W/body, headers in $W/headers.
# Namespaces: HOLD[NODE] is a holder process (`unshare --net sleep`) whose network namespace is NODE's;
#   nx NODE CMD… runs CMD there; api NODE METHOD PATH … talks to NODE's dashboard on 127.0.0.1:11180.
# Roots: g45_make_root ROOT ENVTEXT — releases/$G45_VER (111-agent stand-in recording the daemon's bearer),
#   etc/env, venvs/rns → the gate venv ($RNS_VENV; bin/rnstatus is a counting wrapper), the integrity seam
#   (cache/packages/python/rns-lock.txt = $RNS_LOCK, venvs/rns/.111-integrity = its sha256), state/.
# Daemons: start_node NODE / stop_node NODE (SIGTERM, $RC) / kill9_node NODE.
# shellcheck disable=SC2016,SC2034
G45_VER=${G45_VER:-2026.10.20}
DAPI=${DAPI:-http://127.0.0.1:11180}
OKS=0
FAILS=${FAILS:-0}
ok() {
  printf 'ok: %s\n' "$*"
  OKS=$((OKS + 1))
}
fail() {
  printf 'FAIL: %s\n' "$*"
  FAILS=$((FAILS + 1))
}
note() { printf 'note: %s\n' "$*"; }
T0=${G45_T0:-${EPOCHREALTIME/,/.}}
elapsed() { awk -v a="$T0" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}'; }
phase() { printf '\n== %s  (t+%ss)\n' "$*" "$(elapsed)"; }
now() { printf '%s' "${EPOCHREALTIME/,/.}"; }
since() { awk -v a="$1" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}'; }

declare -A HOLD ROOT DPID
HELPERS=()
NSX=()
req() {
  local m=$1 u=$2
  shift 2
  : >"$W/body"
  : >"$W/headers"
  ST=$("${NSX[@]}" curl -sS -o "$W/body" -D "$W/headers" -w '%{http_code}' -X "$m" --max-time "${REQ_TIMEOUT:-20}" "$@" "$u" 2>"$W/curl.err") || true
  [ -n "$ST" ] || ST=000
}
has_json() { [ -s "$1" ] && grep -q '[^[:space:]]' "$1" 2>/dev/null; }
jb() { has_json "$W/body" && jq -e "$@" "$W/body" >/dev/null 2>&1; }
jf() {
  local f=$1
  shift
  has_json "$f" && jq -e "$@" "$f" >/dev/null 2>&1
}
jget() { jq -r "$@" "$W/body" 2>/dev/null || true; }
hdr() { grep -i "^$1:" "$W/headers" 2>/dev/null | head -1 | cut -d: -f2- | tr -d '\r' | sed 's/^ //'; }
snip() { head -c 300 "$W/body" 2>/dev/null | tr '\n\r' '  '; }
t() { # DESC CONDITION (eval'd now)
  if eval "$2"; then ok "$1"; else fail "$1  [HTTP ${ST:-?}: $(snip)]"; fi
}
wait_for() { # SECONDS CONDITION — poll the eval'd CONDITION every 0.3 s
  local end=$((SECONDS + $1))
  while :; do
    if eval "$2"; then return 0; fi
    [ "$SECONDS" -ge "$end" ] && return 1
    sleep 0.3
  done
}
tw() { # SECONDS DESC CONDITION
  wait_for "$1" "$3" || true
  t "$2" "$3"
}
nx() {
  local n=$1
  shift
  nsenter -t "${HOLD[$n]}" -n -- "$@"
}
bg_in() { # NODE LOG CMD… — background process in NODE's netns; $BGPID
  local n=$1 log=$2
  shift 2
  nsenter -t "${HOLD[$n]}" -n -- "$@" >"$log" 2>&1 &
  BGPID=$!
  HELPERS+=("$BGPID")
}
api() { # NODE METHOD PATH [curl args…]
  local n=$1 m=$2 p=$3
  shift 3
  NSX=(nsenter -t "${HOLD[$n]}" -n --)
  req "$m" "$DAPI$p" "$@"
  NSX=()
}
jsend() { api "$1" "$2" "$3" -H 'Content-Type: application/json' -d "$4"; }
cmdline() { tr '\0' ' ' <"/proc/$1/cmdline" 2>/dev/null; }
pids_like() { # NODE PATTERN… — pids in NODE's netns whose cmdline holds every PATTERN
  local n=$1 ns p c pat hit out=""
  shift
  ns=$(readlink "/proc/${HOLD[$n]}/ns/net" 2>/dev/null) || return 0
  for p in /proc/[0-9]*; do
    p=${p#/proc/}
    [ "$p" = "$$" ] && continue
    [ "$(readlink "/proc/$p/ns/net" 2>/dev/null)" = "$ns" ] || continue
    c=$(cmdline "$p")
    [ -n "$c" ] || continue
    hit=1
    for pat in "$@"; do
      case "$c" in *"$pat"*) ;; *) hit=0 ;; esac
    done
    [ "$hit" = 1 ] && out="$out $p"
  done
  printf '%s' "${out# }"
}
rnsd_pid() { pids_like "$1" "bin/rnsd" "--config" "${ROOT[$1]}/rns" | awk '{print $1}'; }
helper_pid() {
  local p
  for p in $(pids_like "$1" "lxmf_sidecar.py" "--config-dir" "${ROOT[$1]}/bridge/lxmf/rns"); do
    printf '%s\n' "$p"
  done | head -1
}
# rnstatus (the gate venv's) against NODE's rnsd → $RS (text)
rs() {
  RS=$(nx "$1" env HOME="${ROOT[$1]}" "$RNS_VENV/bin/rnstatus" --config "${ROOT[$1]}/rns" "${@:2}" 2>&1)
}
# rnstatus block of the interface whose header holds NAME (Type[NAME…]) → stdout
rs_block() { # NAME (reads $RS)
  printf '%s\n' "$RS" | awk -v n="$1" '
    /^ [A-Za-z][A-Za-z ]*\[/ { show = (index($0, "[" n) > 0) }
    /^ *$/ { show = 0 }
    show { print }'
}
rs_status() { rs_block "$1" | sed -n 's/^ *Status *: *//p' | head -1; } # NAME → Up | Down | ""
rnsd_log_count() { grep -c -F "$2" "${ROOT[$1]}/rns/rnsd.log" 2>/dev/null || true; } # NODE TEXT

g45_make_root() { # ROOT ENVTEXT
  local root=$1 envtext=$2 rel
  rel=$root/releases/$G45_VER
  mkdir -p "$rel/bin" "$root/etc" "$root/state" "$root/venvs/rns/bin" "$root/cache/packages/python"
  ln -sfn "releases/$G45_VER" "$root/current"
  cat >"$rel/bin/111-agent" <<'EOF'
#!/bin/sh
# G4/G5 stand-in for 111-agent: record the daemon's per-spawn bearer (for the 401 checks), then idle.
umask 077
if [ -n "${AGENT111_STATE_DIR:-}" ] && [ -n "${AGENT111_DAEMON_TOKEN:-}" ]; then
  printf '%s' "$AGENT111_DAEMON_TOKEN" >"$AGENT111_STATE_DIR/g45-agent.token.tmp" &&
    mv "$AGENT111_STATE_DIR/g45-agent.token.tmp" "$AGENT111_STATE_DIR/g45-agent.token"
fi
exec sleep 1000000
EOF
  chmod 755 "$rel/bin/111-agent"
  printf '%s\n' "$envtext" >"$root/etc/env"
  ln -s "$RNS_VENV/bin/rnsd" "$root/venvs/rns/bin/rnsd"
  cat >"$root/venvs/rns/bin/rnstatus" <<EOF
#!/bin/sh
# G4/G5 fixture: count every rnstatus 111d runs, then run the real one (the gate venv's, rns 1.5.7)
echo "\$*" >>"$root/rnstatus-calls.log"
exec "$RNS_VENV/bin/rnstatus" "\$@"
EOF
  chmod 755 "$root/venvs/rns/bin/rnstatus"
  cp "$RNS_LOCK" "$root/cache/packages/python/rns-lock.txt"
  printf '%s' "$RNS_LOCKSHA" >"$root/venvs/rns/.111-integrity"
}

g45_identity() { # ROOT → prints the 64-hex node id (or nothing)
  local nid
  nid=$(NODE111_ROOT=$1 "$BIN111" identity generate 2>"$1/identity.log" | tail -1) || true
  if [[ "$nid" =~ ^[0-9a-f]{64}$ ]]; then printf '%s\n' "$nid"; fi
}

start_node() { # NODE
  local n=$1 d0 up=0 code
  d0=$(now)
  NODE111_ROOT=${ROOT[$n]} nsenter -t "${HOLD[$n]}" -n -- "$BIN111" daemon >>"${ROOT[$n]}/daemon.log" 2>&1 &
  DPID[$n]=$!
  for _ in $(seq 1 300); do
    code=$(nx "$n" curl -s -o /dev/null -w '%{http_code}' --max-time 2 "$DAPI/health" 2>/dev/null) || true
    case $code in 200 | 503) up=1 && break ;; esac
    kill -0 "${DPID[$n]}" 2>/dev/null || break
    sleep 0.2
  done
  if [ "$up" = 1 ]; then
    ok "$n: 111 daemon answers /health (HTTP $code) $(since "$d0")s after launch (pid ${DPID[$n]})"
  else
    fail "$n: 111 daemon never answered /health"
    tail -20 "${ROOT[$n]}/daemon.log" | sed "s/^/  $n daemon| /"
  fi
}
stop_node() { # NODE → $RC ("" when it did not exit within 30 s)
  local n=$1 p=${DPID[$1]:-}
  RC=""
  [ -n "$p" ] || return 0
  kill -TERM "$p" 2>/dev/null || true
  for _ in $(seq 1 150); do
    kill -0 "$p" 2>/dev/null || break
    sleep 0.2
  done
  if ! kill -0 "$p" 2>/dev/null; then
    wait "$p" && RC=0 || RC=$?
    DPID[$n]=""
  fi
}
kill9_node() { # NODE — SIGKILL the daemon (its supervised children die with it: PDEATHSIG)
  local p=${DPID[$1]:-}
  [ -n "$p" ] || return 0
  kill -KILL "$p" 2>/dev/null || true
  wait "$p" 2>/dev/null || true
  DPID[$1]=""
}
# make_netns NODE… — one holder per node (its own network namespace, lo up)
make_netns() {
  local n myns
  myns=$(readlink /proc/self/ns/net)
  for n in "$@"; do
    unshare --net sleep 1000000 &
    HOLD[$n]=$!
    HELPERS+=("${HOLD[$n]}")
  done
  for n in "$@"; do
    wait_for 5 '[ "$(readlink /proc/${HOLD[$n]}/ns/net 2>/dev/null)" != "$myns" ]' || true
    nx "$n" ip link set lo up
  done
}
# pair_session NODE LANURL JAR [curl args…] — the Layer 4 pair-code ceremony (smoke.sh 5d): the code from
# `111 dashboard` (run in NODE's netns), POST /auth/pair from the given source → cookie jar; $CODE, $ST
pair_session() {
  local n=$1 url=$2 jar=$3 cli
  shift 3
  cli=$(NODE111_ROOT=${ROOT[$n]} nx "$n" "$BIN111" dashboard 2>&1) || true
  printf '%s\n' "$cli" >"$W/dashboard-$n.out"
  CODE=$(printf '%s\n' "$cli" | sed -n 's/^Pair code: *\([0-9A-Z]\{4\}-[0-9A-Z]\{4\}\).*/\1/p' | head -1)
  [ -n "$CODE" ] || CODE=$(jq -r '.code // empty' "${ROOT[$n]}/state/dashboard-pair.json" 2>/dev/null)
  req POST "$url/api/v1/auth/pair" "$@" -c "$jar" -H 'Content-Type: application/json' -d "{\"code\":\"$CODE\"}"
}
# write a kind:device proposal the way 111-agent does (agent/proposals/<id>.json, node-id stamped) → $PID_
propose_device() { # NODE ID NODEID BODYJSON TITLE
  local n=$1 id=$2 nid=$3 body=$4 title=${5:-"G4/G5 device proposal"} dir
  dir=${ROOT[$n]}/agent/proposals
  mkdir -p "$dir"
  jq -nc --arg id "$id" --arg nid "$nid" --arg b "$body" --arg t "$title" \
    '{schema: 1, id: $id, node_id: $nid, task_id: null, kind: "device", title: $t, body: $b, created_at: "2026-10-20T00:00:00Z"}' \
    >"$dir/.$id.tmp" && mv "$dir/.$id.tmp" "$dir/$id.json"
}
decision_file() { printf '%s' "${ROOT[$1]}/agent/decisions/$2.json"; } # NODE ID
