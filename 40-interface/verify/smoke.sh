#!/usr/bin/env bash
# 40-interface/verify/smoke.sh — Layer 4 end-to-end smoke (spec §14, TASKS.md I1), and with
# --three-node the Layer 5 three-node harness (os/50-messaging/README.md §20 G1, TASKS.md V1).
#
#   bash 40-interface/verify/smoke.sh               single node (Layer 4; steps 1–5 below)
#   bash 40-interface/verify/smoke.sh --three-node  hub + members + requester (Layer 5; see
#                                                "three-node mode" further down)
#
#   1. UI      pnpm install (only when node_modules is stale) + pnpm build → ui/dist
#   2. Rust    cargo build (debug) of `111` (os/10-bootstrap) and `111-agent` (os/30-intelligence).
#              A debug `111` reads ui/dist from disk per request (rust-embed debug mode),
#              so a UI rebuild needs no Rust rebuild — asserted below with a probe file.
#   3. Root    a scratch NODE111_ROOT: identity (`111 identity generate`), releases/<ver>
#              with the KB (os/00-knowledge entries) + payload manifest, `111-agent`, and
#              `stub-llama.py` installed as `bin/llama-server`; a dummy .gguf; LAN on.
#   4. Run     inside `unshare -rnpf` (user + net + pid namespaces: loopback plus
#              dummy0 192.168.77.5/24, no internet): `111 daemon`, which itself
#              supervises `111-agent` and the stub model server (its real child path).
#   5. Assert  static UI + cache headers, Host guard, /api/v1 read routes + schema
#              keys, pairing from a real non-loopback source (dummy0), CSRF, events
#              reset + Last-Event-ID replay, chat relay (token/tool/done/error),
#              history, proposals + decisions, agent task → proposal frames, update
#              check, per-service restart (supervised child → 202 + back LIVE with a
#              new pid; non-supervised → 409), node restart (exit 0, children joined).
#              Also: CSP + security headers, problem+json for framework errors, the
#              Layer 1 /api/status + /v1 gated for unpaired LAN visitors.
#              SSE tapes go to ui/test/tapes/ (see SMOKE_TAPES).
#
# Every check prints `ok:` or `FAIL:` (`note:` lines are information only); the
# script exits 1 on any FAIL. No check may pass on a missing answer: JSON assertions go
# through jb/jf/jqs (they refuse an empty input — `jq -e` alone exits 0 on one), counts
# and peer numbers read nothing from a failed request, and every "nothing left / nothing
# changed" check carries a positive control (the thing was there before).
# Every process dies with the pid namespace; the
# scratch root is removed on success (kept on failure, or with SMOKE_KEEP=1).
#
# Env: SMOKE_KEEP=1        keep the scratch root
#      SMOKE_TAPES=1       re-record the SSE tapes (default: record only missing ones,
#                          so routine runs do not rewrite the UI's test fixtures)
#      SMOKE_TAPES_DIR=DIR write tapes into DIR instead of ui/test/tapes (with SMOKE_TAPES=1:
#                          a fresh recording to diff against the committed fixtures)
#      SMOKE_SKIP_BUILD=1  dev loop only: reuse ui/dist and target/ as they are
#      TMPDIR              where the scratch root goes (mktemp)
#
# Three-node mode (--three-node; Layer 5 §9, §20 G1). The UI build is skipped (no browser
# here); `cargo build` runs (a no-op after the single-node mode). Four scratch roots with
# their own identities, each a real `111 daemon` (+ its own 111-agent and stub model) in
# its own net namespace, joined by veth pairs to a bridge in one user+pid namespace:
#      br0 10.111.0.1/24 · H 10.111.0.10 (hub) · A 10.111.0.11 · B 10.111.0.12 (members)
#      · C 10.111.0.13 (never enrolled: the non-member of (4) and the requester of (7)).
# Dashboards stay on loopback; every assertion goes through each node's own loopback
# /api/v1 (curl via nsenter) or its CLI. The fabric listener follows fabric/settings
# (hub_role or listen_lan); hand-configured direct peers live in that file (`peers`).
# Recording relays (tcp-lan-relay.py) sit on two paths so the wire can be counted and
# replayed: R1 in B's netns (10.111.0.12:11184 → B:11183; A's direct peer address for B)
# and R2 in H's netns (10.111.0.10:11184 → H:11183; A enrols through it). H carries a
# Meshtastic binding to V2's stub-meshmud.py (meshmud + presence + fake board roles).
# Scenarios (TASKS.md V1): (1) cards + node1… · (2) direct, 16 fragments · (5) replay ·
# (4) non-member · (7) message request · (8) agent tools · (3) hub custody · (6) unclean
# restart · (9) Meshtastic · (10) clean restart. SSE tapes: events-l5-*.sse, chat-l5-*.sse.
# Checks are condition strings eval'd by t() at check time, so single-quoted
# `$vars`, variables read only inside them, and helpers called only from them
# are intended:
# shellcheck disable=SC2016,SC2034,SC2317
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
OS=$(cd "$HERE/../.." && pwd)
UI=$OS/40-interface/ui
BOOT=$OS/10-bootstrap
INTEL=$OS/30-intelligence
TAPES=${SMOKE_TAPES_DIR:-$UI/test/tapes}
export PATH="$HOME/.cargo/bin:$PATH:/usr/sbin:/sbin"

LANIP=192.168.77.5
B=http://127.0.0.1:11180
L=http://$LANIP:11180

FAILS=0
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
T0=${SMOKE_T0:-${EPOCHREALTIME/,/.}}
elapsed() { awk -v a="$T0" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}'; }
phase() { printf '\n== %s  (t+%ss)\n' "$*" "$(elapsed)"; }

# ------------------------------------------------------------------ helpers (inner)

# req METHOD URL [curl args…] → $ST (status, 000 = no answer), $W/hdr, $W/body
# (NSX: an optional command prefix — three-node mode runs curl inside a node's netns)
NSX=()
req() {
  local m=$1 u=$2
  shift 2
  : >"$W/hdr"
  : >"$W/body"
  ST=$("${NSX[@]}" curl -sS -o "$W/body" -D "$W/hdr" -w '%{http_code}' -X "$m" --max-time "${REQ_TIMEOUT:-15}" "$@" "$u" 2>"$W/curl.err") || true
  [ -n "$ST" ] || ST=000
}
# hdr NAME → first value of that response header (case-insensitive, CR stripped)
hdr() {
  awk -v n="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')" '
    { line=$0; sub(/\r$/, "", line); k=tolower(line); sub(/:.*/, "", k) }
    k==n { sub(/^[^:]*:[ \t]*/, "", line); print line; exit }' "$W/hdr"
}
ct_is() { case "$(hdr content-type)" in "$1"*) return 0 ;; *) return 1 ;; esac }
# JSON checks. jq (1.6 here) exits 0 under -e when its input holds NO value at all (an empty
# or whitespace-only body, an empty pipe, an empty "$(…)" capture), so a bare `jq -e` passes
# exactly when the request failed. Every JSON assertion goes through one of these, which first
# require a non-blank byte:  jb FILTER… (the last body, $W/body) · jf FILE FILTER… (a file) ·
# jqs FILTER… (stdin: an SSE frame's data, a pipeline)
has_json() { [ -s "$1" ] && grep -q '[^[:space:]]' "$1" 2>/dev/null; }
jb() { has_json "$W/body" && jq -e "$@" "$W/body" >/dev/null 2>&1; }
jf() {
  local f=$1
  shift
  has_json "$f" && jq -e "$@" "$f" >/dev/null 2>&1
}
jqs() {
  local in
  in=$(cat)
  [ -n "${in//[[:space:]]/}" ] && printf '%s' "$in" | jq -e "$@" >/dev/null 2>&1
}
jget() { jq -r "$@" "$W/body" 2>/dev/null || true; }
snip() { head -c 300 "$W/body" 2>/dev/null | tr '\n\r' '  '; }
# t DESC CONDITION — CONDITION is eval'd at check time (pass it single-quoted)
t() {
  if eval "$2"; then ok "$1"; else fail "$1  [HTTP ${ST:-?}: $(snip)]"; fi
}
# csp_strict VALUE — the dashboard policy: same-origin only, no inline or eval script
csp_strict() {
  local c=$1
  [[ $c == *"default-src 'self'"* && $c == *"script-src 'self';"* &&
    $c == *"connect-src 'self'"* && $c == *"frame-ancestors 'none'"* &&
    $c == *"base-uri 'self'"* && $c != *unsafe-eval* ]]
}
problem_is() { # problem_is STATUS CODE — last response is that problem+json
  [ "$ST" = "$1" ] && ct_is application/problem+json && [ "$(jget .code)" = "$2" ]
}
# sse OUT SECONDS URL [curl args…] — capture a stream for SECONDS (timeout = normal end)
sse() {
  local out=$1 secs=$2 u=$3
  shift 3
  curl -sN --max-time "$secs" -D "$out.hdr" -o "$out" "$@" "$u" 2>/dev/null || true
}
# frame N FILE → the Nth SSE frame (blank-line separated), keep-alive comments skipped
frame() { awk -v n="$1" 'BEGIN{RS=""} /^:/{next} {i++} i==n{print; exit}' "$2"; }
field() { printf '%s\n' "$1" | sed -n "s/^$2: \{0,1\}//p" | head -1; }
# tape SRC NAME [ROOT] — save a captured stream as ui/test/tapes/NAME (only when
# missing or SMOKE_TAPES=1); the scratch root path is rewritten to the production root
TAPED=""
tape() {
  [ -s "$1" ] || return 0
  mkdir -p "$TAPES"
  if [ "${SMOKE_TAPES:-0}" = 1 ] || [ ! -s "$TAPES/$2" ]; then
    sed "s#${3:-$ROOT}#/var/lib/111#g" "$1" >"$TAPES/$2"
    TAPED="$TAPED $2"
  fi
}
events_in() { sed -n 's/^event: \{0,1\}//p' "$1" | tr -d '\r' | sort | uniq -c | awk '{printf "%s×%s ", $2, $1}'; }

# ------------------------------------------------------------------ inner (in the namespaces)

