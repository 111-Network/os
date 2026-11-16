#!/usr/bin/env bash
# 7-relay/verify/presence.sh — Layer 7 gate G8 (spec §15 G8; §2, §3.5, §7.1, §7.7, §10.1; row V2):
#   presence, the Nearby `111` rows, the registry and the mDNS record; hub H, members M1 and M2 in netns,
#   M1 and M2 also direct.
#
#   bash 7-relay/verify/presence.sh              runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G8=1 bash 7-relay/verify/presence.sh    lock run: a missing prerequisite is a FAIL
#
# Shape: one user + pid namespace (`unshare -rnpf`, /proc remounted) holding one network namespace per node on the
# bridge $L7_G8_BR (10.111.78.0/24, 7-relay/verify/lib.sh L7_G8_*): H .10 (hub role, LAN-bound listener, the stub
# ntfy server on its own loopback), M1 .11 and M2 .12 (members of H). Every node runs the real debug `111 daemon`
# (BIN111, or $CARGO_TARGET_DIR/{release,debug}/111) on its own NODE111_ROOT; everything is asserted through a node's
# own loopback /api/v1 (curl via nsenter), a raw mDNS probe, an AF_PACKET logger and /proc.
#   M1 and M2 also reach each other directly, through two recording relays (4-interface/verify/tcp-lan-relay.py, the
#   smoke.sh three-node pattern): killing the relays is the clean way to cut the direct link, restarting them the way
#   to bring it back, and the relays' captures give a real replayed announce.
#   M1 carries the Layer 6 bridge stub (NODE111_BRIDGE_STUBS, 6-transport/verify/bridge.sh:18-19) with a licensed
#   board (stub-mesh), an unlicensed one (stub-app) and a stub named `lxmf` (the one network id the hop read looks
#   for), and a fake Reticulum venv (venvs/rns/bin/{rnsd,rnstatus,rnpath}; rnsd holds the `@rns/111` socket and the
#   fake `rnpath` counts its spawns).
#   M2 runs under a small LD_PRELOAD shim (built here with gcc) that makes bind() of a route-netlink socket on the
#   address groups (0x551) fail: the "second bind failing" of the re-arm item.
#
# What is asserted (every item is ok:/FAIL: lines; a missing real prerequisite is a note: and SKIPPED for that item):
#   1  presence rows: a `111` row on an inbound message and on `delivered`, the §2.2 strings, a growing heard_ago_s
#   2  the LAN-vs-hub split (I1): tcp-lan while a connection is attributed, hub with the direct link cut, both facts
#   3  a replayed announce (a real capture), a card and a keepalive refresh nothing
#   4  the heartbeat codes exact and nothing written (I4); the validator unit tests (cargo, when a test build exists)
#   5  with the bridge stub reporting a licensed board, no Layer 7 path sends to it
#   6  the mdns-sd re-arm: the address watch bound => the check rests at 0 (the mdns thread idles) and an address
#      event changes the A records within seconds; the second bind failing => the 5 s check stays (the thread keeps
#      waking, the log line); one re-arm per event
#   7  `_n111._tcp`: only with lan_record, LAN sharing and a LAN-bound listener; TXT exactly v, r, k; A records only;
#      a browse stops within 5 s; no alias, load, battery, position or model; a role-bit change re-mints once
#   8  the `gateway` row walks its four codes
#   9  the hop read: one rnpath spawn per POST /nearby/hops, none on a GET, 409 component_unavailable, 429 within 30 s
#   10 lost-hub recovery dials no address it was not given (an outbound SYN log of M1)
#
# Env: BIN111 · G8_SETTLE (seconds a fact waits to age, default 3) · G8_KEEP=1 keep the scratch root · TMPDIR for the
# scratch root. Dev knobs, each REFUSED (a FAIL) under REQUIRE_G8=1: G8_UNTIL=N (stop after phase N) · G8_SKIP_CARGO=1 ·
# G8_REARM_WIN / G8_REARM_EW (the re-arm windows, floor 20 s) · G8_SETTLE_MAX (the settle wait) · G8_DIAG=1 (strace).
# Under REQUIRE_G8=1 a missing replay capture, gcc or shim is a FAIL too, not a note.
# Nothing of ours stays running: every process dies with the namespaces or is stopped first.
#
# Checks are condition strings eval'd by t() at check time, so single-quoted `$vars` are intended:
# shellcheck disable=SC2016,SC2034,SC2317,SC2329,SC2086
set -u
export LC_ALL=C
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=7-relay/verify/lib.sh disable=SC1091
. 7-relay/verify/lib.sh

OS=$L7_OS
SMOKE_DIR=$OS/4-interface/verify
RELAY_PY=$SMOKE_DIR/tcp-lan-relay.py
STUB_NTFY=$L7_VERIFY/stub-ntfy.py
SETTLE=${G8_SETTLE:-3}
read -ra NODES <<<"$L7_G8_NODES"
declare -A NIP=([H]=$L7_G8_IP_H [M1]=$L7_G8_IP_M1 [M2]=$L7_G8_IP_M2)
declare -A VETH=([H]=$L7_G8_VETH_H [M1]=$L7_G8_VETH_M1 [M2]=$L7_G8_VETH_M2)
DAPI=http://127.0.0.1:$L7_PORT_DASH
DASH_HOSTS=111.local,111-2.local,111-3.local # the dashboards' mDNS names (a name taken is renamed -2, -3)
RELAY_PORT=11184 # the recording relays' listen port on M1 and M2 (a peer address of the other member)
NEW_IP=10.111.78.111 # the address event of the re-arm item
MOVED_IP=10.111.78.110 # H's new address of the lost-hub item

OKS=0
ok() {
  printf 'ok: %s\n' "$*"
  OKS=$((OKS + 1))
}
fail() {
  printf 'FAIL: %s\n' "$*"
  FAILS=$((FAILS + 1))
}
T0=${EPOCHREALTIME/,/.}
elapsed() { awk -v a="$T0" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}'; }
phase() { printf '\n== %s  (t+%ss)\n' "$*" "$(elapsed)"; }
now() { printf '%s' "${EPOCHREALTIME/,/.}"; }
since() { awk -v a="$1" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}'; }

# ------------------------------------------------------------------ the generated helper programs (scratch only)
# mdnsq.py: a raw mDNS probe. Binds 5353 (reuse), joins 224.0.0.251 on --ip, sends one PTR query for --service (and
# optionally an A query for --host) and collects every response for --wait seconds. Prints one JSON object:
# {"ptr": [...], "srv": {inst: [port, target]}, "txt": {inst: [strings]}, "a": {host: [ips]}, "aaaa": {host: [..]}}.
write_mdnsq() {
  cat >"$1" <<'PY'
import json, socket, struct, sys, time
def parse_name(b, o):
    labels, jumped, end = [], False, o
    for _ in range(128):
        n = b[o]
        if n == 0:
            o += 1
            break
        if n & 0xC0 == 0xC0:
            ptr = ((n & 0x3F) << 8) | b[o + 1]
            if not jumped:
                end = o + 2
            jumped, o = True, ptr
            continue
        labels.append(b[o + 1:o + 1 + n].decode("utf-8", "replace"))
        o += 1 + n
    if not jumped:
        end = o
    return ".".join(labels), end
def parse(b, res):
    if len(b) < 12:
        return
    qd, an, ns, ar = struct.unpack("!4H", b[4:12])
    o = 12
    for _ in range(qd):
        _, o = parse_name(b, o)
        o += 4
    for _ in range(an + ns + ar):
        nm, o = parse_name(b, o)
        t, c, ttl, rl = struct.unpack("!HHIH", b[o:o + 10])
        o += 10
        rd = b[o:o + rl]
        if t == 12:
            res["ptr"].add(parse_name(b, o)[0])
        elif t == 33:
            res["srv"][nm] = [struct.unpack("!H", rd[4:6])[0], parse_name(b, o + 6)[0]]
        elif t == 16:
            strs, p = [], 0
            while p < len(rd):
                strs.append(rd[p + 1:p + 1 + rd[p]].decode("utf-8", "replace"))
                p += 1 + rd[p]
            res["txt"][nm] = strs
        elif t == 1:
            res["a"].setdefault(nm, set()).add(socket.inet_ntoa(rd))
        elif t == 28:
            res["aaaa"].setdefault(nm, set()).add(socket.inet_ntop(socket.AF_INET6, rd))
        o += rl
def question(name, qtype):
    q = b"".join(bytes([len(l)]) + l.encode() for l in name.split(".")) + b"\0"
    return q + struct.pack("!HH", qtype, 1)
ip, service, wait = sys.argv[1], sys.argv[2], float(sys.argv[3])
hosts = [h for h in (sys.argv[4].split(",") if len(sys.argv) > 4 else []) if h]
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
try:
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
except OSError:
    pass
s.bind(("", 5353))
s.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP, socket.inet_aton("224.0.0.251") + socket.inet_aton(ip))
s.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF, socket.inet_aton(ip))
s.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_LOOP, 0)
res = {"ptr": set(), "srv": {}, "txt": {}, "a": {}, "aaaa": {}}
qs = [(service, 12)] + [(h, 1) for h in hosts]
hdr = struct.pack("!6H", 0, 0, len(qs), 0, 0, 0)
s.sendto(hdr + b"".join(question(n, t) for n, t in qs), ("224.0.0.251", 5353))
end = time.monotonic() + wait
while time.monotonic() < end:
    s.settimeout(max(0.05, end - time.monotonic()))
    try:
        data, _ = s.recvfrom(9000)
    except socket.timeout:
        break
    try:
        parse(data, res)
    except Exception:
        pass
print(json.dumps({"ptr": sorted(res["ptr"]), "srv": res["srv"], "txt": res["txt"],
                  "a": {k: sorted(v) for k, v in res["a"].items()},
                  "aaaa": {k: sorted(v) for k, v in res["aaaa"].items()}}))
PY
}

# pktlog.py: an AF_PACKET logger on one interface (outbound only): every TCP SYN and every mDNS query, one JSON line
# each with a monotonic time (CLOCK_MONOTONIC, the shell's `cat /proc/uptime` clock is not used: times are compared
# only to each other inside this file's own output and to `now` printed by `pktlog.py --now`).
write_pktlog() {
  cat >"$1" <<'PY'
import json, signal, socket, struct, sys, time
iface, out = sys.argv[1], sys.argv[2]
stop = False
def bye(*_):
    global stop
    stop = True
signal.signal(signal.SIGTERM, bye)
signal.signal(signal.SIGINT, bye)
def qname(p):
    labels, o = [], 12
    try:
        while p[o]:
            labels.append(p[o + 1:o + 1 + p[o]].decode("utf-8", "replace"))
            o += 1 + p[o]
    except IndexError:
        pass
    return ".".join(labels)
s = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.ntohs(3))
s.bind((iface, 0))
s.settimeout(0.5)
f = open(out, "w", buffering=1)
f.write(json.dumps({"ready": time.time()}) + "\n")
while not stop:
    try:
        d, a = s.recvfrom(65535)
    except socket.timeout:
        continue
    out = a[2] == 4
    if d[12:14] == b"\x08\x06" and out and len(d) >= 42 and d[20:22] == b"\x00\x01":
        f.write(json.dumps({"t": time.time(), "k": "arp", "dst": socket.inet_ntoa(d[38:42])}) + "\n")
        continue
    if len(d) < 34 or d[12:14] != b"\x08\x00":
        continue
    ihl = (d[14] & 15) * 4
    proto = d[23]
    dst = socket.inet_ntoa(d[30:34])
    t = time.time()
    l4 = 14 + ihl
    if proto == 6 and out and len(d) >= l4 + 14:
        sport, dport = struct.unpack("!HH", d[l4:l4 + 4])
        flags = d[l4 + 13]
        if flags & 2 and not flags & 16:
            f.write(json.dumps({"t": t, "k": "syn", "dst": dst, "port": dport}) + "\n")
    elif proto == 17 and len(d) >= l4 + 8:
        sport, dport = struct.unpack("!HH", d[l4:l4 + 4])
        p = d[l4 + 8:]
        if dport == 5353 and len(p) >= 12:
            resp = bool(p[2] & 0x80)
            if not out:
                f.write(json.dumps({"t": t, "k": "mdnsin", "resp": resp}) + "\n")
            elif not resp:
                f.write(json.dumps({"t": t, "k": "mdnsq", "name": qname(p)}) + "\n")
            else:
                f.write(json.dumps({"t": t, "k": "mdnsr"}) + "\n")
        elif dport == 53 and out:
            f.write(json.dumps({"t": t, "k": "dns", "name": qname(p)}) + "\n")
PY
}

# bindshim.c: bind() of a route-netlink socket on exactly the address watch's groups (0x551) fails with EPERM.
write_shim() {
  cat >"$1" <<'C'
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <linux/netlink.h>
#include <sys/socket.h>
int bind(int fd, const struct sockaddr *a, socklen_t l) {
  static int (*real)(int, const struct sockaddr *, socklen_t);
  if (!real) real = (int (*)(int, const struct sockaddr *, socklen_t))dlsym(RTLD_NEXT, "bind");
  if (a && a->sa_family == AF_NETLINK && l >= sizeof(struct sockaddr_nl)) {
    const struct sockaddr_nl *n = (const void *)a;
    if (n->nl_groups == 0x551) { errno = EPERM; return -1; }
  }
  return real(fd, a, l);
}
C
}

