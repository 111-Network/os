#!/usr/bin/env bash
# 6-transport/verify/spine.sh — Layer 6 gate G4 (spec §16 G4; §11 whole): the transport spine on real daemons.
#
#   bash 6-transport/verify/spine.sh                 runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G4=1 bash 6-transport/verify/spine.sh    lock run: a missing prerequisite is a FAIL
#
# 1. The spine's unit and integration tests (B4's first cut, kept): the golden renders per posture and stanza set
#    (byte-exact, tests/fixtures/rns/*.golden), migration with the divergence line, the v1/v2 takeover seam (a v2
#    file is left alone by v1: `v1_leaves_v2_alone_and_v2_regenerates_v1`, lib and integration), the rnstatus
#    parser goldens, the posture apply path, the join round trip, the PortTable claims — each a named check — then
#    the modes on a rendered scratch root, stat'ed independently of the Rust asserts.
# 2. Real daemons. One user + pid + mount namespace (unshare -rnpf, /proc remounted) holds two network namespaces A
#    (10.66.0.1, LAN sharing on: the hub) and B (10.66.0.2, loopback dashboard: the phone) on one veth. Each runs the
#    debug `111 daemon` on its own scratch root; each daemon supervises a real rnsd (rns 1.5.7 from the gate venv,
#    verify/rns-venv.sh, built from the LXMF helper's lock) on the config the renderer writes from
#    state/interfaces.toml. Checks, in order:
#    - migration: A starts as a Layer 5 root (state/devices.json with a Meshtastic binding, an rns/config of
#      schema-version 1 with an RNode stanza, no interfaces.toml) → interfaces.toml is born (0600) with both entries,
#      the daemon logs it, rns/config is re-rendered at schema-version 2; the RNode without a region is left out;
#    - modes on the real daemon root: rns/ 0700, rns/config 0600, secrets/ 0700 + 0600 files, interfaces.toml 0600,
#      and (after the helper is enabled on B) bridge/lxmf/rns 0700 with a 0600 config carrying rnsd's rpc_key;
#    - the Pi rule: no rnstatus ran before the first reader; GET /transport is the demand and the facts appear within
#      one tick, matching the gate's own rnstatus run against the same rnsd;
#    - an RNode stanza that cannot open (/dev/ttyACM9 does not exist): the region set through PATCH
#      /transport/posture renders and --attaches it; rnstatus shows it Down while rnsd stays up (same pid);
#    - live reload / attach / detach through 111d, never a restart (rnsd's pid unchanged, rnsd.log names each op):
#      a link change (netlink → re-render → `--reload` of the AutoInterface), PATCH lan.ifac (the Backbone stanza
#      reloaded with its IFAC), PATCH stock_autointerface on (the second AutoInterface attached: RECORDED — whether
#      both come up or their ports collide is printed as note:, only "rnsd stays up" is asserted) and off (detached);
#    - the QR-join flow and the phone relay: B's session is paired with A through the Layer 4 pair code (from
#      `111 dashboard` on A, POST /auth/pair from B's netns), POST /transport/join over it returns the descriptor
#      (no-store; the IFAC passphrase present on the paired and the loopback session); unpaired, cross-site and the
#      agent's bearer are refused; the descriptor is applied on B and B's rnsd reaches A's Backbone listener:
#      rnstatus on A reads "Clients   : 1" (the exact line is printed), B's TCPInterface is Up;
#    - the posture restart path on B with the real LXMF helper (component lxmf enabled through the API, installed
#      offline from cache/packages/lxmf): PATCH enable_transport → the applier's rns.posture.set → the helper stops,
#      rnsd restarts, the helper returns (sampled every 50 ms: the order is asserted); then a double crash (kill -9
#      rnsd twice within the supervisor's back-off; G4_CRASH_ROUNDS rounds, default 3, since the race is timing)
#      must not partition: no respawned rnsd starts as a client of another holder of @rns/111 (its rnsd.log line
#      "connected to another shared local instance"; the holders are sampled and named), rnsd returns, the helper
#      is the shared instance's client again (every @rns/111 listener is rnsd's), lxmf live;
#    - migration divergence: devices.json hand-edited while A is stopped → the next start keeps interfaces.toml
#      byte-exact, logs the "interfaces.toml wins" line, and devices.json mirrors the TOML again.
#
# Deviation (recorded for the reviewer): neither a route nor a CLI exposes `join::apply_join` on the phone side, so
# the gate writes the entry `apply_join` writes (`join::client_entry`: `111 hub <address>`, owner rnsd,
# TCPClientInterface, target_host/target_port, ifac + network_name, the passphrase in
# secrets/ifac-client-<hex name> 0600) and restarts B's daemon; the Rust round trip of apply_join itself is the
# `join_descriptor_and_apply_join_round_trip` check in step 1. For the same reason the TCPClient attach is checked
# at B's start, not as a live `--attach` (the live attach is checked on the stock AutoInterface).
#
# Prerequisites: unprivileged user + network namespaces, python3 >= 3.11 with venv (the gate venv: PyPI once, or a
# pip cache), ip, nsenter, curl, jq, ss; the debug 111 (cargo build; SMOKE_SKIP_BUILD=1 reuses it; CARGO_TARGET_DIR
# and CARGO_BUILD_JOBS from the environment). Nothing of ours stays running: every process dies with the pid
# namespace or is stopped first; the scratch root is removed on success (kept on failure or with SMOKE_KEEP=1);
# the venv cache is removed unless G4_KEEP_VENV=1. The last check is the hygiene one.
# Checks are condition strings eval'd by t() at check time:
# shellcheck disable=SC2016,SC2034,SC2317
set -u

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
OS=$(cd "$HERE/../.." && pwd)
export PATH="$HOME/.cargo/bin:$PATH:/usr/sbin:/sbin"
HELPER_SRC=$OS/6-transport/sidecars/lxmf-sidecar/lxmf_sidecar.py
A_IP=10.66.0.1
B_IP=10.66.0.2
. "$HERE/lib.sh" # skip_or_fail, have_netns, finish
. "$HERE/rns-venv.sh"
. "$HERE/spine/node.sh" # ok/fail/note (counting), req/jb/t/tw, nx/api, roots, daemons

# ------------------------------------------------------------------ inner (in the namespaces)

