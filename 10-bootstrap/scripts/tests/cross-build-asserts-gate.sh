#!/bin/sh
# cross-build asserts gate (Wave 2 review B1 m1): the release asserts of scripts/cross-build.sh
# must refuse a binary that is not static, or that does not carry SQLCipher's codec and the vendored
# OpenSSL named by Cargo.lock's openssl-src. mesh111's own strings ("PRAGMA cipher_version",
# "SQLCipher is not linked …") are not enough: the pre-bounce assert accepted them, so a plain-SQLite
# release passed. The release env must reach cargo with CC/AR pinned and the escape hatches to a
# system SQLite / libcrypto unset, even when the caller's environment sets them.
#
# Hermetic: cross-build.sh + scripts/musl-env.sh on a scratch tree, stub cross toolchains, a stub
# `cargo` that "builds" a chosen fake binary per target and logs its environment, and a fake
# `readelf` (scripts/tests/fakes.sh). The PATH holds only links to the host tools needed.
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPTS="$(dirname "$HERE")"
# shellcheck source=scripts/tests/fakes.sh
. "$HERE/fakes.sh"
T=$(mktemp -d /tmp/111-xbuild-asserts.XXXXXX)
trap 'rm -rf "$T"' EXIT
FAILS=0
pass() { printf '%s\n' "asserts-gate: ok: $*"; }
fail() { printf '%s\n' "asserts-gate: FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

mkdir -p "$T/10-bootstrap/scripts" "$T/home" "$T/fake"
cp "$SCRIPTS/cross-build.sh" "$SCRIPTS/musl-env.sh" "$T/10-bootstrap/scripts/"
SH=$(command -v sh)
sysbin "$T/sysbin" dirname mkdir cp grep sed awk wc tr head env || exit 1
fake_toolchains "$T/home" x86_64 aarch64
fake_readelf "$T/stubs"
# `cargo build --release --target T`: the fake binary $FAKE/T becomes the build output.
# shellcheck disable=SC2016
stub cargo "$T/stubs" 't=
prev=
for a in "$@"; do
  [ "$prev" = --target ] && t=$a
  prev=$a
done
env >>"$FAKE/cargo-env.$t"
mkdir -p "$CARGO_TARGET_DIR/$t/release"
cp "$FAKE/$t" "$CARGO_TARGET_DIR/$t/release/111"'

X=x86_64-unknown-linux-musl
A=aarch64-unknown-linux-musl
# release X86_TEXT ARM_TEXT [mesh111|plain] — cross-build with these fake binaries; the caller's
# environment carries every escape hatch, which the release env must drop.
release() {
  rm -rf "$T/target" "$T/out" "$T/fake"/cargo-env.*
  fake_bin "$T/fake/$X" X86-64 "$1"
  fake_bin "$T/fake/$A" AArch64 "$2"
  if [ "${3:-mesh111}" = mesh111 ]; then
    fake_lock "$T/10-bootstrap/Cargo.lock" mesh111
  else
    fake_lock "$T/10-bootstrap/Cargo.lock"
  fi
  env -i HOME="$T/home" PATH="$T/stubs:$T/sysbin" CARGO_TARGET_DIR="$T/target" FAKE="$T/fake" \
    LIBSQLITE3_SYS_USE_PKG_CONFIG=1 LIBSQLITE3_FLAGS=-USQLITE_HAS_CODEC SQLCIPHER_LIB_DIR=/usr/lib \
    OPENSSL_NO_VENDOR=1 OPENSSL_DIR=/usr OPENSSL_LIB_DIR=/usr/lib OPENSSL_STATIC=0 \
    X86_64_UNKNOWN_LINUX_MUSL_OPENSSL_NO_VENDOR=1 AARCH64_UNKNOWN_LINUX_MUSL_OPENSSL_DIR=/usr \
    CC_aarch64_unknown_linux_musl=cc AR_x86_64_unknown_linux_musl=ar \
    "$SH" "$T/10-bootstrap/scripts/cross-build.sh" --allow-no-ui 2026.10.14 "$T/out" 2>&1
}

# expect_ok NAME OUTPUT RC NEEDLE...
expect_ok() {
  _name=$1 _out=$2 _rc=$3
  shift 3
  _ok=1
  [ "$_rc" -eq 0 ] && echo "$_out" | grep -q "cross-build OK" || _ok=0
  for _n in "$@"; do
    echo "$_out" | grep -qF -- "$_n" || _ok=0
  done
  if [ "$_ok" = 1 ]; then
    pass "$_name"
  else
    fail "$_name (rc=$_rc)"
    echo "$_out" >&2
  fi
}

# expect_refused NAME OUTPUT RC NEEDLE
expect_refused() {
  if [ "$3" -ne 0 ] && echo "$2" | grep -qF -- "$4" && ! echo "$2" | grep -q "cross-build OK"; then
    pass "$1"
  else
    fail "$1 (rc=$3, wanted: $4)"
    echo "$2" >&2
  fi
}

OUT=$(release "$CODEC" "$CODEC")
RC=$?
expect_ok "static binaries carrying SQLCipher + OpenSSL 3.6.3 pass" "$OUT" "$RC" \
  "ok: x86_64 static (no INTERP, no NEEDED)" "ok: aarch64 static AArch64 (no INTERP, no NEEDED)" \
  "ok: x86_64 links SQLCipher (cipher_plaintext_header_size) + OpenSSL 3.6.3, statically" \
  "ok: aarch64 links SQLCipher (cipher_plaintext_header_size) + OpenSSL 3.6.3, statically"

# The release env cargo saw, for each target.
for t in $X $A; do
  envf="$T/fake/cargo-env.$t"
  bad=$(grep -E '^([A-Z0-9_]*_)?(LIBSQLITE3_SYS_USE_PKG_CONFIG|LIBSQLITE3_FLAGS|SQLCIPHER_LIB_DIR|OPENSSL_NO_VENDOR|OPENSSL_DIR|OPENSSL_LIB_DIR|OPENSSL_STATIC)=' "$envf" 2>/dev/null)
  missing=
  for kv in CC_x86_64_unknown_linux_musl=x86_64-linux-musl-gcc AR_x86_64_unknown_linux_musl=x86_64-linux-musl-ar \
    CC_aarch64_unknown_linux_musl=aarch64-linux-musl-gcc AR_aarch64_unknown_linux_musl=aarch64-linux-musl-ar \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc; do
    grep -qx "$kv" "$envf" 2>/dev/null || missing="$missing $kv"
  done
  if [ -f "$envf" ] && [ -z "$bad" ] && [ -z "$missing" ]; then
    pass "cargo ($t) ran with CC/AR/linkers pinned and every SQLite/OpenSSL escape hatch unset"
  else
    fail "release env for $t: set:${bad:+ $bad} missing:${missing:- none}"
  fi
done

OUT=$(release "$OWN_STRINGS" "$CODEC")
expect_refused "x86_64 with only mesh111's own SQLCipher strings (plain SQLite) refused" "$OUT" $? \
  "carries no SQLCipher codec (cipher_plaintext_header_size)"

OUT=$(release "$CODEC" "$OWN_STRINGS")
expect_refused "aarch64 with only mesh111's own SQLCipher strings (plain SQLite) refused" "$OUT" $? \
  "carries no SQLCipher codec (cipher_plaintext_header_size)"

OUT=$(release "$CODEC" "cipher_plaintext_header_size OpenSSL 3.6.30 1 Jan 2027")
expect_refused "aarch64 carrying another OpenSSL than Cargo.lock's openssl-src refused" "$OUT" $? \
  "carries no statically linked OpenSSL 3.6.3"

OUT=$(release "cipher_plaintext_header_size" "$CODEC")
expect_refused "x86_64 without OpenSSL's version text (system libcrypto) refused" "$OUT" $? \
  "carries no statically linked OpenSSL 3.6.3"

OUT=$(release "$CODEC FAKE-INTERP" "$CODEC")
expect_refused "x86_64 with a PT_INTERP header refused" "$OUT" $? "111-x86_64-unknown-linux-musl is dynamically linked"

OUT=$(release "$CODEC" "$CODEC FAKE-NEEDED")
expect_refused "aarch64 with a DT_NEEDED entry refused" "$OUT" $? "111-aarch64-unknown-linux-musl is dynamically linked"

OUT=$(release "$CODEC" "$CODEC" plain)
RC=$?
expect_ok "no mesh111 in Cargo.lock: the SQLCipher assert says it does not apply" "$OUT" "$RC" \
  "SQLCipher assert not applicable"

# mesh111 locked but no openssl-src (+<OpenSSL version>) to name the expected OpenSSL.
rm -rf "$T/target" "$T/out"
printf '[[package]]\nname = "mesh111"\nversion = "0.1.0"\n' >"$T/10-bootstrap/Cargo.lock"
OUT=$(env -i HOME="$T/home" PATH="$T/stubs:$T/sysbin" CARGO_TARGET_DIR="$T/target" FAKE="$T/fake" \
  "$SH" "$T/10-bootstrap/scripts/cross-build.sh" --allow-no-ui 2026.10.14 "$T/out" 2>&1)
expect_refused "mesh111 without openssl-src in Cargo.lock refused" "$OUT" $? "Cargo.lock has mesh111 but no openssl-src"

if [ "$FAILS" -eq 0 ]; then
  printf '%s\n' "asserts-gate: ALL CASES GREEN"
  exit 0
fi
printf '%s\n' "asserts-gate: $FAILS case(s) failed"
exit 1
