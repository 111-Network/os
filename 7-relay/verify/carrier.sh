#!/usr/bin/env bash
# 7-relay/verify/carrier.sh — Layer 7 gate G10 (spec §15 G10; §5.9, §5.10, §6.1–§6.4; row V4):
#   the ntfy carrier; gateway hubs G and G′ in network namespaces, joined only through stub-ntfy.py.
#
#   bash 7-relay/verify/carrier.sh                  G10: runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G10=1 bash 7-relay/verify/carrier.sh    lock run: a missing prerequisite is a FAIL
#   REQUIRE_NTFY=1 ...                              the real-server run (NTFY_BIN, a pinned ntfy release) becomes a FAIL where
#                                                   it cannot run; without NTFY_BIN its items are SKIPPED (notes below)
#   bash 7-relay/verify/carrier.sh --smoke          row P1: the prototype's smoke sequence against the stub;
#                                                   writes 7-relay/verify/logs/ntfy-smoke-<calver>-stub.md
#
# G10 (row V4). One user + net + pid namespace (`unshare -rnpf`, so everything dies with it) holds the bridge l7g10wan
# (10.111.71.0/24) and five network namespaces, each its own `NODE111_ROOT`:
#   NTFY .1  the stub server (python3, stdlib)       G .11 / GP .12  the two gateway hubs, a real `111 daemon` each
#   LG   10.111.70.11 on G's LAN (G .1)              LGP 10.111.80.11 on G′'s LAN (G′ .1): members of G and of G′
# G and GP enrol at nothing but each other once (a `carrier` pair's enrolment rides tcp-lan, §4.6 step 3), then the two
# bridge ports are isolated (`bridge_slave isolated on`: G and GP cannot reach each other at layer 2, NTFY reaches both)
# and a sniffer on each gateway's eth0 proves that no frame addressed to the other gateway leaves it from then on (with a
# positive control: a datagram to it is seen). From there on the only path between the hubs is the stub.
#
# What each part proves, and how a broken daemon turns it FAIL (each is also run against a negative where it was cheap):
#   1  5A: the stub's access log (every request: ip, method, topic, status, bytes) is empty while nothing happened, a
#      row waiting with the stub up makes no request over a quiet window, and a Forward event / a route added in the
#      netns / Flush now each make exactly GET(drain) POST GET(drain) on the two topics, then `ss`, the daemon's
#      children and its thread names show no connection, process or 5B thread. A daemon that polled, held a stream, or
#      posted twice turns the sequence or the idle checks FAIL.
#   2  the row stays `pending` after the post answered 200 and commits only after G′'s ack has been posted on the
#      reverse topic AND G has drained it; the sender's thread reads `relayed` (reason `forwarding`) until the far
#      member's ACK has crossed back; the far member holds the message once.
#   3  5B: one ESTABLISHED socket to the stub and one thread `111d-ntfy-5b`; a stub restart (cache lost) → one re-post
#      and one delivery; re-posts counted per round (`reposts`); a dropped ack heals by the re-post and a DUPLICATE; an
#      ack is posted once.
#   4  the largest POST body of the whole run is ≤ 4,096 B (so a 4,097 B body was never posted); an item over 2,937 B is
#      refused at the gate (`forward.too_large`) and posted never; a 413 (a stub with a small limit) returns the row
#      (refused, counted); a 429 keeps it and the daemon makes no retry of its own (no timer).
#   5  a third client polling the topic reads base64url batches whose frames hold neither the message text nor any
#      NodeId; a batch it posts (a frame no hub sealed) is dropped and counted in the fabric's `dropped`; a made-up topic
#      yields nothing; only the two topics are ever used.
#   6  metered: past the cap no flush (HTTP 409 `metered_cap`, no request from the daemon at all); raising it (consent
#      again) resumes. The licensed-board refusal has no wire source in v1 (a note, see DEFECTS FOUND in the row's
#      report). Switch off: strace on every daemon thread sees no connect() to the stub while events fire (the same
#      strace sees it while the switch is on: the positive control).
#   7  writes logs/ntfy-smoke-<calver>-stub.md (a stub run is never the real server's name): this run's section
#      "## End-to-end between two gateway hubs in netns (G10)" and the records, keeping P1's sections when its file exists.
#
# Env: BIN111 (the `111` binary; else $CARGO_TARGET_DIR/{release,debug}/111) · G10_SETTLE (3 s: how long an expected
# non-event is watched) · G10_QUIET (10 s: the window of item 1's "no request until an event") · TMPDIR for the scratch
# root · G10_KEEP=1 keeps it · G10_STOP_AFTER=<phase> (a dev knob, refused under REQUIRE_G10=1) · G10_NEG=<name> negative
# controls: `poll` (a stray GET from G's address in item 1's quiet window: the "no request until an event" line must turn FAIL; the run exits 1).
# the jq filters and the wait_for conditions are single-quoted on purpose (jq variables, eval'd conditions)
# shellcheck disable=SC2016,SC2034,SC2013,SC2015,SC2329
set -u
export LC_ALL=C
# a proxy in the environment would hijack the daemon's ureq client (it reads *_proxy) and every curl here: the gate talks
# to a stub at a private address and must reach it directly
unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY all_proxy ALL_PROXY no_proxy NO_PROXY
SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=7-relay/verify/lib.sh disable=SC1091
. 7-relay/verify/lib.sh

STUB=$L7_VERIFY/stub-ntfy.py
EGRESS=$L7_OS/6-transport/verify/egress-log.py
PORT=$L7_PORT_FABRIC
NP=$L7_PORT_NTFY
SETTLE=${G10_SETTLE:-3}
QUIET=${G10_QUIET:-10}
DAPI=http://127.0.0.1:$L7_PORT_DASH
NTFY_URL=http://$L7_G10_IP_NTFY:$NP

# ------------------------------------------------------------------ the P1 smoke mode (unchanged)
stub_pid=""
scratch=""
cleanup_smoke() {
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
  trap cleanup_smoke EXIT

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
  # keep G10's own section across the rewrite (the cargo test rewrites the whole file)
  [ -f "$log" ] && cp "$log" "$scratch/prev.md"
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
  if [ -s "$scratch/prev.md" ] && [ -s "$log" ]; then
    python3 -I - "$scratch/prev.md" "$log" <<'PY2'
import re, sys
prev, new = sys.argv[1:3]
m = re.search(r"## End-to-end between two gateway hubs in netns \(G10\)\n.*?(?=\n## |\Z)", open(prev).read(), flags=re.S)
if m:
    with open(new, "a") as f:
        f.write("\n" + m.group(0).rstrip("\n") + "\n")
PY2
  fi
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

# ------------------------------------------------------------------ G10: python helpers (stdlib)
write_tools() { # DIR
  # tp.py: the "third client" of item 5 and the topic readers.
  #   tp.py poll BASE TOPIC [SINCE]      -> JSON {"status", "n", "msgs": [{"id", "len", "b64": ..}]} (a poll; since=all by default)
  #   tp.py post BASE TOPIC BODYKIND     -> HTTP status of a POST of a batch: forged | text
  #   tp.py sealed BASE TOPIC MARKER... -> JSON {"batches", "frames", "frame_lens", "found": [markers found in the base64 text or the frames]}
  cat >"$1/tp.py" <<'PY'
import base64, json, sys, urllib.error, urllib.request

opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def req(method, url, body=None):
    r = urllib.request.Request(url, data=body, method=method)
    if body is not None:
        r.add_header("Content-Type", "text/plain")
    try:
        with opener.open(r, timeout=10) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def poll(base, topic, since="all"):
    s, b = req("GET", "%s/%s/json?poll=1&since=%s" % (base, topic, since))
    msgs = [json.loads(l) for l in b.decode().splitlines() if l.strip()] if s == 200 else []
    return s, [m for m in msgs if m.get("event") == "message"]


def unb64(t):
    return base64.urlsafe_b64decode(t + "=" * (-len(t) % 4))


def frames(raw):  # 0x01 | n | (len u16 BE | frame)...
    assert raw[0] == 1, "batch version"
    n, o, out = raw[1], 2, []
    for _ in range(n):
        ln = int.from_bytes(raw[o:o + 2], "big")
        out.append(raw[o + 2:o + 2 + ln])
        o += 2 + ln
    assert o == len(raw), "batch length"
    return out


cmd = sys.argv[1]
if cmd == "poll":
    s, msgs = poll(sys.argv[2], sys.argv[3], sys.argv[4] if len(sys.argv) > 4 else "all")
    print(json.dumps({"status": s, "n": len(msgs), "msgs": [{"id": m["id"], "len": len(m["message"])} for m in msgs]}))
elif cmd == "post":
    forged = b"\x5a" * 400
    raw = b"\x01\x01" + len(forged).to_bytes(2, "big") + forged
    body = base64.urlsafe_b64encode(raw).decode().rstrip("=")
    s, _ = req("POST", "%s/%s" % (sys.argv[2], sys.argv[3]), body.encode())
    print(s)
elif cmd == "sealed":
    s, msgs = poll(sys.argv[2], sys.argv[3])
    marks = [(bytes.fromhex(m[4:]) if m.startswith("hex:") else m.encode()) for m in sys.argv[4:]]
    labels = {k: m for k, m in zip(marks, sys.argv[4:])}
    found, flens, nfr, bad = [], [], 0, 0
    for m in msgs:
        text = m["message"].encode()
        for k in marks:
            if k in text:
                found.append(labels[k][:12])
        try:
            raw = unb64(m["message"])
            for f in frames(raw):
                nfr += 1
                flens.append(len(f))
                for k in marks:
                    if k in f:
                        found.append(labels[k][:12])
        except Exception:
            bad += 1
    print(json.dumps({"status": s, "batches": len(msgs), "frames": nfr, "frame_lens": flens, "found": found, "undecodable": bad}))
PY
}

# ------------------------------------------------------------------ inner: runs inside the namespaces
declare -A HOLD=() ROOT=() DPID=() NID=() IPS=() VETH=()
HELPERS=()
ST=000
W=""
BIN=""
STUB_PID=""
STUBP=()
SNIFFP=()

nx() {
  local n=$1
  shift
  nsenter -t "${HOLD[$n]}" -n -- "$@"
}
api() { # NODE METHOD PATH [JSON] -> $ST, body in $W/body
  local n=$1 m=$2 p=$3 d=${4-}
  local args=(-sS -o "$W/body" -w '%{http_code}' -X "$m" --max-time 30)
  [ -n "$d" ] && args+=(-H 'Content-Type: application/json' -d "$d")
  : >"$W/body"
  ST=$(nsenter -t "${HOLD[$n]}" -n -- curl "${args[@]}" "$DAPI$p" 2>"$W/curl.err") || true
  [ -n "$ST" ] || ST=000
}
has_json() { [ -s "$1" ] && grep -q '[^[:space:]]' "$1" 2>/dev/null; }
jget() { jq -r "$@" "$W/body" 2>/dev/null || true; }
jnum() { # a counter of the last body, -1 when absent or not a number (so arithmetic on it never aborts the run)
  local v
  v=$(jget "$1")
  case $v in '' | *[!0-9]*) echo -1 ;; *) echo "$v" ;; esac
}
jb() { has_json "$W/body" && jq -e "$@" "$W/body" >/dev/null 2>&1; }
snip() { head -c 300 "$W/body" 2>/dev/null | tr '\n\r' '  '; }
wait_for() { # SECONDS CONDITION (eval'd)
  local end=$((SECONDS + $1))
  while :; do
    if eval "$2"; then return 0; fi
    [ "$SECONDS" -ge "$end" ] && return 1
    sleep 0.3
  done
}
# t DESC CONDITION: an ok/FAIL line; the condition reads the last $ST and body
t() {
  if eval "$2"; then ok "$1"; else fail "$1  [HTTP ${ST:-?}: $(snip)]"; fi
}
# tw SECONDS DESC CONDITION: waits for the condition, then reports it
tw() {
  wait_for "$1" "$3" || true
  if ! eval "$3"; then diag_state; fi
  t "$2" "$3"
}
# diag_state: the counters a failed wait is judged by
diag_state() {
  note "state: pending G=$(pend G GP) GP=$(pend GP G), connections G=$(carrier_conns G) GP=$(carrier_conns GP), 5B threads G=$(threads_named G 111d-ntfy-5b), stub log lines $(nlines)"
}

