#!/usr/bin/env bash
# 4-interface/verify/panel-lint.sh — panel offline lint + G3 (os/5-messaging/README.md section 13 "Offline lint", section 20 G3,
# TASKS.md V2).
#
#   0. Self-test   cdp/selftest.mjs: the lint's rules flag every hostile mutation of a clean recording (no browser).
#   1. Build       pnpm build (ui/dist) + cargo build --bin 111 (debug; rust-embed reads ui/dist from disk).
#   2. Root        a scratch NODE111_ROOT with: the Meshtastic panel bundle at current/panels/meshtastic-web (the real
#                  bundle when PANEL_LINT_BUNDLE=<dir> names one, else the placeholder under cdp/placeholder-bundle that
#                  has the same API shape), a Meshtastic binding in state/devices.json (tcp uplink to a fake board), and
#                  the stub meshmud (stub-meshmud.py behind current/bin/meshmud) + a fake presence reader, so the daemon's
#                  own supervised children produce a LIVE `meshtastic` service and an enabled panel — the real path.
#   3. Run         inside `unshare -rnpf` (user + net + pid namespaces: loopback + dummy0 192.168.77.5 and .9, no
#                  internet): `111 daemon`, then
#                    a. the HTTP matrix with curl (ticket swap, cookie attributes, guards, headers, shim round trip,
#                       dequeue-nothing, drive-by POST, upgrade),
#                    b. headless Chrome over CDP (cdp/panel-lint.mjs): dashboard on the LAN address, pair, Panel tab,
#                       ticket into the panel origin, 10 s of the panel running, every Network request recorded and the
#                       G3 list asserted; the same on http://localhost (the 308); a hostile canary bundle the lint must
#                       flag; probes from other origins; the table of requests at the end.
#
# Every check prints `ok:` / `FAIL:` / `note:`; the script exits 1 on any FAIL. Every process dies with the pid
# namespace; the scratch root is removed on success (kept on failure, or with PANEL_LINT_KEEP=1).
#
# Env: PANEL_LINT_KNOWN_GAPS=1     checks labelled "GAP (blocked-by …)" are defects already reported to their owner; by default
#                                   they FAIL (the gate stays red until the fix lands), with this they print as notes
#      PANEL_LINT_BUNDLE=<dir>      lint a real Meshtastic web client build (meshtastic-web-build.sh output) instead of
#                                   the placeholder; the placeholder run does NOT replace that external gate
#      PANEL_LINT_DWELL=10          seconds the panel runs in the frame
#      PANEL_LINT_SKIP_BUILD=1      dev loop only: reuse ui/dist and target/ as they are
#      PANEL_LINT_KEEP=1            keep the scratch root
#      CHROME=<path>                the browser (default google-chrome, then playwright's chromium)
#      TMPDIR                       where the scratch root goes
# Checks are condition strings eval'd by t() at check time (the same idiom as smoke.sh):
# shellcheck disable=SC2016,SC2034,SC2317
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
OS=$(cd "$HERE/../.." && pwd)
UI=$OS/4-interface/ui
BOOT=$OS/1-bootstrap
CDP=$HERE/cdp
STUB=$HERE/stub-meshmud.py
MANIFEST=$BOOT/crates/node111/assets/panels/meshtastic.toml
export PATH="$HOME/.cargo/bin:$PATH:/usr/sbin:/sbin"

LANIP=192.168.77.5
CROSSIP=192.168.77.9
B=http://127.0.0.1:11180
DWELL=${PANEL_LINT_DWELL:-10}
KNOWN_GAPS=${PANEL_LINT_KNOWN_GAPS:-0}

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
T0=${PANEL_LINT_T0:-${EPOCHREALTIME/,/.}}
elapsed() { awk -v a="$T0" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}'; }
phase() { printf '\n== %s  (t+%ss)\n' "$*" "$(elapsed)"; }

# ------------------------------------------------------------------ helpers (inner)

