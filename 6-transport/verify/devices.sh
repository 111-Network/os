#!/usr/bin/env bash
# 6-transport/verify/devices.sh — Layer 6 gate G5 (spec §16 G5; §12 whole): the device core on a real daemon.
#
#   bash 6-transport/verify/devices.sh                 runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G5=1 bash 6-transport/verify/devices.sh    lock run: a missing prerequisite is a FAIL
#
# 1. C4's first cut, kept: the device core's acceptance tests (cargo test --test devices_core: uevent fixtures
#    replayed through the fixture HotplugSource kick the tick and the 2-tick debounce holds, the PortTable
#    collisions, the pre-bind check, the identify sequence on fakes, the applier with injected probe failures),
#    the module tests (cargo test --lib devices), the seven uevent fixtures checked independently of the Rust
#    parser, and AF_NETLINK kept by every unit that restricts address families.
# 2. One real `111 daemon` (the debug build) in an unprivileged user + pid + mount + network namespace whose /dev
#    and /sys/{bus,class/tty} are the gate's own tmpfs trees: twelve CP2102 boards (10c4:ea60, the Heltec V3 row of
#    lora-table.toml) whose ttys are ptys served by verify/devices/meshcore-pty.py (the MeshCore pty stub, modelled
#    only on the MIT kiss_modem_protocol.md facts of R05 §3.1), the gate venv's real rnsd (verify/rns-venv.sh),
#    verify/devices/fake-rnodeconf.sh as venvs/rns/bin/rnodeconf, verify/devices/stub-meshmud.py as meshmud and as
#    the presence reader. LAN sharing is on (a veth address) so the proposals are accepted on a paired session.
#    Checks:
#    - hotplug: the daemon holds one NETLINK_KOBJECT_UEVENT socket (the production HotplugSource); a board added
#      after start appears within two poll ticks and leaves when unplugged (the fallback; a kernel uevent cannot be
#      sent into this namespace — the kick itself is step 1's fixture replay);
#    - the identify sequence through POST /devices/{id}/identify on the paired session: the fake rnodeconf (Match,
#      the sequence stops), the stub meshmud (Match: bound, the port handed to meshmud), the MeshCore pty stub
#      (Match: GetVersion then GetIdentity asked), three NoMatch ⇒ unknown persisted, a missing rnodeconf ⇒ Error
#      and unknown NOT persisted, consent required (no flag, false), the 30 s rate limit (429 with a reason), the
#      agent's bearer refused;
#    - PortTable collisions answer 409 port_busy through the API: rnsd (an interfaces.toml entry) against an
#      identify, meshmud (the bound board) against an identify and an upsert, a running identify probe against an
#      upsert; a foreign loopback holder (a python socket on Madmail's IMAP port 11242) is caught by the pre-bind
#      check: component enable answers 409 port_busy naming it (needs Madmail's pinned payload, fetched once into
#      the gate's cache with its sha256 checked; without it that check is SKIPPED, or a FAIL under REQUIRE_G5=1);
#    - the applier's four actions through kind: device proposals accepted on the paired session: radio.region.set
#      kept; rns.interface.upsert of a board with no RNode behind its pty (rnsd shows it Down) reverted byte-exact
#      (interfaces.toml and rns/config) after the 30 s probe; a crash mid-apply (kill -9 the daemon inside the
#      probe window) leaves applied: {pending: true} and a restart never applies it again (409 already_decided);
#      rns.interface.remove of that half-applied entry kept (absent from rnstatus); rns.posture.set kept (rnsd
#      restarted); unsupported_action for meshtastic.set, ble.pair, ble.forget, firmware.flash (the proposal stays
#      open); the one-shots: identify through a proposal (accepting is the consent; a dismissed one probes
#      nothing), service.restart of a supervised child (rnsd restarts; the route answers 202) and of an
#      unsupervised one (409 not_supervised).
# 3. Bench steps (external gate G1, the owner's hardware) are printed as note: lines: they are not G5's.
#
# Prerequisites: unprivileged user + network + mount namespaces, python3 >= 3.11 with venv, ip, nsenter, curl, jq,
# ss, mount; the debug 111 (cargo build; SMOKE_SKIP_BUILD=1 reuses it). Nothing of ours stays running: every
# process dies with the pid namespace or is stopped first; the scratch root is removed on success (kept on
# failure or with SMOKE_KEEP=1); the venv and the Madmail payload cache are removed unless G5_KEEP_CACHE=1.
# Checks are condition strings eval'd by t() at check time:
# shellcheck disable=SC2016,SC2034,SC2317
set -u

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
OS=$(cd "$HERE/../.." && pwd)
export PATH="$HOME/.cargo/bin:$PATH:/usr/sbin:/sbin"
FIXD=$HERE/devices
GOLDENS=$OS/1-bootstrap/crates/node111/tests/fixtures/rnodeconf
LANIP=192.168.77.5
MADMAIL_URL=https://github.com/themadorg/madmail/releases/download/v2.30.5/madmail-linux-amd64
MADMAIL_SHA=ad89fb23de9580cd736c7caa8136422415cb8779954c16e366d94f9325b989d1
. "$HERE/lib.sh"
. "$HERE/rns-venv.sh"
. "$HERE/spine/node.sh"

BOARDS=12
byid() { printf 'usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_G5D%s-if00-port0' "$1"; }
devpath() { printf '/dev/serial/by-id/%s' "$(byid "$1")"; }

# ------------------------------------------------------------------ inner (in the namespaces)

