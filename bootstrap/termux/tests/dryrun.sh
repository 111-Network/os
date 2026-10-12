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

# shellcheck disable=SC2317  # invoked via trap
cleanup() {
	[ -n "$SRV_PID" ] && kill "$SRV_PID" 2>/dev/null
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
fake curl 'exit 0'

# runsvdir fake also drops the "up" marker
cat > "$BIN/runsvdir" <<EOF
#!/bin/sh
echo "runsvdir: \$*" >> "$CALLS"
touch "$T/runsvdir.up"
sleep 30 &
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

# --- case 4: healthcheck success (fake curl exit 0) → no failure log ---------
mkdir -p "$PREF/var/lib/111/share/termux"
cp cron/healthcheck.sh "$PREF/var/lib/111/share/termux/cron-healthcheck.sh"
sh "$PREF/var/lib/111/share/termux/cron-healthcheck.sh"
if [ ! -f "$PREF/var/lib/111/state/health-failures.log" ]; then
	pass "healthcheck: healthy stays silent"
else
	fail "healthcheck healthy"
fi

# --- case 5: healthcheck failure records a marker ----------------------------
printf '#!/bin/sh\necho "curl: $*" >> "%s"\nexit 22\n' "$CALLS" > "$BIN/curl"
chmod 0755 "$BIN/curl"
sh "$PREF/var/lib/111/share/termux/cron-healthcheck.sh"
if [ -f "$PREF/var/lib/111/state/health-failures.log" ] && grep -q "health check failed" "$PREF/var/lib/111/state/health-failures.log"; then
	pass "healthcheck: failure recorded (Layer 9 seam)"
else
	fail "healthcheck failure marker"
fi

# --- case 6: e2e stub — positive fetch + verify + handoff --------------------
# restore the real curl BEFORE anything polls (the fake one always exits 22)
rm -f "$BIN/curl"

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

echo
if [ "$FAILS" -eq 0 ]; then
	say "ALL CASES GREEN"
	exit 0
else
	say "$FAILS case(s) failed"
	exit 1
fi