# req METHOD URL [curl args…] → $ST (status, 000 = no answer), $W/hdr, $W/body
req() {
  local m=$1 u=$2
  shift 2
  : >"$W/hdr"
  : >"$W/body"
  ST=$(curl -sS -o "$W/body" -D "$W/hdr" -w '%{http_code}' -X "$m" --max-time "${REQ_TIMEOUT:-10}" "$@" "$u" 2>"$W/curl.err") || true
  [ -n "$ST" ] || ST=000
}
# hdr NAME → first value of that response header (case-insensitive, CR stripped)
hdr() {
  awk -v n="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')" '
    { line=$0; sub(/\r$/, "", line); k=tolower(line); sub(/:.*/, "", k) }
    k==n { sub(/^[^:]*:[ \t]*/, "", line); print line; exit }' "$W/hdr"
}
# hdr_all NAME → every value, one per line
hdr_all() {
  awk -v n="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')" '
    { line=$0; sub(/\r$/, "", line); k=tolower(line); sub(/:.*/, "", k) }
    k==n { sub(/^[^:]*:[ \t]*/, "", line); print line }' "$W/hdr"
}
ct_is() { case "$(hdr content-type)" in "$1"*) return 0 ;; *) return 1 ;; esac; }
# an empty body is not a match: `jq -e` exits 0 on empty input
jb() { [ -s "$W/body" ] && jq -e "$@" "$W/body" >/dev/null 2>&1; }
jget() { jq -r "$@" "$W/body" 2>/dev/null || true; }
snip() { head -c 200 "$W/body" 2>/dev/null | tr '\n\r' '  '; }
t() {
  if eval "$2"; then ok "$1"; else fail "$1  [HTTP ${ST:-?}: $(snip)]"; fi
}
# tg DESC OWNER CONDITION — like t(), for a defect that is reported and waits for its owner's fix: a FAIL by default (the gate
# is red until it is fixed), a loud note under PANEL_LINT_KNOWN_GAPS=1 (everything else stays strict)
tg() {
  if eval "$3"; then
    ok "$1"
  elif [ "$KNOWN_GAPS" = 1 ]; then
    note "KNOWN GAP ($2): $1  [HTTP ${ST:-?}: $(snip)]"
  else
    fail "GAP ($2): $1  [HTTP ${ST:-?}: $(snip)]"
  fi
}
problem_is() { [ "$ST" = "$1" ] && ct_is application/problem+json && [ "$(jget .code)" = "$2" ]; }
no_cors() { ! grep -qi '^access-control-' "$W/hdr"; }
# lan_ports_ok "PORTS…" PANEL_PORT — every LAN-reachable listener is one we know (80 is the dashboard's alias, when it bound)
lan_ports_ok() {
  local q
  for q in $1; do
    case $q in 80 | 11180 | 11300 | "$2") ;; *) return 1 ;; esac
  done
  [[ " $1" == *" 11180 "* && " $1" == *" $2 "* ]]
}
# panel_policy ORIGIN_HOST → the response policy of §13 holds on the last response
panel_policy() {
  local h=$1 csp
  csp=$(hdr_all content-security-policy | tr '\n' ',')
  [ -z "$(hdr x-frame-options)" ] &&
    [[ $csp == *"frame-ancestors http://$h:11180 http://$h:80"* ]] &&
    [ "$(hdr x-content-type-options)" = nosniff ] && [ "$(hdr referrer-policy)" = no-referrer ] && no_cors
}

# ------------------------------------------------------------------ inner (in the namespaces)