inner() {
  : "${SMOKE_WORK:?}" "${BIN111:?}"
  W=$SMOKE_WORK/http
  ROOT=$SMOKE_WORK/root
  mkdir -p "$W"
  # globals, not locals: the EXIT trap may run after inner() returned
  PROBE=""
  DPID=""
  cleanup_inner() {
    if [ -n "$PROBE" ]; then rm -f "$PROBE"; fi
    if [ -n "$DPID" ] && kill -0 "$DPID" 2>/dev/null; then
      kill -TERM "$DPID" 2>/dev/null || true
      for _ in 1 2 3 4 5 6 7 8 9 10; do
        kill -0 "$DPID" 2>/dev/null || break
        sleep 0.3
      done
    fi
    # leaving pid 1 of the namespace kills whatever is left (agent, stub model, curl)
    printf 'FAILS=%s\n' "$FAILS" >"$SMOKE_WORK/inner.result"
  }
  trap cleanup_inner EXIT
  trap 'exit 130' INT TERM

  phase "4. namespace + daemon"
  # util-linux unshare --fork starts its child with SIGINT+SIGTERM ignored, and
  # ignored dispositions survive exec — the daemon's children would then ignore
  # its SIGTERM. The outer resets them; prove it (SigIgn bits 1 and 14 clear).
  local sigign
  sigign=$(awk '/^SigIgn:/ {print $2}' /proc/self/status)
  t "namespace shell has default SIGINT/SIGTERM (SigIgn=$sigign)" '(( (16#$sigign & 0x4002) == 0 ))'
  ip link set lo up
  if ip link add dummy0 type dummy && ip addr add "$LANIP/24" dev dummy0 && ip link set dummy0 up; then
    ok "netns: lo up, dummy0 $LANIP/24 (no route off-box)"
  else
    fail "netns: cannot create dummy0 — LAN checks will fail"
  fi

  local d0=${EPOCHREALTIME/,/.}
  NODE111_ROOT=$ROOT "$BIN111" daemon >"$ROOT/daemon.log" 2>&1 &
  DPID=$!
  local up=0
  for _ in $(seq 1 300); do
    code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 2 "$B/health" 2>/dev/null) || true
    case $code in 200 | 503) up=1 && break ;; esac
    kill -0 "$DPID" 2>/dev/null || break
    sleep 0.2
  done
  if [ "$up" = 1 ]; then
    ok "111 daemon answers /health on :11180 (HTTP $code) $(awk -v a="$d0" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}')s after launch"
  else
    fail "111 daemon never answered /health"
    sed 's/^/  daemon| /' "$ROOT/daemon.log" | tail -30
    exit 1
  fi

  # the daemon spawns 111-agent + llama-server (stub) from current/bin; the relay
  # only dials while its mirror says `running` (2 s tick)
  local agent_ok=0
  for _ in $(seq 1 150); do
    req GET "$B/api/v1/agent/status"
    [ "$ST" = 200 ] && agent_ok=1 && break
    sleep 0.2
  done
  req GET "$B/api/v1/status"
  if [ "$agent_ok" = 1 ] && [ "$(jget .intelligence.agent)" = running ]; then
    ok "daemon spawned 111-agent itself (intelligence.agent=running; relay reaches it)"
  else
    fail "111-agent not reachable through the relay within 30 s (intelligence=$(jget -c .intelligence))"
    tail -20 "$ROOT/agent/111-agent.log" 2>/dev/null | sed 's/^/  agent| /'
  fi
  t "daemon spawned the stub model as llama-server (intelligence.model_server=running)" \
    '[ "$(jget .intelligence.model_server)" = running ]'

  # ---------------------------------------------------------------- static UI
  phase "5a. static UI + Host guard"
  req GET "$B/"
  t "GET / → 200 text/html, cache-control: no-cache, <div id=\"root\">" \
    '[ "$ST" = 200 ] && ct_is text/html && [ "$(hdr cache-control)" = no-cache ] && grep -q "<div id=\"root\">" "$W/body"'
  local csp
  csp=$(hdr content-security-policy)
  t "GET / carries a strict CSP (script-src 'self' only, connect-src 'self', frame-ancestors 'none')" \
    'csp_strict "$csp"'
  t "GET / carries X-Content-Type-Options: nosniff and Referrer-Policy: no-referrer" \
    '[ "$(hdr x-content-type-options)" = nosniff ] && [ "$(hdr referrer-policy)" = no-referrer ]'
  local served_sha dist_sha
  served_sha=$(sha256sum <"$W/body" | cut -c1-64)
  dist_sha=$(sha256sum <"$UI/dist/index.html" | cut -c1-64)
  t "GET / serves the current dist/index.html byte-for-byte (title $(grep -o '<title>[^<]*</title>' "$W/body" | head -1))" \
    '[ "$served_sha" = "$dist_sha" ]'
  local asset
  asset=$(grep -o 'assets/[^"]*\.js' "$W/body" | head -1 || true)
  if [ -n "$asset" ]; then
    req GET "$B/$asset"
    t "GET /$asset → 200 text/javascript, cache-control: public, max-age=31536000, immutable" \
      '[ "$ST" = 200 ] && ct_is text/javascript && [ "$(hdr cache-control)" = "public, max-age=31536000, immutable" ]'
    t "the served asset is the dist file" \
      '[ "$(sha256sum <"$W/body" | cut -c1-64)" = "$(sha256sum <"$UI/dist/$asset" | cut -c1-64)" ]'
  else
    fail "index.html references no assets/*.js"
  fi
  PROBE=$UI/dist/smoke-probe-$RANDOM$RANDOM.txt
  printf 'probe %s\n' "$RANDOM" >"$PROBE"
  req GET "$B/$(basename "$PROBE")"
  t "debug 111 reads ui/dist from disk per request (a file written after start is served → no Rust rebuild after pnpm build)" \
    '[ "$ST" = 200 ] && cmp -s "$W/body" "$PROBE"'
  rm -f "$PROBE"
  PROBE=""
  req GET "$B/v1"
  t "GET /v1 → 200 legacy page containing \"111 node\"" '[ "$ST" = 200 ] && grep -q "111 node" "$W/body"'
  req GET "$B/fonts/GeistMono-Variable.woff2"
  t "GET /fonts/GeistMono-Variable.woff2 → 200 font/woff2" '[ "$ST" = 200 ] && ct_is font/woff2'
  req GET "$B/api/v1/nope"
  t "unknown /api/v1 route → 404 problem+json" '[ "$ST" = 404 ] && ct_is application/problem+json'
  req POST "$B/api/v1/proposals/not-a-ulid/decide" -H 'Content-Type: application/json' -d '{}'
  t "POST to an unknown /api/v1 path → 404 problem+json (not the static catch-all's 405)" 'problem_is 404 not_found'
  req GET "$B/api/v1/services/%FF"
  t "bad path parameter (not UTF-8) → 400 problem+json with the reason in detail" \
    'problem_is 400 bad_request && jb ".detail | length > 0"'
  req GET "$B/api/v1/node/restart"
  t "wrong method on an /api/v1 route → 405 problem+json" 'problem_is 405 bad_request'
  req GET "$B/" -H 'Host: evil.com'
  t "Host: evil.com → 400 problem+json" 'problem_is 400 bad_request'
  req GET "$B/" -H 'Host: 111.localhost'
  t "Host: 111.localhost → 200" '[ "$ST" = 200 ]'
  req GET "$B/" -H 'Host: 111.local'
  t "Host: 111.local → 200" '[ "$ST" = 200 ]'
  req GET "$B/" -H 'Host: 127.0.0.1.evil.com'
  t "Host: 127.0.0.1.evil.com → 400" '[ "$ST" = 400 ]'

  # ---------------------------------------------------------------- status + reads
  phase "5b. /api/v1 reads (loopback)"
  req GET "$B/api/v1/status"
  t "GET /api/v1/status → 200 application/json" '[ "$ST" = 200 ] && ct_is application/json'
  local missing
  missing=$(jget -c '(["schema","node","transport_state","health","update","intelligence","bind","lan_enabled","auth","layers"] - keys) + ((["node_id","label","bech32","alias","category","version","uptime_sec"] - (.node|keys)) | map("node."+.)) + ((["state","pending","last_check","clock_fault"] - (.update|keys)) | map("update."+.)) + ((["urls","lan"] - (.bind|keys)) | map("bind."+.))')
  t "status carries every required key (Status schema)  missing=$missing" '[ "$missing" = "[]" ]'
  t "status.auth = {loopback:true, paired:false} from loopback" '[ "$(jget -c .auth)" = "{\"loopback\":true,\"paired\":false}" ]'
  t "status.node.node_id = the generated identity ($SMOKE_NODE_ID)" '[ "$(jget .node.node_id)" = "${SMOKE_NODE_ID:-x}" ]'
  t "status.node.label + bech32 present (label $(jget .node.label))" \
    'jb ".node.label | test(\"^[a-z0-9]{4}-[a-z0-9]{4}-[a-z0-9]{2}$\")" && jb ".node.bech32 | startswith(\"node1\")"'
  t "status.lan_enabled = true and bind.lan = true (DASHBOARD_LAN=true)" 'jb ".lan_enabled == true and .bind.lan == true"'
  t "status.bind.urls non-empty and has a LAN URL: $(jget -c .bind.urls)" \
    'jb "(.bind.urls | length > 0) and (.bind.urls | any(startswith(\"http://111.local\") or startswith(\"http://$LANIP\")))"'
  t "status.transport_state is a contract value ($(jget .transport_state))" \
    'jb ".transport_state | IN(\"no transport\", \"isolated\", \"joined\", \"transport degraded\")"'
  t "status.layers is an int array ($(jget -c .layers))" 'jb ".layers | type == \"array\" and all(type == \"number\")"'
  local port80=0
  if jb '.bind.urls | index("http://111.localhost")'; then
    port80=1
    req GET "http://127.0.0.1/" -H 'Host: 111.localhost'
    t ":80 bound (root in the userns) → http://111.localhost/ serves the UI" '[ "$ST" = 200 ] && ct_is text/html'
  else
    note ":80 did not bind in this namespace — http://111.localhost not exercised"
  fi

  req GET "$B/api/status"
  t "Layer 1 /api/status still 200 (additive seam unchanged)" '[ "$ST" = 200 ] && jb ".schema == 1 and has(\"intelligence\")"'
  req GET "$B/api/v1/auth/session"
  t "GET /api/v1/auth/session from loopback → loopback:true, loopback_trusted:true (not Termux), device_id:null" \
    '[ "$ST" = 200 ] && jb ".loopback == true and .loopback_trusted == true and .device_id == null"'

  req GET "$B/api/v1/services"
  t "GET /api/v1/services → 9 records, ids agent,codeagent,fabric,gateway,kb,meshtastic,model,radio,transport (Layer 5 grew the registry)" \
    '[ "$ST" = 200 ] && [ "$(jget -c "[.services[].id] | sort")" = "[\"agent\",\"codeagent\",\"fabric\",\"gateway\",\"kb\",\"meshtastic\",\"model\",\"radio\",\"transport\"]" ]'
  t "every service record has the required keys" \
    'jb "all(.services[]; ([\"id\",\"name\",\"plain_name\",\"kind\",\"state\",\"since\",\"reason_code\",\"reason\",\"can_run_here\",\"description\",\"actions\",\"detail\"] - keys) == [])"'
  note "service states: $(jget -r '[.services[] | "\(.id)=\(.state)"] | join(" ")')"
  req GET "$B/api/v1/services/agent"
  t "GET /api/v1/services/agent → 200 record" '[ "$ST" = 200 ] && [ "$(jget .id)" = agent ]'
  req GET "$B/api/v1/services/nope"
  t "GET /api/v1/services/nope → 4xx problem+json" 'case $ST in 4*) ct_is application/problem+json ;; *) false ;; esac'
  req GET "$B/api/v1/services/agent/log?n=5"
  t "GET /api/v1/services/agent/log?n=5 → LogTail ($(jget '.lines|length') lines from $(jget .source))" \
    '[ "$ST" = 200 ] && jb ".service == \"agent\" and (.lines | type == \"array\") and (.lines | length <= 5)"'
  req GET "$B/api/v1/devices"
  t "GET /api/v1/devices → 200 {devices:[…]} ($(jget '.devices|length') records)" '[ "$ST" = 200 ] && jb ".devices | type == \"array\""'
  req GET "$B/api/v1/settings"
  t "GET /api/v1/settings → 200 {schema, alias, lan_enabled, offer}" \
    '[ "$ST" = 200 ] && jb "([\"schema\",\"alias\",\"lan_enabled\",\"offer\"] - keys) == []"'
  req GET "$B/api/v1/kb"
  t "GET /api/v1/kb → 200 text/plain = share/kb/llms.txt" \
    '[ "$ST" = 200 ] && ct_is text/plain && cmp -s "$W/body" "$ROOT/current/share/kb/llms.txt"'
  req GET "$B/api/v1/kb/entry?path=models/qwen3-4b.md"
  t "GET /api/v1/kb/entry?path=models/qwen3-4b.md → 200 markdown" '[ "$ST" = 200 ] && ct_is text/markdown'
  req GET "$B/api/v1/kb/entry?path=../etc/passwd"
  t "GET /api/v1/kb/entry?path=../etc/passwd → 400 (traversal refused)" 'problem_is 400 bad_request'
  req GET "$B/api/v1/kb/entry?path=/etc/passwd"
  t "GET /api/v1/kb/entry?path=/etc/passwd → 400 (absolute refused)" '[ "$ST" = 400 ]'

  # ---------------------------------------------------------------- events + settings
  phase "5c. events: reset, then Last-Event-ID replay"
  sse "$W/events.sse" 5 "$B/api/v1/events"
  local f1 id1
  f1=$(frame 1 "$W/events.sse")
  id1=$(field "$f1" id)
  t "GET /api/v1/events (fresh) → text/event-stream, first frame event: reset with an id ($(events_in "$W/events.sse"))" \
    'grep -qi "^content-type: text/event-stream" "$W/events.sse.hdr" && [ "$(field "$f1" event)" = reset ] && [[ "$id1" =~ ^[0-9]+$ ]]'
  t "reset frame data = {\"reason\":\"expired\"} (fresh client)" '[ "$(field "$f1" data)" = "{\"reason\":\"expired\"}" ]'

  req PATCH "$B/api/v1/settings" -H 'Content-Type: application/json' -d '{"alias":"smoke"}'
  t "PATCH /api/v1/settings {alias:smoke} → 200 alias=smoke" '[ "$ST" = 200 ] && [ "$(jget .alias)" = smoke ]'
  req GET "$B/api/v1/status"
  t "status.node.alias = smoke after the PATCH" '[ "$(jget .node.alias)" = smoke ]'
  req PATCH "$B/api/v1/settings" -H 'Content-Type: application/json' -d '{"alias":"x"}' -H 'Origin: http://evil.com'
  t "loopback PATCH with a foreign Origin → 403 (CSRF applies to loopback too)" '[ "$ST" = 403 ] && ct_is application/problem+json'

  sse "$W/events-replay.sse" 3 "$B/api/v1/events" -H "Last-Event-ID: ${id1:-0}"
  local rf rid
  rf=$(frame 1 "$W/events-replay.sse")
  rid=$(field "$rf" id)
  t "Last-Event-ID: $id1 → no reset frame, replay starts with id $rid > $id1 ($(events_in "$W/events-replay.sse"))" \
    '! grep -q "^event: reset" "$W/events-replay.sse" && [[ "$rid" =~ ^[0-9]+$ ]] && [ "$rid" -gt "${id1:-0}" ]'
  t "the replay carries the alias change as a status frame" \
    'grep "^data: " "$W/events-replay.sse" | sed "s/^data: //" | jqs "select(.node.alias? == \"smoke\")"'
  sse "$W/events-stale.sse" 2 "$B/api/v1/events" -H "Last-Event-ID: 5"
  t "Last-Event-ID from another daemon lifetime → reset {reason:restart}" \
    '[ "$(field "$(frame 1 "$W/events-stale.sse")" data)" = "{\"reason\":\"restart\"}" ]'
  tape "$W/events.sse" events.sse
  tape "$W/events-replay.sse" events-replay.sse

  # ---------------------------------------------------------------- LAN pairing
  phase "5d. auth from a real non-loopback source ($LANIP on dummy0)"
  local lan=(--interface "$LANIP")
  local jar=$W/lan.jar
  req GET "$L/api/v1/status" "${lan[@]}"
  t "LAN GET /api/v1/status without a session → 401 problem+json unauthenticated" 'problem_is 401 unauthenticated'
  REQ_TIMEOUT=3 req GET "$L/api/v1/events" "${lan[@]}"
  t "LAN GET /api/v1/events without a session → 401" '[ "$ST" = 401 ]'
  req GET "$L/" "${lan[@]}"
  t "LAN GET / (static) still serves without a session (SPA shows /pair)" '[ "$ST" = 200 ] && ct_is text/html'
  req GET "$L/api/status" "${lan[@]}"
  t "LAN GET /api/status (Layer 1 seam) without a session → 401 problem+json" 'problem_is 401 unauthenticated'
  req GET "$L/v1" "${lan[@]}"
  t "LAN GET /v1 (minimal page) without a session → 401 problem+json" 'problem_is 401 unauthenticated'
  req GET "$L/health" "${lan[@]}"
  t "LAN GET /health stays open (200|503)" 'case $ST in 200 | 503) true ;; *) false ;; esac'
  req GET "$L/api/v1/auth/session" "${lan[@]}"
  t "LAN GET /api/v1/auth/session → {loopback:false, paired:false, lan_enabled:true}" \
    '[ "$ST" = 200 ] && jb ".loopback == false and .paired == false and .lan_enabled == true"'
  req POST "$L/api/v1/auth/pair" "${lan[@]}" -H 'Content-Type: application/json' -d '{"code":"WRONG-ONE"}'
  t "LAN POST /auth/pair with a wrong code → 401" 'problem_is 401 unauthenticated'

  local cli code filecode
  cli=$(NODE111_ROOT=$ROOT "$BIN111" dashboard 2>&1) || true
  printf '%s\n' "$cli" >"$W/cli.out"
  code=$(printf '%s\n' "$cli" | sed -n 's/^Pair code: *\([0-9A-Z]\{4\}-[0-9A-Z]\{4\}\).*/\1/p' | head -1)
  filecode=$(jq -r .code "$ROOT/state/dashboard-pair.json" 2>/dev/null || true)
  t "\`111 dashboard\` prints the bound URLs + LAN state + pair code ($code)" \
    'grep -q "^  http://" "$W/cli.out" && grep -q "LAN sharing: on" "$W/cli.out" && [ -n "$code" ]'
  t "\`111 dashboard\` pair code = state/dashboard-pair.json" '[ -n "$code" ] && [ "$code" = "$filecode" ]'
  t "\`111 dashboard\` prints a QR pair link (LAN on)" '[ -n "$code" ] && grep -q "#/pair?code=$code" "$W/cli.out"'
  [ -n "$code" ] || code=$filecode

  req POST "$L/api/v1/auth/pair" "${lan[@]}" -c "$jar" -H 'Content-Type: application/json' -d "{\"code\":\"$code\"}"
  local cookie
  cookie=$(hdr set-cookie)
  t "LAN POST /auth/pair with the code → 200 paired, loopback:false" '[ "$ST" = 200 ] && jb ".paired == true and .loopback == false"'
  t "set-cookie: s111=<64 hex>; HttpOnly; SameSite=Strict; Path=/; Max-Age=15552000" \
    '[[ "$cookie" =~ ^s111=[0-9a-f]{64}\;\ HttpOnly\;\ SameSite=Strict\;\ Path=/\;\ Max-Age=15552000$ ]]'
  local token
  token=$(awk '$6 == "s111" {print $7}' "$jar" 2>/dev/null | tail -1)
  req GET "$L/api/v1/status" "${lan[@]}" -b "$jar"
  t "LAN GET /api/v1/status with the cookie → 200, auth = {loopback:false, paired:true}" \
    '[ "$ST" = 200 ] && [ "$(jget -c .auth)" = "{\"loopback\":false,\"paired\":true}" ]'
  req GET "$L/api/v1/services" "${lan[@]}" -b "$jar"
  t "LAN GET /api/v1/services with the cookie → 200" '[ "$ST" = 200 ]'
  req GET "$L/api/status" "${lan[@]}" -b "$jar"
  t "LAN GET /api/status with the cookie → 200" '[ "$ST" = 200 ] && jb ".schema == 1"'
  req PATCH "$L/api/v1/settings" "${lan[@]}" -b "$jar" -H 'Content-Type: application/json' -d '{"lan_enabled":false}'
  t "LAN PATCH /settings {lan_enabled:false} → 403 loopback_only" 'problem_is 403 loopback_only'
  req PATCH "$L/api/v1/settings" "${lan[@]}" -b "$jar" -H 'Content-Type: application/json' -d '{"offer":"accepted"}'
  t "LAN PATCH /settings {offer:accepted} → 403 loopback_only" 'problem_is 403 loopback_only'
  req POST "$L/api/v1/update/check" "${lan[@]}" -b "$jar" -H 'Content-Type: text/plain' -d '{}'
  t "CSRF: LAN POST without a JSON content-type → 415 problem+json" '[ "$ST" = 415 ] && ct_is application/problem+json'
  req POST "$L/api/v1/update/check" "${lan[@]}" -b "$jar" -H 'Content-Type: application/json' -H 'Origin: http://evil.com' -d '{}'
  t "CSRF: LAN POST with a foreign Origin → 403" '[ "$ST" = 403 ]'
  sse "$W/lan-events.sse" 2 "$L/api/v1/events" "${lan[@]}" -b "$jar"
  t "LAN paired GET /api/v1/events → stream opens (first frame reset)" \
    '[ "$(field "$(frame 1 "$W/lan-events.sse")" event)" = reset ]'

  local comp=""
  for _ in $(seq 1 40); do
    req GET "$B/api/v1/devices"
    comp=$(jget -r '[.devices[] | select(.kind == "companion")] | first | "\(.id) \(.state)"' 2>/dev/null)
    [ -n "$comp" ] && [ "$comp" != "null null" ] && break
    comp=""
    sleep 0.5
  done
  t "the paired browser appears in /api/v1/devices as a companion ($comp)" '[ -n "$comp" ]'
  req GET "$L/api/v1/auth/session" "${lan[@]}" -b "$jar"
  t "LAN GET /auth/session names that companion as this browser (device_id $(jget .device_id))" \
    '[ -n "$comp" ] && [ "$ST" = 200 ] && [ "$(jget .device_id)" = "${comp%% *}" ]'

  req POST "$L/api/v1/auth/logout" "${lan[@]}" -b "$jar" -c "$jar" -H 'Content-Type: application/json' -d '{}'
  t "LAN POST /auth/logout → 204" '[ "$ST" = 204 ]'
  req GET "$L/api/v1/status" "${lan[@]}" -H "Cookie: s111=${token:-none}"
  t "the old cookie is revoked server-side → 401" '[ -n "$token" ] && problem_is 401 unauthenticated'

  # ---------------------------------------------------------------- chat relay
  phase "5e. chat relay (stub model through the real 111-agent)"
  local cj='Content-Type: application/json'
  curl -sN --max-time 60 -D "$W/chat.sse.hdr" -o "$W/chat.sse" -H "$cj" \
    -d '{"conversation_id":"conv-smoke-1","message":"hello"}' "$B/api/v1/chat" || true
  local done1 toks
  done1=$(awk 'BEGIN{RS=""} /event: done/' "$W/chat.sse" | sed -n 's/^data: //p' | head -1)
  toks=$(awk 'BEGIN{RS=""} /event: token/' "$W/chat.sse" | sed -n 's/^data: //p' | jq -rj .text 2>/dev/null || true)
  t "POST /api/v1/chat (stream) → text/event-stream with event: token … event: done ($(events_in "$W/chat.sse"))" \
    'grep -qi "^content-type: text/event-stream" "$W/chat.sse.hdr" && grep -q "^event: token" "$W/chat.sse" && grep -q "^event: done" "$W/chat.sse"'
  t "done = {conversation_id:conv-smoke-1, reply = concatenated tokens (\"$toks\"), tool_trace:[]}" \
    'printf "%s" "$done1" | jqs --arg r "$toks" ".conversation_id == \"conv-smoke-1\" and .reply == \$r and .tool_trace == []"'
  tape "$W/chat.sse" chat.sse

  curl -sN --max-time 60 -o "$W/chat-tool.sse" -H "$cj" \
    -d '{"conversation_id":"conv-smoke-2","message":"what is meshtastic?"}' "$B/api/v1/chat" || true
  t "tool turn → event: tool {name:kb.read} then done with tool_trace ($(events_in "$W/chat-tool.sse"))" \
    'awk "BEGIN{RS=\"\"} /event: tool/" "$W/chat-tool.sse" | grep -q "\"name\":\"kb.read\"" && sed -n "/^event: done/,/^$/p" "$W/chat-tool.sse" | sed -n "s/^data: //p" | jqs ".tool_trace | any(.name == \"kb.read\")"'
  tape "$W/chat-tool.sse" chat-tool.sse

  req POST "$B/api/v1/chat" -H "$cj" -d '{"conversation_id":"conv-smoke-1","message":"hello again","stream":false}' --max-time 60
  t "POST /api/v1/chat stream:false → JSON ChatReply" \
    '[ "$ST" = 200 ] && ct_is application/json && jb ".conversation_id == \"conv-smoke-1\" and (.reply | length > 0) and (.tool_trace | type == \"array\")"'

  curl -sN --max-time 60 -D "$W/chat-err.sse.hdr" -o "$W/chat-err.sse" -H "$cj" \
    -d '{"conversation_id":"conv-smoke-3","message":"boom"}' "$B/api/v1/chat" || true
  t "model failure → event: error (mid-stream) or 5xx problem+json (pre-stream)" \
    'grep -q "^event: error" "$W/chat-err.sse" || { grep -qi "^content-type: application/problem+json" "$W/chat-err.sse.hdr" && head -1 "$W/chat-err.sse.hdr" | grep -q " 5"; }'
  note "model failure surfaced as: $(head -1 "$W/chat-err.sse.hdr" | tr -d '\r') $(head -c 160 "$W/chat-err.sse" | tr '\n' ' ')"
  tape "$W/chat-err.sse" chat-error.sse

  req POST "$B/api/v1/chat" -H "$cj" -d '{"conversation_id":"bad id!","message":"x"}'
  t "invalid conversation_id → 400 problem+json" 'problem_is 400 bad_request'
  req GET "$B/api/v1/chat/history?conversation_id=conv-smoke-1"
  t "GET /api/v1/chat/history → messages include user \"hello\" + assistant \"Hello there\" ($(jget '.messages|length') msgs)" \
    '[ "$ST" = 200 ] && jb "(.messages | any(.role == \"user\" and .content == \"hello\")) and (.messages | any(.role == \"assistant\" and .content == \"Hello there\"))"'
  req GET "$B/api/v1/agent/status"
  t "GET /api/v1/agent/status → state in local|offloaded|unavailable ($(jget .state), model reachable=$(jget .model.reachable))" \
    '[ "$ST" = 200 ] && jb ".state | IN(\"local\", \"offloaded\", \"unavailable\")"'

  # ---------------------------------------------------------------- proposals + task
  phase "5f. proposals, decisions, agent task → proposal frames"
  local pdir=$ROOT/agent/proposals now
  now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  mkdir -p "$pdir"
  jq -n --arg now "$now" --arg nid "${SMOKE_NODE_ID:-}" \
    '{schema:1, id:"prop-smoke", node_id:$nid, task_id:null, kind:"propose", title:"Smoke proposal", body:"## Smoke\nwritten by 40-interface/verify/smoke.sh", created_at:$now}' \
    >"$pdir/prop-smoke.json"
  req GET "$B/api/v1/proposals"
  t "GET /api/v1/proposals → includes prop-smoke (open, decision:null)" \
    '[ "$ST" = 200 ] && jb ".proposals | any(.id == \"prop-smoke\" and .decision == null)"'
  req GET "$B/api/v1/proposals/prop-smoke"
  t "GET /api/v1/proposals/prop-smoke → 200 Proposal" '[ "$ST" = 200 ] && jb ".id == \"prop-smoke\" and .title == \"Smoke proposal\""'
  req POST "$B/api/v1/proposals/prop-smoke/decision" -H "$cj" -d '{"decision":"accepted"}'
  t "POST /proposals/prop-smoke/decision {accepted} → 200 decided" '[ "$ST" = 200 ] && jb ".decision.decision == \"accepted\""'
  t "decision recorded in agent/decisions/prop-smoke.json" 'jf "$ROOT/agent/decisions/prop-smoke.json" ".decision == \"accepted\""'
  req POST "$B/api/v1/proposals/prop-smoke/decision" -H "$cj" -d '{"decision":"dismissed"}'
  t "deciding again → 409 already_decided" 'problem_is 409 already_decided'
  req GET "$B/api/v1/proposals?status=decided"
  t "GET /api/v1/proposals?status=decided → includes prop-smoke" 'jb ".proposals | any(.id == \"prop-smoke\")"'
  req GET "$B/api/v1/proposals/..%2Fetc"
  t "GET /api/v1/proposals/<bad id> → 404" '[ "$ST" = 404 ]'

  # background capture while the agent runs a task: agent + proposal frames
  curl -sN --max-time 45 -o "$W/events-activity.sse" "$B/api/v1/events" -H "Last-Event-ID: ${rid:-${id1:-0}}" 2>/dev/null &
  local cap=$!
  sleep 0.5
  req POST "$B/api/v1/agent/task" -H "$cj" -d '{"kind":"diagnose","note":"smoke"}'
  local task_id
  task_id=$(jget .task_id)
  t "POST /api/v1/agent/task {diagnose} → 202 task_id ($task_id)" '[ "$ST" = 202 ] && [ -n "$task_id" ] && [ "$task_id" != null ]'
  local prop=""
  for _ in $(seq 1 60); do
    req GET "$B/api/v1/proposals"
    prop=$(jget -r --arg t "$task_id" '[.proposals[] | select(.task_id == $t)] | first | .id // empty')
    [ -n "$prop" ] && break
    sleep 0.5
  done
  t "the task's proposal shows up in /api/v1/proposals ($prop)" '[ -n "$prop" ]'
  for _ in $(seq 1 20); do
    grep -q "\"id\":\"${prop:-none}\"" "$W/events-activity.sse" 2>/dev/null && break
    sleep 0.5
  done
  kill "$cap" 2>/dev/null || true
  wait "$cap" 2>/dev/null || true
  t "events stream republished the agent's frames (event: agent) ($(events_in "$W/events-activity.sse"))" \
    'grep -q "^event: agent" "$W/events-activity.sse"'
  t "events stream carried event: proposal for $prop" \
    'awk "BEGIN{RS=\"\"} /event: proposal/" "$W/events-activity.sse" | grep -q "\"id\":\"${prop:-none}\""'
  tape "$W/events-activity.sse" events-activity.sse
  if [ -n "$TAPED" ]; then
    note "tapes recorded into ${TAPES#"$OS"/}:$TAPED"
  else
    note "tapes in ui/test/tapes already present — kept (SMOKE_TAPES=1 re-records)"
  fi

  # ---------------------------------------------------------------- per-service restart
  phase "5g. per-service restart (supervised child vs not)"
  req GET "$B/api/v1/services/model"
  local mpid0 mstate0
  mpid0=$(jget .detail.pid)
  mstate0=$(jget .state)
  t "model is a supervised child: actions has restart ($(jget -c .actions), state $mstate0, pid $mpid0)" \
    'jb ".kind == \"child\" and (.actions | index(\"restart\"))" && [[ "$mpid0" =~ ^[0-9]+$ ]]'
  req POST "$B/api/v1/services/model/restart" -H 'Content-Type: application/json' -d '{}'
  t "POST /api/v1/services/model/restart → 202 {schema:1, id:model, restarting:true}" \
    '[ "$ST" = 202 ] && jb ". == {schema: 1, id: \"model\", restarting: true}"'
  local mpid1="" mstate1=""
  for _ in $(seq 1 100); do
    req GET "$B/api/v1/services/model"
    mpid1=$(jget .detail.pid)
    mstate1=$(jget .state)
    [[ "$mpid1" =~ ^[0-9]+$ ]] && [ "$mpid1" != "$mpid0" ] && [ "$mstate1" = live ] && break
    sleep 0.3
  done
  t "model comes back LIVE with a new pid ($mpid0 → $mpid1, state $mstate1)" \
    '[[ "$mpid1" =~ ^[0-9]+$ ]] && [ "$mpid1" != "$mpid0" ] && [ "$mstate1" = live ]'
  t "the old llama-server process is gone" '[[ "$mpid0" =~ ^[0-9]+$ ]] && ! kill -0 "$mpid0" 2>/dev/null'
  req GET "$B/api/v1/services/transport"
  t "transport (detection-only stub here) offers no restart ($(jget .reason_code), actions $(jget -c .actions))" \
    '[ "$ST" = 200 ] && jb "(.actions | type) == \"array\" and (.actions | index(\"restart\")) == null"'
  req POST "$B/api/v1/services/transport/restart" -H 'Content-Type: application/json' -d '{}'
  t "POST /api/v1/services/transport/restart → 409 not_supervised" 'problem_is 409 not_supervised'
  req POST "$B/api/v1/services/kb/restart" -H 'Content-Type: application/json' -d '{}'
  t "POST /api/v1/services/kb/restart → 409 not_supervised" 'problem_is 409 not_supervised'
  req POST "$B/api/v1/services/nope/restart" -H 'Content-Type: application/json' -d '{}'
  t "POST /api/v1/services/nope/restart → 404 not_found" 'problem_is 404 not_found'
  req POST "$L/api/v1/services/model/restart" --interface "$LANIP" -H 'Content-Type: application/json' -d '{}'
  t "LAN POST /services/model/restart without a session → 401" 'problem_is 401 unauthenticated'

  # ---------------------------------------------------------------- actions
  phase "5h. update check + node restart"
  req POST "$B/api/v1/update/check" -H "$cj" -d '{}'
  t "POST /api/v1/update/check → 202 {started:true}" '[ "$ST" = 202 ] && jb ".started == true"'
  local before
  before=$(pgrep -l -x '111-agent|llama-server|python3' | tr '\n' ' ' || true)
  req POST "$B/api/v1/node/restart" -H "$cj" -d '{}'
  t "POST /api/v1/node/restart → 202 {restarting:true}" '[ "$ST" = 202 ] && jb ".restarting == true"'
  local gone=0 rc=255
  for _ in $(seq 1 25); do
    kill -0 "$DPID" 2>/dev/null || {
      gone=1
      break
    }
    sleep 0.2
  done
  if [ "$gone" = 1 ]; then
    wait "$DPID" && rc=0 || rc=$?
    DPID=""
  fi
  t "daemon exits 0 within 5 s of the restart reply (exit $rc)" '[ "$gone" = 1 ] && [ "$rc" = 0 ]'
  local left=""
  for _ in $(seq 1 25); do
    left=$(pgrep -l -x '111-agent|llama-server|python3' | tr '\n' ' ' || true)
    [ -z "$left" ] && break
    sleep 0.2
  done
  t "the daemon stopped its children on exit (before: ${before% }; left: ${left:-none})" '[ -n "$before" ] && [ -z "$left" ]'
  note "daemon log tail:"
  tail -8 "$ROOT/daemon.log" | sed 's/^/  daemon| /'
}