# ---- the stub (in the NTFY namespace), its access log and what the log says. A background job is `nsenter … &` itself,
# never the nx function: the pid kept must be the process that gets the signal.
NLOG=""
stub_start() { # NAME PORT [stub args…] -> the access log $W/ntfy-NAME.jsonl; the pid in STUBP
  local name=$1 port=$2
  shift 2
  NLOG=$W/ntfy-$name.jsonl
  NLOGMAP[$name]=$NLOG
  : >"$NLOG"
  nsenter -t "${HOLD[NTFY]}" -n -- python3 -I "$STUB" --host "$L7_G10_IP_NTFY" --port "$port" --quiet --access-log "$NLOG" "$@" >"$W/stub-$name.out" 2>"$W/stub-$name.err" &
  STUBP+=($!)
  STUB_PID=$!
  if wait_for 10 '[ -s "$W/stub-$name.out" ]'; then return 0; fi
  fail "the stub $name printed no port ($(head -c 200 "$W/stub-$name.err"))"
  return 1
}
stub_kill() {
  [ -n "$STUB_PID" ] || return 0
  kill "$STUB_PID" 2>/dev/null
  wait "$STUB_PID" 2>/dev/null
  STUB_PID=""
}
nlines() { wc -l <"$NLOG" | tr -d ' '; }
# reqs_after N [JQ-FILTER] -> the access-log lines after the first N, one compact JSON line each, as `<ip-name> <M> <topic> <status> <req> <poll|held|post>`
reqs_after() {
  tail -n +"$(($1 + 1))" "$NLOG" | jq -r --arg g "$L7_G10_IP_G" --arg gp "$L7_G10_IP_GP" '
    (.ip | if . == $g then "G" elif . == $gp then "GP" else .
     end) as $who
    | (.p | ltrimstr("/") | split("?")[0] | split("/")[0]) as $topic
    | (if .m == "POST" then "post" elif (.p | contains("poll=1")) then "poll" elif (.p | startswith("/v1/health")) then "health" else "held" end) as $kind
    | "\($who) \(.m) \($topic) \(.s) \(.req) \($kind)"' 2>/dev/null
}
# seq_after N -> "G:poll:<t>,G:post:<t>,…" with the topics named TAB (G→GP), TBA (GP→G) when known
TAB=""
TBA=""
seq_after() {
  local line who kind topic out=""
  while read -r who _ topic _ _ kind; do
    [ -n "$who" ] || continue
    case $topic in "$TAB") topic=AB ;; "$TBA") topic=BA ;; esac
    out="$out$who:$kind:$topic "
  done < <(reqs_after "$1")
  printf '%s' "${out% }"
}
count_after() { # N WHO KIND [TOPIC-NAME AB|BA]
  local want=${4:-} n=0 who kind topic
  while read -r who _ topic _ _ kind; do
    [ "$who" = "$2" ] && [ "$kind" = "$3" ] || continue
    case $topic in "$TAB") topic=AB ;; "$TBA") topic=BA ;; esac
    if [ -z "$want" ] || [ "$topic" = "$want" ]; then n=$((n + 1)); fi
  done < <(reqs_after "$1")
  printf '%s' "$n"
}

# ---- the carrier and the pair, read from /relay
relay() { api "$1" GET /api/v1/relay; }
pend() { # NODE FARNODE -> pending of that pair
  relay "$1"
  jget --arg f "${NID[$2]}" '[.paired_hubs[]? | select(.node_id == $f)] | first | .pending // -1'
}
reposts() { # NODE FARNODE
  relay "$1"
  jget --arg f "${NID[$2]}" '[.paired_hubs[]? | select(.node_id == $f)] | first | .reposts // -1'
}
fabric_counter() { # NODE KEY
  api "$1" GET /api/v1/fabric
  jget ".counters.$2 // -1"
}
# patch NODE JSON -> $ST; a consent_required round trip is answered once with the hash (CONSENTS counts them)
CONSENTS=0
patch() {
  api "$1" PATCH /api/v1/relay/settings "$2"
  if [ "$ST" = 409 ] && [ "$(jget .code)" = consent_required ]; then
    local h
    h=$(jget .consent.hash)
    CONSENTS=$((CONSENTS + 1))
    api "$1" PATCH /api/v1/relay/settings "$(jq -nc --argjson p "$2" --arg h "$h" '$p + {accept_warning: $h}')"
  fi
}
# carrier_idle NODE -> 0 when the daemon holds no connection to the stub (not TIME-WAIT), has no extra child and no 5B thread
carrier_conns() { # NODE -> number of sockets to the stub that are not TIME-WAIT
  nx "$1" ss -tanH dst "$L7_G10_IP_NTFY:$NP" 2>/dev/null | grep -vc TIME-WAIT
}
daemon_pid() {
  local p=${DPID[$1]:-}
  [ -n "$p" ] || return 1
  if [ "$(cat /proc/"$p"/comm 2>/dev/null)" = 111 ]; then
    printf '%s\n' "$p"
  else
    pgrep -P "$p" -x 111 | head -n 1
  fi
}
threads_named() { # NODE NAME
  local pid th n=0
  pid=$(daemon_pid "$1") || return 0
  for th in /proc/"$pid"/task/*; do
    [ "$(cat "$th/comm" 2>/dev/null)" = "$2" ] && n=$((n + 1))
  done
  printf '%s' "$n"
}
children_of() { # NODE -> sorted "comm" of the daemon's children
  local pid c
  pid=$(daemon_pid "$1") || return 0
  for c in $(cat /proc/"$pid"/task/*/children 2>/dev/null); do cat /proc/"$c"/comm 2>/dev/null; done | sort | tr '\n' ' '
}
rss_kb() { awk '/^VmRSS:/ { print $2 }' "/proc/$(daemon_pid "$1")/status" 2>/dev/null; }
hwm_kb() { awk '/^VmHWM:/ { print $2 }' "/proc/$(daemon_pid "$1")/status" 2>/dev/null; }
# idle_ok NODE -> 0 when nothing of the carrier is left open
idle_ok() {
  [ "$(carrier_conns "$1")" = 0 ] && [ "$(threads_named "$1" 111d-ntfy-5b)" = 0 ]
}

# ---- messages
MID=""
send() { # FROM TO TEXT
  api "$1" POST /api/v1/messages "$(jq -nc --arg to "${NID[$2]}" --arg t "$3" '{to: $to, text: $t}')"
  MID=$(jget .id)
}
# mstate NODE PEER MID -> "<state>/<reason_code>"
mstate() {
  api "$1" GET "/api/v1/messages/threads/${NID[$2]}?limit=100"
  jget --arg id "$3" '[.messages[]? | select(.id == $id)] | first | "\(.state)/\(.reason_code // "-")"'
}
# nrecv NODE PEER TEXT -> how many messages with that text the thread holds
nrecv() {
  api "$1" GET "/api/v1/messages/threads/${NID[$2]}?limit=100"
  jget --arg x "$3" '[.messages[]? | select(.text == $x)] | length'
}
flush_now() { api "$1" POST /api/v1/relay/carrier/flush '{}'; }
check_now() { api "$1" POST /api/v1/relay/carrier/check '{}'; }
# idle_wait NODE -> waits until no connection is open and the daemon's last request is over (the stub log quiet for 1.5 s)
quiet_log() { # waits until the access log has not grown for 1.5 s (max 20 s)
  local a b end=$((SECONDS + 20))
  a=$(nlines)
  while [ "$SECONDS" -lt "$end" ]; do
    sleep 1.5
    b=$(nlines)
    [ "$a" = "$b" ] && return 0
    a=$b
  done
  return 1
}
# one full round trip of the carrier between the gateways: G's put reaches GP, GP's acks and G's get back
pump() { # waits for G's pending to drain and (optionally, $1 = MID) the far ack
  local i
  for i in 1 2 3 4 5 6; do
    flush_now GP
    quiet_log
    flush_now G
    quiet_log
    [ "$(pend G GP)" = 0 ] && [ "$(pend GP G)" = 0 ] && return 0
  done
  return 1
}

# ---- the gateways' own egress: nothing addressed to the other gateway leaves a gateway's eth0
declare -A SNIFFPID=()
sniff_start() { # NAME NODE OWNIP OTHERIP
  local n=$1 node=$2
  nsenter -t "${HOLD[$node]}" -n -- python3 -I "$EGRESS" sniff --iface eth0 --allow "$3,$L7_G10_IP_NTFY" --control-dst "$4" \
    --out "$W/$n.jsonl" --summary "$W/$n.summary.json" --ready "$W/$n.ready" >"$W/$n.err" 2>&1 &
  SNIFFPID[$n]=$!
  SNIFFP+=($!)
  wait_for 8 '[ -s "$W/$n.ready" ]'
}
sniff_halt() { # NAME: SIGTERM the logger (it writes its summary on it) and wait for it, in this shell
  local p=${SNIFFPID[$1]:-}
  [ -n "$p" ] || return 1
  kill -TERM "$p" 2>/dev/null
  wait "$p" 2>/dev/null
  SNIFFPID[$1]=""
}
sniff_counts() { # NAME -> "<off_link_out> <control_out> <peer_out>"
  python3 -I -c 'import json,sys
c = json.load(open(sys.argv[1]))["counts"]
print(c.get("off_link_out", 0), c.get("control_out", 0), c.get("peer_out", 0))' "$W/$1.summary.json" 2>/dev/null || echo "? ? ?"
}

stop_node() { # NODE: TERM, then KILL
  local p=${DPID[$1]:-}
  [ -n "$p" ] || return 0
  kill -TERM "$p" 2>/dev/null || true
  for _ in $(seq 1 100); do
    kill -0 "$p" 2>/dev/null || break
    sleep 0.2
  done
  kill -KILL "$p" 2>/dev/null || true
  wait "$p" 2>/dev/null || true
}
start_node() { # NODE [KEY=VAL...: extra environment of the daemon]
  local n=$1 up=0 code
  shift
  NODE111_ROOT=${ROOT[$n]} nsenter -t "${HOLD[$n]}" -n -- env "$@" "$BIN" daemon >>"${ROOT[$n]}/daemon.log" 2>&1 &
  DPID[$n]=$!
  for _ in $(seq 1 300); do
    code=$(nx "$n" curl -s -o /dev/null -w '%{http_code}' --max-time 2 "$DAPI/health" 2>/dev/null) || true
    case $code in 200 | 503) up=1 && break ;; esac
    kill -0 "${DPID[$n]}" 2>/dev/null || break
    sleep 0.2
  done
  if [ "$up" != 1 ]; then
    fail "$n: 111 daemon never answered /health"
    tail -20 "${ROOT[$n]}/daemon.log" | sed "s/^/  $n daemon| /"
    return 1
  fi
  local _sn=$n
  wait_for 30 'api "$_sn" GET /api/v1/fabric; [ "$ST" = 200 ] && jb ".store.open == true"' || true
  return 0
}

# ---- more stubs: a second and third server (the 413 and 429 phases), the access log in use, a restart that loses the cache
declare -A NLOGMAP=()
use_log() { NLOG=${NLOGMAP[$1]}; }
stub_restart() { # the main stub: the same port and log, the cache lost
  stub_kill
  nsenter -t "${HOLD[NTFY]}" -n -- python3 -I "$STUB" --host "$L7_G10_IP_NTFY" --port "$NP" --quiet --access-log "${NLOGMAP[main]}" --burst 5000 --replenish 0.01 --keepalive 5 \
    >"$W/stub-main.restart.out" 2>>"$W/stub-main.err" &
  STUBP+=($!)
  STUB_PID=$!
  wait_for 10 '[ -s "$W/stub-main.restart.out" ]'
}
# status_after N WHO KIND -> the status codes of those requests, space separated
status_after() {
  local who kind st out=""
  while read -r who _ _ st _ kind; do
    [ "$who" = "$2" ] && [ "$kind" = "$3" ] && out="$out$st "
  done < <(reqs_after "$1")
  printf '%s' "${out% }"
}
# max_post_body -> the largest POST body (bytes) any gateway sent to the stub in log NLOG
max_post_body() { jq -s '[.[] | select(.m == "POST") | .req] | max // 0' "$NLOG"; }
# topics_used -> the distinct topics of the gateways' requests, one per line
topics_used() {
  jq -r --arg g "$L7_G10_IP_G" --arg gp "$L7_G10_IP_GP" 'select(.ip == $g or .ip == $gp) | .p | ltrimstr("/") | split("?")[0] | split("/")[0] | select(. != "v1")' "$NLOG" | sort -u
}
tp() { nx NTFY python3 -I "$W/tp.py" "$@"; }
# strace_connects NODE SECONDS: every thread of the daemon under `strace -f -e trace=connect`, in the background
strace_start() {
  local pid pl=() th
  pid=$(daemon_pid "$1")
  for th in /proc/"$pid"/task/*; do pl+=(-p "${th##*/}"); done
  : >"$W/strace-$1.log"
  timeout "$2" strace -f -qq -e trace=connect -o "$W/strace-$1.log" "${pl[@]}" >/dev/null 2>"$W/strace-$1.err" &
  STRACEP=$!
  sleep 1
}
strace_wait() { wait "$STRACEP" 2>/dev/null; }
strace_to_stub() { grep -c "sin_port=htons($NP)" "$W/strace-$1.log" 2>/dev/null; }
STRACEP=""