inner() {
  : "${PL_WORK:?}" "${BIN111:?}"
  W=$PL_WORK/http
  ROOT=$PL_WORK/root
  REL=$ROOT/releases/${PL_VER:?}
  mkdir -p "$W"
  DPID=""
  cleanup_inner() {
    if [ -n "$DPID" ] && kill -0 "$DPID" 2>/dev/null; then
      kill -TERM "$DPID" 2>/dev/null || true
      for _ in 1 2 3 4 5 6 7 8 9 10; do
        kill -0 "$DPID" 2>/dev/null || break
        sleep 0.3
      done
    fi
    # leaving pid 1 of the namespace kills whatever is left (board, attacker server, chrome, stubs)
    printf 'FAILS=%s OKS=%s\n' "$FAILS" "$OKS" >"$PL_WORK/inner.result"
  }
  trap cleanup_inner EXIT
  trap 'exit 130' INT TERM

  phase "3. namespace + node"
  local sigign
  sigign=$(awk '/^SigIgn:/ {print $2}' /proc/self/status)
  t "namespace shell has default SIGINT/SIGTERM (SigIgn=$sigign)" '(( (16#$sigign & 0x4002) == 0 ))'
  ip link set lo up
  if ip link add dummy0 type dummy && ip addr add "$LANIP/24" dev dummy0 && ip addr add "$CROSSIP/24" dev dummy0 && ip link set dummy0 up; then
    ok "netns: lo up, dummy0 $LANIP/24 + $CROSSIP/24 (a second address = a second site; no route off-box)"
  else
    fail "netns: cannot create dummy0 — the LAN scenario cannot run"
  fi
  t "the namespace has no way out: no default route (ip route: $(ip route | tr '\n' ' '))" '! ip route | grep -q "^default"'

  # the fake board (the uplink of the binding), the other-origin server the probes use
  python3 -I "$STUB" --role board --listen 127.0.0.1:14403 >"$W/board.log" 2>&1 &
  python3 -I -m http.server 11300 --bind 0.0.0.0 --directory "$CDP/attacker" >"$W/attacker.log" 2>&1 &
  for _ in $(seq 1 50); do
    curl -s -o /dev/null --max-time 1 "http://127.0.0.1:11300/" && break
    sleep 0.1
  done
  t "other-origin test server answers on :11300 ($LANIP and $CROSSIP)" \
    'curl -s -o /dev/null --max-time 2 "http://$LANIP:11300/" && curl -s -o /dev/null --max-time 2 "http://$CROSSIP:11300/"'

  local d0=${EPOCHREALTIME/,/.}
  NODE111_ROOT=$ROOT "$BIN111" daemon >"$ROOT/daemon.log" 2>&1 &
  DPID=$!
  local up=0 code
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

  # ---------------------------------------------------------------- the Meshtastic service goes LIVE by itself
  phase "3a. meshtastic LIVE (stub meshmud + fake reader under the daemon's own supervision)"
  local state="" listening="" i
  for i in $(seq 1 120); do
    req GET "$B/api/v1/services/meshtastic"
    state=$(jget .state)
    listening=$(jget .panel.state.listening)
    # the panel listener comes up on the registry's next 2 s tick after the service leaves NOT SET
    [ "$state" = live ] && [ "$listening" = true ] && break
    sleep 0.5
  done
  if [ "$state" = live ] && [ "$listening" = true ]; then
    ok "service meshtastic is LIVE and its panel listener is up after ~$((i / 2)) s ($(jget .reason_code): nodes heard $(jget .detail.nodes_heard), board $(jget .detail.board))"
  else
    fail "service meshtastic never went LIVE (state $state, reason $(jget .reason_code): $(jget .reason))"
    note "daemon log tail:"
    tail -25 "$ROOT/daemon.log" | sed 's/^/  daemon| /'
    note "meshmud log tail:"
    tail -15 "$ROOT/meshtastic/meshmud.log" 2>/dev/null | sed 's/^/  meshmud| /'
    exit 1
  fi
  t "the service record carries the panel (enabled, listening, health up): $(jget -c '.panel | {port, enabled: .state.enabled, listening: .state.listening, health: .state.health}')" \
    'jb ".panel.id == \"meshtastic\" and .panel.state.enabled == true and .panel.state.listening == true"'
  PANEL_PORT=$(jget .panel.port)
  PH=127.0.0.1
  P=http://$PH:$PANEL_PORT
  t "panel slot is in the 11190–11229 block and persisted in state/panels.json ($PANEL_PORT)" \
    '[ "$PANEL_PORT" -ge 11190 ] && [ "$PANEL_PORT" -le 11229 ] && [ "$(jq -r .slots.meshtastic "$ROOT/state/panels.json")" = "$PANEL_PORT" ]'
  local dash_binds panel_binds
  dash_binds=$(ss -ltnH | awk '{print $4}' | sed -n 's/:11180$//p' | sort | tr '\n' ' ')
  panel_binds=$(ss -ltnH | awk -v p=":$PANEL_PORT" '{ if (substr($4, length($4) - length(p) + 1) == p) { sub(p "$", "", $4); print $4 } }' | sort | tr '\n' ' ')
  t "the panel listener is bound exactly where the dashboard is (dashboard: ${dash_binds:-?}; panel: ${panel_binds:-?})" \
    '[ -n "$dash_binds" ] && [ "$dash_binds" = "$panel_binds" ]'
  t "meshmud's downlink LISTENs on 127.0.0.1:11230 and nowhere else (no HTTP downlink; 111d's shim is the only HTTP surface for the radio)" \
    'ss -ltn | grep -q "127.0.0.1:11230 " && [ "$(ss -ltnH | grep -c ":11230 ")" = 1 ]'
  local lan_ports
  lan_ports=$(ss -ltnH | awk '$4 !~ /^(127\.0\.0\.1|\[::1\])/ {n=split($4, a, ":"); print a[n]}' | sort -un | tr '\n' ' ')
  t "the only sockets reachable from the LAN are the dashboard (:11180, :80), the panel ($PANEL_PORT) and this lint's test server (:11300): $lan_ports" \
    'lan_ports_ok "$lan_ports" "$PANEL_PORT"'
  req GET "$B/api/v1/panels"
  t "GET /api/v1/panels lists meshtastic with allow [serial, bluetooth] and the declared optional hosts" \
    'jb "[.panels[] | select(.id == \"meshtastic\")] | length == 1 and (.[0].allow == [\"serial\",\"bluetooth\"]) and (.[0].optional_hosts | length == 2)"'
  req GET "$B/"
  local dcsp
  dcsp=$(hdr content-security-policy)
  t "dashboard CSP on 127.0.0.1 has frame-src $P and frame-ancestors 'none' ($dcsp)" \
    '[[ $dcsp == *"frame-src $P"* && $dcsp == *"frame-ancestors '"'"'none'"'"'"* ]] && [ "$(hdr x-frame-options)" = DENY ]'

  # ---------------------------------------------------------------- the HTTP matrix (curl, loopback)
  phase "3b. panel HTTP matrix (curl)"
  local cj='Content-Type: application/json' tok ticket url
  req POST "$B/api/v1/panels/meshtastic/session" -H "$cj" -d '{}'
  url=$(jget .url)
  t "POST /api/v1/panels/meshtastic/session → 200 {url, expires_in:30} on the panel origin ($(jget .expires_in) s)" \
    '[ "$ST" = 200 ] && [[ $url == "$P/"* ]] && [ "$(jget .expires_in)" = 30 ]'
  ticket=$(printf '%s' "$url" | sed -n 's/.*p111_ticket=\([0-9a-f]*\).*/\1/p')
  t "the ticket is 128-bit hex (${#ticket} chars) in the p111_ticket parameter" '[[ $ticket =~ ^[0-9a-f]{32}$ ]]'

  req GET "$P/"
  t "no cookie: GET / → 401 unauthenticated (there is no loopback skip)" 'problem_is 401 unauthenticated'
  t "…and the refusal already carries the response policy (no XFO, nosniff, no-referrer, no CORS, frame-ancestors)" 'panel_policy $PH'

  req GET "$url"
  local sc loc
  sc=$(hdr set-cookie)
  loc=$(hdr location)
  t "ticket swap: GET <ticket url> → 303 Location: / ($loc)" '[ "$ST" = 303 ] && [ "$loc" = "/" ]'
  t "…Set-Cookie: p111_meshtastic=<hex>; HttpOnly; SameSite=Strict; Path=/ (no Domain, no Max-Age/Expires, no Secure)" \
    '[[ $sc =~ ^p111_meshtastic=[0-9a-f]{32,}\;\ HttpOnly\;\ SameSite=Strict\;\ Path=/$ ]]'
  t "…Cache-Control: no-store on the swap and the response policy holds" '[ "$(hdr cache-control)" = no-store ] && panel_policy $PH'
  tok=${sc#p111_meshtastic=}
  tok=${tok%%;*}
  local ck="Cookie: p111_meshtastic=$tok"

  req GET "$url"
  t "spent ticket replayed without a cookie → 403 panel_ticket_expired, no Set-Cookie" 'problem_is 403 panel_ticket_expired && [ -z "$(hdr set-cookie)" ]'
  req POST "$B/api/v1/panels/meshtastic/session" -H "$cj" -d '{}'
  local url2
  url2=$(jget .url)
  req GET "$url2" -H "$ck"
  t "a fresh ticket presented with a valid cookie is ignored (and spent) → 303 to the entry, no new cookie" '[ "$ST" = 303 ] && [ "$(hdr location)" = "/" ] && [ -z "$(hdr set-cookie)" ]'
  req GET "$url2"
  t "…and that ticket is spent for everyone → 403 panel_ticket_expired" 'problem_is 403 panel_ticket_expired'

  req GET "$P/" -H "$ck"
  t "with the cookie: GET / → 200 text/html, the bundle's index.html byte-for-byte (served unmodified)" \
    '[ "$ST" = 200 ] && ct_is text/html && [ "$(sha256sum <"$W/body" | cut -c1-64)" = "$(sha256sum <"$BUNDLE_SERVED/index.html" | cut -c1-64)" ]'
  t "…response policy: frame-ancestors = the dashboard origins on this host, nosniff, no-referrer, no X-Frame-Options, no Access-Control-*" 'panel_policy $PH'
  req GET "$P/" -H "$ck" -H 'Origin: http://evil.example' -H 'Sec-Fetch-Site: same-origin'
  t "a request that offers an Origin still gets no Access-Control-* back" '[ "$ST" = 200 ] && no_cors'
  req GET "$P/some/client/route" -H "$ck" -H 'Accept: text/html'
  t "SPA fallback: an unknown client route serves index.html (200)" '[ "$ST" = 200 ] && ct_is text/html'
  req GET "$P/api/v1/nope" -H "$ck" -H 'Sec-Fetch-Site: same-origin'
  t "…but never under an API path: GET /api/v1/nope → problem+json, not index.html" '! grep -qi "<html" "$W/body" && case $ST in 4* | 5*) true ;; *) false ;; esac'
  req GET "$P/app.js" -H "$ck" -H 'Sec-Fetch-Site: same-site' 2>/dev/null || true
  t "static assets from the bundle are served with their own media type (app.js → text/javascript)" '[ "$ST" = 200 ] && ct_is text/javascript'

  # guards
  req GET "$P/" -H "$ck" -H 'Host: localhost:'"$PANEL_PORT"
  t "Host: localhost:$PANEL_PORT → 400 (panels never see localhost; the main listener 308s it)" '[ "$ST" = 400 ]'
  req GET "$P/" -H "$ck" -H 'Host: evil.example:'"$PANEL_PORT"
  t "Host: evil.example → 400" '[ "$ST" = 400 ]'
  req GET "$P/" -H "$ck" -H 'Host: 127.0.0.1:11180'
  t "the dashboard's authority on the panel's port is not the panel's → 400" '[ "$ST" = 400 ]'
  req GET "http://localhost:11180/" --resolve localhost:11180:127.0.0.1
  t "the main listener answers Host: localhost with a 308 to http://127.0.0.1:11180/ ($(hdr location))" '[ "$ST" = 308 ] && [ "$(hdr location)" = "http://127.0.0.1:11180/" ]'
  req OPTIONS "$P/api/v1/toradio" -H "$ck" -H 'Origin: http://127.0.0.1:11180' -H 'Access-Control-Request-Method: PUT'
  t "OPTIONS (a preflight) → 403, no Access-Control-* (a panel's own JS is same-origin; there is no legitimate preflight)" '[ "$ST" = 403 ] && no_cors'
  req GET "$P/" -H "$ck" -H 'Sec-Fetch-Site: cross-site'
  t "Sec-Fetch-Site: cross-site on a page → 403" '[ "$ST" = 403 ]'
  req GET "$P/ws" -H "$ck" -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H "Origin: $P" -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ=='
  t "a WebSocket upgrade is refused → 400 (no WebSocket in v1)" '[ "$ST" = 400 ]'

  # the shim round trip, then the cross-site / same-site / drive-by attempts that must dequeue and send nothing
  local same=(-H "$ck" -H "Origin: $P" -H 'Sec-Fetch-Site: same-origin')
  req PUT "$P/api/v1/toradio" "${same[@]}" -H 'Content-Type: application/x-protobuf' --data-binary 'want_config'
  t "shim: PUT /api/v1/toradio (want_config, same-origin) → 200, empty body" '[ "$ST" = 200 ] && [ ! -s "$W/body" ]'
  sleep 0.5
  req GET "$P/api/v1/fromradio?all=true" "${same[@]}"
  t "shim: GET /api/v1/fromradio?all=true → 200 application/x-protobuf, frame 1 = my_node_info" \
    '[ "$ST" = 200 ] && ct_is application/x-protobuf && grep -q my_node_info "$W/body"'
  local n
  for n in 1 2 3; do
    req GET "$P/api/v1/fromradio?all=true" "${same[@]}"
    t "shim: frame $((n + 1)) = node_info #$n (one frame per call, whatever 'all' says; the board's debug text is skipped)" \
      '[ "$ST" = 200 ] && jb ".node_info.num > 0 and (.node_info.long_name | length > 0)"'
  done
  req GET "$P/api/v1/fromradio?all=true" "${same[@]}"
  t "shim: frame 5 = config_complete_id" '[ "$ST" = 200 ] && jb ".config_complete_id == 1"'
  req GET "$P/api/v1/fromradio?all=true" "${same[@]}"
  t "shim: then an empty 200 (the client polls until empty)" '[ "$ST" = 200 ] && [ ! -s "$W/body" ]'
  req PUT "$P/api/v1/toradio" "${same[@]}" --data-binary 'hello-lint'
  sleep 0.3
  req GET "$P/api/v1/fromradio?all=true" "${same[@]}"
  t "shim: bytes survive the reframing in both directions (echo of hello-lint = 68656c6c6f2d6c696e74)" '[ "$ST" = 200 ] && [ "$(jget .echo_hex)" = 68656c6c6f2d6c696e74 ]'

  req PUT "$P/api/v1/toradio" "${same[@]}" --data-binary 'want_config'
  sleep 0.5
  req GET "$P/api/v1/fromradio?all=true" -H "$ck" -H 'Sec-Fetch-Site: cross-site' -H 'Origin: http://192.168.77.9:11300'
  t "cross-site GET /fromradio (with the victim's cookie) → 403, no frame in the body, no Access-Control-*" \
    '[ "$ST" = 403 ] && ! grep -q my_node_info "$W/body" && no_cors'
  req GET "$P/api/v1/fromradio?all=true" -H "$ck" -H 'Sec-Fetch-Site: same-site' -H 'Origin: http://127.0.0.1:11180'
  t "same-site (another origin on this host, e.g. the dashboard) GET /fromradio with the cookie and Sec-Fetch-Site: same-site (what Chrome sends to 127.0.0.1) → 403, no Access-Control-*" \
    '[ "$ST" = 403 ] && ! grep -q my_node_info "$W/body" && no_cors'
  req GET "$P/api/v1/fromradio?all=true"
  t "no cookie: GET /fromradio → 401" 'problem_is 401 unauthenticated'
  req POST "$P/api/v1/toradio" -H "$ck" -H 'Origin: http://evil.example' -H 'Content-Type: text/plain' --data-binary 'DRIVEBY-MARKER'
  t "drive-by POST /toradio with a foreign Origin → 403" '[ "$ST" = 403 ]'
  req POST "$P/api/v1/toradio" -H "$ck" -H 'Content-Type: text/plain' --data-binary 'DRIVEBY-MARKER'
  t "drive-by POST with neither Origin nor Referer → 403 (a guarded request must name its origin)" '[ "$ST" = 403 ]'
  req PUT "$P/api/v1/toradio" -H "$ck" -H 'Origin: http://evil.example' --data-binary 'DRIVEBY-MARKER'
  t "drive-by PUT with a foreign Origin → 403" '[ "$ST" = 403 ]'
  req GET "$P/api/v1/fromradio?all=true" "${same[@]}"
  t "…and nothing was dequeued: the legitimate GET still gets frame 1 (my_node_info)" '[ "$ST" = 200 ] && grep -q my_node_info "$W/body"'
  sleep 0.3
  t "…and nothing was sent: the stub meshmud log never saw DRIVEBY-MARKER, but did see hello-lint" \
    '! grep -q DRIVEBY-MARKER "$ROOT/meshtastic/meshmud.log" && grep -q "frame-in .* hello-lint" "$ROOT/meshtastic/meshmud.log"'
  t "…and every frame the shim sent was a whole 0x94 0xC3 frame (no framing error in the stub's log)" \
    '! grep -q "framing-error" "$ROOT/meshtastic/meshmud.log" && grep -q "frame-in" "$ROOT/meshtastic/meshmud.log"'
  # what a browser sends from another origin on this host to http://<LAN IP>: Origin + Referer of that origin and NO Sec-Fetch-*
  # (Chrome sends Fetch Metadata only to trustworthy origins: https, localhost, 127.0.0.1). The cookie rides along (same site).
  drain() {
    local k
    for k in $(seq 1 15); do
      req GET "$P/api/v1/fromradio?all=true" "${same[@]}"
      [ -s "$W/body" ] || break
    done
  }
  drain
  req PUT "$P/api/v1/toradio" "${same[@]}" --data-binary 'want_config'
  sleep 0.5
  req GET "$P/api/v1/fromradio?all=true" -H "$ck" -H 'Origin: http://127.0.0.1:11300' -H 'Referer: http://127.0.0.1:11300/'
  tg "same-site cross-origin GET /fromradio with the cookie but no Sec-Fetch-* (a browser's request to a LAN-IP origin) → 403, not served" \
    "blocked-by P1 guard.rs: API-path GETs rely on Sec-Fetch-Site alone; also refuse a foreign Origin/Referer" \
    '[ "$ST" = 403 ] && ! grep -q my_node_info "$W/body"'
  req GET "$P/api/v1/fromradio?all=true" "${same[@]}"
  tg "…and nothing was dequeued: the next legitimate GET still gets frame 1 (my_node_info), not frame 2" \
    "blocked-by P1 guard.rs (same cause)" \
    '[ "$ST" = 200 ] && grep -q my_node_info "$W/body"'
  drain
  req GET "$P/api/v1/fromradio?all=true" -H "$ck" -H 'Referer: http://127.0.0.1:11300/'
  tg "a Referer-only cross-origin GET (an <img> on another local origin; no Origin, no Sec-Fetch-*) → 403" \
    "blocked-by P1 guard.rs (same cause)" \
    '[ "$ST" = 403 ]'
  note "stub meshmud log: $(grep -c 'frame-in' "$ROOT/meshtastic/meshmud.log") frames received, $(grep -c 'downlink conn .* accepted' "$ROOT/meshtastic/meshmud.log") downlink connections accepted"

  # ---------------------------------------------------------------- the browser
  phase "3c. headless Chrome over CDP (cdp/panel-lint.mjs)"
  local cdp_rc=0
  mkdir -p "$W/cdp"
  # --manifest is the file the daemon embeds; --root locates state/panels.json, the pair code, and the served bundle
  timeout 420 node "$CDP/panel-lint.mjs" --root "$ROOT" --out "$W/cdp" --dwell "$DWELL" \
    --lan-host "$LANIP" --cross-host "$CROSSIP" --attacker-port 11300 --manifest "$MANIFEST" \
    --tables-file "$W/cdp/tables.txt" \
    --bundle-kind "$([ -n "${PANEL_LINT_BUNDLE:-}" ] && echo real || echo placeholder)" \
    --bundle-label "${PANEL_LINT_BUNDLE:-placeholder (4-interface/verify/cdp/placeholder-bundle)}" \
    2>&1 | tee "$W/cdp.out" || cdp_rc=$?
  t "the CDP driver ran to the end (exit $cdp_rc)" '[ "$(grep -c "^DRIVER FAILS=" "$W/cdp.out")" = 1 ]'
  local left=""
  for _ in $(seq 1 25); do
    left=$(pgrep -l -x 'chrome|google-chrome|chromium' | tr '\n' ' ' || true)
    [ -z "$left" ] && break
    sleep 0.2
  done
  t "Chrome is gone after the driver (left: ${left:-none})" '[ -z "$left" ]'

  # ---------------------------------------------------------------- shutdown
  phase "3d. shutdown: every child dies with the daemon"
  kill -TERM "$DPID" 2>/dev/null || true
  local gone=0 rc=255
  for _ in $(seq 1 50); do
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
  t "daemon exits 0 within 10 s of SIGTERM (exit $rc)" '[ "$gone" = 1 ] && [ "$rc" = 0 ]'
  local kids=""
  for _ in $(seq 1 25); do
    kids=$(pgrep -af 'stub-meshmud.py --role (meshmud|presence)|start-daemon' | tr '\n' ' ' || true)
    [ -z "$kids" ] && break
    sleep 0.2
  done
  t "the daemon stopped meshmud and the reader (left: ${kids:-none})" '[ -z "$kids" ]'
  note "daemon log tail:"
  tail -6 "$ROOT/daemon.log" | sed 's/^/  daemon| /'
}

# ------------------------------------------------------------------ outer

if [ "${1:-}" = --inner ]; then
  inner
  exit $((FAILS > 0))
fi

for tool in curl jq python3 unshare ip sha256sum node ss; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "FAIL: $tool not found — the panel lint needs it"
    exit 1
  }