# fake Reticulum tools for M1's venvs/rns (the daemon's transport spine supervises `rnsd`, reads `rnstatus`, and the
# hop read runs `rnpath -t -j`): rnsd binds and listens the abstract socket the rendered config names (@rns/<name>) so
# the daemon's ownership gate (N/transport/rnstatus.rs `owner::ready`) passes; rnpath counts every spawn.
write_fake_rns() { # ROOT
  local root=$1 bin=$1/venvs/rns/bin
  mkdir -p "$bin" "$root/cache/packages/python"
  cat >"$bin/rnsd" <<EOF
#!/bin/sh
# L7 G8 fixture: a stand-in rnsd that holds the shared-instance socket and does nothing else
exec python3 -I -c 'import re, socket, sys, time
cfg = sys.argv[1] + "/config"
name = "default"
try:
    m = re.search(r"^\s*instance_name\s*=\s*(\S+)", open(cfg).read(), re.M)
    if m:
        name = m.group(1).strip("\"\x27")
except OSError:
    pass
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind("\0rns/" + name)
s.listen(8)
print("fake rnsd listening on @rns/" + name, flush=True)
while True:
    time.sleep(3600)' "\${2:-$root/rns}"
EOF
  cat >"$bin/rnstatus" <<EOF
#!/bin/sh
# L7 G8 fixture: counts its spawns (the Pi rule: no reader, no rnstatus), prints a one-line status
echo "\$*" >>"$root/rnstatus-calls.log"
echo "Shared Instance[rns/111]"
EOF
  cat >"$bin/rnpath" <<EOF
#!/bin/sh
# L7 G8 fixture: counts its spawns, prints a path table (the shape of §10.1 rule 1)
echo "\$*" >>"$root/rnpath-calls.log"
printf '[{"hash":"%s","timestamp":1,"via":"%s","hops":2,"expires":2,"interface":"fake0"}]\n' \
  00112233445566778899aabbccddeeff ffeeddccbbaa99887766554433221100
EOF
  chmod 755 "$bin/rnsd" "$bin/rnstatus" "$bin/rnpath"
  printf '# L7 G8 fixture lock: stands in for the release lock\n' >"$root/cache/packages/python/rns-lock.txt"
  printf '%s' "$(sha256sum "$root/cache/packages/python/rns-lock.txt" | cut -c1-64)" >"$root/venvs/rns/.111-integrity"
}

# ------------------------------------------------------------------ inner helpers
declare -A HOLD=() DPID=() NROOT=() NID=() STARTS=()
nx() {
  local n=$1
  shift
  nsenter -t "${HOLD[$n]}" -n -- "$@"
}
NSX=()
req() {
  local m=$1 u=$2
  shift 2
  : >"$W/hdr"
  : >"$W/body"
  ST=$("${NSX[@]}" curl -sS -o "$W/body" -D "$W/hdr" -w '%{http_code}' -X "$m" --max-time "${REQ_TIMEOUT:-20}" "$@" "$u" 2>"$W/curl.err") || true
  [ -n "$ST" ] || ST=000
}
api() {
  local n=$1 m=$2 p=$3
  shift 3
  NSX=(nsenter -t "${HOLD[$n]}" -n --)
  req "$m" "$DAPI$p" "$@"
  NSX=()
}
jsend() { api "$1" "$2" "$3" -H 'Content-Type: application/json' -d "$4"; }
has_json() { [ -s "$1" ] && grep -q '[^[:space:]]' "$1" 2>/dev/null; }
jb() { has_json "$W/body" && jq -e "$@" "$W/body" >/dev/null 2>&1; }
jget() { jq -r "$@" "$W/body" 2>/dev/null || true; }
hdr() { grep -i "^$1:" "$W/hdr" 2>/dev/null | head -1 | cut -d: -f2- | tr -d '\r' | sed 's/^ //'; }
snip() { head -c 300 "$W/body" 2>/dev/null | tr '\n\r' '  '; }
t() {
  if eval "$2"; then ok "$1"; else fail "$1  [HTTP ${ST:-?}: $(snip)]"; fi
}
problem_is() { [ "$ST" = "$1" ] && [ "$(jget .code)" = "$2" ]; }
wait_for() {
  local end=$((SECONDS + $1))
  while :; do
    if eval "$2"; then return 0; fi
    [ "$SECONDS" -ge "$end" ] && return 1
    sleep 0.4
  done
}
tw() { # SECONDS DESC CONDITION
  wait_for "$1" "$3" || true
  t "$2" "$3"
}
sha() { sha256sum "$1" 2>/dev/null | cut -c1-64; }

start_node() { # NODE [extra env KEY=VAL…]
  local n=$1 up=0 code
  shift
  STARTS[$n]=$((${STARTS[$n]:-0} + 1))
  NODE111_ROOT=${NROOT[$n]} nsenter -t "${HOLD[$n]}" -n -- env "$@" "$BIN111" daemon >>"${NROOT[$n]}/daemon.log" 2>&1 &
  DPID[$n]=$!
  for _ in $(seq 1 300); do
    code=$(nx "$n" curl -s -o /dev/null -w '%{http_code}' --max-time 2 "$DAPI/health" 2>/dev/null) || true
    case $code in 200 | 503) up=1 && break ;; esac
    kill -0 "${DPID[$n]}" 2>/dev/null || break
    sleep 0.2
  done
  if [ "$up" != 1 ]; then
    fail "$n: 111 daemon never answered /health"
    tail -20 "${NROOT[$n]}/daemon.log" | sed "s/^/  $n daemon| /"
    return 1
  fi
  local _sn=$n
  wait_for 20 'api "$_sn" GET /api/v1/fabric; [ "$ST" = 200 ] && jb ".store.open == true"' || true
  ok "$n: 111 daemon up (pid ${DPID[$n]}), fabric store open"
}
stop_node() { # NODE
  local n=$1 p=${DPID[$1]:-}
  [ -n "$p" ] || return 0
  kill -TERM "$p" 2>/dev/null || true
  for _ in $(seq 1 100); do
    kill -0 "$p" 2>/dev/null || break
    sleep 0.2
  done
  kill -KILL "$p" 2>/dev/null || true
  wait "$p" 2>/dev/null || true
  DPID[$n]=""
}
# the daemon's real pid (nsenter + env exec the daemon: the child of $DPID, or $DPID itself after the exec)
daemon_pid() {
  local p=${DPID[$1]:-}
  [ -n "$p" ] || return 1
  if [ "$(cat /proc/"$p"/comm 2>/dev/null)" = 111 ]; then
    printf '%s\n' "$p"
  else
    pgrep -P "$p" -x 111 | head -n 1
  fi
}