# the log-shape functions are the gate's eyes: before any daemon runs they are shown to FAIL on a log that is wrong
selftest() {
  local save=$NLOG
  NLOG=$W/selftest.jsonl
  local g=$L7_G10_IP_G gp=$L7_G10_IP_GP
  TAB=111aaaa
  TBA=111bbbb
  {
    jq -nc --arg ip "$g" '{ip: $ip, m: "GET", p: "/111bbbb/json?poll=1&since=all", s: 200, req: 0}'
    jq -nc --arg ip "$g" '{ip: $ip, m: "POST", p: "/111aaaa", s: 200, req: 1500}'
    jq -nc --arg ip "$g" '{ip: $ip, m: "GET", p: "/111bbbb/json?poll=1&since=abc", s: 200, req: 0}'
    jq -nc --arg ip "$gp" '{ip: $ip, m: "GET", p: "/111aaaa/json", s: 200, req: 0}'
  } >"$NLOG"
  if [ "$(seq_after 0)" = "G:poll:BA G:post:AB G:poll:BA GP:held:AB" ] && [ "$(count_after 0 G post AB)" = 1 ] && [ "$(count_after 0 G post BA)" = 0 ] &&
    [ "$(count_after 1 G poll)" = 1 ] && [ "$(max_post_body)" = 1500 ] && [ "$(status_after 0 G poll)" = "200 200" ]; then
    ok "selftest: the log readers name each request (who, kind, topic AB/BA), count them, find the largest POST body"
  else
    fail "selftest: the log readers read a synthetic log wrongly ($(seq_after 0) | $(count_after 0 G post AB) | $(max_post_body))"
  fi
  # a shape with one request too many must not equal the wanted shape
  jq -nc --arg ip "$g" '{ip: $ip, m: "POST", p: "/111aaaa", s: 200, req: 10}' >>"$NLOG"
  if [ "$(seq_after 0)" != "G:poll:BA G:post:AB G:poll:BA GP:held:AB" ] && [ "$(count_after 0 G post AB)" = 2 ]; then
    ok "selftest: a second POST changes the shape and the count (the one-post-per-flush check can fail)"
  else
    fail "selftest: an extra POST was not noticed by the log readers"
  fi
  NLOG=$save
}

bundle_set_address() { # BUNDLE ADDRESS ("" strips it)
  python3 -I -c '
import sys
b, a = sys.argv[1:3]
p = b.split(":")
print(":".join(p[:4] + [a] + p[-2:]))' "$1" "$2"
}

inner() {
  # negative control: an inner run killed part-way must make the outer run FAIL (no sentinel)
  [ "${G10_NEG:-}" = die ] && exit 9
  W=${G10_WORK:?}
  BIN=${G10_BIN:?}
  local n i
  IPS[NTFY]=$L7_G10_IP_NTFY IPS[G]=$L7_G10_IP_G IPS[GP]=$L7_G10_IP_GP
  VETH[NTFY]=$L7_G10_VETH_NTFY VETH[G]=$L7_G10_VETH_G VETH[GP]=$L7_G10_VETH_GP

  cleanup_inner() {
    local p
    for p in "${SNIFFP[@]}" "${STUBP[@]}"; do kill "$p" 2>/dev/null; done
    for n in G GP LG LGP; do
      p=${DPID[$n]:-}
      [ -n "$p" ] && kill -TERM "$p" 2>/dev/null
    done
    for _ in $(seq 1 50); do
      local alive=0
      for n in G GP LG LGP; do kill -0 "${DPID[$n]:-0}" 2>/dev/null && alive=1; done
      [ "$alive" = 0 ] && break
      sleep 0.2
    done
    for n in G GP LG LGP; do
      p=${DPID[$n]:-}
      [ -n "$p" ] && kill -KILL "$p" 2>/dev/null
    done
    for p in "${HELPERS[@]}"; do kill "$p" 2>/dev/null; done
    wait 2>/dev/null
  }
  trap cleanup_inner EXIT
  trap 'exit 130' INT TERM

  ok "namespaces: user + net + pid, /proc is this namespace's own"
  write_tools "$W"

  # ================================================================== 1. topology
  ip link set lo up
  if ! { ip link add "$L7_G10_WAN_BR" type bridge && ip link set "$L7_G10_WAN_BR" up; }; then
    fail "cannot create the bridge $L7_G10_WAN_BR"
    return
  fi
  for n in $L7_G10_NODES; do
    unshare --net sleep 100000 &
    HOLD[$n]=$!
    HELPERS+=("$!")
  done
  sleep 0.5
  local built=1
  for n in NTFY G GP; do
    if ip link add "${VETH[$n]}" type veth peer name "${VETH[$n]}p" &&
      ip link set "${VETH[$n]}p" netns "${HOLD[$n]}" &&
      nx "$n" ip link set "${VETH[$n]}p" name eth0 &&
      nx "$n" ip addr add "${IPS[$n]}/24" dev eth0 && nx "$n" ip link set eth0 up && nx "$n" ip link set lo up &&
      ip link set "${VETH[$n]}" master "$L7_G10_WAN_BR" && ip link set "${VETH[$n]}" up; then
      :
    else
      built=0
    fi
  done
  # the LANs: G (.1) with LG (.11), GP (.1) with LGP (.11); a veth pair across two namespaces each
  local pair hub leaf hubip leafip vh vl
  for pair in "G LG $L7_G10_LAN_IP_G $L7_G10_IP_LG $L7_G10_VETH_LG" "GP LGP $L7_G10_LAN_IP_GP $L7_G10_IP_LGP $L7_G10_VETH_LGP"; do
    read -r hub leaf hubip leafip vl <<<"$pair"
    vh=${vl}x
    if ip link add "$vl" type veth peer name "$vh" &&
      ip link set "$vl" netns "${HOLD[$leaf]}" && ip link set "$vh" netns "${HOLD[$hub]}" &&
      nx "$leaf" ip link set "$vl" name eth0 && nx "$hub" ip link set "$vh" name eth1 &&
      nx "$leaf" ip addr add "$leafip/24" dev eth0 && nx "$hub" ip addr add "$hubip/24" dev eth1 &&
      nx "$leaf" ip link set eth0 up && nx "$hub" ip link set eth1 up && nx "$leaf" ip link set lo up; then
      :
    else
      built=0
    fi
  done
  # a default route through the WAN port, so a metered rule that names the interface of the default route matches eth0
  nx G ip route add default via "$L7_G10_IP_NTFY" dev eth0 && nx GP ip route add default via "$L7_G10_IP_NTFY" dev eth0 || built=0
  if [ "$built" = 1 ]; then
    ok "five netns: NTFY ${IPS[NTFY]}, G ${IPS[G]} + LAN $L7_G10_LAN_IP_G (LG $L7_G10_IP_LG), GP ${IPS[GP]} + LAN $L7_G10_LAN_IP_GP (LGP $L7_G10_IP_LGP) on $L7_G10_WAN_BR $L7_G10_WAN_NET"
  else
    fail "the topology could not be built"
    return
  fi
  if nx G ping -c1 -W2 "${IPS[GP]}" >/dev/null 2>&1; then ok "before pairing G reaches GP over the bridge (ping): the one-off enrolment has a path"; else note "no ping answer between G and GP (ICMP may be unavailable); the enrolment decides"; fi

  # ================================================================== 2. the stub, the daemons
  stub_start main "$NP" --burst 5000 --replenish 0.01 --keepalive 5 || return
  local sp
  sp=$(cat "$W/stub-main.out")
  if [ "$sp" = "$NP" ] && nx NTFY ss -ltnH 2>/dev/null | grep -q "${IPS[NTFY]}:$NP "; then
    ok "stub-ntfy.py listens on ${IPS[NTFY]}:$NP in the NTFY namespace (the only host the two gateways may reach)"
  else
    fail "the stub did not bind ${IPS[NTFY]}:$NP (printed '$sp')"
    return
  fi
  for n in G GP LG LGP; do
    ROOT[$n]=$W/root-$n
    g6_make_root "${ROOT[$n]}" ""
    NID[$n]=$(g6_identity "${ROOT[$n]}" "$BIN")
    [ -n "${NID[$n]}" ] || {
      fail "$n: no identity ($(head -c 200 "${ROOT[$n]}/identity.log"))"
      return
    }
    case $n in
      G | GP) g6_fabric_settings "${ROOT[$n]}" '{"schema":1,"hub_role":true,"listen_lan":true}' ;;
      *) g6_fabric_settings "${ROOT[$n]}" '{"schema":1}' ;;
    esac
    printf '{"schema":1,"updates":{"check_internet":false}}\n' >"${ROOT[$n]}/state/relay.json"
    chmod 600 "${ROOT[$n]}/state/relay.json"
  done
  for n in G GP LG LGP; do start_node "$n" || return; done
  ok "four real daemons up with a fabric: G ${NID[G]:0:8}, GP ${NID[GP]:0:8}, LG ${NID[LG]:0:8}, LGP ${NID[LGP]:0:8}"
  note "daemon threads before any switch: G: $(for th in /proc/"$(daemon_pid G)"/task/*; do cat "$th/comm"; done | sort | uniq -c | tr '\n' ';')"
  [ "$(threads_named G 111d-ntfy-5b)" = 0 ] || fail "G has a 111d-ntfy-5b thread with every switch off"
  [ "$(nlines)" = 0 ] || fail "the stub saw $(nlines) request(s) before any switch was on"

  # ---- enrolment: each leaf at its own gateway, over the gateway's LAN address
  local code card
  for pair in "G LG $L7_G10_LAN_IP_G" "GP LGP $L7_G10_LAN_IP_GP"; do
    read -r hub leaf hubip <<<"$pair"
    api "$hub" POST /api/v1/fabric/hub/offer '{}'
    code=$(jget .code)
    card=$(jget .card.text)
    api "$leaf" POST /api/v1/fabric/hub/enroll "$(jq -nc --arg a "$hubip:$PORT" --arg c "$code" --arg k "$card" '{address: $a, code: $c, card: $k}')"
    t "$leaf: enrolled at its gateway $hub ($hubip:$PORT) and reachable" '[ "$ST" = 200 ] && jb ".enrolled == true and .reachable == true" && [ "$(jget .node_id)" = "${NID[$hub]}" ]'
  done
  local card_lg card_lgp
  api LG GET /api/v1/contacts/me/card
  card_lg=$(jget .text)
  api LGP GET /api/v1/contacts/me/card
  card_lgp=$(jget .text)
  api LG POST /api/v1/contacts "$(jq -nc --arg c "$card_lgp" '{card: $c}')"
  t "LG holds LGP's card (the home-hub hint names G′)" '[ "$ST" = 200 ] && jb ".has_key == true"'
  api LGP POST /api/v1/contacts "$(jq -nc --arg c "$card_lg" '{card: $c}')"
  t "LGP holds LG's card (the hint names G)" '[ "$ST" = 200 ] && jb ".has_key == true"'

  # each member also holds the far hub's card (a hub is a contact with the mailbox role): a put whose hint names it reads `forwarding`
  local card_g card_gp
  api G GET /api/v1/contacts/me/card
  card_g=$(jget .text)
  api GP GET /api/v1/contacts/me/card
  card_gp=$(jget .text)
  api LG POST /api/v1/contacts "$(jq -nc --arg c "$card_gp" '{card: $c}')"
  t "LG holds G′'s own card (the hub the hint names)" '[ "$ST" = 200 ]'
  api LGP POST /api/v1/contacts "$(jq -nc --arg c "$card_g" '{card: $c}')"
  t "LGP holds G's own card" '[ "$ST" = 200 ]'

  # ================================================================== 3. the carrier pair, then the stub is the only path
  local bg bgp
  api G POST /api/v1/relay/pair/offer '{"role":"peer","via":"carrier"}'
  bg=$(jget .bundle.text)
  api GP POST /api/v1/relay/pair/offer '{"role":"peer","via":"carrier"}'
  bgp=$(jget .bundle.text)
  if [[ "$bg" == 111PAIR:* ]] && [[ "$bgp" == 111PAIR:* ]]; then
    ok "two carrier offers: 111PAIR bundles (G's, G′'s)"
  else
    fail "the carrier offers failed ('${bg:0:30}', '${bgp:0:30}')"
    return
  fi
  # G scans G′'s bundle with no address (G dials nothing); G′ scans G's with G's WAN address and enrols over tcp-lan
  api G POST /api/v1/relay/pair "$(jq -nc --arg b "$(bundle_set_address "$bgp" "")" '{bundle: $b, role: "peer", via: "carrier"}')"
  local st1=$ST
  api GP POST /api/v1/relay/pair "$(jq -nc --arg b "$(bundle_set_address "$bg" "${IPS[G]}:$PORT")" '{bundle: $b, role: "peer", via: "carrier"}')"
  local st2=$ST
  note "G scanned G′'s bundle (HTTP $st1), G′ scanned G's (HTTP $st2)"
  whole_gp() { relay GP; jb --arg f "${NID[G]}" '[.paired_hubs[]? | select(.node_id == $f and .via == "carrier")] | length == 1'; }
  whole_g() { relay G; jb --arg f "${NID[GP]}" '[.paired_hubs[]? | select(.node_id == $f and .via == "carrier")] | length == 1'; }
  tw 40 "both gateways record the other as a paired hub, via carrier" 'whole_g && whole_gp'
  relay G
  note "G's pair after pairing: $(jget -c '.paired_hubs')"
  [ "${G10_STOP_AFTER:-}" = pair ] && return

  ip link set dev "$L7_G10_VETH_G" type bridge_slave isolated on && ip link set dev "$L7_G10_VETH_GP" type bridge_slave isolated on || fail "cannot isolate the two gateway ports on $L7_G10_WAN_BR"
  if nx G timeout 3 bash -c "exec 3<>/dev/tcp/${IPS[GP]}/$PORT" 2>/dev/null; then
    fail "G still reaches G′'s fabric port after the isolation: the stub is not the only path"
  else
    ok "after the pairing the ports of G and G′ are isolated on $L7_G10_WAN_BR: G cannot open ${IPS[GP]}:$PORT (the stub, a port that is not isolated, is the only host both reach)"
  fi
  if nx G timeout 3 bash -c "exec 3<>/dev/tcp/${IPS[NTFY]}/$NP" 2>/dev/null && nx GP timeout 3 bash -c "exec 3<>/dev/tcp/${IPS[NTFY]}/$NP" 2>/dev/null; then
    ok "both gateways still reach the stub (${IPS[NTFY]}:$NP)"
  else
    fail "a gateway cannot reach the stub after the isolation"
  fi
  # a TCP connection between the two gateways must not exist (a carrier pair talks only through its carrier)
  tw 10 "no TCP connection between the gateways remains after the enrolment (no socket outside TIME-WAIT)" \
    '[ "$(nx G ss -tanH dst "${IPS[GP]}" 2>/dev/null | grep -vc TIME-WAIT)" = 0 ] && [ "$(nx GP ss -tanH dst "${IPS[G]}" 2>/dev/null | grep -vc TIME-WAIT)" = 0 ]'
  # the egress sniffers of both gateways: any frame addressed to the other gateway is off-link for them
  sniff_start sg G "${IPS[G]}" "${IPS[GP]}" || fail "the egress sniffer of G did not bind"
  sniff_start sgp GP "${IPS[GP]}" "${IPS[G]}" || fail "the egress sniffer of GP did not bind"
  # a counter on each gateway's OUTPUT hook for packets addressed to the other gateway: it counts a dial (a SYN) even when the
  # isolated bridge port leaves the ARP unanswered and no frame but an ARP request ever reaches the wire
  local ng nother
  for ng in G GP; do
    if [ "$ng" = G ]; then nother=${IPS[GP]}; else nother=${IPS[G]}; fi
    nx "$ng" nft add table inet g10 && nx "$ng" nft 'add chain inet g10 out { type filter hook output priority 0 ; }' &&
      nx "$ng" nft add rule inet g10 out ip daddr "$nother" counter || fail "$ng: cannot install the output counter for packets to the other gateway"
  done
  # their positive control: a datagram to the other gateway (a permanent neighbour entry: the bridge would drop the ARP)
  nx G ip neigh replace "${IPS[GP]}" lladdr 02:00:00:00:10:01 dev eth0 nud permanent
  nx G python3 -I -c 'import socket,sys
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.sendto(b"111-g3-control g10", (sys.argv[1], 9))' "${IPS[GP]}" 2>/dev/null
  nx G ip neigh del "${IPS[GP]}" dev eth0 2>/dev/null
  nx GP ip neigh replace "${IPS[G]}" lladdr 02:00:00:00:10:02 dev eth0 nud permanent
  nx GP python3 -I -c 'import socket,sys
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.sendto(b"111-g3-control g10", (sys.argv[1], 9))' "${IPS[G]}" 2>/dev/null
  nx GP ip neigh del "${IPS[G]}" dev eth0 2>/dev/null
  sleep 1
  OUT0_G=$(out_to_other G)
  OUT0_GP=$(out_to_other GP)
  t "the output counters see the control datagram (G ${OUT0_G}, G′ ${OUT0_GP}: each at least 1, so a later dial would be counted)" '[ "$OUT0_G" -ge 1 ] && [ "$OUT0_GP" -ge 1 ]'
  [ "${G10_STOP_AFTER:-}" = isolate ] && return

  # ================================================================== 4. the carrier on (consent), quiet
  local nl0
  patch G "$(jq -nc --arg s "$NTFY_URL" '{gateway: {enabled: true}, carrier: {kind: "ntfy", server: $s, mode: "5A"}}')"
  t "G: gateway on, carrier ntfy at $NTFY_URL, 5A (after the consent round trip: $CONSENTS asked)" '[ "$ST" = 200 ]'
  patch GP "$(jq -nc --arg s "$NTFY_URL" '{gateway: {enabled: true}, carrier: {kind: "ntfy", server: $s, mode: "5A"}}')"
  t "GP: the same (consent round trips so far: $CONSENTS)" '[ "$ST" = 200 ]'
  relay G
  note "G's /relay after switching on: carrier $(jget -c '.carrier'), gateway $(jget -c '.gateway')"
  [ "${G10_STOP_AFTER:-}" = on ] && return

  # ================================================================== item 1: 5A makes no request until an event
  echo "== item 1: 5A, no request until an event; the sequence on each trigger; nothing left open"
  local negp=""
  if [ "${G10_NEG:-}" = poll ]; then
    note "G10_NEG=poll: a stray poll from G's address reaches the stub in the quiet window, so item 1 must FAIL (the negative control)"
    (sleep $((QUIET / 2)); nx G curl -sS -o /dev/null "$NTFY_URL/g10negctl/json?poll=1") &
    negp=$!
  fi
  sleep "$QUIET"
  [ -n "$negp" ] && wait "$negp"
  if [ "$(nlines)" = 0 ]; then
    ok "item 1: both gateways on with a pair and nothing to send: no request reached the stub in $QUIET s (log empty), no connection ($(carrier_conns G) G, $(carrier_conns GP) GP), no 5B thread"
  else
    fail "item 1: the stub saw $(nlines) request(s) with nothing to send: $(reqs_after 0 | head -3 | tr '\n' '|')"
  fi
  [ "$(carrier_conns G)" = 0 ] && [ "$(carrier_conns GP)" = 0 ] || fail "item 1: a connection to the stub is open with nothing to send"
  local ch0
  ch0=$(children_of G)
  note "G's child processes while idle: [$ch0]"
  [ "${G10_STOP_AFTER:-}" = quiet ] && return
  items
}