inner() {
  : "${G4_WORK:?}" "${BIN111:?}" "${RNS_VENV:?}"
  WORK=$G4_WORK
  W=$WORK/http
  mkdir -p "$W"
  ROOT[A]=$WORK/A
  ROOT[B]=$WORK/B
  NID_A=$(cat "$WORK/A.nid" 2>/dev/null)
  cleanup_inner() {
    local n p
    for n in A B; do
      p=${DPID[$n]:-}
      if [ -n "$p" ] && kill -0 "$p" 2>/dev/null; then kill -TERM "$p" 2>/dev/null || true; fi
    done
    for _ in $(seq 1 75); do
      kill -0 "${DPID[A]:-0}" 2>/dev/null || kill -0 "${DPID[B]:-0}" 2>/dev/null || break
      sleep 0.2
    done
    for p in "${HELPERS[@]}"; do kill -TERM "$p" 2>/dev/null || true; done
    printf 'FAILS=%s\nOKS=%s\n' "$FAILS" "$OKS" >"$WORK/inner.result"
  }
  trap cleanup_inner EXIT
  trap 'exit 130' INT TERM
  local n d0 r0 r1 c0 sha0 pass tok desc mp mm

  # ---------------------------------------------------------------- 1. namespaces
  phase "1. namespaces: A $A_IP (hub) and B $B_IP (phone) on one veth"
  ip link set lo up
  make_netns A B
  if ip link add vA type veth peer name vB &&
    ip link set vA netns "${HOLD[A]}" && ip link set vB netns "${HOLD[B]}" &&
    nx A ip link set vA name eth0 && nx B ip link set vB name eth0 &&
    nx A ip addr add "$A_IP/24" dev eth0 && nx B ip addr add "$B_IP/24" dev eth0 &&
    nx A ip link set eth0 up && nx B ip link set eth0 up; then
    ok "netns A (holder ${HOLD[A]}) eth0 $A_IP/24 ↔ netns B (holder ${HOLD[B]}) eth0 $B_IP/24"
  else
    fail "netns/veth setup failed"
  fi
  if [ -d /proc/sys/net/ipv6 ]; then
    tw 15 "both ends have a usable IPv6 link-local address (the AutoInterface needs one)" \
      '[ -n "$(nx A ip -6 -o addr show dev eth0 scope link -tentative 2>/dev/null)" ] && [ -n "$(nx B ip -6 -o addr show dev eth0 scope link -tentative 2>/dev/null)" ]'
  else
    note "this kernel has no IPv6 (/proc/sys/net/ipv6 is absent): every AutoInterface reads Up but cannot autoconfigure, so the second-AutoInterface port question is recorded without IPv6 traffic"
  fi

  # ---------------------------------------------------------------- 2. migration (A starts as a Layer 5 root)
  phase "2. migration: A's Layer 5 root (devices.json binding + an rns/config v1 RNode stanza) under the Layer 6 daemon"
  t "A: before the first Layer 6 start there is no interfaces.toml; rns/config is schema-version 1 with the RNode stanza; devices.json carries the binding" \
    '[ ! -e "${ROOT[A]}/state/interfaces.toml" ] && grep -qx "schema-version = 1" "${ROOT[A]}/rns/config" &&
     grep -q "\[\[Bench RNode\]\]" "${ROOT[A]}/rns/config" && jq -e ".devices[].meshtastic.node_id == \"!0a0a0a0a\"" "${ROOT[A]}/state/devices.json" >/dev/null'
  start_node A
  local tomlA=${ROOT[A]}/state/interfaces.toml
  tw 20 "A: interfaces.toml is born from devices.json and rns/config, 0600" '[ -f "$tomlA" ] && [ "$(stat -c %a "$tomlA")" = 600 ]'
  cp "$tomlA" "$WORK/interfaces-A-born.toml" 2>/dev/null
  t "A: it carries the Meshtastic binding as an owner = \"111d\" entry (tcp uplink 127.0.0.1:4403) and the RNode as an owner = \"rnsd\" entry on /dev/ttyACM9" \
    'python3 -I - "$tomlA" <<"PY"
import sys, tomllib
f = tomllib.load(open(sys.argv[1], "rb"))
ifs = f.get("interface", [])
mesh = [i for i in ifs if i.get("owner") == "111d" and i.get("type") == "meshtastic"]
rn = [i for i in ifs if i.get("owner") == "rnsd" and i.get("type") == "RNodeInterface" and i.get("port") == "/dev/ttyACM9"]
ok = f.get("schema") == 1 and len(ifs) == 2 and len(mesh) == 1 and len(rn) == 1 and "4403" in str(mesh[0].get("uplink", ""))
ok = ok and rn[0].get("params", {}).get("frequency") == 869525000
sys.exit(0 if ok else 1)
PY'
  t "A: the daemon logged the migration (\"$(grep -m1 -o 'interfaces.toml created from devices.json and rns/config ([0-9]* interface(s))' "${ROOT[A]}/daemon.log")\")" \
    'grep -q "interfaces.toml created from devices.json and rns/config (2 interface(s))" "${ROOT[A]}/daemon.log"'
  tw 30 "A: 111d supervises rnsd from venvs/rns on the rendered config (pid $(rnsd_pid A))" '[ -n "$(rnsd_pid A)" ]'
  t "A: rnsd is the daemon's own child (PPid ${DPID[A]})" '[ "$(awk "/^PPid:/{print \$2}" /proc/$(rnsd_pid A)/status 2>/dev/null)" = "${DPID[A]}" ]'
  t "A: rns/config was re-rendered at schema-version 2 (share_instance Yes, instance 111, an rpc_key, the LAN Backbone on 4242)" \
    'grep -qx "schema-version = 2" "${ROOT[A]}/rns/config" && grep -q "share_instance = Yes" "${ROOT[A]}/rns/config" &&
     grep -q "instance_name = 111" "${ROOT[A]}/rns/config" && grep -Eq "^  rpc_key = [0-9a-f]{64}$" "${ROOT[A]}/rns/config" &&
     grep -q "type = BackboneInterface" "${ROOT[A]}/rns/config"'
  t "A: the migrated RNode has no region yet, so it is left out of the render with its reason in the log (fail closed)" \
    '! grep -q "\[\[Bench RNode\]\]" "${ROOT[A]}/rns/config" && grep -q "interface \"Bench RNode\" is not rendered" "${ROOT[A]}/daemon.log"'
  t "A: devices.json still carries the binding (the write-only mirror for a Layer 5 rollback)" \
    'jq -e "[.devices[] | select(.meshtastic != null)] | length == 1" "${ROOT[A]}/state/devices.json" >/dev/null'
  for m in "rns 700" "rns/config 600" "secrets 700" "secrets/rns-rpc-key 600" "state/interfaces.toml 600"; do
    read -r mp mm <<<"$m"
    t "A: mode $mm $mp on the daemon's root" '[ ! -L "${ROOT[A]}/$mp" ] && [ "$(stat -c %a "${ROOT[A]}/$mp" 2>/dev/null)" = "$mm" ]'
  done

  # ---------------------------------------------------------------- 3. facts: the Pi rule and rnstatus
  phase "3. A: the Pi rule, then rnstatus facts in GET /transport"
  sleep 3
  t "Pi rule: no rnstatus ran on A before anything read the facts (daemon up $(since "$(stat -c %Y "${ROOT[A]}/daemon.log")")s)" \
    '[ ! -s "${ROOT[A]}/rnstatus-calls.log" ]'
  api A GET /api/v1/transport
  t "A: GET /transport answers (the demand) with the posture of interfaces.toml (share_instance true, backbone listen on 4242)" \
    '[ "$ST" = 200 ] && jb ".posture.share_instance == true and .posture.lan.backbone_listen == true and .posture.lan.port == 4242"'
  tw 40 "A: within one tick the facts are filled from rnstatus (interfaces_state ok; the Shared Instance, the AutoInterface and the Backbone listener up)" \
    'api A GET /api/v1/transport; jb ".interfaces_state == \"ok\" and ([.interfaces[] | select(.up)] | map(.name) | (index(\"Auto Interface\") != null and index(\"111 LAN Backbone\") != null and index(\"rns/111\") != null))"'
  cp "$W/body" "$WORK/transport-A-1.json"
  t "Pi rule positive control: the reader made 111d run rnstatus ($(wc -l <"${ROOT[A]}/rnstatus-calls.log" 2>/dev/null || echo 0) calls, each --config <root>/rns)" \
    '[ -s "${ROOT[A]}/rnstatus-calls.log" ] && ! grep -qv -- "--config ${ROOT[A]}/rns" "${ROOT[A]}/rnstatus-calls.log"'
  rs A
  printf '%s\n' "$RS" >"$WORK/rnstatus-A-1.txt"
  t "A: every interface 111d reports is in the gate's own rnstatus run with the same up/down ($(jq -c '[.interfaces[] | "\(.type)[\(.name)]=\(.up)"]' "$WORK/transport-A-1.json"))" \
    'jq -r ".interfaces[] | \"\(.name)\t\(.up)\"" "$WORK/transport-A-1.json" | { okk=1; while IFS="	" read -r nm up; do
       st=$(rs_status "$nm"); want=Down; [ "$up" = true ] && want=Up; [ "$st" = "$want" ] || okk=0; done; [ $okk = 1 ]; }'
  note "A rnstatus (real, rns 1.5.7): $(printf '%s' "$RS" | grep -E '^ [A-Za-z ]+\[' | tr -s ' ' | tr '\n' ';')"

  # ---------------------------------------------------------------- 4. an RNode that cannot open
  phase "4. A: an RNode stanza that cannot open shows Down while rnsd stays up"
  r0=$(rnsd_pid A)
  c0=$(rnsd_log_count A 'Interface "Bench RNode" was attached')
  jsend A PATCH /api/v1/transport/posture '{"region":"EU_868"}'
  cp "$W/body" "$WORK/patch-region.json"
  t "A: PATCH /transport/posture {region: EU_868} → 200 applied (the applier's radio.region.set; the request is the confirmation)" \
    '[ "$ST" = 200 ] && jb ".applied == true and .reverted == false and .status.region == \"EU_868\""'
  t "A: [radio] region = \"EU_868\" in interfaces.toml; rns/config now carries [[Bench RNode]] on /dev/ttyACM9 with the EU_868 plan" \
    'grep -q "^region = \"EU_868\"" "$tomlA" && grep -q "\[\[Bench RNode\]\]" "${ROOT[A]}/rns/config" && grep -q "port = /dev/ttyACM9" "${ROOT[A]}/rns/config"'
  tw 15 "A: 111d attached it live (rnsd.log: Interface \"Bench RNode\" was attached; no restart)" \
    '[ "$(rnsd_log_count A "Interface \"Bench RNode\" was attached")" -gt "$c0" ]'
  tw 20 "A: rnstatus lists RNodeInterface[Bench RNode] with Status Down" 'rs A; [ "$(rs_status "Bench RNode")" = Down ]'
  t "A: rnsd logged that the port cannot be opened ($(grep -m1 -o 'Could not open serial port[^.]*' "${ROOT[A]}/rns/rnsd.log"))" \
    'grep -q "Could not open serial port" "${ROOT[A]}/rns/rnsd.log"'
  sleep 3
  rs A
  t "A: rnsd stays up: the same pid ($r0) and the Shared Instance Up" '[ "$(rnsd_pid A)" = "$r0" ] && [ "$(rs_status "rns/111")" = Up ]'
  tw 45 "A: GET /transport shows Bench RNode up: false (from the cache tick)" \
    'api A GET /api/v1/transport; jb "[.interfaces[] | select(.name == \"Bench RNode\" and .up == false)] | length == 1"'

  # ---------------------------------------------------------------- 5. live reload / attach / detach
  phase "5. A: live reload, attach, detach through 111d (rnsd keeps its pid)"
  r0=$(rnsd_pid A)
  c0=$(rnsd_log_count A 'Interface "Auto Interface" was reloaded')
  nx A ip link add lk0 type veth peer name lk1 && nx A ip link set lk0 up && nx A ip link set lk1 up
  tw 25 "A: a link change (veth lk0 added) → netlink → re-render → --reload of the AutoInterface (rnsd.log)" \
    '[ "$(rnsd_log_count A "Interface \"Auto Interface\" was reloaded")" -gt "$c0" ]'
  t "A: … without an rnsd restart (pid $r0)" '[ "$(rnsd_pid A)" = "$r0" ]'
  c0=$(rnsd_log_count A 'Interface "111 LAN Backbone" was reloaded')
  jsend A PATCH /api/v1/transport/posture '{"posture":{"lan":{"ifac":true}}}'
  t "A: PATCH lan.ifac = true → 200 applied" '[ "$ST" = 200 ] && jb ".applied == true and .status.posture.lan.ifac == true"'
  tw 15 "A: the Backbone stanza was reloaded live with its IFAC (rnsd.log), rnsd pid $r0 unchanged" \
    '[ "$(rnsd_log_count A "Interface \"111 LAN Backbone\" was reloaded")" -gt "$c0" ] && [ "$(rnsd_pid A)" = "$r0" ]'
  t "A: the passphrase was minted in secrets/ifac-passphrase (0600) and rendered into the Backbone stanza (network_name 111-lan), never into interfaces.toml" \
    '[ "$(stat -c %a "${ROOT[A]}/secrets/ifac-passphrase" 2>/dev/null)" = 600 ] && pass=$(tr -d "\n" <"${ROOT[A]}/secrets/ifac-passphrase") &&
     grep -q "network_name = 111-lan" "${ROOT[A]}/rns/config" && grep -q "passphrase = $pass" "${ROOT[A]}/rns/config" && ! grep -q "$pass" "$tomlA"'
  tw 15 "A: rnstatus shows the Backbone with Network 111-lan" 'rs A; rs_block "111 LAN Backbone" | grep -Eq "Network *: *111-lan"'
  c0=$(rnsd_log_count A 'Interface "Reticulum Auto Interface" was attached')
  jsend A PATCH /api/v1/transport/posture '{"posture":{"stock_autointerface":true}}'
  t "A: PATCH stock_autointerface = true → 200 applied" '[ "$ST" = 200 ] && jb ".applied == true and .status.posture.stock_autointerface == true"'
  tw 15 "A: the second AutoInterface (group reticulum) was attached live (rnsd.log), rnsd pid $r0 unchanged" \
    '[ "$(rnsd_log_count A "Interface \"Reticulum Auto Interface\" was attached")" -gt "$c0" ] && [ "$(rnsd_pid A)" = "$r0" ]'
  sleep 6
  rs A
  printf '%s\n' "$RS" >"$WORK/rnstatus-A-two-auto.txt"
  note "second AutoInterface RECORDED: [Auto Interface] Status $(rs_status "Auto Interface" | tr -d ' ') · [Reticulum Auto Interface] Status $(rs_status "Reticulum Auto Interface" | tr -d ' ') · rnsd.log on it: $(grep -F 'Reticulum Auto Interface' "${ROOT[A]}/rns/rnsd.log" | grep -Eiv 'attached|reloaded|detached' | sed 's/^\[[^]]*\] //' | tr -s ' ' | head -3 | tr '\n' '|')$(grep -Eic 'address already in use|errno 98' "${ROOT[A]}/rns/rnsd.log") 'address in use' line(s)"
  t "A: with two AutoInterfaces rnsd stays up (pid $r0, the Shared Instance Up)" '[ "$(rnsd_pid A)" = "$r0" ] && [ "$(rs_status "rns/111")" = Up ]'
  c0=$(rnsd_log_count A 'Interface "Reticulum Auto Interface" was detached')
  jsend A PATCH /api/v1/transport/posture '{"posture":{"stock_autointerface":false}}'
  t "A: PATCH stock_autointerface = false → 200 applied" '[ "$ST" = 200 ] && jb ".applied == true"'
  tw 15 "A: … detached live (rnsd.log), gone from rnstatus, rnsd pid $r0 unchanged" \
    '[ "$(rnsd_log_count A "Interface \"Reticulum Auto Interface\" was detached")" -gt "$c0" ] && rs A && [ -z "$(rs_block "Reticulum Auto Interface")" ] && [ "$(rnsd_pid A)" = "$r0" ]'
  t "A: positive control for the 'gone' check: the first AutoInterface is still listed" '[ -n "$(rs_block "Auto Interface")" ]'

  # ---------------------------------------------------------------- 6. QR join and the phone relay
  phase "6. QR join between two 111d and the phone relay (A's Backbone ← B's TCPClient)"
  start_node B
  tw 30 "B: 111d supervises rnsd (pid $(rnsd_pid B))" '[ -n "$(rnsd_pid B)" ]'
  local jar=$W/b-paired.jar url=http://$A_IP:11180
  NSX=(nsenter -t "${HOLD[B]}" -n --)
  req POST "$url/api/v1/transport/join" -H 'Content-Type: application/json' -d '{}'
  t "B → A unpaired: POST /transport/join → 401 (only a paired or loopback session)" '[ "$ST" = 401 ]'
  pair_session A "$url" "$jar"
  t "B's session pairs with A through the Layer 4 pair code (\`111 dashboard\` on A shows it and the QR link; POST /auth/pair from B → 200 paired)" \
    '[ "$ST" = 200 ] && jb ".paired == true and .loopback == false" && grep -q "#/pair?code=$CODE" "$W/dashboard-A.out"'
  req POST "$url/api/v1/transport/join" -b "$jar" -H 'Content-Type: application/json' -H 'Origin: http://evil.example' -d '{}'
  t "B → A paired but cross-site (Origin evil.example) → 403" '[ "$ST" = 403 ]'
  req POST "$url/api/v1/transport/join" -b "$jar" -H 'Content-Type: application/json' -d '{}'
  cp "$W/body" "$WORK/join-paired.json"
  cp "$W/headers" "$WORK/join-paired.headers"
  NSX=()
  pass=$(tr -d '\n' <"${ROOT[A]}/secrets/ifac-passphrase" 2>/dev/null)
  t "B → A paired: POST /transport/join → 200, Cache-Control: no-store, {schema 1, address $A_IP, port 4242, ifac_name 111-lan, the passphrase}" \
    '[ "$ST" = 200 ] && [ "$(hdr cache-control)" = no-store ] &&
     jb --arg a "$A_IP" --arg p "$pass" ".schema == 1 and .address == \$a and .port == 4242 and .ifac_name == \"111-lan\" and .ifac_passphrase == \$p" && [ -n "$pass" ]'
  jsend A POST /api/v1/transport/join '{}'
  t "A loopback: POST /transport/join → 200 with the passphrase too" '[ "$ST" = 200 ] && jb --arg p "$pass" ".ifac_passphrase == \$p"'
  tok=${ROOT[A]}/agent/g45-agent.token
  wait_for 20 '[ -s "$tok" ]' || true
  TOKEN=$(cat "$tok" 2>/dev/null || true)
  t "A: the agent's per-spawn bearer is readable from the stand-in agent (64 hex)" '[[ "$TOKEN" =~ ^[0-9a-f]{64}$ ]]'
  api A POST /api/v1/transport/join -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{}'
  t "A: the agent's bearer on POST /transport/join → 401, no descriptor" '[ "$ST" = 401 ] && ! grep -q "$pass" "$W/body"'
  api A GET /api/v1/transport -H "Authorization: Bearer $TOKEN"
  t "A: the agent's bearer reads GET /transport (200) and no credential is in it" '[ "$ST" = 200 ] && ! grep -q "$pass" "$W/body" && ! grep -q "rpc_key" "$W/body"'
  t "A: the passphrase was never logged (daemon.log)" '! grep -q "$pass" "${ROOT[A]}/daemon.log"'
  # the phone side: what join::apply_join writes (deviation: no route or CLI exposes it; see the header)
  stop_node B
  t "B: 111 daemon exits 0 on SIGTERM before the join is applied (rc $RC)" '[ "$RC" = 0 ]'
  local entry="111 hub $A_IP" hexname
  hexname=$(printf '%s' "$entry" | od -An -tx1 | tr -d ' \n')
  umask 077
  mkdir -p "${ROOT[B]}/secrets"
  printf '%s' "$(jq -r '.ifac_passphrase' "$WORK/join-paired.json")" >"${ROOT[B]}/secrets/ifac-client-$hexname"
  python3 -I - "${ROOT[B]}/state/interfaces.toml" "$WORK/join-paired.json" <<'PY'
