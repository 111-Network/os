#!/usr/bin/env bash
# 7-relay/verify/origin.sh — Layer 7 gate G12 (spec §15 G12; §9.1–§9.5; row V6): the hub as update origin
# and the Layer 1 trust fixes T1–T6, through the real `111 daemon` and the real `111 update --from` verb.
#
#   bash 7-relay/verify/origin.sh                runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G12=1 bash 7-relay/verify/origin.sh  lock run: a missing prerequisite is a FAIL
#
# Namespaces (`unshare -rnpf`, user + net + pid; everything dies with them): the outer one holds the bridge
# l7g12br0 10.111.72.1/24 (no route off-box), the verb scenarios (loopback only) and two small python servers
# (a request log standing in for `payload.url`, a slow origin for the concurrent run); hub O 10.111.72.10 and
# member U 10.111.72.11 are one netns each (bridge.sh's holder pattern), each with its own NODE111_ROOT.
#
# Fixtures: a stdlib python builder (written to the scratch root) makes a real payload tarball, a channel manifest
# and its blob/<sha256>, and signs it with `minisign` and the node111 tests' throwaway keys
# (1-bootstrap/crates/node111/tests/fixtures/{minisign,l7-L1}; the keychain tails are copied from the l7-L1
# manifests, so no signer is invented). The debug binary's compiled key IS the test key (keys/manifest-pub.key).
#
#   1  the verb (T1, T2, T3, T5, §9.2): a signed `beta` under the stable name refused; a generation-1 release
#      only through a valid chain; a broken `signature_by_previous` refused; the floor generation held;
#      min-upgrade-from above the node refused before any blob; `--from <dir>` stages and writes the request file
#      and nothing else; a tampered blob refused with nothing left staged; a held `state/staging.lock` exits 75;
#      a concurrent run stages once; the payload of a `--from <url>` run is fetched at blob/<sha256>, never at
#      `payload.url`, and no last-update-check.json is written; the transient RSS of a staged update.
#   2  the hub (D7, §9.1): O serves /origin/v1/* only with hub_role, LAN sharing and `origin.serve` consented;
#      byte-exact bodies; non-GET 405; no session; a missing blob 404s; off at run time and at a restart.
#   3  the member (L4, §9.3, S4): U enrolled at O, no internet route: stages the release from O at origin_base
#      (blob/<sha256>, payload.url never asked), writes the per-origin record and no last-update-check.json; a
#      missing blob there stages nothing, and the next attribution recovers.
#   4  `updates.check_internet` (§9.3): false => no internet base contacted (the egress log on U's veth: no
#      off-link frame, no DNS query for an internet base) and no lock held; true (the positive control) => the
#      daemon's own check reaches for the internet base and holds the staging lock, so a verb run exits 75.
#
# Not here (node111 tests, row V6 / I14): T3's helper refusal and every flip (root-only), the walk of
# `check_origins` over several origins (updater_stage), each switch alone at the route (origin_serve).
#
# Env: BIN111 (the `111` binary; else $CARGO_TARGET_DIR/{release,debug}/111; a debug build: it reads the test key)
# · ORIGIN_ATTRIB (60 s, the wait for an attribution and a stage) · ORIGIN_EGRESS (20 s, each check_internet pass)
# · ORIGIN_KEEP=1 keeps the scratch root · ORIGIN_NEG=noserve starts the hub without LAN sharing and the hub role
# (a daemon that does not serve: the serving lines must FAIL) · TMPDIR for the scratch root.
# Conditions are strings eval'd by chk(), so single-quoted `$vars` are intended:
# shellcheck disable=SC2016,SC2034,SC2317,SC2154,SC2329
set -u
export LC_ALL=C
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=7-relay/verify/lib.sh disable=SC1091
. 7-relay/verify/lib.sh

EGRESS=$L7_OS/6-transport/verify/egress-log.py
FIX=$L7_OS/1-bootstrap/crates/node111/tests/fixtures
BR=$L7_G12_BR
IP_O=$L7_G12_IP_O
IP_U=$L7_G12_IP_U
GW=10.111.72.1
DASH=http://127.0.0.1:$L7_PORT_DASH
ATTRIB=${ORIGIN_ATTRIB:-60}
EGRESS_SECS=${ORIGIN_EGRESS:-20}
REL=2026.11.02 # the release every scenario offers (the nodes run G6_VER 2026.10.20)

