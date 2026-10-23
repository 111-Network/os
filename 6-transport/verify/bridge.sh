#!/usr/bin/env bash
# 6-transport/verify/bridge.sh — Layer 6 gate G1 (spec §16 G1; TASKS.md V1): the bridge core
# through the real /api/v1 of real `111 daemon`s with three StubAdapters of differing caps.
#
#   bash 6-transport/verify/bridge.sh                 runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G1=1 bash 6-transport/verify/bridge.sh    lock run: a missing prerequisite is a FAIL
#
# What runs (every check prints ok:/FAIL:, note: is information; exit 1 on any FAIL):
#   1. cargo   the debug `111` (1-bootstrap; CARGO_TARGET_DIR is honoured) — the G1 scenario hook
#              (`N/bridge/stub.rs` `scenario`, debug builds only) is what enables the stubs;
#              mesh111's store tests that §16 G1 names (V2 opens at V3, the prune order with the
#              foreign quota, bounded foreign_seen; G1_MESH_TARGET_DIR is mesh111's target dir),
#              and "a V3 store is refused by a V2 schema" as a mesh111 unit test with
#              user_version = 3 run on the Layer 5 tag's own V2 mesh111 (`git archive
#              checkpoint-layer-5`, read-only, built in the scratch root).
#   2. roots   two scratch roots A and B (identity, releases/<ver>, a stand-in 111-agent that
#              records the daemon's bearer, fabric/settings A→B direct), a scenario file each
#              (NODE111_BRIDGE_STUBS): A carries stub-mesh (channel + DM, 232/220 B, licensed caps,
#              LongFast airtime, two accounts, live licensed/uplink/region facts), stub-lxmf (DM +
#              recipient lists, 32-hex syntax, globally unique ids) and stub-app (DM + groups,
#              120 B groups); B carries stub-mesh and stub-app. The two nodes' stubs share an "air"
#              directory: an accepted send on one is heard on the other (the cross-node loops).
#   3. netns   `unshare -rnpf` (user + net + pid): br0 10.111.6.1/24, A 10.111.6.11 and
#              B 10.111.6.12 on veth pairs; every daemon, curl and stand-in dies with the
#              namespace. Every assertion goes through a node's own loopback /api/v1 (curl via
#              nsenter) or the stub's send log (what reached the "radio").
#   4. tapes   network_message and bridge SSE frames of A's run into 6-transport/verify/tapes/
#              (only when missing, or G1_TAPES=1; G1_TAPES_DIR=DIR writes elsewhere).
#
# Env: SMOKE_KEEP=1 keep the scratch root · G1_SKIP_BUILD=1 (or SMOKE_SKIP_BUILD=1) reuse the built
#      `111` · G1_SKIP_CARGO_TESTS=1 skip the mesh111 test stages (noted, never counted) ·
#      TMPDIR where the scratch root goes. Nothing of ours stays running; the scratch root is
#      removed on success.
#
# Checks are condition strings eval'd by t() at check time, so single-quoted `$vars` are intended:
# shellcheck disable=SC2016,SC2034,SC2317
set -u

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
OS=$(cd "$HERE/../.." && pwd)
BOOT=$OS/1-bootstrap
MSG=$OS/5-messaging
TAPES=${G1_TAPES_DIR:-$HERE/tapes}
export PATH="$HOME/.cargo/bin:$PATH:/usr/sbin:/sbin"
# shellcheck source=6-transport/verify/lib.sh
. "$HERE/lib.sh"
# shellcheck source=6-transport/verify/root.sh
. "$HERE/root.sh"

OKS=0
ok() {
  printf 'ok: %s\n' "$*"
  OKS=$((OKS + 1))
}
fail() {
  printf 'FAIL: %s\n' "$*"
  FAILS=$((FAILS + 1))
}
T0=${G1_T0:-${EPOCHREALTIME/,/.}}
elapsed() { awk -v a="$T0" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}'; }
phase() { printf '\n== %s  (t+%ss)\n' "$*" "$(elapsed)"; }

# ------------------------------------------------------------------ helpers (inner)

NODES=(A B)
declare -A HOLD=() DPID=() NID=() NROOT=() CTLD=() NIP=([A]=10.111.6.11 [B]=10.111.6.12)
DAPI=http://127.0.0.1:11180
NETM=stub-mesh
NETL=stub-lxmf
NETA=stub-app
# accounts of this node on each stub network (the scenario files carry the same)
ACC_MA='!a0000001'
ACC_MA2='!a0000002'
ACC_MB='!b0000001'
ACC_AA=app-a
ACC_AB=app-b
ACC_LA=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
TOKEN=""

nx() {
  local n=$1
  shift
  nsenter -t "${HOLD[$n]}" -n -- "$@"
}
# req METHOD URL [curl args…] → $ST (000 = no answer), $W/hdr, $W/body (NSX: netns prefix)
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
# bearer NODE METHOD PATH [JSON] — the agent's bearer (the daemon's real per-spawn token)
bearer() {
  if [ $# -ge 4 ]; then
    api "$1" "$2" "$3" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d "$4"
  else
    api "$1" "$2" "$3" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json'
  fi
}
has_json() { [ -s "$1" ] && grep -q '[^[:space:]]' "$1" 2>/dev/null; }
jb() { has_json "$W/body" && jq -e "$@" "$W/body" >/dev/null 2>&1; }
jget() { jq -r "$@" "$W/body" 2>/dev/null || true; }
jqs() {
  local in
  in=$(cat)
  [ -n "${in//[[:space:]]/}" ] && printf '%s' "$in" | jq -e "$@" >/dev/null 2>&1
}
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
settle() { sleep "${1:-7}"; } # ≥ 3 daemon ticks (2 s): nothing more may happen after this

# -- the scenario hook's control directory (N/bridge/stub.rs `scenario`)
CSEQ=0
# ctl NODE JSON — run one command in NODE's hook; $CTL = its answer; 0 when ok
ctl() {
  local d=${CTLD[$1]} f
  CSEQ=$((CSEQ + 1))
  f=$(printf '%s/%08d' "$d" "$CSEQ")
  printf '%s' "$2" >"$f.tmp" && mv "$f.tmp" "$f.json"
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
# tx NET ACCOUNT KIND NATIVE SENDER NAME ID BODY [META] → one Inbound::Text
tx() {
  jq -nc --arg net "$1" --arg acc "$2" --arg kind "$3" --arg nat "$4" --arg s "$5" --arg nm "$6" \
    --arg id "$7" --arg b "$8" --argjson meta "${9:-"{}"}" \
    '{Text: {conv: {network: $net, account: $acc, kind: $kind, native_id: $nat}, sender_native: $s,
      sender_name: $nm, auth: "channel-key", native_msg_id: $id, body: $b, claim_ts: null, meta: $meta}}'
}
# cv NET ACCOUNT KIND NATIVE TITLE → one Inbound::Conversation
cv() {
  jq -nc --arg net "$1" --arg acc "$2" --arg kind "$3" --arg nat "$4" --arg t "$5" \
    '{Conversation: {conv: {network: $net, account: $acc, kind: $kind, native_id: $nat}, title: $t}}'
}
# feed NODE STUB ITEM… — one feed command (one wake-up)
feed() {
  local n=$1 s=$2
  shift 2
  ctl "$n" "$(printf '%s\n' "$@" | jq -sc --arg s "$s" '{op: "feed", stub: $s, items: .}')"
}
script_out() { ctl "$1" "$(jq -nc --arg s "$2" --argjson o "$3" --argjson r "${4:-false}" '{op: "script", stub: $s, outcomes: $o, replace: $r}')"; }
facts() { ctl "$1" "$(jq -nc --arg s "$2" --argjson f "$3" '{op: "facts", stub: $s, facts: $f}')"; }
# mesh_facts NODE [licensed] [region_set] [uplink of ch:up]
mesh_facts() {
  local acc=$ACC_MA second=',{"id":"!a0000002","label":"second board","licensed":false}'
  if [ "$1" = B ]; then acc=$ACC_MB second=""; fi
  printf '{"accounts":[{"id":"%s","label":"board","licensed":%s}%s],"channels":[{"account":"%s","native_id":"ch:up","uplink":%s,"default_key":false}],"region_set":%s,"ids_globally_unique":false,"parts_may_reorder":true,"licensed_caps":{"channel":232,"dm":232,"group":null}}' \
    "$acc" "${2:-false}" "$second" "$acc" "${4:-false}" "${3:-true}"
}
# app_facts NODE [licensed] [region_set]
app_facts() {
  local acc=$ACC_AA
  [ "$1" = B ] && acc=$ACC_AB
  printf '{"accounts":[{"id":"%s","label":"bridge account","licensed":%s}],"channels":[],"region_set":%s,"ids_globally_unique":true,"parts_may_reorder":false,"licensed_caps":null}' \
    "$acc" "${2:-false}" "${3:-true}"
}
lxmf_facts() {
  printf '{"accounts":[{"id":"%s","label":null,"licensed":false}],"channels":[],"region_set":null,"ids_globally_unique":true,"parts_may_reorder":false,"licensed_caps":null}' "$ACC_LA"
}
# fc NODE NET NATIVE → the conversation's fc- id (waits up to 10 s; nothing when absent)
fc() {
  local i id
  for i in $(seq 1 50); do
    api "$1" GET "/api/v1/networks/$2/conversations"
    id=$(jget --arg n "$3" '[.conversations[]? | select(.native_id == $n)] | first | .id // empty')
    if [ -n "$id" ]; then
      printf '%s' "$id"
      return 0
    fi
    sleep 0.2
  done
}
# rows NODE NET FC → every message of the conversation as one JSON array (paged); nothing on error
RSEQ=0
rows() {
  local since=0 more pages
  RSEQ=$((RSEQ + 1))
  pages=$W/rows.$RSEQ
  : >"$pages"
  while :; do
    api "$1" GET "/api/v1/networks/$2/conversations/$3?limit=500&since_seq=$since"
    if ! { [ "$ST" = 200 ] && jb '.messages | type == "array"'; }; then
      rm -f "$pages"
      return 0
    fi
    jq -c .messages "$W/body" >>"$pages"
    more=$(jget .more)
    [ "$more" = true ] || break
    since=$(jget '.messages | last | .seq')
  done
  jq -sc 'add // []' "$pages"
  rm -f "$pages"
}
# nmsg NODE NET FC JQ-SELECT → how many messages satisfy it (nothing when the read failed)
nmsg() {
  local r
  r=$(rows "$1" "$2" "$3")
  [ -n "$r" ] || return 0
  printf '%s' "$r" | jq --arg x "${5:-}" "[.[] | select($4)] | length"
}
# msg NODE NET FC JQ-SELECT → the first matching message (compact JSON) or nothing
msg() {
  local r
  r=$(rows "$1" "$2" "$3")
  [ -n "$r" ] || return 0
  printf '%s' "$r" | jq -c "[.[] | select($4)] | first // empty"
}
# sentc NODE STUB SUBSTRING → sends of STUB whose text contains SUBSTRING (the stub's log)
sentc() {
  local f=${CTLD[$1]}/$2.sent.jsonl
  if [ -s "$f" ]; then jq -s --arg t "$3" '[.[] | select(.text | contains($t))] | length' "$f"; else echo 0; fi
}
# sentp NODE STUB PREFIX → sends of STUB whose text starts with PREFIX
sentp() {
  local f=${CTLD[$1]}/$2.sent.jsonl
  if [ -s "$f" ]; then jq -s --arg t "$3" '[.[] | select(.text | startswith($t))] | length' "$f"; else echo 0; fi
}
nlines() { # nlines NODE STUB → sends of STUB so far
  local f=${CTLD[$1]}/$2.sent.jsonl
  if [ -s "$f" ]; then wc -l <"$f"; else echo 0; fi
}
sent_from() { # sent_from NODE STUB N → the texts of the sends after the first N
  local f=${CTLD[$1]}/$2.sent.jsonl
  [ -s "$f" ] && tail -n +$(($3 + 1)) "$f" | jq -r .text
}
sent_texts() { # sent_texts NODE STUB SUBSTRING → the matching sent texts, in send order
  local f=${CTLD[$1]}/$2.sent.jsonl
  [ -s "$f" ] && jq -r --arg t "$3" 'select(.text | contains($t)) | .text' "$f"
}
# edge NODE BE → GET /bridge/edges/BE into $W/body
edge() { api "$1" GET "/api/v1/bridge/edges/$2"; }
# mkedge NODE SRC DST [OPTS] — the 409 round-trip then the create; $EDGE = be- id
mkedge() {
  local body h
  body=$(jq -nc --arg s "$2" --arg d "$3" --argjson o "${4:-"{}"}" '{src: $s, dst: $d} + $o')
  jsend "$1" POST /api/v1/bridge/edges "$body"
  h=$(jget .consent.hash)
  EDGE=""
  [ "$ST" = 409 ] && [ -n "$h" ] || return 1
  jsend "$1" POST /api/v1/bridge/edges "$(jq -c --arg h "$h" '. + {accept_warning: $h}' <<<"$body")"
  EDGE=$(jget .id)
  [ "$ST" = 201 ] && [ -n "$EDGE" ]
}
# fsend NODE NET FC TEXT [SIZE_MODE] — POST …/messages → $FM (first row), $FMS (all ids)
fsend() {
  local b
  b=$(jq -nc --arg t "$4" --arg m "${5:-refuse}" '{text: $t, size_mode: $m}')
  jsend "$1" POST "/api/v1/networks/$2/conversations/$3/messages" "$b"
  FM=$(jget '.id // empty')
  FMS=$(jget '(.ids // [.id]) | join(" ")')
}
sha256() { printf '%s' "$1" | sha256sum | cut -d' ' -f1; }
# sse_data FILE EVENT → the data of every frame named EVENT (one per line)
sse_data() {
  python3 -I -c 'import sys
ev = sys.argv[2]
text = open(sys.argv[1], encoding="utf-8", errors="replace").read().replace("\r\n", "\n")
for block in text.split("\n\n"):
    e = d = None
    for line in block.split("\n"):
        if line.startswith("event:"):
            e = line[6:].strip()
        elif line.startswith("data:"):
            v = line[5:]
            v = v[1:] if v.startswith(" ") else v
            d = v if d is None else d + "\n" + v
    if e == ev and d is not None:
        print(d)' "$1" "$2" 2>/dev/null || true
}
# sse_frames FILE EVENT → the raw frames named EVENT (blank-line separated), for a tape
sse_frames() {
  python3 -I -c 'import sys
ev = sys.argv[2]
text = open(sys.argv[1], encoding="utf-8", errors="replace").read().replace("\r\n", "\n")
out = [b for b in text.split("\n\n") if any(l.startswith("event:") and l[6:].strip() == ev for l in b.split("\n"))]
sys.stdout.write("".join(b + "\n\n" for b in out))' "$1" "$2" 2>/dev/null || true
}
nready() { { wc -l <"${CTLD[$1]}/ready.jsonl"; } 2>/dev/null || echo 0; } # the hook's ready lines so far
# start_node NODE — `111 daemon` in NODE's netns with its scenario; waits for /health, the
# fabric and the hook's ready line
start_node() {
  local n=$1 d0=${EPOCHREALTIME/,/.} up=0 code r0
  r0=$(nready "$n")
  NODE111_ROOT=${NROOT[$n]} NODE111_BRIDGE_STUBS=${NROOT[$n]}/g1-scenario.json \
    nsenter -t "${HOLD[$n]}" -n -- "$BIN111" daemon >>"${NROOT[$n]}/daemon.log" 2>&1 &
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
  wait_for 20 '[ "$(nready "$_sn")" -gt "$r0" ]' || true
  t "$n: 111 daemon up (pid ${DPID[$n]}, $(awk -v a="$d0" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}')s): fabric open, the G1 hook enabled $(tail -1 "${CTLD[$n]}/ready.jsonl" 2>/dev/null | jq -c .stubs 2>/dev/null)" \
    '[ "$(nready "$_sn")" -gt "$r0" ] && api "$_sn" GET /api/v1/fabric && jb ".store.open == true"'
}
stop_node() { # stop_node NODE [SIGNAL] — and wait for it (and its listener) to go
  local n=$1 p=${DPID[$1]:-} sig=${2:-TERM}
  [ -n "$p" ] || return 0
  kill "-$sig" "$p" 2>/dev/null || true
  for _ in $(seq 1 100); do
    kill -0 "$p" 2>/dev/null || break
    sleep 0.2
  done
  wait "$p" 2>/dev/null || true
  DPID[$n]=""
  local _n=$n
  wait_for 10 '! nx "$_n" curl -s -o /dev/null --max-time 1 "$DAPI/health"' || true
}

