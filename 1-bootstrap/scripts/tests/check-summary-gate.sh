#!/bin/sh
# check.sh summary gate (Wave 2 review B1 m2, m3): scripts/check.sh is the lock gate and the
# release-checklist gate, so its last line must say what ran.
#   - "ALL GREEN" only when every stage ran, G2 (the SQLCipher static link, both musl targets)
#     included; a skipped G2 ends "GREEN (G2 SKIPPED — not a lock run)", and REQUIRE_G2=1 fails
#     before any stage when the cross toolchains are absent.
#   - G2 asserts what it prints: every linked test binary is a static ELF for its target (x86_64
#     too, no INTERP/NEEDED), and the mesh111 lib-test binary carries SQLCipher's codec and
#     Cargo.lock's OpenSSL on both targets.
#   - ../50-messaging/scripts/check.sh runs its musl checks with both cross toolchains or neither,
#     with the same pinned CC/AR as scripts/musl-env.sh.
#
# Hermetic: check.sh + scripts/musl-env.sh on a scratch tree whose other stages are stubs, a stub
# `cargo` that links fake test binaries (scripts/tests/fakes.sh) and logs its calls, a fake
# `readelf`; the PATH holds only links to the host tools needed.
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPTS="$(dirname "$HERE")"
OS="$(dirname "$(dirname "$SCRIPTS")")"
# shellcheck source=scripts/tests/fakes.sh
. "$HERE/fakes.sh"
T=$(mktemp -d /tmp/111-check-gate.XXXXXX)
trap 'rm -rf "$T"' EXIT
FAILS=0
pass() { printf '%s\n' "check-gate: ok: $*"; }
fail() { printf '%s\n' "check-gate: FAIL: $*" >&2; FAILS=$((FAILS + 1)); }
SH=$(command -v sh)
X=x86_64-unknown-linux-musl
A=aarch64-unknown-linux-musl

# ---- the scratch tree: the real check.sh and musl-env.sh, every other stage a stub that passes.
R="$T/os"
B="$R/10-bootstrap"
mkdir -p "$B/scripts/notices/tests" "$B/systemd" "$R/40-interface/ui/node_modules" \
  "$R/50-messaging/scripts" "$R/50-messaging/sidecars/meshtastic-presence" "$T/home" "$T/fake"
cp "$SCRIPTS/check.sh" "$SCRIPTS/musl-env.sh" "$B/scripts/"
printf 'echo musl-env-test stub\n' >"$B/scripts/musl-env-test.sh"
printf 'print("gen-notices stub")\n' >"$B/scripts/gen-notices.py"
printf 'import unittest\n\n\nclass Stub(unittest.TestCase):\n    def test_stub(self):\n        pass\n' \
  >"$B/scripts/notices/tests/test_stub.py"
printf 'echo build-scripts-gate stub\n' >"$B/scripts/notices/tests/build-scripts-gate.sh"
mkdir -p "$B/scripts/tests"
cp "$B/scripts/notices/tests/test_stub.py" "$B/scripts/tests/test_stub.py"
for g in check-summary-gate cross-build-ui-gate cross-build-asserts-gate; do
  printf 'echo %s stub\n' "$g" >"$B/scripts/tests/$g.sh"
done
: >"$B/systemd/111node.service"
: >"$B/systemd/111node-update.service"
: >"$B/systemd/111node-update.path"
: >"$R/40-interface/ui/node_modules/.modules.yaml"
printf 'echo messaging-check stub\n' >"$R/50-messaging/scripts/check.sh"
fake_lock "$R/50-messaging/Cargo.lock"
printf 'echo presence stub\n' >"$R/50-messaging/sidecars/meshtastic-presence/run-tests.sh"
printf 'print("gen_lock stub")\n' >"$R/50-messaging/sidecars/meshtastic-presence/gen_lock.py"

