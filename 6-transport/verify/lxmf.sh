#!/usr/bin/env bash
# 6-transport/verify/lxmf.sh — Layer 6 gate G3 (spec §16 G3; §6 whole; §15.10 egress): the LXMF helper, end to end.
#
#   bash 6-transport/verify/lxmf.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G3=1 bash 6-transport/verify/lxmf.sh     lock run: a missing prerequisite is a FAIL
#
# Shape (R07's): one user + pid namespace (unshare -rnpf, /proc remounted) holding two network namespaces A and B
# joined by one veth (A 10.66.0.1/24, B 10.66.0.2/24, each with a default route into the veth so any off-link
# attempt is a visible packet). Each netns runs a real `111 daemon` on its own scratch root, which supervises its
# own real `rnsd` (rns 1.5.7) on the config the Layer 6 renderer writes from `state/interfaces.toml`: A shares its
# LAN (DASHBOARD_LAN=true) so the renderer gives it the `111 LAN Backbone` BackboneInterface server (listen_on
# 0.0.0.0, port 4242; the renderer has no per-address form), B carries a `TCPClientInterface` entry to
# 10.66.0.1:4242 (the phone-relay stanza, `params = {target_host, target_port}`). Both keep the default
# `111.network` AutoInterface. Each daemon enables component `lxmf` through the API like a person would (licence
# notice → acknowledgement → offline install from `cache/packages/lxmf/` → start after rnsd serves → `ready`), so
# the far end of every LXMF message is the other node's helper.
#
# Prerequisites built here (the real install path needs no network; this gate needs PyPI once):
#   - G3_CACHE (default ${TMPDIR:-/tmp}/111-g3-cache): a venv `venv-lxmf` from the helper's lock
#     (6-transport/sidecars/lxmf-sidecar/lxmf-sidecar-lock.txt, `pip install --require-hashes`) and the lock's
#     wheels for this machine (`pip download --require-hashes --only-binary=:all:`). The lock carries rns too, so
#     the gate's rnsd and rnstatus come from that one venv: each root's `venvs/rns` is a directory whose bin/rnsd
#     links to it and whose bin/rnstatus is a counting wrapper (the production split is venvs/rns built from the
#     release's rns-lock.txt and venvs/lxmf from the lxmf lock; here the lxmf lock stands in as
#     cache/packages/python/rns-lock.txt so the daemon's integrity check passes). `venvs/lxmf` itself is built by
#     111d at enable, offline, from the wheels seeded into each root's `cache/packages/lxmf/` (§2.1 rule 3: the USB
#     drop path). Reused across runs while the lock is unchanged; deleted at the end unless SMOKE_KEEP=1 or
#     G3_KEEP_CACHE=1.
#   - the debug `111` (cargo build in 1-bootstrap; CARGO_TARGET_DIR / CARGO_BUILD_JOBS from the environment;
#     SMOKE_SKIP_BUILD=1 reuses it).
#   - fixtures: verify/egress-log.py (AF_PACKET logger + `strace -f -e trace=connect` wrapper that also taps the
#     helper's JSON Lines), verify/lxmf/fake-board.py (A's channel source: a protobuf-speaking stand-in for meshmud
#     and its board, so 111d's own Meshtastic client gives the relay edge a real channel), and, for the one check
#     the spec leaves unreachable with real libraries (an identical `msg` line twice), the scripted fake helper of
#     the node111 tests (1-bootstrap/crates/node111/tests/fake_lxmf_sidecar.py).
#
# The release path `current/share/lxmf-sidecar/lxmf_sidecar.py` of each root is a launcher that runs the real
# helper (a copy of 6-transport/sidecars/lxmf-sidecar/lxmf_sidecar.py) under egress-log.py's wrapper: 111d's
# component runner, piped stdio, post-spawn check, respawns and stop path are all the real ones.
#
# No stand-in for a product path: every protocol line a helper reads is written by 111d (the wrapper runs without
# `--inject`, and the gate asserts that no `+ ` line exists in either tap). The two cases the V2 run had to drive by
# hand are product paths now (fix wave FA2): announce on request (§6 (3); 111d writes `announce {}` after every
# `ready` and when a send parks as `peer_unannounced`, at most once per 60 s per helper run) and the path request of
# a parked send (§6 (4); the helper calls `RNS.Transport.request_path`, its rnsd answers from its path table or asks
# the network, and the peer's announce comes back as a path response). Announce before reply (A → B while A has never
# heard B) and the restart case (A's restarted helper has forgotten every peer) are both recovered that way. To keep
# the parked state observable for the row checks, B's rnsd is frozen (SIGSTOP) for a few seconds while A's send
# parks; the release after SIGCONT is the product's.
#
# Assertions (every one is an ok:/FAIL: line; the script exits 1 on any FAIL; "nothing" checks carry positive
# controls): the Pi rule (no rnstatus without a reader, one after GET /transport) · the helper never becomes the
# instance (started before rnsd it exits 3 and no socket of it listens; under 111d `ss -xlp` shows only rnsd on
# @rns/111, rnstatus shows the shared instance, the first line is `ready`) · a start while rnsd does not serve is
# detected as `partitioned` by the runner's post-spawn check and recovered once rnsd serves (adapter_down →
# live in GET /networks) · announce before reply (peer_unannounced, 111d's announce on the park, the helper's path
# request answered by the peer's announce as a path response, then delivered on heard) · signed: false with
# why_unsigned source_unknown before the source announced, true after · both directions · the same text twice is
# two rows · hash dedupe (an exact replay of a real msg line, plus a new hash twice, through the fake helper) ·
# the custom-field round trip (a Meshtastic channel → LXMF recipient-list edge: the far helper reports
# FIELD_CUSTOM_TYPE 111.relay, 111d handed it over and the far 111d stores the copy marker first, never an edge
# input: an LXMF DM is no edge endpoint) · history before the edge is never relayed · monotone states and a late failed after delivered ignored · delivery in seconds with no
# stamp cost · an rnsd crash on A: one delivery, no regression, the helper re-attached · zero off-link packets on
# both veths and no connect() but the shared instance from any helper · a restarted helper recovers its peer
# through the path request (no announce of the peer) · 111d's announce after every ready · enable (acknowledged) → disable →
# remove files leaves nothing installed while the cache survives, until remove-cache.
#
# Nothing of ours stays running: every process dies with the pid namespace or is stopped explicitly first; the
# scratch root is removed on success (kept on failure, or with SMOKE_KEEP=1). The last check is the hygiene one.
# Checks are condition strings eval'd by t() at check time:
# shellcheck disable=SC2016,SC2034,SC2317
set -u

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
OS=$(cd "$HERE/../.." && pwd)
export PATH="$HOME/.cargo/bin:$PATH:/usr/sbin:/sbin"
LOCK=$OS/6-transport/sidecars/lxmf-sidecar/lxmf-sidecar-lock.txt
HELPER_SRC=$OS/6-transport/sidecars/lxmf-sidecar/lxmf_sidecar.py
FAKE_HELPER=$OS/1-bootstrap/crates/node111/tests/fake_lxmf_sidecar.py
EGRESS=$HERE/egress-log.py
BOARD=$HERE/lxmf/fake-board.py
VER=2026.10.20
A_IP=10.66.0.1
B_IP=10.66.0.2
CONTROL_DST=192.0.2.1
DAPI=http://127.0.0.1:11180

. "$HERE/lib.sh" # FAILS, skip_or_fail, have_netns, finish; ok/fail/note are redefined below to count
OKS=0
ok() {
  printf 'ok: %s\n' "$*"
  OKS=$((OKS + 1))
}
fail() {
  printf 'FAIL: %s\n' "$*"
  FAILS=$((FAILS + 1))
}
note() { printf 'note: %s\n' "$*"; }
T0=${G3_T0:-${EPOCHREALTIME/,/.}}
elapsed() { awk -v a="$T0" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}'; }
phase() { printf '\n== %s  (t+%ss)\n' "$*" "$(elapsed)"; }
now() { printf '%s' "${EPOCHREALTIME/,/.}"; }
since() { awk -v a="$1" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}'; }

# ------------------------------------------------------------------ helpers (inner)

declare -A HOLD ROOT DPID SNIFF
HELPERS=()
NSX=()
# req METHOD URL [curl args…] → $ST (000 = no answer), $W/body
req() {
  local m=$1 u=$2
  shift 2
  : >"$W/body"
  ST=$("${NSX[@]}" curl -sS -o "$W/body" -w '%{http_code}' -X "$m" --max-time "${REQ_TIMEOUT:-20}" "$@" "$u" 2>"$W/curl.err") || true
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
tw() { # SECONDS DESC CONDITION — wait up to SECONDS for CONDITION, then check it
  wait_for "$1" "$3" || true
  t "$2" "$3"
}
nx() { # NODE CMD… — CMD inside NODE's net namespace
  local n=$1
  shift
  nsenter -t "${HOLD[$n]}" -n -- "$@"
}
bg_in() { # NODE LOG CMD… — a background process inside NODE's netns; $BGPID is its own pid
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
jsend() { # NODE METHOD PATH JSON
  api "$1" "$2" "$3" -H 'Content-Type: application/json' -d "$4"
}
cmdline() { tr '\0' ' ' <"/proc/$1/cmdline" 2>/dev/null; }
# pids_like NODE PATTERN… — pids in NODE's netns whose cmdline contains every PATTERN (fixed strings)
pids_like() {
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
# the helper: the egress wrapper 111d spawned (its pid is the component child's) and the program it traces
wrapper_pid() { pids_like "$1" "egress-log.py" " wrap " "${ROOT[$1]}/g3/" | awk '{print $1}'; }
helper_pid() {
  local p
  for p in $(pids_like "$1" "lxmf_sidecar.py" "--config-dir" "${ROOT[$1]}/bridge/lxmf/rns"); do
    case "$(cmdline "$p")" in strace* | *egress-log.py*) ;; *) printf '%s\n' "$p" ;; esac
  done | head -1
}
# our Python processes in NODE's netns other than the egress logger (rnsd is one)
pythons() {
  local p
  for p in $(pids_like "$1" python); do
    case "$(cmdline "$p")" in *"egress-log.py sniff"*) ;; *) printf '%s\n' "$p" ;; esac
  done | wc -l
}