# -- the Nearby 111 source
nearby() { api "$1" GET /api/v1/nearby; }
# nrow NODE NODEID → the row of that node in NODE's `111` source (compact JSON), or nothing
nrow() {
  nearby "$1"
  [ "$ST" = 200 ] || return 0
  jq -c --arg id "$2" '[.sources[]? | select(.kind == "111") | .nodes[]? | select(.id == $id)] | first // empty' "$W/body" 2>/dev/null
}
nrows() { # NODE → the 111 source's node ids, sorted
  nearby "$1"
  [ "$ST" = 200 ] || return 0
  jq -r '[.sources[]? | select(.kind == "111") | .nodes[]?.id] | sort | join(" ")' "$W/body" 2>/dev/null
}
rowf() { printf '%s' "$1" | jq -r "$2" 2>/dev/null; }
# peers_of NODE → attributed peers on the tcp-lan link
peers_of() {
  api "$1" GET /api/v1/fabric
  [ "$ST" = 200 ] || return 0
  jget 'if (.links | type) == "array" then ([.links[] | select(.id == "tcp-lan") | .peers] | add // 0) else empty end'
}
# send NODE TO_NODEID TEXT → $MID
send() {
  jsend "$1" POST /api/v1/messages "$(jq -nc --arg to "$2" --arg t "$3" '{to: $to, text: $t}')"
  MID=$(jget .id)
}
delivered() { # NODE PEER MID
  api "$1" GET "/api/v1/messages/threads/$2?limit=100"
  jb --arg id "$3" '[.messages[]? | select(.id == $id and .state == "delivered")] | length == 1'
}
# a thread's wakeups: summed voluntary context switches of the threads of PID whose comm matches RE
vcs_sum() { # PID RE
  local p=$1 re=$2 th c cm s=0
  for th in /proc/"$p"/task/*; do
    cm=$(cat "$th/comm" 2>/dev/null) || continue
    [[ "$cm" =~ $re ]] || continue
    c=$(awk '/^voluntary_ctxt_switches:/ { print $2 }' "$th/status" 2>/dev/null) || continue
    s=$((s + ${c:-0}))
  done
  printf '%s\n' "$s"
}
thread_names() { # PID
  local th
  for th in /proc/"$1"/task/*; do cat "$th/comm" 2>/dev/null; done | sort | uniq -c | tr '\n' ';'
}
# mdns NODE_OBSERVER [WAIT] [HOST] → $W/mdns.json (a raw probe from that node's netns)
mdns() {
  nx "$1" python3 -I "$W/mdnsq.py" "${NIP[$1]}" _n111._tcp.local "${2:-2}" ${3:-} >"$W/mdns.json" 2>"$W/mdns.err" || true
}
# the record of the instance named by the L2 label of $1's node, from the last probe: its TXT strings (one per line)
rec_txt() { jq -r '.txt | to_entries[] | select(.key | endswith("._n111._tcp.local")) | .value[]' "$W/mdns.json" 2>/dev/null; }
rec_has() { has_json "$W/mdns.json" && [ "$(jq '[.txt | keys[] | select(endswith("._n111._tcp.local"))] | length' "$W/mdns.json" 2>/dev/null)" -ge 1 ]; }
rec_a() { jq -r '[.a | to_entries[] | .value[]] | unique | join(" ")' "$W/mdns.json" 2>/dev/null; }
# srv_of_n111 → "port host" of the first _n111._tcp instance
srv_port() { jq -r '[.srv | to_entries[] | select(.key | endswith("._n111._tcp.local")) | .value[0]] | first // empty' "$W/mdns.json" 2>/dev/null; }
srv_target() { jq -r '[.srv | to_entries[] | select(.key | endswith("._n111._tcp.local")) | .value[1]] | first // empty' "$W/mdns.json" 2>/dev/null; }
# probe_until NODE COND [SECONDS] — probe repeatedly (2 s windows) until COND (eval'd) holds
probe_until() {
  local end=$((SECONDS + ${3:-10}))
  while :; do
    mdns "$1" 2
    if eval "$2"; then return 0; fi
    [ "$SECONDS" -ge "$end" ] && return 1
  done
}

# ------------------------------------------------------------------ inner (in the namespaces)
inner() {
  : "${G8_WORK:?}" "${BIN111:?}"
  W=$G8_WORK/http
  mkdir -p "$W"
  local n
  for n in "${NODES[@]}"; do
    NROOT[$n]=$G8_WORK/$n
    NID[$n]=$(cat "$G8_WORK/$n.nid" 2>/dev/null || echo none)
  done
  local RELAYP=() PKT=() STUBP=""
  cleanup_inner() {
    local p
    for p in "${PKT[@]:-}" "${RELAYP[@]:-}" ${STUBP:-}; do [ -n "$p" ] && kill "$p" 2>/dev/null; done
    for n in "${NODES[@]}"; do
      if [ -n "${DPID[$n]:-}" ]; then kill -TERM "${DPID[$n]}" 2>/dev/null || true; fi
    done
    for _ in $(seq 1 30); do
      local alive=0
      for n in "${NODES[@]}"; do [ -n "${DPID[$n]:-}" ] && kill -0 "${DPID[$n]}" 2>/dev/null && alive=1; done
      [ "$alive" = 0 ] && break
      sleep 0.3
    done
    printf 'FAILS=%s\n' "$FAILS" >"$G8_WORK/inner.result"
  }
  trap cleanup_inner EXIT
  trap 'exit 130' INT TERM

  # ---------------------------------------------------------------- network
  phase "1. namespaces: $L7_G8_BR $L7_G8_NET, H $L7_G8_IP_H, M1 $L7_G8_IP_M1, M2 $L7_G8_IP_M2"
  ip link set lo up
  if ip link add "$L7_G8_BR" type bridge && ip addr add 10.111.78.1/24 dev "$L7_G8_BR" && ip link set "$L7_G8_BR" up; then
    ok "bridge $L7_G8_BR 10.111.78.1/24 up (no route off-box)"
  else
    fail "cannot create $L7_G8_BR"
  fi
  local myns
  myns=$(readlink /proc/self/ns/net)
  for n in "${NODES[@]}"; do
    unshare --net sleep 100000 &
    HOLD[$n]=$!
  done
  for n in "${NODES[@]}"; do
    wait_for 5 '[ "$(readlink /proc/${HOLD[$n]}/ns/net 2>/dev/null)" != "$myns" ]' || true
    if ip link add "${VETH[$n]}" type veth peer name eth0 netns "${HOLD[$n]}" && ip link set "${VETH[$n]}" master "$L7_G8_BR" up &&
      nx "$n" ip link set lo up && nx "$n" ip addr add "${NIP[$n]}/24" dev eth0 && nx "$n" ip link set eth0 up; then
      ok "$n: netns (holder ${HOLD[$n]}) eth0 ${NIP[$n]}/24 on $L7_G8_BR, host-side veth ${VETH[$n]}"
    else
      fail "$n: netns/veth setup failed"
    fi
  done
  if [ -n "$(ip route show default)" ]; then fail "a default route exists"; fi

  write_mdnsq "$W/mdnsq.py"
  write_pktlog "$W/pktlog.py"

  # the direct link: two recording relays (killing both cuts it; starting them brings it back)
  relays_up() { # RUN — the captures go to cap/RUN/{m1,m2}
    mkdir -p "$G8_WORK/cap/$1/m1" "$G8_WORK/cap/$1/m2"
    nsenter -t "${HOLD[M1]}" -n -- python3 -I "$RELAY_PY" relay --listen "${NIP[M1]}:$RELAY_PORT" --to "${NIP[M1]}:$L7_PORT_FABRIC" --dir "$G8_WORK/cap/$1/m1" >"$G8_WORK/relay-m1.log" 2>&1 &
    RELAYP[0]=$!
    nsenter -t "${HOLD[M2]}" -n -- python3 -I "$RELAY_PY" relay --listen "${NIP[M2]}:$RELAY_PORT" --to "${NIP[M2]}:$L7_PORT_FABRIC" --dir "$G8_WORK/cap/$1/m2" >"$G8_WORK/relay-m2.log" 2>&1 &
    RELAYP[1]=$!
    wait_for 8 'nx M1 timeout 2 bash -c "exec 3<>/dev/tcp/${NIP[M1]}/$RELAY_PORT" 2>/dev/null && nx M2 timeout 2 bash -c "exec 3<>/dev/tcp/${NIP[M2]}/$RELAY_PORT" 2>/dev/null'
  }
  relays_down() {
    local p
    for p in "${RELAYP[@]}"; do kill "$p" 2>/dev/null; done
    for p in "${RELAYP[@]}"; do wait "$p" 2>/dev/null; done
    RELAYP=()
  }
  if relays_up run1; then
    ok "relays up: M1 ${NIP[M1]}:$RELAY_PORT → M1:$L7_PORT_FABRIC, M2 ${NIP[M2]}:$RELAY_PORT → M2:$L7_PORT_FABRIC (the direct link's wire)"
  else
    fail "the recording relays did not come up"
  fi

  # ---------------------------------------------------------------- daemons
  phase "2. three daemons: H (hub), M1 (bridge stubs, fake rnsd), M2 (bind shim)"
  start_node H
  start_node M1 NODE111_BRIDGE_STUBS="${NROOT[M1]}/g8-scenario.json"
  tw 20 "M1: the bridge scenario hook enabled its stubs ($(tail -1 "${NROOT[M1]}/g8-ctl/ready.jsonl" 2>/dev/null | jq -c .stubs 2>/dev/null))" '[ -s "${NROOT[M1]}/g8-ctl/ready.jsonl" ]'
  if [ -n "${G8_SHIM:-}" ] && [ -f "$G8_SHIM" ]; then
    start_node M2 LD_PRELOAD="$G8_SHIM"
  else
    start_node M2
  fi
  local pH pM1 pM2
  pH=$(daemon_pid H)
  pM1=$(daemon_pid M1)
  pM2=$(daemon_pid M2)
  note "daemon threads: H: $(thread_names "$pH")"
  note "daemon threads: M2: $(thread_names "$pM2")"
  for n in H M1 M2; do # the egress logs (outbound SYNs, ARP who-has, mDNS questions), on each member's eth0
    nsenter -t "${HOLD[$n]}" -n -- python3 -I "$W/pktlog.py" eth0 "$G8_WORK/pkt-$n.jsonl" >/dev/null 2>&1 &
    PKT+=($!)
  done

  # ---------------------------------------------------------------- enrolment + contacts
  phase "3. hub enrolment (H offers, M1 and M2 enrol), contact cards both ways between M1 and M2"
  local code addr card
  for n in M1 M2; do
    jsend H POST /api/v1/fabric/hub/offer '{}'
    code=$(jget .code)
    addr=$(jget .address)
    card=$(jget .card.text)
    jsend "$n" POST /api/v1/fabric/hub/enroll "$(jq -nc --arg a "$addr" --arg c "$code" --arg k "$card" '{address: $a, code: $c, card: $k}')"
    t "$n: POST /fabric/hub/enroll {H's offer} → 200, enrolled and reachable, node_id = H (address $addr)" \
      '[ "$ST" = 200 ] && jb ".enrolled == true and .reachable == true" && [ "$(jget .node_id)" = "${NID[H]}" ]'
  done
  local card_m1 card_m2
  api M1 GET /api/v1/contacts/me/card
  card_m1=$(jget .text)
  api M2 GET /api/v1/contacts/me/card
  card_m2=$(jget .text)
  jsend M1 POST /api/v1/contacts "$(jq -nc --arg c "$card_m2" '{card: $c}')"
  t "M1: POST /contacts {card: M2} → M2 has_key" '[ "$ST" = 200 ] && jb ".has_key == true"'
  jsend M2 POST /api/v1/contacts "$(jq -nc --arg c "$card_m1" '{card: $c}')"
  t "M2: POST /contacts {card: M1} → M1 has_key" '[ "$ST" = 200 ] && jb ".has_key == true"'
  sleep "$SETTLE"
  # a card is not a fact: no row for the other member yet (only the home hub's, from hub.reachable)
  local rows2 rows1
  rows2=$(nrows M2)
  rows1=$(nrows M1)
  t "a pasted card creates no row: M2's 111 source holds only its hub ($rows2), M1's only its hub ($rows1)" \
    '[ "$rows2" = "${NID[H]}" ] && [ "$rows1" = "${NID[H]}" ]'
  local hrow
  hrow=$(nrow M2 "${NID[H]}")
  t "the hub's row is the home hub's: tcp-lan, \"your hub answered\", a heard_ago_s and no heard_seq ($(rowf "$hrow" '[.evidence[]? | "\(.transport): \(.text)"] | join("; ")'))" \
    '[ "$(rowf "$hrow" ".evidence | length")" = 1 ] && [ "$(rowf "$hrow" ".evidence[0].transport")" = tcp-lan ] && [ "$(rowf "$hrow" ".evidence[0].text")" = "your hub answered" ] && [ "$(rowf "$hrow" ".heard_ago_s | type")" = number ] && [ "$(rowf "$hrow" ".heard_seq")" = null ]'
  nearby M2
  t "the source is \"111 nodes heard by this node\", kind 111, device_id null, num 0 rows" \
    'jb "[.sources[] | select(.kind == \"111\" and .label == \"111 nodes heard by this node\" and .device_id == null)] | length == 1" && jb "[.sources[] | select(.kind == \"111\") | .nodes[] | select(.num != 0)] | length == 0"'

  # ---------------------------------------------------------------- 1. a message and its confirmation
  phase "4. presence rows: an inbound message and an own delivered, the §2.2 strings, a growing heard_ago_s"
  # the direct link: M1 and M2 each hold the other as a peer through the relays; make sure it is attributed first
  local a1 a2 mid
  send M1 "${NID[M2]}" "g8 first message, over the direct link"
  mid=$MID
  tw 40 "M1 → M2: delivered" 'delivered M1 "${NID[M2]}" "$mid"'
  local r2 r1
  r2=$(nrow M2 "${NID[M1]}")
  r1=$(nrow M1 "${NID[M2]}")
  t "M2 (inbound message): a 111 row for M1 — transport tcp-lan, \"a message from them arrived (verified)\" ($(rowf "$r2" '[.evidence[]? | "\(.transport): \(.text)"] | join("; ")'))" \
    '[ "$(rowf "$r2" ".id")" = "${NID[M1]}" ] && [ "$(rowf "$r2" "[.evidence[] | select(.transport == \"tcp-lan\" and .text == \"a message from them arrived (verified)\")] | length")" = 1 ]'
  t "M1 (own message turned delivered): a 111 row for M2 — \"they confirmed your message\" ($(rowf "$r1" '[.evidence[]? | "\(.transport): \(.text)"] | join("; ")'))" \
    '[ "$(rowf "$r1" ".id")" = "${NID[M2]}" ] && [ "$(rowf "$r1" "[.evidence[] | select(.text == \"they confirmed your message\")] | length")" = 1 ]'
  t "the row carries num 0, a numeric heard_seq, a numeric heard_ago_s, last_heard_hint null, the contact's signed roles (a list)" \
    '[ "$(rowf "$r2" ".num")" = 0 ] && [ "$(rowf "$r2" ".heard_seq | type")" = number ] && [ "$(rowf "$r2" ".heard_ago_s | type")" = number ] && [ "$(rowf "$r2" ".last_heard_hint")" = null ] && [ "$(rowf "$r2" ".roles | type")" = array ]'
  a1=$(rowf "$r2" .heard_ago_s)
  local seq1
  seq1=$(rowf "$r2" .heard_seq)
  sleep $((SETTLE + 2))
  r2=$(nrow M2 "${NID[M1]}")
  a2=$(rowf "$r2" .heard_ago_s)
  t "heard_ago_s grows with no new fact: $a1 s → $a2 s after $((SETTLE + 2)) s (heard_seq unchanged at $seq1)" \
    '[ "$a2" -ge $((a1 + SETTLE + 1)) ] && [ "$(rowf "$r2" .heard_seq)" = "$seq1" ]'
  # the positive control of "refreshes nothing" below: a new message does refresh (age falls, seq rises)
  send M1 "${NID[M2]}" "g8 second message"
  mid=$MID
  tw 40 "M1 → M2: second message delivered" 'delivered M1 "${NID[M2]}" "$mid"'
  r2=$(nrow M2 "${NID[M1]}")
  t "a new message is a new fact: heard_seq rose from $seq1 to $(rowf "$r2" .heard_seq) and the age fell to $(rowf "$r2" .heard_ago_s) s" \
    '[ "$(rowf "$r2" .heard_seq)" -gt "$seq1" ] && [ "$(rowf "$r2" .heard_ago_s)" -lt "$a2" ]'

  # ---------------------------------------------------------------- 2. the LAN-vs-hub split
  phase "5. the LAN-vs-hub split (I1): the direct link cut, the same contact's next message arrives through H"
  local pa0 pb0
  pa0=$(peers_of M2)
  pb0=$(peers_of M1)
  relays_down
  wait_for 90 '[ "$(peers_of M1)" = $((pb0 - 1)) ] && [ "$(peers_of M2)" = $((pa0 - 1)) ]' || true
  t "the relays are down and the direct link is gone (attributed peers M1 $pb0 → $(peers_of M1), M2 $pa0 → $(peers_of M2); each keeps the hub)" \
    '[ "$(peers_of M1)" = $((pb0 - 1)) ] && [ "$(peers_of M2)" = $((pa0 - 1)) ]'
  send M1 "${NID[M2]}" "g8 through the hub"
  mid=$MID
  tw 60 "M1 → M2 through H: delivered" 'delivered M1 "${NID[M2]}" "$mid"'
  r2=$(nrow M2 "${NID[M1]}")
  r1=$(nrow M1 "${NID[M2]}")
  t "M2's row for M1 shows both facts: tcp-lan (the direct message) and hub (\"a message from them arrived (verified)\") — $(rowf "$r2" '[.evidence[]? | "\(.transport): \(.text)"] | join("; ")')" \
    '[ "$(rowf "$r2" "[.evidence[] | select(.transport == \"tcp-lan\")] | length")" = 1 ] && [ "$(rowf "$r2" "[.evidence[] | select(.transport == \"hub\" and .text == \"a message from them arrived (verified)\")] | length")" = 1 ] && [ "$(rowf "$r2" ".evidence | length")" = 2 ]'
  t "M1's row for M2 shows both facts too: the confirmation came back through the hub — $(rowf "$r1" '[.evidence[]? | "\(.transport): \(.text)"] | join("; ")')" \
    '[ "$(rowf "$r1" "[.evidence[] | select(.transport == \"tcp-lan\")] | length")" = 1 ] && [ "$(rowf "$r1" "[.evidence[] | select(.transport == \"hub\" and .text == \"they confirmed your message\")] | length")" = 1 ]'
  local seq_hub
  seq_hub=$(rowf "$r2" .heard_seq)

  # ---------------------------------------------------------------- helpers of the later phases
  until_phase() { [ "${G8_UNTIL:-99}" -ge "$1" ]; } # a dev knob: stop after phase N
  frame_counts() { # RUN → "conns=N hello=N announce=N sealed=N keepalive=N" over the relay captures of that run
    local files=()
    mapfile -t files < <(find "$G8_WORK/cap/$1" -type f \( -name '*.c2s' -o -name '*.s2c' \) 2>/dev/null)
    if [ ${#files[@]} -eq 0 ]; then
      echo "conns=0 hello=0 announce=0 sealed=0 keepalive=0"
      return
    fi
    python3 -I "$RELAY_PY" frames "${files[@]}" 2>/dev/null | jq -rs '"conns=\(length) hello=\([.[] | select(.hello)] | length) announce=\([.[].frames[]? | select(.[0] == 2)] | length) sealed=\([.[].frames[]? | select(.[0] == 1)] | length) keepalive=\([.[].frames[]? | select(.[0] == 3)] | length)"'
  }
  fcount() { printf '%s\n' "$2" | tr ' ' '\n' | sed -n "s/^$1=//p"; } # fcount KEY "k=v k=v"
  hops_calls() {
    local n
    n=$({ wc -l <"${NROOT[M1]}/rnpath-calls.log"; } 2>/dev/null) || n=0
    [ -n "$n" ] || n=0
    printf '%s\n' "${n// /}"
  }
  CSEQ=0
  CTLD=${NROOT[M1]}/g8-ctl
  ctl() { # JSON → the hook's answer in $CTL (0 when ok)
    local f
    CSEQ=$((CSEQ + 1))
    f=$(printf '%s/%08d' "$CTLD" "$CSEQ")
    printf '%s' "$1" >"$f.tmp" && mv "$f.tmp" "$f.json"
    local end=$((SECONDS + 15))
    while [ ! -s "$f.done" ]; do
      if [ "$SECONDS" -ge "$end" ]; then
        CTL=""
        return 1
      fi
      sleep 0.05
    done
    CTL=$(cat "$f.done")
    rm -f "$f.done"
    [ "$(printf '%s' "$CTL" | jq -r .ok 2>/dev/null)" = true ]
  }
  fcid() { # NET NATIVE → the conversation's fc- id on M1 (waits up to 10 s)
    local i id
    for i in $(seq 1 50); do
      api M1 GET "/api/v1/networks/$1/conversations"
      id=$(jget --arg n "$2" '[.conversations[]? | select(.native_id == $n)] | first | .id // empty')
      if [ -n "$id" ]; then
        printf '%s' "$id"
        return 0
      fi
      sleep 0.2
    done
  }
  host_of() { # IP → the mDNS host whose A records hold IP in the last probe
    jq -r --arg ip "$1" '[.a | to_entries[] | select(.value | index($ip)) | .key] | first // empty' "$W/mdns.json" 2>/dev/null
  }
  # the record of NODE ($1 is its NodeId) in the last probe: the instance whose TXT carries k=<id>
  rec_of() { jq -c --arg k "k=$1" '[.txt | to_entries[] | select(.key | endswith("._n111._tcp.local")) | select(.value | index($k))] | first // empty' "$W/mdns.json" 2>/dev/null; }
  rec_has_k() { [ -n "$(rec_of "$1")" ]; }

  # ---------------------------------------------------------------- 6. a replayed announce, a card, a keepalive
  until_phase 6 || return 0
  phase "6. a replayed announce, a card and a keepalive refresh nothing (and the hop read's first calls, on M1)"
  r2=$(nrow M2 "${NID[M1]}")
  local age0 seq0 ev0 tk0 hops_t0
  age0=$(rowf "$r2" .heard_ago_s)
  seq0=$(rowf "$r2" .heard_seq)
  ev0=$(printf '%s' "$r2" | jq -c .evidence)
  tk0=$(now)
  # the hop read, item 1 (the rest of it is phase 12): no spawn on any GET, one per accepted POST, 429 within 30 s
  for _ in 1 2 3; do nearby M1; done
  api M1 GET /api/v1/relay
  api M1 GET /api/v1/registry
  api M1 GET /api/v1/nearby/hops
  t "the hop read: no rnpath spawn on any GET (3 × /nearby, /relay, /registry, GET /nearby/hops → $ST); $(hops_calls) spawns" \
    '[ "$(hops_calls)" = 0 ] && [[ "$ST" =~ ^4 ]]'
  jsend M1 POST /api/v1/nearby/hops '{}'
  hops_t0=$(now)
  t "POST /nearby/hops → 202 {started: true} (the lxmf stub counts as the Reticulum component, the fake rnsd holds the instance)" \
    '[ "$ST" = 202 ] && jb ".started == true"'
  tw 15 "… and exactly one \`rnpath\` ran ($(cat "${NROOT[M1]}/rnpath-calls.log" 2>/dev/null | head -1 | sed "s#${NROOT[M1]}#<root>#"))" '[ "$(hops_calls)" = 1 ]'
  jsend M1 POST /api/v1/nearby/hops '{}'
  local ra
  ra=$(hdr retry-after)
  t "a second POST within 30 s → 429 rate_limited, Retry-After $ra (1..30)" '[ "$ST" = 429 ] && [ "$(jget .code)" = rate_limited ] && [ "${ra:-0}" -ge 1 ] && [ "${ra:-0}" -le 30 ]'
  sleep 1
  t "… and it spawned nothing (still $(hops_calls) rnpath run)" '[ "$(hops_calls)" = 1 ]'
  for n in M2 H; do
    jsend "$n" POST /api/v1/nearby/hops '{}'
    t "$n (no Reticulum component): POST /nearby/hops → 409 component_unavailable, nothing spawned" 'problem_is 409 component_unavailable'
  done
  # (a) a card, pasted again
  jsend M2 POST /api/v1/contacts "$(jq -nc --arg c "$card_m1" '{card: $c}')"
  t "M2 pastes M1's card again → 200 (a card is no fact)" '[ "$ST" = 200 ]'
  # (b) the direct link comes back: hello and announce both ways on link-up, then keepalives (the link is attributed
  # only by a fresh sealed packet, so presence must see nothing of it)
  relays_up run2 || fail "the relays did not come back"
  # (c) a REAL replayed announce: a capture of the first run's link, replayed from a fresh socket at M2
  local cap
  cap=$(find "$G8_WORK/cap/run1/m2" -name '*.c2s' -size +100c 2>/dev/null | head -1)
  if [ -n "$cap" ]; then
    nx M1 python3 -I "$RELAY_PY" replay --capture "$cap" --to "${NIP[M2]}:$L7_PORT_FABRIC" --hold 4 --out "$W/replay.json" >/dev/null 2>&1 || true
    t "an announce captured on the first M1 → M2 link is replayed from a fresh socket (it sent $(jq -c .sent "$W/replay.json" 2>/dev/null))" \
      'has_json "$W/replay.json" && jq -e "(.sent | to_entries | map(select(.key | test(\"announce|^2$\"))) | length) > 0" "$W/replay.json" >/dev/null'
  else
    note "no capture of the first M1 → M2 link is on disk: the replayed-announce step is SKIPPED"
    [ "${REQUIRE_G8:-0}" = 1 ] && fail "REQUIRE_G8=1 and no capture of the first M1 → M2 link: the replayed announce could not be sent"
  fi
  # (d) keepalives: wait until a keepalive crossed the new link (the period is 25 s) and the hop read's window passed
  wait_for 60 'fc=$(frame_counts run2); [ "$(fcount keepalive "$fc")" -ge 1 ]' || true
  local fc
  fc=$(frame_counts run2)
  t "the link came back through the relays and its traffic is on the wire: $fc (a hello, an announce on link-up, a keepalive)" \
    '[ "$(fcount hello "$fc")" -ge 1 ] && [ "$(fcount announce "$fc")" -ge 1 ] && [ "$(fcount keepalive "$fc")" -ge 1 ]'
  wait_for 40 '[ "$(awk -v a="$hops_t0" -v b="${EPOCHREALTIME/,/.}" "BEGIN{print (b-a >= 31)}")" = 1 ]' || true
  r2=$(nrow M2 "${NID[M1]}")
  local el
  el=$(awk -v a="$tk0" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%d", b-a}')
  t "after a card re-paste, a link-up announce, a replayed announce and keepalives (${el} s) the row is untouched: heard_seq $seq0 → $(rowf "$r2" .heard_seq), evidence unchanged, heard_ago_s $age0 → $(rowf "$r2" .heard_ago_s) (kept growing, never reset)" \
    '[ "$(rowf "$r2" .heard_seq)" = "$seq0" ] && [ "$(printf "%s" "$r2" | jq -c .evidence)" = "$ev0" ] && [ "$(rowf "$r2" .heard_ago_s)" -ge $((age0 + el - 2)) ]'
  local want_ids
  want_ids=$(printf '%s\n' "${NID[H]}" "${NID[M1]}" | sort | tr '\n' ' ' | sed 's/ $//')
  t "the 111 source of M2 holds exactly M1 and the hub ($(nrows M2 | sed 's/[0-9a-f]\{58\}//g'))" '[ "$(nrows M2)" = "$want_ids" ]'
  jsend M1 POST /api/v1/nearby/hops '{}'
  t "the hop read, 31 s after the first: POST /nearby/hops → 202 again and a second rnpath ran ($(hops_calls) spawns in all)" \
    '[ "$ST" = 202 ] && wait_for 15 "[ \"\$(hops_calls)\" = 2 ]"'

  # ---------------------------------------------------------------- 7. the heartbeat codes
  until_phase 7 || return 0
  phase "7. the heartbeat codes, exact, and nothing written (I4), on M1 (the node holding the licensed board)"
  local rl0 rl1 stat0 tr
  rl0=$(sha "${NROOT[M1]}/state/relay.json")
  api M1 GET /api/v1/relay
  stat0=$(jq -c .presence "$W/body")
  for tr in tcp-lan kiss-modem meshtastic-client; do
    jsend M1 PATCH /api/v1/relay/settings "$(jq -nc --arg t "$tr" '{presence: {heartbeat: [{transport: $t, interval_min: 15}]}}')"
    t "PATCH presence.heartbeat[$tr] 15 → 409 no_such_transport" 'problem_is 409 no_such_transport'
  done
  jsend M1 PATCH /api/v1/relay/settings '{"presence": {"heartbeat": [{"transport": "tcp-lan", "interval_min": 0}]}}'
  t "PATCH presence.heartbeat[tcp-lan] 0 (events only) → 409 no_such_transport too (every write is refused in v1)" 'problem_is 409 no_such_transport'
  jsend M1 PATCH /api/v1/relay/settings '{"presence": {"heartbeat": [{"transport": "rns", "interval_min": 60}]}}'
  t "PATCH presence.heartbeat[rns] 60 → 409 settings_locked, the detail names PATCH /api/v1/bridge/settings ($(jget .detail | head -c 120))" \
    'problem_is 409 settings_locked && jb "(.detail | tostring) | contains(\"PATCH /api/v1/bridge/settings\")"'
  rl1=$(sha "${NROOT[M1]}/state/relay.json")
  api M1 GET /api/v1/relay
  t "nothing was written: state/relay.json is as before (${rl0:-absent} → ${rl1:-absent}) and the presence block of GET /relay is unchanged" \
    '[ "$rl0" = "$rl1" ] && [ "$(jq -c .presence "$W/body")" = "$stat0" ]'
  mkdir -p "${NROOT[M1]}/agent/proposals"
  jq -nc --arg n "${NID[M1]}" '{schema: 1, id: "prop-g8-presence", node_id: $n, kind: "relay", title: "beat", created_at: "2026-11-16T00:00:00Z",
    body: ({why: "keep the radar fresh", actions: [{action: "presence.interval", transport: "tcp-lan", minutes: 30}]} | tojson)}' >"${NROOT[M1]}/agent/proposals/prop-g8-presence.json"
  jsend M1 POST /api/v1/proposals/prop-g8-presence/decision '{"decision": "accepted"}'
  t "the presence.interval proposal action → 409 unsupported_action ($(jget .detail | head -c 100))" 'problem_is 409 unsupported_action'
  api M1 GET /api/v1/proposals/prop-g8-presence
  t "… the proposal stays open (decision null) and state/relay.json is still as before" \
    '[ "$ST" = 200 ] && jb ".decision == null" && [ "$(sha "${NROOT[M1]}/state/relay.json")" = "$rl0" ]'
  # the positive control of "nothing written": a permitted write does change the file
  jsend M1 PATCH /api/v1/relay/settings '{"updates": {"check_internet": false}}'
  t "positive control: a permitted write (updates.check_internet = false) → 200 and state/relay.json changes" \
    '[ "$ST" = 200 ] && [ "$(sha "${NROOT[M1]}/state/relay.json")" != "$rl0" ]'

  # ---------------------------------------------------------------- the helpers of the phases below
  dials() { # NODE SINCE → the unique addresses the node tried to reach since SINCE: every TCP SYN and every ARP who-has
    jq -r --argjson s "$2" 'select((.k == "syn" or .k == "arp") and .t >= $s) | .dst' "$G8_WORK/pkt-$1.jsonl" 2>/dev/null | sort -u | tr '\n' ' ' | sed 's/ $//'
  }
  syns() { # NODE SINCE → the unique "dst:port" of every TCP SYN the node sent since SINCE (epoch s), sorted
    jq -r --argjson s "$2" 'select(.k == "syn" and .t >= $s) | "\(.dst):\(.port)"' "$G8_WORK/pkt-$1.jsonl" 2>/dev/null | sort -u | tr '\n' ' ' | sed 's/ $//'
  }
  mq_count() { # NODE FROM TO REGEX → mDNS questions the node sent in [FROM, TO] whose name matches REGEX
    jq -r --argjson a "$2" --argjson b "$3" --arg re "$4" 'select(.k == "mdnsq" and .t >= $a and .t <= $b and (.name | test($re))) | .t' "$G8_WORK/pkt-$1.jsonl" 2>/dev/null | wc -l | tr -d ' '
  }
  mdns_in() { # NODE FROM TO → mDNS packets (queries and responses) that reached the node's eth0 in [FROM, TO]
    jq -r --argjson a "$2" --argjson b "$3" 'select(.k == "mdnsin" and .t >= $a and .t <= $b) | .t' "$G8_WORK/pkt-$1.jsonl" 2>/dev/null | wc -l | tr -d ' '
  }
  mq_last() { # NODE FROM REGEX → the time of the last such question since FROM (0 when none)
    jq -r --argjson a "$2" --arg re "$3" 'select(.k == "mdnsq" and .t >= $a and (.name | test($re))) | .t' "$G8_WORK/pkt-$1.jsonl" 2>/dev/null | tail -1 | awk '{ print ($1 == "" ? 0 : $1) }'
  }
  quiet_wait() { # MAX PID… → 0 when the mdns-sd threads of every PID woke at most once per 10 s sample, twice in a row, within MAX s; QW_T, QW_S set
    local max=$1 p cur busy ok2=0 t0 line
    shift
    local -A pv=()
    QW_S=""
    t0=$(now)
    for p in "$@"; do pv[$p]=$(wakes "$p"); done
    while [ "$(since "$t0" | cut -d. -f1)" -lt "$max" ]; do
      sleep 10
      busy=0
      line=""
      for p in "$@"; do
        cur=$(wakes "$p")
        line="$line $((cur - pv[$p]))"
        [ $((cur - pv[$p])) -gt 1 ] && busy=1
        pv[$p]=$cur
      done
      QW_S="$QW_S [$line ]"
      if [ "$busy" = 0 ]; then ok2=$((ok2 + 1)); else ok2=0; fi
      if [ "$ok2" -ge 2 ]; then
        QW_T=$(since "$t0")
        return 0
      fi
    done
    QW_T=$(since "$t0")
    return 1
  }
  rec_seen() { # NODE → 0 when NODE's _n111 record is on the LAN (probed from another node's namespace: a node's own mDNS answers do not loop back)
    local obs=H
    [ "$1" = H ] && obs=M2
    mdns "$obs" 3
    rec_has_k "${NID[$1]}"
  }
  rec_host_a() { # NODEID → the A records of the host the node's record points at, in the last probe
    local inst tgt
    inst=$(rec_of "$1" | jq -r .key 2>/dev/null)
    tgt=$(jq -r --arg i "$inst" '.srv[$i][1] // empty' "$W/mdns.json" 2>/dev/null)
    jq -r --arg h "$tgt" '(.a[$h] // []) | join(" ")' "$W/mdns.json" 2>/dev/null
  }
  card_of() { api "$1" GET /api/v1/contacts/me/card; jget .text; }
  rec_r() { printf '%s' "$(rec_of "$1")" | jq -r '[.value[] | select(startswith("r="))] | first // empty' 2>/dev/null; }
  wakes() { vcs_sum "$1" '^mDNS_daemon$'; } # PID → voluntary context switches of the mdns-sd thread
  cv() { jq -nc --arg net "$1" --arg acc "$2" --arg kind "$3" --arg nat "$4" --arg t "$5" '{Conversation: {conv: {network: $net, account: $acc, kind: $kind, native_id: $nat}, title: $t}}'; }
  sent_lines() { local f=$CTLD/$1.sent.jsonl; if [ -s "$f" ]; then wc -l <"$f" | tr -d ' '; else echo 0; fi; }

  # ---------------------------------------------------------------- 8. the mDNS record
  until_phase 8 || return 0
  phase "8. the _n111._tcp record: gated by lan_record, exactly v, r, k, A records only, no alias"
  jsend H PATCH /api/v1/settings '{"alias": "g8-secret-alias-zzz"}'
  [ "$ST" = 200 ] || note "H: PATCH /settings alias answered $ST"
  mdns M2 3 "$DASH_HOSTS"
  t "positive control of the probe: with lan_record off it still hears the dashboards' A records ($(rec_a)), but no _n111._tcp record of H, M1 or M2" \
    '[ -n "$(rec_a)" ] && ! rec_seen H && ! rec_seen M1 && ! rec_seen M2'
  jsend H PATCH /api/v1/relay/settings '{"discovery": {"lan_record": true}}'
  t "H: PATCH discovery.lan_record = true → 200 (no consent: it advertises a hint, not a role)" '[ "$ST" = 200 ]'
  jsend M2 PATCH /api/v1/relay/settings '{"discovery": {"lan_record": true}}'
  [ "$ST" = 200 ] || note "M2: PATCH discovery.lan_record answered $ST"
  tw 40 "H's record appears on the LAN (a probe from M2)" 'mdns M2 3; rec_has_k "${NID[H]}"'
  local rec txt hrec_port
  mdns M2 3
  rec=$(rec_of "${NID[H]}")
  txt=$(printf '%s' "$rec" | jq -c '.value | sort')
  t "the TXT is exactly v=1, r=<4 hex>, k=<H's NodeId> and nothing else ($txt)" \
    '[ "$(printf "%s" "$rec" | jq ".value | length")" = 3 ] && printf "%s" "$rec" | jq -e --arg k "k=${NID[H]}" ".value | (index(\"v=1\") != null) and (index(\$k) != null) and (map(select(test(\"^r=[0-9a-f]{4}\$\"))) | length == 1)" >/dev/null'
  hrec_port=$(srv_port)
  local htarget
  htarget=$(printf '%s' "$rec" | jq -r '.key')
  t "SRV: port $L7_PORT_FABRIC (the fabric listener), a host of the dashboard's mDNS name ($(srv_target)); A records only: no AAAA for it ($(jq -c '.aaaa' "$W/mdns.json"))" \
    '[ "$hrec_port" = "$L7_PORT_FABRIC" ] && [ -n "$(srv_target)" ] && [ "$(jq "[.aaaa | to_entries[] | select(.key == \"$(srv_target)\")] | length" "$W/mdns.json")" = 0 ] && [ -n "$(rec_a)" ]'
  t "no alias, load, battery, position or model in anything the probe heard: the alias set on H (g8-secret-alias-zzz) appears nowhere, the instance is the L2 label ($htarget)" \
    '! grep -q "g8-secret-alias-zzz" "$W/mdns.json" && ! printf "%s" "$htarget" | grep -qi "g8-secret"'
  t "H's record names the right host: its A records (from the same answer) include H's address ${NIP[H]}" \
    'jq -e --arg h "$(srv_target)" --arg ip "${NIP[H]}" ".a[\$h] // [] | index(\$ip) != null" "$W/mdns.json" >/dev/null'

  # ---------------------------------------------------------------- 9. the gateway row
  until_phase 9 || return 0
  phase "9. the gateway row walks its four codes: gateway_none, carrier_pending, carrier_ok, gateway_off (hub H, the stub ntfy server on H's loopback)"
  local ntfy="http://127.0.0.1:$L7_PORT_NTFY" gbody ghash GW_TOPIC=""
  gw() { api H GET /api/v1/services/gateway; GWC=$(jget .reason_code); }
  gw
  t "gateway_none: a hub with no gateway configured → state unset, reason_code gateway_none, detail.carrier null, pending 0 ($(jget -c '[.state, .reason_code, .detail]'))" \
    '[ "$GWC" = gateway_none ] && [ "$(jget .state)" = unset ] && jb ".detail.carrier == null and .detail.pending == 0 and .detail.error == null"'
  api H GET /api/v1/relay
  t "GET /relay agrees: gateway.state gateway_none, and the retired wan_* codes appear nowhere" \
    'jb ".gateway.state == \"gateway_none\"" && ! grep -q "wan_ok\|wan_unknown\|wan_offline" "$W/body"'
  gbody=$(jq -nc --arg s "$ntfy" '{gateway: {enabled: true}, carrier: {kind: "ntfy", server: $s, mode: "5A"}}')
  jsend H PATCH /api/v1/relay/settings "$gbody"
  t "H: carrier ntfy at the stub + gateway on, no consent → 409 consent_required (action carrier_enable; nothing written)" \
    'problem_is 409 consent_required && jb ".consent.action == \"carrier_enable\""'
  ghash=$(jget .consent.hash)
  gw
  t "… and the row is still gateway_none (a refused write changed nothing)" '[ "$GWC" = gateway_none ]'
  jsend H PATCH /api/v1/relay/settings "$(jq -c --arg h "$ghash" '. + {accept_warning: $h}' <<<"$gbody")"
  t "H: the same write with accept_warning = the hash → 200" '[ "$ST" = 200 ]'
  jsend H POST /api/v1/relay/carrier/check '{}'
  t "POST /relay/carrier/check → 202 started (the stub is down: nothing answers on $ntfy)" '[ "$ST" = 202 ] && jb ".started == true"'
  tw 20 "carrier_pending: the check failed → state unset, reason_code carrier_pending, detail.error set, checked_ago_s a number, carrier ntfy, mode 5A" \
    'gw; [ "$GWC" = carrier_pending ] && jb ".detail.error != null and (.detail.checked_ago_s | type) == \"number\" and .detail.carrier == \"ntfy\" and .detail.mode == \"5A\" and .state == \"unset\"" '
  nx H python3 -I "$STUB_NTFY" --port "$L7_PORT_NTFY" --quiet >"$G8_WORK/stub-ntfy.out" 2>&1 &
  STUBP=$!
  wait_for 8 'nx H curl -s -o /dev/null --max-time 1 "$ntfy/v1/health"' || true
  jsend H POST /api/v1/relay/carrier/check '{}'
  t "the stub is up; POST /relay/carrier/check → 202 started" '[ "$ST" = 202 ] && jb ".started == true"'
  tw 20 "carrier_ok: the carrier answered → state live, reason_code carrier_ok, error null, pending 0, checked_ago_s a number" \
    'gw; [ "$GWC" = carrier_ok ] && jb ".state == \"live\" and .detail.error == null and .detail.pending == 0 and (.detail.checked_ago_s | type) == \"number\""'
  jsend H PATCH /api/v1/relay/settings '{"gateway": {"enabled": false}}'
  t "H: gateway.enabled false → 200" '[ "$ST" = 200 ]'
  tw 15 "gateway_off: off by choice → state unset, reason_code gateway_off (the services row follows within its 2 s tick)" 'gw; [ "$GWC" = gateway_off ] && [ "$(jget .state)" = unset ]'
  api H GET /api/v1/relay
  t "GET /relay agrees: gateway.state gateway_off" 'jb ".gateway.state == \"gateway_off\""'
  jsend H PATCH /api/v1/relay/settings '{"gateway": {"enabled": true}}'
  t "H: gateway.enabled true again → 200 (the consent holds: nothing is asked twice)" '[ "$ST" = 200 ]'
  jsend H POST /api/v1/relay/carrier/check '{}'
  tw 20 "… and the row is carrier_ok again after a check (the four codes are four states of one row, not a one-way walk)" 'gw; [ "$GWC" = carrier_ok ]'

  # ---------------------------------------------------------------- 10. role bit 3
  until_phase 10 || return 0
  phase "10. role bit 3 (the gateway bit): a change re-mints the announce once, a repeat re-mints nothing"
  local c0 c1 c2 c3 st2 n0 n1 n2 n3
  ctr() { od -An -tu8 -j6 -N8 "${NROOT[H]}/identity/counters" 2>/dev/null | tr -d ' '; } # the fsynced announce counter of H (header: magic, version, clean flag, announce_last LE u64)
  c0=$(card_of H)
  n0=$(ctr)
  jsend H PATCH /api/v1/relay/settings '{"discovery": {"public_gateway_bit": true}}'
  t "H: discovery.public_gateway_bit true, no consent → 409 consent_required (action gateway_bit)" 'problem_is 409 consent_required && jb ".consent.action == \"gateway_bit\""'
  ghash=$(jget .consent.hash)
  t "a refused write re-mints nothing: H's card is unchanged and the announce counter is still ${n0:-?}" '[ "$(card_of H)" = "$c0" ] && [ -n "$n0" ] && [ "$(ctr)" = "$n0" ]'
  jsend H PATCH /api/v1/relay/settings "$(jq -nc --arg h "$ghash" '{discovery: {public_gateway_bit: true}, accept_warning: $h}')"
  t "H: the same write with accept_warning → 200" '[ "$ST" = 200 ]'
  c1=$(card_of H)
  n1=$(ctr)
  t "the role bit is announced: H's own card changed, and by exactly one announce (counter $n0 → $n1)" '[ -n "$c1" ] && [ "$c1" != "$c0" ] && [ -n "$n1" ] && [ "$n1" = "$((n0 + 1))" ]'
  jsend H PATCH /api/v1/relay/settings "$(jq -nc --arg h "$ghash" '{discovery: {public_gateway_bit: true}, accept_warning: $h}')"
  st2=$ST
  c2=$(card_of H)
  n2=$(ctr)
  t "the same write again (a repeat, nothing changed) → 200 and no re-mint: the card is byte-identical, the counter stays $n1 (now ${n2:-?})" '[ "$st2" = 200 ] && [ "$c2" = "$c1" ] && [ "$n2" = "$n1" ]'
  jsend H PATCH /api/v1/relay/settings '{"updates": {"check_internet": false}}'
  n3=$(ctr)
  t "an unrelated write (updates.check_internet) re-mints nothing either: the card is unchanged and the counter stays $n1 (now ${n3:-?})" '[ "$(card_of H)" = "$c1" ] && [ "$n3" = "$n1" ]'

  # ---------------------------------------------------------------- 11. the registry: browse, the record's gates
  until_phase 11 || return 0
  phase "11. the registry: no browse until asked, a browse stops within 5 s, the record's three gates, the role in the TXT"
  t "M2's record is up too (its lan_record is on, LAN sharing and a LAN-bound listener hold; a probe from H)" 'rec_seen M2'
  tw 45 "H's TXT r= follows the role bit: r=000b (mailbox, always-on hub, gateway) within one LAN refresh ($(rec_r "${NID[H]}"))" \
    'mdns M2 3; [ "$(rec_r "${NID[H]}")" = r=000b ]'
  local tb0 tb1 tb_cut tb_last q_before q_in q_after
  tb0=$(now)
  q_before=$(mq_count M1 0 "$tb0" '_n111\._tcp')
  t "no browse timer: M1 sent $q_before mDNS question(s) for _n111._tcp since it started, up to now" '[ "$q_before" = 0 ]'
  api M1 GET /api/v1/registry
  t "GET /registry before any browse is empty and started nothing ($(jget -c '.rows | length') rows)" 'jb ".rows | length == 0" && [ "$(mq_count M1 "$tb0" "$(now)" "_n111\\._tcp")" = 0 ]'
  jsend M1 POST /api/v1/registry/browse '{}'
  tb0=$(now)
  t "M1: POST /registry/browse → 202 started" '[ "$ST" = 202 ] && jb ".started == true"'
  tw 12 "the rows arrive: a hint row for H (address ${NIP[H]}:$L7_PORT_FABRIC, answered_by hint) and one for M2" \
    'api M1 GET /api/v1/registry; jb "([.rows[] | select(.node_id == \"${NID[H]}\" and .address == \"${NIP[H]}:$L7_PORT_FABRIC\" and (.answered_by | tostring | test(\"hint\")))] | length >= 1) and ([.rows[] | select(.node_id == \"${NID[M2]}\" and .address == \"${NIP[M2]}:$L7_PORT_FABRIC\")] | length >= 1)"'
  t "no alias, load, battery, position or model in a row; the labels are L2 labels ($(jget -c '[.rows[]?.label]' | head -c 160))" \
    '! grep -qi "g8-secret\|battery\|position\|model\|load" "$W/body"'
  sleep 8
  tb1=$(now)
  tb_cut=$(awk -v a="$tb0" 'BEGIN{printf "%.3f", a + 5.6}')
  q_in=$(mq_count M1 "$tb0" "$tb_cut" '_n111\._tcp')
  tb_last=$(mq_last M1 "$tb0" '_n111\._tcp')
  q_after=$(mq_count M1 "$tb_cut" "$tb1" '_n111\._tcp')
  local tb_gap
  tb_gap=$(awk -v a="$tb_cut" -v b="$tb1" 'BEGIN{printf "%.1f", b-a}')
  t "the browse ran and stopped: $q_in question(s) for _n111._tcp in its first 5.6 s, the last at +$(awk -v a="$tb0" -v b="$tb_last" 'BEGIN{printf "%.1f", b-a}') s, and $q_after in the next ${tb_gap}s (resolution: mdns-sd re-asks on a doubling schedule, 0, 1, 3, 7 s, so this window proves the stop before about +7 s; the exact 5 s is BROWSE_FOR, pinned by the registry lib test)" \
    '[ "$q_in" -ge 1 ] && [ "$q_after" = 0 ]'
  # the gates: one record, three inputs; M1 gets a record to lose, H loses lan_record, M1 its listener, M2 LAN sharing
  jsend M1 PATCH /api/v1/relay/settings '{"discovery": {"lan_record": true}}'
  tw 45 "M1: lan_record on (LAN sharing pinned, listen_lan true) → its record appears, TXT exactly v, r, k" \
    'rec_seen M1 && [ "$(rec_of "${NID[M1]}" | jq ".value | length")" = 3 ]'
  jsend H PATCH /api/v1/relay/settings '{"discovery": {"lan_record": false}}'
  t "H: discovery.lan_record false → 200 (gate 1: the switch)" '[ "$ST" = 200 ]'
  jsend M1 PATCH /api/v1/fabric/settings '{"listen_lan": false}'
  t "M1: PATCH /fabric/settings listen_lan false → 200 (gate 2: a LAN-bound fabric listener)" '[ "$ST" = 200 ]'
  jsend M2 PATCH /api/v1/settings '{"lan_enabled": false}'
  t "M2: PATCH /settings lan_enabled false → 200 (gate 3: LAN sharing, which owns the mDNS daemon; it takes effect on restart)" '[ "$ST" = 200 ]'
  stop_node M2
  start_node M2 LD_PRELOAD="$G8_SHIM"
  pM2=$(daemon_pid M2)
  tw 50 "all three records are gone from the LAN (H: switch off, M1: no LAN listener, M2: no LAN sharing)" \
    '! rec_seen H && ! rec_seen M1 && ! rec_seen M2'
  jsend M2 POST /api/v1/registry/browse '{}'
  t "M2 (LAN sharing off, no mDNS daemon): POST /registry/browse → 409 lan_off, nothing started" 'problem_is 409 lan_off'
  jsend H PATCH /api/v1/relay/settings '{"discovery": {"lan_record": true}}'
  jsend M1 PATCH /api/v1/fabric/settings '{"listen_lan": true}'
  jsend M2 PATCH /api/v1/settings '{"lan_enabled": true}'
  stop_node M2
  start_node M2 LD_PRELOAD="$G8_SHIM"
  pM2=$(daemon_pid M2)
  tw 60 "the positive control: with every gate restored all three records are back" \
    'rec_seen H && rec_seen M1 && rec_seen M2'
  # the role bit off again: one more re-mint
  c3=$(card_of H)
  n0=$(ctr)
  jsend H PATCH /api/v1/relay/settings '{"discovery": {"public_gateway_bit": false}}'
  n1=$(ctr)
  t "H: public_gateway_bit false → 200 and one re-mint: the card changed again and the announce counter rose by exactly one ($n0 → $n1)" '[ "$ST" = 200 ] && [ "$(card_of H)" != "$c3" ] && [ -n "$n0" ] && [ "$n1" = "$((n0 + 1))" ]'
  c3=$(card_of H)
  jsend H PATCH /api/v1/relay/settings '{"discovery": {"public_gateway_bit": false}}'
  t "… and writing false again re-mints nothing (card and counter $n1 unchanged)" '[ "$(card_of H)" = "$c3" ] && [ "$(ctr)" = "$n1" ]'

  # ---------------------------------------------------------------- 12. the mdns-sd re-arm
  until_phase 12 || return 0
  phase "12. the mdns-sd re-arm: the check rests at 0 once the address watch is bound, an address event moves the A records and rests again; the second bind failing keeps the 5 s check"
  mdns M1 3 "$DASH_HOSTS"
  local hH hM2 WIN=$(( ${G8_REARM_WIN:-30} < 20 ? 20 : ${G8_REARM_WIN:-30} )) EW=$(( ${G8_REARM_EW:-20} < 20 ? 20 : ${G8_REARM_EW:-20} )) NEW2=10.111.78.112
  hH=$(host_of "${NIP[H]}")
  hM2=$(host_of "${NIP[M2]}")
  t "the probe resolves H's and M2's dashboard hosts from their A records ($hH ${NIP[H]}, $hM2 ${NIP[M2]})" '[ -n "$hH" ] && [ -n "$hM2" ] && [ "$hH" != "$hM2" ]'
  local lg2
  lg2=$(grep -c 'mDNS keeps its 5 s address check (the address watch could not bind' "${NROOT[M2]}/daemon.log" 2>/dev/null)
  t "the log, once per start with LAN sharing: M2 (the bind refused) wrote \"mDNS keeps its 5 s address check (the address watch could not bind …)\" $lg2 time(s) in its ${STARTS[M2]} starts (one of them with LAN sharing off); H and M1 wrote none ($(grep -c 'mDNS keeps its 5 s' "${NROOT[H]}/daemon.log" 2>/dev/null), $(grep -c 'mDNS keeps its 5 s' "${NROOT[M1]}/daemon.log" 2>/dev/null))" \
    '[ "$lg2" -ge 1 ] && [ "$lg2" -lt "${STARTS[M2]}" ] && [ "$(grep -c "mDNS keeps its 5 s" "${NROOT[H]}/daemon.log")" = 0 ] && [ "$(grep -c "mDNS keeps its 5 s" "${NROOT[M1]}/daemon.log")" = 0 ]'
  local ah a1 a2 bh b1 b2 dh d1 d2
  local tw0 tw1 qw
  qw=0
  quiet_wait "${G8_SETTLE_MAX:-150}" "$pH" "$pM1" && qw=1
  note "settling: wakeups per 10 s sample (H M1):$QW_S, after ${QW_T} s"
  t "after the last registration and probe the mdns-sd threads of H and M1 settle to at most one wakeup per 10 s ($QW_T s, samples$QW_S)" '[ "$qw" = 1 ]'
  tw0=$(now)
  ah=$(wakes "$pH")
  a1=$(wakes "$pM1")
  a2=$(wakes "$pM2")
  sleep "$WIN"
  tw1=$(now)
  bh=$(wakes "$pH")
  b1=$(wakes "$pM1")
  b2=$(wakes "$pM2")
  dh=$((bh - ah))
  d1=$((b1 - a1))
  d2=$((b2 - a2))
  if [ "${G8_DIAG:-0}" = 1 ] && command -v strace >/dev/null 2>&1; then
    local th
    for th in /proc/"$pH"/task/*; do
      [ "$(cat "$th/comm" 2>/dev/null)" = mDNS_daemon ] && timeout 8 strace -tt -T -p "${th##*/}" 2>&1 | head -60 | sed 's/^/  diag H mDNS_daemon| /'
    done
  fi
  note "baseline window ($WIN s): mdns-sd thread wakeups H $dh, M1 $d1, M2 $d2"
  t "the check rests at 0 where the address watch bound: the mdns-sd thread of H woke $dh times and of M1 $d1 times in $WIN s (limit $((WIN / 10)); a 5 s check is about $((WIN / 5)))" \
    '[ "$dh" -le $((WIN / 10)) ] && [ "$d1" -le $((WIN / 10)) ]'
  t "…and with the second bind failing the 5 s check stays: M2's mdns-sd thread woke $d2 times in the same $WIN s (at least $((WIN / 5 - 1)))" '[ "$d2" -ge $((WIN / 5 - 1)) ]'
  # one address event on H (bound) and on M2 (refused), at once
  local te tH="" tM="" i
  te=$(now)
  # a renumbering (the old address goes, the new one comes: what a DHCP change does), not a second address on the interface: two
  # addresses of one subnet on one interface make two mdns-sd daemons answer each other's probes (observed: a name-conflict ping-pong
  # that keeps both threads at 25 to 40 wakeups a second; see the report)
  nx H sh -c "ip addr del ${NIP[H]}/24 dev eth0; ip addr add $NEW_IP/24 dev eth0"
  nx M2 sh -c "ip addr del ${NIP[M2]}/24 dev eth0; ip addr add $NEW2/24 dev eth0"
  for i in $(seq 1 8); do
    mdns M1 2
    ah=$(rec_host_a "${NID[H]}")
    a2=$(rec_host_a "${NID[M2]}")
    if [ -z "$tH" ] && [[ " $ah " == *" $NEW_IP "* ]] && [[ " $ah " != *" ${NIP[H]} "* ]]; then tH=$(since "$te"); fi
    if [ -z "$tM" ] && [[ " $a2 " == *" $NEW2 "* ]] && [[ " $a2 " != *" ${NIP[M2]} "* ]]; then tM=$(since "$te"); fi
    [ -n "$tH" ] && [ -n "$tM" ] && break
  done
  t "H: an address event (renumbered ${NIP[H]} → $NEW_IP) → the A records of H's record changed after ${tH:-never} s (at most 8: the 2 s debounce, the 1 s check, the 1.5 s one-shot sleep)" '[ -n "$tH" ] && awk -v x="$tH" "BEGIN{exit !(x <= 8)}"'
  t "M2 (second bind failing): ${NIP[M2]} → $NEW2 → the A records of M2's record changed after ${tM:-never} s (the 5 s check: at most 9)" '[ -n "$tM" ] && awk -v x="$tM" "BEGIN{exit !(x <= 9)}"'
  if [ "${G8_DIAG:-0}" = 1 ] && command -v strace >/dev/null 2>&1; then
    local th
    sleep 4
    for th in /proc/"$pH"/task/*; do
      [ "$(cat "$th/comm" 2>/dev/null)" = mDNS_daemon ] && timeout 4 strace -tt -T -p "${th##*/}" 2>&1 | head -50 | sed 's/^/  diag H after event| /'
    done
    tail -5 "${NROOT[H]}/daemon.log" | sed 's/^/  diag H log| /'
  fi
  qw=0
  quiet_wait 60 "$pH" && qw=1
  t "after the event H's check is back at 0: its mdns-sd thread is quiet again (at most one wakeup per 10 s, twice) within 60 s of the event, after $QW_T s (samples$QW_S; a check left at 1 s would wake about 10 times per sample)" '[ "$qw" = 1 ]'
  a2=$(wakes "$pM2")
  ah=$(wakes "$pH")
  sleep "$EW"
  dh=$(($(wakes "$pH") - ah))
  d2=$(($(wakes "$pM2") - a2))
  t "…and it stays at rest: $dh wakeups in the next $EW s (limit $((EW / 10 + 1)))" '[ "$dh" -le $((EW / 10 + 1)) ]'
  t "…and M2 keeps waking ($d2 times in $EW s, at least $((EW / 5 - 1)))" '[ "$d2" -ge $((EW / 5 - 1)) ]'
  # the second event: the addresses go again (every event re-arms, one at a time)
  te=$(now)
  nx H sh -c "ip addr del $NEW_IP/24 dev eth0; ip addr add ${NIP[H]}/24 dev eth0"
  nx M2 sh -c "ip addr del $NEW2/24 dev eth0; ip addr add ${NIP[M2]}/24 dev eth0"
  tH=""
  tM=""
  for i in $(seq 1 8); do
    mdns M1 2
    ah=$(rec_host_a "${NID[H]}")
    a2=$(rec_host_a "${NID[M2]}")
    if [ -z "$tH" ] && [[ " $ah " != *" $NEW_IP "* ]] && [[ " $ah " == *" ${NIP[H]} "* ]]; then tH=$(since "$te"); fi
    if [ -z "$tM" ] && [[ " $a2 " != *" $NEW2 "* ]] && [[ " $a2 " == *" ${NIP[M2]} "* ]]; then tM=$(since "$te"); fi
    [ -n "$tH" ] && [ -n "$tM" ] && break
  done
  t "the second event (renumbered back): the A records of H's record hold ${NIP[H]} again after ${tH:-never} s, M2's hold ${NIP[M2]} after ${tM:-never} s (at most 8 and 9)" '[ -n "$tH" ] && [ -n "$tM" ] && awk -v x="$tH" -v y="$tM" "BEGIN{exit !(x <= 8 && y <= 9)}"'
  qw=0
  quiet_wait 60 "$pH" && qw=1
  t "…and H rests again after the second event, after $QW_T s (samples$QW_S): every event re-arms and then returns to 0, one at a time" '[ "$qw" = 1 ]'
  t "the log line is once per start, not per event: M2 still $lg2 line(s) (it was $lg2 before the events: $(grep -c 'mDNS keeps its 5 s' "${NROOT[M2]}/daemon.log")), H and M1 none" \
    '[ "$(grep -c "mDNS keeps its 5 s" "${NROOT[M2]}/daemon.log")" = "$lg2" ] && [ "$(grep -c "mDNS keeps its 5 s" "${NROOT[H]}/daemon.log")" = 0 ]'

  tw 180 "the renumbering is over: M1 and M2 have their hub back at ${NIP[H]} (reachable) before the next phase (they redial with a back-off)" \
    'api M1 GET /api/v1/fabric; jb ".hub.reachable == true" && api M2 GET /api/v1/fabric && jb ".hub.reachable == true"'

  # ---------------------------------------------------------------- 13. the licensed board
  until_phase 13 || return 0
  phase "13. with NODE111_BRIDGE_STUBS reporting a licensed board (stub-mesh on M1), no Layer 7 path sends to it"
  local fcg fcm sa0 sm0 sl0 cbody chash mid13
  mesh_facts() { # LICENSED(true|false) → stub-mesh's facts (the scenario's, with the live licensed flag)
    printf '{"accounts":[{"id":"!a0000001","label":"licensed board","licensed":%s}],"channels":[{"account":"!a0000001","native_id":"ch:up","uplink":false,"default_key":false}],"region_set":true,"ids_globally_unique":false,"parts_may_reorder":true,"licensed_caps":{"channel":232,"dm":232,"group":null}}' "$1"
  }
  if ctl "$(jq -nc --argjson i "[$(cv stub-app app-a group grp:g8 'g8 control')]" '{op: "feed", stub: "stub-app", items: $i}')" &&
    ctl "$(jq -nc --argjson i "[$(cv stub-mesh '!a0000001' channel ch:up 'g8 uplink channel')]" '{op: "feed", stub: "stub-mesh", items: $i}')"; then
    ok "the scenario hook took two feeds: a conversation on stub-app and one on the licensed stub-mesh"
  else
    fail "the scenario hook did not answer a feed ($CTL)"
  fi
  fcg=$(fcid stub-app grp:g8)
  fcm=$(fcid stub-mesh ch:up)
  jsend M1 POST "/api/v1/networks/stub-app/conversations/${fcg:-none}/messages" '{"text": "g8 control app", "size_mode": "refuse"}'
  tw 15 "positive control 1: a Layer 6 send to the unlicensed stub-app reaches that stub's radio log (its counter reads 1)" '[ "$(sent_lines stub-app)" = 1 ]'
  # positive control 2: the same stub-mesh log does record a send when the board is not licensed (so an empty log below means something)
  ctl "$(jq -nc --argjson f "$(mesh_facts false)" '{op: "facts", stub: "stub-mesh", facts: $f}')" || fail "the hook did not take the unlicensed facts ($CTL)"
  jsend M1 POST "/api/v1/networks/stub-mesh/conversations/${fcm:-none}/messages" '{"text": "g8 control mesh", "size_mode": "refuse"}'
  tw 15 "positive control 2: with the board reporting unlicensed (a live facts command) a Layer 6 send reaches stub-mesh's log (reads 1)" '[ "$(sent_lines stub-mesh)" = 1 ]'
  ctl "$(jq -nc --argjson f "$(mesh_facts true)" '{op: "facts", stub: "stub-mesh", facts: $f}')" || fail "the hook did not take the licensed facts back ($CTL)"
  sa0=$(sent_lines stub-app)
  sm0=$(sent_lines stub-mesh)
  sl0=$(sent_lines lxmf)
  # now every Layer 7 path on M1, the board licensed: M1 becomes a hub with a gateway and an ntfy carrier, checks and flushes, sends and
  # receives 111 messages (direct and through the hub), reads Nearby, the registry and the hops
  jsend M1 PATCH /api/v1/fabric/settings '{"hub_role": true}'
  t "M1: PATCH /fabric/settings hub_role true → 200 (a hub, so a carrier can be built on it)" '[ "$ST" = 200 ]'
  cbody='{"gateway": {"enabled": true}, "carrier": {"kind": "ntfy", "server": "http://10.111.78.10:11280", "mode": "5A"}}'
  jsend M1 PATCH /api/v1/relay/settings "$cbody"
  chash=$(jget .consent.hash)
  if [ "$ST" = 409 ] && [ -n "$chash" ]; then
    jsend M1 PATCH /api/v1/relay/settings "$(jq -c --arg h "$chash" '. + {accept_warning: $h}' <<<"$cbody")"
  fi
  t "M1 (the licensed board still attached) takes the gateway and ntfy carrier settings with their consent → 200" '[ "$ST" = 200 ]'
  jsend M1 POST /api/v1/relay/carrier/check '{}'
  t "M1: POST /relay/carrier/check → 202 started (the daemon's licensed rule is structural: the carrier rides IP, never a board: N/carrier/mod.rs RuntimeOptions::licensed)" '[ "$ST" = 202 ] && jb ".started == true"'
  jsend M1 POST /api/v1/relay/carrier/flush '{}'
  t "M1: POST /relay/carrier/flush → 202" '[ "$ST" = 202 ]'
  send M1 "${NID[M2]}" "g8 message while a licensed board is attached"
  mid13=$MID
  tw 40 "M1 → M2: a 111 message is delivered (a presence fact is made, a Layer 7 path is live)" 'delivered M1 "${NID[M2]}" "$mid13"'
  nearby M1
  api M1 GET /api/v1/relay
  api M1 GET /api/v1/registry
  jsend M1 POST /api/v1/registry/browse '{}'
  sleep 8
  t "none of it reached a board: stub-mesh's log $sm0 → $(sent_lines stub-mesh), the lxmf stub's $sl0 → $(sent_lines lxmf), stub-app's $sa0 → $(sent_lines stub-app) (nothing of Layer 7 rides a radio)" \
    '[ "$(sent_lines stub-mesh)" = "$sm0" ] && [ "$(sent_lines lxmf)" = "$sl0" ] && [ "$(sent_lines stub-app)" = "$sa0" ]'
  note "the heartbeat validator's licensed_board and 9 min bans are the node111 unit tests of the last step; in v1 no transport takes a heartbeat (phase 7), so no presence beat exists to send"
  jsend M1 PATCH /api/v1/relay/settings '{"gateway": {"enabled": false}, "carrier": {"kind": "none"}}'
  jsend M1 PATCH /api/v1/fabric/settings '{"hub_role": false}'

  # ---------------------------------------------------------------- 14. lost-hub recovery
  until_phase 14 || return 0
  phase "14. lost-hub recovery: H moves to a new address; M1's \"Find my hub\" dials only what it was given (an outbound SYN and ARP log of M1 and of M2)"
  api M1 GET /api/v1/fabric
  t "before: M1 is enrolled at H (${NIP[H]}:$L7_PORT_FABRIC) and reachable; M2 the same" \
    '[ "$(jget .hub.address)" = "${NIP[H]}:$L7_PORT_FABRIC" ] && jb ".hub.reachable == true" && api M2 GET /api/v1/fabric && jb ".hub.reachable == true"'
  local hcond tl0 dm1 dm2 allowed1 allowed2 odd x sy1
  # stale neighbour entries of the earlier renumberings (.111, .112) would be revalidated by the kernel with an ARP who-has that no daemon
  # asked for: drop them, so every ARP in the log below is a dial
  nx M1 ip neigh flush dev eth0
  nx M2 ip neigh flush dev eth0
  tl0=$(now)
  stop_node H
  nx H ip addr del "${NIP[H]}/24" dev eth0
  nx H ip addr add "$MOVED_IP/24" dev eth0
  start_node H
  pH=$(daemon_pid H)
  note "H is back at $MOVED_IP (its LAN sharing, hub role and record switch are from its state); the members still hold ${NIP[H]}"
  note "H's record before the check (probe from M2): $(rec_seen H && rec_of "${NID[H]}" | head -c 300) A=$(rec_host_a "${NID[H]}")"
  hcond='rec_seen H && [[ " $(rec_host_a "${NID[H]}") " == *" $MOVED_IP "* ]] && [[ " $(rec_host_a "${NID[H]}") " != *" ${NIP[H]} "* ]]'
  wait_for 40 "$hcond" || true
  note "H's record at the end of the wait: $(rec_of "${NID[H]}" | head -c 200) A=$(rec_host_a "${NID[H]}") srv=$(srv_target) all-A=$(rec_a)"
  t "H's record is on the LAN at the new address: the A records of the host its SRV names hold $MOVED_IP and not ${NIP[H]}" "$hcond"
  tw 20 "M1 lost the hub: hub.reachable false while the address it holds is dead" 'api M1 GET /api/v1/fabric; jb ".hub.reachable == false"'
  jsend M1 POST /api/v1/registry/browse '{}'
  t "M1: \"Find my hub\" = POST /registry/browse → 202 (no new route)" '[ "$ST" = 202 ]'
  tw 40 "M1 has the hub back at the found address: hub.address $MOVED_IP:$L7_PORT_FABRIC, reachable (the address is kept only after the handshake named the hub) ($(jget -c '[.hub.address, .hub.reachable]'))" \
    'api M1 GET /api/v1/fabric; [ "$(jget .hub.address)" = "$MOVED_IP:$L7_PORT_FABRIC" ] && jb ".hub.reachable == true and .hub.node_id == \"${NID[H]}\""'
  api M2 GET /api/v1/fabric
  t "the control: M2 never browsed and still holds the dead address ${NIP[H]}:$L7_PORT_FABRIC, hub unreachable ($(jget -c '[.hub.address, .hub.reachable]'))" \
    '[ "$(jget .hub.address)" = "${NIP[H]}:$L7_PORT_FABRIC" ] && jb ".hub.reachable == false"'
  dm1=$(dials M1 "$tl0")
  dm2=$(dials M2 "$tl0")
  allowed1="${NIP[H]} $MOVED_IP ${NIP[M2]}"
  allowed2="${NIP[H]} ${NIP[M1]}"
  odd=""
  for x in $dm1; do [[ " $allowed1 " == *" $x "* ]] || odd="$odd M1→$x"; done
  for x in $dm2; do [[ " $allowed2 " == *" $x "* ]] || odd="$odd M2→$x"; done
  t "egress: M1 reached out only to addresses it was given — ${NIP[H]} (its enrolment), $MOVED_IP (the DNS-SD record of its hub) and ${NIP[M2]} (its peer): [$dm1]; M2 only to ${NIP[H]} and ${NIP[M1]}: [$dm2]; anything else: [${odd# }]" \
    '[ -z "$odd" ] && [[ " $dm1 " == *" $MOVED_IP "* ]] && [[ " $dm1 " == *" ${NIP[H]} "* ]] && [[ " $dm2 " == *" ${NIP[H]} "* ]]'
  sy1=$(syns M1 "$tl0")
  odd=""
  for x in $sy1; do [[ " ${NIP[H]}:$L7_PORT_FABRIC $MOVED_IP:$L7_PORT_FABRIC ${NIP[M2]}:$RELAY_PORT " == *" $x "* ]] || odd="$odd $x"; done
  t "egress by port: M1's TCP SYNs went only to H's two addresses on $L7_PORT_FABRIC and its peer ${NIP[M2]}:$RELAY_PORT, and not to the registry hit of M2 (${NIP[M2]}:$L7_PORT_FABRIC): [$sy1], anything else: [${odd# }]" \
    '[ -z "$odd" ] && [[ " $sy1 " == *" $MOVED_IP:$L7_PORT_FABRIC "* ]]'
  t "…no sweep of the /24: $(printf '%s\n' $dm1 | wc -l) distinct addresses for M1, $(printf '%s\n' $dm2 | wc -l) for M2" '[ "$(printf "%s\n" $dm1 | wc -l)" -le 3 ] && [ "$(printf "%s\n" $dm2 | wc -l)" -le 2 ]'
  # the negative control of the egress check: one deliberate dial to an address nobody was given is caught
  local tn
  tn=$(now)
  nx M1 timeout 2 bash -c 'exec 3<>/dev/tcp/10.111.78.77/11183' 2>/dev/null || true
  sleep 1
  t "negative control: a deliberate dial of 10.111.78.77 from M1 is seen by the egress log ($(dials M1 "$tn")) and flagged as not given" \
    '[[ " $(dials M1 "$tn") " == *" 10.111.78.77 "* ]] && [[ " $allowed1 " != *" 10.111.78.77 "* ]]'
  t "and nothing reached it during the lost-hub round either: stub-mesh's log still $(sent_lines stub-mesh) (it read $sm0 after the control), the lxmf stub's $(sent_lines lxmf)" '[ "$(sent_lines stub-mesh)" = "$sm0" ] && [ "$(sent_lines lxmf)" = 0 ]'
}

