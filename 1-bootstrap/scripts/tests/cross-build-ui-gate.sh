#!/bin/sh
# cross-build UI gate: a release build must fail without a fresh
# 4-interface/ui/dist unless --allow-no-ui is given, and without both musl
# cross toolchains it must fail before anything is built. Hermetic: copies
# cross-build.sh and the scripts/musl-env.sh it sources into a scratch tree
# with a fake UI, stub cross toolchains, a stub `cargo` that stops the build
# right after the gate, and (per case) a stub `pnpm`. The build's PATH holds
# only links to the host tools it needs (scripts/tests/fakes.sh `sysbin`), so
# a system pnpm cannot leak in and no case is ever skipped.
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPTS="$(dirname "$HERE")"
# shellcheck source=scripts/tests/fakes.sh
. "$HERE/fakes.sh"
T=$(mktemp -d /tmp/111-xbuild-gate.XXXXXX)
trap 'rm -rf "$T"' EXIT
FAILS=0
pass() { printf '%s\n' "ui-gate: ok: $*"; }
fail() { printf '%s\n' "ui-gate: FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

mkdir -p "$T/1-bootstrap/scripts" "$T/home" "$T/home-bare"
cp "$SCRIPTS/cross-build.sh" "$SCRIPTS/musl-env.sh" "$T/1-bootstrap/scripts/"
UI="$T/4-interface/ui"
SH=$(command -v sh)
sysbin "$T/sysbin" dirname mkdir find head || exit 1
fake_toolchains "$T/home" x86_64 aarch64
stub cargo "$T/stubs" 'echo CARGO-REACHED
exit 3'
# $2/$3 expand inside the stub, not here.
# shellcheck disable=SC2016
stub pnpm "$T/pnpm" 'echo PNPM-REACHED
[ "$3" = build ] && mkdir -p "$2/dist" && echo built > "$2/dist/index.html"
exit 0'

reset_ui() {
	rm -rf "$UI"
	mkdir -p "$UI/src" "$UI/node_modules"
	: > "$UI/node_modules/.modules.yaml"
	echo '{}' > "$UI/package.json"
	echo '<!doctype html>' > "$UI/index.html"
	echo 'export {}' > "$UI/src/main.ts"
}

# HOME holds only the stub toolchains: no ~/.cargo/bin or real toolchains leak in.
build() { # extra PATH dir ("" = none); then cross-build args
	_extra=$1
	shift
	env -i HOME="${XB_HOME:-$T/home}" PATH="${_extra:+$_extra:}$T/stubs:$T/sysbin" \
		"$SH" "$T/1-bootstrap/scripts/cross-build.sh" "$@" 2026.10.14 "$T/out" 2>&1
}

reset_ui
OUT=$(build "")
RC=$?
if [ $RC -eq 1 ] && echo "$OUT" | grep -q "dist is empty" && ! echo "$OUT" | grep -q CARGO-REACHED; then
	pass "no pnpm + no dist → refused before cargo"
else
	fail "missing dist (rc=$RC)"
	echo "$OUT" >&2
fi

reset_ui
mkdir -p "$UI/dist"
echo old > "$UI/dist/index.html"
touch -d '2020-01-01 00:00' "$UI/dist/index.html"
OUT=$(build "")
RC=$?
if [ $RC -eq 1 ] && echo "$OUT" | grep -q "dist is stale" && ! echo "$OUT" | grep -q CARGO-REACHED; then
	pass "no pnpm + stale dist → refused before cargo"
else
	fail "stale dist (rc=$RC)"
	echo "$OUT" >&2
fi

touch -d '2020-01-01 00:00' "$UI/src/main.ts" "$UI/package.json" "$UI/index.html"
touch "$UI/dist/index.html"
OUT=$(build "")
if echo "$OUT" | grep -q CARGO-REACHED; then
	pass "no pnpm + fresh dist → build proceeds"
else
	fail "fresh dist refused"
	echo "$OUT" >&2
fi

reset_ui
OUT=$(build "" --allow-no-ui)
if echo "$OUT" | grep -q CARGO-REACHED && echo "$OUT" | grep -q "NOT a release build"; then
	pass "--allow-no-ui → build proceeds, labeled"
else
	fail "--allow-no-ui"
	echo "$OUT" >&2
fi

reset_ui
OUT=$(build "$T/pnpm")
if echo "$OUT" | grep -q CARGO-REACHED && [ -f "$UI/dist/index.html" ]; then
	pass "pnpm present → UI rebuilt, build proceeds"
else
	fail "pnpm rebuild"
	echo "$OUT" >&2
fi

# No cross toolchains: refused before the UI build and before cargo, whatever the UI state.
reset_ui
OUT=$(XB_HOME="$T/home-bare" build "$T/pnpm")
RC=$?
if [ $RC -eq 1 ] && echo "$OUT" | grep -q "musl cross toolchains absent" &&
	! echo "$OUT" | grep -q "CARGO-REACHED\|PNPM-REACHED"; then
	pass "no musl cross toolchains → refused before the UI build and cargo"
else
	fail "missing toolchains (rc=$RC)"
	echo "$OUT" >&2
fi

if [ "$FAILS" -eq 0 ]; then
	printf '%s\n' "ui-gate: ALL CASES GREEN"
	exit 0
fi
printf '%s\n' "ui-gate: $FAILS case(s) failed"
exit 1