# the gate's private /dev and /sys trees (this mount namespace only; the host's are never written)
fake_trees() {
  local keep=$WORK/olddev
  mkdir -p "$keep"
  mount --rbind /dev "$keep" || return 1
  mount -t tmpfs -o mode=755 tmpfs /dev || return 1
  local n
  for n in null zero random urandom tty full; do
    touch "/dev/$n" && mount --bind "$keep/$n" "/dev/$n" || return 1
  done
  mkdir -p /dev/pts /dev/shm /dev/serial/by-id
  mount -t devpts -o newinstance,ptmxmode=0666,mode=620 devpts /dev/pts || return 1
  ln -s pts/ptmx /dev/ptmx
  mount -t tmpfs tmpfs /dev/shm
  ln -s /proc/self/fd /dev/fd
  ln -s fd/0 /dev/stdin
  ln -s fd/1 /dev/stdout
  ln -s fd/2 /dev/stderr
  umount -l "$keep"
  mount -t tmpfs tmpfs /sys/bus || return 1
  mount -t tmpfs tmpfs /sys/class/tty || return 1
  mkdir -p /sys/bus/usb/devices
}
# plug BOARD — the sysfs USB device + interface + tty, /dev/ttyUSB<n> (a pty served by the stub), the by-id link
plug() {
  local i=$1 bus=1-$1 st=$WORK/g5/pty/$1
  local dir=/sys/bus/usb/devices/$bus
  mkdir -p "$dir/$bus:1.0/ttyUSB$i" "$st"
  printf '10c4\n' >"$dir/idVendor"
  printf 'ea60\n' >"$dir/idProduct"
  printf '00\n' >"$dir/bDeviceClass"
  printf 'Silicon Labs\n' >"$dir/manufacturer"
  printf 'CP2102 USB to UART Bridge Controller\n' >"$dir/product"
  printf 'G5D%s\n' "$i" >"$dir/serial"
  printf 'ff\n' >"$dir/$bus:1.0/bInterfaceClass"
  [ -f "$st/mode" ] || printf 'silent\n' >"$st/mode"
  rm -f "$st/ready"
  python3 -I "$FIXD/meshcore-pty.py" --link "/dev/ttyUSB$i" --state "$st" >"$st/stub.log" 2>&1 &
  HELPERS+=("$!")
  PTYPID[$i]=$!
  wait_for 5 '[ -s "$st/ready" ]' || return 1
  mkdir -p "/sys/class/tty/ttyUSB$i"
  ln -s "$dir/$bus:1.0" "/sys/class/tty/ttyUSB$i/device"
  ln -s "../../ttyUSB$i" "/dev/serial/by-id/$(byid "$i")"
}
unplug() {
  local i=$1
  rm -f "/dev/serial/by-id/$(byid "$i")" "/sys/class/tty/ttyUSB$i/device"
  rmdir "/sys/class/tty/ttyUSB$i" 2>/dev/null
  rm -rf "/sys/bus/usb/devices/1-$i"
  kill -TERM "${PTYPID[$i]}" 2>/dev/null
  rm -f "/dev/ttyUSB$i"
}
# the device id of BOARD (from GET /devices: the record naming its USB serial) → $DEV
dev_of() {
  api N GET /api/v1/devices
  cp "$W/body" "$W/devices.json" 2>/dev/null
  DEV=$(jq -r --arg s "G5D$1" '[.devices[]? | select(tostring | test("\\b" + $s + "\\b"))] | first | .id // empty' "$W/devices.json" 2>/dev/null)
}
rec_of() { # BOARD → $W/rec.json (the device record)
  api N GET "/api/v1/devices/${ID[$1]}"
  cp "$W/body" "$W/rec.json" 2>/dev/null
}
family_of() { rec_of "$1"; jq -r '.firmware.family // empty' "$W/rec.json" 2>/dev/null; }
# paired-session calls (the LAN source address and the cookie)
lan() { # METHOD PATH [curl args…]
  local m=$1 p=$2
  shift 2
  NSX=(nsenter -t "${HOLD[N]}" -n --)
  req "$m" "http://$LANIP:11180$p" --interface "$LANIP" -b "$JAR" "$@"
  NSX=()
}
lanj() { lan "$1" "$2" -H 'Content-Type: application/json' -d "$3" "${@:4}"; }
identify() { REQ_TIMEOUT=${2:-150} lanj POST "/api/v1/devices/${ID[$1]}/identify" '{"consent": true}'; } # BOARD
asked() { { tr '\n' ' ' <"$WORK/g5/pty/$1/asked"; } 2>/dev/null | sed 's/ $//'; }
rnodeconf_calls_for() { grep -c "G5D$1-" "$WORK/g5/rnodeconf-calls.log" 2>/dev/null || true; }
# propose + accept on the paired session → $ST and $W/body; PROPS++
PROPS=0
accept() { # ID BODY [timeout]
  propose_device N "$1" "$NID" "$2"
  REQ_TIMEOUT=${3:-90} lanj POST "/api/v1/proposals/$1/decision" '{"decision": "accepted"}'
}
dec() { jq -c "$2" "$(decision_file N "$1")" 2>/dev/null; } # ID JQ
toml_name_of() { # DEVICE-ID → the interfaces.toml entry name for that device
  python3 -I -c 'import sys, tomllib
f = tomllib.load(open(sys.argv[1], "rb"))
print(next((i["name"] for i in f.get("interface", []) if i.get("device") == sys.argv[2]), ""))' "${ROOT[N]}/state/interfaces.toml" "$1" 2>/dev/null
}