import json, sys
toml_path, desc = sys.argv[1], json.load(open(sys.argv[2]))
text = open(toml_path).read().rstrip("\n")
text += '\n\n[[interface]]\nname = "111 hub %s"\nowner = "rnsd"\ntype = "TCPClientInterface"\n' % desc["address"]
text += 'params = { target_host = "%s", target_port = %d, ifac = true, network_name = "%s" }\n' % (
    desc["address"], desc["port"], desc["ifac_name"])
open(toml_path, "w").write(text)
PY
  umask 022
  t "B: the join entry is written as apply_join would (TCPClientInterface to $A_IP:4242, ifac on; the passphrase only in secrets/ifac-client-<hex> 0600)" \
    'grep -q "name = \"111 hub $A_IP\"" "${ROOT[B]}/state/interfaces.toml" && [ "$(stat -c %a "${ROOT[B]}/secrets/ifac-client-$hexname")" = 600 ] &&
     ! grep -q "$pass" "${ROOT[B]}/state/interfaces.toml"'
  start_node B
  tw 30 "B: rnsd restarted on the new render (pid $(rnsd_pid B))" '[ -n "$(rnsd_pid B)" ]'
  t "B: rns/config carries [[111 hub $A_IP]] TCPClientInterface with target $A_IP:4242, network_name 111-lan and the passphrase" \
    'grep -q "\[\[111 hub $A_IP\]\]" "${ROOT[B]}/rns/config" && grep -q "type = TCPClientInterface" "${ROOT[B]}/rns/config" &&
     grep -q "target_host = $A_IP" "${ROOT[B]}/rns/config" && grep -q "target_port = 4242" "${ROOT[B]}/rns/config" &&
     grep -q "network_name = 111-lan" "${ROOT[B]}/rns/config" && grep -q "passphrase = $pass" "${ROOT[B]}/rns/config"'
  tw 60 "B's rnsd reaches A: the TCP connection B → $A_IP:4242 is established" 'nx B ss -tnH state established 2>/dev/null | grep -q "$A_IP:4242"'
  tw 30 "A: rnstatus on the hub reads Clients: 1 on the Backbone listener" 'rs A; rs_block "111 LAN Backbone" | grep -Eq "^ *Clients *: *1$"'
  note "the exact rnstatus line on A: '$(rs_block "111 LAN Backbone" | grep -E '^ *Clients' | head -1)'; header '$(rs_block "111 LAN Backbone" | head -1 | sed 's/^ *//')'"
  tw 20 "B: rnstatus shows the client interface Up ($(rs B; rs_block "111 hub $A_IP" | head -1 | sed 's/^ *//'))" 'rs B; [ "$(rs_status "111 hub $A_IP")" = Up ]'
  tw 45 "A: GET /transport reports the Backbone with peers 1" \
    'api A GET /api/v1/transport; jb "[.interfaces[] | select(.name == \"111 LAN Backbone\" and .peers == 1 and .up)] | length == 1"'
  tw 45 "B: GET /transport reports \"111 hub $A_IP\" up" \
    'api B GET /api/v1/transport; jb --arg n "111 hub $A_IP" "[.interfaces[] | select(.name == \$n and .up)] | length == 1"'

  # ---------------------------------------------------------------- 7. posture restart with the helper
  phase "7. B: the posture restart path with the LXMF helper, then a double rnsd crash"
  local notice
  jsend B POST /api/v1/bridge/components/lxmf/enable '{}'
  notice=$(jget '.consent.hash // .hash // empty')
  t "B: enable lxmf without the acknowledgement → 409 consent_required with the notice hash" '[ "$ST" = 409 ] && [[ "$notice" =~ ^[0-9a-f]{64}$ ]]'
  d0=$(now)
  REQ_TIMEOUT=240 jsend B POST /api/v1/bridge/components/lxmf/enable "$(jq -nc --arg h "$notice" '{accept_notice_hash: $h}')"
  cp "$W/body" "$WORK/enable-lxmf-B.json"
  t "B: the acknowledged enable → 200 in $(since "$d0")s (installed offline from cache/packages/lxmf)" '[ "$ST" = 200 ]'
  tw 60 "B: the helper runs as rnsd's client (pid $(helper_pid B)) and rnstatus shows the instance serving it" \
    '[ -n "$(helper_pid B)" ] && rs B && rs_block "rns/111" | grep -Eq "Serving *: *[1-9]"'
  for m in "bridge/lxmf 700" "bridge/lxmf/rns 700" "bridge/lxmf/rns/config 600" "secrets 700" "secrets/rns-rpc-key 600"; do
    read -r mp mm <<<"$m"
    t "B: mode $mm $mp on the daemon's root (the helper's client dir)" '[ ! -L "${ROOT[B]}/$mp" ] && [ "$(stat -c %a "${ROOT[B]}/$mp" 2>/dev/null)" = "$mm" ]'
  done
  local key
  key=$(sed -n 's/^  rpc_key = \([0-9a-f]\{64\}\)$/\1/p' "${ROOT[B]}/rns/config")
  t "B: the helper's client config carries rnsd's rpc_key and no interface stanza" \
    '[ -n "$key" ] && grep -q "rpc_key = $key" "${ROOT[B]}/bridge/lxmf/rns/config" && ! grep -q "^ *\[\[" "${ROOT[B]}/bridge/lxmf/rns/config"'
  tw 30 "B: GET /networks shows lxmf live" 'api B GET /api/v1/networks; jb "[.networks[] | select(.id == \"lxmf\" and .state == \"live\")] | length == 1"'
  local h1 seq=$WORK/restart-seq.log enpid
  r1=$(rnsd_pid B)
  h1=$(helper_pid B)
  local pl0
  pl0=$(wc -l <"${ROOT[B]}/rns/rnsd.log")
  : >"$seq"
  bg_in B "$W/patch-posture.out" curl -sS -o "$W/patch-posture.json" -w '%{http_code}' --max-time 120 -X PATCH \
    -H 'Content-Type: application/json' -d '{"posture":{"enable_transport":true}}' "$DAPI/api/v1/transport/posture"
  enpid=$BGPID
  for _ in $(seq 1 2400); do
    printf '%s %s %s\n' "$(now)" "$(rnsd_pid B)" "$(helper_pid B)" >>"$seq"
    kill -0 "$enpid" 2>/dev/null || break
    sleep 0.05
  done
  wait "$enpid" 2>/dev/null
  for _ in $(seq 1 200); do
    printf '%s %s %s\n' "$(now)" "$(rnsd_pid B)" "$(helper_pid B)" >>"$seq"
    [ -n "$(helper_pid B)" ] && [ "$(helper_pid B)" != "$h1" ] && break
    sleep 0.1
  done
  ST=$(cat "$W/patch-posture.out" 2>/dev/null)
  cp "$W/patch-posture.json" "$W/body" 2>/dev/null
  t "B: PATCH enable_transport = true → 200 applied, not reverted (the applier's rns.posture.set kept)" \
    '[ "$ST" = 200 ] && jb ".applied == true and .reverted == false and .status.posture.enable_transport == true"'
  t "B: rns/config now has enable_transport = Yes" 'grep -q "enable_transport = Yes" "${ROOT[B]}/rns/config"'
  t "B: the restarted rnsd became the instance, not a client of a transient holder of @rns/111 (rnsd.log: $(tail -n +"$((pl0 + 1))" "${ROOT[B]}/rns/rnsd.log" | grep -c 'connected to another shared local instance') \"connected to another shared local instance\" line(s))" \
    '! tail -n +"$((pl0 + 1))" "${ROOT[B]}/rns/rnsd.log" | grep -q "connected to another shared local instance"'
  # the sampled order: helper h1 gone → rnsd r1 replaced → a new helper
  local order
  order=$(awk -v r1="$r1" -v h1="$h1" '
    { r=$2; h=$3 }
    stage==0 && h!=h1 { stage=1; if (r!=r1) bad="rnsd changed before the helper stopped" }
    stage==1 && r!="" && r!=r1 { stage=2; if (h!="") bad="a helper ran when rnsd came back" }
    stage==2 && h!="" && h!=h1 { stage=3 }
    END { if (bad!="") print bad; else print "stage" stage }' "$seq")
  t "B: the order (sampled every 50 ms): the helper ($h1) stopped, then rnsd restarted ($r1 → $(rnsd_pid B)), then the helper returned ($(helper_pid B)) [$order]" \
    '[ "$order" = stage3 ]'
  tw 60 "B: after the restart lxmf is live again and rnsd serves the helper" \
    'api B GET /api/v1/networks; jb "[.networks[] | select(.id == \"lxmf\" and .state == \"live\")] | length == 1" && rs B && rs_block "rns/111" | grep -Eq "Serving *: *[1-9]"'
  # the double crash: kill -9 rnsd, and its respawn again within the supervisor's back-off — G4_CRASH_ROUNDS times
  # (default 3, spaced past the supervisor's 60 s crash-loop window: whether a respawn races a transient holder of
  # the instance is timing, so one round can miss it).
  # From the first kill on every holder of @rns/111 is sampled every 50 ms (pid → program), so a partition names
  # who held the instance.
  local k1 k2 k3 round rounds=${G4_CRASH_ROUNDS:-3} reasons owners lines0 p0 holders sampler ssx kicked=""
  for round in $(seq 1 "$rounds"); do
    # rounds are spaced past the supervisor's crash-loop window (5 exits in 60 s ⇒ degraded, by policy)
    if [ -n "$kicked" ]; then
      wait_for 70 '[ $((SECONDS - kicked)) -ge 62 ]' || true
    fi
    reasons=$WORK/double-crash-$round-networks.log
    owners=$WORK/double-crash-$round-owners.log
    if ! wait_for 90 'api B GET /api/v1/networks; jb "[.networks[] | select(.id == \"lxmf\" and .state == \"live\")] | length == 1" && [ -n "$(helper_pid B)" ]'; then
      fail "B: double crash round $round: lxmf is not live before the crash (the previous round did not recover)"
      break
    fi
    lines0=$(wc -l <"${ROOT[B]}/rns/rnsd.log")
    : >"$owners"
    ( for _ in $(seq 1 400); do
        for p0 in $(nx B ss -xlpH 2>/dev/null | grep 'rns/111' | grep -o 'pid=[0-9]*' | cut -d= -f2 | sort -u); do
          printf '%s %s %s\n' "$(now)" "$p0" "$(tr '\0' '\n' <"/proc/$p0/cmdline" 2>/dev/null | sed -n 2p | xargs -r basename)" >>"$owners"
        done
        sleep 0.05
      done ) &
    sampler=$!
    HELPERS+=("$sampler")
    k1=$(rnsd_pid B)
    kicked=$SECONDS
    kill -KILL "$k1" 2>/dev/null
    wait_for 10 '[ -n "$(rnsd_pid B)" ] && [ "$(rnsd_pid B)" != "$k1" ]' || true
    k2=$(rnsd_pid B)
    kill -KILL "$k2" 2>/dev/null
    t "B: double crash round $round: rnsd $k1 killed, its respawn $k2 killed again right away" '[ -n "$k2" ] && [ "$k2" != "$k1" ]'
    sleep 15
    kill "$sampler" 2>/dev/null
    wait "$sampler" 2>/dev/null
    : >"$reasons"
    for _ in $(seq 1 75); do
      api B GET /api/v1/networks
      jq -c '[.networks[]? | select(.id == "lxmf") | {state, c: .component.state, r: .component.reason_code}]' "$W/body" >>"$reasons" 2>/dev/null
      k3=$(rnsd_pid B)
      if [ -n "$k3" ] && [ "$k3" != "$k2" ] && jb '[.networks[] | select(.id == "lxmf" and .state == "live")] | length == 1' && [ -n "$(helper_pid B)" ]; then
        rs B
        rs_block "rns/111" | grep -Eq "Serving *: *[1-9]" && break
      fi
      sleep 1
    done
    k3=$(rnsd_pid B)
    holders=$(awk -v r="$k3" '$2 != r && !($2 in seen) {seen[$2]=1; n[$3]++} END {for (c in n) printf "%d× %s; ", n[c], c}' "$owners")
    note "round $round: lxmf through the double crash (GET /networks each second): $(uniq "$reasons" | tr '\n' ' ' | head -c 300)"
    note "round $round: @rns/111 holders other than the final rnsd $k3 from the first kill on (sampled every 50 ms, by program): ${holders:-none}"
    t "B: round $round: rnsd returned ($k3) after the double crash" '[ -n "$k3" ] && [ "$k3" != "$k2" ] && [ "$k3" != "$k1" ]'
    t "B: round $round: no respawned rnsd started as a client of another instance (rnsd.log: $(tail -n +"$((lines0 + 1))" "${ROOT[B]}/rns/rnsd.log" | grep -c 'connected to another shared local instance') \"connected to another shared local instance\" line(s))" \
      '! tail -n +"$((lines0 + 1))" "${ROOT[B]}/rns/rnsd.log" | grep -q "connected to another shared local instance"'
    ssx=$(nx B ss -xlp 2>/dev/null)
    printf '%s\n' "$ssx" >"$WORK/ss-x-B-$round.txt"
    t "B: round $round: no partition: every @rns/111 listener is rnsd's (pid $k3), none is the helper's" \
      'printf "%s\n" "$ssx" | grep "rns/111" | grep -q "pid=$k3," && ! printf "%s\n" "$ssx" | grep "rns/111" | grep -qv "pid=$k3,"'
    t "B: round $round: the helper ($(helper_pid B)) is the shared instance's client again and lxmf is live" \
      '[ -n "$(helper_pid B)" ] && rs B && rs_block "rns/111" | grep -Eq "Serving *: *[1-9]" &&
       api B GET /api/v1/networks && jb "[.networks[] | select(.id == \"lxmf\" and .state == \"live\")] | length == 1"'
    tw 30 "B: round $round: the TCPClient to A came back with rnsd" 'rs B; [ "$(rs_status "111 hub $A_IP")" = Up ]'
  done

  # ---------------------------------------------------------------- 8. migration divergence
  phase "8. A: a hand-edited devices.json diverges; the next start keeps interfaces.toml and logs it"
  stop_node A
  t "A: 111 daemon exits 0 on SIGTERM (rc $RC)" '[ "$RC" = 0 ]'
  tw 10 "A: nothing of A's daemon is left in its netns (rnsd stopped with it)" '[ -z "$(rnsd_pid A)" ]'
  sha0=$(sha256sum "$tomlA" | cut -c1-64)
  python3 -I - "${ROOT[A]}/state/devices.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
for e in d["devices"].values():
    m = e.get("meshtastic")
    if m:
        m["node_id"], m["node_num"] = "!0b0b0b0b", 185273099
        m["uplink"] = {"kind": "tcp", "host": "127.0.0.1", "port": 4404}
json.dump(d, open(p, "w"))
PY
  t "A: devices.json now names a different board (!0b0b0b0b on :4404) than interfaces.toml" \
    'jq -e "[.devices[].meshtastic.node_id] | index(\"!0b0b0b0b\") != null" "${ROOT[A]}/state/devices.json" >/dev/null'
  local lines0
  lines0=$(wc -l <"${ROOT[A]}/daemon.log")
  start_node A
  tw 20 "A: the start logged the divergence (\"… interfaces.toml wins and devices.json now mirrors it\")" \
    'tail -n +"$((lines0 + 1))" "${ROOT[A]}/daemon.log" | grep -q "differs from interfaces.toml.*interfaces.toml wins"'
  note "$(tail -n +"$((lines0 + 1))" "${ROOT[A]}/daemon.log" | grep -m1 'interfaces.toml wins' | head -c 300)"
  t "A: interfaces.toml is byte-exact as before the start (sha256 ${sha0:0:12}…)" '[ "$(sha256sum "$tomlA" | cut -c1-64)" = "$sha0" ]'
  t "A: devices.json mirrors the TOML again (!0a0a0a0a, not the hand edit)" \
    'jq -e "[.devices[].meshtastic | select(. != null) | .node_id] == [\"!0a0a0a0a\"]" "${ROOT[A]}/state/devices.json" >/dev/null'
  tw 30 "A: rnsd is back on the kept TOML (region EU_868, IFAC on: Bench RNode and the IFAC Backbone rendered)" \
    '[ -n "$(rnsd_pid A)" ] && grep -q "\[\[Bench RNode\]\]" "${ROOT[A]}/rns/config" && grep -q "network_name = 111-lan" "${ROOT[A]}/rns/config"'

  # ---------------------------------------------------------------- 9. stop
  phase "9. stop both daemons"
  for n in B A; do
    stop_node "$n"
    t "$n: 111 daemon exits 0 on SIGTERM (rc $RC)" '[ "$RC" = 0 ]'
    tw 15 "$n: nothing of this run is left in its netns (rnsd, helper)" '[ -z "$(rnsd_pid "$n")" ] && [ -z "$(helper_pid "$n")" ]'
  done
}

