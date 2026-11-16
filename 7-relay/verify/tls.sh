#!/usr/bin/env bash
# 7-relay/verify/tls.sh — Layer 7 gate G11 (spec §15 G11; §4.1–§4.7; row V5):
#   TLS 1.3 raw public keys for paired-hub links; real `111 daemon`s on TEST-NET in network namespaces.
#
#   bash 7-relay/verify/tls.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G11=1 bash 7-relay/verify/tls.sh    lock run: a missing prerequisite is a FAIL
#   REQUIRE_OPENSSL_RPK=1 ...                   the OpenSSL >= 3.2 peer items become a FAIL where they cannot run
#
# One user + net + pid namespace (`unshare -rnpf`, so everything dies with it) holds a bridge l7g11br0 and four
# network namespaces on 203.0.113.0/24 (RFC 5737 TEST-NET-3, so §4.1 does not count the links private):
#   X .11 and Y .12  the two hubs of the pair      Z .13  a plain member of X (§4.1 rule 3)
#   Q .14            a third hub whose key X never pinned
# Each runs the real `111 daemon` (its own NODE111_ROOT, hub_role and listen_lan on). Everything is driven through
# each node's loopback /api/v1 (curl via nsenter). The wire is read by an AF_PACKET tap on each node's veth end
# (inline below: tcpdump is absent, and 6-transport/verify/egress-log.py keeps no TCP payload): every TCP segment
# of port 11183 is kept, the streams are re-assembled per connection, and the TLS records, the ClientHello's
# extensions, the first byte and a search for each hub's own announce bytes (its `fabric/announce`) are read.
#
# What each part proves, and how a broken daemon turns it FAIL:
#   * the handshake (ClientHello versions = TLS 1.3 only, signature_algorithms = Ed25519 only, client_ and
#     server_certificate_type = raw public key only, no SNI; ServerHello = TLS 1.3; the first byte 0x16; no `111F`
#     hello and no announce of either hub in clear): a daemon that ran plain on TEST-NET shows 0x31 and the hello.
#   * the refusals (a wrong client key, a wrong server key, a matched pin with another NodeId, an X.509 certificate
#     with an Ed25519 and with an ECDSA key, TLS 1.2): a daemon that accepted any of them would show an attributed
#     peer, a pair that turns whole, or (the python clients, which decrypt their own session) 111 bytes received.
#     A control (a python TLS server on loopback) proves the clients can complete a handshake.
#   * plain inbound: the hello, the announce and a real member's ENROLL all in clear on the same listener, with a pin
#     present, and the same with no pin at all: the listener peeks the first byte of every inbound connection (§4.1
#     rule 2; row D5's follow-up), so a silent connection gets no byte and a 111F hello gets the Layer 5 wire.
#   * plain outbound to a non-private pair: 409 consent_required, no SYN toward the far hub, nothing recorded;
#     after the hash is accepted, the pair runs plain and its hello is visible in clear (the detector's positive
#     control too), the hash is kept in fabric/settings.
#
# G11_NEG=lan is the negative control: the honest pairing is marked lan = true (plain on TEST-NET), and the run must FAIL.
# Env: BIN111 (the `111` binary; else $CARGO_TARGET_DIR/{release,debug}/111) · G11_WINDOW (100 s: the idle
# window of the keepalive record; a dev run may shorten it) · G11_SETTLE (8 s: how long a refused dial is watched)
# · TMPDIR for the scratch root · G11_KEEP=1 keeps it. Nothing of ours stays running.
# the jq filters and the wait_for conditions are single-quoted on purpose (jq variables, eval'd conditions)
# shellcheck disable=SC2016,SC2034
set -u
export LC_ALL=C
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=7-relay/verify/lib.sh disable=SC1091
. 7-relay/verify/lib.sh

WINDOW=${G11_WINDOW:-100}
SETTLE=${G11_SETTLE:-8}
PORT=$L7_PORT_FABRIC

# the warning text of §4.6 step 6, verbatim
PAIR_PLAIN_TEXT="This hub is on the open internet and this link cannot be encrypted. Anyone on the path between the two hubs can see which hubs talk, when, and how much. Message contents stay sealed. Pair anyway?"

# ------------------------------------------------------------------ the tap and the stream reader (python, stdlib)
write_tools() { # DIR
  cat >"$1/cap.py" <<'PY'
# cap.py sniff OUTDIR IFACE...   : AF_PACKET tap, TCP segments of port 11183 -> OUTDIR/segs.json on SIGTERM
# cap.py report OUTDIR NAME=ANNOUNCEFILE...  : the per-connection facts as JSON on stdout (a name may repeat: one file per version)
import json, os, select, signal, socket, struct, sys

PORT = 11183
HELLO = b"111F\x01"


def sniff(outdir, ifaces):
    socks = []
    for i in ifaces:
        s = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(3))
        s.bind((i, 0))
        socks.append(s)
    segs = []
    stop = []
    signal.signal(signal.SIGTERM, lambda *a: stop.append(1))
    signal.signal(signal.SIGINT, lambda *a: stop.append(1))
    open(os.path.join(outdir, "ready"), "w").close()
    while not stop:
        try:
            r, _, _ = select.select(socks, [], [], 0.2)
        except InterruptedError:
            continue
        for s in r:
            data, addr = s.recvfrom(262144)
            if addr[2] == 4:  # PACKET_OUTGOING: each frame is read once, at its sender's end
                continue
            if len(data) < 54 or data[12:14] != b"\x08\x00" or data[23] != 6:
                continue
            ihl = (data[14] & 15) * 4
            tot = struct.unpack("!H", data[16:18])[0]
            src, dst = socket.inet_ntoa(data[26:30]), socket.inet_ntoa(data[30:34])
            t = 14 + ihl
            sport, dport, seq = struct.unpack("!HHI", data[t:t + 8])
            flags = data[t + 13]
            off = (data[t + 12] >> 4) * 4
            if PORT not in (sport, dport):
                continue
            segs.append([round(__import__("time").monotonic(), 3), src, sport, dst, dport, seq, flags, data[t + off:14 + tot].hex()])
    with open(os.path.join(outdir, "segs.json"), "w") as f:
        json.dump(segs, f)


def assemble(items):  # [(seq, bytes)] -> bytes
    items = sorted((s, p) for s, p in items if p)
    out, cur = b"", None
    for seq, p in items:
        if cur is None:
            cur = seq
        end = seq + len(p)
        if end <= cur:
            continue
        out += p[max(0, cur - seq):]
        cur = end
    return out


def records(b):
    out, i = [], 0
    while i + 5 <= len(b):
        n = struct.unpack("!H", b[i + 3:i + 5])[0]
        out.append((b[i], b[i + 5:i + 5 + n]))
        i += 5 + n
    return out


def exact(b):  # True when b is a whole number of TLS records (no garbage, no short tail)
    i = 0
    while i + 5 <= len(b):
        i += 5 + struct.unpack("!H", b[i + 3:i + 5])[0]
    return i == len(b) and len(b) > 0


def exts(b):
    out, i = {}, 0
    while i + 4 <= len(b):
        t, n = struct.unpack("!HH", b[i:i + 4])
        out[t] = b[i + 4:i + 4 + n]
        i += 4 + n
    return out


def hello(rec, server):
    """parse the first handshake message of a record payload: a ClientHello or a ServerHello"""
    try:
        if rec[0] != (2 if server else 1):
            return None
        b = rec[4:]
        i = 2 + 32
        i += 1 + b[i]
        res = {}
        if server:
            res["cipher"] = struct.unpack("!H", b[i:i + 2])[0]
            i += 3
        else:
            n = struct.unpack("!H", b[i:i + 2])[0]
            i += 2 + n
            i += 1 + b[i]
        n = struct.unpack("!H", b[i:i + 2])[0]
        e = exts(b[i + 2:i + 2 + n])
        sv = e.get(43, b"")
        if server:
            res["version"] = struct.unpack("!H", sv[:2])[0] if len(sv) >= 2 else None
        else:
            res["versions"] = [struct.unpack("!H", sv[1 + k:3 + k])[0] for k in range(0, sv[0], 2)] if sv else []
            sa = e.get(13, b"")
            res["sigalgs"] = [struct.unpack("!H", sa[2 + k:4 + k])[0] for k in range(0, len(sa) - 2, 2)]
            res["client_cert_types"] = list(e[19][1:]) if 19 in e else None
            res["server_cert_types"] = list(e[20][1:]) if 20 in e else None
            res["sni"] = 0 in e
        return res
    except Exception:
        return None