# ------------------------------------------------------------------ python helpers (written to the scratch root)
write_helpers() {
  local d=$1
  cat >"$d/mk.py" <<'PYEOF'
import argparse, hashlib, io, os, platform, random, subprocess, tarfile

FIX = os.environ["L7_FIX"]
ap = argparse.ArgumentParser()
ap.add_argument("out")
ap.add_argument("--release", default="2026.11.02")
ap.add_argument("--channel", default="stable")
ap.add_argument("--chain", default="gen0")  # gen0 gen1 broken-link gen2-broken (the l7-L1 keychain tails)
ap.add_argument("--signer", default="test")  # test gen1 gen2 other
ap.add_argument("--min-from", default="2026.10.01")
ap.add_argument("--pad", type=int, default=0)  # incompressible payload bytes (the RSS figure)
ap.add_argument("--decoy", default="http://127.0.0.1:9/never-fetched.tar.gz")
ap.add_argument("--no-blob", action="store_true")
ap.add_argument("--tamper", action="store_true")  # blob bytes differ from the pin, same size
a = ap.parse_args()
triple = "x86_64-unknown-linux-musl" if platform.machine() == "x86_64" else "aarch64-unknown-linux-musl"
files = [("bin/111", b"#!/bin/sh\necho fake 111 daemon\n", 0o755), ("share/kb/llms.txt", b"index\n", 0o644),
         ("share/kb/reticulum.md", b"entry " + a.release.encode() + b"\n", 0o644)]
if a.pad:
    files.append(("share/pad.bin", random.Random(111).randbytes(a.pad), 0o644))
files.append(("share/payload.manifest", "".join("%s  %s\n" % (hashlib.sha256(b).hexdigest(), p) for p, b, _ in files).encode(), 0o644))
buf = io.BytesIO()
with tarfile.open(fileobj=buf, mode="w:gz", format=tarfile.GNU_FORMAT) as t:
    for p, b, m in files:
        ti = tarfile.TarInfo(p)
        ti.size, ti.mode, ti.mtime = len(b), m, 0
        t.addfile(ti, io.BytesIO(b))
payload = buf.getvalue()
psha = hashlib.sha256(payload).hexdigest()
tail = {"gen0": "beta", "gen1": "gen1", "broken-link": "broken-link", "gen2-broken": "gen2-broken"}[a.chain]
txt = open("%s/l7-L1/%s/channel-111-stable.toml" % (FIX, tail)).read()
keychain = txt[txt.index("[[keychain]]"):]
head = '''manifest-version = 1
date = "%(rel)s"
channel = "%(ch)s"
release = "%(rel)s"
min-upgrade-from = "%(mf)s"

[pkg.111.target.%(t)s]
url = "https://example.invalid/111"
sha256 = "%(a)s"
size = 100

[payload.%(t)s]
url = "%(decoy)s"
sha256 = "%(psha)s"
size = %(plen)d

[rns-lock]
url = "https://example.invalid/rns-lock.txt"
sha256 = "%(a)s"
size = 100

[kb]
url = "https://example.invalid/kb.tar.gz"
sha256 = "%(a)s"
size = 100

''' % dict(rel=a.release, ch=a.channel, mf=a.min_from, t=triple, a="a" * 64, decoy=a.decoy, psha=psha, plen=len(payload))
os.makedirs(os.path.join(a.out, "blob"), exist_ok=True)
m = os.path.join(a.out, "channel-111-stable.toml")
open(m, "w").write(head + keychain)
key = {"test": "minisign/test.key", "other": "minisign/other.key", "gen1": "l7-L1/gen1.key", "gen2": "l7-L1/gen2.key"}[a.signer]
sig = m + ".minisig"
if os.path.exists(sig):
    os.unlink(sig)
subprocess.run([os.environ.get("MINISIGN", "minisign"), "-S", "-W", "-s", "%s/%s" % (FIX, key), "-t", "l7-V6 gate", "-m", m, "-x", sig],
               check=True, stdout=subprocess.DEVNULL)
if not a.no_blob:
    body = bytearray(payload)
    if a.tamper:
        body[-1] ^= 0xFF
    open(os.path.join(a.out, "blob", psha), "wb").write(bytes(body))
print(psha, len(payload))
PYEOF
  cat >"$d/srv.py" <<'PYEOF'
# srv.py DIR BIND PORT LOG BLOB_DELAY_S — a logging static file server (GET only): one "GET <path>" line per request
import http.server, os, sys, time

root, bind, port, log, delay = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4], float(sys.argv[5])


class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        with open(log, "a") as f:
            f.write("GET %s\n" % self.path)
        if self.path.startswith("/blob/") and delay:
            time.sleep(delay)
        p = os.path.normpath(os.path.join(root, self.path.lstrip("/")))
        if not p.startswith(root) or not os.path.isfile(p):
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        b = open(p, "rb").read()
        self.send_response(200)
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)


class S(http.server.ThreadingHTTPServer):
    daemon_threads = True


S((bind, port), H).serve_forever()
PYEOF
  cat >"$d/rss.py" <<'PYEOF'
# rss.py CMD... — run CMD, print its exit code and its peak RSS in KiB (ru_maxrss of the child alone)
import resource, subprocess, sys

p = subprocess.run(sys.argv[1:], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
print(p.returncode, resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss)
PYEOF
}