sysbin "$T/sysbin" sh dirname mktemp rm cp grep sed head find xargs ls cat env mkdir python3 || exit 1
fake_readelf "$T/stubs"
for t in shellcheck git systemd-analyze pnpm; do stub "$t" "$T/opt"; done
# `cargo test --workspace --no-run --target T --message-format=json`: one compiler-artifact line per
# fake binary in $FAKE/T (mesh111-lib is the mesh111 lib-test binary). Every call is logged.
# shellcheck disable=SC2016
stub cargo "$T/stubs" 'echo "cargo $*" >>"$FAKE/cargo.log"
case " $* " in
  *" --no-run "*)
    t=
    prev=
    for a in "$@"; do
      [ "$prev" = --target ] && t=$a
      prev=$a
    done
    for f in "$FAKE/$t"/*; do
      [ -f "$f" ] || continue
      n=${f##*/}
      kind=test
      [ "$n" = mesh111-lib ] && kind=lib && n=mesh111
      printf "{\"reason\":\"compiler-artifact\",\"target\":{\"kind\":[\"%s\"],\"name\":\"%s\"},\"executable\":\"%s\"}\n" "$kind" "$n" "$f"
    done ;;
esac
exit 0'

# bins X86_LIB X86_TEST ARM_LIB ARM_TEST — the fake test binaries G2 links ("" = not linked).
bins() {
  rm -rf "$T/fake/$X" "$T/fake/$A"
  [ -z "$1" ] || fake_bin "$T/fake/$X/mesh111-lib" X86-64 "$1"
  [ -z "$2" ] || fake_bin "$T/fake/$X/flood-test" X86-64 "$2"
  [ -z "$3" ] || fake_bin "$T/fake/$A/mesh111-lib" AArch64 "$3"
  [ -z "$4" ] || fake_bin "$T/fake/$A/flood-test" AArch64 "$4"
}

# run HOME PATH-EXTRA [VAR=VALUE...] — check.sh in the scratch tree; output in $OUT, status in $RC.
run() {
  _home=$1 _extra=$2
  shift 2
  : >"$T/fake/cargo.log"
  OUT=$(env -i HOME="$_home" PATH="$T/stubs${_extra:+:$_extra}:$T/sysbin" FAKE="$T/fake" "$@" \
    "$SH" "$B/scripts/check.sh" 2>&1)
  RC=$?
  LAST=$(printf '%s\n' "$OUT" | tail -n 1)
}
show() {
  printf '%s\n' "$OUT" | tail -n 25 >&2
}

fake_toolchains "$T/home" x86_64 aarch64
bins "$CODEC" "plain test binary" "$CODEC" "plain test binary"

run "$T/home" "$T/opt"
if [ "$RC" = 0 ] && [ "$LAST" = "ALL GREEN" ] &&
  echo "$OUT" | grep -qF "ok: 2 x86_64 test binaries static X86-64 (no INTERP/NEEDED); 1 carry SQLCipher (cipher_plaintext_header_size) + OpenSSL 3.6.3" &&
  echo "$OUT" | grep -qF "ok: 2 aarch64 test binaries static AArch64 (no INTERP/NEEDED); 1 carry SQLCipher (cipher_plaintext_header_size) + OpenSSL 3.6.3" &&
  grep -qF "cargo test --workspace --quiet --target $X" "$T/fake/cargo.log"; then
  pass "every stage ran, G2 on both targets → ALL GREEN"
else
  fail "full run (rc=$RC, last line: $LAST)"
  show
fi

run "$T/home" ""
if [ "$RC" = 0 ] && [ "$LAST" = "GREEN (shellcheck, build-scripts gate, systemd-analyze, ui SKIPPED — not a lock run)" ]; then
  pass "G2 ran, other tools absent → GREEN (… SKIPPED — not a lock run)"
else
  fail "skipped tools (rc=$RC, last line: $LAST)"
  show
fi

mkdir -p "$T/home-bare"
run "$T/home-bare" "$T/opt"
if [ "$RC" = 0 ] && [ "$LAST" = "GREEN (G2 SKIPPED — not a lock run); also skipped: musl checks" ] &&
  ! echo "$OUT" | grep -q "ALL GREEN" && ! grep -q -- "--target" "$T/fake/cargo.log"; then
  pass "no cross toolchains → G2 skipped, never ALL GREEN"
else
  fail "G2 skipped (rc=$RC, last line: $LAST)"
  show
fi

mkdir -p "$T/home-x86"
fake_toolchains "$T/home-x86" x86_64
run "$T/home-x86" "$T/opt"
if [ "$RC" = 0 ] && echo "$LAST" | grep -q "^GREEN (G2 SKIPPED — not a lock run)" &&
  ! grep -q -- "--target" "$T/fake/cargo.log"; then
  pass "one cross toolchain only → G2 skipped, no musl build attempted"