# ------------------------------------------------------------------ three-node helpers (inner)

N3_NODES=(H A B C)
declare -A HOLD=() DPID=() NID=() NROOT=() NIP=([H]=10.111.0.10 [A]=10.111.0.11 [B]=10.111.0.12 [C]=10.111.0.13)
HELPERS=()
R1PID=""
R2PID=""
DAPI=http://127.0.0.1:11180

# nx NODE CMD… — run CMD inside NODE's net namespace
nx() {
  local n=$1
  shift
  nsenter -t "${HOLD[$n]}" -n -- "$@"
}
# bg_in NODE LOG CMD… — a background helper inside NODE's netns; $BGPID is its real pid
# (nsenter execs CMD, so the pid is the helper's own — never a subshell's), added to HELPERS
bg_in() {
  local n=$1 log=$2
  shift 2
  nsenter -t "${HOLD[$n]}" -n -- "$@" >"$log" 2>&1 &
  BGPID=$!
  HELPERS+=("$BGPID")
}
# api NODE METHOD PATH [curl args…] — req() against NODE's own loopback /api/v1
api() {
  local n=$1 m=$2 p=$3
  shift 3
  NSX=(nsenter -t "${HOLD[$n]}" -n --)
  req "$m" "$DAPI$p" "$@"
  NSX=()
}
jsend() { # jsend NODE METHOD PATH JSON — a JSON write (CSRF: application/json from loopback)
  local n=$1 m=$2 p=$3 d=$4
  api "$n" "$m" "$p" -H 'Content-Type: application/json' -d "$d"
}
# wait_for SECONDS CONDITION — poll the eval'd CONDITION every 0.4 s
wait_for() {
  local end=$((SECONDS + $1))
  while :; do
    if eval "$2"; then return 0; fi
    [ "$SECONDS" -ge "$end" ] && return 1
    sleep 0.4
  done
}
# send FROM TO TEXT → $MID (the 202's MessageId, "" when refused; $ST the status)
send() {
  jsend "$1" POST /api/v1/messages "$(jq -nc --arg to "${NID[$2]}" --arg text "$3" '{to: $to, text: $text}')"
  MID=$(jget .id)
  [ "$ST" = 202 ] || MID=""
}
# msg_get NODE PEER ID → that message of NODE's thread with PEER (compact JSON, text cut
# to its length: text_len), or nothing
msg_get() {
  api "$1" GET "/api/v1/messages/threads/${NID[$2]}?limit=${4:-60}"
  jget -c --arg id "$3" '(.messages // [])[] | select(.id == $id) | .text_len = (.text | length) | .text = .text[0:80]'
}
# wait_msg NODE PEER ID JQCOND SECONDS → 0 once the message satisfies JQCOND; $M = last seen
wait_msg() {
  local end=$((SECONDS + $5))
  while :; do
    M=$(msg_get "$1" "$2" "$3")
    if [ -n "$M" ] && printf '%s' "$M" | jq -e "$4" >/dev/null 2>&1; then return 0; fi
    [ "$SECONDS" -ge "$end" ] && return 1
    sleep 0.3
  done
}
mfield() { printf '%s' "${M:-null}" | jq -r "$1" 2>/dev/null || true; }
# fab_diag NODE… → one note per node with its fabric view (hub, links, custody, counters);
# for a failed wait, so the log says what each node saw instead of only "not delivered"
fab_diag() {
  local n
  for n in "$@"; do
    api "$n" GET /api/v1/fabric
    note "diag $n fabric: $(jget -c '{hub: {enrolled: .hub.enrolled, reachable: .hub.reachable, cursor: .hub.cursor, members: .hub.members}, links: [.links[]? | {id, state, peers}], custody: .custody, counters: .counters}')"
  done
}
# count_text NODE PEER TEXT → how many messages in NODE's thread with PEER carry TEXT
# (404, no thread yet, counts 0; any other failed read prints nothing — never a number)
count_text() {
  api "$1" GET "/api/v1/messages/threads/${NID[$2]}?limit=500"
  case $ST in 200) ;; 404) echo 0 && return 0 ;; *) return 0 ;; esac
  jget --arg t "$3" 'if (.messages | type) == "array" then ([.messages[] | select(.text == $t)] | length) else empty end'
}
# thread_len NODE PEER → messages in NODE's thread with PEER (same rule as count_text)
thread_len() {
  api "$1" GET "/api/v1/messages/threads/${NID[$2]}?limit=500"
  case $ST in 200) ;; 404) echo 0 && return 0 ;; *) return 0 ;; esac
  jget 'if (.messages | type) == "array" then (.messages | length) else empty end'
}
# peers_of NODE → attributed tcp-lan peers (GET /fabric links[].peers); nothing unless the
# read answered 200 with a links array (a failed read must not count as "0 peers")
peers_of() {
  api "$1" GET /api/v1/fabric
  [ "$ST" = 200 ] || return 0
  jget 'if (.links | type) == "array" then ([.links[] | select(.id == "tcp-lan") | .peers] | add // 0) else empty end'
}
# hello_at IP PORT → 0 when a tcp-lan listener answers with "111F" 0x01
hello_at() {
  python3 -I -c 'import socket, sys
try:
    s = socket.create_connection((sys.argv[1], int(sys.argv[2])), 3)
    s.settimeout(3)
    b = s.recv(5)
except OSError:
    b = b""
sys.exit(0 if b == b"111F\x01" else 1)' "$1" "$2"
}
# sse_on NODE FILE → background capture of NODE's /api/v1/events into FILE ($SSEP = pid)
sse_on() {
  nsenter -t "${HOLD[$1]}" -n -- curl -sN --max-time 900 -o "$2" "$DAPI/api/v1/events" 2>/dev/null &
  SSEP=$!
}
sse_off() {
  kill "$1" 2>/dev/null || true
  wait "$1" 2>/dev/null || true
}
# sse_data FILE EVENT → the data of every frame named EVENT, in order (one per line)
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
# mframes FILE ID → "<change> <state>" for every `message` frame about message ID
mframes() { sse_data "$1" message | jq -r --arg id "$2" 'select(.message.id == $id) | "\(.change) \(.message.state)"' 2>/dev/null || true; }
# chat NODE CONV MESSAGE → $W/chat-CONV.sse (stream); $DONE = the done frame's data
chat() {
  nx "$1" curl -sN --max-time 60 -o "$W/chat-$2.sse" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg c "$2" --arg m "$3" '{conversation_id: $c, message: $m}')" "$DAPI/api/v1/chat" 2>/dev/null || true
  DONE=$(sse_data "$W/chat-$2.sse" "done" | head -1)
}
# toolres NODE TOOL → the newest result 111-agent handed the stub model for TOOL
toolres() { jq -rs --arg n "$2" '[.[] | select(.name == $n)] | last | .content // ""' "${NROOT[$1]}/stub-tools.log" 2>/dev/null || true; }
# procs_in NODE → "pid:comm …" of every process in NODE's netns (holder + harness helpers excluded)
procs_in() {
  local n=$1 ns p out=""
  ns=$(readlink "/proc/${HOLD[$n]}/ns/net" 2>/dev/null) || return 0
  for p in /proc/[0-9]*; do
    p=${p#/proc/}
    [ "$p" = "${HOLD[$n]}" ] && continue
    case " ${HELPERS[*]} " in *" $p "*) continue ;; esac
    [ "$(readlink "/proc/$p/ns/net" 2>/dev/null)" = "$ns" ] || continue
    out="$out $p:$(cat "/proc/$p/comm" 2>/dev/null)"
  done
  printf '%s' "${out# }"
}
# start_node NODE — `111 daemon` in NODE's netns (pid in DPID); waits for /health
start_node() {
  local n=$1 d0=${EPOCHREALTIME/,/.} up=0 code
  NODE111_ROOT=${NROOT[$n]} nsenter -t "${HOLD[$n]}" -n -- "$BIN111" daemon >>"${NROOT[$n]}/daemon.log" 2>&1 &
  DPID[$n]=$!
  for _ in $(seq 1 300); do
    code=$(nx "$n" curl -s -o /dev/null -w '%{http_code}' --max-time 2 "$DAPI/health" 2>/dev/null) || true
    case $code in 200 | 503) up=1 && break ;; esac
    kill -0 "${DPID[$n]}" 2>/dev/null || break
    sleep 0.2
  done
  if [ "$up" = 1 ]; then
    ok "$n: 111 daemon answers /health (HTTP $code) $(awk -v a="$d0" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}')s after launch (pid ${DPID[$n]})"
    # the fabric is wired after the dashboard answers: wait until its routes serve
    local _sn=$n
    wait_for 20 'api "$_sn" GET /api/v1/fabric; [ "$ST" = 200 ] && jb ".store.open == true"' || true
    t "$n: the fabric is up (GET /fabric store.open, $(awk -v a="$d0" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}')s after launch)" \
      '[ "$ST" = 200 ] && jb ".store.open == true"'
  else
    fail "$n: 111 daemon never answered /health"
    tail -20 "${NROOT[$n]}/daemon.log" | sed "s/^/  $n daemon| /"
  fi
}
# agent_up NODE — wait until the daemon relays to its own 111-agent
agent_up() {
  local _an=$1
  wait_for 40 'api "$_an" GET /api/v1/agent/status; [ "$ST" = 200 ]' || true
  api "$_an" GET /api/v1/status
  t "$1: daemon spawned its 111-agent + stub model (intelligence.agent=$(jget .intelligence.agent), model_server=$(jget .intelligence.model_server))" \
    '[ "$(jget .intelligence.agent)" = running ] && [ "$(jget .intelligence.model_server)" = running ]'
}
# stop_node NODE — SIGTERM; $RC exit code ("" when it did not exit within 20 s)
stop_node() {
  local n=$1 p=${DPID[$1]:-}
  RC=""
  [ -n "$p" ] || return 0
  kill -TERM "$p" 2>/dev/null || true
  for _ in $(seq 1 100); do
    kill -0 "$p" 2>/dev/null || break
    sleep 0.2
  done
  if ! kill -0 "$p" 2>/dev/null; then
    wait "$p" && RC=0 || RC=$?
    DPID[$n]=""
  fi
}
# find_prop — A's open message proposal whose body is $pbody → $pid (inner3 locals)
find_prop() {
  api A GET /api/v1/proposals
  pid=$(jget --arg b "$pbody" '[(.proposals // [])[] | select(.kind == "message" and .body == $b and .decision == null)] | first | .id // empty')
  [ -n "$pid" ]
}
# epoch_of NODE PEER ID → msg_id.epoch of that message
epoch_of() {
  msg_get "$1" "$2" "$3" | jq -r '.msg_id.epoch // empty' 2>/dev/null || true
}
# last_own_epoch NODE PEER → msg_id.epoch of NODE's newest own message to PEER
last_own_epoch() {
  api "$1" GET "/api/v1/messages/threads/${NID[$2]}?limit=60"
  jget '[(.messages // [])[] | select(.dir == "own")] | last | .msg_id.epoch // empty'
}