# a wanted shape: a flush is a drain, a post, a drain
FLUSH_G="G:poll:BA G:post:AB G:poll:BA"
FLUSH_GP="GP:poll:AB GP:post:BA GP:poll:AB"

# expect_shape DESC N WANTED: ok when the requests after line N have exactly the wanted shape
expect_shape() {
  local got
  got=$(seq_after "$2")
  if [ "$got" = "$3" ]; then ok "$1: [$got]"; else fail "$1: got [$got], want [$3]"; dump_after "$2"; fi
}
# expect_prefix DESC N WANTED: the requests after line N START with the wanted shape (what the flush's own consequences
# add after it, a round the arrival of a frame at the far member sets off, is noted, not asserted)
expect_prefix() {
  local got
  got=$(seq_after "$2")
  if [ "$got" = "$3" ]; then
    ok "$1: [$got]"
  elif [[ "$got" == "$3 "* ]]; then
    ok "$1: starts [$3]"
    note "$1: then, from the flush's consequences: [${got#"$3 "}]"
  else
    fail "$1: got [$got], want [$3] first"
    dump_after "$2"
  fi
}
dump_after() { tail -n +"$(($1 + 1))" "$NLOG" | jq -c '[.ip, .m, (.p | .[0:60]), .s, .req]' 2>/dev/null | head -n 16 | sed 's/^/  log| /'; }
# idle_checks DESC NODE: nothing of the carrier is left open on NODE
idle_checks() {
  local c p th
  c=$(carrier_conns "$2")
  p=$(children_of "$2")
  th=$(threads_named "$2" 111d-ntfy-5b)
  if [ "$c" = 0 ] && [ "$th" = 0 ] && [ "$p" = "$CHILD0" ]; then
    ok "$1: $2 has no connection to the stub (ss: $c not in TIME-WAIT), no 111d-ntfy-5b thread ($th), the same child processes as before ([$p])"
  else
    fail "$1: $2 holds $c connection(s) to the stub, $th 5B thread(s), children [$p] (before: [$CHILD0])"
  fi
}
CHILD0=""