# ------------------------------------------------------------------ outer
for tool in python3 ip unshare nsenter jq curl awk sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || skip_or_fail G8 "$tool is absent"
done
have_netns || skip_or_fail G8 "unprivileged network namespaces are not available (unshare -Urnm)"
unshare --user --map-root-user --net --pid --fork --mount-proc true >/dev/null 2>&1 ||
  skip_or_fail G8 "a pid namespace with its own /proc is not available (unshare --pid --fork --mount-proc)"
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
if [ -z "$BIN111" ] || [ ! -x "$BIN111" ]; then skip_or_fail G8 "no built 111 binary (set BIN111 or CARGO_TARGET_DIR)"; fi
BIN111=$(cd "$(dirname "$BIN111")" && pwd)/$(basename "$BIN111")
[ -r "$RELAY_PY" ] || skip_or_fail G8 "missing $RELAY_PY"
[ -r "$STUB_NTFY" ] || skip_or_fail G8 "missing $STUB_NTFY"

if [ "${1:-}" = --inner ]; then
  inner
  exit 0
fi

if [ "${REQUIRE_G8:-0}" = 1 ]; then
  for k in G8_UNTIL G8_SKIP_CARGO G8_REARM_WIN G8_REARM_EW G8_SETTLE_MAX G8_DIAG; do
    if [ -n "${!k:-}" ]; then
      printf 'FAIL: REQUIRE_G8=1 refuses the dev knob %s=%s (it drops part of the gate)\n' "$k" "${!k}"
      echo "G8: 1 FAIL"
      exit 1
    fi
  done