# ------------------------------------------------------------------ inner: runs inside the namespaces
declare -A HOLD=() DPID=() ROOT=()
inner() {
  local work=${G12_WORK:?} bin=${G12_BIN:?}
  # a node has no agent proxy: the daemon's https attempts must go to the bases (the box's env sends them to a proxy on loopback)
  unset HTTPS_PROXY https_proxy HTTP_PROXY http_proxy ALL_PROXY all_proxy
  W=$work/http
  mkdir -p "$W"
  # the box's resolver may be a loopback stub that fails at once; give the nodes one that is off-link and silent
  printf 'nameserver 192.0.2.53\noptions timeout:2 attempts:3\n' >"$work/resolv.conf"
  mount --bind "$work/resolv.conf" /etc/resolv.conf 2>/dev/null || note "could not bind a harness resolv.conf; the host's is used"
  local sniff_pid="" srv_pids=()
  export L7_FIX=$FIX

  cleanup_inner() {
    local n p
    for n in O U; do [ -n "${DPID[$n]:-}" ] && kill -TERM "${DPID[$n]}" 2>/dev/null; done
    for p in ${sniff_pid:-} "${srv_pids[@]}" "${HOLD[@]}"; do kill "$p" 2>/dev/null; done
    for _ in $(seq 1 30); do
      local alive=0
      for n in O U; do [ -n "${DPID[$n]:-}" ] && kill -0 "${DPID[$n]}" 2>/dev/null && alive=1; done
      [ "$alive" = 0 ] && break
      sleep 0.3
    done
  }
  trap cleanup_inner EXIT
  trap 'exit 130' INT TERM

  # ---- small helpers
  nx() { # NODE CMD... ; OUT = this (outer) namespace
    local n=$1
    shift
    if [ "$n" = OUT ]; then "$@"; else nsenter -t "${HOLD[$n]}" -n -- "$@"; fi
  }
  wait_for() { # SECS CMD... ; polls until CMD succeeds
    local end=$((SECONDS + $1))
    shift
    while :; do
      if "$@"; then return 0; fi
      [ "$SECONDS" -ge "$end" ] && return 1
      sleep 0.4
    done
  }
  chk() { # DESC COND : ok when the shell condition COND (eval'd here, so it sees the caller's locals) holds, else FAIL
    if eval "$2"; then ok "$1"; else fail "$1"; fi
  }
  sha() { sha256sum "$1" | cut -d' ' -f1; }
  ST=000
  HC() { # NODE METHOD URL [curl args] -> $ST, $W/h, $W/b
    local n=$1 m=$2 u=$3
    shift 3
    : >"$W/h"
    : >"$W/b"
    ST=$(nx "$n" curl -sS -o "$W/b" -D "$W/h" -w '%{http_code}' -X "$m" --max-time 20 "$@" "$u" 2>"$W/curl.err") || true
    [ -n "$ST" ] || ST=000
  }
  hdr() { grep -i "^$1:" "$W/h" | head -1 | cut -d: -f2- | tr -d '\r' | sed 's/^ *//'; }
  jpatch() { HC "$1" PATCH "$DASH$2" -H 'Content-Type: application/json' -d "$3"; }
  verb() { # ROOT [NODE] ARGS... -> $VRC, $VOUT (stdout+stderr), run as the node user in NODE's netns
    local root=$1 node=$2
    shift 2
    VOUT=$(NODE111_ROOT=$root nx "$node" "$bin" update "$@" 2>&1)
    VRC=$?
  }
  R=""
  fresh_root() { # NAME [ENVTEXT] -> $R, a scratch node root (current -> 2026.10.20, a stand-in agent); not in a subshell: ROOT[] is kept
    local r=$work/$1
    g6_make_root "$r" "${2:-}"
    ROOT[$1]=$r
    R=$r
  }
  staged() { [ -s "$1/state/update-request.json" ] && [ -f "$1/state/staging/$REL/payload.tar.gz" ]; }
  nothing_staged() { [ ! -e "$1/state/update-request.json" ] && [ -z "$(ls -A "$1/state/staging" 2>/dev/null)" ]; }

  ns_moved() { [ "$(readlink "/proc/$1/ns/net" 2>/dev/null)" != "$2" ]; }

  # ---- network: br 10.111.72.1/24 in this namespace; O and U one netns each
  ok "namespaces: user + net + pid, /proc is this namespace's own"
  ip link set lo up
  if ip link add "$BR" type bridge && ip addr add "$GW/24" dev "$BR" && ip link set "$BR" up; then
    ok "bridge $BR $GW/24 up (no route off-box)"
  else
    fail "cannot create $BR"
    return
  fi
  local myns n
  myns=$(readlink /proc/self/ns/net)
  for n in O U; do
    unshare --net sleep 100000 &
    HOLD[$n]=$!
  done
  local vname ip
  for n in O U; do
    wait_for 5 ns_moved "${HOLD[$n]}" "$myns" || true
    if [ "$n" = O ]; then
      vname=$L7_G12_VETH_O
      ip=$IP_O
    else
      vname=$L7_G12_VETH_U
      ip=$IP_U
    fi
    if ip link add "$vname" type veth peer name eth0 netns "${HOLD[$n]}" && ip link set "$vname" master "$BR" up &&
      nx "$n" ip link set lo up && nx "$n" ip addr add "$ip/24" dev eth0 && nx "$n" ip link set eth0 up; then
      ok "$n: netns (holder ${HOLD[$n]}) eth0 $ip/24 on $BR, no default route"
    else
      fail "$n: netns/veth setup failed"
      return
    fi
    if [ -n "$(nx "$n" ip route show default)" ]; then fail "$n: a default route exists"; fi
  done

  # ---- the servers: a request log standing in for every payload.url, a slow origin for the concurrent run
  local DECOY_PORT=18972 SLOW_PORT=18973
  : >"$work/decoy.log"
  : >"$work/slow.log"
  python3 -I "$work/srv.py" "$work/empty" "$GW" "$DECOY_PORT" "$work/decoy.log" 0 &
  srv_pids+=($!)
  mkdir -p "$work/empty" "$work/slow"
  local DECOY=http://$GW:$DECOY_PORT/never-fetched.tar.gz

  # ---- the origin directories (fixtures)
  mk() { # NAME [mk.py args] -> $work/o-NAME ; sets PSHA
    local name=$1
    shift
    PSHA=$(python3 -I "$work/mk.py" "$work/o-$name" --decoy "$DECOY" "$@" | cut -d' ' -f1)
  }
  mk ok
  local PIN=$PSHA
  mk beta --channel beta
  mk tamper --tamper
  mk gen1 --chain gen1 --signer gen1
  mk gen1root --chain gen1 --signer test
  mk broken --chain broken-link --signer gen1
  mk gen2 --chain gen2-broken --signer gen2
  mk gen2ok --chain gen2-broken --signer gen1
  mk minfrom --min-from 2099.01.01
  mk noblob --no-blob
  [ -s "$work/o-ok/channel-111-stable.toml.minisig" ] && ok "fixtures: signed manifests built with minisign (test key, l7-L1 gen1/gen2 keys), payload sha256 $PIN"

  # =================================================================== 1. the verb
  echo
  echo "== 1. the verb: T1, T2, T3, T5 and the staging lock (outer namespace, no daemon)"
  local r
  # T1
  fresh_root t1; r=$R
  verb "$r" OUT --from "$work/o-beta"
  chk "T1: a validly signed 'beta' manifest under the stable name is refused (exit $VRC: ${VOUT##*$'\n'})" 'test "$VRC" = 1 && grep -q '\''not the channel asked for ("stable")'\'' <<<"$VOUT"'
  chk "T1: nothing staged and no update request after the refusal" 'nothing_staged "$r"'
  fresh_root t1c; r=$R
  verb "$r" OUT --from "$work/o-ok"
  chk "T1 control: the same manifest under channel 'stable' stages (exit $VRC)" 'test "$VRC" = 0 && staged "$r"'
  # T2
  fresh_root t2a; r=$R
  verb "$r" OUT --from "$work/o-gen1"
  chk "T2: a generation-1 release signed by gen 1, linked from the compiled key, stages (exit $VRC)" 'test "$VRC" = 0 && staged "$r"'
  fresh_root t2b; r=$R
  verb "$r" OUT --from "$work/o-gen1root"
  chk "T2: the same chain signed by the compiled key only is refused (not the highest generation reached)" 'test "$VRC" = 1 && grep -q '\''does not verify under keychain generation 1'\'' <<<"$VOUT" && nothing_staged "$r"'
  fresh_root t2c; r=$R
  verb "$r" OUT --from "$work/o-broken"
  chk "T2: a broken signature_by_previous is refused (the walk stopped at generation 0)" 'test "$VRC" = 1 && grep -q "generation 1'\''s signature_by_previous does not verify under generation 0" <<<"$VOUT" && nothing_staged "$r"'
  fresh_root t2d; r=$R
  verb "$r" OUT --from "$work/o-gen2"
  chk "T2: generation 2 behind a broken link is never reached: a manifest signed by it is refused" 'test "$VRC" = 1 && grep -q "generation 2'\''s signature_by_previous does not verify under generation 1" <<<"$VOUT" && nothing_staged "$r"'
  fresh_root t2e; r=$R
  verb "$r" OUT --from "$work/o-gen2ok"
  chk "T2: the same chain's manifest signed by generation 1 (the highest reached) stages" 'test "$VRC" = 0 && staged "$r"'
  fresh_root t2f; r=$R
  printf '{"schema":1,"highest_release":"","highest_manifest_date":"","highest_keychain_generation":1}\n' >"$r/state/floor.json"
  verb "$r" OUT --from "$work/o-ok"
  chk "T2: with the floor at generation 1 a generation-0 release is refused (keychain gen 0 vs floor 1)" 'test "$VRC" = 1 && grep -q '\''keychain gen 0 vs floor'\'' <<<"$VOUT" && nothing_staged "$r"'
  fresh_root t2g; r=$R
  printf '{"schema":1,"highest_release":"","highest_manifest_date":"","highest_keychain_generation":1}\n' >"$r/state/floor.json"
  verb "$r" OUT --from "$work/o-gen1"
  chk "T2 control: the same floor lets the generation-1 release through" 'test "$VRC" = 0 && staged "$r"'
  # T3 (the daemon's staging decision; the helper's refusal is a node111 test)
  fresh_root t3; r=$R
  verb "$r" OUT --from "$work/o-minfrom"
  chk "T3: a release whose min-upgrade-from (2099.01.01) is above the running 2026.10.20 is refused, nothing staged (${VOUT##*$'\n'})" 'test "$VRC" = 1 && grep -q '\''needs at least 2099.01.01 installed'\'' <<<"$VOUT" && nothing_staged "$r"'
  # --from <dir>: stages, writes the request file and nothing else; tamper; missing blob
  fresh_root dir; r=$R
  verb "$r" OUT --from "$work/o-ok"
  chk "--from <dir>: exit 0 and 'staged $REL' (${VOUT##*$'\n'})" 'test "$VRC" = 0 && grep -q "^staged $REL" <<<"$VOUT"'
  chk "--from <dir>: the staged payload is byte-for-byte the pinned blob (sha256 $PIN)" 'test "$(sha "$r/state/staging/$REL/payload.tar.gz" 2>/dev/null)" = "$PIN"'
  chk "--from <dir>: the request file names $REL and the staged path" 'jq -e --arg v "$REL" '\''.version == $v and (.staged_path | endswith("/state/staging/" + $v))'\'' "$r/state/update-request.json" >/dev/null'
  chk "T5: state/ holds only staging, staging.lock, update-origins.json, update-request.json" 'test "$(ls "$r/state" | tr '\''\n'\'' '\'' '\'')" = "staging staging.lock update-origins.json update-request.json "'
  chk "T5: no release installed and no current switch (releases/ = the one the node runs)" 'test "$(ls "$r/releases")" = "2026.10.20" && test "$(readlink "$r/current")" = "releases/2026.10.20"'
  fresh_root tamper; r=$R
  verb "$r" OUT --from "$work/o-tamper"
  chk "a tampered blob is refused (${VOUT##*$'\n'}) and staging is removed (no staging dir, no request)" 'test "$VRC" = 1 && grep -q '\''payload sha256 .* != manifest'\'' <<<"$VOUT" && nothing_staged "$r"'
  fresh_root noblob; r=$R
  verb "$r" OUT --from "$work/o-noblob"
  chk "a missing blob (the signed manifest pins one, the directory holds none) stages nothing and exits 1" 'test "$VRC" = 1 && grep -q '\''payload fetch failed'\'' <<<"$VOUT" && nothing_staged "$r"'

  # the staging lock held by another process (the same flock the daemon's check takes)
  fresh_root lock; r=$R
  python3 -I -c 'import fcntl, sys, time
