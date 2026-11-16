#!/usr/bin/env bash
# 7-relay/verify/forward.sh — Layer 7 gate G9 (spec §15 G9; §5; row V3): cross-hub custody forward.
#
#   bash 7-relay/verify/forward.sh              runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G9=1 bash 7-relay/verify/forward.sh lock run: a missing prerequisite is a FAIL
#
# Real `111 daemon`s, each in a network namespace of its own (and its own NODE111_ROOT), inside one user + net + pid
# namespace (`unshare -rnpf`, so everything dies with it):
#   A .11, B .12, C .13 on TEST-NET-1 192.0.2.0/24 (bridge l7g9wan): hubs; RFC 5737 is not private, so §4.1 rule 1
#   puts every hub-to-hub link in TLS 1.3 raw public keys
#   M, M2 members of A on A's LAN 10.111.79.0/24 (A .1); D, D2 members of B on 10.111.89.0/24 (B .1)
#   E a node no hub holds (no link at all): the destination nobody can find
# Everything is driven through each node's loopback /api/v1 (curl via nsenter). The wire is read by one AF_PACKET tap
# on the host end of every veth (each frame is read once, at its sender's end): the hint TLV's 11 bytes show as the
# length of the one sealed frame a member puts to its hub, the probes as application-data records of put size on a
# TLS flow, a plaintext 111F hello on a TEST-NET flow as the leak. A daemon's frames to a member carry no secret and
# are listed as (type, length) pairs: the tape of item 1.
#
# What each item proves, and how a broken daemon turns it FAIL, is written beside the item.
#
# Items (G9_ITEMS="1 2 3 4 5 6 7" selects some in a dev run; "6o" is item 6's old-hub part alone; the lock run is all).
# Item 6's old hub B is a Layer 6 build exported from G9_OLD_REV (default checkpoint-layer-6; the tag is absent from some
# clones: the lead sets G9_OLD_REV=561cb5e, the last commit before the Layer 7 scaffold) and built once with its own
# CARGO_TARGET_DIR; without it that part is SKIPPED (a FAIL under REQUIRE_G9=1).
#
# Env: BIN111 (the `111` binary; else $CARGO_TARGET_DIR/{release,debug}/111) · G9_OLD_REV · G9_OLD_BIN (a ready-built old
# `111`: no build) · G9_OLD_SRC (an exported tree of that revision, reused) · G9_OLD_TARGET (the old build's cargo target
# dir, default under the scratch root; a binary of the same revision found there is reused) · G9_OLD_JOBS (2) · G9_SETTLE
# (3 s: how long an absent event is watched) · G9_WAIT (60 s: the longest wait for a redial after a link went away; the
# daemon's back-off is 0.5 to 60 s) · G9_WINDOW (60 s: the idle window of the records; 0 skips them) · G9_WRITE_TAPE=1
# (item 1 writes the tape from this run) · G9_NEG=nopair (a negative control: no pairing is made, the pair items FAIL) · G9_KEEP=1 keeps the scratch root · G9_PAUSE=1 (a dev aid: after the pairing the
# run evals $W/cmd.<n> files until $W/cmd.quit) · TMPDIR for the scratch root. Nothing of ours stays running.
# the jq filters and the wait_for conditions are single-quoted on purpose (jq variables, eval'd conditions)
# shellcheck disable=SC2016,SC2034
set -u
export LC_ALL=C
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=7-relay/verify/lib.sh disable=SC1091
. 7-relay/verify/lib.sh

SETTLE=${G9_SETTLE:-3}
WAITS=${G9_WAIT:-60}
WINDOW=${G9_WINDOW:-60}
ITEMS=${G9_ITEMS:-"1 2 3 4 5 6 7"}
PORT=$L7_PORT_FABRIC
OLD_REV=${G9_OLD_REV:-$L7_G9_OLD_TAG}
TAPE=$L7_TAPES/g9-no-pairing-a-to-member.tape

# ------------------------------------------------------------------ the tap and the stream reader (python, stdlib)
write_tools() { # DIR
  cat >"$1/cap.py" <<'PY'
# cap.py sniff OUT IFACE... : AF_PACKET tap, TCP segments and ARP requests -> OUT (JSON lines) until SIGTERM
# cap.py report OUT         : the flows of OUT as one JSON object on stdout:
#   {"flows": [{s, sp, d, dp, t0, t1, first, hello, rec: [[t, type, len]...], frames: [[t, type, len]...], fin, rst}],
#    "arp": [[t, spa, tpa]...], "syn": [[t, s, d, dp]...]}
# rec = the TLS records of a flow that starts with 0x16 (type, length); frames = the 111 wire of a flow that starts with
# the 111F hello (type, length of each frame after it).
import json, os, select, signal, socket, struct, sys, time

PORT = 11183
HELLO = b"111F\x01"


def sniff(out, ifaces):
    socks = []
    for i in ifaces:
        s = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(3))
        s.bind((i, 0))
        socks.append(s)
    f = open(out, "w", buffering=1)
    stop = []
    signal.signal(signal.SIGTERM, lambda *a: stop.append(1))
    signal.signal(signal.SIGINT, lambda *a: stop.append(1))
    open(out + ".ready", "w").close()
    while not stop:
        try:
            r, _, _ = select.select(socks, [], [], 0.2)
        except InterruptedError:
            continue
        for s in r:
            data, addr = s.recvfrom(262144)
            if addr[2] == 4:  # PACKET_OUTGOING: every frame is read once, at its sender's end
                continue
            t = round(time.time(), 3)
            et = data[12:14]
            if et == b"\x08\x06" and len(data) >= 42 and data[20:22] == b"\x00\x01":
                f.write(json.dumps({"t": t, "k": "arp", "spa": socket.inet_ntoa(data[28:32]), "tpa": socket.inet_ntoa(data[38:42])}) + "\n")
                continue
            if len(data) < 54 or et != b"\x08\x00" or data[23] != 6:
                continue
            ihl = (data[14] & 15) * 4
            tot = struct.unpack("!H", data[16:18])[0]
            t4 = 14 + ihl
            sport, dport, seq = struct.unpack("!HHI", data[t4:t4 + 8])
            if PORT not in (sport, dport):
                continue
            fl = data[t4 + 13]
            off = (data[t4 + 12] >> 4) * 4
            f.write(json.dumps({"t": t, "k": "tcp", "s": socket.inet_ntoa(data[26:30]), "sp": sport, "d": socket.inet_ntoa(data[30:34]),
                                "dp": dport, "seq": seq, "fl": fl, "p": data[t4 + off:14 + tot].hex()}) + "\n")
    f.close()


def assemble(segs):  # [(seq, t, bytes)] -> (bytes, [(offset, t)])
    segs = sorted((s, t, p) for s, t, p in segs if p)
    out, marks, cur = b"", [], None
    for seq, t, p in segs:
        if cur is None:
            cur = seq
        end = seq + len(p)
        if end <= cur:
            continue
        marks.append((len(out), t))
        out += p[max(0, cur - seq):]
        cur = end
    return out, marks


def when(marks, off):
    t = marks[0][1] if marks else 0
    for o, mt in marks:
        if o <= off:
            t = mt
        else:
            break
    return t


def report(out):
    flows, arp, syn = {}, [], []
    for line in open(out):
        try:
            r = json.loads(line)
        except ValueError:
            continue
        if r["k"] == "arp":
            arp.append([r["t"], r["spa"], r["tpa"]])
            continue
        if r["fl"] & 2 and not r["fl"] & 16:
            syn.append([r["t"], r["s"], r["d"], r["dp"]])
        key = (r["s"], r["sp"], r["d"], r["dp"])
        f = flows.setdefault(key, {"segs": [], "t0": r["t"], "t1": r["t"], "fin": False, "rst": False})
        f["segs"].append((r["seq"], r["t"], bytes.fromhex(r["p"])))
        f["t1"] = r["t"]
        f["fin"] = f["fin"] or bool(r["fl"] & 1)
        f["rst"] = f["rst"] or bool(r["fl"] & 4)
    res = []
    for (s, sp, d, dp), f in sorted(flows.items(), key=lambda kv: kv[1]["t0"]):
        b, marks = assemble(f["segs"])
        rec, frames = [], []
        if b[:1] == b"\x16":
            i = 0
            while i + 5 <= len(b):
                n = struct.unpack("!H", b[i + 3:i + 5])[0]
                rec.append([when(marks, i), b[i], n])
                i += 5 + n
        elif b[:5] == HELLO:
            i = 5
            while i + 3 <= len(b):
                n = struct.unpack("!H", b[i + 1:i + 3])[0]
                frames.append([when(marks, i), b[i], n])
                i += 3 + n
        res.append({"s": s, "sp": sp, "d": d, "dp": dp, "t0": f["t0"], "t1": f["t1"], "bytes": len(b), "first": b[0] if b else None,
                    "hello": HELLO in b, "rec": rec, "frames": frames, "fin": f["fin"], "rst": f["rst"]})
    json.dump({"flows": res, "arp": arp, "syn": syn}, sys.stdout)


if sys.argv[1] == "sniff":
    sniff(sys.argv[2], sys.argv[3:])
else:
    report(sys.argv[2])
PY
}

# ------------------------------------------------------------------ inner: runs inside the namespaces
declare -A HOLD ROOT DPID NID IPS VETH LOGN
HELPERS=()
ST=000
CAP_PID=""

api() { # NODE METHOD PATH [JSON] -> $ST, body in $W/body
  local n=$1 m=$2 p=$3 d=${4-}
  local args=(-sS -o "$W/body" -w '%{http_code}' -X "$m" --max-time 30 -H 'Content-Type: application/json')
  [ -n "$d" ] && args+=(-d "$d")
  : >"$W/body"
  ST=$(nsenter -t "${HOLD[$n]}" -n -- curl "${args[@]}" "http://127.0.0.1:$L7_PORT_DASH$p" 2>"$W/curl.err") || true
  [ -n "$ST" ] || ST=000
}
jget() { jq -r "$@" "$W/body" 2>/dev/null || true; }
jb() { [ -s "$W/body" ] && jq -e "$@" "$W/body" >/dev/null 2>&1; }
wait_for() { # SECONDS CONDITION (eval'd)
  local end=$((SECONDS + $1))
  while :; do
    if eval "$2"; then return 0; fi
    [ "$SECONDS" -ge "$end" ] && return 1
    sleep 0.3
  done
}
nx() { local n=$1; shift; nsenter -t "${HOLD[$n]}" -n -- "$@"; }
now() { date +%s.%N; }
rss_kb() { awk '/^VmRSS:/ { print $2 }' "/proc/${DPID[$1]}/status" 2>/dev/null; }
bundle_set() { # BUNDLE FIELD VALUE -> the bundle with the field replaced (spki | address)
  python3 -I -c '
import sys
b, f, v = sys.argv[1:4]
p = b.split(":")
spki, addr = p[3], ":".join(p[4:-2])
if f == "spki": spki = v
else: addr = v
print(":".join([p[0], p[1], p[2], spki, addr, p[-2], p[-1]]))' "$1" "$2" "$3"
}

# ---- the tap
cap_start() {
  local ifs=() n
  for n in $L7_G9_NODES; do [ -n "${VETH[$n]:-}" ] && ifs+=("${VETH[$n]}"); done
  ifs+=("${VETH[AL]}" "${VETH[BL]}")
  rm -f "$W/cap.jsonl" "$W/cap.jsonl.ready"
  python3 -I "$W/cap.py" sniff "$W/cap.jsonl" "${ifs[@]}" >"$W/cap.err" 2>&1 &
  CAP_PID=$!
  local _
  for _ in $(seq 1 50); do [ -e "$W/cap.jsonl.ready" ] && return 0; sleep 0.1; done
  fail "the AF_PACKET tap did not start ($(head -c 200 "$W/cap.err"))"
  return 1
}
cap_report() { python3 -I "$W/cap.py" report "$W/cap.jsonl" >"$W/rep.json" 2>"$W/rep.err" || fail "the capture report failed ($(head -c 200 "$W/rep.err"))"; }
# frames of the plain 111 wire from SRC ip to DST ip in [T0, T1]: "type len" lines (keepalives, type 3, left out)
plain_frames() { # SRC DST T0 T1
  cap_report
  jq -r --arg s "$1" --arg d "$2" --argjson a "$3" --argjson b "$4" \
    '[.flows[] | select(.s == $s and .d == $d) | .frames[] | select(.[0] >= $a and .[0] <= $b and .[1] != 3)] | sort_by(.[0]) | .[] | "\(.[1]) \(.[2])"' "$W/rep.json"
}
plain_frames_t() { # the same with the time first: "t type len"
  cap_report
  jq -r --arg s "$1" --arg d "$2" --argjson a "$3" --argjson b "$4" \
    '[.flows[] | select(.s == $s and .d == $d) | .frames[] | select(.[0] >= $a and .[0] <= $b and .[1] != 3)] | sort_by(.[0]) | .[] | "\(.[0]) \(.[1]) \(.[2])"' "$W/rep.json"
}
# application-data records of at least MIN bytes on TLS flows from SRC to DST in [T0, T1]: "t len" lines
tls_big() { # SRC DST T0 T1 MIN
  cap_report
  jq -r --arg s "$1" --arg d "$2" --argjson a "$3" --argjson b "$4" --argjson m "$5" \
    '.flows[] | select(.s == $s and .d == $d) | .rec[] | select(.[0] >= $a and .[0] <= $b and .[1] == 23 and .[2] >= $m) | "\(.[0]) \(.[2])"' "$W/rep.json"
}
# the largest sealed (type 1) frame of the plain wire SRC -> DST in the window: the one CUSTODY_PUT of a member
put_len() { plain_frames "$@" | awk '$1 == 1 && $2 > m { m = $2 } END { print m + 0 }'; }
put_time() { plain_frames_t "$@" | awk '$2 == 1 && $3 > m { m = $3; t = $1 } END { printf "%.3f\n", t }'; }
first_after() { # "t len" lines on stdin, T: the first time >= T (empty when none)
  awk -v t="$1" '$1 >= t { print $1; exit }'
}
big_count() { tls_big "$@" | grep -c .; }