if [ "${1:-}" = --inner ]; then
  inner
  exit $((FAILS > 0))
fi

# ------------------------------------------------------------------ outer
echo "6-transport/verify/spine.sh — Layer 6 gate G4"
command -v cargo >/dev/null 2>&1 || skip_or_fail G4 "cargo is not installed"
have_netns || skip_or_fail G4 "unprivileged network namespaces are not available (unshare -Urnm)"
for tool in python3 ip nsenter curl jq ss unshare sha256sum od; do
  command -v "$tool" >/dev/null 2>&1 || skip_or_fail G4 "$tool is not installed"
done
[ -r "$HELPER_SRC" ] || { echo "FAIL: G4: the LXMF helper source is missing ($HELPER_SRC)"; exit 1; }

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/111-spine.XXXXXX") || { echo "FAIL: mktemp"; exit 1; }
VENV_DIR=${G4_VENV_DIR:-${TMPDIR:-/tmp}/111-g4-rns-venv}
cleanup() {
  if [ "${SMOKE_KEEP:-0}" = 1 ] || [ "$FAILS" -gt 0 ]; then
    echo "note: scratch kept at $SCRATCH (A/ B/ daemon.log rns/rnsd.log, http/, rnstatus-*.txt, restart-seq.log)"
  else
    rm -rf "$SCRATCH"
  fi
  if [ "${SMOKE_KEEP:-0}" != 1 ] && [ "${G4_KEEP_VENV:-0}" != 1 ]; then rns_venv_remove "$VENV_DIR"; fi
}
trap cleanup EXIT