f = open(sys.argv[1], "a")
fcntl.flock(f, fcntl.LOCK_EX)
time.sleep(60)' "$r/state/staging.lock" &
  local holder=$!
  sleep 0.5
  verb "$r" OUT --from "$work/o-ok"
  chk "S1: a --from run while state/staging.lock is held exits 75 ('the daemon is checking now')" 'test "$VRC" = 75 && grep -q '\''the daemon is checking now'\'' <<<"$VOUT"'
  chk "S1: it staged nothing, wrote no request and no per-origin record" 'nothing_staged "$r" && test ! -e "$r/state/update-origins.json"'
  kill "$holder" 2>/dev/null
  wait "$holder" 2>/dev/null
  verb "$r" OUT --from "$work/o-ok"
  chk "S1 control: with the lock free the same run stages (exit $VRC)" 'test "$VRC" = 0 && staged "$r"'

  # a concurrent run stages once, the payload at blob/<sha256> over http, never at payload.url, no last-update-check.json
  cp -r "$work/o-ok/." "$work/slow/"
  python3 -I "$work/srv.py" "$work/slow" 127.0.0.1 "$SLOW_PORT" "$work/slow.log" 2.5 &
  srv_pids+=($!)
  wait_for 5 bash -c "exec 3<>/dev/tcp/127.0.0.1/$SLOW_PORT" 2>/dev/null || true
  : >"$work/slow.log"
  local decoy0
  decoy0=$(wc -l <"$work/decoy.log")
  fresh_root conc; r=$R
  (NODE111_ROOT=$r "$bin" update --from "http://127.0.0.1:$SLOW_PORT/" >"$work/conc-a.out" 2>&1; echo $? >"$work/conc-a.rc") &
  local ca=$!
  sleep 1.2
  verb "$r" OUT --from "http://127.0.0.1:$SLOW_PORT/"
  local brc=$VRC bout=$VOUT
  wait "$ca"
  chk "S1: two concurrent runs: the first stages (exit $(cat "$work/conc-a.rc"))" 'test "$(cat "$work/conc-a.rc")" = 0'
  chk "S1: the second, started while the first holds the lock, exits 75" 'test "$brc" = 75 && grep -q '\''the daemon is checking now'\'' <<<"$bout"'
  chk "S1: staged once: one staging dir, one blob GET on the origin ($(grep -c '^GET /blob/' "$work/slow.log") blob GET)" 'test "$(ls "$r/state/staging" | wc -l)" = 1 && test "$(grep -c '\''^GET /blob/'\'' "$work/slow.log")" = 1'
  chk "S5: a typed --from <url> fetched the manifest, its signature and GET /blob/$PIN, in that order" 'test "$(tr '\''\n'\'' '\'' '\'' <"$work/slow.log")" = "GET /channel-111-stable.toml GET /channel-111-stable.toml.minisig GET /blob/$PIN "'
  chk "S5: payload.url (a decoy on $GW:$DECOY_PORT) was never requested" 'test "$(wc -l <"$work/decoy.log")" = "$decoy0"'
  # the decoy's log must be able to show a request (else the line above proves nothing)
  curl -s -o /dev/null --max-time 5 "http://$GW:$DECOY_PORT/probe" || true
  chk "S5 control: the decoy's request log does record a request to it" 'test "$(wc -l <"$work/decoy.log")" = $((decoy0 + 1))'
  chk "S5: no last-update-check.json (record_success is for internet bases only) and the per-origin record is kind 'url'" 'test ! -e "$r/state/last-update-check.json" && jq -e '\''.origins[0].kind == "url" and .origins[0].answer == "staged"'\'' "$r/state/update-origins.json" >/dev/null'

  # the transient RSS of a staged update (§9.6): a stage of a 5.6 MB payload against a refused-manifest baseline
  mk pad --pad 5600000
  local pad_size
  pad_size=$(stat -c %s "$work/o-pad/blob/$PSHA")
  fresh_root rssb; r=$R
  local base big
  base=$(NODE111_ROOT=$r python3 -I "$work/rss.py" "$bin" update --from "$work/o-beta")
  fresh_root rssb2; r=$R
  big=$(NODE111_ROOT=$r python3 -I "$work/rss.py" "$bin" update --from "$work/o-pad")
  if [ "${big%% *}" = 0 ] && staged "$r" && [ "${base%% *}" = 1 ]; then
    ok "RSS record: the verb staged a $pad_size-byte payload with peak RSS ${big##* } KiB; a refused manifest (the baseline) peaks at ${base##* } KiB: the transient is $((${big##* } - ${base##* })) KiB (debug build; the payload is read whole, §9.6)"
  else
    fail "RSS record: the measuring runs did not behave (stage '$big', baseline '$base')"
  fi

  # =================================================================== 2. the hub
  echo
  echo "== 2. the hub O (D7, §9.1): /origin/v1/* behind hub_role, LAN sharing and origin.serve"
  local O_ENV="DASHBOARD_LAN=true" O_HUB=true O_LAN=true
  if [ "${ORIGIN_NEG:-}" = noserve ]; then
    O_ENV=""
    O_HUB=false
    O_LAN=false
    note "ORIGIN_NEG=noserve: the hub starts without LAN sharing and without the hub role (the serving lines must FAIL)"
  fi
  local OR UR NID_O NID_U
  fresh_root O "$O_ENV"; OR=$R
  fresh_root U ""; UR=$R
  NID_O=$(g6_identity "$OR" "$bin")
  NID_U=$(g6_identity "$UR" "$bin")
  if [ -n "$NID_O" ] && [ -n "$NID_U" ]; then ok "identities: O $NID_O, U $NID_U"; else
    fail "identity generation failed"
    return
  fi
  g6_fabric_settings "$OR" "{\"schema\":1,\"hub_role\":$O_HUB,\"listen_lan\":$O_LAN}"
  g6_fabric_settings "$UR" '{"schema":1,"listen_lan":false}'

  start_daemon() { # NODE -> waits for /health
    local n=$1 up=0 code
    NODE111_ROOT=${ROOT[$n]} nsenter -t "${HOLD[$n]}" -n -- "$bin" daemon >>"$work/$n.daemon.log" 2>&1 &
    DPID[$n]=$!
    for _ in $(seq 1 150); do
      code=$(nx "$n" curl -s -o /dev/null -w '%{http_code}' --max-time 2 "$DASH/health" 2>/dev/null) || true
      case $code in 200 | 503)
        up=1
        break
        ;;
      esac
      kill -0 "${DPID[$n]}" 2>/dev/null || break
      sleep 0.2
    done
    if [ "$up" = 1 ]; then return 0; fi
    fail "$n: 111 daemon never answered /health"
    tail -15 "$work/$n.daemon.log" | sed "s/^/  $n daemon| /"
    return 1
  }
  fabric_open() { HC "$1" GET "$DASH/api/v1/fabric" && [ "$ST" = 200 ] && jq -e '.store.open == true' "$W/b" >/dev/null 2>&1; }
  wait_fabric() { # NODE -> the fabric is running (the role bits and the origin gate are set after /health answers)
    wait_for 30 fabric_open "$1" || note "$1: the fabric store was not open after 30 s"
  }
  stop_daemon() {
    local n=$1 p=${DPID[$1]:-}
    [ -n "$p" ] || return 0
    kill -TERM "$p" 2>/dev/null
    for _ in $(seq 1 100); do
      kill -0 "$p" 2>/dev/null || break
      sleep 0.2
    done
    wait "$p" 2>/dev/null
    DPID[$n]=""
  }
  start_daemon O || return
  wait_fabric O
  ok "O: 111 daemon up (pid ${DPID[O]}), hub_role=$O_HUB, LAN sharing $([ -n "$O_ENV" ] && echo on || echo off)"
  wait_for 10 nx U bash -c "exec 3<>/dev/tcp/$IP_O/$L7_PORT_DASH" 2>/dev/null || true

  O_GET() { HC U GET "http://$IP_O:$L7_PORT_DASH/origin/v1/$1"; } # from the member's netns: a LAN peer, no session
  local M=channel-111-stable.toml S=channel-111-stable.toml.minisig
  O_GET "$M"
  chk "hub with origin.serve off: GET /origin/v1/$M answers 404 (HTTP $ST)" 'test "$ST" = 404'

  # the consent gate, then on
  jpatch O /api/v1/relay/settings '{"origin":{"serve":true}}'
  if [ "$ST" = 409 ] && [ "$(jq -r .code "$W/b")" = consent_required ]; then
    ok "PATCH origin.serve true: 409 consent_required (action $(jq -r .consent.action "$W/b"))"
    local hash
    hash=$(jq -r .consent.hash "$W/b")
    jpatch O /api/v1/relay/settings "{\"origin\":{\"serve\":true},\"accept_warning\":\"$hash\"}"
    chk "the same call with accept_warning = the hash answers 200, origin.serve true" 'test "$ST" = 200 && jq -e '\''.origin.serve == true'\'' "$W/b" >/dev/null'
  else
    fail "PATCH origin.serve true: wanted 409 consent_required, got HTTP $ST $(head -c 200 "$W/b")"
  fi
  O_GET "$M"
  chk "serving, but no release held yet: still 404 (as when nothing is admitted)" 'test "$ST" = 404'

  # admit: the hub stages the release with the verb, as the node user; a serving hub admits what it stages
  verb "$OR" O --from "$work/o-ok"
  chk "O: 111 update --from <dir> stages $REL and (origin.serve on) admits it into state/origin/ (exit $VRC)" 'test "$VRC" = 0 && test -s "$OR/state/origin/$M" && test -s "$OR/state/origin/$S" && test -s "$OR/state/origin/blob/$PIN"'
  HC O GET "$DASH/api/v1/relay"
  chk "GET /api/v1/relay: origin.serve true, origin.bytes > 0 ($(jq -c .origin "$W/b" 2>/dev/null))" 'jq -e '\''.origin.serve == true and .origin.bytes > 0'\'' "$W/b" >/dev/null'

  O_GET "$M"
  local s1=$ST
  chk "serving: GET $M from the LAN (no session): 200, byte-exact (HTTP $s1)" 'test "$s1" = 200 && cmp -s "$W/b" "$work/o-ok/$M"'
  chk "  the CSP is the dashboard's and nothing sets a cookie" 'test -n "$(hdr content-security-policy)" && test -z "$(hdr set-cookie)"'
  O_GET "$S"
  chk "serving: GET $S: 200, byte-exact" 'test "$ST" = 200 && cmp -s "$W/b" "$work/o-ok/$S"'
  O_GET "blob/$PIN"
  chk "serving: GET blob/$PIN: 200, the body's sha256 is the pin and its length the pinned size" 'test "$ST" = 200 && test "$(sha "$W/b")" = "$PIN" && test "$(hdr content-length)" = "$(stat -c %s "$work/o-ok/blob/$PIN")"'
  HC U GET "http://$IP_O:$L7_PORT_DASH/api/v1/relay"
  chk "no session needed here, but the session gate is real: /api/v1/relay from the same peer is 401 (HTTP $ST)" 'test "$ST" = 401'
  local meth bad=""
  for meth in POST PUT PATCH DELETE; do
    for p in "$M" "$S" "blob/$PIN"; do
      HC U "$meth" "http://$IP_O:$L7_PORT_DASH/origin/v1/$p" -H 'Content-Length: 0'
      [ "$ST" = 405 ] || bad="$bad $meth/$p=$ST"
    done
  done
  chk "non-GET: POST, PUT, PATCH, DELETE on all three names answer 405${bad:+ (not:$bad)}" 'test -z "$bad"'
  bad=""
  for p in "$M" "$S" "blob/$PIN"; do
    HC U HEAD "http://$IP_O:$L7_PORT_DASH/origin/v1/$p" --head
    [ "$ST" = 405 ] || bad="$bad HEAD/$p=$ST"
  done
  chk "non-GET: HEAD on all three names answers 405 (§9.1 GET-only)${bad:+ (not:$bad)}" 'test -z "$bad"'
  O_GET "blob/$(printf 'e%.0s' $(seq 1 64))"
  chk "a missing blob (a well-formed sha256 no pin names) 404s" 'test "$ST" = 404'
  O_GET "blob/..%2f$M"
  chk "a path-shaped blob name 404s" 'test "$ST" = 404'
  O_GET "blob/$(printf 'E%.0s' $(seq 1 64))"
  chk "an uppercase (misnamed) sha256 404s" 'test "$ST" = 404'
  O_GET "nothing-here.toml"
  chk "a name outside the three 404s" 'test "$ST" = 404'
  HC O GET "$DASH/origin/v1/$M" -H 'Host: localhost'
  chk "a loopback request with Host: localhost is served, not redirected (the machine-contract exemption; HTTP $ST)" 'test "$ST" = 200'

  # the switch off at run time, then on again
  jpatch O /api/v1/relay/settings '{"origin":{"serve":false}}'
  O_GET "$M"
  chk "origin.serve turned off at run time: the routes answer 404 at once (PATCH $ST)" 'test "$ST" = 404'
  jpatch O /api/v1/relay/settings '{"origin":{"serve":true}}'
  if [ "$ST" = 409 ]; then
    jpatch O /api/v1/relay/settings "{\"origin\":{\"serve\":true},\"accept_warning\":\"$(jq -r .consent.hash "$W/b")\"}"
  fi
  O_GET "$M"
  chk "and on again (a switch turned off forgets its consent and asks again): 200" 'test "$ST" = 200'

  # LAN sharing off, then the hub role off, each alone, across a restart; origin.serve stays true in relay.json
  stop_daemon O
  printf 'DASHBOARD_LAN=false\n' >"$OR/etc/env"
  start_daemon O || return
  wait_fabric O
  HC O GET "$DASH/origin/v1/$M"
  chk "restart with LAN sharing off (hub role and origin.serve still on): 404 on loopback (HTTP $ST)" 'test "$ST" = 404'
  stop_daemon O
  printf 'DASHBOARD_LAN=true\n' >"$OR/etc/env"
  g6_fabric_settings "$OR" '{"schema":1,"hub_role":false,"listen_lan":true}'
  start_daemon O || return
  wait_fabric O
  O_GET "$M"
  chk "restart with the hub role off (LAN sharing and origin.serve on): 404 from the LAN (HTTP $ST)" 'test "$ST" = 404'
  stop_daemon O
  g6_fabric_settings "$OR" '{"schema":1,"hub_role":true,"listen_lan":true}'
  start_daemon O || return
  wait_fabric O
  o_serves() { O_GET "$M" && [ "$ST" = 200 ]; }
  local t0=$SECONDS
  wait_for 20 o_serves || true
  note "after the restart the routes answered 200 $((SECONDS - t0)) s after the fabric store opened (before the fabric runs they answer 404: fail closed)"
  chk "restart with all three on: 200 again, the held release survived the restarts (HTTP $ST)" 'test "$ST" = 200 && cmp -s "$W/b" "$work/o-ok/$M"'
  if [ "$ST" != 200 ]; then nx O ss -ltn 2>&1 | sed 's/^/  O ss| /'; tail -5 "$work/O.daemon.log" | sed 's/^/  O daemon| /'; fi

  # =================================================================== 3. the member
  echo
  echo "== 3. the member U (L4, S4, S5): no internet route, enrolled at O"
  start_daemon U || return
  ok "U: 111 daemon up (pid ${DPID[U]}), no default route"
  local rss_before
  rss_before=$(awk '/^VmRSS:/ { print $2 }' /proc/"${DPID[U]}"/status)
  local offer bundle
  offer=$(NODE111_ROOT=$OR nx O "$bin" fabric offer 2>&1) || true
  bundle=$(printf '%s\n' "$offer" | sed -n "s/^On the member: 111 fabric enroll '\\(.*\\)'\$/\\1/p" | head -1)
  if [[ "$bundle" == 111HUB:* ]]; then
    ok "O: \`111 fabric offer\` printed a 111HUB: bundle"
  else
    fail "O: no 111HUB: bundle in the offer: $(printf '%s' "$offer" | head -c 200)"
  fi
  local decoy1
  decoy1=$(wc -l <"$work/decoy.log")
  local en
  en=$(NODE111_ROOT=$UR nx U "$bin" fabric enroll "$bundle" --address "$IP_O:$L7_PORT_FABRIC" 2>&1) || true
  chk "U: \`111 fabric enroll\` at $IP_O:$L7_PORT_FABRIC: $(printf '%s' "$en" | head -c 90)" 'grep -q "^Enrolled with hub $NID_O" <<<"$en"'
  wait_for "$ATTRIB" staged "$UR" || true
  if staged "$UR"; then
    ok "U staged $REL from its home hub within the wait (the daemon's own check on the attribution edge)"
  else
    fail "U staged nothing from its hub in ${ATTRIB}s"
    tail -8 "$work/U.daemon.log" | sed 's/^/  U daemon| /'
  fi
  chk "S4: the staged payload is the pinned blob (sha256 $PIN), the request file names $REL" 'test "$(sha "$UR/state/staging/$REL/payload.tar.gz" 2>/dev/null)" = "$PIN" && jq -e --arg v "$REL" '\''.version == $v'\'' "$UR/state/update-request.json" >/dev/null'
  chk "S4: the daemon log says it staged it from the home hub" 'grep -q "staged $REL from the home hub" "$work/U.daemon.log"'
  chk "S5: payload.url (the decoy) was never requested by the member" 'test "$(wc -l <"$work/decoy.log")" = $((decoy1))'
  chk "S5: no last-update-check.json on U (a hub answer never counts as the dashboard's last successful check)" 'test ! -e "$UR/state/last-update-check.json"'
  HC U GET "$DASH/api/v1/update/origins"
  chk "GET /api/v1/update/origins: the hub origin (http://$IP_O:$L7_PORT_DASH/origin/v1/) is listed, its last answer ok, newest_seen $REL" \
    'jq -e --arg b "http://$IP_O:$L7_PORT_DASH/origin/v1/" --arg v "$REL" '\''[.origins[] | select(.kind == "hub" and .url == $b and .last_answer.ok == true and .newest_seen == $v)] | length == 1'\'' "$W/b" >/dev/null'
  chk "S4: U's per-origin record holds one origin, the home hub, answer 'staged'" \
    'jq -e '\''(.origins | length) == 1 and .origins[0].kind == "home_hub" and .origins[0].answer == "staged"'\'' "$UR/state/update-origins.json" >/dev/null'
  local rss_after hwm
  rss_after=$(awk '/^VmRSS:/ { print $2 }' /proc/"${DPID[U]}"/status)
  hwm=$(awk '/^VmHWM:/ { print $2 }' /proc/"${DPID[U]}"/status)
  note "RSS record (U's daemon, debug build): VmRSS $rss_before KiB before enrolling, $rss_after KiB after staging the small fixture payload ($(stat -c %s "$UR/state/staging/$REL/payload.tar.gz" 2>/dev/null) bytes), VmHWM $hwm KiB; the 5.6 MB transient is measured on the verb above"

  # a missing blob at the hub: nothing staged, the 404 is the hub's; the next attribution recovers
  stop_daemon U
  rm -rf -- "$UR/state/staging"
  rm -f -- "$UR/state/update-request.json" "$UR/state/update-origins.json"
  mv "$OR/state/origin/blob/$PIN" "$work/blob.held"
  O_GET "blob/$PIN"
  local hs=$ST
  O_GET "$M"
  chk "O's blob removed from state/origin/blob: the blob 404s (HTTP $hs) while the manifest still answers (HTTP $ST)" 'test "$hs" = 404 && test "$ST" = 200'
  : >"$work/U.daemon.log"
  start_daemon U || return
  hub_rec() { jq -e '[.origins[] | select(.kind == "home_hub")] | length == 1' "$UR/state/update-origins.json" >/dev/null 2>&1; }
  wait_for "$ATTRIB" hub_rec || true
  chk "U with the blob missing at its hub: nothing staged, no request file" 'nothing_staged "$UR"'
  chk "  the hub's record answers not_found ($(jq -c '[.origins[] | {kind, answer}]' "$UR/state/update-origins.json" 2>/dev/null)): its 404 on the blob" \
    'jq -e '\''[.origins[] | select(.kind == "home_hub" and .answer == "not_found")] | length == 1'\'' "$UR/state/update-origins.json" >/dev/null'
  chk "  the daemon log carries the hub's failure, not a stage" 'grep -q "the home hub'\''s origin" "$work/U.daemon.log" && ! grep -q '\''staged .* from the home hub'\'' "$work/U.daemon.log"'
  mv "$work/blob.held" "$OR/state/origin/blob/$PIN"
  stop_daemon U
  : >"$work/U.daemon.log"
  start_daemon U || return
  wait_for "$ATTRIB" staged "$UR" || true
  chk "the blob back at the hub: the next attribution (a daemon restart) stages $REL" 'staged "$UR"'
  stop_daemon U
  stop_daemon O
  ok "O and U stopped"

  # =================================================================== 4. updates.check_internet
  echo
  echo "== 4. updates.check_internet: the egress log on U's veth, with a default route and no answer behind it"
  nx U ip route add default via "$GW" dev eth0 || fail "U: cannot add the black-hole default route"
  local E1 E2
  fresh_root E1 ""; E1=$R
  fresh_root E2 ""; E2=$R
  g6_identity "$E1" "$bin" >/dev/null
  g6_identity "$E2" "$bin" >/dev/null
  mkdir -p "$E1/state" "$E2/state"
  printf '{"schema":1,"updates":{"check_internet":false}}\n' >"$E1/state/relay.json"
  printf '{"schema":1,"updates":{"check_internet":true}}\n' >"$E2/state/relay.json"
  chmod 600 "$E1/state/relay.json" "$E2/state/relay.json"

  sniff_run() { # NAME -> starts the logger on U's eth0
    : >"$work/$1.err"
    nsenter -t "${HOLD[U]}" -n -- python3 -I "$EGRESS" sniff --iface eth0 --allow "$IP_O,$IP_U" \
      --out "$work/$1.jsonl" --summary "$work/$1.summary.json" --ready "$work/$1.ready" >"$work/$1.err" 2>&1 &
    sniff_pid=$!
    wait_for 5 test -s "$work/$1.ready"
  }
  sniff_end() { # NAME -> $OFF = off-link frames out
    kill -TERM "$sniff_pid" 2>/dev/null
    wait "$sniff_pid" 2>/dev/null
    sniff_pid=""
    OFF=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["counts"].get("off_link_out", 0))' "$work/$1.summary.json" 2>/dev/null || echo '?')
  }
  local OFF

  # pass A: check_internet = false
  : >"$work/U.daemon.log"
  ROOT[U]=$E1
  if sniff_run passA; then
    start_daemon U || return
    sleep 3
    verb "$E1" U --from "$work/o-ok"
    chk "check_internet=false: the daemon holds no staging lock, so a --from run stages at once (exit $VRC)" 'test "$VRC" = 0 && staged "$E1"'
    sleep "$EGRESS_SECS"
    sniff_end passA
    stop_daemon U
    chk "check_internet=false: no internet base contacted: $OFF off-link frames out of U in ${EGRESS_SECS}s (the hourly loop's first round included; a dashboard wake is not exercised here)" 'test "$OFF" = 0'
    chk "  the daemon did not run an internet check (no 'updater: degraded', no last-update-check.json)" 'test ! -e "$E1/state/last-update-check.json" && ! grep -q '\''updater: degraded'\'' "$work/U.daemon.log"'
  else
    fail "the AF_PACKET logger did not bind U's eth0 ($(head -c 200 "$work/passA.err"))"
  fi

  # pass B: check_internet = true (the positive control)
  : >"$work/U.daemon.log"
  ROOT[U]=$E2
  if sniff_run passB; then
    start_daemon U || return
    lock_held() { python3 -I -c 'import fcntl,sys
f=open(sys.argv[1],"a+")
try: fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)
except OSError: sys.exit(0)
sys.exit(1)' "$E2/state/staging.lock" 2>/dev/null; }
    wait_for 15 lock_held || note "the daemon never took state/staging.lock within 15 s"
    verb "$E2" U --from "$work/o-ok"
    chk "check_internet=true: the daemon's own check holds state/staging.lock while it reaches for the internet base: a --from run exits 75 and stages nothing (exit $VRC)" 'test "$VRC" = 75 && grep -q '\''the daemon is checking now'\'' <<<"$VOUT" && nothing_staged "$E2" && ! jq -e '\''[.origins[]? | select(.kind == "dir")] | length > 0'\'' "$E2/state/update-origins.json" >/dev/null 2>&1'
    sleep "$EGRESS_SECS"
    sniff_end passB
    stop_daemon U
    chk "check_internet=true (positive control): the logger sees off-link frames out of U ($OFF)" 'test "$OFF" != 0 && test "$OFF" != '\''?'\'''
    chk "  and a DNS query for an internet base ($(grep -o '"dns": "[^"]*"' "$work/passB.jsonl" 2>/dev/null | sort -u | tr '\n' ' '))" 'grep -Eq '\''"dns": "(install\.111\.network|github\.com)'\'' "$work/passB.jsonl"'
  else
    fail "the AF_PACKET logger did not bind U's eth0 for pass B ($(head -c 200 "$work/passB.err"))"
  fi
  stop_daemon U
  cleanup_inner
  trap - EXIT
  ok "daemons, logger and servers stopped"
}