# ------------------------------------------------------------------ three-node (inner)

inner3() {
  : "${SMOKE_WORK:?}" "${BIN111:?}"
  W=$SMOKE_WORK/http
  CAP=$SMOKE_WORK/capture
  mkdir -p "$W" "$CAP"
  local n
  for n in "${N3_NODES[@]}"; do
    NROOT[$n]=$SMOKE_WORK/$n
    NID[$n]=$(cat "$SMOKE_WORK/$n.nid" 2>/dev/null || echo none)
  done
  cleanup_inner3() {
    local p
    for p in ${SSEP_RUN_H:-}; do kill "$p" 2>/dev/null || true; done
    for n in "${N3_NODES[@]}"; do
      p=${DPID[$n]:-}
      if [ -n "$p" ]; then kill -TERM "$p" 2>/dev/null || true; fi
    done
    for _ in $(seq 1 30); do
      local alive=0
      for n in "${N3_NODES[@]}"; do
        p=${DPID[$n]:-}
        [ -n "$p" ] && kill -0 "$p" 2>/dev/null && alive=1
      done
      [ "$alive" = 0 ] && break
      sleep 0.3
    done
    # leaving pid 1 of the namespace kills everything else (holders, relays, stubs)
    printf 'FAILS=%s\n' "$FAILS" >"$SMOKE_WORK/inner.result"
  }
  trap cleanup_inner3 EXIT
  trap 'exit 130' INT TERM

  # ---------------------------------------------------------------- 4. network
  phase "4. namespaces: br0 10.111.0.1/24 + one netns per node (veth)"
  local sigign
  sigign=$(awk '/^SigIgn:/ {print $2}' /proc/self/status)
  t "namespace shell has default SIGINT/SIGTERM (SigIgn=$sigign)" '(( (16#$sigign & 0x4002) == 0 ))'
  ip link set lo up
  if ip link add br0 type bridge && ip addr add 10.111.0.1/24 dev br0 && ip link set br0 up; then
    ok "bridge br0 10.111.0.1/24 up (no route off-box)"
  else
    fail "cannot create br0"
  fi
  local myns
  myns=$(readlink /proc/self/ns/net)
  for n in "${N3_NODES[@]}"; do
    unshare --net sleep 100000 &
    HOLD[$n]=$!
    HELPERS+=("${HOLD[$n]}")
  done
  for n in "${N3_NODES[@]}"; do
    wait_for 5 '[ "$(readlink /proc/${HOLD[$n]}/ns/net 2>/dev/null)" != "$myns" ]' || true
    if ip link add "v$n" type veth peer name eth0 netns "${HOLD[$n]}" &&
      ip link set "v$n" master br0 up &&
      nx "$n" ip link set lo up &&
      nx "$n" ip addr add "${NIP[$n]}/24" dev eth0 &&
      nx "$n" ip link set eth0 up; then
      ok "$n: netns (holder ${HOLD[$n]}) eth0 ${NIP[$n]}/24 on br0"
    else
      fail "$n: netns/veth setup failed"
    fi
  done
  t "A reaches B over the bridge (ping 10.111.0.12 from A's netns)" 'nx A ping -c1 -W3 10.111.0.12 >/dev/null 2>&1'

  # ---------------------------------------------------------------- 5. helpers + daemons
  phase "5. recording relays, fake board, four daemons"
  bg_in B "$SMOKE_WORK/r1.log" python3 -I "$HERE/tcp-lan-relay.py" relay --listen 10.111.0.12:11184 --to 10.111.0.12:11183 --dir "$CAP/r1"
  R1PID=$BGPID
  bg_in H "$SMOKE_WORK/r2.log" python3 -I "$HERE/tcp-lan-relay.py" relay --listen 10.111.0.10:11184 --to 10.111.0.10:11183 --dir "$CAP/r2"
  R2PID=$BGPID
  bg_in H "$SMOKE_WORK/board.log" python3 -I "$HERE/stub-meshmud.py" --role board --listen 127.0.0.1:4403
  wait_for 5 'grep -q "tcp-lan-relay:" "$SMOKE_WORK/r1.log" && grep -q "tcp-lan-relay:" "$SMOKE_WORK/r2.log"' || true
  t "relays up: R1 10.111.0.12:11184 → B:11183, R2 10.111.0.10:11184 → H:11183 (frames recorded raw)" \
    'kill -0 "$R1PID" 2>/dev/null && kill -0 "$R2PID" 2>/dev/null'
  for n in B H C A; do start_node "$n"; done
  sse_on H "$W/H-run.sse"
  SSEP_RUN_H=$SSEP
  for n in "${N3_NODES[@]}"; do agent_up "$n"; done
  for n in "${N3_NODES[@]}"; do
    api "$n" GET /api/v1/fabric
    t "$n: GET /fabric → store open, tcp-lan registered (mtu $(jget '.links[0].profile.mtu_bytes'), state $(jget '.links[0].state'))" \
      '[ "$ST" = 200 ] && jb ".schema == 1 and .store.open == true and (.links | any(.id == \"tcp-lan\"))"'
  done
  wait_for 10 'hello_at 10.111.0.11 11183 && hello_at 10.111.0.12 11183' || true
  t "LAN listeners follow fabric/settings: A and B answer the tcp-lan hello, C and H (hub role still off) do not" \
    'hello_at 10.111.0.11 11183 && hello_at 10.111.0.12 11183 && ! hello_at 10.111.0.13 11183 && ! hello_at 10.111.0.10 11183'

  # ---------------------------------------------------------------- 6. hub role + enrollment
  phase "6. hub role + enrollment (§7.3; H by PATCH, A by the CLI through R2, B by the API)"
  jsend H PATCH /api/v1/fabric/settings '{"hub_role": true}'
  t "H: PATCH /fabric/settings {hub_role:true} → 200 FabricSettings hub_role:true" '[ "$ST" = 200 ] && jb ".hub_role == true"'
  t "H: the listener binds at once (10.111.0.10:11183 answers the hello)" 'wait_for 5 "hello_at 10.111.0.10 11183"'
  local offer_out code_a bundle_a enroll_out
  offer_out=$(NODE111_ROOT=${NROOT[H]} nx H "$BIN111" fabric offer 2>&1) || true
  printf '%s\n' "$offer_out" >"$W/offer-a.out"
  code_a=$(sed -n 's/^Enrollment code: *\([^ ]*\).*/\1/p' "$W/offer-a.out" | head -1)
  bundle_a=$(sed -n "s/^On the member: 111 fabric enroll '\\(.*\\)'\$/\\1/p" "$W/offer-a.out" | head -1)
  t "H: \`111 fabric offer\` prints a one-time code ($code_a), the hub address 10.111.0.10:11183 and a 111HUB: bundle" \
    '[ -n "$code_a" ] && [[ "$bundle_a" == 111HUB:* ]] && grep -q "^Hub address: *10.111.0.10:11183" "$W/offer-a.out"'
  enroll_out=$(NODE111_ROOT=${NROOT[A]} nx A "$BIN111" fabric enroll "$bundle_a" --address 10.111.0.10:11184 2>&1) || true
  printf '%s\n' "$enroll_out" >"$W/enroll-a.out"
  t "A: \`111 fabric enroll <bundle> --address 10.111.0.10:11184\` (via R2) → \"Enrolled with hub <H> … (reachable)\"" \
    'grep -q "^Enrolled with hub ${NID[H]} at 10.111.0.10:11184 (reachable)" "$W/enroll-a.out"'
  [ -s "$W/enroll-a.out" ] && note "A enroll: $(head -c 200 "$W/enroll-a.out" | tr '\n' ' ')"
  jsend H POST /api/v1/fabric/hub/offer '{}'
  local code_b addr_b card_h
  code_b=$(jget .code)
  addr_b=$(jget .address)
  card_h=$(jget .card.text)
  t "H: POST /fabric/hub/offer → 200 HubOffer (code $code_b, address $addr_b, card, bundle), Cache-Control: no-store" \
    '[ "$ST" = 200 ] && [ "$addr_b" = 10.111.0.10:11183 ] && [ -n "$card_h" ] && jb ".bundle.text | startswith(\"111HUB:\")" && [ "$(hdr cache-control)" = no-store ]'
  t "one code per member: the second offer's code differs from the first" '[ -n "$code_a" ] && [ -n "$code_b" ] && [ "$code_b" != "$code_a" ]'
  jsend B POST /api/v1/fabric/hub/enroll "$(jq -nc --arg a "$addr_b" --arg c "$code_b" --arg k "$card_h" '{address: $a, code: $c, card: $k}')"
  t "B: POST /fabric/hub/enroll {address, code, card} → 200 FabricHub enrolled + reachable, node_id = H" \
    '[ "$ST" = 200 ] && jb ".enrolled == true and .reachable == true" && [ "$(jget .node_id)" = "${NID[H]}" ]'
  jsend C POST /api/v1/fabric/hub/enroll "$(jq -nc --arg c "$code_b" --arg k "$card_h" '{address: "10.111.0.10:11183", code: $c, card: $k}')"
  t "a spent code enrols nobody: C with B's used code → 502 upstream_error (the hub refused), C stays a non-member" \
    '[ -n "$code_b" ] && problem_is 502 upstream_error'
  api C GET /api/v1/fabric
  t "C: no home hub after the refusal (hub.enrolled false)" '[ "$ST" = 200 ] && jb ".hub.enrolled == false"'
  api H GET /api/v1/fabric
  t "H: hub.members = 2 (A and B), offer spent (offer_active false), custody empty" \
    'jb ".hub.members == 2 and .hub.offer_active == false and .custody.count == 0"'
  for n in A B; do
    api "$n" GET /api/v1/fabric
    t "$n: hub enrolled + reachable, node_id = H, cursor $(jget .hub.cursor)" \
      'jb ".hub.enrolled == true and .hub.reachable == true" && [ "$(jget .hub.node_id)" = "${NID[H]}" ]'
  done
  wait_for 30 'api H GET /api/v1/services/fabric; [ "$(jget .state)" = live ]' || true
  t "H: fabric service LIVE (a hub with members) — $(jget .reason_code)" '[ "$(jget .state)" = live ]'
  wait_for 20 'api A GET /api/v1/services/fabric; [ "$(jget .state)" = live ]' || true
  t "A: fabric service LIVE (enrolled hub reachable) — $(jget .reason_code)" '[ "$(jget .state)" = live ]'
  # C's dial to A is up, but hello and announce authenticate nothing (§8.1): a
  # connection is attributed only by a fresh sealed packet, and C has sent A
  # nothing yet — so the row is honest: NOT SET. It goes LIVE (peer_linked) in
  # (7) below, after C's first message to A. A LIVE here would mean C's refused
  # enrollment left an attributed link to H (the leak the polish wave closed).
  sleep 1
  api C GET /api/v1/services/fabric
  t "C: fabric service NOT SET before any sealed traffic to its configured peer — the link to A is up but unattributed, no hub ($(jget .reason_code))" \
    '[ "$(jget .state)" = unset ] && [ "$(jget .reason_code)" = not_enrolled ]'
  t "C: tcp-lan counts no attributed peer yet ($(peers_of C))" '[ "$(peers_of C)" = 0 ]'
  local hpe
  wait_for 10 'hpe=$(peers_of H); [ "$hpe" = 2 ]' || true
  t "H: tcp-lan attributed peers = 2 (A and B): C's refused enrollment released its dial to H ($hpe)" '[ "$hpe" = 2 ]'
  api C GET "/api/v1/contacts/${NID[H]}"
  t "C: the hub card pinned only for the refused attempt is forgotten (GET /contacts/H → 404)" '[ "$ST" = 404 ]'

  # ---------------------------------------------------------------- 7. (1) cards
  phase "7. (1) cards: node1… card request via the hub, pasted cards both ways (§7.2)"
  api A GET /api/v1/status
  local a_bech card_a card_b card_c
  a_bech=$(jget .node.bech32)
  jsend B POST /api/v1/contacts "$(jq -nc --arg n "$a_bech" '{node_id: $n}')"
  t "B: POST /contacts {node_id: A's node1… ($a_bech)} → 200 Contact for A (has_key $(jget .has_key) until the hub answers)" \
    '[ "$ST" = 200 ] && [ "$(jget .node_id)" = "${NID[A]}" ]'
  wait_for 20 'api B GET /api/v1/contacts/${NID[A]}; jb ".has_key == true"' || true
  t "B: the hub answered the node1… card request (A enrolled, B attributed) → A has_key:true (trust $(jget .trust))" \
    '[ "$ST" = 200 ] && jb ".has_key == true"'
  api A GET /api/v1/contacts/me/card
  card_a=$(jget .text)
  t "A: GET /contacts/me/card → {text, qr} (qr: ${#card_a}-char text, $(jget .qr.size)×$(jget .qr.size) modules)" \
    '[ "$ST" = 200 ] && [ -n "$card_a" ] && jb ".qr.size > 20 and (.qr.rows | length) == .qr.size and (.qr.rows | all(length == $(jget .qr.size)))"'
  api B GET /api/v1/contacts/me/card
  card_b=$(jget .text)
  api C GET /api/v1/contacts/me/card
  card_c=$(jget .text)
  jsend A POST /api/v1/contacts "$(jq -nc --arg c "$card_b" '{card: $c}')"
  t "A: POST /contacts {card: B's pasted card} → B has_key:true, trust oob" \
    '[ "$ST" = 200 ] && [ "$(jget .node_id)" = "${NID[B]}" ] && jb ".has_key == true and .trust == \"oob\""'
  jsend B POST /api/v1/contacts "$(jq -nc --arg c "$card_a" '{card: $c}')"
  t "B: POST /contacts {card: A's pasted card} → A has_key:true (already known: returned unchanged)" \
    '[ "$ST" = 200 ] && [ "$(jget .node_id)" = "${NID[A]}" ] && jb ".has_key == true"'
  jsend C POST /api/v1/contacts "$(jq -nc --arg c "$card_a" '{card: $c}')"
  t "C: POST /contacts {card: A's card} → has_key:true (C will write to A first, (7))" '[ "$ST" = 200 ] && jb ".has_key == true"'
  jsend B POST /api/v1/contacts "$(jq -nc --arg c "$card_c" '{card: $c}')"
  t "B: POST /contacts {card: C's card} → has_key:true (C is not a member, (4))" '[ "$ST" = 200 ] && jb ".has_key == true"'
  api A GET /api/v1/contacts
  t "A holds no contact for C (C will arrive as a message request)" \
    '[ "$ST" = 200 ] && jb "(.contacts | map(.node_id) | index(\"${NID[C]}\")) == null"'

  # ---------------------------------------------------------------- 8. (2) direct
  phase "8. (2) direct A → B over tcp-lan (through R1): a 16-fragment message, then a small one"
  python3 -I -c 'import json, sys
text = ("L5 big message " + "0123456789abcdef" * 62500)[:1000000]
json.dump({"to": sys.argv[1], "text": text}, open(sys.argv[2], "w"))' "${NID[B]}" "$W/big.json"
  sse_on A "$W/A-big.sse"
  local sp_big=$SSEP big
  api A POST /api/v1/messages -H 'Content-Type: application/json' --data-binary "@$W/big.json"
  big=$(jget .id)
  t "A: POST /messages with a 1,000,000-character text → 202 {id} ($big)" '[ "$ST" = 202 ] && [ -n "$big" ]'
  wait_msg A B "$big" '.state == "delivered"' 90 || true
  t "A: the big message reaches delivered (B's ACK of every fragment) — state $(mfield .state)" '[ "$(mfield .state)" = delivered ]'
  sleep 1
  sse_off "$sp_big"
  local nbig
  nbig=$(python3 -I "$HERE/tcp-lan-relay.py" frames "$CAP"/r1/*.c2s 2>/dev/null | jq -s '[.[].frames[] | select(.[0] == 1 and .[1] > 60000)] | length' 2>/dev/null || echo 0)
  t "the wire (R1 recording A → B) carried it as 16 sealed fragments of ≈62.5 KB each ($nbig frames > 60000 B)" '[ "$nbig" = 16 ]'
  api B GET "/api/v1/messages/threads/${NID[A]}?limit=500"
  local bigcopies
  bigcopies=$(python3 -I -c 'import json, sys
body = json.load(open(sys.argv[1]))
sent = json.load(open(sys.argv[2]))["text"]
print(sum(1 for m in body.get("messages", []) if m.get("text") == sent))' "$W/body" "$W/big.json" 2>/dev/null || echo 0)
  t "B's thread holds the reassembled text exactly once, byte-identical ($bigcopies copies)" '[ "$bigcopies" = 1 ]'
  local bigf bign biglast
  bigf=$(mframes "$W/A-big.sse" "$big")
  bign=$(printf '%s' "$bigf" | grep -c . || true)
  biglast=$(printf '%s\n' "$bigf" | tail -1 | cut -d' ' -f2)
  t "A's SSE coalesces it: $bign message frames for 16 fragments [$(printf '%s' "$bigf" | tr '\n' ',')], ending delivered" \
    '[ "$bign" -ge 1 ] && [ "$bign" -le 4 ] && [ "$biglast" = delivered ]'
  note "gap: no API exposes a message's fragment count or path (Message has msg_id only); the count above is read off the wire recording"

  local small="hello B, direct over tcp-lan" sm
  sse_on A "$W/A-direct.sse"
  local sp_dir=$SSEP
  send A B "$small"
  sm=$MID
  t "A: POST /messages {to: B, text} → 202 {id: $sm}" '[ -n "$sm" ]'
  wait_msg A B "$sm" '.state == "delivered"' 30 || true
  t "A: the small message goes queued → sent → delivered on the direct link (state $(mfield .state), reason $(mfield .reason_code))" '[ "$(mfield .state)" = delivered ]'
  sleep 0.6
  sse_off "$sp_dir"
  t "B's thread shows it exactly once (dir in, received_seq set)" \
    '[ "$(count_text B A "$small")" = 1 ] && jb --arg t "$small" "[.messages[] | select(.text == \$t and .dir == \"in\" and (.received_seq | type) == \"number\")] | length == 1"'
  local df
  df=$(mframes "$W/A-direct.sse" "$sm" | tr '\n' ',')
  t "A's SSE: message frames for it [${df%,}] — first new, last delivered" \
    '[ "$(mframes "$W/A-direct.sse" "$sm" | head -1 | cut -d" " -f1)" = new ] && [ "$(mframes "$W/A-direct.sse" "$sm" | tail -1 | cut -d" " -f2)" = delivered ]'
  tape "$W/A-direct.sse" events-l5-direct.sse "${NROOT[A]}"

  # ---------------------------------------------------------------- 9. (5) replay
  phase "9. (5) replay: A's recorded sealed frames to B on a fresh socket (§4 A11, §8.1)"
  local capA peers0 peers1 len0 len1
  capA=$(python3 -I "$HERE/tcp-lan-relay.py" frames "$CAP"/r1/*.c2s 2>/dev/null | jq -rs 'map(select(.hello)) | max_by([.frames[] | select(.[0] == 1)] | length) | .file // empty' 2>/dev/null || true)
  peers0=$(peers_of B)
  len0=$(thread_len B A)
  if [ -n "$capA" ]; then
    python3 -I "$HERE/tcp-lan-relay.py" replay --capture "$capA" --to 10.111.0.12:11183 \
      --card-request "${NID[A]}" --hold 4 --out "$W/replay-b.json" >/dev/null 2>&1 &
    local rp=$!
    sleep 2
    peers1=$(peers_of B)
    wait "$rp" || true
  fi
  t "replayed from the bridge: hello + A's announce + $(jq -r '.sent.sealed // 0' "$W/replay-b.json" 2>/dev/null) sealed frames + a card request (capture $(basename "${capA:-none}"))" \
    'jf "$W/replay-b.json" ".sent.announce == 1 and .sent.sealed > 0"'
  t "B answered the fresh socket with hello + its own announce only: no sealed frame, no card answer ($(jq -c .received_types "$W/replay-b.json" 2>/dev/null))" \
    'jf "$W/replay-b.json" ".received_hello and (.received_types.sealed // 0) == 0 and (.received_types.announce // 0) == 1"'
  t "B never attributed the replay socket (tcp-lan attributed peers $peers0 → ${peers1:-?} while it was open)" '[ -n "${peers1:-}" ] && [ "$peers1" = "$peers0" ]'
  len1=$(thread_len B A)
  t "no duplicate delivery: B's thread with A unchanged ($len0 → $len1), \"$small\" still once" \
    '[[ "$len0" =~ ^[0-9]+$ ]] && [ "$len1" = "$len0" ] && [ "$(count_text B A "$small")" = 1 ]'
  note "gap: B's replay counters cannot be asserted — mesh111 FabricStatus.counters (stale, dropped, …) exist but GET /fabric (fabric/json.rs status()) and the contract's FabricStatus omit them"

  # ---------------------------------------------------------------- 10. (4) non-member
  phase "10. (4) B → C (not enrolled at H): REJECT_UNKNOWN_DEST shown honestly (§9)"
  send B C "to C, who is not a member of our hub"
  local nm=$MID
  wait_msg B C "$nm" '.reason_code == "hub_not_yours"' 30 || true
  t "B: state sent, reason_code hub_not_yours, reason \"$(mfield .reason)\"" \
    '[ "$(mfield .state)" = sent ] && [ "$(mfield .reason_code)" = hub_not_yours ] && [ -n "$(mfield ".reason // empty")" ]'
  api H GET /api/v1/fabric
  t "H took no custody for it (custody.count $(jget .custody.count))" '[ "$ST" = 200 ] && jb ".custody.count == 0"'
  api C GET "/api/v1/messages/threads/${NID[B]}"
  t "C never saw it (no thread with B: 404)" '[ "$ST" = 404 ]'

  # ---------------------------------------------------------------- 11. (7) requests
  phase "11. (7) message request: C (no contact entry on A) writes first over tcp-lan (INTRO)"
  sse_on A "$W/A-request.sse"
  local sp_req=$SSEP rq="hello A, this is C writing first"
  send C A "$rq"
  local cm=$MID
  t "C: POST /messages {to: A} → 202 ($cm)" '[ -n "$cm" ]'
  wait_for 30 'api A GET /api/v1/contacts/${NID[C]}; [ "$ST" = 200 ] && jb ".request == true"' || true
  t "A lists a message request from C (Contact.request:true, trust $(jget .trust))" '[ "$ST" = 200 ] && jb ".request == true and .trust == \"tofu\""'
  api A GET /api/v1/messages/threads
  t "A: GET /messages/threads has C's thread marked request:true" 'jb "(.threads | map(select(.peer == \"${NID[C]}\")) | first | .request) == true"'
  t "A holds the message unshown-but-kept: thread request:true, text present once" \
    '[ "$(count_text A C "$rq")" = 1 ] && jb ".request == true"'
  sleep 1
  wait_msg C A "$cm" '.state != "queued"' 5 || true
  t "C's copy is not delivered while A has not accepted (no ACK for a pending request): $(mfield .state)" '[ "$(mfield .state)" = sent ]'
  api A GET "/api/v1/contacts/${NID[C]}"
  t "ignore is client-side: on the node the request simply stays pending" 'jb ".request == true"'
  jsend A PATCH "/api/v1/contacts/${NID[C]}" '{"accept": true}'
  t "A: PATCH /contacts/C {accept:true} → 200 request:false" '[ "$ST" = 200 ] && jb ".request == false"'
  wait_msg C A "$cm" '.state == "delivered"' 30 || true
  t "C's copy reaches delivered (A ACKs exactly what it shows)" '[ "$(mfield .state)" = delivered ]'
  wait_for 10 'api C GET /api/v1/services/fabric; [ "$(jget .state)" = live ]' || true
  t "C: fabric service LIVE through its configured direct peer once A's sealed ACK attributed the link, no hub ($(jget .reason_code): $(jget .reason))" \
    '[ "$(jget .state)" = live ] && [ "$(jget .reason_code)" = peer_linked ] && [ "$(peers_of C)" = 1 ]'
  t "A's thread with C shows the held message exactly once" '[ "$(count_text A C "$rq")" = 1 ]'
  sleep 0.6
  sse_off "$sp_req"
  t "A's SSE carried the request (contact change request) and the message" \
    'sse_data "$W/A-request.sse" contact | jq -se "any(.[]; .change == \"request\" and .contact.node_id == \"${NID[C]}\")" >/dev/null 2>&1 && sse_data "$W/A-request.sse" message | jq -se "any(.[]; .change == \"request\" and .peer == \"${NID[C]}\")" >/dev/null 2>&1'
  tape "$W/A-request.sse" events-l5-request.sse "${NROOT[A]}"

  # ---------------------------------------------------------------- 12. (8) agent
  phase "12. (8) agent tools on A (stub model → 111-agent → daemon-minted bearer → 111d)"
  local tr
  chat A conv-l5-read "@tool messages.read {\"peer\": \"${NID[B]}\", \"limit\": 1}"
  t "messages.read through /v1/chat: event tool + done.tool_trace = [{messages.read, ok:true}]" \
    'grep -q "^event: tool" "$W/chat-conv-l5-read.sse" && printf "%s" "$DONE" | jqs ".tool_trace == [{\"name\": \"messages.read\", \"ok\": true}]"'
  tr=$(toolres A messages.read)
  t "messages.read result carries A's thread with B (\"$small\")" '[[ "$tr" == *"$small"* ]]'
  tape "$W/chat-conv-l5-read.sse" chat-l5-tool.sse "${NROOT[A]}"
  chat A conv-l5-contacts "@tool contacts.read {}"
  tr=$(toolres A contacts.read)
  t "contacts.read answered (ok:true) and lists B and C" \
    'printf "%s" "$DONE" | jqs ".tool_trace == [{\"name\": \"contacts.read\", \"ok\": true}]" && [[ "$tr" == *"${NID[B]}"* && "$tr" == *"${NID[C]}"* ]]'
  chat A conv-l5-fabric "@tool fabric.read {}"
  tr=$(toolres A fabric.read)
  t "fabric.read answered (ok:true) with the home hub (H's node_id)" \
    'printf "%s" "$DONE" | jqs ".tool_trace == [{\"name\": \"fabric.read\", \"ok\": true}]" && [[ "$tr" == *"${NID[H]}"* ]]'
  chat A conv-l5-nearby "@tool nearby.read {}"
  tr=$(toolres A nearby.read)
  t "nearby.read answered (ok:true) inside the labelled untrusted block (A has no board: no sources)" \
    'printf "%s" "$DONE" | jqs ".tool_trace == [{\"name\": \"nearby.read\", \"ok\": true}]" && [[ "$tr" == *"UNTRUSTED: heard over the air from strangers"* ]]'
  local ptext="proposed by the agent on A"
  chat A conv-l5-send "@tool messages.send {\"to\": \"${NID[B]}\", \"text\": \"$ptext\"}"
  t "messages.send answered (ok:true): it writes a proposal, it never sends" \
    'printf "%s" "$DONE" | jqs ".tool_trace == [{\"name\": \"messages.send\", \"ok\": true}]" && [ "$(count_text B A "$ptext")" = 0 ]'
  local pid="" pbody
  pbody="to: ${NID[B]}"$'\n\n'"$ptext"
  wait_for 15 find_prop || true
  t "A: GET /proposals lists it — kind message, body \"to: <B>\\n\\n$ptext\" ($pid)" '[ -n "$pid" ]'
  jsend A POST "/api/v1/proposals/${pid:-none}/decision" '{"decision": "accepted"}'
  t "A: POST /proposals/$pid/decision {accepted} → 200 (111d sends it)" '[ "$ST" = 200 ] && jb ".decision.decision == \"accepted\""'
  wait_for 30 '[ "$(count_text B A "$ptext")" = 1 ]' || true
  t "B receives the accepted proposal's text exactly once" '[ "$(count_text B A "$ptext")" = 1 ]'

  # ---------------------------------------------------------------- 13. (3) hub custody
  phase "13. (3) hub custody: A off the LAN, B → A relayed by H, A back → delivered once (§9)"
  local pa0 pb0
  pa0=$(peers_of A)
  pb0=$(peers_of B)
  kill "$R1PID" 2>/dev/null || true
  wait "$R1PID" 2>/dev/null || true
  wait_for 20 '[ "$(peers_of A)" = $((pa0 - 1)) ] && [ "$(peers_of B)" = $((pb0 - 1)) ]' || true
  t "R1 stopped: the A ↔ B direct link is gone (attributed peers A $pa0 → $(peers_of A), B $pb0 → $(peers_of B); B keeps only the hub)" \
    '[ "$(peers_of A)" = $((pa0 - 1)) ] && [ "$(peers_of B)" = $((pb0 - 1)) ] && [ "$(peers_of B)" = 1 ]'
  nx A ip link set eth0 down
  ok "A: eth0 down (A is off the LAN; its daemon keeps running)"
  sse_on B "$W/B-relayed.sse"
  local sp_rel=$SSEP away="while you were away (held by the hub)"
  send B A "$away"
  local ra=$MID
  wait_msg B A "$ra" '.state == "relayed"' 30 || true
  t "B: relayed — H's CUSTODY_ACK ACCEPTED for a known member (state $(mfield .state), reason $(mfield .reason_code))" '[ "$(mfield .state)" = relayed ]'
  api H GET /api/v1/fabric
  t "H holds custody for A (custody.count $(jget .custody.count), $(jget .custody.bytes) B)" 'jb ".custody.count >= 1"'
  sleep 1
  nx A ip link set eth0 up
  ok "A: eth0 up again"
  wait_msg B A "$ra" '.state == "delivered"' 120 || true
  t "A back on the LAN → B's message delivered (state $(mfield .state))" '[ "$(mfield .state)" = delivered ]'
  t "A's thread with B shows it exactly once" '[ "$(count_text A B "$away")" = 1 ]'
  sleep 0.6
  sse_off "$sp_rel"
  local rf
  rf=$(mframes "$W/B-relayed.sse" "$ra" | cut -d' ' -f2 | uniq | tr '\n' ' ')
  t "B's SSE for it: ${rf% } (expected sent → relayed → delivered)" '[ "${rf% }" = "queued sent relayed delivered" ] || [ "${rf% }" = "sent relayed delivered" ]'
  tape "$W/B-relayed.sse" events-l5-relayed.sse "${NROOT[B]}"

  # ---------------------------------------------------------------- 13b. capacity refusal, hub crash
  phase "13b. (3, G1) custody refused at capacity (mailbox_full), then the hub killed -9 while it holds A's item"
  jsend H PATCH /api/v1/fabric/settings '{"quotas": {"custody": 0}}'
  t "H: PATCH /fabric/settings {quotas: {custody: 0}} → 200 (custody quota 0: capacity shaping, §10.4)" \
    '[ "$ST" = 200 ] && jb ".quotas.custody == 0"'
  nx A ip link set eth0 down
  ok "A: eth0 down again"
  local full="refused while the hub has no room, then retried"
  send B A "$full"
  local fm=$MID
  wait_msg B A "$fm" '.reason_code == "mailbox_full"' 30 || true
  t "B: sent, reason_code mailbox_full (H answered REJECT_FULL), reason \"$(mfield .reason)\"" \
    '[ "$(mfield .state)" = sent ] && [ "$(mfield .reason_code)" = mailbox_full ] && [ -n "$(mfield ".reason // empty")" ]'
  note "gap: store_full itself (§10.4 watermark) cannot be forced from outside — the store's high watermark follows the disk; the class quota above is the reachable capacity refusal"
  jsend H PATCH /api/v1/fabric/settings '{"quotas": {"custody": null}}'
  t "H: custody quota cleared (quotas.custody null)" '[ "$ST" = 200 ] && jb ".quotas.custody == null"'
  jsend B POST "/api/v1/messages/${fm:-none}/retry" '{}'
  t "B: POST /messages/$fm/retry → 200 Message" '[ "$ST" = 200 ] && [ "$(jget .id)" = "$fm" ]'
  wait_msg B A "$fm" '.state == "relayed"' 30 || true
  t "B: the retry is relayed (custody ACCEPTED, reason cleared: $(mfield .reason_code))" '[ "$(mfield .state)" = relayed ]'
  kill -9 "${DPID[H]}" 2>/dev/null || true
  wait "${DPID[H]}" 2>/dev/null || true
  DPID[H]=""
  local horph
  horph=$(procs_in H)
  note "H: 111d killed -9 while holding A's item; orphaned children reaped by the harness: ${horph:-none}"
  for p in $horph; do kill -9 "${p%%:*}" 2>/dev/null || true; done
  wait_for 5 '[ -z "$(procs_in H)" ]' || true
  start_node H
  agent_up H
  api H GET /api/v1/fabric
  t "H reopened its store after the unclean stop: custody still held (count $(jget .custody.count)), members $(jget .hub.members)" \
    'jb ".store.open == true and .custody.count >= 1 and .hub.members == 2"'
  nx A ip link set eth0 up
  ok "A: eth0 up again"
  if ! wait_msg B A "$fm" '.state == "delivered"' 120; then
    note "diag B msg $fm: $(printf '%s' "${M:-null}" | jq -c '{state, reason_code, msg_id}' 2>/dev/null)"
    fab_diag A B H
  fi
  t "A fetched it from the restarted hub by cursor → delivered (state $(mfield .state))" '[ "$(mfield .state)" = delivered ]'
  t "A's thread shows it exactly once (custody that survived a hub crash is not redelivered)" '[ "$(count_text A B "$full")" = 1 ]'

  # ---------------------------------------------------------------- 14. (6) unclean restart
  phase "14. (6) A killed -9: custody fetched by cursor, a replayed announce gets no push, new epoch"
  send A B "before the crash"
  local bc=$MID e0 e1
  wait_msg A B "$bc" '.state == "delivered"' 30 || true
  e0=$(mfield .msg_id.epoch)
  t "A → B (via the hub) delivered before the crash (epoch $e0)" '[ "$(mfield .state)" = delivered ] && [ -n "$e0" ]'
  kill -9 "${DPID[A]}" 2>/dev/null || true
  wait "${DPID[A]}" 2>/dev/null || true
  DPID[A]=""
  local orphans
  orphans=$(procs_in A)
  note "A: 111d killed -9; its children outlive it (no PR_SET_PDEATHSIG; a service manager's cgroup kill reaps them in production): ${orphans:-none} — the harness reaps them"
  for p in $orphans; do kill -9 "${p%%:*}" 2>/dev/null || true; done
  wait_for 5 '[ -z "$(procs_in A)" ]' || true
  local crashed="written while A was crashed"
  send B A "$crashed"
  local wc=$MID
  wait_msg B A "$wc" '.state == "relayed"' 30 || true
  t "B → A while A is dead: relayed (H holds it, state $(mfield .state))" '[ "$(mfield .state)" = relayed ]'
  local capH c0 c1 hp0 hp1
  capH=$(python3 -I "$HERE/tcp-lan-relay.py" frames "$CAP"/r2/*.c2s 2>/dev/null | jq -rs 'map(select(.hello)) | max_by([.frames[] | select(.[0] == 1)] | length) | .file // empty' 2>/dev/null || true)
  api H GET /api/v1/fabric
  c0=$(jget .custody.count)
  hp0=$(peers_of H)
  if [ -n "$capH" ]; then
    python3 -I "$HERE/tcp-lan-relay.py" replay --capture "$capH" --to 10.111.0.10:11183 --hold 4 --out "$W/replay-h.json" >/dev/null 2>&1 &
    local rh=$!
    sleep 2
    hp1=$(peers_of H)
    wait "$rh" || true
  fi
  t "replayed A's recorded hub session (announce + $(jq -r '.sent.sealed // 0' "$W/replay-h.json" 2>/dev/null) sealed: ENROLL, FETCH, COMMIT…) to H on a fresh socket" \
    'jf "$W/replay-h.json" ".sent.announce == 1 and .sent.sealed > 0"'
  t "H pushed nothing to it although it holds A's custody: no sealed frame back ($(jq -c .received_types "$W/replay-h.json" 2>/dev/null))" \
    'jf "$W/replay-h.json" ".received_hello and (.received_types.sealed // 0) == 0"'
  api H GET /api/v1/fabric
  c1=$(jget .custody.count)
  t "H never attributed it (peers $hp0 → ${hp1:-?}) and still holds the custody ($c0 → $c1)" '[ "${hp1:-x}" = "$hp0" ] && [ "$c1" = "$c0" ] && [ "${c1:-0}" -ge 1 ]'
  start_node A
  agent_up A
  wait_msg B A "$wc" '.state == "delivered"' 60 || true
  t "A restarted, fetched its mailbox by cursor → B's message delivered (state $(mfield .state))" '[ "$(mfield .state)" = delivered ]'
  t "A's thread: \"$crashed\" once, \"$away\" still once (nothing redelivered)" \
    '[ "$(count_text A B "$crashed")" = 1 ] && [ "$(count_text A B "$away")" = 1 ]'
  send A B "after the crash"
  local ac=$MID
  wait_msg A B "$ac" '.state == "delivered"' 30 || true
  e1=$(mfield .msg_id.epoch)
  t "A's next message uses a new epoch (msg_id.epoch $e0 → $e1) and is delivered" '[ -n "$e1" ] && [ -n "$e0" ] && [ "$e1" -gt "$e0" ] && [ "$(mfield .state)" = delivered ]'
  t "B's thread: \"before the crash\" once, \"after the crash\" once (nothing redelivered)" \
    '[ "$(count_text B A "before the crash")" = 1 ] && [ "$(count_text B A "after the crash")" = 1 ]'

  # ---------------------------------------------------------------- 15. (9) Meshtastic
  phase "15. (9) Meshtastic on H: stub meshmud downlink, bound board, Nearby, panel ticket (§13, §14)"
  wait_for 40 'api H GET /api/v1/services/meshtastic; [ "$(jget .state)" = live ]' || true
  t "H: meshtastic service LIVE ($(jget .reason_code): $(jget .reason))" '[ "$(jget .state)" = live ]'
  local pport
  # the panel host enables a panel's listener on its own reconcile tick after the registry
  # publishes the service as set up — wait for it like the UI's Panel tab does
  wait_for 20 'api H GET /api/v1/services/meshtastic; jb ".panel.state.enabled == true and .panel.state.listening == true"' || true
  pport=$(jget .panel.port)
  t "H: the service record carries its panel, enabled and listening (port $pport in 11190–11229, state $(jget -c .panel.state))" \
    '[[ "$pport" =~ ^[0-9]+$ ]] && [ "$pport" -ge 11190 ] && [ "$pport" -le 11229 ] && jb ".panel.state.enabled == true and .panel.state.listening == true"'
  api H GET /api/v1/nearby
  t "H: GET /nearby → one meshtastic source, the stub's 3 nodes ($(jget -c '[.sources[0].nodes[]?.long_name]'))" \
    '[ "$ST" = 200 ] && jb ".sources | length == 1 and .[0].kind == \"meshtastic\" and (.[0].nodes | length) == 3 and (.[0].nodes | any(.long_name == \"Hilltop relay\"))"'
  t "Nearby is grouped by source (\"$(jget '.sources[0].label')\")" 'jb ".sources[0].label | startswith(\"heard via Meshtastic on \")"'
  chat H conv-l5-nearby "@tool nearby.read {}"
  tr=$(toolres H nearby.read)
  t "names reach the agent only inside the labelled untrusted block (nearby.read on H: label + \"Hilltop relay\")" \
    'printf "%s" "$DONE" | jqs ".tool_trace == [{\"name\": \"nearby.read\", \"ok\": true}]" && [[ "$tr" == *"UNTRUSTED: heard over the air from strangers"* && "$tr" == *"Hilltop relay"* ]]'
  jsend H POST /api/v1/panels/meshtastic/session '{}'
  local turl
  turl=$(jget .url)
  t "H: POST /panels/meshtastic/session → 200 one-time ticket URL on the panel origin ($(printf '%s' "$turl" | sed 's/p111_ticket=.*/p111_ticket=…/'))" \
    '[ "$ST" = 200 ] && [[ "$turl" =~ ^http://127\.0\.0\.1:${pport}/.*p111_ticket=[0-9a-f]{32}$ ]] && [ "$(hdr cache-control)" = no-store ]'
  NSX=(nsenter -t "${HOLD[H]}" -n --)
  req GET "$turl"
  NSX=()
  local pcookie
  pcookie=$(hdr set-cookie)
  t "the panel listener swaps the ticket for the host-only cookie p111_meshtastic (HTTP $ST)" \
    '[[ "$ST" =~ ^30[0-9]$ ]] && [[ "$pcookie" == p111_meshtastic=* ]] && [[ "$pcookie" == *HttpOnly* && "$pcookie" == *SameSite=Strict* ]]'
  sleep 1
  sse_off "$SSEP_RUN_H"
  SSEP_RUN_H=""
  t "H's SSE carried fabric, contact, nearby and panels frames" \
    '[ -n "$(sse_data "$W/H-run.sse" fabric | head -1)" ] && [ -n "$(sse_data "$W/H-run.sse" contact | head -1)" ] && [ -n "$(sse_data "$W/H-run.sse" nearby | head -1)" ] && [ -n "$(sse_data "$W/H-run.sse" panels | head -1)" ]'
  tape "$W/H-run.sse" events-l5-hub.sse "${NROOT[H]}"

  # ---------------------------------------------------------------- 16. (10) clean restart
  phase "16. (10) clean shutdown of every node, reopen without an epoch jump (§4, §10.1)"
  send H A "from the hub before the clean restart"
  local hm=$MID
  wait_msg H A "$hm" '.state == "delivered"' 30 || true
  t "H → A delivered (direct on A's hub connection)" '[ "$(mfield .state)" = delivered ]'
  declare -A EP0=()
  EP0[A]=$(last_own_epoch A B)
  EP0[B]=$(last_own_epoch B A)
  EP0[C]=$(last_own_epoch C A)
  EP0[H]=$(last_own_epoch H A)
  note "epochs before the clean restart: A=${EP0[A]} B=${EP0[B]} C=${EP0[C]} H=${EP0[H]}"
  local left before dp
  for n in "${N3_NODES[@]}"; do
    # positive control: the daemon is seen in its netns before the stop (procs_in can't go vacuous)
    dp=${DPID[$n]:-}
    before=$(procs_in "$n")
    stop_node "$n"
    wait_for 10 '[ -z "$(procs_in "$n")" ]' || true
    left=$(procs_in "$n")
    t "$n: SIGTERM → 111 daemon exits 0 (exit ${RC:-none}), children joined ($(wc -w <<<"$before") processes in its netns before, left: ${left:-none})" \
      '[ -n "$dp" ] && [[ " $before " == *" $dp:"* ]] && [ "$RC" = 0 ] && [ -z "$left" ]'
    t "$n: the fabric closed cleanly (no \"clean close failed/timed out\" in daemon.log)" \
      '[ -s "${NROOT[$n]}/daemon.log" ] && ! grep -q "clean close \(failed\|timed out\)\|identity counters not closed cleanly" "${NROOT[$n]}/daemon.log"'
  done
  for n in "${N3_NODES[@]}"; do start_node "$n"; done
  send A B "after a clean restart (A)"
  t "A: reopened store keeps its epoch (${EP0[A]} → $(epoch_of A B "$MID"))" '[ -n "$MID" ] && [ -n "${EP0[A]}" ] && [ "$(epoch_of A B "$MID")" = "${EP0[A]}" ]'
  send B A "after a clean restart (B)"
  t "B: reopened store keeps its epoch (${EP0[B]} → $(epoch_of B A "$MID"))" '[ -n "$MID" ] && [ -n "${EP0[B]}" ] && [ "$(epoch_of B A "$MID")" = "${EP0[B]}" ]'
  send C A "after a clean restart (C)"
  t "C: reopened store keeps its epoch (${EP0[C]} → $(epoch_of C A "$MID"))" '[ -n "$MID" ] && [ -n "${EP0[C]}" ] && [ "$(epoch_of C A "$MID")" = "${EP0[C]}" ]'
  send H A "after a clean restart (H)"
  t "H: reopened store keeps its epoch (${EP0[H]} → $(epoch_of H A "$MID"))" '[ -n "$MID" ] && [ -n "${EP0[H]}" ] && [ "$(epoch_of H A "$MID")" = "${EP0[H]}" ]'
  local hm2=$MID
  wait_msg H A "$hm2" '.state == "delivered"' 30 || true
  t "after the restart the fabric still delivers (H → A $(mfield .state); custody count at H back to $(api H GET /api/v1/fabric; jget .custody.count))" \
    '[ "$(mfield .state)" = delivered ]'
  if [ -n "$TAPED" ]; then
    note "tapes recorded into ${TAPES#"$OS"/}:$TAPED"
  else
    note "tapes in ui/test/tapes already present — kept (SMOKE_TAPES=1 re-records)"
  fi
  note "elapsed in the namespaces: $(elapsed)s"
}

# ------------------------------------------------------------------ outer

case "${1:-}" in
  --inner)
    inner
    exit $((FAILS > 0))
    ;;
  --inner3)
    inner3
    exit $((FAILS > 0))
    ;;
  --three-node) MODE=three ;;
  "") MODE=single ;;
  *)
    echo "usage: bash 40-interface/verify/smoke.sh [--three-node]" >&2
    exit 2
    ;;