# ---- messages
MID=""
send() { # NODE TO TEXT -> $MID
  api "$1" POST /api/v1/messages "$(jq -nc --arg to "$2" --arg t "$3" '{to: $to, text: $t}')"
  MID=$(jget .id)
  [ "$ST" = 202 ] || note "send $1 -> ${2:0:8}: HTTP $ST $(head -c 200 "$W/body")"
}
mstate() { # NODE PEER MID -> "state reason"
  api "$1" GET "/api/v1/messages/threads/$2?limit=100"
  jq -r --arg id "$3" '[.messages[]? | select(.id == $id)] | first | "\(.state) \(.reason_code // "-")"' "$W/body" 2>/dev/null
}
mtext() { # n chars: a text of exactly N characters starting with TAG
  python3 -I -c 'import sys; t, n = sys.argv[1], int(sys.argv[2]); print((t + " " + "x" * n)[:n])' "$1" "$2"
}
recv_count() { # NODE PEER TEXT -> how many messages of the thread with PEER carry TEXT
  api "$1" GET "/api/v1/messages/threads/$2?limit=200"
  jq -r --arg t "$3" '[.messages[]? | select(.text == $t)] | length' "$W/body" 2>/dev/null
}
relay_get() { api "$1" GET /api/v1/relay; }
pending_of() { # NODE FAR -> pending forward rows toward FAR
  relay_get "$1"
  jq -r --arg f "${NID[$2]}" '[.paired_hubs[]? | select(.node_id == $f) | .pending] | first // "none"' "$W/body" 2>/dev/null
}
reach_of() { # NODE FAR -> true | false | none
  relay_get "$1"
  jq -r --arg f "${NID[$2]}" '[.paired_hubs[]? | select(.node_id == $f)] | first | if . == null then "none" else (.reachable | tostring) end' "$W/body" 2>/dev/null
}
refused_of() { relay_get "$1"; jq -r '.forward.refused // "none"' "$W/body" 2>/dev/null; }
toolarge_of() { relay_get "$1"; jq -r '.forward.too_large // "none"' "$W/body" 2>/dev/null; }
pushed_of() { api "$1" GET /api/v1/fabric; jq -r '.counters.items_pushed // "none"' "$W/body" 2>/dev/null; }
custody_of() { api "$1" GET /api/v1/fabric; jq -r '"\(.custody.count // "none")"' "$W/body" 2>/dev/null; }

# ---- daemons
start_node() { # NODE [BINARY]
  NODE111_ROOT=${ROOT[$1]} nsenter -t "${HOLD[$1]}" -n -- "${2:-$BIN}" daemon >>"$W/daemon-$1.log" 2>&1 &
  DPID[$1]=$!
}
stop_node() { # NODE
  local p=${DPID[$1]:-}
  [ -n "$p" ] || return 0
  kill -TERM "$p" 2>/dev/null
  local _
  for _ in $(seq 1 60); do kill -0 "$p" 2>/dev/null || break; sleep 0.2; done
  kill -KILL "$p" 2>/dev/null
  wait "$p" 2>/dev/null
  DPID[$1]=""
}
wait_up() { # NODE [SECONDS]
  local wn=$1
  wait_for "${2:-40}" 'api "$wn" GET /api/v1/fabric; [ "$ST" = 200 ] && jb ".store.open == true"'
}
link_state() { # NODE up|down: the host end of the node's WAN veth
  ip link set "${VETH[$1]}" "$2"
}

# the pairing of two hubs, each owner scanning the other's bundle; the address field is the WAN address of the hub
pair_scan() { # X Y RX VIA : X records Y as RX; Y's record is the complement; X scans first, Y a little later
  local x=$1 y=$2 rx=$3 via=$4 ry bx by
  case $rx in leaf) ry=gateway ;; gateway) ry=leaf ;; *) ry=peer ;; esac
  # the bundle a hub SHOWS carries the role it records for the scanner: Y shows ry, X (picking rx) scans it
  api "$y" POST /api/v1/relay/pair/offer "$(jq -nc --arg r "$ry" --arg v "$via" '{role: $r, via: $v}')"
  by=$(bundle_set "$(jget .bundle.text)" address "${IPS[$y]}:$PORT")
  api "$x" POST /api/v1/relay/pair/offer "$(jq -nc --arg r "$rx" --arg v "$via" '{role: $r, via: $v}')"
  bx=$(bundle_set "$(jget .bundle.text)" address "${IPS[$x]}:$PORT")
  api "$x" POST /api/v1/relay/pair "$(jq -nc --arg b "$by" --arg r "$rx" --arg v "$via" '{bundle: $b, role: $r, via: $v}')"
  local s1=$ST
  sleep 2
  api "$y" POST /api/v1/relay/pair "$(jq -nc --arg b "$bx" --arg r "$ry" --arg v "$via" '{bundle: $b, role: $r, via: $v}')"
  note "pairing $x <-> $y ($x records $y as $rx, $y records $x as $ry): scans answered $x HTTP $s1, $y HTTP $ST"
}
# the persisted record of a pair, as the daemon keeps it beside the fabric's (fabric/settings paired_hubs[]: ours and
# theirs, row M4b): the pair forwards only when both are true on both sides. GET /relay shows no such flag, only
# reachable and tls, and a pair whose second enrolment failed reads reachable all the same.
pair_rec_whole() { # X Y -> 0 when X's record of Y has ours and theirs
  jq -e --arg f "${NID[$2]}" '[.paired_hubs[]? | select(.node_id == $f and .ours == true and .theirs == true)] | length == 1' "${ROOT[$1]}/fabric/settings" >/dev/null 2>&1
}
pair_rec_half() { # X Y -> 0 when X's record of Y has exactly one of ours / theirs
  jq -e --arg f "${NID[$2]}" '[.paired_hubs[]? | select(.node_id == $f and ((.ours == true) != (.theirs == true)))] | length == 1' "${ROOT[$1]}/fabric/settings" >/dev/null 2>&1
}
pair_whole() { # X Y -> 0 when both read the pair reachable over rpk and both hold it whole
  local x=$1 y=$2
  relay_get "$x"
  jb --arg f "${NID[$y]}" '[.paired_hubs[]? | select(.node_id == $f and .reachable == true and .tls == "rpk")] | length == 1' || return 1
  relay_get "$y"
  jb --arg f "${NID[$x]}" '[.paired_hubs[]? | select(.node_id == $f and .reachable == true and .tls == "rpk")] | length == 1' || return 1
  pair_rec_whole "$x" "$y" && pair_rec_whole "$y" "$x"
}
unpair() { # X Y: both sides drop the record
  api "$1" DELETE "/api/v1/relay/paired/${NID[$2]}"
  api "$2" DELETE "/api/v1/relay/paired/${NID[$1]}"
  sleep 2
}
# pair_hubs X Y [ROLE [VIA]]: scan, wait for a whole pair over rpk, and when the two enrolments did not both land unpair and
# try again. Found by this gate's first runs: a hub holds ONE enrolment attempt at a time (mesh111 `Core::enrolling`, a newer
# attempt supersedes the older), and GET /relay reads a pair reachable over rpk while the attempt that makes it whole is
# still outstanding: a second pairing started then supersedes the first and leaves it half (ours = false) for good.
pair_hubs() {
  local x=$1 y=$2 rx=${3:-peer} via=${4:-tcp} try
  for try in 1 2 3; do
    pair_scan "$x" "$y" "$rx" "$via"
    if wait_for "$WAITS" 'pair_whole "$x" "$y"'; then
      [ "$try" -gt 1 ] && note "pairing $x <-> $y needed $try attempts (the first left a half pair)"
      return 0
    fi
    note "pairing $x <-> $y attempt $try left no whole pair (records: $x $(jq -c '[.paired_hubs[]? | {i: .node_id[0:8], o: .ours, t: .theirs}]' "${ROOT[$x]}/fabric/settings"), $y $(jq -c '[.paired_hubs[]? | {i: .node_id[0:8], o: .ours, t: .theirs}]' "${ROOT[$y]}/fabric/settings"))"
    unpair "$x" "$y"
  done
  return 1
}

inner() {
  W=${G9_WORK:?}
  BIN=${G9_BIN:?}
  local n i
  IPS[A]=$L7_G9_IP_A IPS[B]=$L7_G9_IP_B IPS[C]=$L7_G9_IP_C
  IPS[M]=$L7_G9_IP_M IPS[M2]=$L7_G9_IP_M2 IPS[D]=$L7_G9_IP_D IPS[D2]=$L7_G9_IP_D2 IPS[E]=""
  VETH[A]=$L7_G9_VETH_A VETH[B]=$L7_G9_VETH_B VETH[C]=$L7_G9_VETH_C VETH[M]=$L7_G9_VETH_M VETH[M2]=$L7_G9_VETH_M2
  VETH[D]=$L7_G9_VETH_D VETH[D2]=$L7_G9_VETH_D2 VETH[AL]=$L7_G9_VETH_AL VETH[BL]=$L7_G9_VETH_BL VETH[E]=""

  # shellcheck disable=SC2329
  cleanup_inner() {
    local p
    [ -n "$CAP_PID" ] && kill "$CAP_PID" 2>/dev/null
    for n in $L7_G9_NODES; do
      p=${DPID[$n]:-}
      [ -n "$p" ] && kill -TERM "$p" 2>/dev/null
    done
    for _ in $(seq 1 50); do
      local alive=0
      for n in $L7_G9_NODES; do kill -0 "${DPID[$n]:-0}" 2>/dev/null && alive=1; done
      [ "$alive" = 0 ] && break
      sleep 0.2
    done
    for n in $L7_G9_NODES; do
      p=${DPID[$n]:-}
      [ -n "$p" ] && kill -KILL "$p" 2>/dev/null
    done
    for p in "${HELPERS[@]}"; do kill "$p" 2>/dev/null; done
    wait 2>/dev/null
  }
  trap cleanup_inner EXIT
  trap 'exit 130' INT TERM

  ok "namespaces: user + net + pid, /proc is this namespace's own"
  ip link set lo up
  # ---- topology
  local br
  for br in "$L7_G9_WAN_BR" "$L7_G9_BR_LA" "$L7_G9_BR_LB"; do
    if ! { ip link add "$br" type bridge && ip link set "$br" up; }; then
      fail "cannot create the bridge $br"
      return
    fi
  done
  for n in $L7_G9_NODES; do
    unshare --net sleep 100000 &
    HOLD[$n]=$!
    HELPERS+=("$!")
  done
  sleep 0.5
  # veth NODE HOSTEND IFNAME IP BRIDGE
  veth() {
    local n=$1 h=$2 ifn=$3 ip=$4 b=$5
    ip link add "$h" type veth peer name "${h}p" &&
      ip link set "${h}p" netns "${HOLD[$n]}" &&
      nx "$n" ip link set "${h}p" name "$ifn" &&
      nx "$n" ip addr add "$ip/24" dev "$ifn" && nx "$n" ip link set "$ifn" up &&
      ip link set "$h" master "$b" && ip link set "$h" up
  }
  local built=1
  for n in $L7_G9_NODES; do nx "$n" ip link set lo up || built=0; done
  veth A "$L7_G9_VETH_A" eth0 "$L7_G9_IP_A" "$L7_G9_WAN_BR" || built=0
  veth B "$L7_G9_VETH_B" eth0 "$L7_G9_IP_B" "$L7_G9_WAN_BR" || built=0
  veth C "$L7_G9_VETH_C" eth0 "$L7_G9_IP_C" "$L7_G9_WAN_BR" || built=0
  veth A "$L7_G9_VETH_AL" eth1 "$L7_G9_LAN_A_IP_A" "$L7_G9_BR_LA" || built=0
  veth B "$L7_G9_VETH_BL" eth1 "$L7_G9_LAN_B_IP_B" "$L7_G9_BR_LB" || built=0
  veth M "$L7_G9_VETH_M" eth0 "$L7_G9_IP_M" "$L7_G9_BR_LA" || built=0
  veth M2 "$L7_G9_VETH_M2" eth0 "$L7_G9_IP_M2" "$L7_G9_BR_LA" || built=0
  veth D "$L7_G9_VETH_D" eth0 "$L7_G9_IP_D" "$L7_G9_BR_LB" || built=0
  veth D2 "$L7_G9_VETH_D2" eth0 "$L7_G9_IP_D2" "$L7_G9_BR_LB" || built=0
  if [ "$built" = 1 ]; then
    ok "eight netns: hubs A ${IPS[A]}, B ${IPS[B]}, C ${IPS[C]} on $L7_G9_WAN_NET (no private range: §4.1 rule 1 applies); A's LAN $L7_G9_LAN_A holds M, M2; B's LAN $L7_G9_LAN_B holds D, D2; E has loopback only"
  else
    fail "the topology could not be built"
    return
  fi
  write_tools "$W"
  cap_start || return

  # ---- eight real daemons
  for n in $L7_G9_NODES; do
    ROOT[$n]=$W/root-$n
    g6_make_root "${ROOT[$n]}" ""
    NID[$n]=$(g6_identity "${ROOT[$n]}" "$BIN")
    [ -n "${NID[$n]}" ] || {
      fail "$n: no identity ($(head -c 200 "${ROOT[$n]}/identity.log"))"
      return
    }
    case $n in
      A | B | C) g6_fabric_settings "${ROOT[$n]}" '{"schema":1,"hub_role":true,"listen_lan":true}' ;;
      *) g6_fabric_settings "${ROOT[$n]}" '{"schema":1}' ;;
    esac
    printf '{"schema":1,"updates":{"check_internet":false}}\n' >"${ROOT[$n]}/state/relay.json"
    chmod 600 "${ROOT[$n]}/state/relay.json"
    start_node "$n"
  done
  for n in $L7_G9_NODES; do
    wait_up "$n" || {
      fail "$n: the daemon's fabric never came up (HTTP $ST)"
      tail -5 "$W/daemon-$n.log" | sed "s/^/  $n daemon| /"
      return
    }
  done
  ok "eight daemons up with a fabric: $(for n in $L7_G9_NODES; do printf '%s %s, ' "$n" "${NID[$n]:0:8}"; done)"
  for n in A B C; do
    wait_for 10 'nx "$n" ss -ltnH 2>/dev/null | grep -q ":$PORT "' || fail "hub $n: nothing listens on :$PORT"
  done
  note "A listens on: $(nx A ss -ltnH | awk '{print $4}' | tr '\n' ' ')"

  run_items
}


