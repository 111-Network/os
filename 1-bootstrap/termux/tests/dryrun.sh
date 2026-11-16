#!/bin/sh
# T13 verification: dry-run harness for the Termux assets. No Android needed:
# builds a fake $PREFIX/$HOME tree and stubs every on-device binary as a
# PATH-injected script that logs its argv.
set -u

cd "$(dirname "$0")/.." || exit 1 # termux/
T=$(mktemp -d /tmp/111-termux-test.XXXXXX)
FAILS=0
PORT=$((21000 + ($$ % 20000)))
SRV_PID=""

# shellcheck disable=SC2317,SC2329  # invoked via trap
cleanup() {
	[ -n "$SRV_PID" ] && kill "$SRV_PID" 2>/dev/null
	# the fake runsvdir's background sleep(s): it must look alive while the cases run, and nothing outlives the harness
	if [ -f "$T/fake-runsvdir.pids" ]; then
		while read -r _p; do kill "$_p" 2>/dev/null; done < "$T/fake-runsvdir.pids"
	fi
	rm -rf "$T"
}
trap cleanup EXIT

say() { printf '%s\n' "t13-test: $*"; }
fail() { printf '%s\n' "t13-test: FAIL: $*" >&2; FAILS=$((FAILS + 1)); }
pass() { printf '%s\n' "t13-test: ok: $*"; }

# --- fake device tree --------------------------------------------------------
PREF="$T/prefix"            # fake $PREFIX
HM="$T/home"                # fake $HOME
BIN="$T/bin"                # fake on-device binaries, PATH-injected
mkdir -p "$PREF/var/lib/111/state" "$PREF/var/log" "$PREF/var/service/111node" "$PREF/tmp" "$HM" "$BIN"

CALLS="$T/calls.log"
touch "$CALLS"

fake() { # $1 = binary name, $2 = body
	printf '#!/bin/sh\necho "%s: $*" >> "%s"\n%s\n' "$1" "$CALLS" "$2" > "$BIN/$1"
	chmod 0755 "$BIN/$1"
}

fake termux-wake-lock 'exit 0'
fake runsvdir 'sleep 30 &'
fake nohup 'shift 0; exec "$@" 2>/dev/null || exit 0'
# pgrep: succeeds only if the marker exists (simulates runsvdir already running)
printf '#!/bin/sh\n[ -f "%s/runsvdir.up" ] && exit 0; exit 1\n' "$T" > "$BIN/pgrep"
chmod 0755 "$BIN/pgrep"
fake sv 'exit 0'

# runsvdir fake also drops the "up" marker
cat > "$BIN/runsvdir" <<EOF
#!/bin/sh
echo "runsvdir: \$*" >> "$CALLS"
touch "$T/runsvdir.up"
sleep 30 &
echo "\$!" >> "$T/fake-runsvdir.pids"
EOF
chmod 0755 "$BIN/runsvdir"

export PREFIX="$PREF" HOME="$HM"
PATH="$BIN:/usr/bin:/bin"
export PATH

# install assets into the fake tree (as the provisioner would)
cp profile.env "$PREF/var/lib/111/profile.env"
cp sv/111node/run "$PREF/var/service/111node/run"
chmod 0755 "$PREF/var/service/111node/run"

# --- case 1: boot script, first boot (runsvdir down) ------------------------
sh boot/00-111-node.sh
if grep -q "termux-wake-lock:" "$CALLS" && grep -q "runsvdir:" "$CALLS" && grep -q "sv: start" "$CALLS"; then
	pass "boot: wake-lock → runsvdir → sv start, in order"
else
	fail "boot first-run sequence"
	cat "$CALLS" >&2
fi

# --- case 2: boot again — idempotent (runsvdir up marker present) ------------
: > "$CALLS"
sh boot/00-111-node.sh
N_RUNSVDIR=$(grep -c "runsvdir:" "$CALLS")
if [ "$N_RUNSVDIR" -eq 0 ] && grep -q "sv: start" "$CALLS"; then
	pass "boot: idempotent (runsvdir not restarted, service ensured)"