# ---- 1. the spine's tests (golden renders, seams, apply path), each a named check
phase "1. the spine's cargo tests (CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-default})"
LOG=$SCRATCH/cargo.log
ctest() { # DESC CARGO-TEST-ARGS…
  local desc=$1
  shift
  if (cd "$OS/1-bootstrap" && cargo test -q -p node111 "$@") >"$LOG" 2>&1; then
    local res
    res=$(grep -E '^test result:' "$LOG" | awk '{p+=$4; f+=$6} END{print p" passed, "f" failed"}')
    if grep -qE '^test result: ok\. [1-9]' "$LOG"; then ok "$desc ($res)"; else fail "$desc: no test ran ($res)"; fi
  else
    tail -n 30 "$LOG" | sed 's/^/  cargo| /'
    fail "$desc"
  fi
}
ctest "cargo test --test transport_spine (the whole spine suite)" --test transport_spine
ctest "golden renders per posture and stanza set (byte-exact tests/fixtures/rns/*.golden)" --test transport_spine golden_renders_per_posture_and_stanza_set
ctest "migration from devices.json and rns/config, then the divergence line" --test transport_spine migration_from_devices_json_and_rns_config_then_divergence
ctest "a v2 file is left alone by v1 and v2 regenerates v1 (integration)" --test transport_spine v1_leaves_v2_alone_and_v2_regenerates_v1
ctest "a v2 file is left alone by v1: existing_schema_version (lib unit test)" --lib transport::config::tests::v1_leaves_v2_alone_and_v2_regenerates_v1
ctest "the join descriptor and apply_join round trip (the phone side the daemon does not expose)" --test transport_spine join_descriptor_and_apply_join_round_trip
ctest "cargo test --lib transport (renderer, interfaces, posture, rnstatus, join units)" --lib transport