items() {
  local MARK nl0 nl1 m1 m2 m3 m4 m5 m6 seq i r0 st
  MARK=g10secret$RANDOM$RANDOM
  CHILD0=$(children_of G)
  NLOGMAP[main]=$NLOG
  FACTS=$W/facts
  selftest

  # ================================================================== item 1, trigger 1: a Forward event
  echo "== item 1: a Forward event (a message arrives at G for a member of G′)"
  nl0=$(nlines)
  send LG LGP "$MARK one"
  m1=$MID
  tw 30 "item 1: G posted its first batch" '[ "$(count_after "$nl0" G post)" -ge 1 ]'
  quiet_log
  TBA=$(reqs_after "$nl0" | awk '$1 == "G" && $6 == "poll" { print $3; exit }')
  TAB=$(reqs_after "$nl0" | awk '$1 == "G" && $6 == "post" { print $3; exit }')
  if [[ "$TAB" =~ ^111[a-z2-7]{32}$ ]] && [[ "$TBA" =~ ^111[a-z2-7]{32}$ ]] && [ "$TAB" != "$TBA" ]; then
    ok "item 1: two distinct topics of the shape 111 + 32 base32 characters (35): G posts to ${TAB:0:9}…, drains ${TBA:0:9}…"
  else
    fail "item 1: topics are not two distinct 35-character 111<base32> names (post '$TAB', poll '$TBA')"
  fi
  expect_shape "item 1 (Forward event): one drain, one post, one drain" "$nl0" "$FLUSH_G"
  idle_checks "item 1 after the Forward flush" G
  [ "$(count_after "$nl0" G held)" = 0 ] || fail "item 1: G opened a held stream in 5A"
  [ "$(reqs_after "$nl0" | grep -c '^GP ')" = 0 ] || fail "item 1: G′ made a request though nothing happened at it"

  # ---- item 2 (first half): the post is a mark, not a commit
  echo "== item 2: the row commits on the far ack only"
  t "item 2: G's post answered $(status_after 0 G post) and the row is still pending at G ($(pend G GP)), the sender LG reads $(mstate LG LGP "$m1")" \
    '[ "$(status_after 0 G post)" = 200 ] && [ "$(pend G GP)" = 1 ] && [ "$(mstate LG LGP "$m1")" = relayed/forwarding ]'

  # ---- item 1 (an ordinary flush): a route added in G's namespace with a posted, unacknowledged row: one drain, no post
  echo "== item 1/3: flushes of G while G′ is silent: each row is posted once"
  r0=$(reposts G GP)
  for i in 1 2 3; do
    nl1=$(nlines)
    nx G ip route add "198.18.$i.0/24" via "$L7_G10_IP_NTFY"
    wait_for 20 '[ "$(nlines)" -gt "$nl1" ]'
    quiet_log
    expect_shape "item 1 (a route added in G's netns, G′ silent, row posted and unacknowledged): one drain, no post" "$nl1" "G:poll:BA"
  done
  t "item 3: three flushes with G′ silent re-posted nothing (G's posts to the G→G′ topic: $(count_after 0 G post AB), reposts $(reposts G GP))" \
    '[ "$(count_after 0 G post AB)" = 1 ] && [ "$(reposts G GP)" = "$r0" ]'

  # ---- item 1, trigger 2: a route event with a row owed (a blackhole to the stub, a message, the blackhole removed)
  echo "== item 1: a route event makes drain, post, drain (the row was owed, the stub unreachable)"
  # a quiet window first: a posted, unacknowledged row, the stub REACHABLE, no event: no request at all
  nl0=$(nlines)
  sleep "$QUIET"
  t "item 1: a posted, unacknowledged row with the stub reachable and no event: no request in ${QUIET} s (the daemon has no retry timer)" '[ "$(nlines)" = "$nl0" ]'
  nx G ip route add blackhole "$L7_G10_IP_NTFY/32"
  sleep 4
  nl0=$(nlines)
  send LG LGP "$MARK two"
  m2=$MID
  tw 20 "item 2: the second row is owed at G (the posted first row and this one)" '[ "$(pend G GP)" = 2 ]'
  sleep "$SETTLE"
  t "item 1: a row owed with the stub unreachable (the harness's own blackhole: this line shows only that the daemon had no other path; the proof of no timer is the quiet window just above and the 6 s after the 429)" '[ "$(nlines)" = "$nl0" ]'
  nx G ip route del blackhole "$L7_G10_IP_NTFY/32"
  wait_for 20 '[ "$(nlines)" -gt "$nl0" ]'
  quiet_log
  expect_shape "item 1 (the route back: a netlink route event): one drain, one post, one drain" "$nl0" "$FLUSH_G"
  idle_checks "item 1 after the route flush" G
  t "item 1: that post carried the owed row only (G posted twice to the G→G′ topic: $(count_after 0 G post AB))" '[ "$(count_after 0 G post AB)" = 2 ]'

  # ---- item 1, trigger 3: Flush now re-posts what is posted and unacknowledged
  echo "== item 1/3: Flush now (a re-post, counted)"
  nl0=$(nlines)
  r0=$(reposts G GP)
  flush_now G
  t "item 1: POST /relay/carrier/flush answers 202" '[ "$ST" = 202 ]'
  wait_for 20 '[ "$(nlines)" -gt "$nl0" ]'
  quiet_log
  expect_prefix "item 1 (Flush now): one drain, one post, one drain" "$nl0" "$FLUSH_G"
  t "item 3: Flush now re-posted the two unacknowledged rows once and the pair's reposts counts it ($r0 -> $(reposts G GP))" '[ "$(reposts G GP)" -gt "$r0" ]'
  local sealed
  sealed=$(tp sealed "$NTFY_URL" "$TAB" "$MARK")
  t "item 3: the G→G′ topic holds 3 batches and 4 frames (1, 1, then both rows re-posted in one batch), one post per row per round: $sealed" \
    '[ "$(jq .batches <<<"$sealed")" = 3 ] && [ "$(jq .frames <<<"$sealed")" = 4 ]'
  idle_checks "item 1 after Flush now" G

  # ---- item 2: G′ flushes; the rows commit only after G drains the acks
  echo "== item 2: G′'s flush, G′'s ack on the reverse topic, G's commit"
  nl0=$(nlines)
  local ba0
  ba0=$(count_after 0 GP post BA)
  flush_now GP
  wait_for 20 '[ "$(count_after "$nl0" GP post BA)" -ge 1 ]'
  quiet_log
  expect_prefix "item 1 (G′, Flush now): one drain, one post (the acks), one drain" "$nl0" "$FLUSH_GP"
  t "item 2: G′ delivered both messages to LGP exactly once although it read each put twice (pkt_ref dedup): one=$(nrecv LGP LG "$MARK one"), two=$(nrecv LGP LG "$MARK two")" \
    '[ "$(nrecv LGP LG "$MARK one")" = 1 ] && [ "$(nrecv LGP LG "$MARK two")" = 1 ]'
  t "item 2: G′'s ack is on the reverse topic but G has not drained it: G still holds both rows (pending $(pend G GP)) and LG still reads $(mstate LG LGP "$m1")" \
    '[ "$(pend G GP)" = 2 ] && [ "$(mstate LG LGP "$m1")" = relayed/forwarding ]'
  nl0=$(nlines)
  check_now G
  tw 30 "item 2: G drained the ack: the rows commit" '[ "$(pend G GP)" = 0 ]'
  quiet_log
  t "item 2: G's first drain after the ack was one poll of the reverse topic ($(seq_after "$nl0"))" '[ "$(seq_after "$nl0" | cut -d" " -f1)" = "G:poll:BA" ]'
  nl0=$(nlines)
  tw 120 "item 2: the far member's ACK came back the same way: LG reads delivered" 'pump; [ "$(mstate LG LGP "$m1")" = delivered/- ] && [ "$(mstate LG LGP "$m2")" = delivered/- ]'
  t "item 2: the return went through the reverse topic (G′ posted $(count_after 0 GP post BA) times to it, $ba0 before) and the far member holds each message once" \
    '[ "$(count_after 0 GP post BA)" -gt "$ba0" ] && [ "$(nrecv LGP LG "$MARK one")" = 1 ] && [ "$(nrecv LGP LG "$MARK two")" = 1 ]'
  t "item 3: both hubs hold no forward row now (G $(pend G GP), G′ $(pend GP G))" '[ "$(pend G GP)" = 0 ] && [ "$(pend GP G)" = 0 ]'

  # ---- item 3: a taken ack is never posted again
  echo "== item 3: acks are posted once"
  nl0=$(nlines)
  flush_now GP
  quiet_log
  flush_now GP
  quiet_log
  flush_now G
  quiet_log
  nx GP ip route add 198.18.9.0/24 via "$L7_G10_IP_NTFY"
  sleep 4
  quiet_log
  t "item 3: with nothing owed, two Flush nows at G′, one at G and a route event at G′ posted nothing (posts: G $(count_after "$nl0" G post), G′ $(count_after "$nl0" GP post)): a taken ack is never posted again" \
    '[ "$(count_after "$nl0" G post)" = 0 ] && [ "$(count_after "$nl0" GP post)" = 0 ]'

  # ---- item 3: a dropped ack heals by a re-post and a DUPLICATE (the stub loses its cache with the ack in it)
  echo "== item 3: a stub restart loses G′'s ack: the cursor misses, G re-posts, G′ answers DUPLICATE"
  send LG LGP "$MARK three"
  m3=$MID
  tw 30 "item 3: G posted the third row" '[ "$(pend G GP)" = 1 ]'
  quiet_log
  flush_now GP
  quiet_log
  t "item 3: G′ read it, delivered it once and posted its ack, which G has not read: pending at G is $(pend G GP), LGP holds it $(nrecv LGP LG "$MARK three") time(s)" \
    '[ "$(pend G GP)" = 1 ] && [ "$(nrecv LGP LG "$MARK three")" = 1 ]'
  r0=$(reposts G GP)
  stub_restart
  nl0=$(nlines)
  check_now G
  tw 30 "item 3: after the restart G read the reverse topic, missed, and re-posted" '[ "$(count_after "$nl0" G post AB)" -ge 1 ]'
  quiet_log
  expect_prefix "item 3 (a since miss at G): a 400 poll, the since=all re-read, one re-post, one drain" "$nl0" "G:poll:BA G:poll:BA G:post:AB G:poll:BA"
  t "item 3: the first of those polls answered 400 (the stub no longer knows the id): $(status_after "$nl0" G poll)" '[ "$(status_after "$nl0" G poll | cut -d" " -f1)" = 400 ]'
  t "item 3: the re-post is counted ($r0 -> $(reposts G GP)) and the row is still pending ($(pend G GP)): the ack was lost" '[ "$(reposts G GP)" -gt "$r0" ] && [ "$(pend G GP)" = 1 ]'
  tw 120 "item 3: G′ answered the re-post (a DUPLICATE, a fresh ack): the row commits and the far member holds the message once" \
    'pump; [ "$(pend G GP)" = 0 ] && [ "$(mstate LG LGP "$m3")" = delivered/- ] && [ "$(nrecv LGP LG "$MARK three")" = 1 ]'
  [ "${G10_STOP_AFTER:-}" = fivea ] && return

  # ================================================================== item 3: 5B
  echo "== item 3: 5B holds one connection on the thread 111d-ntfy-5b"
  local rss0 rss1 rss2 th0
  rss0=$(rss_kb G)
  nl0=$(nlines)
  patch G '{"carrier": {"mode": "5B"}}'
  t "item 3: G switched to 5B (HTTP $ST)" '[ "$ST" = 200 ]'
  tw 20 "item 3: one held connection to the stub and one thread 111d-ntfy-5b" '[ "$(carrier_conns G)" = 1 ] && [ "$(threads_named G 111d-ntfy-5b)" = 1 ]'
  sleep 2
  rss1=$(rss_kb G)
  t "item 3: the held stream is one GET of the reverse topic without poll=1: $(seq_after "$nl0")" '[ "$(count_after "$nl0" G held BA)" = 1 ] && [ "$(count_after "$nl0" G poll)" = 0 ] && [ "$(count_after "$nl0" G post)" = 0 ]'
  fact "rss_5a=$rss0 kB"
  fact "rss_5b=$rss1 kB"
  note "RSS record, G's daemon (debug build): $rss0 kB in 5A, $rss1 kB with the 5B stream held (+$((rss1 - rss0)) kB); threads: 111d-ntfy-5b $(threads_named G 111d-ntfy-5b); held connections $(carrier_conns G)"
  nl0=$(nlines)
  send LG LGP "$MARK four"
  m4=$MID
  tw 30 "item 3 (5B): one post per Forward event" '[ "$(count_after "$nl0" G post AB)" = 1 ]'
  sleep "$SETTLE"
  t "item 3 (5B): that was G's only request (no drain, no second post: $(seq_after "$nl0")) and the connection count stays $(carrier_conns G)" '[ "$(seq_after "$nl0")" = "G:post:AB" ] && [ "$(carrier_conns G)" = 1 ]'
  nl1=$(nlines)
  flush_now GP
  quiet_log
  tw 30 "item 3 (5B): G′'s ack reached G over the held stream: the row commits with no request from G" \
    '[ "$(pend G GP)" = 0 ] && [ "$(count_after "$nl1" G poll)" = 0 ] && [ "$(count_after "$nl1" G held)" = 0 ]'
  # a stub restart with an unacknowledged row: one re-post (a since miss at the reconnect) and one delivery
  send LG LGP "$MARK five"
  m5=$MID
  tw 30 "item 3 (5B): the fifth row is posted and unacknowledged" '[ "$(pend G GP)" = 1 ] && [ "$(count_after 0 G post AB)" -ge 6 ]'
  nl0=$(nlines)
  r0=$(reposts G GP)
  th0=$(count_after 0 G post AB)
  stub_restart
  tw 60 "item 3 (5B): the stream reconnected (one connection, one thread again) and G re-posted once" \
    '[ "$(count_after "$nl0" G post AB)" = 1 ] && [ "$(carrier_conns G)" = 1 ] && [ "$(threads_named G 111d-ntfy-5b)" = 1 ]'
  sleep "$SETTLE"
  t "item 3 (5B): exactly one re-post after the restart: posts to the G→G′ topic ${th0} -> $(count_after 0 G post AB), reposts $(reposts G GP)" '[ "$(count_after "$nl0" G post AB)" = 1 ] && [ "$(reposts G GP)" -gt "$r0" ]'
  flush_now GP
  quiet_log
  tw 60 "item 3 (5B): one delivery: LGP holds the fifth message and G's row commits" '[ "$(nrecv LGP LG "$MARK five")" = 1 ] && [ "$(pend G GP)" = 0 ]'
  tw 60 "item 3 (5B): the far ACKs return (LG reads delivered for the fourth and fifth)" 'flush_now GP; quiet_log; [ "$(mstate LG LGP "$m4")" = delivered/- ] && [ "$(mstate LG LGP "$m5")" = delivered/- ]'
  rss2=$(hwm_kb G)
  fact "hwm_5b=$rss2 kB"
  note "5B record: G's VmHWM $rss2 kB after the 5B phase (held stream, a restart, a re-post); one stream and one thread throughout"
  local t5a
  t5a=$SECONDS
  patch G '{"carrier": {"mode": "5A"}}'
  use_log main
  tw 40 "item 3: back in 5A the stream is torn down: no connection, no 111d-ntfy-5b thread" '[ "$(carrier_conns G)" = 0 ] && [ "$(threads_named G 111d-ntfy-5b)" = 0 ]'
  note "item 3 record: the 5B thread ended $((SECONDS - t5a)) s after the switch to 5A (it notices at the next read: the stub's keepalive here is 5 s; a default server's is 45 s, so up to that)"
  fact "teardown_5b=$((SECONDS - t5a)) s after the switch (stub keepalive 5 s)"
  [ "${G10_STOP_AFTER:-}" = fiveb ] && return
  items_more "$MARK"
  [ -n "${G10_STOP_AFTER:-}" ] || true
  finish_run
  write_log
}