def frames(b):  # the 111 plain wire: hello, then type(1) len(2) body
    out = []
    if b[:5] != HELLO:
        return out
    i = 5
    while i + 3 <= len(b) and len(out) < 8:
        n = struct.unpack("!H", b[i + 1:i + 3])[0]
        out.append([b[i], n])
        i += 3 + n
    return out


def report(outdir, ann):
    segs = json.load(open(os.path.join(outdir, "segs.json")))
    announces = []  # (hub name, bytes): every version of each hub's own announce sampled during the run
    for a in ann:
        name, path = a.split("=", 1)
        try:
            announces.append((name, open(path, "rb").read()))
        except OSError:
            pass
    flows, syns = {}, {}
    for t, src, sport, dst, dport, seq, flags, hx in segs:
        if flags & 2 and not flags & 16:
            syns["%s>%s" % (src, dst)] = syns.get("%s>%s" % (src, dst), 0) + 1
        if dport == PORT and sport != PORT:
            key, d = (src, sport, dst), "c"
        elif sport == PORT and dport != PORT:
            key, d = (dst, dport, src), "s"
        else:
            continue
        f = flows.setdefault(key, {"c": [], "s": [], "t0": t, "t1": t, "fin": False, "rst": False})
        f[d].append((seq, bytes.fromhex(hx)))
        f["t1"] = t
        f["fin"] = f["fin"] or bool(flags & 1)
        f["rst"] = f["rst"] or bool(flags & 4)
    out = []
    for (cip, cport, sip), f in sorted(flows.items(), key=lambda kv: kv[1]["t0"]):
        c, s = assemble(f["c"]), assemble(f["s"])
        cr, sr = records(c), records(s)
        r = {
            "cip": cip, "cport": cport, "sip": sip, "t0": f["t0"], "dur": round(f["t1"] - f["t0"], 2),
            "fin": f["fin"], "rst": f["rst"], "c2s": len(c), "s2c": len(s),
            "c2s_first": c[0] if c else None, "s2c_first": s[0] if s else None,
            "clear_hello": HELLO in c or HELLO in s,
            "clear_announces": sorted({n for n, a in announces if a and (a in c or a in s)}),
            "c2s_types": sorted({x[0] for x in cr}), "s2c_types": sorted({x[0] for x in sr}),
            "c2s_exact": exact(c), "s2c_exact": exact(s),
            "c2s_rec": [x[0] for x in cr][:12], "s2c_rec": [x[0] for x in sr][:12],
            "ch": hello(cr[0][1], False) if cr and cr[0][0] == 0x16 else None,
            "sh": hello(sr[0][1], True) if sr and sr[0][0] == 0x16 else None,
            "ch_len": (5 + len(cr[0][1])) if cr and cr[0][0] == 0x16 else None,
            "c2s_frames": frames(c), "s2c_frames": frames(s),
        }
        out.append(r)
    json.dump({"flows": out, "syns": syns}, sys.stdout)


if sys.argv[1] == "sniff":
    sniff(sys.argv[2], sys.argv[3:])
else:
    report(sys.argv[2], sys.argv[3:])
PY
  cat >"$1/client.py" <<'PY'
# client.py tls IP PORT MAXV [CERT KEY] : a TLS client (python ssl, X.509 only: no raw public keys here); prints one line
# client.py listen IP PORT              : connect, send nothing, print what the server says first within 1.5 s
# client.py hello IP PORT               : connect, send the 111F hello, print the first bytes back
# client.py control CERT KEY MAXV       : loopback TLS server + client in one process: the client script's positive control
import socket, ssl, sys, threading

mode = sys.argv[1]


def ctx_client(maxv, cert, key):
    c = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    c.check_hostname = False
    c.verify_mode = ssl.CERT_NONE
    if cert:
        c.load_cert_chain(cert, key)
    if maxv == "1.2":
        c.maximum_version = ssl.TLSVersion.TLSv1_2
    else:
        c.minimum_version = ssl.TLSVersion.TLSv1_3
    return c


def tls(ip, port, maxv, cert, key):
    try:
        s = socket.create_connection((ip, port), 5)
    except OSError as e:
        return "noconnect %s" % e
    try:
        t = ctx_client(maxv, cert, key).wrap_socket(s)
    except (ssl.SSLError, OSError) as e:
        return "refused-in-handshake %s:%s" % (type(e).__name__, getattr(e, "reason", None) or str(e)[:60])
    t.settimeout(4)
    try:
        d = t.recv(64)
    except (ssl.SSLError, OSError) as e:
        return "refused-after-handshake %s:%s" % (type(e).__name__, getattr(e, "reason", None) or str(e)[:60])
    return ("accepted bytes=%d first=%s" % (len(d), d[:6].hex())) if d else "closed-no-bytes"


if mode == "tls":
    ip, port, maxv = sys.argv[2], int(sys.argv[3]), sys.argv[4]
    print(tls(ip, port, maxv, *(sys.argv[5:7] if len(sys.argv) > 6 else (None, None))))
elif mode in ("listen", "hello"):
    s = socket.create_connection((sys.argv[2], int(sys.argv[3])), 5)
    s.settimeout(1.5)
    if mode == "hello":
        s.sendall(b"111F\x01")
    # read until 1.5 s of silence (or the peer closes): a single recv() may return the hello's segment alone
    d = b""
    try:
        while len(d) < 4096:
            chunk = s.recv(4096 - len(d))
            if not chunk:
                break
            d += chunk
    except OSError:
        pass
    # bytes received, the first five (the 111F hello) and the next three (a frame's type and big-endian length)
    print("%d %s %s" % (len(d), d[:5].hex(), d[5:8].hex()))
elif mode == "control":
    cert, key, maxv = sys.argv[2], sys.argv[3], sys.argv[4]
    sc = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    sc.load_cert_chain(cert, key)
    srv = socket.socket()
    srv.bind(("127.0.0.1", 0))
    srv.listen(1)

    def serve():
        c, _ = srv.accept()
        try:
            t = sc.wrap_socket(c, server_side=True)
            t.sendall(b"pong")
            t.close()
        except (ssl.SSLError, OSError):
            pass

    threading.Thread(target=serve, daemon=True).start()
    print(tls("127.0.0.1", srv.getsockname()[1], maxv, cert, key))
PY
}

# ------------------------------------------------------------------ inner: runs inside the namespaces
declare -A HOLD ROOT DPID NID IPS VETH
HELPERS=()
ST=000

api() { # NODE METHOD PATH [JSON] -> $ST, body in $W/body
  local n=$1 m=$2 p=$3 d=${4-}
  local args=(-sS -o "$W/body" -w '%{http_code}' -X "$m" --max-time 30)
  [ -n "$d" ] && args+=(-H 'Content-Type: application/json' -d "$d")
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
    sleep 0.4
  done
}
pair_up() { # NODE FAR -> 0 when NODE reads its pair with FAR reachable over rpk
  api "$1" GET /api/v1/relay
  jb --arg f "${NID[$2]}" '[.paired_hubs[]? | select(.node_id == $f and .reachable == true and .tls == "rpk")] | length == 1'
}
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
bundle_get() { # BUNDLE FIELD (spki | address)
  python3 -I -c '
import sys
p = sys.argv[1].split(":")
print(p[3] if sys.argv[2] == "spki" else ":".join(p[4:-2]))' "$1" "$2"
}
nx() { local n=$1; shift; nsenter -t "${HOLD[$n]}" -n -- "$@"; }