# ---- 1b. the modes on a rendered scratch root (independent stat check)
RROOT=$SCRATCH/rendered
mkdir -p "$RROOT"
if (cd "$OS/1-bootstrap" && SPINE_KEEP_ROOT="$RROOT" cargo test -q -p node111 --test transport_spine \
  modes_on_a_scratch_root) >"$LOG" 2>&1; then
  want_mode() { # PATH MODE
    if [ ! -e "$RROOT/$1" ]; then fail "$1 is missing"; return; fi
    if [ -L "$RROOT/$1" ]; then fail "$1 is a symlink"; return; fi
    got=$(stat -c '%a' "$RROOT/$1")
    if [ "$got" = "$2" ]; then ok "rendered root: mode $2 $1"; else fail "rendered root: $1 has mode $got, want $2"; fi
  }
  want_mode rns 700
  want_mode rns/config 600
  want_mode secrets 700
  want_mode secrets/rns-rpc-key 600
  want_mode bridge/lxmf 700
  want_mode bridge/lxmf/rns 700
  want_mode bridge/lxmf/rns/config 600
  want_mode state/interfaces.toml 600
  if head -n 20 "$RROOT/rns/config" | grep -q '^schema-version = 2$'; then
    ok "rendered root: rns/config is schema-version 2"
  else
    fail "rendered root: rns/config does not carry schema-version = 2"
  fi
  key=$(sed -n 's/^  rpc_key = \([0-9a-f]\{64\}\)$/\1/p' "$RROOT/rns/config")
  if [ -n "$key" ] && grep -q "^  rpc_key = $key\$" "$RROOT/bridge/lxmf/rns/config"; then
    ok "rendered root: the helper's client config carries rnsd's rpc_key"
  else
    fail "rendered root: rpc_key missing or different between rns/config and bridge/lxmf/rns/config"
  fi
  if grep -q '^\[\[' "$RROOT/bridge/lxmf/rns/config"; then
    fail "rendered root: the helper's client config has an interface stanza"
  else
    ok "rendered root: the helper's client config has no interfaces"
  fi