if [ "${1:-}" = --inner ]; then
  . 6-transport/verify/root.sh
  inner
  printf 'FAILS=%d\n' "$FAILS" >"${G12_WORK:?}/inner.result"
  exit 0
fi

# ------------------------------------------------------------------ outer
for tool in python3 ip unshare awk nsenter curl jq minisign; do
  command -v "$tool" >/dev/null 2>&1 || skip_or_fail G12 "$tool is absent"
done
have_netns || skip_or_fail G12 "unprivileged network namespaces are not available (unshare -Urnm)"
unshare --user --map-root-user --net --pid --fork --mount-proc true >/dev/null 2>&1 ||
  skip_or_fail G12 "a pid namespace with its own /proc is not available (unshare --pid --fork --mount-proc)"
python3 -I -c 'import random,sys; sys.exit(0 if hasattr(random.Random, "randbytes") else 1)' ||
  skip_or_fail G12 "python3 >= 3.9 is needed (random.randbytes)"
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
if [ -z "$BIN111" ] || [ ! -x "$BIN111" ]; then skip_or_fail G12 "no built 111 binary (set BIN111 or CARGO_TARGET_DIR)"; fi
BIN111=$(cd "$(dirname "$BIN111")" && pwd)/$(basename "$BIN111")
[ -r "$EGRESS" ] || skip_or_fail G12 "missing $EGRESS"
for f in minisign/test.key l7-L1/gen1.key l7-L1/gen2.key l7-L1/gen1/channel-111-stable.toml; do
  [ -r "$FIX/$f" ] || skip_or_fail G12 "missing fixture $FIX/$f"