esac

for tool in curl jq python3 unshare ip sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "FAIL: $tool not found — the smoke needs it"
    exit 1
  }
done
if [ "$MODE" = three ]; then
  command -v nsenter >/dev/null 2>&1 || {
    echo "FAIL: nsenter not found — the three-node smoke needs it"
    exit 1
  }
fi

WORK=$(mktemp -d -t 111-smoke.XXXXXX)
cleanup() {
  if [ "${SMOKE_KEEP:-0}" = 1 ] || [ "$FAILS" -gt 0 ]; then
    echo "note: scratch kept at $WORK (daemon.log, agent/, http/)"
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT
LOGS=$WORK/logs
mkdir -p "$LOGS"

if [ "$MODE" = three ]; then
  echo "40-interface/verify/smoke.sh --three-node — Layer 5 three-node harness (scratch $WORK)"
else
  echo "40-interface/verify/smoke.sh — Layer 4 end-to-end (scratch $WORK)"
fi

# ------------------------------------------------------------------ 1. UI
phase "1. UI build"
if [ "$MODE" = three ]; then
  note "three-node mode: UI build skipped (no browser here; the single-node mode builds and serves ui/dist)"
elif [ "${SMOKE_SKIP_BUILD:-0}" = 1 ]; then
  note "SMOKE_SKIP_BUILD=1 — UI build skipped (dev loop)"
elif ! command -v pnpm >/dev/null 2>&1; then
  fail "pnpm not on PATH — cannot build the UI"
else
  mods=$UI/node_modules/.modules.yaml
  if [ -f "$mods" ] && [ "$mods" -nt "$UI/pnpm-lock.yaml" ] && [ "$mods" -nt "$UI/package.json" ]; then
    ok "ui: node_modules fresh — install skipped"
  elif pnpm --dir "$UI" install --frozen-lockfile >"$LOGS/pnpm-install.log" 2>&1; then
    ok "ui: pnpm install --frozen-lockfile"
  else
    fail "ui: pnpm install --frozen-lockfile (log: $LOGS/pnpm-install.log)"
    tail -15 "$LOGS/pnpm-install.log" | sed 's/^/  pnpm| /'
  fi
  t1=${EPOCHREALTIME/,/.}
  if pnpm --dir "$UI" build >"$LOGS/pnpm-build.log" 2>&1; then
    ok "ui: pnpm build ($(awk -v a="$t1" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}')s)"
  else
    fail "ui: pnpm build (log: $LOGS/pnpm-build.log)"
    tail -20 "$LOGS/pnpm-build.log" | sed 's/^/  vite| /'
  fi
fi
if [ "$MODE" = single ]; then
  if [ -f "$UI/dist/index.html" ]; then ok "ui: dist/index.html exists"; else fail "ui: dist/index.html missing"; fi
  if compgen -G "$UI/dist/assets/*.js" >/dev/null; then
    ok "ui: dist/assets/*.js present ($(find "$UI/dist/assets" -name '*.js' | wc -l) files)"
  else
    fail "ui: no dist/assets/*.js"
  fi
fi

# ------------------------------------------------------------------ 2. Rust
phase "2. cargo build (debug)"
TGT_BOOT=${CARGO_TARGET_DIR:-$BOOT/target}
TGT_INTEL=${CARGO_TARGET_DIR:-$INTEL/target}
BIN111=$TGT_BOOT/debug/111
BINAGENT=$TGT_INTEL/debug/111-agent
if [ "${SMOKE_SKIP_BUILD:-0}" = 1 ]; then
  note "SMOKE_SKIP_BUILD=1 — cargo builds skipped (dev loop)"
else
  for spec in "$BOOT:111" "$INTEL:111-agent"; do
    dir=${spec%%:*}
    bin=${spec##*:}
    t1=${EPOCHREALTIME/,/.}
    if (cd "$dir" && cargo build --bin "$bin") >"$LOGS/cargo-$bin.log" 2>&1; then
      ok "cargo build --bin $bin ($(awk -v a="$t1" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}')s)"
    else
      fail "cargo build --bin $bin (log: $LOGS/cargo-$bin.log)"
      tail -20 "$LOGS/cargo-$bin.log" | sed 's/^/  cargo| /'
    fi
  done
fi
for b in "$BIN111" "$BINAGENT"; do
  [ -x "$b" ] || {
    fail "binary missing: $b — cannot run the daemon"
    exit 1
  }
done
if [ "$MODE" = single ] && [ "$BIN111" -ot "$UI/dist/index.html" ]; then
  note "111 binary is older than ui/dist — served UI comes from disk (debug rust-embed), checked below"
fi

# ------------------------------------------------------------------ 3. scratch root(s)
VER=2026.10.13
# make_root ROOT ENVFILE_TEXT [STUB_ARGS] — releases/<ver> with the KB + payload manifest,
# bin/111-agent → target, bin/llama-server → stub-llama.py (+ STUB_ARGS), a dummy .gguf
make_root() {
  local root=$1 envtext=$2 stub_args=${3:-}
  local rel=$root/releases/$VER
  local kb=$rel/share/kb
  mkdir -p "$rel/bin" "$kb" "$root/models/current" "$root/etc" "$root/state"
  ln -s "releases/$VER" "$root/current"
  ln -s "$BINAGENT" "$rel/bin/111-agent"
  cat >"$rel/bin/llama-server" <<EOF
#!/bin/sh
# smoke stand-in for llama-server: same flags (--model --host --port), stub replies
exec python3 -I "$HERE/stub-llama.py"$stub_args "\$@"
EOF
  chmod 755 "$rel/bin/llama-server"
  printf 'GGUF smoke placeholder — never loaded (the stub ignores --model)\n' >"$root/models/current/stub.gguf"
  # KB = the knowledge tree's entries (what a release packs into share/kb), no _meta/docs
  (cd "$OS/00-knowledge" && find . -path ./_meta -prune -o -type f \( -name '*.md' -o -name 'llms.txt' \) -print) |
    while read -r f; do
      case $f in ./AGENTS.md | ./README.md | ./TASKS.md) continue ;; esac
      mkdir -p "$kb/$(dirname "$f")"
      cp "$OS/00-knowledge/$f" "$kb/$f"
    done
  (cd "$rel" && find share/kb -type f -print0 | sort -z | xargs -0 sha256sum) >"$rel/share/payload.manifest"
  printf '%s\n' "$envtext" >"$root/etc/env"
}

# reset the SIGINT/SIGTERM that unshare ignores in its child (see inner())
if env --default-signal true 2>/dev/null; then
  RESET_SIG=(env --default-signal)
else
  RESET_SIG=(python3 -c 'import os,signal,sys
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])')
fi
INNER_LOG=$WORK/inner.log