# ------------------------------------------------------------------ the world
declare -A HUBCARD
chk() { # DESC CONDITION (eval'd)
  if eval "$2"; then ok "$1"; else fail "$1 [not so: $2]"; fi
}
card_of() { api "$1" GET /api/v1/contacts/me/card; jget .text; }
add_contact() { # NODE CARD
  api "$1" POST /api/v1/contacts "$(jq -nc --arg c "$2" '{card: $c}')"
  [ "$ST" = 200 ] || note "add_contact on $1: HTTP $ST $(head -c 200 "$W/body")"
}
enrol() { # MEMBER HUB: the hub offers, the member enrols at the hub's LAN address
  local m=$1 h=$2 lan code card
  case $h in A) lan=$L7_G9_LAN_A_IP_A ;; B) lan=$L7_G9_LAN_B_IP_B ;; esac
  api "$h" POST /api/v1/fabric/hub/offer '{}'
  code=$(jget .code) card=$(jget .card.text)
  HUBCARD[$h]=$card
  api "$m" POST /api/v1/fabric/hub/enroll "$(jq -nc --arg a "$lan:$PORT" --arg c "$code" --arg k "$card" '{address: $a, code: $c, card: $k}')"
  if [ "$ST" = 200 ] && jb '.enrolled == true and .reachable == true' && [ "$(jget .node_id)" = "${NID[$h]}" ]; then
    return 0
  fi
  fail "$m could not enrol at $h (HTTP $ST $(head -c 200 "$W/body"))"
  return 1
}
wait_state() { # NODE PEER MID "state reason" [SECONDS]
  local n=$1 p=$2 id=$3 want=$4
  wait_for "${5:-15}" '[ "$(mstate "$n" "$p" "$id")" = "$want" ]'
}
settle() { sleep "$SETTLE"; }
blk_on() { # SRC DST: drop SRC -> DST on the WAN bridge (one direction); callable again for another rule
  nft list table bridge g9 >/dev/null 2>&1 || {
    nft add table bridge g9 2>"$W/nft.err" && nft 'add chain bridge g9 blk { type filter hook forward priority 0 ; }' 2>>"$W/nft.err"
  } || return 1
  nft add rule bridge g9 blk ip saddr "$1" ip daddr "$2" drop 2>>"$W/nft.err"
}
blk_off() { nft delete table bridge g9 2>/dev/null; }
# the put sizes: a message of TEXT_LEN characters; the records and frames below are measured, not assumed
TEXT_LEN=100
PUTMIN=250 # a TLS application record this long is a custody put of a message, an ack put or CUSTODY_ACK is shorter on A -> X

setup_members() {
  local c
  CARD_D2=$(card_of D2) CARD_E=$(card_of E)
  CARD_D0=$(card_of D)
  enrol M A || return 1
  enrol M2 A || return 1
  enrol D B || return 1
  enrol D2 B || return 1
  wait_for 10 '[ "$(card_of D)" != "$CARD_D0" ]'
  CARD_D=$(card_of D)
  CARD_M=$(card_of M) CARD_M2=$(card_of M2)
  if [ "$CARD_D" != "$CARD_D0" ]; then
    ok "D's card carries its home hub once D is enrolled at B (the card text changed from ${#CARD_D0} to ${#CARD_D} characters); D2's and E's cards (taken before any enrolment) carry none"
  else
    fail "D's card did not change when D enrolled at B: the hint cannot be told apart"
  fi
  for c in "$CARD_D" "$CARD_D2" "$CARD_E" "${HUBCARD[B]}" "$CARD_M2"; do add_contact M "$c"; done
  for c in "$CARD_D" "$CARD_E" "${HUBCARD[B]}" "$CARD_M"; do add_contact M2 "$c"; done
  add_contact D "$CARD_M"
  add_contact D2 "$CARD_M"
  ok "members enrolled (M, M2 at A over ${L7_G9_LAN_A_IP_A}:$PORT; D, D2 at B over ${L7_G9_LAN_B_IP_B}:$PORT) and the cards exchanged"
}