done
if ! node -e 'process.exit(typeof WebSocket === "function" && typeof structuredClone === "function" ? 0 : 1)' 2>/dev/null; then
  echo "FAIL: node $(node --version) has no built-in WebSocket (need Node >= 22)"
  exit 1
fi

WORK=$(mktemp -d -t 111-panel-lint.XXXXXX)
cleanup() {
  if [ "${PANEL_LINT_KEEP:-0}" = 1 ] || [ "$FAILS" -gt 0 ]; then
    echo "note: scratch kept at $WORK (root/daemon.log, root/meshtastic/meshmud.log, http/, http/cdp/*.recording.json)"
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT
LOGS=$WORK/logs
mkdir -p "$LOGS"

echo "4-interface/verify/panel-lint.sh — panel offline lint + G3 (scratch $WORK)"
if [ "$KNOWN_GAPS" = 1 ]; then
  note "PANEL_LINT_KNOWN_GAPS=1 — checks labelled GAP print as notes instead of FAIL; everything else is strict"
fi

# ------------------------------------------------------------------ 0. rules self-test
phase "0. the lint's rules can fail (cdp/selftest.mjs)"
if node "$CDP/selftest.mjs" >"$LOGS/selftest.out" 2>&1; then
  ok "selftest: $(grep -c '^ok:' "$LOGS/selftest.out") rule checks ok — a clean recording passes, each hostile mutation is flagged by its rule"
else
  fail "selftest: the rules do not discriminate (log: $LOGS/selftest.out)"
  grep '^FAIL' "$LOGS/selftest.out" | head -20
fi

# ------------------------------------------------------------------ 1. builds
phase "1. builds"
if [ "${PANEL_LINT_SKIP_BUILD:-0}" = 1 ]; then
  note "PANEL_LINT_SKIP_BUILD=1 — UI and cargo builds skipped (dev loop)"
else
  if ! command -v pnpm >/dev/null 2>&1; then
    fail "pnpm not on PATH — cannot build the UI"
  else
    mods=$UI/node_modules/.modules.yaml
    if ! { [ -f "$mods" ] && [ "$mods" -nt "$UI/pnpm-lock.yaml" ] && [ "$mods" -nt "$UI/package.json" ]; }; then
      pnpm --dir "$UI" install --frozen-lockfile >"$LOGS/pnpm-install.log" 2>&1 || fail "ui: pnpm install (log: $LOGS/pnpm-install.log)"
    fi
    if pnpm --dir "$UI" build >"$LOGS/pnpm-build.log" 2>&1; then
      ok "ui: pnpm build"
    else
      fail "ui: pnpm build (log: $LOGS/pnpm-build.log)"
      tail -15 "$LOGS/pnpm-build.log" | sed 's/^/  vite| /'
    fi
  fi
  t1=${EPOCHREALTIME/,/.}
  if (cd "$BOOT" && cargo build --bin 111) >"$LOGS/cargo-111.log" 2>&1; then
    ok "cargo build --bin 111 ($(awk -v a="$t1" -v b="${EPOCHREALTIME/,/.}" 'BEGIN{printf "%.1f", b-a}')s)"
  else
    fail "cargo build --bin 111 (log: $LOGS/cargo-111.log)"
    tail -20 "$LOGS/cargo-111.log" | sed 's/^/  cargo| /'
  fi
fi
BIN111=${CARGO_TARGET_DIR:-$BOOT/target}/debug/111
[ -x "$BIN111" ] || {
  fail "binary missing: $BIN111"
  exit 1
}
if [ -f "$UI/dist/index.html" ]; then ok "ui: dist/index.html exists"; else fail "ui: dist/index.html missing (run pnpm build)"; fi

# ------------------------------------------------------------------ 2. scratch root
phase "2. scratch node root"
ROOT=$WORK/root
VER=2026.10.14
REL=$ROOT/releases/$VER
BUNDLE_SRC=${PANEL_LINT_BUNDLE:-$CDP/placeholder-bundle}
BUNDLE_SERVED=$REL/panels/meshtastic-web
mkdir -p "$REL/bin" "$REL/share/meshtastic-presence" "$BUNDLE_SERVED" "$ROOT/etc" "$ROOT/state" "$ROOT/venvs/presence/bin"
ln -s "releases/$VER" "$ROOT/current"
if [ ! -f "$BUNDLE_SRC/index.html" ]; then
  fail "bundle $BUNDLE_SRC has no index.html"
  exit 1
fi
cp -R "$BUNDLE_SRC"/. "$BUNDLE_SERVED"/
# the stub Meshtastic stack, in the paths 111d's production config uses
cat >"$REL/bin/meshmud" <<EOF
#!/bin/sh
# panel-lint stand-in for meshmud: the same CLI (start-daemon -config <file>), a TCP downlink that speaks the
# 0x94 0xC3 framing and answers want_config with my_node_info + a node list
exec python3 -I "$STUB" --role meshmud "\$@"
EOF
chmod 755 "$REL/bin/meshmud"
cat >"$REL/share/meshtastic-presence/meshtastic_presence.py" <<EOF
# panel-lint stand-in for the presence reader (venvs/presence/bin/python -I <this> --endpoint H:P --out DIR)
import runpy, sys
sys.argv = ["$STUB", "--role", "presence"] + sys.argv[1:]
runpy.run_path("$STUB", run_name="__main__")
EOF
ln -s "$(command -v python3)" "$ROOT/venvs/presence/bin/python"
# the binding the identify step would have written: a Meshtastic board reached over TCP (the fake board on :14403)
cat >"$ROOT/state/devices.json" <<'EOF'
{"schema": 1, "devices": {"lintboard0001": {
  "first_seen": "2026-10-01T00:00:00Z", "last_seen": "2026-10-01T00:00:00Z",
  "firmware": {"family": "meshtastic", "version": "2.7.11.abcdef0", "source": "probe"},
  "meshtastic": {"uplink": {"kind": "tcp", "host": "127.0.0.1", "port": 14403},
                 "node_num": 168430090, "node_id": "!0a0a0a0a", "hw_model": "HELTEC_V3", "long_name": "Lint board"}}}}
EOF
printf '# panel-lint: LAN sharing on so the dashboard is reached as a LAN visitor and pairing is exercised\nDASHBOARD_LAN=true\n' >"$ROOT/etc/env"
NODE_ID=$(NODE111_ROOT=$ROOT "$BIN111" identity generate 2>"$LOGS/identity.log" | tail -1) || true
if [[ "$NODE_ID" =~ ^[0-9a-f]{64}$ ]]; then ok "identity generated ($NODE_ID)"; else fail "111 identity generate → '$NODE_ID' ($(head -c 200 "$LOGS/identity.log"))"; fi
ok "root: current/panels/meshtastic-web ← $(basename "$BUNDLE_SRC") ($(find "$BUNDLE_SERVED" -type f | wc -l) files), bin/meshmud → stub-meshmud.py, venvs/presence → python3, devices.json binds tcp://127.0.0.1:14403"
if [ -n "${PANEL_LINT_BUNDLE:-}" ]; then
  note "bundle under test: the REAL bundle at $PANEL_LINT_BUNDLE"
else
  note "bundle under test: the PLACEHOLDER (same API shape, no real client code) — the real-bundle run is the external gate (PANEL_LINT_BUNDLE=<dir>)"
fi

# ------------------------------------------------------------------ 3. namespaces
INNER_LOG=$WORK/inner.log
if env --default-signal true 2>/dev/null; then
  RESET_SIG=(env --default-signal)
else
  RESET_SIG=(python3 -c 'import os,signal,sys
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])')
fi
PL_WORK=$WORK BIN111=$BIN111 PL_VER=$VER PL_T0=$T0 BUNDLE_SERVED=$BUNDLE_SERVED PANEL_LINT_DWELL=$DWELL \
  unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
  "${RESET_SIG[@]}" bash "$HERE/panel-lint.sh" --inner 2>&1 | tee "$INNER_LOG" || true
inner_fails=$(grep -c '^FAIL:' "$INNER_LOG" || true)
if ! grep -q '^FAILS=' "$WORK/inner.result" 2>/dev/null; then
  fail "the namespace run did not finish (killed or crashed — see above)"
fi
FAILS=$((FAILS + inner_fails))

echo
if [ -s "$WORK/http/cdp/tables.txt" ]; then
  cat "$WORK/http/cdp/tables.txt"
fi
total_ok=$((OKS + $(grep -c '^ok:' "$INNER_LOG" || true)))
if [ "$FAILS" -eq 0 ]; then
  echo "PANEL-LINT GREEN — $total_ok checks ok, 0 FAIL, in $(elapsed)s"
  exit 0
fi
echo "PANEL-LINT RED — $FAILS FAIL line(s) in $(elapsed)s"
exit 1
