#!/bin/sh
# T1 verification: shim behavior under dash, bash, and busybox sh.
# Hermetic: serves fixture binaries from 127.0.0.1 with python3 http.server,
# resolves @@TOKENS@@ into a scratch copy of the shim, never touches the host.
set -u

cd "$(dirname "$0")/.." || exit 1 # shim/
SHIM_SRC="$PWD/install.sh"
T=$(mktemp -d /tmp/111-shim-test.XXXXXX)
PORT=$((20000 + ($$ % 20000)))
FAILS=0
SRV_PID=""

# shellcheck disable=SC2317  # invoked via trap, not direct flow
cleanup() {
	[ -n "$SRV_PID" ] && kill "$SRV_PID" 2>/dev/null
	rm -rf "$T"
}
trap cleanup EXIT

say() { printf '%s\n' "t1-test: $*"; }
fail() { printf '%s\n' "t1-test: FAIL: $*" >&2; FAILS=$((FAILS + 1)); }
pass() { printf '%s\n' "t1-test: ok: $*"; }

# --- fixtures: two fake binaries with distinct content ---------------------
FIX="$T/fixtures"
mkdir -p "$FIX"
# $* and $(id -u) expand inside the fake binaries, not here.
# shellcheck disable=SC2016
printf '#!/bin/sh\necho "fake 111 installer x86_64: $*"\necho "fake uid=$(id -u)"\n' > "$FIX/111-x86_64-unknown-linux-musl"
# shellcheck disable=SC2016
printf '#!/bin/sh\necho "fake 111 installer aarch64: $*"\necho "fake uid=$(id -u)"\n' > "$FIX/111-aarch64-unknown-linux-musl"
SHA_X=$(sha256sum "$FIX/111-x86_64-unknown-linux-musl" | cut -d' ' -f1)
SHA_A=$(sha256sum "$FIX/111-aarch64-unknown-linux-musl" | cut -d' ' -f1)

# --- resolve tokens into a scratch shim ------------------------------------
sed -e "s/@@SHIM_VERSION@@/2026.10.12/" \
	-e "s/@@SHA256_X86_64@@/$SHA_X/" \
	-e "s/@@SHA256_AARCH64@@/$SHA_A/" \
	-e "s/@@MIN_INSTALLER_VERSION@@/2026.10.01/" \
	"$SHIM_SRC" > "$T/install.sh"

# --- fixture server ---------------------------------------------------------
python3 -m http.server --bind 127.0.0.1 "$PORT" -d "$FIX" >/dev/null 2>&1 &
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

# --- run one case under one shell -------------------------------------------
# Env knobs are exported BEFORE the call (valid POSIX names propagate into any
# shell). Usage: run_shim SHELL [shim args...]; knobs via `set_hooks`.
BASE_URL="http://127.0.0.1:$PORT"
export SHIM111_BASE_URL="$BASE_URL"

set_hooks() { # $1 dry_run, $2 fake_uname, $3 fake_bit, $4 fake_priv, $5 dest ("" = unset)
	if [ -n "$1" ]; then export SHIM111_DRY_RUN="$1"; else unset SHIM111_DRY_RUN 2>/dev/null || true; fi
	if [ -n "$2" ]; then export SHIM111_FAKE_UNAME="$2"; else unset SHIM111_FAKE_UNAME 2>/dev/null || true; fi
	if [ -n "$3" ]; then export SHIM111_FAKE_BIT="$3"; else unset SHIM111_FAKE_BIT 2>/dev/null || true; fi
	if [ -n "$4" ]; then export SHIM111_FAKE_PRIV="$4"; else unset SHIM111_FAKE_PRIV 2>/dev/null || true; fi
	if [ -n "${5:-}" ]; then export SHIM111_DEST="$5"; else unset SHIM111_DEST 2>/dev/null || true; fi
}