done

WORK=$(mktemp -d "${TMPDIR:-/tmp}/l7-origin.XXXXXX") || exit 1
cleanup() {
  if [ "${ORIGIN_KEEP:-0}" = 1 ]; then note "kept $WORK"; else rm -rf "${WORK:?}"; fi
}
trap cleanup EXIT
write_helpers "$WORK"

if env --default-signal true 2>/dev/null; then
  RESET_SIG=(env --default-signal)
else
  RESET_SIG=(python3 -c 'import os,signal,sys
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])')
fi
INNER_LOG=$WORK/inner.log
G12_WORK=$WORK G12_BIN=$BIN111 \
  unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
  "${RESET_SIG[@]}" bash "$0" --inner >"$INNER_LOG" 2>&1 &
NS_PID=$!
trap 'kill -TERM "$NS_PID" 2>/dev/null; wait "$NS_PID" 2>/dev/null; cleanup; exit 130' INT TERM
tail -n +1 -f "$INNER_LOG" --pid="$NS_PID" &
TAIL_PID=$!
wait "$NS_PID" 2>/dev/null
wait "$TAIL_PID" 2>/dev/null
trap cleanup EXIT
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
  if { tr '\0' '\n' <"$p/environ"; } 2>/dev/null | grep -qE "^(NODE111_ROOT|G12_WORK)=$WORK\$" ||
    { tr '\0' '\n' <"$p/cmdline"; } 2>/dev/null | grep -qF "$WORK"; then
    left="$left $pid:$(cat "$p/comm" 2>/dev/null)"
  fi
done
if [ -z "$left" ]; then ok "no daemon, server, logger or holder of this run is left"; else fail "left running:$left"; fi
finish G12