fi
WORK=$(mktemp -d "${TMPDIR:-/tmp}/l7-g8.XXXXXX") || exit 1
cleanup() {
  if [ "${G8_KEEP:-0}" = 1 ]; then note "kept $WORK"; else rm -rf "${WORK:?}"; fi
}
trap cleanup EXIT

# ---- the roots (identities are made here, outside the namespaces: nothing of the daemon runs yet)
for n in "${NODES[@]}"; do
  g6_make_root "$WORK/$n" ""
  nid=$(g6_identity "$WORK/$n" "$BIN111")
  if [ -n "$nid" ]; then
    printf '%s\n' "$nid" >"$WORK/$n.nid"
    ok "$n: identity generated ($nid)"
  else
    fail "$n: 111 identity generate ($(head -c 200 "$WORK/$n/identity.log"))"
  fi
done
NID_H=$(cat "$WORK/H.nid" 2>/dev/null || true)
NID_M1=$(cat "$WORK/M1.nid" 2>/dev/null || true)
NID_M2=$(cat "$WORK/M2.nid" 2>/dev/null || true)
g6_fabric_settings "$WORK/H" '{"schema": 1, "hub_role": true, "listen_lan": true}'
g6_fabric_settings "$WORK/M1" "{\"schema\": 1, \"listen_lan\": true, \"peers\": [{\"node_id\": \"$NID_M2\", \"address\": \"$L7_G8_IP_M2:11184\"}]}"
g6_fabric_settings "$WORK/M2" "{\"schema\": 1, \"listen_lan\": true, \"peers\": [{\"node_id\": \"$NID_M1\", \"address\": \"$L7_G8_IP_M1:11184\"}]}"
# LAN sharing: H and M1 through the env file's pin, M2 through the setting (`111 config dashboard.lan true`)
printf '# G8: LAN sharing pinned on\nDASHBOARD_LAN=true\n' >"$WORK/H/etc/env"
printf '# G8: LAN sharing pinned on\nDASHBOARD_LAN=true\n' >"$WORK/M1/etc/env"
NODE111_ROOT=$WORK/M2 "$BIN111" config dashboard.lan true >"$WORK/M2-lan.log" 2>&1 || fail "M2: 111 config dashboard.lan true ($(head -c 200 "$WORK/M2-lan.log"))"
write_fake_rns "$WORK/M1"
mkdir -p "$WORK/M1/g8-ctl"
cat >"$WORK/M1/g8-scenario.json" <<EOF
{"control": "$WORK/M1/g8-ctl", "node": "m1",
 "stubs": [
  {"id":"lxmf","label":"LXMF stub","tag":"SL","kinds":["dm","group"],"max_text_bytes":{"channel":null,"dm":287,"group":287},"recipient_syntax":{"pattern":"^[0-9a-f]{32}$","hint":"a 32-hex destination"},
   "facts":{"accounts":[{"id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","label":null,"licensed":false}],"channels":[],"region_set":null,"ids_globally_unique":true,"parts_may_reorder":false,"licensed_caps":null}},
  {"id":"stub-mesh","label":"Licensed mesh board","tag":"SM","kinds":["channel","dm"],"max_text_bytes":{"channel":232,"dm":220,"group":null},"airtime":{"duty_cycle_permille":10,"bits_per_s":1000,"preset":"LongFast"},
   "facts":{"accounts":[{"id":"!a0000001","label":"licensed board","licensed":true}],"channels":[{"account":"!a0000001","native_id":"ch:up","uplink":false,"default_key":false}],"region_set":true,"ids_globally_unique":false,"parts_may_reorder":true,"licensed_caps":{"channel":232,"dm":232,"group":null}}},
  {"id":"stub-app","label":"App stub","tag":"SA","kinds":["dm","group"],"max_text_bytes":{"channel":null,"dm":4000,"group":120},
   "facts":{"accounts":[{"id":"app-a","label":"bridge account","licensed":false}],"channels":[],"region_set":true,"ids_globally_unique":true,"parts_may_reorder":false,"licensed_caps":null}}]}
