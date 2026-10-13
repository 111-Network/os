#!/usr/bin/env bash
# interface/verify/smoke.sh — Layer 4 end-to-end smoke (spec §14, TASKS.md I1).
#
#   1. UI      pnpm install (only when node_modules is stale) + pnpm build → ui/dist
#   2. Rust    cargo build (debug) of `111` (os/bootstrap) and `111-agent` (os/intelligence).
#              A debug `111` reads ui/dist from disk per request (rust-embed debug mode),
#              so a UI rebuild needs no Rust rebuild — asserted below with a probe file.
#   3. Root    a scratch NODE111_ROOT: identity (`111 identity generate`), releases/<ver>
#              with the KB (os/knowledge entries) + payload manifest, `111-agent`, and
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
# script exits 1 on any FAIL. Every process dies with the pid namespace; the
# scratch root is removed on success (kept on failure, or with SMOKE_KEEP=1).
#
# Env: SMOKE_KEEP=1        keep the scratch root
#      SMOKE_TAPES=1       re-record the SSE tapes (default: record only missing ones,
#                          so routine runs do not rewrite the UI's test fixtures)
#      SMOKE_SKIP_BUILD=1  dev loop only: reuse ui/dist and target/ as they are
#      TMPDIR              where the scratch root goes (mktemp)
# Checks are condition strings eval'd by t() at check time, so single-quoted
# `$vars`, variables read only inside them, and helpers called only from them
# are intended:
# shellcheck disable=SC2016,SC2034,SC2317
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
OS=$(cd "$HERE/../.." && pwd)
UI=$OS/interface/ui
BOOT=$OS/bootstrap
INTEL=$OS/intelligence
TAPES=$UI/test/tapes
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
req() {
  local m=$1 u=$2
  shift 2
  : >"$W/hdr"
  : >"$W/body"
  ST=$(curl -sS -o "$W/body" -D "$W/hdr" -w '%{http_code}' -X "$m" --max-time "${REQ_TIMEOUT:-15}" "$@" "$u" 2>"$W/curl.err") || true
  [ -n "$ST" ] || ST=000
}
# hdr NAME → first value of that response header (case-insensitive, CR stripped)
hdr() {
  awk -v n="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')" '
    { line=$0; sub(/\r$/, "", line); k=tolower(line); sub(/:.*/, "", k) }
    k==n { sub(/^[^:]*:[ \t]*/, "", line); print line; exit }' "$W/hdr"
}
ct_is() { case "$(hdr content-type)" in "$1"*) return 0 ;; *) return 1 ;; esac }
jb() { jq -e "$@" "$W/body" >/dev/null 2>&1; }
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
# tape SRC NAME — save a captured stream as ui/test/tapes/NAME (only when missing
# or SMOKE_TAPES=1); the scratch root path is rewritten to the production root
TAPED=""
tape() {
  [ -s "$1" ] || return 0
  mkdir -p "$TAPES"
  if [ "${SMOKE_TAPES:-0}" = 1 ] || [ ! -s "$TAPES/$2" ]; then
    sed "s#$ROOT#/var/lib/111#g" "$1" >"$TAPES/$2"
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
  t "GET /api/v1/auth/session from loopback → loopback:true" '[ "$ST" = 200 ] && jb ".loopback == true"'

  req GET "$B/api/v1/services"
  t "GET /api/v1/services → 7 records, ids agent,codeagent,gateway,kb,model,radio,transport" \
    '[ "$ST" = 200 ] && [ "$(jget -c "[.services[].id] | sort")" = "[\"agent\",\"codeagent\",\"gateway\",\"kb\",\"model\",\"radio\",\"transport\"]" ]'
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
    'grep "^data: " "$W/events-replay.sse" | sed "s/^data: //" | jq -e "select(.node.alias? == \"smoke\")" >/dev/null 2>&1'
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
  t "\`111 dashboard\` pair code = state/dashboard-pair.json" '[ "$code" = "$filecode" ]'
  t "\`111 dashboard\` prints a QR pair link (LAN on)" 'grep -q "#/pair?code=$code" "$W/cli.out"'
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

  req POST "$L/api/v1/auth/logout" "${lan[@]}" -b "$jar" -c "$jar" -H 'Content-Type: application/json' -d '{}'
  t "LAN POST /auth/logout → 204" '[ "$ST" = 204 ]'
  req GET "$L/api/v1/status" "${lan[@]}" -H "Cookie: s111=${token:-none}"
  t "the old cookie is revoked server-side → 401" 'problem_is 401 unauthenticated'

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
    'printf "%s" "$done1" | jq -e --arg r "$toks" ".conversation_id == \"conv-smoke-1\" and .reply == \$r and .tool_trace == []" >/dev/null'
  tape "$W/chat.sse" chat.sse

  curl -sN --max-time 60 -o "$W/chat-tool.sse" -H "$cj" \
    -d '{"conversation_id":"conv-smoke-2","message":"what is meshtastic?"}' "$B/api/v1/chat" || true
  t "tool turn → event: tool {name:kb.read} then done with tool_trace ($(events_in "$W/chat-tool.sse"))" \
    'awk "BEGIN{RS=\"\"} /event: tool/" "$W/chat-tool.sse" | grep -q "\"name\":\"kb.read\"" && sed -n "/^event: done/,/^$/p" "$W/chat-tool.sse" | sed -n "s/^data: //p" | jq -e ".tool_trace | any(.name == \"kb.read\")" >/dev/null'
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
    '{schema:1, id:"prop-smoke", node_id:$nid, task_id:null, kind:"propose", title:"Smoke proposal", body:"## Smoke\nwritten by interface/verify/smoke.sh", created_at:$now}' \
    >"$pdir/prop-smoke.json"
  req GET "$B/api/v1/proposals"
  t "GET /api/v1/proposals → includes prop-smoke (open, decision:null)" \
    '[ "$ST" = 200 ] && jb ".proposals | any(.id == \"prop-smoke\" and .decision == null)"'
  req GET "$B/api/v1/proposals/prop-smoke"
  t "GET /api/v1/proposals/prop-smoke → 200 Proposal" '[ "$ST" = 200 ] && jb ".id == \"prop-smoke\" and .title == \"Smoke proposal\""'
  req POST "$B/api/v1/proposals/prop-smoke/decision" -H "$cj" -d '{"decision":"accepted"}'
  t "POST /proposals/prop-smoke/decision {accepted} → 200 decided" '[ "$ST" = 200 ] && jb ".decision.decision == \"accepted\""'
  t "decision recorded in agent/decisions/prop-smoke.json" 'jq -e ".decision == \"accepted\"" "$ROOT/agent/decisions/prop-smoke.json" >/dev/null 2>&1'
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
    note "tapes recorded into ui/test/tapes:$TAPED"
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
  t "the old llama-server process is gone" '! kill -0 "$mpid0" 2>/dev/null'
  req GET "$B/api/v1/services/transport"
  t "transport (detection-only stub here) offers no restart ($(jget .reason_code), actions $(jget -c .actions))" \
    'jb "(.actions | index(\"restart\")) == null"'
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
  t "the daemon stopped its children on exit (left: ${left:-none})" '[ -z "$left" ]'
  note "daemon log tail:"
  tail -8 "$ROOT/daemon.log" | sed 's/^/  daemon| /'
}