if [ "$MODE" = three ]; then
  phase "3. scratch roots H A B C (identities, fabric/settings, H's Meshtastic binding)"
  STUB_MM=$HERE/stub-meshmud.py
  if [ ! -f "$STUB_MM" ]; then
    fail "40-interface/verify/stub-meshmud.py (V2's stub meshmud) is missing — scenario (9) cannot run"
  fi
  for n in H A B C; do
    make_root "$WORK/$n" "# three-node smoke: dashboard on loopback; the fabric listener follows fabric/settings" \
      " --tool-log \"$WORK/$n/stub-tools.log\""
    nid=$(NODE111_ROOT=$WORK/$n "$BIN111" identity generate 2>"$LOGS/identity-$n.log" | tail -1) || true
    if [[ "$nid" =~ ^[0-9a-f]{64}$ ]]; then
      printf '%s\n' "$nid" >"$WORK/$n.nid"
      ok "$n: identity generated ($nid)"
    else
      fail "$n: 111 identity generate → '$nid' ($(head -c 200 "$LOGS/identity-$n.log"))"
    fi
  done
  NID_A=$(cat "$WORK/A.nid" 2>/dev/null || true)
  NID_B=$(cat "$WORK/B.nid" 2>/dev/null || true)
  # fabric/settings (§15 settings + the node-local `peers` fact): H's hub role is switched
  # on later through PATCH /fabric/settings; A and B listen on the LAN; A keeps a direct
  # dial to B through the recording relay R1; C dials A directly (its INTRO path, (7)).
  fabset() {
    mkdir -p "$1/fabric"
    chmod 700 "$1/fabric"
    printf '%s\n' "$2" >"$1/fabric/settings"
    chmod 600 "$1/fabric/settings"
  }
  fabset "$WORK/H" '{"schema": 1, "hub_role": false, "listen_lan": false}'
  fabset "$WORK/A" "{\"schema\": 1, \"listen_lan\": true, \"peers\": [{\"node_id\": \"$NID_B\", \"address\": \"10.111.0.12:11184\"}]}"
  fabset "$WORK/B" '{"schema": 1, "listen_lan": true}'
  fabset "$WORK/C" "{\"schema\": 1, \"listen_lan\": false, \"peers\": [{\"node_id\": \"$NID_A\", \"address\": \"10.111.0.11:11183\"}]}"
  ok "fabric/settings: H all off (hub role via the API), A+B listen_lan, A→B peer via R1 :11184, C→A peer :11183"
  # H: the Meshtastic stand-ins (§14): meshmud = V2's stub (downlink 127.0.0.1:11230, tcp
  # uplink to a fake board on 127.0.0.1:4403), presence reader = the stub's presence role
  # behind venvs/presence/bin/python, a placeholder panel bundle, and the board binding.
  relH=$WORK/H/releases/$VER
  cat >"$relH/bin/meshmud" <<EOF