EOF
if jq -e . "$WORK/M1/g8-scenario.json" >/dev/null 2>&1; then
  ok "M1 scenario: stub-mesh with a licensed board, stub-app, and a stub named lxmf"
else
  fail "the M1 scenario is not JSON"
fi

# ---- the bind shim for M2 (gcc; without it the second-bind-failing item is SKIPPED)
G8_SHIM=""
if command -v gcc >/dev/null 2>&1; then
  write_shim "$WORK/bindshim.c"
  if gcc -shared -fPIC -o "$WORK/bindshim.so" "$WORK/bindshim.c" -ldl 2>"$WORK/gcc.err"; then
    G8_SHIM=$WORK/bindshim.so
    ok "the bind shim is built (bind() of a route-netlink socket on groups 0x551 fails with EPERM, for M2 only)"
  else
    note "the bind shim did not compile ($(head -c 200 "$WORK/gcc.err")): the second-bind-failing item is SKIPPED"
    [ "${REQUIRE_G8:-0}" = 1 ] && fail "REQUIRE_G8=1 and the bind shim did not compile"
  fi
else
  note "gcc is absent: the second-bind-failing item is SKIPPED"
  [ "${REQUIRE_G8:-0}" = 1 ] && fail "REQUIRE_G8=1 and gcc is absent: the second-bind-failing item could not run"