# ------------------------------------------------------------------ outer

if [ "${1:-}" = --inner ]; then
  inner
  exit $((FAILS > 0))
fi

for tool in curl jq python3 unshare ip sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "FAIL: $tool not found — the smoke needs it"
    exit 1
  }
done

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

echo "interface/verify/smoke.sh — Layer 4 end-to-end (scratch $WORK)"

# ------------------------------------------------------------------ 1. UI
phase "1. UI build"
if [ "${SMOKE_SKIP_BUILD:-0}" = 1 ]; then
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
if [ -f "$UI/dist/index.html" ]; then ok "ui: dist/index.html exists"; else fail "ui: dist/index.html missing"; fi
if compgen -G "$UI/dist/assets/*.js" >/dev/null; then
  ok "ui: dist/assets/*.js present ($(find "$UI/dist/assets" -name '*.js' | wc -l) files)"
else
  fail "ui: no dist/assets/*.js"
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
if [ "$BIN111" -ot "$UI/dist/index.html" ]; then
  note "111 binary is older than ui/dist — served UI comes from disk (debug rust-embed), checked below"
fi

# ------------------------------------------------------------------ 3. scratch root
phase "3. scratch node root"
ROOT=$WORK/root
VER=2026.10.13
REL=$ROOT/releases/$VER
KB=$REL/share/kb
mkdir -p "$REL/bin" "$KB" "$ROOT/models/current" "$ROOT/etc" "$ROOT/state"
ln -s "releases/$VER" "$ROOT/current"
ln -s "$BINAGENT" "$REL/bin/111-agent"
cat >"$REL/bin/llama-server" <<EOF
#!/bin/sh
# smoke stand-in for llama-server: same flags (--model --host --port), stub replies
exec python3 -I "$HERE/stub-llama.py" "\$@"
EOF
chmod 755 "$REL/bin/llama-server"
printf 'GGUF smoke placeholder — never loaded (the stub ignores --model)\n' >"$ROOT/models/current/stub.gguf"
# KB = the knowledge tree's entries (what a release packs into share/kb), no _meta/docs
(cd "$OS/knowledge" && find . -path ./_meta -prune -o -type f \( -name '*.md' -o -name 'llms.txt' \) -print) |
  while read -r f; do
    case $f in ./AGENTS.md | ./README.md | ./TASKS.md) continue ;; esac
    mkdir -p "$KB/$(dirname "$f")"
    cp "$OS/knowledge/$f" "$KB/$f"
  done
(cd "$REL" && find share/kb -type f -print0 | sort -z | xargs -0 sha256sum) >"$REL/share/payload.manifest"
printf '# smoke: LAN sharing on so pairing auth is exercised\nDASHBOARD_LAN=true\n' >"$ROOT/etc/env"
NODE_ID=$(NODE111_ROOT=$ROOT "$BIN111" identity generate 2>"$LOGS/identity.log" | tail -1) || true
if [[ "$NODE_ID" =~ ^[0-9a-f]{64}$ ]]; then
  ok "identity generated ($NODE_ID)"
else
  fail "111 identity generate → '$NODE_ID' ($(head -c 200 "$LOGS/identity.log"))"
fi
ok "root: releases/$VER (bin/111-agent → target, bin/llama-server → stub-llama.py), $(grep -c '\.md$' "$REL/share/payload.manifest") KB entries pinned, models/current/stub.gguf, etc/env DASHBOARD_LAN=true"

# ------------------------------------------------------------------ 4–5. namespaces
INNER_LOG=$WORK/inner.log
# reset the SIGINT/SIGTERM that unshare ignores in its child (see inner())
if env --default-signal true 2>/dev/null; then
  RESET_SIG=(env --default-signal)
else
  RESET_SIG=(python3 -c 'import os,signal,sys
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])')
fi
SMOKE_WORK=$WORK BIN111=$BIN111 SMOKE_NODE_ID=$NODE_ID SMOKE_T0=$T0 \
  unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
  "${RESET_SIG[@]}" bash "$HERE/smoke.sh" --inner 2>&1 | tee "$INNER_LOG" || true
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