# ------------------------------------------------------------------ inner (in the namespaces)

inner() {
  : "${G1_WORK:?}" "${BIN111:?}"
  W=$G1_WORK/http
  mkdir -p "$W"
  local n
  for n in "${NODES[@]}"; do
    NROOT[$n]=$G1_WORK/$n
    CTLD[$n]=$G1_WORK/$n/g1-ctl
    NID[$n]=$(cat "$G1_WORK/$n.nid" 2>/dev/null || echo none)
  done
  SSEP=""
  cleanup_inner() {
    [ -n "$SSEP" ] && kill "$SSEP" 2>/dev/null
    for n in "${NODES[@]}"; do
      if [ -n "${DPID[$n]:-}" ]; then kill -TERM "${DPID[$n]}" 2>/dev/null || true; fi
    done
    for _ in $(seq 1 30); do
      local alive=0
      for n in "${NODES[@]}"; do [ -n "${DPID[$n]:-}" ] && kill -0 "${DPID[$n]}" 2>/dev/null && alive=1; done
      [ "$alive" = 0 ] && break
      sleep 0.3
    done
    # leaving pid 1 of the namespace kills whatever is left (holders, stand-in agents, curl)
    printf 'FAILS=%s\n' "$FAILS" >"$G1_WORK/inner.result"
  }
  trap cleanup_inner EXIT
  trap 'exit 130' INT TERM

  # ---------------------------------------------------------------- network
  phase "1. namespaces: br0 10.111.6.1/24, A and B on veth pairs"
  ip link set lo up
  if ip link add br0 type bridge && ip addr add 10.111.6.1/24 dev br0 && ip link set br0 up; then
    ok "bridge br0 10.111.6.1/24 up (no route off-box)"
  else
    fail "cannot create br0"
  fi
  local myns
  myns=$(readlink /proc/self/ns/net)
  for n in "${NODES[@]}"; do
    unshare --net sleep 100000 &
    HOLD[$n]=$!
  done
  for n in "${NODES[@]}"; do
    wait_for 5 '[ "$(readlink /proc/${HOLD[$n]}/ns/net 2>/dev/null)" != "$myns" ]' || true
    if ip link add "v$n" type veth peer name eth0 netns "${HOLD[$n]}" && ip link set "v$n" master br0 up &&
      nx "$n" ip link set lo up && nx "$n" ip addr add "${NIP[$n]}/24" dev eth0 && nx "$n" ip link set eth0 up; then
      ok "$n: netns (holder ${HOLD[$n]}) eth0 ${NIP[$n]}/24 on br0"
    else
      fail "$n: netns/veth setup failed"
    fi
  done

  # ---------------------------------------------------------------- daemons
  phase "2. two daemons with the G1 scenario hook"
  start_node B
  start_node A
  t "A reaches B's fabric listener over the bridge (TCP 10.111.6.12:11183 from A's netns)" \
    'wait_for 10 "nx A timeout 3 bash -c \"exec 3<>/dev/tcp/10.111.6.12/11183\" 2>/dev/null"'
  for n in "${NODES[@]}"; do
    api "$n" GET /api/v1/networks
    t "$n: GET /networks lists the scenario's stubs live ($(jget -c '[.networks[]? | select(.id | startswith("stub-")) | "\(.id)=\(.state)"]'))" \
      '[ "$ST" = 200 ] && jb "[.networks[] | select(.id | startswith(\"stub-\")) | select(.state == \"live\")] | length == $( [ $n = A ] && echo 3 || echo 2 )"'
  done
  local tok=${NROOT[A]}/agent/g6-agent.token
  wait_for 20 '[ -s "$tok" ]' || true
  TOKEN=$(cat "$tok" 2>/dev/null || true)
  t "A: the agent's bearer is the daemon's per-spawn token (64 hex, read from the stand-in agent's environment)" '[[ "$TOKEN" =~ ^[0-9a-f]{64}$ ]]'

  # SSE of A for the tapes (network_message, bridge) — the whole run
  nx A curl -sN --max-time 1800 -o "$W/A-events.sse" "$DAPI/api/v1/events" 2>/dev/null &
  SSEP=$!

  # ---------------------------------------------------------------- 111 contacts
  phase "3. 111 contacts A ↔ B (pasted cards) and a baseline 111 message"
  local card_a card_b mid t1 lat0
  api A GET /api/v1/contacts/me/card
  card_a=$(jget .text)
  api B GET /api/v1/contacts/me/card
  card_b=$(jget .text)
  jsend A POST /api/v1/contacts "$(jq -nc --arg c "$card_b" '{card: $c}')"
  t "A: POST /contacts {card: B} → B has_key" '[ "$ST" = 200 ] && jb ".has_key == true"'
  jsend B POST /api/v1/contacts "$(jq -nc --arg c "$card_a" '{card: $c}')"
  t "B: POST /contacts {card: A} → A has_key" '[ "$ST" = 200 ] && jb ".has_key == true"'
  t1=${EPOCHREALTIME/,/.}
  jsend A POST /api/v1/messages "$(jq -nc --arg to "${NID[B]}" '{to: $to, text: "g1 baseline 111 message"}')"
  mid=$(jget .id)
  wait_for 40 'api A GET "/api/v1/messages/threads/${NID[B]}?limit=50"; jb --arg id "$mid" "[.messages[]? | select(.id == \$id and .state == \"delivered\")] | length == 1"' || true
  lat0=$(awk -v a="$t1" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.2f", b-a}')
  t "A → B 111 message delivered over tcp-lan (baseline ${lat0}s)" 'jb --arg id "$mid" "[.messages[]? | select(.id == \$id and .state == \"delivered\")] | length == 1"'

  # ---------------------------------------------------------------- conversations
  phase "4. foreign conversations (stub feeds) and the one ingest function"
  local c
  feed A $NETM \
    "$(cv $NETM $ACC_MA channel ch:solo 'solo channel')" "$(cv $NETM $ACC_MA channel ch:fan 'fan-out channel')" \
    "$(cv $NETM $ACC_MA channel ch:hist 'history channel')" "$(cv $NETM $ACC_MA channel ch:up 'uplinked channel')" \
    "$(cv $NETM $ACC_MA channel ch:order 'order channel')" "$(cv $NETM $ACC_MA channel ch:prop 'proposal channel')" \
    "$(cv $NETM $ACC_MA channel ch:pin 'pinned channel')" "$(cv $NETM $ACC_MA channel ch:burst 'burst channel')" \
    "$(cv $NETM $ACC_MA channel ch:loop 'loop channel')" "$(cv $NETM $ACC_MA dm 'dm:!c0ffee01' 'a DM')"
  t "A: feed conversation upserts into stub-mesh" '[ -n "$CTL" ] && printf "%s" "$CTL" | jqs ".ok == true"'
  feed A $NETA \
    "$(cv $NETA $ACC_AA group grp:solo 'solo group')" "$(cv $NETA $ACC_AA group grp:z 'chain end')" \
    "$(cv $NETA $ACC_AA group grp:hist 'history group')" "$(cv $NETA $ACC_AA group grp:up 'up source')" \
    "$(cv $NETA $ACC_AA group grp:order 'order group')" "$(cv $NETA $ACC_AA group grp:prop 'proposal group')" \
    "$(cv $NETA $ACC_AA group grp:prune 'prune group')" "$(cv $NETA $ACC_AA group grp:crash 'crash group')" \
    "$(cv $NETA $ACC_AA group grp:loop 'loop group')"
  feed B $NETM "$(cv $NETM $ACC_MB channel ch:loop 'loop channel')"
  feed B $NETA "$(cv $NETA $ACC_AB group grp:loop 'loop group')"
  declare -A FC=()
  for c in ch:solo ch:fan ch:hist ch:up ch:order ch:prop ch:pin ch:burst ch:loop 'dm:!c0ffee01'; do FC[A:$NETM:$c]=$(fc A $NETM "$c"); done
  for c in grp:solo grp:z grp:hist grp:up grp:order grp:prop grp:prune grp:crash grp:loop; do FC[A:$NETA:$c]=$(fc A $NETA "$c"); done
  FC[B:$NETM:ch:loop]=$(fc B $NETM ch:loop)
  FC[B:$NETA:grp:loop]=$(fc B $NETA grp:loop)
  local missing=0 k
  for k in "${!FC[@]}"; do [[ "${FC[$k]}" =~ ^fc-[0-9]+$ ]] || missing=$((missing + 1)); done
  t "every scenario conversation has an fc- id (${#FC[@]} conversations, $missing missing)" '[ "$missing" = 0 ] && [ "${#FC[@]}" = 21 ]'
  local SOLO=${FC[A:$NETM:ch:solo]} GSOLO=${FC[A:$NETA:grp:solo]} GZ=${FC[A:$NETA:grp:z]}

  # the one ingest function: Cc + Cf stripped (U+202E, zero-width), NFC, \n and \t kept
  local evil
  evil=$(jq -nc --arg net $NETM --arg acc "$ACC_MA" '{Text: {conv: {network: $net, account: $acc, kind: "channel", native_id: "ch:solo"},
    sender_native: "!e0000001", sender_name: "Mallory‮​txt.exe", auth: "channel-key", native_msg_id: "evil-1",
    body: "ingest‮ check​ zero‍width﻿ bom⁦iso⁩ é tab\there\nline two\u0007",
    claim_ts: null, meta: {}}}')
  feed A $NETM "$evil" "$(tx $NETM $ACC_MA channel ch:solo '!e0000002' 'Plain Pete' 'plain-1' 'plain text control')"
  local ev=""
  wait_for 10 'ev=$(msg A $NETM "$SOLO" ".sender.native == \"!e0000001\""); [ -n "$ev" ]' || true
  t "ingest strips U+202E, U+200B, U+200D, U+FEFF, U+2066/2069 and BEL from the body, NFC-normalises (e+U+0301 → é), keeps \\t and \\n: $(printf '%s' "$ev" | jq -c .text)" \
    '[ -n "$ev" ] && printf "%s" "$ev" | jqs ".text == \"ingest check zerowidth bomiso é tab\\there\\nline two\""'
  t "ingest strips them from the sender name too ($(printf '%s' "$ev" | jq -c .sender.name))" \
    '[ -n "$ev" ] && printf "%s" "$ev" | jqs ".sender.name == \"Mallorytxt.exe\""'
  t "the plain text beside it is stored unchanged (positive control)" \
    '[ "$(nmsg A $NETM "$SOLO" ".text == \"plain text control\"")" = 1 ]'

  # foreign rows never reach the 111 threads
  api A GET /api/v1/messages/threads
  t "GET /messages/threads lists only 111 threads (the contact B), no foreign row or conversation ($(jget -c '[.threads[]?.peer]'))" \
    '[ "$ST" = 200 ] && jb "(.threads | length) == 1 and .threads[0].peer == \"${NID[B]}\"" && ! grep -q "fm-\|fc-\|ingest\|plain text control" "$W/body"'
  api A GET "/api/v1/messages/threads/${NID[B]}?limit=500"
  t "the 111 thread with B holds only the 111 message (no foreign text)" \
    '[ "$ST" = 200 ] && jb "[.messages[] | select(.text | test(\"ingest|plain text control\"))] | length == 0" && jb "(.messages | length) == 1"'
  api A GET /api/v1/messages/threads/fc-1
  t "a foreign conversation id is not a 111 peer (GET /messages/threads/fc-1 → 4xx)" '[[ "$ST" =~ ^4 ]]'

  # ---------------------------------------------------------------- 401 matrix
  phase "5. the agent's bearer: 401 on every Layer 6 write, reads pass"
  local FAN=${FC[A:$NETM:ch:fan]}
  local -a WRITES=(
    "POST /api/v1/networks/$NETM/conversations {\"kind\":\"dm\",\"native_id\":\"dm:!c0ffee02\"}"
    "POST /api/v1/networks/$NETM/conversations/$FAN/messages {\"text\":\"bearer write\"}"
    "POST /api/v1/messages/fanout/preview {\"text\":\"x\",\"targets\":[{\"network\":\"$NETM\",\"conv\":\"$FAN\"}]}"
    "POST /api/v1/messages/fanout {\"text\":\"bearer fanout\",\"targets\":[{\"network\":\"$NETM\",\"conv\":\"$FAN\"}]}"
    "POST /api/v1/bridge/edges {\"src\":\"$SOLO\",\"dst\":\"$GSOLO\"}"
    "PATCH /api/v1/bridge/edges/be-1 {\"paused\":true}"
    "DELETE /api/v1/bridge/edges/be-1 -"
    "PATCH /api/v1/bridge/settings {\"announce\":{\"interval_min\":0}}"
    "POST /api/v1/bridge/components/lxmf/enable {}"
    "POST /api/v1/bridge/components/lxmf/disable {}"
    "POST /api/v1/bridge/components/lxmf/remove-files {}"
    "PATCH /api/v1/transport/posture {\"order\":[\"111\"]}"
    "POST /api/v1/transport/join {}"
    "POST /api/v1/messages/fm-1/retry {}"
    "POST /api/v1/messages/fm-1/cancel {}"
    "POST /api/v1/proposals/prop-g1-none/decision {\"decision\":\"accepted\"}"
    "POST /api/v1/devices/0000000000000000/identify {}"
    "POST /api/v1/services/transport/restart {}"
  )
  local w m p b bad=0 badlist=""
  for w in "${WRITES[@]}"; do
    m=${w%% *}
    p=${w#* }
    b=${p#* }
    p=${p%% *}
    if [ "$b" = - ]; then bearer A "$m" "$p"; else bearer A "$m" "$p" "$b"; fi
    if ! problem_is 401 unauthenticated; then
      bad=$((bad + 1))
      badlist="$badlist $m $p→$ST"
    fi
  done
  t "the agent's bearer answers 401 unauthenticated on all ${#WRITES[@]} Layer 6 writes (incl. POST /devices/{id}/identify and POST /services/{id}/restart)${badlist:+ — not:$badlist}" '[ "$bad" = 0 ]'
  # the contract's non-GET operations of the Layer 6 paths are all in the list above
  local ops
  ops=$(python3 -I - "$OS/4-interface/api/openapi.yaml" <<'PY'
import re, sys
paths = {}
cur = None
for line in open(sys.argv[1], encoding="utf-8"):
    m = re.match(r"^  (/api/v1/\S+):\s*$", line)
    if m:
        cur = m.group(1)
        continue
    if re.match(r"^\S", line):
        cur = None
    m = re.match(r"^    (get|post|put|patch|delete):", line)
    if cur and m and m.group(1) != "get":
        paths.setdefault(cur, []).append(m.group(1))
l6 = ("/api/v1/networks", "/api/v1/messages/fanout", "/api/v1/bridge", "/api/v1/transport")
print(sum(len(v) for k, v in paths.items() if k.startswith(l6)))
PY
  )
  t "the list covers every non-GET operation the contract declares under /networks, /messages/fanout, /bridge and /transport ($ops of them, plus retry/cancel of fm- ids, decisions, identify and restart)" \
    '[ -n "$ops" ] && [ "$ops" -eq 13 ]'
  bearer A GET /api/v1/networks
  t "the agent's bearer still reads GET /networks (200, $(jget '.networks | length') networks)" '[ "$ST" = 200 ] && jb ".networks | length >= 3"'
  bearer A GET /api/v1/bridge/edges
  t "… and GET /bridge/edges (200)" '[ "$ST" = 200 ] && jb ".edges | type == \"array\""'
  api A GET "/api/v1/networks/$NETM/conversations/$FAN"
  t "nothing the refused writes asked for exists (ch:fan holds no row, no edge, no DM dm:!c0ffee02)" \
    '[ "$ST" = 200 ] && jb ".messages | length == 0" && api A GET /api/v1/bridge/edges && jb ".edges | length == 0" && [ -z "$(fc A $NETM "dm:!c0ffee02" 2>/dev/null | head -c1)" ]'
  local h0
  jsend A POST /api/v1/bridge/edges "$(jq -nc --arg s "$SOLO" --arg d "$GSOLO" '{src: $s, dst: $d}')"
  h0=$(jget .consent.hash)
  bearer A POST /api/v1/bridge/edges "$(jq -nc --arg s "$SOLO" --arg d "$GSOLO" --arg h "$h0" '{src: $s, dst: $d, accept_warning: $h}')"
  t "the bearer with the CURRENT warning hash is still 401 and creates nothing (the hash is no credential)" \
    '[ -n "$h0" ] && problem_is 401 unauthenticated && api A GET /api/v1/bridge/edges && jb ".edges | length == 0"'

  # ---------------------------------------------------------------- fan-out
  phase "6. fan-out: preview, refuse, split + cancel, adapter state, consent gates, a 111 target"
  local LIST_ID=g1list FANP
  jsend A PATCH /api/v1/bridge/settings "$(jq -nc --arg n $NETL --arg a 0123456789abcdef0123456789abcdef --arg b fedcba9876543210fedcba9876543210 \
    '{recipient_lists: [{id: "g1list", network: $n, name: "G1 list", members: [$a, $b]}]}')"
  t "PATCH /bridge/settings recipient list on stub-lxmf (32-hex members) → 200" '[ "$ST" = 200 ] && jb ".recipient_lists | length == 1"'
  local LISTC
  LISTC=$(fc A $NETL list:g1list)
  t "the recipient list is a group conversation list:g1list ($LISTC)" '[[ "$LISTC" =~ ^fc- ]]'
  local big long
  big=$(printf 'x%.0s' $(seq 1 260))
  long=$(python3 -I -c 'print(" ".join("w%03d" % i for i in range(1, 121)))')
  local before_fan lx0
  before_fan=$(nmsg A $NETM "$FAN" 'true')
  api A GET /api/v1/networks/$NETL/conversations
  lx0=$(jget '.conversations | length')
  jsend A POST /api/v1/messages/fanout/preview "$(jq -nc --arg t "$big" --arg c "$FAN" --arg l "$LISTC" --arg nm $NETM --arg nl $NETL \
    '{text: $t, targets: [{network: $nm, conv: $c}, {network: $nl, conv: $l}]}')"
  cp "$W/body" "$W/preview.json"
  t "preview: 260 B into stub-mesh's channel is refused '$(jq -r '.items[0].plan.refusal.detail' "$W/preview.json" 2>/dev/null)' (cap 232), ok:false" \
    '[ "$ST" = 200 ] && jb ".ok == false and .items[0].plan.refusal.code == \"oversize\" and .items[0].plan.cap == 232 and (.items[0].plan.refusal.detail | test(\"260 B > 232 B\"))"'
  t "preview: the airtime refusal facts — an estimate from the LongFast profile '$(jq -r '.items[0].plan.airtime_estimate.label' "$W/preview.json" 2>/dev/null)', the region's duty cycle warning" \
    'jb ".items[0].plan.airtime_estimate.millis > 0 and (.items[0].plan.airtime_estimate.label | test(\"estimate\")) and ([.items[0].plan.warnings[].code] | index(\"duty_cycle\") != null)"'
  t "preview: the recipient list target fits (287 B per DM) with no refusal" 'jb ".items[1].plan.refusal == null and .items[1].plan.cap == 287"'
  t "preview writes nothing (ch:fan still holds $before_fan rows)" '[ "$(nmsg A $NETM "$FAN" "true")" = "$before_fan" ]'
  # refuse: 409 with the per-target report, nothing queued (both targets)
  jsend A POST /api/v1/messages/fanout "$(jq -nc --arg t "$big" --arg c "$FAN" --arg l "$LISTC" --arg nm $NETM --arg nl $NETL \
    '{text: $t, targets: [{network: $nm, conv: $c}, {network: $nl, conv: $l}]}')"
  t "fan-out of an oversize text in refuse mode → 409 oversize with the per-target report" \
    '[ "$ST" = 409 ] && jb "(.targets | length) >= 1 and .targets[0].code == \"oversize\""'
  sleep 2.5
  t "… and queues nothing: no row on ch:fan, no DM to the list members, no send reached either stub" \
    '[ "$(nmsg A $NETM "$FAN" "true")" = 0 ] && api A GET /api/v1/networks/$NETL/conversations && [ -n "$lx0" ] && [ "$(jget ".conversations | length")" = "$lx0" ] && [ "$(sentc A $NETM xxxxxxxx)" = 0 ] && [ "$(sentc A $NETL xxxxxxxx)" = 0 ]'
  jsend A POST /api/v1/messages/fanout "$(jq -nc --arg c "$FAN" --arg l "$LISTC" --arg nm $NETM --arg nl $NETL --arg b "${NID[B]}" \
    '{text: "g1 sized fan-out", targets: [{network: $nm, conv: $c}, {network: $nl, conv: $l}, {node_id: $b}]}')"
  cp "$W/body" "$W/fanout-ok.json"
  t "positive control: a sized text → 202 one send group (sg-), an fm- row per foreign target (the list: one per member) and a msg- row for the 111 contact" \
    '[ "$ST" = 202 ] && jb "(.group_id | startswith(\"sg-\")) and (.items | length) == 4 and ([.items[0:3][].id | startswith(\"fm-\")] | all) and (.items[3].id | startswith(\"msg-\"))"'
  local m111
  m111=$(jq -r '.items[3].id' "$W/fanout-ok.json" 2>/dev/null)
  wait_for 20 '[ "$(sentc A $NETM "g1 sized fan-out")" = 1 ] && [ "$(sentc A $NETL "g1 sized fan-out")" = 2 ]' || true
  t "the copies reached the carriers: one on stub-mesh, one DM per list member on stub-lxmf" \
    '[ "$(sentc A $NETM "g1 sized fan-out")" = 1 ] && [ "$(sentc A $NETL "g1 sized fan-out")" = 2 ]'
  wait_for 30 'api B GET "/api/v1/messages/threads/${NID[A]}?limit=50"; jb "[.messages[]? | select(.text == \"g1 sized fan-out\")] | length == 1"' || true
  t "the 111 target is an ordinary sealed message: B holds it once ($m111)" 'jb "[.messages[]? | select(.text == \"g1 sized fan-out\")] | length == 1"'
  # split: per-part rows; cancel drops unsent parts only
  script_out A $NETM '[{"accept":{"evidence":[["sent","accepted by the radio"]]}}, {"fail":{"reason":"rate_limited","retry":"later"}}, {"fail":{"reason":"rate_limited","retry":"later"}}, {"fail":{"reason":"rate_limited","retry":"later"}}, {"fail":{"reason":"rate_limited","retry":"later"}}, {"fail":{"reason":"rate_limited","retry":"later"}}, {"fail":{"reason":"rate_limited","retry":"later"}}, {"fail":{"reason":"rate_limited","retry":"later"}}, {"fail":{"reason":"rate_limited","retry":"later"}}]' true
  jsend A POST /api/v1/messages/fanout "$(jq -nc --arg t "$long" --arg c "$FAN" --arg nm $NETM '{text: $t, targets: [{network: $nm, conv: $c, size_mode: "split"}]}')"
  cp "$W/body" "$W/split.json"
  local parts nparts
  parts=$(jq -r '.items[0].ids // [] | join(" ")' "$W/split.json" 2>/dev/null)
  nparts=$(jq -r '.items[0].plan.parts // 0' "$W/split.json" 2>/dev/null)
  t "split: a ${#long}-byte text → 202 with $nparts parts, one fm- row per part (ids: $parts)" \
    '[ "$ST" = 202 ] && [ "$nparts" -ge 2 ] && [ "$(wc -w <<<"$parts")" = "$nparts" ]'
  local prows=""
  wait_for 15 'prows=$(rows A $NETM "$FAN"); printf "%s" "$prows" | jqs "[.[] | select(.part != null and .part.n == $nparts)] | (length == $nparts) and (map(select(.state == \"sent\")) | length) >= 1"' || true
  t "every part is its own row ((k/n) = $(printf '%s' "$prows" | jq -c '[.[] | select(.part != null) | "\(.part.k)/\(.part.n):\(.state)"]' 2>/dev/null)), each within the cap with the (k/n) prefix" \
    'printf "%s" "$prows" | jqs "[.[] | select(.part != null and .part.n == $nparts)] | length == $nparts and all(.text | (startswith(\"(\") and (utf8bytelength <= 232)))"'
  local p1 p2 cancelled=0 kept=0
  p1=$(printf '%s' "$prows" | jq -r '[.[] | select(.part != null and .part.n == '"$nparts"' and .state == "sent")] | first | .id // empty')
  for p2 in $(printf '%s' "$prows" | jq -r '.[] | select(.part != null and .part.n == '"$nparts"' and .state == "queued") | .id'); do
    for _ in 1 2 3 4 5 6 7 8; do
      jsend A POST "/api/v1/messages/$p2/cancel" '{}'
      [ "$ST" = 202 ] && break
      sleep 0.3
    done
    [ "$ST" = 202 ] && cancelled=$((cancelled + 1))
  done
  jsend A POST "/api/v1/messages/$p1/cancel" '{}'
  local p1st=$ST
  wait_for 6 'prows=$(rows A $NETM "$FAN"); printf "%s" "$prows" | jqs "[.[] | select(.part != null and .part.n == $nparts)] | length == 1"' || true
  t "cancel drops the unsent parts only: $cancelled queued part(s) cancelled (202), the sent part refused ($p1st) and kept ($(printf '%s' "$prows" | jq -c '[.[] | select(.part != null) | "\(.part.k)/\(.part.n):\(.state)"]' 2>/dev/null))" \
    '[ "$cancelled" = $((nparts - 1)) ] && [[ "$p1st" =~ ^4 ]] && printf "%s" "$prows" | jqs "[.[] | select(.part != null and .part.n == $nparts)] | length == 1 and .[0].state == \"sent\""'
  script_out A $NETM '[]' true
  # adapter-state failure: region unset → the row queues with the reason (202, not a call error)
  facts A $NETM "$(mesh_facts A false false)"
  jsend A POST /api/v1/messages/fanout "$(jq -nc --arg c "$FAN" --arg nm $NETM '{text: "g1 region unset", targets: [{network: $nm, conv: $c}]}')"
  local rfm
  rfm=$(jget '.items[0].id // empty')
  t "region unset on the stub: fan-out → 202 (adapter state is not a call error), preview warning region_unset" \
    '[ "$ST" = 202 ] && jb "[.items[0].plan.warnings[].code] | index(\"region_unset\") != null"'
  sleep 4.5
  t "the row stays queued with reason_code region_unset and nothing was handed to the stub" \
    '[ "$(nmsg A $NETM "$FAN" ".id == \"$rfm\" and .state == \"queued\" and .reason_code == \"region_unset\"")" = 1 ] && [ "$(sentc A $NETM "g1 region unset")" = 0 ]'
  facts A $NETM "$(mesh_facts A false true)"
  wait_for 15 '[ "$(nmsg A $NETM "$FAN" ".id == \"$rfm\" and .state == \"sent\"")" = 1 ]' || true
  t "region set again: the same row goes out once (sent; one send reached the stub)" \
    '[ "$(nmsg A $NETM "$FAN" ".id == \"$rfm\" and .state == \"sent\"")" = 1 ] && [ "$(sentc A $NETM "g1 region unset")" = 1 ]'
  # consent gates: a licensed account (HAM unconfirmed) and an uplinked channel (send switch off)
  facts A $NETM "$(mesh_facts A true true)"
  jsend A POST /api/v1/messages/fanout/preview "$(jq -nc --arg c "$FAN" --arg nm $NETM '{text: "g1 ham gate", targets: [{network: $nm, conv: $c}]}')"
  t "preview on a licensed account without a HAM confirmation: refusal ham_unconfirmed with the consent text" \
    '[ "$ST" = 200 ] && jb ".ok == false and .items[0].plan.refusal.code == \"ham_unconfirmed\" and .items[0].plan.refusal.consent.action == \"ham_confirm\""'
  jsend A POST /api/v1/messages/fanout "$(jq -nc --arg c "$FAN" --arg nm $NETM '{text: "g1 ham gate", targets: [{network: $nm, conv: $c}]}')"
  t "fan-out to it → 409 consent_required with the per-target report (consent ham_confirm: the exact text and its hash)" \
    '[ "$ST" = 409 ] && jb ".code == \"consent_required\" and (.targets | length) == 1 and .targets[0].consent.action == \"ham_confirm\" and (.targets[0].consent.hash | test(\"^[0-9a-f]{64}$\"))" && [ "$(jget .targets[0].consent.hash)" = "$(sha256 "$(jget .targets[0].consent.warning_text)")" ]'
  facts A $NETM "$(mesh_facts A false true true)"
  local UPC=${FC[A:$NETM:ch:up]}
  jsend A POST /api/v1/messages/fanout "$(jq -nc --arg c "$UPC" --arg nm $NETM '{text: "g1 uplink gate", targets: [{network: $nm, conv: $c}]}')"
  t "an uplinked channel with its send switch off: fan-out → 409 consent_required (consent uplink_send)" \
    '[ "$ST" = 409 ] && jb ".code == \"consent_required\" and .targets[0].consent.action == \"uplink_send\""'
  jsend A POST /api/v1/messages/fanout/preview "$(jq -nc --arg c "$UPC" --arg nm $NETM '{text: "g1 uplink gate", targets: [{network: $nm, conv: $c}]}')"
  t "… whose preview refusal is uplink_off" '[ "$ST" = 200 ] && jb ".items[0].plan.refusal.code == \"uplink_off\""'
  sleep 2.5
  t "neither gated send queued or reached the stub" \
    '[ "$(nmsg A $NETM "$FAN" ".text == \"g1 ham gate\"")" = 0 ] && [ "$(nmsg A $NETM "$UPC" "true")" = 0 ] && [ "$(sentc A $NETM "gate")" = 0 ]'
  facts A $NETM "$(mesh_facts A)"

  # ---------------------------------------------------------------- relay edges (one node)
  phase "7. relay edges on A: consent round-trip, proposal, endpoints, history, guards, ok_to_mqtt, live consent"
  local body1 h1 E1
  body1=$(jq -nc --arg s "$SOLO" --arg d "$GSOLO" '{src: $s, dst: $d}')
  jsend A POST /api/v1/bridge/edges "$body1"
  h1=$(jget .consent.hash)
  t "POST /bridge/edges without accept_warning → 409 consent_required with the exact text; hash = sha256(text)" \
    'problem_is 409 consent_required && jb ".consent.action == \"edge_enable\" and (.consent.warning_text | test(\"will have their messages copied\"))" && [ "$h1" = "$(sha256 "$(jget .consent.warning_text)")" ]'
  jsend A POST /api/v1/bridge/edges "$(jq -c '. + {accept_warning: "0000000000000000000000000000000000000000000000000000000000000000"}' <<<"$body1")"
  t "a stale hash → 409 consent_required again; nothing created" 'problem_is 409 consent_required && api A GET /api/v1/bridge/edges && jb ".edges | length == 0"'
  jsend A POST /api/v1/bridge/edges "$(jq -c --arg h "$h1" '. + {accept_warning: $h}' <<<"$body1")"
  E1=$(jget .id)
  t "with the hash → 201 edge $E1 (enabled_by loopback, consent_hash recorded, cursor_seq = the store's seq at enable)" \
    '[ "$ST" = 201 ] && jb ".enabled_by == \"loopback\" and .consent_hash == \"$h1\" and .relayed_seq == 0 and .cursor_seq > 0"'
  jsend A POST /api/v1/bridge/edges "$(jq -nc --arg s "${FC[A:$NETM:dm:!c0ffee01]}" --arg d "$GSOLO" '{src: $s, dst: $d}')"
  t "a DM endpoint → 400 bad_request" 'problem_is 400 bad_request'
  jsend A POST /api/v1/bridge/edges "$(jq -nc --arg s "$SOLO" '{src: $s, dst: $s}')"
  t "src = dst → 400 bad_request" 'problem_is 400 bad_request'
  # the agent's proposal creates nothing until accepted; accept runs the 409 round-trip
  local prop=prop-g1-edge pb ph
  pb=$(printf 'from: %s\nto: %s\n\nthe proposal channel and group talk about the same thing' "${FC[A:$NETM:ch:prop]}" "${FC[A:$NETA:grp:prop]}")
  mkdir -p "${NROOT[A]}/agent/proposals"
  jq -nc --arg id "$prop" --arg nid "${NID[A]}" --arg b "$pb" \
    '{schema: 1, id: $id, node_id: $nid, task_id: null, kind: "bridge_edge", title: "relay proposal channel to group", body: $b, created_at: "2026-10-20T10:00:00Z"}' \
    >"${NROOT[A]}/agent/proposals/$prop.json"
  api A GET "/api/v1/proposals/$prop"
  t "the agent's bridge_edge proposal is listed, undecided" '[ "$ST" = 200 ] && jb ".kind == \"bridge_edge\" and .decision == null"'
  t "… and creates nothing until accepted (one edge only: $E1)" 'api A GET /api/v1/bridge/edges && jb ".edges | length == 1"'
  jsend A POST "/api/v1/proposals/$prop/decision" '{"decision": "accepted"}'
  ph=$(jget .consent.hash)
  t "accepting without the hash → 409 consent_required carrying text and hash (edge_enable); still nothing created" \
    'problem_is 409 consent_required && [ "$ph" = "$(sha256 "$(jget .consent.warning_text)")" ] && api A GET /api/v1/bridge/edges && jb ".edges | length == 1"'
  jsend A POST "/api/v1/proposals/$prop/decision" "$(jq -nc --arg h "$ph" '{decision: "accepted", accept_warning: $h}')"
  t "accepting with the hash → 200; the edge exists with consent_hash = that hash" \
    '[ "$ST" = 200 ] && api A GET /api/v1/bridge/edges && jb "(.edges | length) == 2 and ([.edges[] | select(.consent_hash == \"$ph\" and .src == \"${FC[A:$NETM:ch:prop]}\")] | length == 1)"'
  # history before enable is never relayed
  local HIST=${FC[A:$NETM:ch:hist]} GHIST=${FC[A:$NETA:grp:hist]} E3
  feed A $NETM "$(tx $NETM $ACC_MA channel ch:hist '!e0000003' Hilda hist-1 'g1 history before enable')"
  wait_for 10 '[ "$(nmsg A $NETM "$HIST" "true")" = 1 ]' || true
  mkedge A "$HIST" "$GHIST"
  E3=$EDGE
  feed A $NETM "$(tx $NETM $ACC_MA channel ch:hist '!e0000003' Hilda hist-2 'g1 after enable')"
  wait_for 15 '[ "$(sentc A $NETA "g1 after enable")" = 1 ]' || true
  settle
  t "history before enable is not relayed; the text after it is (copies on grp:hist: $(nmsg A $NETA "$GHIST" '.dir == "own"'))" \
    '[ -n "$E3" ] && [ "$(sentc A $NETA "g1 history before enable")" = 0 ] && [ "$(sentc A $NETA "g1 after enable")" = 1 ] && [ "$(nmsg A $NETA "$GHIST" ".dir == \"own\" and .relayed_from.edge == \"$E3\"")" = 1 ]'
  local cp3
  cp3=$(msg A $NETA "$GHIST" '.dir == "own"')
  t "the copy carries the marker [Hilda/SM] and relayed_from {msg, edge}" \
    'printf "%s" "$cp3" | jqs "(.text == \"[Hilda/SM] g1 after enable\") and (.relayed_from.msg | startswith(\"fm-\")) and .relayed_from.edge == \"$E3\""'
  # a relayed copy is never relayed (guard 1): E1 ch:solo → grp:solo, E2 grp:solo → grp:z
  local E2
  mkedge A "$GSOLO" "$GZ"
  E2=$EDGE
  feed A $NETM "$(tx $NETM $ACC_MA channel ch:solo '!e0000004' Rita relay-1 'g1 relay one')"
  wait_for 15 '[ "$(sentc A $NETA "g1 relay one")" -ge 1 ]' || true
  feed A $NETA "$(tx $NETA $ACC_AA group grp:solo stranger-1 Sam sam-1 'g1 stranger in the group')"
  wait_for 15 '[ "$(sentc A $NETA "g1 stranger in the group")" -ge 1 ]' || true
  settle
  t "a relayed copy is never relayed: grp:solo's copy of 'g1 relay one' did not travel on to grp:z (edges are non-transitive)" \
    '[ -n "$E2" ] && [ "$(sentc A $NETA "g1 relay one")" = 1 ] && [ "$(nmsg A $NETA "$GZ" ".text | contains(\"g1 relay one\")")" = 0 ]'
  t "positive control: an inbound text on grp:solo is relayed to grp:z once ([Sam/SA])" \
    '[ "$(nmsg A $NETA "$GZ" ".text == \"[Sam/SA] g1 stranger in the group\"")" = 1 ]'
  # a human-typed marker: not relayed, the row says why
  feed A $NETM "$(tx $NETM $ACC_MA channel ch:solo '!e0000005' Typist typed-1 '[bob/SM] g1 typed marker')" \
    "$(tx $NETM $ACC_MA channel ch:solo '!e0000005' Typist typed-2 '(2/3) [bob/SA] g1 typed part marker')"
  settle
  t "a text that parses as already relayed (also behind a (k/n) prefix) is relayed nowhere and reads not_relayed = marker" \
    '[ "$(sentc A $NETA "g1 typed")" = 0 ] && [ "$(nmsg A $NETM "$SOLO" "(.text | contains(\"g1 typed\")) and .not_relayed == \"marker\"")" = 2 ]'
  # own echo from any account of this node, and a recorded own native id
  local ownid
  ownid=$(jq -rs '[.[] | select(.native_id != null)] | last | .native_id // empty' "${CTLD[A]}/$NETM.sent.jsonl" 2>/dev/null)
  feed A $NETM "$(tx $NETM $ACC_MA channel ch:solo "$ACC_MA2" 'second board' echo-1 'g1 echo from our second board')" \
    "$(tx $NETM $ACC_MA channel ch:solo "$ACC_MA" 'board' echo-2 'g1 echo from our first board')" \
    "$(tx $NETM $ACC_MA channel ch:solo '!e0000006' 'Not us' "$ownid" 'g1 echo carrying our own id')" \
    "$(tx $NETM $ACC_MA channel ch:solo '!e0000006' 'Not us' fresh-1 'g1 not an echo')"
  wait_for 10 '[ "$(nmsg A $NETM "$SOLO" ".text == \"g1 not an echo\"")" = 1 ]' || true
  settle 3
  t "own echo dropped from any account of this node (both boards) and on a recorded own id ($ownid): never stored, never relayed" \
    '[ -n "$ownid" ] && [ "$(nmsg A $NETM "$SOLO" ".text | contains(\"g1 echo\")")" = 0 ] && [ "$(sentc A $NETA "g1 echo")" = 0 ]'
  t "positive control: the stranger's text in the same batch is stored and relayed" \
    '[ "$(nmsg A $NETM "$SOLO" ".text == \"g1 not an echo\"")" = 1 ] && wait_for 10 "[ \"\$(sentc A $NETA \"g1 not an echo\")\" = 1 ]"'
  # oversize on a refuse edge: counted, never truncated
  local r0
  edge A "$E1"
  r0=$(jget .counters.refused)
  feed A $NETM "$(tx $NETM $ACC_MA channel ch:solo '!e0000007' Olga over-1 "g1 oversize $(printf 'y%.0s' $(seq 1 180))")"
  wait_for 15 'edge A "$E1"; [ "$(jget .counters.refused)" = $((r0 + 1)) ]' || true
  t "a copy over the destination's 120 B cap on a refuse edge is not made (refused $r0 → $(jget .counters.refused)); nothing truncated reached the stub" \
    '[ "$(jget .counters.refused)" = $((r0 + 1)) ] && [ "$(sentc A $NETA "g1 oversize")" = 0 ]'
  # honour_ok_to_mqtt on and off
  jsend A PATCH "/api/v1/bridge/edges/$E1" '{"honour_ok_to_mqtt": true}'
  t "PATCH honour_ok_to_mqtt: true → 200" '[ "$ST" = 200 ] && jb ".honour_ok_to_mqtt == true"'
  r0=$(jget .counters.refused)
  feed A $NETM "$(tx $NETM $ACC_MA channel ch:solo '!e0000008' Mia mqtt-1 'g1 mqtt denied' '{"ok_to_mqtt": false}')" \
    "$(tx $NETM $ACC_MA channel ch:solo '!e0000008' Mia mqtt-2 'g1 mqtt allowed' '{"ok_to_mqtt": true}')"
  wait_for 15 '[ "$(sentc A $NETA "g1 mqtt allowed")" = 1 ]' || true
  settle 4
  edge A "$E1"
  t "honour on: ok_to_mqtt=false is not relayed and counts refused ($r0 → $(jget .counters.refused)); ok_to_mqtt=true is relayed" \
    '[ "$(sentc A $NETA "g1 mqtt denied")" = 0 ] && [ "$(sentc A $NETA "g1 mqtt allowed")" = 1 ] && [ "$(jget .counters.refused)" = $((r0 + 1)) ]'
  jsend A PATCH "/api/v1/bridge/edges/$E1" '{"honour_ok_to_mqtt": false}'
  feed A $NETM "$(tx $NETM $ACC_MA channel ch:solo '!e0000008' Mia mqtt-3 'g1 mqtt off denied bit' '{"ok_to_mqtt": false}')"
  wait_for 15 '[ "$(sentc A $NETA "g1 mqtt off denied bit")" = 1 ]' || true
  t "honour off (the default): the same bit is relayed" '[ "$(sentc A $NETA "g1 mqtt off denied bit")" = 1 ]'
  # live consent: a licensed flip on the destination account pauses the edge; a new confirmation resumes it
  facts A $NETA "$(app_facts A true)"
  feed A $NETM "$(tx $NETM $ACC_MA channel ch:solo '!e0000009' Lena lic-1 'g1 after the licensed flip')"
  wait_for 15 'edge A "$E1"; jb ".paused == true"' || true
  t "the destination account turned licensed: the edge pauses at the next copy (paused, pause_reason consent_paused); no copy made" \
    'jb ".paused == true and .pause_reason == \"consent_paused\"" && [ "$(sentc A $NETA "g1 after the licensed flip")" = 0 ] && [ "$(nmsg A $NETA "$GSOLO" ".text | contains(\"g1 after the licensed flip\")")" = 0 ]'
  jsend A PATCH "/api/v1/bridge/edges/$E1" '{"paused": false}'
  local hl
  hl=$(jget .consent.hash)
  t "resuming without the new hash → 409 consent_required with the NEW text (licensed add-on) and its hash" \
    'problem_is 409 consent_required && jb ".consent.action == \"edge_resume\" and (.consent.addons | index(\"licensed_destination\") != null)" && [ "$hl" != "$h1" ] && [ "$hl" = "$(sha256 "$(jget .consent.warning_text)")" ]'
  jsend A PATCH "/api/v1/bridge/edges/$E1" "$(jq -nc --arg h "$hl" '{paused: false, accept_warning: $h}')"
  t "resuming with the new hash → 200, unpaused, consent_hash = the new hash" '[ "$ST" = 200 ] && jb ".paused == false and .consent_hash == \"$hl\""'
  wait_for 15 '[ "$(nmsg A $NETA "$GSOLO" ".text | contains(\"g1 after the licensed flip\")")" = 1 ]' || true
  t "the waiting text is relayed after the confirmation, and its copy waits at the HAM gate (queued, ham_unconfirmed)" \
    '[ "$(nmsg A $NETA "$GSOLO" "(.text | contains(\"g1 after the licensed flip\")) and .state == \"queued\" and .reason_code == \"ham_unconfirmed\"")" = 1 ] && [ "$(sentc A $NETA "g1 after the licensed flip")" = 0 ]'
  facts A $NETA "$(app_facts A false)"
  feed A $NETM "$(tx $NETM $ACC_MA channel ch:solo '!e0000009' Lena lic-2 'g1 after the licensed flip back')"
  wait_for 15 'edge A "$E1"; jb ".paused == true"' || true
  t "flipping licensed back is a change too: paused again (consent_paused)" 'jb ".paused == true and .pause_reason == \"consent_paused\""'
  jsend A PATCH "/api/v1/bridge/edges/$E1" '{"paused": false}'
  hl=$(jget .consent.hash)
  jsend A PATCH "/api/v1/bridge/edges/$E1" "$(jq -nc --arg h "$hl" '{paused: false, accept_warning: $h}')"
  wait_for 15 '[ "$(sentc A $NETA "g1 after the licensed flip back")" = 1 ] && [ "$(sentc A $NETA "g1 after the licensed flip")" -ge 2 ]' || true
  t "confirmed again: both waiting copies go out once each (the gate lifted with the licence)" \
    '[ "$(sentc A $NETA "g1 after the licensed flip back")" = 1 ] && [ "$(sentc A $NETA "[Lena/SM] g1 after the licensed flip")" = 2 ]'
  # live consent: the uplink of the destination channel
  local GUP=${FC[A:$NETA:grp:up]} E4 hu
  mkedge A "$GUP" "$UPC"
  E4=$EDGE
  feed A $NETA "$(tx $NETA $ACC_AA group grp:up stranger-2 Uma up-1 'g1 before the uplink flip')"
  wait_for 15 '[ "$(sentc A $NETM "g1 before the uplink flip")" = 1 ]' || true
  facts A $NETM "$(mesh_facts A false true true)"
  feed A $NETA "$(tx $NETA $ACC_AA group grp:up stranger-2 Uma up-2 'g1 after the uplink flip')"
  wait_for 15 'edge A "$E4"; jb ".paused == true"' || true
  t "the destination channel turned uplinked: the edge $E4 pauses (consent_paused) before any copy of the next text" \
    '[ -n "$E4" ] && [ "$(sentc A $NETM "g1 before the uplink flip")" = 1 ] && jb ".paused == true and .pause_reason == \"consent_paused\"" && [ "$(sentc A $NETM "g1 after the uplink flip")" = 0 ]'
  jsend A PATCH "/api/v1/bridge/edges/$E4" '{"paused": false}'
  hu=$(jget .consent.hash)
  t "… the new text names the uplink (uplinked_destination add-on)" 'problem_is 409 consent_required && jb ".consent.addons | index(\"uplinked_destination\") != null"'
  jsend A PATCH "/api/v1/bridge/edges/$E4" "$(jq -nc --arg h "$hu" '{paused: false, accept_warning: $h}')"
  wait_for 15 '[ "$(nmsg A $NETM "$UPC" "(.text | contains(\"g1 after the uplink flip\")) and .reason_code == \"uplink_off\"")" = 1 ]' || true
  t "confirmed: the copy is made and waits at the channel's send switch (queued, uplink_off), nothing transmitted" \
    '[ "$(nmsg A $NETM "$UPC" "(.text | contains(\"g1 after the uplink flip\")) and .state == \"queued\" and .reason_code == \"uplink_off\"")" = 1 ] && [ "$(sentc A $NETM "g1 after the uplink flip")" = 0 ]'
  facts A $NETM "$(mesh_facts A)"
  # user rows go ahead of relay copies (destination region unset, then set)
  local ORD=${FC[A:$NETM:ch:order]} GORD=${FC[A:$NETA:grp:order]} E5
  mkedge A "$ORD" "$GORD"
  E5=$EDGE
  facts A $NETA "$(app_facts A false false)"
  feed A $NETM "$(tx $NETM $ACC_MA channel ch:order '!e000000a' Otto ord-1 'g1 order relay copy')"
  wait_for 15 '[ "$(nmsg A $NETA "$GORD" ".text | contains(\"g1 order relay copy\")")" = 1 ]' || true
  fsend A $NETA "$GORD" 'g1 order user row'
  t "with the destination's region unset a relay copy (older) and a user row (newer) both wait" \
    '[ "$ST" = 202 ] && [ "$(nmsg A $NETA "$GORD" ".state == \"queued\"")" = 2 ] && [ "$(sentc A $NETA "g1 order")" = 0 ]'
  facts A $NETA "$(app_facts A)"
  wait_for 15 '[ "$(sentc A $NETA "g1 order")" = 2 ]' || true
  t "region set: the user row reaches the stub ahead of the older relay copy (send order: $(sent_texts A $NETA 'g1 order' | tr '\n' '|'))" \
    '[ "$(sent_texts A $NETA "g1 order" | head -1)" = "g1 order user row" ] && [ "$(sentc A $NETA "g1 order")" = 2 ]'

  # ---------------------------------------------------------------- crash-safe handoff
  phase "8. a pending row neither blocks later rows nor is re-sent after a crash"
  local GCR=${FC[A:$NETA:grp:crash]} cr1 cr2
  script_out A $NETA '["hold"]' true
  fsend A $NETA "$GCR" 'g1 crash held row'
  cr1=$FM
  wait_for 10 '[ "$(sentc A $NETA "g1 crash held row")" = 1 ]' || true
  fsend A $NETA "$GCR" 'g1 crash later row'
  cr2=$FM
  wait_for 15 '[ "$(nmsg A $NETA "$GCR" ".id == \"$cr2\" and .state == \"sent\"")" = 1 ]' || true
  t "the row handed to a carrier that never answers stays queued while the later row goes out (sent): a pending row blocks nothing" \
    '[ "$(nmsg A $NETA "$GCR" ".id == \"$cr1\" and .state == \"queued\"")" = 1 ] && [ "$(nmsg A $NETA "$GCR" ".id == \"$cr2\" and .state == \"sent\"")" = 1 ] && [ "$(sentc A $NETA "g1 crash held row")" = 1 ]'
  jsend A POST "/api/v1/messages/$cr1/cancel" '{}'
  t "a handed (pending) row is not cancellable (4xx)" '[[ "$ST" =~ ^4 ]]'
  local relayed_before
  relayed_before=$(sentc A $NETA "[")
  stop_node A KILL
  t "A killed with SIGKILL mid-handoff" '[ -z "${DPID[A]}" ]'
  start_node A
  wait_for 20 'api A GET /api/v1/networks; jb "[.networks[] | select(.id | startswith(\"stub-\")) | select(.state == \"live\")] | length == 3"' || true
  settle 8
  local crow
  crow=$(msg A $NETA "$GCR" ".id == \"$cr1\"")
  t "after the restart the pending row reads 'may have been sent': queued, uncertain, reason_code handoff_pending ($(printf '%s' "$crow" | jq -c '{state, uncertain, reason_code}' 2>/dev/null))" \
    'printf "%s" "$crow" | jqs ".state == \"queued\" and .uncertain == true and .reason_code == \"handoff_pending\""'
  t "… and was never handed again: one send of it reached the stub across the crash" '[ "$(sentc A $NETA "g1 crash held row")" = 1 ]'
  t "the ID map held across the restart: no origin was relayed a second time (relay sends $relayed_before → $(sentc A $NETA "["))" \
    '[ "$(sentc A $NETA "[")" = "$relayed_before" ]'
  jsend A POST "/api/v1/messages/$cr1/retry" '{}'
  local cr3
  cr3=$(jget '.id // empty')
  wait_for 15 '[ "$(sentc A $NETA "g1 crash held row")" = 2 ]' || true
  t "resend? (POST /messages/{fm}/retry) mints a NEW row ($cr3) that is handed once: the user's explicit second act, never an automatic one" \
    '[ "$ST" = 200 ] && [[ "$cr3" =~ ^fm- ]] && [ "$cr3" != "$cr1" ] && [ "$(sentc A $NETA "g1 crash held row")" = 2 ]'

  # ---------------------------------------------------------------- two nodes: loops
  phase "9. two nodes, reverse edges, split both ways: no loop (boards named ']' and 'W1AW/7')"
  local LA=${FC[A:$NETM:ch:loop]} GLA=${FC[A:$NETA:grp:loop]} LB=${FC[B:$NETM:ch:loop]} GLB=${FC[B:$NETA:grp:loop]} EA EB
  mkedge A "$LA" "$GLA" '{"oversize": "split"}'
  EA=$EDGE
  mkedge B "$GLB" "$LB" '{"oversize": "split"}'
  EB=$EDGE
  t "A: edge ch:loop → grp:loop ($EA), B: the reverse grp:loop → ch:loop ($EB), both split" '[ -n "$EA" ] && [ -n "$EB" ]'
  local long1 long2
  long1="g1 loop mesh origin $(python3 -I -c 'print(" ".join("m%03d" % i for i in range(40)))')"
  long2="g1 loop app origin $(python3 -I -c 'print(" ".join("a%03d" % i for i in range(70)))')"
  local am0 aa0 bm0 ba0
  am0=$(nlines A $NETM)
  aa0=$(nlines A $NETA)
  bm0=$(nlines B $NETM)
  ba0=$(nlines B $NETA)
  # a third party on the mesh channel, heard by both nodes' boards
  feed A $NETM "$(tx $NETM $ACC_MA channel ch:loop '!e000000b' ']' loop-m1 "$long1")"
  feed B $NETM "$(tx $NETM $ACC_MB channel ch:loop '!e000000b' ']' loop-m1 "$long1")"
  # a third party in the app group, heard by both nodes' accounts
  feed A $NETA "$(tx $NETA $ACC_AA group grp:loop stranger-3 'W1AW/7' loop-a1 "$long2")"
  feed B $NETA "$(tx $NETA $ACC_AB group grp:loop stranger-3 'W1AW/7' loop-a1 "$long2")"
  wait_for 30 '[ "$(sentp A $NETA "[?/SM] ")" -ge 2 ] && [ "$(sentp B $NETM "[W1AW7/SA] ")" -ge 2 ]' || true
  settle 12
  local acopies bcopies arecv brecv
  acopies=$(sentp A $NETA "[?/SM] ")
  bcopies=$(sentp B $NETM "[W1AW7/SA] ")
  t "A split the mesh origin into $acopies parts for the 120 B group, each led by the marker [?/SM] (the name ']' reduced to '?') then (k/n), each ≤ 120 B" \
    '[ "$acopies" -ge 2 ] && [ "$(sent_from A $NETA "$aa0" | grep -c "^\[?/SM\] ([0-9]*/$acopies) ")" = "$acopies" ] && [ "$(sent_from A $NETA "$aa0" | awk "length(\$0) > 120" | wc -l)" = 0 ]'
  t "B split the app origin into $bcopies parts for the 232 B channel, each led by [W1AW7/SA] ('W1AW/7' reduced)" \
    '[ "$bcopies" -ge 2 ] && [ "$(sent_from B $NETM "$bm0" | grep -c "^\[W1AW7/SA\] ([0-9]*/$bcopies) ")" = "$bcopies" ]'
  t "no loop: every send of the phase is one part of the first copy — A sent $(( $(nlines A $NETA) - aa0 )) on the group and $(( $(nlines A $NETM) - am0 )) on the mesh, B $(( $(nlines B $NETM) - bm0 )) on the mesh and $(( $(nlines B $NETA) - ba0 )) on the group" \
    '[ $(( $(nlines A $NETA) - aa0 )) = "$acopies" ] && [ $(( $(nlines B $NETM) - bm0 )) = "$bcopies" ] && [ $(( $(nlines A $NETM) - am0 )) = 0 ] && [ $(( $(nlines B $NETA) - ba0 )) = 0 ]'
  brecv=$(nmsg B $NETA "$GLB" '.dir == "in" and (.text | startswith("[?/SM] "))')
  arecv=$(nmsg A $NETM "$LA" '.dir == "in" and (.text | startswith("[W1AW7/SA] "))')
  t "each node heard the other's parts on the air from a board named ']' / 'W1AW/7' (B: $brecv, A: $arecv), each kept from relaying (not_relayed = marker)" \
    '[ "$brecv" = "$acopies" ] && [ "$arecv" = "$bcopies" ] && [ "$(nmsg B $NETA "$GLB" ".dir == \"in\" and (.text | startswith(\"[?/SM] \")) and .not_relayed == \"marker\"")" = "$acopies" ] && [ "$(nmsg A $NETM "$LA" ".dir == \"in\" and (.text | startswith(\"[W1AW7/SA] \")) and .not_relayed == \"marker\"")" = "$bcopies" ]'
  t "the parts arrived from the far board's name as sent on the air (sender.name ']' at B, 'W1AW/7' at A)" \
    '[ "$(nmsg B $NETA "$GLB" ".dir == \"in\" and (.text | startswith(\"[?/SM] \")) and .sender.name == \"]\"")" = "$acopies" ] && [ "$(nmsg A $NETM "$LA" ".dir == \"in\" and (.text | startswith(\"[W1AW7/SA] \")) and .sender.name == \"W1AW/7\"")" = "$bcopies" ]'
  local a2 b2
  settle 7
  a2=$(sentp A $NETA "[?/SM] ")
  b2=$(sentp B $NETM "[W1AW7/SA] ")
  t "the copy counts are stable after a further settle ($acopies → $a2, $bcopies → $b2)" '[ "$a2" = "$acopies" ] && [ "$b2" = "$bcopies" ]'
  edge A "$EA"
  t "A's edge counted one relayed origin (counters.relayed $(jget .counters.relayed))" 'jb ".counters.relayed == 1"'
  edge B "$EB"
  t "B's edge counted one relayed origin (counters.relayed $(jget .counters.relayed))" 'jb ".counters.relayed == 1"'

  # ---------------------------------------------------------------- the burst
  phase "10. the 1,000-row burst on A and 111 delivery latency A → B"
  local BURST=${FC[A:$NETM:ch:burst]} t2 m2 lat
  ctl A "$(jq -nc --arg s $NETM --arg a "$ACC_MA" '{op: "burst", stub: $s, conv: {network: $s, account: $a, kind: "channel", native_id: "ch:burst"}, n: 1000, sender_native: "!e00000bb", sender_name: "Flood", prefix: "burst"}')"
  t2=${EPOCHREALTIME/,/.}
  jsend A POST /api/v1/messages "$(jq -nc --arg to "${NID[B]}" '{to: $to, text: "g1 during the burst"}')"
  m2=$(jget .id)
  wait_for 30 'api A GET "/api/v1/messages/threads/${NID[B]}?limit=50"; jb --arg id "$m2" "[.messages[]? | select(.id == \$id and .state == \"delivered\")] | length == 1"' || true
  lat=$(awk -v a="$t2" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.2f", b-a}')
  printf 'note: 111 delivery latency A → B during the 1,000-row burst: %ss (baseline %ss)\n' "$lat" "$lat0"
  t "a 111 message sent during the burst is delivered in ${lat}s (bound 10 s)" \
    'jb --arg id "$m2" "[.messages[]? | select(.id == \$id and .state == \"delivered\")] | length == 1" && awk -v l="$lat" "BEGIN{exit !(l < 10)}"'
  wait_for 30 '[ "$(nmsg A $NETM "$BURST" ".dir == \"in\"")" = 1000 ]' || true
  t "all 1,000 burst rows ingested, none lost (dropped counter $(api A GET /api/v1/networks >/dev/null; jget '.networks[] | select(.id == "stub-mesh") | .dropped'))" \
    '[ "$(nmsg A $NETM "$BURST" ".dir == \"in\"")" = 1000 ]'

  # ---------------------------------------------------------------- prune
  phase "11. prune order, the foreign quota, relayed_seq after a shed, the never list"
  local PIN=${FC[A:$NETM:ch:pin]} GPR=${FC[A:$NETA:grp:prune]} E6 pinid
  pinid=${PIN#fc-}
  ctl A "$(jq -nc --argjson c "$pinid" '{op: "pin", conv: $c, pinned: true}')"
  t "ch:pin pinned (scenario hook; no R-B route pins)" '[ -n "$CTL" ] && printf "%s" "$CTL" | jqs ".ok == true"'
  mkedge A "$PIN" "$GPR"
  E6=$EDGE
  facts A $NETA "$(app_facts A false false)"
  feed A $NETM "$(tx $NETM $ACC_MA channel ch:pin '!e000000c' Petra pin-1 'g1 pinned origin one')" \
    "$(tx $NETM $ACC_MA channel ch:pin '!e000000c' Petra pin-2 'g1 pinned origin two')"
  wait_for 15 '[ "$(nmsg A $NETA "$GPR" ".dir == \"own\" and .relayed_from != null")" = 2 ]' || true
  fsend A $NETA "$GPR" 'g1 user row never pruned'
  local ucr=$FM
  t "a backlog: two relay copies and one user row queued on grp:prune (region unset), nothing handed" \
    '[ -n "$E6" ] && [ "$(nmsg A $NETA "$GPR" ".state == \"queued\"")" = 3 ] && [ "$(sentc A $NETA "g1 pinned origin")" = 0 ] && [ "$(sentc A $NETA "g1 user row never")" = 0 ]'
  jsend A PATCH /api/v1/fabric/settings '{"quotas": {"foreign": 30000}}'
  t "PATCH /fabric/settings {quotas: {foreign: 30000}} → 200 (the foreign shaping quota)" '[ "$ST" = 200 ] && jb ".quotas.foreign == 30000"'
  wait_for 80 'n=$(nmsg A $NETM "$BURST" ".dir == \"in\""); [ -n "$n" ] && [ "$n" -lt 1000 ]' || true
  local brows nb lo hi
  brows=$(rows A $NETM "$BURST")
  nb=$(printf '%s' "$brows" | jq '[.[] | select(.dir == "in")] | length' 2>/dev/null)
  lo=$(printf '%s' "$brows" | jq -r '[.[] | select(.dir == "in") | .text | ltrimstr("burst ") | tonumber] | min // 0' 2>/dev/null)
  t "the capacity pass evicted foreign inbound to the quota: $nb of 1,000 burst rows remain, exactly the newest (burst $lo … 1000)" \
    '[ -n "$nb" ] && [ "$nb" -gt 0 ] && [ "$nb" -lt 1000 ] && [ "$lo" = $((1001 - nb)) ]'
  edge A "$E6"
  t "2a before 2b: the relay backlog is still queued after the partial pass (shed $(jget .counters.shed))" \
    'jb ".counters.shed == 0" && [ "$(nmsg A $NETA "$GPR" ".state == \"queued\"")" = 3 ]'
  t "older unpinned inbound went first (the ingest test rows on ch:solo are gone), the pinned origins stay" \
    '[ "$(nmsg A $NETM "$SOLO" ".dir == \"in\"")" = 0 ] && [ "$(nmsg A $NETM "$PIN" ".dir == \"in\"")" = 2 ]'
  jsend A PATCH /api/v1/fabric/settings '{"quotas": {"foreign": 1}}'
  wait_for 80 'edge A "$E6"; jb ".counters.shed >= 2"' || true
  local pmax
  pmax=$(rows A $NETM "$PIN" | jq '[.[] | select(.dir == "in") | .seq] | max // 0')
  edge A "$E6"
  t "quota 1: 2b shed both backlog copies (counters.shed $(jget .counters.shed)), relayed_seq $(jget .relayed_seq) ≥ the newest origin's seq $pmax" \
    'jb ".counters.shed == 2" && [ "$(jget .relayed_seq)" -ge "$pmax" ] && [ "$pmax" -gt 0 ]'
  t "the never list: the pinned conversation's rows and the undelivered user row survive; every burst row is gone" \
    '[ "$(nmsg A $NETM "$PIN" ".dir == \"in\"")" = 2 ] && [ "$(nmsg A $NETA "$GPR" ".id == \"$ucr\" and .state == \"queued\"")" = 1 ] && [ "$(nmsg A $NETM "$BURST" "true")" = 0 ]'
  jsend A PATCH /api/v1/fabric/settings '{"quotas": {"foreign": null}}'
  facts A $NETA "$(app_facts A)"
  wait_for 15 '[ "$(sentc A $NETA "g1 user row never pruned")" = 1 ]' || true
  settle
  t "region set again: the user row goes out; the shed copies are never made again although their origins are still held (relayed_seq held)" \
    '[ "$(sentc A $NETA "g1 user row never pruned")" = 1 ] && [ "$(sentc A $NETA "g1 pinned origin")" = 0 ] && [ "$(nmsg A $NETA "$GPR" ".relayed_from != null")" = 0 ]'

  # ---------------------------------------------------------------- component acknowledgement
  phase "12. enabling an optional component without the recorded acknowledgement"
  api A GET /api/v1/bridge/components
  t "GET /bridge/components lists lxmf, disabled, nothing installed (state $(jget '.components[] | select(.id == "lxmf") | .state'))" \
    '[ "$ST" = 200 ] && jb "[.components[] | select(.id == \"lxmf\" and .installed_bytes == null and .acknowledged == false)] | length == 1"'
  jsend A POST /api/v1/bridge/components/lxmf/enable '{}'
  t "POST /bridge/components/lxmf/enable without accept_notice_hash → 409 consent_required carrying the licence notice and its hash" \
    'problem_is 409 consent_required && jb ".consent.action == \"component_enable\"" && [ "$(jget .consent.hash)" = "$(sha256 "$(jget .consent.warning_text)")" ]'
  jsend A POST /api/v1/bridge/components/lxmf/enable '{"accept_notice_hash": "0000000000000000000000000000000000000000000000000000000000000000"}'
  t "a stale acknowledgement → 409 consent_required again" 'problem_is 409 consent_required'
  t "nothing was installed, prepared or started: no components/lxmf, no venvs/lxmf, no bridge/lxmf, no python child" \
    '[ ! -e "${NROOT[A]}/components/lxmf" ] && [ ! -e "${NROOT[A]}/venvs/lxmf" ] && [ ! -e "${NROOT[A]}/bridge/lxmf" ] && ! pgrep -f lxmf_sidecar >/dev/null'

  # ---------------------------------------------------------------- tapes
  phase "13. SSE: network_message and bridge frames (tapes)"
  sleep 1
  kill "$SSEP" 2>/dev/null
  wait "$SSEP" 2>/dev/null
  SSEP=""
  local nm br burstf
  nm=$(sse_data "$W/A-events.sse" network_message | grep -c . || true)
  br=$(sse_data "$W/A-events.sse" bridge | grep -c . || true)
  burstf=$(sse_data "$W/A-events.sse" network_message | jq -c --arg c "$BURST" 'select(.conv == $c)' 2>/dev/null | grep -c . || true)
  t "A's stream carried $nm network_message frames ({network, conv, change, id per row}, never the text) and $br bridge frames" \
    '[ "$nm" -gt 0 ] && [ "$br" -gt 0 ] && sse_data "$W/A-events.sse" network_message | jq -se "all(has(\"network\") and has(\"conv\") and has(\"change\") and (has(\"text\") | not) and (has(\"body\") | not)) and any(has(\"id\"))" >/dev/null 2>&1'
  t "the 1,000-row burst was coalesced per conversation: $burstf network_message frames for ch:burst" '[ "$burstf" -ge 1 ] && [ "$burstf" -lt 200 ]'
  t "no foreign text appears on the stream (network_message frames carry no body)" '! grep -q "g1 loop mesh origin\|burst 000500" "$W/A-events.sse"'
  if [ "${G1_TAPES:-0}" = 1 ] || [ ! -s "$TAPES/events-l6-network_message.sse" ] || [ ! -s "$TAPES/events-l6-bridge.sse" ]; then
    mkdir -p "$TAPES"
    sse_frames "$W/A-events.sse" network_message | sed "s#$G1_WORK#/var/lib/111#g" >"$TAPES/events-l6-network_message.sse"
    sse_frames "$W/A-events.sse" bridge | sed "s#$G1_WORK#/var/lib/111#g" >"$TAPES/events-l6-bridge.sse"
    note "tapes written: $TAPES/events-l6-network_message.sse, $TAPES/events-l6-bridge.sse"
  fi
  t "tapes present in $(basename "$TAPES")/ (events-l6-network_message.sse, events-l6-bridge.sse)" \
    '[ -s "$TAPES/events-l6-network_message.sse" ] && [ -s "$TAPES/events-l6-bridge.sse" ]'

  # ---------------------------------------------------------------- stop
  phase "14. stop"
  for n in B A; do
    stop_node "$n" TERM
    t "$n: SIGTERM → the daemon exited" '[ -z "${DPID[$n]}" ]'
  done
  grep -h "111 bridge scenario" "${NROOT[A]}/daemon.log" | sort | uniq -c | sed 's/^/note: A| /' | head -5
}

# ------------------------------------------------------------------ outer

case "${1:-}" in
  --inner)
    inner
    exit $((FAILS > 0))
    ;;
  "") ;;
  *)
    echo "usage: bash 6-transport/verify/bridge.sh" >&2
    exit 2
    ;;
esac

echo "6-transport/verify/bridge.sh — Layer 6 G1 bridge smoke"
for tool in curl jq python3 unshare nsenter ip sha256sum cargo git tar; do
  command -v "$tool" >/dev/null 2>&1 || skip_or_fail G1 "$tool is not on PATH"
done
have_netns || skip_or_fail G1 "unprivileged network namespaces are not available (unshare -Urnm)"

WORK=$(mktemp -d -t 111-g1.XXXXXX)
cleanup() {
  if [ "${SMOKE_KEEP:-0}" = 1 ] || [ "$FAILS" -gt 0 ]; then
    echo "note: scratch kept at $WORK (A/daemon.log, B/daemon.log, A/g1-ctl, http/)"
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT
LOGS=$WORK/logs
W=$WORK/http
mkdir -p "$LOGS" "$W"

# ------------------------------------------------------------------ 1. cargo
phase "cargo: the debug 111 and the store tests G1 names"
TGT_BOOT=${CARGO_TARGET_DIR:-$BOOT/target}
BIN111=$TGT_BOOT/debug/111
if [ "${G1_SKIP_BUILD:-${SMOKE_SKIP_BUILD:-0}}" = 1 ]; then
  note "G1_SKIP_BUILD=1 — cargo build skipped, reusing $BIN111"
else
  t1=${EPOCHREALTIME/,/.}
  if (cd "$BOOT" && cargo build --bin 111) >"$LOGS/cargo-111.log" 2>&1; then
    ok "cargo build --bin 111 (debug: the G1 scenario hook is compiled in; $(awk -v a="$t1" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}')s)"
  else
    fail "cargo build --bin 111 (log: $LOGS/cargo-111.log)"
    tail -20 "$LOGS/cargo-111.log" | sed 's/^/  cargo| /'
  fi
fi
[ -x "$BIN111" ] || skip_or_fail G1 "no debug 111 binary at $BIN111"

# mesh111 store tests (§16 G1: V2 opens at V3, prune order, the foreign quota, bounded foreign_seen)
run_tests() { # run_tests DIR TARGET LOG NAME… — each NAME passing is one ok line
  local dir=$1 tgt=$2 log=$3
  shift 3
  (cd "$dir" && CARGO_TARGET_DIR=$tgt cargo test -p mesh111 --lib -- --exact "$@") >"$log" 2>&1
  local name
  for name in "$@"; do
    if grep -q "^test $name \.\.\. ok$" "$log"; then
      ok "cargo test (mesh111): $name"
    else
      fail "cargo test (mesh111): $name did not pass (log: $log)"
    fi
  done
}
MESH_TGT=${G1_MESH_TARGET_DIR:-$MSG/target}
if [ "${G1_SKIP_CARGO_TESTS:-0}" = 1 ]; then
  note "G1_SKIP_CARGO_TESTS=1 — the mesh111 store tests and the V2-tag build are skipped (not counted)"
else
  run_tests "$MSG" "$MESH_TGT" "$LOGS/cargo-mesh111.log" \
    store::tests::a_v2_store_opens_at_v3_and_keeps_its_rows \
    store::tests::a_store_newer_than_v3_is_refused \
    store::tests::pruning_runs_2a_then_2b_between_steps_2_and_3_and_keeps_the_never_list \
    store::tests::the_foreign_shaping_quota_evicts_first_and_never_refuses \
    store::tests::foreign_seen_is_bounded_per_edge_and_evicts_the_oldest \
    store::tests::a_copy_with_a_pending_handoff_is_pruned_as_handed_never_shed
  # "a V3 store is refused by a V2 schema": the Layer 5 tag's own mesh111 (schema 2), with one
  # test added in the scratch copy that sets user_version = 3 and opens it
  L5=$WORK/l5
  mkdir -p "$L5"
  if git -C "$OS" archive checkpoint-layer-5 50-messaging/Cargo.toml 50-messaging/Cargo.lock 50-messaging/crates/mesh111 20-identity 2>"$LOGS/l5-archive.log" | tar -x -C "$L5" 2>>"$LOGS/l5-archive.log"; then
    v2=$(sed -n 's/^pub const SCHEMA_VERSION: i64 = \([0-9]*\);/\1/p' "$L5/50-messaging/crates/mesh111/src/store/schema.rs")
    t "the Layer 5 tag (checkpoint-layer-5) ships mesh111 at schema version $v2" '[ "$v2" = 2 ]'
    cat >>"$L5/50-messaging/crates/mesh111/src/store/tests.rs" <<'EOF'

/// Layer 6 G1 (added by 6-transport/verify/bridge.sh in its scratch copy of the Layer 5 tag):
/// a store at user_version 3 (a V3 store) is refused by this V2 build.
#[test]
fn g1_a_v3_store_is_refused_by_this_v2_build() {
    let (t, s) = ready();
    s.conn.pragma_update(None, "user_version", 3).unwrap();
    s.close(1).unwrap();
    assert!(matches!(
        Store::open(t.path(), &key(), &params(1)),
        Err(StoreError::SchemaTooNew { found: 3, supported: 2 })
    ));
}
EOF
    (cd "$L5/50-messaging" && CARGO_TARGET_DIR=$MESH_TGT cargo test --locked -p mesh111 --lib -- --exact store::tests::g1_a_v3_store_is_refused_by_this_v2_build) >"$LOGS/cargo-l5.log" 2>&1
    t "cargo test (mesh111 at checkpoint-layer-5, V2): a V3 store (user_version = 3) is refused with SchemaTooNew {found: 3, supported: 2}" \
      'grep -q "^test store::tests::g1_a_v3_store_is_refused_by_this_v2_build \.\.\. ok$" "$LOGS/cargo-l5.log"'
  else
    fail "git archive checkpoint-layer-5 (log: $LOGS/l5-archive.log)"
  fi
fi

# ------------------------------------------------------------------ 2. roots
phase "roots A and B: identities, fabric/settings, the G1 scenario files"
for n in A B; do
  g6_make_root "$WORK/$n" "# G1 bridge smoke: dashboard on loopback; the fabric listener follows fabric/settings"
  nid=$(g6_identity "$WORK/$n" "$BIN111")
  if [ -n "$nid" ]; then
    printf '%s\n' "$nid" >"$WORK/$n.nid"
    ok "$n: identity generated ($nid)"
  else
    fail "$n: 111 identity generate ($(head -c 200 "$WORK/$n/identity.log"))"
  fi
done
NID_B=$(cat "$WORK/B.nid" 2>/dev/null || true)
g6_fabric_settings "$WORK/A" "{\"schema\": 1, \"listen_lan\": true, \"peers\": [{\"node_id\": \"$NID_B\", \"address\": \"10.111.6.12:11183\"}]}"
g6_fabric_settings "$WORK/B" '{"schema": 1, "listen_lan": true}'
MESH_SPEC='"id":"stub-mesh","label":"Mesh stub","tag":"SM","kinds":["channel","dm"],"max_text_bytes":{"channel":232,"dm":220,"group":null},"airtime":{"duty_cycle_permille":10,"bits_per_s":1000,"preset":"LongFast"}'
APP_SPEC='"id":"stub-app","label":"App stub","tag":"SA","kinds":["dm","group"],"max_text_bytes":{"channel":null,"dm":4000,"group":120}'
LXMF_SPEC='"id":"stub-lxmf","label":"LXMF stub","tag":"SL","kinds":["dm","group"],"max_text_bytes":{"channel":null,"dm":287,"group":287},"recipient_syntax":{"pattern":"^[0-9a-f]{32}$","hint":"a 32-hex destination"}'
mf() { # the initial facts (the inner script's mesh_facts/app_facts with their defaults)
  case $1 in
    mesh-A) printf '{"accounts":[{"id":"!a0000001","label":"board","licensed":false},{"id":"!a0000002","label":"second board","licensed":false}],"channels":[{"account":"!a0000001","native_id":"ch:up","uplink":false,"default_key":false}],"region_set":true,"ids_globally_unique":false,"parts_may_reorder":true,"licensed_caps":{"channel":232,"dm":232,"group":null}}' ;;
    mesh-B) printf '{"accounts":[{"id":"!b0000001","label":"board","licensed":false}],"channels":[{"account":"!b0000001","native_id":"ch:up","uplink":false,"default_key":false}],"region_set":true,"ids_globally_unique":false,"parts_may_reorder":true,"licensed_caps":{"channel":232,"dm":232,"group":null}}' ;;
    app-A) printf '{"accounts":[{"id":"app-a","label":"bridge account","licensed":false}],"channels":[],"region_set":true,"ids_globally_unique":true,"parts_may_reorder":false,"licensed_caps":null}' ;;
    app-B) printf '{"accounts":[{"id":"app-b","label":"bridge account","licensed":false}],"channels":[],"region_set":true,"ids_globally_unique":true,"parts_may_reorder":false,"licensed_caps":null}' ;;
    lxmf-A) printf '{"accounts":[{"id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","label":null,"licensed":false}],"channels":[],"region_set":null,"ids_globally_unique":true,"parts_may_reorder":false,"licensed_caps":null}' ;;
  esac
}
cat >"$WORK/A/g1-scenario.json" <<EOF
{"control": "$WORK/A/g1-ctl", "node": "a", "air": "$WORK/air",
 "stubs": [{$MESH_SPEC, "facts": $(mf mesh-A), "air_name": "]"},
           {$LXMF_SPEC, "facts": $(mf lxmf-A)},
           {$APP_SPEC, "facts": $(mf app-A), "air_name": "]"}]}