else
  tail -n 30 "$LOG" | sed 's/^/  cargo| /'
  fail "rendering the scratch root (modes_on_a_scratch_root)"
fi

# ---- 2. prerequisites of the real-daemon part
phase "2. prerequisites: the gate's rns venv ($VENV_DIR), the debug 111"
if rns_venv_ensure "$VENV_DIR"; then
  ok "gate venv from the lock: rns $("$RNS_VENV/bin/pip" show rns 2>/dev/null | sed -n 's/^Version: //p'), lxmf $("$RNS_VENV/bin/pip" show lxmf 2>/dev/null | sed -n 's/^Version: //p'), $("$RNS_VENV/bin/python" --version 2>&1); $(ls "$RNS_WHEELS" | wc -l) wheels"
else
  skip_or_fail G4 "$RNS_VENV_ERR"
fi
TGT=${CARGO_TARGET_DIR:-$OS/1-bootstrap/target}
BIN111=$TGT/debug/111
if [ "${SMOKE_SKIP_BUILD:-0}" = 1 ]; then
  note "SMOKE_SKIP_BUILD=1 — cargo build skipped"
else
  t1=$(now)
  if (cd "$OS/1-bootstrap" && cargo build --bin 111) >"$LOG" 2>&1; then
    ok "cargo build --bin 111 ($(since "$t1")s)"
  else
    tail -20 "$LOG" | sed 's/^/  cargo| /'
    fail "cargo build --bin 111"
  fi