else
	fail "boot idempotence"
	cat "$CALLS" >&2
fi

# --- case 3: runit run script execs the daemon with throttle state -----------
mkdir -p "$PREF/var/lib/111/current/bin"
printf '#!/bin/sh\necho "daemon-ran: $*" >> "%s"\n' "$CALLS" > "$PREF/var/lib/111/current/bin/111"
chmod 0755 "$PREF/var/lib/111/current/bin/111"
sh "$PREF/var/service/111node/run"
if grep -q "daemon-ran: daemon" "$CALLS" && [ -f "$PREF/var/lib/111/state/111node.laststart" ]; then
	pass "runit run: daemon exec'd, throttle marker written"
else
	fail "runit run"
	cat "$CALLS" >&2
fi

# (cases 4 and 5, the cron health check, went with the Termux cron seam: Layer 9 §3.3 rule 6)

# --- case 6: e2e stub — positive fetch + verify + handoff --------------------
mkdir -p "$T/srv"
printf '#!/bin/sh\necho "payload-executed: $@" >> "%s"\n' "$CALLS" > "$T/srv/payload"
E2E_SHA=$(sha256sum "$T/srv/payload" | cut -d' ' -f1)
python3 -m http.server --bind 127.0.0.1 "$PORT" -d "$T/srv" >/dev/null 2>&1 &
SRV_PID=$!
_i=0
while ! curl -fsS -o /dev/null "http://127.0.0.1:$PORT/" 2>/dev/null; do
	_i=$((_i + 1))
	[ "$_i" -gt 50 ] && {
		say "fixture server never came up"
		exit 1
	}
	sleep 0.1
done

SIZE=$(wc -c < e2e-stub.sh)
if [ "$SIZE" -gt 4096 ]; then
	fail "e2e-stub.sh exceeds 4KB ($SIZE bytes)"
else
	pass "e2e-stub.sh within 4KB ($SIZE bytes)"
fi

sed -e "s/@@E2_PORT@@/$PORT/" \
	-e "s/@@PAYLOAD_SHA256@@/$E2E_SHA/" \
	-e "s|@@DEST@@|$PREF/tmp/111-payload|" \
	e2e-stub.sh > "$T/e2e.sh"
OUT=$(sh "$T/e2e.sh" 2>&1)
if echo "$OUT" | grep -q "payload verified" && grep -q "payload-executed" "$CALLS"; then
	pass "e2e stub: fetch → verify → handoff"
else
	fail "e2e positive"
	echo "$OUT" >&2
fi

# --- case 7: e2e stub — wrong sha refused, no handoff ------------------------
: > "$CALLS"
sed -e "s/@@E2_PORT@@/$PORT/" \
	-e "s/@@PAYLOAD_SHA256@@/0000000000000000000000000000000000000000000000000000000000000000/" \
	-e "s|@@DEST@@|$PREF/tmp/111-payload2|" \
	e2e-stub.sh > "$T/e2e-bad.sh"
OUT=$(sh "$T/e2e-bad.sh" 2>&1) && RC=0 || RC=$?
if [ "$RC" -ne 0 ] && echo "$OUT" | grep -q "sha256 mismatch" && ! grep -q "payload-executed" "$CALLS"; then
	pass "e2e stub: sha mismatch refuses, no handoff"
else
	fail "e2e negative (rc=$RC)"
	echo "$OUT" >&2
fi