EOF
cat >"$WORK/B/g1-scenario.json" <<EOF
{"control": "$WORK/B/g1-ctl", "node": "b", "air": "$WORK/air",
 "stubs": [{$MESH_SPEC, "facts": $(mf mesh-B), "air_name": "W1AW/7"},
           {$APP_SPEC, "facts": $(mf app-B), "air_name": "W1AW/7"}]}
EOF
if jq -e . "$WORK/A/g1-scenario.json" >/dev/null 2>&1 && jq -e . "$WORK/B/g1-scenario.json" >/dev/null 2>&1; then
  ok "scenario files: A stub-mesh (232/220 B, LongFast, two accounts) + stub-lxmf (DM + lists) + stub-app (120 B groups); B stub-mesh + stub-app; one shared air"
else
  fail "the scenario files are not JSON"
fi

# ------------------------------------------------------------------ 3. the namespaces
if env --default-signal true 2>/dev/null; then
  RESET_SIG=(env --default-signal)
else
  RESET_SIG=(python3 -c 'import os,signal,sys
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])')
fi
INNER_LOG=$WORK/inner.log
G1_WORK=$WORK BIN111=$BIN111 G1_T0=$T0 G1_TAPES=${G1_TAPES:-0} G1_TAPES_DIR=$TAPES \
  unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
  "${RESET_SIG[@]}" bash "$HERE/bridge.sh" --inner 2>&1 | tee "$INNER_LOG" || true