# --phone fixtures: a read-only bin dir (privileged install step needed), a
# writable one (no privilege at all), and sudo/doas stubs that record any call.
RO="$T/ro-bin"
RW="$T/rw-bin"
STUBS="$T/stubs"
mkdir -p "$RO" "$RW" "$STUBS"
chmod 0555 "$RO"
for _p in sudo doas; do
	printf '#!/bin/sh\necho "%s $*" >> "%s/priv-calls"\nexit 1\n' "$_p" "$T" > "$STUBS/$_p"
	chmod 0755 "$STUBS/$_p"
done
ME=$(id -u)
ME_NAME=$(id -un)

run_shim() { # $1 = shell label; remaining args = shim args
	_sh=$1
	shift
	case "$_sh" in
		dash) dash "$T/install.sh" "$@" ;;
		bash) bash "$T/install.sh" "$@" ;;
		busybox) busybox sh "$T/install.sh" "$@" ;;
	esac
}

run_script() { # $1 = shell label, $2 = script path; remaining = shim args
	_sh=$1
	_script=$2
	shift 2
	case "$_sh" in
		dash) dash "$_script" "$@" ;;
		bash) bash "$_script" "$@" ;;
		busybox) busybox sh "$_script" "$@" ;;
	esac
}

for SH in dash bash busybox; do
	# 1. happy path (dry-run): downloads, verifies, prints privileged commands
	set_hooks 1 "" "" ""
	OUT=$(run_shim "$SH" 2>&1)
	RC=$?
	if [ $RC -eq 0 ] &&
		echo "$OUT" | grep -q "sha256 verified: $SHA_X" &&
		echo "$OUT" | grep -q "would run: install -m 0755" &&
		echo "$OUT" | grep -q "111-bootstrap install --min-installer-version=2026.10.01"; then
		pass "$SH: happy path verifies + dry-run prints commands"
	else
		fail "$SH: happy path (rc=$RC)"
		echo "$OUT" >&2
	fi

	# --phone, destination needs privilege: sudo/doas for the install step
	# ONLY; the installer is exec'd as the invoking user (it refuses root)
	set_hooks 1 "" "" "" "$RO/111-bootstrap"
	OUT=$(run_shim "$SH" --phone 2>&1)
	RC=$?
	if [ $RC -eq 0 ] &&
		echo "$OUT" | grep -Eq "privilege mechanism: (sudo|doas) \(binary install step only\)" &&
		echo "$OUT" | grep -q "would run: install -m 0755 .* $RO/111-bootstrap" &&
		echo "$OUT" | grep -q "would run as $ME_NAME, no sudo: $RO/111-bootstrap install --min-installer-version=2026.10.01 '--phone'" &&
		! echo "$OUT" | grep -q "would run: sudo\|would run: doas"; then
		pass "$SH: --phone escalates for the binary install only"
	else
		fail "$SH: --phone dry-run (rc=$RC)"
		echo "$OUT" >&2
	fi

	# --phone, user-writable destination: real run, no sudo/doas at all, and
	# the installer runs as us with the shim args (incl. --yes) passed through
	set_hooks "" "" "" "" "$RW/111-bootstrap"
	rm -f "$T/priv-calls" "$RW/111-bootstrap"
	OUT=$(PATH="$STUBS:$PATH" run_shim "$SH" --phone --yes 2>&1)
	RC=$?
	if [ $RC -eq 0 ] &&
		echo "$OUT" | grep -q "fake 111 installer x86_64: install --min-installer-version=2026.10.01 --phone --yes" &&
		echo "$OUT" | grep -q "fake uid=$ME\$" &&
		[ -x "$RW/111-bootstrap" ] && [ ! -e "$T/priv-calls" ]; then
		pass "$SH: --phone with a writable destination runs as the user, no sudo"
	else
		fail "$SH: --phone user run (rc=$RC, priv calls: $(cat "$T/priv-calls" 2>/dev/null))"
		echo "$OUT" >&2
	fi

	# --phone, no privilege mechanism and a read-only destination: the two
	# exact steps (root install, then the user run), exit 1
	set_hooks "" "" "" none "$RO/111-bootstrap"
	OUT=$(run_shim "$SH" --phone 2>&1)
	RC=$?
	if [ $RC -eq 1 ] &&
		echo "$OUT" | grep -q "1. as root: *su -c 'install -m 0755 .* $RO/111-bootstrap'" &&
		echo "$OUT" | grep -q "2. then as YOU, no sudo: $RO/111-bootstrap install --min-installer-version=2026.10.01 '--phone'"; then
		pass "$SH: --phone without privilege prints root-install + user-run steps"
	else
		fail "$SH: --phone no-privilege path (rc=$RC)"
		echo "$OUT" >&2
	fi
	set_hooks 1 "" "" ""

	# 2. tampered pin: wrong sha refused before any privileged action
	sed -e "s/@@SHIM_VERSION@@/2026.10.12/" \
		-e "s/@@SHA256_X86_64@@/0000000000000000000000000000000000000000000000000000000000000000/" \
		-e "s/@@SHA256_AARCH64@@/0000000000000000000000000000000000000000000000000000000000000000/" \
		-e "s/@@MIN_INSTALLER_VERSION@@/2026.10.01/" \
		"$SHIM_SRC" > "$T/install-badpin.sh"
	OUT=$(run_script "$SH" "$T/install-badpin.sh" 2>&1)
	if echo "$OUT" | grep -q "sha256 mismatch" && ! echo "$OUT" | grep -q "would run:"; then
		pass "$SH: tampered hash refused before exec"
	else
		fail "$SH: tamper refusal"
		echo "$OUT" >&2
	fi

	# 3. armv7l refusal
	set_hooks 1 armv7l "" ""
	OUT=$(run_shim "$SH" 2>&1)
	RC=$?
	if [ $RC -ne 0 ] && echo "$OUT" | grep -q "not supported"; then
		pass "$SH: armv7l refused"
	else
		fail "$SH: armv7l refusal"
		echo "$OUT" >&2
	fi

	# 4. 32-bit userspace on aarch64 kernel: warn + kernel-arch binary
	set_hooks 1 aarch64 32 ""
	OUT=$(run_shim "$SH" 2>&1)
	if echo "$OUT" | grep -q "32-bit userspace" && echo "$OUT" | grep -q "111-aarch64-unknown-linux-musl"; then
		pass "$SH: 32-bit userspace warns + uses kernel arch"
	else
		fail "$SH: 32-bit userspace handling"
		echo "$OUT" >&2
	fi

	# 5. no privilege path: prints exact su -c line, exit 1
	set_hooks "" "" "" none
	OUT=$(run_shim "$SH" 2>&1)
	RC=$?
	if [ $RC -eq 1 ] && echo "$OUT" | grep -q "su -c 'install -m 0755"; then
		pass "$SH: no-privilege prints su -c line"
	else
		fail "$SH: no-privilege path (rc=$RC)"
		echo "$OUT" >&2
	fi

	# 6. truncated shim executes nothing (syntax error, never verifies)
	head -c $(($(wc -c < "$T/install.sh") * 3 / 5)) "$T/install.sh" > "$T/trunc.sh"
	set_hooks 1 "" "" ""
	OUT=$(run_script "$SH" "$T/trunc.sh" 2>&1)
	if ! echo "$OUT" | grep -q "sha256 verified"; then
		pass "$SH: truncated shim never reaches verification"
	else
		fail "$SH: truncated shim executed!"
		echo "$OUT" >&2
	fi
done

echo
if [ "$FAILS" -eq 0 ]; then
	say "ALL CASES GREEN (dash, bash, busybox sh)"
	exit 0
else
	say "$FAILS case(s) failed"
	exit 1
fi