# --- case 8: runit finish hook appends the Layer 9 witness line (G19 (10), dry-run half) ---
HEAL="$PREF/var/lib/111/state/heal"
LINE_RE='^v1 boot=([0-9a-f]{32}|-) result=(exit|signal) code=([0-9]{1,3}|-) signal=([0-9]{1,3}|-)$'
# state/heal/ missing: only the log line, no inbox, exit 0
rm -rf "$HEAL"
sh sv/111node/finish 1 0 && RC=0 || RC=$?
if [ "$RC" -eq 0 ] && [ ! -e "$HEAL/inbox" ]; then
	pass "finish: state/heal/ missing -> exit 0, nothing written"
else
	fail "finish with state/heal/ missing (rc=$RC)"
fi
mkdir -p "$HEAL"
sh sv/111node/finish 75 0 && RC=0 || RC=$?
sh sv/111node/finish -1 9 && RC2=0 || RC2=$?
if [ "$RC" -eq 0 ] && [ "$RC2" -eq 0 ] && [ "$(wc -l < "$HEAL/inbox")" -eq 2 ] \
	&& sed -n 1p "$HEAL/inbox" | grep -Eq "$LINE_RE" && sed -n 1p "$HEAL/inbox" | grep -Eq 'result=exit code=75 signal=-$' \
	&& sed -n 2p "$HEAL/inbox" | grep -Eq "$LINE_RE" && sed -n 2p "$HEAL/inbox" | grep -Eq 'result=signal code=- signal=9$'; then
	pass "finish: a code exit and a signal exit each append one well-formed v1 line"
else
	fail "finish witness lines"
	cat "$HEAL/inbox" >&2
fi
i=0
while [ "$i" -lt 300 ]; do sh sv/111node/finish 1 0; i=$((i + 1)); done
BAD=$(grep -Evc "$LINE_RE" "$HEAL/inbox")
MAXB=$(awk '{ if (length($0) > m) m = length($0) } END { print m + 0 }' "$HEAL/inbox")
if [ "$(wc -l < "$HEAL/inbox")" -eq 256 ] && [ "$BAD" -eq 0 ] && [ "$MAXB" -le 96 ] \
	&& [ "$(stat -c %a "$HEAL/inbox")" = 600 ] && [ -z "$(find "$HEAL" -name '.inbox.tmp-*')" ]; then
	pass "finish: 300 exits leave 256 lines, all well-formed, 0600, no temp left"
else
	fail "finish retention (lines=$(wc -l < "$HEAL/inbox") bad=$BAD max=$MAXB)"
fi

# a failing write must leave the old inbox byte for byte; a stale temp from a killed run is swept
BEFORE=$(cat "$HEAL/inbox")
echo stale > "$HEAL/.inbox.tmp-1"
FAKEBIN=$(mktemp -d)
printf '#!/bin/sh\nexit 1\n' > "$FAKEBIN/tail"
chmod +x "$FAKEBIN/tail"
PATH="$FAKEBIN:$PATH" sh sv/111node/finish 1 0 && RC=0 || RC=$?
rm -rf "$FAKEBIN"
if [ "$RC" -eq 0 ] && [ "$(cat "$HEAL/inbox")" = "$BEFORE" ] && [ -z "$(find "$HEAL" -name '.inbox.tmp-*')" ]; then
	pass "finish: a failed write keeps the old inbox intact; stale temps are swept"
else
	fail "finish failed-write handling (rc=$RC)"
fi