# GET /networks on NODE; $W/body := the entry of network NET (the list is in $W/networks.json)
net_of() {
  api "$1" GET /api/v1/networks
  cp "$W/body" "$W/networks-$1.json" 2>/dev/null
  jq --arg n "$2" '.networks[] | select(.id == $n)' "$W/networks-$1.json" >"$W/body" 2>/dev/null || : >"$W/body"
}
lxmf_live() {
  net_of "$1" lxmf
  jb '.state == "live" and .component.state == "running" and (.component.reason_code == null)'
}
net_state_is() { net_of "$1" "$2" && jb --arg s "$3" '.state == $s'; }
# the helper's protocol as the wrapper tapped it: `< ` from the helper, `> ` to it; `# start` per run
tapf() { printf '%s' "${ROOT[$1]}/g3/tap.log"; }
tap_from() { sed -n 's/^< //p' "$(tapf "$1")" 2>/dev/null; }
tap_to() { sed -n 's/^> //p' "$(tapf "$1")" 2>/dev/null; }
tap_last_run_first() { # NODE → the first helper line of the newest run
  awk '/^# start /{f=1; next} f && /^< /{sub(/^< /, ""); line=$0; f=0} END{print line}' "$(tapf "$1")" 2>/dev/null
}
tap_has() { tap_from "$1" | jq -e "$2" >/dev/null 2>&1; } # NODE JQ-SELECT
tap_last_run() { # NODE → the helper lines of the newest run
  awk '/^# start /{n++; buf=""} n && /^< /{sub(/^< /, ""); buf=buf $0 "\n"} END{printf "%s", buf}' "$(tapf "$1")" 2>/dev/null
}
heard_in_last_run() { tap_last_run "$1" | jq -e --arg d "$2" 'select(.type == "heard" and .dest == $d)' >/dev/null 2>&1; }
tap_to_last_run() { # NODE → the lines 111d wrote to the newest run
  awk '/^# start /{n++; buf=""} n && /^> /{sub(/^> /, ""); buf=buf $0 "\n"} END{printf "%s", buf}' "$(tapf "$1")" 2>/dev/null
}
# the `announce {}` lines 111d wrote to NODE's newest helper run (§6 (3) announce on request)
announces_to() { tap_to_last_run "$1" | grep -cx '{"type":"announce"}' || true; }
injected() { grep -c '^+ ' "$(tapf "$1")" 2>/dev/null || true; } # NODE → lines not written by 111d
# pos_in_last_run NODE JQ-BOOL → the 1-based position of the first helper line of NODE's newest run for which JQ-BOOL
# holds (0 when none); for ordering checks
pos_in_last_run() {
  tap_last_run "$1" | jq -Rnr "[inputs | fromjson?] | [to_entries[] | select(.value | $2) | .key + 1] | first // 0" 2>/dev/null || echo 0
}
tap_to_has() { tap_to "$1" | jq -e "$2" >/dev/null 2>&1; }
# conv_of NODE NET KIND NATIVE_SUBSTRING → $CONV (fc- id or "")
conv_of() {
  api "$1" GET "/api/v1/networks/$2/conversations"
  cp "$W/body" "$W/convs-$1-$2.json" 2>/dev/null
  CONV=$(jq -r --arg k "$3" --arg s "$4" \
    '[.conversations[]? | select(.kind == $k and ((.native_id // "") | contains($s)))] | first | .id // empty' "$W/body" 2>/dev/null)
}
open_dm() { # NODE DEST → $CONV
  jsend "$1" POST /api/v1/networks/lxmf/conversations "$(jq -nc --arg d "$2" '{kind: "dm", native_id: $d}')"
  cp "$W/body" "$W/open-dm-$1.json" 2>/dev/null
  CONV=$(jget '.id // .conversation.id // empty')
}
send_text() { # NODE CONV TEXT → $MID ("" unless 202)
  jsend "$1" POST "/api/v1/networks/lxmf/conversations/$2/messages" "$(jq -nc --arg t "$3" '{text: $t}')"
  MID=$(jget '.id // empty')
  [ "$ST" = 202 ] || MID=""
}
thread() { # NODE NET CONV → $W/thread-NODE.json; every own row's state is appended to $W/states-NODE.log
  api "$1" GET "/api/v1/networks/$2/conversations/$3?limit=500"
  cp "$W/body" "$W/thread-$1.json" 2>/dev/null
  jq -r '.messages[]? | select(.dir == "own") | "\(.id) \(.state) \(.reason_code) \(.evidence)"' "$W/thread-$1.json" >>"$W/states-$1.log" 2>/dev/null || true
}
# the states NODE's API showed for row ID, in the order they were read (every GET of its thread samples it)
seen_states() { awk -v id="$2" '$1 == id {print $2}' "$W/states-$1.log" 2>/dev/null | uniq; }
field_of() { # NODE NET CONV ID FIELD → the row's FIELD (empty when absent)
  thread "$1" "$2" "$3"
  jq -r --arg id "$4" --arg f "$5" '.messages[]? | select(.id == $id) | .[$f] // empty' "$W/thread-$1.json" 2>/dev/null
}
state_is() { [ "$(field_of "$1" "$2" "$3" "$4" state)" = "$5" ]; } # NODE NET CONV ID STATE
rows_with_text() { # NODE NET CONV TEXT → the number of rows whose text is TEXT
  thread "$1" "$2" "$3"
  jq --arg t "$4" '[.messages[]? | select(.text == $t)] | length' "$W/thread-$1.json" 2>/dev/null || echo -1
}
rows_are() { [ "$(rows_with_text "$1" "$2" "$3" "$4")" = "$5" ]; } # NODE NET CONV TEXT N
row_by_text() { # NODE NET CONV TEXT → $W/row.json (the newest row with that text)
  thread "$1" "$2" "$3"
  jq --arg t "$4" '[.messages[]? | select(.text == $t)] | last // empty' "$W/thread-$1.json" >"$W/row.json" 2>/dev/null || : >"$W/row.json"
}
msgline() { # NODE TEXT → $W/msgline.json, the newest helper msg line with that text
  tap_from "$1" | jq -c --arg t "$2" 'select(.type == "msg" and .text == $t)' 2>/dev/null | tail -1 >"$W/msgline.json"
}
nearby_has() { # NODE DEST
  api "$1" GET /api/v1/nearby
  jb --arg d "$2" '[.sources[]? | select(.kind == "reticulum") | .nodes[]?.id] | index($d) != null'
}
count_class() { grep -c "\"class\": \"$2\"" "$WORK/egress-$1.jsonl" 2>/dev/null || true; } # NODE CLASS
start_node() {
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
consent_hash() { jget '.consent.hash // .hash // empty'; }
monotone() { # stdin: states, one per line — never regresses (queued < sent < relayed < delivered; failed terminal)
  awk 'BEGIN{r["queued"]=1; r["sent"]=2; r["relayed"]=3; r["delivered"]=4; r["failed"]=5; last=0; ok=1}
       { v=r[$1]; if (v=="") next; if (last==5 && v!=5) ok=0; if (last==4 && v!=4) ok=0;
         if (v<last && v!=5) ok=0; last=v } END {exit !ok}'
}

# ------------------------------------------------------------------ inner (in the namespaces)

inner() {
  : "${G3_WORK:?}" "${BIN111:?}" "${VENV:?}"
  WORK=$G3_WORK
  W=$WORK/http
  mkdir -p "$W"
  ROOT[A]=$WORK/A
  ROOT[B]=$WORK/B
  cleanup_inner() {
    local n p
    for n in A B; do
      p=${DPID[$n]:-}
      if [ -n "$p" ] && kill -0 "$p" 2>/dev/null; then
        kill -CONT "$(rnsd_pid "$n")" 2>/dev/null || true
        kill -TERM "$p" 2>/dev/null || true
      fi
    done
    for _ in $(seq 1 75); do
      kill -0 "${DPID[A]:-0}" 2>/dev/null || kill -0 "${DPID[B]:-0}" 2>/dev/null || break
      sleep 0.2
    done
    for p in "${HELPERS[@]}"; do kill -TERM "$p" 2>/dev/null || true; done
    # leaving pid 1 of the namespace kills whatever is left
    printf 'FAILS=%s\n' "$FAILS" >"$WORK/inner.result"
  }
  trap cleanup_inner EXIT
  trap 'exit 130' INT TERM
  local d0 n

  # ---------------------------------------------------------------- 1. the two netns + egress loggers
  phase "1. namespaces: A $A_IP and B $B_IP on one veth; AF_PACKET loggers on both ends"
  local sigign
  sigign=$(awk '/^SigIgn:/ {print $2}' /proc/self/status)
  t "namespace shell has default SIGINT/SIGTERM (SigIgn=$sigign)" '(( (16#$sigign & 0x4002) == 0 ))'
  ip link set lo up
  local myns
  myns=$(readlink /proc/self/ns/net)
  for n in A B; do
    unshare --net sleep 100000 &
    HOLD[$n]=$!
    HELPERS+=("${HOLD[$n]}")
  done
  for n in A B; do
    wait_for 5 '[ "$(readlink /proc/${HOLD[$n]}/ns/net 2>/dev/null)" != "$myns" ]' || true
  done
  if ip link add vA type veth peer name vB &&
    ip link set vA netns "${HOLD[A]}" && ip link set vB netns "${HOLD[B]}" &&
    nx A ip link set vA name eth0 && nx B ip link set vB name eth0 &&
    nx A ip link set lo up && nx B ip link set lo up &&
    nx A ip addr add "$A_IP/24" dev eth0 && nx B ip addr add "$B_IP/24" dev eth0 &&
    nx A ip link set eth0 up && nx B ip link set eth0 up &&
    nx A ip route add default via "$B_IP" && nx B ip route add default via "$A_IP"; then
    ok "netns A (holder ${HOLD[A]}) eth0 $A_IP/24 ↔ netns B (holder ${HOLD[B]}) eth0 $B_IP/24, default routes into the veth"
  else
    fail "netns/veth setup failed"
  fi
  tw 10 "A reaches B over the veth (a TCP connect to $B_IP:9 is refused by B, not lost)" \
    'nx A python3 -I -c "import socket,sys
try: socket.create_connection((sys.argv[1], 9), timeout=1)
except ConnectionRefusedError: sys.exit(0)
sys.exit(1)" "$B_IP"'
  for n in A B; do
    rm -f "$WORK/sniff-$n.ready"
    bg_in "$n" "$WORK/sniff-$n.err" python3 -I "$EGRESS" sniff --iface eth0 --allow "$A_IP,$B_IP" \
      --control-dst "$CONTROL_DST" --out "$WORK/egress-$n.jsonl" --summary "$WORK/egress-$n.summary.json" \
      --ready "$WORK/sniff-$n.ready"
    SNIFF[$n]=$BGPID
  done
  tw 10 "AF_PACKET loggers bound on A's and B's eth0" '[ -s "$WORK/sniff-A.ready" ] && [ -s "$WORK/sniff-B.ready" ]'
  # positive control (R07 §5): an off-link datagram from each netns must be seen by that netns' logger
  for n in A B; do
    nx "$n" python3 -I -c 'import socket,sys; s=socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.sendto(b"111-g3-control " + sys.argv[2].encode(), (sys.argv[1], 9))' "$CONTROL_DST" "$n" 2>/dev/null || true
  done
  tw 5 "positive control: each logger saw its own off-link datagram to $CONTROL_DST:9" \
    '[ "$(count_class A control)" -ge 1 ] && [ "$(count_class B control)" -ge 1 ]'

  # ---------------------------------------------------------------- 2. the trap, at the process level
  phase "2. the helper started before rnsd never becomes the instance (R07 check 1b)"
  local trap_dir=$WORK/trap trc listened=0 tp
  mkdir -p "$trap_dir/rns" "$trap_dir/store"
  chmod 700 "$trap_dir/rns" "$trap_dir/store"
  printf '[reticulum]\n  share_instance = Yes\n  instance_name = 111\n  rpc_key = %s\n  enable_transport = No\n\n[interfaces]\n' \
    "$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')" >"$trap_dir/rns/config"
  chmod 600 "$trap_dir/rns/config"
  d0=$(now)
  bg_in A "$trap_dir/helper.out" env HOME="$trap_dir" "$VENV/bin/python" -I "$WORK/helper/lxmf_sidecar.py" \
    --config-dir "$trap_dir/rns" --identity "$trap_dir/identity" --storage "$trap_dir/store"
  tp=$BGPID
  for _ in $(seq 1 300); do
    kill -0 "$tp" 2>/dev/null || break
    if nx A ss -xlp 2>/dev/null | grep 'rns/111' | grep -q python; then listened=1; fi
    sleep 0.1
  done
  if kill -0 "$tp" 2>/dev/null; then
    kill -KILL "$tp" 2>/dev/null
    trc=timeout
  else
    wait "$tp" 2>/dev/null
    trc=$?
  fi
  t "no rnsd: the helper exits 3 (partitioned) after $(since "$d0")s instead of serving (exit $trc)" '[ "$trc" = 3 ]'
  t "no rnsd: no socket of the helper ever listened on @rns/111 (ss -xlp sampled every 0.1 s)" '[ "$listened" = 0 ]'
  t "no rnsd: the helper said partitioned and never ready ($(head -c 160 "$trap_dir/helper.out" | tr '\n' ' '))" \
    'grep -q "partitioned" "$trap_dir/helper.out" && ! grep -q "\"ready\"" "$trap_dir/helper.out"'
  rm -rf "$trap_dir"

  # ---------------------------------------------------------------- 3. node B: daemon, rnsd, the Pi rule, enable
  phase "3. node B: 111 daemon with rnsd (TCPClient to A), the Pi rule, enable lxmf"
  start_node B
  tw 60 "B: 111d supervises rnsd from venvs/rns on the rendered config (pid $(rnsd_pid B))" '[ -n "$(rnsd_pid B)" ]'
  t "B: rns/config is 111d's render (schema-version 2, share_instance Yes, instance 111, the TCPClientInterface to $A_IP:4242), 0600 in a 0700 dir" \
    'grep -qx "schema-version = 2" "${ROOT[B]}/rns/config" && grep -q "share_instance = Yes" "${ROOT[B]}/rns/config" &&
     grep -q "instance_name = 111" "${ROOT[B]}/rns/config" && grep -q "type = TCPClientInterface" "${ROOT[B]}/rns/config" &&
     grep -q "target_host = $A_IP" "${ROOT[B]}/rns/config" && [ "$(stat -c %a "${ROOT[B]}/rns/config")" = 600 ] &&
     [ "$(stat -c %a "${ROOT[B]}/rns")" = 700 ]'
  # the Pi rule: with no reader the facts tick runs nothing (no rnstatus, no Python of ours); a reader wakes it
  note "B: idle 35 s (longer than the 30 s facts tick) with no reader and no component enabled"
  sleep 35
  t "Pi rule: no rnstatus ran on B while nothing read the facts ($(wc -l <"${ROOT[B]}/rnstatus-calls.log" 2>/dev/null || echo 0) calls)" \
    '[ ! -s "${ROOT[B]}/rnstatus-calls.log" ]'
  t "Pi rule: rnsd is B's only Python process ($(pythons B))" '[ "$(pythons B)" = 1 ]'
  api B GET /api/v1/transport
  cp "$W/body" "$W/transport-B.json"
  t "B: GET /transport answers (the demand)" '[ "$ST" = 200 ]'
  tw 40 "Pi rule positive control: a reader makes the tick run rnstatus within one period" '[ -s "${ROOT[B]}/rnstatus-calls.log" ]'
  local rs
  rs=$(nx B env HOME="${ROOT[B]}" "$VENV/bin/rnstatus" --config "${ROOT[B]}/rns" 2>&1)
  printf '%s\n' "$rs" >"$WORK/rnstatus-B-before.txt"
  t "B: rnstatus (the gate venv's) shows rnsd's shared instance ($(printf '%s' "$rs" | grep -m1 -i 'shared instance' | tr -s ' '))" \
    'printf "%s" "$rs" | grep -qi "Shared Instance"'

  t "B: before enable nothing of lxmf is installed (no venvs/lxmf, no bridge/lxmf); the cache holds the seeded wheels" \
    '[ ! -e "${ROOT[B]}/venvs/lxmf" ] && [ ! -e "${ROOT[B]}/bridge/lxmf" ] && [ "$(ls "${ROOT[B]}/cache/packages/lxmf" | wc -l)" -ge 6 ]'
  jsend B POST /api/v1/bridge/components/lxmf/enable '{}'
  cp "$W/body" "$W/enable-409-B.json"
  local notice
  notice=$(consent_hash)
  t "B: enable without the acknowledgement answers 409 consent_required with the notice and its hash" \
    '[ "$ST" = 409 ] && [ "$(jget .code)" = consent_required ] && [[ "$notice" =~ ^[0-9a-f]{64}$ ]]'
  t "B: the refused enable installed and started nothing" '[ ! -e "${ROOT[B]}/venvs/lxmf" ] && [ -z "$(wrapper_pid B)" ]'
  d0=$(now)
  REQ_TIMEOUT=240 jsend B POST /api/v1/bridge/components/lxmf/enable "$(jq -nc --arg h "$notice" '{accept_notice_hash: $h}')"
  cp "$W/body" "$W/enable-B.json"
  t "B: the acknowledged enable answers 200 in $(since "$d0")s (component $(jget .state), reason $(jget .reason_code))" '[ "$ST" = 200 ]'
  t "B: venvs/lxmf was built by 111d, offline, from cache/packages/lxmf (a link into venvs/lxmf.d/, bin/python, the lock's sha256 marker)" \
    '[ -L "${ROOT[B]}/venvs/lxmf" ] && [ -x "${ROOT[B]}/venvs/lxmf/bin/python" ] &&
     [ "$(cat "${ROOT[B]}/venvs/lxmf/.111-integrity" 2>/dev/null)" = "$LOCKSHA" ] &&
     case "$(readlink "${ROOT[B]}/venvs/lxmf")" in lxmf.d/*) true ;; *) false ;; esac'
  tw 30 "B: the helper is up and its first protocol line was ready" '[ "$(tap_last_run_first B | jq -r .type 2>/dev/null)" = ready ]'
  B_DEST=$(tap_last_run_first B | jq -r '.dest // empty' 2>/dev/null)
  B_PUB=$(tap_last_run_first B | jq -r '.pubkey // empty' 2>/dev/null)
  t "B: ready carries a 32-hex destination ($B_DEST), a 64-byte public key and instance 111" \
    '[[ "$B_DEST" =~ ^[0-9a-f]{32}$ ]] && [[ "$B_PUB" =~ ^[0-9a-f]{128}$ ]] && [ "$(tap_last_run_first B | jq -r .instance)" = 111 ]'
  tw 30 "B: GET /networks shows lxmf live with the helper's address as the account" 'lxmf_live B && [[ "$(jget .account)" == *"$B_DEST"* ]]'
  local hp ssx
  hp=$(helper_pid B)
  t "B: the helper (pid $hp) runs on venvs/lxmf's interpreter under the wrapper 111d spawned (pid $(wrapper_pid B))" \
    '[ -n "$hp" ] && [[ "$(cmdline "$hp")" == *"${ROOT[B]}/venvs/lxmf/bin/python"* ]]'
  ssx=$(nx B ss -xlp 2>/dev/null)
  printf '%s\n' "$ssx" >"$WORK/ss-x-B.txt"
  t "B: the helper attaches as a client: every @rns/111 listener is rnsd's (pid $(rnsd_pid B))" \
    'printf "%s\n" "$ssx" | grep "rns/111" | grep -q "pid=$(rnsd_pid B)," && ! printf "%s\n" "$ssx" | grep "rns/111" | grep -qv "pid=$(rnsd_pid B),"'
  t "B: the helper listens on no socket (ss -lxtunp shows no pid $hp)" '[ -n "$hp" ] && ! nx B ss -lxtunp 2>/dev/null | grep -q "pid=$hp,"'
  t "B: the helper holds no TCP or UDP socket at all (ss -tuanp; its AF_UNIX connects are checked by strace below)" \
    '[ -n "$hp" ] && ! nx B ss -tuanp 2>/dev/null | grep -q "pid=$hp,"'
  rs=$(nx B env HOME="${ROOT[B]}" "$VENV/bin/rnstatus" --config "${ROOT[B]}/rns" 2>&1)
  printf '%s\n' "$rs" >"$WORK/rnstatus-B-after.txt"
  t "B: rnstatus shows one shared instance, rnsd's, serving the helper ($(printf '%s' "$rs" | grep -m1 -i 'serving' | tr -s ' '))" \
    '[ "$(printf "%s" "$rs" | grep -ci "Shared Instance\[")" = 1 ] && printf "%s" "$rs" | grep -qiE "Serving *: *[1-9]"'

  # ---------------------------------------------------------------- 4. node A: daemon, Backbone, the partition
  phase "4. node A: 111 daemon with rnsd (BackboneInterface server); a start while rnsd does not serve"
  start_node A
  tw 60 "A: 111d supervises rnsd (pid $(rnsd_pid A)); rns/config carries the LAN BackboneInterface on port 4242" \
    '[ -n "$(rnsd_pid A)" ] && grep -q "type = BackboneInterface" "${ROOT[A]}/rns/config" && grep -q "port = 4242" "${ROOT[A]}/rns/config"'
  tw 30 "A: rnsd listens on :4242 (ss -ltn)" 'nx A ss -ltnH 2>/dev/null | grep -q ":4242 "'
  tw 90 "B's rnsd TCPClient is connected to A:4242 (ss -tn in B)" 'nx B ss -tnH state established 2>/dev/null | grep -q "$A_IP:4242"'
  # The enable request blocks until the component's health answers. As soon as 111d's runner has spawned the
  # helper (it gates on rnstatus first), rnsd is frozen (SIGSTOP): the helper now runs while the instance does not
  # serve, the start-order trap, and the post-spawn check must catch it.
  jsend A POST /api/v1/bridge/components/lxmf/enable '{}'
  notice=$(consent_hash)
  t "A: enable without the acknowledgement answers 409 consent_required" '[ "$ST" = 409 ] && [[ "$notice" =~ ^[0-9a-f]{64}$ ]]'
  bg_in A "$W/enable-A.out" curl -sS -o "$W/enable-A.json" -w '%{http_code}' --max-time 300 -X POST \
    -H 'Content-Type: application/json' -d "$(jq -nc --arg h "$notice" '{accept_notice_hash: $h}')" \
    "$DAPI/api/v1/bridge/components/lxmf/enable"
  local enpid=$BGPID frozen=""
  for _ in $(seq 1 6000); do
    if [ -n "$(wrapper_pid A)" ]; then
      frozen=$(rnsd_pid A)
      kill -STOP "$frozen" 2>/dev/null
      break
    fi
    kill -0 "$enpid" 2>/dev/null || break
    sleep 0.02
  done
  t "A: the runner spawned the helper after its gate passed, and rnsd ($frozen) was frozen right after" '[ -n "$frozen" ]'
  wait "$enpid" 2>/dev/null
  ST=$(cat "$W/enable-A.out" 2>/dev/null)
  cp "$W/enable-A.json" "$W/body" 2>/dev/null
  t "A: the enable answers 200 with reason partitioned (component $(jget .state), reason $(jget .reason_code))" \
    '[ "$ST" = 200 ] && [ "$(jget .reason_code)" = partitioned ]'
  net_of A lxmf
  cp "$W/body" "$W/net-A-partitioned.json"
  t "A: GET /networks: lxmf not live, its component adapter_down/partitioned (state $(jget .state), component $(jget .component.state)/$(jget .component.reason_code))" \
    'jb ".state != \"live\" and .component.reason_code == \"partitioned\""'
  tw 30 "A: the runner stopped the partitioned helper (none runs while rnsd is frozen)" '[ -z "$(helper_pid A)" ]'
  kill -CONT "$frozen" 2>/dev/null
  d0=$(now)
  wait_for 150 'lxmf_live A' || true
  A_READY=$(now) # an upper bound for the ready of A's current helper run (the announce bound, step 5)
  t "A: recovered once rnsd serves again: lxmf live, component running, no reason ($(since "$d0")s after SIGCONT)" 'lxmf_live A'
  t "A: the recovered helper's first line was ready" '[ "$(tap_last_run_first A | jq -r .type 2>/dev/null)" = ready ]'
  tw 10 "A: announce on request (§6 (3)): the first line 111d wrote to the new run is announce {}, once" \
    '[ "$(tap_to_last_run A | head -1)" = "{\"type\":\"announce\"}" ] && [ "$(announces_to A)" = 1 ]'
  t "B: 111d wrote announce {} once to B's run too, first ($(announces_to B))" \
    '[ "$(tap_to_last_run B | head -1)" = "{\"type\":\"announce\"}" ] && [ "$(announces_to B)" = 1 ]'
  A_DEST=$(tap_last_run_first A | jq -r '.dest // empty' 2>/dev/null)
  t "A: the account is the helper's address ($A_DEST)" '[[ "$A_DEST" =~ ^[0-9a-f]{32}$ ]] && net_of A lxmf && [[ "$(jget .account)" == *"$A_DEST"* ]]'
  ssx=$(nx A ss -xlp 2>/dev/null)
  t "A: every @rns/111 listener is rnsd's" \
    'printf "%s\n" "$ssx" | grep "rns/111" | grep -q "pid=$(rnsd_pid A)," && ! printf "%s\n" "$ssx" | grep "rns/111" | grep -qv "pid=$(rnsd_pid A),"'

  # ---------------------------------------------------------------- 5. announce before reply, signed, both ways
  phase "5. announce before reply (111d's announce on the park, the helper's path request), signed false then true, both directions"
  tw 90 "B heard A's announce (GET /nearby on B lists $A_DEST under reticulum)" 'nearby_has B "$A_DEST"'
  api A GET /api/v1/nearby
  t "A has not heard B (B announced once, at its start, before A ran): no $B_DEST in A's Nearby" '[ "$ST" = 200 ] && ! nearby_has A "$B_DEST"'
  local convAB convBA convA_B convB_A
  open_dm A "$B_DEST"
  convAB=$CONV
  t "A: the DM to B's address opens by native id ($convAB, native $(jget .native_id))" '[[ "$convAB" == fc-* ]]'
  open_dm B "$A_DEST"
  convBA=$CONV
  # B → A first, while A has never heard B: delivered, but the signature cannot be checked
  local txt_unsigned="g3 b-to-a before b announced" mid_unsigned
  d0=$(now)
  send_text B "$convBA" "$txt_unsigned"
  mid_unsigned=$MID
  tw 60 "B → A: delivered" 'state_is B lxmf "$convBA" "$mid_unsigned" delivered'
  note "B → A delivered $(since "$d0")s after the POST"
  tw 20 "A: the message from B is stored once, in the DM with B's address" \
    'conv_of A lxmf dm "$B_DEST" && [[ "$CONV" == fc-* ]] && rows_are A lxmf "$CONV" "$txt_unsigned" 1'
  convA_B=$CONV
  row_by_text A lxmf "$convA_B" "$txt_unsigned"
  t "A: unsigned while B never announced to it: auth none ($(jq -r .auth "$W/row.json" 2>/dev/null))" '[ "$(jq -r .auth "$W/row.json" 2>/dev/null)" = none ]'
  msgline A "$txt_unsigned"
  t "A: the helper reported signed false, why_unsigned source_unknown ($(jq -c '{signed, why_unsigned}' "$W/msgline.json" 2>/dev/null))" \
    'jf "$W/msgline.json" ".signed == false and .why_unsigned == \"source_unknown\""'
  t "A: an inbound message taught A's helper nothing of B (no heard of $B_DEST in its run)" '! heard_in_last_run A "$B_DEST"'
  # A → B: the send parks; 111d announces A (the park comes > 60 s after this run's ready, outside the bound) and the
  # helper asks its rnsd for B's path. B's rnsd is frozen for a few seconds so the parked state can be read.
  local wait_s
  wait_s=$(awk -v a="$A_READY" -v b="$(now)" 'BEGIN{w = 62 - (b - a); printf "%d", (w > 0 ? w + 1 : 0)}')
  if [ "$wait_s" -gt 0 ]; then
    note "waiting ${wait_s}s: A's park must come more than 60 s after its run's ready (one announce per 60 s per run, §6 (3))"
    sleep "$wait_s"
  fi
  local ann_A0 ann_B0 brnsd txt_parked="g3 a-to-b before b announced" mid_parked
  ann_A0=$(announces_to A)
  ann_B0=$(announces_to B)
  brnsd=$(rnsd_pid B)
  kill -STOP "$brnsd" 2>/dev/null
  t "B: rnsd ($brnsd) frozen while A's send parks (the far side holds still; nothing answers A's path request yet)" \
    '[ -n "$brnsd" ] && grep -q "^State:.*T" "/proc/$brnsd/status" 2>/dev/null'
  send_text A "$convAB" "$txt_parked"
  mid_parked=$MID
  thread A lxmf "$convAB" # the first sample of the row's states
  t "A: a send to an unannounced peer answers 202 ($mid_parked)" '[ -n "$mid_parked" ]'
  tw 20 "A: the helper parked it (state queued, reason peer_unannounced)" \
    'tap_has A "select(.type == \"state\" and .state == \"queued\" and .reason == \"peer_unannounced\")"'
  tw 10 "A: announce on request (§6 (3)): 111d wrote announce {} to its helper when the send parked ($ann_A0 → $(announces_to A))" \
    '[ "$(announces_to A)" = $((ann_A0 + 1)) ]'
  parked_row() {
    thread A lxmf "$convAB"
    jq --arg id "$mid_parked" '.messages[] | select(.id == $id)' "$W/thread-A.json" >"$W/row.json" 2>/dev/null
    jf "$W/row.json" '.state == "queued" and (.reason_code == "peer_unannounced" or .evidence == "waiting for their announce")'
  }
  wait_for 15 parked_row || true
  t "A: the row says it waits queued for the announce (reason_code $(jq -r .reason_code "$W/row.json"), evidence '$(jq -r .evidence "$W/row.json")')" parked_row
  jf "$W/row.json" '.reason_code == "peer_unannounced"' ||
    note "A: the parked row carries reason_code $(jq -r .reason_code "$W/row.json"): the core's SendEvent::Evidence takes no reason and the only queued reason it takes (a pre-air Failed, Later) would re-hand a send the helper holds parked; recorded as the core's follow-up (FA2 (c))"
  sleep 2
  t "A: the parked send still waits while nothing answers (state $(field_of A lxmf "$convAB" "$mid_parked" state))" 'state_is A lxmf "$convAB" "$mid_parked" queued'
  t "A: the parked send's helper never heard B yet" '! heard_in_last_run A "$B_DEST"'
  kill -CONT "$brnsd" 2>/dev/null
  d0=$(now)
  # B's rnsd now reads A's path request (forwarded by A's rnsd) and B's side answers with B's announce as a path response
  tw 30 "A's helper heard B: its path request was answered (a path response; B's helper did not announce)" 'heard_in_last_run A "$B_DEST"'
  tw 30 "A: GET /nearby lists $B_DEST under reticulum" 'nearby_has A "$B_DEST"'
  tw 90 "announce before reply: A's parked send went on the heard and was delivered ($(since "$d0")s after SIGCONT)" 'state_is A lxmf "$convAB" "$mid_parked" delivered'
  local sid_parked p_park p_heard p_done
  sid_parked=$(tap_to A | jq -r --arg t "$txt_parked" 'select(.type == "send" and .text == $t) | .id' 2>/dev/null | tail -1)
  p_park=$(pos_in_last_run A ".type == \"state\" and .id == \"$sid_parked\" and .reason == \"peer_unannounced\"")
  p_heard=$(pos_in_last_run A ".type == \"heard\" and .dest == \"$B_DEST\"")
  p_done=$(pos_in_last_run A ".type == \"state\" and .id == \"$sid_parked\" and .state == \"delivered\"")
  t "A: the helper's lines for send $sid_parked are parked ($p_park) → heard B ($p_heard) → delivered ($p_done), in that order" \
    '[ "$p_park" -gt 0 ] && [ "$p_heard" -gt "$p_park" ] && [ "$p_done" -gt "$p_heard" ]'
  t "B: 111d wrote no announce to B's helper meanwhile ($ann_B0 → $(announces_to B)) and nothing was injected (A $(injected A), B $(injected B))" \
    '[ "$(announces_to B)" = "$ann_B0" ] && [ "$(injected A)" = 0 ] && [ "$(injected B)" = 0 ]'
  tw 20 "B: A's parked text arrived once" 'conv_of B lxmf dm "$A_DEST" && [[ "$CONV" == fc-* ]] && rows_are B lxmf "$CONV" "$txt_parked" 1'
  convB_A=$CONV
  row_by_text B lxmf "$convB_A" "$txt_parked"
  msgline B "$txt_parked"
  t "B: signed, A had announced: auth signed, the helper's signed true ($(jq -r .auth "$W/row.json" 2>/dev/null); $(jq -c '{signed, why_unsigned}' "$W/msgline.json" 2>/dev/null))" \
    '[ "$(jq -r .auth "$W/row.json" 2>/dev/null)" = signed ] && jf "$W/msgline.json" ".signed == true and .why_unsigned == null"'
  local txt_signed="g3 b-to-a after b announced" mid_signed
  send_text B "$convBA" "$txt_signed"
  mid_signed=$MID
  tw 60 "B → A again: delivered" 'state_is B lxmf "$convBA" "$mid_signed" delivered'
  tw 20 "A: stored once" 'rows_are A lxmf "$convA_B" "$txt_signed" 1'
  row_by_text A lxmf "$convA_B" "$txt_signed"
  msgline A "$txt_signed"
  t "A: signed true once the source's announce (the path response) reached A (auth $(jq -r .auth "$W/row.json" 2>/dev/null); $(jq -c '{signed, why_unsigned}' "$W/msgline.json" 2>/dev/null))" \
    '[ "$(jq -r .auth "$W/row.json" 2>/dev/null)" = signed ] && jf "$W/msgline.json" ".signed == true and .why_unsigned == null"'
  # delivery in seconds (the stamp facts are read from rnsd's stored announces at the end)
  local txt_fast="g3 timing probe" mid_fast secs
  d0=$(now)
  send_text A "$convAB" "$txt_fast"
  mid_fast=$MID
  wait_for 60 'state_is A lxmf "$convAB" "$mid_fast" delivered' || true
  secs=$(since "$d0")
  t "A → B delivered in seconds ($secs s from POST to delivered, under 10 s)" \
    'state_is A lxmf "$convAB" "$mid_fast" delivered && awk -v s="$secs" "BEGIN{exit !(s < 10)}"'
  # the same text twice is two messages (distinct LXMF hashes)
  local txt_twice="g3 same text twice" m1 m2
  send_text A "$convAB" "$txt_twice"
  m1=$MID
  send_text A "$convAB" "$txt_twice"
  m2=$MID
  tw 60 "A: the same text twice is two rows ($m1, $m2), both delivered" \
    '[ -n "$m1" ] && [ "$m1" != "$m2" ] && state_is A lxmf "$convAB" "$m1" delivered && state_is A lxmf "$convAB" "$m2" delivered'
  tw 20 "B: two rows of that text, from two helper msg lines with different hashes" \
    'rows_are B lxmf "$convB_A" "$txt_twice" 2 && [ "$(tap_from B | jq -r --arg t "$txt_twice" "select(.type == \"msg\" and .text == \$t) | .hash" 2>/dev/null | sort -u | wc -l)" = 2 ]'
  # monotone states: as GET showed them to the gate (every read sampled the rows) and as the helper reported them
  local bad=0 id
  for id in "$mid_parked" "$mid_fast" "$m1" "$m2" "$mid_unsigned" "$mid_signed"; do
    seen_states A "$id" | monotone && seen_states B "$id" | monotone || bad=$((bad + 1))
  done
  t "A and B: every own row's states as read were monotone (parked: $(seen_states A "$mid_parked" | tr '\n' ' '))" \
    '[ "$bad" = 0 ] && [ "$(seen_states A "$mid_parked" | head -1)" = queued ] && [ "$(seen_states A "$mid_parked" | tail -1)" = delivered ]'
  bad=0
  for n in A B; do
    for id in $(tap_last_run "$n" | jq -r 'select(.type == "state") | .id' 2>/dev/null | sort -u); do
      tap_last_run "$n" | jq -r --arg id "$id" 'select(.type == "state" and .id == $id) | .state' 2>/dev/null | monotone || bad=$((bad + 1))
    done
  done
  t "A and B: the helpers' state lines are monotone per send id (this run of each helper)" '[ "$bad" = 0 ]'

  # ---------------------------------------------------------------- 6. an rnsd crash on A
  phase "6. rnsd crash on A: the helper stops, rnsd respawns, the helper returns; one delivery, no regression"
  local rnsd_old wrap_old txt_restart="g3 b-to-a across an rnsd restart" mid_restart
  rnsd_old=$(rnsd_pid A)
  wrap_old=$(wrapper_pid A)
  kill -KILL "$rnsd_old" 2>/dev/null
  d0=$(now)
  send_text B "$convBA" "$txt_restart"
  mid_restart=$MID
  t "B: the send during A's crash is accepted (202 $mid_restart)" '[ -n "$mid_restart" ]'
  tw 90 "A: the supervisor respawned rnsd (old $rnsd_old)" '[ -n "$(rnsd_pid A)" ] && [ "$(rnsd_pid A)" != "$rnsd_old" ]'
  tw 150 "A: the helper was stopped and re-attached under a new wrapper (old $wrap_old), lxmf live" \
    'lxmf_live A && [ -n "$(wrapper_pid A)" ] && [ "$(wrapper_pid A)" != "$wrap_old" ]'
  tw 60 "B's TCPClient reconnected to A's new rnsd" 'nx B ss -tnH state established 2>/dev/null | grep -q "$A_IP:4242"'
  wait_for 240 'state_is B lxmf "$convBA" "$mid_restart" delivered' || true
  t "B → A sent during the crash is delivered ($(since "$d0")s; state on B $(field_of B lxmf "$convBA" "$mid_restart" state); helper: $(tap_from B | jq -r --arg id "$(tap_to B | jq -r --arg t "$txt_restart" 'select(.type == "send" and .text == $t) | .id' 2>/dev/null | tail -1)" 'select(.type == "state" and .id == $id) | .state + (if .reason then "/" + .reason else "" end)' 2>/dev/null | tr '\n' ' '))" \
    'state_is B lxmf "$convBA" "$mid_restart" delivered'
  sleep 5
  t "A: exactly one row of that text (one delivery: $(rows_with_text A lxmf "$convA_B" "$txt_restart"))" 'rows_are A lxmf "$convA_B" "$txt_restart" 1'
  t "B: the restart row's states never regressed ($(seen_states B "$mid_restart" | tr '\n' ' '))" \
    'seen_states B "$mid_restart" | monotone && [ -n "$(seen_states B "$mid_restart")" ]'
  bad=0
  for id in "$mid_parked" "$mid_fast" "$m1" "$m2"; do state_is A lxmf "$convAB" "$id" delivered || bad=$((bad + 1)); done
  for id in "$mid_unsigned" "$mid_signed"; do state_is B lxmf "$convBA" "$id" delivered || bad=$((bad + 1)); done
  t "A and B: every earlier delivered row is still delivered after the restart" '[ "$bad" = 0 ]'

  # ---------------------------------------------------------------- 7. the relay custom field
  phase "7. the custom-field round trip: a Meshtastic channel on A → an LXMF recipient list [B]"
  tw 90 "A: the Meshtastic network is live on the fake board (111d's own client)" 'net_state_is A meshtastic live'
  local chan list edge ehash
  tw 30 "A: the board's channel 0 is a conversation" 'conv_of A meshtastic channel "" && [[ "$CONV" == fc-* ]]'
  chan=$CONV
  printf 'g3 history before the edge\n' >"${ROOT[A]}/g3/board/text-1"
  tw 30 "A: a channel post before the edge arrives on the channel" 'rows_are A meshtastic "$chan" "g3 history before the edge" 1'
  jsend A PATCH /api/v1/bridge/settings "$(jq -nc --arg d "$B_DEST" '{recipient_lists: [{id: "g3far", network: "lxmf", name: "G3 far end", members: [$d]}]}')"
  t "A: the recipient list [B] is saved (PATCH /bridge/settings)" '[ "$ST" = 200 ]'
  tw 10 "A: the list is a group conversation on lxmf" 'conv_of A lxmf group "g3far" && [[ "$CONV" == fc-* ]]'
  list=$CONV
  jsend A POST /api/v1/bridge/edges "$(jq -nc --arg s "$chan" --arg d "$list" '{src: $s, dst: $d}')"
  cp "$W/body" "$W/edge-409.json"
  ehash=$(consent_hash)
  t "A: the edge needs the warning confirmed (409 consent_required; addons $(jget '(.consent.addons // .addons // []) | join(",")'))" \
    '[ "$ST" = 409 ] && [[ "$ehash" =~ ^[0-9a-f]{64}$ ]]'
  jsend A POST /api/v1/bridge/edges "$(jq -nc --arg s "$chan" --arg d "$list" --arg h "$ehash" '{src: $s, dst: $d, accept_warning: $h}')"
  edge=$(jget '.id // empty')
  t "A: the edge is created with the hash (201, $edge, enabled_by $(jget .enabled_by))" '[ "$ST" = 201 ] && [[ "$edge" == be-* ]]'
  # A's helper restarted in step 6 and a shared-instance client keeps no known destinations across a restart: the
  # relay copy to B parks, the helper requests B's path, A's rnsd (or the network) answers, the copy goes (§6 (4))
  local ann_B7 relay_text="[Relayer/M] g3 relay probe"
  ann_B7=$(announces_to B)
  t "A: the restarted helper run starts with no known peers: it has not heard B" '! heard_in_last_run A "$B_DEST"'
  t "A: 111d announced the restarted run once, first (§6 (3): the peers relearn A)" \
    '[ "$(tap_to_last_run A | head -1)" = "{\"type\":\"announce\"}" ]'
  printf 'g3 relay probe\n' >"${ROOT[A]}/g3/board/text-2"
  tw 30 "A: the relay probe arrives on the channel" 'rows_are A meshtastic "$chan" "g3 relay probe" 1'
  tw 90 "B: the relayed copy arrives as '$relay_text'" 'rows_are B lxmf "$convB_A" "$relay_text" 1'
  local sid_relay
  sid_relay=$(tap_to A | jq -r --arg t "$relay_text" 'select(.type == "send" and .text == $t) | .id' 2>/dev/null | tail -1)
  p_park=$(pos_in_last_run A ".type == \"state\" and .id == \"$sid_relay\" and .reason == \"peer_unannounced\"")
  p_heard=$(pos_in_last_run A ".type == \"heard\" and .dest == \"$B_DEST\"")
  t "restart case: A's restarted helper parked the copy ($sid_relay at $p_park) and recovered B through its path request (heard at $p_heard), not by waiting for B's announce" \
    '[ "$p_park" -gt 0 ] && [ "$p_heard" -gt "$p_park" ]'
  t "restart case: B did not announce (111d wrote $ann_B7 → $(announces_to B) announces to B's run) and nothing was injected (A $(injected A), B $(injected B))" \
    '[ "$(announces_to B)" = "$ann_B7" ] && [ "$(injected A)" = 0 ] && [ "$(injected B)" = 0 ]'
  t "A: 111d handed the copy to its helper with fields.relay {v 1, net meshtastic}" \
    'tap_to_has A "select(.type == \"send\" and .text == \"$relay_text\" and .fields.relay.v == 1 and .fields.relay.net == \"meshtastic\")"'
  msgline B "$relay_text"
  t "B: the far helper got FIELD_CUSTOM_TYPE 111.relay with its data ($(jq -c .fields.relay "$W/msgline.json" 2>/dev/null))" \
    'jf "$W/msgline.json" ".fields.relay.v == 1 and .fields.relay.net == \"meshtastic\" and ((.fields.relay.id // \"\") | length) >= 1 and ((.fields.relay.mid // \"\") | test(\"^[0-9a-f]{32}$\"))"'
  row_by_text B lxmf "$convB_A" "$relay_text"
  net_of B lxmf
  t "B: the far 111d stored the copy as received, marker first (dir in, sender $(jq -r .sender.native "$W/row.json" 2>/dev/null)), and dropped no helper line as noise (dropped $(jget .dropped))" \
    'jf "$W/row.json" ".dir == \"in\" and (.text | startswith(\"[Relayer/M] \"))" && jb ".dropped == 0"'
  note "B: not_relayed is $(jq -r .not_relayed "$W/row.json" 2>/dev/null): 111d records it only on an edge input (bridge/edges.rs guard 3), and an LXMF DM is never one (checked below)"
  copy_row_delivered() {
    thread A lxmf "$list"
    jq -e --arg t "$relay_text" --arg e "$edge" '[.messages[]? | select(.text == $t and .relayed_from.edge == $e and .dir == "own" and .state == "delivered")] | length == 1' "$W/thread-A.json" >/dev/null 2>&1
  }
  wait_for 30 copy_row_delivered || true
  t "A: the copy is an own row of the edge in the list conversation (relayed_from $(jq -c --arg t "$relay_text" '[.messages[]? | select(.text == $t)] | first | .relayed_from' "$W/thread-A.json" 2>/dev/null)), delivered" \
    copy_row_delivered
  t "B: history from before the edge was never relayed" 'rows_are B lxmf "$convB_A" "[Relayer/M] g3 history before the edge" 0'
  sleep 3
  t "B: exactly one copy arrived" 'rows_are B lxmf "$convB_A" "$relay_text" 1'
  jsend B POST /api/v1/bridge/edges "$(jq -nc --arg s "$convB_A" --arg d "$convB_A" '{src: $s, dst: $d}')"
  t "B: an LXMF DM can never be an edge endpoint (400)" '[ "$ST" = 400 ]'

  # ---------------------------------------------------------------- 8. hash dedupe + a late failed, scripted helper
  phase "8. hash dedupe and a late failed after delivered (B on the node111 tests' scripted fake helper)"
  local real_line h1 h2 h3
  real_line=$(tap_from B | jq -c --arg t "$txt_parked" 'select(.type == "msg" and .text == $t)' 2>/dev/null | head -1)
  t "B: the tap holds the real helper's msg line of A's parked text (to replay it)" '[ -n "$real_line" ]'
  jsend B POST /api/v1/bridge/components/lxmf/disable '{}'
  tw 20 "B: disabled (200) before the swap; no helper runs" '[ "$ST" = 200 ] && [ -z "$(wrapper_pid B)" ]'
  h1=$(printf 'g3-dedupe-1' | sha256sum | cut -c1-64)
  h2=$(printf 'g3-dedupe-2' | sha256sum | cut -c1-64)
  h3=$(printf 'g3-late-failed' | sha256sum | cut -c1-64)
  jq -n --argjson real "${real_line:-null}" --arg dest "$B_DEST" --arg pub "$B_PUB" --arg h1 "$h1" --arg h2 "$h2" --arg h3 "$h3" '
    {runs: [[
      {emit: {type: "ready", dest: $dest, pubkey: $pub, instance: "111"}},
      {sleep_ms: 3000},
      {emit: $real},
      {emit: ($real + {hash: $h1, text: "g3 dedupe probe"})},
      {sleep_ms: 300},
      {emit: ($real + {hash: $h1, text: "g3 dedupe probe"})},
      {emit: ($real + {hash: $h2, text: "g3 dedupe control"})},
      {expect: "send", as: "s1"},
      {state: {ref: "s1", hash: $h3, state: "sent"}},
      {sleep_ms: 1500},
      {state: {ref: "s1", hash: $h3, state: "delivered"}},
      {sleep_ms: 3000},
      {state: {ref: "s1", hash: $h3, state: "failed", reason: "rejected"}}
    ]]}' >"${ROOT[B]}/g3/fake-scenario.json"
  printf 'fake\n' >"${ROOT[B]}/g3/helper-mode"
  REQ_TIMEOUT=120 jsend B POST /api/v1/bridge/components/lxmf/enable '{}'
  t "B: enabled on the scripted helper (200)" '[ "$ST" = 200 ]'
  tw 30 "B: lxmf live on the scripted helper" 'lxmf_live B'
  tw 20 "dedupe positive control: a new hash arrives (g3 dedupe control)" 'rows_are B lxmf "$convB_A" "g3 dedupe control" 1'
  t "hash dedupe: the same hash twice is one row (g3 dedupe probe: $(rows_with_text B lxmf "$convB_A" "g3 dedupe probe"))" \
    'rows_are B lxmf "$convB_A" "g3 dedupe probe" 1'
  t "hash dedupe: an exact replay of the real helper's msg line is dropped (A's parked text: $(rows_with_text B lxmf "$convB_A" "$txt_parked") row)" \
    'rows_are B lxmf "$convB_A" "$txt_parked" 1'
  local txt_late="g3 late failed probe" mid_late
  send_text B "$convBA" "$txt_late"
  mid_late=$MID
  tw 20 "B: the scripted send reaches delivered" 'state_is B lxmf "$convBA" "$mid_late" delivered'
  tw 10 "B: the scripted helper wrote its late failed line" 'tap_has B "select(.type == \"state\" and .state == \"failed\" and .hash == \"$h3\")"'
  for _ in 1 2 3 4 5 6 7 8 9 10; do field_of B lxmf "$convBA" "$mid_late" state >/dev/null; sleep 0.3; done
  t "B: a late failed after delivered is ignored (states read: $(seen_states B "$mid_late" | tr '\n' ' '))" \
    'state_is B lxmf "$convBA" "$mid_late" delivered && seen_states B "$mid_late" | monotone && ! seen_states B "$mid_late" | grep -q failed'
  jsend B POST /api/v1/bridge/components/lxmf/disable '{}'
  printf 'real\n' >"${ROOT[B]}/g3/helper-mode"

  # ---------------------------------------------------------------- 9. disable → remove files → remove cache
  phase "9. enable (acknowledged) → disable → remove files leaves nothing installed; the cache until remove-cache"
  jsend A POST /api/v1/bridge/components/lxmf/disable '{}'
  t "A: disable answers 200" '[ "$ST" = 200 ]'
  tw 30 "A: no helper, wrapper or strace runs after disable" '[ -z "$(wrapper_pid A)" ] && [ -z "$(helper_pid A)" ] && [ -z "$(pids_like A strace)" ]'
  tw 30 "B: nothing of the helper runs after disable" '[ -z "$(wrapper_pid B)" ] && [ -z "$(pids_like B lxmf_sidecar)" ] && [ -z "$(pids_like B strace)" ]'
  tw 10 "A: lxmf reads disabled in GET /networks" 'net_state_is A lxmf disabled'
  for n in A B; do
    t "$n: positive control: the install is there before remove files (venvs/lxmf, venvs/lxmf.d, bridge/lxmf)" \
      '[ -e "${ROOT[$n]}/venvs/lxmf" ] && [ -d "${ROOT[$n]}/venvs/lxmf.d" ] && [ -d "${ROOT[$n]}/bridge/lxmf" ]'
    jsend "$n" POST /api/v1/bridge/components/lxmf/remove-files '{}'
    t "$n: remove files answers 200" '[ "$ST" = 200 ]'
    t "$n: nothing of lxmf is installed (no venvs/lxmf, venvs/lxmf.d, bridge/lxmf, components/lxmf)" \
      '[ ! -e "${ROOT[$n]}/venvs/lxmf" ] && [ ! -e "${ROOT[$n]}/venvs/lxmf.d" ] && [ ! -e "${ROOT[$n]}/bridge/lxmf" ] && [ ! -e "${ROOT[$n]}/components/lxmf" ]'
    t "$n: the cache survives remove files ($(ls "${ROOT[$n]}/cache/packages/lxmf" 2>/dev/null | wc -l) wheels) and venvs/rns is untouched" \
      '[ "$(ls "${ROOT[$n]}/cache/packages/lxmf" 2>/dev/null | wc -l)" -ge 6 ] && [ -e "${ROOT[$n]}/venvs/rns/bin/rnsd" ]'
    jsend "$n" POST /api/v1/bridge/components/lxmf/remove-files '{"cache": true}'
    t "$n: remove cache deletes cache/packages/lxmf (HTTP $ST)" '[ "$ST" = 200 ] && [ ! -e "${ROOT[$n]}/cache/packages/lxmf" ]'
    api "$n" GET /api/v1/bridge/components
    t "$n: the component reads declared again ($(jget '.components[]? | select(.id == "lxmf") | .state'))" \
      '[ "$(jget ".components[]? | select(.id == \"lxmf\") | .state")" = declared ]'
  done

  # ---------------------------------------------------------------- 10. egress
  phase "10. egress: the AF_PACKET logs of both veths, strace connect() of every real helper run"
  for n in A B; do kill -TERM "${SNIFF[$n]}" 2>/dev/null; done
  local off ctl peer link runs unix other
  for n in A B; do
    wait_for 10 '[ -s "$WORK/egress-$n.summary.json" ]' || true
    off=$(count_class "$n" off_link)
    ctl=$(count_class "$n" control)
    peer=$(jq '[.counts | to_entries[] | select(.key | startswith("peer_")) | .value] | add // 0' "$WORK/egress-$n.summary.json" 2>/dev/null)
    link=$(jq '[.counts | to_entries[] | select(.key | startswith("link_")) | .value] | add // 0' "$WORK/egress-$n.summary.json" 2>/dev/null)
    t "$n's veth: the logger ran through the gate and saw the traffic ($peer peer, $link link-scope, $ctl control packets)" \
      'jf "$WORK/egress-$n.summary.json" ".counts" && [ "${peer:-0}" -gt 0 ] && [ "${ctl:-0}" -ge 1 ]'
    t "$n's veth: ZERO off-link packets (anything but $A_IP↔$B_IP, IPv6 link-local and link-scope)" '[ "${off:-1}" = 0 ]'
    [ "${off:-0}" = 0 ] || grep '"class": "off_link"' "$WORK/egress-$n.jsonl" | head -5 | sed "s/^/  $n off-link| /"
    runs=$(find "${ROOT[$n]}/g3" -name 'strace-real-*.log' 2>/dev/null | wc -l)
    unix=$(cat "${ROOT[$n]}/g3"/strace-real-*.log 2>/dev/null | grep 'connect(' | grep -c '@"rns/111')
    other=$(cat "${ROOT[$n]}/g3"/strace-real-*.log 2>/dev/null | grep 'connect(' | grep -vc '@"rns/111')
    t "$n: strace traced $runs real helper run(s) and saw connect() to the shared instance ($unix to @rns/111)" '[ "$runs" -ge 1 ] && [ "$unix" -ge 1 ]'
    t "$n: no helper connect() but to rnsd's shared instance ($other other)" '[ "$other" = 0 ]'
    [ "$other" = 0 ] || cat "${ROOT[$n]}/g3"/strace-real-*.log | grep 'connect(' | grep -v '@"rns/111' | head -5 | sed "s/^/  $n connect| /"
  done

  # ---------------------------------------------------------------- 11. stop
  phase "11. stop both daemons"
  for n in B A; do
    stop_node "$n"
    t "$n: 111 daemon exits 0 on SIGTERM (rc $RC)" '[ "$RC" = 0 ]'
    tw 10 "$n: nothing of this run is left in its netns (rnsd, helper, strace, board)" '[ -z "$(pids_like "$n" "$WORK")" ]'
  done
  # no stamp cost (§6 (3)): the announces each rnsd stored (rns/storage/known_destinations, written at its exit) carry
  # the LXMF peer data [name, stamp_cost, …]; the helper advertises none
  local sc
  for n in A B; do
    local other=$B_DEST
    [ "$n" = B ] && other=$A_DEST
    sc=$("$VENV/bin/python" -I -c '
import sys
import RNS.vendor.umsgpack as m
import LXMF
d = m.unpackb(open(sys.argv[1], "rb").read())
e = d.get(bytes.fromhex(sys.argv[2]))
if e is None:
    print("absent")
else:
    app = e[3]
    print("stamp_cost=%r data=%r" % (LXMF.stamp_cost_from_app_data(app), m.unpackb(app) if app else None))
' "${ROOT[$n]}/rns/storage/known_destinations" "$other" 2>&1 | tail -1)
    t "$n: the peer's announce stored by rnsd advertises no stamp cost ($sc)" '[[ "$sc" == "stamp_cost=None "* ]]'
  done
}

# ------------------------------------------------------------------ outer

if [ "${1:-}" = --inner ]; then
  LOCKSHA=$(sha256sum "$LOCK" | cut -c1-64)
  inner
  exit $((FAILS > 0))
fi

have_netns || skip_or_fail G3 "unprivileged network namespaces are not available (unshare -Urnm)"
for tool in python3 strace ip nsenter curl jq ss unshare sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || skip_or_fail G3 "$tool is not installed"
done
python3 -c 'import sys; sys.exit(sys.version_info < (3, 11))' 2>/dev/null ||
  skip_or_fail G3 "python3 is older than 3.11 (the lxmf lock's floor)"
[ -r "$LOCK" ] && [ -r "$HELPER_SRC" ] && [ -r "$FAKE_HELPER" ] && [ -r "$BOARD" ] && [ -r "$EGRESS" ] ||
  { echo "FAIL: G3: a fixture is missing (lock, helper, fake helper, fake board, egress-log.py)"; exit 1; }

echo "6-transport/verify/lxmf.sh — Layer 6 gate G3"
# ---- the venv + wheels (cached across runs while the lock is unchanged)
G3_CACHE=${G3_CACHE:-${TMPDIR:-/tmp}/111-g3-cache}
VENV=$G3_CACHE/venv-lxmf
WHEELS=$G3_CACHE/wheels
LOCKSHA=$(sha256sum "$LOCK" | cut -c1-64)
mkdir -p "$G3_CACHE"
phase "0. prerequisites: the venv and wheels from the lock ($G3_CACHE), the debug 111"
if [ "$(cat "$G3_CACHE/lock.sha256" 2>/dev/null)" = "$LOCKSHA" ] && [ -x "$VENV/bin/rnsd" ] && [ -x "$VENV/bin/rnstatus" ] &&
  [ "$(ls "$WHEELS" 2>/dev/null | wc -l)" -ge 6 ]; then
  ok "venv-lxmf and the lock's wheels are cached for this lock ($(ls "$WHEELS" | wc -l) wheels)"
else
  rm -rf "$VENV" "$WHEELS" "$G3_CACHE/lock.sha256"
  mkdir -p "$WHEELS"
  if python3 -m venv "$VENV" >"$G3_CACHE/pip.log" 2>&1 &&
    "$VENV/bin/pip" download -q --require-hashes --only-binary=:all: -r "$LOCK" -d "$WHEELS" >>"$G3_CACHE/pip.log" 2>&1 &&
    "$VENV/bin/pip" install -q --require-hashes --no-index --find-links "$WHEELS" -r "$LOCK" >>"$G3_CACHE/pip.log" 2>&1; then
    printf '%s\n' "$LOCKSHA" >"$G3_CACHE/lock.sha256"
    ok "venv-lxmf built from the lock (pip install --require-hashes; $(ls "$WHEELS" | wc -l) wheels downloaded)"
  else
    tail -5 "$G3_CACHE/pip.log" >&2
    rm -rf "$VENV" "$WHEELS"
    skip_or_fail G3 "cannot build the venv from the lock (PyPI unreachable?): $(grep -m1 -E 'ERROR|error' "$G3_CACHE/pip.log")"
  fi
fi
"$VENV/bin/python" -c 'import RNS, LXMF; print(RNS.__version__ if hasattr(RNS, "__version__") else "")' >/dev/null 2>&1 ||
  { fail "the venv cannot import RNS and LXMF"; }
note "rns $("$VENV/bin/pip" show rns 2>/dev/null | sed -n 's/^Version: //p'), lxmf $("$VENV/bin/pip" show lxmf 2>/dev/null | sed -n 's/^Version: //p'), $("$VENV/bin/python" --version 2>&1)"

TGT=${CARGO_TARGET_DIR:-$OS/1-bootstrap/target}
BIN111=$TGT/debug/111
if [ "${SMOKE_SKIP_BUILD:-0}" = 1 ]; then
  note "SMOKE_SKIP_BUILD=1 — cargo build skipped"
else
  t1=$(now)
  if (cd "$OS/1-bootstrap" && cargo build --bin 111) >"$G3_CACHE/cargo.log" 2>&1; then
    ok "cargo build --bin 111 ($(since "$t1")s)"
  else
    tail -20 "$G3_CACHE/cargo.log" | sed 's/^/  cargo| /'
    fail "cargo build --bin 111"
  fi
fi
[ -x "$BIN111" ] || { echo "FAIL: $BIN111 missing"; exit 1; }

WORK=$(mktemp -d "${TMPDIR:-/tmp}/111-g3.XXXXXX")
cleanup() {
  local keep=0
  if [ "${SMOKE_KEEP:-0}" = 1 ] || [ "$FAILS" -gt 0 ]; then
    keep=1
    echo "note: scratch kept at $WORK (A/ B/ daemon.log, g3/tap.log, g3/strace-*.log, egress-*.jsonl, http/)"
  else
    rm -rf "$WORK"
  fi
  if [ "${SMOKE_KEEP:-0}" != 1 ] && [ "${G3_KEEP_CACHE:-0}" != 1 ]; then
    rm -rf "$VENV" "$WHEELS" "$G3_CACHE/lock.sha256" "$G3_CACHE/pip.log" "$G3_CACHE/cargo.log"
    rmdir "$G3_CACHE" 2>/dev/null || true
  fi
}
trap cleanup EXIT

phase "0b. scratch roots A and B ($WORK)"
mkdir -p "$WORK/helper"
cp "$HELPER_SRC" "$WORK/helper/lxmf_sidecar.py"
make_root() { # NODE
  local n=$1 root=$WORK/$1 rel
  rel=$root/releases/$VER
  mkdir -p "$rel/bin" "$rel/share/lxmf-sidecar" "$root/etc" "$root/state" "$root/g3/board" \
    "$root/cache/packages/python" "$root/cache/packages/lxmf" "$root/venvs/rns/bin"
  ln -s "releases/$VER" "$root/current"
  # the release's helper path: the real helper (or, on request, the scripted fake) under the egress wrapper
  cat >"$rel/share/lxmf-sidecar/lxmf_sidecar.py" <<EOF
# G3 fixture (6-transport/verify/lxmf.sh): this release path runs the LXMF helper under egress-log.py's
# strace + JSON Lines tap. <root>/g3/helper-mode "fake" selects the node111 tests' scripted fake helper.
import os, sys
G3 = "$root/g3"
try:
    mode = open(os.path.join(G3, "helper-mode")).read().strip()
except OSError:
    mode = "real"
if mode == "fake":
    prog = [sys.executable, "-I", "$FAKE_HELPER", os.path.join(G3, "fake-scenario.json"), os.path.join(G3, "fake-record.log")]
else:
    mode, prog = "real", [sys.executable, "-I", "$WORK/helper/lxmf_sidecar.py"]
os.execv(sys.executable, [sys.executable, "-I", "$EGRESS", "wrap",
                          "--strace-log", os.path.join(G3, "strace-%s-%d.log" % (mode, os.getpid())),
                          "--tap", os.path.join(G3, "tap.log"), "--"] + prog + sys.argv[1:])
EOF
  printf 'real\n' >"$root/g3/helper-mode"
  # rnsd's venv: the gate venv's rnsd, and a counting rnstatus (the Pi-rule check reads the count)
  ln -s "$VENV/bin/rnsd" "$root/venvs/rns/bin/rnsd"
  cat >"$root/venvs/rns/bin/rnstatus" <<EOF
#!/bin/sh
# G3 fixture: count every rnstatus 111d runs, then run the real one
echo "\$*" >>"$root/rnstatus-calls.log"
exec "$VENV/bin/rnstatus" "\$@"
EOF
  chmod 755 "$root/venvs/rns/bin/rnstatus"
  cp "$LOCK" "$root/cache/packages/python/rns-lock.txt"
  printf '%s' "$LOCKSHA" >"$root/venvs/rns/.111-integrity"
  cp "$WHEELS"/* "$root/cache/packages/lxmf/"
  if [ "$n" = A ]; then
    printf '# G3 node A: LAN sharing on (the Backbone listener)\nDASHBOARD_LAN=true\n' >"$root/etc/env"
    cat >"$rel/bin/meshmud" <<EOF
#!/bin/sh
# G3 fixture: meshmud and its board in one (6-transport/verify/lxmf/fake-board.py)
exec python3 -I "$BOARD" --ctl "$root/g3/board" "\$@"
EOF
    chmod 755 "$rel/bin/meshmud"
    cat >"$root/state/devices.json" <<'EOF'
{"schema": 1, "devices": {"abcd1234abcd1234": {
  "first_seen": "2026-10-01T00:00:00Z", "last_seen": "2026-10-01T00:00:00Z",
  "firmware": {"family": "meshtastic", "version": "2.7.11.abcdef0", "source": "probe"},
  "meshtastic": {"uplink": {"kind": "tcp", "host": "127.0.0.1", "port": 4403},
                 "node_num": 168430090, "node_id": "!0a0a0a0a", "hw_model": "HELTEC_V3"}}}}
EOF
  else
    printf '# G3 node B: loopback dashboard\n' >"$root/etc/env"
    cat >"$root/state/interfaces.toml" <<EOF
# G3 node B: the phone-relay client stanza to A's Backbone listener (§11)
schema = 1

[[interface]]
name = "111 hub $A_IP"
owner = "rnsd"
type = "TCPClientInterface"
params = { target_host = "$A_IP", target_port = 4242 }
EOF
    chmod 600 "$root/state/interfaces.toml"
  fi
  local nid
  nid=$(NODE111_ROOT=$root "$BIN111" identity generate 2>"$root/identity.log" | tail -1) || true
  if [[ "$nid" =~ ^[0-9a-f]{64}$ ]]; then
    ok "$n: root built (identity $nid; current → releases/$VER; venvs/rns → the gate venv; cache/packages/lxmf seeded with $(ls "$root/cache/packages/lxmf" | wc -l) wheels)"
  else
    fail "$n: 111 identity generate → '$nid' ($(head -c 200 "$root/identity.log"))"
  fi
}
make_root A
make_root B

# reset the SIGINT/SIGTERM that unshare ignores in its child (smoke.sh's note)
if env --default-signal true 2>/dev/null; then
  RESET_SIG=(env --default-signal)
else
  RESET_SIG=(python3 -c 'import os,signal,sys
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])')
fi
INNER_LOG=$WORK/inner.log
G3_WORK=$WORK BIN111=$BIN111 VENV=$VENV G3_T0=$T0 \
  unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
  "${RESET_SIG[@]}" bash "$HERE/lxmf.sh" --inner 2>&1 | tee "$INNER_LOG" || true
inner_fails=$(grep -c '^FAIL:' "$INNER_LOG" || true)
inner_oks=$(grep -c '^ok:' "$INNER_LOG" || true)
if ! grep -q '^FAILS=' "$WORK/inner.result" 2>/dev/null; then
  fail "the namespace run did not finish (killed or crashed — see above)"
fi
FAILS=$((FAILS + inner_fails))

# ---- hygiene: nothing of ours outlives the namespaces
phase "12. hygiene"
left=""
for p in /proc/[0-9]*; do
  p=${p#/proc/}
  [ "$p" = "$$" ] && continue
  c=$(tr '\0' ' ' <"/proc/$p/cmdline" 2>/dev/null)
  case "$c" in *"$WORK"*) left="$left $p:${c:0:60}" ;; esac
done
t "hygiene: no process of this run is left (rnsd, helper, strace, board, daemons)" '[ -z "$left" ]'
[ -z "$left" ] || note "left:$left"
echo
echo "G3: $((OKS + inner_oks)) ok, $FAILS FAIL in $(elapsed)s"
finish G3