fi

if env --default-signal true 2>/dev/null; then
  RESET_SIG=(env --default-signal)
else
  RESET_SIG=(python3 -c 'import os,signal,sys
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])')
fi
INNER_LOG=$WORK/inner.log
: >"$INNER_LOG"
G8_WORK=$WORK BIN111=$BIN111 G8_SHIM=$G8_SHIM \
  unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
  "${RESET_SIG[@]}" bash "$0" --inner >"$INNER_LOG" 2>&1 &
NS_PID=$!
trap 'kill -TERM "$NS_PID" 2>/dev/null; wait "$NS_PID" 2>/dev/null; cleanup; exit 130' INT TERM HUP
tail -n +1 -f --pid="$NS_PID" "$INNER_LOG" &
TAIL_PID=$!
wait "$NS_PID" || true
wait "$TAIL_PID" 2>/dev/null || true
trap cleanup EXIT
inner_fails=$(grep -c '^FAIL:' "$INNER_LOG" || true)
if grep -q 'command not found' "$INNER_LOG"; then
  fail "the inner run hit a command that does not exist (a check that cannot fail): $(grep -m3 'command not found' "$INNER_LOG" | tr '\n' ' ')"
fi
if ! grep -q '^FAILS=' "$WORK/inner.result" 2>/dev/null; then
  fail "the namespace run did not finish (killed or crashed, see above)"