#!/bin/sh
# smoke stand-in for meshmud (CI-built Go binary; no Go on the dev box): V2's stub
exec python3 -I "$STUB_MM" --role meshmud "\$@"
EOF
  chmod 755 "$relH/bin/meshmud"
  mkdir -p "$relH/share/meshtastic-presence" "$WORK/H/venvs/presence/bin" "$relH/panels/meshtastic-web"
  ln -s "$(command -v python3)" "$WORK/H/venvs/presence/bin/python"
  cat >"$relH/share/meshtastic-presence/meshtastic_presence.py" <<EOF
# smoke stand-in for the meshtastic-presence sidecar: V2's stub, presence role
import runpy, sys
sys.argv = ["$STUB_MM", "--role", "presence"] + sys.argv[1:]
runpy.run_path("$STUB_MM", run_name="__main__")
EOF
  printf '<!doctype html><title>stub panel</title><p>smoke placeholder for the Meshtastic web bundle</p>\n' \
    >"$relH/panels/meshtastic-web/index.html"
  cat >"$WORK/H/state/devices.json" <<'EOF'
{"schema": 1, "devices": {"abcd1234abcd1234": {
  "first_seen": "2026-10-01T00:00:00Z", "last_seen": "2026-10-01T00:00:00Z",
  "firmware": {"family": "meshtastic", "version": "2.7.11.abcdef0", "source": "probe"},
  "meshtastic": {"uplink": {"kind": "tcp", "host": "127.0.0.1", "port": 4403},
                 "node_num": 168430090, "node_id": "!0a0a0a0a", "hw_model": "HELTEC_V3"}}}}