# the tap: one sniffer over the host ends of the veths (each frame is read once, at its sender's end)
CAP_PID=""
ann_sample() { # copy each hub's own announce (re-minted when its role bits change, so every version is kept)
  local n f
  mkdir -p "$W/ann"
  for n in $L7_G11_NODES; do
    f=${ROOT[$n]}/fabric/announce
    [ -s "$f" ] && cp "$f" "$W/ann/$n.$(sha256sum "$f" | cut -c1-12)"
  done
}
cap_start() { # NAME
  ann_sample
  local d=$W/cap-$1
  mkdir -p "$d"
  rm -f "$d/ready"
  python3 -I "$W/cap.py" sniff "$d" "${VETH[X]}" "${VETH[Y]}" "${VETH[Z]}" "${VETH[Q]}" >"$d/err" 2>&1 &
  CAP_PID=$!
  local _
  for _ in $(seq 1 50); do [ -e "$d/ready" ] && return 0; sleep 0.1; done
  fail "the AF_PACKET tap did not start ($(head -c 200 "$d/err"))"
  return 1
}
cap_stop() { # NAME -> $W/NAME.json
  local d=$W/cap-$1
  [ -n "$CAP_PID" ] || return 1
  sleep 0.5
  kill -TERM "$CAP_PID" 2>/dev/null
  wait "$CAP_PID" 2>/dev/null
  CAP_PID=""
  ann_sample
  local args=() f b
  for f in "$W"/ann/*; do
    b=$(basename "$f")
    args+=("${b%%.*}=$f")
  done
  python3 -I "$W/cap.py" report "$d" "${args[@]}" >"$W/$1.json" 2>"$d/report.err" ||
    fail "the capture report of $1 failed ($(head -c 200 "$d/report.err"))"
}
# flows of a capture: CLIENT-NAME SERVER-NAME ("" = any) -> JSON lines
cflows() { # NAME CIP SIP -> the flows a client at CIP opened to a server at SIP, one JSON line each
  jq -c --arg c "$2" --arg s "$3" '.flows[] | select(.cip == $c and .sip == $s)' "$W/$1.json" 2>/dev/null
}
# fjq NAME FILTER [jq args] -> a jq filter over the capture's flows array (the filter reads `.flows`)
fjq() { local n=$1 f=$2; shift 2; jq -r "$@" "$f" "$W/$n.json" 2>/dev/null; }
syns() { jq -r --arg k "$2>$3" '.syns[$k] // 0' "$W/$1.json" 2>/dev/null; } # NAME SRCIP DSTIP

inner() {
  W=${G11_WORK:?}
  local bin=${G11_BIN:?}
  local n i
  IPS[X]=$L7_G11_IP_X IPS[Y]=$L7_G11_IP_Y IPS[Z]=$L7_G11_IP_Z IPS[Q]=$L7_G11_IP_Q
  VETH[X]=$L7_G11_VETH_X VETH[Y]=$L7_G11_VETH_Y VETH[Z]=$L7_G11_VETH_Z VETH[Q]=$L7_G11_VETH_Q

  cleanup_inner() {
    local p
    [ -n "$CAP_PID" ] && kill "$CAP_PID" 2>/dev/null
    for n in $L7_G11_NODES; do
      p=${DPID[$n]:-}
      [ -n "$p" ] && kill -TERM "$p" 2>/dev/null
    done
    for _ in $(seq 1 50); do
      local alive=0
      for n in $L7_G11_NODES; do kill -0 "${DPID[$n]:-0}" 2>/dev/null && alive=1; done
      [ "$alive" = 0 ] && break
      sleep 0.2
    done
    for n in $L7_G11_NODES; do
      p=${DPID[$n]:-}
      [ -n "$p" ] && kill -KILL "$p" 2>/dev/null
    done
    for p in "${HELPERS[@]}"; do kill "$p" 2>/dev/null; done
    wait 2>/dev/null
  }
  trap cleanup_inner EXIT
  trap 'exit 130' INT TERM

  ok "namespaces: user + net + pid, /proc is this namespace's own"
  # ---- topology: bridge l7g11br0, one netns per hub, TEST-NET-3 addresses
  ip link set lo up
  if ! { ip link add "$L7_G11_BR" type bridge && ip link set "$L7_G11_BR" up; }; then
    fail "cannot create the bridge $L7_G11_BR"
    return
  fi
  for n in $L7_G11_NODES; do
    unshare --net sleep 100000 &
    HOLD[$n]=$!
    HELPERS+=("$!")
  done
  sleep 0.5
  local built=1
  for n in $L7_G11_NODES; do
    if ip link add "${VETH[$n]}" type veth peer name "${VETH[$n]}p" &&
      ip link set "${VETH[$n]}p" netns "${HOLD[$n]}" &&
      nx "$n" ip link set "${VETH[$n]}p" name eth0 &&
      nx "$n" ip addr add "${IPS[$n]}/24" dev eth0 && nx "$n" ip link set eth0 up && nx "$n" ip link set lo up &&
      ip link set "${VETH[$n]}" master "$L7_G11_BR" && ip link set "${VETH[$n]}" up; then
      :
    else
      built=0
    fi
  done
  if [ "$built" = 1 ]; then
    ok "four netns (X ${IPS[X]}, Y ${IPS[Y]}, Z ${IPS[Z]}, Q ${IPS[Q]}) on $L7_G11_BR, $L7_G11_NET: no private range, so §4.1 rule 1 applies to every link"
  else
    fail "the topology could not be built"
    return
  fi
  if nx X ping -c1 -W2 "${IPS[Y]}" >/dev/null 2>&1; then ok "X reaches Y over the bridge (ping)"; else note "no ping answer (ICMP may be unavailable); TCP decides below"; fi

  # ---- four real daemons: hub_role and listen_lan on, check_internet off (no updater traffic)
  for n in $L7_G11_NODES; do
    ROOT[$n]=$W/root-$n
    g6_make_root "${ROOT[$n]}" ""
    NID[$n]=$(g6_identity "${ROOT[$n]}" "$bin")
    [ -n "${NID[$n]}" ] || {
      fail "$n: no identity ($(head -c 200 "${ROOT[$n]}/identity.log"))"
      return
    }
    g6_fabric_settings "${ROOT[$n]}" '{"schema":1,"hub_role":true,"listen_lan":true}'
    printf '{"schema":1,"updates":{"check_internet":false}}\n' >"${ROOT[$n]}/state/relay.json"
    chmod 600 "${ROOT[$n]}/state/relay.json"
    NODE111_ROOT=${ROOT[$n]} nsenter -t "${HOLD[$n]}" -n -- "$bin" daemon >"$W/daemon-$n.log" 2>&1 &
    DPID[$n]=$!
  done
  for n in $L7_G11_NODES; do
    if wait_for 40 'api "$n" GET /api/v1/fabric; [ "$ST" = 200 ] && jb ".store.open == true"'; then
      :
    else
      fail "$n: the daemon's fabric never came up (HTTP $ST)"
      tail -5 "$W/daemon-$n.log" | sed "s/^/  $n daemon| /"
      return
    fi
  done
  ok "four daemons up with a fabric: X ${NID[X]:0:8}, Y ${NID[Y]:0:8}, Z ${NID[Z]:0:8}, Q ${NID[Q]:0:8}"
  for n in $L7_G11_NODES; do
    wait_for 10 'nx "$n" ss -ltnH 2>/dev/null | grep -q ":$PORT "' || fail "$n: nothing listens on :$PORT"
  done
  ok "every hub listens on :$PORT (hub_role + listen_lan)"
  sleep 3
  local rss_before
  rss_before=$(rss_kb X)

  # ---- control for the python clients (a handshake they can finish), certificates for the refusal clients
  openssl req -x509 -newkey ed25519 -nodes -keyout "$W/ed.key" -out "$W/ed.crt" -subj /CN=g11 -days 2 >/dev/null 2>&1
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -keyout "$W/ec.key" -out "$W/ec.crt" -subj /CN=g11 -days 2 >/dev/null 2>&1
  if [ -s "$W/ed.crt" ] && [ -s "$W/ec.crt" ]; then
    ok "two X.509 certificates made for the refusal clients (Ed25519 and ECDSA P-256)"
  else
    fail "openssl could not make the X.509 test certificates"
  fi
  local ctl
  for v in 1.3 1.2; do
    ctl=$(nx Z python3 -I "$W/client.py" control "$W/ec.crt" "$W/ec.key" "$v" 2>&1)
    case $ctl in
      "accepted bytes=4 first="*) ok "control: the python client completes a TLS $v handshake with a certificate and reads 4 bytes from a TLS server on loopback ($ctl)" ;;
      *) fail "control: the python TLS $v client did not complete a handshake with a plain TLS server: $ctl" ;;
    esac
  done

  # ================================================================== 1. no pin: the listener peeks all the same
  # hello_and_announce "<count> <first 5 hex> <next 3 hex>": the 111F hello, then one whole announce frame (type 0x02)
  hello_and_announce() {
    local n h f
    read -r n h f <<<"$1"
    [ "$h" = 3131314601 ] && [ "${f:0:2}" = 02 ] && [ ${#f} = 6 ] && [ "$n" = $((5 + 3 + 16#${f:2:4})) ]
  }
  # (§4.1 rule 2: the first byte of every inbound connection is peeked; row D5's follow-up after this gate's first runs,
  # when a pin-less hub spoke first and so showed a TLS client its hello and announce in clear)
  local r
  r=$(nx Z python3 -I "$W/client.py" listen "${IPS[X]}" "$PORT" 2>&1)
  if [ "${r%% *}" = 0 ]; then
    ok "no pin at X: the listener peeks, a silent connection gets no byte"
  else
    fail "no pin at X: a silent connection was answered with '$r' (want no byte: §4.1 rule 2 peeks first)"
  fi
  r=$(nx Z python3 -I "$W/client.py" hello "${IPS[X]}" "$PORT" 2>&1)
  if hello_and_announce "$r"; then
    ok "no pin at X: a client that sends the 111F hello is answered with the hello and one whole announce frame (${r%% *} bytes), the plain wire as before Layer 7"
  else
    fail "no pin at X: a client that sent the 111F hello got '$r' (want the hello and the announce)"
  fi

  # ================================================================== 2. offers
  local bundle_x bundle_y bundle_q spki_x spki_y spki_q
  api X POST /api/v1/relay/pair/offer '{"role":"peer","via":"tcp"}'
  bundle_x=$(jget .bundle.text) spki_x=$(jget .link_spki)
  api Y POST /api/v1/relay/pair/offer '{"role":"peer","via":"tcp"}'
  bundle_y=$(jget .bundle.text) spki_y=$(jget .link_spki)
  api Q POST /api/v1/relay/pair/offer '{"role":"peer","via":"tcp"}'
  bundle_q=$(jget .bundle.text) spki_q=$(jget .link_spki)
  if [[ "$bundle_x" == 111PAIR:* ]] && [[ "$bundle_y" == 111PAIR:* ]] && [[ "$bundle_q" == 111PAIR:* ]] &&
    [ "${#spki_x}" = 71 ] && [ "${#spki_y}" = 71 ] && [ "${#spki_q}" = 71 ] &&
    [ "$spki_x" != "$spki_y" ] && [ "$spki_x" != "$spki_q" ]; then
    ok "three offers: 111PAIR bundles, each with its own 71-character link SPKI (X's address field: $(bundle_get "$bundle_x" address))"
  else
    fail "the pairing offers failed (X $ST, '${bundle_x:0:30}', '${bundle_y:0:30}', '${bundle_q:0:30}')"
    return
  fi
  if [ "$(stat -c %a "${ROOT[X]}/secrets/link-key" 2>/dev/null)" = 600 ] && [ "$(stat -c %a "${ROOT[X]}/secrets" 2>/dev/null)" = 700 ]; then
    ok "X's link key is a file of its own: secrets/link-key 0600 in secrets/ 0700 (D5), minted at the first offer"
  else
    fail "X's secrets/link-key is not 0600 in a 0700 directory ($(stat -c %a "${ROOT[X]}/secrets/link-key" 2>&1), $(stat -c %a "${ROOT[X]}/secrets" 2>&1))"
  fi
  if [ "$(wc -c <"${ROOT[X]}/secrets/link-key")" -le 128 ] && ! grep -qF "${NID[X]}" "${ROOT[X]}/secrets/link-key"; then
    ok "the link key is not the identity: its file does not hold X's NodeId"
  else
    fail "secrets/link-key holds X's NodeId or is oversized"
  fi

  # ================================================================== 3. the refusals, with real daemons
  # Y scans X's honest bundle. X scans Y's card and code with Q's link SPKI swapped in: X now pins Q's key as Y's.
  #   X -> Y   X expects Q's key, Y shows its own: a wrong server key
  #   Y -> X   Y's key is not the one X pinned: a wrong client key
  #   Q -> X   Q's key IS the one X pinned (as Y's), Q's announce names Q: a matched pin and another NodeId (S7)
  cap_start refuse || return
  api Y POST /api/v1/relay/pair "$(jq -nc --arg b "$bundle_x" '{bundle: $b, role: "peer", via: "tcp"}')"
  local st_y=$ST
  api X POST /api/v1/relay/pair "$(jq -nc --arg b "$(bundle_set "$bundle_y" spki "$spki_q")" '{bundle: $b, role: "peer", via: "tcp"}')"
  local st_x=$ST
  api Q POST /api/v1/relay/pair "$(jq -nc --arg b "$bundle_x" '{bundle: $b, role: "peer", via: "tcp"}')"
  local st_q=$ST
  note "the three scans answered HTTP Y $st_y, X $st_x, Q $st_q (202 = the attempt runs on, 502 = refused inside the first wait)"
  sleep "$SETTLE"
  cap_stop refuse

  local nf
  # wrong server key: every dial from X to Y
  nf=$(cflows refuse "${IPS[X]}" "${IPS[Y]}" | grep -c .)
  if [ "$nf" -ge 1 ]; then
    local bad
    bad=$(cflows refuse "${IPS[X]}" "${IPS[Y]}" | jq -sc '[.[] | select(.c2s_first != 22 or .ch_len == null or (.c2s - .ch_len) > 64 or .clear_hello)] | length')
    if [ "$bad" = 0 ]; then
      ok "wrong server key: X dialled Y $nf time(s) in TLS (first byte 0x16) and sent nothing after its ClientHello but an alert (<= 64 B), no 111F in clear: the dialler rejected Y's key before any 111 byte"
    else
      fail "wrong server key: $bad of $nf X->Y flow(s) sent more than a ClientHello and an alert, or started plain: $(cflows refuse "${IPS[X]}" "${IPS[Y]}" | head -2 | cut -c1-300)"
    fi
  else
    fail "wrong server key: X made no TCP connection to Y (the scan answered HTTP $st_x)"
  fi
  # wrong client key: Y -> X
  # only the dials X's TLS path answered count (ServerHello 0x16 or alert 0x15 first): the pre-pin dials X answered in
  # plain (s2c 0x31, the defect judged in section 4) are not a handshake refusal. Each must have ended (FIN or RST).
  nf=$(cflows refuse "${IPS[Y]}" "${IPS[X]}" | jq -sc '[.[] | select(.c2s_first == 22 and (.s2c_first == 21 or .s2c_first == 22) and (.fin or .rst) and (.clear_hello | not))] | length')
  api X GET /api/v1/fabric
  local px
  px=$(jget '[.links[]? | select(.id == "tcp-lan") | .peers] | add // 0')
  api Y GET /api/v1/fabric
  local py
  py=$(jget '[.links[]? | select(.id == "tcp-lan") | .peers] | add // 0')
  api Q GET /api/v1/fabric
  local pq
  pq=$(jget '[.links[]? | select(.id == "tcp-lan") | .peers] | add // 0')
  if [ "${nf:-0}" -ge 1 ] && [ "$px" = 0 ] && [ "$py" = 0 ]; then
    ok "wrong client key: Y dialled X in TLS $nf time(s) that X answered in its TLS path (ServerHello or alert, then closed); X, whose pin for Y is Q's key, attributed no peer (X $px, Y $py): refused at the handshake"
  else
    fail "wrong client key: Y->X TLS dials $nf, attributed peers X $px, Y $py (want >= 1 dial answered by X's TLS path and closed, 0 peers)"
  fi
  # a matched SPKI whose announce names another NodeId: Q -> X
  # In TLS 1.3 the ServerHello precedes the client's authentication, so a ServerHello alone shows nothing. The proof that
  # X's verifier ACCEPTED Q's key (the pin matched) is what only an accepted client gets: X's own 111 hello and announce
  # inside the TLS stream (many more server bytes than the refused wrong-client-key flows of Y), and the client's Finished
  # and announce coming back. The refusal is then the announce's (no peer attributed), not the handshake's.
  local qmax ymax qn
  qmax=$(cflows refuse "${IPS[Q]}" "${IPS[X]}" | jq -sc '[.[] | select(.c2s_first == 22 and .ch != null and .sh != null and (.clear_hello | not)) | .s2c] | max // 0')
  qn=$(cflows refuse "${IPS[Q]}" "${IPS[X]}" | jq -sc '[.[] | select(.c2s_first == 22 and .ch != null and .sh != null and (.clear_hello | not))] | length')
  ymax=$(cflows refuse "${IPS[Y]}" "${IPS[X]}" | jq -sc '[.[] | select(.c2s_first == 22 and (.s2c_first == 21 or .s2c_first == 22)) | .s2c] | max // 0')
  # the peers count over the settle window, not one sample
  local pk=0 sx sq i
  for i in 1 2 3 4; do
    api X GET /api/v1/fabric
    sx=$(jget '[.links[]? | select(.id == "tcp-lan") | .peers] | add // 0')
    api Q GET /api/v1/fabric
    sq=$(jget '[.links[]? | select(.id == "tcp-lan") | .peers] | add // 0')
    pk=$((pk + sx + sq))
    sleep 1
  done
  note "S7 flows: Q->X $qn TLS flow(s) with a ServerHello, largest server side $qmax B; Y->X refused flows, largest server side $ymax B; attributed peers summed over 4 samples: $pk"
  if [ "${qn:-0}" -ge 1 ] && [ "$qmax" -gt $((ymax + 100)) ] && [ "$pk" = 0 ] && [ "$pq" = 0 ] && [ "$px" = 0 ]; then
    ok "matched SPKI, other NodeId: Q's key is the one X pinned for Y; X's verifier accepted it (X sent $qmax B inside the stream against $ymax B to the refused wrong key) and still no peer was attributed on X or Q over 4 samples: refused on the announce (S7)"
  else
    fail "matched SPKI, other NodeId: Q->X TLS flows $qn, server bytes $qmax (refused wrong-key flows $ymax), attributed peers X $px, Q $pq, sampled $pk"
  fi
  # a flow that STARTS plain (0x31) or carries the hello / an announce while X, Y or Q dials a pinned non-private pair is a
  # plain fallback, whatever its first byte: none may exist. (The TLS-dial-to-a-pin-less-hub answer is judged in section 4.)
  local pfirst
  pfirst=$(fjq refuse '[.flows[] | select(.c2s_first != 22 and (.clear_hello or (.clear_announces | length > 0)))] | length')
  if [ "${pfirst:-x}" = 0 ]; then
    ok "refusal phase: no hub dialled another in plain (no flow that starts with a 111 byte, none with the hello or an announce in a client-first position)"
  else
    fail "refusal phase: $pfirst flow(s) between the hubs started plain or carry the 111F hello / an announce in clear: $(fjq refuse '[.flows[] | select(.c2s_first != 22 and (.clear_hello or (.clear_announces | length > 0)))] | .[0] | {cip, sip, c2s_first, clear_announces}' -c)"
  fi
  # no daemon became whole
  local whole
  api X GET /api/v1/relay
  whole=$(jq -r '[.paired_hubs[]? | select(.reachable == true)] | length' "$W/body")
  api Y GET /api/v1/relay
  whole=$((whole + $(jq -r '[.paired_hubs[]? | select(.reachable == true)] | length' "$W/body")))
  if [ "$whole" = 0 ]; then
    ok "none of the three refused pairs shows reachable on X or Y"
  else
    fail "a refused pair reads reachable ($whole)"
  fi
  # the refusal is the handshake's, not a 111 one: no announce of X, Y or Q travelled in clear to the TLS dialler that
  # met a hub with a pin, and the plain (pre-pin) leak is judged in section 4
  # forget the tampered state: unpair everything
  api X DELETE "/api/v1/relay/paired/${NID[Y]}"
  api X DELETE "/api/v1/relay/paired/${NID[Q]}"
  api Y DELETE "/api/v1/relay/paired/${NID[X]}"
  api Q DELETE "/api/v1/relay/paired/${NID[X]}"
  sleep 1
  api X GET /api/v1/relay
  local left_pairs=0 un
  for un in X Y Q; do
    api "$un" GET /api/v1/relay
    jb '.paired_hubs | length == 0' || { left_pairs=$((left_pairs + 1)); note "$un still holds a pair after the unpair: $(jget -c '.paired_hubs')"; }
  done
  if [ "$left_pairs" = 0 ]; then ok "unpaired everywhere (DELETE /relay/paired): X, Y and Q hold no pair again"; else fail "$left_pairs hub(s) of X, Y, Q still hold a pair after the unpair"; fi
  # nothing in flight between X and Y before the honest pairing's capture starts
  if wait_for 15 '[ "$(nx X ss -tnH state established 2>/dev/null | grep -c "${IPS[Y]}")" = 0 ]'; then
    ok "no TCP connection between X and Y is open when the honest pairing's capture starts"
  else
    fail "a connection between X and Y is still open after the unpair: the pair capture would read an earlier session"
  fi

  # ================================================================== 4. the honest pairing, over TLS
  api X POST /api/v1/relay/pair/offer '{"role":"peer","via":"tcp"}'
  bundle_x=$(jget .bundle.text)
  api Y POST /api/v1/relay/pair/offer '{"role":"peer","via":"tcp"}'
  bundle_y=$(jget .bundle.text)
  cap_start pair || return
  local lan=false # G11_NEG=lan: the negative control, the pair marked lan = true (runs plain): the TLS items must FAIL
  [ "${G11_NEG:-}" = lan ] && lan=true && note "G11_NEG=lan: the honest pairing is marked lan = true, so this run must FAIL the TLS items"
  api X POST /api/v1/relay/pair "$(jq -nc --arg b "$bundle_y" --argjson l "$lan" '{bundle: $b, role: "peer", via: "tcp", lan: $l}')"
  local st1=$ST
  api Y POST /api/v1/relay/pair "$(jq -nc --arg b "$bundle_x" --argjson l "$lan" '{bundle: $b, role: "peer", via: "tcp", lan: $l}')"
  local st2=$ST
  note "X scanned Y's bundle first (HTTP $st1), then Y scanned X's (HTTP $st2)"
  local up=0
  if wait_for 90 'pair_up X Y && pair_up Y X'; then up=1; fi
  if [ "$up" = 1 ]; then
    ok "both hubs read the pair reachable with tls = rpk on GET /relay (the pinned handshake ran and both ENROLLs landed)"
  else
    fail "the pair never became reachable with tls = rpk on both hubs"
    api X GET /api/v1/relay
    note "X /relay paired_hubs: $(jget -c '.paired_hubs')"
    api Y GET /api/v1/relay
    note "Y /relay paired_hubs: $(jget -c '.paired_hubs')"
  fi
  sleep 3
  cap_stop pair
  local rss_after
  rss_after=$(rss_kb X)
  # the live connection(s) of the pair: a TLS flow with a ServerHello and more than the handshake
  local pf
  # the live session: the X<->Y flow still OPEN at the end of the capture (no FIN, no RST), with a ClientHello and a
  # ServerHello seen and application-data records in both directions. A refused or stale dial is closed and cannot be it.
  local npairflows
  npairflows=$(fjq pair '[.flows[] | select((.cip == $x and .sip == $y) or (.cip == $y and .sip == $x))] | length' --arg x "${IPS[X]}" --arg y "${IPS[Y]}")
  pf=$(jq -c --arg x "${IPS[X]}" --arg y "${IPS[Y]}" '[.flows[] | select(((.cip == $x and .sip == $y) or (.cip == $y and .sip == $x)) and (.fin | not) and (.rst | not) and .c2s_first == 22 and .sh != null and .ch != null and (.c2s_rec | index(23)) != null and (.s2c_rec | index(23)) != null)] | sort_by(-.s2c) | .[0] // empty' "$W/pair.json")
  local nplain
  nplain=$(fjq pair '[.flows[] | select(((.cip == $x and .sip == $y) or (.cip == $y and .sip == $x)) and .c2s_first != 22)] | length' --arg x "${IPS[X]}" --arg y "${IPS[Y]}")
  if [ "$nplain" = 0 ] && [ "${npairflows:-0}" -ge 1 ]; then
    ok "every one of the $npairflows X<->Y connection(s) of the pairing starts with a TLS record (0x16): none starts plain"
  else
    fail "$nplain of $npairflows X<->Y connection(s) of the pairing do not start with a TLS handshake (a plain fallback or a plain-first dial)"
  fi
  if [ -n "$pf" ] && printf '%s' "$pf" | jq -e '(.c2s_exact and .s2c_exact)' >/dev/null; then
    ok "the live session is one TCP flow open at the end of the capture, both directions whole TLS records (record types c2s $(printf '%s' "$pf" | jq -c .c2s_types), s2c $(printf '%s' "$pf" | jq -c .s2c_types))"
  else
    fail "no X<->Y flow is open at the end of the capture with a handshake and application data both ways, or its bytes are not whole TLS records: $(printf '%s' "$pf" | cut -c1-300)"
  fi
  if [ -n "$pf" ] && printf '%s' "$pf" | jq -e '.ch.versions == [772] and .sh.version == 772' >/dev/null; then
    ok "the wire of the pair: the first byte 0x16, a ClientHello that offers TLS 1.3 only (supported_versions [0x0304]) and a ServerHello that selects it"
  else
    fail "the pair's wire is not a TLS 1.3 handshake: $(printf '%s' "$pf" | cut -c1-400)"
  fi
  if printf '%s' "$pf" | jq -e '.ch.sigalgs == [2055]' >/dev/null 2>&1; then
    ok "signature_algorithms in the ClientHello is exactly [ed25519 0x0807]"
  else
    fail "signature_algorithms is not Ed25519 only: $(printf '%s' "$pf" | jq -c '.ch.sigalgs' 2>&1)"
  fi
  if printf '%s' "$pf" | jq -e '.ch.client_cert_types == [2] and .ch.server_cert_types == [2]' >/dev/null 2>&1; then
    ok "client_certificate_type and server_certificate_type are exactly [RawPublicKey 2] (RFC 7250 both ways, no X.509 offered)"
  else
    fail "the certificate types are not raw public key only: client $(printf '%s' "$pf" | jq -c '.ch.client_cert_types' 2>&1) server $(printf '%s' "$pf" | jq -c '.ch.server_cert_types' 2>&1)"
  fi
  if printf '%s' "$pf" | jq -e '.ch.sni == false' >/dev/null 2>&1; then ok "no SNI in the ClientHello (§4.3)"; else fail "the ClientHello carries a server_name"; fi
  if printf '%s' "$pf" | jq -e '(.c2s_rec | index(23)) != null and (.s2c_rec | index(23)) != null' >/dev/null 2>&1; then
    ok "both directions carry application-data records after the handshake (the 111 wire runs inside the TLS stream)"
  else
    fail "no application-data records in one direction of the pair's connection"
  fi
  # "no cleartext": nothing of the pair is in clear after the second pin; the first dial's window is judged apart
  local late_clear
  late_clear=$(fjq pair '[.flows[] | select(((.cip == $x and .sip == $y) or (.cip == $y and .sip == $x)) and (.clear_hello or (.clear_announces | length > 0)) and .s2c_first != 49)] | length' --arg x "${IPS[X]}" --arg y "${IPS[Y]}")
  if [ "$late_clear" = 0 ]; then
    ok "no connection between X and Y (whatever its first byte) carries the 111F hello or either hub's announce in clear, apart from the pin-less-hub answer judged next"
  else
    fail "a connection between X and Y carries a cleartext hello or announce ($late_clear)"
  fi
  # the pairing window: a TLS client that dials a hub with no pin yet is answered in clear (§4.1 rule 2 as ruled in D5b)
  local leaks
  leaks=$(fjq pair '[.flows[] | select(((.cip == $x and .sip == $y) or (.cip == $y and .sip == $x)) and .c2s_first == 22 and .s2c_first == 49)] | length' --arg x "${IPS[X]}" --arg y "${IPS[Y]}")
  local leaks_refuse
  leaks_refuse=$(fjq refuse '[.flows[] | select(.c2s_first == 22 and .s2c_first == 49)] | length')
  local leaked_names
  leaked_names=$({ fjq pair '.flows[] | select(.c2s_first == 22 and .s2c_first == 49) | .clear_announces[]'; fjq refuse '.flows[] | select(.c2s_first == 22 and .s2c_first == 49) | .clear_announces[]'; } | sort -u | tr '\n' ',')
  if [ "$leaks" = 0 ] && [ "$leaks_refuse" = 0 ]; then
    ok "no TLS dial met a hub that answered in clear (no cleartext announce on the veth, the pairing window included)"
  else
    fail "cleartext announce on the veth in the pairing window: a TLS dial to a hub that held no pin yet was answered with the 111F hello and the hub's announce in clear ($leaks connection(s) in the honest pairing, $leaks_refuse in the refusal phase; announces seen in clear: ${leaked_names:-none}); D5b's no-peek-while-no-pin ruling leaves the first dial of a pairing exposed"
  fi
  if [ -n "$rss_before" ] && [ -n "$rss_after" ]; then
    note "RSS of X: $rss_before kB before any pair, $rss_after kB with one pinned TLS session up ($((rss_after - rss_before)) kB; debug build)"
  fi

  # ================================================================== 5. the idle window (keepalive bytes/day)
  # let the pairing's simultaneous dials settle (the two hubs dial each other; the loser of the tie is closed a moment
  # later): the idle window starts when X holds exactly one connection to Y and has for 8 s running
  local stable=0 t_end=$((SECONDS + 60))
  while [ "$SECONDS" -lt "$t_end" ] && [ "$stable" -lt 8 ]; do
    if [ "$(nx X ss -tnH state established 2>/dev/null | grep -c "${IPS[Y]}")" = 1 ]; then stable=$((stable + 1)); else stable=0; fi
    sleep 1
  done
  [ "$stable" -ge 8 ] || note "the X<->Y connection did not settle to one for 8 s within 60 s; the idle check below reads what it finds"
  note "idle window: ${WINDOW} s of the pinned pair, nothing else attached"
  cap_start idle || return
  sleep "$WINDOW"
  cap_stop idle
  local wb
  wb=$(fjq idle '[.flows[] | select((.cip == $x and .sip == $y) or (.cip == $y and .sip == $x)) | (.c2s + .s2c)] | add // 0' --arg x "${IPS[X]}" --arg y "${IPS[Y]}")
  local nconn
  nconn=$(fjq idle '[.flows[] | select((.cip == $x and .sip == $y) or (.cip == $y and .sip == $x))] | length' --arg x "${IPS[X]}" --arg y "${IPS[Y]}")
  local nontls
  nontls=$(fjq idle '[.flows[] | select(((.cip == $x and .sip == $y) or (.cip == $y and .sip == $x)) and (((.c2s_types - [23]) | length) > 0 or ((.s2c_types - [23]) | length) > 0 or (.c2s_exact | not) or (.s2c_exact | not) or ((.c2s > 0 or .s2c > 0) | not)))] | length' --arg x "${IPS[X]}" --arg y "${IPS[Y]}")
  local clear_idle
  clear_idle=$(fjq idle '[.flows[] | select(.clear_hello or (.clear_announces | length > 0))] | length')
  if [ "${wb:-0}" -gt 0 ] && [ "$nconn" = 1 ] && [ "$clear_idle" = 0 ] && [ "$nontls" = 0 ]; then
    ok "idle pair: one TCP connection, $wb payload bytes both ways in ${WINDOW} s (every byte of the flow parses as whole TLS application-data records, type 0x17 only), nothing in clear"
    note "keepalive bytes/day of one pinned pair: $((wb * 86400 / WINDOW)) B/day payload (TLS records, both directions; $wb B in ${WINDOW} s, keepalive every 25 s; TCP/IP headers and ACKs not counted)"
  else
    fail "idle pair: $nconn connection(s), $wb payload bytes, $clear_idle flow(s) with cleartext, $nontls flow(s) that are not whole TLS application-data records"
  fi
  if pair_up X Y; then ok "the pair is still reachable after the idle window"; else fail "the pair dropped during the idle window"; fi

  # ================================================================== 6. refused handshakes and plain members, with the pin present
  cap_start clients || return
  local res
  # X.509: Ed25519 and ECDSA keys, TLS 1.3; TLS 1.2 with and without a certificate
  res=$(nx Z python3 -I "$W/client.py" tls "${IPS[X]}" "$PORT" 1.3 "$W/ed.crt" "$W/ed.key" 2>&1)
  case $res in
    refused-in-handshake*|"refused-after-handshake SSLError"*) ok "an X.509 certificate (Ed25519 key, TLS 1.3) is refused by X's listener: $res, no application byte reached the client" ;;
    *) fail "an X.509 Ed25519 certificate was not refused: $res" ;;
  esac
  res=$(nx Z python3 -I "$W/client.py" tls "${IPS[X]}" "$PORT" 1.3 "$W/ec.crt" "$W/ec.key" 2>&1)
  case $res in
    refused-in-handshake*|"refused-after-handshake SSLError"*) ok "a second X.509 case (ECDSA P-256 certificate; refused at certificate-type negotiation like the Ed25519 one, so the key type is not reached here: that is relay_tls's a_non_ed25519_key_is_refused_both_ways): $res" ;;
    *) fail "an X.509 ECDSA certificate was not refused: $res" ;;
  esac
  res=$(nx Z python3 -I "$W/client.py" tls "${IPS[X]}" "$PORT" 1.2 "$W/ec.crt" "$W/ec.key" 2>&1)
  case $res in
    refused-in-handshake*|"refused-after-handshake SSLError"*) ok "TLS 1.2 (with a certificate) is refused: $res" ;;
    *) fail "TLS 1.2 with a certificate was not refused: $res" ;;
  esac
  res=$(nx Z python3 -I "$W/client.py" tls "${IPS[X]}" "$PORT" 1.2 2>&1)
  case $res in
    refused-in-handshake*|"refused-after-handshake SSLError"*) ok "TLS 1.2 (no certificate) is refused: $res" ;;
    *) fail "TLS 1.2 without a certificate was not refused: $res" ;;
  esac
  res=$(nx Z python3 -I "$W/client.py" tls "${IPS[Y]}" "$PORT" 1.3 "$W/ed.crt" "$W/ed.key" 2>&1)
  case $res in
    refused-in-handshake*|"refused-after-handshake SSLError"*) ok "the same X.509 client is refused by Y too: $res" ;;
    *) fail "Y did not refuse an X.509 client: $res" ;;
  esac
  # plain, with the pin present: the hello is served, the announce comes back
  r=$(nx Z python3 -I "$W/client.py" hello "${IPS[X]}" "$PORT" 2>&1)
  if hello_and_announce "$r"; then
    ok "plain inbound with a pin present: a client that sends the 111F hello is answered with the hello and one whole announce frame (${r%% *} bytes) on the same listener"
  else
    fail "plain inbound with a pin present: the hello got '$r'"
  fi
  r=$(nx Z python3 -I "$W/client.py" listen "${IPS[X]}" "$PORT" 2>&1)
  if [ "${r%% *}" = 0 ]; then
    ok "with a pin present the listener peeks: a silent connection gets no byte (§4.1 rule 2), as with no pin"
  else
    fail "with a pin present a silent connection was answered with '$r': no peek"
  fi
  # a real member enrols over the plain listener (§4.1 rule 3)
  api X POST /api/v1/fabric/hub/offer '{}'
  local code addr card
  code=$(jget .code) addr=$(jget .address) card=$(jget .card.text)
  api Z POST /api/v1/fabric/hub/enroll "$(jq -nc --arg a "$addr" --arg c "$code" --arg k "$card" '{address: $a, code: $c, card: $k}')"
  if [ "$ST" = 200 ] && jb '.enrolled == true and .reachable == true' && [ "$(jget .node_id)" = "${NID[X]}" ]; then
    ok "a member (Z) enrolled with hub X at $addr over the plain tcp-lan link while X holds a pinned pair (ENROLL_OK, reachable)"
  else
    fail "the member Z could not enrol with X (HTTP $ST: $(head -c 200 "$W/body"))"
  fi
  sleep 2
  cap_stop clients
  local zf
  zf=$(jq -c --arg z "${IPS[Z]}" --arg x "${IPS[X]}" '[.flows[] | select(.cip == $z and .sip == $x and .c2s_first == 49 and .clear_hello)] | sort_by(-.c2s) | .[0] // empty' "$W/clients.json")
  if [ -n "$zf" ] && printf '%s' "$zf" | jq -e '.clear_announces | index("X") != null' >/dev/null 2>&1 &&
    printf '%s' "$zf" | jq -e '.c2s_frames[0][0] == 2 and (.c2s_frames[0][1] >= 137 and .c2s_frames[0][1] <= 169) and .s2c_frames[0][0] == 2' >/dev/null 2>&1; then
    ok "the member's wire is the Layer 5 §8.1 grammar byte for byte: '111F' 0x01, then an announce frame (type 0x02, length $(printf '%s' "$zf" | jq '.c2s_frames[0][1]')) in clear, both ways; X's own announce bytes found in the stream (the detector's positive control)"
  else
    fail "the member's connection is not the plain tcp-lan wire: $(printf '%s' "$zf" | cut -c1-300)"
  fi
  # the refusal was the TLS path's: each refused python client was answered with a TLS record (an alert 0x15 or a
  # handshake 0x16), never with the plain 111F hello (what a listener with no pin says first)
  local nrefused nanswered
  nrefused=$(fjq clients '[.flows[] | select(.cip == $z and .sip == $x and .c2s_first == 22)] | length' --arg z "${IPS[Z]}" --arg x "${IPS[X]}")
  nanswered=$(fjq clients '[.flows[] | select(.cip == $z and .sip == $x and .c2s_first == 22 and (.s2c_first == 21 or .s2c_first == 22))] | length' --arg z "${IPS[Z]}" --arg x "${IPS[X]}")
  if [ "${nrefused:-0}" -ge 4 ] && [ "$nrefused" = "$nanswered" ]; then
    ok "all $nrefused refused TLS clients were answered by X's TLS path (first server byte 0x15 or 0x16), none by the plain wire"
  else
    fail "the refused TLS clients: $nrefused flows to X, $nanswered answered with a TLS record (want >= 4, all)"
  fi
  # the python clients' flows to X: nothing of 111 came back in clear on a TLS client's flow
  local tf
  tf=$(fjq clients '[.flows[] | select(.cip == $z and .sip == $x and .c2s_first == 22 and (.clear_hello or (.clear_announces | length > 0)))] | length' --arg z "${IPS[Z]}" --arg x "${IPS[X]}")
  if [ "$tf" = 0 ]; then ok "no refused TLS client (Z -> X) was answered with a 111F hello or an announce in clear"; else fail "$tf refused TLS client flow(s) saw cleartext 111 bytes"; fi

  # ================================================================== 7. plain outbound to a non-private pair needs the warning
  api Q POST /api/v1/relay/pair/offer '{"role":"peer","via":"tcp"}'
  bundle_q=$(jget .bundle.text)
  local plain_q
  plain_q=$(bundle_set "$bundle_q" spki "")
  cap_start plain || return
  api X POST /api/v1/relay/pair "$(jq -nc --arg b "$plain_q" '{bundle: $b, role: "peer", via: "tcp"}')"
  local st_c=$ST hash
  if [ "$st_c" = 409 ] && jb '(.code == "consent_required" or .error == "consent_required" or .type == "consent_required" or (tostring | contains("consent_required")))'; then
    hash=$(jget '.consent.hash // .consent.text_hash // .hash // empty')
    if [ "$(jget '.consent.action')" = pair_plain ] && [ "$(jget '.consent.warning_text')" = "$PAIR_PLAIN_TEXT" ] && [ "$(jget '.consent.subject')" = "${NID[Q]}" ]; then
      ok "a pair with no link key at a non-private address: 409 consent_required, action pair_plain, subject Q's NodeId, the §4.6 step 6 text verbatim"
    else
      fail "409 consent_required, but the consent is not pair_plain with the verbatim text: $(jget -c .consent | cut -c1-300)"
    fi
  else
    fail "a plain pair to a non-private address was not asked for consent (HTTP $st_c: $(head -c 300 "$W/body"))"
  fi
  sleep 2
  api X GET /api/v1/relay
  local before_n
  before_n=$(jq -r --arg q "${NID[Q]}" '[.paired_hubs[]? | select(.node_id == $q)] | length' "$W/body")
  cap_stop plain
  local syn_q
  syn_q=$(syns plain "${IPS[X]}" "${IPS[Q]}")
  if [ "$before_n" = 0 ] && [ "$syn_q" = 0 ] && [ "$(cflows plain "${IPS[X]}" "${IPS[Q]}" | grep -c .)" = 0 ]; then
    ok "refusing pairs nothing: no pair for Q on X, and X sent no SYN and no byte to ${IPS[Q]}:$PORT while the warning was open (outbound plain refused without a recorded warning)"
  else
    fail "the unaccepted plain pair left a trace: pair rows $before_n, SYNs X>Q $syn_q"
  fi
  # accept it
  cap_start plain2 || return
  if [ -n "$hash" ]; then
    api X POST /api/v1/relay/pair "$(jq -nc --arg b "$plain_q" --arg h "$hash" '{bundle: $b, role: "peer", via: "tcp", accept_warning: $h}')"
    if [ "$ST" = 202 ]; then ok "with accept_warning = the hash of the text: 202, the pair is recorded"; else fail "the accepted plain pair answered HTTP $ST: $(head -c 200 "$W/body")"; fi
  else
    fail "no consent hash to accept"
  fi
  wait_for 20 'api X GET /api/v1/relay; jb --arg q "${NID[Q]}" "[.paired_hubs[] | select(.node_id == \$q and .tls == \"plain\")] | length == 1"' || true
  sleep 3
  cap_stop plain2
  if jb --arg q "${NID[Q]}" '[.paired_hubs[] | select(.node_id == $q and .tls == "plain" and .lan == false)] | length == 1'; then
    ok "X's /relay shows the accepted pair as tls = plain (lan = false: it is the warning, not the LAN mark, that allows it)"
  else
    fail "the accepted pair is not tls = plain on /relay: $(jget -c '.paired_hubs')"
  fi
  local pq_flow
  pq_flow=$(cflows plain2 "${IPS[X]}" "${IPS[Q]}" | jq -sc '[.[] | select(.c2s_first == 49 and .clear_hello)] | length')
  if [ "${pq_flow:-0}" -ge 1 ]; then
    ok "the accepted plain pair dials in clear: the 111F hello and X's announce are on the veth ($pq_flow connection(s)): the detector sees cleartext where there is some"
  else
    fail "the accepted plain pair made no plain connection to Q (flows: $(cflows plain2 "${IPS[X]}" "${IPS[Q]}" | head -2 | cut -c1-200))"
  fi
  if grep -Eq '"warning_hash": *"[0-9a-f]{64}"' "${ROOT[X]}/fabric/settings" 2>/dev/null || jq -e '[.paired_hubs[]? | select(.warning_hash != null)] | length == 1' "${ROOT[X]}/fabric/settings" >/dev/null 2>&1; then
    ok "the text hash is kept with the pair in X's fabric/settings (warning_hash)"
  else
    fail "no warning_hash in X's fabric/settings: $(grep -c warning_hash "${ROOT[X]}/fabric/settings" 2>&1)"
  fi

  # ================================================================== 8. OpenSSL raw-public-key peer
  local osv
  osv=$(openssl version 2>/dev/null | awk '{ print $2 }')
  if [ -n "$osv" ] && [ "$(printf '%s\n3.2.0\n' "$osv" | sort -V | head -n 1)" = 3.2.0 ]; then
    if [ "${REQUIRE_OPENSSL_RPK:-0}" = 1 ]; then
      fail "REQUIRE_OPENSSL_RPK=1: the OpenSSL peer step is NOT IMPLEMENTED in this gate (OpenSSL $osv has RPK, but no step was written: its s_client/s_server RPK options could not be verified on the box that wrote the gate)"
    else
      note "OpenSSL RPK items SKIPPED: the OpenSSL peer step is NOT IMPLEMENTED in this gate (OpenSSL $osv could run one)"
    fi
  elif [ "${REQUIRE_OPENSSL_RPK:-0}" = 1 ]; then
    fail "REQUIRE_OPENSSL_RPK=1: OpenSSL ${osv:-absent} is older than 3.2 (no RPK): the OpenSSL peer against our server and client cannot run"
  else
    note "OpenSSL peer against our server and client SKIPPED: OpenSSL ${osv:-absent} < 3.2 has no raw-public-key support (REQUIRE_OPENSSL_RPK=1 makes this a FAIL)"
  fi

  # the daemons are stopped by the trap; say it once the logs are read
  local lg
  for n in $L7_G11_NODES; do
    lg=$(grep -ci 'panic' "$W/daemon-$n.log" 2>/dev/null)
    [ "${lg:-0}" = 0 ] || fail "$n: the daemon log mentions a panic ($lg)"
  done
  cleanup_inner
  trap - EXIT
  ok "daemons, tap and namespace holders stopped"
}

if [ "${1:-}" = --inner ]; then
  inner
  printf 'FAILS=%d\n' "$FAILS" >"${G11_WORK:?}/inner.result"
  exit 0
fi

# ------------------------------------------------------------------ outer
skip_item() { # a missing prerequisite of one item
  if [ "${REQUIRE_G11:-0}" = 1 ]; then fail "$1 (REQUIRE_G11=1)"; else note "SKIPPED: $1"; fi
}
for tool in python3 ip unshare nsenter curl jq openssl ss awk; do
  command -v "$tool" >/dev/null 2>&1 || skip_or_fail G11 "$tool is absent"
done
have_netns || skip_or_fail G11 "unprivileged network namespaces are not available (unshare -Urnm)"
unshare --user --map-root-user --net --pid --fork --mount-proc true >/dev/null 2>&1 ||
  skip_or_fail G11 "a pid namespace with its own /proc is not available (unshare --pid --fork --mount-proc)"
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
if [ -z "$BIN111" ] || [ ! -x "$BIN111" ]; then skip_or_fail G11 "no built 111 binary (set BIN111 or CARGO_TARGET_DIR)"; fi
BIN111=$(cd "$(dirname "$BIN111")" && pwd)/$(basename "$BIN111")

WORK=$(mktemp -d "${TMPDIR:-/tmp}/l7-tls.XXXXXX") || exit 1
cleanup() {
  if [ "${G11_KEEP:-0}" = 1 ]; then note "kept $WORK"; else rm -rf "${WORK:?}"; fi
}
trap cleanup EXIT
write_tools "$WORK"

# ---- the dependency graph (S10): ring is the one provider, aws-lc-rs and any second one are absent
TREE=$WORK/tree.txt
if command -v cargo >/dev/null 2>&1 && [ -f 1-bootstrap/Cargo.toml ]; then
  if (cd 1-bootstrap && cargo tree --offline -e features -p node111) >"$TREE" 2>"$WORK/tree.err" && [ -s "$TREE" ]; then
    if grep -q 'tokio-rustls' "$TREE" && grep -q 'rustls' "$TREE" && grep -Eq '(^|[^a-z-])ring( |$)' "$TREE"; then
      ok "cargo tree -e features -p node111 names tokio-rustls, rustls and ring (the graph is the right one)"
      if grep -Eq 'aws-lc|aws_lc' "$TREE"; then
        fail "cargo tree shows aws-lc-rs / aws-lc-sys: a second crypto provider is in node111's graph"
      else
        ok "cargo tree shows no aws-lc-rs (S10)"
      fi
      other=$(grep -Eo 'rustls-rustcrypto|rustls-openssl|rustls-symcrypt|boring|native-tls|rustls-graviola|rustls feature "(fips|prefer-post-quantum|aws[_-]lc[_-]rs)"' "$TREE" | sort -u | tr '\n' ' ')
      if [ -z "$other" ]; then
        ok "no second TLS crypto provider (no rustls-rustcrypto, rustls-openssl, symcrypt, boring, native-tls, graviola, no rustls fips / prefer-post-quantum / aws_lc_rs feature) in the graph"
        if grep -q 'openssl-sys' "$TREE"; then note "openssl-sys is in the graph under libsqlite3-sys (the SQLCipher store, Layer 5), not under rustls or tokio-rustls: it is not a TLS provider of these links"; fi
      else
        fail "another TLS or crypto provider is in the graph: $other"
      fi
    else
      fail "cargo tree output does not name tokio-rustls, rustls and ring (positive control failed: $(head -c 200 "$TREE"))"
    fi
  else
    skip_item "cargo tree failed offline ($(head -c 200 "$WORK/tree.err"))"
  fi
else
  skip_item "cargo is absent: the dependency graph item (no aws-lc-rs, no second provider) cannot run"
fi

if env --default-signal true 2>/dev/null; then
  RESET_SIG=(env --default-signal)
else
  RESET_SIG=(python3 -c 'import os,signal,sys
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])')
fi
INNER_LOG=$WORK/inner.log
G11_WORK=$WORK G11_BIN=$BIN111 G11_WINDOW=$WINDOW G11_SETTLE=$SETTLE REQUIRE_OPENSSL_RPK=${REQUIRE_OPENSSL_RPK:-0} \
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
if [ -z "$left" ]; then ok "no daemon, tap or helper of this run is left"; else fail "left running:$left"; fi
# (the bridge and veths live inside the unshare --net namespace and die with it: the process scan above is the evidence)
finish G11