inner() {
  : "${G5_WORK:?}" "${BIN111:?}" "${RNS_VENV:?}"
  WORK=$G5_WORK
  W=$WORK/http
  mkdir -p "$W" "$WORK/g5/pty" "$WORK/g5/rnodeconf" "$WORK/g5/meshmud"
  : >"$WORK/g5/rnodeconf-calls.log"
  : >"$WORK/g5/meshmud-calls.log"
  ROOT[N]=$WORK/N
  NID=$(cat "$WORK/N.nid" 2>/dev/null)
  JAR=$W/paired.jar
  declare -gA ID PTYPID
  cleanup_inner() {
    local p=${DPID[N]:-}
    if [ -n "$p" ] && kill -0 "$p" 2>/dev/null; then kill -TERM "$p" 2>/dev/null || true; fi
    for _ in $(seq 1 75); do kill -0 "${DPID[N]:-0}" 2>/dev/null || break; sleep 0.2; done
    for p in "${HELPERS[@]}"; do kill -TERM "$p" 2>/dev/null || true; done
    printf 'FAILS=%s\nOKS=%s\n' "$FAILS" "$OKS" >"$WORK/inner.result"
  }
  trap cleanup_inner EXIT
  trap 'exit 130' INT TERM
  local i d0 r0 r1 sha_t sha_c p id

  # ---------------------------------------------------------------- 1. namespace, trees, boards
  phase "1. namespace: private /dev and /sys trees, $BOARDS CP2102 boards on ptys, LAN $LANIP"
  ip link set lo up
  if fake_trees; then
    ok "private /dev (tmpfs; null, zero, random, urandom, tty, full bound; a new devpts) and /sys/bus, /sys/class/tty (tmpfs) in this mount namespace"
  else
    fail "the private /dev and /sys trees could not be mounted"
  fi
  t "positive control: the host's /dev/serial is not ours to touch (ours is a tmpfs here: $(stat -f -c %T /dev))" '[ "$(stat -f -c %T /dev)" = tmpfs ]'
  make_netns N
  if nx N ip link add eth0 type veth peer name eth1 && nx N ip link set eth1 up &&
    nx N ip addr add "$LANIP/24" dev eth0 && nx N ip link set eth0 up; then
    ok "netns N (holder ${HOLD[N]}): eth0 $LANIP/24 (a veth pair inside N) for the paired LAN session"
  else
    fail "netns N: the LAN address could not be set up"
  fi
  for i in $(seq 1 $((BOARDS - 1))); do
    plug "$i" || fail "board $i: the pty stub did not come up"
  done
  t "boards 1–$((BOARDS - 1)): /dev/serial/by-id links → /dev/ttyUSB<n> → a pty each, sysfs CP2102 10c4:ea60 behind each tty" \
    '[ "$(ls /dev/serial/by-id | wc -l)" = $((BOARDS - 1)) ] && [ -c "$(readlink -f "$(devpath 1)")" ] &&
     [ "$(cat "$(readlink -f /sys/class/tty/ttyUSB1/device)/../idVendor")" = 10c4 ]'
  # the board stand-ins' modes (the files are read at each answer)
  printf 'info-provisioned.txt\n' >"$WORK/g5/rnodeconf/G5D1-"
  printf 'info-tnc.txt\n' >"$WORK/g5/rnodeconf/G5D10-"
  printf 'match\n0a0a0a0a\n' >"$WORK/g5/meshmud/G5D2-"
  printf 'slow:14\n' >"$WORK/g5/meshmud/G5D7-"
  printf 'meshcore\n' >"$WORK/g5/pty/3/mode"
  # board 6: rnsd's (an interfaces.toml entry before the start; the region set so it renders)
  cat >"${ROOT[N]}/state/interfaces.toml" <<EOF
# G5: board 6 is driven by rnsd (owner = "rnsd"): an identify of it must answer port_busy
schema = 1

[radio]
region = "EU_868"

[[interface]]
name = "G5 board 6"
owner = "rnsd"
type = "RNodeInterface"
port = "$(devpath 6)"
params = { frequency = 869525000, bandwidth = 125000, txpower = 14, spreadingfactor = 8, codingrate = 5 }
EOF
  chmod 600 "${ROOT[N]}/state/interfaces.toml"

  # ---------------------------------------------------------------- 2. the daemon, hotplug, pairing
  phase "2. N: 111 daemon, the boards in GET /devices, hotplug, the paired session"
  start_node N
  tw 30 "N: rnsd runs (pid $(rnsd_pid N))" '[ -n "$(rnsd_pid N)" ]'
  local inodes nl
  inodes=$(for f in /proc/"${DPID[N]}"/fd/*; do readlink "$f"; done 2>/dev/null | sed -n 's/^socket:\[\([0-9]*\)\]$/\1/p' | sort -u)
  nl=$(nx N awk 'NR > 1 && $2 == 15 {print $10, $4}' /proc/net/netlink 2>/dev/null | while read -r ino grp; do
    printf '%s\n' "$inodes" | grep -qx "$ino" && printf '%s ' "$grp"; done)
  t "N: the daemon holds one NETLINK_KOBJECT_UEVENT socket (the production HotplugSource; groups ${nl:-none})" \
    '[ "$(printf "%s" "$nl" | wc -w)" = 1 ] && [ $((16#$(printf "%s" "$nl" | tr -d " ") & 1)) = 1 ]'
  tw 15 "N: GET /devices lists the $((BOARDS - 1)) boards as radios (two 5 s poll ticks at most)" \
    'api N GET /api/v1/devices; jb "[.devices[] | select(.kind == \"radio\")] | length == $((BOARDS - 1))"'
  cp "$W/body" "$WORK/devices-1.json"
  for i in $(seq 1 $((BOARDS - 1))); do
    dev_of "$i"
    ID[$i]=$DEV
  done
  t "N: each board has its own device id (${ID[1]}, ${ID[2]}, …)" \
    '[ "$(printf "%s\n" "${ID[@]}" | grep -c .)" = $((BOARDS - 1)) ] && [ "$(printf "%s\n" "${ID[@]}" | sort -u | grep -c .)" = $((BOARDS - 1)) ]'
  t "N: identify is offered on a free board (board 1 actions $(jq -c --arg d "${ID[1]}" '.devices[] | select(.id == $d) | .actions' "$WORK/devices-1.json"))" \
    'jq -e --arg d "${ID[1]}" ".devices[] | select(.id == \$d) | .actions | index(\"identify\") != null" "$WORK/devices-1.json" >/dev/null'
  rec_of 6
  t "N: board 6 (rnsd's interfaces.toml entry) reads port_busy naming rnsd and offers no identify" \
    'jf "$W/rec.json" ".reason_code == \"port_busy\" and .detail.port_held_by == \"rnsd\" and (.actions | index(\"identify\") == null)"'
  d0=$(now)
  plug "$BOARDS" || fail "board $BOARDS: the pty stub did not come up"
  tw 15 "hotplug fallback: board $BOARDS plugged after start appears in GET /devices" \
    'api N GET /api/v1/devices; jb "[.devices[] | select(tostring | test(\"G5D$BOARDS\\\\b\")) | select(.state != \"disconnected\")] | length == 1"'
  note "board $BOARDS appeared $(since "$d0")s after the plug (no kernel uevent reaches this namespace: the 5 s poll, two ticks)"
  unplug "$BOARDS"
  tw 15 "hotplug fallback: unplugged, board $BOARDS reads disconnected" \
    'api N GET /api/v1/devices; jb "[.devices[] | select(tostring | test(\"G5D$BOARDS\\\\b\")) | select(.state == \"disconnected\")] | length == 1"'
  NSX=(nsenter -t "${HOLD[N]}" -n --)
  pair_session N "http://$LANIP:11180" "$JAR" --interface "$LANIP"
  NSX=()
  t "N: a LAN session paired with the pair code from \`111 dashboard\` (POST /auth/pair from $LANIP → 200 paired)" \
    '[ "$ST" = 200 ] && jb ".paired == true and .loopback == false"'
  lanj POST "/api/v1/devices/${ID[1]}/identify" '{"consent": true}' -H 'Origin: http://evil.example'
  t "N: a cross-site identify on the paired session → 403" '[ "$ST" = 403 ]'

  # ---------------------------------------------------------------- 3. identify
  phase "3. the identify sequence through the API (fake rnodeconf, stub meshmud, MeshCore pty stub)"
  lanj POST "/api/v1/devices/${ID[1]}/identify" '{}'
  t "consent required: identify without the consent flag → 400" '[ "$ST" = 400 ]'
  lanj POST "/api/v1/devices/${ID[1]}/identify" '{"consent": false}'
  t "consent required: {\"consent\": false} → 400" '[ "$ST" = 400 ]'
  api N POST "/api/v1/devices/${ID[1]}/identify" -H "Authorization: Bearer $(cat "${ROOT[N]}/agent/g45-agent.token" 2>/dev/null)" \
    -H 'Content-Type: application/json' -d '{"consent": true}'
  t "the agent's bearer on identify (with the consent flag) → 401" '[ "$ST" = 401 ]'
  t "… none of these ran a probe (rnodeconf never ran, no pty was asked)" '[ ! -s "$WORK/g5/rnodeconf-calls.log" ] && [ -z "$(asked 1)" ]'
  identify 1
  cp "$W/body" "$WORK/identify-1.json"
  t "board 1: the fake rnodeconf answers Device info → 200, firmware rnode 1.86 (source probe)" \
    '[ "$ST" = 200 ] && jb ".firmware.family == \"rnode\" and .firmware.version == \"1.86\" and .firmware.source == \"probe\""'
  t "board 1: rnodeconf ran once as \`--info <the by-id port>\`; the sequence stopped (no meshmud start, the pty never asked)" \
    '[ "$(rnodeconf_calls_for 1)" = 1 ] && grep -q -- "--info $(devpath 1)" "$WORK/g5/rnodeconf-calls.log" &&
     ! grep -q "G5D1-" "$WORK/g5/meshmud-calls.log" 2>/dev/null && [ -z "$(asked 1)" ]'
  t "board 1: the family is persisted (GET /devices/{id}: rnode) and the consent remembered (state/identify-consent.json 0600)" \
    '[ "$(family_of 1)" = rnode ] && [ "$(stat -c %a "${ROOT[N]}/state/identify-consent.json" 2>/dev/null)" = 600 ] &&
     grep -q "${ID[1]}" "${ROOT[N]}/state/identify-consent.json"'
  identify 1
  t "board 1 again within 30 s → 429 rate_limited with a reason (\"$(jget '.detail // .title' | head -c 100)\")" \
    '[ "$ST" = 429 ] && [ "$(jget .code)" = rate_limited ] && [ -n "$(jget ".detail // .title // empty")" ]'
  t "… and the refused probe ran nothing" '[ "$(rnodeconf_calls_for 1)" = 1 ]'
  identify 3
  cp "$W/body" "$WORK/identify-3.json"
  t "board 3: rnodeconf NoMatch, meshmud NoMatch, the MeshCore pty stub answers → 200 firmware meshcore 3" \
    '[ "$ST" = 200 ] && jb ".firmware.family == \"meshcore\" and .firmware.version == \"3\" and .firmware.source == \"probe\""'
  t "board 3: the stub was asked GetVersion then GetIdentity (sub-commands $(asked 3)); rnodeconf and meshmud ran first" \
    '[ "$(asked 3)" = "11 01" ] && [ "$(rnodeconf_calls_for 3)" = 1 ] && grep -q "want_config $(devpath 3) nomatch" "$WORK/g5/meshmud-calls.log" 2>/dev/null'
  t "board 3: meshcore persisted" '[ "$(family_of 3)" = meshcore ]'
  identify 4
  cp "$W/body" "$WORK/identify-4.json"
  t "board 4: three NoMatch (rnodeconf no-rnode, meshmud nomatch, a silent pty) → 200 and family unknown persisted" \
    '[ "$ST" = 200 ] && jb ".firmware.family == \"unknown\"" && [ "$(family_of 4)" = unknown ]'
  t "board 4: every step ran (rnodeconf, meshmud, GetVersion asked)" \
    '[ "$(rnodeconf_calls_for 4)" = 1 ] && grep -q "want_config $(devpath 4) nomatch" "$WORK/g5/meshmud-calls.log" && [[ "$(asked 4)" == 11* ]]'
  mv "${ROOT[N]}/venvs/rns/bin/rnodeconf" "$WORK/g5/rnodeconf.away"
  identify 5
  cp "$W/body" "$WORK/identify-5.json"
  t "board 5: rnodeconf missing ⇒ Error (not NoMatch): with two NoMatch the run is inconclusive → 502 naming the missing tool" \
    '[ "$ST" = 502 ] && grep -qi "rnodeconf" "$W/body"'
  t "board 5: unknown NOT persisted (no probe result: firmware $(rec_of 5; jq -c .firmware "$W/rec.json"), against board 4's source probe)" \
    'rec_of 5; jf "$W/rec.json" ".firmware == null or .firmware.source != \"probe\"" && rec_of 4 && jf "$W/rec.json" ".firmware.source == \"probe\""'
  mv "$WORK/g5/rnodeconf.away" "${ROOT[N]}/venvs/rns/bin/rnodeconf"
  identify 6
  t "PortTable: rnsd holds board 6 → identify answers 409 port_busy naming rnsd; nothing ran on it" \
    '[ "$ST" = 409 ] && [ "$(jget .code)" = port_busy ] && grep -q rnsd "$W/body" && [ "$(rnodeconf_calls_for 6)" = 0 ]'
  rs N
  note "board 6 under rnsd: RNodeInterface[G5 board 6] Status $(rs_status "G5 board 6" | tr -d ' ') (a pty with no RNode behind it; the stub saw $(sort -u "$WORK/g5/pty/6/other" 2>/dev/null | tr '\n' ' ')frame types and answered none)"
  t "board 6: a pty with no RNode behind it shows Down while rnsd stays up" '[ "$(rs_status "G5 board 6")" = Down ] && [ "$(rs_status "rns/111")" = Up ]'

  # ---------------------------------------------------------------- 4. the applier through proposals
  phase "4. the applier's four actions through kind: device proposals on the paired session"
  accept prop-g5-region '{"why":"G5 region","actions":[{"action":"radio.region.set","region":"US"}]}'
  t "radio.region.set US → 200, the decision record applied and kept" \
    '[ "$ST" = 200 ] && [ "$(dec prop-g5-region ".applied.applied")" = true ] && [ "$(dec prop-g5-region ".applied.reverted")" = false ]'
  t "… [radio] region = \"US\" in interfaces.toml, which still parses" \
    'grep -q "^region = \"US\"" "${ROOT[N]}/state/interfaces.toml" && python3 -I -c "import tomllib,sys; tomllib.load(open(sys.argv[1],\"rb\"))" "${ROOT[N]}/state/interfaces.toml"'
  accept prop-g5-region-back '{"why":"G5 region","actions":[{"action":"radio.region.set","region":"EU_868"}]}'
  t "radio.region.set EU_868 → kept" '[ "$ST" = 200 ] && [ "$(dec prop-g5-region-back ".applied.applied")" = true ]'
  for a in meshtastic.set ble.pair ble.forget firmware.flash; do
    accept "prop-g5-u-${a//./-}" "$(jq -nc --arg a "$a" --arg d "${ID[8]}" '{why: "G5", actions: [{action: $a, device_id: $d}]}')"
    t "$a → 409 unsupported_action; no decision is written, the proposal stays open" \
      '[ "$ST" = 409 ] && [ "$(jget .code)" = unsupported_action ] && [ ! -e "$(decision_file N "prop-g5-u-${a//./-}")" ] &&
       api N GET "/api/v1/proposals/prop-g5-u-${a//./-}" && jb ".decision == null"'
  done
  upsert() { jq -nc --arg d "$1" '{why: "G5 upsert", actions: [{action: "rns.interface.upsert", device_id: $d, type: "RNodeInterface",
    params: {frequency: 869525000, bandwidth: 125000, txpower: 14, spreadingfactor: 8, codingrate: 5}}]}'; }
  sha_t=$(sha256sum "${ROOT[N]}/state/interfaces.toml" | cut -c1-64)
  sha_c=$(sha256sum "${ROOT[N]}/rns/config" | cut -c1-64)
  r0=$(rnsd_pid N)
  d0=$(now)
  accept prop-g5-upsert "$(upsert "${ID[8]}")" 120
  cp "$W/body" "$WORK/upsert-8.json"
  t "rns.interface.upsert of board 8 (no RNode behind its pty) → the probe saw it Down for 30 s and reverted ($(since "$d0")s; reason \"$(dec prop-g5-upsert '.applied.reason' | head -c 120)\")" \
    '[ "$ST" = 200 ] && [ "$(dec prop-g5-upsert ".applied.applied")" = false ] && [ "$(dec prop-g5-upsert ".applied.reverted")" = true ]'
  t "… interfaces.toml and rns/config are byte-exact as before (sha256)" \
    '[ "$(sha256sum "${ROOT[N]}/state/interfaces.toml" | cut -c1-64)" = "$sha_t" ] && [ "$(sha256sum "${ROOT[N]}/rns/config" | cut -c1-64)" = "$sha_c" ]'
  t "… rnsd detached it (no stanza of board 8 in rnstatus), still the same rnsd ($r0)" \
    'rs N; ! printf "%s" "$RS" | grep -q "G5D8\|board 8" && [ "$(rnsd_pid N)" = "$r0" ] && [ -n "$(rs_block "rns/111")" ]'
  note "board 8's attach was real: the stub saw $(sort -u "$WORK/g5/pty/8/other" 2>/dev/null | wc -l) frame type(s) from rnsd while it was attached"
  # a crash mid-apply: kill -9 the daemon inside the probe window
  propose_device N prop-g5-crash "$NID" "$(upsert "${ID[9]}")"
  bg_in N "$W/crash.out" curl -sS -o "$W/crash.json" -w '%{http_code}' --max-time 120 --interface "$LANIP" -b "$JAR" \
    -X POST -H 'Content-Type: application/json' -d '{"decision": "accepted"}' "http://$LANIP:11180/api/v1/proposals/prop-g5-crash/decision"
  tw 20 "crash mid-apply: the pending record is on disk and the entry of board 9 saved before the probe" \
    '[ "$(dec prop-g5-crash ".applied.pending")" = true ] && grep -q "${ID[9]}" "${ROOT[N]}/state/interfaces.toml"'
  sleep 2
  kill9_node N
  local dsha tsha
  dsha=$(sha256sum "$(decision_file N prop-g5-crash)" | cut -c1-64)
  tsha=$(sha256sum "${ROOT[N]}/state/interfaces.toml" | cut -c1-64)
  t "… kill -9 of the daemon inside the probe window: the record stays applied: {pending: true}" \
    '[ "$(dec prop-g5-crash ".applied.pending")" = true ] && [ "$(dec prop-g5-crash ".decision")" = "\"accepted\"" ]'
  tw 15 "… its children went with it (no rnsd left)" '[ -z "$(rnsd_pid N)" ]'
  start_node N
  tw 30 "N: back up with rnsd" '[ -n "$(rnsd_pid N)" ]'
  api N GET /api/v1/proposals/prop-g5-crash
  t "after the restart the proposal reads decided (accepted, half-applied)" '[ "$ST" = 200 ] && jb ".decision.decision == \"accepted\""'
  lanj POST /api/v1/proposals/prop-g5-crash/decision '{"decision": "accepted"}'
  t "a retried accept → 409 already_decided: never applied a second time" '[ "$ST" = 409 ] && [ "$(jget .code)" = already_decided ]'
  t "… the decision record and interfaces.toml are unchanged by the restart and the retry (sha256)" \
    '[ "$(sha256sum "$(decision_file N prop-g5-crash)" | cut -c1-64)" = "$dsha" ] && [ "$(sha256sum "${ROOT[N]}/state/interfaces.toml" | cut -c1-64)" = "$tsha" ]'
  local nm9
  nm9=$(toml_name_of "${ID[9]}")
  tw 20 "the half-applied entry \"$nm9\" is visible in rnstatus (rendered at the start)" 'rs N; [ -n "$nm9" ] && [ -n "$(rs_block "$nm9")" ]'
  accept prop-g5-remove "$(jq -nc --arg n "$nm9" '{why: "G5 remove", actions: [{action: "rns.interface.remove", name: $n}]}')" 60
  t "rns.interface.remove \"$nm9\" → kept" '[ "$ST" = 200 ] && [ "$(dec prop-g5-remove ".applied.applied")" = true ]'
  t "… absent from rnstatus and from interfaces.toml" 'rs N; [ -z "$(rs_block "$nm9")" ] && [ -z "$(toml_name_of "${ID[9]}")" ] && [ -n "$(rs_block "rns/111")" ]'
  r0=$(rnsd_pid N)
  local pl0
  pl0=$(wc -l <"${ROOT[N]}/rns/rnsd.log")
  accept prop-g5-posture '{"why":"G5 posture","actions":[{"action":"rns.posture.set","enable_transport":true}]}' 120
  t "rns.posture.set {enable_transport: true} → kept (rnsd restarted: $r0 → $(rnsd_pid N); rns/config enable_transport = Yes)" \
    '[ "$ST" = 200 ] && [ "$(dec prop-g5-posture ".applied.applied")" = true ] && [ -n "$(rnsd_pid N)" ] && [ "$(rnsd_pid N)" != "$r0" ] &&
     grep -q "enable_transport = Yes" "${ROOT[N]}/rns/config"'
  t "… the restarted rnsd became the instance, not a client of a transient holder of @rns/111 (rnsd.log: $(tail -n +"$((pl0 + 1))" "${ROOT[N]}/rns/rnsd.log" | grep -c 'connected to another shared local instance') \"connected to another shared local instance\" line(s))" \
    '! tail -n +"$((pl0 + 1))" "${ROOT[N]}/rns/rnsd.log" | grep -q "connected to another shared local instance"'

  # ---------------------------------------------------------------- 5. one-shots and the remaining collisions
  phase "5. the one-shots, port_busy for meshmud and a running probe"
  propose_device N prop-g5-dismiss "$NID" "$(jq -nc --arg d "${ID[11]}" '{why: "G5", actions: [{action: "identify", device_id: $d}]}')"
  lanj POST /api/v1/proposals/prop-g5-dismiss/decision '{"decision": "dismissed"}'
  t "an identify proposal dismissed → 200 dismissed, and nothing was probed on board 11" \
    '[ "$ST" = 200 ] && jb ".decision.decision == \"dismissed\"" && [ "$(rnodeconf_calls_for 11)" = 0 ] && [ -z "$(asked 11)" ]'
  accept prop-g5-identify "$(jq -nc --arg d "${ID[10]}" '{why: "G5", actions: [{action: "identify", device_id: $d}]}')" 150
  t "identify through a proposal (accepting it is the consent) → 200, the result in the record: rnode 1.85" \
    '[ "$ST" = 200 ] && [ "$(dec prop-g5-identify ".applied.applied")" = true ] && [ "$(family_of 10)" = rnode ] &&
     jf "$W/rec.json" ".firmware.version == \"1.85\""'
  r0=$(rnsd_pid N)
  accept prop-g5-restart '{"why":"G5","actions":[{"action":"service.restart","service_id":"transport"}]}' 60
  wait_for 20 '[ -n "$(rnsd_pid N)" ] && [ "$(rnsd_pid N)" != "$r0" ]' || true
  t "service.restart transport (supervised) through a proposal → 200 applied, rnsd restarted ($r0 → $(rnsd_pid N))" \
    '[ "$ST" = 200 ] && [ "$(dec prop-g5-restart ".applied.applied")" = true ] && [ -n "$(rnsd_pid N)" ] && [ "$(rnsd_pid N)" != "$r0" ]'
  accept prop-g5-restart-ns '{"why":"G5","actions":[{"action":"service.restart","service_id":"gateway"}]}'
  t "service.restart gateway (not supervised) through a proposal → 409 not_supervised; no decision written" \
    '[ "$ST" = 409 ] && [ "$(jget .code)" = not_supervised ] && [ ! -e "$(decision_file N prop-g5-restart-ns)" ]'
  r0=$(rnsd_pid N)
  lanj POST /api/v1/services/transport/restart '{}'
  wait_for 20 '[ -n "$(rnsd_pid N)" ] && [ "$(rnsd_pid N)" != "$r0" ]' || true
  t "POST /services/transport/restart → 202, rnsd restarted ($r0 → $(rnsd_pid N))" '[ "$ST" = 202 ] && [ -n "$(rnsd_pid N)" ] && [ "$(rnsd_pid N)" != "$r0" ]'
  lanj POST /api/v1/services/gateway/restart '{}'
  t "POST /services/gateway/restart → 409 not_supervised" '[ "$ST" = 409 ] && [ "$(jget .code)" = not_supervised ]'
  # a running identify probe holds board 7 (the stub meshmud answers it only after 14 s)
  bg_in N "$W/id7.out" curl -sS -o "$W/id7.json" -w '%{http_code}' --max-time 150 --interface "$LANIP" -b "$JAR" \
    -X POST -H 'Content-Type: application/json' -d '{"consent": true}' "http://$LANIP:11180/api/v1/devices/${ID[7]}/identify"
  local idpid=$BGPID
  tw 15 "board 7: the identify probe is running (meshmud started on its port)" 'grep -q "start $(devpath 7)" "$WORK/g5/meshmud-calls.log" 2>/dev/null'
  sleep 6
  accept prop-g5-busy7 "$(upsert "${ID[7]}")"
  t "PortTable: an upsert of board 7 while its identify probe runs → 409 port_busy naming the probe (identify:…)" \
    '[ "$ST" = 409 ] && [ "$(jget .code)" = port_busy ] && grep -q "identify:" "$W/body" && [ ! -e "$(decision_file N prop-g5-busy7)" ]'
  wait "$idpid" 2>/dev/null
  t "board 7: the slow probe then finished (three NoMatch → unknown; HTTP $(cat "$W/id7.out" 2>/dev/null))" '[ "$(cat "$W/id7.out" 2>/dev/null)" = 200 ] && [ "$(family_of 7)" = unknown ]'
  identify 2
  cp "$W/body" "$WORK/identify-2.json"
  t "board 2: rnodeconf NoMatch, then the stub meshmud answers my_node_info → 200 firmware meshtastic 2.7.11 (source probe)" \
    '[ "$ST" = 200 ] && jb ".firmware.family == \"meshtastic\" and .firmware.source == \"probe\""'
  t "board 2: bound — interfaces.toml carries an owner = \"111d\" meshtastic entry on its port (the spine is the binding's home)" \
    'python3 -I -c "import sys, tomllib
f = tomllib.load(open(sys.argv[1], \"rb\"))
sys.exit(0 if any(i.get(\"owner\") == \"111d\" and i.get(\"type\") == \"meshtastic\" and sys.argv[2] in str(i.get(\"uplink\", \"\")) for i in f.get(\"interface\", [])) else 1)" \
     "${ROOT[N]}/state/interfaces.toml" "$(devpath 2)"'
  t "board 2: the identify child stays as meshmud (one start for board 2) and the MeshCore step never asked its pty" \
    '[ "$(grep -c "start $(devpath 2)" "$WORK/g5/meshmud-calls.log")" = 1 ] && [ -z "$(asked 2)" ] && [ -n "$(pids_like N stub-meshmud.py --role meshmud)" ]'
  sleep 6 # one registry tick: its port owners are read on the tick (a finished probe's claim reads until then)
  accept prop-g5-busy2 "$(upsert "${ID[2]}")"
  t "PortTable: an upsert of the bound board 2 → 409 port_busy naming meshmud" \
    '[ "$ST" = 409 ] && [ "$(jget .code)" = port_busy ] && grep -q meshmud "$W/body"'
  note "waiting out board 2's 30 s rate limit for the meshmud identify collision"
  sleep 31
  identify 2
  t "PortTable: identify of the bound board 2 → 409 port_busy naming meshmud" \
    '[ "$ST" = 409 ] && [ "$(jget .code)" = port_busy ] && grep -q meshmud "$W/body"'

  # ---------------------------------------------------------------- 6. the pre-bind check (a foreign loopback holder)
  phase "6. a foreign loopback holder on a component port: the pre-bind check"
  if [ -s "${ROOT[N]}/cache/packages/madmail/madmail-linux-amd64" ]; then
    local hold hpid notice
    bg_in N "$W/holder.out" python3 -I -c 'import socket, time
s = socket.socket(); s.bind(("127.0.0.1", 11242)); s.listen(1); print("held", flush=True); time.sleep(600)'
    hpid=$BGPID
    tw 5 "a python socket outside 111 listens on 127.0.0.1:11242 (Madmail's IMAP port)" 'nx N ss -ltnH 2>/dev/null | grep -q "127.0.0.1:11242 "'
    lanj POST /api/v1/bridge/components/madmail/enable '{}'
    notice=$(jget '.consent.hash // .hash // empty')
    REQ_TIMEOUT=120 lanj POST /api/v1/bridge/components/madmail/enable "$(jq -nc --arg h "$notice" '{accept_notice_hash: $h}')"
    cp "$W/body" "$WORK/enable-madmail-held.json"
    t "enable madmail (acknowledged) → 409 port_busy naming the holder outside 111 (\"$(jget '.detail // .title' | head -c 120)\")" \
      '[ "$ST" = 409 ] && [ "$(jget .code)" = port_busy ] && grep -qi "outside 111" "$W/body"'
    t "… nothing of madmail started" '[ -z "$(pids_like N madmail)" ]'
    kill "$hpid" 2>/dev/null
    tw 5 "the holder is gone" '! nx N ss -ltnH 2>/dev/null | grep -q "127.0.0.1:11242 "'
    REQ_TIMEOUT=120 lanj POST /api/v1/bridge/components/madmail/enable "$(jq -nc --arg h "$notice" '{accept_notice_hash: $h}')"
    cp "$W/body" "$WORK/enable-madmail.json"
    t "positive control: without the holder the same enable passes the pre-bind check → 200 (state $(jget .state))" '[ "$ST" = 200 ]'
    tw 20 "… madmail runs and listens on 127.0.0.1:11242" '[ -n "$(pids_like N madmail)" ] && nx N ss -ltnH 2>/dev/null | grep -q "127.0.0.1:11242 "'
    REQ_TIMEOUT=60 lanj POST /api/v1/bridge/components/madmail/disable '{}'
    tw 20 "disable madmail → 200 and it stops" '[ "$ST" = 200 ] && [ -z "$(pids_like N madmail)" ]'
  elif [ "${REQUIRE_G5:-0}" = 1 ]; then
    fail "the pre-bind check through the API needs Madmail's pinned payload in the gate cache (REQUIRE_G5=1)"
  else
    note "SKIPPED: the API-level pre-bind check (Madmail's pinned payload could not be cached); step 1's a_foreign_loopback_holder_is_caught_by_the_pre_bind_check covers the check itself"
  fi

  # ---------------------------------------------------------------- 7. stop
  phase "7. stop"
  stop_node N
  t "N: 111 daemon exits 0 on SIGTERM (rc $RC)" '[ "$RC" = 0 ]'
  tw 15 "N: nothing of the daemon is left (rnsd, meshmud, the reader)" \
    '[ -z "$(rnsd_pid N)" ] && [ -z "$(pids_like N stub-meshmud.py)" ] && [ -z "$(pids_like N madmail)" ]'
}

if [ "${1:-}" = --inner ]; then
  inner
  exit $((FAILS > 0))
fi

# ------------------------------------------------------------------ outer
echo "6-transport/verify/devices.sh — Layer 6 gate G5"
command -v cargo >/dev/null 2>&1 || skip_or_fail G5 "cargo is not installed"
SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/111-devices.XXXXXX") || { echo "FAIL: mktemp"; exit 1; }
CACHE=${G5_CACHE:-${TMPDIR:-/tmp}/111-g5-cache}
cleanup() {
  if [ "${SMOKE_KEEP:-0}" = 1 ] || [ "$FAILS" -gt 0 ]; then
    echo "note: scratch kept at $SCRATCH (N/ daemon.log rns/rnsd.log, g5/, http/)"
  else
    rm -rf "$SCRATCH"
  fi
  if [ "${SMOKE_KEEP:-0}" != 1 ] && [ "${G5_KEEP_CACHE:-0}" != 1 ]; then
    rns_venv_remove "$CACHE/venv-dir"
    rm -rf "$CACHE/madmail"
    rmdir "$CACHE" 2>/dev/null || true
  fi
}
trap cleanup EXIT
LOG=$SCRATCH/cargo.log

# ---- 1. C4's first cut: the device core's tests, the fixtures, the unit
phase "1. the device core's cargo tests and fixtures (CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-default})"
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
ctest "cargo test --test devices_core (the whole device-core suite)" --test devices_core
ctest "uevent fixtures replayed through the fixture HotplugSource kick the tick; a change still takes two ticks" --test devices_core uevents_kick_the_tick_and_a_change_still_takes_two_ticks
ctest "PortTable collisions among rnsd, 111d's meshmud and an identify probe answer port_busy" --test devices_core port_table_collisions_answer_port_busy
ctest "a foreign loopback holder is caught by the pre-bind check" --test devices_core a_foreign_loopback_holder_is_caught_by_the_pre_bind_check
ctest "a crash mid-apply leaves pending and a restart never applies twice (in-process)" --test devices_core a_crash_mid_apply_leaves_pending_and_a_restart_never_applies_twice
ctest "cargo test --lib devices (uevent parser, the uevent socket, rnodeconf parser, KISS deframer, vocabulary)" --lib devices

FIX=$OS/1-bootstrap/crates/node111/tests/fixtures/uevent
want_uevent() { # FILE ACTION SUBSYSTEM — the kernel's shape: ACTION@DEVPATH, then KEY=VALUE, NUL-separated
  f="$FIX/$1"
  if [ ! -f "$f" ]; then fail "uevent fixture $1 is missing"; return; fi
  fields=$(tr '\0' '\n' <"$f")
  head=$(printf '%s\n' "$fields" | head -n 1)
  case "$head" in
    "$2@"/*) ;;
    *) fail "$1: header $head is not $2@<devpath>"; return ;;
  esac
  if printf '%s\n' "$fields" | grep -qx "ACTION=$2" && printf '%s\n' "$fields" | grep -qx "SUBSYSTEM=$3"; then
    ok "uevent $1: $2 on $3"
  else
    fail "$1: not ACTION=$2 SUBSYSTEM=$3"
  fi
}
want_uevent usb-add.uevent add usb
want_uevent usb-bind.uevent bind usb
want_uevent tty-add.uevent add tty
want_uevent tty-remove.uevent remove tty
want_uevent usb-remove.uevent remove usb
want_uevent net-add.uevent add net
want_uevent usb-change.uevent change usb
for unit in "$OS"/1-bootstrap/systemd/*.service; do
  line=$(grep -E '^RestrictAddressFamilies=' "$unit" || true)
  if [ -z "$line" ]; then
    ok "$(basename "$unit"): no RestrictAddressFamilies (AF_NETLINK allowed)"
  elif printf '%s' "$line" | grep -q 'AF_NETLINK'; then
    ok "$(basename "$unit"): $line"
  else
    fail "$(basename "$unit") restricts address families without AF_NETLINK: $line"
  fi
done

# ---- 2. prerequisites of the real-daemon part
phase "2. prerequisites: namespaces, the gate venv, Madmail's pinned payload, the debug 111"
have_netns || skip_or_fail G5 "unprivileged network namespaces are not available (unshare -Urnm)"
for tool in python3 ip nsenter curl jq ss unshare mount sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || skip_or_fail G5 "$tool is not installed"
done
unshare -Urm sh -c 'mount -t tmpfs tmpfs /sys/class/tty' 2>/dev/null ||
  skip_or_fail G5 "a mount namespace cannot overmount /sys/class/tty (the private sysfs tree)"
for f in fake-rnodeconf.sh stub-meshmud.py meshcore-pty.py; do
  [ -r "$FIXD/$f" ] || { echo "FAIL: G5: fixture verify/devices/$f is missing"; exit 1; }
done
if rns_venv_ensure "$CACHE/venv-dir"; then
  ok "gate venv from the lock: rns $("$RNS_VENV/bin/pip" show rns 2>/dev/null | sed -n 's/^Version: //p') ($("$RNS_VENV/bin/python" --version 2>&1))"
else
  skip_or_fail G5 "$RNS_VENV_ERR"
fi
mkdir -p "$CACHE/madmail"
MM=$CACHE/madmail/madmail-linux-amd64
if [ "$(uname -m)" = x86_64 ]; then
  if [ "$(sha256sum "$MM" 2>/dev/null | cut -c1-64)" != "$MADMAIL_SHA" ]; then
    rm -f "$MM"
    curl -sSfL --max-time 600 -o "$MM.part" "$MADMAIL_URL" 2>"$CACHE/madmail/curl.err" && mv "$MM.part" "$MM"
  fi
  if [ "$(sha256sum "$MM" 2>/dev/null | cut -c1-64)" = "$MADMAIL_SHA" ]; then
    ok "Madmail 2.30.5's pinned payload is in the gate cache (sha256 matches the manifest pin)"
  else
    rm -f "$MM" "$MM.part"
    note "Madmail's pinned payload could not be fetched ($(head -c 160 "$CACHE/madmail/curl.err" 2>/dev/null)): the API-level pre-bind check will SKIP"
  fi
else
  note "not x86_64: Madmail's x86_64 pin does not apply; the API-level pre-bind check will SKIP"
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

phase "2b. the scratch root N ($SCRATCH)"
W=$SCRATCH/http
mkdir -p "$W" "$SCRATCH/g5"
root=$SCRATCH/N
g45_make_root "$root" "# G5 node N: LAN sharing on (the paired session that accepts the proposals)
DASHBOARD_LAN=true"
rel=$root/releases/$G45_VER
sed -e "s|@G5@|$SCRATCH/g5|g" -e "s|@GOLDENS@|$GOLDENS|g" "$FIXD/fake-rnodeconf.sh" >"$root/venvs/rns/bin/rnodeconf"
chmod 755 "$root/venvs/rns/bin/rnodeconf"
cat >"$rel/bin/meshmud" <<EOF
#!/bin/sh
# G5 stand-in for meshmud (6-transport/verify/devices/stub-meshmud.py)
exec python3 -I "$FIXD/stub-meshmud.py" --role meshmud --g5 "$SCRATCH/g5" "\$@"
EOF
chmod 755 "$rel/bin/meshmud"
mkdir -p "$rel/share/meshtastic-presence" "$root/venvs/presence/bin"
ln -s "$(command -v python3)" "$root/venvs/presence/bin/python"
cat >"$rel/share/meshtastic-presence/meshtastic_presence.py" <<EOF
# G5 stand-in for the meshtastic-presence reader: 6-transport/verify/devices/stub-meshmud.py, presence role
import runpy, sys
sys.argv = ["$FIXD/stub-meshmud.py", "--role", "presence", "--g5", "$SCRATCH/g5"] + sys.argv[1:]
runpy.run_path("$FIXD/stub-meshmud.py", run_name="__main__")
EOF
if [ -s "$MM" ]; then
  mkdir -p "$root/cache/packages/madmail"
  ln "$MM" "$root/cache/packages/madmail/madmail-linux-amd64" 2>/dev/null || cp "$MM" "$root/cache/packages/madmail/"
fi
nid=$(g45_identity "$root")
printf '%s\n' "$nid" >"$SCRATCH/N.nid"
if [ -n "$nid" ]; then
  ok "N: root built (identity ${nid:0:16}…; venvs/rns → the gate venv + the fake rnodeconf; bin/meshmud and the presence reader → stub-meshmud.py)"
else
  fail "N: 111 identity generate failed ($(head -c 200 "$root/identity.log"))"
fi

if env --default-signal true 2>/dev/null; then
  RESET_SIG=(env --default-signal)
else
  RESET_SIG=(python3 -c 'import os,signal,sys
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])')
fi
INNER_LOG=$SCRATCH/inner.log
G5_WORK=$SCRATCH BIN111=$BIN111 RNS_VENV=$RNS_VENV RNS_LOCK=$RNS_LOCK RNS_LOCKSHA=$RNS_LOCKSHA G45_T0=$T0 \
  unshare --user --map-root-user --net --pid --fork --kill-child --mount-proc \
  "${RESET_SIG[@]}" bash "$HERE/devices.sh" --inner 2>&1 | tee "$INNER_LOG" || true
inner_fails=$(grep -c '^FAIL:' "$INNER_LOG" || true)
inner_oks=$(grep -c '^ok:' "$INNER_LOG" || true)
if ! grep -q '^FAILS=' "$SCRATCH/inner.result" 2>/dev/null; then
  fail "the namespace run did not finish (killed or crashed — see above)"
fi
FAILS=$((FAILS + inner_fails))

# ---- 3. the bench steps (external gate G1; not G5's)
phase "3. bench steps (external gate G1, hardware; information only)"
for s in "serial open without an unwanted reset under rnodeconf, meshmud and the MeshCore probe (Heltec V3, T-Beam)" \
  "real hotplug on bare Linux: a plug and an unplug published within the kicked tick" \
  "a real RNode answering rnodeconf --info (the text parser is unverified on hardware)" \
  "a MeshCore KISS modem answering GetVersion and GetIdentity" \
  "ModemManager or brltty holding a CP210x tty reads port_busy naming it"; do
  note "bench (G1): $s"
done

phase "4. hygiene"
left=""
for p in /proc/[0-9]*; do
  p=${p#/proc/}
  [ "$p" = "$$" ] && continue
  c=$(tr '\0' ' ' <"/proc/$p/cmdline" 2>/dev/null)
  case "$c" in *"$SCRATCH"* | *meshcore-pty.py* | *stub-meshmud.py*) left="$left $p:${c:0:60}" ;; esac
done
t "hygiene: no process of this run is left (daemon, rnsd, stubs, holders)" '[ -z "$left" ]'
[ -z "$left" ] || note "left:$left"
echo
echo "G5: $((OKS + inner_oks)) ok, $FAILS FAIL in $(elapsed)s"
finish G5