# ---- the rest of the items (called by items after item 3)
items_more() {
  local MARK=$1 nl0 nl1 st i r0 m body frame k edge sealed
  # ================================================================== item 4: sizes
  echo "== item 4: the body limit, the item limit, 413 and 429"
  use_log main
  local text1500
  text1500=$(head -c 1500 /dev/zero | tr '\0' 'x')
  nl0=$(nlines)
  send LG LGP "$text1500"
  tw 30 "item 4: a 1,500 B text is posted by G" '[ "$(count_after "$nl0" G post AB)" -ge 1 ]'
  quiet_log
  body=$(reqs_after "$nl0" | awk '$1 == "G" && $6 == "post" { print $5; exit }')
  # one frame in the batch: raw = 1 + 1 + 2 + frame, base64url unpadded
  frame=$(python3 -I -c 'import sys; b = int(sys.argv[1]); print(b * 3 // 4 - 4)' "$body")
  k=$((frame - 1500))
  edge=$((3068 - k))
  fact "frame_1500=$frame B"
  fact "body_1500=$body B (ratio $(python3 -I -c 'import sys; print(round(int(sys.argv[1]) / int(sys.argv[2]), 3))' "$body" "$frame"))"
  fact "overhead=$k B"
  fact "edge_text=$edge B"
  note "item 4 record: a 1,500 B text is a $body B ntfy body: one frame of $frame B (the text + $k B of item header, wrap and hint); base64url ratio $(python3 -I -c 'import sys; print(round(int(sys.argv[1]) / int(sys.argv[2]), 3))' "$body" "$frame"); the largest text that fits max_frame 3,068 B is $edge B"
  pump >/dev/null 2>&1 || true
  local textedge text_over
  textedge=$(head -c "$edge" /dev/zero | tr '\0' 'y')
  text_over=$(head -c "$((edge + 1))" /dev/zero | tr '\0' 'z')
  nl0=$(nlines)
  send LG LGP "$textedge"
  tw 30 "item 4: a text sized to the largest frame is posted" '[ "$(count_after "$nl0" G post AB)" -ge 1 ]'
  quiet_log
  body=$(reqs_after "$nl0" | awk '$1 == "G" && $6 == "post" { print $5; exit }')
  fact "edge_body=$body B"
  t "item 4: the largest item the gate lets through (a frame of 3,068 B) is posted as a body of exactly 4,096 B (got $body)" '[ "$body" = 4096 ]'
  pump >/dev/null 2>&1 || true
  local big0 big1
  relay G
  big0=$(jnum .forward.too_large)
  nl0=$(nlines)
  send LG LGP "$text_over"
  tw 30 "item 4: one byte more is refused at the gate (forward.too_large counts it)" 'relay G; [ "$(jnum .forward.too_large)" = "$((big0 + 1))" ] && [ "$big0" -ge 0 ]'
  sleep "$SETTLE"
  t "item 4: the oversized item was never posted (G posted $(count_after "$nl0" G post) time(s) after it)" '[ "$(count_after "$nl0" G post)" = 0 ]'
  t "item 4: no body above 4,096 B reached the stub in the whole run (largest: $(max_post_body) B), and no 413 was answered" '[ "$(max_post_body)" -le 4096 ] && ! grep -q "\"s\": 413" "$NLOG"'
  # two large rows owed together: the packer must split them (3,072 raw bytes per body), so the 4,096 B bound is exercised by a split, not by the item limit alone
  pump >/dev/null 2>&1 || true
  local ta tb
  ta=$(head -c 1500 /dev/zero | tr '\0' 'a')
  tb=$(head -c 1500 /dev/zero | tr '\0' 'b')
  nx G ip route add blackhole "$L7_G10_IP_NTFY/32"
  sleep 3
  nl0=$(nlines)
  send LG LGP "$ta"
  send LG LGP "$tb"
  tw 20 "item 4: two 1,500 B rows are owed at G (pending $(pend G GP)) with the stub unreachable" '[ "$(pend G GP)" = 2 ]'
  nx G ip route del blackhole "$L7_G10_IP_NTFY/32"
  tw 30 "item 4: released, G posts the two owed rows in TWO bodies, each at or under 4,096 B" \
    '[ "$(count_after "$nl0" G post AB)" = 2 ] && [ "$(reqs_after "$nl0" | awk '"'"'$1 == "G" && $6 == "post" && $5 > 4096'"'"' | grep -c .)" = 0 ]'
  quiet_log
  t "item 4: both posts were answered 200 and the packer made no 413 (statuses: $(status_after "$nl0" G post))" '[ "$(status_after "$nl0" G post | wc -w)" = 2 ] && [ "$(status_after "$nl0" G post)" = "200 200" ]'
  note "item 4: the two bodies were $(reqs_after "$nl0" | awk '$1 == "G" && $6 == "post" { printf "%s B ", $5 }')"
  tw 120 "item 4: and both rows were delivered once" 'pump; [ "$(nrecv LGP LG "$ta")" = 1 ] && [ "$(nrecv LGP LG "$tb")" = 1 ]'

  # ---- item 3, re-post trigger (1): a far frame returns with no ack for a posted row (the far hub never read it: the stub lost it)
  echo "== item 3: a far hub's frame without an ack re-arms the posted row, once"
  local rp0
  nl0=$(nlines)
  send LG LGP "$MARK rp1"
  tw 30 "item 3 re-post (1): G posted the row ($(pend G GP) pending, GP has not read it)" '[ "$(count_after "$nl0" G post AB)" = 1 ] && [ "$(pend G GP)" = 1 ]'
  stub_restart
  sleep 1
  send LGP LG "$MARK rp2"
  tw 30 "item 3 re-post (1): G′ answered the new message by posting on the reverse topic (the stub forgot the first row)" '[ "$(count_after "$nl0" GP post BA)" -ge 1 ]'
  quiet_log
  rp0=$(reposts G GP)
  local ev=0
  # the drain that reads G′'s frame marks the row (no ack in it); the row is re-armed and re-posted by the flush that follows
  while [ "$ev" -lt 3 ] && [ "$(reposts G GP)" = "$rp0" ]; do
    ev=$((ev + 1))
    nl1=$(nlines)
    nx G ip route add "198.18.4$ev.0/24" via "$L7_G10_IP_NTFY"
    wait_for 20 '[ "$(nlines)" -gt "$nl1" ]'
    quiet_log
    sleep 2
  done
  t "item 3 re-post (1): once G′'s frame (no ack for the row) was drained, the row was re-posted exactly once, counted (reposts $rp0 -> $(reposts G GP) after $ev G-only event(s))" '[ "$(reposts G GP)" = "$((rp0 + 1))" ]'
  for i in 1 2; do
    nl1=$(nlines)
    nx G ip route add "198.18.5$i.0/24" via "$L7_G10_IP_NTFY"
    wait_for 20 '[ "$(nlines)" -gt "$nl1" ]'
    quiet_log
    sleep 2
  done
  t "item 3 re-post (1): further G-only flushes with G′ silent re-post nothing (reposts still $(reposts G GP))" '[ "$(reposts G GP)" = "$((rp0 + 1))" ]'
  tw 120 "item 3 re-post (1): both messages arrive once" 'pump; [ "$(nrecv LGP LG "$MARK rp1")" = 1 ] && [ "$(nrecv LG LGP "$MARK rp2")" = 1 ]'

  # ---- item 5: a third client (before the stubs are replaced)
  echo "== item 5: a third client on the topics"
  local nidhex_lg nidhex_lgp
  sealed=$(tp sealed "$NTFY_URL" "$TAB" "$MARK" "g10secret" "hex:${NID[LG]}" "hex:${NID[LGP]}" "hex:${NID[G]}" "hex:${NID[GP]}" "${NID[LG]}" "${NID[LGP]}")
  local sealed2
  sealed2=$(tp sealed "$NTFY_URL" "$TBA" "$MARK" "g10secret" "hex:${NID[LG]}" "hex:${NID[LGP]}" "hex:${NID[G]}" "hex:${NID[GP]}" "${NID[LG]}" "${NID[LGP]}")
  t "item 5: a third client polling the G→G′ topic reads $(jq .batches <<<"$sealed") batches of $(jq .frames <<<"$sealed") base64url frames (every one decodes) and finds neither a message text, a marker nor a NodeId in them: found=$(jq -c .found <<<"$sealed")" \
    '[ "$(jq .batches <<<"$sealed")" -ge 1 ] && [ "$(jq .undecodable <<<"$sealed")" = 0 ] && [ "$(jq .found <<<"$sealed")" = "[]" ]'
  t "item 5: and the reverse topic: $(jq .batches <<<"$sealed2") batches, $(jq .frames <<<"$sealed2") frames, found=$(jq -c .found <<<"$sealed2")" \
    '[ "$(jq .batches <<<"$sealed2")" -ge 1 ] && [ "$(jq .undecodable <<<"$sealed2")" = 0 ] && [ "$(jq .found <<<"$sealed2")" = "[]" ]'
  # the control: the same scan finds a marker and a NodeId in a message posted in clear
  nx NTFY curl -sS -o /dev/null -X POST --data "g10secretCTL ${NID[LG]}" "$NTFY_URL/g10control"
  local ctl
  ctl=$(tp sealed "$NTFY_URL" g10control "g10secret" "${NID[LG]}")
  t "item 5 control: the same reader finds the marker and the NodeId in a message posted in clear: $(jq -c .found <<<"$ctl")" '[ "$(jq ".found | length" <<<"$ctl")" = 2 ]'
  t "item 5: the daemons used the two topics only (G and G′ asked for: $(topics_used | tr '\n' ' '))" '[ "$(topics_used | wc -l)" = 2 ]'
  local wrong
  wrong=$(tp poll "$NTFY_URL" "111$(printf 'a%.0s' $(seq 1 32))")
  t "item 5: a topic of the right shape that no hub uses yields nothing ($wrong)" '[ "$(jq .n <<<"$wrong")" = 0 ] && [ "$(jq .status <<<"$wrong")" = 200 ]'
  local d0 fst
  d0=$(fabric_counter G dropped)
  fst=$(tp post "$NTFY_URL" "$TBA")
  t "item 5: the third client posts a batch of one frame no hub sealed to the topic G reads: HTTP $fst" '[ "$fst" = 200 ]'
  check_now G
  tw 20 "item 5: G dropped and counted that frame, once (fabric counters.dropped $d0 +1)" '[ "$(fabric_counter G dropped)" = "$((d0 + 1))" ]'
  quiet_log
  check_now G
  quiet_log
  t "item 5: the drain cursor moved past it: a second check reads nothing new and the counter stays ($(fabric_counter G dropped))" '[ "$(fabric_counter G dropped)" = "$((d0 + 1))" ]'
  [ "${G10_STOP_AFTER:-}" = third ] && return

  # ================================================================== item 4: 413, on a stub whose message limit is 64 B
  echo "== item 4: a 413 returns the row, a 429 keeps it"
  stub_start small "$((NP + 1))" --message-limit 64 --burst 5000 --replenish 0.01 || return
  patch G "$(jq -nc --arg s "http://$L7_G10_IP_NTFY:$((NP + 1))" '{carrier: {server: $s}}')"
  t "item 4: G now points at a server with a 64 B message limit (consent asked again for the new server: HTTP $ST)" '[ "$ST" = 200 ]'
  use_log small
  quiet_log
  relay G
  local ref0
  ref0=$(jnum .forward.refused)
  nl0=$(nlines)
  send LG LGP "$MARK 413"
  tw 30 "item 4: G's post was answered 413" '[ "$(status_after "$nl0" G post)" = 413 ]'
  quiet_log
  tw 20 "item 4: a 413 is Rejected: with no other target the row is refused and counted (forward.refused +1, nothing pending)" 'relay G; [ "$(jnum .forward.refused)" = "$((ref0 + 1))" ] && [ "$ref0" -ge 0 ] && [ "$(pend G GP)" = 0 ]'
  t "item 4: and G posted once, did not retry (posts: $(count_after "$nl0" G post))" '[ "$(count_after "$nl0" G post)" = 1 ]'
  # ---- 429: one token left when the post is made
  stub_start lim "$((NP + 2))" --burst 3 --replenish 3600 || return
  patch G "$(jq -nc --arg s "http://$L7_G10_IP_NTFY:$((NP + 2))" '{carrier: {server: $s}}')"
  t "item 4: G now points at a server that allows three requests (HTTP $ST)" '[ "$ST" = 200 ]'
  use_log lim
  quiet_log
  local used
  used=$(reqs_after 0 | awk '$1 == "G" && $6 != "health"' | grep -c .)
  for i in $(seq 1 $((2 - used))); do
    nx G curl -sS -o /dev/null "http://$L7_G10_IP_NTFY:$((NP + 2))/primezz$i/json?poll=1"
  done
  nl0=$(nlines)
  send LG LGP "$MARK 429"
  tw 30 "item 4: G's post was answered 429" '[ "$(status_after "$nl0" G post)" = 429 ]'
  nl1=$(nlines)
  sleep "$((SETTLE + 3))"
  t "item 4: a 429 is Retry: the row stays pending ($(pend G GP)) and the daemon made no retry of its own in $((SETTLE + 3)) s (no timer: $(($(nlines) - nl1)) request(s))" '[ "$(pend G GP)" = 1 ] && [ "$(nlines)" = "$nl1" ]'
  relay G
  t "item 4: nothing was refused by the 429 (forward.refused unchanged: $(jget .forward.refused))" '[ "$(jnum .forward.refused)" = "$((ref0 + 1))" ]'
  # back to the main stub: the retained row goes out on the next event
  patch G "$(jq -nc --arg s "$NTFY_URL" '{carrier: {server: $s}}')"
  t "item 4: G points at the main stub again (HTTP $ST)" '[ "$ST" = 200 ]'
  use_log main
  nl0=$(nlines)
  nx G ip route add 198.18.20.0/24 via "$L7_G10_IP_NTFY"
  tw 30 "item 4: after the 429 the row went out on the next event" '[ "$(count_after "$nl0" G post AB)" -ge 1 ]'
  quiet_log
  tw 120 "item 4: and was delivered once (LGP holds it)" 'pump; [ "$(nrecv LGP LG "$MARK 429")" = 1 ]'
  [ "${G10_STOP_AFTER:-}" = limits ] && return

  # ================================================================== item 6: the metered cap, the switch
  echo "== item 6: the metered cap and the switch"
  patch G '{"metered": [{"match": {"iface": "eth0"}, "daily_cap_bytes": 1}]}'
  t "item 6: a metered rule on eth0 (the default route's interface) with a 1 B cap (consent round trip, HTTP $ST)" '[ "$ST" = 200 ]'
  sleep 1
  # the window counts carrier bytes from the rule on: the first flush is allowed and reaches the 1 B cap, the next is refused
  flush_now G
  t "item 6: the first flush after the rule is accepted (202: no bytes counted yet)" '[ "$ST" = 202 ]'
  quiet_log
  nl0=$(nlines)
  flush_now G
  t "item 6: past the cap Flush now answers 409 metered_cap" '[ "$ST" = 409 ] && [ "$(jget .code)" = metered_cap ]'
  check_now G
  t "item 6: and Check now too" '[ "$ST" = 409 ] && [ "$(jget .code)" = metered_cap ]'
  relay G
  t "item 6: /relay says the cap is reached (gateway.metered_cap_reached)" 'jb ".gateway.metered_cap_reached == true"'
  send LG LGP "$MARK metered"
  nx G ip route add 198.18.21.0/24 via "$L7_G10_IP_NTFY"
  sleep "$((SETTLE + 3))"
  t "item 6: a message and a route event past the cap made no request from G ($(count_after "$nl0" G post) posts, $(reqs_after "$nl0" | grep -c '^G ') requests) and the row waits (pending $(pend G GP))" \
    '[ "$(reqs_after "$nl0" | grep -c "^G ")" = 0 ] && [ "$(pend G GP)" = 1 ]'
  patch G '{"metered": [{"match": {"iface": "eth0"}, "daily_cap_bytes": 100000000}]}'
  t "item 6: the cap raised to 100 MB (consent asked again: HTTP $ST)" '[ "$ST" = 200 ]'
  nl0=$(nlines)
  flush_now G
  t "item 6: Flush now is accepted again (202)" '[ "$ST" = 202 ]'
  tw 30 "item 6: and the waiting row is posted" '[ "$(count_after "$nl0" G post AB)" -ge 1 ]'
  tw 120 "item 6: and delivered" 'pump; [ "$(nrecv LGP LG "$MARK metered")" = 1 ]'
  note "item 6: the carrier's own licensed_board 409 has no source on the wire in v1 (RuntimeOptions::production licensed = || false; N/carrier/mod.rs:308-327): that refusal is SKIPPED here and covered in-process by tests/carrier_flow.rs; the wire-level I12 claim (no Layer 7 path sends to a licensed board) is checked at the end with a bridge stub"
  # switch off: the same strace sees the connect() while on (control) and none while off
  strace_start G 40
  nx G ip route add 198.18.22.0/24 via "$L7_G10_IP_NTFY"
  sleep 5
  strace_wait
  local on_conn
  on_conn=$(strace_to_stub G)
  t "item 6 control: with the switch on, a route event makes G connect() to the stub (strace saw $on_conn)" '[ "$on_conn" -ge 1 ]'
  patch G '{"gateway": {"enabled": false}}'
  t "item 6: gateway.enabled false (HTTP $ST)" '[ "$ST" = 200 ]'
  sleep 2
  nl0=$(nlines)
  strace_start G 30
  nx G ip route add 198.18.23.0/24 via "$L7_G10_IP_NTFY"
  flush_now G
  nx G ip route del 198.18.23.0/24 2>/dev/null
  send LG LGP "$MARK off"
  sleep 6
  strace_wait
  t "item 6: with the switch off the same strace saw no connect() to the stub across a route event, a route change back, Flush now ($ST) and a message ($(strace_to_stub G) connects), and the stub saw nothing from G ($(reqs_after "$nl0" | grep -c '^G '))" \
    '[ "$(strace_to_stub G)" = 0 ] && [ "$(reqs_after "$nl0" | grep -c "^G ")" = 0 ]'
  t "item 6: and nothing of the carrier is open at G (connections $(carrier_conns G), 5B threads $(threads_named G 111d-ntfy-5b))" '[ "$(carrier_conns G)" = 0 ] && [ "$(threads_named G 111d-ntfy-5b)" = 0 ]'
  licensed_phase "$MARK"
  return 0
}