else
  fail "half-installed toolchains (rc=$RC, last line: $LAST)"
  show
fi

run "$T/home-bare" "$T/opt" REQUIRE_G2=1
if [ "$RC" != 0 ] && echo "$OUT" | grep -q "FAIL: REQUIRE_G2=1" && [ ! -s "$T/fake/cargo.log" ]; then
  pass "REQUIRE_G2=1 without cross toolchains → fails before any stage"
else
  fail "REQUIRE_G2=1 (rc=$RC, last line: $LAST)"
  show
fi

# expect_red NAME NEEDLE — check.sh must fail with NEEDLE and print no GREEN line.
expect_red() {
  run "$T/home" "$T/opt"
  if [ "$RC" != 0 ] && echo "$OUT" | grep -qF -- "$2" && ! echo "$OUT" | grep -q "GREEN"; then
    pass "$1"
  else
    fail "$1 (rc=$RC, wanted: $2)"
    show
  fi
}

bins "$CODEC" "plain test binary" "$OWN_STRINGS" "plain test binary"
expect_red "aarch64 mesh111 lib tests without SQLCipher's codec (mesh111's own strings only) → red" \
  "mesh111-lib carries no SQLCipher codec (cipher_plaintext_header_size)"

bins "cipher_plaintext_header_size" "plain test binary" "$CODEC" "plain test binary"
expect_red "x86_64 mesh111 lib tests without OpenSSL 3.6.3 → red" \
  "mesh111-lib carries no statically linked OpenSSL 3.6.3"

bins "$CODEC" "plain test binary FAKE-NEEDED" "$CODEC" "plain test binary"
expect_red "an x86_64 test binary with a DT_NEEDED entry → red" "flood-test is dynamically linked"

bins "$CODEC" "plain test binary" "$CODEC" "plain test binary FAKE-INTERP"
expect_red "an aarch64 test binary with a PT_INTERP header → red" "flood-test is dynamically linked"

bins "" "plain test binary" "$CODEC" "plain test binary"
expect_red "no mesh111 lib-test binary among the x86_64 ones → red" \
  "the mesh111 lib-test binary is not among the $X test binaries"

# ---- ../50-messaging/scripts/check.sh: musl checks with both toolchains or neither.
M="$T/msg/50-messaging"
mkdir -p "$M/scripts"
cp "$OS/50-messaging/scripts/check.sh" "$M/scripts/"
# shellcheck disable=SC2016
stub cargo "$T/msg/stubs" 'echo "cargo $*" >>"$FAKE/msg-cargo.log"
case " $* " in
  *" --target aarch64-unknown-linux-musl "*) env | grep "^CC_aarch64_unknown_linux_musl=" >>"$FAKE/msg-cargo.log" ;;
esac
exit 0'
msg() { # HOME
  : >"$T/fake/msg-cargo.log"
  OUT=$(env -i HOME="$1" PATH="$T/msg/stubs:$T/sysbin" FAKE="$T/fake" "$SH" "$M/scripts/check.sh" 2>&1)
  RC=$?
  LAST=$(printf '%s\n' "$OUT" | tail -n 1)
}
msg "$T/home-x86"
if [ "$RC" = 0 ] && ! grep -q -- "--target" "$T/fake/msg-cargo.log" && [ "$LAST" = "check.sh: green (musl checks SKIPPED)" ]; then
  pass "messaging: one cross toolchain only → musl checks skipped, said so"
else
  fail "messaging half-installed toolchains (rc=$RC, last line: $LAST)"
  show
fi
msg "$T/home"
if [ "$RC" = 0 ] && [ "$LAST" = "check.sh: all green" ] &&
  grep -qx "cargo check --workspace --target $X" "$T/fake/msg-cargo.log" &&
  grep -qx "cargo check --workspace --target $A" "$T/fake/msg-cargo.log" &&
  grep -qx "CC_aarch64_unknown_linux_musl=aarch64-linux-musl-gcc" "$T/fake/msg-cargo.log"; then
  pass "messaging: both cross toolchains → both musl checks, CC pinned"
else
  fail "messaging both toolchains (rc=$RC, last line: $LAST)"
  show
fi

if [ "$FAILS" -eq 0 ]; then
  printf '%s\n' "check-gate: ALL CASES GREEN"
  exit 0
fi
printf '%s\n' "check-gate: $FAILS case(s) failed"
exit 1
