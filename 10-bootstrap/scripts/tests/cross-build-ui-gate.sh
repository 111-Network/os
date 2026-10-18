#!/bin/sh
# cross-build UI gate: a release build must fail without a fresh
# 40-interface/ui/dist unless --allow-no-ui is given. Hermetic: copies
# cross-build.sh into a scratch tree with a fake UI, a stub `cargo` that stops
# the build right after the gate, and (per case) a stub `pnpm`.
set -u

SRC="$(cd "$(dirname "$0")/.." && pwd)/cross-build.sh"
T=$(mktemp -d /tmp/111-xbuild-gate.XXXXXX)
trap 'rm -rf "$T"' EXIT
FAILS=0
pass() { printf '%s\n' "ui-gate: ok: $*"; }
fail() { printf '%s\n' "ui-gate: FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

mkdir -p "$T/10-bootstrap/scripts" "$T/stubs" "$T/pnpm" "$T/home"
cp "$SRC" "$T/10-bootstrap/scripts/cross-build.sh"
UI="$T/40-interface/ui"
printf '#!/bin/sh\necho CARGO-REACHED\nexit 3\n' > "$T/stubs/cargo"
# $2/$3 expand inside the stub, not here.
# shellcheck disable=SC2016
printf '#!/bin/sh\n[ "$3" = build ] && mkdir -p "$2/dist" && echo built > "$2/dist/index.html"\nexit 0\n' > "$T/pnpm/pnpm"
chmod 0755 "$T/stubs/cargo" "$T/pnpm/pnpm"

reset_ui() {
	rm -rf "$UI"
	mkdir -p "$UI/src" "$UI/node_modules"
	: > "$UI/node_modules/.modules.yaml"
	echo '{}' > "$UI/package.json"
	echo '<!doctype html>' > "$UI/index.html"
	echo 'export {}' > "$UI/src/main.ts"
}

# HOME points at an empty dir: no ~/.cargo/bin or ~/.local/toolchains leak in.
build() { # extra PATH dir ("" = none); then cross-build args
	_extra=$1
	shift
	HOME="$T/home" PATH="${_extra:+$_extra:}$T/stubs:/usr/bin:/bin" \
		sh "$T/10-bootstrap/scripts/cross-build.sh" "$@" 2026.10.14 "$T/out" 2>&1
}

if (PATH=/usr/bin:/bin && command -v pnpm >/dev/null 2>&1); then
	printf '%s\n' "ui-gate: SKIPPED (a system pnpm in /usr/bin or /bin makes the no-pnpm cases unreachable)"
	exit 0
fi

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

if [ "$FAILS" -eq 0 ]; then
	printf '%s\n' "ui-gate: ALL CASES GREEN"
	exit 0
fi
printf '%s\n' "ui-gate: $FAILS case(s) failed"
exit 1