fi
FAILS=$((FAILS + inner_fails))

# ---- the validator unit tests (D2: the bans and the refused heartbeat writes); a test build, a full compile on first use
echo
echo "== 15. the validator unit tests: cargo test -p node111 --lib (relay::presence, and the two mDNS re-arm tests)"
if [ "${G8_SKIP_CARGO:-0}" = 1 ]; then
  note "G8_SKIP_CARGO=1: the unit tests are SKIPPED (the licensed_board and 9 min bans, the refused heartbeat writes, the re-arm interval)"
elif ! command -v cargo >/dev/null 2>&1; then
  note "cargo is absent: the unit tests are SKIPPED"
  [ "${REQUIRE_G8:-0}" = 1 ] && fail "REQUIRE_G8=1 and cargo is absent: the unit tests could not run"
else
  CARGO_LOG=$WORK/cargo.log
  MT=dashboard::mdns::tests
  cargo_rc=0
  (cd 1-bootstrap && env -u TMPDIR CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test -p node111 --lib -- --nocapture relay::presence \
    $MT::the_real_daemon_reads_zero_after_the_bind_and_after_each_event $MT::a_real_address_change_changes_the_a_records_through_the_re_arm) >"$CARGO_LOG" 2>&1 || cargo_rc=$?
  for tn in relay::presence::tests::the_bans_apply_after_the_transport_check relay::presence::tests::every_v1_heartbeat_write_is_refused_by_transport \
    $MT::the_real_daemon_reads_zero_after_the_bind_and_after_each_event $MT::a_real_address_change_changes_the_a_records_through_the_re_arm; do
    if grep -q "$tn ... ok" "$CARGO_LOG"; then
      ok "unit test $tn passes"
    else
      fail "unit test $tn did not pass ($(grep -E '^test result|error' "$CARGO_LOG" | head -2 | tr '\n' ' '))"
    fi
  done
  if grep -q '^SKIPPED' "$CARGO_LOG"; then
    note "a re-arm lib test printed SKIPPED: $(grep -m2 '^SKIPPED' "$CARGO_LOG" | tr '\n' ' ')"
    [ "${REQUIRE_G8:-0}" = 1 ] && fail "REQUIRE_G8=1 and a re-arm lib test skipped itself (the box lacks mDNS or a network namespace)"
  fi
  if [ "$cargo_rc" = 0 ] && grep -qE '^test result: ok\.' "$CARGO_LOG"; then
    ok "cargo test exit 0: $(grep -E '^test result' "$CARGO_LOG" | head -1)"
  else
    fail "cargo test exit $cargo_rc: $(grep -E '^test result|^failures:|FAILED' "$CARGO_LOG" | head -4 | tr '\n' ' ')"
  fi
fi

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
if [ -z "$left" ]; then ok "no daemon, relay, probe or logger of this run is left"; else fail "left running:$left"; fi
finish G8