EOF
  ok "H: bin/meshmud → stub-meshmud.py --role meshmud, presence reader → --role presence, panels/meshtastic-web placeholder, devices.json binding (tcp uplink 127.0.0.1:4403)"

  SMOKE_WORK=$WORK BIN111=$BIN111 SMOKE_T0=$T0 \
    unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
    "${RESET_SIG[@]}" bash "$HERE/smoke.sh" --inner3 2>&1 | tee "$INNER_LOG" || true
else
  phase "3. scratch node root"
  ROOT=$WORK/root
  make_root "$ROOT" "# smoke: LAN sharing on so pairing auth is exercised
DASHBOARD_LAN=true"
  REL=$ROOT/releases/$VER
  NODE_ID=$(NODE111_ROOT=$ROOT "$BIN111" identity generate 2>"$LOGS/identity.log" | tail -1) || true
  if [[ "$NODE_ID" =~ ^[0-9a-f]{64}$ ]]; then
    ok "identity generated ($NODE_ID)"
  else
    fail "111 identity generate → '$NODE_ID' ($(head -c 200 "$LOGS/identity.log"))"
  fi
  ok "root: releases/$VER (bin/111-agent → target, bin/llama-server → stub-llama.py), $(grep -c '\.md$' "$REL/share/payload.manifest") KB entries pinned, models/current/stub.gguf, etc/env DASHBOARD_LAN=true"

  # ---------------------------------------------------------------- 4–5. namespaces
  SMOKE_WORK=$WORK BIN111=$BIN111 SMOKE_NODE_ID=$NODE_ID SMOKE_T0=$T0 \
    unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
    "${RESET_SIG[@]}" bash "$HERE/smoke.sh" --inner 2>&1 | tee "$INNER_LOG" || true
fi
inner_fails=$(grep -c '^FAIL:' "$INNER_LOG" || true)
if ! grep -q '^FAILS=' "$WORK/inner.result" 2>/dev/null; then
  fail "the namespace run did not finish (killed or crashed — see above)"
fi
FAILS=$((FAILS + inner_fails))

echo
total_ok=$((OKS + $(grep -c '^ok:' "$INNER_LOG" || true)))
if [ "$FAILS" -eq 0 ]; then
  echo "SMOKE GREEN — $total_ok checks ok, 0 FAIL, in $(elapsed)s"
  exit 0
fi
echo "SMOKE RED — $FAILS FAIL line(s) in $(elapsed)s"
exit 1