# --- case 9: the APK step's record, state/apk.json (Layer 9 §12.6; G19 (11), dry-run half) ---
# `111 install --phone` installs the three pinned APKs (step 3) and keeps their pins; on step 4's
# run-as channel, once Termux is bootstrapped (this fake tree), it writes them to state/apk.json.
# REC and the three commands are the provisioner's own, byte for byte and in its order, for the
# pins of three fixed APK fixtures: tests/phone_flow.rs fails when `111 install --phone` sends
# anything else, and its transcript test puts them after the APK step. Replayed here as run-as
# runs them: each one `sh -c` in the Termux root, the write with the record on stdin.
REC='{"schema":1,"apk":{"com.termux":"5d994501492ad52262147b765f6b51344bc4221a5bfe5015a5054046ffcb9857","com.termux.api":"1bbdd4e8285ec36bff0487be380941f7519871c4bad07083cb9c9fa9bd0d92cf","com.termux.boot":"b289a36d7a2cfb5f957b5bd795b61d3a6f7d3ddb8699be01f81547c8a0cc9343"}}'
# shellcheck disable=SC2016 # the provisioner's text: $PREFIX expands in run-as's shell
REC_WRITE='mkdir -p $(dirname $PREFIX/var/lib/111/state/.apk.json.tmp) && cat > $PREFIX/var/lib/111/state/.apk.json.tmp  && echo WRITE_OK'
# shellcheck disable=SC2016 # as above
REC_HASH='sha256sum $PREFIX/var/lib/111/state/.apk.json.tmp | cut -d'\'' '\'' -f1 | grep -qx 2af6c5161f4b74954bf82cc0b92e1ee4be4fe4ab9f7a2b2e8487dfddf7332756 && echo HASH_OK'
# shellcheck disable=SC2016 # as above
REC_LAND='chmod 0600 $PREFIX/var/lib/111/state/.apk.json.tmp && mv -f $PREFIX/var/lib/111/state/.apk.json.tmp $PREFIX/var/lib/111/state/apk.json && echo APK_RECORD_OK'
# the three calls in order, each answering the marker the provisioner checks for
push_record() {
	[ "$(printf '%s\n' "$REC" | sh -c "$REC_WRITE")" = WRITE_OK ] \
		&& [ "$(sh -c "$REC_HASH")" = HASH_OK ] \
		&& [ "$(sh -c "$REC_LAND")" = APK_RECORD_OK ]
}
# the file holds the record and a newline, 0600, and no temp file is left beside it
record_ok() { # $1 = the Termux root's state/
	[ -f "$1/apk.json" ] \
		&& [ "$(sha256sum < "$1/apk.json" | cut -d' ' -f1)" = "$(printf '%s\n' "$REC" | sha256sum | cut -d' ' -f1)" ] \
		&& [ "$(stat -c %a "$1/apk.json")" = 600 ] && [ ! -e "$1/.apk.json.tmp" ]
}
# a phone whose node never started: no state/ yet, the record creates it
FRESH="$T/fresh-prefix"
mkdir -p "$FRESH/var/lib/111"
if (PREFIX="$FRESH" && export PREFIX && push_record) && record_ok "$FRESH/var/lib/111/state"; then
	pass "apk record: the provisioner's write lands state/apk.json, 0600, state/ created"
else
	fail "apk record on a phone with no state/"
fi
# a re-run: an old world-readable record and the temp file of a killed run are replaced whole
RS="$PREF/var/lib/111/state"
printf '%s\n' '{"schema":1,"apk":{}}' > "$RS/apk.json"
printf 'half a rec' > "$RS/.apk.json.tmp"
chmod 0644 "$RS/apk.json" "$RS/.apk.json.tmp"
if push_record && record_ok "$RS"; then
	pass "apk record: a re-run replaces the old record whole, 0600, through the temp file"
else
	fail "apk record re-run"
fi
# the Termux side leaves it as pushed: Termux:Boot's script, a daemon start, a daemon exit
echo 0 > "$RS/111node.laststart" # past the run script's 2 s throttle
sh boot/00-111-node.sh && sh "$PREF/var/service/111node/run" && sh sv/111node/finish 0 0 && RC=0 || RC=$?
if [ "$RC" -eq 0 ] && record_ok "$RS"; then
	pass "apk record: boot, run and finish leave state/apk.json byte for byte, 0600"
else
	fail "apk record across boot, run and finish (rc=$RC)"
fi

echo
if [ "$FAILS" -eq 0 ]; then
	say "ALL CASES GREEN"
	exit 0
else
	say "$FAILS case(s) failed"
	exit 1
fi