# ---- item 6, the licensed board (I12): G carries the Layer 6 bridge stubs, one of them a licensed board; no Layer 7 path may send to it
licensed_phase() {
  local MARK=$1 ctld seq=0 f end fcm fcg sm0 sa0
  echo "== item 6: a licensed board attached to G"
  ctld=${ROOT[G]}/g10-ctl
  mkdir -p "$ctld"
  cat >"${ROOT[G]}/g10-scenario.json" <<EOF
{"control": "$ctld", "node": "g",
 "stubs": [
  {"id":"stub-mesh","label":"Licensed mesh board","tag":"SM","kinds":["channel","dm"],"max_text_bytes":{"channel":232,"dm":220,"group":null},"airtime":{"duty_cycle_permille":10,"bits_per_s":1000,"preset":"LongFast"},
   "facts":{"accounts":[{"id":"!a0000001","label":"licensed board","licensed":true}],"channels":[{"account":"!a0000001","native_id":"ch:up","uplink":false,"default_key":false}],"region_set":true,"ids_globally_unique":false,"parts_may_reorder":true,"licensed_caps":{"channel":232,"dm":232,"group":null}}},
  {"id":"stub-app","label":"App stub","tag":"SA","kinds":["dm","group"],"max_text_bytes":{"channel":null,"dm":4000,"group":120},
   "facts":{"accounts":[{"id":"app-a","label":"bridge account","licensed":false}],"channels":[],"region_set":true,"ids_globally_unique":true,"parts_may_reorder":false,"licensed_caps":null}}]}
EOF
  stop_node G
  start_node G NODE111_BRIDGE_STUBS="${ROOT[G]}/g10-scenario.json" || return 0
  tw 20 "G restarted with the bridge scenario hook: stub-mesh (a licensed board) and stub-app attached" '[ -s "$ctld/ready.jsonl" ]'
  [ -s "$ctld/ready.jsonl" ] || return 0
  gctl() { # JSON -> 0 when the hook answered ok
    seq=$((seq + 1))
    f=$(printf '%s/%08d' "$ctld" "$seq")
    printf '%s' "$1" >"$f.tmp" && mv "$f.tmp" "$f.json"
    end=$((SECONDS + 15))
    while [ ! -s "$f.done" ]; do
      [ "$SECONDS" -ge "$end" ] && return 1
      sleep 0.05
    done
    [ "$(jq -r .ok "$f.done" 2>/dev/null)" = true ]
  }
  sent_n() { if [ -s "$ctld/$1.sent.jsonl" ]; then wc -l <"$ctld/$1.sent.jsonl" | tr -d ' '; else echo 0; fi; }
  gcid() { # NET NATIVE -> the conversation id on G
    local i id
    for i in $(seq 1 50); do
      api G GET "/api/v1/networks/$1/conversations"
      id=$(jget --arg n "$2" '[.conversations[]? | select(.native_id == $n)] | first | .id // empty')
      [ -n "$id" ] && { printf '%s' "$id"; return 0; }
      sleep 0.2
    done
  }
  cvj() { jq -nc --arg net "$1" --arg acc "$2" --arg kind "$3" --arg nat "$4" --arg t "$5" '{Conversation: {conv: {network: $net, account: $acc, kind: $kind, native_id: $nat}, title: $t}}'; }
  mesh_facts() { printf '{"accounts":[{"id":"!a0000001","label":"licensed board","licensed":%s}],"channels":[{"account":"!a0000001","native_id":"ch:up","uplink":false,"default_key":false}],"region_set":true,"ids_globally_unique":false,"parts_may_reorder":true,"licensed_caps":{"channel":232,"dm":232,"group":null}}' "$1"; }
  gctl "$(jq -nc --argjson i "[$(cvj stub-app app-a group grp:g10 'g10 control')]" '{op: "feed", stub: "stub-app", items: $i}')" &&
    gctl "$(jq -nc --argjson i "[$(cvj stub-mesh '!a0000001' channel ch:up 'g10 uplink')]" '{op: "feed", stub: "stub-mesh", items: $i}')" || fail "item 6: the scenario hook did not take the feeds"
  fcg=$(gcid stub-app grp:g10)
  fcm=$(gcid stub-mesh ch:up)
  api G POST "/api/v1/networks/stub-app/conversations/${fcg:-none}/messages" '{"text": "g10 control app", "size_mode": "refuse"}'
  tw 15 "item 6 positive control 1: a Layer 6 send to the unlicensed stub-app reaches that stub's radio log (it reads 1)" '[ "$(sent_n stub-app)" = 1 ]'
  gctl "$(jq -nc --argjson f "$(mesh_facts false)" '{op: "facts", stub: "stub-mesh", facts: $f}')" || fail "item 6: the hook did not take the unlicensed facts"
  api G POST "/api/v1/networks/stub-mesh/conversations/${fcm:-none}/messages" '{"text": "g10 control mesh", "size_mode": "refuse"}'
  tw 15 "item 6 positive control 2: with stub-mesh reporting unlicensed a Layer 6 send reaches its log (it reads 1)" '[ "$(sent_n stub-mesh)" = 1 ]'
  gctl "$(jq -nc --argjson f "$(mesh_facts true)" '{op: "facts", stub: "stub-mesh", facts: $f}')" || fail "item 6: the hook did not take the licensed facts back"
  sm0=$(sent_n stub-mesh)
  sa0=$(sent_n stub-app)
  patch G '{"gateway": {"enabled": true}, "carrier": {"kind": "ntfy", "mode": "5A"}}'
  t "item 6: G (the licensed board attached) takes the gateway and carrier settings (HTTP $ST)" '[ "$ST" = 200 ]'
  local nl0
  nl0=$(nlines)
  send LG LGP "$MARK licensed"
  tw 60 "item 6: a forward through G over the carrier works with the licensed board attached (the stub saw a G post to the G→G′ topic)" '[ "$(count_after "$nl0" G post AB)" -ge 1 ]'
  flush_now G
  check_now G
  tw 120 "item 6: and the message is delivered once at LGP" 'pump; [ "$(nrecv LGP LG "$MARK licensed")" = 1 ]'
  sleep 3
  t "item 6: no Layer 7 path sent to the licensed board: stub-mesh's radio log $sm0 -> $(sent_n stub-mesh), stub-app's $sa0 -> $(sent_n stub-app) (the carrier rides IP; the positive controls above show the log does record a send when one is made)" \
    '[ "$(sent_n stub-mesh)" = "$sm0" ] && [ "$(sent_n stub-app)" = "$sa0" ]'
}

# ---- the log of item 7 and the end-of-run checks
out_to_other() { nx "$1" nft list chain inet g10 out 2>/dev/null | grep -o 'packets [0-9]*' | head -1 | cut -d' ' -f2 | grep . || echo -1; }
finish_run() {
  local r rgp
  t "no packet of either gateway was addressed to the other since the control datagram (nft OUTPUT counters: G ${OUT0_G} -> $(out_to_other G), G′ ${OUT0_GP} -> $(out_to_other GP)): a redial would show here even though the isolated port lets only an ARP out; no SYN-SENT socket to the other gateway (G $(nx G ss -tanH dst "${IPS[GP]}" 2>/dev/null | grep -c SYN-SENT), G′ $(nx GP ss -tanH dst "${IPS[G]}" 2>/dev/null | grep -c SYN-SENT))" \
    '[ "$(out_to_other G)" = "$OUT0_G" ] && [ "$(out_to_other GP)" = "$OUT0_GP" ] && [ "$(nx G ss -tanH dst "${IPS[GP]}" 2>/dev/null | grep -c SYN-SENT)" = 0 ] && [ "$(nx GP ss -tanH dst "${IPS[G]}" 2>/dev/null | grep -c SYN-SENT)" = 0 ]'
  sniff_halt sgp
  sniff_halt sg
  rgp=$(sniff_counts sgp)
  r=$(sniff_counts sg)
  t "no frame addressed to the other gateway left either gateway's eth0 in the whole run (G: off-link out ${r%% *}, G′: ${rgp%% *}; the control datagram was seen: G ${r#* }, G′ ${rgp#* })" \
    '[ "${r%% *}" = 0 ] && [ "${rgp%% *}" = 0 ] && [ "$(cut -d" " -f2 <<<"$r")" -ge 1 ] && [ "$(cut -d" " -f2 <<<"$rgp")" -ge 1 ]'
}