pair_world() {
  local pr px py
  if [ "${G9_NEG:-}" = nopair ]; then
    note "G9_NEG=nopair: the negative control, no pairing is made: every item that needs a pair must FAIL"
    return 0
  fi
  for pr in "A C" "A B" "B C"; do
    px=${pr% *} py=${pr#* }
    if pair_hubs "$px" "$py"; then
      ok "$px <-> $py whole and reachable over rpk (GET /relay on both)"
    else
      fail "$px <-> $py never became whole over rpk"
      relay_get "$px"; note "$px relay: $(jget -c .paired_hubs)"
      relay_get "$py"; note "$py relay: $(jget -c .paired_hubs)"
      return 1
    fi
  done
}

# the tape scenario: a member (re)connects to its hub and puts a message for a contact the hub does not hold; the
# frames the hub sends it, keepalives left out, are one "type length" line each (the 6-transport/verify/tapes/ precedent)
# tape_run SENDER HUB HUB_LAN_IP SENDER_IP DEST -> $W/tape.<HUB>.now, MID, ST
tape_run() {
  local s=$1 h=$2 hip=$3 sip=$4 dest=$5 t0 t1 txt
  stop_node "$s"
  t0=$(now)
  start_node "$s"
  wait_up "$s"
  wait_for 30 'api "$s" GET /api/v1/fabric; jb ".hub.reachable == true"' || note "the tape sender $s is not reachable at its hub yet"
  txt=$(mtext g9-tape "$TEXT_LEN")
  send "$s" "$dest" "$txt"
  TAPE_TEXT=$txt
  wait_state "$s" "$dest" "$MID" "sent hub_not_yours" 20
  TAPE_ST=$(mstate "$s" "$dest" "$MID")
  settle
  t1=$(now)
  plain_frames "$hip" "$sip" "$t0" "$t1" >"$W/tape.$h.now"
}
tape_lines() { grep -v '^#' "$1" 2>/dev/null; }
tape_compare() { # FILE WHAT
  if [ ! -f "$TAPE" ]; then
    fail "$2: the committed tape $TAPE is missing (G9_WRITE_TAPE=1 writes it from this run)"
    return
  fi
  if [ "$(tape_lines "$1")" = "$(tape_lines "$TAPE")" ]; then
    ok "$2: the frames equal the committed tape ($(tape_lines "$TAPE" | grep -c .) frames: $(tape_lines "$TAPE" | tr '\n' ',' | sed 's/,$//'))"
  else
    fail "$2: the frames differ from the committed tape. now: $(tape_lines "$1" | tr '\n' ',') tape: $(tape_lines "$TAPE" | tr '\n' ',')"
  fi
}

# ------------------------------------------------------------------ item 1: no pairing, and the tape
item1() {
  note "---- item 1: no pairing: REJECT_UNKNOWN_DEST and a Layer 6 hub's frames"
  local lines
  relay_get A
  chk "A holds no pair: GET /relay paired_hubs is empty" 'jb ".paired_hubs | length == 0"'
  if [ "${WINDOW:-0}" -gt 0 ] && [[ " $ITEMS " == *" 7 "* ]]; then
    local v0 v1
    v0=$(vcs_sum "${DPID[A]}")
    sleep "$WINDOW"
    v1=$(vcs_sum "${DPID[A]}")
    BASE_RATE=$(awk -v a="$v0" -v b="$v1" -v w="$WINDOW" 'BEGIN { printf "%.1f", (b - a) * 60 / w }')
    note "RECORD wakeups: A with no pair (2 members, idle) $BASE_RATE wakeups/minute over ${WINDOW} s"
  fi
  tape_run M A "$L7_G9_LAN_A_IP_A" "$L7_G9_IP_M" "${NID[D]}"
  chk "no pairing: M's put for D (not a member of A) was answered REJECT_UNKNOWN_DEST: the message reads 'sent', reason hub_not_yours (got '$TAPE_ST')" '[ "$TAPE_ST" = "sent hub_not_yours" ]'
  chk "A holds no forward row and refused none: forward.refused 0" '[ "$(refused_of A)" = 0 ]'
  lines=$(grep -c . "$W/tape.A.now")
  note "A -> M frames from M's reconnect to the answer (type length): $(tr '\n' ',' <"$W/tape.A.now")"
  chk "the tape holds A's frames (an announce and the sealed answers): $lines frames" '[ "$lines" -ge 2 ]'
  if [ "${G9_WRITE_TAPE:-0}" = 1 ]; then
    [ "${REQUIRE_G9:-0}" = 1 ] && fail "G9_WRITE_TAPE=1 with REQUIRE_G9=1: a gate run does not rewrite its own reference (the tape is written, not compared, below)"
    {
      printf '# 7-relay/verify/tapes/g9-no-pairing-a-to-member.tape: the frames a hub with no pairing sends a member (G9 item 1, I13).\n'
      printf '# One line per frame, "type length" (type 1 sealed packet, 2 announce; keepalives left out), in wire order, from the\n'
      printf '# member'"'"'s reconnect to the hub'"'"'s answer to its CUSTODY_PUT for a contact the hub does not hold. Recorded by forward.sh\n'
      printf '# (G9_WRITE_TAPE=1) and equal for a Layer 6 build (item 6) and this tree.\n'
      cat "$W/tape.A.now"
    } >"$TAPE"
    note "tape written: $TAPE (not compared: the comparison would hold for any frames)"
  else
    tape_compare "$W/tape.A.now" "A's frames to M with no pairing"
  fi
}

# ------------------------------------------------------------------ item 2: the hint
item2() {
  note "---- item 2: with the hint (Sequence A: M -> A -> B -> D)"
  local txt t0 t1 mid st lh tb
  txt=$(mtext g9-2 "$TEXT_LEN")
  # B is stopped first: the row A holds is then readable as 'pending' (the store is SQLCipher; the harness holds no key)
  stop_node B
  if wait_for 20 '[ "$(reach_of A B)" = false ]'; then ok "B stopped: A reads the pair with B unreachable (the row has nowhere to go yet)"; else fail "A still reads B reachable after B stopped"; fi
  REF0=$(refused_of A)
  t0=$(now)
  send M "${NID[D]}" "$txt"; mid=$MID
  wait_state M "${NID[D]}" "$mid" "relayed forwarding" 20
  st=$(mstate M "${NID[D]}" "$mid")
  settle
  t1=$(now)
  chk "A answered ACCEPTED while B was away: M's message reads 'relayed', reason 'forwarding' (got '$st')" '[ "$st" = "relayed forwarding" ]'
  chk "A holds the row for the hint's pair, uncommitted: A's pair with B shows pending 1 (a committed row is not pending), nothing refused" \
    '[ "$(pending_of A B)" = 1 ] && [ "$(refused_of A)" = "$REF0" ]'
  chk "no probe of C: the hint named B only: pending toward C is $(pending_of A C) and no put-sized record went A -> C ($(big_count "${IPS[A]}" "${IPS[C]}" "$t0" "$t1" "$PUTMIN"))" \
    '[ "$(pending_of A C)" = 0 ] && [ "$(big_count "${IPS[A]}" "${IPS[C]}" "$t0" "$t1" "$PUTMIN")" = 0 ]'
  chk "D received nothing yet" '[ "$(recv_count D "${NID[M]}" "$txt")" = 0 ]'
  HINT_PUT_LEN=$(put_len "$L7_G9_IP_M" "$L7_G9_LAN_A_IP_A" "$t0" "$t1")
  note "M's CUSTODY_PUT toward A, hint present: one sealed frame of $HINT_PUT_LEN bytes on the LAN wire"
  # B comes back: A resumes, B answers ACCEPTED, A commits, D receives once, D's ACK crosses B -> A
  local tb0
  tb0=$(now)
  start_node B
  wait_up B
  if wait_for "$WAITS" 'pair_whole A B'; then ok "B back: the pair with A is reachable over rpk again"; else fail "the pair A <-> B did not come back within ${WAITS} s"; fi
  if wait_for 20 '[ "$(pending_of A B)" = 0 ]'; then ok "A's row committed: the pair's pending count fell to 0 (A commits only on ACCEPTED or DUPLICATE; the row was new at B: ACCEPTED)"; else fail "A's row for B is still pending ($(pending_of A B))"; fi
  chk "A's put reached B on the TLS pair: an application record of put size A -> B after B came back" '[ "$(big_count "${IPS[A]}" "${IPS[B]}" "$tb0" "$(now)" "$PUTMIN")" -ge 1 ]'
  if wait_for 20 '[ "$(recv_count D "${NID[M]}" "$txt")" = 1 ]'; then ok "D received the message once"; else fail "D has $(recv_count D "${NID[M]}" "$txt") copies of the message (want 1)"; fi
  if wait_state M "${NID[D]}" "$mid" "delivered -" "$WAITS"; then ok "M shows 'delivered': D's ACK came back (D -> B -> A -> M)"; else fail "M never showed delivered (it reads '$(mstate M "${NID[D]}" "$mid")')"; fi
  # the window opens at A's put to B (the first put-sized record A -> B after B came back): the reconnect's own hello and
  # announce (a 161 B record or more B -> A at every link-up) come before it, so they cannot meet the count
  local tp tbpre tbputs
  tp=$(tls_big "${IPS[A]}" "${IPS[B]}" "$tb0" "$(now)" "$PUTMIN" | first_after "$tb0")
  tbpre=$(tls_big "${IPS[B]}" "${IPS[A]}" "$tb0" "${tp:-$tb0}" 150 | wc -l)
  tb=$(tls_big "${IPS[B]}" "${IPS[A]}" "${tp:-0}" "$(now)" 150 | wc -l)
  tbputs=$(tls_big "${IPS[B]}" "${IPS[A]}" "${tp:-0}" "$(now)" "$PUTMIN" | wc -l)
  note "B -> A records of 150 B or more: $tbpre during the reconnect (before A's put, not counted), $tb after it; put-sized after it: $tbputs"
  chk "D's ACK crossed B -> A as an ordinary forward: $tb application record(s) of 150 bytes or more B -> A after A's put to B (the reconnect's $tbpre are not counted) and none put-sized ($tbputs: a bounce would be)" '[ -n "$tp" ] && [ "$tb" -ge 1 ] && [ "$tbputs" = 0 ]'
  settle
  chk "still one copy at D after the settle (no re-delivery)" '[ "$(recv_count D "${NID[M]}" "$txt")" = 1 ]'
  chk "A's pair with B is empty and nothing was refused" '[ "$(pending_of A B)" = 0 ] && [ "$(refused_of A)" = "$REF0" ]'
}

# ------------------------------------------------------------------ item 3: no hint
item3() {
  note "---- item 3: without a hint (probe order, cache, no pair at all)"
  local txt txt2 t0 t1 mid mid2 order tput tack tc tb
  relay_get A
  order=$(jq -r '[.paired_hubs[]?.node_id] | join(" ")' "$W/body")
  chk "A's paired_hubs order is C then B (the order the owner sees: A paired C first)" '[ "$order" = "${NID[C]} ${NID[B]}" ]'
  txt=$(mtext g9-3 "$TEXT_LEN")
  # A's probe of C goes into a blackhole: the ACK to M must not wait for it (C2), and B is not probed before C has answered
  if blk_on "${IPS[A]}" "${IPS[C]}"; then
    t0=$(now)
    send M "${NID[D2]}" "$txt"; mid=$MID
    wait_state M "${NID[D2]}" "$mid" "relayed -" 15
    sleep 2
    t1=$(now)
    chk "ACCEPTED before any probe has an answer: with A -> C blackholed M reads 'relayed' (plain: no hint, no reason; reads '$(mstate M "${NID[D2]}" "$mid")') while the probe of C sits unanswered" '[ "$(mstate M "${NID[D2]}" "$mid")" = "relayed -" ]'
    chk "probes in paired_hubs order, one put at a time: A sent the put to C first ($(big_count "${IPS[A]}" "${IPS[C]}" "$t0" "$t1" "$PUTMIN") record(s) toward C) and none to B while C had not answered ($(big_count "${IPS[A]}" "${IPS[B]}" "$t0" "$t1" "$PUTMIN"))" \
      '[ "$(big_count "${IPS[A]}" "${IPS[C]}" "$t0" "$t1" "$PUTMIN")" -ge 1 ] && [ "$(big_count "${IPS[A]}" "${IPS[B]}" "$t0" "$t1" "$PUTMIN")" = 0 ]'
    blk_off
  else
    fail "the nft bridge rule could not be installed ($(head -c 200 "$W/nft.err")): the ACK-before-probe check cannot run"
    t0=$(now)
    send M "${NID[D2]}" "$txt"; mid=$MID
  fi
  wait_state M "${NID[D2]}" "$mid" "delivered -" "$WAITS"
  settle
  t1=$(now)
  chk "M's message to D2 (a card with no hint) is delivered: C refused once the blackhole lifted, the next pair, B, ACCEPTED, D2's ACK came back (reads '$(mstate M "${NID[D2]}" "$mid")')" '[ "$(mstate M "${NID[D2]}" "$mid")" = "delivered -" ]'
  chk "D2 received it once" '[ "$(recv_count D2 "${NID[M]}" "$txt")" = 1 ]'
  NOHINT_PUT_LEN=$(put_len "$L7_G9_IP_M" "$L7_G9_LAN_A_IP_A" "$t0" "$t1")
  chk "the hint's 11 bytes: the hinted put (item 2) was $((HINT_PUT_LEN)) B, the same-sized first message to D2, whose card carries no hint, was $NOHINT_PUT_LEN B (both are first contacts: each carries the introduction until it is acknowledged): exactly 11 more (TLV 0x02: n_ext, type, len, 8 B)" '[ "$((HINT_PUT_LEN - NOHINT_PUT_LEN))" = 11 ]'
  tput=$(put_time "$L7_G9_IP_M" "$L7_G9_LAN_A_IP_A" "$t0" "$t1")
  tack=$(plain_frames_t "$L7_G9_LAN_A_IP_A" "$L7_G9_IP_M" "$tput" "$t1" | awk '$2 == 1 { print $1; exit }')
  tc=$(tls_big "${IPS[A]}" "${IPS[C]}" "$tput" "$t1" "$PUTMIN" | first_after "$tput")
  tb=$(tls_big "${IPS[A]}" "${IPS[B]}" "$tput" "$t1" "$PUTMIN" | first_after "$tput")
  note "timeline (s after M's put at the wire): A's ack to M +$(awk -v a="${tack:-0}" -v p="$tput" 'BEGIN { printf "%.3f", a - p }'), put to C +$(awk -v a="${tc:-0}" -v p="$tput" 'BEGIN { printf "%.3f", a - p }'), put to B +$(awk -v a="${tb:-0}" -v p="$tput" 'BEGIN { printf "%.3f", a - p }')"
  chk "the put to C (first in paired_hubs order) left before the put to B" '[ -n "$tc" ] && [ -n "$tb" ] && awk -v c="$tc" -v b="$tb" "BEGIN { exit !(c < b) }"'
  # the second message: one put, no probe
  txt2=$(mtext g9-3b "$TEXT_LEN")
  t0=$(now)
  send M "${NID[D2]}" "$txt2"; mid2=$MID
  wait_state M "${NID[D2]}" "$mid2" "delivered -" "$WAITS"
  settle
  t1=$(now)
  chk "second message to D2: delivered" '[ "$(mstate M "${NID[D2]}" "$mid2")" = "delivered -" ]'
  NOHINT2_PUT_LEN=$(put_len "$L7_G9_IP_M" "$L7_G9_LAN_A_IP_A" "$t0" "$t1")
  note "M's put of the second (unhinted, not first contact) message to D2: $NOHINT2_PUT_LEN B, the reference for item 6's old-hub part"
  chk "cache: A -> B carried exactly one put-sized record for the second message ($(big_count "${IPS[A]}" "${IPS[B]}" "$t0" "$t1" "$PUTMIN")) and A -> C none ($(big_count "${IPS[A]}" "${IPS[C]}" "$t0" "$t1" "$PUTMIN"))" \
    '[ "$(big_count "${IPS[A]}" "${IPS[B]}" "$t0" "$t1" "$PUTMIN")" = 1 ] && [ "$(big_count "${IPS[A]}" "${IPS[C]}" "$t0" "$t1" "$PUTMIN")" = 0 ]'
  # no pair at all
  local x
  for x in C B; do unpair A "$x"; done
  relay_get A
  chk "A holds no pair after the unpair (DELETE /relay/paired on both sides)" 'jb ".paired_hubs | length == 0"'
  local txt3 mid3
  txt3=$(mtext g9-3c "$TEXT_LEN")
  send M "${NID[D2]}" "$txt3"; mid3=$MID
  wait_state M "${NID[D2]}" "$mid3" "sent hub_not_yours" 15
  chk "no pair at all: M's put for D2 is answered REJECT_UNKNOWN_DEST ('sent', hub_not_yours; reads '$(mstate M "${NID[D2]}" "$mid3")')" '[ "$(mstate M "${NID[D2]}" "$mid3")" = "sent hub_not_yours" ]'
  # pair again (A with C, then A with B): the order is C then B once more
  for x in C B; do
    pair_hubs A "$x" || fail "A <-> $x did not come back whole after the re-pair"
  done
  relay_get A
  order=$(jq -r '[.paired_hubs[]?.node_id] | join(" ")' "$W/body")
  chk "re-paired: A's order is C then B again" '[ "$order" = "${NID[C]} ${NID[B]}" ]'
}

# ------------------------------------------------------------------ item 4: every pair refusing
item4() {
  note "---- item 4: every pair refusing, a pair that is down, the per-origin share"
  local txt t0 t1 mid st r0 r1
  r0=$(refused_of A)
  txt=$(mtext g9-4a "$TEXT_LEN")
  t0=$(now)
  send M "${NID[E]}" "$txt"; mid=$MID
  wait_state M "${NID[E]}" "$mid" "relayed -" 15
  st=$(mstate M "${NID[E]}" "$mid")
  chk "M's message to E (nobody's member, no hint) is ACCEPTED by A: M reads plain 'relayed' (no forwarding reason without a hint; got '$st')" '[ "$st" = "relayed -" ]'
  wait_for 20 '[ "$(refused_of A)" = "$((r0 + 1))" ]'
  settle
  t1=$(now)
  r1=$(refused_of A)
  chk "every pair refused: A counts the row in forward_refused ($r0 -> $r1)" '[ "$r1" = "$((r0 + 1))" ]'
  chk "A probed both pairs (a put-sized record to C and one to B) and neither pair holds the row: pending 0, 0" \
    '[ "$(big_count "${IPS[A]}" "${IPS[C]}" "$t0" "$t1" "$PUTMIN")" -ge 1 ] && [ "$(big_count "${IPS[A]}" "${IPS[B]}" "$t0" "$t1" "$PUTMIN")" -ge 1 ] && [ "$(pending_of A C)" = 0 ] && [ "$(pending_of A B)" = 0 ]'
  chk "M stays 'relayed' (v1 has no later status; the sender keeps its own copy): still '$(mstate M "${NID[E]}" "$mid")'" '[ "$(mstate M "${NID[E]}" "$mid")" = "relayed -" ]'
  chk "the refusal is final at hop 1: B and C forwarded nothing to each other (no put-sized record B -> C or C -> B), and B and C count no refused row" \
    '[ "$(big_count "${IPS[B]}" "${IPS[C]}" "$t0" "$t1" "$PUTMIN")" = 0 ] && [ "$(big_count "${IPS[C]}" "${IPS[B]}" "$t0" "$t1" "$PUTMIN")" = 0 ] && [ "$(refused_of B)" = 0 ] && [ "$(refused_of C)" = 0 ]'
  # a pair that is down keeps the row protected
  stop_node B
  wait_for 20 '[ "$(reach_of A B)" = false ]'
  txt=$(mtext g9-4b "$TEXT_LEN")
  r0=$(refused_of A)
  send M "${NID[E]}" "$txt"; mid=$MID
  wait_state M "${NID[E]}" "$mid" "relayed -" 15
  settle
  settle
  chk "B down: the row is not refused while a pair has not answered (forward_refused stays $r0, reads $(refused_of A)) and A's pair with B shows it pending ($(pending_of A B))" \
    '[ "$(refused_of A)" = "$r0" ] && [ "$(pending_of A B)" = 1 ]'
  start_node B
  wait_up B
  wait_for "$WAITS" 'pair_whole A B' || fail "A <-> B did not come back"
  if wait_for 30 '[ "$(refused_of A)" = "$((r0 + 1))" ] && [ "$(pending_of A B)" = 0 ]'; then
    ok "B back: it refused (not its member), the row is now refused (forward_refused $((r0 + 1))) and no longer pending"
  else
    fail "after B came back the row is not refused (refused $(refused_of A), pending $(pending_of A B))"
  fi
  # the per-origin share: a test-sized quota.transit with every pair down, so each held row stays protected
  stop_node B
  stop_node C
  wait_for 20 '[ "$(reach_of A B)" = false ] && [ "$(reach_of A C)" = false ]'
  api A PATCH /api/v1/fabric/settings '{"quotas":{"transit":12000}}'
  chk "A: quota.transit = 12000 (per-origin share 1500 B)" '[ "$ST" = 200 ]'
  local i got_full=0 acc=0
  for i in 1 2 3 4 5 6; do
    txt=$(mtext "g9-4c$i" 400)
    send M "${NID[E]}" "$txt"; mid=$MID
    wait_for 10 '[ "$(mstate M "${NID[E]}" "$mid")" = "relayed -" ] || [ "$(mstate M "${NID[E]}" "$mid")" = "sent mailbox_full" ]'
    st=$(mstate M "${NID[E]}" "$mid")
    if [ "$st" = "relayed -" ]; then acc=$((acc + 1)); elif [ "$st" = "sent mailbox_full" ]; then
      got_full=1
      break
    else break; fi
  done
  chk "M over its share: $acc forward put(s) ACCEPTED, then REJECT_FULL ('sent', mailbox_full)" '[ "$acc" -ge 1 ] && [ "$got_full" = 1 ]'
  txt=$(mtext g9-4m2 400)
  send M2 "${NID[E]}" "$txt"; mid=$MID
  wait_for 10 '[ "$(mstate M2 "${NID[E]}" "$mid")" = "relayed -" ] || [ "$(mstate M2 "${NID[E]}" "$mid")" = "sent mailbox_full" ]'
  st=$(mstate M2 "${NID[E]}" "$mid")
  chk "a second member's forward put is ACCEPTED while M is refused (M2 reads '$st')" '[ "$st" = "relayed -" ]'
  api A PATCH /api/v1/fabric/settings '{"quotas":{"transit":null}}'
  start_node B
  start_node C
  wait_up B
  wait_up C
  wait_for "$WAITS" 'pair_whole A B && pair_whole A C' || fail "the pairs did not come back after the share check"
  wait_for 30 '[ "$(pending_of A B)" = 0 ] && [ "$(pending_of A C)" = 0 ]' || note "rows of the share check still pending: B $(pending_of A B), C $(pending_of A C)"
}

# ------------------------------------------------------------------ item 5: loops and roles
item5() {
  note "---- item 5: loops, the role matrix"
  local txt t0 t1 mid r0 rb rc offer_c bundle
  # (a) three hubs, all peers: an item for nobody's member never passes hop 1
  r0=$(refused_of A) rb=$(refused_of B) rc=$(refused_of C)
  txt=$(mtext g9-5a "$TEXT_LEN")
  t0=$(now)
  send M "${NID[E]}" "$txt"; mid=$MID
  wait_for 20 '[ "$(refused_of A)" = "$((r0 + 1))" ]'
  settle
  t1=$(now)
  chk "A, B, C all peers: A refused the item once (forward_refused $r0 -> $(refused_of A)); B and C forwarded nothing (no put-sized record B -> C, C -> B, B -> A, C -> A) and hold no row (refused B $(refused_of B), C $(refused_of C), pending B -> C $(pending_of B C), C -> B $(pending_of C B))" \
    '[ "$(refused_of A)" = "$((r0 + 1))" ] && [ "$(big_count "${IPS[B]}" "${IPS[C]}" "$t0" "$t1" "$PUTMIN")" = 0 ] && [ "$(big_count "${IPS[C]}" "${IPS[B]}" "$t0" "$t1" "$PUTMIN")" = 0 ] && [ "$(big_count "${IPS[B]}" "${IPS[A]}" "$t0" "$t1" "$PUTMIN")" = 0 ] && [ "$(big_count "${IPS[C]}" "${IPS[A]}" "$t0" "$t1" "$PUTMIN")" = 0 ] && [ "$(refused_of B)" = "$rb" ] && [ "$(refused_of C)" = "$rc" ] && [ "$(pending_of B C)" = 0 ] && [ "$(pending_of C B)" = 0 ]'
  note "a bounce DUPLICATE (an item returning to A) and a hint naming A at A are not reachable from outside: the first needs a hub that returns a row (no role bit makes one), the second a member that lies (the sender's TLV is never its own hub's, M/fabric.rs hinted_elsewhere); both are unit-tested in mesh111 (tests/forward.rs). The DUPLICATE answer itself is exercised by item 6 (a restarted A re-puts a row B already took)"
  # (b) the role matrix
  api C POST /api/v1/relay/pair/offer '{"role":"peer","via":"tcp"}'
  bundle=$(jget .bundle.text)
  relay_get A
  local before
  before=$(jq -c '.paired_hubs' "$W/body")
  api A POST /api/v1/relay/pair "$(jq -nc --arg b "$bundle" '{bundle: $b, role: "leaf", via: "tcp"}')"
  chk "a scan whose pick is not the bundle's complement (the bundle says peer, the pick leaf): 409 pair_role_mismatch (got $ST $(jget -r .code))" '[ "$ST" = 409 ] && [ "$(jget -r .code)" = pair_role_mismatch ]'
  api A POST /api/v1/relay/pair "$(jq -nc --arg b "$bundle" '{bundle: $b, role: "leaf", via: "carrier"}')"
  chk "a leaf pick with via carrier: refused 409 pair_role_mismatch (got $ST $(jget -r .code))" '[ "$ST" = 409 ] && [ "$(jget -r .code)" = pair_role_mismatch ]'
  api C POST /api/v1/relay/pair/offer '{"role":"gateway","via":"carrier"}'
  chk "an offer of role gateway with via carrier is refused at the offer too: 409 pair_role_mismatch (got $ST $(jget -r .code))" '[ "$ST" = 409 ] && [ "$(jget -r .code)" = pair_role_mismatch ]'
  relay_get A
  chk "nothing was recorded by the refused scans: A's paired_hubs is as before" '[ "$(jq -c ".paired_hubs" "$W/body")" = "$before" ]'
  # (c) a put against the pair's recorded role: A records C as its leaf, C records A as its gateway
  local x
  unpair A C
  if pair_hubs A C leaf; then
    relay_get A
    chk "A records C as leaf (the pair is whole over rpk)" 'jb --arg f "${NID[C]}" "[.paired_hubs[]? | select(.node_id == \$f and .role == \"leaf\" and .via == \"tcp\")] | length == 1"'
    relay_get C
    chk "C records A as gateway" 'jb --arg f "${NID[A]}" "[.paired_hubs[]? | select(.node_id == \$f and .role == \"gateway\" and .via == \"tcp\")] | length == 1"'
    r0=$(refused_of A) rc=$(refused_of C)
    txt=$(mtext g9-5c "$TEXT_LEN")
    t0=$(now)
    send M "${NID[E]}" "$txt"; mid=$MID
    wait_for 20 '[ "$(refused_of A)" = "$((r0 + 1))" ]'
    settle
    t1=$(now)
    chk "a put that arrives against the recorded role is refused: A's put for a non-member reached C (its gateway side, put-sized record A -> C: $(big_count "${IPS[A]}" "${IPS[C]}" "$t0" "$t1" "$PUTMIN")), C answered REJECT_UNKNOWN_DEST and kept nothing (refused C $(refused_of C), pending C -> B $(pending_of C B), no record C -> B), A refused the row ($(refused_of A))" \
      '[ "$(big_count "${IPS[A]}" "${IPS[C]}" "$t0" "$t1" "$PUTMIN")" -ge 1 ] && [ "$(refused_of C)" = "$rc" ] && [ "$(pending_of C B)" = 0 ] && [ "$(big_count "${IPS[C]}" "${IPS[B]}" "$t0" "$t1" "$PUTMIN")" = 0 ] && [ "$(refused_of A)" = "$((r0 + 1))" ]'
  else
    fail "A <-> C as leaf and gateway never became whole"
  fi
  # back to peers: the later items
  unpair A C
  pair_hubs A C || fail "A <-> C did not come back as peers"
}

# ------------------------------------------------------------------ item 6: restart, quotas, REJECT_FULL
item6() {
  note "---- item 6: a restart resumes on link-up and B deduplicates; Transit quota; REJECT_FULL retried"
  local txt t0 t1 mid st tr
  # (a) A restarts with a protected row B already took (B's CUSTODY_ACK is lost on the way back)
  txt=$(mtext g9-6a "$TEXT_LEN")
  if blk_on "${IPS[B]}" "${IPS[A]}"; then
    ok "the B -> A direction of the TEST-NET bridge is blackholed (nft bridge drop): A's puts reach B, B's answers do not reach A"
    local ts
    ts=$(now)
    send M "${NID[D]}" "$txt"; mid=$MID
    wait_state M "${NID[D]}" "$mid" "relayed forwarding" 20
    wait_for 20 '[ "$(recv_count D "${NID[M]}" "$txt")" = 1 ]'
    settle
    chk "B took the put (D has it once) but A never saw B's answer: the row is still pending at A (pending $(pending_of A B)) and M has not been told 'delivered'" \
      '[ "$(recv_count D "${NID[M]}" "$txt")" = 1 ] && [ "$(pending_of A B)" = 1 ] && [ "$(mstate M "${NID[D]}" "$mid")" = "relayed forwarding" ]'
    stop_node A
    blk_off
    tr=$(now)
    start_node A
    wait_up A
    if wait_for "$WAITS" 'pair_whole A B'; then ok "A restarted with a protected row: the pair with B came back on link-up"; else fail "the pair A <-> B did not come back after A's restart"; fi
    if wait_for "$WAITS" '[ "$(pending_of A B)" = 0 ]'; then
      ok "the restarted A resumed the row on link-up and B answered DUPLICATE (the put went again, A committed: pending 0)"
    else
      fail "after A's restart the row is still pending ($(pending_of A B))"
    fi
    chk "the put went to B again after A's restart: an application record of put size A -> B (re-put $(big_count "${IPS[A]}" "${IPS[B]}" "$tr" "$(now)" "$PUTMIN"))" '[ "$(big_count "${IPS[A]}" "${IPS[B]}" "$tr" "$(now)" "$PUTMIN")" -ge 1 ]'
    settle
    # a re-push of the same item would be a sealed frame of the same length as the first delivery's (the largest sealed frame
    # B -> D in the first window); other B -> D frames in the second window (an unrelated item) have another length
    local pd0 pd1 pd2
    pd0=$(put_len "$L7_G9_LAN_B_IP_B" "$L7_G9_IP_D" "$ts" "$tr")
    pd1=$(plain_frames "$L7_G9_LAN_B_IP_B" "$L7_G9_IP_D" "$tr" "$(now)" | awk -v n="$pd0" '$1 == 1 && $2 == n' | wc -l)
    pd2=$(plain_frames "$L7_G9_LAN_B_IP_B" "$L7_G9_IP_D" "$ts" "$tr" | awk -v n="$pd0" '$1 == 1 && $2 == n' | wc -l)
    note "B -> D sealed frames (type length) after A's restart: $(plain_frames "$L7_G9_LAN_B_IP_B" "$L7_G9_IP_D" "$tr" "$(now)" | awk '{ printf "%s/%s ", $1, $2 }'); first delivery: $(plain_frames "$L7_G9_LAN_B_IP_B" "$L7_G9_IP_D" "$ts" "$tr" | awk '{ printf "%s/%s ", $1, $2 }')"
    chk "B deduplicated: D still has the message exactly once, and B did not push the item to D again (a sealed frame of the first delivery's length, $pd0 B: $pd2 in the first window, $pd1 after A's restart)" \
      '[ "$(recv_count D "${NID[M]}" "$txt")" = 1 ] && [ "$pd0" -gt 0 ] && [ "$pd2" -ge 1 ] && [ "$pd1" = 0 ]'
    if wait_state M "${NID[D]}" "$mid" "delivered -" "$WAITS"; then ok "M shows 'delivered' once the blackhole is gone (D's ACK crossed B -> A)"; else fail "M never showed delivered after the restart ($(mstate M "${NID[D]}" "$mid"))"; fi
  else
    fail "the nft bridge rule could not be installed ($(head -c 200 "$W/nft.err")): the restart-and-dedup item cannot run"
  fi
  # (b) a test-sized Transit quota: forward puts REJECT_FULL, a member's put for a member ACCEPTED
  api A PATCH /api/v1/fabric/settings '{"quotas":{"transit":64}}'
  txt=$(mtext g9-6b "$TEXT_LEN")
  send M "${NID[E]}" "$txt"; mid=$MID
  wait_for 15 '[ "$(mstate M "${NID[E]}" "$mid")" = "sent mailbox_full" ] || [ "$(mstate M "${NID[E]}" "$mid")" = "relayed -" ]'
  st=$(mstate M "${NID[E]}" "$mid")
  chk "quota.transit = 64: M's forward put is REJECT_FULL ('sent', mailbox_full; reads '$st')" '[ "$st" = "sent mailbox_full" ]'
  txt=$(mtext g9-6b2 "$TEXT_LEN")
  send M "${NID[M2]}" "$txt"; mid=$MID
  wait_for 15 '[ "$(mstate M "${NID[M2]}" "$mid")" = "delivered -" ] || [ "$(mstate M "${NID[M2]}" "$mid")" = "relayed -" ]'
  st=$(mstate M "${NID[M2]}" "$mid")
  chk "the same hub, the same moment: M's put for a member (M2) is ACCEPTED ('$st': relayed or delivered)" '[ "$st" = "relayed -" ] || [ "$st" = "delivered -" ]'
  api A PATCH /api/v1/fabric/settings '{"quotas":{"transit":null}}'
  # (c) REJECT_FULL from B (its custody quota): kept, retried on the next forwarder run and on the pair's next attribution
  local c1 c2 c3 m1 m2 m3
  api B PATCH /api/v1/fabric/settings '{"quotas":{"custody":100}}'
  c1=$(mtext g9-6c1 "$TEXT_LEN")
  r0=$(refused_of A)
  t0=$(now)
  send M "${NID[D]}" "$c1"; m1=$MID
  wait_state M "${NID[D]}" "$m1" "relayed forwarding" 20
  settle
  chk "B's custody quota is 100: A's put reached B ($(big_count "${IPS[A]}" "${IPS[B]}" "$t0" "$(now)" "$PUTMIN") put-sized record(s) A -> B) and B answered REJECT_FULL: A keeps the row for B (pending $(pending_of A B)), D has nothing, nothing refused" \
    '[ "$(big_count "${IPS[A]}" "${IPS[B]}" "$t0" "$(now)" "$PUTMIN")" -ge 1 ] && [ "$(pending_of A B)" = 1 ] && [ "$(recv_count D "${NID[M]}" "$c1")" = 0 ] && [ "$(refused_of A)" = "$r0" ]'
  api B PATCH /api/v1/fabric/settings '{"quotas":{"custody":null}}'
  t0=$(now)
  settle
  settle
  chk "B's quota lifted: no timer retries it (still pending $(pending_of A B), D has $(recv_count D "${NID[M]}" "$c1"), $(big_count "${IPS[A]}" "${IPS[B]}" "$t0" "$(now)" "$PUTMIN") put-sized record(s) A -> B since)" '[ "$(pending_of A B)" = 1 ] && [ "$(recv_count D "${NID[M]}" "$c1")" = 0 ] && [ "$(big_count "${IPS[A]}" "${IPS[B]}" "$t0" "$(now)" "$PUTMIN")" = 0 ]'
  c2=$(mtext g9-6c2 "$TEXT_LEN")
  t0=$(now)
  send M "${NID[D]}" "$c2"; m2=$MID
  if wait_for 30 '[ "$(pending_of A B)" = 0 ] && [ "$(recv_count D "${NID[M]}" "$c1")" = 1 ] && [ "$(recv_count D "${NID[M]}" "$c2")" = 1 ]'; then
    ok "the next forwarder run for the pair (a new row) retried the REJECT_FULL row: both messages at D once, pending 0 ($(big_count "${IPS[A]}" "${IPS[B]}" "$t0" "$(now)" "$PUTMIN") put-sized record(s) A -> B: the held row and the new one)"
  else
    fail "the next forwarder run did not retry the REJECT_FULL row (pending $(pending_of A B), D has c1 $(recv_count D "${NID[M]}" "$c1"), c2 $(recv_count D "${NID[M]}" "$c2"))"
  fi
  api B PATCH /api/v1/fabric/settings '{"quotas":{"custody":100}}'
  c3=$(mtext g9-6c3 "$TEXT_LEN")
  send M "${NID[D]}" "$c3"; m3=$MID
  wait_state M "${NID[D]}" "$m3" "relayed forwarding" 20
  settle
  chk "again REJECT_FULL at B: the row is held (pending $(pending_of A B))" '[ "$(pending_of A B)" = 1 ] && [ "$(recv_count D "${NID[M]}" "$c3")" = 0 ]'
  api B PATCH /api/v1/fabric/settings '{"quotas":{"custody":null}}'
  stop_node B
  start_node B
  wait_up B
  if wait_for "$WAITS" 'pair_whole A B'; then ok "B restarted: the pair is attributed again"; else fail "the pair did not come back"; fi
  if wait_for 30 '[ "$(pending_of A B)" = 0 ] && [ "$(recv_count D "${NID[M]}" "$c3")" = 1 ]'; then
    ok "the pair's next attribution retried the REJECT_FULL row with no new message: delivered once, pending 0"
  else
    fail "the attribution did not retry the REJECT_FULL row (pending $(pending_of A B), D has $(recv_count D "${NID[M]}" "$c3"))"
  fi
}

# ------------------------------------------------------------------ item 7: non-hub, deleted settings, half pair, the wire
item7() {
  note "---- item 7: a non-hub, a deleted fabric/settings, a half pair, nothing in clear, the records"
  local t0 t1 n r0 txt mid rows
  # (a) the non-hub refuses paired_hubs and dials nothing; the same row in a hub dials (the control)
  api M2 POST /api/v1/relay/pair/offer '{"role":"peer","via":"tcp"}'
  chk "a non-hub refuses to pair: POST /relay/pair/offer on M2 is 409 needs_hub (got $ST $(jget -r .code))" '[ "$ST" = 409 ] && [ "$(jget -r .code)" = needs_hub ]'
  api M2 POST /api/v1/relay/pair '{"bundle":"111PAIR:x:y::192.0.2.50:11183:peer:tcp","role":"peer","via":"tcp"}'
  chk "a non-hub refuses a scan as well: POST /relay/pair on M2 is 409 needs_hub (got $ST $(jget -r .code))" '[ "$ST" = 409 ] && [ "$(jget -r .code)" = needs_hub ]'
  local row_m2 row_c
  # the extra row is a copy of one of C's own whole pair rows (the shape the daemon writes: ours, theirs, link_spki,
  # far_holds_address) with a node id no hub has and an address nobody answers at, marked lan (plain)
  local ghost tmpl
  ghost=$(printf 'f%.0s' $(seq 1 64))
  tmpl=$(jq -c '.paired_hubs[0]' "${ROOT[C]}/fabric/settings")
  row_m2=$(jq -c --arg id "$ghost" --arg a "10.111.79.99:11183" '.node_id = $id | .address = $a | .lan = true | .far_holds_address = true' <<<"$tmpl")
  row_c=$(jq -c --arg id "$ghost" --arg a "192.0.2.99:11183" '.node_id = $id | .address = $a | .lan = true | .far_holds_address = true' <<<"$tmpl")
  cp "${ROOT[M2]}/fabric/settings" "$W/m2-settings.bak"
  cp "${ROOT[C]}/fabric/settings" "$W/c-settings.bak"
  stop_node M2
  jq --argjson r "$row_m2" '.paired_hubs = ((.paired_hubs // []) + [$r])' "$W/m2-settings.bak" >"${ROOT[M2]}/fabric/settings"
  chmod 600 "${ROOT[M2]}/fabric/settings"
  t0=$(now)
  start_node M2
  wait_up M2
  sleep 12
  t1=$(now)
  cap_report
  n=$(jq -r --argjson a "$t0" --argjson b "$t1" '[.arp[] | select(.[0] >= $a and .[0] <= $b and .[2] == "10.111.79.99")] | length' "$W/rep.json")
  rows=$(api M2 GET /api/v1/relay; jq -r '.paired_hubs | length' "$W/body")
  chk "a non-hub that holds a paired_hubs row dials nothing: no ARP who-has for the row's address 10.111.79.99 in 12 s ($n), and GET /relay lists $rows pair(s)" '[ "$n" = 0 ]'
  # the control: the same kind of row in a hub (C) is dialled
  stop_node C
  jq --argjson r "$row_c" '.paired_hubs = ((.paired_hubs // []) + [$r])' "$W/c-settings.bak" >"${ROOT[C]}/fabric/settings"
  chmod 600 "${ROOT[C]}/fabric/settings"
  t0=$(now)
  start_node C
  wait_up C
  wait_for 25 'cap_report; [ "$(jq -r --argjson a "$t0" "[.arp[] | select(.[0] >= \$a and .[2] == \"192.0.2.99\")] | length" "$W/rep.json")" -ge 1 ]'
  chk "the control dialled within the non-hub's watch time: the hub's first ARP for its injected row came within 12 s of its start" \
    '[ "$(jq -r --argjson a "$t0" "[.arp[] | select(.[0] >= \$a and .[2] == \"192.0.2.99\") | .[0] - \$a] | first // 999" "$W/rep.json" | cut -d. -f1)" -le 12 ]'
  note "the non-hub's gate (a put toward M2 while it holds the injected row) is not driven here: it is a mesh111 unit matter (cargo test -p mesh111 --test forward), declared not covered by this gate"
  note "RECORD the hub's first dial of the injected row: ARP who-has $(jq -r --argjson a "$t0" '[.arp[] | select(.[0] >= $a and .[2] == "192.0.2.99") | .[0] - $a] | first // "never" | tostring' "$W/rep.json") s after its start (the non-hub above was watched for 12 s)"
  chk "control: the same row in a hub (C, address 192.0.2.99) is dialled: ARP who-has seen on the TEST-NET veth, so the tap and the row can fail the check above" \
    '[ "$(jq -r --argjson a "$t0" "[.arp[] | select(.[0] >= \$a and .[2] == \"192.0.2.99\")] | length" "$W/rep.json")" -ge 1 ]'
  stop_node C
  cp "$W/c-settings.bak" "${ROOT[C]}/fabric/settings"
  start_node C
  wait_up C
  stop_node M2
  cp "$W/m2-settings.bak" "${ROOT[M2]}/fabric/settings"
  start_node M2
  wait_up M2
  wait_for "$WAITS" 'pair_whole A C && pair_whole A B' || fail "the pairs did not return after the control"
  # (b) a deleted fabric/settings never rebuilds a paired hub as the home hub; a member's does (the control)
  stop_node M2
  rm -f "${ROOT[M2]}/fabric/settings"
  start_node M2
  wait_up M2
  wait_for 30 'api M2 GET /api/v1/fabric; jb ".hub.enrolled == true"'
  api M2 GET /api/v1/fabric
  chk "control: M2's deleted fabric/settings is rebuilt from its own announce, which names A: home hub A again ($(jget -r '.hub.node_id // "none"' | cut -c1-8))" '[ "$(jget -r ".hub.node_id // \"none\"")" = "${NID[A]}" ]'
  cp "$W/m2-settings.bak" "$W/m2-settings.restore"
  stop_node A
  cp "${ROOT[A]}/fabric/settings" "$W/a-settings.bak"
  rm -f "${ROOT[A]}/fabric/settings"
  start_node A
  wait_up A
  sleep 6
  api A GET /api/v1/fabric
  chk "A's fabric/settings deleted: A, which is enrolled at B and C by its pairings, has no home hub (enrolled $(jget -r .hub.enrolled), node_id $(jget -r '.hub.node_id // "none"'))" '[ "$(jget -r .hub.enrolled)" = false ] && [ "$(jget -r ".hub.node_id // \"none\"")" = none ]'
  stop_node A
  cp "$W/a-settings.bak" "${ROOT[A]}/fabric/settings"
  chmod 600 "${ROOT[A]}/fabric/settings"
  start_node A
  wait_up A
  wait_for "$WAITS" 'pair_whole A C && pair_whole A B' || fail "the pairs of A did not come back after the settings were restored"
  # (b2) the same with exactly one pair: the one candidate the rebuild could pick is a paired hub and must not be
  unpair A B
  stop_node A
  cp "${ROOT[A]}/fabric/settings" "$W/a1-settings.bak"
  rm -f "${ROOT[A]}/fabric/settings"
  start_node A
  wait_up A
  sleep 6
  api A GET /api/v1/fabric
  chk "A with exactly one pair (C) and its fabric/settings deleted: no home hub (enrolled $(jget -r .hub.enrolled), node_id $(jget -r '.hub.node_id // "none"')): the only hub A knows is a paired hub, never picked" '[ "$(jget -r .hub.enrolled)" = false ] && [ "$(jget -r ".hub.node_id // \"none\"")" = none ]'
  stop_node A
  cp "$W/a1-settings.bak" "${ROOT[A]}/fabric/settings"
  chmod 600 "${ROOT[A]}/fabric/settings"
  start_node A
  wait_up A
  pair_hubs A B || fail "A <-> B did not come back after the single-pair case"
  wait_for "$WAITS" 'pair_whole A C && pair_whole A B' || fail "the pairs of A did not come back after the single-pair case"
  # (c) a half pair forwards nothing. A real half pair cannot be made through the API (a hub holds one enrolment attempt, and
  # a scan whose far side has not pinned the scanner's key lands no record: the far hub closes the dial), so A's persisted
  # record of C is made half: its `theirs` (C enrolled here) is set false while A is stopped, the shape the daemon writes
  # for a pair whose second enrolment failed. C's own record of A stays whole.
  stop_node A
  jq --arg f "${NID[C]}" '(.paired_hubs[] | select(.node_id == $f) | .theirs) = false' "${ROOT[A]}/fabric/settings" >"$W/a-half.json" && cp "$W/a-half.json" "${ROOT[A]}/fabric/settings"
  chmod 600 "${ROOT[A]}/fabric/settings"
  start_node A
  wait_up A
  if wait_for 30 'pair_rec_half A C'; then
    ok "the half pair exists: A's persisted record of C has exactly one of ours / theirs true ($(jq -c --arg f "${NID[C]}" '[.paired_hubs[]? | select(.node_id == $f) | {ours, theirs}]' "${ROOT[A]}/fabric/settings")): half, as the daemon kept it across the restart"
  else
    fail "no half pair at A after the restart: A's record of C is $(jq -c --arg f "${NID[C]}" '[.paired_hubs[]? | select(.node_id == $f) | {ours, theirs}]' "${ROOT[A]}/fabric/settings")"
  fi
  sleep "$SETTLE"
  r0=$(refused_of A)
  txt=$(mtext g9-7c "$TEXT_LEN")
  t0=$(now)
  send M "${NID[E]}" "$txt"; mid=$MID
  wait_state M "${NID[E]}" "$mid" "relayed -" 15
  wait_for 20 '[ "$(refused_of A)" -ge "$((r0 + 1))" ]'
  settle
  t1=$(now)
  chk "a half pair forwards nothing: A's only whole pair (B) refused and the row is refused (forward_refused $r0 -> $(refused_of A)); nothing pending toward C ($(pending_of A C): the half pair, shown above to exist) and no put-sized record A -> C ($(big_count "${IPS[A]}" "${IPS[C]}" "$t0" "$t1" "$PUTMIN"))" \
    '[ "$(refused_of A)" -ge "$((r0 + 1))" ] && { [ "$(pending_of A C)" = 0 ] || [ "$(pending_of A C)" = none ]; } && [ "$(big_count "${IPS[A]}" "${IPS[C]}" "$t0" "$t1" "$PUTMIN")" = 0 ] && [ "$(big_count "${IPS[A]}" "${IPS[B]}" "$t0" "$t1" "$PUTMIN")" -ge 1 ]'
  unpair A C
  pair_hubs A C || fail "A <-> C did not come back after the half pair"
  # the pairing window: A scans C's bundle before C has pinned A's key (C never scans A's). No record comes to exist; what the
  # wire shows is the check below: C must close A's TLS dial before any 111 byte, not answer it in clear
  unpair A C
  api C POST /api/v1/relay/pair/offer '{"role":"peer","via":"tcp"}'
  local bc
  bc=$(bundle_set "$(jget .bundle.text)" address "${IPS[C]}:$PORT")
  api A POST /api/v1/relay/pair "$(jq -nc --arg b "$bc" '{bundle: $b, role: "peer", via: "tcp"}')"
  note "A scanned C's bundle before C pinned A's key (HTTP $ST), C never scans A's: the pairing window, 8 s"
  sleep 8
  unpair A C
  pair_hubs A C || fail "A <-> C did not come back after the pairing-window step"
  # (d) no plaintext 111F hello on the TEST-NET veth, the whole run; the detector's control
  cap_report
  local wan_flows plain_first leaks
  local wanf='(.s == $a or .s == $b or .s == $c) and (.d == $a or .d == $b or .d == $c)'
  wan_flows=$(jq -r --arg a "${IPS[A]}" --arg b "${IPS[B]}" --arg c "${IPS[C]}" "[.flows[] | select($wanf)] | length" "$W/rep.json")
  # a hub-to-hub connection whose dialling side starts with anything but a TLS record (0x16): a plain link
  plain_first=$(jq -r --arg a "${IPS[A]}" --arg b "${IPS[B]}" --arg c "${IPS[C]}" "[.flows[] | select($wanf and .dp == 11183 and .first != null and .first != 22)] | length" "$W/rep.json")
  # a TLS dial answered with the 111F hello (and the announce) in clear by the listening hub
  leaks=$(jq -r --arg a "${IPS[A]}" --arg b "${IPS[B]}" --arg c "${IPS[C]}" '.flows as $all | [.flows[] | select((.s == $a or .s == $b or .s == $c) and (.d == $a or .d == $b or .d == $c) and .sp == 11183 and (.hello or .first == 49)) | . as $r | select([$all[] | select(.s == $r.d and .sp == $r.dp and .d == $r.s and .dp == $r.sp and .first == 22)] | length > 0)] | length' "$W/rep.json")
  chk "no plain hub-to-hub link on TEST-NET: of $wan_flows hub-to-hub flows captured in the run, $plain_first were dialled by a side that starts with anything but a TLS record (0x16): no plaintext 111F hello left a dialling hub" '[ "$wan_flows" -ge 6 ] && [ "$plain_first" = 0 ]'
  chk "no TLS dial was answered in clear: a hub that held no pin yet for the dialler (the pairing window: the first scanner dials before the far owner has scanned) must close before any 111 byte; $leaks flow(s) of the listening hub began with the 111F hello and its announce" '[ "$leaks" = 0 ]'
  t0=$(now)
  nx C python3 -I -c 'import socket,sys
s = socket.create_connection((sys.argv[1], int(sys.argv[2])), 5)
s.settimeout(1.5)
s.sendall(b"111F\x01")
d = b""
try:
    while len(d) < 4096:
        c = s.recv(4096 - len(d))
        if not c: break
        d += c
except OSError:
    pass
print(len(d))' "${IPS[B]}" "$PORT" >"$W/clear-control.out" 2>&1
  sleep 1
  cap_report
  local cc
  cc=$(jq -r --arg a "${IPS[A]}" --arg b "${IPS[B]}" --arg c "${IPS[C]}" --argjson t "$t0" "[.flows[] | select($wanf and .dp == 11183 and .t0 >= \$t and .first != null and .first != 22)] | length" "$W/rep.json")
  chk "control: a client that dials B from TEST-NET and starts with the 111F hello IS seen by the plain-link detector ($cc flow; B answered $(cat "$W/clear-control.out") bytes), so a plain hub-to-hub link would have failed the check above" '[ "$cc" -ge 1 ]'
  # (e) records per paired hub
  local held
  held=$(nx A ss -tnH state established 2>/dev/null | awk '{ print $4, $3 }' | grep -c '192\.0\.2\.')
  note "RECORD held connections: A holds $held established TCP connection(s) on the TEST-NET interface (2 pairs; one per pair expected)"
  if [ "${WINDOW:-0}" -gt 0 ]; then
    local w0 w1 vtot0 vtot1 rate
    vtot0=$(vcs_sum "${DPID[A]}")
    t0=$(now)
    sleep "$WINDOW"
    t1=$(now)
    vtot1=$(vcs_sum "${DPID[A]}")
    rate=$(awk -v a="$vtot0" -v b="$vtot1" -v w="$WINDOW" 'BEGIN { printf "%.1f", (b - a) * 60 / w }')
    note "RECORD wakeups: A (2 paired hubs, 2 members, idle) $rate wakeups/minute over ${WINDOW} s; with no pair (item 1) ${BASE_RATE:-not measured} wakeups/minute"
    cap_report
    local kb_ab kb_ba
    kb_ab=$(jq -r --arg a "${IPS[A]}" --arg b "${IPS[B]}" --argjson t0 "$t0" --argjson t1 "$t1" '[.flows[] | select(.s == $a and .d == $b) | .rec[] | select(.[0] >= $t0 and .[0] <= $t1) | .[2] + 5] | add // 0' "$W/rep.json")
    kb_ba=$(jq -r --arg a "${IPS[A]}" --arg b "${IPS[B]}" --argjson t0 "$t0" --argjson t1 "$t1" '[.flows[] | select(.s == $b and .d == $a) | .rec[] | select(.[0] >= $t0 and .[0] <= $t1) | .[2] + 5] | add // 0' "$W/rep.json")
    note "RECORD keepalive bytes (TLS records, headers of TCP and IP not counted): A -> B $kb_ab B and B -> A $kb_ba B in ${WINDOW} s = $(awk -v a="$kb_ab" -v b="$kb_ba" -v w="$WINDOW" 'BEGIN { printf "%.0f", (a + b) * 86400 / w }') B a day both ways for the one pair; plain (a lan = true pair) not measured here (no plain non-private pair idles in this run; G11 records the TLS session too)"
  fi
}
vcs_sum() { # PID: the summed voluntary context switches of every thread
  local t c s=0
  for t in /proc/"$1"/task/*; do
    c=$(awk '/^voluntary_ctxt_switches:/ { print $2 }' "$t/status" 2>/dev/null) || continue
    s=$((s + ${c:-0}))
  done
  echo "$s"
}

# ------------------------------------------------------------------ item 6, the old hub: a Layer 6 build as hub B
item6old() {
  note "---- item 6 (the old hub): a Layer 6 build as B skips the hint TLV and answers as today"
  if [ -z "${G9_OLD_BIN:-}" ]; then
    if [ "${REQUIRE_G9:-0}" = 1 ]; then
      fail "item 6, old hub B: ${G9_OLD_WHY:-no old binary} (REQUIRE_G9=1)"
    else
      printf 'G9 SKIPPED (item 6, the old hub B): %s\n' "${G9_OLD_WHY:-no old binary}"
      note "SKIPPED item 6, the old hub B: ${G9_OLD_WHY:-no old binary}"
    fi
    return
  fi
  local OB=$G9_OLD_BIN rootold=$W/root-Bold t0 t1 txt mid abundle acode acard bcode bbundle st lm lf
  stop_node B
  NID[B0]=${NID[B]}
  ROOT[B]=$rootold
  g6_make_root "$rootold" ""
  NID[B]=$(g6_identity "$rootold" "$OB")
  [ -n "${NID[B]}" ] || {
    fail "the old hub has no identity ($(head -c 200 "$rootold/identity.log"))"
    return
  }
  g6_fabric_settings "$rootold" '{"schema":1,"hub_role":true,"listen_lan":true}'
  start_node B "$OB"
  if wait_up B; then ok "the old hub B is up: a Layer 6 build ($OLD_REV; $("$OB" --version 2>&1 | head -1)), id ${NID[B]:0:8}, in B's netns"; else fail "the old hub B did not come up"; return; fi
  # D enrols at the old hub; M re-reads D's card (the hint now names the old hub)
  CARD_D0=$(card_of D)
  api D GET /api/v1/fabric
  local oldhome
  oldhome=$(jget '.hub.node_id // empty')
  api D DELETE "/api/v1/contacts/$oldhome"
  note "D forgot its home hub ${oldhome:0:8} (DELETE /contacts: HTTP $ST) to enrol at the old hub"
  enrol D B || return
  if wait_for 10 '[ "$(card_of D)" != "$CARD_D0" ]'; then ok "D's card changed after it enrolled at the old hub (the hint now names the old hub)"; else fail "D's card did not change after it enrolled at the old hub: M's put would carry no hint"; fi
  add_contact M "$(card_of D)"
  chk "M took D's new card: POST /contacts answered 200 or 201 (got $ST)" '[ "$ST" = 200 ] || [ "$ST" = 201 ]'
  # I13: the old hub's frames to a member that puts for a contact it does not hold equal the tape. Taken before the
  # pairing: the old build's ENROLL_OK gives the hub a home hub (A) and an 8 B hint in its announce (149 B, not 141 B)
  tape_run D B "$L7_G9_LAN_B_IP_B" "$L7_G9_IP_D" "${NID[M]}"
  chk "an old hub answers a non-member put 'sent hub_not_yours' (D reads '$TAPE_ST')" '[ "$TAPE_ST" = "sent hub_not_yours" ]'
  tape_compare "$W/tape.B.now" "the Layer 6 build's frames to a member (D at the old hub)"
  # pairing with a hub that cannot pair: the old hub enrols at A with A's pair code (its ENROLL redeems the code),
  # and A scans a bundle built by hand from the old hub's own offer, marked lan (plain: the old build has no TLS)
  api A DELETE "/api/v1/relay/paired/${NID[B0]}"
  note "A forgot its pair with the head-build B ${NID[B0]:0:8} (HTTP $ST): the old hub takes its address"
  api A POST /api/v1/relay/pair/offer '{"role":"peer","via":"tcp"}'
  abundle=$(jget .bundle.text)
  IFS=: read -r _ acard acode _ <<<"$abundle"
  api B POST /api/v1/fabric/hub/enroll "$(jq -nc --arg a "${IPS[A]}:$PORT" --arg c "$acode" --arg k "$(card_of A)" '{address: $a, code: $c, card: $k}')"
  note "the old hub enrolled at A with A's pairing code: HTTP $ST"
  api B POST /api/v1/fabric/hub/offer '{}'
  bcode=$(jget .code)
  bbundle="111PAIR:$(card_b32 "$(jget .card.text)"):$bcode::${IPS[B]}:$PORT:peer:tcp"
  api A POST /api/v1/relay/pair "$(jq -nc --arg b "$bbundle" '{bundle: $b, role: "peer", via: "tcp", lan: true}')"
  note "A scanned the old hub's hand-built bundle (lan = true): HTTP $ST $(head -c 160 "$W/body")"
  if wait_for "$WAITS" '[ "$(reach_of A B)" = true ] && pair_rec_whole A B'; then ok "A's pair with the old hub is whole (both enrolments landed: the old hub redeemed A's pairing code, A its offer) and reachable, plain (lan = true: the old build has no TLS)"; else
    fail "A's pair with the old hub never became reachable"
    relay_get A
    note "A relay: $(jget -c .paired_hubs)"
    return
  fi
  txt=$(mtext g9-6old "$TEXT_LEN")
  local rr
  rr=$(refused_of A)
  t0=$(now)
  send M "${NID[D]}" "$txt"; mid=$MID
  wait_for 30 '[ "$(pending_of A B)" = 0 ] && [ "$(recv_count D "${NID[M]}" "$txt")" = 1 ]'
  settle
  t1=$(now)
  chk "the old hub skipped the unknown TLV and answered as today: A's row committed (pending $(pending_of A B)), D has the message once ($(recv_count D "${NID[M]}" "$txt")), nothing refused" \
    '[ "$(pending_of A B)" = 0 ] && [ "$(recv_count D "${NID[M]}" "$txt")" = 1 ] && [ "$(refused_of A)" = "$rr" ]'
  lm=$(put_len "$L7_G9_IP_M" "$L7_G9_LAN_A_IP_A" "$t0" "$t1")
  lf=$(put_len "${IPS[A]}" "${IPS[B]}" "$t0" "$t1")
  chk "the TLV was on the wire to the old hub: A's put to it is as long as M's put to A ($lf B and $lm B: A repeats what it received), and M's put is 11 B longer than an unhinted put of the same text length (reference ${NOHINT2_PUT_LEN:-not measured: item 3 did not run}), so those 11 B were the hint" \
    '[ "$lm" -gt 0 ] && [ "$lf" = "$lm" ] && { [ -z "${NOHINT2_PUT_LEN:-}" ] || [ "$lm" = "$((NOHINT2_PUT_LEN + 11))" ]; }'
  chk "the old hub has no forwarding: D's ACK for M gets no further and M stays 'relayed' ($(mstate M "${NID[D]}" "$mid")); that M's put carried the hint is shown by the length line above, not by this one" '[ "$(mstate M "${NID[D]}" "$mid")" = "relayed -" ] || [ "$(mstate M "${NID[D]}" "$mid")" = "relayed forwarding" ]'
  # the detector's second control: the plain pair is visible on the TEST-NET veth
  cap_report
  local plain
  plain=$(jq -r --arg a "${IPS[A]}" --arg b "${IPS[B]}" '[.flows[] | select(((.s == $a and .d == $b) or (.s == $b and .d == $a)) and .hello)] | length' "$W/rep.json")
  chk "the detector sees the plain pair: $plain flow(s) between A and the old hub carry the 111F hello in clear" '[ "$plain" -ge 1 ]'
}
card_b32() { # a card text -> the base32 field of a pairing bundle (the text with its prefix removed)
  local t=$1
  printf '%s' "${t#*:}"
}

# a dev aid (G9_PAUSE=1): after the pairing the run waits and evals $W/cmd.<n> files (in this shell, with every helper)
# until $W/cmd.quit exists; the output of each lands in $W/out.<n>. Never part of a lock run.
pause_loop() {
  local i=1
  note "pause: eval $W/cmd.<n> files until $W/cmd.quit exists"
  while [ ! -e "$W/cmd.quit" ]; do
    if [ -f "$W/cmd.$i" ]; then
      eval "$(cat "$W/cmd.$i")" >"$W/out.$i" 2>&1
      : >"$W/done.$i"
      i=$((i + 1))
    fi
    sleep 0.4
  done
}
run_items() {
  setup_members || return
  case " $ITEMS " in *" 1 "*) item1 ;; esac
  case " $ITEMS " in *" 2 "* | *" 3 "* | *" 4 "* | *" 5 "* | *" 6 "* | *" 6o "* | *" 7 "*) pair_world || return ;; esac
  local i
  [ -n "${G9_PAUSE:-}" ] && pause_loop
  for i in 2 3 4 5 6 7; do
    case " $ITEMS " in *" $i "*) "item$i" ;; esac
  done
  case " $ITEMS " in *" 6 "* | *" 6o "*) item6old ;; esac
}
if [ "${1:-}" = --inner ]; then
  inner
  printf 'FAILS=%d\n' "$FAILS" >"${G9_WORK:?}/inner.result"
  exit 0
fi

# ------------------------------------------------------------------ outer
for tool in python3 ip unshare awk jq curl nsenter nft ss git; do
  command -v "$tool" >/dev/null 2>&1 || skip_or_fail G9 "$tool is absent"
done
have_netns || skip_or_fail G9 "unprivileged network namespaces are not available (unshare -Urnm)"
unshare --user --map-root-user --net --pid --fork --mount-proc true >/dev/null 2>&1 ||
  skip_or_fail G9 "a pid namespace with its own /proc is not available (unshare --pid --fork --mount-proc)"
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
if [ -z "$BIN111" ] || [ ! -x "$BIN111" ]; then skip_or_fail G9 "no built 111 binary (set BIN111 or CARGO_TARGET_DIR)"; fi
BIN111=$(cd "$(dirname "$BIN111")" && pwd)/$(basename "$BIN111")

WORK=$(mktemp -d "${TMPDIR:-/tmp}/l7-g9.XXXXXX") || exit 1
cleanup() {
  if [ "${G9_KEEP:-0}" = 1 ]; then note "kept $WORK"; else rm -rf "${WORK:?}"; fi
}
trap cleanup EXIT

if env --default-signal true 2>/dev/null; then
  RESET_SIG=(env --default-signal)
else
  RESET_SIG=(python3 -c 'import os,signal,sys
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])')
fi
# item 6's old hub: a Layer 6 build exported from $OLD_REV (git archive) and built once with its own CARGO_TARGET_DIR
G9_OLD_BIN=${G9_OLD_BIN:-}
G9_OLD_WHY=""
if [[ " $ITEMS " == *" 6 "* || " $ITEMS " == *" 6o "* ]] && [ -z "$G9_OLD_BIN" ]; then
  if ! old_sha=$(git -C "$L7_OS" rev-parse --verify -q "$OLD_REV^{commit}" 2>/dev/null); then
    G9_OLD_WHY="the revision '$OLD_REV' does not resolve in this clone (set G9_OLD_REV, e.g. 561cb5e, the last commit before the Layer 7 scaffold)"
  elif ! command -v cargo >/dev/null 2>&1; then
    G9_OLD_WHY="cargo is absent: the Layer 6 hub cannot be built from $OLD_REV"
  else
    old_target=${G9_OLD_TARGET:-$WORK/old-target}
    mkdir -p "$old_target"
    avail_kb=$(df -Pk "$old_target" | awk 'NR == 2 { print $4 }')
    if [ -x "$old_target/debug/111" ] && [ "$(cat "$old_target/.g9-rev" 2>/dev/null)" = "$old_sha" ]; then
      G9_OLD_BIN=$old_target/debug/111
      note "item 6: the Layer 6 hub of $OLD_REV is already built ($G9_OLD_BIN)"
    elif [ "${avail_kb:-0}" -lt 4194304 ]; then
      G9_OLD_WHY="less than 4 GB of disk is free ($((avail_kb / 1024)) MB): the Layer 6 hub is not built"
    else
      old_src=${G9_OLD_SRC:-$WORK/old}
      if [ ! -d "$old_src/1-bootstrap" ]; then
        mkdir -p "$old_src"
        git -C "$L7_OS" archive "$old_sha" | tar -x -C "$old_src"
      fi
      note "item 6: building the Layer 6 hub from $OLD_REV ($old_sha, exported to $old_src) into $old_target (cargo build -p node111 --bin 111, 2 jobs)"
      if (cd "$old_src/1-bootstrap" && env -u TMPDIR CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${G9_OLD_JOBS:-2}" CARGO_PROFILE_DEV_DEBUG=0 CARGO_TARGET_DIR="$old_target" \
        cargo build -p node111 --bin 111 --offline >"$WORK/old-build.log" 2>&1 ||
        env -u TMPDIR CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${G9_OLD_JOBS:-2}" CARGO_PROFILE_DEV_DEBUG=0 CARGO_TARGET_DIR="$old_target" \
          cargo build -p node111 --bin 111 >>"$WORK/old-build.log" 2>&1) && [ -x "$old_target/debug/111" ]; then
        printf '%s\n' "$old_sha" >"$old_target/.g9-rev"
        G9_OLD_BIN=$old_target/debug/111
      else
        G9_OLD_WHY="the Layer 6 hub of $OLD_REV did not build (see $WORK/old-build.log: $(tail -n 2 "$WORK/old-build.log" | tr '\n' ' '))"
      fi
      [ -z "${G9_OLD_SRC:-}" ] && rm -rf "$WORK/old"
    fi
  fi
fi
INNER_LOG=$WORK/inner.log
# the namespace run goes in the background so a TERM or INT sent to this script alone ends it too (--kill-child
# fires only when the unshare process itself dies)
G9_WORK=$WORK G9_BIN=$BIN111 G9_OLD_BIN=$G9_OLD_BIN G9_OLD_WHY=$G9_OLD_WHY G9_OLD_REV=$OLD_REV \
  unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
  "${RESET_SIG[@]}" bash "$0" --inner >"$INNER_LOG" 2>&1 &
UNSHARE_PID=$!
trap 'kill -TERM "$UNSHARE_PID" 2>/dev/null; wait "$UNSHARE_PID" 2>/dev/null; cleanup; exit 130' INT TERM
tail -n +1 -f "$INNER_LOG" --pid="$UNSHARE_PID" &
TAIL_PID=$!
wait "$UNSHARE_PID" || true
wait "$TAIL_PID" 2>/dev/null || true
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
if [ -z "$left" ]; then ok "no daemon, tap or stand-in of this run is left"; else fail "left running:$left"; fi
finish G9