inner_fails=$(grep -c '^FAIL:' "$INNER_LOG" || true)
if ! grep -q '^FAILS=' "$WORK/inner.result" 2>/dev/null; then
  fail "the namespace run did not finish (killed or crashed — see above)"
fi
FAILS=$((FAILS + inner_fails))

# ------------------------------------------------------------------ hygiene
phase "hygiene: nothing of ours left running"
left=""
for p in /proc/[0-9]*; do
  pid=${p#/proc/}
  [ "$pid" = $$ ] && continue
  if { tr '\0' ' ' <"$p/cmdline"; } 2>/dev/null | grep -qF "$WORK" || { tr '\0' '\n' <"$p/environ"; } 2>/dev/null | grep -qF "NODE111_ROOT=$WORK"; then
    left="$left $pid:$(cat "$p/comm" 2>/dev/null)"
  fi
done
t "no daemon, stand-in agent, holder or curl of this run is left (/proc cmdline + NODE111_ROOT)${left:+:$left}" '[ -z "$left" ]'
total_ok=$((OKS + $(grep -c '^ok:' "$INNER_LOG" || true)))
echo
if [ "$FAILS" -eq 0 ]; then
  echo "G1: $total_ok ok, 0 FAIL in $(elapsed)s"
else
  echo "G1: $total_ok ok, $FAILS FAIL in $(elapsed)s"
fi
finish G1