# ---- item 7: the smoke log of this run (a stub run: logs/ntfy-smoke-<calver>-stub.md, P1's sections kept)
FACTS=""
fact() { printf '%s\n' "$1" >>"$FACTS"; }
write_log() {
  local calver log stamp
  calver=${NODE111_BUILD_VERSION:-$(date -u +%Y.%m.%d)}
  log=$L7_LOGS/ntfy-smoke-$calver-stub.md
  stamp=$(date -u +%Y-%m-%dT%H:%MZ)
  mkdir -p "$L7_LOGS"
  use_log main
  local tbl
  tbl=$(jq -r --arg g "$L7_G10_IP_G" --arg gp "$L7_G10_IP_GP" --arg ab "$TAB" --arg ba "$TBA" '
    ((if .ip == $g then "G" elif .ip == $gp then "G′" else .ip end)) as $who
    | (.p | ltrimstr("/") | split("?")[0] | split("/")[0]) as $t
    | ((if $t == $ab then "G→G′" elif $t == $ba then "G′→G" else $t end)) as $topic
    | (if .m == "POST" then "post" elif (.p | contains("poll=1")) then "drain" else "held" end) as $kind
    | "| \($who) | \(.m) | \($kind) | \($topic) | \(.s) | \(.req) | \(.resp // "stream") |"' "${NLOGMAP[main]}" 2>/dev/null | head -n 24)
  fact "posts_total=$(jq -s '[.[] | select(.m == "POST" and (.ip == "'"$L7_G10_IP_G"'" or .ip == "'"$L7_G10_IP_GP"'"))] | length' "${NLOGMAP[main]}")"
  fact "max_body=$(jq -s '[.[] | select(.m == "POST" and (.ip == "'"$L7_G10_IP_G"'" or .ip == "'"$L7_G10_IP_GP"'")) | .req] | max // 0' "${NLOGMAP[main]}") B"
  local base=""
  if [ ! -s "$log" ]; then
    # no P1 log of this day: start from the newest earlier one so its §6.4 sections are kept (stamped as carried)
    local cand
    for cand in "$L7_LOGS"/ntfy-smoke-*-stub.md; do
      [ -f "$cand" ] && [ "$cand" != "$log" ] && base=$cand
    done
    [ -n "$base" ] && cp "$base" "$log"
  fi
  if [ "$FAILS" != 0 ]; then
    note "this run has $FAILS FAIL line(s): the smoke log in the repo is left as it was"
    return 0
  fi
  python3 -I - "$log" "$stamp" "$FACTS" "$tbl" "$(basename "$BIN")" "${base:+$(basename "$base")}" <<'PY'
import re, sys
path, stamp, facts_path, table, binname, carried = sys.argv[1:7]
facts = dict(l.split("=", 1) for l in open(facts_path).read().splitlines() if "=" in l)
sec = []
a = sec.append
a("## End-to-end between two gateway hubs in netns (G10)")
a("")
a("Run %s by `7-relay/verify/carrier.sh` (row V4, gate G10) against `stub-ntfy.py`, the stdlib stand-in (never a real ntfy server: no `NTFY_BIN` on this box;" % stamp)
a("the real-server items are marked \"not run from this box\" in the owner-server section above, and this file keeps the `-stub` name).")
a("Two real `111 daemon` gateways G and G′ (debug build), each with a member (LG, LGP), in five network namespaces; G and G′ enrolled once over tcp-lan,")
a("then their bridge ports were isolated so that the only host both reach is the stub. A sniffer on each gateway's eth0 saw no frame addressed to the other.")
a("")
a("Request sequence of the first delivery (the stub's access log, the first rows; the topics are named by direction):")
a("")
a("| who | method | kind | topic | status | request body B | response body B |")
a("|---|---|---|---|---|---|---|")
a(table)
a("")
a("Records:")
for k, label in [
    ("frame_1500", "a 1,500 B text: the sealed frame in one batch"),
    ("body_1500", "…and its ntfy message body (base64url of 0x01 ‖ n ‖ len ‖ frame)"),
    ("overhead", "item header, wrap and hint added to the text"),
    ("edge_text", "the largest text that fits `max_frame` (3,068 B)"),
    ("edge_body", "…posted as a body of"),
    ("rss_5a", "G's RSS in 5A (debug build)"),
    ("rss_5b", "G's RSS with the 5B stream held"),
    ("hwm_5b", "G's VmHWM after the 5B phase"),
    ("teardown_5b", "the 5B thread's teardown after the switch back to 5A"),
    ("posts_total", "POST requests of G and G′ in the main run"),
    ("max_body", "the largest POST body of the main run"),
]:
    if k in facts:
        a("- %s: %s" % (label, facts[k]))
a("- not recorded: per-request latency (the stub's access log carries none), the server's own version and config (a stub), whether a server forwards to a push service (a stub)")
a("")
text = "\n".join(sec) + "\n"
try:
    old = open(path).read()
except OSError:
    old = ""
if not old:
    old = "# ntfy smoke log (stub run)\n\nWritten by `7-relay/verify/carrier.sh` against `stub-ntfy.py`; run `carrier.sh --smoke` for the prototype sections of §6.4.\n\n"
old = old.replace("- end-to-end delivery between two gateway hubs in netns (G10, row V4): **not run from this box**",
                  "- end-to-end delivery between two gateway hubs in netns (G10, row V4) against a real server: **not run from this box** (run against the stub only: the section below)")
if carried:
    old = old.rstrip("\n") + "\n\n> The sections above are P1's (`carrier.sh --smoke`), carried here from `%s` because no P1 log of this day exists; rerun `--smoke` to regenerate them.\n\n" % carried
old = re.sub(r"## End-to-end between two gateway hubs in netns \(G10\)\n.*?(?=\n## |\Z)", "", old, flags=re.S).rstrip("\n") + "\n\n"
open(path, "w").write(old + text)
PY
  local sec miss="" carried_note=""
  [ -n "$base" ] && carried_note="; the smoke sections carried from $(basename "$base")"
  for sec in "## End-to-end between two gateway hubs in netns (G10)" "## Requests" "## The 4,096 / 4,097 B boundary" "## 429" \
    "## \`since\` after a stub restart" "## Keepalive and the 5B thread" "## RSS during a flush" "## Bytes per frame" "## Owner-server items"; do
    grep -qF "$sec" "$log" 2>/dev/null || miss="$miss [$sec]"
  done
  if [ -s "$log" ] && [ -z "$miss" ]; then
    ok "wrote ${log#"$L7_OS"/} (the G10 section and every §6.4 section of the smoke log are in it; a stub run, so the -stub name$carried_note)"
  else
    fail "the smoke log ${log#"$L7_OS"/} is missing$miss"
  fi
}

# ------------------------------------------------------------------ outer
case ${1:-} in
  --smoke)
    smoke
    exit 0
    ;;
  "" | --inner) ;;
  *)
    printf 'usage: %s [--smoke]\n' "$0" >&2
    exit 2
    ;;
esac
for tool in python3 ip unshare nsenter jq curl awk sha256sum ss; do
  command -v "$tool" >/dev/null 2>&1 || skip_or_fail G10 "$tool is absent"
done
have_netns || skip_or_fail G10 "unprivileged network namespaces are not available (unshare -Urnm)"
unshare --user --map-root-user --net --pid --fork --mount-proc true >/dev/null 2>&1 ||
  skip_or_fail G10 "a pid namespace with its own /proc is not available (unshare --pid --fork --mount-proc)"
command -v strace >/dev/null 2>&1 || skip_or_fail G10 "strace is absent (the switch-off item needs it: an attempt is visible only as a syscall)"
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
if [ -z "$BIN111" ] || [ ! -x "$BIN111" ]; then skip_or_fail G10 "no built 111 binary (set BIN111 or CARGO_TARGET_DIR)"; fi
BIN111=$(cd "$(dirname "$BIN111")" && pwd)/$(basename "$BIN111")
[ -r "$STUB" ] || skip_or_fail G10 "missing $STUB"

if [ "${1:-}" = --inner ]; then
  inner
  # the sentinel: written only when the inner run reached its end (a shell killed or aborted by `set -u` leaves none)
  printf 'FAILS=%s\n' "$FAILS" >"${G10_WORK:?}/inner.result"
  exit 0
fi

if [ "${REQUIRE_G10:-0}" = 1 ]; then
  for k in G10_STOP_AFTER G10_SETTLE G10_QUIET G10_NEG; do
    if [ -n "${!k:-}" ]; then
      printf 'FAIL: REQUIRE_G10=1 refuses the dev knob %s=%s (it drops or weakens part of the gate)\n' "$k" "${!k}"
      echo "G10: 1 FAIL"
      exit 1
    fi
  done
fi

WORK=$(mktemp -d "${TMPDIR:-/tmp}/l7-g10.XXXXXX") || exit 1
cleanup() {
  if [ "${G10_KEEP:-0}" = 1 ]; then note "kept $WORK"; else rm -rf "${WORK:?}"; fi
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
: >"$INNER_LOG"
G10_WORK=$WORK G10_BIN=$BIN111 \
  unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
  "${RESET_SIG[@]}" bash "$SELF" --inner >"$INNER_LOG" 2>&1 &
NS_PID=$!
trap 'kill -TERM "$NS_PID" 2>/dev/null; wait "$NS_PID" 2>/dev/null; cleanup; exit 130' INT TERM HUP
tail -n +1 -f --pid="$NS_PID" "$INNER_LOG" &
TAIL_PID=$!
NS_RC=0
wait "$NS_PID" || NS_RC=$?
wait "$TAIL_PID" 2>/dev/null || true
trap cleanup EXIT
inner_fails=$(grep -c '^FAIL:' "$INNER_LOG" || true)
if [ ! -s "$WORK/inner.result" ] || [ "$NS_RC" != 0 ]; then
  fail "the inner run did not finish (exit status $NS_RC, sentinel $([ -s "$WORK/inner.result" ] && echo present || echo missing)): the checks after its last line never ran; last lines: $(tail -n 3 "$INNER_LOG" | tr '\n' ' ')"
fi
if grep -q 'command not found' "$INNER_LOG"; then
  fail "the inner run hit a command that does not exist (a check that cannot fail): $(grep -m3 'command not found' "$INNER_LOG" | tr '\n' ' ')"
fi
FAILS=$((FAILS + inner_fails))

# the real server: NTFY_BIN selects a pinned ntfy release (§15 G10); none is on this box
if [ -n "${NTFY_BIN:-}" ] && [ -x "$NTFY_BIN" ]; then
  note "NTFY_BIN=$NTFY_BIN is present, but this harness runs the stub only (the real server needs a recording front for the request log): the real-server run is SKIPPED"
  [ "${REQUIRE_NTFY:-0}" = 1 ] && fail "REQUIRE_NTFY=1 and the real-server run is not built into this harness"
else
  note "NTFY_BIN is not set or not executable (no pinned real ntfy release on this box): the real-server items (the server's version and config, retention with and without cache-file, since=<id> after eviction, the keepalive interval, a forward to a push service) are SKIPPED; this run's log is the stub's (logs/ntfy-smoke-<calver>-stub.md)"
  [ "${REQUIRE_NTFY:-0}" = 1 ] && fail "REQUIRE_NTFY=1 and NTFY_BIN is absent"
fi

# hygiene: nothing of this run is left
left=""
for p in /proc/[0-9]*; do
  pid=${p#/proc/}
  [ "$pid" = $$ ] && continue
  if { tr '\0' '\n' <"$p/environ"; } 2>/dev/null | grep -qE "^(NODE111_ROOT|G10_WORK)=$WORK" ||
    { tr '\0' '\n' <"$p/cmdline"; } 2>/dev/null | grep -qF "$WORK"; then
    left="$left $pid:$(cat "$p/comm" 2>/dev/null)"
  fi
done
if [ -z "$left" ]; then ok "no daemon, stub, sniffer or helper of this run is left"; else fail "left running:$left"; fi
finish G10