fi
[ -x "$BIN111" ] || { echo "FAIL: $BIN111 missing"; exit 1; }

phase "2b. scratch roots A (a Layer 5 root to migrate) and B ($SCRATCH)"
W=$SCRATCH/http
mkdir -p "$W"
for n in A B; do
  root=$SCRATCH/$n
  if [ $n = A ]; then
    g45_make_root "$root" "# G4 node A: LAN sharing on (the hub's Backbone listener and the pair code)
DASHBOARD_LAN=true"
    mkdir -p "$root/rns"
    cat >"$root/state/devices.json" <<'EOF'
{"schema": 1, "devices": {"abcd1234abcd1234": {
  "first_seen": "2026-10-01T00:00:00Z", "last_seen": "2026-10-01T00:00:00Z",
  "firmware": {"family": "meshtastic", "version": "2.7.11.abcdef0", "source": "probe"},
  "meshtastic": {"uplink": {"kind": "tcp", "host": "127.0.0.1", "port": 4403},
                 "node_num": 168430090, "node_id": "!0a0a0a0a", "hw_model": "HELTEC_V3"}}}}
EOF
    cat >"$root/rns/config" <<'EOF'
# 111 node rnsd config (Reticulum INI format), rendered by 111d. Do not edit.
# G4 fixture: the shape a Layer 5 (renderer v1) node left behind, plus a hand-added RNode stanza.
schema-version = 1

[reticulum]
  share_instance = No
  enable_transport = No

[interfaces]
  [[Auto Interface]]
    type = AutoInterface
    enabled = yes
    group_id = 111.network
  [[Bench RNode]]
    type = RNodeInterface
    enabled = yes
    port = /dev/ttyACM9
    frequency = 869525000
    bandwidth = 125000
    txpower = 14
    spreadingfactor = 8
    codingrate = 5
EOF
  else
    g45_make_root "$root" "# G4 node B: loopback dashboard (the phone)"
    mkdir -p "$root/releases/$G45_VER/share/lxmf-sidecar" "$root/cache/packages/lxmf"
    cp "$HELPER_SRC" "$root/releases/$G45_VER/share/lxmf-sidecar/lxmf_sidecar.py"
    cp "$RNS_WHEELS"/* "$root/cache/packages/lxmf/"
  fi
  nid=$(g45_identity "$root")
  printf '%s\n' "$nid" >"$SCRATCH/$n.nid"
  if [ -n "$nid" ]; then
    ok "$n: root built (identity ${nid:0:16}…; current → releases/$G45_VER; venvs/rns → the gate venv)"
  else
    fail "$n: 111 identity generate failed ($(head -c 200 "$root/identity.log"))"
  fi
done

if env --default-signal true 2>/dev/null; then
  RESET_SIG=(env --default-signal)
else
  RESET_SIG=(python3 -c 'import os,signal,sys
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])')
fi
INNER_LOG=$SCRATCH/inner.log
G4_WORK=$SCRATCH BIN111=$BIN111 RNS_VENV=$RNS_VENV RNS_LOCK=$RNS_LOCK RNS_LOCKSHA=$RNS_LOCKSHA G45_T0=$T0 \
  unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
  "${RESET_SIG[@]}" bash "$HERE/spine.sh" --inner 2>&1 | tee "$INNER_LOG" || true
inner_fails=$(grep -c '^FAIL:' "$INNER_LOG" || true)
inner_oks=$(grep -c '^ok:' "$INNER_LOG" || true)
if ! grep -q '^FAILS=' "$SCRATCH/inner.result" 2>/dev/null; then
  fail "the namespace run did not finish (killed or crashed — see above)"
fi
FAILS=$((FAILS + inner_fails))

# ---- hygiene: nothing of ours outlives the namespaces
phase "10. hygiene"
left=""
for p in /proc/[0-9]*; do
  p=${p#/proc/}
  [ "$p" = "$$" ] && continue
  c=$(tr '\0' ' ' <"/proc/$p/cmdline" 2>/dev/null)
  case "$c" in *"$SCRATCH"*) left="$left $p:${c:0:60}" ;; esac
done
t "hygiene: no process of this run is left (daemons, rnsd, helper, rnstatus, holders)" '[ -z "$left" ]'
[ -z "$left" ] || note "left:$left"
echo
echo "G4: $((OKS + inner_oks)) ok, $FAILS FAIL in $(elapsed)s"
finish G4
